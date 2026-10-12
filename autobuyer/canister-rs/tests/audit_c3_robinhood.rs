//! Audit round C, part C3 (docs/v9/audit-2026-10/c3-robinhood-chain.md): the Robinhood Chain (4663) NFT path. Proofs of concept for
//! the High findings on the canister's side. Each test PASSED by exhibiting the bug; its comment says what a fix must refuse. FIXED
//! (2026-10-12): each is inverted into a `fixed_` regression that the fix passes. Pure: the rules the 4663 side runs
//! (`ethnft::floor_from_on`, `ethnft::nft_only`, `ethnft::buy_cap`, `ethnft::sale_blocks`, `EthState::relay_busy`), with the `rh_*`
//! collections of `tests/ethnft_sol.rs` (StonkBrokers, CASHCAT on 4663). The flow halves are in `tests/ethnft_sol.rs`
//! (`fixed_c3_03_…`) and the canister's own unit tests (`ethnet::tests::fixed_c3_04_…`, `fixed_c3_06_…`).

use etch_autobuyer::ethflow::{self as ef, BPhase, EthBuy, EthState};
use etch_autobuyer::ethnft::{self as en, EthCollection, EthNftConfig, FloorParams, Sale, Seen};
use etch_autobuyer::seaport as sp;

const E: u128 = 1_000_000_000_000_000_000;
const RH: u64 = 4663;
/// StonkBrokers on Robinhood Chain (an ERC-6551 collection: each NFT is a token-bound account holding tokenized stocks).
const STONK: [u8; 20] = [0x53, 0x9c, 0xdd, 0x04, 0x2c, 0x2f, 0x3d, 0x93, 0xeb, 0xc5, 0xbe, 0x7d, 0xff, 0xf0, 0xc3, 0xbc, 0x5c, 0x4f, 0xab, 0xf0];
/// CASHCAT on Robinhood Chain (the `rh_*` tests' collection).
const CASHCAT: [u8; 20] = [0xe3, 0xb3, 0x4c, 0x4b, 0xb0, 0xf1, 0x2c, 0x82, 0x14, 0x37, 0x45, 0xee, 0xe6, 0xa6, 0xcf, 0x4e, 0x31, 0x54, 0xb1, 0xfa];
/// Block times measured over ~1,000 blocks (docs/v4/eth-nft-canister.md §2, §3): Ethereum 12.10 s, Robinhood Chain 0.104 s. In
/// milliseconds.
const ETH_BLOCK_MS: u64 = 12_100;
const RH_BLOCK_MS: u64 = 104;
const NOW: u64 = 1_791_565_575;

fn stonk_col() -> EthCollection {
    // deployments/launch/robinhood-collections.json: maxPurchaseWei 1.84 ETH, tokenBound true, nftValueBps 2,000 (audit C3-01)
    EthCollection { collection: format!("0x{}", hex::encode(STONK)), wrapper: "books".into(), module: "books".into(),
        max_purchase_wei: 1_840_000_000_000_000_000, max_premium_bps: 500, chain_id: Some(RH), token_bound: Some(true), nft_value_bps: Some(2_000) }
}

fn chain(id: u64) -> etch_autobuyer::chains::EvmChain {
    etch_autobuyer::chains::EvmChain { chain_id: id, rpc_urls: vec!["https://a".into(), "https://b".into()], bucket: 8,
        seaport: "0x0000000000000068F116a894984e2DB1123eB395".into(), seaport_codehash: format!("0x{}", "ab".repeat(32)),
        max_fee_wei: 200_000_000_000, priority_fee_wei: 1_000_000_000, gas_cap: 400_000, fee_recipients: vec![], max_fee_bps: 1_500,
        min_order_life_s: 300, signed_zones: None, zone_registries: None }
}

fn seen(offerer: u8, token: u128, price: u128) -> Seen {
    Seen { offerer: [offerer; 20], token: sp::word_u128(token), price }
}

fn sale(token: u128, price: u128, seller: u8, buyer: u8, block: u64) -> Sale {
    Sale { token: sp::word_u128(token), price, seller: [seller; 20], buyer: [buyer; 20], block }
}

