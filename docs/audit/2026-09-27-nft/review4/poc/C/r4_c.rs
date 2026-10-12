
// =====================================================================================================================================
// Review round 4, part C (docs/audit/2026-09-27-nft/review4/R4-canister-wayback.md): each test PASSES while the weakness is there
// =====================================================================================================================================

/// A Relay quote 4663 → Ethereum for `amount` at `now` (the measured fixture, its deadlines moved to now + 7 days and re-hashed:
/// every rule of `relayeth::judge` holds on it), as `back_quote` does for the way back.
fn fwd_quote(amount: u128, now: u64) -> Vec<u8> {
    let us = hex::encode(US);
    let mut v: serde_json::Value = serde_json::from_str(QUOTE).unwrap();
    let a = amount.to_string();
    v["steps"][0]["items"][0]["data"]["value"] = a.clone().into();
    {
        let o = &mut v["protocol"]["v2"]["orderData"];
        o["inputs"][0]["payment"]["amount"] = a.clone().into();
        for r in o["inputs"][0]["refunds"].as_array_mut().unwrap() {
            r["deadline"] = (now + 7 * 86_400).into();
        }
        o["output"]["deadline"] = (now + 7 * 86_400).into();
        o["output"]["payments"][0]["minimumAmount"] = (amount / 1000 * 977).to_string().into();
    }
    v["protocol"]["v2"]["paymentDetails"]["amount"] = a.into();
    let id = rc::order_id(&v["protocol"]["v2"]["orderData"]).unwrap();
    v["steps"][0]["items"][0]["data"]["data"] = format!("0x49290c1c{}{us}{}", "0".repeat(24), hex::encode(id)).into();
    v["protocol"]["v2"]["orderId"] = format!("0x{}", hex::encode(id)).into();
    serde_json::to_vec(&v).unwrap()
}

fn relay_deposits(f: &Fake) -> usize {
    f.home_calls.iter().filter(|(to, ..)| *to == rc::RELAY_DEPOSITORY_4663).count()
}

// ---- C4-H-1: the 4663 half of B-M-4 was never fixed: a home pin one bucket back reads our Relay deposit as refunded, and the next
// home transaction is signed at a nonce already used: the one 4663 line of the whole canister waits for ever ---------------------------

#[test]
fn c4_h1_a_4663_pin_one_bucket_back_reads_the_deposit_as_refunded_and_wedges_the_home_line_for_good() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    ef::put_quote(&mut st, id, fwd_quote(DEPOSIT, f.now), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Bridging { .. }), "{:?}", phase(&st));
    assert_eq!(relay_deposits(&f), 1);
    let nonce_before = f.home_nonce;
    // one of the two 4663 providers answers a head one bucket (1,024 blocks, about 100 s) lower: `choose_pin` accepts it (the heads
    // are one bucket apart) and the pin is one bucket lower than last tick's, under the block our Relay deposit mined in
    f.home_back = Some((DEPOSIT, 1));
    f.now += 60;
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("REFUNDED"), "the deposit's own spend, unseen at the lower pin, reads as Relay's refund:\n{l}");
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    // the runner hands in the next quote (it is asked for one); at the same lower pin it is signed at the deposit's own nonce
    ef::put_quote(&mut st, id, fwd_quote(DEPOSIT, f.now), f.now).unwrap();
    f.now += 60;
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("a home transaction is out");
    assert_eq!(p.nonce, nonce_before - 1, "signed at the nonce our first deposit already used");
    // the provider catches up; Relay fills the FIRST order on Ethereum as it always would
    f.home_back = None;
    f.eth_balance += ARRIVED;
    let calls = f.home_calls.len();
    let mut waiting = 0;
    for _ in 0..(3 * 24 * 3) {
        f.now += 1_200;
        fresh_modules(&mut f);
        waiting += tick(&mut f, &mut st).iter().filter(|l| l.contains("waiting rather than guessing")).count();
    }
    println!("C4-H-1: after 3 days: pending {:?} at nonce {} (chain nonce {}), buy {:?}, home calls since {}, relay deposits {}, \
        Ethereum balance {} of which earmarked {}", st.pending.as_ref().map(|p| p.step.clone()), p.nonce, f.home_nonce, phase(&st),
        f.home_calls.len() - calls, relay_deposits(&f), f.eth_balance, st.earmarked_except(u64::MAX));
    assert!(st.pending.as_ref().is_some_and(|x| x.hash == p.hash), "the stale-nonce transaction is still the one home transaction out");
    assert!(waiting >= 200, "every tick: 'nonce … is used but … not visible yet: waiting rather than guessing'");
    assert_eq!(f.home_calls.len(), calls, "nothing reaches 4663 again: no floor post, record, mark, mint or noteSent, for any flow");
    assert!(matches!(phase(&st), BPhase::Depositing { .. }), "the buy holds the Relay leg for good: {:?}", phase(&st));
    assert!(st.relay_busy());
    assert_eq!(st.earmarked_except(u64::MAX), 0, "the first order's real arrival on Ethereum is nobody's: its watch was dropped");
}

