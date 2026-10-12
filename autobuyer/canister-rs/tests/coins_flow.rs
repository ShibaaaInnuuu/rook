//! W6: the coins against the fake chain. DBC's, DAMM v2's and Metaplex's own effects are LiteSVM's to check (`solana/tests/solix2_fixtures.rs`,
//! `tests/solix2_builders.rs`); here their accounts are planted from the landed fixtures (`tests/fixtures/solix2/flow.json`, patched to the
//! World's keys) and what is checked is what the coins decide from them, what they sign (and in which order), how the books take it (the fee
//! cash and the redemption as links), and what the state records: a collection's opening, discovery and graduation, a fee claim and cash,
//! a coin redemption.
mod desk_common;
mod fakerpc;

use base64::Engine;
use desk_common::*;
use etch_autobuyer::books::{self, ListingKind, ONE};
use etch_autobuyer::coins::state::{self as cs, CoinRec, CoinStage, ColStage, CollectionPin, RedeemState, Vault};
use etch_autobuyer::coins::{fees, grad, open, redeem, unlock};
use etch_autobuyer::desk::state;
use etch_autobuyer::pooldesk::state as pool;
use etch_autobuyer::solix2::decode::{DbcPool, Mint};
use etch_autobuyer::solix2::guard;
use etch_autobuyer::solix2::{self, dbc};
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::soltx::{self, Pubkey};
use serde_json::Value;

const RENT82: u64 = 1_461_600;
/// The fixture's config raises 5 wX by its migration price.
const T_WX: u64 = 5;

fn flow() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solix2/flow.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// A landed account of the flow: its owner and bytes.
fn fixture(name: &str) -> (Pubkey, Vec<u8>) {
    let f = flow();
    let a = f["accounts"].as_array().unwrap().iter().find(|a| a["name"] == name).unwrap_or_else(|| panic!("no account {name}"));
    (soltx::key(a["owner"].as_str().unwrap()), base64::engine::general_purpose::STANDARD.decode(a["data"].as_str().unwrap()).unwrap())
}

fn n(v: &Value) -> u128 {
    match v {
        Value::Number(x) => x.as_u64().unwrap() as u128,
        Value::String(s) => s.parse().unwrap(),
        _ => panic!("a number, got {v}"),
    }
}

fn k_fc() -> Pubkey {
    pk(KeyPath::Claimer)
}
fn cfg_key() -> Pubkey {
    pk(KeyPath::Config(COLL as u32))
}
fn k_tre() -> Pubkey {
    pk(KeyPath::Treasury)
}
fn k_desk() -> Pubkey {
    pk(KeyPath::Desk(COLL as u32))
}

/// The fixture's coin (the base mint of its DBC pool).
fn coin() -> Pubkey {
    soltx::key(flow()["keys"]["coin_mint"].as_str().unwrap())
}

/// Collection 0 pinned with the fixture's curve (`T_wX` = 5) and the World's listing; `wx_mint`: `Some(WX)` reads the World's mint back,
/// `None` makes one.
fn pin(wx_mint: Option<Pubkey>) -> CollectionPin {
    let f = flow();
    let cc = f["instructions"].as_array().unwrap().iter().find(|i| i["name"] == "dbc.create_config").unwrap()["params"].clone();
    CollectionPin {
        index: COLL as u32,
        slug: "bayc".into(),
        name: "Wrapped Bored Ape".into(),
        symbol: "wBOREDAPE".into(),
        uri: "https://rook.example/wx/bayc.json".into(),
        t_wx: T_WX,
        sqrt_start_price: n(&cc["sqrt_start_price"]),
        curve: cc["curve"].as_array().unwrap().iter().map(|p| (n(&p[0]), n(&p[1]))).collect(),
        kind: ListingKind::EthNft { chain_id: 1, collection: [0x5a; 20], custody: [0x11; 20] },
        params: books::default_params(20 * SOL),
        wx_mint,
        config: None,
    }
}

/// The wX metadata as Token Metadata leaves it (key, update authority `K_coll`, the mint; the rest is the names, not read).
fn plant_metadata(w: &mut World, mint: Pubkey) {
    let mut d = vec![0u8; 679];
    d[0] = 4;
    d[1..33].copy_from_slice(&module());
    d[33..65].copy_from_slice(&mint);
    w.plant(solix2::metadata_address(&mint), solix2::id(solix2::METADATA_PROGRAM), unlock::METADATA_RENT, d);
}

