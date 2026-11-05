use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct RpcClient {
    endpoint: String,
    http: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNodeInfo {
    pub pubkey: String,
    pub gossip: Option<String>,
    pub tpu: Option<String>,
    #[serde(rename = "tpuQuic", default)]
    pub tpu_quic: Option<String>,
    #[serde(rename = "tpuForwardsQuic", default)]
    pub tpu_forwards_quic: Option<String>,
    pub rpc: Option<String>,
    pub version: Option<String>,
    #[serde(rename = "clientId", default)]
    pub client_id: Option<String>,
    pub feature_set: Option<u32>,
    pub shred_version: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteAccountInfo {
    #[serde(rename = "votePubkey")]
    pub vote_pubkey: String,
    #[serde(rename = "nodePubkey")]
    pub node_pubkey: String,
    #[serde(rename = "activatedStake")]
    pub activated_stake: u64,
    pub epoch_vote_account: Option<bool>,
    pub commission: Option<u8>,
    pub last_vote: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteAccountsResponse {
    pub current: Vec<VoteAccountInfo>,
    pub delinquent: Vec<VoteAccountInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpochInfo {
    pub absolute_slot: u64,
    pub block_height: u64,
    pub epoch: u64,
    pub slot_index: u64,
    pub slots_in_epoch: u64,
    pub transaction_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestBlockhash {
    pub blockhash: String,
    pub last_valid_block_height: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockTransaction {
    pub signature: String,
    pub index_in_block: usize,
    pub fee: u64,
    pub compute_units_consumed: Option<u64>,
    pub err: Option<Value>,
    pub memo: Option<String>,
    pub priority_fee_micro_lamports: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailedBlock {
    pub slot: u64,
    pub blockhash: String,
    pub previous_blockhash: String,
    pub parent_slot: u64,
    pub block_time: Option<i64>,
    pub total_transactions: usize,
    pub transactions: Vec<BlockTransaction>,
}

impl RpcClient {
    pub fn new(endpoint: &str) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            endpoint: endpoint.to_string(),
            http,
        }
    }

    #[allow(dead_code)]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn rpc_call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });

        let mut backoff = Duration::from_millis(500);
        for attempt in 1..=5 {
            let res = self.http.post(&self.endpoint).json(&payload).send().await;

            match res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let json_val: Value = resp.json().await?;
                        if let Some(err) = json_val.get("error") {
                            anyhow::bail!("RPC error for {}: {}", method, err);
                        }
                        if let Some(result) = json_val.get("result") {
                            return Ok(result.clone());
                        }
                        anyhow::bail!("Missing result in RPC response: {:?}", json_val);
                    } else if status.as_u16() == 429 {
                        warn!(
                            "RPC rate-limited (429) on attempt {}. Backing off {:?}...",
                            attempt, backoff
                        );
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                    } else {
                        anyhow::bail!("HTTP status error {}: {}", status, method);
                    }
                }
                Err(e) => {
                    if attempt == 5 {
                        anyhow::bail!("Failed RPC call {} after 5 attempts: {}", method, e);
                    }
                    warn!("RPC connection error on attempt {}: {}. Retrying...", attempt, e);
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                }
            }
        }

        anyhow::bail!("Exhausted retries for RPC call: {}", method)
    }

    pub async fn get_slot(&self) -> anyhow::Result<u64> {
        let res = self.rpc_call("getSlot", json!([{"commitment": "processed"}])).await?;
        res.as_u64().ok_or_else(|| anyhow::anyhow!("Invalid slot returned"))
    }

    pub async fn get_balance(&self, pubkey: &str) -> anyhow::Result<u64> {
        let res = self.rpc_call("getBalance", json!([pubkey, {"commitment": "processed"}])).await?;
        let value = res.get("value").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(value)
    }

    pub async fn get_epoch_info(&self) -> anyhow::Result<EpochInfo> {
        let res = self.rpc_call("getEpochInfo", json!([{"commitment": "processed"}])).await?;
        let info: EpochInfo = serde_json::from_value(res)?;
        Ok(info)
    }

    pub async fn get_cluster_nodes(&self) -> anyhow::Result<Vec<ClusterNodeInfo>> {
        let res = self.rpc_call("getClusterNodes", json!([])).await?;
        let nodes: Vec<ClusterNodeInfo> = serde_json::from_value(res)?;
        Ok(nodes)
    }

    pub async fn get_vote_accounts(&self) -> anyhow::Result<VoteAccountsResponse> {
        let res = self.rpc_call("getVoteAccounts", json!([{"commitment": "processed"}])).await?;
        let votes: VoteAccountsResponse = serde_json::from_value(res)?;
        Ok(votes)
    }

    pub async fn get_latest_blockhash(&self) -> anyhow::Result<LatestBlockhash> {
        let res = self
            .rpc_call(
                "getLatestBlockhash",
                json!([{"commitment": "processed"}]),
            )
            .await?;
        let value = res.get("value").ok_or_else(|| anyhow::anyhow!("Missing blockhash value"))?;
        let bh: LatestBlockhash = serde_json::from_value(value.clone())?;
        Ok(bh)
    }

    pub async fn get_leader_schedule(
        &self,
        slot: Option<u64>,
    ) -> anyhow::Result<HashMap<String, Vec<usize>>> {
        let params = match slot {
            Some(s) => json!([s, {"commitment": "processed"}]),
            None => json!([null, {"commitment": "processed"}]),
        };
        let res = self.rpc_call("getLeaderSchedule", params).await?;
        let map: HashMap<String, Vec<usize>> = serde_json::from_value(res)?;
        Ok(map)
    }

    pub async fn send_transaction(&self, base64_tx: &str) -> anyhow::Result<String> {
        let res = self
            .rpc_call(
                "sendTransaction",
                json!([
                    base64_tx,
                    {
                        "encoding": "base64",
                        "skipPreflight": true,
                        "preflightCommitment": "processed",
                        "maxRetries": 3
                    }
                ]),
            )
            .await?;

        res.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("Invalid signature returned from sendTransaction"))
    }

    /// Dispatches a burst of transactions in a single batched JSON-RPC HTTP request
    /// to minimize client-side inter-packet serialization latency.
    pub async fn send_transaction_batch(&self, base64_txs: &[String]) -> anyhow::Result<Vec<String>> {
        if base64_txs.is_empty() {
            return Ok(Vec::new());
        }

        let batch: Vec<Value> = base64_txs
            .iter()
            .enumerate()
            .map(|(idx, tx)| {
                json!({
                    "jsonrpc": "2.0",
                    "id": idx + 1,
                    "method": "sendTransaction",
                    "params": [
                        tx,
                        {
                            "encoding": "base64",
                            "skipPreflight": true,
                            "preflightCommitment": "processed",
                            "maxRetries": 3
                        }
                    ]
                })
            })
            .collect();

        let resp = self.http.post(&self.endpoint).json(&batch).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Batch sendTransaction HTTP error: {}", resp.status());
        }

        let json_arr: Vec<Value> = resp.json().await?;
        let mut sigs = Vec::with_capacity(json_arr.len());

        for item in json_arr {
            if let Some(sig) = item.get("result").and_then(|r| r.as_str()) {
                sigs.push(sig.to_string());
            } else if let Some(err) = item.get("error") {
                warn!("Batch tx item error: {}", err);
            }
        }

        Ok(sigs)
    }

    #[allow(dead_code)]
    pub async fn get_signature_status(
        &self,
        signature: &str,
    ) -> anyhow::Result<Option<u64>> {
        let res = self
            .rpc_call(
                "getSignatureStatuses",
                json!([[signature], {"searchTransactionHistory": true}]),
            )
            .await?;

        if let Some(statuses) = res.get("value").and_then(|v| v.as_array()) {
            if let Some(status) = statuses.first().and_then(|s| s.as_object()) {
                if let Some(slot) = status.get("slot").and_then(|s| s.as_u64()) {
                    return Ok(Some(slot));
                }
            }
        }
        Ok(None)
    }

    /// Fetches a block and parses the exact transaction positions and order
    pub async fn get_block(&self, slot: u64) -> anyhow::Result<DetailedBlock> {
        let res = self
            .rpc_call(
                "getBlock",
                json!([
                    slot,
                    {
                        "encoding": "jsonParsed",
                        "transactionDetails": "full",
                        "rewards": false,
                        "commitment": "confirmed",
                        "maxSupportedTransactionVersion": 1
                    }
                ]),
            )
            .await?;

        let blockhash = res
            .get("blockhash")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string();
        let prev_blockhash = res
            .get("previousBlockhash")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string();
        let parent_slot = res
            .get("parentSlot")
            .and_then(|p| p.as_u64())
            .unwrap_or(0);
        let block_time = res.get("blockTime").and_then(|t| t.as_i64());

        let raw_txs = res
            .get("transactions")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let total_transactions = raw_txs.len();
        let mut parsed_txs = Vec::with_capacity(total_transactions);

        for (idx, tx_val) in raw_txs.into_iter().enumerate() {
            let meta = tx_val.get("meta");
            let tx_obj = tx_val.get("transaction");

            let sig = tx_obj
                .and_then(|t| t.get("signatures"))
                .and_then(|s| s.as_array())
                .and_then(|arr| arr.first())
                .and_then(|sig| sig.as_str())
                .unwrap_or("")
                .to_string();

            let fee = meta
                .and_then(|m| m.get("fee"))
                .and_then(|f| f.as_u64())
                .unwrap_or(5000);

            let cu_consumed = meta
                .and_then(|m| m.get("computeUnitsConsumed"))
                .and_then(|c| c.as_u64());

            let err = meta.and_then(|m| m.get("err")).cloned();

            let mut memo = None;
            let mut priority_fee = None;

            if let Some(msg) = tx_obj.and_then(|t| t.get("message")) {
                if let Some(instructions) = msg.get("instructions").and_then(|i| i.as_array()) {
                    for ix in instructions {
                        if let Some(prog) = ix.get("program").and_then(|p| p.as_str()) {
                            if prog == "spl-memo" {
                                if let Some(parsed) = ix.get("parsed").and_then(|p| p.as_str()) {
                                    memo = Some(parsed.to_string());
                                }
                            }
                        }
                    }
                }
            }

            // If fee > 5000 base fee, calculate estimated priority fee
            if fee > 5000 {
                let extra_lamports = fee - 5000;
                let cu = cu_consumed.unwrap_or(300).max(1);
                priority_fee = Some((extra_lamports as u128 * 1_000_000 / cu as u128) as u64);
            }

            parsed_txs.push(BlockTransaction {
                signature: sig,
                index_in_block: idx,
                fee,
                compute_units_consumed: cu_consumed,
                err,
                memo,
                priority_fee_micro_lamports: priority_fee,
            });
        }

        Ok(DetailedBlock {
            slot,
            blockhash,
            previous_blockhash: prev_blockhash,
            parent_slot,
            block_time,
            total_transactions,
            transactions: parsed_txs,
        })
    }
}
