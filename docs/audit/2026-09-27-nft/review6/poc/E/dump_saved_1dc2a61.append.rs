
// Review round 6, part E: a saved state written by the 1dc2a61 build (fix round 4) with every round-4 field of the Ethereum side
// present (round4_state(), the build's own fixture), as `pre_upgrade` writes `(saved,)`. Appended to tests/audit_nft_r4.rs of a
// `git archive` export of 1dc2a61 and run there; the hex is restored by the HEAD build (r6_e_restore.append.rs).
#[test]
fn r6_e_dump_saved_1dc2a61() {
    use etch_autobuyer::canister::Saved;
    let mut eth = round4_state();
    eth.inbox.payouts = Some(vec![ef::PayoutClaim { buy: 1, home: true, tx: [9; 32], wei: 3, at: 5 }]);
    let saved: Saved = (None, vec![], true, vec!["installed".into()], Some(Default::default()), Some(Default::default()), Some(Default::default()),
        Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(eth));
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/saved-1dc2a61-rich.hex"), hex::encode(candid::encode_args((saved,)).unwrap())).unwrap();
}
