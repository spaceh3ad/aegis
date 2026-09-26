use aegis_core::{parse_transaction, Blocklist};
use base64::{engine::general_purpose::STANDARD, Engine};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use solana_sdk::{
    message::{v0, MessageHeader, VersionedMessage},
    pubkey::Pubkey,
    signature::Signature,
    transaction::VersionedTransaction,
};
use std::{io::Write, path::PathBuf};

fn make_v0_tx(keys: &[Pubkey]) -> String {
    let msg = v0::Message {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 0,
        },
        account_keys: keys.to_vec(),
        recent_blockhash: solana_sdk::hash::Hash::default(),
        instructions: vec![],
        address_table_lookups: vec![],
    };
    let tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::V0(msg),
    };
    STANDARD.encode(bincode::serialize(&tx).unwrap())
}

fn write_temp_blocklist(keys: &[&str]) -> (tempfile_helper::TempFile, PathBuf) {
    let path = std::env::temp_dir().join(format!("aegis_bench_{}.txt", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    for k in keys {
        writeln!(f, "{}", k).unwrap();
    }
    (tempfile_helper::TempFile(path.clone()), path)
}

mod tempfile_helper {
    use std::path::PathBuf;
    pub struct TempFile(pub PathBuf);
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn bench_parse_transaction(c: &mut Criterion) {
    let keys: Vec<Pubkey> = (0..4).map(|_| Pubkey::new_unique()).collect();
    let b64 = make_v0_tx(&keys);

    c.bench_function("parse_transaction/v0_4keys", |b| {
        b.iter(|| parse_transaction(black_box(&b64)).unwrap())
    });

    let many_keys: Vec<Pubkey> = (0..16).map(|_| Pubkey::new_unique()).collect();
    let b64_large = make_v0_tx(&many_keys);

    c.bench_function("parse_transaction/v0_16keys", |b| {
        b.iter(|| parse_transaction(black_box(&b64_large)).unwrap())
    });
}

fn bench_blocklist_check(c: &mut Criterion) {
    let clean_keys: Vec<String> = (0..4).map(|_| Pubkey::new_unique().to_string()).collect();
    let blocked = Pubkey::new_unique();
    let blocked_str = blocked.to_string();

    let (_tmp, path) = write_temp_blocklist(&[&blocked_str]);
    let blocklist = Blocklist::load_from_file(&path).unwrap();

    let clean_tx = make_v0_tx(&clean_keys.iter().map(|s| s.parse().unwrap()).collect::<Vec<_>>());
    let blocked_tx = make_v0_tx(&[blocked]);

    let clean_inspected = parse_transaction(&clean_tx).unwrap();
    let blocked_inspected = parse_transaction(&blocked_tx).unwrap();

    c.bench_function("blocklist_check/allow", |b| {
        b.iter(|| blocklist.check(black_box(&clean_inspected)))
    });

    c.bench_function("blocklist_check/block", |b| {
        b.iter(|| blocklist.check(black_box(&blocked_inspected)))
    });
}

fn bench_full_pipeline(c: &mut Criterion) {
    let blocked = Pubkey::new_unique();
    let blocked_str = blocked.to_string();
    let clean_keys: Vec<Pubkey> = (0..4).map(|_| Pubkey::new_unique()).collect();

    let (_tmp, path) = write_temp_blocklist(&[&blocked_str]);
    let blocklist = Blocklist::load_from_file(&path).unwrap();

    let clean_b64 = make_v0_tx(&clean_keys);
    let blocked_b64 = make_v0_tx(&[blocked]);

    c.bench_function("full_pipeline/allow", |b| {
        b.iter(|| {
            let tx = parse_transaction(black_box(&clean_b64)).unwrap();
            blocklist.check(black_box(&tx))
        })
    });

    c.bench_function("full_pipeline/block", |b| {
        b.iter(|| {
            let tx = parse_transaction(black_box(&blocked_b64)).unwrap();
            blocklist.check(black_box(&tx))
        })
    });
}

criterion_group!(
    benches,
    bench_parse_transaction,
    bench_blocklist_check,
    bench_full_pipeline
);
criterion_main!(benches);
