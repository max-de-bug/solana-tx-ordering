use crate::rpc::RpcClient;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClientType {
    Agave,
    JitoSolana,
    Firedancer,
    AgaveBam,
    SolanaLabsLegacy,
    UnknownFork,
}

impl fmt::Display for ClientType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientType::Agave => write!(f, "Agave (Anza)"),
            ClientType::JitoSolana => write!(f, "Jito-Solana"),
            ClientType::Firedancer => write!(f, "Firedancer (Jump Crypto)"),
            ClientType::AgaveBam => write!(f, "Agave-BAM"),
            ClientType::SolanaLabsLegacy => write!(f, "Solana Labs (Legacy)"),
            ClientType::UnknownFork => write!(f, "Unknown/Unclassified fork"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetValidator {
    pub identity: String,
    pub name: String,
    pub client_type: ClientType,
    pub version_string: String,
    pub stake_sol: f64,
    /// The direct TPU QUIC address (`tpuQuic` from getClusterNodes), not UDP.
    pub tpu_ip: Option<String>,
}

impl TargetValidator {
    pub fn classify_client(client_id: Option<&str>, version_str: &str) -> ClientType {
        if let Some(cid) = client_id {
            let cid_lower = cid.to_lowercase();
            if cid_lower.contains("jito") {
                return ClientType::JitoSolana;
            } else if cid_lower.contains("firedancer") || cid_lower.contains("frankendancer") {
                return ClientType::Firedancer;
            } else if cid_lower.contains("bam") {
                return ClientType::AgaveBam;
            } else if cid_lower.contains("agave") {
                return ClientType::Agave;
            }
        }

        let v_lower = version_str.to_lowercase();
        if v_lower.contains("jito") {
            ClientType::JitoSolana
        } else if v_lower.contains("firedancer")
            || v_lower.contains("frankendancer")
            || version_str.starts_with("0.")
            || version_str.starts_with("26.")
        {
            ClientType::Firedancer
        } else if version_str.starts_with("4.") || version_str.starts_with("2.") {
            ClientType::Agave
        } else if version_str.starts_with("1.18") {
            ClientType::SolanaLabsLegacy
        } else {
            ClientType::UnknownFork
        }
    }

    pub fn client_class_key(&self) -> String {
        format!("{}|{}", self.client_type, self.version_string)
    }

    #[allow(dead_code)]
    pub fn is_valid_identity(&self) -> bool {
        Pubkey::from_str(&self.identity)
            .map(|_| true)
            .unwrap_or(false)
    }
}

/// Deterministic, clearly synthetic validators for dry-run pipeline tests.
/// These are not real mainnet identities and must not appear in an empirical
/// report as measurements of real operators.
pub fn synthetic_fixture_validators() -> Vec<TargetValidator> {
    let classes = [
        ClientType::Agave,
        ClientType::JitoSolana,
        ClientType::Firedancer,
        ClientType::AgaveBam,
    ];
    classes
        .iter()
        .enumerate()
        .flat_map(|(class_idx, class)| {
            (0..2).map(move |i| {
                let mut bytes = [0u8; 32];
                for (j, byte) in bytes.iter_mut().enumerate() {
                    *byte = (class_idx * 31 + i * 17 + j * 7) as u8;
                }
                let identity = bs58::encode(bytes).into_string();
                TargetValidator {
                    identity,
                    name: format!("Synthetic {} fixture {}", class, i + 1),
                    client_type: *class,
                    version_string: format!("synthetic-{}", class),
                    stake_sol: 0.0,
                    tpu_ip: None,
                }
            })
        })
        .collect()
}

/// Discovers validators from `getClusterNodes` and `getVoteAccounts`.
///
/// This function never invents validator identities. If the RPC is blocked or
/// a requested identity is missing, it returns an error so a live campaign
/// cannot silently fall back to a fake roster.
pub async fn discover_cluster_validators(
    rpc: &RpcClient,
    target_identities: &[String],
    max_validators: usize,
    require_tpu_quic: bool,
) -> anyhow::Result<Vec<TargetValidator>> {
    info!("Discovering validators from getClusterNodes and getVoteAccounts...");
    let nodes = rpc
        .get_cluster_nodes()
        .await
        .map_err(|e| anyhow::anyhow!("getClusterNodes failed: {e:#}"))?;
    if nodes.is_empty() {
        anyhow::bail!(
            "getClusterNodes returned no nodes; refusing to fabricate a validator roster"
        );
    }
    let votes = rpc
        .get_vote_accounts()
        .await
        .map_err(|e| anyhow::anyhow!("getVoteAccounts failed: {e:#}"))?;

    let mut stake_map = HashMap::new();
    for vote in votes.current {
        stake_map.insert(
            vote.node_pubkey,
            vote.activated_stake as f64 / 1_000_000_000.0,
        );
    }

    let mut node_map: HashMap<String, &crate::rpc::ClusterNodeInfo> = HashMap::new();
    for node in &nodes {
        if Pubkey::from_str(&node.pubkey).is_ok() {
            node_map.entry(node.pubkey.clone()).or_insert(node);
        }
    }

    if !target_identities.is_empty() {
        let mut result = Vec::new();
        let mut missing = Vec::new();
        for identity in target_identities {
            let Some(node) = node_map.get(identity).copied() else {
                missing.push(identity.clone());
                continue;
            };
            let version = node
                .version
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let client_type = TargetValidator::classify_client(node.client_id.as_deref(), &version);
            let stake = stake_map.get(identity).copied().unwrap_or(0.0);
            let tpu_quic = node.tpu_quic.clone();
            if require_tpu_quic && tpu_quic.is_none() {
                warn!("requested validator {identity} has no tpuQuic endpoint; skipping");
                continue;
            }
            result.push(TargetValidator {
                identity: identity.clone(),
                name: format!("Validator {}", &identity[..identity.len().min(8)]),
                client_type,
                version_string: version,
                stake_sol: stake,
                tpu_ip: tpu_quic,
            });
        }
        if !missing.is_empty() {
            anyhow::bail!(
                "requested validator identities were not found in getClusterNodes: {}",
                missing.join(", ")
            );
        }
        if result.len() < target_identities.len() {
            anyhow::bail!(
                "only {} of {} requested validators have usable endpoints",
                result.len(),
                target_identities.len()
            );
        }
        return Ok(result);
    }

    let mut candidates: Vec<(TargetValidator, f64)> = nodes
        .iter()
        .filter_map(|node| {
            let stake = stake_map.get(&node.pubkey).copied().unwrap_or(0.0);
            if stake < 1_000.0 {
                return None;
            }
            let version = node.version.clone()?;
            let client_type = TargetValidator::classify_client(node.client_id.as_deref(), &version);
            let tpu_quic = node.tpu_quic.clone();
            if require_tpu_quic && tpu_quic.is_none() {
                return None;
            }
            Some((
                TargetValidator {
                    identity: node.pubkey.clone(),
                    name: format!("Validator {}", &node.pubkey[..node.pubkey.len().min(8)]),
                    client_type,
                    version_string: version,
                    stake_sol: stake,
                    tpu_ip: tpu_quic,
                },
                stake,
            ))
        })
        .collect();

    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Round-robin by (client class, version) so the campaign spans forks rather
    // than taking 14 Agave nodes. Within a class, keep the highest stake.
    let mut groups: BTreeMap<String, Vec<TargetValidator>> = BTreeMap::new();
    for (validator, _) in candidates {
        groups
            .entry(validator.client_class_key())
            .or_default()
            .push(validator);
    }
    let mut group_names: Vec<String> = groups.keys().cloned().collect();
    // Prefer major known clients first, but retain all versions for provenance.
    group_names.sort_by_key(|name| {
        if name.contains("Agave (Anza)") {
            0
        } else if name.contains("Jito") {
            1
        } else if name.contains("Firedancer") {
            2
        } else if name.contains("BAM") {
            3
        } else {
            4
        }
    });

    let mut result = Vec::new();
    let mut round = 0usize;
    while result.len() < max_validators {
        let mut added = false;
        for name in &group_names {
            if let Some(group) = groups.get(name) {
                if let Some(validator) = group.get(round) {
                    if !result
                        .iter()
                        .any(|v: &TargetValidator| v.identity == validator.identity)
                    {
                        result.push(validator.clone());
                        added = true;
                    }
                }
            }
            if result.len() >= max_validators {
                break;
            }
        }
        if !added {
            break;
        }
        round += 1;
    }

    if result.len() < 6 {
        anyhow::bail!(
            "discovered only {} eligible validators; refusing to fabricate identities. \
             Check the RPC endpoint or pass explicit --target-validators.",
            result.len()
        );
    }

    let classes: HashSet<String> = result.iter().map(|v| v.client_class_key()).collect();
    info!(
        "Discovered {} validators across {} client/version classes",
        result.len(),
        classes.len()
    );
    Ok(result)
}

pub fn distinct_client_classes(validators: &[TargetValidator]) -> usize {
    validators
        .iter()
        .map(|v| v.client_class_key())
        .collect::<HashSet<_>>()
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_clients() {
        assert_eq!(
            TargetValidator::classify_client(Some("jito"), "4.4.0"),
            ClientType::JitoSolana
        );
        assert_eq!(
            TargetValidator::classify_client(None, "0.1.0"),
            ClientType::Firedancer
        );
        assert_eq!(
            TargetValidator::classify_client(None, "4.4.0"),
            ClientType::Agave
        );
        assert_eq!(
            TargetValidator::classify_client(Some("bam"), "4.4.0"),
            ClientType::AgaveBam
        );
    }

    #[test]
    fn synthetic_fixtures_are_valid_and_not_real_claims() {
        let fixtures = synthetic_fixture_validators();
        assert_eq!(fixtures.len(), 8);
        for validator in &fixtures {
            assert!(validator.is_valid_identity());
            assert!(validator.name.contains("Synthetic"));
            assert_eq!(validator.stake_sol, 0.0);
            assert!(validator.tpu_ip.is_none());
        }
    }

    #[test]
    fn distinct_classes_counts_version_strings() {
        let fixtures = synthetic_fixture_validators();
        assert_eq!(distinct_client_classes(&fixtures), 4);
    }
}
