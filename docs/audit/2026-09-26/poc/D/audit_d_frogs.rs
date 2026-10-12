//! AUDIT D PoC harness: a verbatim copy of tests/frogs.rs's fake world (lines 1-730 and its deposit helpers), instrumented
//! with a read counter and a snapshot of the saved state at every Bitcoin send. The PoC tests are at the bottom.
//!
//! Graduation buys through item sinks (a collection's buy queue; a graduating curve), the mint desk and holder deposits
//! (docs/v3/curve.md, "Canister: …") against a fake world: contracts that check what the real ones check, a Bitcoin that
//! confirms, reorganises and loses listings, ord and explorers that say where an inscription is, and a relayer that hands in
//! offers good and bad.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::btcnet::BtcSeen;
use etch_autobuyer::btcsend::parse_signed;
use etch_autobuyer::custody::{Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::deposits::{self, DPhase, InView, OutView, Reason, TxView};
use etch_autobuyer::evmtx;
use etch_autobuyer::frogbuy::{self, FPhase, Target};
use etch_autobuyer::frogflow::{self, FStep, FrogIo, FrogState, Offer};
use etch_autobuyer::itemsink::{Clock, SinkKind, SinkRec};
use etch_autobuyer::looper::EvmView;
use etch_autobuyer::psbt::{Psbt, Tx, TxIn, TxOut};
use etch_autobuyer::{buyqueue, curvebuy, mintdesk};
use etch_autobuyer::floorsink;
use etch_autobuyer::floortrigger::{FloorItem, FloorState, WPhase};
use etch_autobuyer::{custodycap, outbound, outflow};
use std::cell::{Cell, RefCell};

thread_local! {
    /// audit D: reads the flow makes of the world (each is one or more replicated HTTPS outcalls on the IC)
    static READS: Cell<usize> = const { Cell::new(0) };
    /// audit D: for every Bitcoin send, the deposit phases as SAVED (durable) at the moment of the send
    static AT_SEND: RefCell<Vec<Vec<(Outpoint, String)>>> = const { RefCell::new(Vec::new()) };
}
fn read() { READS.with(|r| r.set(r.get() + 1)); }

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

const E: u128 = 1_000_000_000_000_000_000;
const T0: u64 = 1_800_000_000;
const B0: u64 = 26_000_000;
const ME: [u8; 20] = [0xcc; 20];
const W: [u8; 20] = [0xaa; 20];
const REG: [u8; 20] = [0xab; 20];
const QUEUE: [u8; 20] = [0x51; 20];
const CURVE: [u8; 20] = [0x52; 20];
const COIN: [u8; 20] = [0x53; 20];
const FACTORY: [u8; 20] = [0x54; 20];
const DESK: [u8; 20] = [0x55; 20];
/// A Floor Module for frogs (docs/v5/floor-feed.md), and one for apes on chain 1 wrapped by an NftWrapper.
const FLOOR: [u8; 20] = [0x56; 20];
const FLOOR_EVM: [u8; 20] = [0x57; 20];
const NFTW: [u8; 20] = [0x58; 20];
const APE: [u8; 20] = [0xbc; 20];
const RECIPIENT: [u8; 20] = [0xbb; 20];
const RATE: u64 = 3_190_000; // sats per ETH
const PRICE: u64 = 900_000; // sats to the seller
const CHAIN: u64 = 4663;

fn p2tr(tag: u8) -> Vec<u8> {
    let mut s = vec![0x51, 0x20];
    s.extend([tag; 32]);
    s
}
fn float() -> Vec<u8> {
    p2tr(0xf1)
}
fn custody() -> Vec<u8> {
    p2tr(0xc2)
}
fn insc(n: u8) -> String {
    format!("{}i0", hex::encode([n; 32]))
}
fn word(v: u128) -> Vec<u8> {
    let mut w = vec![0u8; 16];
    w.extend(v.to_be_bytes());
    w
}
fn aword(a: &[u8; 20]) -> Vec<u8> {
    let mut w = vec![0u8; 12];
    w.extend(a);
    w
}

// ---- our signed EVM transactions, read back -------------------------------------------------------------------------------

fn rlp_items(b: &[u8]) -> Vec<Vec<u8>> {
    fn item(b: &[u8], i: &mut usize) -> Vec<u8> {
        let p = b[*i];
        let (start, len) = match p {
            0x00..=0x7f => (*i, 1),
            0x80..=0xb7 => (*i + 1, (p - 0x80) as usize),
            0xb8..=0xbf => { let n = (p - 0xb7) as usize; (*i + 1 + n, b[*i + 1..*i + 1 + n].iter().fold(0, |a, &x| a * 256 + x as usize)) }
            0xc0..=0xf7 => (*i + 1, (p - 0xc0) as usize),
            _ => { let n = (p - 0xf7) as usize; (*i + 1 + n, b[*i + 1..*i + 1 + n].iter().fold(0, |a, &x| a * 256 + x as usize)) }
        };
        *i = start + len;
        b[start..start + len].to_vec()
    }
    let mut i = 1;
    let list = item(b, &mut i);
    let mut j = 0;
    let mut out = Vec::new();
    while j < list.len() {
        out.push(item(&list, &mut j));
    }
    out
}
fn uint(b: &[u8]) -> u128 {
    b.iter().fold(0u128, |a, &x| a * 256 + x as u128)
}
fn arg(data: &[u8], n: usize) -> u128 {
    u128::from_be_bytes(data[4 + 32 * n + 16..4 + 32 * n + 32].try_into().unwrap())
}
fn arg_word(data: &[u8], n: usize) -> [u8; 32] {
    data[4 + 32 * n..4 + 32 * n + 32].try_into().unwrap()
}

// ---- the world ---------------------------------------------------------------------------------------------------------------

#[derive(Clone)]
struct QueueC {
    queued: u128,
    cap: u128,
    buffer: u128,
    recorded: u64,
    head: (u64, u64),
    head_amount: u128,
    payee: [u8; 20],
}
#[derive(Clone)]
struct CurveC {
    state: u8,
    n: u64,
    k: u64,
    spent: u128,
    budget: u128,
    buffer: u128,
    deadline: (u64, u64),
}
#[derive(Clone)]
struct Req {
    status: u8,
    requester: [u8; 20],
    at: u64,
    at_block: u64,
    amount: u128,
    cost: u128,
}

struct BtcTx {
    height: Option<u64>,
    /// The block the explorers name for it: kept from when it was mined, so a replaced block shows as a mismatch.
    hash: Option<String>,
    view: Option<TxView>,
}

struct World {
    now: u64,
    clock: Clock,
    custodian: [u8; 20],
    root: [u8; 32],
    minted: HashSet<[u8; 32]>,
    held: HashSet<[u8; 32]>,
    balances: HashMap<[u8; 20], u128>,
    paid: u128,
    queue: QueueC,
    curve: Option<CurveC>,
    desk_ttl: u64,
    requests: Vec<Req>,
    desk_assigned: u128,
    nonce: u64,
    mempool: Vec<Vec<u8>>,
    receipts: HashMap<[u8; 32], bool>,
    calls: Vec<([u8; 20], Vec<u8>, bool)>,
    tip: u64,
    blocks: HashMap<u64, String>,
    txs: HashMap<[u8; 32], BtcTx>,
    loc: HashMap<String, Location>,
    contents: HashMap<Outpoint, Contents>,
    outputs: HashMap<Outpoint, (Output, bool)>,
    outspends: HashMap<Outpoint, ([u8; 32], bool)>,
    utxos: Vec<Utxo>,
    btc_sent: Vec<Vec<u8>>,
    offers: HashMap<u64, Offer>,
    saved: Option<FrogState>,
    floor: FloorC,
    floor_evm: FloorC,
    /// NftWrapper units held by the EVM module (minted to it by the NFT wrapper's custodian mint, which this branch does not build).
    nft_units_at_module: u128,
    saved_floor: Option<FloorState>,
    /// H2: the caps in force and the book (generous by default: the tests below that are about the caps set their own)
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
    /// H3: the limiter when limits are set (None: every act allowed), and every ask and record by key
    limits: Option<outflow::Limits>,
    outflow: outflow::Outflow,
    asked: Vec<(String, outflow::Flow)>,
    recorded: Vec<String>,
    paused: bool,
}

fn frogs_key() -> String {
    custodycap::btc_key(&W)
}

fn apes_key() -> String {
    custodycap::evm_key(1, &APE)
}

fn generous() -> custodycap::CapPolicy {
    let c = |k: String| custodycap::CollectionCap { collection: k, max_items: 1_000, max_value_wei: 1_000_000 * E, max_item_wei: 1_000 * E };
    custodycap::CapPolicy { collections: vec![c(frogs_key()), c(apes_key())], global_max_items: 1_000, global_max_value_wei: 1_000_000 * E }
}

/// The Floor Module as the canister assumes it (floorsink.rs): net ETH-backed units, the escrow, the posted floor.
#[derive(Clone)]
struct FloorC {
    net: u128,
    escrow: u128,
    buffer: u128,
    price: u128,
    posted_at: u64,
    ttl: u64,
    max_cost: u128,
    arrived: u64,
}

fn block_hash_for(h: u64, salt: u8) -> String {
    format!("{:02x}{}", salt, hex::encode(evmtx::keccak(&h.to_be_bytes()))[2..].to_string())
}

impl World {
    fn new() -> Self {
        let mut w = World {
            now: T0,
            clock: Clock { time: T0, parent_block: B0 },
            custodian: ME,
            root: [0; 32],
            minted: HashSet::new(),
            held: HashSet::new(),
            balances: HashMap::new(),
            paid: 0,
            queue: QueueC { queued: 0, cap: 0, buffer: 0, recorded: 0, head: (0, 0), head_amount: 0, payee: ME },
            curve: None,
            desk_ttl: 24 * 3600,
            requests: vec![],
            desk_assigned: 0,
            nonce: 0,
            mempool: vec![],
            receipts: HashMap::new(),
            calls: vec![],
            tip: 800_000,
            blocks: HashMap::new(),
            txs: HashMap::new(),
            loc: HashMap::new(),
            contents: HashMap::new(),
            outputs: HashMap::new(),
            outspends: HashMap::new(),
            utxos: vec![],
            btc_sent: vec![],
            offers: HashMap::new(),
            saved: None,
            floor: FloorC { net: 0, escrow: 0, buffer: 0, price: 0, posted_at: 0, ttl: 0, max_cost: 0, arrived: 0 },
            floor_evm: FloorC { net: 0, escrow: 0, buffer: 0, price: 0, posted_at: 0, ttl: 0, max_cost: 0, arrived: 0 },
            nft_units_at_module: 0,
            saved_floor: None,
            caps: generous(),
            book: custodycap::Book::default(),
            limits: None,
            outflow: outflow::Outflow::default(),
            asked: vec![],
            recorded: vec![],
            paused: false,
        };
        // custody's plain float for return fees: two outputs, each its own transaction, mined at the tip
        w.blocks.insert(w.tip, block_hash_for(w.tip, 0));
        for tag in [0xfe, 0xfd] {
            let fee = Outpoint { txid: [tag; 32], vout: 0 };
            w.utxos.push(Utxo { outpoint: fee, value: 20_000, confirmed: true });
            w.contents.insert(fee, Contents { inscriptions: vec![], runes: vec![], value: 20_000 });
            w.outputs.insert(fee, (Output { script: custody(), value: 20_000, confirmed: true }, false));
            w.txs.insert([tag; 32], BtcTx { height: Some(w.tip), hash: Some(w.blocks[&w.tip].clone()), view: None });
        }
        w
    }
    fn advance(&mut self, secs: u64) {
        self.now += secs;
        self.clock.time += secs;
        self.clock.parent_block += secs / 12;
    }
    /// A collection with these inscriptions: the root of a tree of their leaves (sorted pairs), and each one's proof.
    fn collection(&mut self, ids: &[String]) -> Vec<Vec<[u8; 32]>> {
        assert!(ids.len() == 2 || ids.len() == 4);
        let pair = |a: [u8; 32], b: [u8; 32]| { let (x, y) = if a <= b { (a, b) } else { (b, a) }; evmtx::keccak(&[x, y].concat()) };
        let l: Vec<[u8; 32]> = ids.iter().map(|i| frogbuy::leaf_of(i)).collect();
        if l.len() == 2 {
            self.root = pair(l[0], l[1]);
            return vec![vec![l[1]], vec![l[0]]];
        }
        let (a, b) = (pair(l[0], l[1]), pair(l[2], l[3]));
        self.root = pair(a, b);
        vec![vec![l[1], b], vec![l[0], b], vec![l[3], a], vec![l[2], a]]
    }
    /// A seller's listed frog at `seller`, as ord and the explorers see it.
    fn list(&mut self, id: &str, seller: Outpoint) {
        self.loc.insert(id.to_string(), Location { output: seller, offset: 0, value: 10_000 });
        self.contents.insert(seller, Contents { inscriptions: vec![id.to_string()], runes: vec![], value: 10_000 });
        self.outputs.insert(seller, (Output { script: p2tr(0xee), value: 10_000, confirmed: true }, false));
    }
    /// The marketplace's PSBT for it: padding, the frog, a payment input; out: padding, the frog to custody, the price, change.
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
        let finals = vec![None, None, Some(vec![vec![7u8; 65]]), None];
        Offer { inscription: id.to_string(), psbt: frogbuy::psbt_bytes(&p, &finals), to_sign: vec![0, 1, 3], proof }
    }
    /// Put the next `n` blocks on the tip, confirming everything in the mempool into the first; a purchase moves its frog.
    fn mine_btc(&mut self, n: u64) {
        for k in 0..n {
            self.tip += 1;
            let h = self.tip;
            self.blocks.insert(h, block_hash_for(h, 0));
            if k > 0 {
                continue;
            }
            let pending: Vec<[u8; 32]> = self.txs.iter().filter(|(_, t)| t.height.is_none()).map(|(id, _)| *id).collect();
            for id in pending {
                self.confirm(id, h);
                self.settle_btc(id);
            }
        }
    }
    fn confirm(&mut self, txid: [u8; 32], h: u64) {
        let hash = self.blocks[&h].clone();
        let t = self.txs.get_mut(&txid).unwrap();
        t.height = Some(h);
        t.hash = Some(hash);
    }
    fn unconfirm(&mut self, txid: [u8; 32]) {
        let t = self.txs.get_mut(&txid).unwrap();
        t.height = None;
        t.hash = None;
    }
    /// A confirmed transaction of ours: what it spent leaves custody's UTXOs; a purchase moves its frog to custody (output 1).
    fn settle_btc(&mut self, txid: [u8; 32]) {
        let Some(raw) = self.btc_sent.iter().find(|r| display(parse_signed(r).unwrap().txid) == txid).cloned() else { return };
        let tx = parse_signed(&raw).unwrap();
        let spent: Vec<Outpoint> = tx.inputs.iter().map(|i| Outpoint { txid: display(i.txid), vout: i.vout }).collect();
        self.utxos.retain(|u| !spent.contains(&u.outpoint));
        if tx.outputs.get(1) != Some(&(10_000, custody())) {
            return;
        }
        for i in &tx.inputs {
            let mut t = i.txid;
            t.reverse();
            let spent = Outpoint { txid: t, vout: i.vout };
            if let Some((id, _)) = self.loc.iter().find(|(_, l)| l.output == spent).map(|(a, b)| (a.clone(), b.clone())) {
                let at = Outpoint { txid, vout: 1 };
                self.loc.insert(id.clone(), Location { output: at, offset: 0, value: 10_000 });
                self.contents.insert(at, Contents { inscriptions: vec![id.clone()], runes: vec![], value: 10_000 });
                self.outputs.insert(at, (Output { script: custody(), value: 10_000, confirmed: true }, false));
                self.utxos.push(Utxo { outpoint: at, value: 10_000, confirmed: true });
                self.outspends.insert(spent, (txid, true));
                if let Some(o) = self.outputs.get_mut(&spent) {
                    o.1 = true;
                }
            }
        }
    }
    /// A holder's deposit transaction, mined `deep` blocks deep, its output 0 carrying `carries` at custody.
    fn deposit(&mut self, tag: u8, outputs: Vec<OutView>, carries: Contents, inputs: Vec<InView>, deep: u64) -> Outpoint {
        let txid = [tag; 32];
        let o = Outpoint { txid, vout: 0 };
        self.mine_btc(0);
        let h = self.tip + 1;
        let view = TxView { txid, inputs, outputs, confirmed: true, height: Some(h), block_hash: None };
        self.txs.insert(txid, BtcTx { height: None, hash: None, view: Some(view) });
        self.mine_btc(deep);
        if let Some(first) = carries.inscriptions.first() {
            self.loc.insert(first.clone(), Location { output: o, offset: 0, value: carries.value });
        }
        self.outputs.insert(o, (Output { script: custody(), value: carries.value, confirmed: true }, false));
        self.contents.insert(o, carries.clone());
        self.utxos.push(Utxo { outpoint: o, value: carries.value, confirmed: true });
        o
    }
    fn wrapped_units(&self, a: &[u8; 20]) -> u128 {
        self.balances.get(a).copied().unwrap_or(0) / E
    }
    fn expired(&self, r: &Req) -> bool {
        self.clock.time >= r.at + self.desk_ttl || self.clock.parent_block >= r.at_block + self.desk_ttl / 12
    }

    /// Execute one call as the contracts would, applying it or not.
    fn exec(&mut self, to: [u8; 20], data: &[u8], apply: bool) -> Result<(), String> {
        let sel: [u8; 4] = data[..4].try_into().unwrap();
        if to == W && sel == deposits::WRAP_DEPOSIT {
            let s_off = arg(data, 0) as usize + 4;
            let s_len = uint(&data[s_off..s_off + 32]) as usize;
            let id = String::from_utf8(data[s_off + 32..s_off + 32 + s_len].to_vec()).unwrap();
            let p_off = arg(data, 1) as usize + 4;
            let n = uint(&data[p_off..p_off + 32]) as usize;
            let proof: Vec<[u8; 32]> = (0..n).map(|i| data[p_off + 32 + 32 * i..p_off + 64 + 32 * i].try_into().unwrap()).collect();
            let txid = arg_word(data, 2);
            let vout = arg(data, 3) as u32;
            let dest: [u8; 20] = arg_word(data, 4)[12..].try_into().unwrap();
            if !frogbuy::verify_proof(&proof, &self.root, &frogbuy::leaf_of(&id)) { return Err("NotInCollection".into()); }
            let key = deposits::outpoint_key(txid, vout);
            if self.minted.contains(&key) { return Err("AlreadyMinted".into()); }
            if self.held.contains(&frogbuy::leaf_of(&id)) { return Err("AlreadyHeld".into()); }
            if apply {
                self.minted.insert(key);
                self.held.insert(frogbuy::leaf_of(&id));
                *self.balances.entry(dest).or_default() += E;
            }
            return Ok(());
        }
        if to == QUEUE && sel == buyqueue::RECORD_ITEM {
            let cost = arg(data, 0);
            let q = self.queue.clone();
            if self.balances.get(&QUEUE).copied().unwrap_or(0) < (q.recorded as u128 + 1) * E { return Err("NoWrapped".into()); }
            if cost > q.cap || cost > q.queued { return Err("OverCap".into()); }
            if apply {
                self.queue.queued -= cost;
                self.queue.recorded += 1;
                self.paid += cost;
            }
            return Ok(());
        }
        if to == QUEUE && sel == buyqueue::REFUND_EXPIRED {
            let (a, b) = self.queue.head;
            if a == 0 || self.clock.time < a || self.clock.parent_block < b { return Err("TooEarly".into()); }
            if apply {
                self.queue.queued -= self.queue.head_amount.min(self.queue.queued);
                self.queue.head = (0, 0);
            }
            return Ok(());
        }
        if to == CURVE {
            let mut c = self.curve.clone().ok_or("no curve")?;
            if sel == curvebuy::RECORD_FROG {
                let cost = arg(data, 0);
                if c.state != 1 || c.k == c.n { return Err("WrongState".into()); }
                if self.balances.get(&CURVE).copied().unwrap_or(0) < (c.k as u128 + 1) * E { return Err("NoWrapped".into()); }
                if c.spent + cost > c.budget + c.buffer { return Err("OverBudget".into()); }
                c.k += 1;
                c.spent += cost;
                if apply { self.paid += cost; }
            } else if sel == curvebuy::OPEN_REFUNDS {
                if c.state != 1 || c.k == c.n { return Err("WrongState".into()); }
                if self.clock.time < c.deadline.0 || self.clock.parent_block < c.deadline.1 { return Err("TooEarly".into()); }
                c.state = 3;
            } else if sel == curvebuy::GRADUATE {
                if c.state != 1 || c.k != c.n { return Err("WrongState".into()); }
                c.state = 2;
            } else {
                return Err("unknown curve call".into());
            }
            if apply { self.curve = Some(c); }
            return Ok(());
        }
        if (to == FLOOR || to == FLOOR_EVM) && sel == floorsink::ITEM_ARRIVED {
            let cost = arg(data, 0);
            let f = if to == FLOOR { self.floor.clone() } else { self.floor_evm.clone() };
            let held = if to == FLOOR { self.balances.get(&FLOOR).copied().unwrap_or(0) } else { self.nft_units_at_module };
            if held < (f.arrived as u128 + 1) * E { return Err("NoUnit".into()); }
            if cost > f.max_cost || cost > f.escrow { return Err("OverMaxItemCost".into()); }
            if apply {
                let m = if to == FLOOR { &mut self.floor } else { &mut self.floor_evm };
                m.escrow -= cost;
                m.arrived += 1;
                m.net = m.net.saturating_sub(E);
                self.paid += cost;
            }
            return Ok(());
        }
        if to == DESK && sel == mintdesk::FULFILL {
            let (id, cost) = (arg(data, 0) as usize, arg(data, 1));
            let r = self.requests.get(id).cloned().ok_or("no request")?;
            if r.status != 1 { return Err("WrongStatus".into()); }
            if self.expired(&r) { return Err("Expired".into()); }
            if cost > r.amount { return Err("BadInput".into()); }
            if self.balances.get(&DESK).copied().unwrap_or(0) < self.desk_assigned + E { return Err("NoWrapped".into()); }
            if apply {
                self.requests[id].status = 2;
                self.requests[id].cost = cost;
                self.desk_assigned += E;
                self.paid += cost;
            }
            return Ok(());
        }
        Err(format!("unknown call to 0x{}", hex::encode(to)))
    }
    fn mine_evm(&mut self) {
        loop {
            let Some(pos) = self.mempool.iter().position(|r| uint(&rlp_items(r)[1]) as u64 == self.nonce) else { return };
            let raw = self.mempool.remove(pos);
            let f = rlp_items(&raw);
            let to: [u8; 20] = f[5].clone().try_into().unwrap();
            let ok = self.exec(to, &f[7], true).is_ok();
            self.calls.push((to, f[7].clone(), ok));
            self.receipts.insert(evmtx::keccak(&raw), ok);
            self.nonce += 1;
        }
    }
}

