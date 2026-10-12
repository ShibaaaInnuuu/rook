//! Review round 4, part A (A4-L-1): the launch install's custody caps and outflow limits (deployments/launch/nft-pilot.json
//! `_custody`) pass the install check (`canister.rs` `validate_custody`, which runs `outflow::exit_fits` per COLLECTION only),
//! but the whole custody cannot leave within the timelock's 72 h less a day: the global item limit (24 a day) lets out 48 items
//! in the two usable days, and the global cap holds 61. Passing = the weakness is present.
//!
//! Run in a private copy of autobuyer/canister-rs: drop this file into tests/ and `cargo test --test a4_exit_fits_global`.

use etch_autobuyer::{custodycap, outflow};

const E: u128 = 1_000_000_000_000_000_000;
const DELAY: u64 = 72 * 3600; // canister.rs ASSUMED_DELAY_SECS, the timelock's floor and stage 0's delay

fn launch() -> (custodycap::CapPolicy, outflow::Limits) {
    // nft-pilot.json: custodyItems / custodyWei per collection; the global caps the sum (61 items, 75 ETH)
    let cols = [("1:0x5af0d9827e0c53e4799bb226655a1de152a425a5", 16), ("1:0xed5af388653567af2f388e6224dc7c4b3241c544", 22),
        ("1:0x60e4d786628fea6478f785a6d7e704777c86a7c6", 17), ("1:0xbd3531da5cf5857e7cfaa92426877b022e612cf8", 4),
        ("1:0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d", 2)];
    let collections: Vec<custodycap::CollectionCap> = cols.iter().map(|(k, n)| custodycap::CollectionCap {
        collection: k.to_string(), max_items: *n, max_value_wei: 15 * E, max_item_wei: 15 * E }).collect();
    let items: u32 = collections.iter().map(|c| c.max_items).sum();
    let p = custodycap::CapPolicy { collections, global_max_items: items, global_max_value_wei: 75 * E };
    // the install's outflow_limits: 12 / 24 items, 3 / 6 mints, 25 ETH, trip 3
    let l = outflow::Limits { per_collection_items: 12, global_items: 24, per_collection_mints: 3, global_mints: 6,
        global_value_wei: 25 * E, trip_factor: 3 };
    (p, l)
}

#[test]
fn a4_l1_the_launch_install_passes_the_per_collection_exit_check_but_the_whole_custody_does_not_fit_the_delay() {
    let (p, l) = launch();
    custodycap::validate(&p).expect("the caps are well-formed");
    outflow::validate(&l).expect("the limits are well-formed");
    outflow::validate_change(&outflow::Limits::stage0(), &l).expect("within x4 of stage 0: an install may start here");
    // what validate_custody checks (canister.rs:285-290): each collection alone
    for c in &p.collections {
        assert!(outflow::exit_fits(c.max_items, l.per_collection_items, DELAY), "{} alone fits", c.collection);
    }
    // what it does not: every holder of every collection leaving at once (a bank run against a pending upgrade)
    assert_eq!(p.global_max_items, 61);
    let fits = outflow::exit_fits(p.global_max_items, l.global_items, DELAY);
    let usable_days = DELAY / outflow::WINDOW_SECS - 1;
    let out = usable_days as u32 * l.global_items;
    println!("global: {} items held at most, {} a day, {} usable days: {} can leave, {} cannot", p.global_max_items, l.global_items,
        usable_days, out, p.global_max_items - out);
    assert!(!fits, "WEAKNESS: the whole custody does not fit the delay, yet the install is accepted");
    // the most global_items an install may start at (x4 stage 0's 6) cannot fix it: 31 a day would be needed
    assert!(outflow::validate_change(&outflow::Limits::stage0(), &outflow::Limits { global_items: 31, ..l.clone() }).is_err());
    assert!(outflow::exit_fits(48, l.global_items, DELAY), "48 items in all would fit");
}