// ---- C3-01 (High): `token_bound` is a label; nothing reads it ---------------------------------------------------------------------

/// C3-01. `EthCollection.token_bound` is never consulted: the floor of a token-bound collection is its listings and sales (whose prices
/// carry what the accounts hold), the buy bound is that floor plus the premium, and no rule of the flow reads the account. A seller
/// who drains a StonkBrokers account and lists the empty NFT at the floor is bought at the full floor: the module pays for stocks
/// that are no longer there, every time (and a contents-blind buyer always ends up with the drained items: adverse selection).
///
/// A fix must refuse this: a token-bound collection is not bought (or not priced) unless the canister reads the account at the pin
/// (its ERC-6551 address from the registry, its holdings) and either prices what it holds or refuses an account that holds less
/// than what the floor's sales held (or the collection is not listed at all). This test then fails: `is_token_bound` gets a caller,
/// or the floor and the bound take the account's value as an input.
///
/// FIXED (2026-10-12): a token-bound collection is priced as its NFT ALONE: `EthCollection.nft_value_bps` (required with `token_bound`,
/// refused without it) is the share of a market price that is the bare NFT, and every listing and counted sale of the collection is
/// taken at that share before the floor is cut (`ethnft::nft_only`, `ethflow::reading_of`). The floor posted, the module's value and the
/// buy bound are then the NFT's: a drained account listed at the full accounts' floor is over the bound, and nothing is ever paid for
/// what an account holds (which no rule reads, and its seller can move before the fill).
#[test]
fn fixed_c3_01_a_drained_token_bound_nft_is_over_the_bound_the_floor_is_the_nft_alone() {
    let col = stonk_col();
    assert!(col.is_token_bound(), "StonkBrokers is configured token-bound");
    let cfg = EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None, logs_max_blocks: None,
        dormancy: None, collections: vec![col.clone()] };
    let chains = [chain(1), chain(RH)];
    assert_eq!(en::validate(&cfg, &chains, 0), Ok(()));
    // the share is required with token_bound, and only there; within (0, 10,000)
    for bad in [None, Some(0), Some(10_000)] {
        let c = EthNftConfig { collections: vec![EthCollection { nft_value_bps: bad, ..col.clone() }], ..cfg.clone() };
        assert!(en::validate(&c, &chains, 0).unwrap_err().contains("nft_value_bps"), "{bad:?}");
    }
    let plain = EthNftConfig { collections: vec![EthCollection { token_bound: None, ..col.clone() }], ..cfg.clone() };
    assert!(en::validate(&plain, &chains, 0).unwrap_err().contains("without token_bound"));
    let c = cfg.cols().unwrap()[0];
    assert_eq!((c.nft_bps, c.chain_id), (2_000, RH), "the parsed collection carries its share and its chain");

    // the floor: three sellers list FULL accounts at 1.47 / 1.50 / 1.55 ETH (the measured floor); the attacker's DRAINED #77 is listed
    // at 1.50, the second-cheapest seller's price. Four counted sales of full accounts at 1.45-1.50, a day apart, confirm it.
    let day = en::sale_blocks(RH).first_span;
    let mut listings = vec![seen(1, 10, 147 * E / 100), seen(2, 11, 150 * E / 100), seen(3, 12, 155 * E / 100), seen(9, 77, 150 * E / 100)];
    let mut sales = vec![sale(20, 148 * E / 100, 0x21, 0x31, 3 * day), sale(21, 145 * E / 100, 0x22, 0x32, 2 * day),
        sale(22, 150 * E / 100, 0x23, 0x33, day), sale(23, 147 * E / 100, 0x24, 0x34, 1)];
    let p = cfg.floor_params();
    // before the fix the floor was the full accounts' 1.50 ETH; now every price is the NFT's share first (what the flow does)
    en::nft_only(&mut listings, &mut sales, c.nft_bps);
    let post = en::floor_from_on(&listings, &sales, &p, 0, &en::sale_blocks(RH)).unwrap();
    assert_eq!((post.price, post.held.clone()), (30 * E / 100, None), "the NFT alone: 20 % of the full accounts' floor");
    // the buy bound the flow applies (`buy_cap`, then `nftnet::check_any` against it): the drained NFT's 1.50 ETH is far over it
    let cap = en::buy_cap(Some((post.price, NOW - 60)), post.price, NOW, col.max_premium_bps, 2 * E, col.max_purchase_wei).unwrap();
    assert_eq!(cap, 315 * E / 1_000);
    assert!(150 * E / 100 > cap, "the drained account at the full floor is refused");
    // a collection that is not token-bound is priced as before
    let mut l2 = vec![seen(1, 10, 147 * E / 100)];
    let mut s2 = vec![sale(20, 148 * E / 100, 0x21, 0x31, 1)];
    en::nft_only(&mut l2, &mut s2, 10_000);
    assert_eq!((l2[0].price, s2[0].price), (147 * E / 100, 148 * E / 100));
    // and the flow reads the share: `is_token_bound` has a caller, the evidence pass scales by the collection's `nft_bps`
    let ethnft = include_str!("../src/ethnft.rs");
    assert!(ethnft.matches("is_token_bound(").count() >= 2, "is_token_bound is read");
    assert!(include_str!("../src/ethflow.rs").contains("en::nft_only(&mut seen, &mut window, c.nft_bps)"), "the evidence pass scales");
}