fn display(internal: [u8; 32]) -> [u8; 32] {
    let mut d = internal;
    d.reverse();
    d
}

impl FrogIo for World {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { CHAIN }
    fn wrapper(&self) -> [u8; 20] { W }
    fn fresh(&mut self) {}
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        let sel: [u8; 4] = data[..4].try_into().unwrap();
        let b = |x: bool| word(x as u128);
        Ok(match (to, sel) {
            (W, deposits::CUSTODIAN) => aword(&self.custodian),
            (W, deposits::REGISTRY) => aword(&REG),
            (W, deposits::LISTING_ID) => word(0),
            (W, deposits::DEPOSIT_MINTED) => b(self.minted.contains(&arg_word(data, 0))),
            (W, deposits::INSCRIPTION_HELD) => b(self.held.contains(&arg_word(data, 0))),
            (REG, deposits::LISTING) => [word(32), word(1), word(0), word(0), word(0), aword(&[9; 20]), self.root.to_vec()].concat(),
            (QUEUE, buyqueue::QUEUED) => word(self.queue.queued),
            (QUEUE, buyqueue::ITEM_CAP) => word(self.queue.cap),
            (QUEUE, buyqueue::BUFFER) => word(self.queue.buffer),
            (QUEUE, buyqueue::ITEMS_RECORDED) => word(self.queue.recorded as u128),
            (QUEUE, buyqueue::HEAD_DEADLINE) => [word(self.queue.head.0 as u128), word(self.queue.head.1 as u128)].concat(),
            (QUEUE, curvebuy::PAYEE) => aword(&self.queue.payee),
            (QUEUE, curvebuy::WRAPPER) | (CURVE, curvebuy::WRAPPER) => aword(&W),
            (CURVE, s) => {
                let c = self.curve.clone().ok_or("no curve")?;
                match s {
                    curvebuy::STATE => word(c.state as u128),
                    curvebuy::FROGS => word(c.n as u128),
                    curvebuy::FROGS_BOUGHT => word(c.k as u128),
                    curvebuy::FROG_SPENT => word(c.spent),
                    curvebuy::FROG_BUDGET => word(c.budget),
                    curvebuy::BUFFER => word(c.buffer),
                    curvebuy::DEADLINE => [word(c.deadline.0 as u128), word(c.deadline.1 as u128)].concat(),
                    curvebuy::PAYEE => aword(&ME),
                    curvebuy::FACTORY => aword(&FACTORY),
                    _ => return Err("unknown curve read".into()),
                }
            }
            (FACTORY, curvebuy::COIN_COUNT) => word(self.curve.is_some() as u128),
            (FACTORY, curvebuy::COINS) => aword(&COIN),
            (FACTORY, curvebuy::ROUTER_OF) => aword(&CURVE),
            (FLOOR, s) | (FLOOR_EVM, s) if s != floorsink::ITEM_ARRIVED => {
                let f = if to == FLOOR { self.floor.clone() } else { self.floor_evm.clone() };
                match s {
                    floorsink::NET_UNITS => word(f.net),
                    floorsink::ESCROW => word(f.escrow),
                    floorsink::BUFFER => word(f.buffer),
                    floorsink::FLOOR => [word(f.price), word(f.posted_at as u128), word(f.ttl as u128)].concat(),
                    floorsink::MAX_ITEM_COST => word(f.max_cost),
                    floorsink::ITEMS_ARRIVED => word(f.arrived as u128),
                    // the module as merged (docs/v6/canister-floor-followups.md §1): the fills price is the posted floor here,
                    // no float inventory, the sale minimum is the floor less 3%, and nothing backs units but the escrow
                    floorsink::PRICE => word(f.price),
                    floorsink::FLOAT_UNITS => word(0),
                    floorsink::MIN_SALE_PROCEEDS => word(f.price * 97 / 100),
                    floorsink::ESCROW_BALANCE => word(f.escrow),
                    floorsink::PAYEE => aword(&ME),
                    floorsink::WRAPPER => aword(if to == FLOOR { &W } else { &NFTW }),
                    _ => return Err("unknown floor read".into()),
                }
            }
            (NFTW, curvebuy::BALANCE_OF) => word(if arg_word(data, 0)[12..] == FLOOR_EVM { self.nft_units_at_module } else { 0 }),
            (DESK, mintdesk::REQUEST_COUNT) => word(self.requests.len() as u128),
            (DESK, mintdesk::TTL) => word(self.desk_ttl as u128),
            (DESK, mintdesk::REQUEST_OF) => {
                let r = self.requests.get(arg(data, 0) as usize).ok_or("no request")?;
                [word(r.status as u128), aword(&r.requester), word(r.at as u128), word(r.at_block as u128), word(r.amount), word(r.cost)].concat()
            }
            (DESK, mintdesk::IS_EXPIRED) => {
                let r = self.requests.get(arg(data, 0) as usize).ok_or("no request")?.clone();
                b(self.expired(&r))
            }
            _ => return Err(format!("unknown read 0x{} {}", hex::encode(to), hex::encode(sel))),
        })
    }
    async fn clock(&mut self) -> Result<Clock, String> { Ok(self.clock) }
    async fn our_evm(&mut self) -> Result<[u8; 20], String> { Ok(ME) }
    async fn evm_nonce(&mut self) -> Result<u64, String> { Ok(self.nonce) }
    async fn base_fee(&mut self) -> Result<u128, String> { Ok(1_000_000) }
    async fn estimate_gas(&mut self, to: [u8; 20], _value: u128, data: &[u8]) -> Result<u64, String> {
        self.exec(to, data, false).map(|_| 100_000)
    }
    async fn sign_evm(&mut self, _hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { Ok((0, [1; 32], [2; 32])) }
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String> {
        let saved = self.saved.as_ref().and_then(|s| s.pending.as_ref()).map(|p| p.raw.clone());
        assert_eq!(saved.as_deref(), Some(raw), "sent EVM bytes that were not saved first");
        if !self.receipts.contains_key(&evmtx::keccak(raw)) && !self.mempool.iter().any(|r| r == raw) {
            self.mempool.push(raw.to_vec());
        }
        Ok(())
    }
    async fn evm_seen(&mut self, hash: [u8; 32]) -> EvmView {
        match self.receipts.get(&hash) {
            Some(ok) => EvmView::Mined { success: *ok },
            None if self.mempool.iter().any(|r| evmtx::keccak(r) == hash) => EvmView::Pending,
            None => EvmView::NotYet,
        }
    }
    async fn float_script(&mut self) -> Result<Vec<u8>, String> { Ok(float()) }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> { Ok(custody()) }
    async fn locate(&mut self, i: &str) -> Result<Option<Location>, String> { read(); Ok(self.loc.get(i).cloned()) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> { read(); Ok(self.contents.get(o).cloned()) }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> { read();
        self.outputs.get(o).cloned().ok_or_else(|| format!("no output {}", o.text()))
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> { Ok(self.outspends.get(o).copied()) }
    async fn tx_status(&mut self, txid: [u8; 32]) -> BtcSeen {
        match self.txs.get(&txid) {
            None => BtcSeen::NotYet,
            Some(BtcTx { height: Some(h), hash: Some(b), .. }) => BtcSeen::Confirmed { height: *h, block_hash: b.clone() },
            Some(_) => BtcSeen::Mempool,
        }
    }
    async fn tip(&mut self) -> Result<u64, String> { Ok(self.tip) }
    async fn block_hash(&mut self, h: u64) -> Result<String, String> { self.blocks.get(&h).cloned().ok_or("no block".into()) }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        let snap = self.saved.as_ref().map(|st| st.deposits.iter().map(|d| (d.outpoint, format!("{:?}", d.phase).split(' ').next().unwrap_or("").to_string())).collect()).unwrap_or_default();
        AT_SEND.with(|a| a.borrow_mut().push(snap));
        let txid = display(parse_signed(raw).map_err(|e| e.to_string())?.txid);
        self.btc_sent.push(raw.to_vec());
        self.txs.entry(txid).or_insert(BtcTx { height: None, hash: None, view: None });
        Ok(())
    }
    async fn custody_utxos(&mut self) -> Result<Vec<Utxo>, String> { read(); Ok(self.utxos.clone()) }
    async fn tx_view(&mut self, txid: [u8; 32]) -> Result<TxView, String> { read();
        let t = self.txs.get(&txid).ok_or("no such transaction")?;
        let mut v = match &t.view {
            Some(v) => v.clone(),
            // our own purchases and returns: no marker
            None => TxView { txid, inputs: vec![], outputs: vec![OutView { value: 10_000, script: custody() }], confirmed: false,
                height: None, block_hash: None },
        };
        v.confirmed = t.height.is_some();
        v.height = t.height;
        v.block_hash = t.hash.clone();
        Ok(v)
    }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(5) }
    async fn rate(&mut self) -> Result<u64, String> { Ok(RATE) }
    async fn input_clean(&mut self, _o: Outpoint) -> Result<(), String> { Ok(()) }
    async fn sign_float(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> { Ok(vec![0x11; 64]) }
    async fn sign_custody(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> { Ok(vec![0x22; 64]) }
    fn take_offer(&mut self, frog: u64) -> Option<Offer> { self.offers.remove(&frog) }
    fn save(&mut self, st: &FrogState) { self.saved = Some(st.clone()); }
    fn save_floor(&mut self, fl: &FloorState) { self.saved_floor = Some(fl.clone()); }
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, key: &str, collection: &str, flow: outflow::Flow, amount: u128) -> Result<(), String> {
        self.asked.push((key.to_string(), flow));
        let Some(l) = self.limits.clone() else { return Ok(()) };
        match self.outflow.check(&l, key, collection, flow, amount, self.now) {
            outflow::Verdict::Allow => Ok(()),
            outflow::Verdict::Wait { why, .. } => Err(format!("waits (outflow limit): {why}")),
            outflow::Verdict::Frozen(c) => {
                self.paused = true;
                Err(format!("refused: custody is frozen: {c:?}"))
            }
        }
    }
    fn outflow_record(&mut self, key: &str, collection: &str, flow: outflow::Flow, amount: u128) {
        self.recorded.push(key.to_string());
        self.outflow.record(key, collection, flow, amount, self.now);
    }
    fn outbound_policy(&self, custody_script: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: custody_script.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true,
            require_event: true }
    }
    fn paused(&self) -> bool { self.paused }
}

