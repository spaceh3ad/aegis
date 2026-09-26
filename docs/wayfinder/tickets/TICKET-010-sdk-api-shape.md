# TICKET-010: solana-sdk deserialization API shape

**Type:** `wayfinder:research`
**Status:** closed

## Question

What is the exact Rust API for deserializing a base64-encoded `sendTransaction` payload?

## Resolution

**Crate:** `solana-sdk` (Agave 2.x compatible). Deserialize chain:

```rust
use solana_sdk::transaction::VersionedTransaction;
use base64::engine::general_purpose::STANDARD;
use bincode::deserialize;

let decoded = STANDARD.decode(&b64_payload)?;
let tx: VersionedTransaction = deserialize(&decoded)?;
```

**Field extraction (v0):**
```rust
match &tx.message {
    VersionedMessage::V0(msg) => {
        // Signers: first num_required_signers keys
        let signers = &msg.account_keys[..msg.header.num_required_signers as usize];
        // Program IDs
        let program_ids: Vec<Pubkey> = msg.instructions.iter()
            .map(|ix| msg.account_keys[ix.program_id_index as usize]).collect();
        // LUT entries for best-effort expansion
        let luts: &[MessageAddressTableLookup] = &msg.address_table_lookups;
        // Each: { account_key, writable_indexes: Vec<u8>, readonly_indexes: Vec<u8> }
    }
    VersionedMessage::Legacy(msg) => { /* same header fields, no address_table_lookups */ }
}
```
