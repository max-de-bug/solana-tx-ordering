use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{error, warn};

#[derive(Debug, Default)]
struct Ledger {
    spent_lamports: u64,
    reserved_lamports: u64,
    total_tx_sent: u64,
    total_tx_confirmed: u64,
}

/// Thread-safe, persistent spend accounting. A send reserves its worst-case
/// fee before dispatch; the actual fee is committed afterwards and any unused
/// reservation is released. The journal makes cap accounting auditable across
/// process restarts.
#[derive(Debug, Clone)]
pub struct SpendTracker {
    cap_lamports: u64,
    ledger: Arc<Mutex<Ledger>>,
    journal_path: Option<PathBuf>,
    start_spent_lamports: u64,
}

#[allow(dead_code)]
impl SpendTracker {
    pub fn new(cap_sol: f64) -> Self {
        Self::with_optional_journal(cap_sol, None).expect("in-memory spend tracker cannot fail")
    }

    /// Opens a persistent journal and replays every committed entry into the
    /// ledger before any new reservation is allowed. A malformed journal is a
    /// hard error: silently starting from zero would let a restart exceed the
    /// 0.05 SOL safety cap.
    pub fn with_journal(cap_sol: f64, path: impl Into<PathBuf>) -> Result<Self> {
        Self::with_optional_journal(cap_sol, Some(path.into()))
    }

