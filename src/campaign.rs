use crate::analyzer::{BurstAnalysisResult, ExecutionMode, PayerMode, TransportKind};
use crate::rpc::DetailedBlock;
use crate::tx_crafting::BurstPattern;
use crate::validator_info::TargetValidator;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const CAMPAIGN_FILE: &str = "campaign.json";
pub const BURSTS_FILE: &str = "bursts.json";
pub const RAW_BLOCKS_DIR: &str = "raw_blocks";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignMeta {
    pub campaign_id: String,
    pub created_at_utc: String,
    pub finished_at_utc: Option<String>,
    pub execution_mode: ExecutionMode,
    pub design_version: u32,
    pub rpc_url: String,
    pub transport: TransportKind,
    pub payer_mode: PayerMode,
    pub send_lead_slots: u64,
    pub target_window_slots: u64,
    pub burst_size: usize,
    pub repetitions: usize,
    pub patterns: Vec<BurstPattern>,
    pub seed: u64,
    pub min_blocks: usize,
    pub permutation_trials: usize,
    pub spend_cap_lamports: u64,
    pub total_bursts: usize,
    pub total_tx_sent: usize,
    pub total_tx_landed: usize,
    pub total_spent_lamports: u64,
    pub memo_prefix: String,
    pub validators: Vec<TargetValidator>,
    pub notes: Vec<String>,
}

impl CampaignMeta {
    pub fn new(
        campaign_id: impl Into<String>,
        execution_mode: ExecutionMode,
        rpc_url: impl Into<String>,
    ) -> Self {
        Self {
            campaign_id: campaign_id.into(),
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            finished_at_utc: None,
            execution_mode,
            design_version: 2,
            rpc_url: rpc_url.into(),
            transport: TransportKind::Rpc,
            payer_mode: PayerMode::Shared,
            send_lead_slots: 2,
            target_window_slots: 4,
            burst_size: 8,
            repetitions: 1,
            patterns: Vec::new(),
            seed: 42,
            min_blocks: 3,
            permutation_trials: 10_000,
            spend_cap_lamports: 50_000_000,
            total_bursts: 0,
            total_tx_sent: 0,
            total_tx_landed: 0,
            total_spent_lamports: 0,
            memo_prefix: crate::tx_crafting::MEMO_PREFIX.to_string(),
            validators: Vec::new(),
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Campaign {
    pub meta: CampaignMeta,
    pub bursts: Vec<BurstAnalysisResult>,
    pub dir: Option<PathBuf>,
}

impl Campaign {
    pub fn new(meta: CampaignMeta, bursts: Vec<BurstAnalysisResult>) -> Self {
        Self {
            meta,
            bursts,
            dir: None,
        }
    }

    pub fn save(&self, root: &Path) -> Result<PathBuf> {
        let dir = if root.ends_with(CAMPAIGN_FILE)
            || root.file_name().is_some_and(|n| n == BURSTS_FILE)
        {
            root.parent().unwrap_or(root).to_path_buf()
        } else {
            root.to_path_buf()
        };
        fs::create_dir_all(&dir)
            .with_context(|| format!("create campaign directory {}", dir.display()))?;
        fs::create_dir_all(dir.join(RAW_BLOCKS_DIR))?;

        let campaign_json = serde_json::to_string_pretty(&self.meta)?;
        fs::write(dir.join(CAMPAIGN_FILE), campaign_json)?;
        let bursts_json = serde_json::to_string_pretty(&self.bursts)?;
        fs::write(dir.join(BURSTS_FILE), bursts_json)?;
        Ok(dir)
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let meta_path = dir.join(CAMPAIGN_FILE);
        let bursts_path = dir.join(BURSTS_FILE);
        let meta: CampaignMeta = serde_json::from_str(
            &fs::read_to_string(&meta_path)
                .with_context(|| format!("read {}", meta_path.display()))?,
        )?;
        let bursts: Vec<BurstAnalysisResult> = serde_json::from_str(
            &fs::read_to_string(&bursts_path)
                .with_context(|| format!("read {}", bursts_path.display()))?,
        )?;
        Ok(Self {
            meta,
            bursts,
            dir: Some(dir.to_path_buf()),
        })
    }

    /// Loads the newest campaign under `data/campaigns` (then `data/simulated`).
    pub fn load_latest(data_dir: &Path) -> Result<Self> {
        for subdir in ["campaigns", "simulated"] {
            let root = data_dir.join(subdir);
            if !root.exists() {
                continue;
            }
            let mut dirs: Vec<PathBuf> = fs::read_dir(&root)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.is_dir() && path.join(CAMPAIGN_FILE).exists())
                .collect();
            dirs.sort();
            if let Some(dir) = dirs.pop() {
                return Self::load(&dir);
            }
        }
        anyhow::bail!(
            "no campaign found under {}; run a dry-run or pass --campaign-dir",
            data_dir.display()
        )
    }

    /// Loads the v1 `measurements.json` array. It is always labelled Legacy.
    pub fn load_legacy(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("read legacy measurements {}", path.display()))?;
        let bursts: Vec<BurstAnalysisResult> = serde_json::from_str(&raw)
            .with_context(|| format!("parse legacy measurements {}", path.display()))?;
        let mut meta = CampaignMeta::new(
            "legacy-v1",
            ExecutionMode::Legacy,
            "https://api.mainnet-beta.solana.com",
        );
        meta.design_version = 1;
        meta.created_at_utc = "2026-11-05T12:23:07Z".to_string();
        meta.finished_at_utc = Some("2026-11-05T12:43:52Z".to_string());
        meta.send_lead_slots = 0;
        meta.memo_prefix = "superteam-ua:".to_string();
        meta.total_bursts = bursts.len();
        meta.total_tx_sent = bursts.iter().map(|b| b.total_tx_sent).sum();
        meta.total_tx_landed = bursts.iter().map(|b| b.total_tx_landed).sum();
        meta.total_spent_lamports = bursts
            .iter()
            .flat_map(|b| b.txs.iter())
            .map(|tx| tx.fee_paid_lamports)
            .sum();
        meta.patterns = {
            let mut patterns = Vec::new();
            for burst in &bursts {
                if !patterns.contains(&burst.pattern) {
                    patterns.push(burst.pattern);
                }
            }
            patterns
        };
        meta.validators = {
            let mut validators: Vec<TargetValidator> = Vec::new();
            for burst in &bursts {
                if validators
                    .iter()
                    .any(|v| v.identity == burst.target_validator_identity)
                {
                    continue;
                }
                validators.push(TargetValidator {
                    identity: burst.target_validator_identity.clone(),
                    name: format!(
                        "Legacy observed validator {}",
                        &burst.target_validator_identity
                            [..burst.target_validator_identity.len().min(8)]
                    ),
                    client_type: burst.validator_client_type,
                    version_string: "not recorded in v1".to_string(),
                    stake_sol: 0.0,
                    tpu_ip: None,
                });
            }
            validators
        };
        meta.notes.push(
            "Legacy v1 dataset imported from measurements.json. No target-block probes; fee tiers \
             collapsed into on-chain ties; original report used cross-block correlations."
                .to_string(),
        );
        meta.notes.push(
            "Measurement window recovered from mainnet getBlockTime: target slot 465703960 = \
             2026-11-05T12:23:07Z; final landed slot 465709787 = 2026-11-05T12:43:52Z. The old \
             report's 19:03 UTC timestamp was wrong."
                .to_string(),
        );
        meta.notes.push(
            "Legacy send timestamps were synthesized at fixed 80 ms intervals; send-order results \
             are not empirical and are suppressed in the report."
                .to_string(),
        );
        meta.notes.push(
            "On-chain memo prefix was superteam-ua:, not the v2 solana-tx-order prefix."
                .to_string(),
        );
        Ok(Self {
            meta,
            bursts,
            dir: None,
        })
    }

    #[allow(dead_code)]
    pub fn save_block(&self, block: &DetailedBlock) -> Result<PathBuf> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("campaign has no directory; call save first"))?;
        save_block_in(dir, block)
    }

    #[allow(dead_code)]
    pub fn load_block(&self, slot: u64) -> Result<DetailedBlock> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("campaign has no directory"))?;
        let path = dir.join(RAW_BLOCKS_DIR).join(format!("{slot}.json"));
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read raw block {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parse raw block {}", path.display()))
    }

    pub fn verifier_output_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|dir| dir.join("verification.json"))
    }
}

