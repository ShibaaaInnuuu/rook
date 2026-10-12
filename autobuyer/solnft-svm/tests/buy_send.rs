//! The canister's Solana NFT purchases and deliveries, executed (docs/v9/solana-nfts-canister.md §7): real mainnet listings and
//! items, the marketplace and Metaplex programs dumped from mainnet, the canister's builders and judges. Each test: membership and
//! the ask verified on the recorded accounts → the purchase built and judged → compiled, signed, run → the NFT is in the canister's
//! wallet (`in_custody` on the state after), it paid exactly what the ask said → delivered wallet to wallet → the redeemer holds it.
mod common;

use common::*;
use etch_autobuyer::solnftbuy::{self as buy, Candidate, Expect, Venue};
use etch_autobuyer::solnftsend as send;
use etch_autobuyer::solnft::{self, Member, Standard};
use etch_autobuyer::soltx::{self, Pubkey as CKey};
use solana_keypair::Keypair;
use solana_signer::Signer;

const FEE: u64 = 5_000;

struct Bought {
    env: Env,
    member: Member,
    paid: u64,
    total: u64,
    rent: u64,
    /// A pNFT transfer closes the escrow's token record and refunds its lamports to the payer (Token Metadata).
    refund: u64,
}

fn listing(standard: &str) -> Standard {
    match standard {
        "legacy" => Standard::Legacy,
        "pnft" => Standard::Programmable,
        _ => Standard::Core,
    }
}

/// Verify, build, judge, run one purchase; check custody after.
fn buy_from(f: &Fixture, venue: Venue, listing_key: &str, extra: &[&Keypair]) -> Bought {
    let mut e = env(f);
    let coll = f.fact("collection");
    let std = listing(f.facts["standard"].as_str().unwrap());
    let asset = f.fact("asset");
    let view = f.view();
    let member = solnft::is_member(&asset, &coll, std, &view).expect("a member");
    let cand = Candidate { venue, listing: f.fact(listing_key), asset };
    let ask = buy::verify_ask(&cand, &member, &view, f.unix_time).expect("a live ask");
    let rent = ask.rent(&member);
    let ixs = buy::buy_instructions(&e.me(), &ask, &member).unwrap();
    let expect = Expect { payer: e.me(), ask: &ask, member: &member, collection: coll, limit: ask.total + rent };
    buy::judge(&ixs, &expect).expect("the judge passes the canonical purchase");
    // one lamport less and the judge refuses
    assert!(buy::judge(&ixs, &Expect { limit: ask.total + rent - 1, ..expect.clone() }).is_err());
    let escrow = match &ask.detail {
        buy::AskDetail::Mmm { pool, .. } => *pool,
        _ => ask.listing,
    };
    let refund = if member.standard == Standard::Programmable {
        e.lamports(&solnft::token_record_address(&asset, &soltx::associated_token_address(&escrow, &asset)))
    } else {
        0
    };
    let before = e.lamports(&e.me());
    let meta = e.run(&ixs, extra).expect("the purchase lands");
    let paid = before - e.lamports(&e.me());
    // custody: the canister's wallet holds the member, no delegate, no lock
    let me = e.me();
    let mut keys = solnft::member_keys(&asset, &coll, std);
    keys.extend(solnft::custody_keys(&asset, &me, std));
    let after = e.view(&keys);
    let m2 = solnft::is_member(&asset, &coll, std, &after).expect("still a member");
    solnft::in_custody(&m2, &me, &after).expect("in the canister's custody");
    println!("{venue:?} {:?}: price {} total {} paid {paid} (rent {rent}, fee {FEE}) CU {}", member.standard, ask.price, ask.total,
        meta.compute_units_consumed);
    Bought { env: e, member: m2, paid, total: ask.total, rent, refund }
}

/// Deliver what was bought to a fresh wallet; check the redeemer's custody and that the canister no longer holds it.
fn deliver(b: &mut Bought) -> u64 {
    let to = ck(&Keypair::new().pubkey());
    let me = b.env.me();
    let ixs = send::send_instructions(&me, &to, &b.member).unwrap();
    send::judge_send(&ixs, &me, &to, &b.member).expect("the canonical delivery");
    let other = ck(&Keypair::new().pubkey());
    assert!(send::judge_send(&ixs, &me, &other, &b.member).is_err(), "judged against another destination");
    let meta = b.env.run(&ixs, &[]).expect("the delivery lands");
    let (asset, coll, std) = (b.member.asset, b.member.collection, b.member.standard);
    let listing_std = if std == Standard::Core { Standard::Core } else { Standard::Legacy };
    let mut keys = solnft::member_keys(&asset, &coll, listing_std);
    keys.extend(solnft::custody_keys(&asset, &to, listing_std));
    keys.extend(solnft::custody_keys(&asset, &me, listing_std));
    let v = b.env.view(&keys);
    let m = solnft::is_member(&asset, &coll, listing_std, &v).unwrap();
    solnft::in_custody(&m, &to, &v).expect("the redeemer holds it");
    assert!(solnft::in_custody(&m, &me, &v).is_err(), "the canister no longer holds it");
    println!("delivery {std:?}: CU {}", meta.compute_units_consumed);
    meta.compute_units_consumed
}

