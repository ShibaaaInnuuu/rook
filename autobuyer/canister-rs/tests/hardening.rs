//! The custody hardening modules (docs/v5/custody-hardening.md, H2–H4), now part of the crate and wired into the flows (their
//! own unit tests run with the library's). The tests below run them together, against the real delivery builder, and against
//! the wired flows (`redeem`, `frogflow`) through their fake worlds.

use etch_autobuyer::{custodycap, outbound, outflow};

use etch_autobuyer::custody::{self, Frog, Outpoint, Utxo};
use etch_autobuyer::policy::script_of;
use etch_autobuyer::psbt;

use custodycap::{CapPolicy, CollectionCap};
use outbound::{BtcPolicy, Prevout, Purpose, Refusal, TicketFacts};
use outflow::{Flow, Limits, Outflow, Verdict};

const E: u128 = 1_000_000_000_000_000_000;
const DAY: u64 = 86_400;
const T0: u64 = 1_790_000_000;
/// A real mainnet taproot address, used as custody (it is the address holding inscription 0 in tests/fixtures/delivery).
const CUSTODY: &str = "bc1pd96xzyue7yvjf24cmu07xasezg3jpm5tyfem4txad5ke2jas4m7qkhe7dy";
const REDEEMER: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";
const ATTACKER: &str = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
const FROGS: &str = "btc:0x1111111111111111111111111111111111111111";

fn frog() -> Frog {
    // inscription 0's real output (tests/fixtures/delivery/ord-inscription-0.json)
    Frog { output: Outpoint::parse("717d25914a1191c30f4c1d709e3ec0e1fab79cc7bf92cd12a93bed68db63416a:0").unwrap(), value: 606 }
}

fn fee() -> Utxo {
    Utxo { outpoint: Outpoint::parse("5d15f954692376399531f7c23f9b37db38802e9f2412d73b1fee42f5a1365f92:2").unwrap(), value: 20_000, confirmed: true }
}

fn prevout(o: &Outpoint, value: u64, script: &[u8]) -> Prevout {
    Prevout { txid: o.internal_txid(), vout: o.vout, value, script: script.to_vec() }
}

/// A delivery built and finalized by the canister's own code, with stand-in signatures (the verifier reads structure, not
/// signatures: the subnet signs a digest `sighash.rs` computed from these same bytes).
fn built(dest: &str) -> (Vec<u8>, Vec<Prevout>, Vec<u8>) {
    let ours = script_of(CUSTODY, "bc").unwrap();
    let to = script_of(dest, "bc").unwrap();
    let p = custody::build(&frog(), &fee(), &to, &ours, 10).unwrap();
    let sigs: Vec<(usize, Vec<u8>)> = (0..p.tx.inputs.len()).map(|i| (i, vec![0x11; 64])).collect();
    let raw = psbt::finalize(&p, &vec![None; p.tx.inputs.len()], &sigs).unwrap();
    (raw, vec![prevout(&frog().output, 606, &ours), prevout(&fee().outpoint, 20_000, &ours)], ours)
}

fn ticket_to(dest: &str) -> TicketFacts {
    let s = script_of(dest, "bc").unwrap();
    TicketFacts { ticket: 0, resolved: true, finalized: true, destination_script: s.clone(), event_destination_script: Some(s), names_this_item: true }
}

fn policy(ours: &[u8]) -> BtcPolicy {
    BtcPolicy { custody_script: ours.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true }
}

#[test]
fn the_verifier_accepts_what_the_real_builder_builds_for_the_tickets_destination() {
    let (raw, pre, ours) = built(REDEEMER);
    let c = outbound::verify_btc(&raw, &pre, &Purpose::Delivery { ticket: ticket_to(REDEEMER), item: pre[0].clone() }, &policy(&ours)).unwrap();
    assert_eq!(c.value, 606, "the frog's whole output");
    assert!(c.fee > 0 && c.fee <= custody::MAX_DELIVERY_FEE_SATS);
    // the parser agrees with the crate's own reading of the same bytes
    let tx = outbound::parse_tx(&raw).unwrap();
    assert_eq!((tx.inputs.len(), tx.outputs.len(), tx.inputs[0].sequence), (2, 2, 0xffff_fffd), "RBF-signalling: H8's conflict-out can replace it");
}

#[test]
fn a_delivery_built_for_another_address_is_refused_against_the_ticket() {
    // the builder was handed the wrong destination (a swapped read, a bug, a new code path): the bytes are internally
    // consistent and `custody::check` passes them, but they do not do what the ticket says
    let (raw, pre, ours) = built(ATTACKER);
    let r = outbound::verify_btc(&raw, &pre, &Purpose::Delivery { ticket: ticket_to(REDEEMER), item: pre[0].clone() }, &policy(&ours));
    assert_eq!(r, Err(Refusal::WrongDestination));
}

