//! Audit round C, part C2: the Ordinals multi-collection change (docs/v9/audit-2026-10/c2-ordinals-multi.md). Each test here PASSED by
//! exhibiting a finding; its comment says what a fix must refuse. FIXED (2026-10-12): each is inverted into a `fixed_` regression, and
//! the fix round's further regressions follow (C2-12's first floor in the world, default-live).
//!
//! The world is `tests/frogs_sol.rs`'s books-home world (the frog flow against the books' Inscriptions listings, the fake Solana chain
//! and Bitcoin), copied here unchanged (a test file cannot reach another's private items), with its `books_two` second collection
//! (NodeMonkes, listing 12, its real members root from `tests/fixtures/ordinals/multi.json`).
#![allow(dead_code)]
mod fakerpc;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::btcnet::BtcSeen;
use etch_autobuyer::btcsend::parse_signed;
use etch_autobuyer::custody::{Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::delivery::TicketRead;
use etch_autobuyer::deposits::TxView;
use etch_autobuyer::evmtx::keccak;
use etch_autobuyer::floortrigger::{FloorCfg, FloorItem, FloorState};
use etch_autobuyer::frogbuy::{self, FPhase};
use etch_autobuyer::frogflow::{self, FStep, FrogIo, FrogState, Offer, ProtectedOffer};
use etch_autobuyer::frogsol;
use etch_autobuyer::itemsink::{Clock, SinkKind, SinkRec};
use etch_autobuyer::looper::{BtcView, EvmView};
use etch_autobuyer::psbt::{Psbt, Tx, TxIn, TxOut};
use etch_autobuyer::redeem::{self, Custody, CustodyIo, Redemption};
use etch_autobuyer::relayconv::{self as rc, CPhase};
use etch_autobuyer::relaysol;
use etch_autobuyer::solkey::SolSigner as _;
use etch_autobuyer::solhome::{self, LineStep, SolHomeCfg, SolIo, SolLine};
use etch_autobuyer::solrpc::{self, Account, Level, SigInfo};
use etch_autobuyer::solsend::{self, LifetimeKind, Line};
use etch_autobuyer::soltx::{self, Instruction, Pubkey};
use etch_autobuyer::{custodycap, outbound, outflow, rates};
use fakerpc::sol::{pair, FakeSigner, FakeSol, SolChain};
use fakerpc::sys::{self, SysWorld, ONE};
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

/// The real SOL → BTC quote of 2026-10-09 for 5 SOL (spike Q5): its `user` and refund, our Solana key; its recipient, our float.
const SOL_BTC_5: &str = include_str!("fixtures/relay-sol/quotes/sol-btc-5.quote.json");
const ME_SOL: &str = "5ecsyZA5SVEuTByKoDCUivha8uzAgeQJmgFnnvokFF21";

/// Audit A5-03: the quote as Relay answers an ask whose `user` is our dedicated Relay key (the deposit's sender and depositor) and whose
/// refunds go to our Solana address: the fixture's instruction keys name the Relay key (the order, so its id, does not name the sender).
fn relay_quote(q: &str) -> Vec<u8> {
    let relay = soltx::b58_encode(&fakerpc::sol::relay_of(&soltx::pubkey(ME_SOL).unwrap()));
    let mut v: Value = serde_json::from_str(q).unwrap();
    for k in v["steps"][0]["items"][0]["data"]["instructions"][0]["keys"].as_array_mut().unwrap() {
        if k["pubkey"] == ME_SOL {
            k["pubkey"] = relay.clone().into();
        }
    }
    serde_json::to_vec(&v).unwrap()
}

fn relay_b58() -> String {
    soltx::b58_encode(&fakerpc::sol::relay_of(&soltx::pubkey(ME_SOL).unwrap()))
}
const FLOAT: &str = "bc1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3t";
/// The quote's fetch time (its deadlines are this + 7 days).
const NOW: u64 = 1_792_170_366 - 7 * 86_400 + 5;
/// SOL/BTC at the quote's own price (658,506 sats expected for 5 SOL): 131,700 sats a SOL; SOL/ETH 22.816 SOL an ETH.
const SATS_PER_SOL: u64 = 131_700;
/// W8: the books' frog collection's wX mint (its record key on the books home), and its module (the collection key's address).
const FROG_WX: Pubkey = [0xC1; 32];
fn books_module() -> Pubkey {
    fakerpc::sol::derived_key(&etch_autobuyer::solkey::KeyPath::Coll(7))
}
const LAMPORTS_PER_ETH: u64 = 22_816_000_000;
/// The advance: 5 SOL for the quote and the Solana keep. The module's backing per net unit is exactly this (2 units, twice it), so it
/// is its `max_item_cost`, the advance's cap.
const ADVANCE: u64 = 5_000_000_000 + solhome::SOL_GAS_KEEP_LAMPORTS as u64;
/// The module's accepted floor: 4.9 SOL a frog (its item cap, + 3 %, covers the advance).
const ACCEPTED: u64 = 4_900_000_000;
const PRICE: u64 = 500_000;
const DEST: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";

fn p2tr(tag: u8) -> Vec<u8> {
    let mut s = vec![0x51, 0x20];
    s.extend([tag; 32]);
    s
}
fn float() -> Vec<u8> {
    etch_autobuyer::policy::script_of(FLOAT, "bc").unwrap()
}
fn custody() -> Vec<u8> {
    p2tr(0xc2)
}
fn insc(n: u8) -> String {
    format!("{}i0", hex::encode([n; 32]))
}
fn display(internal: [u8; 32]) -> [u8; 32] {
    let mut d = internal;
    d.reverse();
    d
}
fn block_hash_for(h: u64) -> String {
    hex::encode(keccak(&h.to_be_bytes()))
}

struct BtcTx {
    height: Option<u64>,
    hash: Option<String>,
}

struct World {
    now: u64,
    paused: bool,
    // ---- Solana: the chain, its providers, our key, the money line, the rates ----
    chain: Rc<RefCell<SolChain>>,
    ps: Vec<FakeSol>,
    signer: FakeSigner,
    sys: Rc<RefCell<SysWorld>>,
    me: Pubkey,
    line: Rc<RefCell<SolLine>>,
    sats_per_sol: Result<u64, String>,
    lamports_per_eth: Result<u64, String>,
    // ---- Bitcoin (tests/frogs.rs's world, cut down) ----
    tip: u64,
    blocks: HashMap<u64, String>,
    txs: HashMap<[u8; 32], BtcTx>,
    raws: HashMap<[u8; 32], Vec<u8>>,
    loc: HashMap<String, Location>,
    contents: HashMap<Outpoint, Contents>,
    outputs: HashMap<Outpoint, (Output, bool)>,
    outspends: HashMap<Outpoint, ([u8; 32], bool)>,
    /// custody's plain UTXOs (delivery fees)
    utxos: Vec<Utxo>,
    float_outs: Vec<Utxo>,
    btc_sent: Vec<Vec<u8>>,
    offers: HashMap<u64, Offer>,
    quotes: HashMap<u64, (Vec<u8>, u64)>,
    // ---- the flows' saves, the caps and the limiter ----
    saved: Option<FrogState>,
    redemptions: BTreeMap<u64, Redemption>,
    cursor: u64,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
    outflow: Vec<(String, outflow::Flow, u128)>,
    root: [u8; 32],
    proofs: Vec<Vec<[u8; 32]>>,
    /// Audit A5-02: the Ethereum side as `FrogIo` sees it (`eth_relay_busy`).
    eth_busy: bool,
    /// Ordinals multi-collection: the Inscriptions listing this world serves now (its wX mint), what the other collections hold that is
    /// canister-wide (Relay orders used, fee inputs reserved), and how frogs are funded.
    wx: Pubkey,
    used_other: Vec<[u8; 32]>,
    reserved_other: Vec<Outpoint>,
    funding: rc::FrogFunding,
}

impl World {
    /// W8: the world on the books home: the frog listing in the books (2 SOL-backed units, the module holding twice the advance), the
    /// module's SOL at the collection key's address; no program of Rook's on the chain.
    fn books() -> World {
        let me = soltx::pubkey(ME_SOL).unwrap();
        let chain = SolChain::new();
        chain.borrow_mut().unix = NOW;
        chain.borrow_mut().set_account(me, soltx::SYSTEM_PROGRAM, 1_000_000_000, vec![]);
        // a collection of four frogs: its membership root and each one's proof (sorted pairs, keccak)
        let ids = [insc(1), insc(2), insc(3), insc(4)];
        let pairh = |a: [u8; 32], b: [u8; 32]| { let (x, y) = if a <= b { (a, b) } else { (b, a) }; keccak(&[x, y].concat()) };
        let l: Vec<[u8; 32]> = ids.iter().map(|i| frogbuy::leaf_of(i)).collect();
        let (a, b) = (pairh(l[0], l[1]), pairh(l[2], l[3]));
        let root = pairh(a, b);
        let proofs = vec![vec![l[1], b], vec![l[0], b], vec![l[3], a], vec![l[2], a]];
        let sys = Rc::new(RefCell::new(SysWorld::default()));
        sys::install(&chain, &sys);
        sys::install_relay(&mut chain.borrow_mut());
        // the line's durable-nonce account exists already (B1's tests open it; here the frogs' steps start on it)
        let nonce = solsend::nonce_account_of(&me, "rook-home-0").unwrap();
        chain.borrow_mut().set_nonce(nonce, me);
        let ps = pair(&chain);
        let key = custodycap::sol_key(&FROG_WX);
        let caps = custodycap::CapPolicy { collections: vec![custodycap::CollectionCap { collection: key, max_items: 1_000,
            max_value_wei: u128::MAX / 4, max_item_wei: u128::MAX / 4 }], global_max_items: 1_000, global_max_value_wei: u128::MAX / 4 };
        let mut w = World { now: NOW, paused: false, chain, ps, signer: FakeSigner::new(me), sys, me,
            line: Rc::new(RefCell::new(SolLine { line: Line { nonce_account: Some(nonce), ..Default::default() }, ..Default::default() })),
            sats_per_sol: Ok(SATS_PER_SOL), lamports_per_eth: Ok(LAMPORTS_PER_ETH), tip: 900_000, blocks: HashMap::new(), txs: HashMap::new(),
            raws: HashMap::new(), loc: HashMap::new(), contents: HashMap::new(), outputs: HashMap::new(), outspends: HashMap::new(), utxos: vec![],
            float_outs: vec![], btc_sent: vec![], offers: HashMap::new(), quotes: HashMap::new(), saved: None, redemptions: BTreeMap::new(),
            cursor: 0, caps, book: custodycap::Book::default(), outflow: vec![], root, proofs, eth_busy: false, wx: FROG_WX, used_other: vec![],
            reserved_other: vec![], funding: rc::FrogFunding::default() };
        w.blocks.insert(w.tip, block_hash_for(w.tip));
        // custody's plain fee input (a delivery's fee)
        let fee = Outpoint { txid: [0xfe; 32], vout: 0 };
        w.utxos.push(Utxo { outpoint: fee, value: 20_000, confirmed: true });
        w.contents.insert(fee, Contents { inscriptions: vec![], runes: vec![], value: 20_000 });
        w.outputs.insert(fee, (Output { script: custody(), value: 20_000, confirmed: true }, false));
        w.txs.insert([0xfe; 32], BtcTx { height: Some(w.tip), hash: Some(w.blocks[&w.tip].clone()) });
        etch_autobuyer::desk::state::reset();
        let root = w.root;
        etch_autobuyer::desk::state::with_mut(|s| {
            s.books.add_collection(7, etch_autobuyer::books::ListingKind::Inscriptions { membership_root: root, custody: FLOAT.into() }, FROG_WX,
                etch_autobuyer::books::default_params(5_000_000_000)).unwrap();
            let c = s.books.collections.get_mut(&7).unwrap();
            let m = &mut c.module;
            m.accepted = ACCEPTED;
            m.target = ACCEPTED;
            m.posted_at = NOW as i64 - 60;
            m.confidence_bps = 8_000;
            m.depth_items = 10;
            m.day_anchor = ACCEPTED;
            m.day_anchor_at = NOW as i64 - 60;
            m.last_good_at = NOW as i64 - 60;
            m.backing = 2 * ADVANCE;
            c.ledger.sol_backed = 2 * ONE;
        });
        w.chain.borrow_mut().set_account(books_module(), soltx::SYSTEM_PROGRAM, 2 * ADVANCE + sys::RENT, vec![]);
        w
    }
    fn ledger(&self) -> etch_autobuyer::books::ledger::Ledger {
        etch_autobuyer::desk::state::with(|s| s.books.get(7).unwrap().ledger.clone())
    }
    fn bmodule(&self) -> etch_autobuyer::books::floor::Module {
        etch_autobuyer::desk::state::with(|s| s.books.get(7).unwrap().module.clone())
    }
    fn module_key(&self) -> [u8; 20] {
        frogsol::module_key(&books_module())
    }
    /// What the fake chain executed, by name (`system:<tag>`, `deposit_native`): never a program instruction of Rook's.
    fn sys_log(&self) -> Vec<String> {
        self.sys.borrow().log.clone()
    }
    /// Land everything sent, at finalized (the fake chain executes it).
    fn land(&mut self) -> Vec<String> {
        self.chain.borrow_mut().process(Level::Finalized)
    }
    /// The frog side's state with the frog module as its floor sink (`set_floor_sink` with the module's base58 address).
    fn states(&self) -> (FrogState, FloorState) {
        let k = self.module_key();
        let st = FrogState { sinks: vec![SinkRec { kind: SinkKind::Floor, sink: k, coin: [0; 20], view: None, refused: None, drove_refunds_at: None,
            drove_settle: false }], ..Default::default() };
        let fl = FloorState { sinks: vec![FloorCfg { module: k, item: FloorItem::Bitcoin }], ..Default::default() };
        (st, fl)
    }
    // ---- Bitcoin ----
    fn mine_btc(&mut self, n: u64) {
        for k in 0..n {
            self.tip += 1;
            let h = self.tip;
            self.blocks.insert(h, block_hash_for(h));
            if k > 0 {
                continue;
            }
            let pending: Vec<[u8; 32]> = self.txs.iter().filter(|(_, t)| t.height.is_none()).map(|(id, _)| *id).collect();
            for id in pending {
                let hash = self.blocks[&h].clone();
                let t = self.txs.get_mut(&id).unwrap();
                t.height = Some(h);
                t.hash = Some(hash);
                self.settle_btc(id);
            }
        }
    }
    /// A confirmed transaction of ours: what it spent leaves the UTXO lists; a purchase moves its frog to custody (output 1); a
    /// delivery moves it to its destination.
    fn settle_btc(&mut self, txid: [u8; 32]) {
        let Some(raw) = self.raws.get(&txid).cloned() else { return };
        let tx = parse_signed(&raw).unwrap();
        let spent: Vec<Outpoint> = tx.inputs.iter().map(|i| Outpoint { txid: display(i.txid), vout: i.vout }).collect();
        self.utxos.retain(|u| !spent.contains(&u.outpoint));
        self.float_outs.retain(|u| !spent.contains(&u.outpoint));
        for s in &spent {
            self.outspends.insert(*s, (txid, true));
            if let Some(o) = self.outputs.get_mut(s) {
                o.1 = true;
            }
        }
        let moved: Vec<(String, Location)> = self.loc.iter().filter(|(_, l)| spent.contains(&l.output)).map(|(a, b)| (a.clone(), b.clone())).collect();
        for (id, _) in moved {
            let (to, script) = if tx.outputs.get(1).is_some_and(|o| o.1 == custody()) { (1, custody()) } else { (0, tx.outputs[0].1.clone()) };
            let at = Outpoint { txid, vout: to };
            let value = tx.outputs[to as usize].0;
            self.loc.insert(id.clone(), Location { output: at, offset: 0, value });
            self.contents.insert(at, Contents { inscriptions: vec![id.clone()], runes: vec![], value });
            self.outputs.insert(at, (Output { script: script.clone(), value, confirmed: true }, false));
            if script == custody() {
                self.utxos.push(Utxo { outpoint: at, value, confirmed: true });
            }
        }
    }
    /// A seller's listed frog at `seller`, as ord and the explorers see it.
    fn list(&mut self, id: &str, seller: Outpoint) {
        self.loc.insert(id.to_string(), Location { output: seller, offset: 0, value: 10_000 });
        self.contents.insert(seller, Contents { inscriptions: vec![id.to_string()], runes: vec![], value: 10_000 });
        self.outputs.insert(seller, (Output { script: p2tr(0xee), value: 10_000, confirmed: true }, false));
    }
    /// The marketplace's PSBT for it (tests/frogs.rs's shape): padding, the frog, the float's payment; out: padding, the frog to custody,
    /// the price, the float's change.
    fn offer_for(&self, frog: u64, id: &str, seller: Outpoint, price: u64, proof: Vec<[u8; 32]>) -> Offer {
        let tx = Tx {
            version: 2,
            inputs: vec![
                TxIn { txid: [0xa1; 32], vout: frog as u32, sequence: 0xffff_fffd },
                TxIn { txid: [0xa2; 32], vout: frog as u32, sequence: 0xffff_fffd },
                TxIn { txid: seller.internal_txid(), vout: seller.vout, sequence: 0xffff_fffd },
                TxIn { txid: [0xa3; 32], vout: frog as u32, sequence: 0xffff_fffd },
            ],
            outputs: vec![
                TxOut { value: 1_200, script: float() },
                TxOut { value: 10_000, script: custody() },
                TxOut { value: price, script: p2tr(0xe5) },
                TxOut { value: 10_000_000 - price - 3_000, script: float() },
            ],
            locktime: 0,
        };
        let utxos = vec![Some(TxOut { value: 600, script: float() }), Some(TxOut { value: 600, script: float() }),
            Some(TxOut { value: 10_000, script: p2tr(0xee) }), Some(TxOut { value: 10_000_000, script: float() })];
        let p = Psbt { tx, witness_utxos: utxos };
        let mut seller_sig = vec![7u8; 65];
        seller_sig[64] = 0x83;
        let finals = vec![None, None, Some(vec![seller_sig]), None];
        Offer { inscription: id.to_string(), psbt: frogbuy::psbt_bytes(&p, &finals), to_sign: vec![0, 1, 3], proof }
    }
    /// Relay fills the order: `sats` to the float, confirmed `deep` blocks.
    fn relay_fills(&mut self, sats: u64, deep: u64) -> Outpoint {
        let o = Outpoint { txid: [0xab; 32], vout: 0 };
        self.txs.insert(o.txid, BtcTx { height: None, hash: None });
        self.mine_btc(deep);
        self.float_outs.push(Utxo { outpoint: o, value: sats, confirmed: true });
        self.outputs.insert(o, (Output { script: float(), value: sats, confirmed: true }, false));
        o
    }
}

impl SolIo for World {
    fn sol_home(&self) -> Option<SolHomeCfg> {
        Some(SolHomeCfg { lifetime: LifetimeKind::DurableNonce, nonce_seed: "rook-home-0".into() })
    }
    async fn sol_me(&mut self) -> Result<Pubkey, String> { Ok(self.me) }
    async fn sol_accounts(&mut self, keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts(&mut self.ps, keys, max_data).await?)
    }
    async fn sol_account_head(&mut self, key: &Pubkey, len: usize) -> Result<Option<Account>, String> {
        Ok(solrpc::account_slice(&mut self.ps, key, 0, len).await?)
    }
    async fn sol_rent(&mut self, space: u64) -> Result<u64, String> { Ok(solrpc::rent_exempt(&mut self.ps, space).await?) }
    async fn sol_relay(&mut self) -> Result<Pubkey, String> { Ok(self.signer.relay) }
    async fn sol_balance_of(&mut self, key: &Pubkey) -> Result<u128, String> { Ok(solrpc::balance(&mut self.ps, key).await? as u128) }
    async fn sol_rate(&mut self) -> Result<u64, String> { self.lamports_per_eth.clone() }
    async fn sol_btc_rate(&mut self) -> Result<u64, String> { self.sats_per_sol.clone() }
    async fn sol_balance(&mut self) -> Result<u128, String> { Ok(solrpc::balance(&mut self.ps, &self.me).await? as u128) }
    async fn sol_clock(&mut self) -> Result<(u64, u64), String> { Ok(solrpc::finalized_clock(&mut self.ps).await?) }
    async fn sol_prepare(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>) -> Result<(), String> {
        solsend::prepare(line, kind, intent, what, ixs, &mut self.ps, &mut self.signer).await
    }
    async fn sol_tick(&mut self, line: &mut Line) -> Result<solsend::Step, String> { Ok(solsend::tick(line, &mut self.ps, &mut self.signer).await?) }
    async fn sol_prepare_keys(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>,
        keys: Vec<etch_autobuyer::solkey::KeyPath>) -> Result<(), String> {
        solsend::prepare_keys(line, kind, intent, what, ixs, keys, &mut self.ps, &mut self.signer).await
    }
    async fn sol_key_of(&mut self, p: &etch_autobuyer::solkey::KeyPath) -> Result<Pubkey, String> { self.signer.key(p).await }
    async fn sol_signatures(&mut self, until: &str, limit: usize) -> Result<Vec<SigInfo>, String> {
        Ok(solrpc::signatures_since(&mut self.ps, &self.me, until, limit).await?)
    }
    async fn sol_transaction(&mut self, sig: &str) -> Result<Option<Value>, String> {
        Ok(solrpc::transaction(&mut self.ps, sig, solhome::TX_BYTES).await?)
    }
    async fn sol_genesis(&mut self) -> Result<String, String> { Ok(solrpc::genesis_hash(&mut self.ps).await?) }
    fn sol_line(&mut self) -> SolLine { self.line.borrow().clone() }
    fn sol_line_save(&mut self, l: &SolLine) { *self.line.borrow_mut() = l.clone(); }
}

