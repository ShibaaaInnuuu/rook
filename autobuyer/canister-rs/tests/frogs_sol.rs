//! The Bitcoin-inscription (frog) side with its home on SOLANA, the BOOKS home (docs/v9/solana-migration.md §2.3, §2.4, §4;
//! docs/v9/canister-only-solana.md §8, decision D-l): the frog flow of `tests/frogs.rs` against the books' Inscriptions listing, every
//! home read from the books and every home step a book update, but the advance (`M → K_main`, signed by the collection's key) and
//! Relay's `deposit_native`, SIMULATED, then signed on the money line and LANDED by the fake Solana chain (`fakerpc::sys`: the System
//! Program and Relay's depository), every read at `finalized` through the fake provider pair (`fakerpc::sol`). Bitcoin is a fake as in
//! `tests/frogs.rs`. The Relay quote is the real SOL → BTC one captured 2026-10-09 (spike Q5: `sol-btc-5`, 5 SOL for at least 645,336
//! sats to its taproot recipient, which is this world's float). The custodian's redemption runs against the same world (`CustodyIo`).
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
use etch_autobuyer::redeem::{self, Custody, CustodyIo, RPhase, Redemption};
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

// ---- the frog buy end to end -------------------------------------------------------------------------------------------------------

#[test]
fn a5_02_a5_08_item_arrived_waits_for_the_ethereum_leg_and_an_unused_turn_is_released() {
    let mut w = World::books();
    let (mut st, mut fl) = w.states();
    to_converting(&mut w, &mut st, &mut fl);
    w.relay_fills(658_506, 3);
    tick(&mut w, &mut st, &mut fl);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(2), seller);
    let o = w.offer_for(0, &insc(2), seller, PRICE, w.proofs[1].clone());
    w.offers.insert(0, o);
    tick(&mut w, &mut st, &mut fl);
    w.mine_btc(1);
    tick(&mut w, &mut st, &mut fl);
    w.mine_btc(5);
    let l = tick(&mut w, &mut st, &mut fl);
    assert!(l.iter().any(|x| x.contains("minted to the module in the books")), "{l:?}");
    assert_eq!(w.land().len(), 0, "the mint is a book update: the flow reads its landing next tick, and reports then");
    // as if the frog had been bought without an advance (inventory, the float): its report is `item_arrived`, which pays us
    st.convs = None;
    w.eth_busy = true;
    // the frog side asked for the line earlier and is at the head of the queue
    w.line.borrow_mut().queue = vec![(solhome::LineOwner::Frog, w.now)];
    let l = tick(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Minted { .. }), "{:?} {l:?}", phase(&st, 0));
    assert!(l.iter().any(|x| x.contains("item_arrived, paying our Solana address) waits")), "{l:?}");
    assert!(w.line.borrow().pending.is_none() && st.sol_step.is_none(), "nothing signed: {l:?}");
    assert!(w.line.borrow().queue.is_empty(), "the unused turn at the head of a free line is given up (A5-08)");
    // behind another owner's turn, the frog side keeps its place
    w.line.borrow_mut().queue = vec![(solhome::LineOwner::Custody, w.now), (solhome::LineOwner::Frog, w.now)];
    tick(&mut w, &mut st, &mut fl);
    assert_eq!(w.line.borrow().queue.iter().map(|(o, _)| *o).collect::<Vec<_>>(), vec![solhome::LineOwner::Custody, solhome::LineOwner::Frog]);
    // the Ethereum leg free: the report is a book update (`item_arrived`, final at once), nothing on the line
    w.eth_busy = false;
    w.line.borrow_mut().queue.clear();
    let l = tick(&mut w, &mut st, &mut fl);
    assert!(l.iter().any(|x| x.contains("item_arrived at") && x.contains("in the books")), "{l:?}");
    assert!(st.sol_step.is_none() && w.land().is_empty(), "{:?} {l:?}", st.sol_step);
}

