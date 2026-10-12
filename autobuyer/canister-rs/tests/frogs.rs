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
use etch_autobuyer::frogflow::{self, FStep, FrogIo, FrogState, Offer, ProtectedOffer};
use etch_autobuyer::itemsink::{Clock, SinkKind, SinkRec};
use etch_autobuyer::looper::EvmView;
use etch_autobuyer::psbt::{Psbt, Tx, TxIn, TxOut};
use etch_autobuyer::{buyqueue, curvebuy, mintdesk};
use etch_autobuyer::floorsink;
use etch_autobuyer::floortrigger::{FloorItem, FloorState, WPhase};
use etch_autobuyer::{custodycap, outbound, outflow};
use etch_autobuyer::relayconv::{self as rc, CPhase};

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
    /// B-L-6: the Ethereum side holds the one Relay leg
    eth_busy: bool,
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
    /// Holder deposits switched on (audit D, H-2: off unless the install says so; these tests exercise them, so on)
    deposits: bool,
    /// Every read of Bitcoin, ord or the explorers the flow makes: each is one or more replicated outcalls on the IC (audit D, H-3)
    reads: usize,
    /// For every Bitcoin send, the deposit phases as SAVED at that moment (audit D, L-1: save, then send)
    at_send: Vec<Vec<(Outpoint, String)>>,
    /// Relay conversions (docs/v6/relay-conversion.md): the install's funding (None in the tests above: the float path), the
    /// runner's quotes, the module's unit in transit and its advance, the float's own UTXOs, the value of the call being executed,
    /// and every Relay deposit (order id, value)
    funding: Option<etch_autobuyer::relayconv::FrogFunding>,
    quotes: HashMap<u64, (Vec<u8>, u64)>,
    transit: u128,
    advanced: u128,
    float_outs: Vec<Utxo>,
    value_in: u128,
    relay_deposits: Vec<([u8; 32], u128)>,
    /// Conversion review C-03: what `fundReserve` paid into the frog module
    reserve: u128,
    /// Protected Ordinals Wallet offers (docs/v8/ow-protected.md): the runner's hand-ins, the float's script and the reference rate
    /// when a fixture names its own, outputs ord shows carrying something, every input checked clean and every digest signed
    protected: HashMap<u64, ProtectedOffer>,
    float_script: Option<Vec<u8>>,
    rate: Option<u64>,
    dirty: HashSet<Outpoint>,
    cleaned: Vec<Outpoint>,
    digests: Vec<[u8; 32]>,
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
            deposits: true,
            reads: 0,
            at_send: vec![],
            funding: None,
            eth_busy: false,
            quotes: HashMap::new(),
            transit: 0,
            advanced: 0,
            float_outs: vec![],
            value_in: 0,
            relay_deposits: vec![],
            reserve: 0,
            protected: HashMap::new(),
            float_script: None,
            rate: None,
            dirty: HashSet::new(),
            cleaned: vec![],
            digests: vec![],
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
        // the seller's key-path signature under SIGHASH_SINGLE|ANYONECANPAY (0x83), the listing shape the judge completes
        let mut seller_sig = vec![7u8; 65];
        seller_sig[64] = 0x83;
        let finals = vec![None, None, Some(vec![seller_sig]), None];
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
        if to == FLOOR && sel == rc::ADVANCE {
            let amount = arg(data, 0);
            if self.transit != 0 { return Err("AdvanceOpen".into()); }
            if amount == 0 || amount > self.floor.max_cost || amount > self.floor.escrow { return Err("OverItemCap".into()); }
            if self.floor.net < E { return Err("NotEnoughBacked".into()); }
            if apply {
                self.transit = E;
                self.advanced = amount;
                self.floor.net -= E;
                self.floor.escrow -= amount;
                *self.balances.entry(ME).or_default() += amount;
            }
            return Ok(());
        }
        if to == FLOOR && sel == rc::SETTLE_ADVANCE {
            let cost = arg(data, 0);
            if self.transit == 0 { return Err("NoAdvance".into()); }
            if cost == 0 || cost > self.advanced { return Err("BadInput".into()); }
            if self.balances.get(&FLOOR).copied().unwrap_or(0) < (self.floor.arrived as u128 + 1) * E { return Err("NoWrapped".into()); }
            if apply {
                self.transit = 0;
                self.advanced = 0;
                self.floor.arrived += 1;
            }
            return Ok(());
        }
        if to == FLOOR && sel == rc::RETURN_ADVANCE {
            if self.transit == 0 { return Err("NoAdvance".into()); }
            if self.balances.get(&ME).copied().unwrap_or(0) < self.value_in { return Err("insufficient funds".into()); }
            if apply {
                self.transit = 0;
                self.advanced = 0;
                self.floor.net += E;
                self.floor.escrow += self.value_in;
                *self.balances.entry(ME).or_default() -= self.value_in;
            }
            return Ok(());
        }
        if to == FLOOR && sel == etch_autobuyer::ethflow::FUND_RESERVE {
            if self.value_in == 0 { return Err("BadInput".into()); }
            if self.balances.get(&ME).copied().unwrap_or(0) < self.value_in { return Err("insufficient funds".into()); }
            if apply {
                self.reserve += self.value_in;
                *self.balances.entry(ME).or_default() -= self.value_in;
            }
            return Ok(());
        }
        if to == rc::RELAY_DEPOSITORY_4663 && sel == rc::DEPOSIT_NATIVE {
            if self.balances.get(&ME).copied().unwrap_or(0) < self.value_in { return Err("insufficient funds".into()); }
            if apply {
                *self.balances.entry(ME).or_default() -= self.value_in;
                self.relay_deposits.push((data[36..68].try_into().unwrap(), self.value_in));
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
            self.value_in = uint(&f[6]);
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

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for World {}

impl FrogIo for World {
    fn eth_relay_busy(&self) -> bool { self.eth_busy }
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
                    s if s == rc::TRANSIT_UNITS => word(if to == FLOOR { self.transit } else { 0 }),
                    floorsink::PAYEE => aword(&ME),
                    floorsink::WRAPPER => aword(if to == FLOOR { &W } else { &NFTW }),
                    _ => return Err("unknown floor read".into()),
                }
            }
            (rc::RELAY_DEPOSITORY_4663, s) if s == rc::ALLOCATOR => aword(&rc::RELAY_ALLOCATOR_4663),
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
    async fn estimate_gas(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<u64, String> {
        self.value_in = value;
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
    async fn float_script(&mut self) -> Result<Vec<u8>, String> { Ok(self.float_script.clone().unwrap_or_else(float)) }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> { Ok(custody()) }
    async fn locate(&mut self, i: &str) -> Result<Option<Location>, String> { self.reads += 1; Ok(self.loc.get(i).cloned()) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> { self.reads += 1; Ok(self.contents.get(o).cloned()) }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        self.reads += 1;
        if let Some(x) = self.outputs.get(o).cloned() {
            return Ok(x);
        }
        // custody's own UTXOs (a fee input, say) read back as the explorers serve them
        self.utxos.iter().find(|u| u.outpoint == *o).map(|u| (Output { script: custody(), value: u.value, confirmed: u.confirmed }, false))
            .ok_or_else(|| format!("no output {}", o.text()))
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
        let snap = self.saved.as_ref().map(|st| st.deposits.iter().map(|d| (d.outpoint,
            format!("{:?}", d.phase).split([' ', '{', '(']).next().unwrap_or("").to_string())).collect()).unwrap_or_default();
        self.at_send.push(snap);
        let txid = display(parse_signed(raw).map_err(|e| e.to_string())?.txid);
        self.btc_sent.push(raw.to_vec());
        self.txs.entry(txid).or_insert(BtcTx { height: None, hash: None, view: None });
        Ok(())
    }
    async fn custody_utxos(&mut self) -> Result<Vec<Utxo>, String> { self.reads += 1; Ok(self.utxos.clone()) }
    async fn tx_view(&mut self, txid: [u8; 32]) -> Result<TxView, String> {
        self.reads += 1;
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
    async fn rate(&mut self) -> Result<u64, String> { Ok(self.rate.unwrap_or(RATE)) }
    async fn input_clean(&mut self, o: Outpoint) -> Result<(), String> {
        self.cleaned.push(o);
        if self.dirty.contains(&o) { Err(format!("refused: {} carries an inscription", o.text())) } else { Ok(()) }
    }
    async fn sign_float(&mut self, h: [u8; 32]) -> Result<Vec<u8>, String> { self.digests.push(h); Ok(vec![0x11; 64]) }
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
    fn deposits_on(&self) -> bool { self.deposits }
    async fn our_balance(&mut self) -> Result<u128, String> { Ok(self.balances.get(&ME).copied().unwrap_or(0)) }
    async fn float_utxos(&mut self) -> Result<Vec<Utxo>, String> { Ok(self.float_outs.clone()) }
    async fn float_address(&mut self) -> Result<String, String> { Ok(float_addr()) }
    fn funding(&self) -> Option<etch_autobuyer::relayconv::FrogFunding> { self.funding }
    fn take_quote(&mut self, frog: u64) -> Option<(Vec<u8>, u64)> { self.quotes.remove(&frog) }
    fn take_protected(&mut self, frog: u64) -> Option<ProtectedOffer> { self.protected.remove(&frog) }
    async fn float_internal_key(&mut self) -> Result<[u8; 32], String> { Ok(FLOAT_INTERNAL) }
}

/// The float's internal key as this world's subnet derives it.
const FLOAT_INTERNAL: [u8; 32] = [0x1f; 32];

fn float_addr() -> String {
    etch_autobuyer::policy::taproot_address(&[0xf1; 32], "bc")
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

// ---- the queue ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_funded_queue_gets_one_item_bought_confirmed_minted_and_reported() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { cap_wei } if cap_wei == 40 * E / 100), "the queue's cap, not all it holds");
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    buy(&mut w, &mut st, 0, &insc(1), seller, proofs[0].clone());
    // six deep and ord shows custody holding exactly it: held, and in the same tick the mint goes out
    tick(&mut w, &mut st);
    let FPhase::Held { buy } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    assert_eq!(buy.outpoint.vout, 1);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 0 }), "wrapDeposit is out");
    w.mine_evm();
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Minted { .. }));
    assert_eq!(w.wrapped_units(&QUEUE), 1, "one wFROG minted to the queue");
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Report { frog: 0 }), "recordItem is out");
    w.mine_evm();
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Settled { .. }));
    assert_eq!((w.queue.recorded, w.paid), (1, buy.cost_wei), "reported and reimbursed at the cost fixed at acceptance");
    // 903,000 sats at 3.19M/ETH plus 2%
    assert_eq!(buy.cost_wei, frogbuy::wei_for_sats(903_000, RATE).unwrap());
    assert!(buy.cost_wei <= 40 * E / 100);
    // the queue now holds less than cap + buffer: nothing more is bought
    tick(&mut w, &mut st);
    assert_eq!(st.frogs.len(), 1);
    // and the custodian can find the frog behind a deposit id
    assert_eq!(st.inscription_for_leaf(&frogbuy::leaf_of(&insc(1))), Some(insc(1)));
    let p = etch_autobuyer::itemsink::progress(&st.sinks[0], &st.frogs);
    assert_eq!((p.recorded, p.in_flight), (1, 0));
}

#[test]
fn the_floor_moved_past_the_buffer_nothing_is_signed_and_the_want_fails_when_time_runs_out() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    // the floor is now 1.5M sats: over the cap in sats at our rate
    let o = w.offer_for(0, &insc(1), seller, 1_500_000, proofs[0].clone());
    w.offers.insert(0, o);
    let lines = tick(&mut w, &mut st);
    assert!(lines.iter().any(|l| l.contains("offer refused") && l.contains("OverCap")), "{lines:?}");
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    assert!(w.btc_sent.is_empty(), "nothing signed or sent");
    // the oldest credit's deadline comes within a purchase's latency: the want fails, having spent nothing
    w.advance(7 * 86_400 - 3 * 3_600);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Failed { .. }));
    // past it on both clocks, the canister drives the queue's refund
    w.advance(3 * 3_600);
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Refunds { sink: QUEUE }));
    w.mine_evm();
    tick(&mut w, &mut st);
    assert_eq!(w.queue.queued, 0, "the credit went back");
}

#[test]
fn the_listing_vanished_before_ours_confirmed_the_record_fails_and_a_new_one_is_wanted() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    let o = w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone());
    w.offers.insert(0, o);
    tick(&mut w, &mut st);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!() };
    // someone else bought it first: the seller's outpoint is spent in a block by another transaction, ours never lands
    w.txs.remove(&buy.outpoint.txid);
    w.outspends.insert(seller, ([0x99; 32], true));
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Failed { ref why } if why.contains("vanished")), "{:?}", phase(&st, 0));
    // the same tick (or the next) wants another frog for the same queue
    tick(&mut w, &mut st);
    assert!(st.frogs.iter().any(|f| f.id == 1 && matches!(f.phase, FPhase::Wanted { .. })));
}

#[test]
fn a_reorganisation_below_six_confirmations_stops_the_mint_until_six_again() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    let o = w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone());
    w.offers.insert(0, o);
    tick(&mut w, &mut st);
    w.mine_btc(6);
    // the EVM side is busy (another transaction of ours is out) while the frog becomes held
    block_on(frogflow::tick(&mut w, &mut st, true, &[]));
    block_on(frogflow::tick(&mut w, &mut st, true, &[]));
    let FPhase::Held { buy } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    // then its block is replaced: the explorers still name the old block, which is no longer the block at that height
    let h = w.txs[&buy.outpoint.txid].height.unwrap();
    w.blocks.insert(h, block_hash_for(h, 0xee));
    let lines = tick(&mut w, &mut st);
    assert!(st.pending.is_none() && w.calls.is_empty(), "nothing minted on a reorganised block: {lines:?}");
    // and it falls out of the chain altogether: back to waiting for six
    w.unconfirm(buy.outpoint.txid);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Sent { .. }), "{:?}", phase(&st, 0));
    assert!(st.pending.is_none() && w.calls.is_empty());
    // three deep again is still not six
    w.mine_btc(3);
    tick(&mut w, &mut st);
    assert!(st.pending.is_none());
    w.mine_btc(3);
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 0 }), "six deep again: minted");
}

#[test]
fn the_same_bytes_are_sent_again_after_an_eviction_never_a_new_signature() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    let o = w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone());
    w.offers.insert(0, o);
    tick(&mut w, &mut st);
    let first = w.btc_sent[0].clone();
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!() };
    w.txs.remove(&buy.outpoint.txid); // lost
    tick(&mut w, &mut st);
    assert_eq!(w.btc_sent.last(), Some(&first), "the saved bytes, again");
    assert_eq!(w.btc_sent.iter().filter(|r| **r != first).count(), 0);
}

#[test]
fn nothing_is_bought_or_minted_unless_we_are_the_wrappers_custodian_and_one_evm_transaction_at_a_time() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    w.custodian = [0x77; 20];
    let mut st = queue_state();
    let lines = tick(&mut w, &mut st);
    assert!(st.frogs.is_empty(), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("not us")));
    // a queue that pays someone else is never served
    w.custodian = ME;
    w.queue.payee = [0x78; 20];
    tick(&mut w, &mut st);
    assert!(st.sinks[0].refused.as_deref().is_some_and(|r| r.contains("payee")));
    assert!(st.frogs.is_empty());
    // others' EVM transaction out: ours waits
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    w.queue.head = (T0 - 1, B0 - 1); // a refund is due
    let lines = block_on(frogflow::tick(&mut w, &mut st, true, &[]));
    assert!(st.pending.is_none() && lines.iter().any(|l| l.contains("one at a time")), "{lines:?}");
}

// ---- a curve, as merged ------------------------------------------------------------------------------------------------------

fn curve_world() -> (World, FrogState, Vec<Vec<[u8; 32]>>) {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2), insc(3), insc(4)]);
    w.curve = Some(CurveC { state: 1, n: 2, k: 0, spent: 0, budget: 70 * E / 100, buffer: 4 * E / 100,
        deadline: (T0 + 3 * 86_400, B0 + 3 * 86_400 / 12) });
    let st = FrogState { factory: Some(FACTORY), ..Default::default() };
    (w, st, proofs)
}

