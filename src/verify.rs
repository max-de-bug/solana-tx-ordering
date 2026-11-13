use crate::campaign::{Campaign, RAW_BLOCKS_DIR};
use crate::rpc::{DetailedBlock, RpcClient};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerificationReport {
    pub generated_at_utc: String,
    pub campaign_id: String,
    pub execution_mode: String,
    pub total_signatures: usize,
    pub unique_signatures: usize,
    pub confirmed: usize,
    pub rpc_errors: usize,
    pub slot_matched: usize,
    pub index_matched: usize,
    pub fee_matched: usize,
    pub memo_present: usize,
    pub target_block_members: usize,
    pub target_blocks_available: usize,
    pub target_blocks_checked: usize,
    pub landed_blocks_available: usize,
    pub landed_blocks_checked: usize,
    pub leaders_checked: usize,
    pub leaders_matched: usize,
    pub tx_errors: usize,
    pub failures: Vec<VerificationFailure>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationFailure {
    pub burst_id: String,
    pub signature: Option<String>,
    pub kind: String,
    pub expected: String,
    pub actual: String,
}

impl VerificationReport {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
            && self.confirmed == self.total_signatures
            && self.index_matched == self.total_signatures
            && self.fee_matched == self.total_signatures
            && self.target_block_members > 0
            && self.tx_errors == 0
    }
}