/// Audit A5-06 and A5-I2: on the Solana home (no sales: nothing relists) a ticket to custody itself is refused like the float's or the
/// buyer's address, never an inert `Relist`; a destination that does not parse for our network (an Ethereum destination on an
/// Inscriptions ticket) is refused once, never an error every tick. Both are then marked undeliverable on the line, so the holder is
/// refunded.
#[test]
fn a5_06_a5_i2_a_ticket_to_custody_or_to_no_bitcoin_address_is_refused_and_marked_undeliverable() {
    use etch_autobuyer::books::ledger::Destination;
    let mut w = World::books();
    let to_custody = etch_autobuyer::policy::taproot_address(&[0xc2; 32], "bc");
    assert_eq!(etch_autobuyer::policy::script_of(&to_custody, "bc").unwrap(), custody());
    let t1 = books_resolved_ticket(&insc(3), Destination::Taproot(to_custody.clone()), NOW as i64 + 7 * 86_400);
    let t2 = books_resolved_ticket(&insc(4), Destination::Eth([0x11; 20]), NOW as i64 + 7 * 86_400);
    let mut c = Custody::default();
    let l = ctick(&mut w, &mut c);
    let r1 = c.redemptions.iter().find(|r| r.ticket == t1).unwrap().clone();
    assert!(matches!(r1.phase, RPhase::Refused { ref reason } if reason.contains("custody address")), "{:?} {l:?}", r1.phase);
    let r2 = c.redemptions.iter().find(|r| r.ticket == t2).unwrap().clone();
    assert!(matches!(r2.phase, RPhase::Refused { ref reason } if reason.starts_with("Malformed")), "{:?} {l:?}", r2.phase);
    assert!(w.btc_sent.is_empty(), "nothing leaves custody");
    // the marks are book updates, read back at the next pass
    for _ in 0..6 {
        ctick(&mut w, &mut c);
        w.land();
    }
    ctick(&mut w, &mut c);
    let ld = w.ledger();
    assert!(ld.tickets[&t1].undeliverable && ld.tickets[&t2].undeliverable, "{:?}", w.sys_log());
    assert!(w.sys_log().is_empty(), "no transaction: {:?}", w.sys_log());
}


// =====================================================================================================================================
// W8 (docs/v9/canister-only-solana.md §8): the frog buy on the BOOKS home: the advance `M → K_main` signed by the collection's key
// (the books advanced first), the Relay leg unchanged, the item's mint and the settlement book updates, final at once.
// =====================================================================================================================================

#[test]
fn books_a_frog_buy_end_to_end_advance_relay_sol_to_btc_buy_custody_mint_item_settle_in_the_books() {
    let mut w = World::books();
    let (mut st, mut fl) = w.states();
    let module_before = w.chain.borrow().accounts[&books_module()].lamports;
    let order_id = to_converting(&mut w, &mut st, &mut fl);
    assert_eq!(w.chain.borrow().accounts[&books_module()].lamports, module_before - ADVANCE, "the advance was paid from the collection key's account");
    assert_eq!((w.bmodule().transit_units, w.bmodule().advanced), (ONE, ADVANCE), "advanced in the books");
    {
        let sw = w.sys.borrow();
        assert_eq!((sw.deposits[0].1, sw.deposits[0].2), (5_000_000_000, order_id), "our own deposit_native, the judged amount and order");
    }
    w.relay_fills(658_506, 3);
    let l = tick(&mut w, &mut st, &mut fl);
    assert!(l.iter().any(|x| x.contains("ARRIVED")), "{l:?}");
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(2), seller);
    let o = w.offer_for(0, &insc(2), seller, PRICE, w.proofs[1].clone());
    w.offers.insert(0, o);
    let l = tick(&mut w, &mut st, &mut fl);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!("{:?} {l:?}", phase(&st, 0)) };
    let want_cost = rc::cost_of(ADVANCE as u128, PRICE + 3_000, 658_506).unwrap();
    assert_eq!(buy.cost_wei, want_cost);
    w.mine_btc(1);
    tick(&mut w, &mut st, &mut fl);
    w.mine_btc(5);
    // six deep and in custody: held, and the mint is a book update with the inscription id and its proof checked against the root
    let l = tick(&mut w, &mut st, &mut fl);
    assert!(l.iter().any(|x| x.contains("minted to the module in the books")), "{l:?}");
    assert_eq!(w.land().len(), 0, "no transaction for the mint");
    let ld = w.ledger();
    let it = ld.items.get(&0).expect("the item");
    assert_eq!((it.inscription_id.as_str(), it.key, it.source_tx, it.vout), (insc(2).as_str(), frogbuy::leaf_of(&insc(2)), buy.outpoint.txid, 1));
    assert_eq!((ld.held, ld.module_units), (1, ONE), "its unit is at the module");
    // minted → the report: settle_advance(cost), a book update
    let l = tick(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Minted { .. }), "{:?} {l:?}", phase(&st, 0));
    assert!(l.iter().any(|x| x.contains("settled at") && x.contains("in the books")), "{l:?}");
    assert_eq!(w.land().len(), 0);
    tick(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Settled { .. }), "{:?}", phase(&st, 0));
    assert!(matches!(conv(&st), CPhase::Settled { cost, spare_sats } if cost == want_cost && spare_sats == 658_506 - PRICE - 3_000));
    let (m, ld) = (w.bmodule(), w.ledger());
    assert_eq!((m.transit_units, m.items_arrived, ld.module_units, ld.sol_backed, ld.held), (0, 1, 0, ONE, 1), "settled: the unit is inscription-backed");
    assert!(w.sys_log().iter().all(|x| x.starts_with("system:") || x == "deposit_native"), "no program instruction: {:?}", w.sys_log());
    let value = rates::lamports_to_wei(ADVANCE as u128, LAMPORTS_PER_ETH);
    assert!(w.outflow.iter().any(|(k, f, v)| k == "buy:frog:0" && *f == outflow::Flow::Value && *v == value), "{:?}", w.outflow);
    assert!(w.book.contains(&insc(2)), "custody's book holds the inscription");
    assert!(st.pending.is_none() && st.sol_step.is_none());
}