fn tick(w: &mut World, st: &mut FrogState) -> Vec<String> {
    let lines = block_on(frogflow::tick(w, st, false, &[]));
    for l in &lines {
        println!("{l}");
    }
    lines
}

fn queue_state() -> FrogState {
    FrogState { sinks: vec![SinkRec { kind: SinkKind::Queue, sink: QUEUE, coin: [0; 20], view: None, refused: None,
        drove_refunds_at: None, drove_settle: false }], ..Default::default() }
}

/// A funded queue: 0.42 ETH, cap 0.40, buffer 0.02; its oldest credit expires in a week.
fn funded(w: &mut World) {
    w.queue = QueueC { queued: 42 * E / 100, cap: 40 * E / 100, buffer: 2 * E / 100, recorded: 0,
        head: (T0 + 7 * 86_400, B0 + 7 * 86_400 / 12), head_amount: 42 * E / 100, payee: ME };
}

fn phase(st: &FrogState, id: u64) -> FPhase {
    st.frogs.iter().find(|f| f.id == id).unwrap().phase.clone()
}

/// Drive a wanted frog through an offer, six blocks and ord to `Held`.
fn buy(w: &mut World, st: &mut FrogState, frog: u64, id: &str, seller: Outpoint, proof: Vec<[u8; 32]>) {
    w.list(id, seller);
    let o = w.offer_for(frog, id, seller, PRICE, proof);
    w.offers.insert(frog, o);
    tick(w, st);
    assert!(matches!(phase(st, frog), FPhase::Signed { .. }), "{:?}", phase(st, frog));
    w.mine_btc(1);
    tick(w, st);
    assert!(matches!(phase(st, frog), FPhase::Sent { .. }));
    w.mine_btc(5);
}

