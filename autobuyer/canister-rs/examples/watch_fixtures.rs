//! Test fixtures for the monitor (`monitor/`, docs/v6/k6-watcher-canary.md): upgrade arguments and query replies encoded by the
//! canister's OWN types, so the monitor's JS decoder is tested against the real encoder and not against itself.
//!
//!   cargo run --example watch_fixtures > ../../monitor/fixtures/canister.json
//!
//! Prints JSON: { "args": { name: hex }, "replies": { name: hex } }. Every hex is Candid bytes (`DIDL…`). Nothing here runs on a
//! network; the principals and addresses are made up, the Bitcoin txids are two real mainnet transactions the canary tests use.
use candid::{Encode, Principal};
use etch_autobuyer::canister::Init;
use etch_autobuyer::custody::Outpoint;
use etch_autobuyer::custodycap::{CapPolicy, CollectionCap};
use etch_autobuyer::deposits::{DPhase, DepositRec, Reason};
use etch_autobuyer::frogbuy::{Bought, FPhase, FrogRec, Target};
use etch_autobuyer::outflow::Limits;
use etch_autobuyer::conflict::{Conflict, ConflictTx};
use etch_autobuyer::outflow::{Act, Cause, Flow, Freeze, Outflow};
use etch_autobuyer::custody::{FeeEntry, FeePool};
use etch_autobuyer::redeem::{Attest, Bump, RPhase, Redemption};
use etch_autobuyer::ticketlog::{Dense, TicketLogs};

const ETH: u128 = 1_000_000_000_000_000_000;
const WRAPPER: &str = "0x00000000000000000000000000000000000000aa";
const MODULE: [u8; 20] = [0xbb; 20];
/// Real mainnet: bc1pfssf… received 2,910 sats in b6ceea8e… and spent them in 2127bc90… to bc1pfqwu… (the canary's fixture).
const IN_TXID: &str = "b6ceea8eb7e46c5784c6ff3eb3af6e5eca91f53ed8a06cdda7fe6f6c1b5f738e";
const OUT_TXID: &str = "2127bc9003b23d8dd342a60cfe9cb44383288c0581a49aac35e513dc8e0fce57";
const OUT_DEST: &str = "bc1pfqwuxf27gj0c8ayer55tg720fpz80d2jprrultlrlzeat2qrzasqkdyn8p";

fn txid(s: &str) -> [u8; 32] {
    hex::decode(s).unwrap().try_into().unwrap()
}

fn operator() -> Principal {
    Principal::from_slice(&[7; 29])
}

/// The Phase-1 install argument (docs/v6/phase1-scope.md §3.2), with mainnet named as K2 will allow it.
fn install() -> Init {
    Init {
        chain_id: 4663,
        vault: "0x00000000000000000000000000000000000000cc".into(),
        rpc_urls: vec!["https://rpc.mainnet.chain.robinhood.com".into(), "https://rpc.ordofi.network".into()],
        btc_api_urls: vec!["https://mempool.space/api".into(), "https://blockstream.info/api".into()],
        key_name: "key_1".into(),
        accept_production_key_name: true,
        bridge_allowlist: vec![],
        btc_network: "mainnet".into(),
        operator: Some(operator()),
        wrapper: Some(WRAPPER.into()),
        evm_chains: None,
        custody_caps: None,
        outflow_limits: None,
        holder_deposits: None,
        wrapper_from_block: None, frog_funding: None, eth_nft: None, home: None,
    }
}

fn caps(items: u32, value: u128, g_items: u32, g_value: u128) -> CapPolicy {
    CapPolicy {
        collections: vec![CollectionCap { collection: format!("btc:{WRAPPER}"), max_items: items, max_value_wei: value, max_item_wei: value }],
        global_max_items: g_items,
        global_max_value_wei: g_value,
    }
}

fn limits(items: u32, g_items: u32, value: u128, trip: u32) -> Limits {
    Limits { per_collection_items: items, global_items: g_items, per_collection_mints: items, global_mints: g_items, global_value_wei: value, trip_factor: trip }
}

fn hexof(b: Vec<u8>) -> String {
    hex::encode(b)
}

