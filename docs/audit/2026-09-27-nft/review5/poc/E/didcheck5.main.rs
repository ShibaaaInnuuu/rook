//! Review round 5, part E: the .did at 1dc2a61 against the canister's own encoder and the old .did. Run as src/main.rs of a crate
//! next to a private copy of autobuyer/ (Cargo.toml: candid =0.10.35, candid_parser =0.1.4, hex, etch_autobuyer by path), with the
//! 2735dac .did as ./old.did and the round-2 fixture's old encodings in ./r2/.
use candid::types::Type;
use candid::IDLArgs;
use candid_parser::utils::{service_compatible, CandidSource};
use etch_autobuyer::ethflow::*;

fn main() {
    let root = "../autobuyer/canister-rs";
    let did = std::fs::read_to_string(format!("{root}/etch_autobuyer.did")).unwrap();
    let old = std::fs::read_to_string("old.did").unwrap();
    println!("service_compatible(new .did, 2735dac .did): {:?}", service_compatible(CandidSource::Text(&did), CandidSource::Text(&old)).map_err(|e| e.to_string()));
    let (env, _) = CandidSource::Text(&did).load().expect("the .did parses");
    let t = Type::from(candid::types::TypeInner::Var("EthState".into()));
    let check = |b: &[u8], what: &str| match IDLArgs::from_bytes_with_types(b, &env, &[t.clone()]) {
        Ok(a) => {
            let again = a.to_bytes_with_types(&env, &[t.clone()]).unwrap();
            let lost = etch_autobuyer::canister::lost_opts(b, &again).unwrap();
            println!("{}: {what}: {} bytes; opts the .did loses: {lost:?}", if lost.is_empty() { "ok" } else { "MISMATCH" }, b.len());
        }
        Err(e) => println!("MISMATCH: {what}: {e}"),
    };
    for (what, path) in [("eth-state-round4.hex (HEAD)", format!("{root}/tests/fixtures/eth-state-round4.hex")),
        ("eth-state-round2.hex (HEAD)", format!("{root}/tests/fixtures/eth-state-round2.hex")),
        ("eth-state-round2 as 24fba50 wrote it", "r2/24fba50.hex".into()), ("eth-state-round2 as fb54018 wrote it", "r2/fb54018.hex".into())] {
        check(&hex::decode(std::fs::read_to_string(path).unwrap().trim()).unwrap(), what);
    }
    // the 1eb5788 build's saved Ethereum side, re-encoded by this build (what eth_nft_state returns after the upgrade)
    let bytes = hex::decode(std::fs::read_to_string(format!("{root}/tests/fixtures/state/saved-1eb5788-rich.hex")).unwrap().trim()).unwrap();
    let saved: etch_autobuyer::canister::Saved = candid::decode_args::<(etch_autobuyer::canister::Saved,)>(&bytes).unwrap().0;
    check(&candid::encode_one(saved.12.unwrap()).unwrap(), "the 1eb5788 state's Ethereum side after the upgrade");
    // every round-4 field present, including the one eth-state-round4.hex leaves out (Inbox.payouts)
    let fx: EthState = candid::decode_one(&hex::decode(std::fs::read_to_string(format!("{root}/tests/fixtures/eth-state-round4.hex")).unwrap().trim()).unwrap()).unwrap();
    println!("eth-state-round4.hex: inbox.payouts {:?}, a buy's late {}, pending.used_since {:?}, sale_scan bad/skipped {:?}",
        fx.inbox.payouts, fx.buys.iter().any(|b| b.late.is_some()), fx.pending.as_ref().and_then(|p| p.used_since),
        fx.sale_scan.as_ref().map(|v| (v[0].bad.is_some(), v[0].skipped.is_some())));
    let mut s = fx.clone();
    s.inbox.payouts = Some(vec![PayoutClaim { buy: 1, home: false, tx: [3; 32], wei: 5, at: 6 }]);
    check(&candid::encode_one(&s).unwrap(), "round 4's fixture state with a payout claim in the inbox");
}
