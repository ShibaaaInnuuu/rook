
// =====================================================================================================================================
// Round 4, part E: the PoCs (each PASSES while the weakness is there)
// =====================================================================================================================================

/// Up to Milady's Relay deposit mined on 4663 (Bridging), returning what the deposit took from our 4663 balance.
fn to_bridging(f: &mut Fake, st: &mut EthState) -> u128 {
    f.m(MODULE).net_units = 2 * E;
    tick(f, st);
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Funded { .. }), "{:?}", phase(st));
    let id = st.buys.last().unwrap().id;
    st.used_orders.clear();
    ef::put_quote(st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    let before = f.home_balance;
    tick(f, st);
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Bridging { .. }), "{:?}", phase(st));
    before - f.home_balance
}

#[test]
fn e4_control_without_the_home_pin_moving_back_the_fill_arrives() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_bridging(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Bridging { .. }), "no refund is read: {:?}", phase(&st));
    f.eth_balance += ARRIVED;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Arrived { .. }), "{:?}", phase(&st));
}

/// E4-M-1: E3-M-1's fix floors every ETHEREUM balance read of the flow, but the forward leg's refund is read from the HOME (4663)
/// balance with no floor. One tick at a home pin below our own Relay deposit's block (a provider's head falls back across one of the
/// home chain's 1,024-block buckets) shows the balance before the deposit: `relayconv::refunded` says the deposit came back. The
/// buy drops its watch and goes Funded for a new quote that the real balance can never pay; it holds the one Relay leg (no
/// collection buys), and Relay's real fill lands at our Ethereum address with nobody watching. The operator's only exit books the
/// advance as a loss while the ETH sits unattributed on Ethereum.
#[test]
fn e4_m1_a_home_pin_that_moves_back_reads_our_own_relay_deposit_as_a_refund() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let spent = to_bridging(&mut f, &mut st);
    let id = st.buys.last().unwrap().id;
    let advanced = st.buy(id).unwrap().advanced;
    println!("AUDIT4-E: Milady's deposit took {spent} wei on 4663 (advanced {advanced})");
    // one tick with the home pin one bucket back, below the deposit's block; Relay has not filled yet (Ethereum shows nothing)
    f.home_pin_back = Some(spent);
    let l = tick(&mut f, &mut st);
    f.home_pin_back = None;
    let b = st.buy(id).unwrap().clone();
    println!("AUDIT4-E after one tick at the lower home pin: {:?}, watch {:?}; {}", b.phase, b.last.is_some(),
        l.iter().find(|x| x.contains("REFUNDED")).cloned().unwrap_or_default());
    assert!(matches!(b.phase, BPhase::Funded { .. }) && b.last.is_none(), "WEAKNESS: our own deposit read as Relay's refund");
    // Relay fills on Ethereum as ordered
    f.eth_balance += ARRIVED;
    for _ in 0..5 {
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::Funded { .. }), "the real arrival is never read: {:?}", st.buy(id).unwrap().phase);
    // the runner hands in a fresh quote: the real balance cannot pay it (the first deposit is at Relay)
    st.used_orders.clear();
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    let refused = l.iter().find(|x| x.contains("under the deposit")).cloned();
    println!("AUDIT4-E the new quote: {refused:?}");
    assert!(refused.is_some(), "{l:?}");
    // the Relay leg is held: Azuki's module wants an item and no buy of it starts
    f.m(MODULE2).net_units = 2 * E;
    for _ in 0..5 {
        tick(&mut f, &mut st);
    }
    assert!(st.relay_busy() && buy_of(&st, AZUKI).is_none(), "WEAKNESS: every collection waits behind the phantom refund");
    // the operator's exit: returnAdvance of what 4663 holds; the module books the rest as a loss, the ETH is at our Ethereum address
    let msg = ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap();
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    let BPhase::Returned { wei } = st.buy(id).unwrap().phase else { panic!("{:?}", st.buy(id).unwrap().phase) };
    println!("AUDIT4-E give back: {msg}; returned {wei} of {advanced} advanced; our Ethereum balance {} of which earmarked {}, \
        leftover booked {}", f.eth_balance, st.earmarked_except(u64::MAX), st.leftover_of(&MILADY));
    assert!(wei < advanced / 100, "WEAKNESS: the module books ≈ the whole advance as a loss");
    assert!(f.eth_balance >= ARRIVED && st.earmarked_except(u64::MAX) == 0 && st.leftover_of(&MILADY) < ARRIVED,
        "WEAKNESS: Relay's fill sits at our Ethereum address, nobody's (the sweep only moves booked leftover)");
}

