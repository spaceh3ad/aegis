use base64::{engine::general_purpose::STANDARD, Engine};
use solana_sdk::{
    instruction::CompiledInstruction,
    message::{MessageHeader, VersionedMessage},
    pubkey::Pubkey,
    transaction::VersionedTransaction,
};
use thiserror::Error;

use crate::types::{InspectedTx, LutLookup};

const COMPUTE_BUDGET_ID: Pubkey =
    solana_sdk::pubkey!("ComputeBudget111111111111111111111111111111");
const SET_CU_LIMIT: u8 = 2;
const SET_CU_PRICE: u8 = 3;

/// Solana caps packets at 1232 bytes; anything encoding to more than this cannot be a valid tx.
/// Also bounds the quadratic cost of base58 decoding.
pub const MAX_ENCODED_TX_LEN: usize = 2048;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("encoded transaction exceeds {MAX_ENCODED_TX_LEN} chars")]
    TooLarge,
    #[error("invalid base64: {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("invalid base58: {0}")]
    InvalidBase58(#[from] bs58::decode::Error),
    #[error("deserialization failed: {0}")]
    Deserialize(#[from] bincode::Error),
    #[error("malformed message: {0}")]
    Malformed(&'static str),
}

pub fn parse_transaction(b64: &str) -> Result<InspectedTx, ParseError> {
    if b64.len() > MAX_ENCODED_TX_LEN {
        return Err(ParseError::TooLarge);
    }
    parse_transaction_bytes(&STANDARD.decode(b64)?)
}

/// `sendTransaction` defaults to base58 when the client omits `encoding`.
pub fn parse_transaction_base58(b58: &str) -> Result<InspectedTx, ParseError> {
    if b58.len() > MAX_ENCODED_TX_LEN {
        return Err(ParseError::TooLarge);
    }
    parse_transaction_bytes(&bs58::decode(b58).into_vec()?)
}

pub fn parse_transaction_bytes(bytes: &[u8]) -> Result<InspectedTx, ParseError> {
    let tx: VersionedTransaction = bincode::deserialize(bytes)?;

    match &tx.message {
        VersionedMessage::Legacy(msg) => inspect(
            &msg.header,
            &msg.account_keys,
            &msg.instructions,
            Vec::new(),
        ),
        VersionedMessage::V0(msg) => {
            let luts = msg
                .address_table_lookups
                .iter()
                .map(|l| LutLookup {
                    table: l.account_key.to_bytes(),
                    writable_indexes: l.writable_indexes.clone(),
                    readonly_indexes: l.readonly_indexes.clone(),
                })
                .collect();
            inspect(&msg.header, &msg.account_keys, &msg.instructions, luts)
        }
    }
}

// Input is attacker-controlled and not sanitized by bincode, so every index is checked.
fn inspect(
    header: &MessageHeader,
    keys: &[Pubkey],
    instructions: &[CompiledInstruction],
    lut_lookups: Vec<LutLookup>,
) -> Result<InspectedTx, ParseError> {
    let num_signers = header.num_required_signatures as usize;
    let signed_readonly = header.num_readonly_signed_accounts as usize;
    let unsigned_readonly = header.num_readonly_unsigned_accounts as usize;

    if num_signers > keys.len() {
        return Err(ParseError::Malformed("more signers than account keys"));
    }
    let writable_signed = num_signers
        .checked_sub(signed_readonly)
        .ok_or(ParseError::Malformed("readonly signers exceed signers"))?;
    let writable_unsigned_end = keys
        .len()
        .checked_sub(unsigned_readonly)
        .filter(|end| *end >= num_signers)
        .ok_or(ParseError::Malformed(
            "readonly unsigned accounts overlap signers",
        ))?;

    let static_keys: Vec<[u8; 32]> = keys.iter().map(|k| k.to_bytes()).collect();
    let signers = static_keys[..num_signers].to_vec();
    let mut writable = static_keys[..writable_signed].to_vec();
    writable.extend_from_slice(&static_keys[num_signers..writable_unsigned_end]);

    let mut program_ids = Vec::with_capacity(instructions.len());
    let mut cu_limit = None;
    let mut cu_price = None;
    for ix in instructions {
        let prog = keys
            .get(ix.program_id_index as usize)
            .ok_or(ParseError::Malformed("program index out of range"))?;
        program_ids.push(prog.to_bytes());

        if *prog == COMPUTE_BUDGET_ID {
            match ix.data.split_first() {
                Some((&SET_CU_LIMIT, rest)) if rest.len() >= 4 => {
                    cu_limit = Some(u32::from_le_bytes(rest[..4].try_into().unwrap()));
                }
                Some((&SET_CU_PRICE, rest)) if rest.len() >= 8 => {
                    cu_price = Some(u64::from_le_bytes(rest[..8].try_into().unwrap()));
                }
                _ => {}
            }
        }
    }

    Ok(InspectedTx {
        static_keys,
        signers,
        writable,
        program_ids,
        lut_lookups,
        loaded_keys: Vec::new(),
        cu_limit,
        cu_price,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(deprecated)]
    use solana_sdk::{
        hash::Hash, message::legacy::Message as LegacyMessage, pubkey::Pubkey, signature::Keypair,
        signer::Signer, system_instruction, transaction::VersionedTransaction,
    };

    #[allow(deprecated)]
    fn make_legacy_tx() -> String {
        let payer = Keypair::new();
        let recipient = Pubkey::new_unique();
        let ix = system_instruction::transfer(&payer.pubkey(), &recipient, 1_000_000);
        let msg = LegacyMessage::new(&[ix], Some(&payer.pubkey()));
        let tx = VersionedTransaction::from(solana_sdk::transaction::Transaction::new(
            &[&payer],
            msg,
            Hash::default(),
        ));
        let bytes = bincode::serialize(&tx).unwrap();
        STANDARD.encode(bytes)
    }

    #[test]
    fn test_parse_legacy_tx() {
        let b64 = make_legacy_tx();
        let result = parse_transaction(&b64).unwrap();
        assert!(!result.signers.is_empty(), "should have signers");
        assert!(!result.program_ids.is_empty(), "should have program ids");
        assert!(result.lut_lookups.is_empty(), "legacy tx has no LUTs");
    }

    #[test]
    fn test_parse_v0_tx() {
        use solana_sdk::{
            message::{v0, v0::MessageAddressTableLookup, MessageHeader, VersionedMessage},
            transaction::VersionedTransaction,
        };

        let payer = Keypair::new();
        let lut_key = Pubkey::new_unique();

        let v0_msg = v0::Message {
            header: MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![payer.pubkey()],
            recent_blockhash: Hash::default(),
            instructions: vec![],
            address_table_lookups: vec![MessageAddressTableLookup {
                account_key: lut_key,
                writable_indexes: vec![0],
                readonly_indexes: vec![],
            }],
        };

        let tx = VersionedTransaction {
            signatures: vec![solana_sdk::signature::Signature::default()],
            message: VersionedMessage::V0(v0_msg),
        };

        let b64 = STANDARD.encode(bincode::serialize(&tx).unwrap());
        let result = parse_transaction(&b64).unwrap();

        assert_eq!(
            result.lut_lookups,
            vec![LutLookup {
                table: lut_key.to_bytes(),
                writable_indexes: vec![0],
                readonly_indexes: vec![]
            }]
        );
        assert_eq!(result.signers.len(), 1);
        assert_eq!(result.signers[0], payer.pubkey().to_bytes());
    }

    #[test]
    fn test_parse_invalid_b64() {
        let err = parse_transaction("!!!not_valid_base64!!!").unwrap_err();
        assert!(matches!(err, ParseError::InvalidBase64(_)));
    }

    #[test]
    fn test_parse_garbage_bytes() {
        let garbage = STANDARD.encode(b"not a transaction at all");
        let err = parse_transaction(&garbage).unwrap_err();
        assert!(matches!(err, ParseError::Deserialize(_)));
    }

    fn encode_v0(
        header: MessageHeader,
        keys: Vec<Pubkey>,
        ixs: Vec<CompiledInstruction>,
    ) -> String {
        use solana_sdk::message::v0;
        let msg = v0::Message {
            header,
            account_keys: keys,
            recent_blockhash: Hash::default(),
            instructions: ixs,
            address_table_lookups: vec![],
        };
        let tx = VersionedTransaction {
            signatures: vec![solana_sdk::signature::Signature::default()],
            message: VersionedMessage::V0(msg),
        };
        STANDARD.encode(bincode::serialize(&tx).unwrap())
    }

    fn header(signers: u8, ro_signed: u8, ro_unsigned: u8) -> MessageHeader {
        MessageHeader {
            num_required_signatures: signers,
            num_readonly_signed_accounts: ro_signed,
            num_readonly_unsigned_accounts: ro_unsigned,
        }
    }

    #[test]
    fn test_compute_budget_extracted() {
        let payer = Pubkey::new_unique();
        let mut limit_data = vec![SET_CU_LIMIT];
        limit_data.extend_from_slice(&600_000u32.to_le_bytes());
        let mut price_data = vec![SET_CU_PRICE];
        price_data.extend_from_slice(&12_345u64.to_le_bytes());

        let b64 = encode_v0(
            header(1, 0, 1),
            vec![payer, COMPUTE_BUDGET_ID],
            vec![
                CompiledInstruction {
                    program_id_index: 1,
                    accounts: vec![],
                    data: limit_data,
                },
                CompiledInstruction {
                    program_id_index: 1,
                    accounts: vec![],
                    data: price_data,
                },
            ],
        );
        let tx = parse_transaction(&b64).unwrap();
        assert_eq!(tx.cu_limit, Some(600_000));
        assert_eq!(tx.cu_price, Some(12_345));
        assert_eq!(tx.writable, vec![payer.to_bytes()]);
    }

    #[test]
    fn test_compute_budget_absent_or_truncated() {
        let b64 = encode_v0(
            header(1, 0, 1),
            vec![Pubkey::new_unique(), COMPUTE_BUDGET_ID],
            vec![CompiledInstruction {
                program_id_index: 1,
                accounts: vec![],
                data: vec![SET_CU_LIMIT, 1],
            }],
        );
        let tx = parse_transaction(&b64).unwrap();
        assert_eq!(tx.cu_limit, None);
        assert_eq!(tx.cu_price, None);
    }

    #[test]
    fn test_malformed_headers_rejected() {
        let k = || Pubkey::new_unique();
        let cases = [
            (header(3, 0, 0), vec![k()]),
            (header(1, 2, 0), vec![k(), k()]),
            (header(1, 0, 5), vec![k(), k()]),
        ];
        for (h, keys) in cases {
            let err = parse_transaction(&encode_v0(h, keys, vec![])).unwrap_err();
            assert!(matches!(err, ParseError::Malformed(_)));
        }
        let oob = encode_v0(
            header(1, 0, 0),
            vec![k()],
            vec![CompiledInstruction {
                program_id_index: 9,
                accounts: vec![],
                data: vec![],
            }],
        );
        assert!(matches!(
            parse_transaction(&oob).unwrap_err(),
            ParseError::Malformed(_)
        ));
    }

    #[test]
    fn test_base58_matches_base64() {
        let b64 = make_legacy_tx();
        let raw = STANDARD.decode(&b64).unwrap();
        let b58 = bs58::encode(&raw).into_string();
        let a = parse_transaction(&b64).unwrap();
        let b = parse_transaction_base58(&b58).unwrap();
        assert_eq!(a.static_keys, b.static_keys);
        assert_eq!(a.program_ids, b.program_ids);
    }

    #[test]
    fn test_oversized_rejected_before_decode() {
        let big = "1".repeat(MAX_ENCODED_TX_LEN + 1);
        assert!(matches!(
            parse_transaction_base58(&big),
            Err(ParseError::TooLarge)
        ));
        assert!(matches!(parse_transaction(&big), Err(ParseError::TooLarge)));
    }

    mod fuzz {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn base58_never_panics(s in "[1-9A-HJ-NP-Za-km-z]{0,2100}") {
                let _ = parse_transaction_base58(&s);
            }

            // Invariant: both encodings of the same bytes produce the same inspection result.
            #[test]
            fn encodings_agree(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
                let a = parse_transaction(&STANDARD.encode(&bytes)).map(|t| t.static_keys);
                let b = parse_transaction_base58(&bs58::encode(&bytes).into_string()).map(|t| t.static_keys);
                prop_assert_eq!(a.ok(), b.ok());
            }

            #[test]
            fn never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
                let _ = parse_transaction(&STANDARD.encode(&bytes));
            }

            #[test]
            fn never_panics_on_arbitrary_headers(
                signers in any::<u8>(),
                ro_signed in any::<u8>(),
                ro_unsigned in any::<u8>(),
                nkeys in 0usize..8,
                prog_idx in any::<u8>(),
                data in proptest::collection::vec(any::<u8>(), 0..16),
            ) {
                let keys: Vec<Pubkey> = (0..nkeys).map(|_| Pubkey::new_unique()).collect();
                let b64 = encode_v0(
                    header(signers, ro_signed, ro_unsigned),
                    keys,
                    vec![CompiledInstruction { program_id_index: prog_idx, accounts: vec![], data }],
                );
                let _ = parse_transaction(&b64);
            }

            // Invariant: on success, signers/writable/programs are drawn from static_keys
            // and writable never exceeds the key count.
            #[test]
            fn parsed_views_are_subsets_of_static_keys(
                signers in 0u8..6,
                ro_signed in 0u8..6,
                ro_unsigned in 0u8..6,
                nkeys in 0usize..6,
                prog_idx in 0u8..6,
            ) {
                let keys: Vec<Pubkey> = (0..nkeys).map(|_| Pubkey::new_unique()).collect();
                let b64 = encode_v0(
                    header(signers, ro_signed, ro_unsigned),
                    keys,
                    vec![CompiledInstruction { program_id_index: prog_idx, accounts: vec![], data: vec![] }],
                );
                if let Ok(tx) = parse_transaction(&b64) {
                    prop_assert!(tx.writable.len() <= tx.static_keys.len());
                    for k in tx.signers.iter().chain(&tx.writable).chain(&tx.program_ids) {
                        prop_assert!(tx.static_keys.contains(k));
                    }
                }
            }
        }
    }
}
