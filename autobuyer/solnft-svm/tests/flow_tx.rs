//! W8c: the Solana NFT flow's ONE transaction (`solnftflow::buy`), executed on the mainnet bytes: the advance `M[i] → K_main`, Tensor's
//! pNFT purchase of a real Mad Lads listing and the link's memo, judged whole by `judge_tx` and signed by both keys; the advance pays the
//! purchase exactly (K_main is left with its fee less the escrow token record's refund); then the delivery to a fresh wallet.
mod common;

use common::*;
use etch_autobuyer::solix2::spl;
use etch_autobuyer::solnftbuy::{self as buy, Candidate, Expect, Venue};
use etch_autobuyer::solnftflow::buy as fbuy;
use etch_autobuyer::solnftsend as send;
use etch_autobuyer::solnft::{self, Standard};
use etch_autobuyer::soltx;
use solana_keypair::Keypair;
use solana_signer::Signer;

const FEE: u64 = 5_000;

#[test]
fn the_flows_advance_purchase_and_memo_land_in_one_transaction_then_the_delivery() {
    let f = fixture("tensor_pnft_mad_lads");
    let mut e = env(&f);
    let module = Keypair::new();
    e.svm.airdrop(&module.pubkey(), 20_000_000_000).unwrap();
    let (coll, asset, view) = (f.fact("collection"), f.fact("asset"), f.view());
    let member = solnft::is_member(&asset, &coll, Standard::Programmable, &view).unwrap();
    let ask = buy::verify_ask(&Candidate { venue: Venue::Tensor, listing: f.fact("list_state"), asset }, &member, &view, f.unix_time).unwrap();
    let amount = ask.total + ask.rent(&member);
    let (me, m) = (e.me(), ck(&module.pubkey()));
    let ixs = fbuy::purchase(&me, &ask, &member).unwrap();
    let memo = "rook:v1:1:advance:0";
    let tx = vec![ixs[0].clone(), soltx::transfer(m, me, amount), ixs[1].clone(), spl::memo(memo, &[])];
    let expect = Expect { payer: me, ask: &ask, member: &member, collection: coll, limit: amount };
    fbuy::judge_tx(&tx, &m, amount, memo, &expect).expect("the judge passes the flow's transaction");
    let refund = e.lamports(&solnft::token_record_address(&asset, &soltx::associated_token_address(&ask.listing, &asset)));
    let (main0, m0) = (e.lamports(&me), e.lamports(&m));
    let meta = e.run(&tx, &[&module]).expect("one transaction lands");
    assert_eq!(m0 - e.lamports(&m), amount, "the advance left M[i]");
    assert_eq!(main0 + refund - 2 * FEE, e.lamports(&me), "the advance paid the purchase exactly: K_main spent the two signatures' fee, less the refund");
    let mut keys = solnft::member_keys(&asset, &coll, Standard::Programmable);
    keys.extend(solnft::custody_keys(&asset, &me, Standard::Programmable));
    let after = e.view(&keys);
    let held = solnft::is_member(&asset, &coll, Standard::Programmable, &after).unwrap();
    solnft::in_custody(&held, &me, &after).expect("in K_main's custody");
    println!("flow tx: CU {}", meta.compute_units_consumed);
    // the delivery, as `solnftflow::deliver` signs it
    let to = ck(&Keypair::new().pubkey());
    let d = send::send_instructions(&me, &to, &held).unwrap();
    send::judge_send(&d, &me, &to, &held).unwrap();
    e.run(&d, &[]).expect("the delivery lands");
    let mut keys = solnft::member_keys(&asset, &coll, Standard::Programmable);
    keys.extend(solnft::custody_keys(&asset, &to, Standard::Programmable));
    let v = e.view(&keys);
    solnft::in_custody(&solnft::is_member(&asset, &coll, Standard::Programmable, &v).unwrap(), &to, &v).expect("the redeemer holds it");
}