impl FrogIo for World {
    fn eth_relay_busy(&self) -> bool { self.eth_busy }
    fn sol_wrapper(&self) -> Option<Pubkey> { Some(self.wx) }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 0 }
    fn wrapper(&self) -> [u8; 20] { self.wx[..20].try_into().unwrap() }
    fn used_elsewhere(&self) -> Vec<[u8; 32]> { self.used_other.clone() }
    fn fresh(&mut self) {}
    // the home is Solana: no EVM read, signature or send is ever asked
    async fn eth_read(&mut self, to: [u8; 20], _d: &[u8]) -> Result<Vec<u8>, String> { panic!("an EVM read of 0x{} on the Solana home", hex::encode(to)) }
    async fn clock(&mut self) -> Result<Clock, String> { Err("no EVM clock on the Solana home".into()) }
    async fn our_evm(&mut self) -> Result<[u8; 20], String> { panic!("our EVM address asked on the Solana home") }
    async fn evm_nonce(&mut self) -> Result<u64, String> { panic!("an EVM nonce on the Solana home") }
    async fn base_fee(&mut self) -> Result<u128, String> { panic!("an EVM fee on the Solana home") }
    async fn estimate_gas(&mut self, _t: [u8; 20], _v: u128, _d: &[u8]) -> Result<u64, String> { panic!("an EVM estimate on the Solana home") }
    async fn sign_evm(&mut self, _h: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { panic!("an EVM signature on the Solana home") }
    async fn send_evm(&mut self, _r: &[u8]) -> Result<(), String> { panic!("an EVM send on the Solana home") }
    async fn evm_seen(&mut self, _h: [u8; 32]) -> EvmView { panic!("an EVM status on the Solana home") }
    async fn float_script(&mut self) -> Result<Vec<u8>, String> { Ok(float()) }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> { Ok(custody()) }
    async fn locate(&mut self, i: &str) -> Result<Option<Location>, String> { Ok(self.loc.get(i).cloned()) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> { Ok(self.contents.get(o).cloned()) }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        if let Some(x) = self.outputs.get(o).cloned() {
            return Ok(x);
        }
        self.utxos.iter().chain(self.float_outs.iter()).find(|u| u.outpoint == *o)
            .map(|u| (Output { script: custody(), value: u.value, confirmed: u.confirmed }, false)).ok_or_else(|| format!("no output {}", o.text()))
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> { Ok(self.outspends.get(o).copied()) }
    async fn tx_status(&mut self, txid: [u8; 32]) -> BtcSeen {
        match self.txs.get(&txid) {
            None => BtcSeen::NotYet,
            Some(BtcTx { height: Some(h), hash: Some(b) }) => BtcSeen::Confirmed { height: *h, block_hash: b.clone() },
            Some(_) => BtcSeen::Mempool,
        }
    }
    async fn tip(&mut self) -> Result<u64, String> { Ok(self.tip) }
    async fn block_hash(&mut self, h: u64) -> Result<String, String> { self.blocks.get(&h).cloned().ok_or("no block".into()) }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        let txid = display(parse_signed(raw).map_err(|e| e.to_string())?.txid);
        self.btc_sent.push(raw.to_vec());
        self.raws.insert(txid, raw.to_vec());
        self.txs.entry(txid).or_insert(BtcTx { height: None, hash: None });
        Ok(())
    }
    async fn custody_utxos(&mut self) -> Result<Vec<Utxo>, String> { Ok(self.utxos.clone()) }
    async fn tx_view(&mut self, _t: [u8; 32]) -> Result<TxView, String> { panic!("holder deposits are off on the Solana home") }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(5) }
    /// Phase B2: satoshis per 10^18 lamports (SOL/BTC), the home's unit for `frogbuy`'s arithmetic.
    async fn rate(&mut self) -> Result<u64, String> { rates::sats_per_home_unit(self.sats_per_sol.clone()?) }
    async fn input_clean(&mut self, _o: Outpoint) -> Result<(), String> { Ok(()) }
    async fn sign_float(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> { Ok(vec![0x11; 64]) }
    async fn sign_custody(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> { Ok(vec![0x22; 64]) }
    fn take_offer(&mut self, frog: u64) -> Option<Offer> { self.offers.remove(&frog) }
    fn save(&mut self, st: &FrogState) { self.saved = Some(st.clone()); }
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) -> Result<(), String> { Ok(()) }
    fn outflow_record(&mut self, key: &str, _c: &str, flow: outflow::Flow, amount: u128) { self.outflow.push((key.into(), flow, amount)); }
    fn outbound_policy(&self, custody_script: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: custody_script.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true }
    }
    fn paused(&self) -> bool { self.paused }
    async fn our_balance(&mut self) -> Result<u128, String> { Ok(solrpc::balance(&mut self.ps, &self.me).await? as u128) }
    async fn float_utxos(&mut self) -> Result<Vec<Utxo>, String> { Ok(self.float_outs.clone()) }
    async fn float_address(&mut self) -> Result<String, String> { Ok(FLOAT.into()) }
    fn funding(&self) -> Option<rc::FrogFunding> { Some(self.funding) }
    fn take_quote(&mut self, frog: u64) -> Option<(Vec<u8>, u64)> { self.quotes.remove(&frog) }
    fn return_quote(&mut self, frog: u64, body: Vec<u8>, at: u64) { self.quotes.entry(frog).or_insert((body, at)); }
    fn take_protected(&mut self, _frog: u64) -> Option<ProtectedOffer> { None }
    async fn float_internal_key(&mut self) -> Result<[u8; 32], String> { Ok([0x1f; 32]) }
}

