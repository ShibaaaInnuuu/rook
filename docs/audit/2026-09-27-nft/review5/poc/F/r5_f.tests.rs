// =====================================================================================================================================
// Review round 5, part F (C4-H-1's fix): PoCs. Each PASSING test shows the weakness.
// =====================================================================================================================================

fn home_calls_to(f: &Fake, to: [u8; 20], s: [u8; 4]) -> usize {
    f.home_calls.iter().filter(|(t, _, d)| *t == to && d.len() >= 4 && sel(d) == s).count()
}

/// Ticks `n` times `step` seconds apart; returns the DROPPED line if any.
fn run_for(f: &mut Fake, st: &mut EthState, n: usize, step: u64) -> Option<String> {
    let mut dropped = None;
    for _ in 0..n {
        f.now += step;
        fresh_modules(f);
        for l in tick(f, st) {
            if l.contains("DROPPED") {
                dropped = Some(l);
            }
        }
    }
    dropped
}

/// F5-M-1 (a): a Relay deposit that MINED (the ETH left, Relay will fill the order) but that every 4663 provider fails to show for an
/// hour is dropped, taken back to `Funded`, and deposited AGAIN from whatever else sits at the shared 4663 address: the same buy's
/// value leaves twice. The first order's watch is gone (`last = None`), so its fill on Ethereum is nobody's.
#[test]
fn f5_m1_a_a_mined_relay_deposit_seen_nowhere_for_an_hour_is_deposited_again() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    // other ETH at the shared 4663 address (the frog side's float, B-L-6; a late payout; the gas keeps): 1 ETH
    f.home_balance += E;
    let start = f.home_balance;
    ef::put_quote(&mut st, id, fwd_quote(DEPOSIT, f.now), f.now).unwrap();
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("the deposit is out");
    assert!(matches!(p.step, ef::Step::Deposit { .. }), "{:?}", p.step);
    assert_eq!(relay_deposits(&f), 1, "the deposit mined: the ETH left for Relay");
    // every provider's index misses it (no receipt at the pin, no transaction by hash), now and an hour later
    f.hide.push(p.hash);
    let dropped = run_for(&mut f, &mut st, 8, 600).expect("dropped after HOME_NONCE_TAKEN_SECS");
    println!("F5-M-1(a): {dropped}");
    assert!(matches!(phase(&st), BPhase::Funded { .. }) && st.buys[0].last.is_none(), "taken back, the first order unwatched: {:?}", phase(&st));
    // the runner hands in the next quote (the buy asks for one): deposited again
    ef::put_quote(&mut st, id, fwd_quote(DEPOSIT, f.now), f.now).unwrap();
    until(&mut f, &mut st, 3, |s| matches!(s.buys[0].phase, BPhase::Bridging { .. }));
    println!("F5-M-1(a): Relay deposits {}, 4663 balance {} -> {} (spent {} for ONE buy of {DEPOSIT})", relay_deposits(&f), start,
        f.home_balance, start - f.home_balance);
    assert_eq!(relay_deposits(&f), 2, "the buy's value was deposited twice");
    assert!(start - f.home_balance >= 2 * DEPOSIT, "2 x the deposit left the 4663 address");
    // H3 counted the advance once; nothing counts the second deposit
    assert_eq!(f.outflow.iter().filter(|(_, fl, _)| *fl == Flow::Value).count(), 1, "one H3 value record for two deposits");
}