#[test]
fn c4_h1b_any_home_act_signed_at_a_lower_pin_after_a_mined_one_wedges_the_line_a_floor_post() {
    // no Relay at all: a floor post mined, the next tick's pin is one bucket back, the next due post is signed at the used nonce
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).posted_at = f.now - 3_000; // urgent: the module was last posted 50 min ago
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    assert!(until(&mut f, &mut st, 3, |s| s.pending.is_none()) && !f.m(MODULE).posted.is_empty(), "the first post mined");
    let posts = f.m(MODULE).posted.len();
    f.home_back = Some((0, 1));
    f.now += 60;
    f.m(MODULE).posted_at = f.now - 3_000;
    // the market moved a little: the next post is not the same transaction as the one that mined (a real re-sign differs anyway: the
    // fee is the lower pin's base fee × 4, the gas its estimate)
    let moved = vec![format!("1:{}", 94 * E / 100), format!("2:{}", 95 * E / 100), format!("3:{}", 97 * E / 100)];
    ef::put_evidence(&mut st, &f.cfg(), MILADY, moved, real_sales(), f.now).unwrap();
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("the next post is out");
    assert_eq!(p.nonce, f.home_nonce - 1, "signed at the nonce the first post used");
    f.home_back = None;
    for _ in 0..200 {
        f.now += 600;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    let now_posts = f.m(MODULE).posted.len();
    println!("C4-H-1 (b): pending {:?} nonce {} vs chain {}; posts {} -> {}", p.step, p.nonce, f.home_nonce, posts, now_posts);
    assert!(st.pending.as_ref().is_some_and(|x| x.hash == p.hash) && p.nonce < f.home_nonce);
    assert_eq!(f.m(MODULE).posted.len(), posts, "no floor is ever posted again: the module goes stale in 1 h, mints and burns revert");
}

// ---- C4-M-1: a way back let go after BACK_LATE_SECS: Relay's late payout is nobody's (the operator can only book a loss), or is read as
// another collection's refund -------------------------------------------------------------------------------------------------------

/// Milady's way back, Relay neither filling nor refunding, past deadline + grace + BACK_LATE_SECS: Stuck, unwatched. Returns
/// (buy id, what the way back sent).
fn to_late_way_back(f: &mut Fake, st: &mut EthState) -> (u64, u128) {
    let id = to_back_funded(f, st);
    let amount = to_back_bridging(f, st, id);
    let BPhase::BackBridging { deadline, .. } = st.buy(id).unwrap().phase else { unreachable!() };
    println!("the way back's order deadline is {} s after it was quoted (Relay's measured quotes: 7 days; the judge allows 1 to 8)",
        deadline - f.now);
    while !matches!(st.buy(id).unwrap().phase, BPhase::Stuck { .. }) {
        f.now += 1_200;
        fresh_modules(f);
        tick(f, st);
    }
    assert!(st.buy(id).unwrap().last.is_none());
    (id, amount)
}