/// W8: a resolved frog ticket in the books, as a redemption the books' `redeem`/`resolve` would leave (the item held, the ticket Resolved).
fn books_resolved_ticket(inscription: &str, dest: etch_autobuyer::books::ledger::Destination, deliver_by: i64) -> u64 {
    books_resolved_ticket_in(7, inscription, dest, deliver_by)
}

/// [`books_resolved_ticket`] in collection `coll`.
fn books_resolved_ticket_in(coll: u16, inscription: &str, dest: etch_autobuyer::books::ledger::Destination, deliver_by: i64) -> u64 {
    use etch_autobuyer::books::ledger::{Item, Stage, Ticket};
    etch_autobuyer::desk::state::with_mut(|s| {
        let c = s.books.collections.get_mut(&coll).unwrap();
        let l = &mut c.ledger;
        let item_id = l.item_count;
        let key = frogbuy::leaf_of(inscription);
        l.items.insert(item_id, Item { item_id, key, source_tx: [0x51; 32], vout: 1, deposit: false, inscription_id: inscription.to_string() });
        l.held_markers.insert(key, item_id);
        let tid = l.ticket_count;
        l.tickets.insert(tid, Ticket { id: tid, stage: Stage::Resolved, owner: [0xa1; 32], round: 9, pinned_at: deliver_by - 7 * 86_400 - 60, deliver_by, item_id,
            delivery_tx: [0; 32], sent_tx: [0; 32], undeliverable: false, destination: dest });
        l.item_count += 1;
        l.held += 1;
        l.ticket_count += 1;
        l.open_tickets += 1;
        l.next_to_resolve += 1;
        tid
    })
}