// ---- C3-02 (High): the sale window and the scan are counted in Ethereum blocks on a 0.104 s chain -------------------------------------

/// C3-02. The floor's sale confirmation (audit 2026-09-27-nft B-H-1 / A-M-1 / B2-M-1) counts the sales of the last `SALE_WINDOW_BLOCKS`
/// = 50,400 blocks: 7 days of Ethereum's 12 s blocks, but under 90 MINUTES of Robinhood Chain's 0.104 s blocks, and the 4663 side
/// uses the same constant. Organically a 4663 collection (Chain Mancers: ≈ 1 sale a day) almost never has `min_sales` = 3 counted
/// sales in the window, so its first post never happens and a later one is held. An attacker with three wash sales (three tokens,
/// six wallets, no round trip: what `counted_sales` counts) and three sybil listings IS the whole sample: the floor is posted at the
/// attacker's price at full confidence, the module's first post takes it whole (no anchor), and the canister then buys from the
/// attacker at that floor plus the premium. The same wash sales against Ethereum's 7-day window sit among the organic sales and the
/// post is held at their 25th percentile plus 10 %.
///
/// The sale scan is cut in the same blocks: 13 chunks of 500 blocks a pass (the shared `logs_max_blocks`, set for Ethereum's Dwellir
/// plan) at the runner's 72 passes a day reads 468,000 blocks a day; 4663 makes 830,769. The forward walk falls behind and re-anchors
/// (`SALE_REANCHOR_LAG_BLOCKS` = 7,200 blocks: 12.5 minutes there) every pass.
///
/// A fix must refuse this: every block-count of the floor (the window, the final depth, the re-anchor lag, the chunk size and the
/// per-pass budget) is per chain (from the chain's block time or in seconds against the pinned block's timestamp), so the 4663 window
/// is 7 days of ITS blocks; this test then fails at the window assertion.
///
/// FIXED (2026-10-12): `ethnft::sale_blocks(chain)` cuts the window (7 days), the final depth (1,536 s), the re-anchor lag and the lag
/// pages (a day, 8 h) and the scan's chunks a pass (3 of the runner's 20-minute cadences of the chain's blocks) from the chain's block
/// time (`chains::block_ms`; an NFT chain without one is refused at install); Ethereum's are exactly the old constants. And a FIRST post
/// (the module's accepted floor 0) needs its counted sales spread over a day of the chain's blocks (`FIRST_POST_SPAN_SECS`), on every
/// chain: a burst of wash sales between fresh wallets is never a first price.
#[test]
fn fixed_c3_02_the_sale_window_is_7_days_of_each_chains_blocks_and_three_wash_sales_are_no_first_post() {
    let (eth, rh) = (en::sale_blocks(1), en::sale_blocks(RH));
    // Ethereum's block counts are exactly the constants it always had
    assert_eq!((eth.window, eth.final_depth, eth.reanchor_lag, eth.lag_sev2, eth.lag_sev1, eth.chunks_per_pass),
        (en::SALE_WINDOW_BLOCKS, en::SALE_FINAL_DEPTH_BLOCKS, en::SALE_REANCHOR_LAG_BLOCKS, en::SALE_LAG_SEV2_BLOCKS, en::SALE_LAG_SEV1_BLOCKS,
            en::SALE_SCAN_CHUNKS_PER_PASS));
    for (b, ms) in [(eth, ETH_BLOCK_MS), (rh, RH_BLOCK_MS)] {
        // within 1 % of the measured block time: the window is a week, never 87 minutes
        assert!(b.window * ms / 1_000 >= 7 * 86_400 * 99 / 100, "7 days: {} s", b.window * ms / 1_000);
        assert!(b.final_depth * ms / 1_000 >= 1_500, "'final' is 25 minutes of the chain's blocks");
        assert!(b.reanchor_lag * ms / 1_000 >= 86_400 * 99 / 100, "a day behind before a re-anchor");
        assert!(b.lag_sev2 * ms / 1_000 >= 8 * 3_600 * 99 / 100);
        let per_day_read = b.chunks_per_pass as u64 * en::SALE_SCAN_MAX_CHUNK_BLOCKS * 72;
        assert!(per_day_read >= 86_400_000 / ms, "the scan reads {per_day_read} blocks a day; the chain makes {}", 86_400_000 / ms);
    }
    assert_eq!(rh.window, 7 * 86_400 * 1_000 / 104);

    let p = FloorParams::default();
    // the attacker: three sybil sellers list CASHCAT at 0.40 ETH (the market is ≈ 0.26 ETH), three wash sales at 0.40 between six of
    // their own wallets within the last hour (one 4663 window before the fix): distinct tokens, distinct wallets, no round trip
    let listings = [seen(0xa1, 1, 40 * E / 100), seen(0xa2, 2, 40 * E / 100), seen(0xa3, 3, 41 * E / 100)];
    for b in [rh, eth] {
        let hour = b.first_span / 24;
        let wash = vec![sale(101, 40 * E / 100, 0xb1, 0xc1, 10 * hour), sale(102, 40 * E / 100, 0xb2, 0xc2, 10 * hour - hour / 2),
            sale(103, 41 * E / 100, 0xb3, 0xc3, 9 * hour)];
        // the first post (accepted 0) from a burst inside one hour: refused, on 4663 and on Ethereum alike
        let e = en::floor_from_on(&listings, &wash, &p, 0, &b).unwrap_err();
        assert!(e.contains("no first post") && e.contains("C3-02"), "{e}");
        // the same three sales spread over a day of the chain's blocks are a first post (still the 30 % band, one per wallet)
        let spread = vec![sale(101, 40 * E / 100, 0xb1, 0xc1, b.first_span + 10), sale(102, 40 * E / 100, 0xb2, 0xc2, b.first_span / 2),
            sale(103, 41 * E / 100, 0xb3, 0xc3, 10)];
        assert_eq!(en::floor_from_on(&listings, &spread, &p, 0, &b).unwrap().price, 40 * E / 100);
        // and a module that already has a floor is held as before (the span is the first post's rule only)
        let held = en::floor_from_on(&listings, &wash, &p, 26 * E / 100, &b).unwrap();
        assert_eq!((held.price, held.held.is_none()), (40 * E / 100, true), "{held:?}");
    }
    // on Ethereum the same wash sales sit among a week of organic sales at ≈ 0.26-0.30 ETH: held at their 25th percentile + 10 % at most
    let wash = vec![sale(101, 40 * E / 100, 0xb1, 0xc1, 1_000), sale(102, 40 * E / 100, 0xb2, 0xc2, 999), sale(103, 41 * E / 100, 0xb3, 0xc3, 998)];
    let organic: Vec<Sale> = (0..10u8).map(|i| sale(200 + i as u128, (29 + (i as u128 % 3)) * E / 100, 0x40 + i, 0x60 + i, 900 - i as u64)).collect();
    let held = en::floor_from_on(&listings, &[wash, organic].concat(), &p, 26 * E / 100, &eth).unwrap();
    assert!(held.held.is_some() && held.price < 34 * E / 100, "{held:?}");
    let _ = CASHCAT;
}

