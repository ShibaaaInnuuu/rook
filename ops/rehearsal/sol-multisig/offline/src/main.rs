//! Spike S3, native half. `cargo run --release -- <report.json>` (written by drive.py):
//!   1. offline derivation: every public key the canister answered is re-derived with ic-ed25519 (a) from the canister-level key +
//!      chain code (schnorr_public_key at the EMPTY path) and (b) from PocketIC's hardcoded `key_1` master key + the canister id; both
//!      compared byte for byte (key and chain code). Also: a Multi-style path is derivable from K_main's own key + chain code.
//!   2. every transaction: decoded (header, keys, instructions), and each signature verified against the key at its index with
//!      ed25519-dalek (verify_strict) and with ic-ed25519 independently.
//!   3. what the same paths give on IC MAINNET (`test_key_1` / `key_1`) for this canister id: printed, not verifiable here.
use base64::Engine;
use ic_ed25519::{CanisterId, DerivationIndex, DerivationPath, MasterPublicKeyId, PocketIcMasterPublicKeyId, PublicKey};
use serde_json::Value;

fn hx(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().expect("hex string")).expect("hex")
}

fn path_of(v: &Value) -> Vec<Vec<u8>> {
    v.as_array().expect("path array").iter().map(hx).collect()
}

fn show_path(p: &[Vec<u8>]) -> String {
    let parts: Vec<String> = p
        .iter()
        .map(|e| match std::str::from_utf8(e) {
            Ok(s) if s.chars().all(|c| c.is_ascii_graphic()) => format!("b\"{s}\""),
            _ => format!("0x{}", hex::encode(e)),
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

fn compact_u16(b: &[u8], at: &mut usize) -> usize {
    let (mut v, mut shift) = (0usize, 0);
    loop {
        let x = b[*at];
        *at += 1;
        v |= ((x & 0x7f) as usize) << shift;
        if x & 0x80 == 0 {
            return v;
        }
        shift += 7;
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: rook_sol_multisig_offline <report.json>");
    let r: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut fails = 0;
    let mut check = |ok: bool, what: String| {
        println!("{} {what}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            fails += 1;
        }
    };

    // ---- 1. offline derivation ----
    let cid = CanisterId::from_text(r["canister_id"].as_str().unwrap()).unwrap();
    let root_pk = PublicKey::deserialize_raw(&hx(&r["root"]["public_key"])).unwrap();
    let root_cc: [u8; 32] = hx(&r["root"]["chain_code"]).try_into().unwrap();
    let (pic_root, pic_root_cc) = PublicKey::derive_pocketic_key(PocketIcMasterPublicKeyId::Key1, &cid, &[]);
    check(
        pic_root == root_pk && pic_root_cc == root_cc,
        format!("canister-level key (empty path) = derive_pocketic_key(Key1, {cid}, []) [key and chain code]"),
    );
    let mut sol_key: Option<(PublicKey, [u8; 32])> = None;
    for k in r["keys"].as_array().unwrap() {
        let p = path_of(&k["path"]);
        let want_pk = hx(&k["public_key"]);
        let want_cc = hx(&k["chain_code"]);
        let dp = DerivationPath::new(p.iter().map(|e| DerivationIndex(e.clone())).collect());
        let (a, acc) = root_pk.derive_subkey_with_chain_code(&dp, &root_cc);
        let (b, bcc) = PublicKey::derive_pocketic_key(PocketIcMasterPublicKeyId::Key1, &cid, &p);
        let addr = bs58::encode(&want_pk).into_string();
        check(
            a.serialize_raw().to_vec() == want_pk && acc.to_vec() == want_cc,
            format!("{} {addr}: from the canister-level key + chain code (derive_subkey_with_chain_code)", show_path(&p)),
        );
        check(
            b.serialize_raw().to_vec() == want_pk && bcc.to_vec() == want_cc,
            format!("{} {addr}: from PocketIC's key_1 master + canister id (derive_pocketic_key)", show_path(&p)),
        );
        if p == vec![b"sol".to_vec()] {
            sol_key = Some((PublicKey::deserialize_raw(&want_pk).unwrap(), want_cc.try_into().unwrap()));
        }
        let (m1, _) = PublicKey::derive_mainnet_key(MasterPublicKeyId::TestKey1, &cid, &p);
        let (m2, _) = PublicKey::derive_mainnet_key(MasterPublicKeyId::Key1, &cid, &p);
        println!(
            "     (if this canister id were on IC mainnet: test_key_1 → {}, key_1 → {})",
            bs58::encode(m1.serialize_raw()).into_string(),
            bs58::encode(m2.serialize_raw()).into_string()
        );
    }
    // hierarchical: a Multi path [sol, x, y] from K_main's key + chain code, with only the remaining elements
    if let Some((spk, scc)) = sol_key {
        for k in r["keys"].as_array().unwrap() {
            let p = path_of(&k["path"]);
            if p.len() > 1 && p[0] == b"sol" {
                let rest = DerivationPath::new(p[1..].iter().map(|e| DerivationIndex(e.clone())).collect());
                let (c, _) = spk.derive_subkey_with_chain_code(&rest, &scc);
                check(
                    c.serialize_raw().to_vec() == hx(&k["public_key"]),
                    format!("{} derivable from K_main ([b\"sol\"]) key + chain code with the rest of the path", show_path(&p)),
                );
            }
        }
    }

    // ---- 2. the transactions ----
    let b64 = base64::engine::general_purpose::STANDARD;
    for t in r["txs"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        let tx = b64.decode(t["tx_base64"].as_str().unwrap()).unwrap();
        let mut at = 0;
        let nsig = compact_u16(&tx, &mut at);
        let sigs: Vec<[u8; 64]> = (0..nsig).map(|i| tx[at + 64 * i..at + 64 * (i + 1)].try_into().unwrap()).collect();
        at += 64 * nsig;
        let msg = &tx[at..];
        let (h0, h1, h2) = (msg[0], msg[1], msg[2]);
        let mut m = 3;
        let nkeys = compact_u16(msg, &mut m);
        let keys: Vec<[u8; 32]> = (0..nkeys).map(|i| msg[m + 32 * i..m + 32 * (i + 1)].try_into().unwrap()).collect();
        m += 32 * nkeys;
        let blockhash = bs58::encode(&msg[m..m + 32]).into_string();
        m += 32;
        let nix = compact_u16(msg, &mut m);
        let mut ixs = Vec::new();
        for _ in 0..nix {
            let prog = msg[m] as usize;
            m += 1;
            let na = compact_u16(msg, &mut m);
            let accts: Vec<usize> = msg[m..m + na].iter().map(|&x| x as usize).collect();
            m += na;
            let nd = compact_u16(msg, &mut m);
            let data = &msg[m..m + nd];
            m += nd;
            ixs.push(format!(
                "{}({} accts, data {})",
                &bs58::encode(keys[prog]).into_string()[..8],
                accts.len(),
                if data.len() <= 12 { hex::encode(data) } else { format!("{} bytes", data.len()) }
            ));
        }
        check(m == msg.len(), format!("{name}: message fully consumed ({} bytes)", msg.len()));
        check(
            nsig == 3 && h0 == 3,
            format!("{name}: {} bytes; {nsig} signatures; header ({h0}, {h1}, {h2}); {nkeys} keys; blockhash/nonce {blockhash}", tx.len()),
        );
        println!("     instructions: {}", ixs.join(" · "));
        let roles = t["signers"].as_array().unwrap();
        for i in 0..nsig {
            let want = roles[i]["address"].as_str().unwrap();
            let key = bs58::encode(keys[i]).into_string();
            let vk = ed25519_dalek::VerifyingKey::from_bytes(&keys[i]).unwrap();
            let dalek = vk.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(&sigs[i])).is_ok();
            let icv = PublicKey::deserialize_raw(&keys[i]).unwrap().verify_signature(msg, &sigs[i]).is_ok();
            check(
                key == want && dalek && icv,
                format!(
                    "{name}: sig[{i}] {} by {} {key} (expected {want}): ed25519-dalek verify_strict {dalek}, ic-ed25519 {icv}",
                    &bs58::encode(sigs[i]).into_string()[..16],
                    roles[i]["role"].as_str().unwrap()
                ),
            );
        }
        if let Some(sig58) = t.get("landed").and_then(|v| v.as_str()) {
            check(sig58 == bs58::encode(sigs[0]).into_string(), format!("{name}: the landed signature is sig[0] (K_main's)"));
        }
    }
    println!("{}", if fails == 0 { "ALL PASS" } else { "SOME FAILED" });
    std::process::exit(if fails == 0 { 0 } else { 1 });
}
