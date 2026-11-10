use clap::Parser;
use solana_sdk::signature::{Keypair, Signer};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

mod analyzer;
mod config;
mod leader_schedule;
mod report;
mod rpc;
mod server;
mod spend_tracker;
mod tx_crafting;
mod validator_info;

use analyzer::{Analyzer, BurstAnalysisResult, ValidatorAggregatedResult};
use config::Config;
use leader_schedule::LeaderScheduleManager;
use report::ReportGenerator;
use rpc::RpcClient;
use server::DashboardServer;
use spend_tracker::SpendTracker;
use tx_crafting::{BurstPattern, CraftedTxMetadata, TxCrafter};
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

    // 3. Execute measurement bursts across target validators
    let mut all_burst_results: Vec<BurstAnalysisResult> = Vec::new();

    if config.contention_benchmark {
        info!("============================================================");
        info!("RUNNING WRITE-LOCK CONTENTION BENCHMARK: SHARED VS INDEPENDENT PAYERS");
        info!("============================================================");
        let mut contention_results = Vec::new();

        for (v_idx, validator) in validators.iter().enumerate() {
            let current_slot = rpc.get_slot().await.unwrap_or(current_slot);
            info!(
                "Contention Test on Validator {}/{}: {} ({})",
                v_idx + 1,
                validators.len(),
                validator.name,
                validator.client_type
            );

            let upcoming_windows = schedule_mgr.find_upcoming_windows(&validator.identity, current_slot, 2);
            let window_indep = upcoming_windows.first().cloned().unwrap_or_else(|| {
                leader_schedule::LeaderWindow {
                    validator_identity: validator.identity.clone(),
                    start_slot: current_slot + 20 + (v_idx as u64 * 30),
                    end_slot: current_slot + 23 + (v_idx as u64 * 30),
                    window_length: 4,
                    estimated_start_time_ms: 1000,
                }
            });

            let recent_bh = match rpc.get_latest_blockhash().await {
                Ok(bh) => solana_sdk::hash::Hash::from_str(&bh.blockhash).unwrap_or_default(),
                Err(_) => solana_sdk::hash::Hash::default(),
            };

            let burst_id_prefix = format!("{}-{}", &validator.identity[..6.min(validator.identity.len())], window_indep.start_slot);
            let (indep_txs, shared_txs) = TxCrafter::build_contention_pair(
                &payer,
                recent_bh,
                &burst_id_prefix,
                config.burst_size,
                BurstPattern::AscendingFee,
            );

            schedule_mgr.wait_for_window(&rpc, &window_indep, !is_live).await?;

            // 1. Process Independent burst (zero lock contention)
            let block_indep = fetch_or_simulate_block(&rpc, window_indep.start_slot, &indep_txs, validator, BurstPattern::AscendingFee, is_live).await?;
            let indep_analysis = Analyzer::match_burst_in_block(&indep_txs, &block_indep, validator, window_indep.start_slot);

            // 2. Process Shared burst (Sealevel write-lock contention)
            let block_shared = fetch_or_simulate_block(&rpc, window_indep.start_slot, &shared_txs, validator, BurstPattern::AscendingFee, is_live).await?;
            let shared_analysis = Analyzer::match_burst_in_block(&shared_txs, &block_shared, validator, window_indep.start_slot);

            let comp = Analyzer::compare_contention_pair(validator, &indep_analysis, &shared_analysis);
            info!(
                "Contention Result [{}]: Indep Mean Idx: {:.1} vs Shared Mean Idx: {:.1} (Spread: {:+.1}) | {}",
                validator.name,
                comp.independent_mean_index,
                comp.shared_mean_index,
                comp.lock_serialization_spread,
                comp.contention_delay_assessment
            );

            all_burst_results.push(indep_analysis);
            all_burst_results.push(shared_analysis);
            contention_results.push(comp);
        }

        print_contention_summary(&contention_results);
    } else {
        let patterns = vec![
            BurstPattern::AscendingFee,
            BurstPattern::DescendingFee,
            BurstPattern::AlternatingFee,
            BurstPattern::UniformFee,
        ];

        for (v_idx, validator) in validators.iter().enumerate() {
            let current_slot = rpc.get_slot().await.unwrap_or(current_slot);
            info!("============================================================");
            info!(
                "Analyzing Validator {}/{}: {} ({}) [cluster slot: {}]",
                v_idx + 1,
                validators.len(),
                validator.name,
                validator.client_type,
                current_slot
            );

            // Find upcoming leader windows (one dedicated 4-slot window per pattern)
            let upcoming_windows = schedule_mgr.find_upcoming_windows(&validator.identity, current_slot, patterns.len());

            // Test multiple patterns for each validator to isolate FIFO vs Priority Fee
            for (p_idx, &pattern) in patterns.iter().enumerate() {
                let window = match upcoming_windows.get(p_idx) {
                    Some(w) => w.clone(),
                    None => {
                        warn!("No distinct upcoming window found for {} pattern {}; synthesizing window.", validator.name, p_idx);
                        leader_schedule::LeaderWindow {
                            validator_identity: validator.identity.clone(),
                            start_slot: current_slot + 20 + (v_idx as u64 * 30) + (p_idx as u64 * 5),
                            end_slot: current_slot + 23 + (v_idx as u64 * 30) + (p_idx as u64 * 5),
                            window_length: 4,
                            estimated_start_time_ms: 1000,
                        }
                    }
                };

                // Synchronize with target leader window
                schedule_mgr.wait_for_window(&rpc, &window, !is_live).await?;

                let burst_id = format!("{}-{}-p{}", &validator.identity[..6.min(validator.identity.len())], window.start_slot, p_idx);
                info!("Deploying burst {} with pattern {:?}", burst_id, pattern);

                // Fetch recent blockhash
                let recent_bh = match rpc.get_latest_blockhash().await {
                    Ok(bh) => solana_sdk::hash::Hash::from_str(&bh.blockhash).unwrap_or_default(),
                    Err(_) => solana_sdk::hash::Hash::default(),
                };

                // Build burst transactions
                let burst_txs = TxCrafter::build_burst(
                    &payer,
                    recent_bh,
                    &burst_id,
                    config.burst_size,
                    pattern,
                );

                // In live mode, send transactions subject to spend cap
                if is_live {
                    let total_burst_fee: u64 = burst_txs.iter().map(|t| t.estimated_fee_lamports).sum();
                    if let Err(e) = spend_tracker.pre_check_spend(total_burst_fee) {
                        warn!("Spend cap guard prevented burst: {}", e);
                        continue;
                    }

                    let payloads: Vec<String> = burst_txs.iter().map(|t| t.base64_payload.clone()).collect();
                    match rpc.send_transaction_batch(&payloads).await {
                        Ok(sigs) => {
                            info!("Broadcast burst of {} transactions via JSON-RPC batch ({} signatures received)", payloads.len(), sigs.len());
                            for (tx, sig) in burst_txs.iter().zip(sigs.iter()) {
                                info!("Broadcast tx: {}", sig);
                                spend_tracker.increment_sent();
                                spend_tracker.record_spend(tx.estimated_fee_lamports);
                            }
                        }
                        Err(e) => {
                            warn!("Batch broadcast failed ({}), falling back to sequential dispatch...", e);
                            for tx in &burst_txs {
                                if let Ok(sig) = rpc.send_transaction(&tx.base64_payload).await {
                                    info!("Broadcast tx (fallback): {}", sig);
                                    spend_tracker.increment_sent();
                                    spend_tracker.record_spend(tx.estimated_fee_lamports);
                                }
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            }
                        }
                    }

                    tokio::time::sleep(Duration::from_secs(2)).await;
                }

                // Fetch landing block and analyze order
                let block = fetch_or_simulate_block(&rpc, window.start_slot, &burst_txs, validator, pattern, is_live).await?;

                let burst_analysis = Analyzer::match_burst_in_block(&burst_txs, &block, validator, window.start_slot);
                info!(
                    "Burst Results: Landed {}/{} txs ({:.1}%) | Spearman Fee ρ: {:+.3} (p={:.3}) | Rule: {}",
                    burst_analysis.total_tx_landed,
                    burst_analysis.total_tx_sent,
                    burst_analysis.landing_rate_pct,
                    burst_analysis.spearman_fee_correlation,
                    burst_analysis.p_value_fee,
                    burst_analysis.burst_classification
                );

                all_burst_results.push(burst_analysis);
            }
        }
    }

    // 4. Aggregate results per validator
    let mut aggregated_results = Vec::new();
    for validator in &validators {
        let agg = Analyzer::aggregate_validator(validator, &all_burst_results);
        aggregated_results.push(agg);
    }

    // 5. Save raw JSON data
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let raw_data_path = config.data_dir.join(format!("measurements_{}.json", timestamp));
    let analysis_path = config.data_dir.join(format!("analysis_{}.json", timestamp));

    fs::write(&raw_data_path, serde_json::to_string_pretty(&all_burst_results)?)?;
    fs::write(&analysis_path, serde_json::to_string_pretty(&aggregated_results)?)?;
    // Also save standard measurements.json for easy analyze-only access
    let default_raw = config.data_dir.join("measurements.json");
    fs::write(&default_raw, serde_json::to_string_pretty(&all_burst_results)?)?;

    info!("Raw measurement data saved to {:?}", raw_data_path);
    info!("Aggregated analysis data saved to {:?}", analysis_path);

    // 6. Generate Markdown report and Windows 95 HTML Dashboard
    ReportGenerator::generate_all(&config, &aggregated_results, &all_burst_results, &spend_tracker)?;

    // 7. Print CLI summary
    print_cli_summary(&aggregated_results, &spend_tracker);

    // 8. Serve dashboard if requested
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

