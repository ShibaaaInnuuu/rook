//! The treasury (test/floor/Treasury.t.sol) and the wake desk (test/wake/WakeDesk.t.sol).

use super::common::*;
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::system_program;
use rook_core::error::RookError;
use rook_core::seeds::*;
use rook_core::state::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// The raffle PDA `["raffle", listing]` of a module (passed to `distribute` whether or not it exists).
pub fn raffle_of(env: &Env, module: &Pubkey) -> Pubkey {
    let listing = env.get::<Module>(module).listing;
    pda(&[RAFFLE, listing.as_ref()])
}

pub fn treasury(env: &Env) -> Treasury {
    env.get(&env.treasury)
}

pub fn admin<D: anchor_lang::InstructionData>(env: &mut Env, data: D, signer: &Keypair) -> Res {
    let i = ix(
        data,
        rook_core::accounts::TreasuryAdmin { owner: signer.pubkey(), registry: env.registry, treasury: env.treasury },
    );
    let s = signer.insecure_clone();
    env.send(&[i], &[&s])
}

pub fn receive(env: &mut Env, lamports: u64) -> Res {
    let u = env.user.insecure_clone();
    let i = ix(
        rook_core::instruction::Receive { lamports },
        rook_core::accounts::Receive { payer: u.pubkey(), treasury: env.treasury, system_program: system_program::ID },
    );
    env.send(&[i], &[&u])
}

pub fn allocate(env: &mut Env, ops: Pubkey) -> Res {
    let u = env.user.insecure_clone();
    let ops = if ops == Pubkey::default() { None } else { Some(ops) };
    let i = ix(rook_core::instruction::Allocate {}, rook_core::accounts::Allocate { treasury: env.treasury, ops });
    env.send(&[i], &[&u])
}

pub fn add_collection(env: &mut Env, module: Pubkey) -> Res {
    let o = env.owner.insecure_clone();
    let i = ix(
        rook_core::instruction::AddCollection {},
        rook_core::accounts::AddCollection { owner: o.pubkey(), registry: env.registry, treasury: env.treasury, module },
    );
    env.send(&[i], &[&o])
}

// Treasury.t.sol: test_theSharesSumToOneHundredPercent_andOnlyTheTimelockChangesThem,
// test_allocateSplitsIncomeByTheShares_andPaysOperations, test_theBooksBalanceAtEveryStep
#[test]
fn shares_allocate_and_ops() {
    let mut env = Env::new();
    let o = env.owner.insecure_clone();
    let u = env.user.insecure_clone();
    expect_err(admin(&mut env, rook_core::instruction::SetShares { reserves_bps: 8_000, ops_bps: 2_000 }, &u), RookError::NotOwner);
    expect_err(admin(&mut env, rook_core::instruction::SetShares { reserves_bps: 8_000, ops_bps: 1_000 }, &o), RookError::BadShares);
    let t = treasury(&env);
    assert_eq!((t.reserves_bps, t.ops_bps), (9_000, 1_000));
    expect_err(allocate(&mut env, Pubkey::default()), RookError::NothingToDo);
    ok(receive(&mut env, 10 * SOL));
    // no ops address: its share joins the reserves
    ok(allocate(&mut env, Pubkey::default()));
    assert_eq!(treasury(&env).reserves_pot, 10 * SOL);
    let ops = Keypair::new().pubkey();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops }, &o));
    ok(receive(&mut env, 10 * SOL));
    expect_err(allocate(&mut env, Keypair::new().pubkey()), RookError::BadInput);
    ok(allocate(&mut env, ops));
    assert_eq!(env.lamports(&ops), SOL);
    assert_eq!(treasury(&env).reserves_pot, 19 * SOL);
    let rent = env.svm.minimum_balance_for_rent_exemption(env.svm.get_account(&env.treasury).unwrap().data.len());
    assert!(env.lamports(&env.treasury) - rent >= treasury(&env).reserves_pot, "the pots never exceed the balance");
}

pub fn apply(env: &mut Env, data: impl anchor_lang::InstructionData) -> Res {
    let u = env.user.insecure_clone();
    let i = ix(data, rook_core::accounts::TreasuryApply { treasury: env.treasury });
    env.send(&[i], &[&u])
}

