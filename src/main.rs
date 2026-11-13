use clap::Parser;
use solana_sdk::signature::{Keypair, Signer};
use std::fs;
use std::sync::Arc;
use tracing::{info, warn};

mod analyzer;
mod campaign;
mod config;
mod experiments;
mod leader_schedule;
mod payer;
mod report;
mod rpc;
mod server;
mod spend_tracker;
mod transport;
mod tx_crafting;
mod validator_info;
mod verify;

use analyzer::{Analyzer, CampaignAnalysis, ExecutionMode, PayerMode};
use campaign::{Campaign, CampaignMeta};
use config::Config;
use experiments::{campaign_dir_for, ExperimentEngine};
use leader_schedule::LeaderScheduleManager;
use payer::{load_keypair, load_payer_pool, PayerPool};
use report::ReportGenerator;
use rpc::RpcClient;
use server::DashboardServer;
use spend_tracker::SpendTracker;
use validator_info::{
    discover_cluster_validators, distinct_client_classes, synthetic_fixture_validators,
    TargetValidator,
};
use verify::{verify_campaign, write_verification};

const BANNER: &str = r#"
╔══════════════════════════════════════════════════════════════════════════╗
║  SOLANA VALIDATOR TRANSACTION ORDERING ENGINE v2                        ║
║  Same-block inference · pre-window dispatch · direct TPU QUIC           ║
╚══════════════════════════════════════════════════════════════════════════╝
"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    println!("{BANNER}");
    let config = Config::parse();
    config.validate()?;
    fs::create_dir_all(&config.data_dir)?;
    fs::create_dir_all(&config.reports_dir)?;

    let rpc = Arc::new(RpcClient::new(&config.rpc_url));

    if config.verify_only {
        let campaign = load_campaign(&config)?;
        let report = verify_campaign(&rpc, &campaign).await?;
        let path = campaign
            .verifier_output_path()
            .unwrap_or_else(|| config.data_dir.join("verification.json"));
        write_verification(&report, &path)?;
        info!(
            "Verification: {}/{} confirmed, {}/{} index matches, {}/{} fee matches, {} failures -> {}",
            report.confirmed,
            report.total_signatures,
            report.index_matched,
            report.total_signatures,
            report.fee_matched,
            report.total_signatures,
            report.failures.len(),
            path.display()
        );
        return Ok(());
    }

    if config.analyze_only {
        let campaign = load_campaign(&config)?;
        let mut analysis = Analyzer::analyze_campaign(&campaign.bursts, &config.analysis_options());
        analysis.campaign_id = campaign.meta.campaign_id.clone();
        enrich_analysis(&mut analysis, &campaign);
        let spend = SpendTracker::new(config.spend_cap_sol);
        let output = ReportGenerator::generate_all(&config, &campaign, &analysis, &spend)?;
        info!(
            "Analysis and reports generated: {}",
            output.markdown.display()
        );
        print_summary(&campaign, &analysis);
        if config.serve_dashboard {
            DashboardServer::run(config.port, &output.dashboard).await?;
        }
        return Ok(());
    }

    if config.is_live() {
        warn!("LIVE MODE ACTIVE: real mainnet transactions will be broadcast.");
    } else {
        info!("DRY-RUN MODE ACTIVE: transactions are simulated; nothing is broadcast.");
    }

    let payers = load_payers(&config, &rpc).await?;
    let validators = load_validators(&config, &rpc).await?;
    info!("Selected {} validators:", validators.len());
    for validator in &validators {
        info!(
            "  {} [{} / {}] stake {:.0} SOL tpuQuic={}",
            validator.identity,
            validator.client_type,
            validator.version_string,
            validator.stake_sol,
            validator.tpu_ip.as_deref().unwrap_or("none")
        );
    }

    let schedule = if config.is_live() {
        LeaderScheduleManager::load(&rpc, false).await?
    } else {
        let identities: Vec<String> = validators.iter().map(|v| v.identity.clone()).collect();
        LeaderScheduleManager::synthetic_for(&identities, 465_000_000)
    };

    let campaign_id = campaign::campaign_id_now(if config.is_live() { "live" } else { "dryrun" });
    let campaign_dir = campaign_dir_for(&config, &campaign_id);
    fs::create_dir_all(&campaign_dir)?;
    // Live spend is journaled globally so a restart cannot reset the cap.
    // Dry-run spend is isolated inside the clearly-labelled simulated campaign.
    let spend_journal = if config.is_live() {
        config.data_dir.join("live").join("spend_journal.jsonl")
    } else {
        campaign_dir.join("spend_journal.jsonl")
    };
    let spend = SpendTracker::with_journal(config.spend_cap_sol, &spend_journal)?;

    let mut engine = ExperimentEngine::new(
        config.clone(),
        (*rpc).clone(),
        schedule,
        spend.clone(),
        payers,
        campaign_dir.clone(),
    );
    let output = engine.run(&validators).await?;
    if output.bursts.is_empty() {
        anyhow::bail!("campaign produced no bursts; refusing to write an empty report");
    }

    let mut meta = CampaignMeta::new(
        campaign_id.clone(),
        if config.is_live() {
            ExecutionMode::Live
        } else {
            ExecutionMode::DryRun
        },
        config.rpc_url.clone(),
    );
    meta.transport = config.transport_kind();
    meta.payer_mode = config.payer_mode_kind();
    meta.send_lead_slots = config.send_lead_slots;
    meta.target_window_slots = config.target_window_slots;
    meta.burst_size = config.burst_size;
    meta.repetitions = config.repetitions;
    meta.patterns = config.burst_patterns();
    meta.seed = config.seed;
    meta.min_blocks = config.min_blocks;
    meta.permutation_trials = config.permutation_trials;
    meta.spend_cap_lamports = config.spend_cap_lamports();
    meta.total_bursts = output.bursts.len();
    meta.total_tx_sent = output.total_tx_sent;
    meta.total_tx_landed = output.total_tx_landed;
    meta.total_spent_lamports = spend.campaign_spent_lamports();
    meta.validators = validators.clone();
    meta.notes.push(format!(
        "Pre-window send lead: {} slots; target window: {} slot(s); primary inference uses the target block only.",
        config.send_lead_slots, config.target_window_slots
    ));
    if output.failed_tasks > 0 {
        meta.notes.push(format!(
            "{} task(s) failed before dispatch or were skipped as late.",
            output.failed_tasks
        ));
    }
    if !config.is_live() {
        meta.notes.push(
            "Dry-run: all block placements are synthetic. This report validates the pipeline only and \
             must not be cited as validator behaviour."
                .to_string(),
        );
    }

    let mut campaign = Campaign::new(meta, output.bursts);
    let saved_dir = campaign.save(&campaign_dir)?;
    campaign.dir = Some(saved_dir);
    info!("Campaign saved to {}", campaign_dir.display());

    if config.is_live() && config.verify {
        let report = verify_campaign(&rpc, &campaign).await?;
        let path = campaign
            .verifier_output_path()
            .unwrap_or_else(|| campaign_dir.join("verification.json"));
        write_verification(&report, &path)?;
        info!(
            "Verification: {} confirmed, {} failures -> {}",
            report.confirmed,
            report.failures.len(),
            path.display()
        );
    }

    let mut analysis = Analyzer::analyze_campaign(&campaign.bursts, &config.analysis_options());
    analysis.campaign_id = campaign.meta.campaign_id.clone();
    enrich_analysis(&mut analysis, &campaign);
    let report_output = ReportGenerator::generate_all(&config, &campaign, &analysis, &spend)?;
    info!("Report: {}", report_output.markdown.display());
    info!("Analysis JSON: {}", report_output.analysis_json.display());
    print_summary(&campaign, &analysis);

    if config.serve_dashboard {
        DashboardServer::run(config.port, &report_output.dashboard).await?;
    }
    Ok(())
}

