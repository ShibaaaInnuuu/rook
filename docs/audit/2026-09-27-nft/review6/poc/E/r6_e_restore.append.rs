
// Review round 6, part E: the saved state the 1dc2a61 build wrote (poc/E/saved-1dc2a61-rich.hex, from dump_saved_1dc2a61.append.rs)
// restores in the HEAD build with nothing lost and every round-5 field absent; and a probe of the largest round-5 state.
#[test]
fn r6_e_saved_1dc2a61_restores_with_nothing_lost() {
    use etch_autobuyer::canister::{lost_elements, lost_opts, Saved, SavedProbe};
    let mut bytes = hex::decode(include_str!("fixtures/state/saved-1dc2a61-rich.hex").trim()).unwrap();
    bytes.resize(bytes.len().div_ceil(65_536) * 65_536, 0);
    let mut de = candid::de::IDLDeserialize::new(&bytes).unwrap();
    let saved: Saved = de.get_value().unwrap();
    let mut de = candid::de::IDLDeserialize::new(&bytes).unwrap();
    let probe: SavedProbe = de.get_value().unwrap();
    let d = [saved.4.is_some(), saved.5.is_some(), saved.6.is_some(), saved.7.is_some(), saved.8.is_some(), saved.9.is_some(),
        saved.10.is_some(), saved.11.is_some(), saved.12.is_some()];
    assert_eq!(lost_elements(&probe, d), Vec::<usize>::new());
    let again = candid::encode_args((saved.clone(),)).unwrap();
    assert_eq!(lost_opts(&bytes, &again).unwrap(), Vec::<String>::new());
    let e = saved.12.unwrap();
    assert!(e.pending.as_ref().is_some_and(|p| p.pin.is_none() && p.used_since == Some(6)));
    assert!(e.buys.iter().all(|b| b.late.as_ref().is_none_or(|l| l.released_at.is_none() && l.credited.is_some())));
    assert!(e.inbox.payouts.as_ref().is_some_and(|p| p.len() == 1) && e.inbox.home_clear.is_none());
    assert!(e.sale_scan.iter().flatten().all(|s| s.done_big.is_none() && s.stuck.is_none() && s.spent.is_none() && s.bad.is_some()));
    assert!(e.pin_floors.is_some() && e.home_floor.is_some() && e.credited.is_some());
    println!("restored: {} bytes, {} buys, pending {:?}", bytes.len(), e.buys.len(), e.pending.as_ref().map(|p| p.nonce));
}

#[test]
fn r6_e_probe_largest_round5_state_decodes_fast() {
    // five collections' scans at their round-5 bounds (32 deferred ranges, 64 remembered blocks, SALE_TODO_MAX), 400 buys with late orders,
    // 64 payout claims, every new opt present
    let w = ef::Watch { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, balance_after: 1, eth_before: 2, eth_out_before: 3, pin: Some(4),
        solver: Some([0xf7; 20]), request_id: Some([0x17; 32]), home_pin: Some(70_166_016) };
    let mut st = EthState::default();
    st.buys = (0..400u64).map(|id| ef::EthBuy { id, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: 1, attempts: 1,
        phase: BPhase::Stuck { why: "x".repeat(200) }, orders: vec![[1; 32]; 4], deposits: vec![], ask: None, last: Some(w.clone()), note: "n".repeat(200),
        sweep: Some(false), arrived_at: Some(3), back_at: Some(4), ended_at: None,
        late: Some(ef::LateOrder { watch: w.clone(), back: true, let_go_at: 5, credited: None, released_at: Some(6) }) }).collect();
    st.sale_scan = Some((0..5u8).map(|c| ef::SaleScan { collection: [c; 20], down: 1, up: 9, chunk: 500,
        todo: (0..512u64).map(|i| ([i as u8; 32], i)).collect(), pend: Some(vec![]), rescan: Some((1, 2)), judged: Some((0..512).map(|i| (i, i)).collect()),
        bad: Some((0..32).map(|i| (i, i + 1)).collect()), skipped: Some((0..32).map(|i| (i, i, 1)).collect()), done_big: Some((0..64).collect()),
        stuck: Some((2, 3)), spent: Some((1, 2)) }).collect());
    st.inbox.payouts = Some((0..64).map(|i| ef::PayoutClaim { buy: i, home: true, tx: [9; 32], wei: 0, at: 5 }).collect());
    st.inbox.home_clear = Some((7, true, 8));
    let bytes = candid::encode_one(&st).unwrap();
    let t = std::time::Instant::now();
    let back: EthState = candid::decode_one(&bytes).unwrap();
    let el = t.elapsed();
    assert_eq!(back, st);
    println!("largest round-5 probe: {} bytes, decoded in {:?} (debug build)", bytes.len(), el);
}