// The operator's decision (2026-10-10): the split starts at 90/10 and every later change of the shares, or of a named
// operations address, waits 24 hours in public before anyone may apply it.
#[test]
fn treasury_changes_wait_a_day() {
    let mut env = Env::new();
    let o = env.owner.insecure_clone();
    let t0 = env.now;
    expect_err(apply(&mut env, rook_core::instruction::ApplyShares {}), RookError::NothingPending);
    ok(admin(&mut env, rook_core::instruction::SetShares { reserves_bps: 8_000, ops_bps: 2_000 }, &o));
    assert_eq!((treasury(&env).reserves_bps, treasury(&env).ops_bps), (9_000, 1_000), "a proposal changes nothing yet");
    env.set_time(t0 + rook_core::state::TREASURY_DELAY - 1);
    expect_err(apply(&mut env, rook_core::instruction::ApplyShares {}), RookError::TooEarly);
    env.set_time(t0 + rook_core::state::TREASURY_DELAY);
    ok(apply(&mut env, rook_core::instruction::ApplyShares {}));
    let t = treasury(&env);
    assert_eq!((t.reserves_bps, t.ops_bps, t.pending_shares_at), (8_000, 2_000, 0));
    expect_err(apply(&mut env, rook_core::instruction::ApplyShares {}), RookError::NothingPending);

    // the first operations address applies at once; a change waits
    let first = Keypair::new().pubkey();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops: first }, &o));
    assert_eq!(treasury(&env).ops, first);
    let second = Keypair::new().pubkey();
    let t1 = env.now;
    ok(admin(&mut env, rook_core::instruction::SetOps { ops: second }, &o));
    assert_eq!(treasury(&env).ops, first, "a change waits");
    expect_err(apply(&mut env, rook_core::instruction::ApplyOps {}), RookError::TooEarly);
    env.set_time(t1 + rook_core::state::TREASURY_DELAY);
    ok(apply(&mut env, rook_core::instruction::ApplyOps {}));
    assert_eq!((treasury(&env).ops, treasury(&env).pending_ops_at), (second, 0));
}

// Treasury.t.sol: test_distributeIsWeightedBySevenDayVolume, test_nothingLeavesButToRegisteredModulesAndOperations
#[test]
fn distribute_is_weighted_by_seven_day_volume_one_collection_per_call() {
    let mut env = Env::new();
    let m0 = env.module;
    let v0 = env.vault;
    // a second collection
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::EthNft { chain_id: 1, collection: [2u8; 20] }, "Azuki", "0x1111111111111111111111111111111111111111", c, c));
    env.init_wrapper();
    env.init_module(default_params());
    let m1 = env.module;
    ok(add_collection(&mut env, m0));
    expect_err(add_collection(&mut env, m0), RookError::AlreadyCollection);
    ok(add_collection(&mut env, m1));
    // volume: 300 SOL of mints on collection 1, 100 SOL on collection 0
    ok(env.post(FLOOR));
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    ok(env.mint_units(&u, &acc, 300 * SOL, 0));
    env.use_listing(0);
    ok(env.post(FLOOR));
    let acc0 = env.token_account(&u.pubkey());
    ok(env.mint_units(&u, &acc0, 100 * SOL, 0));
    ok(receive(&mut env, 40 * SOL));
    ok(allocate(&mut env, Pubkey::default()));
    let begin = |env: &mut Env, metas: Vec<Pubkey>| {
        let mut i = ix(rook_core::instruction::BeginDistribution {}, rook_core::accounts::BeginDistribution { treasury: env.treasury });
        for m in metas {
            i.accounts.push(anchor_lang::solana_program::instruction::AccountMeta::new_readonly(m, false));
        }
        let u = env.user.insecure_clone();
        env.send(&[i], &[&u])
    };
    expect_err(begin(&mut env, vec![m1, m0]), RookError::NotCollection);
    expect_err(begin(&mut env, vec![m0]), RookError::BadInput);
    ok(begin(&mut env, vec![m0, m1]));
    let dist = |env: &mut Env, index: u8, module: Pubkey, vault: Pubkey| {
        let u = env.user.insecure_clone();
        let i = ix(
            rook_core::instruction::Distribute { index },
            rook_core::accounts::Distribute {
                treasury: env.treasury,
                module,
                module_vault: vault,
                ops: None,
                raffle: raffle_of(env, &module),
            },
        );
        env.send(&[i], &[&u])
    };
    let v1 = pda(&[MODULE_VAULT, m1.as_ref()]);
    expect_err(dist(&mut env, 0, m1, v1), RookError::NotCollection);
    ok(dist(&mut env, 0, m0, v0));
    expect_err(dist(&mut env, 0, m0, v0), RookError::NothingToDo);
    let meta = ok(dist(&mut env, 1, m1, v1));
    println!("distribute (two collections, under the cap, no ops): {} CU", meta.compute_units_consumed);
    let s0 = env.get::<Module>(&m0).surplus;
    let s1 = env.get::<Module>(&m1).surplus;
    assert_eq!(s0, 40 * SOL / 4, "no ops address: the whole 40 SOL is the reserves pot");
    assert_eq!(s1, 40 * SOL * 3 / 4);
    assert_eq!(treasury(&env).reserves_pot, 0);
    env.check_books();
}

// ---- the per-collection reserve cap (operator decision 2026-10-10) ----------------------------------------------------

/// One registered collection with seven-day volume (100 SOL of mints at `FLOOR`); its accepted floor read back.
pub fn one_collection() -> Env {
    let mut env = Env::new();
    let m = env.module;
    ok(add_collection(&mut env, m));
    ok(env.post(FLOOR));
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    ok(env.mint_units(&u, &acc, 100 * SOL, 0));
    env
}