/// F5-M-1 (a, control): the same without other ETH at the address: no double spend, but the buy waits `Funded` for good with the
/// Relay leg held (every collection's new buys wait), and the first order's fill on Ethereum is nobody's.
#[test]
fn f5_m1_a_control_without_other_eth_the_buy_holds_the_relay_leg_and_the_first_fill_is_nobodys() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    ef::put_quote(&mut st, id, fwd_quote(DEPOSIT, f.now), f.now).unwrap();
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("the deposit is out");
    f.hide.push(p.hash);
    run_for(&mut f, &mut st, 8, 600).expect("dropped");
    // Relay fills the first order on Ethereum; the runner keeps handing in quotes for three days
    f.eth_balance += ARRIVED;
    let mut last = String::new();
    for _ in 0..(3 * 24) {
        f.now += 3_600;
        fresh_modules(&mut f);
        if matches!(st.buys[0].phase, BPhase::Funded { .. }) {
            let _ = ef::put_quote(&mut st, id, fwd_quote(DEPOSIT, f.now), f.now);
        }
        last = tick(&mut f, &mut st).join(" / ");
    }
    println!("F5-M-1(a control): after 3 days: {:?}; earmarked on Ethereum {}; last tick: {last}", phase(&st), st.earmarked_except(u64::MAX));
    assert_eq!(relay_deposits(&f), 1);
    assert!(st.buys[0].in_relay(), "the buy still holds the Relay leg: {:?}", phase(&st));
    assert_eq!(st.earmarked_except(u64::MAX), 0, "the first order's fill ({ARRIVED} wei on Ethereum) is nobody's");
}

/// F5-M-1 (b): a gas-pot sweep's `fundReserve` that mined but is seen nowhere for an hour is sent again: `fundReserve` has no
/// contract-level idempotence, so the sweep's value is paid into the module twice, from whatever else sits at the address.
#[test]
fn f5_m1_b_a_mined_fund_reserve_seen_nowhere_for_an_hour_is_paid_again() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let wei = 3 * E / 10;
    st.buys.push(ef::EthBuy { id: 7, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: 0, attempts: 0,
        phase: BPhase::Returning { wei }, orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(),
        sweep: Some(true), arrived_at: None, back_at: None, ended_at: None, late: None });
    f.home_balance = E; // the sweep's 0.3 ETH arrived back on 4663, beside 0.7 ETH of other value at the shared address
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("fundReserve is out");
    assert!(matches!(p.step, ef::Step::GiveBack { .. }), "{:?}", p.step);
    f.hide.push(p.hash);
    let dropped = run_for(&mut f, &mut st, 8, 600).expect("dropped");
    println!("F5-M-1(b): {dropped}");
    until(&mut f, &mut st, 4, |s| matches!(s.buys[0].phase, BPhase::Returned { .. }));
    let n = home_calls_to(&f, MODULE, ef::FUND_RESERVE);
    println!("F5-M-1(b): fundReserve calls {n}; paid in {} for a sweep of {wei}; the sweep is {:?}", E - f.home_balance, st.buys[0].phase);
    assert_eq!(n, 2, "fundReserve went out twice");
    assert_eq!(E - f.home_balance, 2 * wei, "twice the sweep's value left the 4663 address");
}

/// F5-M-1 (c): an `advance` that mined but is seen nowhere for an hour is taken back to `NeedAdvance` with `advanced = 0`; the
/// module's advance stays open (no second advance, no return: nothing tracks it) and its ETH sits at our address, nobody's.
#[test]
fn f5_m1_c_a_mined_advance_seen_nowhere_for_an_hour_orphans_the_advance() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    let bal0 = f.home_balance;
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("the advance is out");
    assert!(matches!(p.step, ef::Step::Advance { .. }), "{:?}", p.step);
    assert!(f.m(MODULE).transit, "the advance mined: the module's advance is open");
    let advanced = f.home_balance - bal0;
    f.hide.push(p.hash);
    let dropped = run_for(&mut f, &mut st, 8, 600).expect("dropped");
    println!("F5-M-1(c): {dropped}");
    run_for(&mut f, &mut st, 6 * 24, 600);
    let tracked: u128 = st.buys.iter().filter(|b| !b.finished()).map(|b| b.advanced + b.earmark()).sum();
    let transit = f.m(MODULE).transit;
    println!("F5-M-1(c): after a day: module transit {}, advances sent {}, open buys {:?}, ETH tracked by any buy {tracked}, 4663 balance \
        over the start {}", transit, home_calls_to(&f, MODULE, rc::ADVANCE),
        st.buys.iter().map(|b| (b.id, b.phase.clone())).collect::<Vec<_>>(), f.home_balance - bal0);
    assert!(f.m(MODULE).transit, "the module's advance is still open");
    assert_eq!(home_calls_to(&f, MODULE, rc::RETURN_ADVANCE), 0, "and nothing returns it");
    assert_eq!(tracked, 0, "no buy tracks the {advanced} wei the module advanced");
    assert!(f.home_balance - bal0 >= advanced - E / 100, "the advanced ETH sits at our address");
}

