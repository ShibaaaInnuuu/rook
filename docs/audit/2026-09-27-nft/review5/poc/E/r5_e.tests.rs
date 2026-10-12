// =====================================================================================================================================
// Review round 5, part E. Appended to the harness by build_harness.py. Tests that PASS show what they name: a weakness (E5-*) or what
// holds (the controls, "holds_*").
// =====================================================================================================================================

/// The most real mints of `wrapper` inside any 24 h window (H3's window: `now - at < 86_400`).
fn most_mints_in_a_day(f: &Fake, wrapper: [u8; 20]) -> (usize, u64) {
    let mut t: Vec<u64> = f.mint_log.iter().filter(|(w, _)| *w == wrapper).map(|(_, t)| *t).collect();
    t.sort();
    let mut best = (0, 0);
    for (i, a) in t.iter().enumerate() {
        let n = t[i..].iter().take_while(|b| **b < a + 86_400).count();
        if n > best.0 {
            best = (n, *a);
        }
    }
    best
}

fn milady_deposits(f: &mut Fake, st: &mut EthState, n: u8, first: u8) -> Vec<[u8; 32]> {
    let mut v = Vec::new();
    for k in 0..n {
        let tx = [first + k; 32];
        deposit_tx(f, tx, MILADY, WRAPPER, 5_000 + (first + k) as u128, RECIPIENT, ETH_PIN - 20 - k as u64);
        ef::put_deposit(st, &f.cfg(), tx, f.now).unwrap();
        v.push(tx);
    }
    v
}

fn step(f: &mut Fake, st: &mut EthState, secs: u64) -> Vec<String> {
    f.now += secs;
    fresh_modules(f);
    evidence(f, st);
    tick(f, st)
}

/// A Milady purchase whose NFT is final on Ethereum: its mint is the next home act (phase `Final`).
fn final_buy(id: u64, token: u128, now: u64) -> ef::EthBuy {
    ef::EthBuy { id, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: DEPOSIT, attempts: 1,
        phase: BPhase::Final { arrived: ARRIVED, token: tok(token), total: 45 * E / 100, tx: [0xc0 + id as u8; 32] }, orders: vec![], deposits: vec![],
        ask: None, last: None, note: String::new(), sweep: None, arrived_at: Some(now - 7 * 3_600), back_at: None, ended_at: None, late: None }
}

// ---- 1. E4-H-1: the next-mint key, re-attacked --------------------------------------------------------------------------------------

#[test]
fn holds_e4_h1_nine_deposits_and_two_purchases_of_one_collection_ask_one_key_at_a_time_and_mint_three_a_day() {
    // review4's PoC shape (nine deposits of one collection in a day) with two purchases of the SAME collection waiting to mint among
    // them (a buy and deposits racing for the collection's next-mint key), at the launch limits with the real limiter, for four days
    let mut f = Fake::new(true);
    launch_limits(&mut f);
    f.deposits_on = true;
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    let mut st = EthState::default();
    st.buys = vec![final_buy(0, 9_100, f.now), final_buy(1, 9_101, f.now)];
    st.next_buy = 2;
    milady_deposits(&mut f, &mut st, 9, 0x90);
    let (mut most, mut keys) = (0, std::collections::BTreeSet::new());
    for _ in 0..(4 * 144) {
        step(&mut f, &mut st, 600);
        most = most.max(mint_demand(&f, Some(MILADY)));
        let (o, _) = f.real_outflow.as_ref().unwrap();
        keys.extend(o.asked.iter().filter(|a| a.flow == Flow::Mint).map(|a| a.key.clone()));
        assert!(o.frozen().is_none() && !f.paused, "never frozen");
    }
    let (n, at) = most_mints_in_a_day(&f, WRAPPER);
    println!("R5-E holds: 2 purchases + 9 deposits of Milady: {} minted, the most mint asks in a window {most}, the most real mints in 24 h \
        {n} (from {at}); keys asked {:?}", f.items.len(), keys);
    assert_eq!(f.items.len(), 11, "all eleven minted");
    assert!(most <= 4, "demand is what went out in the window plus one: {most}");
    assert!(n <= 3, "never more than per_collection_mints (3) real mints of the collection in 24 h: {n}");
    assert_eq!(keys.len(), 11, "one key per mint that went out: mint:eth:<col>:0..10");
}

