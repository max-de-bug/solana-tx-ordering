use crate::rpc::{BlockTransaction, DetailedBlock};
use crate::tx_crafting::{BurstPattern, CraftedTxMetadata};
use crate::validator_info::{ClientType, TargetValidator};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// How a burst was produced. Legacy data has no provenance field, so it
/// deserialises as `Legacy` and is never mixed with fresh measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExecutionMode {
    #[default]
    Legacy,
    Live,
    DryRun,
    Mixed,
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutionMode::Legacy => write!(f, "legacy"),
            ExecutionMode::Live => write!(f, "live"),
            ExecutionMode::DryRun => write!(f, "dry-run"),
            ExecutionMode::Mixed => write!(f, "mixed"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransportKind {
    #[default]
    Rpc,
    TpuQuic,
    Synthetic,
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportKind::Rpc => write!(f, "json-rpc"),
            TransportKind::TpuQuic => write!(f, "tpu-quic"),
            TransportKind::Synthetic => write!(f, "synthetic"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PayerMode {
    #[default]
    Shared,
    Independent,
}

impl fmt::Display for PayerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PayerMode::Shared => write!(f, "shared"),
            PayerMode::Independent => write!(f, "independent"),
        }
    }
}

fn default_design_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurstProvenance {
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    #[serde(default = "default_design_version")]
    pub design_version: u32,
    #[serde(default)]
    pub transport: TransportKind,
    #[serde(default)]
    pub payer_mode: PayerMode,
    /// Slot observed immediately before dispatch. Empty for legacy data.
    #[serde(default)]
    pub send_slot: u64,
    #[serde(default)]
    pub send_started_ns: u64,
    #[serde(default)]
    pub send_finished_ns: u64,
    #[serde(default)]
    pub late_send: bool,
    #[serde(default)]
    pub target_window_start: u64,
    #[serde(default)]
    pub target_window_end: u64,
    #[serde(default)]
    pub target_block_fetched: bool,
    #[serde(default)]
    pub target_block_hash: String,
    #[serde(default)]
    pub memo_prefix: String,
    #[serde(default)]
    pub send_order_randomized: bool,
    #[serde(default)]
    pub fee_assignment_randomized: bool,
}