#[test]
fn a_graduating_curve_gets_n_frogs_recorded_and_the_canister_drives_graduation() {
    let (mut w, mut st, proofs) = curve_world();
    tick(&mut w, &mut st);
    assert_eq!(st.sinks.len(), 1, "discovered through the factory");
    assert!(matches!(phase(&st, 0), FPhase::Wanted { cap_wei } if cap_wei == 37 * E / 100), "(0.70 + 0.04) / 2");
    for (frog, n) in [(0u64, 1u8), (1, 2)] {
        buy(&mut w, &mut st, frog, &insc(n), Outpoint { txid: [0xe0 + n; 32], vout: 0 }, proofs[n as usize - 1].clone());
        tick(&mut w, &mut st); // held → mint
        w.mine_evm();
        tick(&mut w, &mut st); // minted → recordFrog
        w.mine_evm();
        tick(&mut w, &mut st); // settled; the next frog is wanted
    }
    let c = w.curve.clone().unwrap();
    assert_eq!((c.k, w.wrapped_units(&CURVE)), (2, 2));
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Settle { sink: CURVE }), "graduate() is driven");
    w.mine_evm();
    tick(&mut w, &mut st);
    assert_eq!(w.curve.as_ref().unwrap().state, 2);
    assert!(st.sinks[0].done());
    let p = etch_autobuyer::itemsink::progress(&st.sinks[0], &st.frogs);
    assert_eq!((p.recorded, p.target), (2, 2), "k of N, readable without a call");
}

#[test]
fn past_the_window_on_both_clocks_the_canister_opens_refunds_and_a_held_frog_becomes_inventory() {
    let (mut w, mut st, proofs) = curve_world();
    tick(&mut w, &mut st);
    // bought and six deep, but the window is inside the mint margin by the time it is held
    w.list(&insc(1), Outpoint { txid: [0xe1; 32], vout: 0 });
    w.offers.insert(0, w.offer_for(0, &insc(1), Outpoint { txid: [0xe1; 32], vout: 0 }, PRICE, proofs[0].clone()));
    tick(&mut w, &mut st);
    w.mine_btc(6);
    w.advance(3 * 86_400 - 1_800);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    let f = st.frogs.iter().find(|f| f.id == 0).unwrap();
    assert!(matches!(f.phase, FPhase::Held { .. }) && f.target.is_none(), "not minted inside the margin: inventory");
    assert!(w.calls.is_empty() && st.pending.is_none());
    // the deadline passes on the timestamp only: not yet
    w.clock.time += 7_200;
    tick(&mut w, &mut st);
    assert!(st.pending.is_none(), "one clock is not both");
    w.advance(3_600);
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Refunds { sink: CURVE }));
    w.mine_evm();
    tick(&mut w, &mut st);
    assert_eq!(w.curve.as_ref().unwrap().state, 3, "Refunding");
    assert!(st.sinks[0].done());
    assert!(matches!(phase(&st, 0), FPhase::Held { .. }), "the frog is ours, unminted, for the next target");
}

// ---- the desk ------------------------------------------------------------------------------------------------------------------

fn desk_world(amount: u128) -> (World, FrogState, Vec<Vec<[u8; 32]>>) {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2), insc(3), insc(4)]);
    w.requests.push(Req { status: 1, requester: [0x42; 20], at: T0, at_block: B0, amount, cost: 0 });
    let st = FrogState { desk: Some(DESK), ..Default::default() };
    (w, st, proofs)
}

#[test]
fn a_desk_request_is_filled_at_cost_never_above_its_maximum() {
    let (mut w, mut st, proofs) = desk_world(35 * E / 100);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { cap_wei } if cap_wei == 35 * E / 100));
    buy(&mut w, &mut st, 0, &insc(1), Outpoint { txid: [0xe1; 32], vout: 0 }, proofs[0].clone());
    tick(&mut w, &mut st);
    w.mine_evm();
    assert_eq!(w.wrapped_units(&DESK), 1);
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if matches!(p.step, FStep::Fulfill { request: 0, .. })));
    w.mine_evm();
    tick(&mut w, &mut st);
    let FPhase::Settled { buy } = phase(&st, 0) else { panic!() };
    assert_eq!((w.requests[0].status, w.requests[0].cost), (2, buy.cost_wei));
    assert!(buy.cost_wei <= w.requests[0].amount);
}

#[test]
fn a_request_below_the_cost_is_never_filled() {
    let (mut w, mut st, proofs) = desk_world(25 * E / 100);
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    let o = w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone());
    w.offers.insert(0, o);
    let lines = tick(&mut w, &mut st);
    assert!(lines.iter().any(|l| l.contains("OverCap")), "{lines:?}");
    assert!(w.btc_sent.is_empty() && w.calls.is_empty());
}

#[test]
fn a_request_that_expires_between_the_mint_and_fulfill_leaves_a_stray_that_fills_the_next() {
    let (mut w, mut st, proofs) = desk_world(35 * E / 100);
    tick(&mut w, &mut st);
    buy(&mut w, &mut st, 0, &insc(1), Outpoint { txid: [0xe1; 32], vout: 0 }, proofs[0].clone());
    tick(&mut w, &mut st);
    w.mine_evm(); // minted to the desk
    // the request's time runs out before fulfill
    w.advance(24 * 3_600);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Stray { .. }), "{:?}", phase(&st, 0));
    assert!(w.calls.iter().all(|(_, d, _)| d[..4] != mintdesk::FULFILL), "no fulfill of an expired request");
    // a new request: filled from the stray with fulfill alone, no second purchase, no second mint
    w.requests.push(Req { status: 1, requester: [0x43; 20], at: w.clock.time, at_block: w.clock.parent_block, amount: 40 * E / 100, cost: 0 });
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if matches!(p.step, FStep::Fulfill { request: 1, frog: 0 })));
    w.mine_evm();
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Settled { .. }));
    assert_eq!(w.requests[1].status, 2);
    assert_eq!(st.frogs.len(), 1, "one frog bought");
    assert_eq!(w.btc_sent.len(), 1);
}

#[test]
fn an_inventory_frog_from_a_closed_curve_fills_a_desk_request() {
    let (mut w, mut st, proofs) = curve_world();
    tick(&mut w, &mut st);
    w.list(&insc(1), Outpoint { txid: [0xe1; 32], vout: 0 });
    w.offers.insert(0, w.offer_for(0, &insc(1), Outpoint { txid: [0xe1; 32], vout: 0 }, PRICE, proofs[0].clone()));
    tick(&mut w, &mut st);
    w.mine_btc(6);
    w.advance(3 * 86_400 - 1_800);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    assert!(st.frogs[0].target.is_none(), "inventory");
    st.desk = Some(DESK);
    w.requests.push(Req { status: 1, requester: [0x42; 20], at: w.clock.time, at_block: w.clock.parent_block, amount: 40 * E / 100, cost: 0 });
    tick(&mut w, &mut st);
    assert_eq!(st.frogs[0].target, Some(Target::Desk(0)));
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 0 }), "no new purchase: the frog in custody goes");
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

#[test]
fn a_marked_deposit_six_deep_is_minted_once_to_the_recipient_it_names() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    let o = w.deposit(0x61, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(1)), sender(), 5);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Pending { ref why } if why.contains("5 of 6")));
    w.mine_btc(1);
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Deposit { outpoint: o }));
    w.mine_evm();
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Minted { recipient, .. } if recipient == RECIPIENT));
    assert_eq!(w.wrapped_units(&RECIPIENT), 1);
    // again and again: never a second mint
    for _ in 0..3 {
        tick(&mut w, &mut st);
    }
    assert_eq!(w.calls.len(), 1);
    assert_eq!(st.deposits.len(), 1, "one record per outpoint, however often custody lists it");
    assert_eq!(st.inscription_for_leaf(&frogbuy::leaf_of(&insc(1))), Some(insc(1)));
}

#[test]
fn a_duplicate_outpoint_already_minted_is_adopted_never_minted_again() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    let o = w.deposit(0x62, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(1)), sender(), 6);
    w.minted.insert(deposits::outpoint_key(o.txid, o.vout));
    w.utxos.push(Utxo { outpoint: o, value: 546, confirmed: true }); // listed twice by the explorer
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Minted { .. }));
    assert!(st.pending.is_none() && w.calls.is_empty());
    assert_eq!(st.deposits.len(), 1);
}

/// Audit R2, N-7: `seen` is pruned to the confirmed listing, so a deposit that leaves it (a reorganisation, or a listing that
/// flapped) and comes back is classified again; its record is never pushed a second time.
#[test]
fn audit2_a_deposit_that_leaves_the_listing_and_comes_back_keeps_one_record() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    let o = w.deposit(0x64, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(1)), sender(), 3);
    tick(&mut w, &mut st);
    assert_eq!(st.deposits.len(), 1);
    let i = w.utxos.iter().position(|u| u.outpoint == o).unwrap();
    let gone = w.utxos.remove(i);
    tick(&mut w, &mut st);
    assert!(!st.seen.contains(&o), "pruned with the listing");
    w.utxos.push(gone);
    tick(&mut w, &mut st);
    assert_eq!(st.deposits.iter().filter(|d| d.outpoint == o).count(), 1, "one record per outpoint, whatever `seen` forgot");
}

#[test]
fn a_deposit_reorganised_below_six_is_not_minted() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    let o = w.deposit(0x63, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(1)), sender(), 6);
    // its block is replaced before the canister looks
    let h = w.txs[&o.txid].height.unwrap();
    w.blocks.insert(h, block_hash_for(h, 0xee));
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Pending { .. }));
    assert!(st.pending.is_none() && w.calls.is_empty());
    // back in a block, only three deep
    let t = w.tip - 2;
    w.confirm(o.txid, t);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Pending { ref why } if why.contains("3 of 6")));
    assert!(st.pending.is_none());
}

#[test]
fn a_missing_marker_is_not_a_deposit_and_is_never_returned() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    // a frog at custody with no OP_RETURN: a vault-bought frog looks exactly like this
    let o = w.deposit(0x64, vec![OutView { value: 546, script: custody() }], frog_on(&insc(1)), sender(), 6);
    tick(&mut w, &mut st);
    assert!(st.deposits.is_empty());
    assert_eq!(st.unmarked, vec![o], "recorded for the reconciliation");
    assert!(w.btc_sent.is_empty() && w.calls.is_empty());
}

#[test]
fn a_malformed_marker_is_refused_and_the_frog_goes_back_to_its_taproot_sender() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    let o = w.deposit(0x65, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT)[..40])],
        frog_on(&insc(1)), sender(), 6);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    let DPhase::Returning { reason, txid, .. } = dphase(&st, &o) else { panic!("{:?}", dphase(&st, &o)) };
    assert!(matches!(reason, Reason::Malformed(_)));
    let ret = parse_signed(w.btc_sent.last().unwrap()).unwrap();
    assert_eq!(display(ret.txid), txid);
    assert_eq!(ret.outputs[0], (546, p2tr(0x5e)), "the whole output, back to input 0's script");
    assert_eq!(display(ret.inputs[0].txid), o.txid, "input 0 is the refused frog");
    assert!(w.calls.is_empty(), "nothing minted");
    w.mine_btc(1);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returned { .. }));
    // the other chain or another wrapper: refused the same way
    let o2 = w.deposit(0x66, vec![OutView { value: 546, script: custody() }, op_return(&payload(1, W, RECIPIENT))], frog_on(&insc(2)), sender(), 6);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o2), DPhase::Returning { reason: Reason::WrongChain(1), .. }));
}

#[test]
fn runes_or_a_second_inscription_on_the_output_are_refused_and_returned() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    let marked = || vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))];
    let runes = Contents { runes: vec!["BITBLOCK".into()], ..frog_on(&insc(1)) };
    let o = w.deposit(0x67, marked(), runes, sender(), 6);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { reason: Reason::Runes(_), .. }), "{:?}", dphase(&st, &o));
    let two = Contents { inscriptions: vec![insc(1), insc(9)], runes: vec![], value: 546 };
    let o2 = w.deposit(0x68, marked(), two, sender(), 6);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o2), DPhase::Returning { reason: Reason::Inscriptions(_), .. } | DPhase::Refused { .. }));
    assert!(w.calls.is_empty(), "neither minted");
}

#[test]
fn a_deposit_not_signed_all_is_returned_and_one_spending_custody_is_held_not_returned() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone()), (insc(2), proofs[1].clone())]);
    let marked = || vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))];
    let mut single = sender();
    single[0].witness = vec![[vec![3u8; 64], vec![0x83]].concat()];
    let o = w.deposit(0x69, marked(), frog_on(&insc(1)), single, 6);
    let mut ours = sender();
    ours.push(InView { prev: Outpoint { txid: [0xfe; 32], vout: 1 }, prev_script: custody(), script_sig: vec![], witness: vec![vec![3; 64]] });
    let o2 = w.deposit(0x6a, marked(), frog_on(&insc(2)), ours, 6);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { reason: Reason::Sighash { input: 0, .. }, .. }), "{:?}", dphase(&st, &o));
    assert!(matches!(dphase(&st, &o2), DPhase::Held { reason: Reason::SpendsCustody(1), .. }), "{:?}", dphase(&st, &o2));
    assert!(w.calls.is_empty());
}

#[test]
fn a_deposit_waits_for_its_proof_and_the_rate_limit() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = FrogState::default();
    let o = w.deposit(0x6b, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(1)), sender(), 6);
    tick(&mut w, &mut st);
    assert_eq!(dphase(&st, &o), DPhase::AwaitingProof { inscription: insc(1) });
    // a proof that does not verify is still a wait
    st.put_proof(insc(1), proofs[1].clone());
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Pending { ref why } if why.contains("does not verify")));
    st.put_proof(insc(1), proofs[0].clone());
    st.mint_times = vec![w.now; 50];
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Pending { ref why } if why.contains("limit")));
    w.advance(86_400);
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Deposit { outpoint: o }));
}

#[test]
fn returns_are_rate_limited_so_marked_junk_cannot_drain_custodys_fee_float() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    st.return_times = vec![w.now; deposits::RETURNS_PER_DAY];
    let o = w.deposit(0x6c, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, [0; 20]))],
        frog_on(&insc(1)), sender(), 6);
    tick(&mut w, &mut st);
    let lines = tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Refused { reason: Reason::ZeroRecipient }), "{:?}", dphase(&st, &o));
    assert!(lines.iter().any(|l| l.contains("the limit")) && w.btc_sent.is_empty(), "waits, not lost: {lines:?}");
    w.advance(86_400);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { .. }));
}

#[test]
fn a_report_that_cannot_go_now_does_not_hold_up_a_deposits_mint() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    st.put_proof(insc(2), proofs[1].clone());
    tick(&mut w, &mut st);
    buy(&mut w, &mut st, 0, &insc(1), Outpoint { txid: [0xe1; 32], vout: 0 }, proofs[0].clone());
    tick(&mut w, &mut st); // held → wrapDeposit to the queue is out
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 0 }));
    // a holder's deposit becomes ready meanwhile, and the keeper lowers the queue's cap under our frog's cost
    let o = w.deposit(0x6d, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(2)), sender(), 6);
    w.mine_evm();
    w.queue.cap = E / 10;
    let lines = tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Minted { .. }));
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Deposit { outpoint: o }), "the deposit went: {lines:?}");
    w.mine_evm();
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Minted { .. }));
    assert!(matches!(phase(&st, 0), FPhase::Minted { .. }), "the report still waits for a cap it fits under");
}

// ---- the Floor Module as an item sink (docs/v5/floor-feed.md §6) --------------------------------------------------------------

fn tickf(w: &mut World, st: &mut FrogState, fl: &mut FloorState) -> Vec<String> {
    let lines = block_on(frogflow::tick_with(w, st, fl, false, &[]));
    for l in &lines {
        println!("{l}");
    }
    lines
}

fn floor_state() -> (FrogState, FloorState) {
    let st = FrogState { sinks: vec![SinkRec { kind: SinkKind::Floor, sink: FLOOR, coin: [0; 20], view: None, refused: None,
        drove_refunds_at: None, drove_settle: false }], ..Default::default() };
    let mut fl = FloorState::default();
    fl.add_sink(FLOOR, FloorItem::Bitcoin).unwrap();
    (st, fl)
}

/// A frog module: `net` units of ETH-backed demand, 1 ETH escrowed, a 0.2-item buffer, a floor posted now for an hour, and
/// `itemArrived` accepting up to 0.7 ETH.
fn floor_funded(w: &mut World, net: u128) {
    w.floor = FloorC { net, escrow: E, buffer: E / 5, price: 28 * E / 100, posted_at: w.clock.time, ttl: 3_600, max_cost: 70 * E / 100, arrived: 0 };
}

