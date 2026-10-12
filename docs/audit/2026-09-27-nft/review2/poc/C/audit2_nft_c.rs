//! Audit 2026-09-27-nft, ROUND 2 re-review, part C (canister flow, concurrency, upgrade): proofs of concept.
//! docs/audit/2026-09-27-nft/review2/R2-canister-flow.md. A PASSING test demonstrates the weakness; each turns red once fixed.
//! The fake world is `tests/audit_nft.rs`'s (copied verbatim, plus one switch `drop_milady` that removes Milady from the
//! configuration, as an upgrade would).

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custodycap::{self, Book, CapPolicy};
use etch_autobuyer::ethflow::{self as ef, BPhase, DPhase, EthIo, EthPin, EthPlan, EthState, OurReceipt, XPhase};
use etch_autobuyer::ethnft::{self as en, EthCollection, EthNftConfig, LogV, ReceiptV, Seen, Stage};
use etch_autobuyer::evmtx::{keccak, Tx1559};
use etch_autobuyer::itemsink::Clock;
use etch_autobuyer::looper::EvmView;
use etch_autobuyer::outflow::Flow;
use etch_autobuyer::relayconv as rc;
use etch_autobuyer::seaport::{self as sp, Word};

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
const QUOTE: &str = include_str!("fixtures/relay-quote-4663-1-2026-09-27.json");
const SALES: &str = include_str!("fixtures/seaport/milady-sales-receipts.json");
const FILL: &str = include_str!("fixtures/seaport/eth-milady-fill-26055691.json");
const US: [u8; 20] = [0x76, 0x02, 0x0b, 0x70, 0x93, 0xbd, 0x35, 0xd2, 0x52, 0x0d, 0x47, 0xde, 0xd7, 0x64, 0xef, 0xc5, 0x54, 0x23, 0x7e, 0xfe];
const NOW: u64 = 1_791_117_628 - 7 * 86_400 + 5;
const MILADY: [u8; 20] = [0x5a, 0xf0, 0xd9, 0x82, 0x7e, 0x0c, 0x53, 0xe4, 0x79, 0x9b, 0xb2, 0x26, 0x65, 0x5a, 0x1d, 0xe1, 0x52, 0xa4, 0x25, 0xa5];
const AZUKI: [u8; 20] = [0xed, 0x5a, 0xf3, 0x88, 0x65, 0x3d, 0x22, 0x8b, 0xa4, 0x3d, 0x7a, 0x3d, 0x2a, 0x37, 0x5a, 0x19, 0x1e, 0x10, 0xc5, 0x44];
const WRAPPER: [u8; 20] = [0xaa; 20];
const MODULE: [u8; 20] = [0xbb; 20];
const WRAPPER2: [u8; 20] = [0xa2; 20];
const MODULE2: [u8; 20] = [0xb2; 20];
const ALICE: [u8; 20] = [0xa1; 20];
const BOB: [u8; 20] = [0xb0; 20];
const ATTACKER: [u8; 20] = [0xee; 20];
const HOLDER: [u8; 20] = [0x33; 20];
const RECIPIENT: [u8; 20] = [0x44; 20];
const DEPOSIT: u128 = 500_000_000_000_000_000;
const ARRIVED: u128 = 498_443_962_449_863_321;
const ETH_PIN: u64 = 26_068_928;
/// What `EthNet::fees` now returns when a provider fails or the providers disagree (never a revert).
const TRANSIENT: &str = "unread (simulation): the providers did not all answer alike (1 ok, 0 revert, 1 failed: chain 1: eth_call: IC0406: \
    SysTransient: Timeout expired): nothing is decided";
/// … and when every provider agrees the call reverts.
const REVERTS: &str = "refused (simulation): it reverts at the pin on every provider: no result: {\"code\":3,\"reverted\":true}";

struct Ticket {
    stage: Stage,
    item: u128,
    dest: [u8; 20],
    deliver_by: u64,
}

#[derive(Clone)]
struct Module {
    net_units: u128,
    item_cap: u128,
    transit: bool,
    accepted: u128,
    posted_at: u64,
    /// `floorBounds()`
    bounds: (u128, u128),
    posted: Vec<(u128, u16, u32)>,
}

impl Module {
    fn new(accepted: u128) -> Self {
        Module { net_units: 0, item_cap: DEPOSIT + rc::GAS_KEEP_WEI, transit: false, accepted, posted_at: NOW - 60, bounds: (0, u128::MAX), posted: vec![] }
    }
}