#[test]
fn tensor_pnft_mad_lads() {
    let f = fixture("tensor_pnft_mad_lads");
    let mut b = buy_from(&f, Venue::Tensor, "list_state", &[]);
    assert_eq!(b.member.standard, Standard::Programmable);
    // the ask's total is what the canister paid, to the lamport, besides rent and the signature fee
    assert_eq!(b.paid, b.total + b.rent + FEE - b.refund, "paid exactly the ask");
    assert!(b.refund > 0);
    deliver(&mut b);
}

#[test]
fn tensor_legacy_famous_fox() {
    let f = fixture("tensor_legacy_fff");
    let mut b = buy_from(&f, Venue::Tensor, "list_state", &[]);
    assert_eq!(b.member.standard, Standard::Legacy);
    assert_eq!(b.paid, b.total + b.rent + FEE);
    deliver(&mut b);
    // the legacy delivery closes the canister's emptied token account: its rent comes back
    let me = b.env.me();
    assert!(b.env.svm.get_account(&pk(&soltx::associated_token_address(&me, &b.member.asset))).map_or(true, |a| a.lamports == 0));
}

#[test]
fn tensor_core_otc_desk() {
    let f = fixture("tensor_core_otc");
    let mut b = buy_from(&f, Venue::Tensor, "list_state", &[]);
    assert_eq!(b.member.standard, Standard::Core);
    // broker fees under rent are skipped by the program (transfer_lamports_min_balance): at most the ask
    assert!(b.paid <= b.total + FEE && b.paid + 1_000_000 >= b.total, "paid {} vs ask {}", b.paid, b.total);
    deliver(&mut b);
}

/// mmm pools need their cosigner's signature (every Magic Eden pool names Magic Eden's notary): the instruction is proven with the
/// recorded pool whose cosigner field is replaced by a stand-in key. Not keyless in production.
fn mmm_with_stand_in(name: &str) -> (Fixture, Keypair) {
    let mut f = fixture(name);
    let cos = Keypair::new();
    let pool = f.fact("pool");
    let acct = f.accounts.iter_mut().find(|(k, _)| *k == pool).unwrap();
    assert_eq!(acct.1.data[153..185], soltx::key(buy::ME_NOTARY), "the recorded pool's cosigner is Magic Eden's notary");
    acct.1.data[153..185].copy_from_slice(&cos.pubkey().to_bytes());
    (f, cos)
}

#[test]
fn mmm_legacy_tff_with_stand_in_cosigner() {
    let (f, cos) = mmm_with_stand_in("mmm_legacy_tff");
    let mut b = buy_from(&f, Venue::Mmm, "sell_state", &[&cos]);
    assert_eq!(b.paid, b.total + b.rent + FEE * 2, "two signatures");
    deliver(&mut b);
}

#[test]
fn mmm_pnft_smb_gen3_with_stand_in_cosigner() {
    let (f, cos) = mmm_with_stand_in("mmm_pnft_smb_gen3");
    let mut b = buy_from(&f, Venue::Mmm, "sell_state", &[&cos]);
    assert_eq!(b.member.rule_set, Some(soltx::key(solnft::COMPATIBILITY_RULE_SET)));
    assert_eq!(b.paid, b.total + b.rent + FEE * 2 - b.refund);
    deliver(&mut b);
}

#[test]
fn mmm_without_the_cosigner_fails_on_chain() {
    // the real pool, Magic Eden's notary as cosigner: the canister cannot produce that signature
    let f = fixture("mmm_legacy_tff");
    let mut e = env(&f);
    let (coll, asset) = (f.fact("collection"), f.fact("asset"));
    let view = f.view();
    let m = solnft::is_member(&asset, &coll, Standard::Legacy, &view).unwrap();
    let ask = buy::verify_ask(&Candidate { venue: Venue::Mmm, listing: f.fact("sell_state"), asset }, &m, &view, f.unix_time).unwrap();
    assert!(!ask.keyless());
    let ixs = buy::buy_instructions(&e.me(), &ask, &m).unwrap();
    assert!(e.run(&ixs, &[]).unwrap_err().contains("no key for signer"));
}

#[test]
fn a_tampered_purchase_fails_the_judge_and_the_chain() {
    let f = fixture("tensor_pnft_mad_lads");
    let mut e = env(&f);
    let (coll, asset) = (f.fact("collection"), f.fact("asset"));
    let view = f.view();
    let m = solnft::is_member(&asset, &coll, Standard::Programmable, &view).unwrap();
    let ask = buy::verify_ask(&Candidate { venue: Venue::Tensor, listing: f.fact("list_state"), asset }, &m, &view, f.unix_time).unwrap();
    let mut ixs = buy::buy_instructions(&e.me(), &ask, &m).unwrap();
    // the NFT to somebody else
    let thief: CKey = ck(&Keypair::new().pubkey());
    ixs[1].accounts[1].key = thief;
    ixs[1].accounts[2].key = soltx::associated_token_address(&thief, &asset);
    let x = Expect { payer: e.me(), ask: &ask, member: &m, collection: coll, limit: u64::MAX };
    assert!(buy::judge(&ixs, &x).unwrap_err().starts_with("J2"));
    // and a max_amount under the listing is refused by the program itself (the price is the ListState's)
    let mut low = buy::buy_instructions(&e.me(), &ask, &m).unwrap();
    low[1].data[8..16].copy_from_slice(&(ask.price - 1).to_le_bytes());
    assert!(e.run(&low, &[]).is_err());
}
