use crate::analyzer::{
    Analyzer, BurstAnalysisResult, BurstProvenance, ExecutionMode, PayerMode, TransportKind,
};
use crate::campaign::save_block_in;
use crate::config::Config;
use crate::leader_schedule::{LeaderScheduleManager, LeaderWindow};
use crate::payer::PayerPool;
use crate::rpc::{DetailedBlock, RpcClient};
use crate::spend_tracker::SpendTracker;
use crate::transport::{parse_tpu_addr, TpuSender};
use crate::tx_crafting::{BurstPattern, CraftedTxMetadata, TxCrafter};
use crate::validator_info::TargetValidator;
use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use solana_sdk::hash::Hash;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};
use tracing::{info, warn};

#[derive(Debug, Clone, Default)]
pub struct ExperimentOutput {
    pub bursts: Vec<BurstAnalysisResult>,
    pub failed_tasks: usize,
    pub total_tx_sent: usize,
    pub total_tx_landed: usize,
}

#[derive(Debug, Clone)]
struct Task {
    validator: TargetValidator,
    window: LeaderWindow,
    pattern: BurstPattern,
    repetition: usize,
}

pub struct ExperimentEngine {
    config: Config,
    rpc: RpcClient,
    schedule: LeaderScheduleManager,
    spend: SpendTracker,
    payers: PayerPool,
    campaign_dir: PathBuf,
    dry_run: bool,
}

impl ExperimentEngine {
    pub fn new(
        config: Config,
        rpc: RpcClient,
        schedule: LeaderScheduleManager,
        spend: SpendTracker,
        payers: PayerPool,
        campaign_dir: PathBuf,
    ) -> Self {
        let dry_run = !config.live;
        Self {
            config,
            rpc,
            schedule,
            spend,
            payers,
            campaign_dir,
            dry_run,
        }
    }

    pub async fn run(&mut self, validators: &[TargetValidator]) -> Result<ExperimentOutput> {
        let tasks = self.build_tasks(validators).await?;
        if tasks.is_empty() {
            anyhow::bail!("no leader windows available for the selected validators");
        }
        info!(
            "Starting {} ({} tasks, {} validators, transport {}, payer mode {})",
            if self.dry_run {
                "dry-run"
            } else {
                "LIVE campaign"
            },
            tasks.len(),
            validators.len(),
            self.config.transport_kind(),
            self.config.payer_mode_kind()
        );

        let mut output = ExperimentOutput::default();
        for (task_idx, task) in tasks.iter().enumerate() {
            match self.run_task(task, task_idx).await {
                Ok((burst, blocks)) => {
                    for block in &blocks {
                        if let Err(e) = save_block_in(&self.campaign_dir, block) {
                            warn!("could not save raw block {}: {e}", block.slot);
                        }
                    }
                    output.total_tx_sent += burst.total_tx_sent;
                    output.total_tx_landed += burst.total_tx_landed;
                    output.bursts.push(burst);
                }
                Err(e) => {
                    output.failed_tasks += 1;
                    warn!(
                        "task {} for {} at slot {} failed: {e:#}",
                        task_idx, task.validator.identity, task.window.start_slot
                    );
                }
            }
        }
        Ok(output)
    }

    async fn build_tasks(&self, validators: &[TargetValidator]) -> Result<Vec<Task>> {
        let patterns = self.config.burst_patterns();
        if patterns.is_empty() {
            anyhow::bail!("no burst patterns selected");
        }
        let needed = patterns.len() * self.config.repetitions;
        let current_slot = if self.dry_run {
            465_000_000
        } else {
            self.rpc.get_slot().await?
        };

        let mut tasks = Vec::new();
        for (validator_idx, validator) in validators.iter().enumerate() {
            let windows = if self.dry_run {
                (0..needed)
                    .map(|i| LeaderWindow {
                        validator_identity: validator.identity.clone(),
                        start_slot: current_slot
                            + 40
                            + (validator_idx as u64 * 30)
                            + (i as u64 * 12),
                        end_slot: current_slot + 43 + (validator_idx as u64 * 30) + (i as u64 * 12),
                        window_length: 4,
                        estimated_start_time_ms: (i as u64 + 1) * 5_000,
                    })
                    .collect::<Vec<_>>()
            } else {
                self.schedule
                    .find_upcoming_windows(&validator.identity, current_slot, needed)
            };

            if windows.len() < needed {
                warn!(
                    "validator {} has only {} upcoming window(s) in the loaded epoch; needed {}",
                    validator.identity,
                    windows.len(),
                    needed
                );
            }

            for (window_idx, window) in windows.into_iter().enumerate() {
                let pattern = patterns[window_idx % patterns.len()];
                let repetition = window_idx / patterns.len();
                tasks.push(Task {
                    validator: validator.clone(),
                    window,
                    pattern,
                    repetition,
                });
            }
        }

        tasks.sort_by_key(|t| t.window.start_slot);
        Ok(tasks)
    }