async fn load_payers(config: &Config, rpc: &RpcClient) -> anyhow::Result<PayerPool> {
    if config.is_live() {
        match config.payer_mode_kind() {
            PayerMode::Shared => {
                let path = config.keypair_path.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("--keypair-path is required for shared live mode")
                })?;
                let payer = load_keypair(path)?;
                let balance = rpc.get_balance(&payer.pubkey().to_string()).await?;
                info!(
                    "Shared payer {} balance {:.6} SOL",
                    payer.pubkey(),
                    balance as f64 / 1_000_000_000.0
                );
                if balance < config.spend_cap_lamports() / 2 {
                    warn!(
                        "Payer balance {:.6} SOL is below half the configured cap {:.6} SOL",
                        balance as f64 / 1_000_000_000.0,
                        config.spend_cap_sol
                    );
                }
                Ok(PayerPool::new(Some(payer), Vec::new()))
            }
            PayerMode::Independent => {
                let path = config.payer_pool_file.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("--payer-pool-file is required for independent live mode")
                })?;
                let payers = load_payer_pool(path)?;
                if payers.len() < config.burst_size {
                    anyhow::bail!(
                        "payer pool has {} keys but --burst-size is {}; provide at least {} independent funded payers",
                        payers.len(),
                        config.burst_size,
                        config.burst_size
                    );
                }
                for payer in payers.iter().take(config.burst_size) {
                    let balance = rpc.get_balance(&payer.pubkey().to_string()).await?;
                    if balance < 1_000_000 {
                        anyhow::bail!(
                            "independent payer {} has {} lamports; fund it before a live campaign",
                            payer.pubkey(),
                            balance
                        );
                    }
                }
                info!(
                    "Loaded {} independent payers; each has at least 0.001 SOL",
                    payers.len()
                );
                Ok(PayerPool::new(None, payers))
            }
        }
    } else {
        let master = Keypair::new();
        info!("Dry-run ephemeral payer: {}", master.pubkey());
        if config.payer_mode_kind() == PayerMode::Independent {
            let payers: Vec<Keypair> = (0..config.burst_size).map(|_| Keypair::new()).collect();
            Ok(PayerPool::new(Some(master), payers))
        } else {
            Ok(PayerPool::new(Some(master), Vec::new()))
        }
    }
}