#[test]
fn a_floor_module_below_one_item_plus_its_buffer_buys_nothing_and_at_it_buys_one_frog_wraps_it_to_the_module_and_claims() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl) = floor_state();
    floor_funded(&mut w, E + E / 5 - 1);
    tickf(&mut w, &mut st, &mut fl);
    assert!(st.frogs.is_empty(), "net demand one wei short of one item + buffer: nothing wanted");
    assert_eq!(st.sinks[0].view.as_ref().map(|v| (v.kind, v.want)), Some((SinkKind::Floor, 0)));
    w.floor.net = E + E / 5;
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { cap_wei } if cap_wei == crate_max()), "no limit set: the policy's 0.5 ETH binds");
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    w.mine_btc(1);
    tickf(&mut w, &mut st, &mut fl);
    w.mine_btc(5);
    // an hour and more later the price is stale; the held frog is still minted and claimed
    w.advance(2 * 3_600);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 0 }), "{:?}", st.pending);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(w.wrapped_units(&FLOOR), 1, "one wFROG wrapped to the module");
    let p = st.pending.clone().unwrap();
    assert_eq!(p.step, FStep::Report { frog: 0 });
    assert_eq!(&rlp_items(&p.raw)[7][..4], &floorsink::ITEM_ARRIVED, "the report to a module is itemArrived");
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    let FPhase::Settled { buy } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    assert_eq!((w.floor.arrived, w.paid, w.floor.net), (1, buy.cost_wei, E / 5), "claimed from the escrow; net back under the trigger");
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(st.frogs.len(), 1, "nothing more is bought");
}

fn crate_max() -> u128 {
    etch_autobuyer::policy::MAX_PURCHASE_WEI
}

#[test]
fn a_stale_floor_starts_no_purchase_until_the_keeper_posts_again() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl) = floor_state();
    floor_funded(&mut w, 3 * E);
    w.floor.posted_at = w.clock.time - 3_600 + etch_autobuyer::policy::CAP_MARGIN_SECS - 1;
    let lines = tickf(&mut w, &mut st, &mut fl);
    assert!(st.frogs.is_empty() && lines.iter().any(|l| l.contains("stale")), "{lines:?}");
    w.floor.posted_at = w.clock.time;
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    // the price goes stale while the frog is still only wanted: the want fails, nothing spent, and waits for a fresh price
    w.advance(3_600);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Failed { .. }));
    assert!(w.btc_sent.is_empty());
}

#[test]
fn the_per_collection_limit_buys_a_0_64_eth_frog_the_global_maximum_refuses() {
    let price = 1_990_000; // sats to the seller: ≈0.64 ETH all-in at 3.19M sats/ETH
    for limited in [false, true] {
        let mut w = World::new();
        let proofs = w.collection(&[insc(1), insc(2)]);
        let (mut st, mut fl) = floor_state();
        if limited {
            fl.set_limit(FLOOR, 70 * E / 100).unwrap();
        }
        floor_funded(&mut w, 2 * E);
        tickf(&mut w, &mut st, &mut fl);
        let FPhase::Wanted { cap_wei } = phase(&st, 0) else { panic!() };
        assert_eq!(cap_wei, if limited { 70 * E / 100 } else { crate_max() });
        let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
        w.list(&insc(1), seller);
        w.offers.insert(0, w.offer_for(0, &insc(1), seller, price, proofs[0].clone()));
        let lines = tickf(&mut w, &mut st, &mut fl);
        if limited {
            let FPhase::Signed { buy } = phase(&st, 0) else { panic!("{lines:?}") };
            assert!(buy.cost_wei > crate_max() && buy.cost_wei < 70 * E / 100, "{}", buy.cost_wei);
        } else {
            assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }), "refused under the global 0.5 ETH: {lines:?}");
            assert!(w.btc_sent.is_empty());
        }
    }
    // and a queue with no limit set behaves exactly as before: the same frog is refused by the global maximum
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    w.queue = QueueC { queued: 2 * E, cap: 70 * E / 100, buffer: 0, recorded: 0, head: (T0 + 7 * 86_400, B0 + 50_400), head_amount: 2 * E, payee: ME };
    let mut st = queue_state();
    let mut fl = FloorState::default();
    tickf(&mut w, &mut st, &mut fl);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, price, proofs[0].clone()));
    let lines = tickf(&mut w, &mut st, &mut fl);
    assert!(lines.iter().any(|l| l.contains("per-purchase maximum")), "{lines:?}");
    // …until a controller gives that queue a limit
    fl.set_limit(QUEUE, 70 * E / 100).unwrap();
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, price, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Signed { .. }));
}

#[test]
fn a_failed_buy_is_retried_a_bad_offer_then_a_vanished_listing_then_a_good_one() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl) = floor_state();
    floor_funded(&mut w, 2 * E);
    tickf(&mut w, &mut st, &mut fl);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    // over the cap: refused, nothing signed, still wanted
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, 3_000_000, proofs[0].clone()));
    let lines = tickf(&mut w, &mut st, &mut fl);
    assert!(lines.iter().any(|l| l.contains("offer refused")) && w.btc_sent.is_empty(), "{lines:?}");
    // a good offer, then the listing vanishes before ours confirms
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    let FPhase::Signed { buy: b } = phase(&st, 0) else { panic!() };
    w.txs.remove(&b.outpoint.txid);
    w.outspends.insert(seller, ([0x99; 32], true));
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Failed { .. }));
    // the module still wants one: a new frog, bought from another seller
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 1), FPhase::Wanted { .. }));
    let seller2 = Outpoint { txid: [0xe2; 32], vout: 0 };
    buy(&mut w, &mut st, 1, &insc(2), seller2, proofs[1].clone());
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 1), FPhase::Held { .. }));
}

#[test]
fn a_4663_reorg_that_undoes_mints_fails_the_want_and_a_held_frog_waits_in_inventory() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl) = floor_state();
    floor_funded(&mut w, E + E / 5);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    // the blocks that minted the demand are reorganised away: the next pin reads less than one item + buffer
    w.floor.net = E;
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Failed { .. }), "{:?}", phase(&st, 0));
    // demand back, bought and held; then another reorg drops it before the mint: not minted, inventory
    w.floor.net = E + E / 5;
    tickf(&mut w, &mut st, &mut fl);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    buy(&mut w, &mut st, 1, &insc(1), seller, proofs[0].clone());
    w.floor.escrow = E / 10; // the escrow the burns drained: it cannot pay for the frog
    tickf(&mut w, &mut st, &mut fl);
    let f = st.frogs.iter().find(|f| f.id == 1).unwrap();
    assert!(matches!(f.phase, FPhase::Held { .. }) && f.target.is_none(), "{f:?}");
    assert!(w.calls.is_empty() && st.pending.is_none(), "nothing minted to a module that cannot pay");
    // the escrow refills and demand grows to two items: the held frog goes back to the module from inventory, and is minted
    w.floor.escrow = E;
    w.floor.net = 2 * E + E / 5;
    tickf(&mut w, &mut st, &mut fl);
    let f = st.frogs.iter().find(|f| f.id == 1).unwrap();
    assert_eq!(f.target, Some(Target::Sink(FLOOR)));
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 1 }), "{:?}", st.pending);
}

#[test]
fn an_evm_floor_sink_starts_an_nft_want_through_the_same_trigger_and_claims_once_the_unit_reaches_the_module() {
    let mut w = World::new();
    let mut st = FrogState { sinks: vec![SinkRec { kind: SinkKind::Floor, sink: FLOOR_EVM, coin: [0; 20], view: None, refused: None,
        drove_refunds_at: None, drove_settle: false }], ..Default::default() };
    let mut fl = FloorState::default();
    fl.add_sink(FLOOR_EVM, FloorItem::Evm { chain_id: 1, collection: APE, wrapper: NFTW }).unwrap();
    fl.set_limit(FLOOR_EVM, 6 * E).unwrap();
    w.floor_evm = FloorC { net: E, escrow: 12 * E, buffer: E / 5, price: 5 * E, posted_at: w.clock.time, ttl: 3_600, max_cost: 55 * E / 10, arrived: 0 };
    let lines = tickf(&mut w, &mut st, &mut fl);
    assert!(fl.wants.is_empty() && st.frogs.is_empty(), "{lines:?}");
    w.floor_evm.net = E + E / 5;
    tickf(&mut w, &mut st, &mut fl);
    assert!(st.frogs.is_empty(), "no frog for an ape module");
    assert_eq!((fl.wants.len(), fl.wants[0].cap_wei), (1, 55 * E / 10), "the module's maxItemCost, under the 6 ETH limit");
    assert!(fl.may_sign(0, 1, &APE, 51 * E / 10).is_ok(), "a 5.1 ETH ape passes the collection limit the global 0.5 would refuse");
    // bought and settled on chain 1 (the NFT line; floornet's endpoints move the want)
    fl.want_mut(0).unwrap().phase = WPhase::Arrived { token_id: "9820".into(), tx_hash: "0xab".into(), cost_wei: 51 * E / 10 };
    let lines = tickf(&mut w, &mut st, &mut fl);
    assert!(st.pending.is_none() && lines.iter().any(|l| l.contains("not minted to it yet")), "{lines:?}");
    // the NFT wrapper's custodian mint puts the unit at the module: the claim goes out, and is paid
    w.nft_units_at_module = E;
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::FloorClaim { want: 0 }), "{:?}", st.pending);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(fl.wants[0].phase, WPhase::Claimed { .. }), "{:?}", fl.wants[0].phase);
    assert_eq!((w.floor_evm.arrived, w.paid), (1, 51 * E / 10));
    assert_eq!(w.saved_floor.as_ref().map(|f| f.wants[0].phase.clone()), Some(fl.wants[0].phase.clone()), "saved as it changed");
}

// ---- the custody hardening, wired (docs/v5/custody-hardening.md, H2–H4) -------------------------------------------------------

fn in_flight(w: &World, id: &str) -> Option<bool> {
    w.book.entries.iter().find(|e| e.id == id).map(|e| e.in_flight)
}

#[test]
fn a_bought_frog_is_reserved_before_signing_held_under_its_inscription_and_its_price_and_mint_are_counted() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    assert!(w.book.entries.is_empty(), "wanted is not yet acquiring: nothing reserved");
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    buy(&mut w, &mut st, 0, &insc(1), seller, proofs[0].clone());
    assert_eq!(in_flight(&w, "frog:0"), Some(true), "reserved at its cost before the float key signed");
    let FPhase::Sent { buy } = phase(&st, 0) else { panic!() };
    assert_eq!(w.book.entries[0].value_wei, buy.cost_wei);
    assert!(w.asked.contains(&("buy:frog:0".to_string(), outflow::Flow::Value)) && w.recorded.contains(&"buy:frog:0".to_string()));
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Held { .. }));
    assert_eq!((in_flight(&w, "frog:0"), in_flight(&w, &insc(1))), (None, Some(false)), "held under its inscription");
    // the wrapDeposit is a mint on our word: asked before it was signed, counted once saved
    assert!(w.asked.contains(&("mint:frog:0".to_string(), outflow::Flow::Mint)) && w.recorded.contains(&"mint:frog:0".to_string()));
}

#[test]
fn with_no_room_under_the_caps_no_frog_is_wanted_and_a_listing_that_vanishes_frees_its_reservation() {
    let mut w = World::new();
    funded(&mut w);
    // one item per collection, and custody already holds one
    w.caps.collections[0].max_items = 1;
    let caps = w.caps.clone();
    custodycap::admit(&caps, &mut w.book, &insc(9), &frogs_key(), 1).unwrap();
    let mut st = queue_state();
    let lines = tick(&mut w, &mut st);
    assert!(st.frogs.is_empty(), "nothing wanted");
    assert!(lines.iter().any(|l| l.contains("custody cap")), "{lines:?}");
    // room again (it was delivered): the frog is wanted, bought, and when the listing vanishes its room comes back
    custodycap::release(&mut w.book, &insc(9)).unwrap();
    let proofs = w.collection(&[insc(1), insc(2)]);
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    let o = w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone());
    w.offers.insert(0, o);
    tick(&mut w, &mut st);
    assert_eq!(in_flight(&w, "frog:0"), Some(true));
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!() };
    w.txs.remove(&buy.outpoint.txid);
    w.outspends.insert(seller, ([0x99; 32], true));
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Failed { .. }));
    assert!(w.book.entries.is_empty(), "nothing of ours was spent: the reservation is gone");
}

/// Review S-01 (ops/rehearsal/sim, 2026-10-09), on the operator's `frog_offer` path: an offer whose seller input is not final
/// (no witness), or whose seller signed other than `SIGHASH_SINGLE|ANYONECANPAY`, is refused by the judge with the float key never
/// asked; the frog stays wanted, and the honest offer after it is bought.
#[test]
fn an_operator_offer_without_the_sellers_final_witness_is_refused_and_nothing_is_signed() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    let honest = w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone());
    let p = Psbt::parse(&honest.psbt).unwrap();
    let mut all = vec![7u8; 65];
    all[64] = 0x01;
    for (finals, why) in [(vec![None, None, None, None], "NotFinal(2)"),
        (vec![None, None, Some(vec![all]), None], "SellerSighash { input: 2, sighash: 1 }")] {
        let o = Offer { psbt: frogbuy::psbt_bytes(&p, &finals), ..honest.clone() };
        w.offers.insert(0, o);
        let lines = tick(&mut w, &mut st);
        assert!(lines.iter().any(|l| l.contains("offer refused, nothing signed") && l.contains(why)), "{why}: {lines:?}");
        assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }), "{why}: {lines:?}");
        assert!(w.digests.is_empty(), "{why}: the float key was asked to sign {} digests", w.digests.len());
        assert!(w.btc_sent.is_empty() && w.book.entries.is_empty() && w.offers.is_empty(), "{why}");
    }
    // the same PSBT with the seller's 0x83 signature: bought, signed on our three inputs
    w.offers.insert(0, honest);
    let lines = tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Signed { .. }), "{lines:?}");
    assert_eq!(w.digests.len(), 3);
}

#[test]
fn an_offer_over_the_value_left_under_the_caps_is_refused_before_anything_is_signed() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    w.caps.collections[0].max_value_wei = E / 10; // under one frog's cost
    let mut st = queue_state();
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    let o = w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone());
    w.offers.insert(0, o);
    let lines = tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("offer refused, nothing signed") && l.contains("CollectionValue")), "{lines:?}");
    assert!(w.btc_sent.is_empty() && w.book.entries.is_empty());
}

#[test]
fn a_deposit_custody_has_no_room_for_is_refused_and_returned_and_one_that_fits_is_counted_until_it_leaves() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    w.caps.collections[0].max_items = 1;
    let mut st = dep_state(&[(insc(1), proofs[0].clone()), (insc(2), proofs[1].clone())]);
    // the first fits and is counted before it is minted
    let o1 = w.deposit(0x61, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(1)), sender(), 6);
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Deposit { outpoint: o1 }));
    assert_eq!(in_flight(&w, &insc(1)), Some(false), "admitted as held");
    assert_eq!(st.deposits[0].admitted, Some(insc(1)));
    assert!(w.asked.contains(&(format!("mint:{}", o1.text()), outflow::Flow::Mint)));
    w.mine_evm();
    tick(&mut w, &mut st);
    // the second does not fit: refused, returned whole to its sender, and never counted
    let o2 = w.deposit(0x62, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(2)), sender(), 6);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    let DPhase::Returning { reason, .. } = dphase(&st, &o2) else { panic!("{:?}", dphase(&st, &o2)) };
    assert!(matches!(reason, Reason::OverCustodyCap(ref e) if e.contains("CollectionItems")), "{reason:?}");
    assert_eq!(parse_signed(w.btc_sent.last().unwrap()).unwrap().outputs[0], (546, p2tr(0x5e)), "back to its sender");
    assert!(w.asked.contains(&(format!("return:{}", o2.text()), outflow::Flow::Return)) && w.recorded.contains(&format!("return:{}", o2.text())),
        "a return is its own flow (audit D, H-2)");
    assert_eq!(w.book.entries.len(), 1, "only the first is counted");
}