struct Fake {
    now: u64,
    paused: bool,
    deposits_on: bool,
    two: bool,
    /// the configuration no longer names Milady (an upgrade dropped it)
    drop_milady: bool,
    home_balance: u128,
    home_nonce: u64,
    modules: HashMap<[u8; 20], Module>,
    keeper: [u8; 20],
    allocator: [u8; 20],
    home_sent: HashMap<[u8; 32], (bool, Vec<u8>)>,
    home_calls: Vec<([u8; 20], u128, Vec<u8>)>,
    tickets: Vec<Ticket>,
    items: Vec<Word>,
    tickets2: Vec<Ticket>,
    items2: Vec<Word>,
    marked: Vec<u128>,
    marked2: Vec<u128>,
    home_final_lag: u64,
    eth_balance: u128,
    eth_nonce: u64,
    /// the nonce as a lower pin shows it, for the next plan only (B-M-4)
    stale_nonce: Option<u64>,
    finalized: u64,
    owners: HashMap<([u8; 20], Word), [u8; 20]>,
    eth_signed: Vec<(Tx1559, [u8; 32])>,
    receipts: HashMap<[u8; 32], OurReceipt>,
    /// the receipt read of this hash fails this many more times
    receipt_err: HashMap<[u8; 32], u32>,
    /// the next transfer plans fail with these texts, in order
    plan_err: Vec<String>,
    /// outflow keys starting with one of these wait (H3's limit for that key)
    wait_prefix: Vec<String>,
    broadcasts: Vec<Vec<u8>>,
    listing_total: u128,
    sale_receipts: HashMap<[u8; 32], ReceiptV>,
    /// `eth_sale_receipt` of these hashes fails (over the size cap, providers disagree, …)
    sale_err: Vec<[u8; 32]>,
    sale_reads: u32,
    deposit_txs: HashMap<[u8; 32], ([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>,
    caps: CapPolicy,
    book: Book,
    outflow: Vec<(String, Flow, u128)>,
}

fn word(v: u128) -> Vec<u8> {
    sp::word_u128(v).to_vec()
}
fn addr_word(a: &[u8; 20]) -> Vec<u8> {
    sp::word_addr(a).to_vec()
}
fn sel(d: &[u8]) -> [u8; 4] {
    d[..4].try_into().unwrap()
}
fn arg(d: &[u8], i: usize) -> u128 {
    u128::from_be_bytes(d[4 + 32 * i + 16..4 + 32 * i + 32].try_into().unwrap())
}
fn tok(v: u128) -> Word {
    sp::word_u128(v)
}
fn hx(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

impl Fake {
    fn new(two: bool) -> Self {
        let mut sale_receipts = HashMap::new();
        let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
        for (k, r) in v["receipts"].as_object().unwrap() {
            let h: [u8; 32] = hex::decode(&k[2..]).unwrap().try_into().unwrap();
            sale_receipts.insert(h, en::receipt_of(r).unwrap().unwrap());
        }
        let mut modules = HashMap::new();
        modules.insert(MODULE, Module::new(93 * E / 100));
        let mut keys = vec![custodycap::evm_key(1, &MILADY)];
        if two {
            modules.insert(MODULE2, Module::new(67 * E / 100));
            keys.push(custodycap::evm_key(1, &AZUKI));
        }
        Fake {
            now: NOW, paused: false, deposits_on: false, two, drop_milady: false, home_balance: 2_000_000_000_000_000, home_nonce: 0, modules, keeper: US,
            allocator: rc::RELAY_ALLOCATOR_4663, home_sent: HashMap::new(), home_calls: vec![], tickets: vec![], items: vec![], tickets2: vec![],
            items2: vec![], marked: vec![], marked2: vec![], home_final_lag: 0, eth_balance: 10_000_000_000_000_000, eth_nonce: 0,
            stale_nonce: None, finalized: ETH_PIN - 100, owners: HashMap::new(), eth_signed: vec![], receipts: HashMap::new(),
            receipt_err: HashMap::new(), plan_err: vec![], wait_prefix: vec![], broadcasts: vec![], listing_total: 45 * E / 100, sale_receipts,
            sale_err: vec![], sale_reads: 0, deposit_txs: HashMap::new(), caps: custodycap::stage0(&keys), book: Book::default(), outflow: vec![],
        }
    }
    fn m(&mut self, module: [u8; 20]) -> &mut Module {
        self.modules.get_mut(&module).unwrap()
    }
    fn cfg(&self) -> EthNftConfig {
        let col = |c: [u8; 20], w: [u8; 20], m: [u8; 20]| EthCollection { collection: hx(&c), wrapper: hx(&w), module: hx(&m),
            max_purchase_wei: 2 * E, max_premium_bps: 500 };
        let mut collections = if self.drop_milady { vec![] } else { vec![col(MILADY, WRAPPER, MODULE)] };
        if self.two {
            collections.push(col(AZUKI, WRAPPER2, MODULE2));
        }
        EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: self.deposits_on, floor: None, collections }
    }
    fn apply_home(&mut self, to: [u8; 20], value: u128, d: &[u8]) -> bool {
        self.home_calls.push((to, value, d.to_vec()));
        if to == rc::RELAY_DEPOSITORY_4663 {
            self.home_balance -= value;
            return true;
        }
        let now = self.now;
        if self.modules.contains_key(&to) {
            let s = sel(d);
            let m = self.m(to);
            if s == rc::ADVANCE {
                m.transit = true;
                self.home_balance += arg(d, 0);
            } else if s == rc::RETURN_ADVANCE {
                m.transit = false;
                self.home_balance -= value;
            } else if s == rc::SETTLE_ADVANCE {
                m.transit = false;
                m.net_units = m.net_units.saturating_sub(E);
            } else if s == en::POST_FLOOR {
                m.posted.push((arg(d, 0), arg(d, 1) as u16, arg(d, 2) as u32));
                m.accepted = arg(d, 0);
                m.posted_at = now;
            } else {
                panic!("an unexpected module call");
            }
            return true;
        }
        match (to, sel(d)) {
            (WRAPPER, s) if s == en::MINT || s == en::MINT_DEPOSIT => self.items.push(d[4..36].try_into().unwrap()),
            (WRAPPER2, s) if s == en::MINT || s == en::MINT_DEPOSIT => self.items2.push(d[4..36].try_into().unwrap()),
            (WRAPPER, s) if s == en::RECORD_DELIVERY => {
                let t = &mut self.tickets[arg(d, 0) as usize];
                if t.stage != Stage::Resolved {
                    return false;
                }
                t.stage = Stage::Delivered;
            }
            (WRAPPER2, s) if s == en::RECORD_DELIVERY => self.tickets2[arg(d, 0) as usize].stage = Stage::Delivered,
            (WRAPPER, s) if s == en::MARK_UNDELIVERABLE => self.marked.push(arg(d, 0)),
            (WRAPPER2, s) if s == en::MARK_UNDELIVERABLE => self.marked2.push(arg(d, 0)),
            _ => panic!("an unexpected home-chain call to 0x{}: {}", hex::encode(to), hex::encode(d)),
        }
        true
    }
    fn mine_eth(&mut self, i: usize, success: bool, block: u64) {
        let (tx, hash) = self.eth_signed[i].clone();
        let fee = 21_000 * 1_000_000_000;
        self.eth_balance -= fee + if success { tx.value } else { 0 };
        self.eth_nonce = self.eth_nonce.max(tx.nonce + 1);
        if success {
            if tx.to == sp::SEAPORT_1_6 {
                self.owners.insert((MILADY, tok(9_180)), US);
            } else if tx.to == MILADY || tx.to == AZUKI {
                let dest: [u8; 20] = tx.data[4 + 32 + 12..4 + 64].try_into().unwrap();
                self.owners.insert((tx.to, tx.data[4 + 64..4 + 96].try_into().unwrap()), dest);
            }
        }
        self.receipts.insert(hash, OurReceipt { block, success, fee });
    }
    fn wrapper_read(tickets: &[Ticket], items: &[Word], d: &[u8]) -> Vec<u8> {
        let s = sel(d);
        if s == en::TICKET_COUNT { word(tickets.len() as u128) }
        else if s == en::TICKET {
            let t = &tickets[arg(d, 0) as usize];
            let st = match t.stage { Stage::None => 0, Stage::Pinned => 1, Stage::Resolved => 2, Stage::Delivered => 3, Stage::Refunded => 4 };
            [word(st), addr_word(&ALICE), word(0), word(0), word(0), word(t.deliver_by as u128), word(0), word(t.item), word(0), addr_word(&t.dest)].concat()
        } else if s == en::ITEM {
            [items[arg(d, 0) as usize].to_vec(), word(1), word(0)].concat()
        } else if s == en::ITEM_OF {
            let t: Word = d[4..36].try_into().unwrap();
            word(items.iter().position(|x| *x == t).map_or(0, |i| i as u128 + 1))
        } else { word(0) }
    }
    fn plan_nonce(&mut self) -> u64 {
        self.stale_nonce.take().unwrap_or(self.eth_nonce)
    }
}

impl EthIo for Fake {
    fn now(&self) -> u64 { self.now }
    fn home_chain(&self) -> u64 { 4663 }
    fn eth_chain(&self) -> u64 { 1 }
    fn config(&self) -> Option<EthNftConfig> { Some(self.cfg()) }
    fn paused(&self) -> bool { self.paused }
    fn eth_max_fee(&self) -> u128 { 50_000_000_000 }
    fn save(&mut self, _st: &EthState) {}
    fn with_caps<R>(&mut self, f: impl FnOnce(&CapPolicy, &mut Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, key: &str, _c: &str, _flow: Flow, _amount: u128) -> Result<(), String> {
        if self.wait_prefix.iter().any(|p| key.starts_with(p.as_str())) {
            Err(format!("waits (outflow limit) until {}: 3 of 3 in the last 24 h", self.now + 3_600))
        } else {
            Ok(())
        }
    }
    fn outflow_record(&mut self, key: &str, _c: &str, flow: Flow, amount: u128) { self.outflow.push((key.into(), flow, amount)); }

    async fn our_evm(&mut self) -> Result<[u8; 20], String> { Ok(US) }
    async fn home_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> {
        if to == rc::RELAY_DEPOSITORY_4663 && sel(d) == rc::ALLOCATOR {
            return Ok(addr_word(&self.allocator));
        }
        let s = sel(d);
        if let Some(m) = self.modules.get(&to).cloned() {
            use etch_autobuyer::floorsink as fs;
            return Ok(if s == fs::NET_UNITS { word(m.net_units) }
                else if s == fs::ESCROW { word(10 * E) }
                else if s == fs::BUFFER { word(E / 20) }
                else if s == fs::FLOOR { [word(m.accepted), word(m.posted_at as u128), word(3_600)].concat() }
                else if s == fs::MAX_ITEM_COST { word(if m.transit { 0 } else { m.item_cap }) }
                else if s == fs::ITEMS_ARRIVED { word(0) }
                else if s == rc::TRANSIT_UNITS { word(if m.transit { E } else { 0 }) }
                else if s == en::ACCEPTED { word(m.accepted) }
                else if s == en::KEEPER { addr_word(&self.keeper) }
                else if s == ef::FLOOR_BOUNDS { [word(m.bounds.0), word(m.bounds.1)].concat() }
                else { word(0) });
        }
        Ok(match to {
            WRAPPER => Fake::wrapper_read(&self.tickets, &self.items, d),
            WRAPPER2 => Fake::wrapper_read(&self.tickets2, &self.items2, d),
            _ => return Err(format!("no contract at 0x{}", hex::encode(to))),
        })
    }
    async fn home_clock(&mut self) -> Result<Clock, String> { Ok(Clock { time: self.now, parent_block: 26_000_000 }) }
    async fn home_balance(&mut self) -> Result<u128, String> { Ok(self.home_balance) }
    async fn home_sign(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> {
        let raw = [&to[..], &value.to_be_bytes(), &self.home_nonce.to_be_bytes(), data].concat();
        Ok((raw.clone(), keccak(&raw), self.home_nonce))
    }
    async fn home_broadcast(&mut self, raw: &[u8]) {
        let h = keccak(raw);
        if self.home_sent.contains_key(&h) {
            return;
        }
        let to: [u8; 20] = raw[..20].try_into().unwrap();
        let value = u128::from_be_bytes(raw[20..36].try_into().unwrap());
        let ok = self.apply_home(to, value, &raw[44..]);
        self.home_nonce += 1;
        self.home_sent.insert(h, (ok, raw.to_vec()));
    }
    async fn home_seen(&mut self, h: [u8; 32]) -> EvmView {
        match self.home_sent.get(&h) {
            Some((ok, _)) => EvmView::Mined { success: *ok },
            None => EvmView::NotYet,
        }
    }
    async fn home_nonce(&mut self) -> Result<u64, String> { Ok(self.home_nonce) }
    async fn home_finalized_time(&mut self) -> Result<u64, String> { Ok(self.now.saturating_sub(self.home_final_lag).saturating_add(1)) }

    async fn eth_pin(&mut self) -> Result<EthPin, String> { Ok(EthPin { number: ETH_PIN, timestamp: self.now, base_fee: 100_000_000 }) }
    async fn eth_finalized(&mut self) -> Result<u64, String> { Ok(self.finalized) }
    async fn eth_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> {
        assert!(to == MILADY || to == AZUKI);
        assert_eq!(sel(d), sp::OWNER_OF);
        let t: Word = d[4..36].try_into().unwrap();
        Ok(addr_word(self.owners.get(&(to, t)).unwrap_or(&[0x0e; 20])))
    }
    async fn eth_balance(&mut self) -> Result<u128, String> { Ok(self.eth_balance) }
    async fn eth_nonce(&mut self) -> Result<u64, String> { Ok(self.eth_nonce) }
    async fn eth_receipt(&mut self, h: [u8; 32]) -> Result<Option<OurReceipt>, String> {
        if let Some(n) = self.receipt_err.get_mut(&h) {
            if *n > 0 {
                *n -= 1;
                return Err("chain 1: eth_getTransactionReceipt: HTTP 429 Too Many Requests".into());
            }
        }
        Ok(self.receipts.get(&h).copied())
    }
    async fn eth_plan_buy(&mut self, _order: &str, _collection: [u8; 20], cap: u128) -> Result<EthPlan, String> {
        if self.listing_total > cap {
            return Err(format!("refused (over cap): the order costs {} wei; the recorded cap is {cap}", self.listing_total));
        }
        let nonce = self.plan_nonce();
        Ok(EthPlan { to: sp::SEAPORT_1_6, value: self.listing_total, data: vec![0xfb, 0x0f, 0x3e, 0xe1], gas: 200_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce, token: tok(9_180), total: self.listing_total })
    }
    async fn eth_plan_transfer(&mut self, collection: [u8; 20], token: Word, dest: [u8; 20]) -> Result<EthPlan, String> {
        if !self.plan_err.is_empty() {
            return Err(self.plan_err.remove(0));
        }
        let nonce = self.plan_nonce();
        Ok(EthPlan { to: collection, value: 0, data: en::transfer_from_call(&US, &dest, token), gas: 60_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce, token, total: 0 })
    }
    async fn eth_sign(&mut self, tx: &Tx1559) -> Result<Vec<u8>, String> {
        let raw = [&tx.nonce.to_be_bytes()[..], &tx.to, &tx.value.to_be_bytes(), &tx.max_fee.to_be_bytes(), &tx.data].concat();
        self.eth_signed.push((tx.clone(), keccak(&raw)));
        Ok(raw)
    }
    async fn eth_broadcast(&mut self, raw: &[u8]) { self.broadcasts.push(raw.to_vec()); }
    async fn eth_listing(&mut self, s: &str, _c: [u8; 20]) -> Result<Seen, String> {
        let (o, p) = s.split_once(':').ok_or("not a fake listing")?;
        Ok(Seen { offerer: [o.parse::<u8>().unwrap(); 20], token: tok(o.parse().unwrap()), price: p.parse().unwrap() })
    }
    async fn eth_sale_receipt(&mut self, h: [u8; 32]) -> Result<Option<ReceiptV>, String> {
        self.sale_reads += 1;
        if self.sale_err.contains(&h) {
            return Err("chain 1: eth_getTransactionReceipt: IC reject, answer over the response cap: Http body exceeds size limit of 200000 bytes".into());
        }
        Ok(self.sale_receipts.get(&h).cloned())
    }
    async fn eth_deposit_tx(&mut self, h: [u8; 32]) -> Result<Option<([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>, String> {
        Ok(self.deposit_txs.get(&h).cloned())
    }
}

fn tick(f: &mut Fake, st: &mut EthState) -> Vec<String> {
    let lines = block_on(ef::tick(f, st, false));
    for l in &lines {
        println!("  | {l}");
    }
    lines
}
fn phase(st: &EthState) -> BPhase {
    st.buys.last().unwrap().phase.clone()
}
fn real_sales() -> Vec<[u8; 32]> {
    let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
    v["receipts"].as_object().unwrap().keys().map(|k| hex::decode(&k[2..]).unwrap().try_into().unwrap()).collect()
}
fn fill_hex() -> String {
    let v: serde_json::Value = serde_json::from_str(FILL).unwrap();
    v["input"].as_str().unwrap().to_string()
}
/// Three honest listings (0.92, 0.93, 0.95 ETH from three offerers): the floor they say is 0.93 (rank 2).
fn listings() -> Vec<String> {
    vec![format!("1:{}", 92 * E / 100), format!("2:{}", 93 * E / 100), format!("3:{}", 95 * E / 100)]
}
fn evidence(f: &Fake, st: &mut EthState) {
    ef::put_evidence(st, &f.cfg(), MILADY, listings(), real_sales(), f.now).unwrap();
}
/// A Seaport 1.6 `OrderFulfilled` in which `seller` sells `token` of `collection` to `buyer` for `price`, with the collection's
/// own `Transfer(seller, buyer, token)`. `seller == buyer` is a self-trade (gas only).
fn sale_receipt(collection: [u8; 20], token: u128, price: u128, seller: [u8; 20], buyer: [u8; 20], block: u64) -> ReceiptV {
    let mut data = Vec::new();
    for w in [[0x77u8; 32].to_vec(), addr_word(&buyer), word(128), word(128 + 160), word(1), word(2), addr_word(&collection), word(token),
        word(1), word(1), word(0), word(0), word(0), word(price), addr_word(&seller)] {
        data.extend(w);
    }
    ReceiptV { block, success: true, logs: vec![
        LogV { address: sp::SEAPORT_1_6, topics: vec![en::ORDER_FULFILLED, sp::word_addr(&seller), sp::word_addr(&[0; 20])], data },
        LogV { address: collection, topics: vec![en::TRANSFER, sp::word_addr(&seller), sp::word_addr(&buyer), tok(token)], data: vec![] },
    ] }
}
fn resolved(f: &mut Fake, second: bool, token: u128, dest: [u8; 20], deliver_by: u64) {
    let (c, items, tickets) = if second { (AZUKI, &mut f.items2, &mut f.tickets2) } else { (MILADY, &mut f.items, &mut f.tickets) };
    items.push(tok(token));
    tickets.push(Ticket { stage: Stage::Resolved, item: (items.len() - 1) as u128, dest, deliver_by });
    f.owners.insert((c, tok(token)), US);
}
fn held_id(c: [u8; 20], token: u128) -> String {
    format!("{}:{}", custodycap::evm_key(1, &c), hex::encode(tok(token)))
}
fn fill_custody(f: &mut Fake, collection: [u8; 20], n: u32) {
    let key = custodycap::evm_key(1, &collection);
    for i in 0..n {
        custodycap::admit(&f.caps, &mut f.book, &format!("{key}:{i}"), &key, 0).unwrap();
    }
}
/// Up to a Milady buy's arrival on Ethereum.
fn to_arrival(f: &mut Fake, st: &mut EthState) {
    f.m(MODULE).net_units = 2 * E;
    tick(f, st);
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Funded { .. }), "{:?}", phase(st));
    let id = st.buys.last().unwrap().id;
    st.used_orders.clear();
    ef::put_quote(st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(f, st);
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Bridging { .. }), "{:?}", phase(st));
    f.eth_balance += ARRIVED;
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Arrived { .. }), "{:?}", phase(st));
}
fn deposit_tx(f: &mut Fake, tx: [u8; 32], c: [u8; 20], w: [u8; 20], t: u128, recipient: [u8; 20], block: u64) {
    let mut padded = en::deposit_data(4663, &w, &recipient);
    padded.resize(64, 0);
    let input = [&en::SAFE_TRANSFER_FROM_DATA[..], &addr_word(&HOLDER), &addr_word(&US), &word(t), &word(0x80), &word(48), &padded].concat();
    let r = ReceiptV { block, success: true, logs: vec![LogV { address: c, topics: vec![en::TRANSFER, sp::word_addr(&HOLDER), sp::word_addr(&US), tok(t)], data: vec![] }] };
    f.deposit_txs.insert(tx, (HOLDER, c, input, r.clone()));
    f.sale_receipts.insert(tx, r);
    f.owners.insert((c, tok(t)), US);
}

fn buy_of(st: &EthState, c: [u8; 20]) -> Option<ef::EthBuy> {
    st.buys.iter().rev().find(|b| b.collection == c).cloned()
}
const POT: u128 = 10_000_000_000_000_000; // the Fake's starting Ethereum balance (the D3 gas pot)

// ---- C2-L-1: a buy that goes Stuck AFTER its arrival keeps its Relay watch ----------------------------------------------------------

#[test]
fn c2_a_stuck_after_arrival_holds_the_relay_leg_then_re_arrives_eth_already_spent_and_stalls_every_other_collection() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }), "{:?}", phase(&st));
    // the fill mines and pays, but the token is not ours at the pin (any of the "not ours" paths: mined_eth's ownerOf, or Bought's
    // ownerOf after finality)
    let i = f.eth_signed.len() - 1;
    f.mine_eth(i, true, ETH_PIN - 10);
    f.owners.insert((MILADY, tok(9_180)), [0x0e; 20]);
    f.m(MODULE2).net_units = 2 * E; // Azuki wants an item
    tick(&mut f, &mut st);
    let real = f.eth_balance;
    assert!(real < POT + ARRIVED - 40 * E / 100, "the fill paid 0.45 ETH: {real}");
    // in ONE tick: resolve_eth reads the paid fill, ownerOf is not us -> Stuck, but the Relay watch `last` is never cleared at
    // arrival, so the Stuck buy is `in_relay` again and buy_pass's Stuck step recomputes the arrival from the stale watch:
    // bal + (eth_out − eth_out_before) − eth_before = what arrived before, although that ETH paid the fill
    let m = buy_of(&st, MILADY).unwrap();
    assert!(m.last.is_some(), "the watch survives the arrival");
    let BPhase::Arrived { wei, .. } = m.phase else { panic!("{:?}", m.phase) };
    println!("AUDIT2-C C2-L-1: re-ARRIVED with {wei} wei earmarked; the address holds {real} wei in all (gas pot {POT})");
    assert!(wei >= ARRIVED - 1, "the spent ETH is earmarked again");
    assert!(st.earmarked_except(u64::MAX) > f.eth_balance, "earmarks exceed the whole balance");
    // Azuki's buy then bridges and arrives with its own ETH, but can never spend it: its available = balance − Milady's phantom
    assert!(buy_of(&st, AZUKI).is_some(), "the leg was freed only by the phantom arrival");
    for _ in 0..2 {
        tick(&mut f, &mut st);
    }
    let az = buy_of(&st, AZUKI).unwrap();
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    f.eth_balance += ARRIVED;
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Arrived { .. }), "{:?}", buy_of(&st, AZUKI).unwrap().phase);
    f.owners.insert((MILADY, tok(9_180)), [0x0e; 20]);
    f.listing_total = 45 * E / 100;
    let azl = vec![format!("1:{}", 66 * E / 100), format!("2:{}", 67 * E / 100), format!("3:{}", 68 * E / 100)];
    let mut hashes = Vec::new();
    for k in 0..5u8 {
        let h = [0xa0 + k; 32];
        f.sale_receipts.insert(h, sale_receipt(AZUKI, 100 + k as u128, 67 * E / 100, [0x10 + k; 20], [0x50 + k; 20], ETH_PIN - 50));
        hashes.push(h);
    }
    let n = f.eth_signed.len();
    let mut last = vec![];
    for _ in 0..3 {
        ef::put_evidence(&mut st, &f.cfg(), AZUKI, azl.clone(), hashes.clone(), f.now).unwrap();
        ef::put_listing(&mut st, az.id, fill_hex(), f.now).unwrap();
        last = tick(&mut f, &mut st);
        f.now += 600;
    }
    let refused: Vec<&String> = last.iter().filter(|l| l.contains("over the cap") || l.contains("cap")).collect();
    println!("AUDIT2-C C2-L-1: Azuki's arrived buy, a 0.45 ETH listing: {refused:?}");
    assert_eq!(f.eth_signed.len(), n, "Azuki's own arrived ETH never buys: Milady's phantom earmark takes it");
    assert!(!ef::give_back_now(&mut st.clone(), m.id, 10 * E).is_ok(), "and there is no operator exit for an Arrived buy");
}

