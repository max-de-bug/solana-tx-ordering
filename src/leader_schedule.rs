use crate::rpc::RpcClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderWindow {
    pub validator_identity: String,
    pub start_slot: u64,
    pub end_slot: u64, // 4 consecutive slots per leader window
    pub window_length: u64,
    pub estimated_start_time_ms: u64,
}

#[derive(Debug, Clone)]
pub struct LeaderScheduleManager {
    schedule: HashMap<String, Vec<usize>>,
    pub first_slot_in_epoch: u64,
}

impl LeaderScheduleManager {
    pub async fn load(rpc: &RpcClient) -> anyhow::Result<Self> {
        info!("Fetching epoch information and leader schedule from RPC...");
        let epoch_info = match rpc.get_epoch_info().await {
            Ok(info) => info,
            Err(e) => {
                info!("Could not load live epoch info ({}). Using synthetic fallback schedule.", e);
                return Ok(Self::create_synthetic(465000000, 1076));
            }
        };

        let schedule = match rpc.get_leader_schedule(Some(epoch_info.absolute_slot)).await {
            Ok(s) => s,
            Err(e) => {
                info!("Could not load leader schedule ({}). Using synthetic fallback schedule.", e);
                return Ok(Self::create_synthetic(epoch_info.absolute_slot, epoch_info.epoch));
            }
        };

        let first_slot = epoch_info.absolute_slot - epoch_info.slot_index;

        info!(
            "Loaded leader schedule for epoch {} (slots {}..{}) with {} validators",
            epoch_info.epoch,
            first_slot,
            first_slot + epoch_info.slots_in_epoch,
            schedule.len()
        );

        Ok(Self {
            schedule,
            first_slot_in_epoch: first_slot,
        })
    }

    /// Creates a deterministic synthetic leader schedule for dry-run / fallback
    pub fn create_synthetic(current_slot: u64, _epoch: u64) -> Self {
        let mut schedule = HashMap::new();
        let curated = crate::validator_info::get_curated_mainnet_validators();
        let first_slot = (current_slot / 432_000) * 432_000;
        let offset = (current_slot - first_slot) as usize;

        for (i, v) in curated.iter().enumerate() {
            let mut slots = Vec::new();
            for s in 0..10 {
                let slot_idx = offset + 20 + (i * 20) + (s * 150);
                slots.push(slot_idx);
                slots.push(slot_idx + 1);
                slots.push(slot_idx + 2);
                slots.push(slot_idx + 3);
            }
            schedule.insert(v.identity.clone(), slots);
        }

        Self {
            schedule,
            first_slot_in_epoch: first_slot,
        }
    }

    /// Finds the next upcoming leader window for a target validator identity
    pub fn find_next_window(
        &self,
        validator_identity: &str,
        current_slot: u64,
    ) -> Option<LeaderWindow> {
        let slot_indices = self.schedule.get(validator_identity)?;
        let current_index = if current_slot >= self.first_slot_in_epoch {
            (current_slot - self.first_slot_in_epoch) as usize
        } else {
            0
        };

        let min_lead_slots = 10;
        let mut upcoming_slots = Vec::new();

        for &idx in slot_indices {
            if idx >= current_index + min_lead_slots {
                upcoming_slots.push(idx);
            }
        }

        upcoming_slots.sort_unstable();

        if let Some(&first_idx) = upcoming_slots.first() {
            let start_slot = self.first_slot_in_epoch + first_idx as u64;
            let slots_until = start_slot.saturating_sub(current_slot);
            let est_time_ms = slots_until * 400;

            return Some(LeaderWindow {
                validator_identity: validator_identity.to_string(),
                start_slot,
                end_slot: start_slot + 3,
                window_length: 4,
                estimated_start_time_ms: est_time_ms,
            });
        }

        None
    }

    /// Wait and synchronize precisely with the leader window onset
    pub async fn wait_for_window(
        &self,
        rpc: &RpcClient,
        window: &LeaderWindow,
        is_dry_run: bool,
    ) -> anyhow::Result<()> {
        info!(
            "Awaiting leader window for {} at slot {} (approx {} ms)...",
            &window.validator_identity[..8.min(window.validator_identity.len())],
            window.start_slot,
            window.estimated_start_time_ms
        );

        if is_dry_run {
            // In dry run, simulate quick countdown without blocking for minutes
            info!("Dry-run mode: simulating window alignment in 1.5s...");
            tokio::time::sleep(Duration::from_millis(1500)).await;
            return Ok(());
        }

        loop {
            let current_slot = rpc.get_slot().await?;
            if current_slot >= window.start_slot {
                info!(
                    "Leader window reached! Current slot: {}, Target: {}",
                    current_slot, window.start_slot
                );
                return Ok(());
            }

            let slots_remaining = window.start_slot - current_slot;
            if slots_remaining > 5 {
                let sleep_ms = (slots_remaining - 4) * 400;
                tokio::time::sleep(Duration::from_millis(sleep_ms.min(3000))).await;
            } else {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
    }
}