/// F5-M-1 (d): "seen nowhere for HOME_NONCE_TAKEN_SECS" is two samples an hour apart, not an hour unseen: ticks in between where a
/// provider shows the transaction (the read is Unsure: the providers disagree) do not restart the clock.
#[test]
fn f5_m1_d_the_hour_is_not_reset_by_a_provider_that_shows_the_transaction() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).posted_at = f.now - 3_000;
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("a post is out");
    f.hide.push(p.hash);
    f.now += 60;
    tick(&mut f, &mut st); // seen nowhere: the clock starts
    let t0 = st.pending.as_ref().and_then(|x| x.used_since).expect("the clock started");
    f.unsure = true; // for the next hour one provider shows it mined, the other does not
    for _ in 0..6 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    assert_eq!(st.pending.as_ref().and_then(|x| x.used_since), Some(t0), "the clock was not reset while a provider showed it");
    f.unsure = false;
    f.now += 60;
    let l = tick(&mut f, &mut st).join("\n");
    println!("F5-M-1(d): {} s after the first unseen read, the FIRST further unseen read drops it: {}", f.now - t0,
        l.lines().find(|x| x.contains("DROPPED")).unwrap_or("(not dropped)"));
    assert!(l.contains("DROPPED"), "{l}");
}

/// F5-L-1: the Ethereum pin floor (`PIN_FLOORS.eth`) is one per canister, not per chain, and it survives an upgrade that moves
/// `eth_nft` to another chain (allowed while nothing is open). On a chain whose blocks are numbered lower every Ethereum pin is
/// refused for good; on any other chain our lower nonce there is refused for good. Only another code upgrade resets it.
#[test]
fn f5_l1_the_ethereum_pin_floor_is_not_keyed_by_chain_and_survives_a_chain_move() {
    let mut fl = etch_autobuyer::pin::PinFloors::default();
    fl.eth.admit_pin("Ethereum", ETH_PIN).unwrap(); // chain 1
    fl.eth.admit_nonce("Ethereum", 40).unwrap();
    // eth_nft moved to another chain listed in evm_chains (a chain with lower block numbers; our address fresh there)
    let e1 = fl.eth.admit_pin("Ethereum", 9_300_000).unwrap_err();
    let e2 = fl.eth.admit_nonce("Ethereum", 0).unwrap_err();
    println!("F5-L-1: {e1}\nF5-L-1: {e2}");
    // (the same holds for every later pin until the other chain passes block ETH_PIN, and for the nonce until it passes 40)
    let src = include_str!("../src/canister.rs");
    let post = src.split("fn post_upgrade(").nth(1).unwrap().split("\n}\n").next().unwrap();
    assert!(post.contains("PIN_FLOORS.with(|f| *f.borrow_mut() = eth.pin_floors.unwrap_or_default());"), "restored whatever the chain");
    assert!(!post.contains("pin_floors") || !post.split("pin_floors").next().unwrap().contains("chain_id ==") , "no chain check guards it");
    assert!(src.contains("kept.eth_nft = arg.eth_nft.clone();"), "the upgrade argument's eth_nft (and its chain_id) is kept");
    assert!(post.contains("if eth.line.is_some() || eth.pending.is_some() || eth.buys.iter().any(|b| !b.finished())"),
        "a chain move is refused only while work is open");
}