    fn with_optional_journal(cap_sol: f64, journal_path: Option<PathBuf>) -> Result<Self> {
        let cap_lamports = (cap_sol * 1_000_000_000.0).round() as u64;
        let mut spent_lamports = 0u64;
        if let Some(path) = &journal_path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if path.exists() {
                let file = fs::File::open(path)
                    .with_context(|| format!("open spend journal {}", path.display()))?;
                for (line_no, line) in BufReader::new(file).lines().enumerate() {
                    let line = line.with_context(|| {
                        format!("read spend journal {} line {}", path.display(), line_no + 1)
                    })?;
                    if line.trim().is_empty() {
                        continue;
                    }
                    let entry: serde_json::Value =
                        serde_json::from_str(&line).with_context(|| {
                            format!(
                                "corrupt spend journal {} line {}; refusing to reset the cap",
                                path.display(),
                                line_no + 1
                            )
                        })?;
                    let lamports = entry
                        .get("lamports")
                        .and_then(|v| v.as_u64())
                        .with_context(|| {
                            format!(
                                "spend journal {} line {} has no lamports field",
                                path.display(),
                                line_no + 1
                            )
                        })?;
                    spent_lamports = spent_lamports.saturating_add(lamports);
                }
            }
        }
        if spent_lamports > cap_lamports {
            anyhow::bail!(
                "SAFETY ERROR: replayed spend journal already totals {} lamports, above cap {}",
                spent_lamports,
                cap_lamports
            );
        }
        Ok(Self {
            cap_lamports,
            ledger: Arc::new(Mutex::new(Ledger {
                spent_lamports,
                ..Ledger::default()
            })),
            journal_path,
            start_spent_lamports: spent_lamports,
        })
    }

    pub fn cap_lamports(&self) -> u64 {
        self.cap_lamports
    }

    pub fn spent_lamports(&self) -> u64 {
        self.ledger
            .lock()
            .expect("spend ledger poisoned")
            .spent_lamports
    }

    /// Spend made by this process, excluding journal entries replayed at
    /// startup. This is the value a single campaign report should show.
    pub fn campaign_spent_lamports(&self) -> u64 {
        self.spent_lamports()
            .saturating_sub(self.start_spent_lamports)
    }

    pub fn reserved_lamports(&self) -> u64 {
        self.ledger
            .lock()
            .expect("spend ledger poisoned")
            .reserved_lamports
    }

    pub fn remaining_lamports(&self) -> u64 {
        let ledger = self.ledger.lock().expect("spend ledger poisoned");
        self.cap_lamports
            .saturating_sub(ledger.spent_lamports)
            .saturating_sub(ledger.reserved_lamports)
    }

    pub fn total_tx_sent(&self) -> u64 {
        self.ledger
            .lock()
            .expect("spend ledger poisoned")
            .total_tx_sent
    }

    pub fn total_tx_confirmed(&self) -> u64 {
        self.ledger
            .lock()
            .expect("spend ledger poisoned")
            .total_tx_confirmed
    }

    /// Atomically reserve worst-case fees for a batch. The reservation is
    /// released automatically if it is dropped without a commit.
    pub fn try_reserve(&self, lamports: u64) -> Result<SpendReservation> {
        let mut ledger = self.ledger.lock().expect("spend ledger poisoned");
        let committed = ledger
            .spent_lamports
            .saturating_add(ledger.reserved_lamports);
        if committed.saturating_add(lamports) > self.cap_lamports {
            anyhow::bail!(
                "SAFETY ERROR: spend cap exceeded: already spent/reserved {:.9} SOL, requested {:.9} SOL, cap {:.9} SOL",
                committed as f64 / 1_000_000_000.0,
                lamports as f64 / 1_000_000_000.0,
                self.cap_lamports as f64 / 1_000_000_000.0
            );
        }
        ledger.reserved_lamports = ledger.reserved_lamports.saturating_add(lamports);
        Ok(SpendReservation {
            tracker: self.clone(),
            reserved: lamports,
            active: true,
        })
    }

    /// Convenience check for callers that do not need a reservation object.
    pub fn pre_check_spend(&self, estimated_fee_lamports: u64) -> Result<()> {
        drop(self.try_reserve(estimated_fee_lamports)?);
        Ok(())
    }

    pub fn record_sent(&self) {
        self.ledger
            .lock()
            .expect("spend ledger poisoned")
            .total_tx_sent += 1;
    }

    pub fn record_confirmed(&self) {
        self.ledger
            .lock()
            .expect("spend ledger poisoned")
            .total_tx_confirmed += 1;
    }

    /// Records an already-committed spend without a prior reservation. Use
    /// sparingly; the reservation API is preferred.
    pub fn record_spend(&self, lamports: u64) -> Result<()> {
        let reservation = self.try_reserve(lamports)?;
        reservation.commit(lamports)
    }

    fn release(&self, lamports: u64) {
        let mut ledger = self.ledger.lock().expect("spend ledger poisoned");
        ledger.reserved_lamports = ledger.reserved_lamports.saturating_sub(lamports);
    }

    fn commit(&self, reserved: u64, actual: u64) -> Result<()> {
        let mut ledger = self.ledger.lock().expect("spend ledger poisoned");
        ledger.reserved_lamports = ledger.reserved_lamports.saturating_sub(reserved);
        ledger.spent_lamports = ledger.spent_lamports.saturating_add(actual);
        if actual > reserved {
            warn!(
                "Actual fee {} lamports exceeded reserved {} lamports; accounting committed anyway.",
                actual, reserved
            );
        }
        if ledger.spent_lamports > self.cap_lamports {
            error!(
                "Spent total {} lamports exceeds configured cap {} lamports.",
                ledger.spent_lamports, self.cap_lamports
            );
        }
        drop(ledger);
        self.append_journal(actual)
    }

    fn append_journal(&self, lamports: u64) -> Result<()> {
        let Some(path) = &self.journal_path else {
            return Ok(());
        };
        let now = chrono::Utc::now().to_rfc3339();
        let spent = self.spent_lamports();
        let line = serde_json::json!({
            "timestamp_utc": now,
            "lamports": lamports,
            "spent_lamports_total": spent,
            "cap_lamports": self.cap_lamports,
        });
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("open spend journal {}", path.display()))?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    pub fn journal_path(&self) -> Option<&Path> {
        self.journal_path.as_deref()
    }

    pub fn summary(&self) -> SpendSummary {
        let ledger = self.ledger.lock().expect("spend ledger poisoned");
        SpendSummary {
            cap_lamports: self.cap_lamports,
            spent_lamports: ledger.spent_lamports,
            campaign_spent_lamports: ledger
                .spent_lamports
                .saturating_sub(self.start_spent_lamports),
            reserved_lamports: ledger.reserved_lamports,
            remaining_lamports: self
                .cap_lamports
                .saturating_sub(ledger.spent_lamports)
                .saturating_sub(ledger.reserved_lamports),
            total_tx_sent: ledger.total_tx_sent,
            total_tx_confirmed: ledger.total_tx_confirmed,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpendSummary {
    pub cap_lamports: u64,
    pub spent_lamports: u64,
    pub campaign_spent_lamports: u64,
    pub reserved_lamports: u64,
    pub remaining_lamports: u64,
    pub total_tx_sent: u64,
    pub total_tx_confirmed: u64,
}

/// RAII reservation. Dropping without committing releases the reserved amount.
#[derive(Debug)]
pub struct SpendReservation {
    tracker: SpendTracker,
    reserved: u64,
    active: bool,
}

impl SpendReservation {
    #[allow(dead_code)]
    pub fn reserved_lamports(&self) -> u64 {
        self.reserved
    }

    pub fn commit(mut self, actual_lamports: u64) -> Result<()> {
        self.active = false;
        self.tracker.commit(self.reserved, actual_lamports)
    }
}

impl Drop for SpendReservation {
    fn drop(&mut self) {
        if self.active {
            self.tracker.release(self.reserved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_blocks_over_cap() {
        let tracker = SpendTracker::new(0.00001);
        let _res = tracker.try_reserve(9_000).unwrap();
        assert!(tracker.try_reserve(2_000).is_err());
    }

    #[test]
    fn dropping_reservation_releases_capacity() {
        let tracker = SpendTracker::new(0.00001);
        {
            let _res = tracker.try_reserve(9_000).unwrap();
        }
        assert!(tracker.try_reserve(9_000).is_ok());
    }

    #[test]
    fn commit_records_actual_not_reserved() {
        let tracker = SpendTracker::new(0.0001);
        let res = tracker.try_reserve(10_000).unwrap();
        res.commit(5_001).unwrap();
        assert_eq!(tracker.spent_lamports(), 5_001);
        assert_eq!(tracker.reserved_lamports(), 0);
    }

    #[test]
    fn journal_is_append_only_jsonl() {
        let path = std::env::temp_dir().join(format!(
            "solana-tx-order-spend-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let tracker = SpendTracker::with_journal(0.001, &path).unwrap();
        tracker.try_reserve(5_001).unwrap().commit(5_001).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"lamports\":5001"));
        let replayed = SpendTracker::with_journal(0.001, &path).unwrap();
        assert_eq!(replayed.spent_lamports(), 5_001);
        assert_eq!(replayed.campaign_spent_lamports(), 0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn replayed_journal_still_enforces_cap() {
        let path = std::env::temp_dir().join(format!(
            "solana-tx-order-spend-replay-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let tracker = SpendTracker::with_journal(0.00002, &path).unwrap();
        tracker.try_reserve(15_000).unwrap().commit(15_000).unwrap();
        let replayed = SpendTracker::with_journal(0.00002, &path).unwrap();
        assert!(replayed.try_reserve(6_000).is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn corrupt_journal_refuses_to_reset_cap() {
        let path = std::env::temp_dir().join(format!(
            "solana-tx-order-spend-corrupt-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        fs::write(&path, "not json\n").unwrap();
        assert!(SpendTracker::with_journal(0.05, &path).is_err());
        let _ = fs::remove_file(path);
    }
}