// ---- C2-M-1: one collection's spent fill is read as another collection's Relay arrival --------------------------------------------

#[test]
fn c2_a_fill_mined_at_the_pin_but_not_yet_counted_when_the_next_deposit_is_watched_is_read_as_that_buys_arrival() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st); // Milady: arrived 0.4984 ETH
    f.m(MODULE).net_units = 0;
    evidence(&f, &mut st);
    f.listing_total = ARRIVED - 3_000_000_000_000_000; // a Milady listing at the buy's cap (what arrived less the gas margin)
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    f.m(MODULE2).net_units = 2 * E; // Azuki wants an item: its Relay leg starts now (Milady is past arrival)
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, MILADY).unwrap().phase, BPhase::Buying { .. }));
    tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    assert!(matches!(az.phase, BPhase::Funded { .. }), "{:?}", az.phase);
    // Milady's fill mines at the pin; the one read of its receipt this tick fails (a 429, or providers not yet alike)
    let i = f.eth_signed.len() - 1;
    let fill_hash = f.eth_signed[i].1;
    f.mine_eth(i, true, ETH_PIN - 5);
    f.receipt_err.insert(fill_hash, 1);
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st); // the line waits; Azuki's quote is judged and its watch takes eth_before AFTER the fill, eth_out BEFORE it
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Depositing { .. }), "{:?}", buy_of(&st, AZUKI).unwrap().phase);
    let bal_before_relay = f.eth_balance;
    tick(&mut f, &mut st); // the receipt is read: eth_out += the fill; Azuki is bridging
    tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    println!("AUDIT2-C C2-M-1: Azuki {:?}; Relay has sent nothing to Ethereum (balance {} → {})", az.phase, bal_before_relay, f.eth_balance);
    assert_eq!(f.eth_balance, bal_before_relay, "no Relay fill reached Ethereum");
    let BPhase::Arrived { wei, .. } = az.phase else { panic!("{:?}", az.phase) };
    assert!(wei >= 488_475_083_200_866_055, "Milady's spent fill counted as Azuki's arrival: {wei}");
    // the real Relay fill for Azuki lands later: unearmarked. Earmarks now exceed what is there, and Azuki's buy is capped by
    // what is really left, while its cost_of() will divide by a phantom `arrived`
    println!("AUDIT2-C C2-M-1: Azuki earmark {wei} wei; the address holds {} wei (gas pot {POT})", f.eth_balance);
    assert!(st.earmarked_except(u64::MAX) > f.eth_balance);
}

