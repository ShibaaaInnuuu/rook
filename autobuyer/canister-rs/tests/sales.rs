//! Protocol sales and relist tickets (docs/v6/canister-floor-followups.md) against a fake world: a Floor Module as merged
//! (ten reads, `itemSold` that checks what the real one checks), a wrapper with tickets (`redeemToRelist`, `recordDelivery`,
//! `wrapDeposit`), a YOYO vault (`listing`, `recordSale`, `reclaimListing`), a Bitcoin that confirms fills and reorganises,
//! ord and explorers, a limiter, and an operator who bridges the sats to 4663 and sometimes delists.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::btcnet::BtcSeen;
use etch_autobuyer::btcsend::parse_signed;
use etch_autobuyer::custody::{Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::deposits::{self, InView, OutView, TxView};
use etch_autobuyer::evmtx;
use etch_autobuyer::floorsink;
use etch_autobuyer::floortrigger::{FloorItem, FloorState};
use etch_autobuyer::frogbuy;
use etch_autobuyer::frogflow::{self, FStep, FrogIo, FrogState, Offer};
use etch_autobuyer::handin::HandinState;
use etch_autobuyer::itemsink::{Clock, SinkKind, SinkRec};
use etch_autobuyer::looper::EvmView;
use etch_autobuyer::psbt::Psbt;
use etch_autobuyer::sale::{self, SPhase, SalePolicy, SaleState};
use etch_autobuyer::{curvebuy, custodycap, outbound, outflow, redeem};

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
const PVAULT: [u8; 20] = [0xac; 20];
const FLOOR: [u8; 20] = [0x56; 20];
const YOYO: [u8; 20] = [0x59; 20];
const COIN: [u8; 20] = [0x53; 20];
const RATE: u64 = 3_190_000;
const CHAIN: u64 = 4663;
/// The module: a 0.64 ETH floor, a 0.65 fills price.
const PRICE: u128 = 650 * E / 1_000;
const ITEM_TXID: [u8; 32] = [0xf0; 32];

fn p2tr(tag: u8) -> Vec<u8> {
    let mut s = vec![0x51, 0x20];
    s.extend([tag; 32]);
    s
}
fn custody() -> Vec<u8> {
    p2tr(0xc2)
}
fn custody_address() -> String {
    etch_autobuyer::policy::taproot_address(&[0xc2; 32], "bc")
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
fn string_tail(s: &str) -> Vec<u8> {
    let mut b = word(s.len() as u128);
    b.extend_from_slice(s.as_bytes());
    b.resize(b.len().div_ceil(32) * 32, 0);
    b
}
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
fn display(internal: [u8; 32]) -> [u8; 32] {
    let mut d = internal;
    d.reverse();
    d
}

// ---- the world ---------------------------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Tk {
    stage: u8,
    owner: [u8; 20],
    acq: u128,
    dest: String,
}

#[derive(Clone)]
struct FloorC {
    net: u128,
    escrow: u128,
    posted_at: u64,
    max_cost: u128,
    arrived: u64,
    float_units: u128,
    reserve: u128,
}

struct BtcTx {
    height: Option<u64>,
    hash: Option<String>,
    view: Option<TxView>,
}

struct World {
    now: u64,
    clock: Clock,
    root: [u8; 32],
    minted: HashSet<[u8; 32]>,
    held: HashSet<[u8; 32]>,
    /// wX balances, wei units.
    balances: HashMap<[u8; 20], u128>,
    /// Our 4663 ETH.
    eth: u128,
    tickets: Vec<Tk>,
    /// The purchase vault's acquisitions.
    insc: HashMap<u128, String>,
    floor: FloorC,
    item_sold: Vec<u128>,
    /// The YOYO vault's listings by ticket: (stage, ask), and what it recorded.
    listings: HashMap<u64, (u8, u128)>,
    vault_units: u128,
    sales_recorded: Vec<(u64, u128)>,
    reclaimed: Vec<u64>,
    nonce: u64,
    mempool: Vec<Vec<u8>>,
    receipts: HashMap<[u8; 32], bool>,
    calls: Vec<([u8; 20], Vec<u8>, u128, bool)>,
    tip: u64,
    blocks: HashMap<u64, String>,
    txs: HashMap<[u8; 32], BtcTx>,
    loc: HashMap<String, Location>,
    contents: HashMap<Outpoint, Contents>,
    outputs: HashMap<Outpoint, (Output, bool)>,
    outspends: HashMap<Outpoint, ([u8; 32], bool)>,
    utxos: Vec<Utxo>,
    btc_sent: Vec<Vec<u8>>,
    custody_signs: usize,
    saved: Option<FrogState>,
    saved_sales: Option<SaleState>,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
    limits: Option<outflow::Limits>,
    outflow: outflow::Outflow,
    asked: Vec<(String, outflow::Flow)>,
    recorded: Vec<String>,
    paused: bool,
}

fn frogs_key() -> String {
    custodycap::btc_key(&W)
}

fn block_hash_for(h: u64) -> String {
    hex::encode(evmtx::keccak(&h.to_be_bytes()))
}

impl World {
    /// The module holds one converted float unit and a 0.5 ETH reserve; the canister holds one seed unit; the wrapper holds two
    /// items (acquisitions 7 and 8, inscriptions 0x07…i0 and 0x08…i0, in custody), the first of which drand draws.
    fn new() -> Self {
        let mut w = World {
            now: T0,
            clock: Clock { time: T0, parent_block: B0 },
            root: [0; 32],
            minted: HashSet::new(),
            held: HashSet::new(),
            balances: HashMap::from([(ME, E), (FLOOR, E)]),
            eth: 10_000_000_000_000_000, // 0.01 ETH: gas, not a report's worth
            tickets: vec![],
            insc: HashMap::from([(7, insc(7)), (8, insc(8))]),
            floor: FloorC { net: E / 2, escrow: E / 2, posted_at: T0, max_cost: 660 * E / 1_000, arrived: 1, float_units: E, reserve: E / 2 },
            item_sold: vec![],
            listings: HashMap::new(),
            vault_units: 0,
            sales_recorded: vec![],
            reclaimed: vec![],
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
            custody_signs: 0,
            saved: None,
            saved_sales: None,
            caps: custodycap::CapPolicy { collections: vec![custodycap::CollectionCap { collection: frogs_key(), max_items: 1_000,
                max_value_wei: 1_000_000 * E, max_item_wei: 1_000 * E }], global_max_items: 1_000, global_max_value_wei: 1_000_000 * E },
            book: custodycap::Book::default(),
            limits: None,
            outflow: outflow::Outflow::default(),
            asked: vec![],
            recorded: vec![],
            paused: false,
        };
        w.blocks.insert(w.tip, block_hash_for(w.tip));
        for (tag, id) in [(0xf0u8, insc(7)), (0xf8, insc(8))] {
            let o = Outpoint { txid: [tag; 32], vout: 1 };
            w.loc.insert(id.clone(), Location { output: o, offset: 0, value: 10_000 });
            w.contents.insert(o, Contents { inscriptions: vec![id.clone()], runes: vec![], value: 10_000 });
            w.outputs.insert(o, (Output { script: custody(), value: 10_000, confirmed: true }, false));
            w.utxos.push(Utxo { outpoint: o, value: 10_000, confirmed: true });
            w.txs.insert([tag; 32], BtcTx { height: Some(w.tip), hash: Some(w.blocks[&w.tip].clone()), view: None });
            w.held.insert(frogbuy::leaf_of(&id));
            let caps = w.caps.clone();
            custodycap::admit(&caps, &mut w.book, &id, &frogs_key(), 640 * E / 1_000).unwrap();
        }
        // custody's plain float for a move's fee
        let fee = Outpoint { txid: [0xfe; 32], vout: 0 };
        w.utxos.push(Utxo { outpoint: fee, value: 20_000, confirmed: true });
        w.contents.insert(fee, Contents { inscriptions: vec![], runes: vec![], value: 20_000 });
        w.outputs.insert(fee, (Output { script: custody(), value: 20_000, confirmed: true }, false));
        w.txs.insert([0xfe; 32], BtcTx { height: Some(w.tip), hash: Some(w.blocks[&w.tip].clone()), view: None });
        // the collection: the two inscriptions' leaves, so a re-deposit has a proof
        let l: Vec<[u8; 32]> = [insc(7), insc(8)].iter().map(|i| frogbuy::leaf_of(i)).collect();
        let (x, y) = if l[0] <= l[1] { (l[0], l[1]) } else { (l[1], l[0]) };
        w.root = evmtx::keccak(&[x, y].concat());
        w
    }
    fn proofs(&self) -> Vec<(String, Vec<[u8; 32]>)> {
        vec![(insc(7), vec![frogbuy::leaf_of(&insc(8))]), (insc(8), vec![frogbuy::leaf_of(&insc(7))])]
    }
    fn advance(&mut self, secs: u64) {
        self.now += secs;
        self.clock.time += secs;
        self.clock.parent_block += secs / 12;
    }
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
    /// A confirmed move of ours: the item is at output 0 of it now; the fee input is spent.
    fn settle_btc(&mut self, txid: [u8; 32]) {
        let Some(raw) = self.btc_sent.iter().find(|r| display(parse_signed(r).unwrap().txid) == txid).cloned() else { return };
        let tx = parse_signed(&raw).unwrap();
        for i in &tx.inputs {
            let spent = Outpoint { txid: display(i.txid), vout: i.vout };
            self.utxos.retain(|u| u.outpoint != spent);
            if let Some(o) = self.outputs.get_mut(&spent) {
                o.1 = true;
            }
            self.outspends.insert(spent, (txid, true));
            if let Some((id, _)) = self.loc.iter().find(|(_, l)| l.output == spent).map(|(a, b)| (a.clone(), b.clone())) {
                let at = Outpoint { txid, vout: 0 };
                self.loc.insert(id.clone(), Location { output: at, offset: 0, value: 10_000 });
                self.contents.insert(at, Contents { inscriptions: vec![id], runes: vec![], value: 10_000 });
                self.outputs.insert(at, (Output { script: custody(), value: 10_000, confirmed: true }, false));
                self.utxos.push(Utxo { outpoint: at, value: 10_000, confirmed: true });
            }
        }
    }
    /// A buyer fills the listing of `item`: a transaction spending a UTXO of theirs and the item, paying custody `pays` sats at
    /// the item's index, mined now.
    fn fill(&mut self, item: Outpoint, pays: u64, tag: u8) -> [u8; 32] {
        let txid = [tag; 32];
        let inp = |o: Outpoint| InView { prev: o, prev_script: vec![], script_sig: vec![], witness: vec![] };
        let view = TxView { txid, inputs: vec![inp(Outpoint { txid: [0xb1; 32], vout: 0 }), inp(item)],
            outputs: vec![OutView { value: 546, script: p2tr(0xb0) }, OutView { value: pays, script: custody() }], confirmed: true, height: None, block_hash: None };
        self.txs.insert(txid, BtcTx { height: None, hash: None, view: Some(view) });
        self.mine_btc(1);
        self.outspends.insert(item, (txid, true));
        if let Some(o) = self.outputs.get_mut(&item) {
            o.1 = true;
        }
        self.utxos.retain(|u| u.outpoint != item);
        txid
    }
    /// drand draws `acq` for ticket `id`.
    fn resolve(&mut self, id: u64, acq: u128) {
        let t = &mut self.tickets[id as usize];
        t.stage = 2;
        t.acq = acq;
    }
    fn ticket_ret(&self, id: u64) -> Vec<u8> {
        let t = &self.tickets[id as usize];
        let mut b = word(32);
        for w in [word(t.stage as u128), aword(&t.owner), word(1), word(T0 as u128), word(B0 as u128), word((T0 + 7 * 86_400) as u128),
            word((B0 + 7 * 7_200) as u128), word(t.acq), vec![0u8; 32], word(10 * 32)] {
            b.extend(w);
        }
        b.extend(string_tail(&t.dest));
        b
    }
    fn floor_fresh(&self) -> bool {
        self.clock.time < self.floor.posted_at + 3_600
    }
    /// One call as the contracts would make it, applied or not.
    fn exec(&mut self, to: [u8; 20], data: &[u8], value: u128, apply: bool) -> Result<(), String> {
        let sel: [u8; 4] = data[..4].try_into().unwrap();
        match (to, sel) {
            (W, sale::REDEEM_TO_RELIST) => {
                if self.balances.get(&ME).copied().unwrap_or(0) < E { return Err("burn: insufficient balance".into()); }
                let open = self.tickets.iter().filter(|t| t.stage == 1 || t.stage == 2).count();
                if self.held.len() < open + 1 { return Err("NoItem".into()); }
                if apply {
                    *self.balances.get_mut(&ME).unwrap() -= E;
                    self.tickets.push(Tk { stage: 1, owner: ME, acq: 0, dest: custody_address() });
                }
                Ok(())
            }
            (W, redeem::RECORD_DELIVERY) => {
                let id = arg(data, 0) as usize;
                let t = self.tickets.get(id).cloned().ok_or("no ticket")?;
                if t.stage != 2 { return Err("WrongStage".into()); }
                if arg_word(data, 1) == [0; 32] { return Err("BadInput".into()); }
                if apply {
                    self.tickets[id].stage = 3;
                    let leaf = frogbuy::leaf_of(&self.insc[&t.acq]);
                    self.held.remove(&leaf);
                }
                Ok(())
            }
            (W, deposits::WRAP_DEPOSIT) => {
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
                    if dest == YOYO { self.vault_units += E; }
                }
                Ok(())
            }
            (FLOOR, sale::ITEM_SOLD) => {
                let proceeds = arg(data, 0);
                if proceeds == 0 || value != proceeds { return Err("BadInput".into()); }
                if !self.floor_fresh() { return Err("Stale".into()); }
                let min = 640 * E / 1_000 * 97 / 100;
                if proceeds < min { return Err(format!("UnderSalePrice({proceeds}, {min})")); }
                if self.floor.float_units < E { return Err("NoInventory".into()); }
                if self.eth < value { return Err("insufficient funds".into()); }
                if apply {
                    self.floor.float_units -= E;
                    self.floor.reserve += proceeds;
                    self.eth -= value;
                    *self.balances.entry(FLOOR).or_default() -= E;
                    *self.balances.entry(ME).or_default() += E;
                    self.item_sold.push(proceeds);
                }
                Ok(())
            }
            (YOYO, sale::RECORD_SALE) => {
                let id = arg(data, 0) as u64;
                let (stage, ask) = self.listings.get(&id).copied().ok_or("WrongStage")?;
                if stage != 1 { return Err("WrongStage".into()); }
                let min = ask * 9_500 / 10_000;
                if value < min { return Err(format!("UnderAsk({value}, {min})")); }
                if self.eth < value { return Err("insufficient funds".into()); }
                if apply {
                    self.listings.insert(id, (2, ask));
                    self.eth -= value;
                    self.sales_recorded.push((id, value));
                }
                Ok(())
            }
            (YOYO, sale::RECLAIM_LISTING) => {
                let id = arg(data, 0) as u64;
                let (stage, ask) = self.listings.get(&id).copied().ok_or("WrongStage")?;
                if stage != 1 { return Err("WrongStage".into()); }
                if self.balances.get(&YOYO).copied().unwrap_or(0) < self.vault_units { return Err("Insufficient".into()); }
                if apply {
                    self.listings.insert(id, (3, ask));
                    self.reclaimed.push(id);
                }
                Ok(())
            }
            _ => Err(format!("unknown call to 0x{}", hex::encode(to))),
        }
    }
    fn mine_evm(&mut self) {
        loop {
            let Some(pos) = self.mempool.iter().position(|r| uint(&rlp_items(r)[1]) as u64 == self.nonce) else { return };
            let raw = self.mempool.remove(pos);
            let f = rlp_items(&raw);
            let to: [u8; 20] = f[5].clone().try_into().unwrap();
            let value = uint(&f[6]);
            let ok = self.exec(to, &f[7], value, true).is_ok();
            self.calls.push((to, f[7].clone(), value, ok));
            self.receipts.insert(evmtx::keccak(&raw), ok);
            self.nonce += 1;
        }
    }
}

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for World {}

