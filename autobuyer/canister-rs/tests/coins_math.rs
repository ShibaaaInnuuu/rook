//! W6: the coins' pure rules (the fee split, the redemption payout, the graduation order) and the coins' and pool desk's persisted states.
use etch_autobuyer::books::ONE;
use etch_autobuyer::coins::fees::{min_units, split, split_paid, Split, MIN_FEE_LAMPORTS};
use etch_autobuyer::coins::grad::{next_step, Next, View, MIN_SURPLUS_UNITS};
use etch_autobuyer::coins::redeem::payout;
use etch_autobuyer::coins::state::{self as cs, CoinRec, CoinStage, CoinsState};
use etch_autobuyer::pooldesk::state::{self as ps, DeskBook, PoolState};
use etch_autobuyer::solix2::decode::DbcPool;

#[test]
fn the_fee_split_gives_the_desk_a_quarter_half_as_wx_while_it_wants_depth() {
    assert_eq!(split(1_000, false), Split { to_desk: 0, desk_burn: 0, cash: 1_000 });
    let s = split(1_000, true);
    assert_eq!(s, Split { to_desk: 125, desk_burn: 125, cash: 750 });
    assert_eq!(s.to_desk + s.burned(), 1_000, "nothing lost");
    // odd amounts: the units add up, the burned half takes the odd unit
    let s = split(1_001, true);
    assert_eq!(s.to_desk + s.desk_burn + s.cash, 1_001);
    // the burn's SOL is split pro rata, rounded toward the treasury
    let (tre, desk) = split_paid(1_000_003, &Split { to_desk: 1, desk_burn: 1, cash: 2 });
    assert_eq!(tre + desk, 1_000_003);
    assert_eq!(desk, 333_334);
    assert_eq!(split_paid(77, &Split { to_desk: 0, desk_burn: 0, cash: 0 }), (77, 0));
}

#[test]
fn a_fee_cash_is_worth_a_transaction() {
    assert_eq!(min_units(0), ONE / 1_000, "no floor: the dust bound");
    // at 1 SOL a wX, 0.05 SOL of value
    assert_eq!(min_units(1_000_000_000), MIN_FEE_LAMPORTS * ONE / 1_000_000_000);
    // a huge floor: never under the dust bound
    assert_eq!(min_units(u64::MAX), ONE / 1_000);
}

#[test]
fn a_redemption_pays_its_share_of_the_vault_rounded_down() {
    assert_eq!(payout(1_000, 3, 1), Some(333));
    assert_eq!(payout(1_000, 3, 3), Some(1_000));
    assert_eq!(payout(1_000, 0, 0), None);
    assert_eq!(payout(1_000, 3, 4), None, "more than the supply");
    assert_eq!(payout(u64::MAX, u64::MAX, u64::MAX - 1), Some(u64::MAX - 1), "u128 inside");
    // n redemptions of one never pay more than the vault in all
    let (mut v, mut s, mut paid) = (1_000_000u64, 7_777u64, 0u64);
    while s > 0 {
        let p = payout(v, s, 1).unwrap();
        v -= p;
        s -= 1;
        paid += p;
    }
    assert!(paid <= 1_000_000);
}

fn pool() -> DbcPool {
    DbcPool { config: [1; 32], creator: [2; 32], base_mint: [3; 32], base_vault: [4; 32], quote_vault: [5; 32], base_reserve: 0, quote_reserve: 0,
        protocol_base_fee: 0, protocol_quote_fee: 0, partner_base_fee: 0, partner_quote_fee: 0, sqrt_price: 1, activation_point: 0, pool_type: 0,
        is_migrated: false, is_partner_withdraw_surplus: false, migration_progress: 0, is_withdraw_leftover: false, migration_fee_withdraw_status: 0,
        finish_curve_timestamp: 0 }
}

fn rec() -> CoinRec {
    CoinRec { coll: 0, coin: [3; 32], pool: [9; 32], creator: [2; 32], launched_at: 0, seq: 0, stage: CoinStage::Curve, vault: None,
        damm_pool: None, partner_nft: None, surplus_tries: 0 }
}