// ---- C2-M-2: an Arrived buy waiting for its listing (D4) holds its collection's deposit mints for as long, with no exit -----------------

#[test]
fn c2_an_arrived_buy_that_cannot_fill_keeps_every_deposit_of_its_collection_unminted_indefinitely() {
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    f.m(MODULE).net_units = 0;
    // the floor rose past the headroom after the advance: every listing is over what arrived (BAYC at 9 ETH has 2.6 %)
    f.listing_total = 2 * E;
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 20);
    ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
    f.finalized = ETH_PIN;
    let mut waits = String::new();
    for _ in 0..(3 * 24 * 3) {
        f.now += 1_200;
        f.m(MODULE).posted_at = f.now - 60;
        evidence(&f, &mut st);
        ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
        for l in tick(&mut f, &mut st) {
            if l.contains("holds the day's mint room") {
                waits = l;
            }
        }
    }
    println!("AUDIT2-C C2-M-2: after 3 days: deposit {:?}; buy {:?}", st.deposits[0].phase, phase(&st));
    assert!(matches!(phase(&st), BPhase::Arrived { .. }));
    assert_eq!(st.deposits[0].phase, XPhase::Mint, "the holder's deposited NFT is ours and final, never minted, never returned");
    assert!(f.items.is_empty());
    assert!(waits.contains("holds the day's mint room"), "{waits}");
    assert!(ef::give_back_now(&mut st.clone(), 0, 10 * E).is_err(), "no operator exit for an Arrived buy");
    assert!(ef::rearm_stray(&mut st.clone(), &f.cfg(), [0x91; 32], tok(950)).is_err(), "nor for the waiting deposit");
}