// ---- holder deposits -----------------------------------------------------------------------------------------------------------

fn payload(chain: u32, w: [u8; 20], r: [u8; 20]) -> Vec<u8> {
    [b"EWD1".to_vec(), chain.to_be_bytes().to_vec(), w.to_vec(), r.to_vec()].concat()
}
fn op_return(d: &[u8]) -> OutView {
    OutView { value: 0, script: [vec![0x6a, d.len() as u8], d.to_vec()].concat() }
}
fn sender() -> Vec<InView> {
    vec![InView { prev: Outpoint { txid: [0x5e; 32], vout: 0 }, prev_script: p2tr(0x5e), script_sig: vec![], witness: vec![vec![3; 64]] }]
}
fn frog_on(id: &str) -> Contents {
    Contents { inscriptions: vec![id.to_string()], runes: vec![], value: 546 }
}
fn dep_state(proofs: &[(String, Vec<[u8; 32]>)]) -> FrogState {
    let mut st = FrogState::default();
    for (i, p) in proofs {
        st.put_proof(i.clone(), p.clone());
    }
    st
}
fn dphase(st: &FrogState, o: &Outpoint) -> DPhase {
    st.deposits.iter().find(|d| d.outpoint == *o).unwrap().phase.clone()
}


// =====================================================================================================================================
// AUDIT D PoCs
// =====================================================================================================================================