#[test]
fn books_a_frog_redemption_from_the_books_ticket_is_sent_noted_and_recorded_in_the_books() {
    let mut w = World::books();
    let frog = Outpoint { txid: [0xf0; 32], vout: 0 };
    w.loc.insert(insc(3), Location { output: frog, offset: 0, value: 546 });
    w.contents.insert(frog, Contents { inscriptions: vec![insc(3)], runes: vec![], value: 546 });
    w.outputs.insert(frog, (Output { script: custody(), value: 546, confirmed: true }, false));
    w.utxos.push(Utxo { outpoint: frog, value: 546, confirmed: true });
    let t = books_resolved_ticket(&insc(3), etch_autobuyer::books::ledger::Destination::Taproot(DEST.into()), NOW as i64 + 7 * 86_400);
    let mut c = Custody::default();
    let l = ctick(&mut w, &mut c);
    let r = c.redemptions.iter().find(|r| r.ticket == t).unwrap().clone();
    assert_eq!((r.acquisition_id, r.destination.as_str()), (0, DEST), "the ticket's item and its taproot destination, from the books: {l:?}");
    let RPhase::Signed { txid, raw, .. } = r.phase.clone() else { panic!("{:?} {l:?}", r.phase) };
    let tx = parse_signed(&raw).unwrap();
    assert_eq!(tx.outputs[0], (546, etch_autobuyer::policy::script_of(DEST, "bc").unwrap()), "the frog to the ticket's destination");
    // a mempool has it: note_sent is a book update, read back from the ticket at the next pass
    let l = ctick(&mut w, &mut c);
    assert!(l.iter().any(|x| x.contains("noted sent in the books")), "{l:?}");
    assert_eq!(w.land().len(), 0, "no transaction");
    assert_eq!(w.ledger().tickets[&t].sent_tx, txid);
    ctick(&mut w, &mut c);
    assert_eq!(c.redemptions[0].noted, Some(true));
    // confirmed on Bitcoin: record_delivery in the books, then Done from the ticket
    w.mine_btc(1);
    let l = ctick(&mut w, &mut c);
    assert!(matches!(c.redemptions[0].phase, RPhase::Confirmed { .. }), "{:?} {l:?}", c.redemptions[0].phase);
    let l = ctick(&mut w, &mut c);
    assert!(l.iter().any(|x| x.contains("recorded delivered in the books")), "{l:?}");
    ctick(&mut w, &mut c);
    assert_eq!(c.redemptions[0].phase, RPhase::Done { txid });
    let ld = w.ledger();
    let tk = &ld.tickets[&t];
    assert_eq!((tk.stage, tk.delivery_tx), (etch_autobuyer::books::ledger::Stage::Delivered, txid));
    assert_eq!((ld.held, ld.open_tickets), (0, 0));
    assert!(w.sys_log().is_empty(), "no program instruction: {:?}", w.sys_log());
    assert!(w.outflow.iter().any(|(k, f, _)| k == &format!("ticket:{}:{t}", FROG_WX[0]) && *f == outflow::Flow::Item));
    // inside the margin a ticket is refused and marked undeliverable in the books
    let t2 = books_resolved_ticket(&insc(4), etch_autobuyer::books::ledger::Destination::Taproot(DEST.into()), NOW as i64 + 86_400);
    ctick(&mut w, &mut c);
    let r2 = c.redemptions.iter().find(|r| r.ticket == t2).unwrap().clone();
    assert!(matches!(r2.phase, RPhase::Refused { ref reason } if reason.contains("InsideMargin")), "{:?}", r2.phase);
    let l = ctick(&mut w, &mut c);
    assert!(l.iter().any(|x| x.contains("marked undeliverable in the books")), "{l:?}");
    ctick(&mut w, &mut c);
    let r2 = c.redemptions.iter().find(|r| r.ticket == t2).unwrap().clone();
    assert!(matches!(r2.attest, Some(redeem::Attest::Done { .. })), "{:?}", r2.attest);
    assert!(w.ledger().tickets[&t2].undeliverable);
}

// =====================================================================================================================================
// Ordinals multi-collection (docs/v9/ordinals-collections.md §4, built 2026-10-12 on the books home): two Inscriptions listings live at
// once, each with its own state, its own books, its own floor and its own redemptions; one custody address, one Solana line, one Relay
// leg, frog ids unique canister-wide. Collection 7 is this file's frogs; collection 12 is NodeMonkes with its real members root and a
// real member and proof (tests/fixtures/ordinals/multi.json, from deployments/launch/btc/nodemonkes-members.json).
// =====================================================================================================================================

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

/// Two floor readings, one per collection, each posted in its own books; each collection's resolve takes only its own outcome.
#[test]
fn multi_two_floor_readings_each_posted_in_its_own_books_and_each_outcome_taken_by_its_collection() {
    let mut w = World::books_two();
    w.now += 3_600;
    let mut fl = World::floors_two();
    // no floor sink: the states only post (the buy is the next test's)
    let (mut st7, mut st12) = (FrogState::default(), FrogState::default());
    st7.floor_reading = Some(frogsol::FrogReading { sats: 640_000, confidence_bps: 8_000, depth_items: 10, at: w.now });
    st12.floor_reading = Some(frogsol::FrogReading { sats: 655_000, confidence_bps: 7_000, depth_items: 6, at: w.now });
    let l = tick(&mut w, &mut st7, &mut fl);
    assert!(l.iter().any(|x| x.contains("post_floor")), "{l:?}");
    w.wx = MONKE_WX;
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(l.iter().any(|x| x.contains("post_floor")), "{l:?}");
    assert_eq!(w.book_outcomes(), vec!["books:7".to_string(), "books:12".to_string()], "collection 12 left collection 7's outcome");
    let (m7, m12) = (w.module_of(7), w.module_of(MONKES));
    assert_eq!((m7.posted_at, m12.posted_at), (w.now as i64, w.now as i64), "both floors posted in their own books");
    assert_eq!((m7.confidence_bps, m7.depth_items, m12.confidence_bps, m12.depth_items), (8_000, 10, 7_000, 6));
    assert!(st7.floor_reading.is_some() && st12.floor_reading.is_some(), "neither has read its outcome yet");
    w.wx = FROG_WX;
    tick(&mut w, &mut st7, &mut fl);
    assert_eq!(w.book_outcomes(), vec!["books:12".to_string()], "collection 7 took its own, and only its own");
    assert!(st7.floor_reading.is_none() && st12.floor_reading.is_some());
    w.wx = MONKE_WX;
    tick(&mut w, &mut st12, &mut fl);
    assert!(st12.floor_reading.is_none() && w.book_outcomes().is_empty());
}

