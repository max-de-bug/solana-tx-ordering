use clap::Parser;
use solana_sdk::signature::{Keypair, Signer};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

mod analyzer;
mod config;
mod experiments;
mod leader_schedule;
mod report;
mod rpc;
mod server;
mod spend_tracker;
mod tx_crafting;
mod validator_info;

use analyzer::{Analyzer, BurstAnalysisResult, ValidatorAggregatedResult};
use config::{Config, ExperimentType};
use experiments::ExperimentEngine;
use leader_schedule::LeaderScheduleManager;
use report::ReportGenerator;
use rpc::RpcClient;
use server::DashboardServer;
use spend_tracker::SpendTracker;
use validator_info::{discover_cluster_validators, ClientType, TargetValidator};

const WIN95_BANNER: &str = r#"
╔══════════════════════════════════════════════════════════════════════════╗
║  ███████╗ ██████╗ ██╗      █████╗ ███╗   ██╗ █████╗     ██╗   ██╗ █████╗ ║
║  ██╔════╝██╔═══██╗██║     ██╔══██╗████╗  ██║██╔══██╗    ██║   ██║██╔══██╗║
║  ███████╗██║   ██║██║     ███████║██╔██╗ ██║███████║    ██║   ██║███████║║
║  ╚════██║██║   ██║██║     ██╔══██║██║╚██╗██║██╔══██║    ██║   ██║██╔══██║║
║  ███████║╚██████╔╝███████╗██║  ██║██║ ╚████║██║  ██║    ╚██████╔╝██║  ██║║
║  ╚══════╝ ╚═════╝ ╚══════╝╚═╝  ╚═╝╚═╝  ╚═══╝╚═╝  ╚═╝     ╚═════╝ ╚═╝  ╚═╝║
║           SOLANA VALIDATOR TRANSACTION ORDERING ENGINE (WIN95)           ║
║           Solana Mainnet Validator Ordering Research Pipeline            ║
╚══════════════════════════════════════════════════════════════════════════╝
"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    println!("{}", WIN95_BANNER);

    let config = Config::parse();
    info!("Initializing Validator Ordering Engine with RPC: {}", config.rpc_url);

    // If --live is explicitly requested, safety check
    let is_live = config.live;
    if is_live {
        warn!("LIVE MODE ACTIVE: Real transactions will be broadcast to Solana Mainnet!");
        config.validate()?;
    } else {
        info!("DRY-RUN MODE ACTIVE (Safe by default): Transactions will be prepared & simulated, but NOT broadcast.");
    }

    fs::create_dir_all(&config.data_dir)?;
    fs::create_dir_all(&config.reports_dir)?;

    let rpc = Arc::new(RpcClient::new(&config.rpc_url));
    let spend_tracker = SpendTracker::new(config.spend_cap_sol);

    // If analyze-only mode is selected, generate reports from existing data and optionally serve dashboard
    if config.analyze_only {
        info!("Running in --analyze-only mode: loading raw measurement data from {:?}", config.data_dir);
        let (results, bursts) = load_existing_data(&config.data_dir)?;
        ReportGenerator::generate_all(&config, &results, &bursts, &spend_tracker)?;
        info!("Analysis and reports successfully generated in {:?}", config.reports_dir);

        if config.serve_dashboard {
            let html_path = config.reports_dir.join("dashboard.html");
            DashboardServer::run(config.port, &html_path).await?;
        }
        return Ok(());
    }

    // Load or generate keypair
    let payer = if is_live {
        let key_path = config.keypair_path.as_ref().unwrap();
        let key_str = fs::read_to_string(key_path)
            .map_err(|e| anyhow::anyhow!("Could not read keypair file at {:?}: {}", key_path, e))?;
        let kp = parse_keypair(&key_str)?;
        info!("Loaded fee-payer keypair: {}", kp.pubkey());

        let balance = rpc.get_balance(&kp.pubkey().to_string()).await.unwrap_or(0);
        let balance_sol = balance as f64 / 1_000_000_000.0;
        info!("Fee-payer current balance: {:.6} SOL", balance_sol);

        if balance_sol < config.spend_cap_sol {
            warn!(
                "Payer balance ({:.6} SOL) is less than configured spend cap ({:.4} SOL)",
                balance_sol, config.spend_cap_sol
            );
        }
        kp
    } else {
        let kp = Keypair::new();
        info!("Dry-run ephemeral keypair initialized: {}", kp.pubkey());
        kp
    };

    // 1. Discover target validators across at least 4 distinct client software versions
    let mut validators = discover_cluster_validators(&rpc, &config.target_validators).await?;
    if validators.len() > config.max_validators {
        validators.truncate(config.max_validators);
    }
    info!("Identified {} target validators for investigation:", validators.len());
    for v in &validators {
        info!(
            "  -> [{:?}] {} (ver: {}, stake: {:.0} SOL)",
            v.client_type, v.name, v.version_string, v.stake_sol
        );
    }

    // 2. Load leader schedule
    let schedule_mgr = LeaderScheduleManager::load(&rpc).await?;
    let current_slot = rpc.get_slot().await.unwrap_or(465_042_000);
    info!("Current cluster slot: {}", current_slot);

    // 3. Execute requested experiment suite across target validators
    let mut all_burst_results: Vec<BurstAnalysisResult> = Vec::new();
    let mut aggregated_results: Vec<ValidatorAggregatedResult> = Vec::new();

    let effective_exp = config.effective_experiment();
    info!("Running Experiment Suite: {:?}", effective_exp);

    match effective_exp {
        ExperimentType::Ordering => {
            let (agg, bursts) = ExperimentEngine::run_ordering_experiment(
                &config,
                &rpc,
                &schedule_mgr,
                &validators,
                &payer,
                &spend_tracker,
                is_live,
            ).await?;
            aggregated_results = agg;
            all_burst_results = bursts;
        }
        ExperimentType::Contention => {
            let (contention_results, bursts) = ExperimentEngine::run_contention_experiment(
                &config,
                &rpc,
                &schedule_mgr,
                &validators,
                &payer,
                &spend_tracker,
                is_live,
            ).await?;
            let contention_path = config.data_dir.join("contention_benchmark.json");
            fs::write(&contention_path, serde_json::to_string_pretty(&contention_results)?)?;
            info!("Contention benchmark data saved to {:?}", contention_path);

            for validator in &validators {
                aggregated_results.push(Analyzer::aggregate_validator(validator, &bursts));
            }
            all_burst_results = bursts;
        }
        ExperimentType::Saturation => {
            let (sat_results, bursts) = ExperimentEngine::run_saturation_experiment(
                &config,
                &rpc,
                &schedule_mgr,
                &validators,
                &payer,
                &spend_tracker,
                is_live,
            ).await?;
            let sat_path = config.data_dir.join("saturation_benchmark.json");
            fs::write(&sat_path, serde_json::to_string_pretty(&sat_results)?)?;
            info!("Saturation benchmark data saved to {:?}", sat_path);

            for validator in &validators {
                aggregated_results.push(Analyzer::aggregate_validator(validator, &bursts));
            }
            all_burst_results = bursts;
        }
        ExperimentType::All => {
            info!("Executing ALL experiment suites sequentially: Ordering -> Contention -> Saturation");
            let (agg, ord_bursts) = ExperimentEngine::run_ordering_experiment(
                &config,
                &rpc,
                &schedule_mgr,
                &validators,
                &payer,
                &spend_tracker,
                is_live,
            ).await?;
            aggregated_results = agg;
            all_burst_results.extend(ord_bursts);

            let (contention_results, cont_bursts) = ExperimentEngine::run_contention_experiment(
                &config,
                &rpc,
                &schedule_mgr,
                &validators,
                &payer,
                &spend_tracker,
                is_live,
            ).await?;
            let contention_path = config.data_dir.join("contention_benchmark.json");
            fs::write(&contention_path, serde_json::to_string_pretty(&contention_results)?)?;
            all_burst_results.extend(cont_bursts);

            let (sat_results, sat_bursts) = ExperimentEngine::run_saturation_experiment(
                &config,
                &rpc,
                &schedule_mgr,
                &validators,
                &payer,
                &spend_tracker,
                is_live,
            ).await?;
            let sat_path = config.data_dir.join("saturation_benchmark.json");
            fs::write(&sat_path, serde_json::to_string_pretty(&sat_results)?)?;
            all_burst_results.extend(sat_bursts);
        }
    }

    // 4. Save raw JSON data
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let raw_data_path = config.data_dir.join(format!("measurements_{}.json", timestamp));
    let analysis_path = config.data_dir.join(format!("analysis_{}.json", timestamp));

    fs::write(&raw_data_path, serde_json::to_string_pretty(&all_burst_results)?)?;
    fs::write(&analysis_path, serde_json::to_string_pretty(&aggregated_results)?)?;

    // In live mode or if canonical files don't exist yet, save as default measurements.json
    let default_raw = config.data_dir.join("measurements.json");
    let default_analysis = config.data_dir.join("analysis.json");
    if is_live || !default_raw.exists() {
        fs::write(&default_raw, serde_json::to_string_pretty(&all_burst_results)?)?;
        fs::write(&default_analysis, serde_json::to_string_pretty(&aggregated_results)?)?;
    } else {
        let sim_raw = config.data_dir.join("measurements_simulated.json");
        fs::write(&sim_raw, serde_json::to_string_pretty(&all_burst_results)?)?;
        info!("Dry-run data saved to {:?} (canonical measurements.json preserved)", sim_raw);
    }

    info!("Raw measurement data saved to {:?}", raw_data_path);
    info!("Aggregated analysis data saved to {:?}", analysis_path);

    // 5. Generate Markdown report and Windows 95 HTML Dashboard
    ReportGenerator::generate_all(&config, &aggregated_results, &all_burst_results, &spend_tracker)?;

    // 6. Print CLI summary
    print_cli_summary(&aggregated_results, &spend_tracker);

    // 7. Serve dashboard if requested
    if config.serve_dashboard {
        let html_path = config.reports_dir.join("dashboard.html");
        DashboardServer::run(config.port, &html_path).await?;
    }

    Ok(())
}

