use std::{path::Path, sync::Arc};

use ahash::AHashMap;
use arc_swap::ArcSwap;
use thiserror::Error;

use crate::types::{EvmTx, InspectedTx, Verdict};

pub const BLOCKLIST_CODE: i32 = -32001;

#[derive(Debug, Error)]
pub enum BlocklistError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{source_name}: invalid address on line {line}: {reason}")]
    InvalidAddress {
        source_name: String,
        line: usize,
        reason: String,
    },
}

/// Addresses from one or more named sources. Each entry remembers which source listed it,
/// so a rejection can say whose list caused it.
#[derive(Debug, Default, Clone)]
pub struct AddressSets {
    svm: AHashMap<[u8; 32], u16>,
    evm: AHashMap<[u8; 20], u16>,
    sources: Vec<String>,
}

impl AddressSets {
    /// Parses one address per line. `0x`-prefixed 20-byte hex is EVM, anything else must be
    /// a base58 32-byte Solana pubkey. Blank lines and `#` comments are ignored.
    pub fn parse(source: &str, text: &str) -> Result<Self, BlocklistError> {
        let mut sets = Self {
            sources: vec![source.to_string()],
            ..Self::default()
        };
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let err = |reason: String| BlocklistError::InvalidAddress {
                source_name: source.to_string(),
                line: i + 1,
                reason,
            };
            if let Some(hex_addr) = line.strip_prefix("0x").or_else(|| line.strip_prefix("0X")) {
                let mut addr = [0u8; 20];
                hex::decode_to_slice(hex_addr, &mut addr).map_err(|e| err(e.to_string()))?;
                sets.evm.entry(addr).or_insert(0);
            } else {
                let mut key = [0u8; 32];
                let n = bs58::decode(line)
                    .onto(&mut key[..])
                    .map_err(|e| err(e.to_string()))?;
                if n != 32 {
                    return Err(err(format!("decoded {n} bytes, expected 32")));
                }
                sets.svm.entry(key).or_insert(0);
            }
        }
        Ok(sets)
    }

    /// Union of all parts. When an address appears in several sources, the earliest part wins.
    pub fn merge<'a>(parts: impl IntoIterator<Item = &'a AddressSets>) -> Self {
        let mut out = Self::default();
        for part in parts {
            let offset = out.sources.len() as u16;
            out.sources.extend(part.sources.iter().cloned());
            for (k, id) in &part.svm {
                out.svm.entry(*k).or_insert(id + offset);
            }
            for (k, id) in &part.evm {
                out.evm.entry(*k).or_insert(id + offset);
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.svm.len() + self.evm.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn svm_source(&self, key: &[u8; 32]) -> Option<&str> {
        self.svm
            .get(key)
            .map(|id| self.sources[*id as usize].as_str())
    }

    pub fn evm_source(&self, addr: &[u8; 20]) -> Option<&str> {
        self.evm
            .get(addr)
            .map(|id| self.sources[*id as usize].as_str())
    }
}

fn block(source: &str) -> Verdict {
    Verdict::Block {
        rule_id: format!("blocklist:{source}"),
        code: BLOCKLIST_CODE,
        message: format!(
            "Transaction references an address on the operator blocklist (source: {source})."
        ),
    }
}

/// Lock-free, atomically swappable view over the merged [`AddressSets`].
#[derive(Clone)]
pub struct Blocklist {
    inner: Arc<ArcSwap<AddressSets>>,
}

impl Blocklist {
    pub fn new(sets: AddressSets) -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(sets)),
        }
    }

    pub fn load_from_file(path: &Path) -> Result<Self, BlocklistError> {
        Ok(Self::new(AddressSets::parse(
            "file",
            &std::fs::read_to_string(path)?,
        )?))
    }

    pub fn reload(&self, path: &Path) -> Result<(), BlocklistError> {
        self.replace(AddressSets::parse("file", &std::fs::read_to_string(path)?)?);
        Ok(())
    }

    pub fn replace(&self, sets: AddressSets) {
        self.inner.store(Arc::new(sets));
    }

    pub fn check(&self, tx: &InspectedTx) -> Option<Verdict> {
        let sets = self.inner.load();
        tx.static_keys
            .iter()
            .chain(&tx.loaded_keys)
            .find_map(|k| sets.svm_source(k))
            .map(block)
    }

    /// Screens both the recovered sender and the call target.
    pub fn check_evm(&self, tx: &EvmTx) -> Option<Verdict> {
        let sets = self.inner.load();
        std::iter::once(&tx.from)
            .chain(tx.to.as_ref())
            .find_map(|a| sets.evm_source(a))
            .map(block)
    }

    pub fn len(&self) -> usize {
        self.inner.load().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
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

    fn svm_tx(keys: Vec<[u8; 32]>) -> InspectedTx {
        InspectedTx {
            static_keys: keys,
            ..Default::default()
        }
    }

    fn evm_tx(from: [u8; 20], to: Option<[u8; 20]>) -> EvmTx {
        EvmTx {
            chain_id: Some(1),
            from,
            to,
            selector: None,
            gas_limit: 21_000,
            priority_fee: 0,
            value_wei: "0".into(),
            tx_type: 2,
        }
    }

    #[test]
    fn test_blocklist_loads() {
        let bl = Blocklist::load_from_file(&fixture_path()).unwrap();
        assert_eq!(bl.len(), 5);
    }

    #[test]
    fn test_blocklist_hit() {
        let bl = Blocklist::load_from_file(&fixture_path()).unwrap();
        let content = std::fs::read_to_string(fixture_path()).unwrap();
        let first_key = content
            .lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap();
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bs58::decode(first_key).into_vec().unwrap());

        match bl.check(&svm_tx(vec![key_bytes])) {
            Some(Verdict::Block { rule_id, code, .. }) => {
                assert_eq!(rule_id, "blocklist:file");
                assert_eq!(code, BLOCKLIST_CODE);
            }
            other => panic!("expected block, got {other:?}"),
        }
    }

    #[test]
    fn test_blocklist_miss() {
        let bl = Blocklist::load_from_file(&fixture_path()).unwrap();
        assert!(bl.check(&svm_tx(vec![[42u8; 32]])).is_none());
    }

    #[test]
    fn test_blocklist_reload() {
        let bl = Blocklist::load_from_file(&fixture_path()).unwrap();
        assert_eq!(bl.len(), 5);

        let new_key = solana_sdk::pubkey::Pubkey::new_unique();
        let tmp_path =
            std::env::temp_dir().join(format!("aegis_test_blocklist_{}.txt", std::process::id()));
        let mut f = std::fs::File::create(&tmp_path).unwrap();
        writeln!(f, "{}", new_key).unwrap();

        bl.reload(&tmp_path).unwrap();
        let _ = std::fs::remove_file(&tmp_path);
        assert_eq!(bl.len(), 1);
        assert!(matches!(
            bl.check(&svm_tx(vec![new_key.to_bytes()])),
            Some(Verdict::Block { .. })
        ));
    }

    #[test]
    fn test_lookup_table_loaded_keys_are_screened() {
        let listed = solana_sdk::pubkey::Pubkey::new_unique();
        let bl = Blocklist::new(AddressSets::parse("file", &listed.to_string()).unwrap());
        let tx = InspectedTx {
            static_keys: vec![[1; 32]],
            loaded_keys: vec![listed.to_bytes()],
            ..Default::default()
        };
        assert!(bl.check(&tx).is_some());
    }

    #[test]
    fn test_short_base58_is_rejected_not_zeroed() {
        // "1111" decodes to 4 zero bytes; it must not become the all-zero System Program id.
        let err = AddressSets::parse("file", "1111\n").unwrap_err();
        assert!(matches!(
            err,
            BlocklistError::InvalidAddress { line: 1, .. }
        ));
    }

    #[test]
    fn test_mixed_evm_and_svm() {
        let text = "# sample\n0x098B716B8Aaf21512996dC57EB0615e2383E2f96\nSo11111111111111111111111111111111111111112\n";
        let sets = AddressSets::parse("file", text).unwrap();
        assert_eq!(sets.len(), 2);
        let bl = Blocklist::new(sets);

        let mut ronin = [0u8; 20];
        hex::decode_to_slice("098B716B8Aaf21512996dC57EB0615e2383E2f96", &mut ronin).unwrap();
        assert!(
            bl.check_evm(&evm_tx(ronin, None)).is_some(),
            "sender screened"
        );
        assert!(
            bl.check_evm(&evm_tx([1; 20], Some(ronin))).is_some(),
            "target screened"
        );
        assert!(bl.check_evm(&evm_tx([1; 20], Some([2; 20]))).is_none());
    }

    #[test]
    fn test_invalid_evm_hex_rejected() {
        assert!(AddressSets::parse("f", "0x1234\n").is_err());
        assert!(AddressSets::parse("f", "0xZZ8B716B8Aaf21512996dC57EB0615e2383E2f96\n").is_err());
    }

    #[test]
    fn test_merge_attributes_first_source() {
        let a = AddressSets::parse("seal", "0x0000000000000000000000000000000000000001\n").unwrap();
        let b = AddressSets::parse(
            "trm",
            "0x0000000000000000000000000000000000000001\n0x0000000000000000000000000000000000000002\n",
        )
        .unwrap();
        let merged = AddressSets::merge([&a, &b]);
        assert_eq!(merged.len(), 2);
        let mut one = [0u8; 20];
        one[19] = 1;
        let mut two = [0u8; 20];
        two[19] = 2;
        assert_eq!(merged.evm_source(&one), Some("seal"));
        assert_eq!(merged.evm_source(&two), Some("trm"));
    }

    mod fuzz {
        use super::*;
        use proptest::collection::{hash_set, vec};
        use proptest::prelude::*;

        fn render(svm: &[[u8; 32]], evm: &[[u8; 20]]) -> String {
            let mut s = String::from("# generated\n\n");
            for k in svm {
                s.push_str(&bs58::encode(k).into_string());
                s.push('\n');
            }
            for a in evm {
                s.push_str(&format!("0x{}\n", hex::encode(a)));
            }
            s
        }

        proptest! {
            #[test]
            fn parse_never_panics(text in "\\PC*") {
                let _ = AddressSets::parse("fuzz", &text);
            }

            // Invariant: a tx is blocked iff it shares at least one key with the list.
            #[test]
            fn check_blocks_iff_intersection(
                listed in hash_set(proptest::array::uniform32(any::<u8>()), 0..32),
                tx_keys in vec(proptest::array::uniform32(any::<u8>()), 0..8),
                reuse in vec(any::<prop::sample::Index>(), 0..3),
            ) {
                let listed: Vec<[u8; 32]> = listed.into_iter().collect();
                let mut keys = tx_keys;
                if !listed.is_empty() {
                    keys.extend(reuse.iter().map(|i| listed[i.index(listed.len())]));
                }
                let bl = Blocklist::new(AddressSets::parse("p", &render(&listed, &[])).unwrap());
                let expected = keys.iter().any(|k| listed.contains(k));
                prop_assert_eq!(bl.check(&svm_tx(keys)).is_some(), expected);
            }

            #[test]
            fn check_evm_blocks_iff_sender_or_target_listed(
                listed in hash_set(proptest::array::uniform20(any::<u8>()), 0..32),
                from in proptest::array::uniform20(any::<u8>()),
                to in proptest::option::of(proptest::array::uniform20(any::<u8>())),
                pick in proptest::option::of(any::<prop::sample::Index>()),
            ) {
                let listed: Vec<[u8; 20]> = listed.into_iter().collect();
                let from = match (pick, listed.is_empty()) {
                    (Some(i), false) => listed[i.index(listed.len())],
                    _ => from,
                };
                let bl = Blocklist::new(AddressSets::parse("p", &render(&[], &listed)).unwrap());
                let expected = listed.contains(&from) || to.is_some_and(|t| listed.contains(&t));
                prop_assert_eq!(bl.check_evm(&evm_tx(from, to)).is_some(), expected);
            }

            // Invariant: merge is a union, and every entry is attributed to a source that listed it.
            #[test]
            fn merge_is_attributed_union(
                a in hash_set(proptest::array::uniform20(0u8..4), 0..16),
                b in hash_set(proptest::array::uniform20(0u8..4), 0..16),
            ) {
                let a: Vec<_> = a.into_iter().collect();
                let b: Vec<_> = b.into_iter().collect();
                let sa = AddressSets::parse("a", &render(&[], &a)).unwrap();
                let sb = AddressSets::parse("b", &render(&[], &b)).unwrap();
                let m = AddressSets::merge([&sa, &sb]);
                let union: std::collections::HashSet<_> = a.iter().chain(&b).collect();
                prop_assert_eq!(m.len(), union.len());
                for addr in union {
                    let expected = if a.contains(addr) { "a" } else { "b" };
                    prop_assert_eq!(m.evm_source(addr), Some(expected));
                }
            }
        }
    }
}