#[test]
fn a_leaked_delivery_is_conflicted_out_by_spending_its_fee_input_back_to_custody() {
    // H8: the delivery was broadcast, evicted, then refused. Spending its fee input to ourselves makes those bytes unminable.
    let (_, pre, ours) = built(REDEEMER);
    let leaked = pre[1].clone();
    let mut raw = vec![2, 0, 0, 0, 0, 1, 1];
    raw.extend(leaked.txid);
    raw.extend(leaked.vout.to_le_bytes());
    raw.extend([0x00, 0xfd, 0xff, 0xff, 0xff, 0x01]);
    raw.extend((20_000u64 - 2_000).to_le_bytes());
    raw.push(ours.len() as u8);
    raw.extend(&ours);
    raw.extend([0x01, 0x40]);
    raw.extend([0x22; 64]);
    raw.extend([0, 0, 0, 0]);
    assert!(outbound::verify_btc(&raw, &[leaked.clone()], &Purpose::ConflictOut { spend: leaked }, &policy(&ours)).is_ok());
}

#[test]
fn forged_tickets_meet_the_rate_limit_then_the_freeze_and_nothing_else_leaves() {
    // T10: colluding 4663 providers forge 40 resolved tickets naming the attacker. Each is internally consistent (the verifier
    // passes it: the forgery is in the read, not the bytes). What bounds the loss is the limit and the tripwire.
    let limits = Limits { per_collection_items: 6, global_items: 10, per_collection_mints: 6, global_mints: 10, global_value_wei: 20 * E, trip_factor: 3 };
    outflow::validate(&limits).unwrap();
    let mut out = Outflow::default();
    let (raw, pre, ours) = built(ATTACKER);
    let mut delivered = 0;
    let mut frozen_at = None;
    for t in 0..40u64 {
        let now = T0 + t * 600;
        let ok = outbound::verify_btc(&raw, &pre, &Purpose::Delivery { ticket: ticket_to(ATTACKER), item: pre[0].clone() }, &policy(&ours));
        assert!(ok.is_ok(), "a consistent forgery passes the bytes check");
        match out.check(&limits, &format!("ticket{t}"), FROGS, Flow::Item, 1, now) {
            Verdict::Allow => {
                out.record(&format!("ticket{t}"), FROGS, Flow::Item, 1, now);
                delivered += 1;
            }
            Verdict::Wait { .. } => {}
            Verdict::Frozen(_) => {
                frozen_at = Some(t);
                break;
            }
        }
    }
    assert_eq!(delivered, 6, "at stage 1: six frogs, not the vault");
    assert_eq!(frozen_at, Some(17), "the 18th distinct ask in a day trips the freeze");
    assert!(matches!(out.check(&limits, "honest", FROGS, Flow::Item, 1, T0 + 3 * DAY), Verdict::Frozen(_)), "until k guardians lift it");
}

#[test]
fn the_operators_nft_drain_is_refused_at_the_target_and_at_the_cap() {
    // T5: nft_set_target(1, <a junk collection>, 1, <the whole balance>), then nft_sign. With H2 wired, both refuse.
    let policy = CapPolicy {
        collections: vec![CollectionCap { collection: "1:0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d".into(), max_items: 3, max_value_wei: 18 * E, max_item_wei: 6 * E }],
        global_max_items: 24,
        global_max_value_wei: 20 * E,
    };
    custodycap::validate(&policy).unwrap();
    let junk = "1:0x000000000000000000000000000000000000beef";
    assert!(matches!(custodycap::check_target_cap(&policy, junk, 40 * E), Err(custodycap::Refusal::NotListed(_))));
    assert!(matches!(custodycap::check_target_cap(&policy, "1:0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d", 40 * E),
        Err(custodycap::Refusal::ItemOverMax { .. })));
    // and the value an honest target may spend in a day is bounded by the outflow limit too
    let limits = Limits { per_collection_items: 6, global_items: 10, per_collection_mints: 6, global_mints: 10, global_value_wei: 20 * E, trip_factor: 3 };
    let mut out = Outflow::default();
    assert_eq!(out.check(&limits, "fill1", "1:0xbc4c", Flow::Value, 6 * E, T0), Verdict::Allow);
}

#[test]
fn stage_one_limits_let_holders_leave_inside_the_timelocks_delay() {
    // docs/v5/launch-gates.md, stage 1: 12 items a collection, 6 out a day, a 72-hour delay
    assert!(outflow::exit_fits(12, 6, 3 * DAY));
    // stage 3 (192 items) at 24 a day does not fit a 72-hour delay: the gate requires a 7-day delay and 32 a day
    assert!(!outflow::exit_fits(192, 24, 3 * DAY));
    assert!(outflow::exit_fits(192, 32, 7 * DAY));
}