#[test]
fn e5_i1_a_reverted_mint_s_record_keeps_its_time_so_four_real_mints_fit_in_24_h_at_a_limit_of_three() {
    // a mint that reverts (the registry paused the listing, the wrapper's own mint limit, ...) is recorded under its key at the time it
    // was SENT, and the key is taken back (n goes down). The next mint of the collection - the same deposit or ANOTHER candidate - goes
    // out under that key (here at once, in the tick that read the revert), and `record` (idempotent per key) keeps the reverted
    // attempt's time. The window therefore frees that slot one tick (600 s) before the real mint's 24 h are up: the collection's fourth
    // real mint goes out 85,800 s after its first. The pause in between (23 h) only spaces the other two. Four real mints of one
    // collection inside 24 h at per_collection_mints 3: one extra per revert, and only for as long as the revert preceded the re-send;
    // the wrapper's own limit (H11) is still a bound. The same held before round 4 for a deposit's own key (mint:deposit:{tx})
    let mut f = Fake::new(false);
    launch_limits(&mut f);
    f.deposits_on = true;
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    let mut st = EthState::default();
    milady_deposits(&mut f, &mut st, 5, 0x90);
    f.mint_revert = 1;
    let mut t0 = None;
    for _ in 0..12 {
        let lines = step(&mut f, &mut st, 600);
        if lines.iter().any(|l| l.contains("mintDeposit reverted")) {
            t0 = Some(f.now);
            break;
        }
    }
    let t0 = t0.expect("the first mint went out and reverted");
    let (o, _) = f.real_outflow.as_ref().unwrap();
    let key0 = o.done.iter().find(|a| a.flow == Flow::Mint).cloned().expect("the reverted mint was recorded");
    println!("R5-E E5-I-1: the mint under {} reverted; recorded at {} (t0 {t0})", key0.key, key0.at);
    // the collection waits 23 h (the canister paused, as an operator's pause or a listing pause would hold it)
    f.paused = true;
    while f.now < t0 + 23 * 3_600 {
        step(&mut f, &mut st, 1_800);
    }
    f.paused = false;
    while f.now < t0 + 24 * 3_600 + 3_000 {
        step(&mut f, &mut st, 600);
    }
    let (n, at) = most_mints_in_a_day(&f, WRAPPER);
    let times: Vec<u64> = f.mint_log.iter().map(|(_, t)| t - t0).collect();
    let (o, _) = f.real_outflow.as_ref().unwrap();
    println!("R5-E E5-I-1: real mints at t0 + {times:?} s; the most in 24 h: {n} (from t0 + {} s); H3's done: {:?}", at - t0,
        o.done.iter().map(|a| (a.key.clone(), a.at.saturating_sub(t0))).collect::<Vec<_>>());
    assert!(n >= 4, "four real mints of one collection inside 24 h at a limit of 3: {n}");
    assert!(o.frozen().is_none());
}