/// An attacker's deposit: one custody output of plain sats in its own transaction, six deep, with an `EWD1` marker that is
/// deliberately malformed (truncated). Nothing about it needs an inscription, a frog, or the operator.
fn junk_deposit(w: &mut World, tag: u8) -> Outpoint {
    let plain = Contents { inscriptions: vec![], runes: vec![], value: 2_000 };
    w.deposit(tag, vec![OutView { value: 2_000, script: custody() }, op_return(&payload(4663, W, RECIPIENT)[..40])], plain, sender(), 6)
}

/// AUDIT-D H-2 PoC. Ten malformed-marker deposits (one Bitcoin transaction's worth of dust) are each `Refused` and "returned";
/// the stage-0 limiter lets three returns out a day and counts every other ask, so the ninth distinct ask trips the H3 freeze,
/// which the canister maps onto its PAUSE: every delivery, conflict-out and purchase stops until k guardians resume through
/// the timelock. The asks that did not go out are forgotten at the resume, but the refused deposits are still there and ask
/// again on the very next tick, so the freeze re-trips at once.
#[test]
fn audit_d_anyone_trips_the_custody_freeze_with_refused_junk_deposits_and_it_re_trips_after_resume() {
    let mut w = World::new();
    w.limits = Some(outflow::Limits::stage0());
    let mut st = FrogState::default();
    let junk: Vec<Outpoint> = (0..10u8).map(|i| junk_deposit(&mut w, 0x80 + i)).collect();
    let mut ticks = 0;
    while !w.paused && ticks < 10 {
        tick(&mut w, &mut st);
        ticks += 1;
    }
    assert!(w.paused, "custody froze (the canister pauses) after {ticks} ticks");
    let returning = junk.iter().filter(|o| matches!(dphase(&st, o), DPhase::Returning { .. })).count();
    let refused = junk.iter().filter(|o| matches!(dphase(&st, o), DPhase::Refused { .. })).count();
    println!("AUDIT-D: frozen after {ticks} ticks; {returning} returns sent, {refused} refused deposits still asking; cause {:?}",
        w.outflow.frozen());
    assert!(matches!(w.outflow.frozen(), Some(outflow::Cause::Tripwire { .. })), "the tripwire, not a person, froze it");
    assert_eq!(returning, 3, "the stage-0 limiter let three returns out");
    // k guardians resume (timelock resume_target -> set_paused(false) -> Outflow::unfreeze(true))
    w.paused = false;
    w.outflow.unfreeze(true).unwrap();
    tick(&mut w, &mut st);
    assert!(w.paused, "one tick after the resume it is frozen again: the refused deposits ask again");
    println!("AUDIT-D: re-frozen one tick after the guardians' resume: {:?}", w.outflow.frozen());
}

