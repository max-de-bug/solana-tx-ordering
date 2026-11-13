use chrono::Utc;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

pub const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub const MEMO_PREFIX: &str = "solana-tx-order:v2";
pub const DEFAULT_COMPUTE_UNIT_LIMIT: u32 = 10_000;

/// Maximum intended priority fee per transaction. Kept small enough that even
/// a 32-probe burst stays far below the 0.05 SOL campaign cap.
pub const MAX_PRIORITY_LAMPORTS: u64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum BurstPattern {
    /// Ascending priority fees (send order 0 has the lowest fee).
    AscendingFee,
    /// Descending priority fees (send order 0 has the highest fee).
    DescendingFee,
    /// Alternating low/high fees.
    AlternatingFee,
    /// Uniform fee, used to study arrival-order / FIFO effects.
    UniformFee,
    /// Primary causal design: fee ranks are assigned independently of dispatch
    /// order, with both permutations derived from a recorded seed.
    RandomizedFee,
}

impl fmt::Display for BurstPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BurstPattern::AscendingFee => write!(f, "ascending-fee"),
            BurstPattern::DescendingFee => write!(f, "descending-fee"),
            BurstPattern::AlternatingFee => write!(f, "alternating-fee"),
            BurstPattern::UniformFee => write!(f, "uniform-fee"),
            BurstPattern::RandomizedFee => write!(f, "randomized-fee"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CraftedTxMetadata {
    pub burst_id: String,
    /// Position in the dispatch sequence (0 = first wire write).
    pub sequence_index: usize,
    pub signature: String,
    pub base64_payload: String,
    /// On-chain compute-unit price derived from `priority_fee_lamports`.
    pub priority_fee_micro_lamports: u64,
    /// Intended priority fee in lamports, chosen in lamport space so distinct
    /// tiers do not collapse into on-chain fee ties.
    pub priority_fee_lamports: u64,
    pub compute_unit_limit: u32,
    pub estimated_fee_lamports: u64,
    pub send_timestamp_ns: u64,
    pub client_send_instant_iso: String,
    pub pattern: BurstPattern,
    pub memo_payload: String,
    /// Rank of the intended fee tier (0 = lowest).
    pub fee_rank: usize,
    /// Actual dispatch position. Equal to `sequence_index` for new data.
    pub send_order: usize,
    pub payer_pubkey: String,
    #[serde(default)]
    pub send_error: Option<String>,
}

impl CraftedTxMetadata {
    pub fn wire_bytes(&self) -> anyhow::Result<Vec<u8>> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&self.base64_payload)
            .map_err(|e| anyhow::anyhow!("invalid base64 payload for {}: {}", self.signature, e))
    }
}

pub struct TxCrafter;

#[allow(dead_code)]
impl TxCrafter {
    /// Fee tiers are selected in lamport space, then converted to a compute
    /// unit price. This avoids the legacy bug where 100 and 1,000 uL/CU both
    /// became a 1-lamport priority fee at a 1,000 CU limit.
    pub fn generate_fee_lamports(burst_size: usize, pattern: BurstPattern) -> Vec<u64> {
        if burst_size == 0 {
            return Vec::new();
        }
        if burst_size == 1 {
            return vec![1];
        }

        let mut ascending = Vec::with_capacity(burst_size);
        let max = MAX_PRIORITY_LAMPORTS as f64;
        for i in 0..burst_size {
            let factor = i as f64 / (burst_size - 1) as f64;
            let value = max.powf(factor).round() as u64;
            let next = if let Some(prev) = ascending.last() {
                value.max(*prev + 1)
            } else {
                value.max(1)
            };
            ascending.push(next);
        }

        match pattern {
            // The tier vector is always ascending; the send-order plan below
            // decides whether dispatch order is ascending, descending,
            // alternating, or randomized.
            BurstPattern::AscendingFee
            | BurstPattern::DescendingFee
            | BurstPattern::AlternatingFee
            | BurstPattern::RandomizedFee => ascending,
            BurstPattern::UniformFee => vec![1; burst_size],
        }
    }

    /// Convert an intended priority fee in lamports to the on-chain compute
    /// unit price. The ceiling guarantees the charged priority fee is never
    /// below the intended value.
    pub fn compute_unit_price(priority_lamports: u64, cu_limit: u32) -> u64 {
        if cu_limit == 0 {
            return 0;
        }
        let numerator = priority_lamports as u128 * 1_000_000u128;
        let denominator = cu_limit as u128;
        numerator.div_ceil(denominator) as u64
    }

    pub fn estimated_fee(priority_lamports: u64) -> u64 {
        5_000 + priority_lamports
    }

