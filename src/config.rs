use clap::{Parser, ValueEnum};
use std::path::PathBuf;

use crate::analyzer::{AnalysisOptions, PayerMode, TransportKind};
use crate::tx_crafting::BurstPattern;

const MAX_LIVE_SPEND_CAP_SOL: f64 = 0.05;

/// Supported experiment suites. The v2 campaign only runs the ordering suite;
/// contention/saturation were dry-run simulators in v1 and are retained only
/// as explicit, rejected legacy selectors so old commands fail loudly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
pub enum ExperimentType {
    /// Same-block ordering campaign (v2, the only supported suite)
    Ordering,
    /// Removed v1 simulator; rejected in v2
    Contention,
    /// Removed v1 simulator; rejected in v2
    Saturation,
    /// Alias for Ordering
    All,
}

impl std::fmt::Display for ExperimentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExperimentType::Ordering => write!(f, "ordering"),
            ExperimentType::Contention => write!(f, "contention"),
            ExperimentType::Saturation => write!(f, "saturation"),
            ExperimentType::All => write!(f, "all"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PatternArg {
    Ascending,
    Descending,
    Alternating,
    Uniform,
    Randomized,
}

impl PatternArg {
    pub fn to_pattern(self) -> BurstPattern {
        match self {
            PatternArg::Ascending => BurstPattern::AscendingFee,
            PatternArg::Descending => BurstPattern::DescendingFee,
            PatternArg::Alternating => BurstPattern::AlternatingFee,
            PatternArg::Uniform => BurstPattern::UniformFee,
            PatternArg::Randomized => BurstPattern::RandomizedFee,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransportArg {
    /// Direct QUIC write to the target validator's tpuQuic endpoint
    TpuQuic,
    /// Batched JSON-RPC sendTransaction (fallback; less precise)
    Rpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PayerModeArg {
    /// One fee payer for all probes; confounded by the fee-payer write lock
    Shared,
    /// One funded payer per probe, loaded from --payer-pool-file
    Independent,
}

/// Solana Validator Transaction Ordering Experiment Engine
#[derive(Parser, Debug, Clone)]
#[command(
    name = "solana-validator-ordering",
    author = "Solana Validator Research Team",
    version = "2.0.0",
    about = "Same-block measurement of transaction ordering across Solana validator clients"
)]
pub struct Config {
    /// Experiment suite. Only `ordering` is supported in v2.
    #[arg(long, value_enum, default_value_t = ExperimentType::Ordering, env = "EXPERIMENT_TYPE")]
    pub experiment: ExperimentType,

    /// Solana JSON-RPC endpoint URL (mainnet-beta by default)
    #[arg(
        long,
        env = "SOLANA_RPC_URL",
        default_value = "https://api.mainnet-beta.solana.com"
    )]
    pub rpc_url: String,

    /// Shared fee-payer keypair JSON (required for --live unless independent payers are used)
    #[arg(long, env = "SOLANA_KEYPAIR_PATH")]
    pub keypair_path: Option<PathBuf>,

    /// JSON array of independent payer keypairs (64-byte arrays or base58 strings).
    /// Required for --payer-mode independent. Never commit this file.
    #[arg(long)]
    pub payer_pool_file: Option<PathBuf>,

    /// Maximum SOL allowed to be spent across the campaign (live hard cap: 0.05)
    #[arg(long, env = "SPEND_CAP_SOL", default_value_t = 0.05)]
    pub spend_cap_sol: f64,

    /// Explicit no-broadcast mode. This is the default; the flag exists so
    /// scripts can be explicit.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Explicit opt-in required to broadcast REAL mainnet transactions
    #[arg(long, default_value_t = false)]
    pub live: bool,

    /// Number of transactions per leader-window burst
    #[arg(long, default_value_t = 8)]
    pub burst_size: usize,

    /// Number of independent leader-window repetitions per validator/pattern
    #[arg(long, default_value_t = 1)]
    pub repetitions: usize,

    /// Fee topologies to run. `randomized` is the primary causal design.
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        default_value = "ascending,descending,alternating,uniform,randomized"
    )]
    pub patterns: Vec<PatternArg>,

    /// Target validators to analyze (comma-separated, empty = auto-discover)
    #[arg(long, value_delimiter = ',')]
    pub target_validators: Vec<String>,

    /// Maximum number of target validators during discovery
    #[arg(long, default_value_t = 14)]
    pub max_validators: usize,

    /// Hard minimum distinct validator identities for a live campaign
    #[arg(long, default_value_t = 6)]
    pub min_validators: usize,

    /// Hard minimum distinct client/fork classes for a live campaign
    #[arg(long, default_value_t = 3)]
    pub min_client_versions: usize,

    /// Permit a live campaign below the validator/client minimums (not recommended)
    #[arg(long, default_value_t = false)]
    pub allow_few_validators: bool,

    /// Dispatch this many slots before the leader window starts
    #[arg(long, default_value_t = 2)]
    pub send_lead_slots: u64,

    /// Leader window length in slots (1..=4; primary analysis remains same-block)
    #[arg(long, default_value_t = 4)]
    pub target_window_slots: u64,

    /// Fetch this many extra slots after the target for diagnostic accounting
    #[arg(long, default_value_t = 1)]
    pub max_landing_lag: u64,

    /// Minimum number of usable same-block clusters required for inference
    #[arg(long, default_value_t = 3)]
    pub min_blocks: usize,

    /// Monte Carlo trials for the stratified within-block permutation test
    #[arg(long, default_value_t = 10_000)]
    pub permutation_trials: usize,

    /// Deterministic seed for fee/send randomization and resampling
    #[arg(long, default_value_t = 42)]
    pub seed: u64,

    /// Ordering transport. tpu-quic is preferred; rpc is a fallback.
    #[arg(long, value_enum, default_value_t = TransportArg::TpuQuic)]
    pub transport: TransportArg,

    /// Fee-payer design. Independent payers remove the shared write-lock confound.
    #[arg(long, value_enum, default_value_t = PayerModeArg::Shared)]
    pub payer_mode: PayerModeArg,

    /// Maximum retries for RPC-dispatched transactions (keep 0 for measurement purity)
    #[arg(long, default_value_t = 0)]
    pub rpc_max_retries: u32,

    /// Directory for campaign data
    #[arg(long, default_value = "./data")]
    pub data_dir: PathBuf,

    /// Directory for reports
    #[arg(long, default_value = "./reports")]
    pub reports_dir: PathBuf,

    /// Explicit campaign directory to load for --analyze-only / --verify-only
    #[arg(long)]
    pub campaign_dir: Option<PathBuf>,

    /// Analyze existing campaign data without sending anything
    #[arg(long, default_value_t = false)]
    pub analyze_only: bool,

    /// Verify signatures, slots, indexes, fees, and leaders against chain data
    #[arg(long, default_value_t = false)]
    pub verify_only: bool,

    /// Also run on-chain verification after a live campaign
    #[arg(long, default_value_t = false)]
    pub verify: bool,

    /// Serve the static dashboard on localhost after report generation
    #[arg(long, default_value_t = false)]
    pub serve_dashboard: bool,

    /// Dashboard port
    #[arg(long, default_value_t = 1995)]
    pub port: u16,

    /// v1 flag retained only to produce a clear migration error
    #[arg(long, default_value_t = false, hide = true)]
    pub contention_benchmark: bool,
}

