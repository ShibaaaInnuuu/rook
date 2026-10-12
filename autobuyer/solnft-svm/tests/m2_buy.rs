//! Spike S5 (docs/v9/spikes/s5-magic-eden-m2.md): every recorded Magic Eden M2 listing of the 12 collections is bought by the canister's
//! own buyer-only purchase (`solnftm2::buy_instructions`, no notary signature) on M2's mainnet bytes, and the canister's judge accepts
//! exactly that purchase and refuses it under a limit one lamport too low.
mod common;
use common::*;
use etch_autobuyer::solnft::{self, Standard};
use etch_autobuyer::solnftbuy::{self as buy, Candidate, Expect, Venue};
use etch_autobuyer::solnftm2 as m2;

fn names() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../canister-rs/tests/fixtures/solnft");
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("m2_") && n != "m2_pnft_mad_lads_wallet.json").map(|n| n.trim_end_matches(".json").to_string()).collect();
    v.sort();
    v
}

#[test]
fn every_recorded_m2_listing_is_bought_without_the_notary_and_judged() {
    let all = names();
    assert!(all.len() >= 22, "{}", all.len());
    for n in all {
        let f = fixture(&n);
        let view = f.view();
        let asset = f.fact("asset");
        let coll = f.fact("collection");
        let member = solnft::is_member(&asset, &coll, Standard::Legacy, &view).expect("member");
        let ask = buy::verify_ask(&Candidate { venue: Venue::M2, listing: f.fact("seller_trade_state"), asset }, &member, &view, f.unix_time)
            .unwrap_or_else(|e| panic!("{n}: {e}"));
        let mut e = env(&f);
        let me = e.me();
        let ixs = m2::buy_instructions(&me, &ask, &member).unwrap();
        let all_in = m2::m2_total(ask.price, &member) + ask.rent(&member);
        let ok = Expect { payer: me, ask: &ask, member: &member, collection: coll, limit: all_in };
        m2::judge(&ixs, &ok).unwrap_or_else(|x| panic!("{n}: judge: {x}"));
        assert!(m2::judge(&ixs, &Expect { limit: all_in - 1, ..ok }).is_err(), "{n}: a limit under the all-in price is refused");
        e.run(&ixs, &[]).unwrap_or_else(|x| panic!("{n}: did not land: {}", x.lines().next().unwrap_or("")));
    }
}
