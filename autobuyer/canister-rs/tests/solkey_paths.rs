//! W3 (docs/v9/canister-only-solana.md §1.1, spike S3 "path style must be pinned before launch"): the canister's key map as derivation
//! paths. The byte encoding of every path is pinned here (changing it changes every address), the text form round-trips, and the IC's own
//! derivation, re-run offline with `ic-ed25519` 0.6 (S3 row 7), gives every `[b"sol", ..]` key as a child of `K_main` and the same
//! answer from the master key + canister id as from the canister-level key + chain code.
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::soltx;
use ic_ed25519::{CanisterId, DerivationIndex, DerivationPath, MasterPublicKeyId, PocketIcMasterPublicKeyId, PublicKey};

fn all() -> Vec<KeyPath> {
    vec![KeyPath::Main, KeyPath::Relay, KeyPath::Coll(0), KeyPath::Coll(3), KeyPath::Claimer, KeyPath::Coin([7; 32]), KeyPath::Desk(3),
        KeyPath::Treasury, KeyPath::Intent(7), KeyPath::Intent(u64::MAX), KeyPath::PositionNft(1)]
}

#[test]
fn the_path_encoding_is_pinned_byte_for_byte() {
    let h = |p: KeyPath| p.derivation().iter().map(hex::encode).collect::<Vec<_>>().join("/");
    // b"sol" = 736f6c; the integers fixed-width little-endian (u32 for a collection, u64 for an intent or a position)
    assert_eq!(h(KeyPath::Main), "736f6c");
    assert_eq!(h(KeyPath::Relay), "736f6c2d72656c6179");
    assert_eq!(h(KeyPath::Coll(3)), "736f6c/636f6c6c/03000000");
    assert_eq!(h(KeyPath::Claimer), "736f6c/636c61696d6572");
    assert_eq!(h(KeyPath::Coin([7; 32])), format!("736f6c/636f696e/{}", "07".repeat(32)));
    assert_eq!(h(KeyPath::Desk(258)), "736f6c/6465736b/02010000");
    assert_eq!(h(KeyPath::Treasury), "736f6c/7472656173757279");
    assert_eq!(h(KeyPath::Intent(7)), "736f6c/696e74656e74/0700000000000000");
    assert_eq!(h(KeyPath::PositionNft(1)), "736f6c/6e6674/0100000000000000");
    // S3's measured paths are exactly these: [sol, intent, 7u64 LE] and [sol, coll, 3u32 LE] (and `sol`, `sol-relay` as before)
    assert_eq!(KeyPath::Intent(7).derivation(), vec![b"sol".to_vec(), b"intent".to_vec(), 7u64.to_le_bytes().to_vec()]);
    assert_eq!(KeyPath::Coll(3).derivation(), vec![b"sol".to_vec(), b"coll".to_vec(), 3u32.to_le_bytes().to_vec()]);
    // every path is distinct
    let mut d: Vec<_> = all().iter().map(|p| p.derivation()).collect();
    d.sort();
    d.dedup();
    assert_eq!(d.len(), all().len());
}

#[test]
fn the_text_form_round_trips_and_refuses_what_it_does_not_know() {
    for p in all() {
        assert_eq!(KeyPath::parse(&p.text()).unwrap(), p, "{}", p.text());
    }
    assert_eq!(KeyPath::parse(" coll:12 ").unwrap(), KeyPath::Coll(12));
    for bad in ["", "coll", "coll:x", "coll:4294967296", "main:1", "intent:-1", "coin:notbase58!", "wallet", "nft"] {
        assert!(KeyPath::parse(bad).is_err(), "{bad}");
    }
}

fn dp(p: &[Vec<u8>]) -> DerivationPath {
    DerivationPath::new(p.iter().map(|e| DerivationIndex(e.clone())).collect())
}

/// S3 row 7 and §1.1: every address can be computed (and pinned in the launch file) before the canister runs. Two offline methods agree,
/// and each multi-element key is a child of K_main: derivable from K_main's key and chain code with the rest of its path.
#[test]
fn every_key_derives_offline_and_each_sol_key_is_a_child_of_k_main() {
    let cid = CanisterId::from_text("uxrrr-q7777-77774-qaaaq-cai").unwrap(); // the S3 replica's canister
    let (root, root_cc) = PublicKey::derive_pocketic_key(PocketIcMasterPublicKeyId::Key1, &cid, &[]);
    let (main, main_cc) = PublicKey::derive_pocketic_key(PocketIcMasterPublicKeyId::Key1, &cid, &KeyPath::Main.derivation());
    let mut seen = Vec::new();
    for p in all() {
        let path = p.derivation();
        let (a, acc) = PublicKey::derive_pocketic_key(PocketIcMasterPublicKeyId::Key1, &cid, &path);
        let (b, bcc) = root.derive_subkey_with_chain_code(&dp(&path), &root_cc);
        assert_eq!((a.serialize_raw(), acc), (b.serialize_raw(), bcc), "{}: master + canister id = canister-level key + chain code", p.text());
        if path.len() > 1 {
            let (c, _) = main.derive_subkey_with_chain_code(&dp(&path[1..]), &main_cc);
            assert_eq!(c.serialize_raw(), a.serialize_raw(), "{}: a child of K_main", p.text());
        }
        seen.push(a.serialize_raw());
        // the mainnet keys are what the ceremony pins (printed by examples/sol_keys.rs); each is a valid ed25519 point
        let (m, _) = PublicKey::derive_mainnet_key(MasterPublicKeyId::Key1, &cid, &path);
        assert!(PublicKey::deserialize_raw(&m.serialize_raw()).is_ok());
        assert_eq!(soltx::b58_encode(&m.serialize_raw()).len() >= 32, true);
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), all().len(), "every path a different address");
    // what the S3 replica's canister ANSWERED for these paths (docs/v9/spikes/s1-s3-canister-only.md §4 row 1): the offline derivation of
    // the pinned format gives the same addresses
    for (p, measured) in [(KeyPath::Main, "5YnN8RHb"), (KeyPath::Intent(7), "AvLdweiP"), (KeyPath::Coll(3), "APy8RtVb"), (KeyPath::Relay, "Hx86fhac")] {
        let (k, _) = PublicKey::derive_pocketic_key(PocketIcMasterPublicKeyId::Key1, &cid, &p.derivation());
        assert!(soltx::b58_encode(&k.serialize_raw()).starts_with(measured), "{}", p.text());
    }
    // the Relay key is a sibling of K_main, not its child (A5-03: it holds nothing K_main's chain code derives)
    let (relay, _) = PublicKey::derive_pocketic_key(PocketIcMasterPublicKeyId::Key1, &cid, &KeyPath::Relay.derivation());
    let (not_relay, _) = main.derive_subkey_with_chain_code(&dp(&[b"-relay".to_vec()]), &main_cc);
    assert_ne!(relay.serialize_raw(), not_relay.serialize_raw());
}