pub fn save_block_in(dir: &Path, block: &DetailedBlock) -> Result<PathBuf> {
    let raw_dir = dir.join(RAW_BLOCKS_DIR);
    fs::create_dir_all(&raw_dir)?;
    let path = raw_dir.join(format!("{}.json", block.slot));
    fs::write(&path, serde_json::to_string_pretty(block)?)?;
    Ok(path)
}

pub fn campaign_id_now(prefix: &str) -> String {
    format!("{}-{}", prefix, chrono::Utc::now().format("%Y%m%dT%H%M%SZ"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{BurstProvenance, ExecutionMode};

    fn sample_burst() -> BurstAnalysisResult {
        BurstAnalysisResult {
            burst_id: "b".to_string(),
            pattern: BurstPattern::RandomizedFee,
            target_validator_identity: "v".to_string(),
            validator_client_type: crate::validator_info::ClientType::Agave,
            target_slot: 1,
            landed_slot: 1,
            slot_lag: 0,
            total_tx_sent: 0,
            total_tx_landed: 0,
            landing_rate_pct: 0.0,
            txs: Vec::new(),
            spearman_fee_correlation: 0.0,
            spearman_send_correlation: 0.0,
            kendall_tau_fee: 0.0,
            p_value_fee: 1.0,
            p_value_send: 1.0,
            burst_classification: String::new(),
            provenance: BurstProvenance {
                execution_mode: ExecutionMode::DryRun,
                design_version: 2,
                ..BurstProvenance::default()
            },
            misses: Vec::new(),
            target_window_start: 1,
            target_window_end: 4,
            target_block_probes: 0,
            target_block_usable: false,
            same_block_fee_tau: None,
            same_block_send_tau: None,
        }
    }

    #[test]
    fn campaign_round_trips() {
        let root = std::env::temp_dir().join(format!(
            "solana-tx-order-campaign-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let meta = CampaignMeta::new("test", ExecutionMode::DryRun, "http://localhost");
        let campaign = Campaign::new(meta, vec![sample_burst()]);
        campaign.save(&root).unwrap();
        let loaded = Campaign::load(&root).unwrap();
        assert_eq!(loaded.meta.campaign_id, "test");
        assert_eq!(loaded.bursts.len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
