use solana_sdk::address_lookup_table::state::AddressLookupTable;
use thiserror::Error;

use crate::types::LutLookup;

pub const LOOKUP_TABLE_PROGRAM_ID: [u8; 32] =
    solana_sdk::address_lookup_table::program::ID.to_bytes();

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LutError {
    #[error("account is not owned by the address lookup table program")]
    WrongOwner,
    #[error("invalid lookup table account data")]
    InvalidData,
    #[error("lookup index {index} out of range for table of {len} addresses")]
    IndexOutOfRange { index: u8, len: usize },
}

/// Decodes the address list of an on-chain Address Lookup Table account.
pub fn parse_lookup_table(owner: &[u8; 32], data: &[u8]) -> Result<Vec<[u8; 32]>, LutError> {
    if *owner != LOOKUP_TABLE_PROGRAM_ID {
        return Err(LutError::WrongOwner);
    }
    let table = AddressLookupTable::deserialize(data).map_err(|_| LutError::InvalidData)?;
    Ok(table.addresses.iter().map(|k| k.to_bytes()).collect())
}

/// Maps one lookup's indexes onto the table's addresses.
pub fn resolve_lookup(lookup: &LutLookup, table: &[[u8; 32]]) -> Result<Vec<[u8; 32]>, LutError> {
    lookup
        .indexes()
        .map(|i| {
            table
                .get(i as usize)
                .copied()
                .ok_or(LutError::IndexOutOfRange {
                    index: i,
                    len: table.len(),
                })
        })
        .collect()
}

/// Index out of range is the only resolve error: it means our copy may be stale.
pub fn needs_refresh(lookup: &LutLookup, cached_len: usize) -> bool {
    lookup.indexes().any(|i| i as usize >= cached_len)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use solana_sdk::{
        address_lookup_table::state::{AddressLookupTable, LookupTableMeta},
        pubkey::Pubkey,
    };
    use std::borrow::Cow;

    pub(crate) fn table_account_data(addresses: &[[u8; 32]]) -> Vec<u8> {
        let keys: Vec<Pubkey> = addresses
            .iter()
            .map(|a| Pubkey::new_from_array(*a))
            .collect();
        AddressLookupTable {
            meta: LookupTableMeta::default(),
            addresses: Cow::Owned(keys),
        }
        .serialize_for_tests()
        .unwrap()
    }

    fn lookup(w: Vec<u8>, r: Vec<u8>) -> LutLookup {
        LutLookup {
            table: [9; 32],
            writable_indexes: w,
            readonly_indexes: r,
        }
    }

    #[test]
    fn test_parse_and_resolve() {
        let addrs = [[1u8; 32], [2u8; 32], [3u8; 32]];
        let parsed =
            parse_lookup_table(&LOOKUP_TABLE_PROGRAM_ID, &table_account_data(&addrs)).unwrap();
        assert_eq!(parsed, addrs.to_vec());
        assert_eq!(
            resolve_lookup(&lookup(vec![2], vec![0]), &parsed).unwrap(),
            vec![[3u8; 32], [1u8; 32]]
        );
    }

    #[test]
    fn test_wrong_owner_and_bad_data() {
        let data = table_account_data(&[[1u8; 32]]);
        assert_eq!(
            parse_lookup_table(&[0u8; 32], &data),
            Err(LutError::WrongOwner)
        );
        assert_eq!(
            parse_lookup_table(&LOOKUP_TABLE_PROGRAM_ID, &data[..10]),
            Err(LutError::InvalidData)
        );
        let mut ragged = data.clone();
        ragged.push(0);
        assert_eq!(
            parse_lookup_table(&LOOKUP_TABLE_PROGRAM_ID, &ragged),
            Err(LutError::InvalidData)
        );
    }

    #[test]
    fn test_out_of_range() {
        let table = vec![[1u8; 32]];
        assert!(needs_refresh(&lookup(vec![], vec![1]), table.len()));
        assert_eq!(
            resolve_lookup(&lookup(vec![], vec![1]), &table),
            Err(LutError::IndexOutOfRange { index: 1, len: 1 })
        );
    }

    mod fuzz {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn parse_never_panics(data in proptest::collection::vec(any::<u8>(), 0..600)) {
                let _ = parse_lookup_table(&LOOKUP_TABLE_PROGRAM_ID, &data);
            }

            // Invariant: resolve succeeds iff no index needs a refresh, and then returns
            // exactly table[i] for each index in writable-then-readonly order.
            #[test]
            fn resolve_matches_indexing(
                n in 0usize..40,
                w in proptest::collection::vec(any::<u8>(), 0..8),
                r in proptest::collection::vec(any::<u8>(), 0..8),
            ) {
                let table: Vec<[u8; 32]> = (0..n).map(|i| [i as u8; 32]).collect();
                let data = table_account_data(&table);
                let parsed = parse_lookup_table(&LOOKUP_TABLE_PROGRAM_ID, &data).unwrap();
                prop_assert_eq!(&parsed, &table);

                let l = lookup(w.clone(), r.clone());
                match resolve_lookup(&l, &parsed) {
                    Ok(keys) => {
                        prop_assert!(!needs_refresh(&l, n));
                        let expected: Vec<[u8; 32]> = w.iter().chain(&r).map(|i| table[*i as usize]).collect();
                        prop_assert_eq!(keys, expected);
                    }
                    Err(_) => prop_assert!(needs_refresh(&l, n)),
                }
            }
        }
    }
}