/// The DBC config from the fixture, as Rook's (quote = the collection's wX, `K_fc` the claimer and the leftover receiver).
fn config_bytes(wx: Pubkey) -> Vec<u8> {
    let (_, mut d) = fixture("dbc_config");
    d[8..40].copy_from_slice(&wx);
    d[40..72].copy_from_slice(&k_fc());
    d[72..104].copy_from_slice(&k_fc());
    d
}
fn plant_config(w: &mut World, at: Pubkey, wx: Pubkey) {
    w.plant(at, dbc::program(), unlock::CONFIG_RENT, config_bytes(wx));
}

/// The coin's mint (6 decimals, the fixed 1B supply; DBC's pool authority is its authority).
fn plant_coin_mint(w: &mut World, supply: u64) {
    let mut d = mint_data(solix2::id(solix2::DBC_POOL_AUTHORITY), supply);
    d[44] = dbc::pins::COIN_DECIMALS;
    w.plant(coin(), solix2::token_program(), RENT82, d);
}

/// A DBC VirtualPool from the fixture under the World's config, planted at the address the curve derives; `patch` edits the bytes first
/// (the flags the graduation reads: 305 is_migrated, 308 migration_progress, 309 is_withdraw_leftover, 311 migration_fee_withdraw_status,
/// 272 partner_quote_fee).
fn plant_pool(w: &mut World, name: &str, patch: impl FnOnce(&mut Vec<u8>)) -> Pubkey {
    let (owner, mut d) = fixture(name);
    d[72..104].copy_from_slice(&cfg_key());
    patch(&mut d);
    let key = dbc::Curve::new(cfg_key(), coin(), WX).pool;
    w.plant(key, owner, 3_841_920, d);
    key
}

/// Coins in `owner`'s coin account (planted: DBC's withdrawals are not run by the fake).
fn plant_coins(w: &mut World, owner: Pubkey, amount: u64) {
    w.plant(soltx::associated_token_address(&owner, &coin()), solix2::token_program(), RENT165, token_data(coin(), owner, amount));
}

fn coin_balance(w: &World, owner: &Pubkey) -> Option<u64> {
    let ata = soltx::associated_token_address(owner, &coin());
    w.chain.borrow().accounts.get(&ata).map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
}

fn coin_supply(w: &World) -> u64 {
    let a = w.chain.borrow().accounts[&coin()].clone();
    Mint::decode(&a.owner, &a.data).unwrap().supply
}

fn stage() -> ColStage {
    cs::with(|s| s.cols[&(COLL as u32)].stage.clone())
}

fn rec() -> CoinRec {
    cs::with(|s| s.coins[&coin()].clone())
}

/// A holder's mint of `lamports` through the books, so the module holds SOL backing and the chain's supply is the books' (the wX a test
/// then plants somewhere are "out of the holder's supply", as `pooldesk.rs` does).
fn back_the_module(w: &mut World, lamports: u64) -> u64 {
    let now = w.now as i64;
    let units = state::with_mut(|s| s.books.mint(COLL, lamports, 0, now).unwrap().units);
    w.inject_supply(units as i64);
    w.deposit_sol(&module(), lamports);
    units
}

/// Pins collection 0 on the World's wX mint, plants the metadata and the config the ceremony would have made, and opens it in one pass.
fn open_on_the_worlds_mint(w: &mut World) {
    open::register(pin(Some(WX))).unwrap();
    open::request_open(COLL as u32, &[1], w.now).unwrap();
    plant_metadata(w, WX);
    plant_config(w, cfg_key(), WX);
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l == "coins: collection 0 is open"), "{log:?}");
    assert_eq!(stage(), ColStage::Open);
    assert_eq!(cs::with(|s| (s.cols[&0].wx_mint, s.cols[&0].config)), (WX, cfg_key()));
}

/// Drives one step: ticks until a line with `signed` and "signed" shows (it may already be in `acc`), plants what the chain would then
/// show, and ticks on until the line reports `signed` landed. The landing tick may already carry the next step signed: `acc` keeps it.
fn step(w: &mut World, acc: &mut Vec<String>, signed: &str, plant: impl FnOnce(&mut World)) {
    let mut plant = Some(plant);
    let mut planted = false;
    if acc.iter().any(|l| l.contains(signed) && l.contains("signed")) {
        plant.take().unwrap()(w);
        planted = true;
    }
    for _ in 0..8 {
        let log = w.coins_tick();
        acc.extend(log.clone());
        if !planted && log.iter().any(|l| l.contains(signed) && l.contains("signed")) {
            plant.take().unwrap()(w);
            planted = true;
        }
        if log.iter().any(|l| l.contains(signed) && l.contains("landed")) {
            assert!(planted, "{signed} landed without being seen signed: {acc:?}");
            acc.retain(|l| !l.contains(signed));
            return;
        }
        w.settle();
    }
    panic!("{signed} never landed: {acc:?}");
}

