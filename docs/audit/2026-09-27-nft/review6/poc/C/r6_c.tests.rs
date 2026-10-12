// review6/poc/C: appended to a private copy of autobuyer/canister-rs/tests/audit_nft_r5.rs (at e04fa16), run with
// cargo test --test audit_nft_r5 r6_c -- --nocapture --test-threads=1   (toolchain 1.98.1). Each test PASSING shows its weakness.

// =====================================================================================================================================
// review round 6, part C (review6/poc/C): the payout judge and the credit after C5-M-1's fix
// =====================================================================================================================================

#[test]
fn r6_c_l1_a_strangers_gas_only_depository_call_naming_the_order_is_credited_at_the_stated_amount() {
    // C6-L-1: the depository path of `judge_payout` takes ANY successful call to Relay's depository that is not `depositNative` and
    // carries the order id anywhere in its input. The real depository answers `allocator()` with 32 trailing bytes (probed read-only on
    // 4663 and Ethereum, 2026-09-28): a stranger's 0-value, gas-only transaction. Handed in by the operator with a stated amount, it is
    // credited out of whatever ETH sits at the shared 4663 address (here the frog side's), and it ends the C5-M-1 hold.
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount) = milady_written_off(&mut f, &mut st);
    run_for(&mut f, &mut st, 2, 1_200);
    f.home_balance += 2 * E; // other flows' ETH at the shared address (the frog side's float, gas keeps)
    let oid = st.buy(mid).unwrap().late.as_ref().unwrap().watch.order_id;
    let crafted = [rc::ALLOCATOR.to_vec(), oid.to_vec()].concat();
    let tx = [0x5c; 32];
    f.payouts.insert((true, tx), ef::PayoutTx { from: ATTACKER, to: rc::RELAY_DEPOSITORY_4663, value: 0, input: crafted, block: HOME_PIN + 5, success: true });
    let named = amount / 100 * 98;
    let home_before = f.home_balance;
    ef::put_payout(&mut st, mid, true, tx, named, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED"), "the stranger's call is credited: {l}");
    assert!(st.late_hold().is_none(), "and the C5-M-1 hold is over");
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(mid).unwrap().phase, BPhase::Returned { .. })));
    assert_eq!(returned_to(&f, MODULE, rc::RETURN_ADVANCE), named, "returnAdvance paid the stated amount");
    // nothing arrived for Milady (the crafted call moved no ETH): the returnAdvance was paid out of the 2 ETH of other flows
    let _ = home_before;
    // the leg is free again: a new order starts, whose watch will read Milady's real payout when it lands (C5-M-1's shape)
    assert!(until(&mut f, &mut st, 8, |s| buy_of(s, AZUKI).is_some()), "a new Relay order starts");
    println!("C6-L-1: stranger's allocator()+orderId call credited {named} wei; hold ended; Azuki's buy {:?}", buy_of(&st, AZUKI).map(|b| b.phase));
}

