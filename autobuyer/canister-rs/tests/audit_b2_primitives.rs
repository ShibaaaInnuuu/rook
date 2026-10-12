//! Audit B2 (docs/v9/audit-2026-10/b2-primitives.md): proofs of concept, pure (no network, no keys). Each test PASSES by exhibiting
//! the bug it names, so the suite stays green until the fix lands; the comment on each says what a fix must refuse. The LiteSVM
//! counterpart of B2-01 (the fee really paid on the program's mainnet bytes) is `autobuyer/solnft-svm/tests/audit_b2_judge.rs`.
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use base64::Engine;
use ed25519_dalek::SigningKey;
use etch_autobuyer::solhome::{self, LineId, LineOwner, SolIo, SolLine, TURN_SECS};
use etch_autobuyer::solix2::{self, decode, guard, mpl, spl};
use etch_autobuyer::solnft::{self, Accounts, Standard};
use etch_autobuyer::solnftbuy::{self as buy, Candidate, Expect, Venue};
use etch_autobuyer::solnftm2 as m2;
use etch_autobuyer::solnftsend as send;
use etch_autobuyer::solrpc::Account;
use etch_autobuyer::soltimer::{self, Run};
use etch_autobuyer::soltx::{self, Borsh, Instruction, Pubkey};
use serde_json::Value;

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn fixture(dir: &str, name: &str) -> Value {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(dir).join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}
fn pk(v: &Value) -> Pubkey {
    soltx::pubkey(v.as_str().unwrap_or_else(|| panic!("a key, got {v}"))).unwrap()
}
fn b64(v: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(v.as_str().unwrap()).unwrap()
}

/// A recorded Solana NFT scenario (`tests/fixtures/solnft`), as the canister reads it at its slot.
struct Nft {
    j: Value,
    view: Accounts,
}
fn nft(name: &str) -> Nft {
    let j = fixture("solnft", name);
    let mut view: Accounts = j["accounts"].as_array().unwrap().iter().map(|a| {
        (pk(&a["pubkey"]), Some(Account { lamports: a["lamports"].as_u64().unwrap(), owner: pk(&a["owner"]), executable: a["executable"].as_bool().unwrap(),
            data: b64(&a["data"]) }))
    }).collect();
    for k in j["absent"].as_array().unwrap() {
        view.insert(pk(k), None);
    }
    Nft { j, view }
}
impl Nft {
    fn fact(&self, k: &str) -> Pubkey {
        pk(&self.j["facts"][k])
    }
    fn now(&self) -> i64 {
        self.j["unix_time"].as_i64().unwrap()
    }
}

/// A wallet on the ed25519 curve (a person's): what a delivery may go to.
fn wallet(seed: u8) -> Pubkey {
    SigningKey::from_bytes(&[seed; 32]).verifying_key().to_bytes()
}

const ME: Pubkey = [0x51; 32];

// ---- B2-01: the judges let a compute-unit PRICE through ---------------------------------------------------------------------------------