#[test]
fn nothing_runs_before_a_pin_and_a_pinned_collection_opens_in_two_signed_steps() {
    let mut w = World::new();
    assert!(w.coins_tick().is_empty(), "nothing pinned: idle");
    // the canister makes the mint: a create_with_seed account of K_main, the config at its derived key
    open::register(pin(None)).unwrap();
    assert!(open::register(pin(None)).unwrap_err().contains("already pinned"));
    // pinned, not requested, the auto-open switch off: nothing opens (operator decision 2026-10-12)
    assert!(w.coins_tick().iter().all(|l| !l.contains("signed")));
    assert_eq!(cs::with(|s| s.signed_today), 0);
    open::request_open(COLL as u32, &[1], w.now).unwrap();
    let mint = open::wx_mint_of(&me(), COLL as u32);
    assert_ne!(mint, WX);
    let main_before = w.lamports(&me());
    // 1. the mint and its metadata, one transaction signed by K_main and K_coll
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("a collection's opening step is signed")), "{log:?}");
    assert_eq!(stage(), ColStage::Opening);
    assert_eq!(cs::with(|s| (s.opened_today, s.signed_today)), (1, 1));
    assert!(w.coins_tick().iter().any(|l| l.contains("waits: the line is busy")), "one step at a time on the crank line");
    w.settle();
    // the chain shows the mint as spl-token writes it: the fixture's bytes with K_coll as the authority
    let a = w.chain.borrow().accounts[&mint].clone();
    let (_, mut want) = fixture("wx_mint");
    want[4..36].copy_from_slice(&module());
    assert_eq!((a.owner, a.lamports, &a.data), (solix2::token_program(), RENT82, &want));
    guard::check_wx_mint(&Mint::decode(&a.owner, &a.data).unwrap(), &module()).unwrap();
    assert!(w.lamports(&me()) <= main_before - RENT82, "K_main paid the rent");
    // Token Metadata's account is the program's to write: planted; without it the pass waits
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("wX mint and metadata landed")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("the wX metadata does not exist yet")), "{log:?}");
    plant_metadata(&mut w, mint);
    w.advance_time(etch_autobuyer::coins::run::OPEN_EVERY);
    // 2. the DBC config, signed by K_main and the config's own key
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("a collection's opening step is signed")), "{log:?}");
    let crank = w.line(etch_autobuyer::solhome::LineId::Crank);
    assert!(matches!(crank.pending.as_ref().map(|p| &p.step), Some(etch_autobuyer::solhome::LineStep::Coin(etch_autobuyer::coins::CoinStep::OpenConfig { coll: 0 }))));
    w.settle();
    plant_config(&mut w, cfg_key(), mint);
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("DBC create_config landed")), "{log:?}");
    assert!(log.iter().any(|l| l == "coins: collection 0 is open"), "{log:?}");
    assert_eq!(stage(), ColStage::Open);
    let c = cs::with(|s| s.cols[&0].clone());
    assert_eq!((c.wx_mint, c.config, c.opened_at), (mint, cfg_key(), w.now));
    assert_eq!(cs::with(|s| (s.opened_today, s.signed_today)), (1, 2));
    assert!(cs::with(|s| s.last.iter().any(|(k, _, what)| k == "open" && what.contains("wBOREDAPE"))));
    // the books list collection 0 already (the World's): still one collection, the wX mint of the listing unchanged
    assert_eq!(w.books().collections.len(), 1);
    // nothing more to open: the pass rests
    assert!(w.coins_tick().is_empty());
    // a collection whose chain side differs from the pin is refused, not opened
    let mut w = World::new();
    open::register(pin(Some(WX))).unwrap();
    open::request_open(COLL as u32, &[1], w.now).unwrap();
    plant_metadata(&mut w, WX);
    let mut d = config_bytes(WX);
    d[40..72].copy_from_slice(&[7; 32]);
    w.plant(cfg_key(), dbc::program(), unlock::CONFIG_RENT, d);
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("REFUSED") && l.contains("fee_claimer")), "{log:?}");
    assert!(matches!(stage(), ColStage::Refused(ref why) if why.contains("fee_claimer")));
    assert!(w.coins_tick().iter().all(|l| !l.contains("signed")), "nothing under a refused config is Rook's");
}

