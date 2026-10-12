//! Audit round C, part C4 (docs/v9/audit-2026-10/c4-opening-runner.md): opening on demand. The PoCs that passed by exhibiting C4-01,
//! C4-02 and C4-04 are inverted into `fixed_` tests (2026-10-12), beside the regressions of C4-03, C4-05, C4-11 and C4-13. The test
//! chain refuses `CreateAccountWithSeed` on an address that holds lamports, as Solana does (C4-13), so C4-01's fix is tested against
//! the real rule.
mod desk_common;
mod fakerpc;

use std::collections::BTreeMap;

use desk_common::*;
use etch_autobuyer::books;
use etch_autobuyer::books::ListingKind;
use etch_autobuyer::coins::state::{self as cs, ColStage, CollectionPin, OPENS_PER_DAY};
use etch_autobuyer::coins::{api, open, run, unlock, CoinStep};
use etch_autobuyer::solix2;
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::solrpc::Account;
use etch_autobuyer::soltx::{self, SYSTEM_PROGRAM};
use serde_json::Value;

fn flow() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solix2/flow.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn n(v: &Value) -> u128 {
    match v {
        Value::Number(x) => x.as_u64().unwrap() as u128,
        Value::String(s) => s.parse().unwrap(),
        _ => panic!("a number, got {v}"),
    }
}

/// A pin as `launch_pins` writes it (`wx_mint: None, config: None`: the canister makes both), with the fixture's curve (`T_wX` = 5).
fn pin_at(index: u32, symbol: &str) -> CollectionPin {
    let f = flow();
    let cc = f["instructions"].as_array().unwrap().iter().find(|i| i["name"] == "dbc.create_config").unwrap()["params"].clone();
    CollectionPin {
        index,
        slug: format!("c{index}"),
        name: format!("Wrapped C{index}"),
        symbol: symbol.into(),
        uri: format!("https://rook.example/wx/c{index}.json"),
        t_wx: 5,
        sqrt_start_price: n(&cc["sqrt_start_price"]),
        curve: cc["curve"].as_array().unwrap().iter().map(|p| (n(&p[0]), n(&p[1]))).collect(),
        kind: ListingKind::EthNft { chain_id: 1, collection: [0x5a; 20], custody: [0x11; 20] },
        params: books::default_params(20 * SOL),
        wx_mint: None,
        config: None,
    }
}

fn stage(i: u32) -> Option<ColStage> {
    cs::with(|s| s.cols.get(&i).map(|c| c.stage.clone()))
}

fn on_the_line(w: &World) -> Option<CoinStep> {
    match w.line(etch_autobuyer::solhome::LineId::Crank).pending.as_ref().map(|p| p.step.clone()) {
        Some(etch_autobuyer::solhome::LineStep::Coin(s)) => Some(s),
        _ => None,
    }
}

fn opening_on_the_line(w: &World) -> Option<u32> {
    match on_the_line(w) {
        Some(CoinStep::OpenMint { coll }) => Some(coll),
        _ => None,
    }
}

/// A caller's principal bytes (a named one).
fn caller(n: u8) -> Vec<u8> {
    vec![n; 29]
}

/// C4-13: the test chain refuses `CreateAccountWithSeed` on an address that holds lamports (`AccountAlreadyInUse`, as Solana's
/// `create_account`), and lets the base allocate it in place (`AllocateWithSeed`), which nobody else can.
#[test]
fn fixed_c4_13_the_test_chain_refuses_create_with_seed_on_a_funded_address_and_allocates_it_in_place() {
    let main = me();
    let mint = open::wx_mint_of(&main, 7);
    let tok = solix2::token_program();
    let fresh = || {
        let mut acc: BTreeMap<[u8; 32], Account> = BTreeMap::new();
        acc.insert(main, Account { lamports: SOL, owner: SYSTEM_PROGRAM, executable: false, data: vec![] });
        acc.insert(mint, Account { lamports: RENT0, owner: SYSTEM_PROGRAM, executable: false, data: vec![] });
        acc
    };
    let mut acc = fresh();
    let create = soltx::create_with_seed(main, mint, main, &open::mint_seed(7), 1_461_600, 82, tok);
    let e = run_ixs(&[create], &mut acc).unwrap_err();
    assert!(e.to_string().contains("AccountAlreadyInUse"), "{e}");
    // the base allocates over it, and tops it up
    let mut acc = fresh();
    run_ixs(&[soltx::allocate_with_seed(mint, main, &open::mint_seed(7), 82, tok), soltx::transfer(main, mint, 1_461_600 - RENT0)], &mut acc).unwrap();
    assert_eq!((acc[&mint].owner, acc[&mint].data.len(), acc[&mint].lamports), (tok, 82, 1_461_600));
    // nobody else can: an unsigned base is refused
    let mut acc = fresh();
    let mut ix = soltx::allocate_with_seed(mint, main, &open::mint_seed(7), 82, tok);
    ix.accounts[1].signer = false;
    assert!(run_ixs(&[ix], &mut acc).unwrap_err().to_string().contains("did not sign"));
}