#[test]
fn c4_m1_a_late_way_back_payout_is_nobodys_after_the_operator_closes_it_as_a_loss() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let (id, amount) = to_late_way_back(&mut f, &mut st);
    // the operator closes it "once Relay says the order is dead": returnAdvance of 0, the module books the whole basis as a loss
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("closed as a loss"));
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { wei: 0 })));
    assert!(f.home_calls.iter().any(|(to, v, d)| *to == MODULE && *v == 0 && sel(d) == rc::RETURN_ADVANCE), "loss booked");
    // Relay pays the way back late (its refund process after the deadline), to our 4663 address, as the order said
    let payout = amount / 100 * 98;
    let home0 = f.home_balance;
    f.home_balance += payout;
    for _ in 0..50 {
        f.now += 1_200;
        fresh_modules(&mut f);
        tick(&mut f, &mut st);
    }
    let again = ef::give_back_now(&mut st, id, f.home_balance, f.now);
    println!("C4-M-1 (a): {payout} wei landed at our 4663 address after the close; home balance {} (was {home0}); eth_give_back: {again:?}; \
        the module got {} wei back", f.home_balance, f.home_calls.iter().filter(|(to, _, d)| *to == MODULE && sel(d) == rc::RETURN_ADVANCE)
        .map(|(_, v, _)| *v).sum::<u128>());
    assert!(f.home_balance >= home0 + payout - E / 1000, "the ETH is at our address …");
    assert!(again.is_err(), "… and no path returns it to the module that lost it (only a timelocked upgrade)");
}

#[test]
fn c4_m1b_a_late_way_back_payout_is_read_as_another_collections_refund_and_orphans_its_real_arrival() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount) = to_late_way_back(&mut f, &mut st);
    // the Relay leg is free: Azuki buys
    f.m(MODULE2).net_units = 2 * E;
    assert!(until(&mut f, &mut st, 6, |s| buy_of(s, AZUKI).is_some_and(|b| matches!(b.phase, BPhase::Funded { .. }))));
    let az = buy_of(&st, AZUKI).unwrap().id;
    let ask = st.buy(az).unwrap().ask.clone().unwrap().amount_wei;
    ef::put_quote(&mut st, az, fwd_quote(ask, f.now), f.now).unwrap();
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Bridging { .. })));
    // while Azuki's order is on its way, Milady's way back is paid out late on 4663
    f.home_balance += amount / 100 * 98;
    fresh_modules(&mut f);
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("REFUNDED"), "Milady's payout is Azuki's 'refund':\n{l}");
    // Relay fills Azuki's real order on Ethereum (nobody watches it any more); Azuki re-deposits Milady's ETH, and that fills too
    f.eth_balance += ARRIVED;
    let ask = st.buy(az).unwrap().ask.clone().unwrap().amount_wei;
    ef::put_quote(&mut st, az, fwd_quote(ask, f.now), f.now).unwrap();
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Bridging { .. })));
    f.eth_balance += ARRIVED;
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Arrived { .. })));
    // the operator closes Milady's way back as Relay's status says: a loss
    ef::give_back_now(&mut st, mid, f.home_balance, f.now).unwrap();
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(mid).unwrap().phase, BPhase::Returned { wei: 0 })));
    let unattributed = f.eth_balance.saturating_sub(st.earmarked_except(u64::MAX)).saturating_sub(POT);
    println!("C4-M-1 (b): Azuki's Relay deposits {}; Milady returned 0 (a loss of its whole advance); Ethereum holds {unattributed} wei over \
        the gas pot that is nobody's (leftover booked {} / {})", relay_deposits(&f) - 1, st.leftover_of(&MILADY), st.leftover_of(&AZUKI));
    assert_eq!(relay_deposits(&f) - 1, 2, "Azuki bridged twice: its own ETH and Milady's");
    assert!(unattributed >= ARRIVED - E / 100, "one arrival on Ethereum belongs to no buy and no collection's leftover");
}

// ---- C4-L-1: the way back's bound is the order's own deadline (7 days in Relay's quotes): every collection's buys wait that long -------

#[test]
fn c4_l1_a_way_back_relay_does_not_fill_holds_every_collections_buys_for_the_orders_whole_deadline() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let id = to_back_funded(&mut f, &mut st);
    to_back_bridging(&mut f, &mut st, id);
    let went = f.now;
    f.m(MODULE2).net_units = 2 * E;
    // the operator's exit is refused before the deadline, whatever Relay's status page says
    let mut refused_until = went;
    while ef::give_back_now(&mut st.clone(), id, 0, f.now).is_err() {
        refused_until = f.now;
        f.now += 3_600;
        fresh_modules(&mut f);
        tick(&mut f, &mut st);
        assert!(buy_of(&st, AZUKI).is_none(), "no other collection buys while the way back is watched");
    }
    println!("C4-L-1: eth_give_back refused for {} h; Azuki waited all that time", (refused_until - went) / 3_600);
    assert!(refused_until - went >= 7 * 86_400 - 3_600);
}