#[test]
fn e5_l1_a_mint_misread_as_dropped_wedges_every_later_deposit_of_its_collection() {
    // round 4's `waited`: a collection whose FIRST deposit candidate returns any error is not asked again that pass. A candidate whose
    // error is permanent therefore holds every later deposit of its collection for good. One such candidate: a mintDeposit that MINED,
    // was seen nowhere for HOME_NONCE_TAKEN_SECS (C4-H-1's drop), and whose wrapper itemOf read failed at the drop (mined_home reads it
    // once; `is_ok_and` turns an error into "not minted"): the deposit is back at `Mint`, its mint's estimate reverts (AlreadyHeld)
    // every pass, and deposit B of the same collection never mints. Azuki's deposit (another collection) is unaffected (C-L-2 holds).
    // Before round 4 the loop went on to B
    let run = |wedge: bool| {
        let mut f = Fake::new(true);
        launch_limits(&mut f);
        f.deposits_on = true;
        f.finalized = ETH_PIN;
        f.m(MODULE).accepted = 93 * E / 100;
        f.m(MODULE2).accepted = 67 * E / 100;
        let mut az_sales = Vec::new();
        for i in 0..5u8 {
            let h = [0xa0 + i; 32];
            f.sale_receipts.insert(h, sale_receipt(AZUKI, 100 + i as u128, 67 * E / 100, [0x10 + i; 20], [0x50 + i; 20], ETH_PIN - 500));
            az_sales.push(h);
        }
        let mut st = EthState::default();
        let txs = milady_deposits(&mut f, &mut st, 2, 0x90);
        deposit_tx(&mut f, [0x70; 32], AZUKI, WRAPPER2, 7_000, RECIPIENT, ETH_PIN - 30);
        ef::put_deposit(&mut st, &f.cfg(), [0x70; 32], f.now).unwrap();
        let mut hidden = false;
        for _ in 0..30 {
            step(&mut f, &mut st, 600);
            azuki_evidence(&f, &mut st, &az_sales);
            if wedge && !hidden {
                if let Some(p) = st.pending.clone() {
                    if matches!(p.step, ef::Step::MintDeposit { tx } if tx == txs[0]) {
                        f.hidden.push(p.hash); // it mined; every provider fails to show it
                        hidden = true;
                        step(&mut f, &mut st, 600); // "nonce used, not visible": waits
                        f.now += ef::HOME_NONCE_TAKEN_SECS;
                        f.item_of_err = 1; // the drop's one itemOf read fails
                        let l = step(&mut f, &mut st, 60);
                        assert!(l.iter().any(|x| x.contains("DROPPED")), "{l:?}");
                    }
                }
            }
        }
        for _ in 0..(3 * 144) {
            step(&mut f, &mut st, 600);
            azuki_evidence(&f, &mut st, &az_sales);
        }
        let last = step(&mut f, &mut st, 600);
        (f, st, last)
    };
    let (f, st, last) = run(true);
    let a = st.deposits.iter().find(|d| d.tx == [0x90; 32]).unwrap().phase.clone();
    let b = st.deposits.iter().find(|d| d.tx == [0x91; 32]).unwrap().phase.clone();
    let az = st.deposits.iter().find(|d| d.tx == [0x70; 32]).unwrap().phase.clone();
    println!("R5-E E5-L-1: after 3 days: A {a:?} (its token is on the wrapper: {}), B {b:?}, Azuki {az:?}; Milady items {}, Azuki items {}",
        f.items.contains(&tok(5_000 + 0x90)), f.items.len(), f.items2.len());
    println!("R5-E E5-L-1: the last tick says: {:?}", last.iter().filter(|l| l.contains("waits")).collect::<Vec<_>>());
    assert!(f.items.contains(&tok(5_000 + 0x90)), "A's unit was minted on 4663");
    assert_eq!(a, ef::XPhase::Mint, "A is back at Mint for good");
    assert_eq!(b, ef::XPhase::Mint, "B never minted in three days");
    assert!(!f.items.contains(&tok(5_000 + 0x91)));
    assert_eq!(az, ef::XPhase::Minted, "another collection is unaffected");
    // control: the same deposits without the misread: B mints
    let (f, st, _) = run(false);
    println!("R5-E E5-L-1 control: Milady items {}, phases {:?}", f.items.len(), st.deposits.iter().map(|d| d.phase.clone()).collect::<Vec<_>>());
    assert!(f.items.contains(&tok(5_000 + 0x91)));
}

// ---- 2. The source-reading test over the H3 ask sites (r4_every_h3_ask_is_keyed_per_logical_outflow), re-run on mutated sources ------

/// `r4_every_h3_ask_is_keyed_per_logical_outflow`'s body, verbatim, over the given sources (the list and the checks unchanged).
fn r4_h3_site_check(files: &[(&str, String)], others: &[(&str, String)]) -> Result<(), String> {
    let listed: &[(&str, &str, &str)] = &[
        ("ethflow.rs", "let key = format!(\"return:eth:{}:{}\", h(&d.tx), h(&d.token));", ""),
        ("ethflow.rs", "let key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);", ""),
        ("ethflow.rs", "let key = mint_key(st, &col, &c.collection);", ""),
        ("ethflow.rs", "let key = mint_key(st, &col, &d.collection);", ""),
        ("ethflow.rs", "format!(\"advance:eth:{}:{n}\", key_of(&c, chain))", ""),
        ("convflow.rs", "format!(\"buy:frog:{}\", c.frog)", ""),
        ("frogflow.rs", "let value_key = format!(\"buy:frog:{id}\");", ""),
        ("frogflow.rs", "let key = format!(\"return:{}\", d.outpoint.text());", ""),
        ("frogflow.rs", "io.outflow_check(&format!(\"claim:{}\", w.id), &key, Flow::Mint, 1)?;", ""),
        ("frogflow.rs", "let (key, collection) = (format!(\"mint:{}\", o.text()), custodycap::btc_key(&w));", ""),
        ("frogflow.rs", "let (key, collection) = (format!(\"mint:frog:{id}\"), custodycap::btc_key(&w));", ""),
        ("handinflow.rs", "let key = format!(\"claim:handin:{}:{id}\", hex::encode(board));", ""),
        ("nftnet.rs", "let fill = format!(\"fill:{}\", plan.order_hash);", ""),
        ("redeem.rs", "let key = format!(\"ticket:{}\", r.ticket);", ""),
        ("saleflow.rs", "let key = format!(\"listing:{ticket}\");", ""),
        ("saleflow.rs", "let (key, collection) = (format!(\"mint:sale:{}\", s.id), custodycap::btc_key(&w));", ""),
        ("saleflow.rs", "let (key, collection) = (format!(\"sale:{}\", s.id), custodycap::btc_key(&io.wrapper()));", ""),
    ];
    let mut sites = 0;
    for (name, src) in files {
        let asks = src.matches("outflow_check(&").count() - src.matches("outflow_check(&mut self").count();
        sites += asks;
        let keys = listed.iter().filter(|(f, ..)| f == name).count();
        if asks != keys {
            return Err(format!("{name}: {asks} H3 ask site(s), {keys} listed"));
        }
        for (_, key, _) in listed.iter().filter(|(f, ..)| f == name) {
            if !src.contains(key) {
                return Err(format!("{name}: the key `{key}` is not in the source any more"));
            }
        }
    }
    if sites != listed.len() {
        return Err("count".into());
    }
    for (name, src) in others {
        if src.matches("outflow_check(&").count() != src.matches("outflow_check(&mut self").count() {
            return Err(format!("{name} asks H3 itself"));
        }
    }
    Ok(())
}