/// C4-01 (High), fixed: a System transfer of `RENT0` to a collection's derived wX mint address no longer refuses it. The pass reads a
/// System-owned, data-less account there as "not made yet" and makes the mint over it (`AllocateWithSeed` + a top-up + `InitializeMint2`,
/// all signed by `K_main`), so the attacker's lamports become part of the mint's rent; the collection opens.
#[test]
fn fixed_c4_01_a_tiny_transfer_to_the_derived_wx_mint_address_no_longer_refuses_the_collection() {
    let mut w = World::new();
    for (i, s) in [(1, "wONE"), (2, "wTWO")] {
        open::register(pin_at(i, s)).unwrap();
    }
    for i in [1u32, 2] {
        w.plant(open::wx_mint_of(&me(), i), SYSTEM_PROGRAM, RENT0, vec![]);
    }
    open::request_open(1, &caller(1), w.now).unwrap();
    open::request_open(2, &caller(2), w.now).unwrap();
    let main_before = w.lamports(&me());
    let log = w.coins_tick();
    assert!(log.iter().all(|l| !l.contains("REFUSED")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("a collection's opening step is signed")), "{log:?}");
    assert_eq!(opening_on_the_line(&w), Some(1));
    w.settle();
    let mint = open::wx_mint_of(&me(), 1);
    let m = w.chain.borrow().accounts[&mint].clone();
    assert_eq!((m.owner, m.data.len(), m.lamports), (solix2::token_program(), 82, unlock::MINT_RENT), "the mint, made over the squatter's lamports");
    assert_eq!(m.data[4..36], pk(KeyPath::Coll(1)), "its mint authority is K_coll[1]");
    assert!(w.lamports(&me()) <= main_before - (unlock::MINT_RENT - RENT0), "K_main topped the rent up");
    assert!(matches!(stage(1), Some(ColStage::Opening)));
    assert!(open::request_open(2, &caller(3), w.now).unwrap().contains("requested already"), "collection 2 still waits its turn, not refused");
}

/// C4-01, the costly variant, fixed: the config's derived address pre-funded between the two steps is initialised over (DBC's
/// `create_config` is an Anchor `init`, which tops up, allocates and assigns an account holding lamports): the pass signs the config
/// step, it does not refuse the collection after the float paid its mint.
#[test]
fn fixed_c4_01_a_prefunded_config_address_is_made_over_not_refused() {
    let mut w = World::new();
    open::register(pin_at(COLL as u32, "wBOREDAPE")).unwrap();
    open::request_open(COLL as u32, &caller(1), w.now).unwrap();
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("a collection's opening step is signed")), "{log:?}");
    w.settle();
    let mint = open::wx_mint_of(&me(), COLL as u32);
    let mut d = vec![0u8; 679];
    d[0] = 4;
    d[1..33].copy_from_slice(&module());
    d[33..65].copy_from_slice(&mint);
    w.plant(solix2::metadata_address(&mint), solix2::id(solix2::METADATA_PROGRAM), unlock::METADATA_RENT, d);
    // the attacker, between the two steps
    w.plant(pk(KeyPath::Config(COLL as u32)), SYSTEM_PROGRAM, RENT0, vec![]);
    let mut log = w.coins_tick();
    w.advance_time(run::OPEN_EVERY);
    log.extend(w.coins_tick());
    assert!(log.iter().all(|l| !l.contains("REFUSED")), "{log:?}");
    assert_eq!(on_the_line(&w), Some(CoinStep::OpenConfig { coll: COLL as u32 }), "{log:?}");
    assert!(matches!(stage(0), Some(ColStage::Opening)));
}

