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

        // Standard priority fee tiers (micro-lamports per Compute Unit)
        // 1 micro-lamport = 0.000001 lamports. At 300 CU, 10,000 uL/CU = 3 lamports extra fee.
        let fee_tiers = match pattern {
            BurstPattern::AscendingFee => {
                vec![
                    100,      // 0.1 uL/CU
                    1_000,    // 1 uL/CU
                    5_000,    // 5 uL/CU
                    20_000,   // 20 uL/CU
                    100_000,  // 100 uL/CU
                    500_000,  // 500 uL/CU
                    2_000_000,// 2,000 uL/CU
                    10_000_000,// 10,000 uL/CU
                ]
            }
            BurstPattern::DescendingFee => {
                vec![
                    10_000_000,
                    2_000_000,
                    500_000,
                    100_000,
                    20_000,
                    5_000,
                    1_000,
                    100,
                ]
            }
            BurstPattern::AlternatingFee => {
                vec![
                    100, 5_000_000, 200, 5_000_000, 300, 5_000_000, 400, 5_000_000,
                ]
            }
            BurstPattern::UniformFee => {
                vec![10_000; burst_size]
            }
        };

        for i in 0..burst_size {
            let priority_fee = fee_tiers[i % fee_tiers.len()];
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
    #[allow(dead_code)]
    pub fn build_independent_burst(
        payers: &[Keypair],
        recent_blockhash: solana_sdk::hash::Hash,
        burst_id: &str,
        pattern: BurstPattern,
    ) -> Vec<CraftedTxMetadata> {
        let memo_program_pubkey = Pubkey::from_str(MEMO_PROGRAM_ID).unwrap();
        let burst_size = payers.len();
        let mut burst = Vec::with_capacity(burst_size);

        let fee_tiers = match pattern {
            BurstPattern::AscendingFee => vec![100, 1_000, 5_000, 20_000, 100_000, 500_000, 2_000_000, 10_000_000],
            BurstPattern::DescendingFee => vec![10_000_000, 2_000_000, 500_000, 100_000, 20_000, 5_000, 1_000, 100],
            BurstPattern::AlternatingFee => vec![100, 5_000_000, 200, 5_000_000, 300, 5_000_000, 400, 5_000_000],
            BurstPattern::UniformFee => vec![10_000; burst_size],
        };

        for (i, payer) in payers.iter().enumerate() {
            let priority_fee = fee_tiers[i % fee_tiers.len()];
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
}