// ---- C2-L-2: an upgrade that drops a collection with a buy on the Relay leg stops every collection's buys ----------------------------

#[test]
fn c2_an_upgrade_dropping_a_collection_whose_buy_holds_the_relay_leg_stops_every_other_collections_buys_for_good() {
    let mut f = Fake::new(true);
    f.m(MODULE).net_units = 2 * E;
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    // an upgrade drops Milady from eth_nft.collections (the chain is kept: post_upgrade's B-L-5 guard does not look at collections)
    f.drop_milady = true;
    f.m(MODULE2).net_units = 2 * E;
    for _ in 0..20 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    println!("AUDIT2-C C2-L-2: buys {:?}; relay_busy {}", st.buys.iter().map(|b| (b.collection == AZUKI, b.phase.clone())).collect::<Vec<_>>(), st.relay_busy());
    assert!(st.relay_busy());
    assert!(buy_of(&st, AZUKI).is_none(), "Azuki wants an item and has room, but the dropped collection's buy holds the Relay leg");
}

// ---- C2-I-1: the C-L-4 probe checks the thirteen elements, not what is nested inside one ----------------------------------------------

#[derive(candid::CandidType, serde::Deserialize, Clone)]
struct SideEthTx {
    // the nft-side (57160b0) shape: none of nft-pool's non-opt fields (`to`, `data`, `gas`, `earlier`, `rivals`)
    kind: ef::EthKind,
    nonce: u64,
    hash: [u8; 32],
    raw: Vec<u8>,
    value: u128,
    max_fee: u128,
    priority_fee: u128,
    signed_at: u64,
}
#[derive(candid::CandidType, serde::Deserialize, Clone)]
struct SideEthState {
    buys: Vec<ef::EthBuy>,
    next_buy: u64,
    deliveries: Vec<ef::Delivery>,
    cursors: Vec<([u8; 20], u128)>,
    deposits: Vec<ef::DepositRec>,
    readings: Vec<ef::Reading>,
    line: Option<SideEthTx>,
    eth_out: u128,
    pending: Option<ef::HomeTx>,
    used_orders: Vec<[u8; 32]>,
    outbound: Vec<([u8; 20], Word, u64)>,
    inbox: ef::Inbox,
    rr: u64,
}