#[test]
fn a_coin_is_discovered_under_the_config_and_graduates_migrate_fee_leftover_burn_backed() {
    let mut w = World::new();
    open_on_the_worlds_mint(&mut w);
    plant_coin_mint(&mut w, dbc::pins::TOTAL_SUPPLY);
    // the curve completed (progress 2) and nobody ran the migration; its partner fee zeroed (the claim is `fees_are_claimed...`'s)
    let pool_key = plant_pool(&mut w, "dbc_pool_graduated", |d| d[272..280].fill(0));
    // discovery: getProgramAccounts on DBC by config
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("coins: coin") && l.contains("of collection 0 found")), "{log:?}");
    let r = rec();
    assert_eq!((r.coll, r.pool, r.seq, r.stage, r.vault), (0, pool_key, 0, CoinStage::Curve, None));
    assert_eq!(r.launched_at, 1_791_504_000);
    assert!(!log.iter().any(|l| l.contains("signed")), "the graduation scan is not due yet: {log:?}");
    assert!(w.coins_tick().iter().all(|l| !l.contains("found")), "found once");
    // the scan comes due
    w.advance_time(grad::DISCOVER_EVERY);
    w.post_floor();
    let mut acc = vec![];
    let vault_owner = pk(KeyPath::Coin(coin()));
    let vault = soltx::associated_token_address(&vault_owner, &WX);
    // 1. the migration (the canister's two position NFT mints sign): the pool is then migrated
    step(&mut w, &mut acc, "DBC migration_damm_v2", |w| {
        plant_pool(w, "dbc_pool_migrated", |_| {});
    });
    let r = rec();
    assert_eq!(r.stage, CoinStage::Migrated);
    assert_eq!(r.partner_nft, Some(pk(KeyPath::PositionNft(1))), "the second NFT is the partner's");
    assert_eq!(r.damm_pool, Some(etch_autobuyer::solix2::damm::Pool::migrated(coin(), WX).pool));
    // 2. the partner's migration fee into the vault (its ATA made in the same transaction): the pool's flag then says it is withdrawn
    //    and the wX are in the vault
    step(&mut w, &mut acc, "withdraw_migration_fee into the vault", |w| {
        plant_pool(w, "dbc_pool_migrated", |d| d[311] = 0b10);
        w.deposit_wx(&vault_owner, 5 * ONE / 2);
        w.inject_supply((5 * ONE / 2) as i64);
    });
    assert_eq!(w.token(&vault_owner), 5 * ONE / 2, "the vault is K_coin's wX ATA (the transaction made it; DBC's fee is planted)");
    // 3. the leftovers to K_fc's coin account (made first), then 4. their burn
    const LEFTOVER: u64 = 200_000_065_854_868;
    step(&mut w, &mut acc, "DBC withdraw_leftover", |w| {
        plant_pool(w, "dbc_pool_migrated", |d| {
            d[311] = 0b10;
            d[309] = 1;
        });
        plant_coins(w, k_fc(), LEFTOVER);
    });
    assert_eq!(coin_balance(&w, &k_fc()), Some(LEFTOVER));
    let r = rec();
    assert_eq!(r.vault.as_ref().map(|v| (v.address, v.owner, v.wx_in, v.coins_burned)), Some((vault, vault_owner, 5 * ONE / 2, 0)), "the vault book opened on the fee");
    let supply_before = coin_supply(&w);
    step(&mut w, &mut acc, &format!("burn {LEFTOVER} leftover coins"), |_| {});
    assert_eq!(coin_supply(&w), supply_before - LEFTOVER, "the fake burned them");
    assert_eq!(coin_balance(&w, &k_fc()), Some(0));
    // 5. the surplus of 1 unit over the threshold is not worth a transaction: backed
    let log = w.coins_tick();
    assert!(!log.iter().any(|l| l.contains("signed")), "{log:?}");
    let r = rec();
    assert_eq!(r.stage, CoinStage::Backed);
    let v = r.vault.unwrap();
    assert_eq!((v.wx_in, v.coins_burned, v.wx_paid, v.backed_at), (5 * ONE / 2, LEFTOVER, 0, w.now));
    let kinds: Vec<String> = cs::with(|s| s.last.iter().map(|(k, ..)| k.clone()).collect());
    for k in ["migrate", "migration_fee", "leftover", "burn_leftover", "backed"] {
        assert!(kinds.contains(&k.to_string()), "{k} in {kinds:?}");
    }
    assert_eq!(cs::with(|s| s.signed_today), 4);
    assert!(cs::with(|s| s.waits.is_empty()));
    // the view the operator reads
    let views = etch_autobuyer::coins::api::coin_vaults(0);
    assert_eq!(views.len(), 1);
    assert_eq!((views[0].stage.as_str(), views[0].wx_in, views[0].vault.as_deref()), ("Backed", 5 * ONE / 2, Some(soltx::b58_encode(&vault).as_str())));
    assert_eq!(etch_autobuyer::coins::api::collections()[0].backed, 1);
    // the pool the canister addresses (the curve's derived address) is the planted, migrated one
    let a = w.chain.borrow().accounts[&pool_key].clone();
    assert!(DbcPool::decode(&a.owner, &a.data).unwrap().is_migrated);
}