impl Default for BurstProvenance {
    fn default() -> Self {
        Self {
            execution_mode: ExecutionMode::Legacy,
            design_version: 1,
            transport: TransportKind::Rpc,
            payer_mode: PayerMode::Shared,
            send_slot: 0,
            send_started_ns: 0,
            send_finished_ns: 0,
            late_send: false,
            target_window_start: 0,
            target_window_end: 0,
            target_block_fetched: false,
            target_block_hash: String::new(),
            memo_prefix: String::new(),
            send_order_randomized: false,
            fee_assignment_randomized: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxLandingResult {
    pub signature: String,
    pub sequence_index: usize,
    pub priority_fee_micro_lamports: u64,
    pub send_timestamp_ns: u64,
    pub landed_in_slot: u64,
    pub index_in_block: usize,
    pub total_transactions_in_block: usize,
    pub relative_block_percentile: f64,
    pub fee_paid_lamports: u64,
    pub compute_units_consumed: Option<u64>,
    /// Position in the dispatch sequence. Falls back to `sequence_index` for
    /// the legacy dataset, where fee rank and send order were the same.
    #[serde(default)]
    pub send_order: Option<usize>,
    /// Rank of the intended fee tier (0 = lowest).
    #[serde(default)]
    pub fee_rank: Option<usize>,
    #[serde(default)]
    pub payer_pubkey: String,
    #[serde(default)]
    pub tx_error: bool,
    #[serde(default)]
    pub send_error: Option<String>,
}

impl TxLandingResult {
    pub fn effective_send_order(&self) -> usize {
        self.send_order.unwrap_or(self.sequence_index)
    }

    pub fn effective_fee_lamports(&self) -> u64 {
        if self.fee_paid_lamports >= 5_000 {
            self.fee_paid_lamports
        } else {
            let priority = (self.priority_fee_micro_lamports as u128 * 1_000u128) / 1_000_000u128;
            5_000 + priority as u64
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeMiss {
    pub signature: String,
    pub sequence_index: usize,
    pub send_order: usize,
    pub priority_fee_micro_lamports: u64,
    pub estimated_fee_lamports: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurstAnalysisResult {
    pub burst_id: String,
    pub pattern: BurstPattern,
    pub target_validator_identity: String,
    pub validator_client_type: ClientType,
    pub target_slot: u64,
    pub landed_slot: u64,
    pub slot_lag: i64,
    pub total_tx_sent: usize,
    pub total_tx_landed: usize,
    pub landing_rate_pct: f64,
    pub txs: Vec<TxLandingResult>,
    /// Legacy per-burst correlation fields. New reports do not use these for
    /// inference; they are retained so old JSON remains loadable.
    #[serde(default)]
    pub spearman_fee_correlation: f64,
    #[serde(default)]
    pub spearman_send_correlation: f64,
    #[serde(default)]
    pub kendall_tau_fee: f64,
    #[serde(default)]
    pub p_value_fee: f64,
    #[serde(default)]
    pub p_value_send: f64,
    #[serde(default)]
    pub burst_classification: String,
    #[serde(default)]
    pub provenance: BurstProvenance,
    #[serde(default)]
    pub misses: Vec<ProbeMiss>,
    #[serde(default)]
    pub target_window_start: u64,
    #[serde(default)]
    pub target_window_end: u64,
    #[serde(default)]
    pub target_block_probes: usize,
    #[serde(default)]
    pub target_block_usable: bool,
    #[serde(default)]
    pub same_block_fee_tau: Option<f64>,
    #[serde(default)]
    pub same_block_send_tau: Option<f64>,
}

impl BurstAnalysisResult {
    pub fn window_start(&self) -> u64 {
        if self.target_window_start > 0 {
            self.target_window_start
        } else if self.provenance.target_window_start > 0 {
            self.provenance.target_window_start
        } else {
            self.target_slot
        }
    }

    pub fn window_end(&self) -> u64 {
        if self.target_window_end >= self.window_start() {
            self.target_window_end
        } else if self.provenance.target_window_end >= self.window_start() {
            self.provenance.target_window_end
        } else {
            self.window_start() + 3
        }
    }

    pub fn target_block_probes(&self) -> usize {
        self.txs
            .iter()
            .filter(|tx| tx.landed_in_slot == self.target_slot)
            .count()
    }

    pub fn target_window_probes(&self) -> usize {
        let start = self.window_start();
        let end = self.window_end();
        self.txs
            .iter()
            .filter(|tx| tx.landed_in_slot >= start && tx.landed_in_slot <= end)
            .count()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentionBenchmarkResult {
    pub validator_identity: String,
    pub validator_name: String,
    pub client_type: ClientType,
    pub independent_burst_id: String,
    pub shared_burst_id: String,
    pub independent_spearman_fee: f64,
    pub shared_spearman_fee: f64,
    pub independent_mean_index: f64,
    pub shared_mean_index: f64,
    pub lock_serialization_spread: f64,
    pub contention_delay_assessment: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SameBlockScope {
    TargetBlock,
    TargetWindow,
    AllBlocks,
}

impl fmt::Display for SameBlockScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SameBlockScope::TargetBlock => write!(f, "target block"),
            SameBlockScope::TargetWindow => write!(f, "target leader window"),
            SameBlockScope::AllBlocks => write!(f, "all observed blocks"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparison {
    Fee,
    SendOrder,
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Comparison::Fee => write!(f, "priority fee"),
            Comparison::SendOrder => write!(f, "send order"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SameBlockStats {
    pub scope: Option<SameBlockScope>,
    pub comparison: Option<Comparison>,
    pub blocks_observed: usize,
    pub blocks_with_two_plus_probes: usize,
    pub blocks_used: usize,
    pub probes_observed: usize,
    pub probes_in_scope: usize,
    pub pairs_total: usize,
    pub pairs_usable: usize,
    pub x_ties: usize,
    pub y_ties: usize,
    pub concordant: usize,
    pub discordant: usize,
    pub win_rate: Option<f64>,
    pub mean_block_tau_b: Option<f64>,
    pub median_block_tau_b: Option<f64>,
    pub ci95_low: Option<f64>,
    pub ci95_high: Option<f64>,
    pub sign_test_p: Option<f64>,
    pub permutation_p: Option<f64>,
    pub permutation_trials: usize,
    pub min_blocks: usize,
    pub verdict: String,
}

impl SameBlockStats {
    #[allow(dead_code)]
    pub fn has_enough_blocks(&self, min_blocks: usize) -> bool {
        self.blocks_used >= min_blocks
    }

    pub fn effect_text(&self) -> String {
        match self.mean_block_tau_b {
            Some(tau) => format!(
                "τb={:+.3} (95% CI {:+.3}..{:+.3}), win rate {}",
                tau,
                self.ci95_low.unwrap_or(f64::NAN),
                self.ci95_high.unwrap_or(f64::NAN),
                self.win_rate
                    .map(|v| format!("{:.1}%", v * 100.0))
                    .unwrap_or_else(|| "n/a".to_string())
            ),
            None => "no usable within-block pairs".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SampleAccounting {
    pub total_bursts: usize,
    pub bursts_with_target_block_probes: usize,
    pub bursts_with_target_window_probes: usize,
    pub total_probes_sent: usize,
    pub total_probes_landed: usize,
    pub target_block_probes: usize,
    pub target_window_probes: usize,
    pub out_of_window_probes: usize,
    pub late_bursts: usize,
    pub missing_target_block_bursts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSameBlock {
    pub validator_identity: String,
    pub validator_name: String,
    pub client_type: ClientType,
    pub version_string: String,
    pub stake_sol: f64,
    pub total_bursts: usize,
    pub target_block_bursts: usize,
    pub target_block_probes: usize,
    pub fee: SameBlockStats,
    pub send: SameBlockStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternSameBlock {
    pub pattern: BurstPattern,
    pub bursts: usize,
    pub target_block_probes: usize,
    pub fee: SameBlockStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignAnalysis {
    pub generated_at_utc: String,
    pub campaign_id: String,
    pub execution_mode: ExecutionMode,
    pub design_version: u32,
    pub sample: SampleAccounting,
    pub fee_target_block: SameBlockStats,
    pub send_target_block: SameBlockStats,
    pub fee_target_window: SameBlockStats,
    pub send_target_window: SameBlockStats,
    pub per_validator: Vec<ValidatorSameBlock>,
    pub per_pattern: Vec<PatternSameBlock>,
    pub warnings: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorAggregatedResult {
    pub validator_identity: String,
    pub validator_name: String,
    pub client_type: ClientType,
    pub version_string: String,
    #[serde(default)]
    pub stake_sol: f64,
    pub total_bursts_analyzed: usize,
    pub total_tx_sent: usize,
    pub total_tx_landed: usize,
    pub overall_landing_rate_pct: f64,
    #[serde(default)]
    pub avg_spearman_fee: f64,
    #[serde(default)]
    pub avg_spearman_send: f64,
    #[serde(default)]
    pub avg_kendall_tau_fee: f64,
    pub final_ordering_rule: String,
    pub rule_strictness_score: f64,
    pub rule_description: String,
    pub burst_summaries: Vec<BurstAnalysisResult>,
    #[serde(default)]
    pub same_block_fee: SameBlockStats,
    #[serde(default)]
    pub same_block_send: SameBlockStats,
    #[serde(default)]
    pub target_block_bursts: usize,
    #[serde(default)]
    pub target_block_probes: usize,
    #[serde(default)]
    pub excluded_bursts: usize,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisOptions {
    pub min_blocks: usize,
    pub permutation_trials: usize,
    pub seed: u64,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            min_blocks: 3,
            permutation_trials: 10_000,
            seed: 42,
        }
    }
}

pub struct Analyzer;

impl Analyzer {
    /// Matches crafted probes against every fetched block and records misses
    /// instead of silently dropping them.
    pub fn match_burst_in_blocks(
        burst_meta: &[CraftedTxMetadata],
        blocks: &[DetailedBlock],
        target_validator: &TargetValidator,
        target_slot: u64,
        target_window_end: u64,
        provenance: BurstProvenance,
    ) -> BurstAnalysisResult {
        let pattern = burst_meta
            .first()
            .map(|m| m.pattern)
            .unwrap_or(BurstPattern::AscendingFee);
        let burst_id = burst_meta
            .first()
            .map(|m| m.burst_id.clone())
            .unwrap_or_default();

        let mut chain_index: HashMap<&str, (&DetailedBlock, &BlockTransaction)> = HashMap::new();
        for block in blocks {
            for tx in &block.transactions {
                chain_index
                    .entry(tx.signature.as_str())
                    .or_insert((block, tx));
            }
        }

        let mut landed_txs = Vec::new();
        let mut misses = Vec::new();

        for meta in burst_meta {
            if let Some((block, block_tx)) = chain_index.get(meta.signature.as_str()) {
                let percentile = if block.total_transactions > 0 {
                    (block_tx.index_in_block as f64 / block.total_transactions as f64) * 100.0
                } else {
                    0.0
                };

                landed_txs.push(TxLandingResult {
                    signature: meta.signature.clone(),
                    sequence_index: meta.sequence_index,
                    priority_fee_micro_lamports: meta.priority_fee_micro_lamports,
                    send_timestamp_ns: meta.send_timestamp_ns,
                    landed_in_slot: block.slot,
                    index_in_block: block_tx.index_in_block,
                    total_transactions_in_block: block.total_transactions,
                    relative_block_percentile: percentile,
                    fee_paid_lamports: block_tx.fee,
                    compute_units_consumed: block_tx.compute_units_consumed,
                    send_order: Some(meta.send_order),
                    fee_rank: Some(meta.fee_rank),
                    payer_pubkey: meta.payer_pubkey.clone(),
                    tx_error: block_tx.err.is_some(),
                    send_error: meta.send_error.clone(),
                });
            } else {
                misses.push(ProbeMiss {
                    signature: meta.signature.clone(),
                    sequence_index: meta.sequence_index,
                    send_order: meta.send_order,
                    priority_fee_micro_lamports: meta.priority_fee_micro_lamports,
                    estimated_fee_lamports: meta.estimated_fee_lamports,
                    reason: "signature not found in fetched block(s)".to_string(),
                });
            }
        }

        // Solana block indexes are the order of execution. Sort for stable
        // downstream table generation; the index itself is preserved.
        landed_txs.sort_by_key(|tx| (tx.landed_in_slot, tx.index_in_block));

        let total_sent = burst_meta.len();
        let total_landed = landed_txs.len();
        let landing_rate = if total_sent > 0 {
            (total_landed as f64 / total_sent as f64) * 100.0
        } else {
            0.0
        };
        let landed_slot = landed_txs
            .iter()
            .map(|tx| tx.landed_in_slot)
            .min()
            .unwrap_or(target_slot);
        let target_block_probes = landed_txs
            .iter()
            .filter(|tx| tx.landed_in_slot == target_slot)
            .count();
        let target_window_start = if provenance.target_window_start > 0 {
            provenance.target_window_start
        } else {
            target_slot
        };
        let target_window_end = if target_window_end >= target_window_start {
            target_window_end
        } else {
            target_window_start + 3
        };
        let target_window_probes = landed_txs
            .iter()
            .filter(|tx| {
                tx.landed_in_slot >= target_window_start && tx.landed_in_slot <= target_window_end
            })
            .count();

        BurstAnalysisResult {
            burst_id,
            pattern,
            target_validator_identity: target_validator.identity.clone(),
            validator_client_type: target_validator.client_type,
            target_slot,
            landed_slot,
            slot_lag: landed_slot as i64 - target_slot as i64,
            total_tx_sent: total_sent,
            total_tx_landed: total_landed,
            landing_rate_pct: landing_rate,
            txs: landed_txs,
            spearman_fee_correlation: 0.0,
            spearman_send_correlation: 0.0,
            kendall_tau_fee: 0.0,
            p_value_fee: 1.0,
            p_value_send: 1.0,
            burst_classification: if target_block_probes >= 2 {
                format!(
                    "{} probes in target block; {} in target window; within-block analysis only",
                    target_block_probes, target_window_probes
                )
            } else {
                "No target-block pair evidence; inspect misses and send timing".to_string()
            },
            provenance: BurstProvenance {
                target_window_start,
                target_window_end,
                ..provenance
            },
            misses,
            target_window_start,
            target_window_end,
            target_block_probes,
            target_block_usable: target_block_probes >= 2,
            same_block_fee_tau: None,
            same_block_send_tau: None,
        }
    }

    /// Backwards-compatible single-block wrapper used by tests and legacy
    /// import paths.
    #[allow(dead_code)]
    pub fn match_burst_in_block(
        burst_meta: &[CraftedTxMetadata],
        block: &DetailedBlock,
        target_validator: &TargetValidator,
        target_slot: u64,
    ) -> BurstAnalysisResult {
        Self::match_burst_in_blocks(
            burst_meta,
            std::slice::from_ref(block),
            target_validator,
            target_slot,
            target_slot + 3,
            BurstProvenance::default(),
        )
    }

    pub fn analyze_campaign(
        bursts: &[BurstAnalysisResult],
        options: &AnalysisOptions,
    ) -> CampaignAnalysis {
        let execution_mode = campaign_execution_mode(bursts);
        let design_version = bursts
            .iter()
            .map(|b| b.provenance.design_version)
            .max()
            .unwrap_or(1);

        let mut sample = SampleAccounting {
            total_bursts: bursts.len(),
            ..SampleAccounting::default()
        };
        for burst in bursts {
            sample.total_probes_sent += burst.total_tx_sent;
            sample.total_probes_landed += burst.txs.len();
            sample.target_block_probes += burst.target_block_probes();
            sample.target_window_probes += burst.target_window_probes();
            if burst.target_block_probes() > 0 {
                sample.bursts_with_target_block_probes += 1;
            } else {
                sample.missing_target_block_bursts += 1;
            }
            if burst.target_window_probes() > 0 {
                sample.bursts_with_target_window_probes += 1;
            }
            if burst.provenance.late_send {
                sample.late_bursts += 1;
            }
        }
        sample.out_of_window_probes = sample
            .total_probes_landed
            .saturating_sub(sample.target_window_probes);

        let refs: Vec<&BurstAnalysisResult> = bursts.iter().collect();
        let fee_target_block =
            compute_same_block_stats(&refs, SameBlockScope::TargetBlock, Comparison::Fee, options);
        let send_target_block = compute_same_block_stats(
            &refs,
            SameBlockScope::TargetBlock,
            Comparison::SendOrder,
            options,
        );
        let fee_target_window = compute_same_block_stats(
            &refs,
            SameBlockScope::TargetWindow,
            Comparison::Fee,
            options,
        );
        let send_target_window = compute_same_block_stats(
            &refs,
            SameBlockScope::TargetWindow,
            Comparison::SendOrder,
            options,
        );

        let mut validator_ids = Vec::new();
        for burst in bursts {
            if !validator_ids.contains(&burst.target_validator_identity) {
                validator_ids.push(burst.target_validator_identity.clone());
            }
        }

        let mut per_validator = Vec::new();
        for identity in validator_ids {
            let vbursts: Vec<&BurstAnalysisResult> = bursts
                .iter()
                .filter(|b| b.target_validator_identity == identity)
                .collect();
            let first = match vbursts.first() {
                Some(b) => *b,
                None => continue,
            };
            let fee = compute_same_block_stats(
                &vbursts,
                SameBlockScope::TargetBlock,
                Comparison::Fee,
                options,
            );
            let send = compute_same_block_stats(
                &vbursts,
                SameBlockScope::TargetBlock,
                Comparison::SendOrder,
                options,
            );
            per_validator.push(ValidatorSameBlock {
                validator_identity: identity.clone(),
                validator_name: validator_name_for(&identity),
                client_type: first.validator_client_type,
                version_string: String::new(),
                stake_sol: 0.0,
                total_bursts: vbursts.len(),
                target_block_bursts: vbursts
                    .iter()
                    .filter(|b| b.target_block_probes() > 0)
                    .count(),
                target_block_probes: vbursts.iter().map(|b| b.target_block_probes()).sum(),
                fee,
                send,
            });
        }

        let patterns = [
            BurstPattern::AscendingFee,
            BurstPattern::DescendingFee,
            BurstPattern::AlternatingFee,
            BurstPattern::UniformFee,
            BurstPattern::RandomizedFee,
        ];
        let mut per_pattern = Vec::new();
        for pattern in patterns {
            let pbursts: Vec<&BurstAnalysisResult> =
                bursts.iter().filter(|b| b.pattern == pattern).collect();
            if pbursts.is_empty() {
                continue;
            }
            per_pattern.push(PatternSameBlock {
                pattern,
                bursts: pbursts.len(),
                target_block_probes: pbursts.iter().map(|b| b.target_block_probes()).sum(),
                fee: compute_same_block_stats(
                    &pbursts,
                    SameBlockScope::TargetBlock,
                    Comparison::Fee,
                    options,
                ),
            });
        }

        let mut warnings = Vec::new();
        if execution_mode == ExecutionMode::Legacy {
            warnings.push(
                "This is the legacy v1 dataset. It has no target-block probes, uses the legacy \
                 fee-tier design (many on-chain fee ties), and its original report applied \
                 cross-block correlations. Treat it as diagnostic only."
                    .to_string(),
            );
        }
        if execution_mode == ExecutionMode::DryRun {
            warnings.push(
                "This campaign is a dry-run simulation. It validates the pipeline, not validator \
                 behaviour."
                    .to_string(),
            );
        }
        if sample.target_block_probes == 0 {
            warnings.push(
                "No probe landed in any target block. Same-block target-block inference is \
                 therefore impossible for this campaign."
                    .to_string(),
            );
        }
        if sample.late_bursts > 0 {
            warnings.push(format!(
                "{} burst(s) were dispatched at or after their target slot and cannot support \
                 pre-window causal claims.",
                sample.late_bursts
            ));
        }

        CampaignAnalysis {
            generated_at_utc: chrono::Utc::now().to_rfc3339(),
            campaign_id: bursts
                .first()
                .map(|b| b.burst_id.clone())
                .unwrap_or_else(|| "empty".to_string()),
            execution_mode,
            design_version,
            sample,
            fee_target_block,
            send_target_block,
            fee_target_window,
            send_target_window,
            per_validator,
            per_pattern,
            warnings,
        }
    }

    #[allow(dead_code)]
    pub fn aggregate_validator(
        validator: &TargetValidator,
        bursts: &[BurstAnalysisResult],
        options: &AnalysisOptions,
    ) -> ValidatorAggregatedResult {
        let v_bursts: Vec<BurstAnalysisResult> = bursts
            .iter()
            .filter(|b| b.target_validator_identity == validator.identity)
            .cloned()
            .collect();
        let refs: Vec<&BurstAnalysisResult> = v_bursts.iter().collect();

        let total_bursts = v_bursts.len();
        let total_sent: usize = v_bursts.iter().map(|b| b.total_tx_sent).sum();
        let total_landed: usize = v_bursts.iter().map(|b| b.total_tx_landed).sum();
        let landing_rate = if total_sent > 0 {
            (total_landed as f64 / total_sent as f64) * 100.0
        } else {
            0.0
        };

        let fee =
            compute_same_block_stats(&refs, SameBlockScope::TargetBlock, Comparison::Fee, options);
        let send = compute_same_block_stats(
            &refs,
            SameBlockScope::TargetBlock,
            Comparison::SendOrder,
            options,
        );

        let avg_spearman_fee = fee.mean_block_tau_b.unwrap_or(0.0);
        let avg_spearman_send = send.mean_block_tau_b.unwrap_or(0.0);
        let avg_kendall = fee.mean_block_tau_b.unwrap_or(0.0);
        let strictness = fee.mean_block_tau_b.unwrap_or(0.0).abs().clamp(0.0, 1.0);

        let (rule, desc) = if fee.blocks_used < options.min_blocks {
            (
                "Insufficient same-block evidence".to_string(),
                format!(
                    "Only {} usable target block(s); at least {} are required for the configured \
                     inference rule. No ordering claim is made.",
                    fee.blocks_used, options.min_blocks
                ),
            )
        } else if fee.permutation_p.map(|p| p < 0.05).unwrap_or(false)
            && fee.mean_block_tau_b.unwrap_or(0.0) > 0.0
        {
            (
                format!(
                    "Higher fee tends to land earlier ({})",
                    validator.client_type
                ),
                format!(
                    "{} across {} usable blocks. This is a within-block association, not proof of \
                     a scheduler implementation.",
                    fee.effect_text(),
                    fee.blocks_used
                ),
            )
        } else if fee.permutation_p.map(|p| p < 0.05).unwrap_or(false)
            && fee.mean_block_tau_b.unwrap_or(0.0) < 0.0
        {
            (
                format!("Higher fee tends to land later ({})", validator.client_type),
                format!(
                    "{} across {} usable blocks. The direction is opposite to a simple priority \
                     queue.",
                    fee.effect_text(),
                    fee.blocks_used
                ),
            )
        } else {
            (
                "No statistically robust ordering rule (null result)".to_string(),
                format!(
                    "{} across {} usable blocks. The campaign does not distinguish a stable \
                     fee-priority rule from noise at the configured threshold.",
                    fee.effect_text(),
                    fee.blocks_used
                ),
            )
        };

        ValidatorAggregatedResult {
            validator_identity: validator.identity.clone(),
            validator_name: validator.name.clone(),
            client_type: validator.client_type,
            version_string: validator.version_string.clone(),
            stake_sol: validator.stake_sol,
            total_bursts_analyzed: total_bursts,
            total_tx_sent: total_sent,
            total_tx_landed: total_landed,
            overall_landing_rate_pct: landing_rate,
            avg_spearman_fee,
            avg_spearman_send,
            avg_kendall_tau_fee: avg_kendall,
            final_ordering_rule: rule,
            rule_strictness_score: strictness,
            rule_description: desc,
            burst_summaries: v_bursts.clone(),
            same_block_fee: fee,
            same_block_send: send,
            target_block_bursts: v_bursts
                .iter()
                .filter(|b| b.target_block_probes() > 0)
                .count(),
            target_block_probes: v_bursts.iter().map(|b| b.target_block_probes()).sum(),
            excluded_bursts: v_bursts
                .iter()
                .filter(|b| b.target_block_probes() < 2)
                .count(),
            execution_mode: campaign_execution_mode(&v_bursts),
        }
    }

    /// Compares average placement between two diagnostic bursts. Retained for
    /// the optional legacy contention command; it is not used for inference.
    #[allow(dead_code)]
    pub fn compare_contention_pair(
        validator: &TargetValidator,
        indep: &BurstAnalysisResult,
        shared: &BurstAnalysisResult,
    ) -> ContentionBenchmarkResult {
        let indep_mean = mean_index(&indep.txs);
        let shared_mean = mean_index(&shared.txs);
        let spread = shared_mean - indep_mean;
        let assessment = if spread > 10.0 {
            format!(
                "Descriptive only: +{:.1} block-index shift for the shared-payer burst.",
                spread
            )
        } else {
            "Descriptive only: no large block-index shift observed.".to_string()
        };

        ContentionBenchmarkResult {
            validator_identity: validator.identity.clone(),
            validator_name: validator.name.clone(),
            client_type: validator.client_type,
            independent_burst_id: indep.burst_id.clone(),
            shared_burst_id: shared.burst_id.clone(),
            independent_spearman_fee: indep.same_block_fee_tau.unwrap_or(0.0),
            shared_spearman_fee: shared.same_block_fee_tau.unwrap_or(0.0),
            independent_mean_index: indep_mean,
            shared_mean_index: shared_mean,
            lock_serialization_spread: spread,
            contention_delay_assessment: assessment,
        }
    }
}

fn campaign_execution_mode(bursts: &[BurstAnalysisResult]) -> ExecutionMode {
    let mut has_live = false;
    let mut has_dry = false;
    for burst in bursts {
        match burst.provenance.execution_mode {
            ExecutionMode::Live => has_live = true,
            ExecutionMode::DryRun => has_dry = true,
            ExecutionMode::Mixed => return ExecutionMode::Mixed,
            ExecutionMode::Legacy => {}
        }
    }
    if has_live && has_dry {
        ExecutionMode::Mixed
    } else if has_live {
        ExecutionMode::Live
    } else if has_dry {
        ExecutionMode::DryRun
    } else {
        ExecutionMode::Legacy
    }
}

pub fn validator_name_for(identity: &str) -> String {
    if identity.len() >= 8 {
        format!("Validator {}", &identity[..8])
    } else {
        format!("Validator {}", identity)
    }
}

#[allow(dead_code)]
pub fn mean_index(txs: &[TxLandingResult]) -> f64 {
    if txs.is_empty() {
        return 0.0;
    }
    txs.iter().map(|t| t.index_in_block as f64).sum::<f64>() / txs.len() as f64
}

/// Computes block-clustered same-block statistics. The unit of observation is
/// a (burst, block) pair; pairs from different blocks are never compared.
pub fn compute_same_block_stats(
    bursts: &[&BurstAnalysisResult],
    scope: SameBlockScope,
    comparison: Comparison,
    options: &AnalysisOptions,
) -> SameBlockStats {
    let mut stats = SameBlockStats {
        scope: Some(scope),
        comparison: Some(comparison),
        min_blocks: options.min_blocks,
        permutation_trials: options.permutation_trials,
        verdict: String::new(),
        ..SameBlockStats::default()
    };

    // (burst_id, slot) -> probes. BTreeMap keeps output deterministic.
    let mut blocks: BTreeMap<(String, u64), Vec<&TxLandingResult>> = BTreeMap::new();
    let mut observed_slots: BTreeMap<(String, u64), ()> = BTreeMap::new();

    for burst in bursts {
        stats.probes_observed += burst.txs.len();
        for tx in &burst.txs {
            observed_slots.insert((burst.burst_id.clone(), tx.landed_in_slot), ());
            if !in_scope(burst, tx, scope) {
                continue;
            }
            blocks
                .entry((burst.burst_id.clone(), tx.landed_in_slot))
                .or_default()
                .push(tx);
        }
    }

    stats.blocks_observed = observed_slots.len();

    let mut block_taus = Vec::new();
    let mut permutation_blocks: Vec<Vec<(f64, f64)>> = Vec::new();

    for ((_burst_id, _slot), txs) in &blocks {
        stats.probes_in_scope += txs.len();
        if txs.len() >= 2 {
            stats.blocks_with_two_plus_probes += 1;
        }

        let n = txs.len();
        let n0 = n * (n - 1) / 2;
        if n0 == 0 {
            continue;
        }

        let mut x_groups: HashMap<u64, usize> = HashMap::new();
        let mut y_groups: HashMap<u64, usize> = HashMap::new();
        let mut block_x = Vec::with_capacity(n);
        let mut block_y = Vec::with_capacity(n);

        for tx in txs {
            let x = comparison_value(tx, comparison);
            block_x.push(x);
            block_y.push(tx.index_in_block as u64);
            *x_groups.entry(x).or_default() += 1;
            *y_groups.entry(tx.index_in_block as u64).or_default() += 1;
        }

        let n1: usize = x_groups.values().map(|t| t * (t - 1) / 2).sum();
        let n2: usize = y_groups.values().map(|t| t * (t - 1) / 2).sum();
        let denom = ((n0 - n1) as f64 * (n0 - n2) as f64).sqrt();

        let mut c = 0usize;
        let mut d = 0usize;
        for i in 0..n {
            for j in (i + 1)..n {
                stats.pairs_total += 1;
                let xi = block_x[i];
                let xj = block_x[j];
                let yi = block_y[i];
                let yj = block_y[j];
                if xi == xj {
                    stats.x_ties += 1;
                    continue;
                }
                if yi == yj {
                    stats.y_ties += 1;
                    continue;
                }
                stats.pairs_usable += 1;
                // x is oriented so that larger x means "should be earlier".
                let concordant = (xi > xj) != (yi > yj);
                if concordant {
                    c += 1;
                } else {
                    d += 1;
                }
            }
        }

        if denom == 0.0 {
            continue;
        }
        stats.concordant += c;
        stats.discordant += d;
        let tau = (c as f64 - d as f64) / denom;
        block_taus.push(tau);
        permutation_blocks.push(
            block_x
                .into_iter()
                .zip(block_y)
                .map(|(x, y)| (x as f64, y as f64))
                .collect(),
        );
    }

    stats.blocks_used = block_taus.len();
    if stats.pairs_usable > 0 {
        stats.win_rate = Some(stats.concordant as f64 / stats.pairs_usable as f64);
        stats.mean_block_tau_b = Some(block_taus.iter().sum::<f64>() / block_taus.len() as f64);
        let mut sorted = block_taus.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        stats.median_block_tau_b = Some(median_sorted(&sorted));
    }

    if block_taus.len() >= 2 {
        let (lo, hi) = block_bootstrap_ci(&block_taus, 2_000, options.seed);
        stats.ci95_low = Some(lo);
        stats.ci95_high = Some(hi);
    }

    stats.sign_test_p = exact_sign_test_p(&block_taus);
    stats.permutation_p = permutation_test(
        &permutation_blocks,
        stats.mean_block_tau_b,
        options.permutation_trials,
        options.seed,
    );

    stats.verdict = verdict_for(&stats, options.min_blocks);
    stats
}

fn in_scope(burst: &BurstAnalysisResult, tx: &TxLandingResult, scope: SameBlockScope) -> bool {
    match scope {
        SameBlockScope::TargetBlock => tx.landed_in_slot == burst.target_slot,
        SameBlockScope::TargetWindow => {
            tx.landed_in_slot >= burst.window_start() && tx.landed_in_slot <= burst.window_end()
        }
        SameBlockScope::AllBlocks => true,
    }
}

fn comparison_value(tx: &TxLandingResult, comparison: Comparison) -> u64 {
    match comparison {
        Comparison::Fee => tx.effective_fee_lamports(),
        // Earlier send order should map to a larger value.
        Comparison::SendOrder => u64::MAX - tx.effective_send_order() as u64,
    }
}

fn median_sorted(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn verdict_for(stats: &SameBlockStats, min_blocks: usize) -> String {
    if stats.blocks_used == 0 {
        return "Insufficient: no block has two or more probes with distinct comparison values and block indexes.".to_string();
    }
    if stats.blocks_used < min_blocks {
        return format!(
            "Insufficient: {} usable block(s), below the pre-registered minimum of {}.",
            stats.blocks_used, min_blocks
        );
    }
    let p = stats.permutation_p.unwrap_or(1.0);
    let tau = stats.mean_block_tau_b.unwrap_or(0.0);
    let ci_excludes_zero = stats
        .ci95_low
        .zip(stats.ci95_high)
        .map(|(lo, hi)| lo > 0.0 || hi < 0.0)
        .unwrap_or(false);

    if p < 0.05 && ci_excludes_zero && tau > 0.0 {
        format!(
            "Positive within-block association ({})",
            comparison_direction(stats.comparison.unwrap_or(Comparison::Fee), true)
        )
    } else if p < 0.05 && ci_excludes_zero && tau < 0.0 {
        format!(
            "Negative within-block association ({})",
            comparison_direction(stats.comparison.unwrap_or(Comparison::Fee), false)
        )
    } else if p < 0.05 {
        "Nominally significant permutation result, but the block-bootstrap CI includes zero; \
         treat as inconclusive."
            .to_string()
    } else {
        "No statistically robust within-block association at the configured threshold.".to_string()
    }
}

fn comparison_direction(comparison: Comparison, positive: bool) -> &'static str {
    match (comparison, positive) {
        (Comparison::Fee, true) => "higher fee tends earlier",
        (Comparison::Fee, false) => "higher fee tends later",
        (Comparison::SendOrder, true) => "earlier send position tends earlier",
        (Comparison::SendOrder, false) => "earlier send position tends later",
    }
}

fn binomial_two_sided_p(k: usize, n: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    if n <= 1 {
        return 1.0;
    }
    let k = k.min(n);
    let mut pmf = Vec::with_capacity(n + 1);
    for i in 0..=n {
        pmf.push(choose(n, i) * 0.5f64.powi(n as i32));
    }
    let lower: f64 = pmf[..=k].iter().sum();
    let upper: f64 = pmf[k..].iter().sum();
    (2.0 * lower.min(upper)).min(1.0)
}

fn choose(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut result = 1.0f64;
    for i in 0..k {
        result *= (n - i) as f64;
        result /= (i + 1) as f64;
    }
    result
}

fn exact_sign_test_p(block_taus: &[f64]) -> Option<f64> {
    let nonzero: Vec<f64> = block_taus
        .iter()
        .copied()
        .filter(|t| t.abs() > f64::EPSILON)
        .collect();
    if nonzero.is_empty() {
        return Some(1.0);
    }
    let positives = nonzero.iter().filter(|t| **t > 0.0).count();
    Some(binomial_two_sided_p(positives, nonzero.len()))
}

fn permutation_test(
    blocks: &[Vec<(f64, f64)>],
    observed: Option<f64>,
    trials: usize,
    seed: u64,
) -> Option<f64> {
    let observed = observed?;
    if blocks.is_empty() || trials == 0 {
        return None;
    }
    let mut rng = StdRng::seed_from_u64(seed ^ 0x5EED_5EED);
    let mut extreme = 0usize;
    let mut xs: Vec<Vec<f64>> = blocks
        .iter()
        .map(|b| b.iter().map(|(x, _)| *x).collect())
        .collect();
    for _ in 0..trials {
        let mut sum = 0.0;
        let mut used = 0usize;
        for (block, x) in blocks.iter().zip(xs.iter_mut()) {
            x.shuffle(&mut rng);
            if let Some(tau) = tau_from_values(x, block.iter().map(|(_, y)| *y)) {
                sum += tau;
                used += 1;
            }
        }
        if used == 0 {
            continue;
        }
        let stat = sum / used as f64;
        if stat.abs() >= observed.abs() - 1e-12 {
            extreme += 1;
        }
    }
    Some((1 + extreme) as f64 / (trials + 1) as f64)
}

fn tau_from_values<I>(xs: &[f64], ys: I) -> Option<f64>
where
    I: IntoIterator<Item = f64>,
{
    let ys: Vec<f64> = ys.into_iter().collect();
    if xs.len() != ys.len() || xs.len() < 2 {
        return None;
    }
    let n = xs.len();
    let n0 = n * (n - 1) / 2;
    let mut x_groups: HashMap<u64, usize> = HashMap::new();
    let mut y_groups: HashMap<u64, usize> = HashMap::new();
    for (x, y) in xs.iter().zip(ys.iter()) {
        *x_groups.entry(x.to_bits()).or_default() += 1;
        *y_groups.entry(y.to_bits()).or_default() += 1;
    }
    let n1: usize = x_groups.values().map(|t| t * (t - 1) / 2).sum();
    let n2: usize = y_groups.values().map(|t| t * (t - 1) / 2).sum();
    let denom = ((n0 - n1) as f64 * (n0 - n2) as f64).sqrt();
    if denom == 0.0 {
        return None;
    }
    let mut c = 0usize;
    let mut d = 0usize;
    for i in 0..n {
        for j in (i + 1)..n {
            let xi = xs[i];
            let xj = xs[j];
            let yi = ys[i];
            let yj = ys[j];
            if xi == xj || yi == yj {
                continue;
            }
            if (xi > xj) != (yi > yj) {
                c += 1;
            } else {
                d += 1;
            }
        }
    }
    Some((c as f64 - d as f64) / denom)
}

fn block_bootstrap_ci(block_taus: &[f64], reps: usize, seed: u64) -> (f64, f64) {
    if block_taus.is_empty() {
        return (0.0, 0.0);
    }
    let mut rng = StdRng::seed_from_u64(seed ^ 0xB007_57A9);
    let mut means = Vec::with_capacity(reps);
    for _ in 0..reps {
        let mut sum = 0.0;
        for _ in 0..block_taus.len() {
            let idx = rng.gen_range(0..block_taus.len());
            sum += block_taus[idx];
        }
        means.push(sum / block_taus.len() as f64);
    }
    means.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lo = means[(reps as f64 * 0.025).floor() as usize];
    let hi = means[((reps as f64 * 0.975).ceil() as usize).min(reps - 1)];
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(sig: &str, fee: u64, slot: u64, index: usize, send_order: usize) -> TxLandingResult {
        TxLandingResult {
            signature: sig.to_string(),
            sequence_index: send_order,
            priority_fee_micro_lamports: 0,
            send_timestamp_ns: 0,
            landed_in_slot: slot,
            index_in_block: index,
            total_transactions_in_block: 100,
            relative_block_percentile: 0.0,
            fee_paid_lamports: fee,
            compute_units_consumed: Some(300),
            send_order: Some(send_order),
            fee_rank: None,
            payer_pubkey: "payer".to_string(),
            tx_error: false,
            send_error: None,
        }
    }

    fn burst(id: &str, target: u64, txs: Vec<TxLandingResult>) -> BurstAnalysisResult {
        BurstAnalysisResult {
            burst_id: id.to_string(),
            pattern: BurstPattern::RandomizedFee,
            target_validator_identity: "val".to_string(),
            validator_client_type: ClientType::Agave,
            target_slot: target,
            landed_slot: txs.first().map(|t| t.landed_in_slot).unwrap_or(target),
            slot_lag: 0,
            total_tx_sent: txs.len(),
            total_tx_landed: txs.len(),
            landing_rate_pct: 100.0,
            txs,
            spearman_fee_correlation: 0.0,
            spearman_send_correlation: 0.0,
            kendall_tau_fee: 0.0,
            p_value_fee: 1.0,
            p_value_send: 1.0,
            burst_classification: String::new(),
            provenance: BurstProvenance {
                target_window_start: target,
                target_window_end: target + 3,
                ..BurstProvenance::default()
            },
            misses: Vec::new(),
            target_window_start: target,
            target_window_end: target + 3,
            target_block_probes: 0,
            target_block_usable: false,
            same_block_fee_tau: None,
            same_block_send_tau: None,
        }
    }

    #[test]
    fn same_block_filters_cross_block_probes() {
        let b = burst(
            "b1",
            100,
            vec![
                tx("a", 5100, 100, 3, 0),
                tx("b", 5200, 100, 1, 1),
                tx("c", 5300, 101, 2, 2),
            ],
        );
        let stats = compute_same_block_stats(
            &[&b],
            SameBlockScope::TargetBlock,
            Comparison::Fee,
            &AnalysisOptions::default(),
        );
        assert_eq!(stats.probes_observed, 3);
        assert_eq!(stats.probes_in_scope, 2);
        assert_eq!(stats.pairs_usable, 1);
        assert_eq!(stats.concordant, 1);
    }

    #[test]
    fn fee_ties_are_excluded_not_counted_as_concordant() {
        let b = burst(
            "b1",
            100,
            vec![tx("a", 5001, 100, 2, 0), tx("b", 5001, 100, 1, 1)],
        );
        let stats = compute_same_block_stats(
            &[&b],
            SameBlockScope::TargetBlock,
            Comparison::Fee,
            &AnalysisOptions::default(),
        );
        assert_eq!(stats.pairs_total, 1);
        assert_eq!(stats.pairs_usable, 0);
        assert_eq!(stats.x_ties, 1);
        assert!(stats.verdict.starts_with("Insufficient"));
    }

    #[test]
    fn sign_test_matches_known_values() {
        assert!((binomial_two_sided_p(5, 5) - 0.0625).abs() < 1e-12);
        assert!((binomial_two_sided_p(0, 5) - 0.0625).abs() < 1e-12);
        assert!((binomial_two_sided_p(2, 4) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn higher_fee_earlier_is_positive() {
        let b = burst(
            "b1",
            100,
            vec![
                tx("a", 5100, 100, 2, 0),
                tx("b", 5200, 100, 1, 1),
                tx("c", 5300, 100, 0, 2),
            ],
        );
        let stats = compute_same_block_stats(
            &[&b],
            SameBlockScope::TargetBlock,
            Comparison::Fee,
            &AnalysisOptions::default(),
        );
        assert_eq!(stats.concordant, 3);
        assert_eq!(stats.discordant, 0);
        assert!(stats.mean_block_tau_b.unwrap() > 0.99);
    }
}