#[test]
fn r6_c_l1_b_the_judge_alone_accepts_any_depository_selector_and_any_request_id_the_runner_names() {
    // C6-L-1 / C6-I-1 (unit): the judge's depository path needs only "to == depository, not depositNative, names the order"; and the
    // request id is whatever the runner's quote says (not bound by the order hash): a request id equal to our own address word makes any
    // call whose input carries our address "name" the order
    let us_word: [u8; 32] = addr_word(&US).try_into().unwrap();
    let w = ef::Watch { order_id: [0x0d; 32], value: 5 * E, min_out: 49 * E / 10, deadline: 1, balance_after: 0, eth_before: 0,
        eth_out_before: 0, pin: Some(26_000_000), solver: Some(SOLVER), request_id: Some(us_word), home_pin: Some(70_000_000) };
    let o = ef::LateOrder { watch: w.clone(), back: false, let_go_at: 2, credited: None, released_at: None };
    // an ERC-20 transfer(us, x) SENT TO THE DEPOSITORY by anyone "names" the order through the request id; credited at 3 ETH stated
    let erc20 = [vec![0xa9, 0x05, 0x9c, 0xbb], addr_word(&US), word(1)].concat();
    let t = ef::PayoutTx { from: ATTACKER, to: rc::RELAY_DEPOSITORY_4663, value: 0, input: erc20, block: 70_000_001, success: true };
    assert_eq!(ef::judge_payout(&o, &t, &[0x01; 32], true, &US, 3 * E, &[]), Ok(3 * E));
    // a plain ETH transfer to us of at least half the value, data = the public order id: credited at its value (it is real ETH, paid
    // by whoever sent it: a griefer who pays half the order ends the hold)
    let t2 = ef::PayoutTx { from: ATTACKER, to: US, value: 5 * E / 2, input: w.order_id.to_vec(), block: 70_000_001, success: true };
    assert_eq!(ef::judge_payout(&o, &t2, &[0x02; 32], true, &US, 0, &[]), Ok(5 * E / 2));
    // a watch whose home pin read failed at deposit (`home_pin().ok().flatten()` → None) drops rule 3 on 4663 entirely
    let mut o3 = o.clone();
    o3.watch.home_pin = None;
    let t3 = ef::PayoutTx { from: ATTACKER, to: US, value: 5 * E / 2, input: w.order_id.to_vec(), block: 1, success: true };
    assert!(ef::judge_payout(&o3, &t3, &[0x03; 32], true, &US, 0, &[]).is_ok(), "a transaction from block 1 passes");

#[test]
fn r6_c_l2_credit_while_the_loss_return_is_out_is_recorded_as_returned_and_never_paid() {
    // C6-L-2: a written-off order closed as a loss (eth_give_back twice: returnAdvance of 0 out) and its late payout credited in a tick
    // where that returnAdvance(0) is not yet resolved (the receipt read is Unsure, or not yet visible at the pin): `payout_pass` runs
    // regardless of `st.pending` and rewrites the phase to Returning{paid}; the pending returnAdvance(0) then resolves and the buy is
    // recorded Returned{paid}, while the module got 0. The credit is spent (the transaction can never be handed in again), the payout
    // sits unattributed at our 4663 address.
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount) = milady_written_off(&mut f, &mut st);
    let l = ef::give_back_now(&mut st, mid, f.home_balance, f.now).unwrap();
    assert!(l.contains("closed as a loss"), "{l}");
    // the next tick signs returnAdvance(0)
    f.now += 600;
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("returnAdvance(0) is out");
    assert!(matches!(p.step, ef::Step::GiveBack { .. }), "{:?}", p.step);
    // Relay pays the way back after all (on 4663, naming the order); the operator hands it in
    let oid = st.buy(mid).unwrap().late.as_ref().unwrap().watch.order_id;
    let payout = amount / 100 * 98;
    f.home_balance += payout;
    f.payouts.insert((true, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: payout, input: oid.to_vec(), block: HOME_PIN + 5, success: true });
    ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).unwrap();
    // this tick the returnAdvance(0)'s receipt is Unsure: it stays pending while the payout is judged
    f.unsure = true;
    f.now += 600;
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED"), "{l}");
    f.unsure = false;
    run_for(&mut f, &mut st, 4, 600);
    let b = st.buy(mid).unwrap();
    println!("C6-L-2: phase {:?}; returned to Milady's module {} wei; late.credited {:?}", b.phase, returned_to(&f, MODULE, rc::RETURN_ADVANCE),
        b.late.as_ref().unwrap().credited.map(|c| c.1));
    assert!(matches!(b.phase, BPhase::Returned { wei } if wei == payout), "recorded as returned with the payout: {:?}", b.phase);
    assert_eq!(returned_to(&f, MODULE, rc::RETURN_ADVANCE), 0, "... but the module was paid nothing");
    assert!(b.late.as_ref().unwrap().credited.is_some());
    assert!(ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).is_err(), "and the payout can never be handed in again");
}

