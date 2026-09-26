use std::{
    path::Path,
    sync::Arc,
};

use ahash::AHashSet;
use arc_swap::ArcSwap;
use thiserror::Error;

use crate::types::{InspectedTx, Verdict};

#[derive(Debug, Error)]
pub enum BlocklistError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid pubkey on line {line}: {source}")]
    InvalidPubkey { line: usize, source: bs58::decode::Error },
}

#[derive(Clone)]
pub struct Blocklist {
    inner: Arc<ArcSwap<AHashSet<[u8; 32]>>>,
}

impl Blocklist {
    pub fn load_from_file(path: &Path) -> Result<Self, BlocklistError> {
        let set = load_set(path)?;
        Ok(Self { inner: Arc::new(ArcSwap::from_pointee(set)) })
    }

    pub fn reload(&self, path: &Path) -> Result<(), BlocklistError> {
        let set = load_set(path)?;
        self.inner.store(Arc::new(set));
        Ok(())
    }

    pub fn check(&self, tx: &InspectedTx) -> Option<Verdict> {
        let set = self.inner.load();
        for key in &tx.static_keys {
            if set.contains(key) {
                return Some(Verdict::Block {
                    rule_id: "ofac-sdn".into(),
                    code: -32001,
                    message: "Transaction contains addresses flagged under statutory compliance sanctions.".into(),
                });
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.inner.load().len()
    }
}

fn load_set(path: &Path) -> Result<AHashSet<[u8; 32]>, BlocklistError> {
    let content = std::fs::read_to_string(path)?;
    let mut set = AHashSet::new();
    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut bytes = [0u8; 32];
        let decoded = bs58::decode(line)
            .into_vec()
            .map_err(|e| BlocklistError::InvalidPubkey { line: i + 1, source: e })?;
        if decoded.len() == 32 {
            bytes.copy_from_slice(&decoded);
        }
        set.insert(bytes);
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture_path() -> std::path::PathBuf {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/blocklist.txt");
        p
    }

    #[test]
    fn test_blocklist_loads() {
        let bl = Blocklist::load_from_file(&fixture_path()).unwrap();
        assert_eq!(bl.len(), 5);
    }

    #[test]
    fn test_blocklist_hit() {
        let bl = Blocklist::load_from_file(&fixture_path()).unwrap();
        // First key from the fixture
        let content = std::fs::read_to_string(fixture_path()).unwrap();
        let first_key = content.lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap();
        let mut key_bytes = [0u8; 32];
        let decoded = bs58::decode(first_key).into_vec().unwrap();
        key_bytes.copy_from_slice(&decoded);

        let tx = InspectedTx {
            static_keys: vec![key_bytes],
            signers: vec![],
            writable: vec![],
            program_ids: vec![],
            lut_refs: vec![],
        };
        assert!(matches!(bl.check(&tx), Some(Verdict::Block { .. })));
    }

    #[test]
    fn test_blocklist_miss() {
        let bl = Blocklist::load_from_file(&fixture_path()).unwrap();
        let random_key = [42u8; 32];
        let tx = InspectedTx {
            static_keys: vec![random_key],
            signers: vec![],
            writable: vec![],
            program_ids: vec![],
            lut_refs: vec![],
        };
        assert!(bl.check(&tx).is_none());
    }

    #[test]
    fn test_blocklist_reload() {
        let bl = Blocklist::load_from_file(&fixture_path()).unwrap();
        assert_eq!(bl.len(), 5);

        let new_key = solana_sdk::pubkey::Pubkey::new_unique();
        let tmp_path = std::env::temp_dir().join("aegis_test_blocklist.txt");
        let mut f = std::fs::File::create(&tmp_path).unwrap();
        writeln!(f, "{}", new_key).unwrap();

        bl.reload(&tmp_path).unwrap();
        assert_eq!(bl.len(), 1);

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&new_key.to_bytes());
        let tx = InspectedTx {
            static_keys: vec![key_bytes],
            signers: vec![],
            writable: vec![],
            program_ids: vec![],
            lut_refs: vec![],
        };
        assert!(matches!(bl.check(&tx), Some(Verdict::Block { .. })));
    }
}
