//! 2026-10-12 (audit C1-03 fixed further; docs/v9/solana-nfts-canister.md §9): Magic Eden M2 is bought by default, and a pNFT's
//! purchase, which does not fit a legacy message as the flow composes it, is signed as a v0 message loading M2's fixed accounts from the
//! canister's own address lookup table (`solnftflow::table`). On the mainnet bytes (M2, Token Metadata, Token Auth Rules) and LiteSVM's
//! Address Lookup Table program: the table is created and filled as the flow makes it, read back as the flow reads it, and the flow's
//! ONE transaction for a real Mad Lads M2 listing (the durable nonce's advance, the advance `M[i] → K_main`, the purchase, the memo),
//! judged whole, compiled by `solsend::compile_with` and signed by both keys, lands: the advance pays the purchase and the item is in
//! `K_main`'s custody.
mod common;

use common::*;
use etch_autobuyer::chainlink::{ledger::OpKind, memo};
use etch_autobuyer::solnft::{self, Standard};
use etch_autobuyer::solnftbuy::{self as buy, AskDetail, Candidate, Expect, Venue};
use etch_autobuyer::solnftflow::{buy as fbuy, table};
use etch_autobuyer::solsend;
use etch_autobuyer::soltx::{self, LookupTable};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

#[test]
fn the_flows_m2_pnft_purchase_lands_as_a_v0_message_with_the_canisters_lookup_table() {
    let f = fixture("m2_mad_lads_4vCV2b");
    let mut e = env(&f);
    let module = Keypair::new();
    e.svm.airdrop(&module.pubkey(), 20_000_000_000).unwrap();
    let (coll, asset, view) = (f.fact("collection"), f.fact("asset"), f.view());
    let member = solnft::is_member(&asset, &coll, Standard::Programmable, &view).unwrap();
    assert_eq!(member.standard, Standard::Programmable);
    let ask = buy::verify_ask(&Candidate { venue: Venue::M2, listing: f.fact("seller_trade_state"), asset }, &member, &view, f.unix_time).unwrap();
    let AskDetail::M2 { house: Some(house), .. } = &ask.detail else { panic!("no auction house") };
    let me = e.me();

    // the table, as `table::pass` makes it: created on a slot of SlotHashes (LiteSVM's: slot 0) and filled, one transaction by K_main
    let addresses = table::fixed_addresses(&me, house);
    let (ixs, key) = table::create_instructions(&me, 0, &addresses);
    e.run(&ixs, &[]).expect("the lookup table is created and filled");
    // read back as `table::usable` reads it: the ALT program's, K_main its authority, active, exactly what it was made with
    let acc = e.svm.get_account(&pk(&key)).unwrap();
    assert_eq!(ck(&acc.owner), soltx::key(soltx::ADDRESS_LOOKUP_TABLE_PROGRAM));
    let st = soltx::decode_lookup_table(&acc.data).unwrap();
    assert_eq!((st.authority, st.deactivation_slot, &st.addresses), (Some(me), u64::MAX, &addresses));
    assert_eq!(st.last_extended_slot, f.slot);
    e.svm.warp_to_slot(f.slot + 1); // an extension is usable from the next slot on (the flow: older than the finalized slot)
    let t = LookupTable { key, addresses: st.addresses };

    // the line's durable nonce (as `solsend::open_nonce_ixs` opens it)
    e.run(&solsend::open_nonce_ixs(&me, "rook-money", 1_447_680).unwrap(), &[]).expect("the nonce account");
    let nonce = solsend::nonce_account_of(&me, "rook-money").unwrap();
    let durable = soltx::decode_nonce(&e.svm.get_account(&pk(&nonce)).unwrap().data).unwrap().durable_nonce;

    // the flow's ONE transaction, judged whole, compiled as `solsend::sign_intent` compiles it with the table
    let m = ck(&module.pubkey());
    let amount = ask.total + ask.rent(&member);
    let purchase = fbuy::purchase(&me, &ask, &member).unwrap();
    let memo = memo::memo(Some(1), OpKind::Advance, 7, None);
    let tx = fbuy::compose(&purchase, &m, &me, amount, &memo);
    let expect = Expect { payer: me, ask: &ask, member: &member, collection: coll, limit: amount };
    fbuy::judge_tx(&tx, &m, amount, &memo, &expect).expect("the judge passes the flow's transaction");
    let all = soltx::with_nonce(nonce, me, &tx);
    assert!(soltx::compile_parts(&all, me, durable).map(|p| soltx::check_size(soltx::wire_size(&p.message())).is_err()).unwrap(),
        "as a legacy message it does not fit");
    let msg = solsend::compile_with(&all, me, durable, std::slice::from_ref(&t)).unwrap();
    let bound = fbuy::composed_size(&me, &m, 1, &purchase, amount, std::slice::from_ref(&t)).unwrap();
    let size = soltx::wire_size(&msg);
    println!("the Mad Lads M2 pNFT purchase as signed: {size} bytes (the flow's bound {bound}; Solana's limit {})", soltx::PACKET_DATA_SIZE);
    assert!(size <= bound && bound + 128 <= soltx::PACKET_DATA_SIZE, "{size} / {bound}");
    let mut sigs = vec![];
    for s in &msg.signers {
        let kp = [&e.canister, &module].into_iter().find(|k| ck(&k.pubkey()) == *s).expect("a signer of ours");
        sigs.push(<[u8; 64]>::try_from(kp.sign_message(&msg.bytes).as_ref()).unwrap());
    }
    assert_eq!(sigs.len(), 2, "K_main and K_coll[i]");
    let wire = soltx::assemble(&msg, &sigs).unwrap();
    let vt: VersionedTransaction = bincode::deserialize(&wire).unwrap();
    let (main0, m0) = (e.lamports(&me), e.lamports(&m));
    let r = e.svm.send_transaction(vt).unwrap_or_else(|x| panic!("{:?}\n{}", x.err, x.meta.logs.join("\n")));
    println!("CU {}", r.compute_units_consumed);
    assert_eq!(m0 - e.lamports(&m), amount, "the advance left M[i], exactly the all-in");
    let main1 = e.lamports(&me);
    println!("K_main: {main0} -> {main1}");
    assert!(main1 + 2 * 5_000 >= main0, "the advance paid the purchase: K_main spent the two signatures' fee at most (a change stays there: C1-I4)");
    let mut keys = solnft::member_keys(&asset, &coll, Standard::Programmable);
    keys.extend(solnft::custody_keys(&asset, &me, Standard::Programmable));
    let after = e.view(&keys);
    let held = solnft::is_member(&asset, &coll, Standard::Programmable, &after).unwrap();
    solnft::in_custody(&held, &me, &after).expect("in K_main's custody");
    assert_ne!(soltx::decode_nonce(&e.svm.get_account(&pk(&nonce)).unwrap().data).unwrap().durable_nonce, durable, "the nonce advanced");
}