/// A buy per collection, each into its own books and its own custody cap, while the other collection is live: collection 7 through Relay
/// (this file's end-to-end), collection 12 from the float (`float_first`) with its real NodeMonke and proof; frog ids unique; each
/// collection's book outcomes taken by it alone.
#[test]
fn multi_a_buy_in_each_of_two_live_collections_lands_in_its_own_books() {
    let mut w = World::books_two();
    let mut fl = World::floors_two();
    let mut st7 = World::state_for(&books_module(), 0);
    // collection 7: the Relay buy to settled
    to_converting(&mut w, &mut st7, &mut fl);
    w.relay_fills(658_506, 3);
    tick(&mut w, &mut st7, &mut fl);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(2), seller);
    let o = w.offer_for(0, &insc(2), seller, PRICE, w.proofs[1].clone());
    w.offers.insert(0, o);
    tick(&mut w, &mut st7, &mut fl);
    w.mine_btc(1);
    tick(&mut w, &mut st7, &mut fl);
    w.mine_btc(5);
    for _ in 0..3 {
        tick(&mut w, &mut st7, &mut fl);
    }
    assert!(matches!(phase(&st7, 0), FPhase::Settled { .. }), "{:?}", phase(&st7, 0));
    // collection 12: the next frog id, bought from the float
    w.wx = MONKE_WX;
    w.funding = rc::FrogFunding { max_fee_bps: 300, float_first: true };
    w.float_outs.push(Utxo { outpoint: Outpoint { txid: [0x60; 32], vout: 0 }, value: 20_000_000, confirmed: true });
    let mut st12 = World::state_for(&monke_module(), st7.next_frog);
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(matches!(phase(&st12, 1), FPhase::Wanted { .. }), "frog 1, never 0 again: {l:?}");
    assert!(st12.convs().is_empty(), "the float covers it: no advance");
    let (_, monke, proof) = fixture("nodemonkes");
    let seller = Outpoint { txid: [0xe2; 32], vout: 0 };
    w.list(&monke, seller);
    let o = w.offer_for(1, &monke, seller, PRICE, proof);
    w.offers.insert(1, o);
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(matches!(phase(&st12, 1), FPhase::Signed { .. }), "{:?} {l:?}", phase(&st12, 1));
    w.mine_btc(1);
    tick(&mut w, &mut st12, &mut fl);
    w.mine_btc(5);
    let l = tick(&mut w, &mut st12, &mut fl);
    assert!(l.iter().any(|x| x.contains("minted to the module in the books")), "{l:?}");
    assert_eq!(w.book_outcomes(), vec!["books:12".to_string()]);
    // collection 7 ticks meanwhile: it does not take collection 12's outcome
    w.wx = FROG_WX;
    tick(&mut w, &mut st7, &mut fl);
    assert_eq!(w.book_outcomes(), vec!["books:12".to_string()]);
    w.wx = MONKE_WX;
    for _ in 0..3 {
        tick(&mut w, &mut st12, &mut fl);
    }
    assert!(st12.frogs[0].finished(), "{:?}", phase(&st12, 1));
    // each item in its own books, proved against its own root
    let (l7, l12) = (w.ledger_of(7), w.ledger_of(MONKES));
    assert_eq!((l7.items[&0].inscription_id.as_str(), l7.held), (insc(2).as_str(), 1));
    assert_eq!((l12.items[&0].inscription_id.as_str(), l12.held), (monke.as_str(), 1));
    // and in its own custody cap: one item each, keyed by its wX mint; collection 12's cap (1 item) is full
    let held = |k: &str| w.book.entries.iter().filter(|e| e.collection == k).count();
    assert_eq!((held(&custodycap::sol_key(&FROG_WX)), held(&custodycap::sol_key(&MONKE_WX))), (1, 1));
    assert!(custodycap::check_acquire(&w.caps, &w.book, &custodycap::sol_key(&MONKE_WX), 1).is_err(), "NodeMonkes' one item is held");
    assert!(custodycap::check_acquire(&w.caps, &w.book, &custodycap::sol_key(&FROG_WX), 1).is_ok(), "the frogs' room is their own");
}