impl Config {
    pub fn is_live(&self) -> bool {
        self.live
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.live && self.dry_run {
            anyhow::bail!(
                "SAFETY ERROR: --live and --dry-run are mutually exclusive. Omit --dry-run for a live campaign."
            );
        }

        if self.contention_benchmark {
            anyhow::bail!(
                "The v1 contention benchmark was a dry-run simulator and has been removed. \
                 Use --experiment ordering; it now records payer mode and send timing for every probe."
            );
        }

        match self.effective_experiment() {
            ExperimentType::Ordering | ExperimentType::All => {}
            ExperimentType::Contention | ExperimentType::Saturation => {
                anyhow::bail!(
                    "The v1 {} suite was a synthetic simulator and cannot support empirical claims. \
                     Use --experiment ordering.",
                    self.effective_experiment()
                );
            }
        }

        if self.burst_size < 4 || self.burst_size > 32 {
            anyhow::bail!(
                "SAFETY ERROR: --burst-size must be between 4 and 32 (got {}).",
                self.burst_size
            );
        }
        if self.repetitions == 0 || self.repetitions > 50 {
            anyhow::bail!("--repetitions must be between 1 and 50.");
        }
        if self.patterns.is_empty() {
            anyhow::bail!("--patterns must contain at least one pattern.");
        }
        if self.send_lead_slots == 0 {
            anyhow::bail!(
                "SAFETY ERROR: --send-lead-slots must be >= 1; sending during the target window \
                 invalidates pre-window ordering inference."
            );
        }
        if self.send_lead_slots > 20 {
            anyhow::bail!("--send-lead-slots must be <= 20.");
        }
        if self.target_window_slots == 0 || self.target_window_slots > 4 {
            anyhow::bail!("--target-window-slots must be between 1 and 4.");
        }
        if self.min_blocks < 2 {
            anyhow::bail!("--min-blocks must be at least 2.");
        }
        if self.permutation_trials < 100 {
            anyhow::bail!("--permutation-trials must be at least 100.");
        }
        if self.max_landing_lag > 8 {
            anyhow::bail!("--max-landing-lag must be <= 8 slots.");
        }

        if !self.spend_cap_sol.is_finite() || self.spend_cap_sol < 0.0 {
            anyhow::bail!("SAFETY ERROR: --spend-cap-sol must be a finite, non-negative value.");
        }

        if self.live {
            if self.spend_cap_sol <= 0.0 {
                anyhow::bail!("SAFETY ERROR: --spend-cap-sol must be strictly positive (> 0.0).");
            }
            if self.spend_cap_sol > MAX_LIVE_SPEND_CAP_SOL {
                anyhow::bail!(
                    "SAFETY ERROR: Spend cap ({:.6} SOL) exceeds the maximum allowed live spend (0.050000 SOL).",
                    self.spend_cap_sol,
                );
            }
            if self.payer_mode == PayerModeArg::Independent {
                if self.payer_pool_file.is_none() {
                    anyhow::bail!(
                        "SAFETY ERROR: --payer-mode independent requires --payer-pool-file."
                    );
                }
            } else if self.keypair_path.is_none() {
                anyhow::bail!(
                    "SAFETY ERROR: --live mode requires --keypair-path or SOLANA_KEYPAIR_PATH \
                     (or --payer-mode independent with a funded pool)."
                );
            }
            if self.min_validators < 6 && !self.allow_few_validators {
                anyhow::bail!(
                    "SAFETY ERROR: --min-validators must be >= 6 for a bounty-grade live campaign \
                     (or pass --allow-few-validators)."
                );
            }
            if self.min_client_versions < 3 && !self.allow_few_validators {
                anyhow::bail!(
                    "SAFETY ERROR: --min-client-versions must be >= 3 for a bounty-grade live campaign \
                     (or pass --allow-few-validators)."
                );
            }
        }

        Ok(())
    }