#[test]
fn fees_are_claimed_to_k_fc_and_cashed_through_the_books_to_the_treasury_and_the_desk() {
    let mut w = World::new();
    open_on_the_worlds_mint(&mut w);
    plant_coin_mint(&mut w, dbc::pins::TOTAL_SUPPLY);
    // a coin on its curve with 0.0404 wX of partner fees unclaimed (the fixture's), worth a transaction at the floor
    const FEE: u64 = 40_404_206;
    plant_pool(&mut w, "dbc_pool_graduated", |d| d[308] = 0);
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("found")) && !log.iter().any(|l| l.contains("signed")), "found, and the curve scan is not due yet: {log:?}");
    // the curve scan comes due; the module backs a holder's 50 SOL of wX: the claimed fee wX are some of those (SOL-backed units
    // wherever they sit)
    w.advance_time(fees::CURVE_EVERY);
    w.post_floor();
    let units = back_the_module(&mut w, 50 * SOL);
    assert!(units > FEE);
    let mut acc = vec![];
    // the claim: claim_trading_fee for K_fc (its wX and coin accounts made first); the pool then shows the fee claimed and K_fc holds it
    step(&mut w, &mut acc, "DBC claim_trading_fee", |w| {
        plant_pool(w, "dbc_pool_graduated", |d| {
            d[308] = 0;
            d[272..280].fill(0);
        });
        w.deposit_wx(&k_fc(), FEE);
    });
    assert_eq!(coin_balance(&w, &k_fc()), Some(0), "K_fc's coin account exists (base fees, none under a quote-fee config)");
    assert_eq!(w.token(&k_fc()), FEE);
    assert_eq!(cs::with(|s| s.fees[&0].curve_wx), FEE, "the fee book: claimed from the curve");
    assert!(!acc.iter().any(|l| l.contains("signed")), "the cash scan is not due yet: {acc:?}");
    // the cash comes due: a quarter to the desk (half as wX, half burned for SOL), the rest burned at the module for the treasury; a link
    w.advance_time(fees::CASH_EVERY);
    w.post_floor();
    let supply_before = w.supply();
    let module_before = w.lamports(&module());
    let (treasury_before, desk_before) = (w.lamports(&k_tre()), w.lamports(&k_desk()));
    assert_eq!(treasury_before + desk_before, 0);
    step(&mut w, &mut acc, "fee wX of collection 0", |_| {});
    let sp = etch_autobuyer::coins::fees::split(FEE, true);
    assert_eq!((sp.to_desk, sp.desk_burn, sp.cash), (5_050_525, 5_050_526, 30_303_155));
    let f = cs::with(|s| s.fees[&0].clone());
    assert_eq!((f.curve_wx, f.cashed_wx, f.to_desk_wx, f.to_desk_burned_wx), (FEE, sp.cash, sp.to_desk, sp.desk_burn));
    let paid = f.cashed_lamports + f.to_desk_lamports;
    assert!(paid > 0);
    // the chain: the burn left K_fc empty, the desk holds its wX, the module paid the treasury and the desk exactly the booked split
    assert_eq!(w.token(&k_fc()), 0);
    assert_eq!(w.token(&k_desk()), sp.to_desk);
    assert_eq!(supply_before - w.supply(), sp.burned());
    assert_eq!(w.lamports(&k_tre()), f.cashed_lamports);
    assert_eq!(w.lamports(&k_desk()), f.to_desk_lamports);
    assert_eq!(module_before - w.lamports(&module()), paid);
    // the price: ≈ the floor (200 SOL a wX, 15 bps spread), and the D-j bound (95 % of the floor) held
    let floor_value = (FLOOR as u128 * sp.burned() as u128 / ONE as u128) as u64;
    assert!(paid * 10_000 >= floor_value * 9_500 && paid <= floor_value, "{paid} for {} units at {FLOOR}", sp.burned());
    // the books: treasury income booked once, the module's backing down by the burn, the desk's fee income noted, the link landed
    let b = w.books();
    assert_eq!((b.treasury.total_received, b.treasury.unallocated), (f.cashed_lamports, f.cashed_lamports));
    assert_eq!(b.get(COLL).unwrap().ledger.sol_backed, units - sp.burned());
    assert_eq!(pool::with(|s| (s.desks[&0].fee_lamports, s.desks[&0].fee_units)), (f.to_desk_lamports, sp.to_desk));
    assert!(cs::with(|s| s.receipts.is_empty()), "the receipt applied once");
    let links: Vec<_> = state::with(|s| s.link.links.values().cloned().collect());
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].phase, etch_autobuyer::chainlink::ledger::Phase::Landed);
    assert!(links[0].memo.starts_with("rook:v1:0:burn:"), "{}", links[0].memo);
    let kinds: Vec<String> = cs::with(|s| s.last.iter().map(|(k, ..)| k.clone()).collect());
    assert!(kinds.contains(&"curve_fee".to_string()) && kinds.contains(&"cash_fees".to_string()), "{kinds:?}");
    // nothing left to cash: the scan rests
    assert!(!w.coins_tick().iter().any(|l| l.contains("signed")));
}