fn main() {
    let mut args: Vec<(&str, String)> = vec![];
    args.push(("install", hexof(Encode!(&install()).unwrap())));
    // a code-only upgrade: the same argument
    args.push(("neutral_same", hexof(Encode!(&install()).unwrap())));
    // null written out as the stage-0 values it means: no effective change
    args.push(("neutral_explicit_stage0", hexof(Encode!(&Init {
        custody_caps: Some(caps(3, 10 * ETH, 6, 10 * ETH)),
        outflow_limits: Some(Limits::stage0()),        // 25 ETH of value a day since 2026-09-27
        ..install()
    }).unwrap())));
    // tighter: 2 items per collection, 5 ETH, 2 out a day
    args.push(("tightened", hexof(Encode!(&Init {
        custody_caps: Some(caps(2, 5 * ETH, 4, 5 * ETH)),
        outflow_limits: Some(limits(2, 4, 5 * ETH, 3)),
        ..install()
    }).unwrap())));
    // stage 0 → stage 1 (×4) with a looser tripwire
    args.push(("loosened", hexof(Encode!(&Init {
        custody_caps: Some(caps(12, 40 * ETH, 24, 40 * ETH)),
        outflow_limits: Some(limits(12, 24, 40 * ETH, 4)),
        ..install()
    }).unwrap())));
    // beyond ×4 at once: the running code refuses it in post_upgrade
    args.push(("loosened_x10", hexof(Encode!(&Init { custody_caps: Some(caps(30, 100 * ETH, 60, 100 * ETH)), ..install() }).unwrap())));
    args.push(("operator_swap", hexof(Encode!(&Init { operator: Some(Principal::from_slice(&[9; 29])), ..install() }).unwrap())));
    args.push(("providers_swap", hexof(Encode!(&Init {
        btc_api_urls: vec!["https://mempool.space/api".into(), "https://btc.example.net/api".into()],
        ..install()
    }).unwrap())));
    args.push(("identity_change", hexof(Encode!(&Init { key_name: "test_key_1".into(), btc_network: "testnet4".into(), ..install() }).unwrap())));
    // audit D (fixes-D-canister.md §6): holder deposits switched on, and the ticket-log scanner's first block named
    args.push(("deposits_on", hexof(Encode!(&Init { holder_deposits: Some(true), wrapper_from_block: Some(71_200_000), ..install() }).unwrap())));

    // the query replies the canary matches against
    let delivered = Redemption {
        ticket: 5,
        acquisition_id: 1,
        destination: OUT_DEST.into(),
        phase: RPhase::Done { txid: txid(OUT_TXID) },
        inscription: Some(format!("{IN_TXID}i0")),
        attest: None,
        noted: None,
    };
    let owed = Redemption { ticket: 6, acquisition_id: 2, destination: OUT_DEST.into(), phase: RPhase::Owed, inscription: None, attest: None, noted: None };
    let wrong_dest = Redemption { destination: "bc1pfssfalzdt9h8uwqcrs73780l723mh2axrul7keaevke7a8pg4a7szrd0ht".into(), ..delivered.clone() };
    let bought = Bought {
        inscription: format!("{IN_TXID}i0"),
        outpoint: Outpoint { txid: txid(IN_TXID), vout: 0 },
        seller: Outpoint { txid: [0x33; 32], vout: 1 },
        spend_sats: 2_910,
        fee_sats: 2_310,
        rate: 3_000_000,
        cost_wei: 650_000_000_000_000_000,
        raw: vec![0x02, 0x00, 0x00, 0x00],
        setup_txid: None,
    };
    let frog = FrogRec { id: 1, target: Some(Target::Sink(MODULE)), phase: FPhase::Settled { buy: bought.clone() } };
    let wanted = FrogRec { id: 2, target: Some(Target::Sink(MODULE)), phase: FPhase::Wanted { cap_wei: ETH } };
    let returned = DepositRec {
        outpoint: Outpoint { txid: [0x44; 32], vout: 0 },
        first_seen: 1,
        phase: DPhase::Returned { reason: Reason::NotInCollection("x".into()), txid: [0x55; 32] },
        admitted: None,
        next_check: None,
        tries: None,
    };
    // audit D, M-1: a delivery re-priced: its current bytes are new, and the transaction the canary will see (OUT_TXID) is the
    // EARLIER version, which confirmed; and one conflicted out, whose conflict (OUT_TXID) spends the fee input back to custody
    let conflict = |tx: [u8; 32]| Conflict {
        leaked_txid: [0x66; 32],
        raw: vec![0x02],
        fee_input: Outpoint { txid: [0x77; 32], vout: 1 },
        item: Outpoint { txid: txid(IN_TXID), vout: 0 },
        tx: Some(ConflictTx { txid: tx, raw: vec![0x02], inputs: vec![Outpoint { txid: [0x77; 32], vout: 1 }], fee: 3_000,
            signed_at: None, earlier: None }),
        earlier: Some(vec![[0x67; 32]]),
        leaked_fee: None,
        held_since: None,
    };
    let bumped = Redemption {
        ticket: 8,
        acquisition_id: 1,
        destination: OUT_DEST.into(),
        phase: RPhase::Sent {
            txid: [0x88; 32],
            raw: vec![0x02],
            fee_input: Outpoint { txid: [0x77; 32], vout: 1 },
            bump: Some(Bump { first_at: 1, last_at: 2, fee: 4_000, earlier: vec![txid(OUT_TXID)] }),
        },
        inscription: Some(format!("{IN_TXID}i0")),
        attest: None,
        noted: None,
    };
    let conflicting = Redemption { ticket: 9, phase: RPhase::Conflicting { reason: "inside the 24 h margin".into(), conflict: conflict(txid(OUT_TXID)) }, ..bumped.clone() };
    // audit D, L-5: a stale frog buy abandoned: the float's conflict (OUT_TXID here) spends one of its inputs back to the float
    let abandoning = FrogRec { id: 3, target: Some(Target::Sink(MODULE)), phase: FPhase::Abandoning { buy: bought.clone(), conflict: conflict(txid(OUT_TXID)) } };
    // docs/v8/ow-protected.md §3.3: a protected Ordinals Wallet sale signed and handed back, watched by its txid
    let handed = FrogRec { id: 4, target: Some(Target::Sink(MODULE)), phase: FPhase::Handed { buy: Bought { raw: vec![], ..bought.clone() },
        sale_txid: hex::encode(bought.outpoint.txid), at: 1_790_000_000 } };
    let parked = DepositRec { outpoint: Outpoint { txid: [0x45; 32], vout: 0 }, first_seen: 1, phase: DPhase::Parked { inscription: format!("{IN_TXID}i0") },
        admitted: None, next_check: None, tries: Some(3) };
    let act = |key: &str, flow: Flow| Act { key: key.into(), collection: format!("btc:{WRAPPER}"), flow, amount: 1, at: 1 };
    let frozen = Outflow {
        done: vec![],
        asked: vec![act("return:1", Flow::Return), act("deliver:9", Flow::Item)],
        freeze: Freeze::Frozen { since: 1_790_000_000_000_000_000, cause: Cause::Tripwire { collection: format!("btc:{WRAPPER}"), flow: Flow::Item, demanded: 9, limit: 3 } },
        acked: Some(vec![act("deliver:7", Flow::Item), act("return:1", Flow::Return)]),
    };
    // canister review2 (fixes-R3-canister.md §9): a conflict-out re-priced (the canary must match every version: the transaction it
    // sees, OUT_TXID, is the EARLIER one); refused redemptions and their markUndeliverable; the scanner reading a dense block by
    // ticket with one unreadable ticket; a fee pool with a quarantined entry
    let repriced = Redemption {
        ticket: 10,
        phase: RPhase::Conflicting {
            reason: "inside the 24 h margin".into(),
            conflict: Conflict {
                tx: Some(ConflictTx { txid: [0x99; 32], raw: vec![0x02], inputs: vec![Outpoint { txid: [0x77; 32], vout: 1 }], fee: 9_000,
                    signed_at: Some(1_790_007_200), earlier: Some(vec![txid(OUT_TXID)]) }),
                ..conflict([0x98; 32])
            },
        },
        attest: Some(Attest::Due { deep: Some([0x99; 32]) }),
        noted: None,
        ..bumped.clone()
    };
    let refused = |ticket: u64, attest: Attest| Redemption { ticket, acquisition_id: 3, destination: OUT_DEST.into(),
        phase: RPhase::Refused { reason: "inside the margin: not sent".into() }, inscription: None, attest: Some(attest), noted: None };
    let attested = vec![
        refused(11, Attest::Sent { raw: vec![0x02, 0xf8], hash: [0xab; 32], nonce: 4 }),
        refused(12, Attest::Done { note: "markUndeliverable mined (success)".into() }),
        refused(13, Attest::Due { deep: None }),
    ];
    let dense = TicketLogs { cursor: Some(71_300_000), span: 1, failures: 0, cap: Some(400_000), hot: Some(71_300_000),
        dense: Some(Dense { block: 71_300_001, after: Some(640), shrink: 0, fails: 0 }), dense_blocks: Some(1),
        unreadable: Some(vec![(71_300_001, 777)]), ..TicketLogs::default() };
    let pool = FeePool {
        entries: vec![FeeEntry { outpoint: Outpoint { txid: [0x21; 32], vout: 0 }, value: 150_000, source: "operator".into(), fails: None, last_failed_at: None }],
        quarantined: Some(vec![FeeEntry { outpoint: Outpoint { txid: [0x22; 32], vout: 1 }, value: 40_000, source: "change".into(), fails: Some(3), last_failed_at: Some(1_790_007_200) }]),
    };
    let failing = TicketLogs { cursor: Some(71_200_000), span: 4_000, failures: 7, last_error: Some("rpc.ordofi.network: -32005 the network is busy".into()), ..TicketLogs::default() };
    let replies: Vec<(&str, String)> = vec![
        ("redemptions_delivered", hexof(Encode!(&vec![owed.clone(), delivered]).unwrap())),
        ("redemptions_wrong_destination", hexof(Encode!(&vec![wrong_dest]).unwrap())),
        ("redemptions_none_delivered", hexof(Encode!(&vec![owed]).unwrap())),
        ("frog_buys", hexof(Encode!(&vec![frog, wanted]).unwrap())),
        ("holder_deposits", hexof(Encode!(&vec![returned]).unwrap())),
        ("custody_caps_stage0", hexof(Encode!(&caps(3, 10 * ETH, 6, 10 * ETH), &Vec::<etch_autobuyer::custodycap::Headroom>::new()).unwrap())),
        ("custody_outflow_stage0", hexof(Encode!(&Limits::stage0(), &etch_autobuyer::outflow::Outflow::default()).unwrap())),
        ("redemptions_bumped", hexof(Encode!(&vec![bumped]).unwrap())),
        ("redemptions_conflicting", hexof(Encode!(&vec![conflicting]).unwrap())),
        ("frog_buys_abandoning", hexof(Encode!(&vec![abandoning]).unwrap())),
        ("frog_buys_handed", hexof(Encode!(&vec![handed]).unwrap())),
        ("holder_deposits_parked", hexof(Encode!(&vec![parked]).unwrap())),
        ("custody_outflow_frozen_acked", hexof(Encode!(&Limits::stage0(), &frozen).unwrap())),
        ("ticket_logs_failing", hexof(Encode!(&Some(failing)).unwrap())),
        ("redemptions_conflict_repriced", hexof(Encode!(&vec![repriced]).unwrap())),
        ("redemptions_attested", hexof(Encode!(&attested).unwrap())),
        ("ticket_logs_dense", hexof(Encode!(&Some(dense)).unwrap())),
        ("custody_fee_pool", hexof(Encode!(&pool).unwrap())),
    ];

    let obj = |v: &Vec<(&str, String)>| v.iter().map(|(k, h)| format!("    \"{k}\": \"{h}\"")).collect::<Vec<_>>().join(",\n");
    println!("{{\n  \"note\": \"encoded by autobuyer/canister-rs/examples/watch_fixtures.rs with the canister's own types; regenerate with cargo run --example watch_fixtures\",\n  \"args\": {{\n{}\n  }},\n  \"replies\": {{\n{}\n  }}\n}}", obj(&args), obj(&replies));
}
