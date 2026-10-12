
// =====================================================================================================================================
// Review round 5, part C (the way back, C4-M-1's fix): appended to a copy of tests/audit_nft_r4.rs (harness unchanged).
// Each test PASSES while the weakness is present.
// =====================================================================================================================================

/// Milady's way back, Relay neither filling nor refunding, a day past its deadline and grace; the operator writes it off (Relay's
/// status says nothing will come) and the Relay leg is free. Azuki then buys and its forward order is `Bridging`.
/// Returns (Milady's buy, what its way back sent, Azuki's buy, Azuki's order value).
fn azuki_bridging_after_milady_write_off(f: &mut Fake, st: &mut EthState) -> (u64, u128, u64, u128) {
    let mid = to_back_funded(f, st);
    let amount = to_back_bridging(f, st, mid);
    let BPhase::BackBridging { deadline, .. } = st.buy(mid).unwrap().phase else { unreachable!() };
    f.m(MODULE2).net_units = 2 * E;
    while f.now <= deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 60 {
        f.now += 1_200;
        fresh_modules(f);
        tick(f, st);
    }
    assert!(buy_of(st, AZUKI).is_none(), "no other order starts while Milady's order is watched");
    let l = ef::give_back_now(st, mid, f.home_balance, f.now).unwrap();
    assert!(l.contains("written off") && !st.relay_busy(), "{l}");
    assert!(until(f, st, 8, |s| buy_of(s, AZUKI).is_some_and(|b| matches!(b.phase, BPhase::Funded { .. }))), "{:?}", buy_of(st, AZUKI));
    let az = buy_of(st, AZUKI).unwrap().id;
    let ask = st.buy(az).unwrap().ask.clone().unwrap().amount_wei;
    ef::put_quote(st, az, fwd_quote(ask, f.now), f.now).unwrap();
    assert!(until(f, st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Bridging { .. })), "{:?}", st.buy(az).unwrap().phase);
    let BPhase::Bridging { value, .. } = st.buy(az).unwrap().phase else { unreachable!() };
    println!("C5: Milady's way back (buy {mid}, {amount} wei) is written off; Azuki's order (buy {az}, {value} wei) is Bridging");
    (mid, amount, az, value)
}

const PAYOUT_TX: [u8; 32] = [0x5a; 32];

#[test]
fn c5_m1_a_payout_after_the_write_off_is_still_read_as_another_collections_refund() {
    // C4-M-1(b), round 4's PoC `c4_m1b_…`, with the one step round 4 added in front: the operator's write-off. The fixer's test
    // `r4_c4_m1b_…` lands the payout BEFORE the write-off (while Milady's watch holds the leg); eth_credit_payout exists for the payout
    // that lands AFTER it, and then the leg is free again
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount, az, _) = azuki_bridging_after_milady_write_off(&mut f, &mut st);
    let deposits0 = relay_deposits(&f) - 1;
    // Relay pays Milady's way back after all (a late fill on 4663, from its solver): the case eth_credit_payout was built for
    let payout = amount / 100 * 98;
    f.home_balance += payout;
    f.payouts.insert((true, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: payout, input: vec![], block: 70_000_000, success: true });
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("REFUNDED"), "Azuki's watch reads Milady's payout as its own refund: {l}");
    assert!(matches!(st.buy(az).unwrap().phase, BPhase::Funded { .. }), "{:?}", st.buy(az).unwrap().phase);
    // the operator hands in Milady's payout, as the runbook says: refused, the ETH is now "Azuki's"
    ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("refused, nothing credited") && l.contains("beside the open buys"), "{l}");
    // Azuki bridges Milady's ETH (a second Relay deposit) while its first order fills on Ethereum
    let ask = st.buy(az).unwrap().ask.clone().unwrap().amount_wei;
    f.now += 60;
    ef::put_quote(&mut st, az, fwd_quote(ask, f.now), f.now).unwrap();
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Bridging { .. })));
    assert_eq!(relay_deposits(&f) - deposits0, 2, "Azuki bridged twice");
    f.eth_balance += ARRIVED; // Azuki's first order's fill
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Arrived { .. })));
    f.eth_balance += ARRIVED; // Azuki's second order's fill (Milady's ETH)
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    let held = st.earmarked_except(u64::MAX) + st.leftover.iter().flatten().map(|(_, w)| *w).sum::<u128>();
    let orphan = f.eth_balance.saturating_sub(held).saturating_sub(POT);
    // Milady: the operator closes it as a loss; the module gets nothing back
    ef::give_back_now(&mut st, mid, f.home_balance, f.now).unwrap();
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(mid).unwrap().phase, BPhase::Returned { wei: 0 })));
    let back = returned_to(&f, MODULE, rc::RETURN_ADVANCE) + returned_to(&f, MODULE, ef::FUND_RESERVE);
    println!("C5-M-1: Milady's late payout {payout} was read as Azuki's refund; Azuki bridged twice; Milady's module got {back}; \
        {orphan} wei at our Ethereum address is nobody's");
    assert_eq!(back, 0);
    assert!(orphan >= ARRIVED - E / 100, "an arrival on Ethereum belongs to nobody: {orphan}");
}