impl CustodyIo for World {
    fn sol_wrapper(&self) -> Option<Pubkey> { Some(self.wx) }
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 0 }
    async fn ticket_count(&mut self) -> Result<(u64, u64), String> { let w = self.wx; frogsol::ticket_count(self, w, "bc").await }
    async fn read_ticket(&mut self, id: u64, _fresh: bool) -> Result<TicketRead, String> {
        let (now, w) = (self.now, self.wx);
        frogsol::ticket_read(self, w, id, now, "bc").await
    }
    async fn inscription_of(&mut self, a: u128) -> Result<String, String> { let w = self.wx; frogsol::inscription_of(self, w, a).await }
    async fn locate(&mut self, i: &str) -> Result<Option<Location>, String> { Ok(self.loc.get(i).cloned()) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> { Ok(self.contents.get(o).cloned()) }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> { FrogIo::output(self, o).await }
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        Ok(self.utxos.iter().filter(|u| !self.outspends.contains_key(&u.outpoint) && self.contents.get(&u.outpoint).is_some_and(|c| c.inscriptions.is_empty()))
            .cloned().collect())
    }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(3) }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { Ok(custody()) }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
    async fn sign_taproot(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> { Ok(vec![0x5a; 64]) }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> { FrogIo::btc_send(self, raw).await }
    async fn btc_seen(&mut self, txid: [u8; 32]) -> BtcView {
        match self.txs.get(&txid) {
            None => BtcView::NotYet,
            Some(BtcTx { height: Some(_), .. }) => BtcView::Confirmed,
            Some(_) => BtcView::Mempool,
        }
    }
    async fn may_record(&mut self) -> Result<bool, String> { let w = self.wx; frogsol::may_record(self, w).await }
    fn wrapper(&self) -> [u8; 20] { self.wx[..20].try_into().unwrap() }
    fn ticket_key(&self, ticket: u64) -> String { format!("ticket:{}:{ticket}", self.wx[0]) }
    fn reserved_elsewhere(&self) -> Vec<Outpoint> { self.reserved_other.clone() }
    async fn evm_nonce(&mut self) -> Result<u64, String> { panic!("an EVM nonce on the Solana home") }
    async fn base_fee(&mut self) -> Result<u128, String> { panic!("an EVM fee on the Solana home") }
    async fn estimate_gas(&mut self, _t: [u8; 20], _v: u128, _d: &[u8]) -> Result<u64, String> { panic!("an EVM estimate on the Solana home") }
    async fn sign_evm(&mut self, _h: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { panic!("an EVM signature on the Solana home") }
    async fn send_evm(&mut self, _r: &[u8]) -> Result<(), String> { panic!("an EVM send on the Solana home") }
    async fn evm_seen(&mut self, _h: [u8; 32]) -> EvmView { panic!("an EVM status on the Solana home") }
    fn save(&mut self, r: &Redemption) { self.redemptions.insert(r.ticket, r.clone()); }
    fn save_cursor(&mut self, c: u64) { self.cursor = c; }
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) -> Result<(), String> { Ok(()) }
    fn outflow_record(&mut self, key: &str, _c: &str, flow: outflow::Flow, amount: u128) { self.outflow.push((key.into(), flow, amount)); }
    async fn scan_ticket_logs(&mut self, _l: &mut etch_autobuyer::ticketlog::TicketLogs, _w: Option<&[u64]>) -> Result<String, String> {
        panic!("no event scan on the Solana home: the tickets are accounts at finalized")
    }
    fn outbound_policy(&self, custody_script: &[u8]) -> outbound::BtcPolicy { FrogIo::outbound_policy(self, custody_script) }
    fn paused(&self) -> bool { self.paused }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> { Ok(self.outspends.get(o).copied()) }
}