#[test]
fn c2_an_nft_side_line_inside_a_decodable_eth_state_is_what_the_probe_sees() {
    let d = ef::Delivery { wrapper: WRAPPER, collection: MILADY, ticket: 3, token: tok(77), destination: ALICE, deliver_by: NOW,
        phase: DPhase::Sent { tx: [1; 32], nonce: 9 }, note: String::new(), refused_since: None };
    let side = SideEthState { buys: vec![], next_buy: 1, deliveries: vec![d], cursors: vec![(WRAPPER, 4)], deposits: vec![], readings: vec![],
        line: Some(SideEthTx { kind: ef::EthKind::Deliver { wrapper: WRAPPER, ticket: 3 }, nonce: 9, hash: [1; 32], raw: vec![0xde, 0xad],
            value: 0, max_fee: 1, priority_fee: 1, signed_at: 1 }), eth_out: 5, pending: None, used_orders: vec![[7; 32]], outbound: vec![],
        inbox: ef::Inbox::default(), rr: 0 };
    let bytes = candid::encode_args((Some(side),)).unwrap();
    let (got,): (Option<EthState>,) = candid::decode_args(&bytes).unwrap();
    match &got {
        None => println!("AUDIT2-C C2-I-1: the whole element decodes as nothing: the probe traps the upgrade (the fixer's claim holds)"),
        Some(s) => println!("AUDIT2-C C2-I-1: the element decodes; line = {:?}; delivery phase = {:?}: the probe sees element 13 present",
            s.line.as_ref().map(|l| l.nonce), s.deliveries[0].phase),
    }
    // demonstrates: the in-flight line is silently gone while the Sent delivery survives
    let s = got.expect("decoded as Some");
    assert!(s.line.is_none() && matches!(s.deliveries[0].phase, DPhase::Sent { .. }));
}
