use base64::{engine::general_purpose::STANDARD, Engine};
use solana_sdk::{
    message::VersionedMessage,
    transaction::VersionedTransaction,
};
use thiserror::Error;

use crate::types::InspectedTx;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid base64: {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("deserialization failed: {0}")]
    Deserialize(#[from] bincode::Error),
}

pub fn parse_transaction(b64: &str) -> Result<InspectedTx, ParseError> {
    let bytes = STANDARD.decode(b64)?;
    let tx: VersionedTransaction = bincode::deserialize(&bytes)?;

    let mut static_keys: Vec<[u8; 32]> = Vec::new();
    let mut signers: Vec<[u8; 32]> = Vec::new();
    let mut writable: Vec<[u8; 32]> = Vec::new();
    let mut program_ids: Vec<[u8; 32]> = Vec::new();
    let mut lut_refs: Vec<[u8; 32]> = Vec::new();

    match &tx.message {
        VersionedMessage::Legacy(msg) => {
            let header = &msg.header;
            let keys = &msg.account_keys;

            for key in keys {
                static_keys.push(key.to_bytes());
            }

            let num_signers = header.num_required_signatures as usize;
            let num_signed_readonly = header.num_readonly_signed_accounts as usize;
            let num_unsigned_readonly = header.num_readonly_unsigned_accounts as usize;

            for key in &keys[..num_signers] {
                signers.push(key.to_bytes());
            }

            let writable_signed = num_signers - num_signed_readonly;
            let writable_unsigned_start = num_signers;
            let writable_unsigned_end = keys.len() - num_unsigned_readonly;

            for key in &keys[..writable_signed] {
                writable.push(key.to_bytes());
            }
            for key in &keys[writable_unsigned_start..writable_unsigned_end] {
                writable.push(key.to_bytes());
            }

            for ix in &msg.instructions {
                let prog = keys[ix.program_id_index as usize];
                program_ids.push(prog.to_bytes());
            }
        }
        VersionedMessage::V0(msg) => {
            let header = &msg.header;
            let keys = &msg.account_keys;

            for key in keys {
                static_keys.push(key.to_bytes());
            }

            let num_signers = header.num_required_signatures as usize;
            let num_signed_readonly = header.num_readonly_signed_accounts as usize;
            let num_unsigned_readonly = header.num_readonly_unsigned_accounts as usize;

            for key in &keys[..num_signers] {
                signers.push(key.to_bytes());
            }

            let writable_signed = num_signers - num_signed_readonly;
            let writable_unsigned_start = num_signers;
            let writable_unsigned_end = keys.len() - num_unsigned_readonly;

            for key in &keys[..writable_signed] {
                writable.push(key.to_bytes());
            }
            for key in &keys[writable_unsigned_start..writable_unsigned_end] {
                writable.push(key.to_bytes());
            }

            for ix in &msg.instructions {
                let prog = keys[ix.program_id_index as usize];
                program_ids.push(prog.to_bytes());
            }

            for lut in &msg.address_table_lookups {
                lut_refs.push(lut.account_key.to_bytes());
            }
        }
    }

    Ok(InspectedTx { static_keys, signers, writable, program_ids, lut_refs })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(deprecated)]
    use solana_sdk::{
        hash::Hash,
        message::legacy::Message as LegacyMessage,
        pubkey::Pubkey,
        signature::Keypair,
        signer::Signer,
        transaction::VersionedTransaction,
        system_instruction,
    };

    #[allow(deprecated)]
    fn make_legacy_tx() -> String {
        let payer = Keypair::new();
        let recipient = Pubkey::new_unique();
        let ix = system_instruction::transfer(&payer.pubkey(), &recipient, 1_000_000);
        let msg = LegacyMessage::new(&[ix], Some(&payer.pubkey()));
        let tx = VersionedTransaction::from(
            solana_sdk::transaction::Transaction::new(&[&payer], msg, Hash::default())
        );
        let bytes = bincode::serialize(&tx).unwrap();
        STANDARD.encode(bytes)
    }

    #[test]
    fn test_parse_legacy_tx() {
        let b64 = make_legacy_tx();
        let result = parse_transaction(&b64).unwrap();
        assert!(!result.signers.is_empty(), "should have signers");
        assert!(!result.program_ids.is_empty(), "should have program ids");
        assert!(result.lut_refs.is_empty(), "legacy tx has no LUTs");
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

        assert_eq!(result.lut_refs.len(), 1);
        assert_eq!(result.lut_refs[0], lut_key.to_bytes());
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
}