// ---- C3-03 (High, ops; the canister's half) ---------------------------------------------------------------------------------------

/// C3-03, the canister's half of `ops/test/audit_c3_robinhood.test.ts`: the runner's pre-judge refuses every Relay quote of a 4663 leg
/// (SOL → 4663 and 4663 → SOL), so it never hands one in. A 4663 buy that advanced is then `Funded` with its ask for ever: no timeout
/// moves a `Funded` buy without a quote, and `Funded` holds the one Relay leg canister-wide (`in_relay`): the Ethereum side's
/// buys (`ethnet::relay_busy_except`), its ways back, and the frog side's Bitcoin conversions (`ethnet::any_relay_busy`) all wait.
/// A fix (in ops) must hand a 4663 quote in; the ops PoC then fails. This half shows the hold: a fix may also time a quote-less
/// `Funded` out (back to its module) so one stuck leg cannot hold every chain's buys; this test then fails.
///
/// FIXED (2026-10-12): the runner judges and hands in both 4663 legs (ops `relaysol.ts` `solLegOf`: `sol-rh`, `rh-sol`), and a `Funded`
/// buy that no quote reached for `FUNDED_QUOTE_SECS` (2 h) goes back to its module (`Returning`, off the leg once returned; the
/// collection then cools down a day): one chain's dead leg no longer holds every chain's. The flow regression is
/// `tests/ethnft_sol.rs::fixed_c3_03_a_funded_buy_no_quote_reaches_goes_back_to_its_module_and_frees_the_relay_leg`.
#[test]
fn fixed_c3_03_a_4663_buy_funded_without_a_quote_is_bounded_by_funded_quote_secs() {
    let mut st: EthState = ef::new_side(RH);
    let id = ef::side_buy_base(RH);
    st.buys.push(EthBuy { id, collection: CASHCAT, module: [0; 20], wrapper: [0; 20], advanced: 20_000_000_000, attempts: 0,
        phase: BPhase::Funded { wei: 20_000_000_000, since: NOW }, orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(),
        sweep: None, arrived_at: None, back_at: None, ended_at: None, late: None });
    assert!(st.relay_busy(), "a Funded 4663 buy holds the one Relay leg while it waits for its quote");
    // the wait is bounded: as long as the way back's (BACK_QUOTE_SECS), and well inside a day
    assert_eq!(ef::FUNDED_QUOTE_SECS, ef::BACK_QUOTE_SECS);
    assert!(ef::FUNDED_QUOTE_SECS < ef::RETURN_COOLDOWN_SECS);
    // once it is back at its module, the leg is free
    st.buys[0].phase = BPhase::Returned { wei: 20_000_000_000 };
    assert!(!st.relay_busy());
}

/// Audit C3-02 / C3-14, the install's side: an NFT chain whose block time this build does not know is refused (its floor could not be cut
/// in its own blocks), and the 4663 side's buy ids stay where they were (its low 24 bits are neither zero nor another side's).
#[test]
fn fixed_c3_02_c3_14_an_nft_chain_without_a_known_block_time_is_refused_and_side_ids_are_unchanged() {
    let col = EthCollection { chain_id: None, token_bound: None, nft_value_bps: None, ..stonk_col() };
    let cfg = EthNftConfig { chain_id: 10, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None, logs_max_blocks: None,
        dormancy: None, collections: vec![col.clone()] };
    assert!(en::validate(&cfg, &[chain(10)], 0).unwrap_err().contains("block time"));
    let ok = EthNftConfig { chain_id: 1, ..cfg };
    assert_eq!(en::validate(&ok, &[chain(1)], 0), Ok(()));
    assert_eq!(ef::side_buy_base(RH), 4663 << 40);
}