/// B2-01. `solnftbuy::judge` (J1), `solnftm2::judge` (J1) and `solnftsend::judge_send` skip every Compute Budget instruction unread. A
/// `SetComputeUnitPrice` is one: its fee, `limit × price / 10⁶` lamports, is paid by the fee payer (`K_main`) on top of everything J3
/// bounds, and is not in `limit`. Any transaction the judge is given from outside (the co-signed marketplace path the module header
/// names) can carry it. A fix must accept `SetComputeUnitLimit` only, or pin the price and add its fee to J3's `all_in`.
#[test]
fn fixed_b2_01_every_nft_judge_refuses_a_compute_unit_price_and_passes_the_unit_limit() {
    let huge = soltx::compute_unit_price(u64::MAX);
    // Tensor (legacy): the purchase with a price of u64::MAX micro-lamports a unit
    let f = nft("tensor_legacy_fff");
    let (asset, coll) = (f.fact("asset"), f.fact("collection"));
    let m = solnft::is_member(&asset, &coll, Standard::Legacy, &f.view).unwrap();
    let ask = buy::verify_ask(&Candidate { venue: Venue::Tensor, listing: f.fact("list_state"), asset }, &m, &f.view, f.now()).unwrap();
    let limit = ask.total + ask.rent(&m);
    let mut ixs = vec![huge.clone()];
    ixs.extend(buy::buy_instructions(&ME, &ask, &m).unwrap());
    let e = Expect { payer: ME, ask: &ask, member: &m, collection: coll, limit };
    let e1 = buy::judge(&ixs, &e).expect_err("FIXED B2-01: the Tensor judge refuses a compute-unit price");
    assert!(e1.contains("SetComputeUnitPrice"), "{e1}");
    buy::judge(&buy::buy_instructions(&ME, &ask, &m).unwrap(), &e).expect("the canonical purchase (with its SetComputeUnitLimit) passes");
    assert!(buy::judge(&buy::buy_instructions(&ME, &ask, &m).unwrap(), &Expect { limit: limit - 1, ..e }).is_err(), "J3 still bounds the purchase to the lamport");
    assert!(buy::judge(&[soltx::Instruction { program: soltx::key(soltx::COMPUTE_BUDGET_PROGRAM), accounts: vec![], data: vec![1, 0, 0, 0, 0] }], &e).unwrap_err().contains("other than SetComputeUnitLimit"), "a heap frame is refused too");
    // the delivery of what was bought
    let mut d = vec![huge.clone()];
    d.extend(send::send_instructions(&ME, &wallet(9), &m).unwrap());
    assert!(send::judge_send(&d, &ME, &wallet(9), &m).unwrap_err().contains("SetComputeUnitPrice"), "FIXED B2-01: judge_send refuses it");
    send::judge_send(&send::send_instructions(&ME, &wallet(9), &m).unwrap(), &ME, &wallet(9), &m).expect("the canonical delivery passes");
    // Magic Eden M2 (pNFT)
    let f = nft("m2_mad_lads_4vCV2b");
    let (asset, coll) = (f.fact("asset"), f.fact("collection"));
    let m = solnft::is_member(&asset, &coll, Standard::Legacy, &f.view).unwrap();
    let ask = buy::verify_ask(&Candidate { venue: Venue::M2, listing: f.fact("seller_trade_state"), asset }, &m, &f.view, f.now()).unwrap();
    let limit = m2::m2_total(ask.price, &m) + ask.rent(&m);
    let mut ixs = vec![huge];
    ixs.extend(m2::buy_instructions(&ME, &ask, &m).unwrap());
    assert!(m2::judge(&ixs, &Expect { payer: ME, ask: &ask, member: &m, collection: coll, limit }).unwrap_err().contains("SetComputeUnitPrice"), "FIXED B2-01: the M2 judge refuses it too");
    m2::judge(&m2::buy_instructions(&ME, &ask, &m).unwrap(), &Expect { payer: ME, ask: &ask, member: &m, collection: coll, limit }).expect("the canonical M2 purchase passes");
}

// ---- B2-02: the Metaplex "rename only" rule reads three trailing bytes ---------------------------------------------------------------

/// `UpdateMetadataAccountV2 { data: Option<DataV2>, update_authority: Option<Pubkey>, primary_sale_happened: Option<bool>,
/// is_mutable: Option<bool> }`, Borsh.
fn update_metadata_moving_the_authority_to(new_authority: Pubkey) -> Instruction {
    let d = Borsh(vec![mpl::UPDATE_METADATA_ACCOUNT_V2, 1]).string("wX").string("WX").string("https://rook.example/wx.json").u16(0).u8(0).u8(0).u8(0)
        .u8(1).bytes(&new_authority) // update_authority: Some(new)
        .u8(0) // primary_sale_happened: None
        .u8(0) // is_mutable: None
        .done();
    Instruction { program: solix2::id(solix2::METADATA_PROGRAM), accounts: vec![soltx::meta([0x33; 32], false, true), soltx::meta([0x44; 32], true, false)], data: d }
}