fn tick(w: &mut World, st: &mut FrogState, fl: &mut FloorState) -> Vec<String> {
    w.chain.borrow_mut().unix = w.now;
    let lines = block_on(frogflow::tick_with(w, st, fl, false, &[]));
    for l in &lines {
        println!("{l}");
    }
    lines
}

fn ctick(w: &mut World, c: &mut Custody) -> Vec<String> {
    w.chain.borrow_mut().unix = w.now;
    let lines = block_on(redeem::tick(w, c, false));
    for l in &lines {
        println!("{l}");
    }
    lines
}

fn phase(st: &FrogState, id: u64) -> FPhase {
    st.frogs.iter().find(|f| f.id == id).unwrap().phase.clone()
}

fn conv(st: &FrogState) -> CPhase {
    st.conv(0).unwrap().phase.clone()
}

/// The quote's own clock: the runner hands it in now.
fn hand_in_quote(w: &mut World) {
    w.quotes.insert(0, (relay_quote(SOL_BTC_5), w.now));
}

/// A frog wanted for the module, its advance landed (5 SOL plus the keep at our address), the runner asked for a SOL → BTC quote.
fn to_funded(w: &mut World, st: &mut FrogState, fl: &mut FloorState) {
    let l = tick(w, st, fl);
    assert!(matches!(phase(st, 0), FPhase::Wanted { cap_wei } if cap_wei == ADVANCE as u128), "{:?} {l:?}", phase(st, 0));
    assert_eq!(conv(st), CPhase::Advancing, "the advance is out on the Solana line this tick: {l:?}");
    assert!(st.sol_step.as_ref().is_some_and(|p| p.step == LineStep::Frog(FStep::Advance { frog: 0 })));
    assert_eq!(w.land().len(), 1);
    tick(w, st, fl);
    assert_eq!(conv(st), CPhase::Funded { wei: 5_000_000_000, since: w.now }, "the advance less the keep is to convert");
    let ask = st.conv(0).unwrap().ask.clone().unwrap();
    assert_eq!((ask.user.as_str(), ask.refund_to.as_str(), ask.recipient.as_str(), ask.origin_chain_id, ask.destination_chain_id,
        ask.origin_currency.as_str(), ask.amount_wei), (relay_b58().as_str(), ME_SOL, FLOAT, relaysol::RELAY_SOLANA, rc::RELAY_BITCOIN, relaysol::SOL_NATIVE,
        5_000_000_000), "the runner asks Relay for exactly this: from our Solana address to the float");
}