pub fn begin_one(env: &mut Env) -> Res {
    let m = env.module;
    let mut i = ix(rook_core::instruction::BeginDistribution {}, rook_core::accounts::BeginDistribution { treasury: env.treasury });
    i.accounts.push(anchor_lang::solana_program::instruction::AccountMeta::new_readonly(m, false));
    let u = env.user.insecure_clone();
    env.send(&[i], &[&u])
}

pub fn dist_one(env: &mut Env, ops: Option<Pubkey>) -> Res {
    let u = env.user.insecure_clone();
    let i = ix(
        rook_core::instruction::Distribute { index: 0 },
        rook_core::accounts::Distribute {
            treasury: env.treasury,
            module: env.module,
            module_vault: env.vault,
            ops,
            raffle: raffle_of(env, &env.module),
        },
    );
    env.send(&[i], &[&u])
}

/// Income `lamports` → allocate → a round over the one collection, ready for `distribute(0)`.
pub fn fund_round(env: &mut Env, lamports: u64) {
    ok(receive(env, lamports));
    let ops = treasury(env).ops;
    ok(allocate(env, ops));
    ok(begin_one(env));
}

pub fn cap_of(env: &Env) -> u64 {
    let t = treasury(env);
    env.module().accepted * t.reserve_cap_bps as u64 / 10_000
}

#[track_caller]
pub fn treasury_covers_pot(env: &Env) {
    let rent = env.svm.minimum_balance_for_rent_exemption(env.svm.get_account(&env.treasury).unwrap().data.len());
    assert!(env.lamports(&env.treasury) - rent >= treasury(env).reserves_pot, "treasury lamports − rent ≥ reserves_pot");
}

/// Rewrites the module's accepted floor (a test-only shortcut: the books do not depend on it).
pub fn set_accepted(env: &mut Env, v: u64) {
    use anchor_lang::AccountSerialize;
    let k = env.module;
    let mut m: Module = env.get(&k);
    m.accepted = v;
    let mut acc = env.svm.get_account(&k).unwrap();
    let mut d = Vec::new();
    m.try_serialize(&mut d).unwrap();
    acc.data[..d.len()].copy_from_slice(&d);
    env.svm.set_account(k, acc).unwrap();
}

#[test]
fn the_reserve_cap_starts_at_two_floors() {
    let env = Env::new();
    let t = treasury(&env);
    assert_eq!((t.reserve_cap_bps, t.pending_reserve_cap_bps, t.pending_reserve_cap_at), (DEFAULT_RESERVE_CAP_BPS, 0, 0));
    assert_eq!(DEFAULT_RESERVE_CAP_BPS, 20_000);
}

// Under the cap: the full share reaches the surplus; ops receives nothing from distribute.
#[test]
fn distribute_under_the_cap_pays_the_full_share() {
    let mut env = one_collection();
    let ops = Keypair::new().pubkey();
    let o = env.owner.insecure_clone();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops }, &o));
    env.svm.airdrop(&ops, SOL).unwrap();
    fund_round(&mut env, 40 * SOL); // 36 SOL to the pot, 4 SOL to ops
    let s0 = env.module().surplus;
    assert!(s0 + 36 * SOL < cap_of(&env), "well under the cap");
    let ops0 = env.lamports(&ops);
    let t0 = treasury(&env);
    let meta = ok(dist_one(&mut env, Some(ops)));
    println!("distribute (under the cap, ops passed): {} CU", meta.compute_units_consumed);
    let t = treasury(&env);
    assert_eq!(env.module().surplus - s0, 36 * SOL);
    assert_eq!(env.lamports(&ops), ops0, "nothing to ops below the cap");
    assert_eq!((t.reserves_pot, t.total_to_reserves - t0.total_to_reserves, t.total_to_ops - t0.total_to_ops), (0, 36 * SOL, 0));
    // the ops account is not needed below the cap
    fund_round(&mut env, 40 * SOL);
    let meta = ok(dist_one(&mut env, None));
    println!("distribute (under the cap, no ops account): {} CU", meta.compute_units_consumed);
    treasury_covers_pot(&env);
    env.check_books();
}

