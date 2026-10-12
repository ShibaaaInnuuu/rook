//! Audit B2 (docs/v9/audit-2026-10/b2-primitives.md): proofs of concept against the marketplace programs' mainnet bytes. The `poc_`
//! test PASSES by exhibiting the bug, so the suite stays green until the fix lands; its comment says what a fix must refuse. The
//! `checked_` test pins a rule the audit verified on the program's own bytes.
mod common;

use common::*;
use etch_autobuyer::solnftbuy::{self as buy, Candidate, Expect, Venue};
use etch_autobuyer::solnft::{self, Standard};
use etch_autobuyer::soltx;

const FEE: u64 = 5_000;
/// `ListState`: discriminator 8, version 1, bump 1, owner 32, asset 32, then `amount: u64`.
const LIST_STATE_AMOUNT_AT: usize = 8 + 1 + 1 + 32 + 32;

struct Judged {
    env: Env,
    ask: buy::Ask,
    member: solnft::Member,
    ixs: Vec<soltx::Instruction>,
    limit: u64,
}

/// The canister's side of a Tensor purchase, exactly as `buy_send.rs` does it: verify at the recorded slot, build, judge at a limit
/// equal to the ask (total + rent), one lamport under refused.
fn judged(name: &str) -> Judged {
    let f = fixture(name);
    let e = env(&f);
    let coll = f.fact("collection");
    let std = match f.facts["standard"].as_str().unwrap() {
        "legacy" => Standard::Legacy,
        "pnft" => Standard::Programmable,
        _ => Standard::Core,
    };
    let asset = f.fact("asset");
    let view = f.view();
    let member = solnft::is_member(&asset, &coll, std, &view).expect("a member");
    let ask = buy::verify_ask(&Candidate { venue: Venue::Tensor, listing: f.fact("list_state"), asset }, &member, &view, f.unix_time).expect("live");
    let limit = ask.total + ask.rent(&member);
    let ixs = buy::buy_instructions(&e.me(), &ask, &member).unwrap();
    let expect = Expect { payer: e.me(), ask: &ask, member: &member, collection: coll, limit };
    buy::judge(&ixs, &expect).expect("the canonical purchase passes");
    assert!(buy::judge(&ixs, &Expect { limit: limit - 1, ..expect }).is_err());
    Judged { env: e, ask, member, ixs, limit }
}

/// B2-01. The judge skips every Compute Budget instruction unread (J1 `continue`), and `judge_send` and `solnftm2::judge` do the same. A
/// `SetComputeUnitPrice` is a Compute Budget instruction whose fee the payer (the canister's `K_main`) pays on top of everything J3
/// bounds: `limit` × `price` / 10⁶ lamports. A fix must allow `SetComputeUnitLimit` only (or pin the price and count its fee in J3).
/// FIXED B2-01: the judge refuses a `SetComputeUnitPrice`; the canonical purchase (its `SetComputeUnitLimit` kept) is judged and lands
/// within its limit on Tensor's mainnet bytes.
#[test]
fn fixed_b2_01_the_judge_refuses_a_compute_unit_price_and_the_canonical_purchase_lands_within_its_limit() {
    let mut j = judged("tensor_legacy_fff");
    const PRICE: u64 = 25_000_000_000;
    let mut ixs = vec![soltx::compute_unit_price(PRICE)];
    ixs.extend(j.ixs.iter().cloned());
    let expect = Expect { payer: j.env.me(), ask: &j.ask, member: &j.member, collection: j.member.collection, limit: j.limit };
    let e = buy::judge(&ixs, &expect).expect_err("FIXED B2-01: refused");
    assert!(e.contains("SetComputeUnitPrice"), "{e}");
    buy::judge(&j.ixs, &expect).expect("the canonical purchase passes");
    let me = j.env.me();
    let before = j.env.lamports(&me);
    j.env.run(&j.ixs, &[]).expect("lands");
    let paid = before - j.env.lamports(&me) - FEE;
    assert!(paid <= j.limit, "paid {paid} within the limit {}", j.limit);
}

/// Checked and found correct (B2 §3): `tensor_max_amount` signs `max_amount = price + royalty`, and Tensor Marketplace binds the
/// listing amount PLUS the royalty to it (`buy.rs:267 PriceMismatch` on its mainnet bytes), so a seller who edits the listing up
/// between the canister's read and the landing cannot raise what the canister pays: the raised purchase is refused on chain.
#[test]
fn checked_b2_tensor_max_amount_binds_a_listing_the_seller_raises_after_the_judge() {
    let mut j = judged("tensor_pnft_mad_lads");
    let royalty = buy::tensor_royalty(j.ask.price, &j.member);
    assert!(royalty > 0, "a pNFT with royalties: {}", j.member.royalty_bps);
    let max_amount = buy::tensor_max_amount(&j.ask, &j.member);
    assert_eq!(max_amount, j.ask.price + royalty, "what the canister signs as max_amount");
    let list_state = pk(&j.ask.listing);
    let me = j.env.me();
    // the seller's `edit` lands first (the program writes exactly this field): to max_amount, and to one lamport over the listing
    for raised in [max_amount, j.ask.price + 1] {
        j.env.svm.expire_blockhash(); // a failed run keeps its blockhash: the next purchase must not be the same signature
        let mut acct = j.env.svm.get_account(&list_state).unwrap();
        acct.data[LIST_STATE_AMOUNT_AT..LIST_STATE_AMOUNT_AT + 8].copy_from_slice(&raised.to_le_bytes());
        j.env.svm.set_account(list_state, acct).unwrap();
        let before = j.env.lamports(&me);
        let e = j.env.run(&j.ixs, &[]).expect_err("the raised listing is refused");
        assert!(e.contains("PriceMismatch"), "{e}");
        assert_eq!(before - j.env.lamports(&me), FEE, "nothing paid but the signature fee of the failed transaction");
    }
}