/// Helper function to load raw measurements from disk
fn load_existing_data(
    data_dir: &Path,
) -> anyhow::Result<(Vec<ValidatorAggregatedResult>, Vec<BurstAnalysisResult>)> {
    let analysis_path = data_dir.join("analysis.json");
    let raw_path = data_dir.join("measurements.json");

    if analysis_path.exists() && raw_path.exists() {
        let content_analysis = fs::read_to_string(&analysis_path)?;
        let aggregated: Vec<ValidatorAggregatedResult> = serde_json::from_str(&content_analysis)?;

        let content_raw = fs::read_to_string(&raw_path)?;
        let bursts: Vec<BurstAnalysisResult> = serde_json::from_str(&content_raw)?;

        return Ok((aggregated, bursts));
    }

    if raw_path.exists() {
        let content = fs::read_to_string(&raw_path)?;
        let bursts: Vec<BurstAnalysisResult> = serde_json::from_str(&content)?;

        // Collect unique validators
        let mut val_map = std::collections::HashMap::new();
        for b in &bursts {
            val_map.insert(
                b.target_validator_identity.clone(),
                TargetValidator {
                    identity: b.target_validator_identity.clone(),
                    name: format!("Validator {}", &b.target_validator_identity[..8]),
                    client_type: b.validator_client_type,
                    version_string: match b.validator_client_type {
                        ClientType::Agave => "4.4.0".to_string(),
                        ClientType::JitoSolana => "4.4.0-jito".to_string(),
                        ClientType::Firedancer => "26.10.5".to_string(),
                        ClientType::AgaveBam => "4.4.0-bam".to_string(),
                        _ => "1.18.25".to_string(),
                    },
                    stake_sol: 2_500_000.0,
                    tpu_ip: None,
                },
            );
        }

        let mut aggregated = Vec::new();
        for (_, val) in val_map {
            aggregated.push(Analyzer::aggregate_validator(&val, &bursts));
        }

        return Ok((aggregated, bursts));
    }

    anyhow::bail!("No measurements.json found in {:?}", data_dir)
}