/// Audit D, H-2: returns have their own budget. Deliveries that used the day's items do not hold a return back (nor returns a
/// delivery); a return waits at its own limit and goes when the window slides; and return demand never trips the freeze, while
/// delivery demand still does.
#[test]
fn a_return_waits_at_its_own_limit_never_starves_deliveries_and_never_trips_the_freeze() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    // stage 0's limits, with the day's three delivery items already gone: a return is not held back by them
    w.limits = Some(outflow::Limits::stage0());
    for k in 0..3 {
        w.outflow.record(&format!("ticket:{k}"), &frogs_key(), outflow::Flow::Item, 1, w.now);
    }
    // ...but three returns already gone that day are its limit
    for k in 0..3 {
        w.outflow.record(&format!("return:earlier:{k}"), &frogs_key(), outflow::Flow::Return, 1, w.now);
    }
    let mut st = dep_state(&[(insc(1), proofs[0].clone())]);
    let o = w.deposit(0x65, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT)[..40])],
        frog_on(&insc(1)), sender(), 6);
    tick(&mut w, &mut st);
    let lines = tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Refused { .. }), "{:?}", dphase(&st, &o));
    assert!(lines.iter().any(|l| l.contains("outflow limit") && l.contains("Return")), "{lines:?}");
    let n = w.btc_sent.len();
    // a day later the window has room: it goes
    w.advance(86_400);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { .. }), "{:?}", dphase(&st, &o));
    assert_eq!(w.btc_sent.len(), n + 1);
    assert!(w.outflow.frozen().is_none());
    // return demand at any multiple of the limit never freezes; delivery demand at three times it still does
    let mut out = outflow::Outflow::default();
    for k in 0..100 {
        let v = out.check(&outflow::Limits::stage0(), &format!("return:junk:{k}"), &frogs_key(), outflow::Flow::Return, 1, w.now);
        assert!(!matches!(v, outflow::Verdict::Frozen(_)), "return {k}: {v:?}");
    }
    for k in 0..9 {
        let v = out.check(&outflow::Limits::stage0(), &format!("forged{k}"), &frogs_key(), outflow::Flow::Item, 1, w.now);
        assert_eq!(matches!(v, outflow::Verdict::Frozen(_)), k == 8, "the ninth distinct delivery ask at three a day trips it");
    }
}

#[test]
fn the_floor_claim_counts_the_ape_under_the_caps_and_is_a_mint_against_the_days_limit() {
    let mut w = World::new();
    let mut st = FrogState { sinks: vec![SinkRec { kind: SinkKind::Floor, sink: FLOOR_EVM, coin: [0; 20], view: None, refused: None,
        drove_refunds_at: None, drove_settle: false }], ..Default::default() };
    let mut fl = FloorState::default();
    fl.add_sink(FLOOR_EVM, FloorItem::Evm { chain_id: 1, collection: APE, wrapper: NFTW }).unwrap();
    fl.set_limit(FLOOR_EVM, 6 * E).unwrap();
    w.floor_evm = FloorC { net: E + E / 5, escrow: 12 * E, buffer: E / 5, price: 5 * E, posted_at: w.clock.time, ttl: 3_600, max_cost: 55 * E / 10, arrived: 0 };
    // apes not listed: no want starts
    w.caps.collections.retain(|c| c.collection != apes_key());
    let lines = tickf(&mut w, &mut st, &mut fl);
    assert!(fl.wants.is_empty() && lines.iter().any(|l| l.contains("NotListed")), "{lines:?}");
    w.caps = generous();
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(fl.wants.len(), 1);
    // bought and settled on chain 1 (in the canister, nft_status puts it in the book under this id)
    fl.want_mut(0).unwrap().phase = WPhase::Arrived { token_id: "9820".into(), tx_hash: "0xab".into(), cost_wei: 51 * E / 10 };
    w.nft_units_at_module = E;
    // the day's one ape mint is spent: the claim waits
    w.limits = Some(outflow::Limits { per_collection_mints: 1, ..outflow::Limits::stage0() });
    w.outflow.record("claim:earlier", &apes_key(), outflow::Flow::Mint, 1, w.now);
    let lines = tickf(&mut w, &mut st, &mut fl);
    assert!(st.pending.is_none() && lines.iter().any(|l| l.contains("outflow limit")), "{lines:?}");
    w.advance(86_400);
    w.floor_evm.posted_at = w.clock.time;
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::FloorClaim { want: 0 }), "{:?}", st.pending);
    let held = etch_autobuyer::nftnet::nft_item_id(1, &APE, "9820").unwrap();
    assert_eq!(in_flight(&w, &held), Some(false), "the claimed ape is counted as held");
    assert!(w.recorded.contains(&"claim:0".to_string()));
}

// ---- H8 for returns: an evicted return is abandoned only by spending its fee input to custody (custody-hardening.md, H8,
// resolved 2026-09-25, point 7) -------------------------------------------------------------------------------------------------

/// A refused deposit's return, signed and sent. Returns the deposit, the return's txid and bytes, and its fee input.
fn a_return_out(w: &mut World, st: &mut FrogState) -> (Outpoint, [u8; 32], Vec<u8>, Outpoint) {
    let proofs = w.collection(&[insc(1), insc(2)]);
    st.put_proof(insc(1), proofs[0].clone());
    let o = w.deposit(0x70, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT)[..40])],
        frog_on(&insc(1)), sender(), 6);
    tick(w, st);
    tick(w, st);
    let DPhase::Returning { txid, raw, fee_input, unseen_since: None, .. } = dphase(st, &o) else { panic!("{:?}", dphase(st, &o)) };
    (o, txid, raw, fee_input)
}

/// The network never relays it (below the mempool floor after a fee spike): each look finds nothing, and the same bytes go again.
fn unrelayed(w: &mut World, txid: [u8; 32]) {
    w.txs.remove(&txid);
}

#[test]
fn a_return_bitcoin_stops_showing_for_72_hours_ends_with_its_fee_input_spent_to_custody_then_goes_back_whole() {
    let mut w = World::new();
    let mut st = FrogState::default();
    let (o, ret, raw, fee_input) = a_return_out(&mut w, &mut st);
    unrelayed(&mut w, ret);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { unseen_since: Some(t), .. } if t == w.now), "the clock starts");
    assert_eq!(w.btc_sent.last().unwrap(), &raw, "the same bytes again");
    // a mempool shows it again: the clock resets
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { unseen_since: None, .. }));
    // then it is gone for good: 71 hours is not yet abandonment
    unrelayed(&mut w, ret);
    tick(&mut w, &mut st);
    w.advance(71 * 3600);
    unrelayed(&mut w, ret);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { .. }));
    let sent_before = w.btc_sent.len();
    // past 72 hours: abandoned. The fee input is spent to custody in the same pass; the frog does not move
    w.advance(2 * 3600);
    unrelayed(&mut w, ret);
    tick(&mut w, &mut st);
    let DPhase::ReturnConflicting { conflict, .. } = dphase(&st, &o) else { panic!("{:?}", dphase(&st, &o)) };
    assert_eq!((conflict.leaked_txid, conflict.fee_input, conflict.item), (ret, fee_input, o));
    let tx = conflict.tx.clone().expect("built at once");
    assert_eq!(w.btc_sent.len(), sent_before + 1, "one new transaction: the conflict, not the return again");
    let p = parse_signed(&tx.raw).unwrap();
    assert_eq!(p.inputs.iter().map(|i| Outpoint { txid: display(i.txid), vout: i.vout }).collect::<Vec<_>>(), vec![fee_input]);
    assert_eq!(p.outputs, vec![(20_000 - tx.fee, custody())], "all of it back to custody");
    assert_eq!(tx.fee, 5 * 212 + 111, "the return's own fee at 5 sat/vB plus 111 vB at 1 sat/vB");
    assert_eq!(st.reserved(), vec![fee_input], "claimed until it settles");
    // a block: the conflict confirms; the old return can never be mined, and the deposit is refused again
    w.mine_btc(1);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Refused { reason: Reason::Malformed(_) }), "{:?}", dphase(&st, &o));
    assert!(!w.utxos.iter().any(|u| u.outpoint == fee_input), "its fee input is spent, in a block");
    // the next pass builds a fresh return at today's rate, from another fee input, the frog whole to its sender as before
    tick(&mut w, &mut st);
    let DPhase::Returning { txid: ret2, fee_input: fee2, .. } = dphase(&st, &o) else { panic!("{:?}", dphase(&st, &o)) };
    assert!(ret2 != ret && fee2 != fee_input);
    let again = parse_signed(w.btc_sent.last().unwrap()).unwrap();
    assert_eq!((display(again.inputs[0].txid), again.outputs[0].clone()), (o.txid, (546, p2tr(0x5e))));
    w.mine_btc(1);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returned { txid, .. } if txid == ret2));
}

#[test]
fn an_abandoned_return_that_confirms_after_all_is_returned_and_a_paused_buyer_signs_no_conflict() {
    let mut w = World::new();
    let mut st = FrogState::default();
    let (o, ret, _, _) = a_return_out(&mut w, &mut st);
    unrelayed(&mut w, ret);
    tick(&mut w, &mut st);
    // paused (a guardian, or an H3 trip): the abandonment is recorded, but nothing is signed or sent
    w.paused = true;
    w.advance(73 * 3600);
    unrelayed(&mut w, ret);
    let sent = w.btc_sent.len();
    let lines = tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::ReturnConflicting { ref conflict, .. } if conflict.tx.is_none()), "{:?}", dphase(&st, &o));
    assert!(lines.iter().any(|l| l.contains("paused")), "{lines:?}");
    assert_eq!(w.btc_sent.len(), sent);
    // resumed: the conflict goes out; the limiter is never asked for it (it is not an outflow)
    w.paused = false;
    let asked = w.asked.len();
    tick(&mut w, &mut st);
    let DPhase::ReturnConflicting { conflict, .. } = dphase(&st, &o) else { panic!() };
    let mine = conflict.tx.expect("built").txid;
    assert_eq!(w.asked.len(), asked);
    // but a miner had kept the old return, and mines it instead of our conflict
    w.txs.remove(&mine);
    w.txs.insert(ret, BtcTx { height: None, hash: None, view: None });
    w.mine_btc(1);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returned { txid, .. } if txid == ret), "{:?}", dphase(&st, &o));
}

// ---- audit D: junk deposits, per-tick cost, save-then-send, stale buys (H-2, H-3, L-1, L-2, L-5) ---------------------------------

/// A stranger's "deposit": one custody output of plain sats in its own transaction, six deep, with an `EWD1` marker that is
/// deliberately malformed (truncated). Nothing about it needs an inscription, a frog, or the operator (the H-2 PoC's).
fn junk_deposit(w: &mut World, tag: u8) -> Outpoint {
    let plain = Contents { inscriptions: vec![], runes: vec![], value: 2_000 };
    w.deposit(tag, vec![OutView { value: 2_000, script: custody() }, op_return(&payload(4663, W, RECIPIENT)[..40])], plain, sender(), 6)
}

/// The same, carrying an inscription each: what a stranger must now spend to make custody return anything at all.
fn inscribed_junk(w: &mut World, tag: u8) -> Outpoint {
    w.deposit(tag, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT)[..40])],
        frog_on(&insc(tag)), sender(), 6)
}

/// AUDIT-D H-2 regression (the PoC `audit_d_anyone_trips_the_custody_freeze_with_refused_junk_deposits_and_it_re_trips_after_resume`,
/// now failing as it should). Phase 1 has holder deposits OFF: ten junk deposits are never read, returned or asked of the
/// limiter, and custody never freezes. Switched on, plain-sat junk is `NotAFrog` (never returned); even inscribed junk goes back
/// at the RETURN limit only, never trips the freeze, and never uses a delivery's budget.
#[test]
fn audit_d_junk_deposits_can_no_longer_freeze_custody() {
    // Phase 1: deposits off
    let mut w = World::new();
    w.deposits = false;
    w.limits = Some(outflow::Limits::stage0());
    let mut st = FrogState::default();
    let junk: Vec<Outpoint> = (0..10u8).map(|i| junk_deposit(&mut w, 0x80 + i)).collect();
    for _ in 0..10 {
        tick(&mut w, &mut st);
    }
    assert!(!w.paused && w.outflow.frozen().is_none(), "never frozen");
    assert_eq!(w.reads, 0, "not one read of Bitcoin, ord or the explorers");
    assert!(st.deposits.is_empty() && w.btc_sent.is_empty() && w.asked.is_empty());
    // switched on: plain sats are never returned, whatever the marker says
    w.deposits = true;
    for _ in 0..10 {
        tick(&mut w, &mut st);
    }
    assert!(junk.iter().all(|o| dphase(&st, o) == DPhase::NotAFrog), "{:?}", st.deposits);
    assert!(!w.paused && w.btc_sent.is_empty() && w.asked.is_empty(), "nothing returned, nothing asked of the limiter");
    // inscribed junk: returned at three a day, their own budget, and never a freeze, over two days of ticks
    let mut w = World::new();
    w.limits = Some(outflow::Limits::stage0());
    let mut st = FrogState::default();
    let junk: Vec<Outpoint> = (0..10u8).map(|i| inscribed_junk(&mut w, 0x90 + i)).collect();
    for k in 0..10u8 {
        // custody's fee inputs: enough for every return, so only the limit decides
        let o = Outpoint { txid: [0x30 + k; 32], vout: 1 };
        w.utxos.push(Utxo { outpoint: o, value: 20_000, confirmed: true });
        w.contents.insert(o, Contents { inscriptions: vec![], runes: vec![], value: 20_000 });
    }
    for _ in 0..48 {
        tick(&mut w, &mut st);
        w.mine_btc(1);
        w.advance(3_600);
    }
    assert!(!w.paused && w.outflow.frozen().is_none(), "never frozen: {:?}", w.outflow.frozen());
    let gone = junk.iter().filter(|o| matches!(dphase(&st, o), DPhase::Returning { .. } | DPhase::Returned { .. })).count();
    assert!((3..=6).contains(&gone), "three a day at stage 0: {gone} in two days");
    assert!(w.asked.iter().all(|(_, f)| *f == outflow::Flow::Return), "no delivery budget touched: {:?}", w.asked);
}

/// AUDIT-D H-3 regression (the PoC `audit_d_pending_deposits_are_re_read_in_full_every_tick_forever`, now failing as it should).
/// Deposits waiting for a membership proof that never comes are not read again: once classified, a tick costs the same one
/// listing whether a stranger made 1, 8 or 40 of them; past 32 open ones nothing more is even classified.
#[test]
fn audit_d_a_ticks_reads_do_not_grow_with_a_strangers_deposits() {
    let steady = |n: u8| {
        let mut w = World::new();
        let mut st = FrogState::default(); // no proofs: Phase 1 never gives one
        for i in 0..n {
            w.deposit(0x20 + i, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))],
                frog_on(&insc(0x20 + i)), sender(), 6);
        }
        for _ in 0..20 {
            tick(&mut w, &mut st); // classify (8 a tick) and first checks (4 a tick)
        }
        let mut counts = Vec::new();
        for _ in 0..5 {
            w.reads = 0;
            tick(&mut w, &mut st);
            counts.push(w.reads);
        }
        (counts, st)
    };
    let (one, _) = steady(1);
    let (eight, _) = steady(8);
    let (forty, st) = steady(40);
    println!("AUDIT-D H-3: reads per tick in steady state with 1, 8 and 40 waiting deposits: {one:?} {eight:?} {forty:?}");
    assert!(one.iter().chain(&eight).chain(&forty).all(|c| *c == 1), "one listing a tick, whatever the count");
    assert_eq!(st.deposits.len(), deposits::MAX_OPEN_DEPOSITS, "no more than 32 tracked");
    assert!(st.deposits.iter().all(|d| matches!(d.phase, DPhase::AwaitingProof { .. })));
}

/// H-3: a deposit that never gets its proof is parked after 14 days (finished, read no more); the operator's proof wakes it.
#[test]
fn a_deposit_waiting_for_its_proof_is_parked_after_two_weeks_and_its_proof_wakes_it() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let mut st = FrogState::default();
    let o = w.deposit(0x6d, vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, RECIPIENT))], frog_on(&insc(1)), sender(), 6);
    tick(&mut w, &mut st);
    assert_eq!(dphase(&st, &o), DPhase::AwaitingProof { inscription: insc(1) });
    w.advance(deposits::PARK_AFTER_SECS);
    tick(&mut w, &mut st);
    assert_eq!(dphase(&st, &o), DPhase::Parked { inscription: insc(1) });
    assert!(st.deposits[0].finished());
    // frog_proof's effect: the proof kept, the deposit awake
    st.put_proof(insc(1), proofs[0].clone());
    st.deposits[0].phase = DPhase::AwaitingProof { inscription: insc(1) };
    tick(&mut w, &mut st);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Deposit { outpoint: o }), "{:?}", dphase(&st, &o));
}