/// C4-01, the operator's way out: a refused collection (here a mint at its derived address owned by another program, which the
/// opening still refuses) is queued again under its own index and symbol by `coins_retry` once the cause is gone, and opens.
#[test]
fn fixed_c4_01_the_operator_retries_a_refused_collection_under_its_own_index() {
    let mut w = World::new();
    open::register(pin_at(1, "wONE")).unwrap();
    let mint = open::wx_mint_of(&me(), 1);
    w.plant(mint, [0x77; 32], RENT0, vec![1, 2, 3]);
    open::request_open(1, &caller(1), w.now).unwrap();
    let log = w.coins_tick();
    assert!(log.iter().any(|l| l.contains("collection 1 REFUSED")), "{log:?}");
    assert!(open::request_open(1, &caller(2), w.now).unwrap_err().contains("refused"));
    assert!(open::retry_refused(2, w.now).unwrap_err().contains("never looked at"));
    // the cause goes (in the test: the chain forgets the account), the operator retries
    w.chain.borrow_mut().accounts.remove(&mint);
    assert!(open::retry_refused(1, w.now).unwrap().contains("queued to open"));
    assert!(open::retry_refused(1, w.now).unwrap_err().contains("not refused"));
    assert_eq!(stage(1), Some(ColStage::Pinned));
    w.advance_time(run::OPEN_EVERY);
    let _ = w.coins_tick();
    assert_eq!(opening_on_the_line(&w), Some(1), "the same index, the same symbol");
}

/// C4-02 (Medium), fixed. The rule: the queue holds at most `MAX_QUEUED` (a day of openings) pending requests; at most
/// `REQUESTS_PER_HOUR` are accepted an hour from everyone, `ANON_REQUESTS_PER_HOUR` of them anonymous (the site's visitors, one
/// caller); a named principal holds one pending request at a time and `CALLER_REQUESTS_PER_HOUR` an hour. A script no longer queues
/// every pinned collection: a creator who asks the next hour waits behind at most the hour's few requests, and the answer says the pace.
#[test]
fn fixed_c4_02_one_caller_no_longer_queues_every_pinned_collection() {
    let mut w = World::new();
    for i in 1..=24u32 {
        open::register(pin_at(i, &format!("wC{i}"))).unwrap();
    }
    // anonymous: two an hour
    open::request_open(24, open::ANONYMOUS, w.now).unwrap();
    open::request_open(23, open::ANONYMOUS, w.now).unwrap();
    assert!(open::request_open(22, open::ANONYMOUS, w.now).unwrap_err().contains("anonymous"));
    // a named caller: one pending request
    open::request_open(21, &caller(9), w.now).unwrap();
    assert!(open::request_open(20, &caller(9), w.now).unwrap_err().contains("one pending request"));
    // fresh principals (free to make): the hour's overall bound
    open::request_open(19, &caller(10), w.now).unwrap();
    assert!(open::request_open(18, &caller(11), w.now).unwrap_err().contains("this hour"));
    assert_eq!(cs::with(|s| s.requested.len()), open::REQUESTS_PER_HOUR);
    // the creator, the next hour: behind the hour's four, and told the pace
    w.advance_time(3_600);
    let r = open::request_open(1, &caller(12), w.now).unwrap();
    assert!(r.contains("after 4 other request(s)") && r.contains(&format!("at most {OPENS_PER_DAY} open a day")), "{r}");
    // the queue never holds more than a day of openings, whoever asks
    for k in 0..3u8 {
        open::request_open(2 + k as u32, &caller(20 + k), w.now).unwrap();
    }
    w.advance_time(3_600);
    assert!(open::request_open(10, &caller(30), w.now).unwrap_err().contains("queued to open already"));
    assert_eq!(cs::with(|s| s.requested.len()), open::MAX_QUEUED);
}

/// C4-03 (canister side), fixed: a Robinhood Chain pin opens only once chain 4663 is in the install's `evm_chains` (the ceremony's
/// read-back); before, a request is refused and the auto-open switch passes over it. An Ethereum or Inscriptions pin needs nothing more.
#[test]
fn fixed_c4_03_a_robinhood_pin_opens_only_once_its_chain_is_installed() {
    let mut w = World::new();
    let rh = |i, s: &str| CollectionPin { kind: ListingKind::EthNft { chain_id: 4663, collection: [0x46; 20], custody: [0x11; 20] }, ..pin_at(i, s) };
    open::register(rh(1, "wRH")).unwrap();
    open::register(pin_at(2, "wTWO")).unwrap();
    let e = open::request_open(1, &caller(1), w.now).unwrap_err();
    assert!(e.contains("cannot open yet") && e.contains("evm_chains") && e.contains("robinhood"), "{e}");
    assert!(open::request_open_with(1, &caller(1), &[1, 4663], w.now).unwrap().contains("requested"));
    // the readiness itself
    assert!(open::pipeline_ready(&ListingKind::EthNft { chain_id: 1, collection: [0; 20], custody: [0; 20] }, &[]).is_ok());
    assert!(open::pipeline_ready(&ListingKind::Inscriptions { membership_root: [0; 32], custody: "bc1p".into() }, &[]).is_ok());
    assert!(open::pipeline_ready(&ListingKind::EthNft { chain_id: 4663, collection: [0; 20], custody: [0; 20] }, &[1]).is_err());
    // the auto-open switch passes over the Robinhood pin (index 1) while its chain is not installed: index 2 opens
    let mut w2 = World::new();
    open::register(rh(1, "wRH")).unwrap();
    open::register(pin_at(2, "wTWO")).unwrap();
    open::set_auto_open(true, w2.now);
    let _ = w2.coins_tick();
    assert_eq!(opening_on_the_line(&w2), Some(2));
    let _ = &mut w;
}