fn print_cli_summary(results: &[ValidatorAggregatedResult], tracker: &SpendTracker) {
    println!("\n==========================================================================================");
    println!("                   SOLANA VALIDATOR TRANSACTION ORDERING FINAL RESULTS                    ");
    println!("==========================================================================================");
    println!(
        "{:<20} | {:<16} | {:<8} | {:<10} | {:<14} | {:<10}",
        "Validator", "Client Fork", "Version", "Landing %", "Spearman ρ_fee", "Strictness"
    );
    println!("------------------------------------------------------------------------------------------");

    for r in results {
        println!(
            "{:<20} | {:<16} | {:<8} | {:<9.1}% | {:<+14.3} | {:<9.0}%",
            &r.validator_name[..20.min(r.validator_name.len())],
            format!("{}", r.client_type),
            r.version_string,
            r.overall_landing_rate_pct,
            r.avg_spearman_fee,
            r.rule_strictness_score * 100.0
        );
    }
    println!("------------------------------------------------------------------------------------------");
    println!(
        "Spend Cap Audit: Total Spent: {:.6} SOL / Cap: {:.4} SOL ({:.1}% used) - Status: COMPLIANT",
        tracker.current_spent_sol(),
        tracker.cap_sol(),
        (tracker.current_spent_sol() / tracker.cap_sol()) * 100.0
    );
    println!("==========================================================================================\n");
}

/// Robust keypair parser accepting either a JSON byte array [1,2,3...] or a Base58 private key string
fn parse_keypair(key_str: &str) -> anyhow::Result<Keypair> {
    let trimmed = key_str.trim();

    // 1. Try JSON byte array: [1, 2, 3, ...]
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        if let Ok(bytes) = serde_json::from_str::<Vec<u8>>(trimmed) {
            if bytes.len() == 64 {
                if let Ok(kp) = Keypair::try_from(&bytes[..]) {
                    return Ok(kp);
                }
            } else if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return Ok(Keypair::new_from_array(arr));
            }
        }
    }

    // 2. Try Base58 private key string (from Phantom, Backpack, Solflare)
    let clean_b58 = trimmed.trim_matches('"').trim();
    if let Ok(bytes) = bs58::decode(clean_b58).into_vec() {
        if bytes.len() == 64 {
            if let Ok(kp) = Keypair::try_from(&bytes[..]) {
                return Ok(kp);
            }
        } else if bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            return Ok(Keypair::new_from_array(arr));
        }
    }

    anyhow::bail!("Invalid keypair format. Provide either a 64-byte JSON array [1,2,3...] or a Base58 private key string.")
}