#[test]
fn r6_c_l2_b_on_ethereum_the_credit_revives_a_buy_whose_module_advance_the_pending_return_closes() {
    // C6-L-2, the Ethereum branch: the forward order written off and closed as a loss (returnAdvance(0) out, unresolved this tick); its
    // late fill on Ethereum credited: `arrive` makes the buy Arrived again; the returnAdvance(0) then resolves ("not returning": nothing
    // changes), so the buy holds ETH on Ethereum for a module whose advance is closed (the module booked the whole advance as a loss),
    // and its custody reservation was released by that same resolution
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    st.used_orders.clear();
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let BPhase::Bridging { deadline, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    f.now = deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 60;
    fresh_modules(&mut f);
    tick(&mut f, &mut st);
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("written off"));
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("closed as a loss"));
    f.now += 600;
    fresh_modules(&mut f);
    tick(&mut f, &mut st);
    assert!(matches!(st.pending.as_ref().map(|p| &p.step), Some(ef::Step::GiveBack { .. })), "returnAdvance(0) is out");
    let oid = st.buy(id).unwrap().late.as_ref().unwrap().watch.order_id;
    f.eth_balance += ARRIVED;
    f.pin_now = Some(ETH_PIN + 300);
    f.finalized = ETH_PIN + 250;
    f.payouts.insert((false, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: ARRIVED, input: oid.to_vec(), block: ETH_PIN + 200, success: true });
    ef::put_payout(&mut st, id, false, PAYOUT_TX, 0, f.now).unwrap();
    f.unsure = true;
    f.now += 600;
    fresh_modules(&mut f);
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED"), "{l}");
    f.unsure = false;
    f.now += 600;
    fresh_modules(&mut f);
    let l = tick(&mut f, &mut st).join("\n");
    let b = st.buy(id).unwrap().clone();
    let transit = f.m(MODULE).transit;
    println!("C6-L-2b: phase {:?}; module transit open {}; returnAdvance paid {}; resolution: {}", b.phase, transit,
        returned_to(&f, MODULE, rc::RETURN_ADVANCE), l.lines().find(|x| x.contains("not returning") || x.contains("returnAdvance")).unwrap_or("-"));
    assert!(matches!(b.phase, BPhase::Arrived { .. }), "{:?}", b.phase);
    assert!(!transit, "the module's advance is closed: it booked the whole advance as a loss");
}

#[test]
fn r6_c_l3_a_credited_forward_payout_buys_an_item_the_custody_book_never_counts() {
    // C6-L-3: the write-off of a forward order leaves it Stuck with no watch, so buy_pass releases its custody reservation `eth:{id}`
    // (the B-M-4 sweep of reservations that can no longer land). A late fill credited on Ethereum (`payout_pass` (false, open) → `arrive`)
    // makes the buy Arrived again with NO reservation; it then fills a listing and mints, and `custodycap::settle(eth:{id}, held)` fails
    // ("not in flight"): the item is in custody and on the wrapper, but not in the custody book (H2's count and the exit rule's `held_fits`)
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    let flight = format!("eth:{id}");
    assert!(f.book.entries.iter().any(|e| e.id == flight && e.in_flight), "the advance reserved its room");
    st.used_orders.clear();
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let BPhase::Bridging { deadline, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    f.now = deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 60;
    fresh_modules(&mut f);
    tick(&mut f, &mut st);
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("written off"));
    f.now += 600;
    fresh_modules(&mut f);
    let l = tick(&mut f, &mut st).join("\n");
    assert!(!f.book.entries.iter().any(|e| e.id == flight), "the write-off released the reservation: {l}");
    // Relay fills it after all; the operator credits the fill
    let oid = st.buy(id).unwrap().late.as_ref().unwrap().watch.order_id;
    f.eth_balance += ARRIVED;
    f.pin_now = Some(ETH_PIN + 300);
    f.finalized = ETH_PIN + 250;
    f.payouts.insert((false, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: ARRIVED, input: oid.to_vec(), block: ETH_PIN + 200, success: true });
    ef::put_payout(&mut st, id, false, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED") && matches!(phase(&st), BPhase::Arrived { .. }), "{l}");
    let entries0 = f.book.entries.len();
    // it fills a listing and mints to the module
    f.pin_now = None;
    f.finalized = 0;
    evidence(&f, &mut st);
    ef::put_listing(&mut st, id, fill_hex(), f.now).unwrap();
    let mut all = tick(&mut f, &mut st);
    f.mine_eth(f.eth_signed.len() - 1, true, ETH_PIN - 10);
    all.extend(tick(&mut f, &mut st));
    f.finalized = ETH_PIN;
    for _ in 0..6 {
        f.now += 600;
        fresh_modules(&mut f);
        all.extend(tick(&mut f, &mut st));
    }
    let all = all.join("\n");
    println!("C6-L-3: phase {:?}; book entries {} -> {}; custody line: {}", phase(&st), entries0, f.book.entries.len(),
        all.lines().find(|x| x.contains("custody book")).unwrap_or("-"));
    assert!(matches!(phase(&st), BPhase::Minted { .. } | BPhase::Settling { .. } | BPhase::Settled { .. }), "{:?}\n{all}", phase(&st));
    assert!(!f.book.contains(&held_id(MILADY, 9_180)), "the item in custody is not in the custody book");
    assert!(all.contains("not in flight"), "{all}");
}