    pub fn build_burst(
        payer: &Keypair,
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id: &str,
        burst_size: usize,
        pattern: BurstPattern,
    ) -> Vec<CraftedTxMetadata> {
        Self::build_burst_with_seed(
            payer,
            recent_blockhash,
            burst_id,
            burst_size,
            pattern,
            seed_from_burst(burst_id, pattern, 0),
            pattern == BurstPattern::RandomizedFee,
        )
    }

    pub fn build_burst_with_seed(
        payer: &Keypair,
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id: &str,
        burst_size: usize,
        pattern: BurstPattern,
        seed: u64,
        randomize_send_order: bool,
    ) -> Vec<CraftedTxMetadata> {
        let payers = vec![payer];
        Self::build_burst_with_payers(
            &payers,
            recent_blockhash,
            burst_id,
            burst_size,
            pattern,
            seed,
            randomize_send_order,
        )
    }

    /// Builds a burst with one independent fee-payer per probe. `payers` is
    /// cycled if it is shorter than `burst_size`, but the caller should
    /// normally pass at least `burst_size` distinct payers.
    pub fn build_burst_with_payers(
        payers: &[&Keypair],
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id: &str,
        burst_size: usize,
        pattern: BurstPattern,
        seed: u64,
        randomize_send_order: bool,
    ) -> Vec<CraftedTxMetadata> {
        let memo_program_pubkey = Pubkey::from_str(MEMO_PROGRAM_ID).unwrap();
        let fee_lamports = Self::generate_fee_lamports(burst_size, pattern);
        let mut rng = StdRng::seed_from_u64(seed);

        // plan[send_order] = fee_rank (0 = lowest fee tier). The tier vector is
        // ascending; only the plan encodes the pattern.
        let mut plan: Vec<usize> = match pattern {
            BurstPattern::AscendingFee | BurstPattern::RandomizedFee => (0..burst_size).collect(),
            BurstPattern::DescendingFee => (0..burst_size).rev().collect(),
            BurstPattern::AlternatingFee => {
                let mut out = Vec::with_capacity(burst_size);
                let mut lo = 0usize;
                let mut hi = burst_size.saturating_sub(1);
                for i in 0..burst_size {
                    if i % 2 == 0 {
                        out.push(lo);
                        lo += 1;
                    } else {
                        out.push(hi);
                        hi = hi.saturating_sub(1);
                    }
                }
                out
            }
            BurstPattern::UniformFee => vec![0; burst_size],
        };
        if pattern == BurstPattern::RandomizedFee || randomize_send_order {
            plan.shuffle(&mut rng);
        }

        let mut burst = Vec::with_capacity(burst_size);
        for (send_order, fee_rank) in plan.into_iter().enumerate() {
            let payer = payers[send_order % payers.len()];
            let priority_lamports = fee_lamports[fee_rank];
            let priority_fee =
                Self::compute_unit_price(priority_lamports, DEFAULT_COMPUTE_UNIT_LIMIT);
            let cu_limit = DEFAULT_COMPUTE_UNIT_LIMIT;

            let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(cu_limit);
            let cu_price_ix = ComputeBudgetInstruction::set_compute_unit_price(priority_fee);

            let send_timestamp_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
            let memo_text = format!(
                "{}:burst={}:send={}:fee_rank={}:pfee={}:plamports={}:ts={}:payer={}",
                MEMO_PREFIX,
                burst_id,
                send_order,
                fee_rank,
                priority_fee,
                priority_lamports,
                send_timestamp_ns,
                payer.pubkey()
            );

            let memo_ix = Instruction {
                program_id: memo_program_pubkey,
                accounts: vec![AccountMeta::new_readonly(payer.pubkey(), true)],
                data: memo_text.as_bytes().to_vec(),
            };

            let message = Message::new(&[cu_limit_ix, cu_price_ix, memo_ix], Some(&payer.pubkey()));
            let mut tx = Transaction::new_unsigned(message);
            tx.sign(&[payer], recent_blockhash);

            let sig = tx.signatures[0].to_string();
            let serialized = bincode::serialize(&tx).unwrap_or_default();
            let base64_payload =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &serialized);

            burst.push(CraftedTxMetadata {
                burst_id: burst_id.to_string(),
                sequence_index: send_order,
                signature: sig,
                base64_payload,
                priority_fee_micro_lamports: priority_fee,
                priority_fee_lamports: priority_lamports,
                compute_unit_limit: cu_limit,
                estimated_fee_lamports: Self::estimated_fee(priority_lamports),
                send_timestamp_ns,
                client_send_instant_iso: Utc::now().to_rfc3339(),
                pattern,
                memo_payload: memo_text,
                fee_rank,
                send_order,
                payer_pubkey: payer.pubkey().to_string(),
                send_error: None,
            });
        }