/// ... then the judged deposit landed: converting, watched at the float.
fn to_converting(w: &mut World, st: &mut FrogState, fl: &mut FloorState) -> [u8; 32] {
    to_funded(w, st, fl);
    hand_in_quote(w);
    let l = tick(w, st, fl);
    assert!(matches!(conv(st), CPhase::Depositing { value: 5_000_000_000, min_sats: 645_336, .. }), "{:?} {l:?}", conv(st));
    // audit A5-03: our key funds the Relay key; the Relay key alone signs the deposit
    assert_eq!(w.land().len(), 1, "the funding lands");
    tick(w, st, fl);
    assert_eq!(w.land().len(), 1, "the deposit lands");
    tick(w, st, fl);
    let CPhase::Converting { order_id, .. } = conv(st) else { panic!("{:?}", conv(st)) };
    order_id
}

const MONKES: u16 = 12;
const MONKE_WX: Pubkey = [0xC2; 32];
const MULTI: &str = include_str!("fixtures/ordinals/multi.json");

/// A collection's members root, one member and its proof, from the launch file's members.
fn fixture(slug: &str) -> ([u8; 32], String, Vec<[u8; 32]>) {
    let v: Value = serde_json::from_str(MULTI).unwrap();
    let h = |s: &str| -> [u8; 32] { hex::decode(s).unwrap().try_into().unwrap() };
    let e = &v[slug];
    (h(e["root"].as_str().unwrap()), e["inscription"].as_str().unwrap().to_string(),
        e["proof"].as_array().unwrap().iter().map(|p| h(p.as_str().unwrap())).collect())
}