#[test]
fn e5_i2_the_h3_site_test_passes_on_sources_where_the_e4_h1_shape_is_back_or_a_new_site_is_unreviewed() {
    let files = || vec![("ethflow.rs", include_str!("../src/ethflow.rs").to_string()), ("convflow.rs", include_str!("../src/convflow.rs").to_string()),
        ("frogflow.rs", include_str!("../src/frogflow.rs").to_string()), ("handinflow.rs", include_str!("../src/handinflow.rs").to_string()),
        ("nftnet.rs", include_str!("../src/nftnet.rs").to_string()), ("redeem.rs", include_str!("../src/redeem.rs").to_string()),
        ("saleflow.rs", include_str!("../src/saleflow.rs").to_string())];
    let others = || vec![("canister.rs", include_str!("../src/canister.rs").to_string()), ("custodynet.rs", include_str!("../src/custodynet.rs").to_string()),
        ("frognet.rs", include_str!("../src/frognet.rs").to_string()), ("ethnet.rs", include_str!("../src/ethnet.rs").to_string())];
    // what holds: the list is complete for the sources as they are (every `outflow_check(&` call in src/ is one of its 17)
    assert_eq!(r4_h3_site_check(&files(), &others()), Ok(()));
    // (a) E4-H-1 undone: mint_key keyed per deposit/buy again (the body changed, every call site identical)
    let mut a = files();
    let old = "format!(\"mint:eth:{col}:{}\", buys + deposits)";
    assert!(a[0].1.contains(old));
    a[0].1 = a[0].1.replace(old, "format!(\"mint:eth:{col}:{}:{}\", buys + deposits, st.deposits.iter().filter(|d| d.phase == XPhase::Mint).count())");
    println!("R5-E E5-I-2 (a) mint_key made per-backlog again: {:?}", r4_h3_site_check(&a, &others()));
    assert_eq!(r4_h3_site_check(&a, &others()), Ok(()), "the test does not see it");
    // (b) the key text kept in a comment, the call site asking under a per-deposit key
    let mut b = files();
    let site = "            let key = mint_key(st, &col, &d.collection);\n            io.outflow_check(&key, &col, Flow::Mint, 1)?;";
    assert!(b[0].1.contains(site));
    b[0].1 = b[0].1.replace(site, "            // let key = mint_key(st, &col, &d.collection);\n            let key = format!(\"mint:deposit:{}\", h(&d.tx));\n            io.outflow_check(&key, &col, Flow::Mint, 1)?;");
    println!("R5-E E5-I-2 (b) the listed key only in a comment: {:?}", r4_h3_site_check(&b, &others()));
    assert_eq!(r4_h3_site_check(&b, &others()), Ok(()), "the test does not see it");
    // (c) a new ask site in a module the test does not read (looper.rs, delivery.rs, a new file, ...)
    let looper = include_str!("../src/looper.rs").to_string() + "\nfn ask(k: &str) -> Result<(), String> { crate::canister::outflow_check(&k.to_string(), \"c\", crate::outflow::Flow::Item, 1) }\n";
    assert!(looper.contains("outflow_check(&"));
    println!("R5-E E5-I-2 (c) a new site in looper.rs: {:?} (looper.rs is not read)", r4_h3_site_check(&files(), &others()));
    // (d) a new site through a helper in a listed file: `io.outflow_check(key, ..)` without `&`
    let mut d = files();
    d[0].1.push_str("\nasync fn ask_again<I: EthIo>(io: &mut I, key: &String) -> Result<(), String> { io.outflow_check(key, \"c\", Flow::Mint, 1) }\n");
    println!("R5-E E5-I-2 (d) a new site in ethflow.rs without `&`: {:?}", r4_h3_site_check(&d, &others()));
    assert_eq!(r4_h3_site_check(&d, &others()), Ok(()), "the test does not see it");
}