// Partial, then at the cap: the surplus is topped up to the cap and the rest goes to ops; at the cap, all of it goes to ops.
#[test]
fn distribute_above_the_cap_sends_the_excess_to_ops() {
    let mut env = one_collection();
    let ops = Keypair::new().pubkey();
    let o = env.owner.insecure_clone();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops }, &o));
    env.svm.airdrop(&ops, SOL).unwrap();
    let cap = cap_of(&env);
    assert_eq!(cap, 2 * env.module().accepted, "two floors");
    // partial: a pot larger than the room under the cap
    let income = (cap + 100 * SOL) * 10 / 9 + 10; // the pot is 90 % of it
    fund_round(&mut env, income);
    let pot = treasury(&env).reserves_pot;
    let s0 = env.module().surplus;
    let room = cap - s0;
    assert!(pot > room);
    let ops0 = env.lamports(&ops);
    let t0 = treasury(&env);
    expect_err(dist_one(&mut env, None), RookError::BadInput); // the excess needs the ops account
    expect_err(dist_one(&mut env, Some(Keypair::new().pubkey())), RookError::BadInput); // and only treasury.ops
    let meta = ok(dist_one(&mut env, Some(ops)));
    println!("distribute (partial: surplus + ops): {} CU", meta.compute_units_consumed);
    let t = treasury(&env);
    assert_eq!(env.module().surplus, cap, "topped up to the cap");
    assert_eq!(env.lamports(&ops) - ops0, pot - room);
    assert_eq!(t.total_to_reserves - t0.total_to_reserves, room);
    assert_eq!(t.total_to_ops - t0.total_to_ops, pot - room);
    assert_eq!(t.reserves_pot, 0);
    treasury_covers_pot(&env);
    env.check_books();
    // at the cap: the whole share goes to ops
    fund_round(&mut env, 50 * SOL);
    let pot = treasury(&env).reserves_pot;
    let ops0 = env.lamports(&ops);
    let meta = ok(dist_one(&mut env, Some(ops)));
    println!("distribute (at the cap: all to ops): {} CU", meta.compute_units_consumed);
    assert_eq!(env.module().surplus, cap);
    assert_eq!(env.lamports(&ops) - ops0, pot);
    assert_eq!(treasury(&env).reserves_pot, 0);
    // over the cap (a lower cap applied later): nothing more reaches the surplus, nothing is taken back
    ok(admin(&mut env, rook_core::instruction::SetReserveCap { bps: MIN_RESERVE_CAP_BPS }, &o));
    let at = env.now + TREASURY_DELAY;
    env.set_time(at);
    ok(apply(&mut env, rook_core::instruction::ApplyReserveCap {}));
    assert!(env.module().surplus > cap_of(&env));
    fund_round(&mut env, 10 * SOL);
    let pot = treasury(&env).reserves_pot;
    let ops0 = env.lamports(&ops);
    ok(dist_one(&mut env, Some(ops)));
    assert_eq!((env.module().surplus, env.lamports(&ops) - ops0), (cap, pot));
    treasury_covers_pot(&env);
    env.check_books();
}

// No accepted floor: the full share, however large, as before the cap.
#[test]
fn distribute_without_a_floor_pays_the_full_share() {
    let mut env = one_collection();
    let ops = Keypair::new().pubkey();
    let o = env.owner.insecure_clone();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops }, &o));
    set_accepted(&mut env, 0);
    fund_round(&mut env, 1_000 * SOL);
    let pot = treasury(&env).reserves_pot;
    assert!(pot > 2 * FLOOR);
    let s0 = env.module().surplus;
    ok(dist_one(&mut env, None));
    assert_eq!(env.module().surplus - s0, pot);
    assert_eq!(treasury(&env).reserves_pot, 0);
    treasury_covers_pot(&env);
    env.check_books();
}

// No ops address: the excess stays in the reserves pot, the index is paid for the round, and the next round may begin.
#[test]
fn distribute_without_ops_keeps_the_excess_and_the_round_closes() {
    let mut env = one_collection();
    let cap = cap_of(&env);
    fund_round(&mut env, cap + 100 * SOL); // no ops: the whole income is the pot
    let pot = treasury(&env).reserves_pot;
    let room = cap - env.module().surplus;
    let t0 = treasury(&env);
    ok(dist_one(&mut env, None));
    let t = treasury(&env);
    assert_eq!(env.module().surplus, cap);
    assert_eq!(t.reserves_pot, pot - room, "the excess stays in the pot");
    assert_eq!((t.total_to_reserves - t0.total_to_reserves, t.total_to_ops - t0.total_to_ops), (room, 0));
    assert!(!t.round_open().unwrap(), "index 0 is paid for this round");
    expect_err(dist_one(&mut env, None), RookError::NothingToDo);
    treasury_covers_pot(&env);
    // the next round opens at once; at the cap, distribute still succeeds and closes it, moving nothing
    ok(begin_one(&mut env));
    let kept = treasury(&env).reserves_pot;
    let meta = ok(dist_one(&mut env, None));
    println!("distribute (at the cap, no ops: kept): {} CU", meta.compute_units_consumed);
    let t = treasury(&env);
    assert_eq!((t.reserves_pot, env.module().surplus), (kept, cap));
    assert!(!t.round_open().unwrap());
    ok(begin_one(&mut env));
    treasury_covers_pot(&env);
    env.check_books();
}