/// H-3: a custody output whose transaction cannot be read (padded past the response cap, say) is set aside after three tries,
/// not read every tick for ever.
#[test]
fn a_custody_output_whose_transaction_cannot_be_read_is_set_aside_after_three_tries() {
    let mut w = World::new();
    let mut st = FrogState::default();
    let o = Outpoint { txid: [0x5f; 32], vout: 0 };
    w.utxos.push(Utxo { outpoint: o, value: 1_000, confirmed: true }); // no transaction the explorers will serve
    for _ in 0..3 {
        tick(&mut w, &mut st);
    }
    assert_eq!(st.unread.clone().unwrap_or_default(), vec![o]);
    w.reads = 0;
    tick(&mut w, &mut st);
    assert_eq!(w.reads, 1, "the listing, and nothing more");
}

/// AUDIT-D L-1 regression (the PoC `audit_d_a_return_is_sent_before_it_is_saved`, now failing as it should): at the moment a
/// return's bytes are handed to Bitcoin, the durable record already says `Returning`, with those bytes.
#[test]
fn audit_d_a_return_is_saved_before_it_is_sent() {
    let mut w = World::new();
    let mut st = FrogState::default();
    let o = inscribed_junk(&mut w, 0x71);
    tick(&mut w, &mut st);
    tick(&mut w, &mut st);
    assert!(matches!(dphase(&st, &o), DPhase::Returning { .. }), "{:?}", dphase(&st, &o));
    let durable = w.at_send.last().expect("a send").iter().find(|(p, _)| *p == o).map(|(_, s)| s.clone());
    assert_eq!(durable.as_deref(), Some("Returning"), "saved, then sent");
}

/// L-2: an upgrade that lands while a buy is being signed loses the continuation: the frog stays wanted with its room reserved.
/// The next tick releases the reservation, so one of stage 0's three slots is not consumed for ever.
#[test]
fn a_reservation_left_by_a_lost_signing_continuation_is_released() {
    let mut w = World::new();
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    // what `accept` had done before the upgrade dropped its callback: the reservation, and nothing after
    let caps = w.caps.clone();
    custodycap::reserve(&caps, &mut w.book, "frog:0", &frogs_key(), E / 4).unwrap();
    let lines = tick(&mut w, &mut st);
    assert!(!w.book.contains("frog:0"), "released");
    assert!(lines.iter().any(|l| l.contains("frog:0") && l.contains("released")), "{lines:?}");
}

/// L-5: a seller who keeps a conflicting spend alive holds a signed buy unseen for ever. After 48 hours it is abandoned: the
/// float's largest input of it is spent back to the float (the float's key, never custody's), and once that confirms the frog
/// has failed, its room under the caps is free, and a new purchase may start.
#[test]
fn a_stale_frog_buy_is_abandoned_by_spending_a_float_input_back_to_the_float() {
    let mut w = World::new();
    funded(&mut w);
    let mut st = queue_state();
    tick(&mut w, &mut st);
    let seller = Outpoint { txid: [0x5e; 32], vout: 3 };
    w.list(&insc(9), seller);
    let proof = w.collection(&[insc(9), insc(10)])[0].clone();
    let offer = w.offer_for(0, &insc(9), seller, PRICE, proof);
    w.offers.insert(0, offer);
    tick(&mut w, &mut st);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    assert!(w.book.contains("frog:0"));
    // the float's inputs as the explorers serve them
    for (t, v) in [([0xa1; 32], 600), ([0xa2; 32], 600), ([0xa3; 32], 10_000_000)] {
        w.outputs.insert(Outpoint { txid: t, vout: 0 }, (Output { script: float(), value: v, confirmed: true }, false));
    }
    // the seller's conflicting spend keeps ours out of every mempool, for two days
    for _ in 0..49 {
        w.txs.remove(&buy.outpoint.txid);
        tick(&mut w, &mut st);
        w.advance(3_600);
    }
    let FPhase::Abandoning { conflict, .. } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    let tx = conflict.tx.clone().expect("built and sent");
    let sent = parse_signed(w.btc_sent.last().unwrap()).unwrap();
    assert_eq!(display(sent.txid), tx.txid);
    assert_eq!(sent.inputs.iter().map(|i| Outpoint { txid: display(i.txid), vout: i.vout }).collect::<Vec<_>>(),
        vec![Outpoint { txid: [0xa3; 32], vout: 0 }], "the float's big input, alone; never the seller's");
    assert_eq!(sent.outputs.len(), 1);
    assert_eq!(sent.outputs[0].1, float(), "paid back to the float, not to custody or anyone else");
    assert!(tx.fee > buy.fee_sats, "it pays more than the buy, so it replaces it wherever the buy is");
    // a block: the conflict confirms; the buy can never be mined
    w.txs.remove(&buy.outpoint.txid);
    w.mine_btc(1);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Failed { ref why } if why.contains("abandoned")), "{:?}", phase(&st, 0));
    assert!(!w.book.contains("frog:0"), "its room is free again");
}

// ---- Relay conversion: a graduation's own ETH pays for its frog (docs/v6/relay-conversion.md) ---------------------------------------

const Q26: &str = include_str!("fixtures/relay-quote-4663-btc-2026-09-26.json");

/// Relay's real 2026-09-26 quote, re-made for this world: our EVM address and float, `amount`, at least `min` sats, deadlines a
/// week out, a salt of our choosing; the order id recomputed and committed in the calldata, as Relay does.
fn quote_for(w: &World, amount: u128, min: u64, salt: u8) -> Vec<u8> {
    let mut v: serde_json::Value = serde_json::from_str(Q26).unwrap();
    let me = format!("0x{}", hex::encode(ME));
    let dl = w.now + 7 * 86_400;
    {
        let o = &mut v["protocol"]["v2"]["orderData"];
        o["salt"] = format!("0x{}", hex::encode([salt; 32])).into();
        o["inputs"][0]["payment"]["amount"] = amount.to_string().into();
        o["inputs"][0]["refunds"][0]["recipient"] = me.clone().into();
        o["inputs"][0]["refunds"][1]["recipient"] = float_addr().into();
        o["inputs"][0]["refunds"][0]["deadline"] = dl.into();
        o["inputs"][0]["refunds"][1]["deadline"] = dl.into();
        o["output"]["payments"][0]["recipient"] = float_addr().into();
        o["output"]["payments"][0]["minimumAmount"] = min.to_string().into();
        o["output"]["deadline"] = dl.into();
    }
    let id = rc::order_id(&v["protocol"]["v2"]["orderData"]).unwrap();
    v["protocol"]["v2"]["orderId"] = format!("0x{}", hex::encode(id)).into();
    v["protocol"]["v2"]["paymentDetails"]["amount"] = amount.to_string().into();
    let d = &mut v["steps"][0]["items"][0]["data"];
    d["from"] = me.into();
    d["value"] = amount.to_string().into();
    d["data"] = format!("0x49290c1c000000000000000000000000{}{}", hex::encode(ME), hex::encode(id)).into();
    serde_json::to_vec(&v).unwrap()
}

fn conv(st: &FrogState) -> etch_autobuyer::relayconv::ConvRec {
    st.convs().last().cloned().expect("a conversion")
}

/// A Relay-funded floor world at the trigger: frog 0 wanted, its advance of 0.5 ETH sent and mined, the ask standing.
fn funded_conversion(w: &mut World) -> (FrogState, FloorState, u128) {
    w.funding = Some(Default::default());
    let (mut st, mut fl) = floor_state();
    floor_funded(w, E + E / 5);
    tickf(w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    let p = st.pending.clone().expect("the advance is the first EVM act");
    assert_eq!(p.step, FStep::Advance { frog: 0 });
    assert_eq!(&rlp_items(&p.raw)[7][..4], &rc::ADVANCE);
    assert_eq!(w.asked.last().map(|a| a.1), Some(outflow::Flow::Value), "H3 counts the advance's value before the module pays");
    w.mine_evm();
    assert_eq!((w.transit, w.advanced, w.balances[&ME]), (E, crate_max(), crate_max()), "the module paid the advance");
    tickf(w, &mut st, &mut fl);
    let c = conv(&st);
    let CPhase::Funded { wei, .. } = c.phase else { panic!("{:?}", c.phase) };
    assert_eq!(wei, crate_max() - rc::GAS_KEEP_WEI, "all but the gas keep is converted");
    let a = c.ask.expect("the runner is told exactly what to quote");
    assert_eq!((a.amount_wei, a.recipient.as_str(), a.user.as_str(), a.refund_to.as_str(), a.destination_chain_id),
        (wei, float_addr().as_str(), format!("0x{}", hex::encode(ME)).as_str(), format!("0x{}", hex::encode(ME)).as_str(), rc::RELAY_BITCOIN));
    assert!(st.pending.is_none(), "no quote yet: nothing is signed");
    (st, fl, wei)
}

/// The float receives `sats` in a new transaction mined `deep` blocks deep.
fn relay_fills(w: &mut World, tag: u8, sats: u64, deep: u64) -> Outpoint {
    let o = Outpoint { txid: [tag; 32], vout: 0 };
    w.txs.insert(o.txid, BtcTx { height: None, hash: None, view: None });
    w.mine_btc(deep);
    w.float_outs.push(Utxo { outpoint: o, value: sats, confirmed: true });
    o
}

#[test]
fn relay_the_module_advances_the_quote_is_judged_the_btc_arrives_the_frog_is_bought_and_settled_against_the_advance() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = funded_conversion(&mut w);
    // an offer before the sats arrive is not signed (and not kept)
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    assert!(w.btc_sent.is_empty() && matches!(phase(&st, 0), FPhase::Wanted { .. }), "no float spend before the conversion arrives");
    // the runner's quote: judged against our own reads and signed
    w.quotes.insert(0, (quote_for(&w, wei, 1_560_000, 1), w.now));
    tickf(&mut w, &mut st, &mut fl);
    let p = st.pending.clone().expect("the deposit");
    assert_eq!(p.step, FStep::Convert { frog: 0 });
    let f = rlp_items(&p.raw);
    assert_eq!(<[u8; 20]>::try_from(f[5].clone()).unwrap(), rc::RELAY_DEPOSITORY_4663, "to the pinned depository");
    assert_eq!(uint(&f[6]), wei, "exactly the ask");
    assert_eq!(&f[7][16..36], &ME, "we are the depositor");
    let order = conv(&st).orders[0];
    assert_eq!(&f[7][36..68], &order, "the calldata commits to the order we judged");
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Converting { min_sats: 1_560_000, .. }), "{:?}", conv(&st).phase);
    assert_eq!(w.relay_deposits, vec![(order, wei)]);
    // a smaller output to the float is not the arrival; the real one counts only three deep
    w.float_outs.push(Utxo { outpoint: Outpoint { txid: [0x70; 32], vout: 0 }, value: 1_000, confirmed: true });
    relay_fills(&mut w, 0x71, 1_580_000, 2);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Converting { .. }), "two deep is not enough");
    w.mine_btc(1);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Arrived { sats: 1_580_000, .. }), "{:?}", conv(&st).phase);
    // the frog: bought with what arrived, at its share of the advance
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    assert_eq!(buy.cost_wei, rc::cost_of(crate_max(), buy.spend_sats, 1_580_000).unwrap(), "its share of the advance");
    w.mine_btc(1);
    tickf(&mut w, &mut st, &mut fl);
    w.mine_btc(5);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 0 }), "{:?} {:?}", st.pending, phase(&st, 0));
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    let p = st.pending.clone().unwrap();
    assert_eq!(p.step, FStep::Report { frog: 0 });
    assert_eq!(&rlp_items(&p.raw)[7][..4], &rc::SETTLE_ADVANCE, "the report of an advanced frog is settleAdvance, never itemArrived");
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Settled { .. }));
    assert_eq!((w.transit, w.floor.arrived, w.paid), (0, 1, 0), "item-backed; nothing reimbursed: the module paid first");
    let CPhase::Settled { cost, spare_sats } = conv(&st).phase else { panic!() };
    assert_eq!((cost, spare_sats), (buy.cost_wei, 1_580_000 - buy.spend_sats));
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(st.frogs.len(), 1, "nothing more is wanted");
}

#[test]
fn relay_every_bad_quote_is_refused_nothing_is_signed_and_the_ask_stands() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = funded_conversion(&mut w);
    let good = quote_for(&w, wei, 1_560_000, 1);
    let mut bad_recipient: serde_json::Value = serde_json::from_slice(&good).unwrap();
    bad_recipient["protocol"]["v2"]["orderData"]["output"]["payments"][0]["recipient"] =
        "bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y".into();
    let cases: Vec<(Vec<u8>, u64, &str)> = vec![
        (serde_json::to_vec(&bad_recipient).unwrap(), w.now, "rule 6"),
        (quote_for(&w, wei - 1, 1_560_000, 2), w.now, "rule 4"),
        (quote_for(&w, wei, 1_000_000, 3), w.now, "rule 10"),
        (good.clone(), w.now - rc::QUOTE_TTL_SECS - 1, "stale"),
    ];
    for (body, at, why) in cases {
        w.quotes.insert(0, (body, at));
        tickf(&mut w, &mut st, &mut fl);
        assert!(st.pending.is_none(), "{why}: nothing signed");
        assert!(conv(&st).note.contains(why), "{why}: {}", conv(&st).note);
        assert!(matches!(conv(&st).phase, CPhase::Funded { .. }) && conv(&st).ask.is_some(), "{why}: the ask stands");
    }
    // relay_quote's twin: a quote for a frog whose conversion is not funded is never taken
    w.quotes.insert(0, (good, w.now));
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Convert { frog: 0 }));
}

#[test]
fn relay_refunds_are_retried_with_new_orders_a_replay_is_refused_and_after_three_the_eth_goes_back() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = funded_conversion(&mut w);
    let first = quote_for(&w, wei, 1_560_000, 1);
    let mut ask = wei;
    for attempt in 1..=3u8 {
        let body = if attempt == 1 { first.clone() } else { quote_for(&w, ask, 1_550_000, attempt) };
        if attempt == 2 {
            // the first order again, after its refund: never twice
            w.quotes.insert(0, (first.clone(), w.now));
            tickf(&mut w, &mut st, &mut fl);
            assert!(st.pending.is_none() && conv(&st).note.contains("rule 12"), "{}", conv(&st).note);
        }
        w.quotes.insert(0, (body, w.now));
        tickf(&mut w, &mut st, &mut fl);
        assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Convert { frog: 0 }), "attempt {attempt}");
        w.mine_evm();
        tickf(&mut w, &mut st, &mut fl);
        assert!(matches!(conv(&st).phase, CPhase::Converting { .. }));
        // Relay could not fill: the deposit comes back to our address, less its refund gas
        // (the first comes back whole, so the very same order would fit the new ask: only rule 12 stops it)
        let gas = if attempt == 1 { 0 } else { 1_000 };
        *w.balances.get_mut(&ME).unwrap() += ask - gas;
        tickf(&mut w, &mut st, &mut fl);
        assert_eq!(conv(&st).attempts, attempt as u32);
        if attempt < 3 {
            assert!(matches!(conv(&st).phase, CPhase::Funded { wei: x, .. } if x == ask - gas), "{:?}", conv(&st).phase);
            ask -= gas;
            // the next quote is for what came back
            return_to_ask(&mut w, &st);
        }
    }
    assert_eq!(conv(&st).orders.len(), 3, "three orders, all different");
    let CPhase::Returning { wei: back } = conv(&st).phase else { panic!("{:?}", conv(&st).phase) };
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::GiveBack { frog: 0 }), "{:?}", st.pending);
    let p = st.pending.clone().unwrap();
    assert_eq!((&rlp_items(&p.raw)[7][..4], uint(&rlp_items(&p.raw)[6])), (&rc::RETURN_ADVANCE[..], back));
    w.mine_evm();
    assert_eq!((w.transit, w.floor.net), (0, E + E / 5), "the unit is ETH-backed again");
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.conv(0).unwrap().phase, CPhase::Returned { .. }));
    assert!(matches!(phase(&st, 0), FPhase::Failed { .. }), "nothing bought");
    assert!(!w.book.entries.iter().any(|e| e.id == "frog:0"), "its custody room is released");
    // conversion review C-08: a day between a module's returned advance and its next one
    assert_eq!(st.frogs.len(), 1, "no new purchase within a day of the return");
    w.advance(rc::RETURN_COOLDOWN_SECS);
    floor_funded(&mut w, E + E / 5);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::NeedAdvance | CPhase::Advancing) && conv(&st).frog == 1, "the module still wants one: a new purchase, a new conversion");
}

