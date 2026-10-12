//! The classic judge on a real, chain-valid marketplace bid PSBT (tests/fixtures/unisat/regtest-classic-bid-2026-10-09.json):
//! built on regtest by ops/rehearsal/sim/market.mjs, the seller's input finalized by the seller's own wallet with
//! SIGHASH_SINGLE|ANYONECANPAY. The same PSBT, signed by the sim's float key, was accepted by bitcoind and confirmed. UniSat's
//! own `create_bid` PSBT could not be obtained on testnet (fixtures/unisat/testnet-probe-2026-10-09.json): when it is, drop it in
//! beside this one and run the same two checks.

use etch_autobuyer::custody::Outpoint;
use etch_autobuyer::frogbuy::{self, Facts, Seller};
use etch_autobuyer::psbt::{self, Psbt};

struct Fx {
    psbt: Vec<u8>,
    to_sign: Vec<usize>,
    float: Vec<u8>,
    custody: Vec<u8>,
    seller: Seller,
    cap: u64,
}

fn fixture() -> Fx {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/unisat/regtest-classic-bid-2026-10-09.json")).unwrap();
    let h = |k: &str| hex::decode(v[k].as_str().unwrap()).unwrap();
    let s = &v["seller"];
    Fx {
        psbt: h("psbt_hex"),
        to_sign: v["to_sign"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as usize).collect(),
        float: h("float_script"),
        custody: h("custody_script"),
        seller: Seller {
            outpoint: Outpoint { txid: hex::decode(s["txid"].as_str().unwrap()).unwrap().try_into().unwrap(), vout: s["vout"].as_u64().unwrap() as u32 },
            offset: s["offset"].as_u64().unwrap(),
            value: s["value"].as_u64().unwrap(),
        },
        cap: v["cap_sats"].as_u64().unwrap(),
    }
}

#[test]
fn a_seller_signed_padded_bid_passes_the_classic_judge_and_finalizes() {
    let f = fixture();
    let p = Psbt::parse(&f.psbt).unwrap();
    let w = psbt::final_witnesses(&f.psbt).unwrap();
    assert_eq!(w.iter().map(|x| x.is_some()).collect::<Vec<_>>(), vec![false, false, true, false], "only the seller's input is final");
    let seller_sig = &w[2].as_ref().unwrap()[0];
    assert_eq!((seller_sig.len(), seller_sig[64]), (65, 0x83), "one 65-byte SIGHASH_SINGLE|ANYONECANPAY key-path signature");
    let j = frogbuy::judge(&p, &w, &f.to_sign, &Facts { float: &f.float, custody: &f.custody, seller: f.seller.clone(), cap_sats: f.cap }).unwrap();
    assert_eq!((j.ours, j.custody_vout, j.spend, j.fee), (vec![0, 1, 3], 1, 201_500, 1_500));
    let sigs: Vec<(usize, Vec<u8>)> = j_ours().into_iter().map(|i| (i, vec![0x11; 64])).collect();
    assert!(psbt::finalize(&p, &w, &sigs).is_ok());
}

fn j_ours() -> Vec<usize> {
    vec![0, 1, 3]
}

/// The same bid with the seller's witness stripped — what a marketplace that keeps the seller's signature server-side (and
/// merges it at `confirm_bid`) would hand over. Before review S-01 the judge passed it and only `finalize`, after the float had
/// signed, refused it (simulation finding; see tests/handins.rs `a_hand_in_without_the_sellers_witness_…`). Now the judge refuses
/// it as `NotFinal`, before any digest is computed; `finalize` still refuses it too (defense in depth).
#[test]
fn the_same_bid_without_the_sellers_witness_is_refused_by_the_judge_and_by_finalize() {
    let f = fixture();
    let p = Psbt::parse(&f.psbt).unwrap();
    let stripped = frogbuy::psbt_bytes(&p, &vec![None; p.tx.inputs.len()]);
    let p2 = Psbt::parse(&stripped).unwrap();
    let w = psbt::final_witnesses(&stripped).unwrap();
    assert!(w.iter().all(|x| x.is_none()));
    let j = frogbuy::judge(&p2, &w, &f.to_sign, &Facts { float: &f.float, custody: &f.custody, seller: f.seller.clone(), cap_sats: f.cap });
    assert_eq!(j, Err(frogbuy::Refused::NotFinal(2)), "the seller's input is not final");
    let sigs: Vec<(usize, Vec<u8>)> = j_ours().into_iter().map(|i| (i, vec![0x11; 64])).collect();
    let e = psbt::finalize(&p2, &w, &sigs).unwrap_err();
    assert!(e.contains("input 2 is not ours and the PSBT carries no witness"), "{e}");
}
