
// ---- round 3, part D (review3/R3-services.md, D3-H-1): a redeemer who moves the NFT on before finality is never recorded --------
// The canister records a delivery only if, at finality, the DESTINATION still owns the token (ethflow.rs delivery_pass, Moved). A
// redeemer (or an arbitrage bot) that moves it on within ~13-25 min leaves the ticket Resolved for ever: never Delivered, never
// marked. EthNftWrapper.refund() then pays the redeemer one unit back after deliverBy + LATE_DELIVERY_WINDOW (30 days), while the
// NFT is gone: an unbacked unit per redemption. Asserts the weakness: passes while it is present.
#[test]
fn d3_h1_a_redeemer_who_moves_the_nft_before_finality_is_never_recorded_and_the_ticket_stays_refundable() {
    const BOB: [u8; 20] = [0xb0; 20];
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }), "{:?}", st.deliveries[0].phase);
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }));
    // ALICE (the redeemer) received token 77 and moved it to her other wallet BOB (or sold it into a bid) in the next blocks
    assert_eq!(f.owners.get(&sp::word_u128(77)), Some(&ALICE));
    f.owners.insert(sp::word_u128(77), BOB);
    f.finalized = ETH_PIN;
    // 40 days of ticks: the canister never records it and never marks it
    for _ in 0..40 {
        f.now += 86_400;
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }), "{:?}", st.deliveries[0].phase);
    assert!(st.deliveries[0].note.contains("not the destination"), "{}", st.deliveries[0].note);
    assert_eq!(f.tickets[0].stage, Stage::Resolved, "never recordDelivery'd: past deliverBy + 30 days EthNftWrapper.refund() mints the unit back");
    assert!(f.marked.is_empty(), "never markUndeliverable'd either");
}