fn monke_module() -> Pubkey {
    fakerpc::sol::derived_key(&etch_autobuyer::solkey::KeyPath::Coll(MONKES as u32))
}

impl World {
    /// [`World::books`] and NodeMonkes as listing 12 (its real root), its module set and funded as the frogs' is; a custody cap each.
    fn books_two() -> World {
        let w = World::books();
        let (root, ..) = fixture("nodemonkes");
        etch_autobuyer::desk::state::with_mut(|s| {
            s.books.add_collection(MONKES, etch_autobuyer::books::ListingKind::Inscriptions { membership_root: root, custody: FLOAT.into() }, MONKE_WX,
                etch_autobuyer::books::default_params(5_000_000_000)).unwrap();
            let src = s.books.collections.get(&7).unwrap().clone();
            let c = s.books.collections.get_mut(&MONKES).unwrap();
            c.module = src.module.clone();
            c.ledger.sol_backed = src.ledger.sol_backed;
        });
        w.chain.borrow_mut().set_account(monke_module(), soltx::SYSTEM_PROGRAM, 2 * ADVANCE + sys::RENT, vec![]);
        let mut w = w;
        w.caps.collections.push(custodycap::CollectionCap { collection: custodycap::sol_key(&MONKE_WX), max_items: 1, max_value_wei: u128::MAX / 4,
            max_item_wei: u128::MAX / 4 });
        w
    }
    fn ledger_of(&self, id: u16) -> etch_autobuyer::books::ledger::Ledger {
        etch_autobuyer::desk::state::with(|s| s.books.get(id).unwrap().ledger.clone())
    }
    fn module_of(&self, id: u16) -> etch_autobuyer::books::floor::Module {
        etch_autobuyer::desk::state::with(|s| s.books.get(id).unwrap().module.clone())
    }
    /// A collection's frog state with its own module as its floor sink (`next_frog` canister-wide), and the one floor state naming both.
    fn state_for(module: &Pubkey, next_frog: u64) -> FrogState {
        FrogState { sinks: vec![SinkRec { kind: SinkKind::Floor, sink: frogsol::module_key(module), coin: [0; 20], view: None, refused: None,
            drove_refunds_at: None, drove_settle: false }], next_frog, ..Default::default() }
    }
    fn floors_two() -> FloorState {
        FloorState { sinks: vec![FloorCfg { module: frogsol::module_key(&books_module()), item: FloorItem::Bitcoin },
            FloorCfg { module: frogsol::module_key(&monke_module()), item: FloorItem::Bitcoin }], ..Default::default() }
    }
    /// The frog outcomes on the line, by the collection their book update names.
    fn book_outcomes(&self) -> Vec<String> {
        self.line.borrow().outcomes.iter().map(|o| o.signature.clone()).filter(|s| s.starts_with("books:")).collect()
    }
}