#[test]
fn a_redemption_burns_the_coins_and_pays_their_share_of_the_vault() {
    let mut w = World::new();
    open_on_the_worlds_mint(&mut w);
    // a backed coin: 800M coins out (the leftovers burned), a vault of 2.5 wX
    const SUPPLY: u64 = 800_000_000_000_000;
    const VAULT: u64 = 5 * ONE / 2;
    plant_coin_mint(&mut w, SUPPLY);
    let vault_owner = pk(KeyPath::Coin(coin()));
    let vault = soltx::associated_token_address(&vault_owner, &WX);
    back_the_module(&mut w, 50 * SOL);
    w.deposit_wx(&vault_owner, VAULT);
    cs::with_mut(|s| {
        s.coins.insert(coin(), CoinRec { coll: 0, coin: coin(), pool: [9; 32], creator: [2; 32], launched_at: 1, seq: 0, stage: CoinStage::Backed,
            vault: Some(Vault { address: vault, owner: vault_owner, wx_in: VAULT, coins_burned: 0, wx_paid: 0, backed_at: 1 }), damm_pool: None, partner_nft: None, surplus_tries: 0 });
        s.next_seq = 1;
    });
    // intent 7 holds 80M coins (10 % of the supply): a tenth of the vault
    const AMOUNT: u64 = 80_000_000_000_000;
    let k_int = intent_key(7);
    plant_coins(&mut w, k_int, AMOUNT);
    let who = wallet(1);
    assert!(redeem::request(7, [1; 32], who, w.now).unwrap_err().contains("not a Rook coin"));
    redeem::request(7, coin(), who, w.now).unwrap();
    redeem::request(7, coin(), who, w.now).unwrap();
    assert_eq!(redeem::status(7).unwrap().state, RedeemState::Requested);
    let main_before = w.lamports(&me());
    // signed: the link begun, the coins' state Signed
    let log = w.coins_tick();
    let want = format!("redeem {AMOUNT} coins of {} for {} wX (intent 7, link 0)", soltx::b58_encode(&coin()), VAULT / 10);
    assert!(log.iter().any(|l| l.contains(&want) && l.contains("signed and sent")), "{log:?}");
    let r = redeem::status(7).unwrap();
    assert_eq!((r.state.clone(), r.amount, r.payout, r.tries), (RedeemState::Signed { link: 0 }, AMOUNT, VAULT / 10, 1));
    assert!(!r.signature.is_empty());
    assert_eq!(state::with(|s| s.link.get(0).map(|l| l.memo.clone())), Some("rook:v1:0:coin:0:i7".into()));
    // landed: the coins burned, the share paid from the vault, the deposit account closed to K_main
    w.settle();
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains(&want) && l.contains("landed")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("link 0 applied")), "{log:?}");
    let r = redeem::status(7).unwrap();
    assert_eq!(r.state, RedeemState::Landed);
    assert_eq!(coin_supply(&w), SUPPLY - AMOUNT);
    assert_eq!(coin_balance(&w, &k_int), None, "closed");
    assert_eq!(w.token(&who), VAULT / 10);
    assert_eq!(w.token(&vault_owner), VAULT - VAULT / 10);
    assert!(w.lamports(&me()) > main_before - RENT165 - 100_000, "the recipient's ATA rent came back from the closed deposit account");
    let v = rec().vault.unwrap();
    assert_eq!((v.coins_burned, v.wx_paid), (AMOUNT, VAULT / 10));
    assert!(cs::with(|s| s.receipts.is_empty()));
    assert_eq!(state::with(|s| s.link.get(0).map(|l| l.phase)), Some(etch_autobuyer::chainlink::ledger::Phase::Landed));
    // the books saw a link that moves no collection money: the backing is untouched
    assert_eq!(w.supply(), state::with(|s| s.books.get(COLL).unwrap().ledger.sol_backed));
    // a request with nothing in its deposit account fails, nothing signed; a request over the supply is refused by the quote
    plant_coins(&mut w, intent_key(8), 0);
    redeem::request(8, coin(), who, w.now).unwrap();
    let log = w.coins_tick();
    assert!(!log.iter().any(|l| l.contains("signed")), "{log:?}");
    assert_eq!(redeem::status(8).unwrap().state, RedeemState::Failed("no coins in the deposit account".into()));
    assert_eq!(cs::with(|s| s.signed_today), 1);
}