/// F5-M-1 (c, as FloorModule.sol is): the module still wants its item after the first advance (only the unit in transit leaves
/// `netUnits`), so the taken-back buy is advanced again; `advance` reverts `AdvanceOpen` at its estimate every tick, and the buy waits
/// in `NeedAdvance`, which holds the one Relay leg: no collection starts a buy again.
#[test]
fn f5_m1_c2_on_the_real_module_the_taken_back_advance_holds_the_relay_leg_for_every_collection() {
    let mut f = Fake::new(true);
    f.real_advance = true;
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("the advance is out");
    assert!(matches!(p.step, ef::Step::Advance { .. }), "{:?}", p.step);
    f.hide.push(p.hash);
    run_for(&mut f, &mut st, 8, 600).expect("dropped");
    // Azuki now wants an item too
    f.m(MODULE2).net_units = 2 * E;
    let mut last = String::new();
    for _ in 0..(24 * 6) {
        f.now += 600;
        fresh_modules(&mut f);
        last = tick(&mut f, &mut st).join(" / ");
    }
    let azuki = buy_of(&st, AZUKI).map(|b| b.phase);
    println!("F5-M-1(c2): after a day: Milady {:?} (in the Relay leg: {}), Azuki {:?}, module transit {}; last tick: {}",
        buy_of(&st, MILADY).map(|b| b.phase), buy_of(&st, MILADY).is_some_and(|b| b.in_relay()), azuki, f.m(MODULE).transit,
        last.split(" / ").filter(|l| l.contains("AdvanceOpen") || l.contains("buy")).collect::<Vec<_>>().join(" / "));
    assert!(buy_of(&st, MILADY).is_some_and(|b| matches!(b.phase, BPhase::NeedAdvance) && b.in_relay()));
    assert!(azuki.as_ref().is_none_or(|p| matches!(p, BPhase::NeedAdvance)) && home_calls_to(&f, MODULE2, rc::ADVANCE) == 0,
        "Azuki never advances: {azuki:?}");
}

/// F5-M-1 (e): a `returnAdvance` that mined but is seen nowhere for an hour is sent again; on the real module it reverts `NoAdvance`
/// at its estimate every tick, and the buy waits in `Returning`, which holds the Relay leg (no operator exit: `eth_give_back` refuses
/// a `Returning` buy).
#[test]
fn f5_m1_e_a_mined_return_advance_seen_nowhere_holds_the_relay_leg_for_good() {
    let mut f = Fake::new(true);
    f.real_advance = true;
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    // the operator gives the advance back (no quote came)
    println!("F5-M-1(e): {}", ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap());
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("returnAdvance is out");
    assert!(matches!(p.step, ef::Step::GiveBack { .. }), "{:?}", p.step);
    assert!(!f.m(MODULE).transit, "it mined: the module's advance is closed");
    f.hide.push(p.hash);
    run_for(&mut f, &mut st, 8, 600).expect("dropped");
    f.m(MODULE2).net_units = 2 * E;
    let mut last = String::new();
    for _ in 0..(24 * 6) {
        f.now += 600;
        fresh_modules(&mut f);
        last = tick(&mut f, &mut st).join(" / ");
    }
    let exit = ef::give_back_now(&mut st, id, f.home_balance, f.now);
    println!("F5-M-1(e): after a day: Milady {:?} (Relay leg held: {}), Azuki {:?}; operator exit: {exit:?}; last tick: {}",
        buy_of(&st, MILADY).map(|b| b.phase), buy_of(&st, MILADY).is_some_and(|b| b.in_relay()), buy_of(&st, AZUKI).map(|b| b.phase),
        last.split(" / ").filter(|l| l.contains("buy 0")).collect::<Vec<_>>().join(" / "));
    assert!(buy_of(&st, MILADY).is_some_and(|b| matches!(b.phase, BPhase::Returning { .. }) && b.in_relay()));
    assert_eq!(home_calls_to(&f, MODULE2, rc::ADVANCE), 0, "Azuki never advances");
    assert!(exit.is_err(), "no operator exit");
}