async fn load_validators(config: &Config, rpc: &RpcClient) -> anyhow::Result<Vec<TargetValidator>> {
    if !config.is_live() {
        let mut validators = synthetic_fixture_validators();
        validators.truncate(config.max_validators);
        return Ok(validators);
    }

    let require_tpu = config.transport_kind() == analyzer::TransportKind::TpuQuic;
    let mut validators = discover_cluster_validators(
        rpc,
        &config.target_validators,
        config.max_validators,
        require_tpu,
    )
    .await?;
    validators.truncate(config.max_validators);
    let classes = distinct_client_classes(&validators);
    if validators.len() < config.min_validators && !config.allow_few_validators {
        anyhow::bail!(
            "live campaign has {} validators but --min-validators is {}; pass --allow-few-validators only for a pilot",
            validators.len(),
            config.min_validators
        );
    }
    if classes < config.min_client_versions && !config.allow_few_validators {
        anyhow::bail!(
            "live campaign spans {} client/version classes but --min-client-versions is {}",
            classes,
            config.min_client_versions
        );
    }
    Ok(validators)
}

fn load_campaign(config: &Config) -> anyhow::Result<Campaign> {
    if let Some(dir) = &config.campaign_dir {
        if dir.is_file() {
            let mut campaign = Campaign::load_legacy(dir)?;
            campaign.dir = dir.parent().map(|p| p.to_path_buf());
            return Ok(campaign);
        }
        if dir.join("measurements.json").exists() && !dir.join("campaign.json").exists() {
            let mut campaign = Campaign::load_legacy(&dir.join("measurements.json"))?;
            campaign.dir = Some(dir.clone());
            return Ok(campaign);
        }
        return Campaign::load(dir);
    }
    match Campaign::load_latest(&config.data_dir) {
        Ok(campaign) => Ok(campaign),
        Err(latest_error) => {
            let legacy = config.data_dir.join("measurements.json");
            if legacy.exists() {
                warn!("No v2 campaign found; loading legacy measurements.json");
                let mut campaign = Campaign::load_legacy(&legacy)?;
                campaign.dir = Some(config.data_dir.clone());
                Ok(campaign)
            } else {
                Err(latest_error)
            }
        }
    }
}

fn enrich_analysis(analysis: &mut CampaignAnalysis, campaign: &Campaign) {
    for validator in &mut analysis.per_validator {
        if let Some(meta) = campaign
            .meta
            .validators
            .iter()
            .find(|v| v.identity == validator.validator_identity)
        {
            validator.validator_name = meta.name.clone();
            validator.client_type = meta.client_type;
            validator.version_string = meta.version_string.clone();
            validator.stake_sol = meta.stake_sol;
        }
    }
}

fn print_summary(campaign: &Campaign, analysis: &CampaignAnalysis) {
    println!(
        "\n=== Campaign {} ({}) ===",
        campaign.meta.campaign_id, campaign.meta.execution_mode
    );
    println!(
        "Probes: {} landed / {} sent; target-block probes: {}; target-window probes: {}",
        analysis.sample.total_probes_landed,
        analysis.sample.total_probes_sent,
        analysis.sample.target_block_probes,
        analysis.sample.target_window_probes
    );
    println!(
        "Target-block fee: {} | p={:?} | blocks={} | verdict: {}",
        analysis.fee_target_block.effect_text(),
        analysis.fee_target_block.permutation_p,
        analysis.fee_target_block.blocks_used,
        analysis.fee_target_block.verdict
    );
    println!(
        "Target-block send order: {} | p={:?} | blocks={} | verdict: {}",
        analysis.send_target_block.effect_text(),
        analysis.send_target_block.permutation_p,
        analysis.send_target_block.blocks_used,
        analysis.send_target_block.verdict
    );
    for warning in &analysis.warnings {
        println!("WARNING: {warning}");
    }
    println!(
        "Spend: {:.9} SOL of {:.9} SOL cap",
        campaign.meta.total_spent_lamports as f64 / 1_000_000_000.0,
        campaign.meta.spend_cap_lamports as f64 / 1_000_000_000.0
    );
}