// =====================================================================================================================================
// C2-01 (High): one collection's pass releases every other collection's in-flight custody-cap reservation
// =====================================================================================================================================

/// C2-01. `frogflow::sweep_stale` (step 4a of every collection's `tick_all`) releases each `frog:<id>` reservation of the ONE custody book
/// whose frog is not in the state it was handed. On the books home that state is one collection's entry, and frog ids are unique
/// canister-wide, so collection 7's pass releases collection 12's purchase in flight. When collection 12's frog then arrives, its
/// `settle` finds nothing in flight: the inscription is held in custody and never booked. Collection 12's cap (1 item here; stage 0's 3
/// in the default caps) and the global cap the exit rule (A4-L-1) sized are no longer what custody holds.
///
/// A fix must refuse this: collection 7's pass must leave `frog:0` (a frog of collection 12) in flight, collection 12's arrival must
/// be held in the book under its collection, and `check_acquire` for NodeMonkes must then refuse a second item.
///
/// FIXED (2026-10-12): `sweep_stale` releases only the reservations of the collection whose state it holds (`collection_key(io)`), and
/// an arrival with no reservation in flight is booked held all the same, loud (`custodycap::hold_arrived`).
#[test]
fn fixed_c2_01_one_collections_pass_releases_anothers_purchase_in_flight_and_its_arrival_is_never_booked() {
    let mut w = World::books_two();
    let mut fl = World::floors_two();
    let monkes = custodycap::sol_key(&MONKE_WX);
    // collection 12 (NodeMonkes) buys from the float: frog 0, signed, its room reserved in the one book
    w.wx = MONKE_WX;
    w.funding = rc::FrogFunding { max_fee_bps: 300, float_first: true };
    w.float_outs.push(Utxo { outpoint: Outpoint { txid: [0x60; 32], vout: 0 }, value: 20_000_000, confirmed: true });
    let mut st12 = World::state_for(&monke_module(), 0);
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(matches!(phase(&st12, 0), FPhase::Wanted { .. }), "{l:?}");
    let (_, monke, proof) = fixture("nodemonkes");
    let seller = Outpoint { txid: [0xe2; 32], vout: 0 };
    w.list(&monke, seller);
    let o = w.offer_for(0, &monke, seller, PRICE, proof);
    w.offers.insert(0, o);
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(matches!(phase(&st12, 0), FPhase::Signed { .. }), "{:?} {l:?}", phase(&st12, 0));
    assert!(w.book.entries.iter().any(|e| e.id == "frog:0" && e.in_flight && e.collection == monkes), "reserved at signing: {:?}", w.book);
    // collection 7 (Bitcoin Frogs) runs its pass: nothing of its own is open (the tick runs it in the same canister tick)
    w.wx = FROG_WX;
    let mut st7 = FrogState { next_frog: st12.next_frog, ..Default::default() };
    let l = tick(&mut w, &mut st7, &mut fl);
    // FIXED: collection 7's pass leaves collection 12's purchase in flight
    assert!(!l.iter().any(|x| x.contains("was reserved for a purchase that is gone")), "{l:?}");
    assert!(w.book.entries.iter().any(|e| e.id == "frog:0" && e.in_flight && e.collection == monkes), "still reserved: {:?}", w.book);
    // collection 12's frog confirms and arrives in custody: its reservation becomes the held NodeMonke
    w.wx = MONKE_WX;
    w.mine_btc(1);
    tick(&mut w, &mut st12, &mut fl);
    w.mine_btc(5);
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(l.iter().any(|x| x.contains("in custody: held")) && !l.iter().any(|x| x.contains("not in flight")), "{l:?}");
    assert!(!matches!(phase(&st12, 0), FPhase::Wanted { .. } | FPhase::Signed { .. } | FPhase::Sent { .. }), "{:?}", phase(&st12, 0));
    // FIXED: the NodeMonke in custody is in the book, so NodeMonkes' one-item cap refuses a second
    let held: Vec<_> = w.book.entries.iter().filter(|e| e.collection == monkes).collect();
    assert_eq!((held.len(), held[0].id.as_str(), held[0].in_flight), (1, monke.as_str(), false), "{:?}", w.book);
    assert!(matches!(custodycap::check_acquire(&w.caps, &w.book, &monkes, 1), Err(custodycap::Refusal::CollectionItems { .. })));
}