/// B2-02. `guard::refuse_forbidden` passes an `UpdateMetadataAccountV2` as "rename only" when its LAST THREE bytes are zero. The
/// three trailing `Option`s are `None` only when the byte before them is the end of the `data` option; a `Some(update_authority)` whose
/// key's last byte is 0x00 (one key in 256) is followed by `None, None` and ends in `[0, 0, 0]` as well. So an instruction that hands
/// the wX metadata's update authority to another key passes the guard. A fix must parse the argument (after `Option<DataV2>`, the next
/// byte must be 0) instead of looking at the tail.
#[test]
fn fixed_b2_02_refuse_forbidden_refuses_a_metadata_update_that_moves_the_update_authority() {
    let mut new_authority = [0xAB; 32];
    new_authority[31] = 0;
    let ix = update_metadata_moving_the_authority_to(new_authority);
    assert!(guard::refuse_forbidden(&[ix]).is_err(), "FIXED B2-02: the guard parses the argument: a moved update authority is refused whatever its key's last byte");
    // the same update with a key that does not end in 0x00 is refused: the rule depends on the key's last byte, not on the argument
    new_authority[31] = 1;
    assert!(guard::refuse_forbidden(&[update_metadata_moving_the_authority_to(new_authority)]).is_err());
    // what the canister builds itself is a rename and passes
    guard::refuse_forbidden(&[mpl::update_metadata_v2([0x33; 32], [0x44; 32], "wX", "WX", "u")]).unwrap();
    // and SetAuthority is refused as documented
    assert!(guard::refuse_forbidden(&[spl::set_authority([1; 32], spl::AuthorityType::MintTokens, Some([2; 32]), [3; 32])]).is_err());
}

// ---- B2-03: a stale place in a line's queue keeps the timer busy ----------------------------------------------------------------------

/// The four lines, nothing else (every other `SolIo` read answers "not configured", which `tick_lines` never asks for an idle line).
struct Lines(BTreeMap<LineId, SolLine>);
impl SolIo for Lines {
    fn sol_line(&mut self) -> SolLine {
        self.sol_line_of(LineId::Money)
    }
    fn sol_line_save(&mut self, l: &SolLine) {
        self.sol_line_save_of(LineId::Money, l)
    }
    fn sol_line_of(&mut self, id: LineId) -> SolLine {
        self.0.get(&id).cloned().unwrap_or_default()
    }
    fn sol_line_save_of(&mut self, id: LineId, l: &SolLine) {
        self.0.insert(id, l.clone());
    }
}

/// B2-03. `SolLine::claim` drops a turn older than `TURN_SECS` (1 h), but only when some owner CLAIMS that line again. `tick_lines`
/// reports a line busy while its queue is non-empty, without that pruning, and the timer takes the busy cadence (every 10 s instead
/// of 60) on it: every run then ticks the desks, the coins, the pool desk and the cranks six times as often, for as long as the stale
/// entry stays. An owner that asked once while the line was busy and never asked again (nor released) leaves it there for good. A fix
/// must prune the queue by `TURN_SECS` in `tick_lines` (or count only fresh entries as work).
#[test]
fn fixed_b2_03_a_stale_queue_entry_is_dropped_by_the_timers_view_after_an_hour() {
    let mut io = Lines(BTreeMap::new());
    let mut money = SolLine::default();
    // the Ethereum side asked at t = 0 while the line was busy; the line is free now and nothing of it is out
    money.queue.push((LineOwner::Eth, 0));
    io.sol_line_save_of(LineId::Money, &money);
    assert!(!io.sol_line_of(LineId::Money).busy(), "nothing is out on the line");
    // within the hour the turn is fresh: the busy cadence
    let (out, _, busy) = block_on(solhome::tick_lines(&mut io, None, TURN_SECS));
    assert!(out.is_empty() && busy);
    assert_eq!(soltimer::next_delay(Run::Ran { busy }), soltimer::BUSY_SECS);
    // past it the turn is dropped by the timer's own pass (FIXED B2-03): the idle cadence, and the queue is empty for good
    let (out, _, busy) = block_on(solhome::tick_lines(&mut io, None, TURN_SECS + 1));
    assert!(out.is_empty() && !busy, "a stale turn is not work");
    assert!(io.sol_line_of(LineId::Money).queue.is_empty());
    assert_ne!(soltimer::next_delay(Run::Ran { busy }), soltimer::BUSY_SECS);
}