/// Rule S12 is canister-wide: a Relay order one collection's conversion used is never judged again for another's.
#[test]
fn multi_a_relay_order_one_collection_used_is_refused_for_another() {
    let mut w = World::books_two();
    let mut fl = World::floors_two();
    let mut st7 = World::state_for(&books_module(), 0);
    to_converting(&mut w, &mut st7, &mut fl);
    w.relay_fills(658_506, 3);
    tick(&mut w, &mut st7, &mut fl);
    assert!(matches!(conv(&st7), CPhase::Arrived { .. }), "{:?}", conv(&st7));
    // collection 12 converts with the same quote: its state never saw the order, the canister has
    w.wx = MONKE_WX;
    w.used_other = rc::used_orders(st7.convs());
    let mut st12 = World::state_for(&monke_module(), st7.next_frog);
    tick(&mut w, &mut st12, &mut fl);
    assert_eq!(w.land().len(), 1, "collection 12's advance lands");
    tick(&mut w, &mut st12, &mut fl);
    assert!(matches!(st12.conv(1).unwrap().phase, CPhase::Funded { .. }), "{:?}", st12.conv(1).unwrap().phase);
    w.quotes.insert(1, (relay_quote(SOL_BTC_5), w.now));
    tick(&mut w, &mut st12, &mut fl);
    let c = st12.conv(1).unwrap();
    assert!(matches!(c.phase, CPhase::Funded { .. }) && c.note.contains("rule S12"), "{:?} {}", c.phase, c.note);
    assert!(w.sys.borrow().deposits.len() == 1, "no second deposit");
}