// ---- 3. A4-L-1's exit rule against the tripwire --------------------------------------------------------------------------------------

#[test]
fn e5_l2_at_stage_0_a_full_collection_redeemed_with_its_days_mints_trips_the_item_tripwire() {
    use etch_autobuyer::outflow::{exit_fits, validate, Limits, Outflow, Verdict};
    // the default install (custody_caps and outflow_limits null): stage 0's limits and 6 items for each Ethereum collection
    let l = Limits::stage0();
    validate(&l).unwrap();
    let key = custodycap::evm_key(1, &MILADY);
    let caps = custodycap::stage0(&[key.clone()]);
    let items = caps.cap(&key).unwrap().max_items;
    assert!(exit_fits(items, l.per_collection_items, 72 * 3_600), "the install check (per collection) passes");
    assert!(exit_fits(12, l.global_items, 72 * 3_600), "and the whole (the default global is capped at 12)");
    let mut o = Outflow::default();
    let t0 = NOW;
    // a bank run: every item of the collection redeemed; 3 deliveries go out, 3 wait (each ticket its own key, one ask each)
    for k in 0..items {
        let v = o.check(&l, &format!("deliver:0xaa:{k}"), &key, Flow::Item, 1, t0 + k as u64 * 60);
        if v == Verdict::Allow {
            o.record(&format!("deliver:0xaa:{k}"), &key, Flow::Item, 1, t0 + k as u64 * 60);
        }
    }
    // the day's mints of the collection (deposits or purchases: 3 a day) land in the room the deliveries freed, and are redeemed
    for m in 0..3u64 {
        assert_eq!(o.check(&l, &format!("mint:eth:{key}:{m}"), &key, Flow::Mint, 1, t0 + 3_600 + m * 600), Verdict::Allow);
        o.record(&format!("mint:eth:{key}:{m}"), &key, Flow::Mint, 1, t0 + 3_600 + m * 600);
    }
    let mut verdicts = Vec::new();
    for k in 0..3u32 {
        verdicts.push(o.check(&l, &format!("deliver:0xaa:{}", items + k), &key, Flow::Item, 1, t0 + 4 * 3_600 + k as u64 * 600));
    }
    println!("R5-E E5-L-2: stage 0 ({} items a day, trip x{}), {items} items in custody: the redemptions of the day's 3 new units: {verdicts:?}",
        l.per_collection_items, l.trip_factor);
    assert!(matches!(verdicts.last().unwrap(), Verdict::Frozen(_)), "the 9th delivery ask of the collection in 24 h freezes custody");
    // the launch numbers do not (22 + 3 < 36)
    let launch = Limits { per_collection_items: 12, global_items: 24, per_collection_mints: 3, global_mints: 6, global_value_wei: 25 * E, trip_factor: 3 };
    assert!(22 + launch.per_collection_mints < launch.per_collection_items * launch.trip_factor);
    // and a trip factor of 2 (allowed by outflow::validate) trips on the bank run alone: 2 × per_day items is both what the exit rule allows
    // and what the tripwire trips at
    let two = Limits { trip_factor: 2, ..launch.clone() };
    validate(&two).unwrap();
    assert!(exit_fits(24, two.per_collection_items, 72 * 3_600));
    let mut o = Outflow::default();
    let mut last = Verdict::Allow;
    for k in 0..24u32 {
        last = o.check(&two, &format!("deliver:0xaa:{k}"), &key, Flow::Item, 1, t0 + k as u64 * 60);
        if last == Verdict::Allow {
            o.record(&format!("deliver:0xaa:{k}"), &key, Flow::Item, 1, t0 + k as u64 * 60);
        }
    }
    println!("R5-E E5-L-2: trip x2, a collection at its exit-rule maximum (24 at 12 a day) redeemed at once: the 24th ask {last:?}");
    assert!(matches!(last, Verdict::Frozen(_)));
}