impl FrogIo for World {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { CHAIN }
    fn wrapper(&self) -> [u8; 20] { W }
    fn fresh(&mut self) {}
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        let sel: [u8; 4] = data[..4].try_into().unwrap();
        let b = |x: bool| word(x as u128);
        Ok(match (to, sel) {
            (W, deposits::CUSTODIAN) => aword(&ME),
            (W, deposits::REGISTRY) => aword(&REG),
            (W, deposits::LISTING_ID) => word(0),
            (W, deposits::DEPOSIT_MINTED) => b(self.minted.contains(&arg_word(data, 0))),
            (W, deposits::INSCRIPTION_HELD) => b(self.held.contains(&arg_word(data, 0))),
            (W, redeem::TICKET_COUNT) => word(self.tickets.len() as u128),
            (W, redeem::TICKET) => self.ticket_ret(arg(data, 0) as u64),
            (W, curvebuy::BALANCE_OF) => {
                let who: [u8; 20] = arg_word(data, 0)[12..].try_into().unwrap();
                word(self.balances.get(&who).copied().unwrap_or(0))
            }
            (REG, deposits::LISTING) => [word(32), word(1), word(0), word(0), word(0), aword(&[9; 20]), self.root.to_vec()].concat(),
            (REG, redeem::VAULT_OF) => aword(&PVAULT),
            (PVAULT, redeem::ACQUISITION) => {
                let r = self.insc.get(&arg(data, 0)).cloned().ok_or("no acquisition")?;
                [word(32), word(0), word(0), word(1), word(0), word(0), word(6 * 32), string_tail(&r)].concat()
            }
            (FLOOR, s) => {
                let f = self.floor.clone();
                match s {
                    floorsink::NET_UNITS => word(f.net),
                    floorsink::ESCROW => word(f.escrow),
                    floorsink::BUFFER => word(E / 5),
                    floorsink::FLOOR => [word(640 * E / 1_000), word(f.posted_at as u128), word(3_600)].concat(),
                    floorsink::MAX_ITEM_COST => word(f.max_cost),
                    floorsink::ITEMS_ARRIVED => word(f.arrived as u128),
                    floorsink::PRICE => word(PRICE),
                    floorsink::FLOAT_UNITS => word(f.float_units),
                    floorsink::MIN_SALE_PROCEEDS => word(640 * E / 1_000 * 97 / 100),
                    floorsink::ESCROW_BALANCE => word(f.escrow + f.reserve),
                    // docs/v6/relay-conversion.md: no advance is open in these tests
                    s if s == etch_autobuyer::relayconv::TRANSIT_UNITS => word(0),
                    floorsink::PAYEE => aword(&ME),
                    floorsink::WRAPPER => aword(&W),
                    _ => return Err("unknown floor read".into()),
                }
            }
            (YOYO, sale::VAULT_WRAPPER) => aword(&W),
            (YOYO, sale::LISTING) => {
                let (stage, ask) = self.listings.get(&(arg(data, 0) as u64)).copied().unwrap_or((0, 0));
                [word(stage as u128), aword(&COIN), word(ask), word(0)].concat()
            }
            _ => return Err(format!("unknown read 0x{} {}", hex::encode(to), hex::encode(sel))),
        })
    }
    async fn clock(&mut self) -> Result<Clock, String> { Ok(self.clock) }
    async fn our_evm(&mut self) -> Result<[u8; 20], String> { Ok(ME) }
    async fn evm_nonce(&mut self) -> Result<u64, String> { Ok(self.nonce) }
    async fn base_fee(&mut self) -> Result<u128, String> { Ok(1_000_000) }
    async fn estimate_gas(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<u64, String> {
        self.exec(to, data, value, false).map(|_| 100_000)
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
    async fn float_script(&mut self) -> Result<Vec<u8>, String> { Ok(p2tr(0xf1)) }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> { Ok(custody()) }
    async fn locate(&mut self, i: &str) -> Result<Option<Location>, String> { Ok(self.loc.get(i).cloned()) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> { Ok(self.contents.get(o).cloned()) }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
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
        let txid = display(parse_signed(raw).map_err(|e| e.to_string())?.txid);
        self.btc_sent.push(raw.to_vec());
        self.txs.entry(txid).or_insert(BtcTx { height: None, hash: None, view: None });
        Ok(())
    }
    async fn custody_utxos(&mut self) -> Result<Vec<Utxo>, String> { Ok(self.utxos.clone()) }
    async fn tx_view(&mut self, txid: [u8; 32]) -> Result<TxView, String> {
        let t = self.txs.get(&txid).ok_or("no such transaction")?;
        let mut v = t.view.clone().unwrap_or(TxView { txid, inputs: vec![], outputs: vec![], confirmed: false, height: None, block_hash: None });
        v.confirmed = t.height.is_some();
        v.height = t.height;
        v.block_hash = t.hash.clone();
        Ok(v)
    }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(5) }
    async fn rate(&mut self) -> Result<u64, String> { Ok(RATE) }
    async fn input_clean(&mut self, _o: Outpoint) -> Result<(), String> { Ok(()) }
    async fn sign_float(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> { Ok(vec![0x11; 64]) }
    async fn sign_custody(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> {
        self.custody_signs += 1;
        Ok(vec![0x22; 64])
    }
    fn take_offer(&mut self, _frog: u64) -> Option<Offer> { None }
    fn save(&mut self, st: &FrogState) { self.saved = Some(st.clone()); }
    fn save_sales(&mut self, ss: &SaleState) { self.saved_sales = Some(ss.clone()); }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
    async fn our_balance(&mut self) -> Result<u128, String> { Ok(self.eth) }
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
        outbound::BtcPolicy { custody_script: custody_script.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true }
    }
    fn paused(&self) -> bool { self.paused }
}

// ---- helpers -----------------------------------------------------------------------------------------------------------------

struct S {
    st: FrogState,
    fl: FloorState,
    hs: HandinState,
    ss: SaleState,
}

fn state(w: &World, policy: Option<SalePolicy>) -> S {
    let st = FrogState { sinks: vec![SinkRec { kind: SinkKind::Floor, sink: FLOOR, coin: [0; 20], view: None, refused: None, drove_refunds_at: None,
        drove_settle: false }], proofs: w.proofs(), ..Default::default() };
    let mut fl = FloorState::default();
    fl.add_sink(FLOOR, FloorItem::Bitcoin).unwrap();
    let mut ss = SaleState::default();
    if let Some(p) = policy {
        ss.set_policy(p, None).unwrap();
    }
    S { st, fl, hs: HandinState::default(), ss }
}

fn policy() -> SalePolicy {
    SalePolicy { module: FLOOR, reserve_floor_wei: E, markup_bps: 500, max_open: 1, fee_bps: 0 }
}

fn tick(w: &mut World, s: &mut S) -> Vec<String> {
    let lines = block_on(frogflow::tick_all(w, &mut s.st, &mut s.fl, &mut s.hs, &mut s.ss, false, &[]));
    for l in &lines {
        println!("{l}");
    }
    lines
}

fn phase(s: &S, id: u64) -> SPhase {
    s.ss.sales.iter().find(|x| x.id == id).expect("a sale").phase.clone()
}

fn step(s: &S) -> Option<FStep> {
    s.st.pending.as_ref().map(|p| p.step.clone())
}

/// Ask sats for the protocol sale: `price × 1.05` at the rate, rounded up.
fn expected_ask_sats() -> u64 {
    sale::ask_sats(sale::ask_wei(PRICE, 500), RATE).unwrap()
}

// ---- the protocol sale -------------------------------------------------------------------------------------------------------

#[test]
fn a_low_reserve_sells_an_inventory_item_and_reports_the_real_net_proceeds_with_the_eth_once_bridged() {
    let mut w = World::new();
    let mut s = state(&w, Some(policy()));
    // 1. planned at the read, redeemToRelist sent as the one EVM transaction, saved first
    let lines = tick(&mut w, &mut s);
    assert!(lines.iter().any(|l| l.contains("sale 0: planned")), "{lines:?}");
    assert_eq!(step(&s), Some(FStep::Relist { sale: 0, count_before: 0 }));
    assert_eq!(phase(&s, 0), SPhase::Redeeming { count_before: 0 });
    assert_eq!(s.ss.extras[0].1.reserve, E / 2);
    // 2. mined: the seed is burned, ticket 0 is ours and pinned
    w.mine_evm();
    tick(&mut w, &mut s);
    assert_eq!((w.balances[&ME], w.tickets.len(), w.tickets[0].owner), (0, 1, ME));
    assert_eq!((phase(&s, 0), s.ss.sales[0].ticket), (SPhase::Pinned, Some(0)));
    assert!(s.ss.sales.len() == 1, "no second sale while one is open");
    // 3. drand draws acquisition 7; the listing is signed by the custody key under 0x83 and recordDelivery goes out
    w.resolve(0, 7);
    let lines = tick(&mut w, &mut s);
    let SPhase::Listed { inscription, outpoint, ask_sats, ask_wei, listing, hash, .. } = phase(&s, 0) else { panic!("{:?}", phase(&s, 0)) };
    assert_eq!((inscription.as_str(), outpoint, ask_sats, ask_wei), (insc(7).as_str(), Outpoint { txid: ITEM_TXID, vout: 1 }, expected_ask_sats(), sale::ask_wei(PRICE, 500)));
    assert_eq!(w.custody_signs, 1);
    let p = Psbt::parse(&listing).unwrap();
    assert_eq!((p.tx.inputs.len(), p.tx.outputs.len(), p.tx.outputs[0].value, p.tx.outputs[0].script.clone()), (1, 1, ask_sats, custody()));
    let wit = etch_autobuyer::psbt::final_witnesses(&listing).unwrap();
    assert_eq!(wit[0].as_ref().unwrap()[0].last(), Some(&0x83));
    assert_eq!(hash, evmtx::keccak(&listing));
    assert_eq!(step(&s), Some(FStep::SaleRecord { sale: 0 }), "{lines:?}");
    let pending = s.st.pending.clone().unwrap();
    assert!(pending.raw.windows(32).any(|x| x == hash), "recordDelivery names the listing's keccak");
    assert_eq!(w.asked, vec![("listing:0".to_string(), outflow::Flow::Item)], "one item out, asked once");
    assert_eq!(w.recorded, vec!["listing:0".to_string()]);
    assert!(w.saved_sales.as_ref().unwrap().sales[0].listing().is_some(), "the listing was saved");
    // 4. recorded: the ticket is Delivered, held − 1, the item still under the caps (it is still ours)
    w.mine_evm();
    tick(&mut w, &mut s);
    assert!(matches!(phase(&s, 0), SPhase::Recorded { .. }));
    assert_eq!(w.tickets[0].stage, 3);
    assert!(!w.held.contains(&frogbuy::leaf_of(&insc(7))));
    assert!(w.book.contains(&insc(7)));
    // 5. a buyer fills it: six deep, paying custody the ask at the item's index → Sold, the cap book released
    let _fill = w.fill(outpoint, ask_sats, 0x77);
    tick(&mut w, &mut s);
    assert!(matches!(phase(&s, 0), SPhase::Recorded { .. }), "one deep: not yet");
    w.mine_btc(5);
    let lines = tick(&mut w, &mut s);
    let SPhase::Sold { sold_sats, rate, proceeds_wei, .. } = phase(&s, 0) else { panic!("{:?} {lines:?}", phase(&s, 0)) };
    assert_eq!((sold_sats, rate), (ask_sats, RATE));
    assert_eq!(proceeds_wei, sale::proceeds_wei(ask_sats, RATE).unwrap(), "net of the conversion");
    assert!(proceeds_wei < ask_wei && proceeds_wei > 640 * E / 1_000 * 97 / 100);
    assert!(!w.book.contains(&insc(7)), "sold: its room under the caps is free");
    assert_eq!(s.ss.sales[0].phase, phase(&s, 0));
    // 6. the report waits for the ETH on 4663 (the operator bridges the sats)
    assert!(lines.iter().any(|l| l.contains("wait to be bridged")), "{lines:?}");
    assert!(step(&s).is_none() && w.item_sold.is_empty());
    w.eth += proceeds_wei;
    let lines = tick(&mut w, &mut s);
    assert_eq!(step(&s), Some(FStep::SaleReport { sale: 0 }), "{lines:?}");
    assert_eq!(w.asked.last(), Some(&("sale:0".to_string(), outflow::Flow::Value)));
    // 7. itemSold{value: proceeds} mined: the module's inventory −1, its reserve +proceeds, the canister holds the seed again
    w.mine_evm();
    let (to, data, value, ok) = w.calls.last().cloned().unwrap();
    assert_eq!((to, &data[..4], value, ok), (FLOOR, &sale::ITEM_SOLD[..], proceeds_wei, true));
    assert_eq!(arg(&data, 0), proceeds_wei, "msg.value == proceeds");
    tick(&mut w, &mut s);
    assert_eq!(phase(&s, 0), SPhase::Reported { proceeds_wei });
    assert_eq!((w.item_sold.clone(), w.floor.float_units, w.floor.reserve, w.balances[&ME]), (vec![proceeds_wei], 0, E / 2 + proceeds_wei, E));
    // 8. the sale itself lifted the reserve over the floor: no new sale, and the page shows why
    tick(&mut w, &mut s);
    assert_eq!(s.ss.sales.len(), 1);
    assert!(s.ss.extras[0].1.reserve >= E && s.ss.extras[0].1.float_units == 0);
    // ...and had it not, the empty inventory would have stopped one
    w.floor.reserve = E / 2;
    let lines = tick(&mut w, &mut s);
    assert_eq!(s.ss.sales.len(), 1);
    assert!(lines.iter().any(|l| l.contains("no inventory")), "{lines:?}");
}

#[test]
fn no_policy_no_low_reserve_no_inventory_no_seed_or_a_stale_floor_sells_nothing() {
    // no policy: the sale side does not even read
    let mut w = World::new();
    let mut s = state(&w, None);
    tick(&mut w, &mut s);
    assert!(s.ss.sales.is_empty() && s.ss.extras.is_empty());
    // the reserve at its floor
    let mut w = World::new();
    w.floor.reserve = E;
    let mut s = state(&w, Some(policy()));
    tick(&mut w, &mut s);
    assert!(s.ss.sales.is_empty());
    // no inventory
    let mut w = World::new();
    w.floor.float_units = E - 1;
    let mut s = state(&w, Some(policy()));
    let lines = tick(&mut w, &mut s);
    assert!(s.ss.sales.is_empty() && lines.iter().any(|l| l.contains("no inventory")), "{lines:?}");
    // no seed
    let mut w = World::new();
    w.balances.insert(ME, E - 1);
    let mut s = state(&w, Some(policy()));
    let lines = tick(&mut w, &mut s);
    assert!(s.ss.sales.is_empty() && lines.iter().any(|l| l.contains("no seed")), "{lines:?}");
    // a stale floor
    let mut w = World::new();
    w.advance(3_600 - 60);
    let mut s = state(&w, Some(policy()));
    let lines = tick(&mut w, &mut s);
    assert!(s.ss.sales.is_empty() && lines.iter().any(|l| l.contains("stale")), "{lines:?}");
    assert!(w.tickets.is_empty() && w.custody_signs == 0 && w.mempool.is_empty());
}

#[test]
fn the_report_waits_while_the_floor_rose_past_the_net_proceeds_and_a_fill_reorganised_out_goes_back_to_watching() {
    let mut w = World::new();
    let mut s = state(&w, Some(policy()));
    tick(&mut w, &mut s);
    w.mine_evm();
    tick(&mut w, &mut s);
    w.resolve(0, 7);
    tick(&mut w, &mut s);
    w.mine_evm();
    tick(&mut w, &mut s);
    let SPhase::Recorded { outpoint, ask_sats, .. } = phase(&s, 0) else { panic!() };
    // a fill, then its block is replaced before six deep: back to watching, nothing reported
    let fill = w.fill(outpoint, ask_sats, 0x77);
    w.mine_btc(2);
    tick(&mut w, &mut s);
    assert!(matches!(phase(&s, 0), SPhase::Recorded { .. }));
    w.txs.get_mut(&fill).unwrap().height = None;
    w.txs.get_mut(&fill).unwrap().hash = None;
    w.outspends.remove(&outpoint);
    w.outputs.get_mut(&outpoint).unwrap().1 = false;
    let lines = tick(&mut w, &mut s);
    assert!(matches!(phase(&s, 0), SPhase::Recorded { .. }), "{lines:?}");
    // it is mined again and buried
    w.txs.get_mut(&fill).unwrap().height = None;
    w.outspends.insert(outpoint, (fill, true));
    w.mine_btc(6);
    tick(&mut w, &mut s);
    let SPhase::Sold { proceeds_wei, .. } = phase(&s, 0) else { panic!("{:?}", phase(&s, 0)) };
    // the ETH is there, but the module's minimum rose above the net (a stale-then-higher floor): the report waits and says so
    w.eth += proceeds_wei;
    w.floor.posted_at = w.now; // still fresh
    let saved = w.exec(FLOOR, &sale::item_sold(proceeds_wei), proceeds_wei, false);
    assert!(saved.is_ok());
    // fake a higher minimum by lowering what we realised: the world's minimum is fixed, so cut the rate's worth instead
    if let Some(x) = s.ss.sale_mut(0) {
        if let SPhase::Sold { proceeds_wei: p, .. } = &mut x.phase {
            *p = 640 * E / 1_000 * 97 / 100 - 1;
        }
    }
    let lines = tick(&mut w, &mut s);
    assert!(step(&s).is_none() && lines.iter().any(|l| l.contains("under the module's minimum")), "{lines:?}");
    assert!(w.item_sold.is_empty());
}

// ---- a YOYO relist -----------------------------------------------------------------------------------------------------------

/// A vault relisted one unit: ticket 0 to custody, owned by the vault, drawn to acquisition 8, listed by the vault at 1.1× cap.
fn relisted(w: &mut World) {
    w.tickets.push(Tk { stage: 2, owner: YOYO, acq: 8, dest: custody_address() });
    w.listings.insert(0, (1, 704 * E / 1_000));
}

#[test]
fn a_vaults_relist_ticket_is_listed_at_the_vaults_ask_recorded_and_its_sale_reported_with_record_sale() {
    let mut w = World::new();
    w.floor.reserve = 2 * E; // the module wants nothing: this is the vault's sale
    relisted(&mut w);
    let mut s = state(&w, Some(policy()));
    // found at the ticket walk, adopted once, the listing signed at the vault's ask
    let lines = tick(&mut w, &mut s);
    assert!(lines.iter().any(|l| l.contains("is a relist by vault")), "{lines:?}");
    assert_eq!((s.ss.sales.len(), s.ss.sales[0].vault, s.ss.sales[0].module, s.ss.sales[0].ticket), (1, Some(YOYO), None, Some(0)));
    let SPhase::Listed { inscription, ask_sats, ask_wei, outpoint, .. } = phase(&s, 0) else { panic!("{:?}", phase(&s, 0)) };
    assert_eq!((inscription.as_str(), ask_wei, ask_sats), (insc(8).as_str(), 704 * E / 1_000, sale::ask_sats(704 * E / 1_000, RATE).unwrap()));
    assert_eq!(step(&s), Some(FStep::SaleRecord { sale: 0 }));
    assert_eq!(s.ss.ticket_cursor, 1);
    w.mine_evm();
    tick(&mut w, &mut s);
    assert!(matches!(phase(&s, 0), SPhase::Recorded { .. }));
    assert_eq!(w.tickets[0].stage, 3);
    // sold: recordSale{value} with the net, at least ask × 0.95
    w.fill(outpoint, ask_sats, 0x78);
    w.mine_btc(6);
    tick(&mut w, &mut s);
    let SPhase::Sold { proceeds_wei, .. } = phase(&s, 0) else { panic!("{:?}", phase(&s, 0)) };
    assert!(proceeds_wei >= sale::relist_min_proceeds(ask_wei));
    w.eth += proceeds_wei;
    tick(&mut w, &mut s);
    assert_eq!(step(&s), Some(FStep::SaleReport { sale: 0 }));
    w.mine_evm();
    let (to, data, value, ok) = w.calls.last().cloned().unwrap();
    assert_eq!((to, &data[..4], value, ok), (YOYO, &sale::RECORD_SALE[..], proceeds_wei, true));
    tick(&mut w, &mut s);
    assert_eq!(phase(&s, 0), SPhase::Reported { proceeds_wei });
    assert_eq!(w.sales_recorded, vec![(0, proceeds_wei)]);
    assert!(w.item_sold.is_empty(), "the module was never told: it was the vault's sale");
    // a second tick adopts nothing twice
    tick(&mut w, &mut s);
    assert_eq!(s.ss.sales.len(), 1);
}

#[test]
fn a_delisted_relist_is_moved_to_a_fresh_outpoint_re_deposited_to_the_vault_and_reclaimed() {
    let mut w = World::new();
    w.floor.reserve = 2 * E;
    relisted(&mut w);
    let mut s = state(&w, Some(policy()));
    tick(&mut w, &mut s);
    // a delist before the record is refused; after it, accepted
    assert!(matches!(phase(&s, 0), SPhase::Listed { .. }));
    w.mine_evm();
    tick(&mut w, &mut s);
    let SPhase::Recorded { inscription, outpoint, .. } = phase(&s, 0) else { panic!() };
    s.ss.sale_mut(0).unwrap().phase = SPhase::Delisting { inscription: inscription.clone(), outpoint };
    // the move: both inputs custody's, every output custody's, signed by the custody key, saved and sent
    let lines = tick(&mut w, &mut s);
    let SPhase::Moving { txid, raw, fee_input, outpoint: new_out, .. } = phase(&s, 0) else { panic!("{:?} {lines:?}", phase(&s, 0)) };
    assert_eq!((fee_input, new_out), (Outpoint { txid: [0xfe; 32], vout: 0 }, Outpoint { txid, vout: 0 }));
    assert_eq!(w.btc_sent.last().unwrap(), &raw);
    let tx = parse_signed(&raw).unwrap();
    assert!(tx.outputs.iter().all(|(_, s)| *s == custody()));
    assert_eq!(tx.outputs[0].0, 10_000);
    assert_eq!(s.ss.reserved(), vec![fee_input], "the move's fee input is reserved from the deliveries");
    // six deep: the item is at the new outpoint; wrapDeposit(…, to = vault) then reclaimListing
    w.mine_btc(6);
    let lines = tick(&mut w, &mut s);
    assert_eq!(phase(&s, 0), SPhase::Redeposit { inscription: inscription.clone(), outpoint: new_out }, "{lines:?}");
    assert_eq!(w.loc[&inscription].output, new_out);
    assert_eq!(step(&s), Some(FStep::SaleDeposit { sale: 0 }), "{lines:?}");
    assert_eq!(w.asked.last(), Some(&("mint:sale:0".to_string(), outflow::Flow::Mint)));
    w.mine_evm();
    let (to, data, _, ok) = w.calls.last().cloned().unwrap();
    assert_eq!((to, &data[..4], ok), (W, &deposits::WRAP_DEPOSIT[..], true));
    assert_eq!(&arg_word(&data, 4)[12..], &YOYO[..], "deposited to the vault");
    assert_eq!(w.balances[&YOYO], E);
    tick(&mut w, &mut s);
    assert_eq!(phase(&s, 0), SPhase::Reclaiming);
    assert_eq!(step(&s), Some(FStep::SaleReclaim { sale: 0 }));
    w.mine_evm();
    tick(&mut w, &mut s);
    assert_eq!(phase(&s, 0), SPhase::Returned);
    assert_eq!(w.reclaimed, vec![0]);
    assert!(w.held.contains(&frogbuy::leaf_of(&inscription)), "back in the set");
}

#[test]
fn a_stranger_relist_ticket_and_a_refunded_one_are_not_listed() {
    // a stranger burned a unit to a custody-destination ticket: no vault behind it, nothing listed, the walk moves on
    let mut w = World::new();
    w.floor.reserve = 2 * E;
    w.tickets.push(Tk { stage: 2, owner: [0x99; 20], acq: 8, dest: custody_address() });
    let mut s = state(&w, Some(policy()));
    let lines = tick(&mut w, &mut s);
    assert!(s.ss.sales.is_empty() && lines.iter().any(|l| l.contains("not listed")), "{lines:?}");
    assert_eq!((s.ss.ticket_cursor, w.custody_signs), (1, 0));
    // an ordinary redemption (to a holder) is not the sale side's either
    let mut w = World::new();
    w.floor.reserve = 2 * E;
    w.tickets.push(Tk { stage: 2, owner: [0x99; 20], acq: 8, dest: etch_autobuyer::policy::taproot_address(&[0x11; 32], "bc") });
    let mut s = state(&w, Some(policy()));
    tick(&mut w, &mut s);
    assert!(s.ss.sales.is_empty() && s.ss.ticket_cursor == 1);
}

// ---- the caps and the limiter ------------------------------------------------------------------------------------------------

#[test]
fn the_limiter_makes_a_second_listing_in_a_day_wait_and_a_frozen_one_signs_nothing() {
    // a vault's relist and a protocol sale in the same day, at one item out a day
    let mut w = World::new();
    w.limits = Some(outflow::Limits { per_collection_items: 1, global_items: 2, per_collection_mints: 6, global_mints: 12,
        global_value_wei: 20 * E, trip_factor: 3 });
    relisted(&mut w);
    let mut s = state(&w, Some(policy()));
    let lines = tick(&mut w, &mut s);
    // the protocol sale (sale 0: the module is read first) is planned and redeems; the vault's relist (ticket 0, sale 1) is
    // listed and takes the day's one item out
    assert_eq!(phase(&s, 0), SPhase::Redeeming { count_before: 1 }, "{lines:?}");
    assert!(matches!(phase(&s, 1), SPhase::Listed { .. }), "{lines:?}");
    w.mine_evm(); // redeemToRelist went first (one at a time); the relist's recordDelivery next tick
    tick(&mut w, &mut s);
    w.mine_evm();
    tick(&mut w, &mut s);
    assert_eq!((phase(&s, 0), s.ss.sales[0].ticket), (SPhase::Pinned, Some(1)));
    assert!(matches!(phase(&s, 1), SPhase::Recorded { .. }));
    w.resolve(1, 7);
    let lines = tick(&mut w, &mut s);
    assert!(matches!(phase(&s, 0), SPhase::Listing { .. }), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("waits (outflow limit)")), "{lines:?}");
    assert_eq!(w.custody_signs, 1, "one listing signed today");
    // the day passes: the second listing goes
    w.advance(24 * 3_600 + 1);
    w.floor.posted_at = w.now;
    tick(&mut w, &mut s);
    assert!(matches!(phase(&s, 0), SPhase::Listed { .. }));
    assert_eq!(w.custody_signs, 2);
    // frozen: nothing is signed
    let mut w = World::new();
    w.limits = Some(outflow::Limits::stage0());
    w.outflow.freeze("a guardian", T0);
    relisted(&mut w);
    let mut s = state(&w, Some(policy()));
    let lines = tick(&mut w, &mut s);
    assert!(matches!(phase(&s, 1), SPhase::Listing { .. }) && w.custody_signs == 0, "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("frozen")) && w.paused, "the trip pauses the canister (H1): {lines:?}");
}

#[test]
fn a_restart_between_the_send_and_the_save_resumes_from_the_saved_step() {
    let mut w = World::new();
    let mut s = state(&w, Some(policy()));
    tick(&mut w, &mut s);
    // the canister dies after `send` saved the pending step and before the sale's phase was saved: the saved states are all
    // there is. The FrogState carries the step (with the ticket count it read), the sale is still Planned.
    let saved_st = w.saved.clone().unwrap();
    let mut saved_ss = w.saved_sales.clone().unwrap();
    assert_eq!(saved_st.pending.as_ref().map(|p| p.step.clone()), Some(FStep::Relist { sale: 0, count_before: 0 }));
    saved_ss.sale_mut(0).unwrap().phase = SPhase::Planned;
    let mut s2 = S { st: saved_st, fl: s.fl.clone(), hs: HandinState::default(), ss: saved_ss };
    w.mine_evm();
    tick(&mut w, &mut s2);
    assert_eq!((phase(&s2, 0), s2.ss.sales[0].ticket), (SPhase::Pinned, Some(0)), "the mined step settles the sale from the step alone");
    assert_eq!(w.tickets.len(), 1, "no second redeem");
    let _ = &mut s;
}

/// Conversion review C-04 (docs/audit/2026-10-09-conv-review-fixes.md): the 4663 balance is shared with the Relay leg; a sale's
/// report (`itemSold{value}`) waits while a conversion holds the leg (a funded conversion's wei waiting for its quote would
/// otherwise pay it), and goes once the leg is free.
#[test]
fn c04_the_sale_report_waits_while_a_conversion_holds_the_relay_leg() {
    let mut w = World::new();
    let mut s = state(&w, Some(policy()));
    tick(&mut w, &mut s);
    w.mine_evm();
    tick(&mut w, &mut s);
    w.resolve(0, 7);
    tick(&mut w, &mut s);
    let SPhase::Listed { outpoint, ask_sats, .. } = phase(&s, 0) else { panic!("{:?}", phase(&s, 0)) };
    w.mine_evm();
    tick(&mut w, &mut s);
    let _fill = w.fill(outpoint, ask_sats, 0x77);
    w.mine_btc(5);
    tick(&mut w, &mut s);
    let SPhase::Sold { proceeds_wei, .. } = phase(&s, 0) else { panic!("{:?}", phase(&s, 0)) };
    // a conversion funded at our address, waiting for its quote: the balance covers the proceeds, but it is the conversion's
    let mut c = etch_autobuyer::relayconv::ConvRec::new(0, FLOOR);
    c.advanced = proceeds_wei;
    c.phase = etch_autobuyer::relayconv::CPhase::Funded { wei: proceeds_wei, since: w.now };
    s.st.convs = Some(vec![c]);
    w.eth += proceeds_wei;
    let lines = tick(&mut w, &mut s);
    assert!(step(&s).is_none() && w.item_sold.is_empty(), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("C-04")), "{lines:?}");
    // the conversion done (settled): the report goes
    s.st.convs.as_mut().unwrap()[0].phase = etch_autobuyer::relayconv::CPhase::Settled { cost: 1, spare_sats: 0 };
    let lines = tick(&mut w, &mut s);
    assert_eq!(step(&s), Some(FStep::SaleReport { sale: 0 }), "{lines:?}");
}