        burst
    }

    /// Independent-payer wrapper retained for the diagnostic contention path.
    pub fn build_independent_burst(
        payers: &[Keypair],
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id: &str,
        pattern: BurstPattern,
    ) -> Vec<CraftedTxMetadata> {
        let refs: Vec<&Keypair> = payers.iter().collect();
        let seed = seed_from_burst(burst_id, pattern, 0);
        Self::build_burst_with_payers(
            &refs,
            recent_blockhash,
            burst_id,
            payers.len(),
            pattern,
            seed,
            false,
        )
    }

    /// Builds one independent-payer and one shared-payer burst for the
    /// optional diagnostic comparison. It is not used for causal inference.
    pub fn build_contention_pair(
        master_payer: &Keypair,
        independent_payers: &[Keypair],
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id_prefix: &str,
        pattern: BurstPattern,
    ) -> (Vec<CraftedTxMetadata>, Vec<CraftedTxMetadata>) {
        let indep = Self::build_independent_burst(
            independent_payers,
            recent_blockhash,
            &format!("{}-indep", burst_id_prefix),
            pattern,
        );
        let shared = Self::build_burst(
            master_payer,
            recent_blockhash,
            &format!("{}-shared", burst_id_prefix),
            independent_payers.len().max(1),
            pattern,
        );
        (indep, shared)
    }
}

fn seed_from_burst(burst_id: &str, pattern: BurstPattern, extra: u64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    burst_id.hash(&mut hasher);
    (pattern as u8).hash(&mut hasher);
    extra.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keypair() -> Keypair {
        Keypair::new()
    }

    #[test]
    fn fee_tiers_are_strictly_increasing_in_lamports() {
        let fees = TxCrafter::generate_fee_lamports(8, BurstPattern::AscendingFee);
        assert_eq!(fees.len(), 8);
        for pair in fees.windows(2) {
            assert!(
                pair[0] < pair[1],
                "fees must be strictly increasing: {pair:?}"
            );
        }
        assert_eq!(fees[0], 1);
        assert_eq!(*fees.last().unwrap(), MAX_PRIORITY_LAMPORTS);
    }

    #[test]
    fn on_chain_fee_tiers_do_not_collapse() {
        let fees = TxCrafter::generate_fee_lamports(8, BurstPattern::RandomizedFee);
        let charged: Vec<u64> = fees
            .iter()
            .map(|lamports| {
                let price = TxCrafter::compute_unit_price(*lamports, DEFAULT_COMPUTE_UNIT_LIMIT);
                let priority = (price as u128 * DEFAULT_COMPUTE_UNIT_LIMIT as u128) / 1_000_000u128;
                priority as u64
            })
            .collect();
        let mut unique = charged.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            charged.len(),
            "charged fees collapsed: {charged:?}"
        );
    }

    #[test]
    fn burst_generation_preserves_pattern_and_unique_signatures() {
        let payer = keypair();
        let burst = TxCrafter::build_burst(
            &payer,
            solana_sdk::hash::Hash::default(),
            "test-burst",
            8,
            BurstPattern::AscendingFee,
        );
        assert_eq!(burst.len(), 8);
        assert_eq!(burst[0].priority_fee_lamports, 1);
        let mut sigs: Vec<_> = burst.iter().map(|t| t.signature.clone()).collect();
        sigs.sort();
        sigs.dedup();
        assert_eq!(sigs.len(), burst.len());
    }

    #[test]
    fn randomized_burst_decouples_fee_rank_from_send_order() {
        let payer = keypair();
        let burst = TxCrafter::build_burst_with_seed(
            &payer,
            solana_sdk::hash::Hash::default(),
            "randomized",
            8,
            BurstPattern::RandomizedFee,
            12345,
            true,
        );
        assert_eq!(burst.len(), 8);
        let ranks: Vec<usize> = burst.iter().map(|t| t.fee_rank).collect();
        let orders: Vec<usize> = burst.iter().map(|t| t.send_order).collect();
        assert_ne!(
            ranks, orders,
            "fee ranks should not be identical to send order"
        );
        assert!(burst.iter().all(|t| t.memo_payload.contains(MEMO_PREFIX)));
    }

    #[test]
    fn independent_burst_uses_distinct_payers() {
        let payers: Vec<Keypair> = (0..4).map(|_| keypair()).collect();
        let burst = TxCrafter::build_independent_burst(
            &payers,
            solana_sdk::hash::Hash::default(),
            "indep",
            BurstPattern::UniformFee,
        );
        assert_eq!(burst.len(), 4);
        let mut payer_keys: Vec<_> = burst.iter().map(|t| t.payer_pubkey.clone()).collect();
        payer_keys.sort();
        payer_keys.dedup();
        assert_eq!(payer_keys.len(), 4);
    }
}