/// The launch install's H3 (`nft-pilot.json` `_custody`: 12 / 24 items, 3 / 6 mints, 25 ETH, trip ×3) and J7's Milady cap (16 items,
/// 15 ETH, the global the collections' sum).
fn launch_limits(f: &mut Fake) {
    let l = etch_autobuyer::outflow::Limits { per_collection_items: 12, global_items: 24, per_collection_mints: 3, global_mints: 6,
        global_value_wei: 25 * E, trip_factor: 3 };
    etch_autobuyer::outflow::validate(&l).unwrap();
    f.real_outflow = Some((Default::default(), l));
    let key = custodycap::evm_key(1, &MILADY);
    f.caps = CapPolicy { collections: vec![custodycap::CollectionCap { collection: key.clone(), max_items: 16, max_value_wei: 15 * E, max_item_wei: 15 * E },
        custodycap::CollectionCap { collection: custodycap::evm_key(1, &AZUKI), max_items: 22, max_value_wei: 15 * E, max_item_wei: 15 * E }],
        global_max_items: 38, global_max_value_wei: 30 * E };
    custodycap::validate_change(&CapPolicy::default(), &f.caps).unwrap();
}

/// E4-H-1: "deposits' mints: one asked at a time (so a backlog never inflates the tripwire's demand)" (`home_act` step 3) is not what
/// the loop does: since fix round 1 (C-L-2) every deposit in `Mint` calls `outflow_check` under its own key in the same pass, and a
/// Wait counts the ask. Nine deposits of one collection final within a day (three mint, six wait) are 9 asks = 3 × the collection's
/// 3 mints a day: the tripwire freezes custody and PAUSES the whole canister (every module stale within its TTL, no mint, burn,
/// delivery or post until the controller resumes). J7's caps (16 Miladys, 22 Azukis) now admit them all.
#[test]
fn e4_h1_nine_deposits_of_one_collection_in_a_day_trip_the_mint_tripwire_and_pause_the_canister() {
    let mut f = Fake::new(true);
    launch_limits(&mut f);
    f.deposits_on = true;
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for k in 0..9u8 {
        let tx = [0x90 + k; 32];
        deposit_tx(&mut f, tx, MILADY, WRAPPER, 950 + k as u128, RECIPIENT, ETH_PIN - 20 - k as u64);
        ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    }
    let mut ticks = 0;
    while !f.paused && ticks < 40 {
        f.now += 600;
        f.m(MODULE).posted_at = f.now - 60;
        f.m(MODULE2).posted_at = f.now - 60;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
        ticks += 1;
    }
    let (o, _) = f.real_outflow.as_ref().unwrap();
    let asks: Vec<String> = o.asked.iter().filter(|a| a.flow == Flow::Mint).map(|a| a.key.clone()).collect();
    println!("AUDIT4-E after {ticks} ticks ({} s): minted {} deposits; mint asks {}; frozen {:?}; paused {}", ticks * 600, f.items.len(),
        asks.len(), o.frozen(), f.paused);
    assert!(f.paused && o.frozen().is_some(), "WEAKNESS: honest deposits froze custody and paused the canister");
    assert_eq!(f.items.len(), 3, "three minted; the six waiting deposits were each asked");
}

/// Control: eight deposits (the collection's demand 8 < 9) wait their turn and never trip.
#[test]
fn e4_control_eight_deposits_wait_for_the_days_room_without_a_freeze() {
    let mut f = Fake::new(true);
    launch_limits(&mut f);
    f.deposits_on = true;
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for k in 0..8u8 {
        let tx = [0x90 + k; 32];
        deposit_tx(&mut f, tx, MILADY, WRAPPER, 950 + k as u128, RECIPIENT, ETH_PIN - 20 - k as u64);
        ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    }
    for _ in 0..40 {
        f.now += 600;
        f.m(MODULE).posted_at = f.now - 60;
        f.m(MODULE2).posted_at = f.now - 60;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    assert!(!f.paused, "no freeze");
    assert_eq!(f.items.len(), 3, "the day's three mints; the others wait");
}

/// E4-L-1: the install's exit rule (`canister.rs` `validate_custody`: `outflow::exit_fits` per collection only). J7's launch caps
/// (61 items in all) at the launch limits (24 items a day in all) need 3 days; the timelock's 72 h less a day gives 2. The
/// default (`custody_caps = null`: 6 items each, 30 in all) at stage 0's 6 a day in all needs 5.
#[test]
fn e4_l1_j7_custody_cannot_all_leave_within_the_delay_at_the_global_item_limit() {
    const DAY: u64 = 86_400;
    let per = [16u32, 22, 17, 4, 2];
    assert!(per.iter().all(|n| etch_autobuyer::outflow::exit_fits(*n, 12, 3 * DAY)), "every collection passes the install's check");
    let total: u32 = per.iter().sum();
    println!("AUDIT4-E J7: {total} items in all at 24 a day: exit_fits = {}; the default 30 at 6 a day: {}", 
        etch_autobuyer::outflow::exit_fits(total, 24, 3 * DAY), etch_autobuyer::outflow::exit_fits(30, 6, 3 * DAY));
    assert!(!etch_autobuyer::outflow::exit_fits(total, 24, 3 * DAY), "WEAKNESS: the whole custody does not fit the delay");
    assert!(!etch_autobuyer::outflow::exit_fits(30, 6, 3 * DAY), "WEAKNESS: nor does the default install's");
}
