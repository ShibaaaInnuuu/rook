// Round-3 re-review (2026-09-27-nft), part A, A3-M-1 (canister half). Appended to a PRIVATE copy of
// autobuyer/canister-rs/tests/audit_nft_r2.rs (its fake world, unchanged) and run there with
// `cargo test --test audit_nft_a3 -- --nocapture`. It PASSES while the weakness is present: after a buy goes back and is returned
// short to its module, the canister starts the next buy of the same collection at once and takes the same round trip again; nothing
// counts the failed round trips or waits between them.

fn a3_forward(f: &mut Fake, st: &mut EthState) -> u64 {
    for _ in 0..6 {
        tick(f, st);
        if matches!(phase(st), BPhase::Funded { .. }) { break; }
    }
    assert!(matches!(phase(st), BPhase::Funded { .. }), "{:?}", phase(st));
    let id = st.buys.last().unwrap().id;
    st.used_orders.clear(); // the fake reuses one Relay fixture; live, every quote is a fresh order
    ef::put_quote(st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    for _ in 0..4 {
        tick(f, st);
        if matches!(phase(st), BPhase::Bridging { .. }) { break; }
    }
    assert!(matches!(phase(st), BPhase::Bridging { .. }), "{:?}", phase(st));
    f.eth_balance += ARRIVED;
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Arrived { .. }), "{:?}", phase(st));
    id
}

fn a3_back(f: &mut Fake, st: &mut EthState, id: u64) -> u128 {
    // the operator's eth_give_back (or ARRIVED_BACK_SECS with no fill: the same path from BackFunded on)
    ef::give_back_now(st, id, 0).unwrap();
    tick(f, st);
    let ask = st.buys.iter().find(|b| b.id == id).unwrap().ask.clone().unwrap();
    st.used_orders.clear();
    ef::put_quote(st, id, back_quote(ask.amount_wei, f.now), f.now).unwrap();
    tick(f, st);
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    tick(f, st);
    assert!(matches!(st.buys.iter().find(|b| b.id == id).unwrap().phase, BPhase::BackBridging { .. }));
    let out = ask.amount_wei / 1_000 * 997;
    f.home_balance += out;
    tick(f, st);
    tick(f, st);
    let b = st.buys.iter().find(|b| b.id == id).unwrap();
    assert!(matches!(b.phase, BPhase::Returned { wei } if wei == out), "{:?}", b.phase);
    out
}

#[test]
fn a3_m1_the_way_back_repeats_without_bound_each_round_trip_returned_short() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E; // the module keeps wanting one item (a pool that has not bought)
    let mut lost = 0u128;
    let mut ids = vec![];
    for cycle in 0..4 {
        let id = a3_forward(&mut f, &mut st);
        let advanced = st.buys.iter().find(|b| b.id == id).unwrap().advanced;
        let back = a3_back(&mut f, &mut st, id);
        lost += advanced - back;
        println!("cycle {cycle}: buy {id} advanced {advanced}, returned {back} (short {})", advanced - back);
        ids.push(id);
    }
    // four distinct buys of the same collection, each advanced, taken to Ethereum, brought back, returned short
    assert_eq!(ids.len(), 4);
    assert!(ids.windows(2).all(|w| w[1] > w[0]));
    assert_eq!(st.buys.iter().filter(|b| matches!(b.phase, BPhase::Returned { .. })).count(), 4);
    let advances = f.home_calls.iter().filter(|(to, _, d)| *to == MODULE && sel(d) == rc::ADVANCE).count();
    let returns = f.home_calls.iter().filter(|(to, _, d)| *to == MODULE && sel(d) == rc::RETURN_ADVANCE).count();
    assert_eq!(returns, 4, "four short returns of one module, back to back");
    assert_eq!(advances, 5, "and the canister has already advanced a fifth time in the same ticks: nothing waits after a return");
    assert!(matches!(phase(&st), BPhase::Advancing | BPhase::Funded { .. }), "the fifth buy is on its way: {:?}", phase(&st));
    println!("lost to the round trips: {lost} wei over 4 cycles ({} bps of one advance each)", lost / 4 * 10_000 / DEPOSIT);
    assert!(lost > 4 * en::ETH_GAS_RESERVE_WEI, "each trip leaves at least the Ethereum gas reserve and both Relay fees behind");
}
