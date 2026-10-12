use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Duration,
};

use aegis_core::{needs_refresh, parse_lookup_table, resolve_lookup, InspectedTx, LutError};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use thiserror::Error;

/// Bounds memory: a full table is 256 * 32 bytes, so this caps the cache at ~80 MiB.
pub const MAX_CACHED_TABLES: usize = 10_000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

/// Resolved table: table address -> its ordered account keys. A cached copy is immutable
/// once a lookup is satisfied (tables only ever grow), so it is shared behind an `Arc`.
type TableCache = RwLock<HashMap<[u8; 32], Arc<Vec<[u8; 32]>>>>;

#[derive(Debug, Error)]
pub enum LutFetchError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("lookup table account not found")]
    NotFound,
    #[error("malformed getAccountInfo response")]
    BadResponse,
    #[error(transparent)]
    Lut(#[from] LutError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LutOutcome {
    /// Transaction has no lookups.
    None,
    Cached,
    Fetched,
}

/// Resolves v0 Address Lookup Table references so accounts loaded through a table are
/// screened like static keys. Existing table entries are immutable (tables only grow),
/// so a cached copy stays correct and is refetched only when an index lies beyond it.
pub struct LutResolver {
    client: reqwest::Client,
    rpc_url: String,
    tables: TableCache,
}

impl LutResolver {
    pub fn new(client: reqwest::Client, rpc_url: String) -> Self {
        Self {
            client,
            rpc_url,
            tables: RwLock::new(HashMap::new()),
        }
    }

    /// Fills `tx.loaded_keys`.
    pub async fn resolve(&self, tx: &mut InspectedTx) -> Result<LutOutcome, LutFetchError> {
        if tx.lut_lookups.is_empty() {
            return Ok(LutOutcome::None);
        }
        let mut outcome = LutOutcome::Cached;
        let mut loaded = Vec::new();
        for lookup in &tx.lut_lookups {
            let cached = self.tables.read().unwrap().get(&lookup.table).cloned();
            let table = match cached {
                Some(t) if !needs_refresh(lookup, t.len()) => t,
                _ => {
                    outcome = LutOutcome::Fetched;
                    let fresh = Arc::new(self.fetch(&lookup.table).await?);
                    let mut tables = self.tables.write().unwrap();
                    if tables.len() >= MAX_CACHED_TABLES {
                        tables.clear();
                    }
                    tables.insert(lookup.table, fresh.clone());
                    fresh
                }
            };
            loaded.extend(resolve_lookup(lookup, &table)?);
        }
        tx.loaded_keys = loaded;
        Ok(outcome)
    }

    pub fn cached_tables(&self) -> usize {
        self.tables.read().unwrap().len()
    }

    // `processed` so a table extended moments ago is visible: the validator lets a tx use
    // new entries one slot after extension, far sooner than `finalized` would show them.
    async fn fetch(&self, table: &[u8; 32]) -> Result<Vec<[u8; 32]>, LutFetchError> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getAccountInfo",
            "params": [bs58::encode(table).into_string(), {"encoding": "base64", "commitment": "processed"}],
        });
        let resp: Value = self
            .client
            .post(&self.rpc_url)
            .timeout(FETCH_TIMEOUT)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let value = resp
            .pointer("/result/value")
            .ok_or(LutFetchError::BadResponse)?;
        if value.is_null() {
            return Err(LutFetchError::NotFound);
        }
        let data_b64 = value
            .pointer("/data/0")
            .and_then(Value::as_str)
            .ok_or(LutFetchError::BadResponse)?;
        let owner_b58 = value
            .get("owner")
            .and_then(Value::as_str)
            .ok_or(LutFetchError::BadResponse)?;

        let data = STANDARD
            .decode(data_b64)
            .map_err(|_| LutFetchError::BadResponse)?;
        let mut owner = [0u8; 32];
        match bs58::decode(owner_b58).onto(&mut owner[..]) {
            Ok(32) => {}
            _ => return Err(LutFetchError::BadResponse),
        }
        Ok(parse_lookup_table(&owner, &data)?)
    }
}
