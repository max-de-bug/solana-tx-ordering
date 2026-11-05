use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{error, info, warn};

/// Thread-safe spend tracker enforcing the strict bounty spend cap (approx. 0.05 SOL)
#[derive(Debug, Clone)]
pub struct SpendTracker {
    cap_lamports: u64,
    spent_lamports: Arc<AtomicU64>,
    total_tx_sent: Arc<AtomicU64>,
    total_tx_confirmed: Arc<AtomicU64>,
}

impl SpendTracker {
    pub fn new(cap_sol: f64) -> Self {
        let cap_lamports = (cap_sol * 1_000_000_000.0).round() as u64;
        Self {
            cap_lamports,
            spent_lamports: Arc::new(AtomicU64::new(0)),
            total_tx_sent: Arc::new(AtomicU64::new(0)),
            total_tx_confirmed: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Pre-checks if sending a transaction with the given maximum estimated fee is permitted.
    /// Returns Ok(()) if within budget, or Err if it would breach the spend cap.
    pub fn pre_check_spend(&self, estimated_fee_lamports: u64) -> anyhow::Result<()> {
        let current_spent = self.spent_lamports.load(Ordering::SeqCst);
        let projected = current_spent.saturating_add(estimated_fee_lamports);

        if projected > self.cap_lamports {
            let current_sol = current_spent as f64 / 1_000_000_000.0;
            let cap_sol = self.cap_lamports as f64 / 1_000_000_000.0;
            let proj_sol = projected as f64 / 1_000_000_000.0;
            error!(
                "SPEND CAP ENFORCEMENT: Transaction rejected! Current: {:.6} SOL, Projected: {:.6} SOL, Cap: {:.6} SOL",
                current_sol, proj_sol, cap_sol
            );
            anyhow::bail!(
                "Spend cap breached: cannot send tx. Current spend {:.6} SOL + est fee {:.6} SOL > cap {:.6} SOL",
                current_sol,
                estimated_fee_lamports as f64 / 1_000_000_000.0,
                cap_sol
            );
        }

        Ok(())
    }

    /// Records an actual fee spent after transaction confirmation
    #[allow(dead_code)]
    pub fn record_spend(&self, actual_fee_lamports: u64) {
        let prev = self.spent_lamports.fetch_add(actual_fee_lamports, Ordering::SeqCst);
        let new_total = prev + actual_fee_lamports;
        self.total_tx_confirmed.fetch_add(1, Ordering::SeqCst);

        let sol = new_total as f64 / 1_000_000_000.0;
        let cap = self.cap_lamports as f64 / 1_000_000_000.0;
        let pct = (new_total as f64 / self.cap_lamports as f64) * 100.0;

        info!(
            "Fee confirmed: +{} lamports. Total spent: {:.6} / {:.6} SOL ({:.1}% of cap)",
            actual_fee_lamports, sol, cap, pct
        );

        if new_total >= self.cap_lamports {
            warn!("SPEND CAP REACHED! No further transactions will be permitted.");
        }
    }

    pub fn increment_sent(&self) {
        self.total_tx_sent.fetch_add(1, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub fn current_spent_lamports(&self) -> u64 {
        self.spent_lamports.load(Ordering::SeqCst)
    }

    pub fn current_spent_sol(&self) -> f64 {
        self.spent_lamports.load(Ordering::SeqCst) as f64 / 1_000_000_000.0
    }

    pub fn cap_sol(&self) -> f64 {
        self.cap_lamports as f64 / 1_000_000_000.0
    }

    pub fn remaining_sol(&self) -> f64 {
        let spent = self.spent_lamports.load(Ordering::SeqCst);
        if spent >= self.cap_lamports {
            0.0
        } else {
            (self.cap_lamports - spent) as f64 / 1_000_000_000.0
        }
    }

    #[allow(dead_code)]
    pub fn total_sent(&self) -> u64 {
        self.total_tx_sent.load(Ordering::SeqCst)
    }

    pub fn total_confirmed(&self) -> u64 {
        self.total_tx_confirmed.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spend_tracker_cap_enforcement() {
        // Cap at 0.0001 SOL = 100,000 lamports
        let tracker = SpendTracker::new(0.0001);
        assert_eq!(tracker.cap_sol(), 0.0001);

        // Pre-check 40,000 lamports should succeed
        assert!(tracker.pre_check_spend(40_000).is_ok());

        // Record 40,000 spend
        tracker.record_spend(40_000);
        assert_eq!(tracker.current_spent_lamports(), 40_000);
        assert_eq!(tracker.total_confirmed(), 1);

        // Pre-check another 50,000 should succeed (total 90,000 <= 100,000)
        assert!(tracker.pre_check_spend(50_000).is_ok());
        tracker.record_spend(50_000);
        assert_eq!(tracker.current_spent_lamports(), 90_000);

        // Pre-check 20,000 should fail (90,000 + 20,000 = 110,000 > 100,000)
        assert!(tracker.pre_check_spend(20_000).is_err());
    }

    #[test]
    fn test_spend_tracker_zero_cap() {
        let tracker = SpendTracker::new(0.0);
        assert!(tracker.pre_check_spend(1).is_err());
    }
}