// The cap changes only through a proposal that waits TREASURY_DELAY; bounds 1_000..=200_000; owner only.
#[test]
fn the_reserve_cap_changes_after_a_day_within_bounds() {
    let mut env = Env::new();
    let o = env.owner.insecure_clone();
    let u = env.user.insecure_clone();
    expect_err(apply(&mut env, rook_core::instruction::ApplyReserveCap {}), RookError::NothingPending);
    expect_err(admin(&mut env, rook_core::instruction::SetReserveCap { bps: 30_000 }, &u), RookError::NotOwner);
    for bad in [0, MIN_RESERVE_CAP_BPS - 1, MAX_RESERVE_CAP_BPS + 1, u32::MAX] {
        expect_err(admin(&mut env, rook_core::instruction::SetReserveCap { bps: bad }, &o), RookError::BadReserveCap);
    }
    ok(admin(&mut env, rook_core::instruction::SetReserveCap { bps: MAX_RESERVE_CAP_BPS }, &o));
    ok(admin(&mut env, rook_core::instruction::SetReserveCap { bps: MIN_RESERVE_CAP_BPS }, &o));
    let t0 = env.now;
    ok(admin(&mut env, rook_core::instruction::SetReserveCap { bps: 30_000 }, &o)); // replaces, restarts the wait
    assert_eq!(treasury(&env).reserve_cap_bps, DEFAULT_RESERVE_CAP_BPS, "a proposal changes nothing yet");
    env.set_time(t0 + TREASURY_DELAY - 1);
    expect_err(apply(&mut env, rook_core::instruction::ApplyReserveCap {}), RookError::TooEarly);
    env.set_time(t0 + TREASURY_DELAY);
    ok(apply(&mut env, rook_core::instruction::ApplyReserveCap {}));
    let t = treasury(&env);
    assert_eq!((t.reserve_cap_bps, t.pending_reserve_cap_bps, t.pending_reserve_cap_at), (30_000, 0, 0));
    expect_err(apply(&mut env, rook_core::instruction::ApplyReserveCap {}), RookError::NothingPending);
}

// FloorModule skim → the treasury (Treasury.t.sol: test_theGraduationFeeAndTheSkimsReachTheTreasury)
#[test]
fn skims_reach_the_treasury() {
    let mut p = default_params();
    p.revenue_target = 0;
    let mut env = Env::with_params(p);
    ok(env.post(FLOOR));
    let acc = env.user_with_units(400 * SOL);
    let u = env.user.insecure_clone();
    ok(env.burn_units(&u, &acc, &u.pubkey(), rook_core::units::ONE, 0));
    let rev = env.module().revenue;
    assert!(rev > 0);
    let t0 = env.lamports(&env.treasury);
    ok(env.skim());
    assert_eq!(env.lamports(&env.treasury) - t0, rev);
    ok(allocate(&mut env, Pubkey::default()));
    env.check_books();
}

// ---- the wake desk --------------------------------------------------------------------------------------------------

fn request(env: &mut Env, who: &Keypair, bond: u64) -> Res {
    let d: Desk = env.get(&env.desk);
    let slot = pda(&[WAKE_SLOT, env.listing.as_ref()]);
    let last = if env.exists(&slot) { env.get::<WakeSlot>(&slot).last_plus_one } else { 0 };
    let last_request = if last != 0 { Some(pda(&[REQUEST, &(last - 1).to_le_bytes()])) } else { None };
    let i = ix(
        rook_core::instruction::Request { bond },
        rook_core::accounts::RequestWake {
            requester: who.pubkey(),
            desk: env.desk,
            listing: env.listing,
            module: env.module,
            wake_slot: slot,
            last_request,
            request: pda(&[REQUEST, &d.request_count.to_le_bytes()]),
            system_program: system_program::ID,
        },
    );
    let w = who.insecure_clone();
    env.send(&[i], &[&w])
}

fn req(env: &Env, id: u64) -> Request {
    env.get(&pda(&[REQUEST, &id.to_le_bytes()]))
}

/// A rook_backing `Vault` serialized by rook_backing's own type (the layout contract: `rook_layouts::backing_vault`), for a coin
/// launched at `launched_at` (its DBC pool's creation) and, if `graduated`, backed at `launched_at + 3_600`.
fn backing_vault(env: &mut Env, listing: Pubkey, creator: Pubkey, launched_at: i64, graduated: bool) -> Pubkey {
    backing_vault_at(env, listing, creator, launched_at, if graduated { Some(launched_at + 3_600) } else { None })
}

fn backing_vault_at(env: &mut Env, listing: Pubkey, creator: Pubkey, launched_at: i64, backed_at: Option<i64>) -> Pubkey {
    use anchor_lang::AccountSerialize;
    let k = Keypair::new().pubkey();
    let v = rook_backing::Vault {
        partner: BackingVaultView::partner_of(&listing),
        coin_mint: Keypair::new().pubkey(),
        dbc_pool: Keypair::new().pubkey(),
        dbc_config: Keypair::new().pubkey(),
        creator,
        wx_account: Keypair::new().pubkey(),
        graduated: backed_at.is_some(),
        lamports_backed: 42 * SOL,
        wx_units: 1,
        coin_supply_at_backing: 1,
        backed_at: backed_at.unwrap_or(0),
        launched_at,
        coins_burned: 0,
        wx_paid: 0,
        bump: 255,
        wx_bump: 254,
    };
    let mut data = Vec::new();
    v.try_serialize(&mut data).unwrap();
    let parsed = BackingVaultView::parse(&data).expect("rook_core parses rook_backing's Vault");
    assert_eq!(
        (parsed.partner, parsed.coin_mint, parsed.creator, parsed.graduated, parsed.backed_at, parsed.launched_at),
        (v.partner, v.coin_mint, v.creator, v.graduated, v.backed_at, v.launched_at),
        "the shared layout reads every field rook_core uses"
    );
    assert_eq!(rook_backing::ID, ROOK_BACKING_ID);
    let lamports = env.svm.minimum_balance_for_rent_exemption(data.len());
    env.svm
        .set_account(k, solana_account::Account { lamports, data, owner: ROOK_BACKING_ID, executable: false, rent_epoch: 0 })
        .unwrap();
    k
}

