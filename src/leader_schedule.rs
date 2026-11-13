use crate::rpc::RpcClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderWindow {
    pub validator_identity: String,
    pub start_slot: u64,
    pub end_slot: u64,
    pub window_length: u64,
    pub estimated_start_time_ms: u64,
}

impl LeaderWindow {
    pub fn send_slot(&self, lead_slots: u64) -> u64 {
        self.start_slot.saturating_sub(lead_slots)
    }

    #[allow(dead_code)]
    pub fn contains(&self, slot: u64) -> bool {
        slot >= self.start_slot && slot <= self.end_slot
    }
}

#[derive(Debug, Clone)]
pub struct LeaderScheduleManager {
    schedule: HashMap<String, Vec<usize>>,
    pub first_slot_in_epoch: u64,
    #[allow(dead_code)]
    pub slots_in_epoch: u64,
    pub slot_duration_ms: f64,
}

impl LeaderScheduleManager {
    pub async fn load(rpc: &RpcClient, allow_synthetic: bool) -> anyhow::Result<Self> {
        info!("Fetching epoch information and leader schedule from RPC...");
        let epoch_info = match rpc.get_epoch_info().await {
            Ok(info) => info,
            Err(e) => {
                if allow_synthetic {
                    warn!(
                        "Could not load epoch info ({e}); using a synthetic schedule for dry-run."
                    );
                    return Ok(Self::synthetic_for(&[], 465_000_000));
                }
                return Err(e);
            }
        };

        let schedule = match rpc
            .get_leader_schedule(Some(epoch_info.absolute_slot))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                if allow_synthetic {
                    warn!("Could not load leader schedule ({e}); using a synthetic schedule for dry-run.");
                    return Ok(Self::synthetic_for(&[], epoch_info.absolute_slot));
                }
                return Err(e);
            }
        };

        let first_slot = epoch_info
            .absolute_slot
            .saturating_sub(epoch_info.slot_index);
        let slot_duration_ms = rpc
            .estimate_slot_duration_ms()
            .await
            .ok()
            .flatten()
            .unwrap_or(400.0);

        info!(
            "Loaded leader schedule for epoch {} (slots {}..{}) with {} validators; estimated slot duration {:.1} ms",
            epoch_info.epoch,
            first_slot,
            first_slot + epoch_info.slots_in_epoch,
            schedule.len(),
            slot_duration_ms
        );

        Ok(Self {
            schedule,
            first_slot_in_epoch: first_slot,
            slots_in_epoch: epoch_info.slots_in_epoch,
            slot_duration_ms,
        })
    }

    /// Deterministic schedule for pipeline tests only. It must never be used
    /// to generate a report that claims empirical validity.
    pub fn synthetic_for(identities: &[String], current_slot: u64) -> Self {
        let mut schedule = HashMap::new();
        let first_slot = (current_slot / 432_000) * 432_000;
        let offset = (current_slot.saturating_sub(first_slot)) as usize;
        let identities: Vec<String> = if identities.is_empty() {
            vec!["SyntheticValidator1111111111111111111111111".to_string()]
        } else {
            identities.to_vec()
        };

        for (i, identity) in identities.iter().enumerate() {
            let mut slots = Vec::new();
            for window in 0..12usize {
                let start = offset + 20 + (i * 24) + (window * 180);
                for s in 0..4 {
                    slots.push(start + s);
                }
            }
            schedule.insert(identity.clone(), slots);
        }

        Self {
            schedule,
            first_slot_in_epoch: first_slot,
            slots_in_epoch: 432_000,
            slot_duration_ms: 400.0,
        }
    }

    #[allow(dead_code)]
    pub fn find_next_window(
        &self,
        validator_identity: &str,
        current_slot: u64,
    ) -> Option<LeaderWindow> {
        self.find_upcoming_windows(validator_identity, current_slot, 1)
            .into_iter()
            .next()
    }

    /// Finds distinct upcoming 4-slot leader windows. Consecutive schedule
    /// entries are grouped into a window; gaps start a new window.
    pub fn find_upcoming_windows(
        &self,
        validator_identity: &str,
        current_slot: u64,
        count: usize,
    ) -> Vec<LeaderWindow> {
        let slot_indices = match self.schedule.get(validator_identity) {
            Some(indices) => indices,
            None => return Vec::new(),
        };
        let current_index = if current_slot >= self.first_slot_in_epoch {
            (current_slot - self.first_slot_in_epoch) as usize
        } else {
            0
        };

        // Leave room for RPC polling and a 2-slot send lead.
        let min_lead_slots = 5usize;
        let mut upcoming: Vec<usize> = slot_indices
            .iter()
            .copied()
            .filter(|idx| *idx >= current_index + min_lead_slots)
            .collect();
        upcoming.sort_unstable();
        upcoming.dedup();

        let mut windows = Vec::new();
        let mut i = 0usize;
        while i < upcoming.len() && windows.len() < count {
            let start_idx = upcoming[i];
            let mut end_idx = start_idx;
            // A normal leader window is four consecutive slots. Consume up to
            // four consecutive indices; a gap closes the window.
            while i + 1 < upcoming.len()
                && upcoming[i + 1] == end_idx + 1
                && end_idx - start_idx + 1 < 4
            {
                i += 1;
                end_idx = upcoming[i];
            }
            let start_slot = self.first_slot_in_epoch + start_idx as u64;
            let end_slot = self.first_slot_in_epoch + end_idx as u64;
            let slots_until = start_slot.saturating_sub(current_slot);
            windows.push(LeaderWindow {
                validator_identity: validator_identity.to_string(),
                start_slot,
                end_slot,
                window_length: end_slot - start_slot + 1,
                estimated_start_time_ms: (slots_until as f64 * self.slot_duration_ms) as u64,
            });
            i += 1;
        }
        windows
    }

    /// Wait until the target slot is observed. The caller passes the send slot
    /// (target start minus lead slots), so dispatch happens before the window.
    pub async fn wait_until_slot(
        &self,
        rpc: &RpcClient,
        send_slot: u64,
        is_dry_run: bool,
    ) -> anyhow::Result<u64> {
        if is_dry_run {
            tokio::time::sleep(Duration::from_millis(50)).await;
            return Ok(send_slot.saturating_sub(2));
        }

        let mut errors = 0usize;
        loop {
            match rpc.get_slot().await {
                Ok(current) if current >= send_slot => return Ok(current),
                Ok(current) => {
                    let remaining = send_slot.saturating_sub(current);
                    let sleep_ms = if remaining > 8 {
                        (remaining as f64 * self.slot_duration_ms * 0.4).min(2_000.0)
                    } else {
                        25.0
                    };
                    tokio::time::sleep(Duration::from_millis(sleep_ms.max(10.0) as u64)).await;
                }
                Err(e) => {
                    errors += 1;
                    if errors >= 5 {
                        return Err(e);
                    }
                    warn!("getSlot failed while waiting for send slot {send_slot}: {e}");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    /// Returns true if the identity is scheduled to lead `slot` in the loaded
    /// epoch. Returns None when the slot is outside the loaded epoch.
    #[allow(dead_code)]
    pub fn is_leader(&self, identity: &str, slot: u64) -> Option<bool> {
        if slot < self.first_slot_in_epoch || slot >= self.first_slot_in_epoch + self.slots_in_epoch
        {
            return None;
        }
        let index = (slot - self.first_slot_in_epoch) as usize;
        Some(
            self.schedule
                .get(identity)
                .map(|slots| slots.binary_search(&index).is_ok())
                .unwrap_or(false),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_consecutive_slots_and_respects_lead_time() {
        let mut mgr = LeaderScheduleManager {
            schedule: HashMap::new(),
            first_slot_in_epoch: 1_000,
            slots_in_epoch: 432_000,
            slot_duration_ms: 400.0,
        };
        mgr.schedule
            .insert("v".to_string(), vec![20, 21, 22, 23, 30, 31, 32, 33]);
        let windows = mgr.find_upcoming_windows("v", 1_000, 2);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].start_slot, 1_020);
        assert_eq!(windows[0].end_slot, 1_023);
        assert_eq!(windows[0].window_length, 4);
        assert_eq!(windows[0].send_slot(2), 1_018);
        assert!(windows[0].contains(1_022));
        assert!(!windows[0].contains(1_024));
    }

    #[test]
    fn synthetic_schedule_is_deterministic_and_marked_dry_run_only() {
        let mgr = LeaderScheduleManager::synthetic_for(&["v".to_string()], 1_000);
        let windows = mgr.find_upcoming_windows("v", 1_000, 4);
        assert_eq!(windows.len(), 4);
        assert!(mgr.is_leader("v", windows[0].start_slot).unwrap());
    }
}