/// Audit B3-10: a pin whose curve DBC would refuse is refused at `register` (a falling point, an empty segment, a curve that cannot
/// raise the threshold), so it never takes an index in the unlock order; the pinned launch curve passes.
#[test]
fn fixed_b3_10_a_curve_dbc_would_refuse_is_refused_at_the_pin() {
    let mut w = World::new();
    let _ = w.coins_tick();
    let good = pin(None);
    let mut falling = good.clone();
    falling.curve[1].0 = falling.curve[0].0;
    assert!(open::register(falling).unwrap_err().contains("does not rise"));
    let mut empty = good.clone();
    empty.curve[0].1 = 0;
    assert!(open::register(empty).unwrap_err().contains("no liquidity"));
    let mut short = good.clone();
    short.t_wx = 1_000_000;
    assert!(open::register(short).unwrap_err().contains("under the threshold"));
    let mut start = good.clone();
    start.sqrt_start_price = good.curve[0].0;
    assert!(open::register(start).unwrap_err().contains("does not rise"));
    open::register(good).unwrap();
}

/// Audit B3-11: an exclusion from a redemption's supply is applied by the coins' pass a day after it is asked, not at once.
#[test]
fn fixed_b3_11_a_supply_exclusion_waits_its_day() {
    let k = [0x77u8; 32];
    cs::with_mut(|s| {
        s.exclude_from_supply.clear();
        s.pending_exclusions = vec![(k, true, 1_000 + 86_400)];
    });
    assert!(cs::with_mut(|s| s.apply_exclusions(1_000 + 86_399)).is_empty());
    assert!(cs::with(|s| s.exclude_from_supply.is_empty()), "not before its day");
    assert_eq!(cs::with_mut(|s| s.apply_exclusions(1_000 + 86_400)), vec![(k, true)]);
    assert_eq!(cs::with(|s| s.exclude_from_supply.clone()), vec![k]);
    assert!(cs::with(|s| s.pending_exclusions.is_empty()));
    // counted again, a day later too
    cs::with_mut(|s| s.pending_exclusions.push((k, false, 5)));
    cs::with_mut(|s| s.apply_exclusions(5));
    assert!(cs::with(|s| s.exclude_from_supply.is_empty()));
}

/// Collection `index` pinned with the fixture's curve under its own symbol (the canister's registry refuses a symbol twice).
fn pin_at(index: u32, symbol: &str) -> CollectionPin {
    CollectionPin { index, symbol: symbol.into(), slug: format!("c{index}"), ..pin(None) }
}

/// The collection whose opening step sits on the crank line, if any.
fn opening_on_the_line(w: &World) -> Option<u32> {
    match w.line(etch_autobuyer::solhome::LineId::Crank).pending.as_ref().map(|p| p.step.clone()) {
        Some(etch_autobuyer::solhome::LineStep::Coin(etch_autobuyer::coins::CoinStep::OpenMint { coll })) => Some(coll),
        _ => None,
    }
}

