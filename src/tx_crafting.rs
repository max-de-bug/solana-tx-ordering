use chrono::Utc;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::str::FromStr;

pub const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum BurstPattern {
    /// Ascending priority fees (tx 0 has lowest fee, tx N has highest fee)
    AscendingFee,
    /// Inverted: Descending priority fees (tx 0 has highest fee, tx N has lowest fee)
    /// Crucial for disentangling FIFO send order from fee-based priority!
    DescendingFee,
    /// Alternating high and low fees
    AlternatingFee,
    /// Uniform fee (tests pure arrival time / FIFO)
    UniformFee,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CraftedTxMetadata {
    pub burst_id: String,
    pub sequence_index: usize,
    pub signature: String,
    pub base64_payload: String,
    pub priority_fee_micro_lamports: u64,
    pub compute_unit_limit: u32,
    pub estimated_fee_lamports: u64,
    pub client_send_timestamp_ns: u64,
    pub client_send_instant_iso: String,
    pub pattern: BurstPattern,
    pub memo_payload: String,
}

pub struct TxCrafter;

impl TxCrafter {
    /// Generates priority fee tiers spanning 100 to 10,000,000 uL/CU dynamically
    /// for any burst size N. For N=8, preserves canonical exact tiers.
    pub fn generate_fee_tiers(burst_size: usize, pattern: BurstPattern) -> Vec<u64> {
        if burst_size <= 1 {
            return vec![10_000];
        }

        match pattern {
            BurstPattern::AscendingFee => {
                if burst_size == 8 {
                    vec![
                        100,        // 0.1 uL/CU
                        1_000,      // 1 uL/CU
                        5_000,      // 5 uL/CU
                        20_000,     // 20 uL/CU
                        100_000,    // 100 uL/CU
                        500_000,    // 500 uL/CU
                        2_000_000,  // 2,000 uL/CU
                        10_000_000, // 10,000 uL/CU
                    ]
                } else {
                    // Logarithmic scale spanning 100 to 10,000,000 uL/CU with strictly distinct values
                    (0..burst_size)
                        .map(|i| {
                            let factor = i as f64 / (burst_size - 1) as f64;
                            let val = 100.0 * 10.0f64.powf(5.0 * factor);
                            val.round() as u64
                        })
                        .collect()
                }
            }
            BurstPattern::DescendingFee => {
                let mut tiers = Self::generate_fee_tiers(burst_size, BurstPattern::AscendingFee);
                tiers.reverse();
                tiers
            }
            BurstPattern::AlternatingFee => {
                let asc = Self::generate_fee_tiers(burst_size, BurstPattern::AscendingFee);
                let mut tiers = Vec::with_capacity(burst_size);
                let half = burst_size / 2;
                for i in 0..burst_size {
                    if i % 2 == 0 {
                        tiers.push(asc[i / 2]);
                    } else {
                        tiers.push(asc[burst_size - 1 - (i / 2).min(half)]);
                    }
                }
                tiers
            }
            BurstPattern::UniformFee => {
                vec![10_000; burst_size]
            }
        }
    }

    /// Crafts a burst of test transactions with precisely controlled priority fees and metadata
    pub fn build_burst(
        payer: &Keypair,
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id: &str,
        burst_size: usize,
        pattern: BurstPattern,
    ) -> Vec<CraftedTxMetadata> {
        let memo_program_pubkey = Pubkey::from_str(MEMO_PROGRAM_ID).unwrap();
        let mut burst = Vec::with_capacity(burst_size);
        let fee_tiers = Self::generate_fee_tiers(burst_size, pattern);

        for (i, &priority_fee) in fee_tiers.iter().enumerate().take(burst_size) {
            // 1,000 CUs provides sufficient budget for SPL Memo (~800 CUs) while keeping lamport spend negligible
            let cu_limit: u32 = 1_000;

            // 1. Compute Budget Instructions
            let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(cu_limit);
            let cu_price_ix = ComputeBudgetInstruction::set_compute_unit_price(priority_fee);

            // 2. Unique Memo Instruction (guarantees payload uniqueness against deduplication)
            let send_timestamp_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
            let memo_text = format!(
                "solana-tx-order:burst={}:seq={}:pfee={}:ts={}",
                burst_id, i, priority_fee, send_timestamp_ns
            );

            let memo_ix = Instruction {
                program_id: memo_program_pubkey,
                accounts: vec![AccountMeta::new_readonly(payer.pubkey(), true)],
                data: memo_text.as_bytes().to_vec(),
            };

            // 3. Assemble Transaction
            let message = Message::new(
                &[cu_limit_ix, cu_price_ix, memo_ix],
                Some(&payer.pubkey()),
            );

            let mut tx = Transaction::new_unsigned(message);
            tx.sign(&[payer], recent_blockhash);

            let sig = tx.signatures[0].to_string();
            let serialized = bincode::serialize(&tx).unwrap_or_default();
            let base64_payload = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &serialized,
            );

            // Calculate worst-case fee in lamports: base fee (5000) + priority fee
            let priority_lamports = ((priority_fee as u128 * cu_limit as u128) / 1_000_000) as u64;
            let estimated_fee_lamports = 5000 + priority_lamports;

            burst.push(CraftedTxMetadata {
                burst_id: burst_id.to_string(),
                sequence_index: i,
                signature: sig,
                base64_payload,
                priority_fee_micro_lamports: priority_fee,
                compute_unit_limit: cu_limit,
                estimated_fee_lamports,
                client_send_timestamp_ns: send_timestamp_ns,
                client_send_instant_iso: Utc::now().to_rfc3339(),
                pattern,
                memo_payload: memo_text,
            });
        }

        burst
    }

    /// Crafts an independent-payer burst where each transaction has a unique fee-payer keypair.
    /// This eliminates fee-payer write-lock collisions and allows true multi-thread scheduling evaluation!
    pub fn build_independent_burst(
        payers: &[Keypair],
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id: &str,
        pattern: BurstPattern,
    ) -> Vec<CraftedTxMetadata> {
        let memo_program_pubkey = Pubkey::from_str(MEMO_PROGRAM_ID).unwrap();
        let burst_size = payers.len();
        let mut burst = Vec::with_capacity(burst_size);
        let fee_tiers = Self::generate_fee_tiers(burst_size, pattern);

        for (i, payer) in payers.iter().enumerate() {
            let priority_fee = fee_tiers[i];
            // 1,000 CUs provides sufficient budget for SPL Memo (~800 CUs)
            let cu_limit: u32 = 1_000;

            let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(cu_limit);
            let cu_price_ix = ComputeBudgetInstruction::set_compute_unit_price(priority_fee);

            let send_timestamp_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
            let memo_text = format!(
                "solana-tx-order:burst={}:seq={}:pfee={}:ts={}:independent=true",
                burst_id, i, priority_fee, send_timestamp_ns
            );

            let memo_ix = Instruction {
                program_id: memo_program_pubkey,
                accounts: vec![AccountMeta::new_readonly(payer.pubkey(), true)],
                data: memo_text.as_bytes().to_vec(),
            };

            let message = Message::new(
                &[cu_limit_ix, cu_price_ix, memo_ix],
                Some(&payer.pubkey()),
            );

            let mut tx = Transaction::new_unsigned(message);
            tx.sign(&[payer], recent_blockhash);

            let sig = tx.signatures[0].to_string();
            let serialized = bincode::serialize(&tx).unwrap_or_default();
            let base64_payload = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &serialized,
            );

            let priority_lamports = ((priority_fee as u128 * cu_limit as u128) / 1_000_000) as u64;
            let estimated_fee_lamports = 5000 + priority_lamports;

            burst.push(CraftedTxMetadata {
                burst_id: burst_id.to_string(),
                sequence_index: i,
                signature: sig,
                base64_payload,
                priority_fee_micro_lamports: priority_fee,
                compute_unit_limit: cu_limit,
                estimated_fee_lamports,
                client_send_timestamp_ns: send_timestamp_ns,
                client_send_instant_iso: Utc::now().to_rfc3339(),
                pattern,
                memo_payload: memo_text,
            });
        }

        burst
    }

    /// Crafts a side-by-side contention benchmark pair:
    /// (1) Independent-payer burst (zero write-lock collision, evaluates parallel queues)
    /// (2) Shared-payer burst (same payer account, forces Sealevel write-lock serialization)
    pub fn build_contention_pair(
        master_payer: &Keypair,
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id_prefix: &str,
        burst_size: usize,
        pattern: BurstPattern,
    ) -> (Vec<CraftedTxMetadata>, Vec<CraftedTxMetadata>) {
        let independent_payers: Vec<Keypair> = (0..burst_size).map(|_| Keypair::new()).collect();
        let indep_burst = Self::build_independent_burst(
            &independent_payers,
            recent_blockhash,
            &format!("{}-indep", burst_id_prefix),
            pattern,
        );
        let shared_burst = Self::build_burst(
            master_payer,
            recent_blockhash,
            &format!("{}-shared", burst_id_prefix),
            burst_size,
            pattern,
        );
        (indep_burst, shared_burst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_crafter_burst_generation() {
        let payer = Keypair::new();
        let bh = solana_sdk::hash::Hash::default();

        let asc_burst = TxCrafter::build_burst(&payer, bh, "test-burst", 8, BurstPattern::AscendingFee);
        assert_eq!(asc_burst.len(), 8);
        assert_eq!(asc_burst[0].priority_fee_micro_lamports, 100);
        assert_eq!(asc_burst[7].priority_fee_micro_lamports, 10_000_000);

        let desc_burst = TxCrafter::build_burst(&payer, bh, "test-burst-desc", 8, BurstPattern::DescendingFee);
        assert_eq!(desc_burst.len(), 8);
        assert_eq!(desc_burst[0].priority_fee_micro_lamports, 10_000_000);
        assert_eq!(desc_burst[7].priority_fee_micro_lamports, 100);
    }

    #[test]
    fn test_independent_payer_burst() {
        let payers: Vec<Keypair> = (0..4).map(|_| Keypair::new()).collect();
        let bh = solana_sdk::hash::Hash::default();

        let burst = TxCrafter::build_independent_burst(&payers, bh, "indep-burst", BurstPattern::AscendingFee);
        assert_eq!(burst.len(), 4);
        assert!(burst[0].memo_payload.contains("independent=true"));
    }

    #[test]
    fn test_high_density_burst_generation_16() {
        let payer = Keypair::new();
        let bh = solana_sdk::hash::Hash::default();

        let burst_16 = TxCrafter::build_burst(&payer, bh, "dense-burst-16", 16, BurstPattern::AscendingFee);
        assert_eq!(burst_16.len(), 16);
        assert_eq!(burst_16[0].priority_fee_micro_lamports, 100);
        assert_eq!(burst_16[15].priority_fee_micro_lamports, 10_000_000);
        // Verify strictly monotonic ordering
        for i in 0..15 {
            assert!(
                burst_16[i].priority_fee_micro_lamports < burst_16[i + 1].priority_fee_micro_lamports,
                "Fees must be strictly increasing: {} vs {}",
                burst_16[i].priority_fee_micro_lamports,
                burst_16[i + 1].priority_fee_micro_lamports
            );
        }
    }

    #[test]
    fn test_high_density_burst_generation_32() {
        let payer = Keypair::new();
        let bh = solana_sdk::hash::Hash::default();

        let burst_32 = TxCrafter::build_burst(&payer, bh, "dense-burst-32", 32, BurstPattern::DescendingFee);
        assert_eq!(burst_32.len(), 32);
        assert_eq!(burst_32[0].priority_fee_micro_lamports, 10_000_000);
        assert_eq!(burst_32[31].priority_fee_micro_lamports, 100);
        for i in 0..31 {
            assert!(
                burst_32[i].priority_fee_micro_lamports > burst_32[i + 1].priority_fee_micro_lamports,
                "Fees must be strictly decreasing: {} vs {}",
                burst_32[i].priority_fee_micro_lamports,
                burst_32[i + 1].priority_fee_micro_lamports
            );
        }
    }

    #[test]
    fn test_contention_pair_generation() {
        let master = Keypair::new();
        let bh = solana_sdk::hash::Hash::default();
        let (indep, shared) = TxCrafter::build_contention_pair(&master, bh, "contention-test", 8, BurstPattern::AscendingFee);
        assert_eq!(indep.len(), 8);
        assert_eq!(shared.len(), 8);
        assert!(indep[0].memo_payload.contains("independent=true"));
        assert!(!shared[0].memo_payload.contains("independent=true"));
    }
}