/// Audit C2-01, the second half: an arrival whose reservation is not in flight (released by an older build's pass, or lost) is booked
/// held all the same, under its collection, with a loud line: custody never holds an item the book does not count.
#[test]
fn fixed_c2_01_an_arrival_with_no_reservation_in_flight_is_booked_all_the_same() {
    let mut w = World::books_two();
    let mut fl = World::floors_two();
    let monkes = custodycap::sol_key(&MONKE_WX);
    w.wx = MONKE_WX;
    w.funding = rc::FrogFunding { max_fee_bps: 300, float_first: true };
    w.float_outs.push(Utxo { outpoint: Outpoint { txid: [0x60; 32], vout: 0 }, value: 20_000_000, confirmed: true });
    let mut st12 = World::state_for(&monke_module(), 0);
    tick(&mut w, &mut st12, &mut fl);
    let (_, monke, proof) = fixture("nodemonkes");
    let seller = Outpoint { txid: [0xe2; 32], vout: 0 };
    w.list(&monke, seller);
    let o = w.offer_for(0, &monke, seller, PRICE, proof);
    w.offers.insert(0, o);
    tick(&mut w, &mut st12, &mut fl);
    assert!(matches!(phase(&st12, 0), FPhase::Signed { .. }));
    // the reservation vanishes (as an older build's pass of another collection released it)
    w.book.entries.retain(|e| e.id != "frog:0");
    w.mine_btc(1);
    tick(&mut w, &mut st12, &mut fl);
    w.mine_btc(5);
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(l.iter().any(|x| x.contains("CUSTODY BOOK, audit C2-01: frog:0 was not in flight")), "{l:?}");
    assert!(w.book.entries.iter().any(|e| e.id == monke && !e.in_flight && e.collection == monkes), "{:?}", w.book);
    assert!(custodycap::check_acquire(&w.caps, &w.book, &monkes, 1).is_err(), "the one-item cap binds");
}

// =====================================================================================================================================
// C2-02 (Medium): frog_sleep puts a collection to sleep while its written-off Relay order still holds the one Relay leg
// =====================================================================================================================================

/// C2-02. `multi::open_work` counts a conversion as open only while it is not `finished()`; a conversion that gave back (`Returned`) is
/// finished even while its written-off order is still watched (`late_open`, conversion review C-03), which holds the ONE Relay leg
/// (`convflow::holds_relay`). `frog_sleep` therefore puts the collection to sleep; a dormant collection is never ticked, so the late
/// watch is never read again (its refund is never credited to the module, its fill never recorded) and the leg stays held for every
/// other collection (`held_elsewhere`) and for the Ethereum and Robinhood sides (`ethnet`'s `relay_busy` reads `multi::holds_relay`)
/// until the operator wakes it again or calls `relay_late_release`.
///
/// A fix must refuse this sleep ("work open: frog 0's written-off order is still watched") or keep ticking a dormant collection's
/// conversions.
///
/// FIXED (2026-10-12): `multi::open_work` reports a written-off order still watched and anything that holds the leg, so the sleep is
/// refused and the collection stays live (its watch read every tick); `frog_sleep` also refuses a collection with an unread outcome on
/// the Solana line.
#[test]
fn fixed_c2_02_a_collection_sleeps_while_its_written_off_order_holds_the_relay_leg_for_every_other() {
    use etch_autobuyer::frogsol::multi;
    use etch_autobuyer::relayconv::{ConvRec, LateConv, Watch};
    let frogs = multi::Listing { id: 7, wx_mint: FROG_WX, root: [7; 32], custody: FLOAT.into() };
    let mut top = FrogState::default();
    multi::wake(&mut top, &frogs, NOW).unwrap();
    let mut c = ConvRec::new(0, [7; 20]);
    c.phase = CPhase::Returned { wei: 5_000_000_000 };
    c.late = Some(LateConv { watch: Watch { order_id: [9; 32], value: 5_000_000_000, min_sats: 645_336, deadline: NOW, balance_after: 1 },
        written_off_at: NOW, balance_after: 1, refund_wei: None, credited: None, filled: None, released_at: None });
    multi::entry_mut(&mut top, 7).convs = Some(vec![c]);
    assert!(multi::holds_relay(&top) && multi::held_elsewhere(&top, MONKES), "the written-off order holds the one leg");
    // FIXED: the sleep is refused while the leg is held; collection 7 stays live, so its late watch is read and the leg freed
    let r = multi::sleep(&mut top, &frogs, NOW + 60);
    assert!(r.as_ref().unwrap_err().contains("frog 0's written-off Relay order is still watched"), "{r:?}");
    assert!(multi::is_live(&top, &FROG_WX), "still live: the tick keeps reading collection 7's watch");
}

// =====================================================================================================================================
// C2-12 (Informational, fixed with the default-live switch): a listing's first floor is bounded by its launch floor
// =====================================================================================================================================

/// Audit C2-12. NodeMonkes (its real members root) with no floor accepted yet: a first reading far under its launch floor (17.44 SOL,
/// graduation-tiers.json) is refused and nothing is posted; one inside the band is posted in its books. The frogs' module (accepted
/// already) is not this rule's.
#[test]
fn fixed_c2_12_a_first_floor_outside_the_band_of_its_launch_floor_is_refused() {
    let mut w = World::books_two();
    etch_autobuyer::desk::state::with_mut(|s| {
        let m = &mut s.books.collections.get_mut(&MONKES).unwrap().module;
        m.accepted = 0;
        m.target = 0;
        m.posted_at = 0;
    });
    let mut fl = World::floors_two();
    w.wx = MONKE_WX;
    let mut st12 = FrogState::default();
    // 640,000 sats at 131,700 sats a SOL: 4.86 SOL, under half of 17.44
    st12.floor_reading = Some(frogsol::FrogReading { sats: 640_000, confidence_bps: 8_000, depth_items: 10, at: w.now });
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(l.iter().any(|x| x.contains("REFUSED") && x.contains("nodemonkes's first floor") && x.contains("audit C2-12")), "{l:?}");
    assert_eq!((w.module_of(MONKES).accepted, st12.floor_reading.clone()), (0, None), "nothing posted, the reading dropped");
    assert!(w.book_outcomes().is_empty());
    // 2,300,000 sats: 17.46 SOL, inside the band: posted
    st12.floor_reading = Some(frogsol::FrogReading { sats: 2_300_000, confidence_bps: 8_000, depth_items: 10, at: w.now });
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(l.iter().any(|x| x.contains("post_floor")), "{l:?}");
    assert!(w.module_of(MONKES).accepted > 0);
    // a later reading is the module's move bound's, not the band's (the frogs' module, accepted already, posts 640,000 sats)
    w.wx = FROG_WX;
    let mut st7 = FrogState::default();
    st7.floor_reading = Some(frogsol::FrogReading { sats: 640_000, confidence_bps: 8_000, depth_items: 10, at: w.now });
    let l = tick(&mut w, &mut st7, &mut fl);
    assert!(l.iter().any(|x| x.contains("post_floor")), "{l:?}");
}