fn return_to_ask(_w: &mut World, st: &FrogState) {
    let c = conv(st);
    let (CPhase::Funded { wei, .. }, Some(a)) = (&c.phase, &c.ask) else { panic!("{:?}", c.phase) };
    assert_eq!(a.amount_wei, *wei);
}

#[test]
fn relay_a_deposit_nobody_fills_or_refunds_is_stuck_blocks_new_purchases_and_a_late_refund_still_counts() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = funded_conversion(&mut w);
    w.quotes.insert(0, (quote_for(&w, wei, 1_560_000, 1), w.now));
    tickf(&mut w, &mut st, &mut fl);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    w.advance(7 * 86_400 + rc::DEADLINE_GRACE_SECS + 1);
    floor_funded(&mut w, 3 * E); // the module wants more, and posts a fresh floor
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Stuck { .. }), "{:?}", conv(&st).phase);
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(st.frogs.len(), 1, "a stuck conversion holds the line: no new purchase");
    assert!(etch_autobuyer::convflow::give_back_now(&mut st.clone(), 0, rc::GAS_KEEP_WEI, w.now).is_err(), "nothing here to give back");
    // Relay refunds late: it is still seen
    *w.balances.get_mut(&ME).unwrap() += wei;
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Funded { .. }), "{:?}", conv(&st).phase);
    // the operator's exit: the ETH goes back instead of a new quote
    let bal = w.balances[&ME];
    etch_autobuyer::convflow::give_back_now(&mut st, 0, bal, w.now).unwrap();
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::GiveBack { frog: 0 }));
}

#[test]
fn relay_float_first_uses_an_operator_float_that_covers_the_frog_and_otherwise_converts() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    w.funding = Some(rc::FrogFunding { max_fee_bps: 300, float_first: true });
    w.float_outs.push(Utxo { outpoint: Outpoint { txid: [0x60; 32], vout: 0 }, value: 2_000_000, confirmed: true });
    let (mut st, mut fl) = floor_state();
    floor_funded(&mut w, E + E / 5);
    tickf(&mut w, &mut st, &mut fl);
    assert!(st.convs().is_empty() && st.pending.is_none(), "the float covers 0.5 ETH at the rate: no advance");
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    w.funding = Some(rc::FrogFunding { max_fee_bps: 300, float_first: true });
    let (mut st, mut fl) = floor_state();
    floor_funded(&mut w, E + E / 5);
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(st.convs().len(), 1, "an empty float: the module's ETH is converted");
}

// ---- protected Ordinals Wallet listings (docs/v8/ow-protected.md §3.2–3.4) ------------------------------------------------------

const OW: &str = include_str!("fixtures/ow/purchase-verify.json");

/// One PSBT's key-value maps, raw: the global map, then one per input, then one per output.
fn psbt_maps(b: &[u8]) -> Vec<Vec<(Vec<u8>, Vec<u8>)>> {
    fn compact(b: &[u8], i: &mut usize) -> usize {
        let f = b[*i];
        *i += 1;
        match f {
            0..=0xfc => f as usize,
            0xfd => { let v = u16::from_le_bytes([b[*i], b[*i + 1]]) as usize; *i += 2; v }
            _ => panic!("no test PSBT is that large"),
        }
    }
    let p = Psbt::parse(b).unwrap();
    let mut i = 5;
    let mut maps = Vec::new();
    for _ in 0..1 + p.tx.inputs.len() + p.tx.outputs.len() {
        let mut m = Vec::new();
        loop {
            let kl = compact(b, &mut i);
            if kl == 0 { break; }
            let k = b[i..i + kl].to_vec();
            i += kl;
            let vl = compact(b, &mut i);
            m.push((k, b[i..i + vl].to_vec()));
            i += vl;
        }
        maps.push(m);
    }
    assert_eq!(i, b.len(), "nothing after the last map");
    maps
}

/// Offsets of output `j`'s value and script inside a PSBT's unsigned transaction (as tests/ow_protected.rs).
fn ow_output_offsets(p: &[u8], j: usize) -> (usize, usize) {
    let t = Psbt::parse(p).unwrap().tx;
    let mut i = 5 + 2 + 3 + 4 + 1; // magic, the key, the 0xfd length, the version, the input count (< 0xfd)
    i += t.inputs.len() * (36 + 1 + 4);
    i += 1; // the output count
    for o in &t.outputs[..j] {
        i += 8 + 1 + o.script.len();
    }
    assert_eq!(p[i + 8], 34);
    (i, i + 9)
}

/// A protected sale from Ordinals Wallet's own vectors, as a build with `from` = the fixture's buyer (the float here) and `to` =
/// this world's custody would be: the output the escrow's first sat lands on pays custody (tests/ow_protected.rs's rewrite).
/// `with_setup`: the fixture's "with setup" purchase, made consistent (its sale states the setup's output 1 at 20,000 sats while
/// the setup pays it 99,100: the sale's input is restated at 99,100 and its last change output takes the difference, 300 sats
/// less, so the fees fit `fee_bound`).
struct OwCase {
    offer: ProtectedOffer,
    float: Vec<u8>,
}

fn ow_case(with_setup: bool, proof: Vec<[u8; 32]>) -> OwCase {
    let d: serde_json::Value = serde_json::from_str(OW).unwrap();
    let (input, link) = if with_setup {
        let c = d["purchases"].as_array().unwrap().iter().find(|c| c["description"] == "with setup").unwrap();
        (c["input"].clone(), c["input"]["links"][0].clone())
    } else {
        let c = &d["sales"][0];
        assert_eq!(c["description"], "honest sale");
        (c["input"].clone(), c["input"].clone())
    };
    let mut sale_hex = link["sale_psbt"].as_str().unwrap().to_string();
    if with_setup {
        let stated = "01012b204e000000000000225120a608";
        assert_eq!(sale_hex.matches(stated).count(), 1);
        sale_hex = sale_hex.replace(stated, &format!("01012b{}225120a608", hex::encode(99_100u64.to_le_bytes())));
    }
    let mut sale = hex::decode(&sale_hex).unwrap();
    if with_setup {
        let (at, _) = ow_output_offsets(&sale, 5);
        let v = u64::from_le_bytes(sale[at..at + 8].try_into().unwrap());
        assert_eq!(v, 17_150);
        sale[at..at + 8].copy_from_slice(&(v + 79_100 + 300).to_le_bytes());
    }
    // the item's output (2: the escrow's first sat, after input 0) pays custody, not the buyer
    let (_, script_at) = ow_output_offsets(&sale, 2);
    sale[script_at..script_at + 34].copy_from_slice(&custody());
    let l = &link["listing"];
    let listing = etch_autobuyer::owbuy::Listing {
        outpoint: Outpoint::parse(l["outpoint"].as_str().unwrap()).unwrap(),
        seller_address: l["seller_address"].as_str().unwrap().into(),
        satoshi_price: l["satoshi_price"].as_u64().unwrap(),
        escrow_price: l["escrow_price_sat"].as_u64(),
        creator_address: l["creator_address"].as_str().map(String::from),
    };
    let float = etch_autobuyer::policy::script_of(input["buyer_address"].as_str().unwrap(), "bc").unwrap();
    let setup_psbt = with_setup.then(|| hex::decode(input["setup"]["psbt"].as_str().unwrap()).unwrap());
    OwCase { offer: ProtectedOffer { inscription: insc(1), sale_psbt: sale, parent: hex::decode(link["parent"]["raw"].as_str().unwrap()).unwrap(),
        listing, setup_psbt, proof }, float }
}

/// A queue wanting frog 0, the float the fixture's buyer, the rate such that the fixture's ~53,000 sats are within the policy's
/// limits, and the inscription where the listing sells it: ord's location, its contents, the explorers' output.
fn ow_world(with_setup: bool) -> (World, FrogState, OwCase) {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    funded(&mut w);
    let c = ow_case(with_setup, proofs[0].clone());
    w.float_script = Some(c.float.clone());
    w.rate = Some(800_000);
    let at = c.offer.listing.outpoint;
    w.loc.insert(insc(1), Location { output: at, offset: 0, value: 546 });
    w.contents.insert(at, Contents { inscriptions: vec![insc(1)], runes: vec![], value: 546 });
    w.outputs.insert(at, (Output { script: p2tr(0xee), value: 546, confirmed: true }, false));
    let mut st = queue_state();
    // review P-01: the collection's creator pinned by a controller (set_ow_creators), as an install for it would
    if let Some(a) = &c.offer.listing.creator_address {
        st.set_ow_creators(vec![a.clone()]).unwrap();
    }
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }));
    (w, st, c)
}

/// The float's inputs of a PSBT, (index, outpoint, value).
fn float_inputs(p: &[u8], float: &[u8]) -> Vec<(usize, Outpoint, u64)> {
    let p = Psbt::parse(p).unwrap();
    (0..p.tx.inputs.len()).filter_map(|i| {
        let u = p.witness_utxos[i].as_ref()?;
        (u.script == float).then(|| (i, Outpoint { txid: display(p.tx.inputs[i].txid), vout: p.tx.inputs[i].vout }, u.value))
    }).collect()
}

/// The signed PSBT is the one handed in, with exactly `0x13` (the float's signature) and `0x17` (its internal key) added to
/// exactly the float's inputs: every other map, pair and byte as it came.
fn assert_signed_only_ours(signed: &[u8], original: &[u8], ours: &[usize]) {
    let (a, b) = (psbt_maps(signed), psbt_maps(original));
    assert_eq!(a.len(), b.len());
    assert_eq!(Psbt::parse(signed).unwrap(), Psbt::parse(original).unwrap(), "the transaction and every prevout unchanged");
    let n_in = Psbt::parse(original).unwrap().tx.inputs.len();
    for (m, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        let input = (1..=n_in).contains(&m).then(|| m - 1);
        if input.is_some_and(|i| ours.contains(&i)) {
            assert_eq!(&x[..y.len()], &y[..], "input {m}: what was there, in order, first");
            let added: Vec<(Vec<u8>, Vec<u8>)> = x[y.len()..].to_vec();
            assert_eq!(added, vec![(vec![0x13], vec![0x11; 64]), (vec![0x17], FLOAT_INTERNAL.to_vec())], "input {m}: 0x13 and 0x17 only");
        } else {
            assert_eq!(x, y, "map {m} untouched");
        }
    }
}

#[test]
fn a_protected_offer_is_judged_signed_on_the_float_inputs_only_handed_back_and_never_broadcast() {
    let (mut w, mut st, c) = ow_world(false);
    assert!(st.protected_signed(0).is_none(), "nothing to hand back while wanted");
    w.protected.insert(0, c.offer.clone());
    let lines = tick(&mut w, &mut st);
    let FPhase::Handed { buy, sale_txid, at } = phase(&st, 0) else { panic!("{:?} {lines:?}", phase(&st, 0)) };
    let sale = Psbt::parse(&c.offer.sale_psbt).unwrap();
    assert_eq!(sale_txid, hex::encode(sale.txid()));
    assert_eq!((buy.outpoint, at, buy.raw.len()), (Outpoint { txid: sale.txid(), vout: 2 }, T0, 0), "custody's output; no bytes held");
    // spend 52,850 (as tests/ow_protected.rs), costed as a classic buy at the rate recorded with it
    assert_eq!((buy.spend_sats, buy.fee_sats, buy.rate), (52_850, 1_000, 800_000));
    assert_eq!(buy.cost_wei, frogbuy::wei_for_sats(52_850, 800_000).unwrap());
    // the float's inputs: checked clean, signed over this PSBT's DEFAULT digests, and nothing else
    let ours = float_inputs(&c.offer.sale_psbt, &c.float);
    assert_eq!(ours.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 2]);
    assert_eq!(w.cleaned, ours.iter().map(|x| x.1).collect::<Vec<_>>());
    let want: Vec<[u8; 32]> = [0, 2].iter().map(|&i| etch_autobuyer::sighash::taproot_key_path(&sale, i).unwrap()).collect();
    assert_eq!(w.digests, want);
    let rec = st.protected_signed(0).expect("handed: the signed PSBTs are there for the runner");
    assert_signed_only_ours(&rec.sale_psbt, &c.offer.sale_psbt, &[0, 2]);
    assert!(rec.setup_psbt.is_none() && rec.setup_txid.is_none());
    assert_eq!((rec.sale_fee, rec.setup_fee), (1_000, 0));
    // H2 and H3, under a classic buy's keys; and nothing broadcast, ever
    assert!(w.book.contains("frog:0"));
    // review P-02: H3 is asked before signing but counts the value only once Bitcoin shows the sale
    assert!(w.asked.contains(&("buy:frog:0".to_string(), outflow::Flow::Value)) && !w.recorded.contains(&"buy:frog:0".to_string()));
    assert!(w.btc_sent.is_empty(), "the canister cannot broadcast a protected sale");
    // an hour on, still unseen: watched, nothing sent
    w.advance(3_600);
    let lines = tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Handed { .. }) && w.btc_sent.is_empty(), "{lines:?}");
    // Ordinals Wallet relays it: Sent, then the usual path to Held
    w.txs.insert(sale.txid(), BtcTx { height: None, hash: None, view: None });
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Sent { .. }), "{:?}", phase(&st, 0));
    assert!(st.protected_signed(0).is_none(), "only while handed");
    assert!(w.recorded.contains(&"buy:frog:0".to_string()), "seen: its value is counted now (P-02)");
    w.mine_btc(6);
    let landed = buy.outpoint;
    w.loc.insert(insc(1), Location { output: landed, offset: 0, value: 546 });
    w.contents.insert(landed, Contents { inscriptions: vec![insc(1)], runes: vec![], value: 546 });
    w.outputs.insert(landed, (Output { script: custody(), value: 546, confirmed: true }, false));
    block_on(frogflow::tick(&mut w, &mut st, true, &[]));
    assert!(matches!(phase(&st, 0), FPhase::Held { .. }), "{:?}", phase(&st, 0));
    assert!(w.btc_sent.is_empty());
    tick(&mut w, &mut st);
    // review P-04: the record is kept while the frog is held (a reorganisation puts it back to sent)
    assert!(st.protected_rec(0).is_some(), "kept while held");
}

#[test]
fn a_protected_sale_never_seen_is_abandoned_after_six_hours_through_the_sales_largest_float_input() {
    let (mut w, mut st, c) = ow_world(false);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let FPhase::Handed { buy, .. } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    for (_, o, v) in float_inputs(&c.offer.sale_psbt, &c.float) {
        w.outputs.insert(o, (Output { script: c.float.clone(), value: v, confirmed: true }, false));
    }
    w.advance(frogflow::PROTECTED_STALE_SECS - 1);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Handed { .. }) && w.btc_sent.is_empty(), "not yet");
    w.advance(1);
    let lines = tick(&mut w, &mut st);
    let FPhase::Abandoning { conflict, .. } = phase(&st, 0) else { panic!("{:?} {lines:?}", phase(&st, 0)) };
    let big = Outpoint { txid: [0x11; 32], vout: 1 };
    // review P-02: never seen, the sale's fee is not outbid: the conflict is built at the market rate
    assert_eq!((conflict.fee_input, conflict.leaked_txid, conflict.leaked_fee), (big, buy.outpoint.txid, Some(0)));
    let escrow_in = &Psbt::parse(&c.offer.sale_psbt).unwrap().tx.inputs[1];
    assert_eq!(conflict.item, Outpoint { txid: display(escrow_in.txid), vout: escrow_in.vout }, "the escrow: never a conflict's input");
    assert_eq!(w.btc_sent.len(), 1, "the conflict, and only it");
    let sent = parse_signed(&w.btc_sent[0]).unwrap();
    assert_eq!(sent.inputs.iter().map(|i| Outpoint { txid: display(i.txid), vout: i.vout }).collect::<Vec<_>>(), vec![big]);
    assert_eq!(sent.outputs.len(), 1);
    assert_eq!(sent.outputs[0].1, c.float, "back to the float");
    assert!(st.protected_signed(0).is_none());
    assert!(!w.recorded.contains(&"buy:frog:0".to_string()), "never seen: never counted (H3)");
    // the conflict confirms: nothing was bought, the room is free
    w.mine_btc(1);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Failed { ref why } if why.contains("abandoned after 6 h")), "{:?}", phase(&st, 0));
    assert!(!w.book.contains("frog:0"));
}