fn reclaim(env: &mut Env, id: u64, vault: Pubkey) -> Res {
    let u = env.user.insecure_clone();
    let r = req(env, id);
    let i = ix(
        rook_core::instruction::Reclaim {},
        rook_core::accounts::Reclaim {
            payer: u.pubkey(),
            desk: env.desk,
            request: pda(&[REQUEST, &id.to_le_bytes()]),
            requester: r.requester,
            backing_vault: vault,
            spent: pda(&[SPENT, vault.as_ref()]),
            system_program: system_program::ID,
        },
    );
    env.send(&[i], &[&u])
}

fn forfeit(env: &mut Env, id: u64) -> Res {
    let u = env.user.insecure_clone();
    let r = req(env, id);
    let i = ix(
        rook_core::instruction::Forfeit {},
        rook_core::accounts::Forfeit {
            desk: env.desk,
            request: pda(&[REQUEST, &id.to_le_bytes()]),
            module: env.module,
            requester: r.requester,
            treasury: env.treasury,
        },
    );
    env.send(&[i], &[&u])
}

fn mark_served(env: &mut Env, id: u64) -> Res {
    let u = env.user.insecure_clone();
    let i = ix(
        rook_core::instruction::MarkServed {},
        rook_core::accounts::MarkServed { desk: env.desk, request: pda(&[REQUEST, &id.to_le_bytes()]), module: env.module },
    );
    env.send(&[i], &[&u])
}

fn withdraw(env: &mut Env, who: &Keypair, id: u64) -> Res {
    let r = req(env, id);
    let i = ix(
        rook_core::instruction::Withdraw {},
        rook_core::accounts::Withdraw {
            requester: who.pubkey(),
            request: pda(&[REQUEST, &id.to_le_bytes()]),
            wake_slot: pda(&[WAKE_SLOT, r.listing.as_ref()]),
        },
    );
    let w = who.insecure_clone();
    env.send(&[i], &[&w])
}

// WakeDesk.t.sol: test_request_needsTheExactBond_andAListedWrapper, test_request_recordsWhatTheCanisterReads_andOneOpenPerCollection,
// test_diM2_aRequestWhileTheCollectionIsAwake_isRefused_andOneAfterItSleepsIsTaken, test_theDeskHoldsOnlyBonds_eachRequestItsOwn
#[test]
fn request_needs_the_bond_one_open_per_collection_and_a_sleeping_collection() {
    let mut env = Env::new();
    let r1 = Keypair::new();
    env.svm.airdrop(&r1.pubkey(), 10 * SOL).unwrap();
    expect_err(request(&mut env, &r1, WAKE_BOND - 1), RookError::WrongBond);
    ok(request(&mut env, &r1, WAKE_BOND));
    let r = req(&env, 0);
    assert_eq!((r.requester, r.at, r.bond, r.closed), (r1.pubkey(), env.now, WAKE_BOND, false));
    let p = pda(&[REQUEST, &0u64.to_le_bytes()]);
    let rent = env.svm.minimum_balance_for_rent_exemption(env.svm.get_account(&p).unwrap().data.len());
    assert_eq!(env.lamports(&p), rent + WAKE_BOND, "each request holds its own bond");
    let r2 = env.user.insecure_clone();
    expect_err(request(&mut env, &r2, WAKE_BOND), RookError::AlreadyRequested);
    env.advance_time(7 * 86_400);
    ok(env.post(FLOOR));
    expect_err(request(&mut env, &r2, WAKE_BOND), RookError::Awake);
    env.advance_time(3 * 3_600);
    ok(request(&mut env, &r2, WAKE_BOND));
}