/// Operator decision 2026-10-12 ("we don't select, the people do"): every collection with a pipeline is pinned and none opens unasked; a
/// creator's `coins_request_open` puts it in a FIFO queue and the opening pass takes the oldest request, whatever its index.
#[test]
fn a_requested_collection_opens_next_whatever_its_index_and_requests_open_in_fifo_order() {
    let mut w = World::new();
    for (i, s) in [(1, "wONE"), (2, "wTWO"), (3, "wTHREE")] {
        open::register(pin_at(i, s)).unwrap();
    }
    // nothing requested, the switch off: nothing signs, the collections stay Pinned
    assert!(w.coins_tick().iter().all(|l| !l.contains("signed")));
    assert_eq!(opening_on_the_line(&w), None);
    assert!(etch_autobuyer::coins::api::collections().iter().all(|c| c.stage == "Pinned"));
    // two requests: 3 first, then 1; a second request of 3 is a no-op answer, not a second place in the queue
    assert!(open::request_open(3, &[1], w.now).unwrap().contains("opens next"));
    assert!(open::request_open(1, &[2], w.now + 1).unwrap().contains("after 1 other request"));
    assert!(open::request_open(3, &[3], w.now + 2).unwrap().contains("requested already"));
    assert_eq!(cs::with(|s| s.requested.clone()), vec![(3, w.now), (1, w.now + 1)]);
    assert_eq!(etch_autobuyer::coins::api::coins_log().3, vec![(3, w.now), (1, w.now + 1)], "coins_log lists the queue");
    let stages: Vec<(u32, String)> = etch_autobuyer::coins::api::collections().into_iter().map(|c| (c.index, c.stage)).collect();
    assert_eq!(stages, vec![(1, "Requested".to_string()), (2, "Pinned".to_string()), (3, "Requested".to_string())]);
    // the oldest request (3) opens first, though 1 and 2 have lower indices
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("a collection's opening step is signed")), "{log:?}");
    assert_eq!(opening_on_the_line(&w), Some(3));
    assert!(etch_autobuyer::coins::api::collections().iter().any(|c| c.index == 3 && c.stage == "Opening"));
    // FIFO: with 1 asked before 3, 1 opens first
    let mut w = World::new();
    for (i, s) in [(1, "wONE"), (3, "wTHREE")] {
        open::register(pin_at(i, s)).unwrap();
    }
    open::request_open(1, &[1], w.now).unwrap();
    open::request_open(3, &[2], w.now).unwrap();
    let _ = w.coins_tick();
    assert_eq!(opening_on_the_line(&w), Some(1));
}

#[test]
fn a_request_is_refused_for_an_unknown_open_or_refused_collection() {
    let mut w = World::new();
    assert!(open::request_open(7, &[1], w.now).unwrap_err().contains("not pinned"));
    open_on_the_worlds_mint(&mut w);
    assert!(open::request_open(COLL as u32, &[1], w.now).unwrap_err().contains("open already"));
    let _ = w.coins_tick();
    assert!(cs::with(|s| s.requested.is_empty()), "the opened collection left the queue");
    open::register(pin_at(4, "wFOUR")).unwrap();
    cs::with_mut(|s| {
        s.cols.insert(4, cs::ColRec { index: 4, stage: ColStage::Refused("its config differs".into()), wx_mint: [0; 32], config: [0; 32], opened_at: 0, checked_at: 0, module: [0; 32] });
    });
    assert!(open::request_open(4, &[1], w.now).unwrap_err().contains("refused"));
}

/// The operator's switch (`coins_set_auto_open`, default off) restores the earlier index-order opening for the float left over.
#[test]
fn with_the_auto_open_switch_on_the_pinned_collections_open_in_index_order() {
    let mut w = World::new();
    assert!(!cs::with(|s| s.auto_open), "off by default");
    for (i, s) in [(5, "wFIVE"), (2, "wTWO")] {
        open::register(pin_at(i, s)).unwrap();
    }
    assert!(w.coins_tick().iter().all(|l| !l.contains("signed")), "off: nothing opens unasked");
    open::set_auto_open(true, w.now);
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("a collection's opening step is signed")), "{log:?}");
    assert_eq!(opening_on_the_line(&w), Some(2), "the lowest pinned index first");
    // a request still goes before the index order
    let mut w = World::new();
    for (i, s) in [(5, "wFIVE"), (2, "wTWO")] {
        open::register(pin_at(i, s)).unwrap();
    }
    open::set_auto_open(true, w.now);
    open::request_open(5, &[1], w.now).unwrap();
    let _ = w.coins_tick();
    assert_eq!(opening_on_the_line(&w), Some(5));
}