    async fn run_task(
        &mut self,
        task: &Task,
        task_idx: usize,
    ) -> Result<(BurstAnalysisResult, Vec<DetailedBlock>)> {
        let burst_id = format!(
            "{}-{}-{}-r{}",
            short_identity(&task.validator.identity),
            task.window.start_slot,
            task.pattern,
            task.repetition
        );
        let send_slot_target = task.window.send_slot(self.config.send_lead_slots);
        let transport = if self.dry_run {
            TransportKind::Synthetic
        } else {
            self.config.transport_kind()
        };

        let observed_send_slot = if self.dry_run {
            send_slot_target
        } else {
            self.schedule
                .wait_until_slot(&self.rpc, send_slot_target, false)
                .await?
        };

        let late_send = observed_send_slot >= task.window.start_slot;
        if late_send && !self.dry_run {
            warn!(
                "SKIP {}: observed send slot {} is not before target window start {}",
                burst_id, observed_send_slot, task.window.start_slot
            );
            // A skipped burst is still accounted for by the caller as a failed
            // timing attempt; no transaction is fabricated or broadcast.
            anyhow::bail!(
                "late send: observed slot {} >= target slot {}",
                observed_send_slot,
                task.window.start_slot
            );
        }

        let recent_blockhash = if self.dry_run {
            Hash::default()
        } else {
            let bh = self.rpc.get_latest_blockhash().await?;
            Hash::from_str(&bh.blockhash)
                .with_context(|| format!("invalid blockhash {}", bh.blockhash))?
        };

        let independent = self.config.payer_mode_kind() == PayerMode::Independent;
        let payer_refs = self.payers.for_burst(self.config.burst_size, independent)?;
        let seed = self
            .config
            .seed
            .wrapping_add((task_idx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut metadata = TxCrafter::build_burst_with_payers(
            &payer_refs,
            recent_blockhash,
            &burst_id,
            self.config.burst_size,
            task.pattern,
            seed,
            task.pattern == BurstPattern::RandomizedFee,
        );

        let estimated_total: u64 = metadata.iter().map(|m| m.estimated_fee_lamports).sum();
        let reservation = self.spend.try_reserve(estimated_total)?;

        let send_started_ns = now_ns();
        let send_started = Instant::now();
        let send_results: Vec<Result<(), String>> = if self.dry_run {
            metadata.iter().map(|_| Ok(())).collect()
        } else {
            self.dispatch(&task.validator, &metadata).await?
        };
        let send_finished_ns = now_ns();
        let send_elapsed_ms = send_started.elapsed().as_millis() as u64;

        let mut accepted = 0usize;
        for (meta, result) in metadata.iter_mut().zip(send_results.iter()) {
            if let Err(e) = result {
                meta.send_error = Some(e.clone());
            } else {
                accepted += 1;
                self.spend.record_sent();
            }
        }

        if accepted == 0 {
            drop(reservation);
            anyhow::bail!("all {} probes failed to dispatch", metadata.len());
        }
        // The fee is deterministic from the requested CU limit and price. Commit
        // the intended fee for accepted probes; rejected probes never paid.
        let committed: u64 = metadata
            .iter()
            .filter(|m| m.send_error.is_none())
            .map(|m| m.estimated_fee_lamports)
            .sum();
        reservation.commit(committed)?;

        let signatures: Vec<String> = metadata.iter().map(|m| m.signature.clone()).collect();
        let (blocks, target_block_fetched, target_block_hash) = if self.dry_run {
            let block = simulate_block(
                &metadata,
                task.window.start_slot,
                self.config.seed ^ task_idx as u64,
            );
            let hash = block.blockhash.clone();
            (vec![block], true, hash)
        } else {
            self.collect_blocks(&signatures, task.window.start_slot)
                .await?
        };

        let provenance = BurstProvenance {
            execution_mode: if self.dry_run {
                ExecutionMode::DryRun
            } else {
                ExecutionMode::Live
            },
            design_version: 2,
            transport,
            payer_mode: self.config.payer_mode_kind(),
            send_slot: observed_send_slot,
            send_started_ns,
            send_finished_ns,
            late_send,
            target_window_start: task.window.start_slot,
            target_window_end: task.window.start_slot + self.config.target_window_slots - 1,
            target_block_fetched,
            target_block_hash,
            memo_prefix: crate::tx_crafting::MEMO_PREFIX.to_string(),
            send_order_randomized: task.pattern == BurstPattern::RandomizedFee,
            fee_assignment_randomized: task.pattern == BurstPattern::RandomizedFee,
        };

        let result = Analyzer::match_burst_in_blocks(
            &metadata,
            &blocks,
            &task.validator,
            task.window.start_slot,
            provenance.target_window_end,
            provenance,
        );
        for _ in 0..result.txs.len() {
            self.spend.record_confirmed();
        }

        info!(
            "{}: target {} observed_send {} dispatch {:.1} ms accepted {}/{} landed {} target_block {}",
            burst_id,
            task.window.start_slot,
            observed_send_slot,
            send_elapsed_ms,
            accepted,
            metadata.len(),
            result.txs.len(),
            result.target_block_probes()
        );

        Ok((result, blocks))
    }

    async fn dispatch(
        &self,
        validator: &TargetValidator,
        metadata: &[CraftedTxMetadata],
    ) -> Result<Vec<Result<(), String>>> {
        match self.config.transport {
            crate::config::TransportArg::TpuQuic => {
                let Some(tpu_addr) = validator.tpu_ip.as_deref() else {
                    anyhow::bail!(
                        "validator {} has no tpuQuic address; use --transport rpc or a validator with tpuQuic",
                        validator.identity
                    );
                };
                let addr = parse_tpu_addr(tpu_addr)?;
                let sender = TpuSender::new(addr)?;
                sender.warmup().await?;
                let wires: Vec<Vec<u8>> = metadata
                    .iter()
                    .map(|m| m.wire_bytes())
                    .collect::<Result<Vec<_>>>()?;
                let results = sender.send_batch(&wires).await;
                sender.close().await;
                Ok(results
                    .into_iter()
                    .map(|r| r.map_err(|e| format!("{e:#}")))
                    .collect())
            }
            crate::config::TransportArg::Rpc => {
                let payloads: Vec<String> =
                    metadata.iter().map(|m| m.base64_payload.clone()).collect();
                let sigs = self
                    .rpc
                    .send_transaction_batch_with_retries(&payloads, self.config.rpc_max_retries)
                    .await?;
                let mut results = vec![Ok(()); metadata.len()];
                if sigs.len() != metadata.len() {
                    // The batch response may omit failed items. Mark missing
                    // probes as send failures rather than pretending success.
                    for result in results.iter_mut().skip(sigs.len()) {
                        *result = Err("RPC batch response omitted this transaction".to_string());
                    }
                }
                Ok(results)
            }
        }
    }

    async fn collect_blocks(
        &self,
        signatures: &[String],
        target_slot: u64,
    ) -> Result<(Vec<DetailedBlock>, bool, String)> {
        // Give the cluster time to include and serve the block.
        wait_for_signatures(&self.rpc, signatures, Duration::from_secs(30)).await?;

        let mut slots = BTreeMap::<u64, ()>::new();
        slots.insert(target_slot, ());
        for lag in 1..=self.config.max_landing_lag {
            slots.insert(target_slot + lag, ());
        }
        let statuses = self.rpc.get_signature_statuses(signatures).await?;
        for status in statuses.into_iter().flatten() {
            if status.slot >= target_slot
                && status.slot <= target_slot + self.config.max_landing_lag
            {
                slots.insert(status.slot, ());
            }
        }

        let mut blocks = Vec::new();
        let mut target_hash = String::new();
        let mut target_fetched = false;
        for slot in slots.keys().copied() {
            match fetch_block_with_retry(&self.rpc, slot, if slot == target_slot { 30 } else { 2 })
                .await
            {
                Ok(block) => {
                    if slot == target_slot {
                        target_fetched = true;
                        target_hash = block.blockhash.clone();
                    }
                    blocks.push(block);
                }
                Err(e) => {
                    if slot == target_slot {
                        warn!("target block {} unavailable: {e:#}", slot);
                    }
                }
            }
        }
        Ok((blocks, target_fetched, target_hash))
    }
}

async fn wait_for_signatures(
    rpc: &RpcClient,
    signatures: &[String],
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    let mut last_seen = 0usize;
    while start.elapsed() < timeout {
        match rpc.get_signature_statuses(signatures).await {
            Ok(statuses) => {
                let seen = statuses.iter().filter(|s| s.is_some()).count();
                if seen > last_seen {
                    last_seen = seen;
                }
                if seen == signatures.len() {
                    return Ok(());
                }
            }
            Err(e) => warn!("signature status poll failed: {e:#}"),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    warn!(
        "only {}/{} signatures observed before timeout; collecting blocks that are available",
        last_seen,
        signatures.len()
    );
    Ok(())
}

async fn fetch_block_with_retry(
    rpc: &RpcClient,
    slot: u64,
    attempts: usize,
) -> Result<DetailedBlock> {
    let mut last_error = None;
    for attempt in 0..attempts {
        match rpc.get_block(slot).await {
            Ok(block) => return Ok(block),
            Err(e) => {
                last_error = Some(e);
                if attempt + 1 < attempts {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("block {slot} unavailable")))
}

fn simulate_block(metadata: &[CraftedTxMetadata], slot: u64, seed: u64) -> DetailedBlock {
    let mut rng = StdRng::seed_from_u64(seed ^ slot);
    let mut used = HashSet::new();
    let mut transactions: Vec<_> = metadata
        .iter()
        .map(|meta| {
            let mut index = 40 + rng.gen_range(0..180usize);
            while !used.insert(index) {
                index += 1;
            }
            crate::rpc::BlockTransaction {
                signature: meta.signature.clone(),
                index_in_block: index,
                fee: meta.estimated_fee_lamports,
                compute_units_consumed: Some(300),
                err: None,
                memo: Some(meta.memo_payload.clone()),
                priority_fee_micro_lamports: Some(meta.priority_fee_micro_lamports),
            }
        })
        .collect();
    transactions.sort_by_key(|t| t.index_in_block);
    let total = transactions
        .iter()
        .map(|t| t.index_in_block)
        .max()
        .unwrap_or(0)
        + 20;
    DetailedBlock {
        slot,
        blockhash: format!("synthetic-{slot}"),
        previous_blockhash: format!("synthetic-{}", slot.saturating_sub(1)),
        parent_slot: slot.saturating_sub(1),
        block_time: None,
        total_transactions: total,
        transactions,
    }
}

fn short_identity(identity: &str) -> &str {
    if identity.len() >= 8 {
        &identity[..8]
    } else {
        identity
    }
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[allow(dead_code)]
pub fn campaign_dir_for(config: &Config, campaign_id: &str) -> PathBuf {
    let subdir = if config.live {
        "campaigns"
    } else {
        "simulated"
    };
    config.data_dir.join(subdir).join(campaign_id)
}

#[allow(dead_code)]
pub fn ensure_campaign_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path.join(crate::campaign::RAW_BLOCKS_DIR))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::Keypair;

    #[test]
    fn simulated_block_has_unique_indexes_and_memo() {
        let payer = Keypair::new();
        let metadata = TxCrafter::build_burst(
            &payer,
            Hash::default(),
            "sim",
            8,
            BurstPattern::RandomizedFee,
        );
        let block = simulate_block(&metadata, 100, 42);
        assert_eq!(block.transactions.len(), 8);
        let mut indexes: Vec<_> = block
            .transactions
            .iter()
            .map(|t| t.index_in_block)
            .collect();
        indexes.sort_unstable();
        indexes.dedup();
        assert_eq!(indexes.len(), 8);
        assert!(block.transactions.iter().all(|t| t
            .memo
            .as_deref()
            .unwrap_or("")
            .contains(crate::tx_crafting::MEMO_PREFIX)));
    }
}