// WakeDesk.t.sol: test_reclaim_paysTheRequesterBack_whenTheyLaunch, test_reclaim_stillLandsInTheGrace_butNotAfter,
// test_reclaim_refusesAFloorBuysOnlyCoin_andOneCoinPaysBackOneBond, test_reclaim_refusesTheSameCoinForTwoRequests
#[test]
fn reclaim_pays_back_when_the_requesters_coin_graduates() {
    let mut env = Env::new();
    let rq = Keypair::new();
    env.svm.airdrop(&rq.pubkey(), 10 * SOL).unwrap();
    ok(request(&mut env, &rq, WAKE_BOND));
    let at = env.now;
    let listing = env.listing;
    let not_grad = backing_vault(&mut env, listing, rq.pubkey(), at + 10, false);
    expect_err(reclaim(&mut env, 0, not_grad), RookError::NotGraduated);
    let other = backing_vault(&mut env, listing, Keypair::new().pubkey(), at + 10, true);
    expect_err(reclaim(&mut env, 0, other), RookError::NotALaunch);
    let before = backing_vault(&mut env, listing, rq.pubkey(), at - 1, true);
    expect_err(reclaim(&mut env, 0, before), RookError::NotALaunch); // launched before the request
    let elsewhere = backing_vault(&mut env, Keypair::new().pubkey(), rq.pubkey(), at + 10, true);
    expect_err(reclaim(&mut env, 0, elsewhere), RookError::NotALaunch);
    let good = backing_vault(&mut env, listing, rq.pubkey(), at + 10, true);
    let b0 = env.lamports(&rq.pubkey());
    env.advance_time(7 * 86_400 + 86_400 - 1); // still in the grace
    ok(reclaim(&mut env, 0, good));
    assert_eq!(env.lamports(&rq.pubkey()) - b0, WAKE_BOND);
    expect_err(reclaim(&mut env, 0, good), RookError::Closed);
    // a later request cannot reuse the coin; and after the grace it is too late
    ok(request(&mut env, &rq, WAKE_BOND));
    expect_err(reclaim(&mut env, 1, good), RookError::NotALaunch); // created before request 1
    let t = env.now + 1;
    let good2 = backing_vault(&mut env, listing, rq.pubkey(), t, true);
    env.advance_time(8 * 86_400);
    expect_err(reclaim(&mut env, 1, good2), RookError::TooLate);
}

// Open question 3 of the as-built doc: a coin launched BEFORE the request that graduates (is backed) AFTER it does not pay
// the bond back (the check is the DBC pool's creation time, not the backing time); one launched at the request does.
#[test]
fn a_coin_launched_before_the_request_never_reclaims() {
    let mut env = Env::new();
    let rq = Keypair::new();
    env.svm.airdrop(&rq.pubkey(), 10 * SOL).unwrap();
    let launched = env.now - 600;
    env.advance_time(60);
    ok(request(&mut env, &rq, WAKE_BOND));
    let at = env.now;
    let listing = env.listing;
    let early = backing_vault_at(&mut env, listing, rq.pubkey(), launched, Some(at + 3_600));
    expect_err(reclaim(&mut env, 0, early), RookError::NotALaunch);
    // a vault that is not exactly rook_backing's Vault (truncated) is no launch
    let k = Keypair::new().pubkey();
    let mut acc = env.svm.get_account(&early).unwrap();
    acc.data.truncate(acc.data.len() - 1);
    env.svm.set_account(k, acc).unwrap();
    expect_err(reclaim(&mut env, 0, k), RookError::NotALaunch);
    let same_second = backing_vault_at(&mut env, listing, rq.pubkey(), at, Some(at + 3_600));
    ok(reclaim(&mut env, 0, same_second));
}

#[test]
fn one_coin_pays_back_one_bond() {
    let mut env = Env::new();
    let rq = Keypair::new();
    env.svm.airdrop(&rq.pubkey(), 10 * SOL).unwrap();
    ok(request(&mut env, &rq, WAKE_BOND));
    // the first request closes by forfeit (not served: back to the requester)
    env.advance_time(8 * 86_400);
    ok(forfeit(&mut env, 0));
    ok(request(&mut env, &rq, WAKE_BOND));
    let listing = env.listing;
    let t = env.now + 5;
    let coin = backing_vault(&mut env, listing, rq.pubkey(), t, true);
    ok(reclaim(&mut env, 1, coin));
    ok(withdraw(&mut env, &rq, 1));
    ok(request(&mut env, &rq, WAKE_BOND));
    let r2_at = req(&env, 2).at;
    assert!(r2_at <= env.now);
    // same coin (created after request 2 too, since time did not move): spent
    let _ = coin;
    expect_err(reclaim(&mut env, 2, coin), RookError::AlreadySpent);
}