#[test]
fn a_protected_sale_with_a_setup_is_signed_in_both_and_abandoned_through_the_setups_input_while_it_is_open() {
    let (mut w, mut st, c) = ow_world(true);
    w.protected.insert(0, c.offer.clone());
    let lines = tick(&mut w, &mut st);
    let FPhase::Handed { buy, .. } = phase(&st, 0) else { panic!("{:?} {lines:?}", phase(&st, 0)) };
    assert_eq!((buy.spend_sats, buy.fee_sats), (53_450, 1_600), "the setup's 900 counted in both");
    let setup = c.offer.setup_psbt.clone().unwrap();
    let setup_txid = Psbt::parse(&setup).unwrap().txid();
    assert_eq!(hex::encode(setup_txid), "58a0663fdf6062b8d84a814b9d5301bfe1100696d4558cd069139aec69de6bef");
    let rec = st.protected_signed(0).unwrap().clone();
    assert_signed_only_ours(rec.setup_psbt.as_ref().unwrap(), &setup, &[0]);
    assert_signed_only_ours(&rec.sale_psbt, &c.offer.sale_psbt, &[0, 2]);
    assert_eq!((rec.setup_txid, rec.sale_fee, rec.setup_fee), (Some(setup_txid), 700, 900));
    // the setup's input checked clean; the sale's inputs are the setup's outputs, which ord cannot know yet
    let setup_in = Outpoint { txid: [0x99; 32], vout: 0 };
    assert_eq!(w.cleaned, vec![setup_in]);
    assert_eq!(w.digests.len(), 3, "one setup input, two sale inputs");
    assert!(w.btc_sent.is_empty() && w.book.contains("frog:0"));
    // six hours, nothing: the setup's input (a confirmed float coin) goes back to the float, which kills setup and sale
    w.outputs.insert(setup_in, (Output { script: c.float.clone(), value: 200_000, confirmed: true }, false));
    w.advance(frogflow::PROTECTED_STALE_SECS);
    let lines = tick(&mut w, &mut st);
    let FPhase::Abandoning { conflict, .. } = phase(&st, 0) else { panic!("{:?} {lines:?}", phase(&st, 0)) };
    // review P-02: neither the setup nor the sale was ever seen: nothing to outbid
    assert_eq!((conflict.fee_input, conflict.leaked_fee, conflict.earlier.clone()), (setup_in, Some(0), Some(vec![setup_txid])));
    let sent = parse_signed(w.btc_sent.last().unwrap()).unwrap();
    assert_eq!(sent.inputs.len(), 1);
    assert_eq!(Outpoint { txid: display(sent.inputs[0].txid), vout: sent.inputs[0].vout }, setup_in);
}

#[test]
fn a_protected_sale_whose_setup_confirmed_is_abandoned_through_the_sales_own_input() {
    let (mut w, mut st, c) = ow_world(true);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let setup_txid = Psbt::parse(c.offer.setup_psbt.as_ref().unwrap()).unwrap().txid();
    // the setup made it into a block; the sale did not
    w.txs.insert(setup_txid, BtcTx { height: None, hash: None, view: None });
    w.mine_btc(1);
    for (_, o, v) in float_inputs(&c.offer.sale_psbt, &c.float) {
        w.outputs.insert(o, (Output { script: c.float.clone(), value: v, confirmed: true }, false));
    }
    w.advance(frogflow::PROTECTED_STALE_SECS);
    let lines = tick(&mut w, &mut st);
    let FPhase::Abandoning { conflict, .. } = phase(&st, 0) else { panic!("{:?} {lines:?}", phase(&st, 0)) };
    assert_eq!((conflict.fee_input, conflict.leaked_fee, conflict.earlier.clone()), (Outpoint { txid: setup_txid, vout: 0 }, Some(0), None),
        "the setup's output 0 (100,000 sats, the sale's largest float input); the sale never seen: nothing to outbid (P-02)");
}

#[test]
fn a_setup_confirming_under_the_conflict_of_its_input_moves_the_conflict_to_the_sales_input() {
    let (mut w, mut st, c) = ow_world(true);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let setup_txid = Psbt::parse(c.offer.setup_psbt.as_ref().unwrap()).unwrap().txid();
    let setup_in = Outpoint { txid: [0x99; 32], vout: 0 };
    w.outputs.insert(setup_in, (Output { script: c.float.clone(), value: 200_000, confirmed: true }, false));
    w.advance(frogflow::PROTECTED_STALE_SECS);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Abandoning { .. }));
    // Ordinals Wallet's package lands the setup first after all: its input is spent, in a block, by the setup
    w.outspends.insert(setup_in, (setup_txid, true));
    w.txs.insert(setup_txid, BtcTx { height: None, hash: None, view: None });
    w.mine_btc(1);
    w.txs.retain(|t, _| *t == setup_txid || !w.btc_sent.iter().any(|r| display(parse_signed(r).unwrap().txid) == *t));
    let lines = tick(&mut w, &mut st);
    let FPhase::Abandoning { conflict, .. } = phase(&st, 0) else { panic!("{:?} {lines:?}", phase(&st, 0)) };
    assert_eq!((conflict.fee_input, conflict.leaked_fee), (Outpoint { txid: setup_txid, vout: 0 }, Some(0)), "the sale never seen (P-02): {lines:?}");
    assert!(w.book.contains("frog:0"), "not failed: the sale could still confirm");
}

#[test]
fn every_refused_protected_offer_leaves_the_frog_wanted_drops_the_offer_and_signs_nothing() {
    type Spoil = fn(&mut World, &mut ProtectedOffer);
    let cases: &[(&str, Spoil)] = &[
        ("the seller is paid 50000 sats, not the 49999 listed", |_, o| o.listing.escrow_price = Some(49_999)),
        ("does not verify against the listing's root", |_, o| o.proof = vec![[0x77; 32]]),
        ("the listing is gone", |w, o| { w.outputs.get_mut(&o.listing.outpoint).unwrap().1 = true; }),
        ("runes", |w, o| { w.contents.get_mut(&o.listing.outpoint).unwrap().runes = vec!["X".into()]; }),
        ("carries an inscription", |w, _| { w.dirty.insert(Outpoint { txid: [0x11; 32], vout: 1 }); }),
        ("sats would leave the float, over the", |w, _| w.rate = Some(100_000)),
        ("the parent does not parse", |_, o| o.parent = vec![1, 2, 3]),
        ("but ord locates the inscription in", |_, o| o.listing.outpoint.vout = 1),
    ];
    for (why, spoil) in cases {
        let (mut w, mut st, c) = ow_world(false);
        let mut o = c.offer.clone();
        spoil(&mut w, &mut o);
        w.protected.insert(0, o);
        let lines = tick(&mut w, &mut st);
        assert!(lines.iter().any(|l| l.contains("protected offer refused, nothing signed") && l.contains(why)), "{why}: {lines:?}");
        assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }), "{why}");
        assert!(w.protected.is_empty(), "{why}: the offer is dropped");
        assert!(w.digests.is_empty() && w.btc_sent.is_empty() && !w.book.contains("frog:0"), "{why}");
        assert!(st.protected.as_ref().is_none_or(|p| p.is_empty()) && st.protected_signed(0).is_none(), "{why}");
        // and the next tick does not see it again
        let lines = tick(&mut w, &mut st);
        assert!(!lines.iter().any(|l| l.contains("protected offer")), "{why}: {lines:?}");
    }
}

#[test]
fn a_protected_sale_that_leaves_the_mempools_is_handed_again_on_its_first_clock_and_nothing_is_sent() {
    let (mut w, mut st, c) = ow_world(false);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let FPhase::Handed { buy, .. } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    w.txs.insert(buy.outpoint.txid, BtcTx { height: None, hash: None, view: None });
    w.advance(3_600);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Sent { .. }));
    // evicted: no bytes to send again, so back to watching, with the hand-back's own clock
    w.txs.remove(&buy.outpoint.txid);
    let lines = tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Handed { at: T0, .. }), "{:?} {lines:?}", phase(&st, 0));
    assert!(w.btc_sent.is_empty(), "nothing sent: the canister holds no final bytes");
    assert!(st.protected_signed(0).is_some(), "the runner may submit the same signed PSBTs again");
}

/// B-L-6: a Bitcoin conversion's advance waits while the Ethereum side holds the one Relay leg (its refund watch would read the
/// advance landing at the shared 4663 address as its own refund), and goes once the leg is free.
#[test]
fn b_l_6_a_conversion_advance_waits_while_the_ethereum_side_holds_the_relay_leg() {
    let mut w = World::new();
    w.funding = Some(Default::default());
    let (mut st, mut fl) = floor_state();
    floor_funded(&mut w, E + E / 5);
    w.eth_busy = true;
    tickf(&mut w, &mut st, &mut fl);
    assert!(st.pending.is_none(), "no advance while the Ethereum side holds the leg: {:?}", st.pending.as_ref().map(|p| &p.step));
    assert!(matches!(conv(&st).phase, CPhase::NeedAdvance));
    assert!(conv(&st).note.contains("B-L-6"), "{}", conv(&st).note);
    assert!(etch_autobuyer::convflow::wants_relay(&st) && !etch_autobuyer::convflow::holds_relay(&st));
    w.eth_busy = false;
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(st.pending.as_ref().map(|p| p.step.clone()), Some(FStep::Advance { frog: 0 }), "the leg is free: the advance goes");
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert!(etch_autobuyer::convflow::holds_relay(&st), "from the advance on, the conversion holds the leg: {:?}", conv(&st).phase);
}

// ---- conversion review C-01..C-08 (docs/audit/2026-10-09-conv-review-fixes.md) ------------------------------------------------------

use etch_autobuyer::convflow;

/// A funded conversion that has arrived (1,580,000 sats, three deep), frog 0 wanted with them.
fn arrived_conversion(w: &mut World) -> (FrogState, FloorState, u128) {
    let (mut st, mut fl, wei) = funded_conversion(w);
    w.quotes.insert(0, (quote_for(w, wei, 1_560_000, 1), w.now));
    tickf(w, &mut st, &mut fl);
    w.mine_evm();
    tickf(w, &mut st, &mut fl);
    relay_fills(w, 0x71, 1_580_000, 3);
    tickf(w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Arrived { .. }), "{:?}", conv(&st).phase);
    (st, fl, wei)
}

/// A conversion whose deposit nobody filled or refunded: `Stuck`, still watched.
fn stuck_conversion(w: &mut World) -> (FrogState, FloorState, u128) {
    let (mut st, mut fl, wei) = funded_conversion(w);
    w.quotes.insert(0, (quote_for(w, wei, 1_560_000, 1), w.now));
    tickf(w, &mut st, &mut fl);
    w.mine_evm();
    tickf(w, &mut st, &mut fl);
    w.advance(7 * 86_400 + rc::DEADLINE_GRACE_SECS + 1);
    tickf(w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Stuck { .. }) && conv(&st).last.is_some());
    (st, fl, wei)
}

#[test]
fn c01_an_arrived_conversion_whose_buy_vanishes_is_wanted_again_and_its_sats_fund_the_next_offer() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, _) = arrived_conversion(&mut w);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    w.txs.remove(&buy.outpoint.txid);
    w.outspends.insert(seller, ([0x99; 32], true));
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { cap_wei } if cap_wei == crate_max()), "wanted again, for the advance: {:?}", phase(&st, 0));
    assert!(matches!(conv(&st).phase, CPhase::Arrived { sats: 1_580_000, .. }), "the conversion stays arrived");
    assert!(w.book.contains("frog:0"), "the room reserved at the advance stays reserved (H2)");
    assert_eq!(w.transit, E, "the module's unit is still in transit, now with a frog to come");
    // the next offer is bought with the same sats
    let seller2 = Outpoint { txid: [0xe2; 32], vout: 0 };
    w.list(&insc(2), seller2);
    w.offers.insert(0, w.offer_for(0, &insc(2), seller2, PRICE, proofs[1].clone()));
    tickf(&mut w, &mut st, &mut fl);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    assert_eq!(buy.cost_wei, rc::cost_of(crate_max(), buy.spend_sats, 1_580_000).unwrap(), "its share of the advance, as before");
    assert_eq!(st.frogs.len(), 1, "no second purchase meanwhile");
}

#[test]
fn c01_an_arrival_with_nothing_buyable_is_given_up_by_the_operator_after_a_week_and_accounted() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, _) = arrived_conversion(&mut w);
    let bal = w.balances[&ME];
    assert!(convflow::give_back_now(&mut st.clone(), 0, bal, w.now).is_err(), "relay_give_back is not the exit for an arrival");
    let e = convflow::arrived_exit_now(&mut st.clone(), 0, bal, w.now).unwrap_err();
    assert!(e.contains("may be given up from"), "{e}");
    w.advance(rc::ARRIVED_EXIT_SECS);
    let l = convflow::arrived_exit_now(&mut st, 0, bal, w.now).unwrap();
    assert!(l.contains("GIVEN UP") && l.contains("1580000 sats stay"), "{l}");
    assert_eq!(conv(&st).abandoned_sats, Some(1_580_000), "the sats left in the float are recorded");
    let CPhase::Returning { wei } = conv(&st).phase else { panic!("{:?}", conv(&st).phase) };
    assert_eq!(wei, bal - rc::GAS_KEEP_WEI, "what is at our address less the gas keep, at most the advance");
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::GiveBack { frog: 0 }), "{:?}", st.pending);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(w.transit, 0, "the module's unit is ETH-backed again");
    assert!(matches!(st.conv(0).unwrap().phase, CPhase::Returned { .. }));
    assert!(matches!(phase(&st, 0), FPhase::Failed { .. }));
    assert!(!w.book.contains("frog:0"), "its room is free");
    assert!(!convflow::holds_relay(&st), "the leg is free");
}

#[test]
fn c01_the_exit_is_refused_while_the_arrivals_frog_has_a_buy_out() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, _) = arrived_conversion(&mut w);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Signed { .. }));
    w.advance(rc::ARRIVED_EXIT_SECS + 1);
    assert!(convflow::arrived_exit_now(&mut st, 0, w.balances[&ME], w.now).unwrap_err().contains("buy out"));
}

#[test]
fn c01_an_advanced_frog_whose_mint_reverts_stays_its_modules_and_is_never_inventory() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, _) = arrived_conversion(&mut w);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    w.mine_btc(1);
    tickf(&mut w, &mut st, &mut fl);
    w.mine_btc(5);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 0 }), "{:?} {:?}", st.pending, phase(&st, 0));
    // the wrapper refuses the mint when it lands
    w.held.insert(frogbuy::leaf_of(&insc(1)));
    w.mine_evm();
    w.held.clear();
    tickf(&mut w, &mut st, &mut fl);
    let f = st.frogs.iter().find(|f| f.id == 0).unwrap();
    assert_eq!(f.target, Some(Target::Sink(FLOOR)), "the advanced frog stays its module's: {:?}", f);
    assert!(st.frogs.iter().all(|f| f.target.is_some()), "nothing advanced is inventory");
    // and it is minted to the module again
    for _ in 0..2 {
        if st.pending.is_none() { tickf(&mut w, &mut st, &mut fl); }
    }
    assert!(matches!(st.pending, Some(ref p) if p.step == FStep::Mint { frog: 0 }), "{:?}", st.pending);
    let p = st.pending.clone().unwrap();
    assert_eq!(&rlp_items(&p.raw)[7][..4], &deposits::WRAP_DEPOSIT);
}

#[test]
fn c02_a_stuck_conversion_still_watched_holds_the_relay_leg() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (st, _, _) = stuck_conversion(&mut w);
    assert!(convflow::holds_relay(&st), "Stuck and still watched against the shared balance: the leg is held");
    assert!(convflow::watching(&st));
}