/// A redemption per collection: each collection's ticket 0 read from its own books, delivered from the one custody address with a fee
/// input the other collection's delivery has not claimed, counted under its own key, recorded in its own books.
#[test]
fn multi_each_collections_ticket_is_delivered_and_recorded_in_its_own_books() {
    use etch_autobuyer::books::ledger::{Destination, Stage};
    let mut w = World::books_two();
    let fee2 = Outpoint { txid: [0xfd; 32], vout: 0 };
    w.utxos.push(Utxo { outpoint: fee2, value: 20_000, confirmed: true });
    w.contents.insert(fee2, Contents { inscriptions: vec![], runes: vec![], value: 20_000 });
    w.outputs.insert(fee2, (Output { script: custody(), value: 20_000, confirmed: true }, false));
    w.txs.insert([0xfd; 32], BtcTx { height: Some(w.tip), hash: Some(w.blocks[&w.tip].clone()) });
    for (n, id) in [(0xf0u8, insc(3)), (0xf1, insc(5))] {
        let o = Outpoint { txid: [n; 32], vout: 0 };
        w.loc.insert(id.clone(), Location { output: o, offset: 0, value: 546 });
        w.contents.insert(o, Contents { inscriptions: vec![id.clone()], runes: vec![], value: 546 });
        w.outputs.insert(o, (Output { script: custody(), value: 546, confirmed: true }, false));
        w.utxos.push(Utxo { outpoint: o, value: 546, confirmed: true });
    }
    let t7 = books_resolved_ticket_in(7, &insc(3), Destination::Taproot(DEST.into()), NOW as i64 + 7 * 86_400);
    let t12 = books_resolved_ticket_in(MONKES, &insc(5), Destination::Taproot(DEST.into()), NOW as i64 + 7 * 86_400);
    assert_eq!((t7, t12), (0, 0), "tickets are numbered per listing");
    let (mut c7, mut c12) = (Custody::default(), Custody::default());
    let l = ctick(&mut w, &mut c7);
    let RPhase::Signed { txid: tx7, fee_input: fee7, .. } = c7.redemptions[0].phase.clone() else { panic!("{:?} {l:?}", c7.redemptions[0].phase) };
    // collection 12's custodian: its own ticket 0, its own item, and a fee input collection 7's delivery has not claimed
    w.wx = MONKE_WX;
    w.reserved_other = c7.redemptions.iter().flat_map(|r| r.reserved()).collect();
    let l = ctick(&mut w, &mut c12);
    let r = c12.redemptions[0].clone();
    assert_eq!((r.ticket, r.inscription.as_deref()), (0, Some(insc(5).as_str())), "{l:?}");
    let RPhase::Signed { txid: tx12, fee_input: fee12, .. } = r.phase.clone() else { panic!("{:?} {l:?}", r.phase) };
    assert_ne!(fee7, fee12, "one custody address: the other collection's fee input is not chosen");
    assert_ne!(tx7, tx12);
    // each counted under its own key: two collections' ticket 0 are two deliveries
    let keys: Vec<&String> = w.outflow.iter().filter(|(_, f, _)| *f == outflow::Flow::Item).map(|(k, _, _)| k).collect();
    assert!(keys.contains(&&format!("ticket:{}:0", FROG_WX[0])) && keys.contains(&&format!("ticket:{}:0", MONKE_WX[0])), "{keys:?}");
    // noted, confirmed, recorded: each in its own books
    for _ in 0..2 {
        w.wx = FROG_WX;
        ctick(&mut w, &mut c7);
        w.wx = MONKE_WX;
        ctick(&mut w, &mut c12);
    }
    w.mine_btc(1);
    for _ in 0..3 {
        w.wx = FROG_WX;
        ctick(&mut w, &mut c7);
        w.wx = MONKE_WX;
        ctick(&mut w, &mut c12);
    }
    assert_eq!((c7.redemptions[0].phase.clone(), c12.redemptions[0].phase.clone()), (RPhase::Done { txid: tx7 }, RPhase::Done { txid: tx12 }));
    let (k7, k12) = (w.ledger_of(7).tickets[&0].clone(), w.ledger_of(MONKES).tickets[&0].clone());
    assert_eq!((k7.stage, k7.delivery_tx, k12.stage, k12.delivery_tx), (Stage::Delivered, tx7, Stage::Delivered, tx12));
}

/// Custody's classification through the frog world: each new custody output read once (ord's contents), a Frog and a NodeMonke handed
/// in with their real proofs routed to their own collections, a stranger's inscription refused once.
#[test]
fn multi_custody_classifies_through_the_world_once() {
    use etch_autobuyer::frogsol::multi;
    let mut w = World::books_two();
    let (froot, frog, fproof) = fixture("bitcoin-frogs");
    let (mroot, monke, mproof) = fixture("nodemonkes");
    for (n, id) in [(0xb1u8, frog.clone()), (0xb2, monke.clone()), (0xb3, insc(9))] {
        let o = Outpoint { txid: [n; 32], vout: 0 };
        w.contents.insert(o, Contents { inscriptions: vec![id], runes: vec![], value: 546 });
        w.utxos.push(Utxo { outpoint: o, value: 546, confirmed: true });
    }
    let mut top = FrogState::default();
    multi::entry_mut(&mut top, 7).put_proof(frog, fproof);
    multi::entry_mut(&mut top, MONKES).put_proof(monke, mproof);
    let roots = [(7, froot), (MONKES, mroot)];
    let (unspent, routes, failed) = block_on(multi::classify(&mut w, &top, &roots)).unwrap();
    assert!(failed.is_empty());
    let lines = multi::apply(&mut top, &unspent, routes);
    let at = |n: u8| Outpoint { txid: [n; 32], vout: 0 };
    assert_eq!(multi::entry(&top, 7).unwrap().unmarked, vec![at(0xb1)]);
    assert_eq!(multi::entry(&top, MONKES).unwrap().unmarked, vec![at(0xb2)]);
    assert_eq!(top.unmarked, vec![at(0xb3)]);
    assert_eq!(lines.iter().filter(|l| l.contains("refused once")).count(), 1, "{lines:?}");
    assert_eq!(top.seen.len(), 4, "the plain fee input too");
    let (_, again, _) = block_on(multi::classify(&mut w, &top, &roots)).unwrap();
    assert!(again.is_empty(), "nothing is read twice");
}