#[test]
fn c5_l1_a_the_judge_credits_another_relay_payment_from_the_shared_solver_paid_from_the_frog_sides_eth() {
    // `orderData.solver` is Relay's one solver (0xf70d…dbef in BOTH measured quotes, both directions): "sent by the order's solver" is
    // true of every Relay payment to us, of any order (our other buys', the frog side's conversions' refunds). The balance check on
    // 4663 counts every ETH that is not an NFT buy's Funded/Returning as free
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let (id, amount, deadline) = way_back_past(&mut f, &mut st, 2);
    let _ = deadline;
    f.now += 3_600;
    ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap();
    ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap();
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { wei: 0 })));
    // 2 ETH of the frog side at the shared 4663 address (B-L-6): a Relay refund of one of its conversions, paid by the same solver,
    // whose input carries that conversion's own request id, not Milady's order id. Milady's order is never paid
    let frog = 2 * E;
    f.home_balance += frog;
    let before = f.home_balance;
    let frog_refund = amount / 100 * 98;
    let other_request = [0x17; 32];
    f.payouts.insert((true, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: frog_refund, input: other_request.to_vec(), block: 1,
        success: true });
    ef::put_payout(&mut st, id, true, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED"), "{l}");
    for _ in 0..4 {
        tick(&mut f, &mut st);
    }
    let got = returned_to(&f, MODULE, ef::FUND_RESERVE);
    println!("C5-L-1(a): a solver payment of another order (mined in block 1, before Milady's order existed) credited as Milady's \
        payout: {got} wei of the frog side's ETH paid into Milady's module (fundReserve)");
    assert_eq!(got, frog_refund);
    assert_eq!(f.home_balance, before - frog_refund, "the frog side's ETH left our address");
}

#[test]
fn c5_l1_b_the_judge_takes_our_own_deposit_or_any_call_carrying_the_id_at_the_amount_the_operator_states() {
    let w = ef::Watch { order_id: [0x0d; 32], value: 5 * E, min_out: 49 * E / 10, deadline: 1, balance_after: 0, eth_before: 0,
        eth_out_before: 0, pin: Some(26_000_000), solver: Some(SOLVER) };
    let o = ef::LateOrder { watch: w.clone(), back: true, let_go_at: 2, credited: None };
    // our own Relay deposit of this very order (depositNative(us, id), from us to the depository): the outflow, not a payout
    let dep = [rc::DEPOSIT_NATIVE.to_vec(), addr_word(&US), w.order_id.to_vec()].concat();
    let ours = ef::PayoutTx { from: US, to: rc::RELAY_DEPOSITORY_4663, value: 5 * E, input: dep, block: 25_999_999, success: true };
    assert_eq!(ef::judge_payout(&o, &ours, &[0x0a; 32], false, &US, 5 * E, &[]), Ok(5 * E));
    // anyone's 0-value call to anything, with the id anywhere in its input (unaligned too), at whatever the operator states
    let junk = [vec![0xde, 0xad], w.order_id.to_vec(), vec![0xbe]].concat();
    let stranger = ef::PayoutTx { from: ATTACKER, to: ATTACKER, value: 0, input: junk, block: 1, success: true };
    assert_eq!(ef::judge_payout(&o, &stranger, &[0x0b; 32], true, &US, 5 * E, &[]), Ok(5 * E));
    println!("C5-L-1(b): our own deposit and a stranger's 0-value call naming the id are both judged payouts of 5 ETH");
}