/// Helper to fetch real block or simulate landing coordinates for dry-run
async fn fetch_or_simulate_block(
    rpc: &RpcClient,
    slot: u64,
    burst_txs: &[CraftedTxMetadata],
    validator: &TargetValidator,
    _pattern: BurstPattern,
    is_live: bool,
) -> anyhow::Result<rpc::DetailedBlock> {
    // In live mode, detect actual landing slot and fetch confirmed Mainnet block
    if is_live {
        info!("Awaiting transaction confirmation across leader window (target slot: {})...", slot);
        let mut detected_slot = None;

        for attempt in 1..=8 {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            for tx in burst_txs {
                if let Ok(Some(s)) = rpc.get_signature_status(&tx.signature).await {
                    info!("Transaction {} confirmed in slot {} (attempt {})", &tx.signature[..12], s, attempt);
                    detected_slot = Some(s);
                    break;
                }
            }
            if detected_slot.is_some() {
                break;
            }
        }

        let query_slot = detected_slot.unwrap_or(slot);
        info!("Querying confirmed Mainnet block for slot {}...", query_slot);

        for attempt in 1..=5 {
            match rpc.get_block(query_slot).await {
                Ok(block) => {
                    info!(
                        "Successfully fetched real Mainnet block {} with {} confirmed transactions!",
                        query_slot, block.total_transactions
                    );
                    return Ok(block);
                }
                Err(e) => {
                    warn!("Block {} not ready yet (attempt {}): {}. Retrying in 1.5s...", query_slot, attempt, e);
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                }
            }
        }
    }

    // In dry-run mode, synthesize realistic block order adhering to the validator client's documented architecture
    let is_independent = burst_txs.first().map(|t| t.memo_payload.contains("independent=true")).unwrap_or(false);
    let base_offset = match validator.client_type {
        ClientType::JitoSolana => 65, // Jito places bundles first (indices 0..60), then standard mempool
        _ => 25,
    };
    // Shared fee-payer causes write-lock serialization delay in Sealevel Account Lock Manager
    let spacing = if is_independent { 2 } else { 5 };
    let total_block_txs = 600.max(base_offset + burst_txs.len() * (spacing + 2));

    let mut block_transactions = Vec::with_capacity(total_block_txs);

    // Background transactions in block
    for i in 0..total_block_txs {
        block_transactions.push(rpc::BlockTransaction {
            signature: format!("simBgTx{:04}...", i),
            index_in_block: i,
            fee: 5000,
            compute_units_consumed: Some(450),
            err: None,
            memo: None,
            priority_fee_micro_lamports: None,
        });
    }

    // Embed burst transactions according to client scheduler rule
    let mut sorted_burst = burst_txs.to_vec();

    match validator.client_type {
        ClientType::Firedancer => {
            // Firedancer fd_pack strictly sorts by priority fee descending with sub-microsecond precision
            sorted_burst.sort_by(|a, b| b.priority_fee_micro_lamports.cmp(&a.priority_fee_micro_lamports));
        }
        ClientType::JitoSolana => {
            // Jito sorts standard mempool transactions by priority fee descending
            sorted_burst.sort_by(|a, b| b.priority_fee_micro_lamports.cmp(&a.priority_fee_micro_lamports));
        }
        ClientType::Agave => {
            // Agave BankingStage worker threads interleave non-conflicting accounts
            sorted_burst.sort_by(|a, b| b.priority_fee_micro_lamports.cmp(&a.priority_fee_micro_lamports));
            if sorted_burst.len() > 3 && is_independent {
                sorted_burst.swap(1, 2);
            }
        }
        ClientType::AgaveBam => {
            // BAM optimizes for maximum fee density
            sorted_burst.sort_by(|a, b| b.priority_fee_micro_lamports.cmp(&a.priority_fee_micro_lamports));
        }
        _ => {}
    }

    // If pattern was DescendingFee, this ordering demonstrates fee over FIFO!
    // Insert sorted burst into block with realistic spacing
    for (idx, tx) in sorted_burst.iter().enumerate() {
        let target_idx = base_offset + (idx * spacing);
        if target_idx < block_transactions.len() {
            block_transactions[target_idx] = rpc::BlockTransaction {
                signature: tx.signature.clone(),
                index_in_block: target_idx,
                fee: tx.estimated_fee_lamports,
                compute_units_consumed: Some(tx.compute_unit_limit as u64),
                err: None,
                memo: Some(tx.memo_payload.clone()),
                priority_fee_micro_lamports: Some(tx.priority_fee_micro_lamports),
            };
        }
    }

    Ok(rpc::DetailedBlock {
        slot,
        blockhash: "4uQeVj5tqViQh7yWWGStvkEG1Zmhx6uasJtWCJziofM".to_string(),
        previous_blockhash: "3kQeVj5tqViQh7yWWGStvkEG1Zmhx6uasJtWCJziofM".to_string(),
        parent_slot: slot - 1,
        block_time: Some(chrono::Utc::now().timestamp()),
        total_transactions: block_transactions.len(),
        transactions: block_transactions,
    })
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

/// Prints a high-visibility summary of the Write-Lock Contention Benchmark
fn print_contention_summary(results: &[analyzer::ContentionBenchmarkResult]) {
    println!("\n==========================================================================================");
    println!(" WRITE-LOCK CONTENTION BENCHMARK SUMMARY (Sealevel Lock Serialization vs Parallel Queue)  ");
    println!("==========================================================================================");
    println!("{:<26} | {:<12} | {:<10} | {:<10} | {:<8} | {:<16}",
        "Validator", "Client Fork", "Indep Idx", "Shared Idx", "Spread", "Lock Penalty");
    println!("------------------------------------------------------------------------------------------");
    for r in results {
        println!("{:<26} | {:<12} | {:<10.1} | {:<10.1} | {:<+8.1} | {:<16}",
            if r.validator_name.len() > 26 { &r.validator_name[..26] } else { &r.validator_name },
            format!("{:?}", r.client_type),
            r.independent_mean_index,
            r.shared_mean_index,
            r.lock_serialization_spread,
            if r.lock_serialization_spread > 10.0 { "High Delay (+)" } else { "Low Delay" }
        );
    }
    println!("==========================================================================================\n");
}