// WakeDesk.t.sol: test_forfeit_sendsTheBondToTheTreasury_onlyAfterTheWindowAndTheGrace_onBothClocks,
// test_forfeit_refundsAWakeTheCanisterNeverServed_andChargesOneItServed, test_diM1_*
#[test]
fn forfeit_charges_a_served_wake_and_refunds_an_unserved_one() {
    let mut env = Env::new();
    let rq = Keypair::new();
    env.svm.airdrop(&rq.pubkey(), 10 * SOL).unwrap();
    ok(request(&mut env, &rq, WAKE_BOND));
    expect_err(mark_served(&mut env, 0), RookError::NotServed);
    env.advance_time(3_600);
    ok(env.post(FLOOR)); // the canister serves the wake inside the window
    ok(mark_served(&mut env, 0));
    assert_eq!(req(&env, 0).served_at, env.now);
    expect_err(forfeit(&mut env, 0), RookError::TooEarly);
    env.advance_time(8 * 86_400);
    ok(env.post(FLOOR)); // a later post neither adds nor takes away
    let t0 = env.lamports(&env.treasury);
    ok(forfeit(&mut env, 0));
    assert_eq!(env.lamports(&env.treasury) - t0, WAKE_BOND);
    expect_err(forfeit(&mut env, 0), RookError::Closed);
    // an unserved wake: the bond goes back
    env.advance_time(3 * 3_600);
    ok(request(&mut env, &rq, WAKE_BOND));
    env.advance_time(8 * 86_400);
    let b0 = env.lamports(&rq.pubkey());
    ok(forfeit(&mut env, 1));
    assert_eq!(env.lamports(&rq.pubkey()) - b0, WAKE_BOND);
}

// WakeDesk.t.sol: test_aRequesterThatRefusesEth_isCreditedItsReclaim_andWithdrawsElsewhere (on Solana: the rent of a closed
// request goes back to its requester)
#[test]
fn withdraw_returns_a_closed_requests_rent() {
    let mut env = Env::new();
    let rq = Keypair::new();
    env.svm.airdrop(&rq.pubkey(), 10 * SOL).unwrap();
    ok(request(&mut env, &rq, WAKE_BOND));
    expect_err(withdraw(&mut env, &rq, 0), RookError::NotClosed);
    let u = env.user.insecure_clone();
    expect_err(withdraw(&mut env, &u, 0), RookError::BadInput);
    env.advance_time(8 * 86_400);
    ok(forfeit(&mut env, 0));
    let b0 = env.lamports(&rq.pubkey());
    ok(withdraw(&mut env, &rq, 0));
    assert!(env.lamports(&rq.pubkey()) > b0);
    assert!(!env.exists(&pda(&[REQUEST, &0u64.to_le_bytes()])));
    // the slot is clear: a new request needs no previous account
    ok(request(&mut env, &rq, WAKE_BOND));
}

// WakeDesk.t.sol: test_constructor_boundsTheBond. D-c (2026-10-09): MIN_BOND / MAX_BOND are init parameters (the launch file's
// lamports), stored, checked as the constructor checked its constants; a zero or inverted bound is refused.
#[test]
fn the_desk_takes_its_bond_bounds_as_parameters() {
    let mut env = Env::bare();
    let d: Desk = env.get(&env.desk);
    assert_eq!((d.bond, d.min_bond, d.max_bond, d.window), (WAKE_BOND, MIN_BOND, MAX_BOND, 7 * 86_400));
    // a second registry-owned desk cannot exist (one PDA); check the refusals on a fresh SVM
    let mut svm = litesvm::LiteSVM::new();
    svm.add_program(rook_core::id(), &rook_core_so()).unwrap();
    std::mem::swap(&mut env.svm, &mut svm);
    let o = env.owner.insecure_clone();
    env.svm.airdrop(&o.pubkey(), 10 * SOL).unwrap();
    env.set_upgrade_authority(Some(&o.pubkey()));
    let i = env.init_registry_ix(&o.pubkey(), &o.pubkey(), &o.pubkey());
    ok(env.send(&[i], &[&o]));
    let init = |env: &mut Env, p: DeskParams| {
        let o = env.owner.insecure_clone();
        let i = ix(
            rook_core::instruction::InitDesk { params: p },
            rook_core::accounts::InitDesk {
                owner: o.pubkey(),
                payer: o.pubkey(),
                registry: env.registry,
                desk: env.desk,
                system_program: system_program::ID,
            },
        );
        env.send(&[i], &[&o])
    };
    let base = desk_params();
    expect_err(init(&mut env, DeskParams { min_bond: 0, ..base }), RookError::BadInput);
    expect_err(init(&mut env, DeskParams { min_bond: MAX_BOND + 1, ..base }), RookError::BadInput);
    expect_err(init(&mut env, DeskParams { bond: MIN_BOND - 1, ..base }), RookError::BadInput);
    expect_err(init(&mut env, DeskParams { bond: MAX_BOND + 1, ..base }), RookError::BadInput);
    expect_err(init(&mut env, DeskParams { window: 86_400 - 1, ..base }), RookError::BadInput);
    expect_err(init(&mut env, DeskParams { window: 30 * 86_400 + 1, ..base }), RookError::BadInput);
    // other launch values (a different SOL/ETH rate) are taken as given: nothing here converts
    let other = DeskParams { bond: 123_456, min_bond: 100_000, max_bond: 9 * SOL, window: 3 * 86_400 };
    ok(init(&mut env, other));
    let d: Desk = env.get(&env.desk);
    assert_eq!((d.bond, d.min_bond, d.max_bond, d.window), (123_456, 100_000, 9 * SOL, 3 * 86_400));
}