#[test]
fn c5_l1_c_a_payout_is_credited_from_a_watched_orders_unread_refund_and_that_order_then_holds_the_relay_leg() {
    // payout_pass runs before buy_pass: on the tick an open watch's refund lands, it is "free" (the check counts only Funded and
    // Returning). A payout handed in then (here: the solver's refund of Azuki's order itself, which the judge takes for Milady's) is paid
    // out of Azuki's refund; Azuki's watch then reads the same rise as its own
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount, az, value) = azuki_bridging_after_milady_write_off(&mut f, &mut st);
    // Relay refunds Azuki's order on 4663 (from its solver); Milady's order is never paid
    let refund = value / 100 * 99;
    assert!(refund <= amount && refund * 2 >= amount, "in the judge's range for Milady's order");
    f.home_balance += refund;
    f.payouts.insert((true, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: refund, input: vec![], block: 70_000_000, success: true });
    ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED") && l.contains("REFUNDED"), "{l}");
    assert!(matches!(st.buy(mid).unwrap().phase, BPhase::Returning { .. }) && matches!(st.buy(az).unwrap().phase, BPhase::Funded { .. }));
    for _ in 0..30 {
        f.now += 600;
        fresh_modules(&mut f);
        tick(&mut f, &mut st);
    }
    let to_milady = returned_to(&f, MODULE, rc::RETURN_ADVANCE);
    println!("C5-L-1(c): Azuki's refund {refund} went to Milady's module ({to_milady}); Azuki is {:?}, holding the Relay leg ({}), with {} \
        wei at our 4663 address", st.buy(az).unwrap().phase, st.relay_busy(), f.home_balance);
    assert_eq!(to_milady, refund);
    assert!(matches!(st.buy(az).unwrap().phase, BPhase::Funded { .. }) && st.relay_busy());
    assert!(f.home_balance < refund / 2, "Azuki's Funded ETH is not there");
}

#[test]
fn c5_l3_a_failed_balance_read_after_the_judge_marks_a_payout_credited_that_was_never_applied() {
    // payout_pass writes `st.credited` and `late.credited` BEFORE the (false, open) branch reads the Ethereum balance again for
    // `arrive`; that read is an outcall and may fail. The claim is consumed, the transaction and the order are "credited", the buy
    // did not move, and no later hand-in can credit it
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
    // Relay fills it after all, on Ethereum (from its solver), final
    f.eth_balance += ARRIVED;
    f.payouts.insert((false, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: ARRIVED, input: vec![], block: ETH_PIN - 200, success: true });
    ef::put_payout(&mut st, id, false, PAYOUT_TX, 0, f.now).unwrap();
    f.payout_balance_fail = true;
    let l = tick(&mut f, &mut st).join("\n");
    f.payout_balance_fail = false;
    assert!(l.contains("refused, nothing credited") && l.contains("429"), "{l}");
    let b = st.buy(id).unwrap();
    println!("C5-L-3: the claim was refused ({}), yet late.credited = {:?}, st.credited = {:?}, the buy is {:?}", 
        l.lines().find(|x| x.contains("eth payout")).unwrap_or(""), b.late.as_ref().unwrap().credited, st.credited.as_ref().map(|v| v.len()), b.phase);
    assert!(b.late.as_ref().unwrap().credited.is_some() && st.credited.as_ref().is_some_and(|v| v.contains(&PAYOUT_TX)));
    assert!(matches!(b.phase, BPhase::Stuck { .. }) && st.earmarked_except(u64::MAX) == 0, "nothing was applied");
    let e = ef::put_payout(&mut st, id, false, PAYOUT_TX, 0, f.now).unwrap_err();
    assert!(e.contains("credited already"), "{e}");
    println!("C5-L-3: handed in again: {e}; {ARRIVED} wei on Ethereum is nobody's, the operator's only exit is the loss");
}
