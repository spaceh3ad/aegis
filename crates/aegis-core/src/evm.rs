use alloy_consensus::{transaction::SignerRecoverable, Transaction, TxEnvelope};
use alloy_eips::eip2718::{Decodable2718, Typed2718};
use thiserror::Error;

use crate::types::EvmTx;

/// Raw-tx bodies above this size are rejected before RLP decoding (EIP-4844 blobs travel separately).
pub const MAX_RAW_TX_BYTES: usize = 128 * 1024;

#[derive(Debug, Error)]
pub enum EvmParseError {
    #[error("missing 0x prefix")]
    MissingPrefix,
    #[error("raw transaction exceeds {MAX_RAW_TX_BYTES} bytes")]
    TooLarge,
    #[error("invalid hex: {0}")]
    InvalidHex(#[from] hex::FromHexError),
    #[error("invalid EIP-2718 envelope: {0}")]
    Decode(#[from] alloy_eips::eip2718::Eip2718Error),
    #[error("signer recovery failed")]
    Recovery,
}

pub fn parse_evm_transaction(raw_hex: &str) -> Result<EvmTx, EvmParseError> {
    let hex_body = raw_hex
        .strip_prefix("0x")
        .or_else(|| raw_hex.strip_prefix("0X"))
        .ok_or(EvmParseError::MissingPrefix)?;
    if hex_body.len() > MAX_RAW_TX_BYTES * 2 {
        return Err(EvmParseError::TooLarge);
    }
    let bytes = hex::decode(hex_body)?;
    let envelope = TxEnvelope::decode_2718_exact(&bytes)?;
    let from = envelope
        .recover_signer()
        .map_err(|_| EvmParseError::Recovery)?;

    let input = envelope.input();
    let selector = (input.len() >= 4).then(|| input[..4].try_into().unwrap());

    Ok(EvmTx {
        chain_id: envelope.chain_id(),
        from: from.into_array(),
        to: envelope.to().map(|a| a.into_array()),
        selector,
        gas_limit: envelope.gas_limit(),
        priority_fee: envelope.priority_fee_or_price(),
        value_wei: envelope.value().to_string(),
        tx_type: envelope.ty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxEip1559, TxLegacy};
    use alloy_eips::eip2718::Encodable2718;
    use alloy_primitives::{address, Address, Bytes, TxKind, U256};
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;

    fn signer() -> PrivateKeySigner {
        "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7"
            .parse()
            .unwrap()
    }

    pub(crate) fn raw_1559(
        signer: &PrivateKeySigner,
        to: Address,
        input: Vec<u8>,
        tip: u128,
    ) -> String {
        let tx = TxEip1559 {
            chain_id: 8453,
            nonce: 7,
            gas_limit: 90_000,
            max_fee_per_gas: 2_000_000_000,
            max_priority_fee_per_gas: tip,
            to: TxKind::Call(to),
            value: U256::from(1_000_000_000_000_000_000u128),
            access_list: Default::default(),
            input: Bytes::from(input),
        };
        let sig = signer.sign_hash_sync(&tx.signature_hash()).unwrap();
        let env: TxEnvelope = tx.into_signed(sig).into();
        format!("0x{}", hex::encode(env.encoded_2718()))
    }

    #[test]
    fn test_parse_eip1559() {
        let s = signer();
        let to = address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045");
        let raw = raw_1559(&s, to, vec![0xa9, 0x05, 0x9c, 0xbb, 0, 0], 1_500_000_000);
        let tx = parse_evm_transaction(&raw).unwrap();
        assert_eq!(tx.from, s.address().into_array());
        assert_eq!(tx.to, Some(to.into_array()));
        assert_eq!(tx.selector, Some([0xa9, 0x05, 0x9c, 0xbb]));
        assert_eq!(tx.chain_id, Some(8453));
        assert_eq!(tx.gas_limit, 90_000);
        assert_eq!(tx.priority_fee, 1_500_000_000);
        assert_eq!(tx.value_wei, "1000000000000000000");
        assert_eq!(tx.tx_type, 2);
    }

    #[test]
    fn test_parse_legacy_contract_creation() {
        let s = signer();
        let tx = TxLegacy {
            chain_id: Some(1),
            nonce: 0,
            gas_price: 30_000_000_000,
            gas_limit: 1_000_000,
            to: TxKind::Create,
            value: U256::ZERO,
            input: Bytes::from(vec![0x60, 0x80]),
        };
        let sig = s.sign_hash_sync(&tx.signature_hash()).unwrap();
        let env: TxEnvelope = tx.into_signed(sig).into();
        let raw = format!("0x{}", hex::encode(env.encoded_2718()));

        let parsed = parse_evm_transaction(&raw).unwrap();
        assert_eq!(parsed.from, s.address().into_array());
        assert_eq!(parsed.to, None);
        assert_eq!(parsed.selector, None);
        assert_eq!(parsed.priority_fee, 30_000_000_000);
        assert_eq!(parsed.tx_type, 0);
    }

    #[test]
    fn test_rejects_bad_input() {
        assert!(matches!(
            parse_evm_transaction("deadbeef"),
            Err(EvmParseError::MissingPrefix)
        ));
        assert!(matches!(
            parse_evm_transaction("0xzz"),
            Err(EvmParseError::InvalidHex(_))
        ));
        assert!(matches!(
            parse_evm_transaction("0x02c0"),
            Err(EvmParseError::Decode(_))
        ));
        let huge = format!("0x{}", "00".repeat(MAX_RAW_TX_BYTES + 1));
        assert!(matches!(
            parse_evm_transaction(&huge),
            Err(EvmParseError::TooLarge)
        ));
    }

    #[test]
    fn test_trailing_bytes_rejected() {
        let raw = raw_1559(&signer(), Address::ZERO, vec![], 1);
        assert!(parse_evm_transaction(&format!("{raw}00")).is_err());
    }

    mod fuzz {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
                let _ = parse_evm_transaction(&format!("0x{}", hex::encode(&bytes)));
            }

            #[test]
            fn never_panics_on_typed_prefix(ty in 0u8..5, body in proptest::collection::vec(any::<u8>(), 0..256)) {
                let mut bytes = vec![ty];
                bytes.extend(body);
                let _ = parse_evm_transaction(&format!("0x{}", hex::encode(&bytes)));
            }

            // Round-trip invariant: any signed tx decodes back to its signer, target and selector.
            #[test]
            fn roundtrip_recovers_signer(
                key in proptest::array::uniform32(1u8..),
                to in proptest::array::uniform20(any::<u8>()),
                input in proptest::collection::vec(any::<u8>(), 0..64),
                tip in any::<u64>(),
            ) {
                let Ok(s) = PrivateKeySigner::from_slice(&key) else { return Ok(()); };
                let raw = raw_1559(&s, Address::from(to), input.clone(), tip as u128);
                let tx = parse_evm_transaction(&raw).unwrap();
                prop_assert_eq!(tx.from, s.address().into_array());
                prop_assert_eq!(tx.to, Some(to));
                prop_assert_eq!(tx.priority_fee, tip as u128);
                let expected_sel: Option<[u8; 4]> = (input.len() >= 4).then(|| input[..4].try_into().unwrap());
                prop_assert_eq!(tx.selector, expected_sel);
            }
        }
    }
}