// ---- against the wired code -----------------------------------------------------------------------------------------------------
// (the flows' own wired tests, with their fake worlds, are in tests/redeem.rs and tests/frogs.rs: the forged-ticket drain that
// stops at six and freezes, the delivery to a destination the finalized ticket does not name, deposits over the cap returned)

#[test]
fn the_operator_cannot_record_an_nft_target_above_the_cap_or_for_a_collection_custody_does_not_list() {
    use etch_autobuyer::chains::NftState;
    use etch_autobuyer::nftnet::record_target;
    const BAYC: &str = "0xBC4CA0EdA7647A8aB7C2061c2E118A18a936f13D";
    // stage 0 for apes on chain 1: one item at most 10 ETH
    let caps = custodycap::stage0(&["1:0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d".to_string()]);
    let mut n = NftState::default();
    // T5: the whole balance as the cap, on a junk collection or on the listed one
    let junk = record_target(&mut n, &caps, 1, "0x000000000000000000000000000000000000beef", "1", 40 * E).unwrap_err();
    assert!(junk.contains("NotListed"), "{junk}");
    let over = record_target(&mut n, &caps, 1, BAYC, "9820", 40 * E).unwrap_err();
    assert!(over.contains("ItemOverMax"), "{over}");
    let other_chain = record_target(&mut n, &caps, 8453, BAYC, "9820", 5 * E).unwrap_err();
    assert!(other_chain.contains("NotListed"), "the same address on another chain is another collection: {other_chain}");
    assert!(n.targets.is_empty(), "nothing was recorded");
    // within the cap: recorded, and a new cap for the same token replaces it
    record_target(&mut n, &caps, 1, BAYC, "9820", 6 * E).unwrap();
    record_target(&mut n, &caps, 1, BAYC, "09820", 5 * E).unwrap();
    assert_eq!((n.targets.len(), n.targets[0].cap_wei.as_str()), (1, "5000000000000000000"));
    // with no caps at all (the empty policy): refused by default
    assert!(record_target(&mut n, &CapPolicy::default(), 1, BAYC, "1", 1).unwrap_err().contains("NotListed"));
}

#[test]
fn stage_zero_is_what_the_launch_gates_say_and_passes_every_install_check() {
    let caps = custodycap::stage0(&[FROGS.to_string()]);
    custodycap::validate(&caps).unwrap();
    custodycap::validate_change(&CapPolicy::default(), &caps).unwrap();
    let c = &caps.collections[0];
    assert_eq!((c.max_items, c.max_value_wei, c.max_item_wei, caps.global_max_items, caps.global_max_value_wei), (3, 10 * E, 10 * E, 6, 10 * E));
    let l = Limits::stage0();
    outflow::validate(&l).unwrap();
    assert_eq!((l.per_collection_items, l.global_items, l.per_collection_mints, l.global_mints, l.global_value_wei, l.trip_factor),
        (3, 6, 3, 6, 25 * E, 3), "stage 0's H3: 25 ETH of value a day since 2026-09-27");
    assert!(outflow::exit_fits(c.max_items, l.per_collection_items, 3 * DAY), "three items leave in a day of a 72-hour delay");
}

#[test]
fn a_pause_freezes_the_limiter_and_only_the_resume_lifts_it_a_tripwire_freeze_included() {
    // what set_paused does to the limiter (canister.rs), and what a trip does to the pause (outflow_check)
    let l = Limits::stage0();
    let mut out = Outflow::default();
    out.freeze("paused", T0);
    assert!(matches!(out.check(&l, "honest", FROGS, Flow::Item, 1, T0 + 1), Verdict::Frozen(_)), "paused: nothing leaves");
    out.unfreeze(true).unwrap();
    assert_eq!(out.check(&l, "honest", FROGS, Flow::Item, 1, T0 + 2), Verdict::Allow);
    for k in 0..9 {
        let _ = out.check(&l, &format!("forged{k}"), FROGS, Flow::Item, 1, T0 + 3);
    }
    assert!(matches!(out.frozen(), Some(outflow::Cause::Tripwire { .. })));
    out.freeze("paused", T0 + 4);
    assert!(matches!(out.frozen(), Some(outflow::Cause::Tripwire { .. })), "the guardian's pause keeps the trip as the cause");
    out.unfreeze(true).unwrap();
    assert_eq!(out.frozen(), None, "k guardians' resume lifts both");
}
