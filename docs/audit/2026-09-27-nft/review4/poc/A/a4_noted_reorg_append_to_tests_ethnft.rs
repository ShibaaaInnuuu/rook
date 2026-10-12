
// ---- review round 4, part A (A4-L-2): appended to a private copy of tests/ethnft.rs ----
/// noteSent is signed as soon as the transfer is seen MINED (before finality). If a reorg then drops that transfer (it is never seen
/// again: no receipt at the pin, the nonce free), the delivery stays `Moved` for ever ("not seen at the pin now: read again next
/// tick"): no path goes to MarkAfter / Mark, and the wrapper's late window is off for a noted ticket (DeliverySentNotRecorded). The
/// redeemer burned the unit, holds no NFT, and nothing but a canister upgrade refunds it. Before round 3 the 30-day late window did.
/// Passing = the weakness is present.
#[test]
fn a4_l2_a_noted_delivery_dropped_by_a_reorg_before_finality_has_no_way_to_a_mark() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }));
    assert_eq!(st.deliveries[0].sent, Some(true), "noted sent on the wrapper");
    // the reorg: the transfer is gone from the canonical chain and is never re-included; the token is ours again
    let hash = f.eth_signed[0].1;
    f.our_logs.remove(&hash);
    f.receipts.remove(&hash);
    f.owners.insert(sp::word_u128(77), US);
    f.finalized = ETH_PIN;
    for _ in 0..(30 * 144) {               // 30 days of 10-minute ticks
        f.now += 600;
        tick(&mut f, &mut st);
    }
    println!("after 30 days: {:?}; note: {}", st.deliveries[0].phase, st.deliveries[0].note);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }), "WEAKNESS: still Moved, waiting for a receipt that will never come");
    assert!(f.marked.is_empty(), "WEAKNESS: never marked undeliverable, so the noted ticket can never refund");
    assert_eq!(f.tickets[0].stage, Stage::Resolved);
}
