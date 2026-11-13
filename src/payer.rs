use anyhow::{Context, Result};
use serde_json::Value;
use solana_sdk::signature::Keypair;
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub struct PayerPool {
    master: Option<Keypair>,
    independent: Vec<Keypair>,
}

impl PayerPool {
    pub fn new(master: Option<Keypair>, independent: Vec<Keypair>) -> Self {
        Self {
            master,
            independent,
        }
    }

    /// Returns the payer references for a burst. Shared mode deliberately
    /// reuses one payer; independent mode requires enough distinct payers.
    pub fn for_burst(&self, size: usize, independent: bool) -> Result<Vec<&Keypair>> {
        if independent {
            if self.independent.len() < size {
                anyhow::bail!(
                    "independent payer pool has {} keys but burst size is {}; fund at least {} payers",
                    self.independent.len(),
                    size,
                    size
                );
            }
            return Ok(self.independent[..size].iter().collect());
        }
        let master = self
            .master
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("shared payer mode requires a master keypair"))?;
        Ok(vec![master; size])
    }
}

pub fn load_keypair(path: &Path) -> Result<Keypair> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("read keypair {}", path.display()))?;
    parse_keypair_value(&serde_json::from_str(&raw)?, &path.display().to_string())
}

pub fn load_payer_pool(path: &Path) -> Result<Vec<Keypair>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("read payer pool {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)?;
    let values = match &value {
        Value::Array(v) => v.clone(),
        Value::Object(obj) => obj
            .get("keys")
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("payer pool object must contain a `keys` array"))?,
        _ => anyhow::bail!("payer pool must be a JSON array or an object with `keys`"),
    };
    let mut keys = Vec::with_capacity(values.len());
    for (idx, item) in values.iter().enumerate() {
        keys.push(parse_keypair_value(
            item,
            &format!("{}#{idx}", path.display()),
        )?);
    }
    if keys.is_empty() {
        anyhow::bail!("payer pool is empty");
    }
    Ok(keys)
}

fn parse_keypair_value(value: &Value, path: &str) -> Result<Keypair> {
    let bytes: Vec<u8> = match value {
        Value::Array(arr) => arr
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| anyhow::anyhow!("keypair bytes must be 0..=255"))
            })
            .collect::<Result<Vec<u8>>>()?,
        Value::String(s) => bs58::decode(s)
            .into_vec()
            .with_context(|| format!("decode base58 keypair {path}"))?,
        Value::Object(obj) => {
            let arr = obj
                .get("secretKey")
                .or_else(|| obj.get("secret_key"))
                .or_else(|| obj.get("key"))
                .ok_or_else(|| anyhow::anyhow!("keypair object must contain secretKey"))?;
            return parse_keypair_value(arr, path);
        }
        _ => anyhow::bail!("keypair {path} must be a byte array or base58 string"),
    };
    Keypair::try_from(bytes.as_slice()).map_err(|e| anyhow::anyhow!("invalid keypair {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::Signer;

    #[test]
    fn independent_pool_requires_enough_keys() {
        let pool = PayerPool::new(None, vec![Keypair::new(), Keypair::new()]);
        assert!(pool.for_burst(3, true).is_err());
        assert_eq!(pool.for_burst(2, true).unwrap().len(), 2);
    }

    #[test]
    fn shared_pool_reuses_master() {
        let pool = PayerPool::new(Some(Keypair::new()), Vec::new());
        let payers = pool.for_burst(4, false).unwrap();
        assert_eq!(payers.len(), 4);
        assert_eq!(payers[0].pubkey(), payers[3].pubkey());
    }
}