/// C4-04 (Low), fixed: a requested collection the pass waits on for a reason of its own (a pinned mint the chain lacks) is passed over
/// after `HEAD_PATIENCE`: collection 2 starts opening within the hour, and collection 1 keeps its place at the queue's head, shown as
/// skipped with the reason (`coins_open_queue`, C4-12).
#[test]
fn fixed_c4_04_a_waiting_head_no_longer_holds_every_later_request() {
    let mut w = World::new();
    open::register(CollectionPin { wx_mint: Some([9; 32]), ..pin_at(1, "wONE") }).unwrap();
    open::register(pin_at(2, "wTWO")).unwrap();
    open::request_open(1, &caller(1), w.now).unwrap();
    open::request_open(2, &caller(2), w.now).unwrap();
    let start = w.now;
    let mut signed_at = None;
    for _ in 0..30 {
        let log = w.coins_tick();
        if log.iter().any(|l| l.contains("signed")) {
            signed_at = Some(w.now);
            break;
        }
        w.advance_time(run::OPEN_EVERY);
    }
    let at = signed_at.expect("collection 2 starts within the hour");
    assert!(at - start >= open::HEAD_PATIENCE, "the head had its patience first");
    assert_eq!(opening_on_the_line(&w), Some(2));
    assert_eq!(cs::with(|s| s.requested.clone()).iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![1, 2], "the head keeps its place");
    let q = api::open_queue(w.now);
    assert_eq!((q[0].index, q[0].position, q[0].skipped, q[0].waiting_since), (1, 0, true, start));
    assert!(q[0].why.contains("does not exist yet"), "{:?}", q[0]);
    assert_eq!((q[1].index, q[1].skipped), (2, false));
}

/// C4-05, fixed: `inspect_message` accepts a stranger's `coins_request_open` only when it would queue the collection: a call that
/// would change nothing (not pinned, already requested, open, refused, not ready, over the limits, a bad argument) is never accepted.
#[test]
fn fixed_c4_05_only_a_request_that_would_queue_is_accepted_as_ingress() {
    let w = World::new();
    open::register(pin_at(1, "wONE")).unwrap();
    let arg = |i: u32| candid::encode_one(i).unwrap();
    assert!(api::may_request(&arg(1), open::ANONYMOUS, &[], w.now));
    assert!(!api::may_request(&arg(9), open::ANONYMOUS, &[], w.now), "not pinned");
    assert!(!api::may_request(b"not candid", open::ANONYMOUS, &[], w.now), "a bad argument");
    open::request_open(1, open::ANONYMOUS, w.now).unwrap();
    assert!(!api::may_request(&arg(1), &caller(5), &[], w.now), "requested already: a no-op");
    open::register(pin_at(2, "wTWO")).unwrap();
    open::register(pin_at(3, "wTHREE")).unwrap();
    open::request_open(2, open::ANONYMOUS, w.now).unwrap();
    assert!(!api::may_request(&arg(3), open::ANONYMOUS, &[], w.now), "over the anonymous limit");
    assert!(api::may_request(&arg(3), &caller(5), &[], w.now), "a named caller still may");
}

/// C4-11, fixed: a request is refused while the coins module is off (nothing would open; the queue would fire at once on the switch).
#[test]
fn fixed_c4_11_a_request_is_refused_while_the_coins_module_is_off() {
    let w = World::new();
    open::register(pin_at(1, "wONE")).unwrap();
    cs::with_mut(|s| s.off = true);
    assert!(open::request_open(1, &caller(1), w.now).unwrap_err().contains("module is off"));
    assert!(!api::may_request(&candid::encode_one(1u32).unwrap(), &caller(1), &[], w.now));
    cs::with_mut(|s| s.off = false);
    open::request_open(1, &caller(1), w.now).unwrap();
    // and the switch `coins_set_auto_open(true)` waits for the pause (canister.rs's `every_operator_method_that_acts_refuses_while_paused`)
    let src = include_str!("../src/coins/api.rs");
    let body = src.split("pub fn coins_set_auto_open").nth(1).unwrap().split("\n}\n").next().unwrap();
    assert!(body.contains("if on { crate::canister::not_paused()?; }"), "{body}");
}
