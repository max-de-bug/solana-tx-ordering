use clap::Parser;
use std::path::PathBuf;

/// Supported experiment suite modes
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
pub enum ExperimentType {
    /// Standard Ordering: 4 fee perturbation topologies (Ascending, Descending, Alternating, Uniform)
    Ordering,
    /// Write-Lock Contention: Evaluates Sealevel lock serialization (shared payer vs independent payers)
    Contention,
    /// Queue Saturation: Evaluates scheduler queue depth across varying burst densities (N=8, 16, 32)
    Saturation,
    /// Comprehensive Research Suite: Runs all experiment suites sequentially
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

/// Solana Validator Transaction Ordering Experiment Engine (Windows 95 Style)
#[derive(Parser, Debug, Clone)]
#[command(
    name = "solana-validator-ordering",
    author = "Solana Validator Research Team",
    version = "1.0.0-win95",
    about = "Measures how Solana validators order transactions within blocks across client forks"
)]
pub struct Config {
    /// Specific experiment suite to execute: ordering (default), contention, saturation, or all
    #[arg(long, value_enum, default_value_t = ExperimentType::Ordering, env = "EXPERIMENT_TYPE")]
    pub experiment: ExperimentType,

    /// Solana JSON-RPC endpoint URL (defaults to mainnet-beta public RPC)
    #[arg(
        long,
        env = "SOLANA_RPC_URL",
        default_value = "https://api.mainnet-beta.solana.com"
    )]
    pub rpc_url: String,

    /// Secondary fallback / TPU forwarding RPC (optional)
    #[arg(long, env = "SOLANA_BACKUP_RPC_URL")]
    pub backup_rpc_url: Option<String>,

    /// Path to fee-payer keypair JSON file (REQUIRED for --live, NEVER commit keys to git!)
    #[arg(long, env = "SOLANA_KEYPAIR_PATH")]
    pub keypair_path: Option<PathBuf>,

    /// Maximum SOL allowed to be spent across the entire experiment (SAFETY CAP)
    #[arg(long, env = "SPEND_CAP_SOL", default_value_t = 0.05)]
    pub spend_cap_sol: f64,

    /// Execute in DRY-RUN mode (safe by default: builds transactions, polls leader schedule,
    /// logs timings, but NEVER broadcasts to network)
    #[arg(long, default_value_t = true)]
    pub dry_run: bool,

    /// Explicit flag required to broadcast REAL transactions to Solana Mainnet
    #[arg(long, default_value_t = false)]
    pub live: bool,

    /// Number of transactions to send per leader window burst (e.g. 8, 16, 32)
    #[arg(long, default_value_t = 8)]
    pub burst_size: usize,

    /// Run dedicated Write-Lock Contention Benchmark (compares shared fee-payer vs independent fee-payers)
    #[arg(long, default_value_t = false)]
    pub contention_benchmark: bool,

    /// Maximum number of target validators to analyze (default: 14)
    #[arg(long, default_value_t = 14)]
    pub max_validators: usize,

    /// Directory where raw measurement JSON logs will be stored
    #[arg(long, default_value = "./data")]
    pub data_dir: PathBuf,

    /// Directory where HTML and Markdown reports will be generated
    #[arg(long, default_value = "./reports")]
    pub reports_dir: PathBuf,

    /// Generate reports from existing raw data in data_dir without running new network measurements
    #[arg(long, default_value_t = false)]
    pub analyze_only: bool,

    /// Target specific validator identity pubkeys (comma-separated, or empty to auto-discover)
    #[arg(long, value_delimiter = ',')]
    pub target_validators: Vec<String>,

    /// Serve the Windows 95 interactive dashboard on localhost after measurement
    #[arg(long, default_value_t = false)]
    pub serve_dashboard: bool,

    /// Port for the embedded Windows 95 dashboard server
    #[arg(long, default_value_t = 1995)]
    pub port: u16,
}

impl Config {
    /// Validates the configuration for safety
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.burst_size < 2 || self.burst_size > 64 {
            anyhow::bail!(
                "SAFETY ERROR: --burst-size must be between 2 and 64 (got {}).",
                self.burst_size
            );
        }

        if self.live {
            if self.keypair_path.is_none() {
                anyhow::bail!(
                    "SAFETY ERROR: --live mode requires --keypair-path or SOLANA_KEYPAIR_PATH env variable."
                );
            }
            if self.spend_cap_sol <= 0.0 {
                anyhow::bail!("SAFETY ERROR: --spend-cap-sol must be strictly positive (> 0.0).");
            }
            if self.spend_cap_sol > 0.10 {
                anyhow::bail!(
                    "SAFETY ERROR: Spend cap ({:.4} SOL) exceeds maximum safe limit (0.10 SOL).",
                    self.spend_cap_sol
                );
            }
        }
        Ok(())
    }

    /// Resolves the effective experiment type, preserving backwards compatibility with --contention-benchmark
    pub fn effective_experiment(&self) -> ExperimentType {
        if self.contention_benchmark {
            ExperimentType::Contention
        } else {
            self.experiment
        }
    }

    /// Convert spend cap in SOL to lamports
    #[allow(dead_code)]
    pub fn spend_cap_lamports(&self) -> u64 {
        (self.spend_cap_sol * 1_000_000_000.0).round() as u64
    }
}