#[test]
fn graduation_goes_migrate_fee_leftover_burn_surplus_backed() {
    let r = rec();
    let t = 100 * ONE;
    let v = |p: &DbcPool, vault: Option<u64>, fc: Option<u64>| next_step(&View { rec: &r, pool: p, threshold: t, vault, fc_coin: fc });
    let mut p = pool();
    assert_eq!(v(&p, None, None), Next::NotYet);
    p.migration_progress = 2;
    assert_eq!(v(&p, None, None), Next::Migrate);
    p.is_migrated = true;
    p.migration_progress = 3;
    assert_eq!(v(&p, None, None), Next::MigrationFee);
    // wX someone sent to the vault's address is not the migration fee: the withdrawal is still made
    assert_eq!(v(&p, Some(1), None), Next::MigrationFee);
    // only the creator's fee withdrawn (bit 0): still the partner's to withdraw
    p.migration_fee_withdraw_status = 0b01;
    assert_eq!(v(&p, Some(5 * ONE), None), Next::MigrationFee);
    p.migration_fee_withdraw_status = 0b11;
    assert_eq!(v(&p, Some(50 * ONE), None), Next::Leftover);
    p.is_withdraw_leftover = true;
    assert_eq!(v(&p, Some(50 * ONE), Some(7)), Next::BurnCoins(7));
    assert_eq!(v(&p, Some(50 * ONE), Some(0)), Next::Backed, "no surplus");
    p.quote_reserve = t + MIN_SURPLUS_UNITS;
    assert_eq!(v(&p, Some(50 * ONE), None), Next::Surplus);
    p.is_partner_withdraw_surplus = true;
    assert_eq!(v(&p, Some(50 * ONE), None), Next::Backed);
}

#[test]
fn the_states_round_trip_and_a_bad_blob_is_refused() {
    let mut c = CoinsState::default();
    c.coins.insert([3; 32], rec());
    c.next_seq = 4;
    c.wait("open:1".into(), "why".into(), 10);
    assert_eq!(cs::decode_state(&cs::encode_state(&c)).unwrap(), c);
    assert!(cs::decode_state(b"not candid").is_err());
    let mut p = PoolState::default();
    p.desks.insert(2, DeskBook { pair: Some([7; 32]), lo: -19, hi: 19, pos_x: 5, pos_y: 6, fee_lamports: 8, ..Default::default() });
    assert_eq!(ps::decode_state(&ps::encode_state(&p)).unwrap(), p);
    assert!(ps::decode_state(b"\x00").is_err());
}

#[test]
fn the_desk_wants_fees_until_its_pool_reaches_target_and_books_what_it_got() {
    ps::reset();
    assert!(ps::wants_fees(0), "no pool yet: it wants fees to open one");
    ps::note_fee_income(0, 5, 6);
    ps::note_fee_income(0, 5, 6);
    assert_eq!(ps::with(|s| (s.desks[&0].fee_lamports, s.desks[&0].fee_units)), (10, 12));
    // an open pool at target on both sides (no floor booked here: the wX side is worth 0, so it is still under target)
    ps::with_mut(|s| {
        let d = s.desks.get_mut(&0).unwrap();
        d.pair = Some([1; 32]);
        d.pos_y = 20_000_000_000;
        d.pos_x = u64::MAX;
    });
    assert!(ps::wants_fees(0), "the wX side is valued at the accepted floor, none here");
    ps::with_mut(|s| s.off = true);
    assert!(!ps::wants_fees(0), "the desk switched off takes nothing");
    ps::reset();
}

/// The bytes the monitor decodes (monitor/test/books.test.mjs): `WRITE_FIXTURES=1` rewrites them.
fn view_fixture(file: &str, bytes: &[u8]) {
    let path = format!("{}/tests/fixtures/views/{file}", env!("CARGO_MANIFEST_DIR"));
    let hex = hex::encode(bytes);
    if std::env::var("WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, format!("{hex}\n")).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), hex, "{file} is stale: WRITE_FIXTURES=1, then the monitor's tests");
}

#[test]
fn the_coins_log_bytes_are_the_monitors_fixture() {
    cs::reset();
    cs::with_mut(|s| {
        s.fees.insert(0, etch_autobuyer::coins::state::FeeBook { curve_wx: 1, lp_wx: 2, cashed_wx: 3, cashed_lamports: 4, to_desk_wx: 5, to_desk_burned_wx: 6, to_desk_lamports: 7 });
        s.wait("open_config:1".into(), "the config's rent is not covered".into(), 1_700_000_000);
        s.landed("open_mint", 1_700_000_500, "collection 0's mint".into());
        // the opening queue (operator decision 2026-10-12): collection 2 asked for by a creator
        s.requested.push((2, 1_700_000_600));
    });
    let v = etch_autobuyer::coins::api::coins_log();
    view_fixture("coins-log.hex", &candid::encode_args(v).unwrap());
    cs::reset();
}