pub async fn verify_campaign(rpc: &RpcClient, campaign: &Campaign) -> Result<VerificationReport> {
    let mut report = VerificationReport {
        generated_at_utc: chrono::Utc::now().to_rfc3339(),
        campaign_id: campaign.meta.campaign_id.clone(),
        execution_mode: campaign.meta.execution_mode.to_string(),
        ..VerificationReport::default()
    };

    let mut signatures = Vec::new();
    let mut seen = HashSet::new();
    for burst in &campaign.bursts {
        for tx in &burst.txs {
            report.total_signatures += 1;
            if seen.insert(tx.signature.clone()) {
                signatures.push(tx.signature.clone());
            }
        }
    }
    report.unique_signatures = signatures.len();

    let mut status_map: HashMap<String, crate::rpc::SignatureStatus> = HashMap::new();
    let (fetched_statuses, unavailable) = fetch_status_map(rpc, &signatures).await;
    status_map.extend(fetched_statuses);
    report.rpc_errors = unavailable.len();
    if !unavailable.is_empty() {
        report.notes.push(format!(
            "{} signature status lookup(s) failed after retries; these are RPC availability \
             failures, not evidence that the transactions are absent.",
            unavailable.len()
        ));
    }

    let epoch_schedule = rpc.get_epoch_schedule().await?;
    let mut leader_schedule_cache: HashMap<u64, HashMap<String, Vec<usize>>> = HashMap::new();
    let mut block_cache: HashMap<u64, DetailedBlock> = HashMap::new();

    // Fetch every block that contains a recorded probe, plus every target
    // block. This lets us verify index, fee, memo, and target-block membership
    // for all probes rather than only the first target slot.
    let slots_to_fetch: HashSet<u64> = campaign
        .bursts
        .iter()
        .flat_map(|burst| {
            std::iter::once(burst.target_slot).chain(burst.txs.iter().map(|tx| tx.landed_in_slot))
        })
        .collect();
    let mut fetched_slots = HashSet::new();
    for slot in slots_to_fetch.iter().copied().collect::<Vec<_>>() {
        if load_block(campaign, rpc, slot, &mut block_cache)
            .await
            .is_some()
        {
            fetched_slots.insert(slot);
            report.landed_blocks_available += 1;
        }
        report.landed_blocks_checked += 1;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    let target_block_available: HashSet<u64> = campaign
        .bursts
        .iter()
        .filter(|burst| block_cache.contains_key(&burst.target_slot))
        .map(|burst| burst.target_slot)
        .collect();

    for burst in &campaign.bursts {
        report.target_blocks_checked += 1;
        if target_block_available.contains(&burst.target_slot) {
            report.target_blocks_available += 1;
        } else {
            report.failures.push(VerificationFailure {
                burst_id: burst.burst_id.clone(),
                signature: None,
                kind: "target_block_unavailable".to_string(),
                expected: format!("canonical block {}", burst.target_slot),
                actual: "getBlock returned no block (skipped or unavailable)".to_string(),
            });
        }

        let epoch = epoch_for_slot(&epoch_schedule, burst.target_slot);
        if let Some(epoch) = epoch {
            if let std::collections::hash_map::Entry::Vacant(e) = leader_schedule_cache.entry(epoch) {
                if let Ok(schedule) = rpc.get_leader_schedule(Some(burst.target_slot)).await {
                    e.insert(schedule);
                }
            }
            if let Some(schedule) = leader_schedule_cache.get(&epoch) {
                report.leaders_checked += 1;
                let epoch_start = epoch_start_slot(&epoch_schedule, epoch);
                let index = burst.target_slot.saturating_sub(epoch_start) as usize;
                let expected_leader = schedule
                    .get(&burst.target_validator_identity)
                    .map(|slots| slots.binary_search(&index).is_ok())
                    .unwrap_or(false);
                if expected_leader {
                    report.leaders_matched += 1;
                } else {
                    report.failures.push(VerificationFailure {
                        burst_id: burst.burst_id.clone(),
                        signature: None,
                        kind: "leader_mismatch".to_string(),
                        expected: burst.target_validator_identity.clone(),
                        actual: format!(
                            "target slot {} is not in the validator's leader schedule (epoch index {})",
                            burst.target_slot, index
                        ),
                    });
                }
            }
        }

        for tx in &burst.txs {
            let status = status_map.get(&tx.signature);
            let Some(status) = status else {
                let kind = if unavailable.contains(&tx.signature) {
                    "rpc_unavailable"
                } else {
                    "not_confirmed"
                };
                report.failures.push(VerificationFailure {
                    burst_id: burst.burst_id.clone(),
                    signature: Some(tx.signature.clone()),
                    kind: kind.to_string(),
                    expected: "confirmed signature status".to_string(),
                    actual: if kind == "rpc_unavailable" {
                        "status lookup failed after retries".to_string()
                    } else {
                        "null status".to_string()
                    },
                });
                continue;
            };
            report.confirmed += 1;
            if status.err.is_some() {
                report.tx_errors += 1;
            }
            if status.slot == tx.landed_in_slot {
                report.slot_matched += 1;
            } else {
                report.failures.push(VerificationFailure {
                    burst_id: burst.burst_id.clone(),
                    signature: Some(tx.signature.clone()),
                    kind: "slot_mismatch".to_string(),
                    expected: tx.landed_in_slot.to_string(),
                    actual: status.slot.to_string(),
                });
            }

            if tx.landed_in_slot == burst.target_slot {
                report.target_block_members += 1;
            }

            let landed_block = block_cache.get(&tx.landed_in_slot);
            let block_tx = landed_block.and_then(|block| {
                block
                    .transactions
                    .iter()
                    .find(|candidate| candidate.signature == tx.signature)
            });

            match (landed_block, block_tx) {
                (None, _) => {
                    report.failures.push(VerificationFailure {
                        burst_id: burst.burst_id.clone(),
                        signature: Some(tx.signature.clone()),
                        kind: "block_unavailable".to_string(),
                        expected: format!("canonical block {}", tx.landed_in_slot),
                        actual: "block not fetched".to_string(),
                    });
                }
                (Some(_), None) => {
                    report.failures.push(VerificationFailure {
                        burst_id: burst.burst_id.clone(),
                        signature: Some(tx.signature.clone()),
                        kind: "missing_from_block".to_string(),
                        expected: format!("present in block {}", tx.landed_in_slot),
                        actual: "not found".to_string(),
                    });
                }
                (Some(_), Some(block_tx)) => {
                    if block_tx.index_in_block == tx.index_in_block {
                        report.index_matched += 1;
                    } else {
                        report.failures.push(VerificationFailure {
                            burst_id: burst.burst_id.clone(),
                            signature: Some(tx.signature.clone()),
                            kind: "index_mismatch".to_string(),
                            expected: tx.index_in_block.to_string(),
                            actual: block_tx.index_in_block.to_string(),
                        });
                    }
                    if block_tx.fee == tx.fee_paid_lamports {
                        report.fee_matched += 1;
                    } else {
                        report.failures.push(VerificationFailure {
                            burst_id: burst.burst_id.clone(),
                            signature: Some(tx.signature.clone()),
                            kind: "fee_mismatch".to_string(),
                            expected: tx.fee_paid_lamports.to_string(),
                            actual: block_tx.fee.to_string(),
                        });
                    }
                    if block_tx
                        .memo
                        .as_deref()
                        .map(|memo| memo.contains(&burst.burst_id))
                        .unwrap_or(false)
                    {
                        report.memo_present += 1;
                    }
                }
            }
        }
    }
    report.notes.push(format!(
        "Fetched {}/{} unique blocks covering target slots and probe landing slots.",
        fetched_slots.len(),
        slots_to_fetch.len()
    ));

    if report.leaders_checked == 0 {
        report.notes.push(
            "Leader schedule could not be checked for target slots; this does not invalidate the \
             same-block ordering statistics, but the provenance claim is incomplete."
                .to_string(),
        );
    }
    if report.tx_errors > 0 {
        report.notes.push(format!(
            "{} of {} recorded transactions landed but failed at runtime. Their signatures, slots, \
             block indexes, and charged fees still match the chain, so the ordering sample is \
             structurally verifiable; however, the transaction instructions did not execute \
             successfully, and the report must not describe them as successful probes.",
            report.tx_errors, report.total_signatures
        ));
    }
    if report.target_block_members == 0 {
        report.notes.push(
            "No recorded transaction landed in its burst's target block. The legacy campaign has \
             zero target-block probes, so it cannot support a same-block ordering claim even though \
             every recorded transaction is confirmed somewhere on chain."
                .to_string(),
        );
    }
    Ok(report)
}

async fn fetch_status_map(
    rpc: &RpcClient,
    signatures: &[String],
) -> (
    HashMap<String, crate::rpc::SignatureStatus>,
    HashSet<String>,
) {
    let mut map = HashMap::new();
    let mut unavailable = HashSet::new();
    for (chunk_idx, chunk) in signatures.chunks(10).enumerate() {
        let mut batch_ok = false;
        let mut backoff = std::time::Duration::from_secs(1);
        for attempt in 1..=5 {
            match rpc.get_signature_statuses(chunk).await {
                Ok(statuses) => {
                    for (sig, status) in chunk.iter().zip(statuses.into_iter()) {
                        if let Some(status) = status {
                            map.insert(sig.clone(), status);
                        }
                    }
                    batch_ok = true;
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        "signature status batch {} attempt {} failed: {e:#}",
                        chunk_idx,
                        attempt
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(std::time::Duration::from_secs(16));
                }
            }
        }

        if !batch_ok {
            // A batch-level failure is not evidence that signatures are absent.
            // Retry each signature individually before marking it unavailable.
            for sig in chunk {
                match rpc.get_signature_status(sig).await {
                    Ok(Some(status)) => {
                        map.insert(sig.clone(), status);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("signature status for {sig} failed: {e:#}");
                        unavailable.insert(sig.clone());
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }
        } else {
            // A few endpoints occasionally truncate a batch. Re-check missing
            // entries individually so a transport quirk is not reported as a
            // missing transaction.
            for sig in chunk {
                if !map.contains_key(sig) {
                    match rpc.get_signature_status(sig).await {
                        Ok(Some(status)) => {
                            map.insert(sig.clone(), status);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!("signature status for {sig} failed: {e:#}");
                            unavailable.insert(sig.clone());
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
        tracing::info!(
            "signature statuses: batch {}/{} complete, {}/{} resolved",
            chunk_idx + 1,
            signatures.len().div_ceil(10),
            map.len(),
            signatures.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    (map, unavailable)
}

async fn load_block(
    campaign: &Campaign,
    rpc: &RpcClient,
    slot: u64,
    cache: &mut HashMap<u64, DetailedBlock>,
) -> Option<DetailedBlock> {
    if let Some(block) = cache.get(&slot) {
        return Some(block.clone());
    }
    if let Some(dir) = &campaign.dir {
        let path = dir.join(RAW_BLOCKS_DIR).join(format!("{slot}.json"));
        if path.exists() {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(block) = serde_json::from_str::<DetailedBlock>(&raw) {
                    cache.insert(slot, block.clone());
                    return Some(block);
                }
            }
        }
    }
    let mut last_error = None;
    for attempt in 0..4 {
        match rpc.get_block(slot).await {
            Ok(block) => {
                cache.insert(slot, block.clone());
                return Some(block);
            }
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            }
        }
    }
    if let Some(error) = last_error {
        tracing::warn!("block {slot} unavailable: {error:#}");
    }
    None
}

fn epoch_for_slot(schedule: &crate::rpc::EpochSchedule, slot: u64) -> Option<u64> {
    if schedule.slots_per_epoch == 0 || slot < schedule.first_normal_slot {
        return None;
    }
    Some(
        schedule.first_normal_epoch
            + (slot - schedule.first_normal_slot) / schedule.slots_per_epoch,
    )
}

fn epoch_start_slot(schedule: &crate::rpc::EpochSchedule, epoch: u64) -> u64 {
    if epoch < schedule.first_normal_epoch {
        return 0;
    }
    schedule.first_normal_slot + (epoch - schedule.first_normal_epoch) * schedule.slots_per_epoch
}

pub fn write_verification(report: &VerificationReport, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create verification directory {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_string_pretty(report)?)
        .with_context(|| format!("write verification report {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_math_for_normal_epoch() {
        let schedule = crate::rpc::EpochSchedule {
            slots_per_epoch: 432_000,
            leader_schedule_slot_offset: 0,
            warmup: false,
            first_normal_epoch: 14,
            first_normal_slot: 5_242_880,
        };
        let slot = 5_242_880 + 432_000 + 5;
        assert_eq!(epoch_for_slot(&schedule, slot), Some(15));
        assert_eq!(epoch_start_slot(&schedule, 15), 5_242_880 + 432_000);
    }
}