    pub fn effective_experiment(&self) -> ExperimentType {
        match self.experiment {
            ExperimentType::All => ExperimentType::Ordering,
            other => other,
        }
    }

    pub fn spend_cap_lamports(&self) -> u64 {
        (self.spend_cap_sol * 1_000_000_000.0).round() as u64
    }

    pub fn analysis_options(&self) -> AnalysisOptions {
        AnalysisOptions {
            min_blocks: self.min_blocks,
            permutation_trials: self.permutation_trials,
            seed: self.seed,
        }
    }

    pub fn burst_patterns(&self) -> Vec<BurstPattern> {
        let mut out = Vec::new();
        for pattern in &self.patterns {
            let p = pattern.to_pattern();
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    pub fn transport_kind(&self) -> TransportKind {
        match self.transport {
            TransportArg::TpuQuic => TransportKind::TpuQuic,
            TransportArg::Rpc => TransportKind::Rpc,
        }
    }

    pub fn payer_mode_kind(&self) -> PayerMode {
        match self.payer_mode {
            PayerModeArg::Shared => PayerMode::Shared,
            PayerModeArg::Independent => PayerMode::Independent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_config(spend_cap_sol: &str) -> Config {
        let spend_cap_arg = format!("--spend-cap-sol={spend_cap_sol}");
        Config::try_parse_from([
            "solana-validator-ordering",
            "--live",
            "--keypair-path",
            "test-keypair.json",
            "--send-lead-slots",
            "2",
            spend_cap_arg.as_str(),
        ])
        .expect("test arguments should parse")
    }

    #[test]
    fn live_spend_cap_accepts_bounty_limit() {
        let config = live_config("0.05");
        assert!(config.validate().is_ok());
        assert_eq!(config.spend_cap_lamports(), 50_000_000);
    }

    #[test]
    fn live_spend_cap_rejects_values_above_bounty_limit() {
        let config = live_config("0.050000001");
        let error = config.validate().expect_err("cap above 0.05 SOL must fail");
        assert!(error.to_string().contains("0.050000 SOL"));
    }

    #[test]
    fn live_spend_cap_rejects_non_finite_values() {
        for value in ["NaN", "inf", "-inf"] {
            let config = live_config(value);
            assert!(config.validate().is_err(), "{value} must be rejected");
        }
    }

    #[test]
    fn lead_slots_must_be_positive() {
        let config =
            Config::try_parse_from(["solana-validator-ordering", "--send-lead-slots", "0"])
                .unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn retired_simulator_suites_are_rejected() {
        let config =
            Config::try_parse_from(["solana-validator-ordering", "--experiment", "contention"])
                .unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn live_independent_payers_require_pool() {
        let config = Config::try_parse_from([
            "solana-validator-ordering",
            "--live",
            "--payer-mode",
            "independent",
            "--send-lead-slots",
            "2",
        ])
        .unwrap();
        assert!(config.validate().is_err());
    }
}
