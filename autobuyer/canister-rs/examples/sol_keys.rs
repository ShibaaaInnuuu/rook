//! W3: the canister's Solana addresses, derived OFFLINE (docs/v9/canister-only-solana.md §1.1, spike S3 row 7), for the ceremony scripts
//! and the launch file. The same derivation the IC runs for `schnorr_public_key(ed25519)`: `ic-ed25519` 0.6, with the canister id as the
//! first path element and `solkey::KeyPath::derivation` (the pinned format) for the rest. Native only (a dev-dependency; not in the wasm).
//!
//!   cargo run --example sol_keys -- --canister <principal> [--key key_1|test_key_1|pocketic:key_1|pocketic:test_key_1] [--colls N] [PATH...]
//!   cargo run --example sol_keys -- --root <canister-level key hex> <chain code hex> [--colls N] [PATH...]
//!
//! PATH is `KeyPath::parse`'s form: main, relay, claimer, treasury, coll:<i>, desk:<i>, coin:<mint>, intent:<n>, nft:<n>. With no PATH:
//! main, relay, claimer, treasury, and coll/desk for 0..N (`--colls`, default 5). One JSON object per line: the path, its elements (hex)
//! and the address (base58). `--root` takes `schnorr_public_key` at the EMPTY path (key and chain code), e.g. from a local replica, and
//! must give the same answers as `--canister` with that replica's key: compare with the canister's `sol_address_of` before pinning.
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::soltx;
use ic_ed25519::{CanisterId, DerivationIndex, DerivationPath, MasterPublicKeyId, PocketIcMasterPublicKeyId, PublicKey};

enum Source {
    Mainnet(MasterPublicKeyId, CanisterId),
    PocketIc(PocketIcMasterPublicKeyId, CanisterId),
    Root(PublicKey, [u8; 32]),
}

fn derive(src: &Source, path: &[Vec<u8>]) -> [u8; 32] {
    match src {
        Source::Mainnet(k, c) => PublicKey::derive_mainnet_key(*k, c, path).0.serialize_raw(),
        Source::PocketIc(k, c) => PublicKey::derive_pocketic_key(*k, c, path).0.serialize_raw(),
        Source::Root(pk, cc) => {
            pk.derive_subkey_with_chain_code(&DerivationPath::new(path.iter().map(|e| DerivationIndex(e.clone())).collect()), cc).0.serialize_raw()
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut canister, mut key, mut root, mut colls, mut paths) = (None, "key_1".to_string(), None, 5u32, Vec::new());
    let mut i = 0;
    let usage = "usage: sol_keys (--canister <principal> [--key key_1|test_key_1|pocketic:key_1|pocketic:test_key_1] | --root <key hex> <chain code hex>) [--colls N] [PATH...]";
    while i < args.len() {
        match args[i].as_str() {
            "--canister" => { canister = Some(args.get(i + 1).expect(usage).clone()); i += 2; }
            "--key" => { key = args.get(i + 1).expect(usage).clone(); i += 2; }
            "--root" => { root = Some((args.get(i + 1).expect(usage).clone(), args.get(i + 2).expect(usage).clone())); i += 3; }
            "--colls" => { colls = args.get(i + 1).expect(usage).parse().expect("--colls N"); i += 2; }
            p => { paths.push(KeyPath::parse(p).unwrap_or_else(|e| panic!("{e}"))); i += 1; }
        }
    }
    let src = match (root, canister) {
        (Some((k, c)), _) => {
            let pk = PublicKey::deserialize_raw(&hex::decode(k).expect("key hex")).expect("an ed25519 key");
            let cc: [u8; 32] = hex::decode(c).expect("chain code hex").try_into().expect("32 bytes of chain code");
            Source::Root(pk, cc)
        }
        (None, Some(c)) => {
            let cid = CanisterId::from_text(&c).expect("a canister principal");
            match key.as_str() {
                "key_1" => Source::Mainnet(MasterPublicKeyId::Key1, cid),
                "test_key_1" => Source::Mainnet(MasterPublicKeyId::TestKey1, cid),
                "pocketic:key_1" => Source::PocketIc(PocketIcMasterPublicKeyId::Key1, cid),
                "pocketic:test_key_1" => Source::PocketIc(PocketIcMasterPublicKeyId::TestKey1, cid),
                k => panic!("unknown key {k}: {usage}"),
            }
        }
        (None, None) => panic!("{usage}"),
    };
    if paths.is_empty() {
        paths = vec![KeyPath::Main, KeyPath::Relay, KeyPath::Claimer, KeyPath::Treasury];
        for c in 0..colls {
            paths.push(KeyPath::Coll(c));
            paths.push(KeyPath::Desk(c));
        }
    }
    for p in paths {
        let el = p.derivation();
        let addr = soltx::b58_encode(&derive(&src, &el));
        let hexes: Vec<String> = el.iter().map(|e| format!("\"{}\"", hex::encode(e))).collect();
        println!("{{\"path\":\"{}\",\"elements\":[{}],\"address\":\"{addr}\"}}", p.text(), hexes.join(","));
    }
}