#[test]
fn c03_give_back_from_stuck_keeps_the_live_order_watched_and_its_late_refund_goes_to_the_module() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = stuck_conversion(&mut w);
    *w.balances.get_mut(&ME).unwrap() += E / 100; // some unrelated ETH at the address
    let bal = w.balances[&ME];
    let l = convflow::give_back_now(&mut st, 0, bal, w.now).unwrap();
    assert!(l.contains("written off and still watched"), "{l}");
    assert!(conv(&st).late_open() && convflow::holds_relay(&st), "the order is kept and holds the leg");
    tickf(&mut w, &mut st, &mut fl);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.conv(0).unwrap().phase, CPhase::Returned { .. }));
    assert!(convflow::holds_relay(&st), "still held after the return: its refund may land");
    // Relay refunds the order late: it is attributed to this conversion, and paid into the module
    *w.balances.get_mut(&ME).unwrap() += wei;
    tickf(&mut w, &mut st, &mut fl);
    let refund = st.conv(0).unwrap().late.clone().unwrap().refund_wei.expect("the late refund is seen");
    assert_eq!(refund, wei.min(w.balances[&ME] - rc::GAS_KEEP_WEI));
    if st.pending.is_none() { tickf(&mut w, &mut st, &mut fl); }
    let p = st.pending.clone().expect("fundReserve");
    assert_eq!(p.step, FStep::LateCredit { frog: 0 });
    assert_eq!((&rlp_items(&p.raw)[7][..], uint(&rlp_items(&p.raw)[6])), (&etch_autobuyer::ethflow::FUND_RESERVE[..], refund));
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(w.reserve, refund, "the module has its refund");
    assert_eq!(st.conv(0).unwrap().late.clone().unwrap().credited, Some(refund));
    assert!(!convflow::holds_relay(&st), "credited: the leg is free");
}

#[test]
fn c03_a_written_off_order_is_released_only_by_the_operator_and_a_late_fill_is_recorded() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, _) = stuck_conversion(&mut w);
    *w.balances.get_mut(&ME).unwrap() += E / 100;
    let bal = w.balances[&ME];
    convflow::give_back_now(&mut st, 0, bal, w.now).unwrap();
    tickf(&mut w, &mut st, &mut fl);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    let mut released = st.clone();
    assert!(convflow::late_release_now(&mut released, 0, w.now).unwrap().contains("released"));
    assert!(!convflow::holds_relay(&released));
    assert!(convflow::late_release_now(&mut released, 0, w.now).is_err(), "once");
    // the order fills late after all: the sats are recorded, the leg is free
    relay_fills(&mut w, 0x72, 1_570_000, 3);
    tickf(&mut w, &mut st, &mut fl);
    let c = st.conv(0).unwrap().clone();
    assert_eq!((c.abandoned_sats, c.late.as_ref().unwrap().filled.as_ref().map(|a| a.sats)), (Some(1_570_000), Some(1_570_000)));
    assert!(!convflow::holds_relay(&st));
}

#[test]
fn c04_a_sale_report_waits_while_the_relay_leg_is_held() {
    // the pure part: holds_relay is what saleflow::report waits on (the flow is exercised in tests/sales.rs)
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (st, _, _) = funded_conversion(&mut w);
    assert!(convflow::holds_relay(&st), "a funded conversion with no quote holds the leg: its wei is not a sale's");
}

#[test]
fn c05_past_the_deadline_a_split_fill_or_a_bitcoin_side_refund_under_the_minimum_is_the_arrival() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = funded_conversion(&mut w);
    w.quotes.insert(0, (quote_for(&w, wei, 1_560_000, 1), w.now));
    tickf(&mut w, &mut st, &mut fl);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    // two outputs, each under the minimum: before the deadline they are not the arrival
    relay_fills(&mut w, 0x73, 500_000, 3);
    relay_fills(&mut w, 0x74, 400_000, 3);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Converting { .. }), "{:?}", conv(&st).phase);
    w.advance(7 * 86_400 + 1);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Arrived { sats: 900_000, .. }), "summed: {:?}", conv(&st).phase);
    // and under half the minimum it is not
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = funded_conversion(&mut w);
    w.quotes.insert(0, (quote_for(&w, wei, 1_560_000, 1), w.now));
    tickf(&mut w, &mut st, &mut fl);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    relay_fills(&mut w, 0x75, 700_000, 3);
    w.advance(7 * 86_400 + rc::DEADLINE_GRACE_SECS + 1);
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(conv(&st).phase, CPhase::Stuck { .. }), "{:?}", conv(&st).phase);
}

#[test]
fn c06_no_reimbursement_is_triggered_while_a_deposit_is_watched_and_no_deposit_while_one_is_owed() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = funded_conversion(&mut w);
    let o = Outpoint { txid: [0x5a; 32], vout: 1 };
    let buy = frogbuy::Bought { inscription: insc(2), outpoint: o, seller: o, spend_sats: 1, fee_sats: 1, rate: RATE, cost_wei: E / 10,
        raw: vec![], setup_txid: None };
    // a frog of ours minted to its sink, its (reimbursing) report owed: the deposit waits
    st.frogs.push(frogbuy::FrogRec { id: 9, target: Some(Target::Sink(FLOOR)), phase: FPhase::Minted { buy: buy.clone() } });
    assert!(convflow::reimbursement_owed(&st, &fl).is_some());
    let mut parked = st.clone();
    parked.frogs[1].phase = FPhase::Settled { buy: buy.clone() };
    w.quotes.insert(0, (quote_for(&w, wei, 1_560_000, 1), w.now));
    // its report cannot go now (a sink this tick cannot read): the deposit is not sent meanwhile
    let mut st2 = st.clone();
    st2.frogs[1].target = Some(Target::Sink([0x77; 20]));
    let lines = tickf(&mut w, &mut st2, &mut fl);
    assert!(!matches!(st2.pending, Some(ref p) if matches!(p.step, FStep::Convert { .. })), "no deposit while a reimbursement is owed: {lines:?}");
    assert!(conv(&st2).note.contains("C-06"), "{}", conv(&st2).note);
    // once paid, the deposit goes; while it is watched, a reimbursing report waits
    w.quotes.insert(0, (quote_for(&w, wei, 1_560_000, 2), w.now));
    tickf(&mut w, &mut parked, &mut fl);
    assert!(matches!(parked.pending, Some(ref p) if p.step == FStep::Convert { frog: 0 }), "{:?}", parked.pending);
    w.mine_evm();
    tickf(&mut w, &mut parked, &mut fl);
    assert!(matches!(conv(&parked).phase, CPhase::Converting { .. }));
    parked.frogs[1].phase = FPhase::Minted { buy };
    let lines = tickf(&mut w, &mut parked, &mut fl);
    assert!(parked.pending.is_none(), "no report while the refund watch reads our balance: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("C-06")), "{lines:?}");
}

// the review's proofs of concept (/tmp/review-conv), asserting the fixed behaviour

#[test]
fn poc_c01_arrived_conversion_whose_buy_vanishes_no_longer_wedges() {
    let mut w = World::new();
    let proofs = w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, _) = arrived_conversion(&mut w);
    let seller = Outpoint { txid: [0xe1; 32], vout: 0 };
    w.list(&insc(1), seller);
    w.offers.insert(0, w.offer_for(0, &insc(1), seller, PRICE, proofs[0].clone()));
    tickf(&mut w, &mut st, &mut fl);
    let FPhase::Signed { buy } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    w.txs.remove(&buy.outpoint.txid);
    w.outspends.insert(seller, ([0x99; 32], true));
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }), "not Failed: wanted again with the arrived sats: {:?}", phase(&st, 0));
    for _ in 0..7 { floor_funded(&mut w, 3 * E); w.advance(86_400); tickf(&mut w, &mut st, &mut fl); }
    assert!(matches!(conv(&st).phase, CPhase::Arrived { .. }) && matches!(phase(&st, 0), FPhase::Wanted { .. }), "still buyable");
    assert!(w.book.contains("frog:0"), "H2 stays reserved while the advance is outstanding");
    // the operator's exit exists once a week has passed with no buy
    let bal = w.balances[&ME];
    assert!(convflow::arrived_exit_now(&mut st, 0, bal, w.now).is_ok());
    tickf(&mut w, &mut st, &mut fl);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert_eq!(w.transit, 0, "the module's unit is no longer in transit");
}

#[test]
fn poc_c02_stuck_conversion_holds_the_leg_so_the_ethereum_side_starts_nothing() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (st, _, _) = stuck_conversion(&mut w);
    assert!(convflow::holds_relay(&st), "Stuck and still watched: the Ethereum side (frog_relay_busy = holds_relay) starts no buy");
}

#[test]
fn poc_c03_give_back_from_stuck_keeps_the_live_order_and_attributes_its_late_refund() {
    let mut w = World::new();
    w.collection(&[insc(1), insc(2)]);
    let (mut st, mut fl, wei) = stuck_conversion(&mut w);
    *w.balances.get_mut(&ME).unwrap() += E / 100;
    let bal = w.balances[&ME];
    convflow::give_back_now(&mut st, 0, bal, w.now).unwrap();
    assert!(conv(&st).late.as_ref().is_some_and(|l| l.open()), "the watch on the live order is kept (as written off)");
    tickf(&mut w, &mut st, &mut fl);
    w.mine_evm();
    tickf(&mut w, &mut st, &mut fl);
    assert!(matches!(st.conv(0).unwrap().phase, CPhase::Returned { .. }));
    *w.balances.get_mut(&ME).unwrap() += wei;
    tickf(&mut w, &mut st, &mut fl);
    assert!(st.conv(0).unwrap().late.as_ref().unwrap().refund_wei.is_some(), "the late refund is attributed to this conversion");
}

// ---- protected-listing review P-01..P-09 (docs/audit/2026-10-09-conv-review-fixes.md) ----------------------------------------------

#[test]
fn p01_a_protected_sale_paying_a_royalty_is_refused_unless_a_controller_pinned_its_creator() {
    let (mut w, mut st, c) = ow_world(false);
    st.ow_creators = None;
    w.protected.insert(0, c.offer.clone());
    let lines = tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { .. }), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("unexpected party")), "the listing's creator_address is not believed: {lines:?}");
    assert!(st.set_ow_creators(vec!["not an address".into()]).is_err());
    assert!(st.set_ow_creators(vec![c.offer.listing.creator_address.clone().unwrap(); 9]).is_err(), "at most eight");
}

#[test]
fn poc_handed_sale_whose_escrow_was_taken_by_a_competitor_fails_without_a_conflict() {
    let (mut w, mut st, c) = ow_world(false);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let FPhase::Handed { .. } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    for (_, o, v) in float_inputs(&c.offer.sale_psbt, &c.float) {
        w.outputs.insert(o, (Output { script: c.float.clone(), value: v, confirmed: true }, false));
    }
    // a competitor's sale through the same passthrough confirmed: the listing and the escrow are both spent, in blocks
    let rec = st.protected_rec(0).unwrap().clone();
    assert_eq!(rec.listing, Some(c.offer.listing.outpoint), "the listed outpoint is kept (P-03)");
    w.outspends.insert(c.offer.listing.outpoint, (rec.escrow.txid, true));
    w.outspends.insert(rec.escrow, ([0xc0; 32], true));
    w.outputs.get_mut(&c.offer.listing.outpoint).unwrap().1 = true;
    let lines = tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Failed { ref why } if why.contains("sold to someone else")), "{:?} {lines:?}", phase(&st, 0));
    assert!(w.btc_sent.is_empty(), "no conflict: nothing of ours was spent");
    assert!(!w.book.contains("frog:0"), "its room is free");
    // and the same at the stale clock: no conflict either
    let (mut w, mut st, c) = ow_world(false);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    w.outspends.insert(c.offer.listing.outpoint, ([0xc1; 32], true)); // the seller moved the item itself
    w.advance(frogflow::PROTECTED_STALE_SECS);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Failed { ref why } if why.contains("not the passthrough")), "{:?}", phase(&st, 0));
    assert!(w.btc_sent.is_empty());
}

#[test]
fn p03_with_c01_a_vanished_protected_listing_of_an_arrived_conversion_puts_the_frog_back_to_wanted() {
    let (mut w, mut st, c) = ow_world(false);
    // the frog's conversion has arrived (as a Relay-funded frog's would): the buy spends what arrived
    let mut cv = rc::ConvRec::new(0, QUEUE);
    cv.advanced = 2 * E / 5;
    cv.phase = CPhase::Arrived { order_id: [7; 32], sats: 60_000 };
    st.convs = Some(vec![cv]);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let FPhase::Handed { .. } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    let rec = st.protected_rec(0).unwrap().clone();
    w.outspends.insert(rec.escrow, ([0xc0; 32], true));
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Wanted { cap_wei } if cap_wei == 2 * E / 5), "{:?}", phase(&st, 0));
    assert!(matches!(conv(&st).phase, CPhase::Arrived { sats: 60_000, .. }));
    assert!(w.btc_sent.is_empty());
}

#[test]
fn p02_a_protected_sale_once_seen_and_evicted_is_conflicted_over_its_fee_and_counted_once() {
    let (mut w, mut st, c) = ow_world(false);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let FPhase::Handed { buy, .. } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    for (_, o, v) in float_inputs(&c.offer.sale_psbt, &c.float) {
        w.outputs.insert(o, (Output { script: c.float.clone(), value: v, confirmed: true }, false));
    }
    // Ordinals Wallet relays it; then mempools drop it
    w.txs.insert(buy.outpoint.txid, BtcTx { height: None, hash: None, view: None });
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Sent { .. }) && st.protected_rec(0).unwrap().seen == Some(true));
    assert_eq!(w.recorded.iter().filter(|k| *k == "buy:frog:0").count(), 1, "counted once, when seen");
    w.txs.remove(&buy.outpoint.txid);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Handed { .. }), "{:?}", phase(&st, 0));
    w.advance(frogflow::PROTECTED_STALE_SECS);
    tick(&mut w, &mut st);
    let FPhase::Abandoning { conflict, .. } = phase(&st, 0) else { panic!("{:?}", phase(&st, 0)) };
    assert_eq!(conflict.leaked_fee, Some(1_000), "seen once: a mempool may still hold it, so its fee is outbid");
    assert_eq!(w.recorded.iter().filter(|k| *k == "buy:frog:0").count(), 1, "still once");
}

#[test]
fn poc_a_protected_buy_back_in_sent_after_held_keeps_its_record_and_can_be_abandoned() {
    let (mut w, mut st, c) = ow_world(false);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let FPhase::Handed { buy, .. } = phase(&st, 0) else { panic!() };
    // held: the record is kept (the sweep no longer drops it)
    st.frogs.iter_mut().find(|f| f.id == 0).unwrap().phase = FPhase::Held { buy: buy.clone() };
    tick(&mut w, &mut st);
    assert!(st.protected_rec(0).is_some(), "kept while held");
    // a reorganisation puts it back to Sent, and Bitcoin no longer shows it: it is watched as handed, never as a classic buy
    st.frogs.iter_mut().find(|f| f.id == 0).unwrap().phase = FPhase::Sent { buy: buy.clone() };
    let lines = tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Handed { .. }), "{:?} {lines:?}", phase(&st, 0));
    for (_, o, v) in float_inputs(&c.offer.sale_psbt, &c.float) {
        w.outputs.insert(o, (Output { script: c.float.clone(), value: v, confirmed: true }, false));
    }
    w.advance(frogflow::PROTECTED_STALE_SECS);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Abandoning { .. }), "{:?}", phase(&st, 0));
    // a record an older build dropped: recognised by its empty bytes; stale, it is stranded loudly rather than stuck
    let (mut w, mut st, c) = ow_world(false);
    w.protected.insert(0, c.offer.clone());
    tick(&mut w, &mut st);
    let FPhase::Handed { buy, .. } = phase(&st, 0) else { panic!() };
    st.protected = None;
    st.frogs.iter_mut().find(|f| f.id == 0).unwrap().phase = FPhase::Sent { buy };
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Handed { .. }), "not a classic Signed with no bytes: {:?}", phase(&st, 0));
    w.advance(frogflow::PROTECTED_STALE_SECS + 10);
    tick(&mut w, &mut st);
    assert!(matches!(phase(&st, 0), FPhase::Stranded { ref why, .. } if why.contains("record is gone")), "{:?}", phase(&st, 0));
}