// ---- B2-04: the DBC config check leaves two migrated-pool fields unpinned --------------------------------------------------------------

/// B2-04. `guard::check_config` decodes `migrated_pool_base_fee_mode` (byte 364) and `enable_first_swap_with_min_fee` (byte 365) but
/// never compares them with Rook's pins (`rook_wx_config`: 0 and, since 2026-10-11, true), and does not decode the migrated pool's market-cap fee
/// scheduler at all. A config whose migrated COIN/wX pool runs a rate-limiter fee mode or a first-swap-at-minimum-fee is accepted as
/// Rook's. A fix must pin both (and the scheduler's bytes) to the values `rook_wx_config` creates.
#[test]
fn fixed_b2_04_check_config_refuses_a_migrated_pool_fee_mode_rook_never_pinned() {
    let f = fixture("solix2", "flow");
    let a = f["accounts"].as_array().unwrap().iter().find(|a| a["name"] == "dbc_config").unwrap();
    let (owner, mut data) = (pk(&a["owner"]), b64(&a["data"]));
    let cc = &f["instructions"].as_array().unwrap().iter().find(|i| i["name"] == "dbc.create_config").unwrap()["params"];
    let n = |v: &Value| -> u128 { match v { Value::Number(x) => x.as_u64().unwrap() as u128, Value::String(s) => s.parse().unwrap(), _ => panic!() } };
    let pin = guard::ConfigPin { wx_mint: pk(&cc["quote_mint"]), k_fc: pk(&cc["fee_claimer"]), t_wx: n(&cc["t_wx"]) as u64, sqrt_start_price: n(&cc["sqrt_start_price"]),
        curve: cc["curve"].as_array().unwrap().iter().map(|p| (n(&p[0]), n(&p[1]))).collect() };
    let c = decode::DbcConfig::decode(&owner, &data).unwrap();
    guard::check_config(&c, &pin).unwrap();
    // since 2026-10-11 Rook's config turns the first swap at the minimum fee ON (the creator's launch buy, `pins::FIRST_SWAP_WITH_MIN_FEE`)
    assert_eq!((c.migrated_pool_base_fee_mode, c.enable_first_swap_with_min_fee), (0, 1), "the config as Rook created it");
    // the same config with a rate-limiter fee mode on the migrated pool
    let mut m = data.clone();
    m[364] = 1;
    let x = decode::DbcConfig::decode(&owner, &m).unwrap();
    assert_eq!(x.migrated_pool_base_fee_mode, 1, "decoded, so the byte is the one");
    let e = guard::check_config(&x, &pin).expect_err("FIXED B2-04: the migrated pool's fee mode is pinned");
    assert!(e.contains("migrated pool fee is not Rook's"), "{e}");
    // and the same config with the first swap at the minimum fee OFF: not the one Rook creates either
    data[365] = 0;
    let y = decode::DbcConfig::decode(&owner, &data).unwrap();
    assert_eq!(y.enable_first_swap_with_min_fee, 0);
    let e = guard::check_config(&y, &pin).expect_err("FIXED B2-04: the first-swap flag is pinned");
    assert!(e.contains("enable_first_swap_with_min_fee 0 (want 1)"), "{e}");
}