/// AUDIT-D H-3 PoC. A deposit with a well-formed marker and any single inscription waits for a membership proof that, in
/// Phase 1, the operator never gives (holder deposits are OFF). It is never finished, and every tick re-reads it in full: the
/// deposit transaction (200 kB reserved per explorer on the IC), the block, the tip, ord's contents and location (every
/// provider, heights before and after), and the output. The count grows with the number of such deposits, without bound.
#[test]
fn audit_d_pending_deposits_are_re_read_in_full_every_tick_forever() {
    let per_tick = |n: u8| {
        let mut w = World::new();
        let mut st = FrogState::default(); // no proofs: Phase 1 never supplies one
        let ds: Vec<Outpoint> = (0..n).map(|i| {
            w.deposit(0x90 + i, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))],
                frog_on(&insc(0x40 + i)), sender(), 6)
        }).collect();
        tick(&mut w, &mut st); // classified
        let mut counts = Vec::new();
        for _ in 0..20 {
            READS.with(|r| r.set(0));
            tick(&mut w, &mut st);
            counts.push(READS.with(|r| r.get()));
        }
        for d in &ds {
            assert!(matches!(dphase(&st, d), DPhase::Pending { ref why } if why.contains("awaiting a membership proof")), "{:?}", dphase(&st, d));
        }
        counts
    };
    let one = per_tick(1);
    let eight = per_tick(8);
    println!("AUDIT-D: world reads per tick with 1 pending deposit: {:?}", &one[..3]);
    println!("AUDIT-D: world reads per tick with 8 pending deposits: {:?}", &eight[..3]);
    // after the first tick (which also classifies), the same reads every tick, forever
    assert!(one[1..].iter().all(|c| *c == one[1]) && eight[1..].iter().all(|c| *c == eight[1]), "the same reads every tick, forever");
    let per_deposit = (eight[1] - one[1]) / 7;
    assert!(per_deposit >= 4, "each pending deposit costs {per_deposit} world reads (tx_view, contents, locate, output; plus block_hash and tip, uncounted) per tick");
    println!("AUDIT-D: {per_deposit} world reads per pending deposit per tick, never ending");
}

/// AUDIT-D L-? PoC. `give_back` sends the signed return before the `Returning` record (with its bytes and fee input) is saved:
/// the caller saves only after `give_back` returns, and `btc_send` is an await in between. On the IC the state at that await is
/// committed as `Refused`: a trap after it, or an upgrade while the send is outstanding, loses the signed bytes, and the next tick
/// signs a second, different return.
#[test]
fn audit_d_a_return_is_sent_before_it_is_saved() {
    let mut w = World::new();
    let mut st = FrogState::default();
    let o = junk_deposit(&mut w, 0x70);
    AT_SEND.with(|a| a.borrow_mut().clear());
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { .. }), "{:?}", dphase(&st, &o));
    let at_send = AT_SEND.with(|a| a.borrow().clone());
    let durable = at_send.last().expect("a send").iter().find(|(p, _)| *p == o).map(|(_, s)| s.clone());
    println!("AUDIT-D: saved phase of the deposit at the moment its return was sent: {durable:?}");
    assert_eq!(durable.as_deref(), Some("Refused"), "the bytes went out while the durable record still said Refused");
}
