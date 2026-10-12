//! Audit 2026-09-27 (nft-side, reviewer B): proofs of concept against the Ethereum NFT side's fake world.
//!
//! The fake world is `tests/ethnft.rs`'s, copied (not modified) and generalised: two collections (Milady and a second one,
//! "Azuki") with their own modules and wrappers, and fault injection for the provider-facing reads (a receipt read that
//! errors once, a sale receipt that errors, a nonce read at a lower pin, a transport failure at the transfer's simulation).
//! Every test here asserts the WEAKNESS: it passes at b48b3ba and should turn red once the finding is fixed.

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custodycap::{self, Book, CapPolicy};
use etch_autobuyer::ethflow::{self as ef, BPhase, DPhase, EthIo, EthPin, EthPlan, EthState, OurReceipt, XPhase};
use etch_autobuyer::ethnft::{self as en, EthCollection, EthNftConfig, FloorParams, LogV, ReceiptV, Seen, Stage};
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
const WRAPPER: [u8; 20] = [0xaa; 20];
const MODULE: [u8; 20] = [0xbb; 20];
/// A second collection (stands in for Azuki; not its real address), its wrapper and module.
const AZUKI: [u8; 20] = [0xed, 0x5a, 0xf3, 0x88, 0x65, 0x3, 0x56, 0x7a, 0xf2, 0xf3, 0x88, 0xe6, 0x22, 0x4a, 0x7c, 0x6c, 0xa6, 0x8b, 0x3a, 0x7a];
const WRAPPER2: [u8; 20] = [0xab; 20];
const MODULE2: [u8; 20] = [0xbc; 20];
const ALICE: [u8; 20] = [0xa1; 20];
const ATTACKER: [u8; 20] = [0xee; 20];
const DEPOSIT: u128 = 500_000_000_000_000_000;
const ETH_PIN: u64 = 26_068_928;

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
    posted: Option<(u128, u16, u32)>,
}

impl Module {
    fn new() -> Self {
        Module { net_units: 2 * E, item_cap: DEPOSIT + rc::GAS_KEEP_WEI, transit: false, accepted: 93 * E / 100, posted_at: NOW - 60, posted: None }
    }
}

struct Fake {
    now: u64,
    paused: bool,
    deposits_on: bool,
    two: bool,
    home_balance: u128,
    home_nonce: u64,
    modules: HashMap<[u8; 20], Module>,
    keeper: [u8; 20],
    allocator: [u8; 20],
    home_sent: HashMap<[u8; 32], (bool, Vec<u8>)>,
    home_calls: Vec<([u8; 20], u128, Vec<u8>)>,
    tickets: Vec<Ticket>,
    items: Vec<Word>,
    marked: Vec<u128>,
    // Ethereum
    eth_balance: u128,
    eth_nonce: u64,
    /// The nonce as a lower pin shows it (a pin that moved backwards between ticks), for the next plan only.
    stale_nonce: Option<u64>,
    finalized: u64,
    owners: HashMap<Word, [u8; 20]>,
    eth_signed: Vec<(Tx1559, [u8; 32])>,
    receipts: HashMap<[u8; 32], OurReceipt>,
    /// `eth_receipt` of these hashes fails once (a provider disagreement, a lagging index, a transient outcall error).
    receipt_err: Vec<[u8; 32]>,
    listing_total: u128,
    /// The next transfer plan fails with this text.
    transfer_err: Option<String>,
    sale_receipts: HashMap<[u8; 32], ReceiptV>,
    /// `eth_sale_receipt` of these hashes fails (over the outcall's size cap, providers disagree, …).
    sale_err: Vec<[u8; 32]>,
    deposit_txs: HashMap<[u8; 32], ([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>,
    caps: CapPolicy,
    book: Book,
    outflow: Vec<(String, Flow, u128)>,
    /// H3 refuses this flow (its daily limit is reached).
    refuse_flow: Option<Flow>,
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

impl Fake {
    fn new(two: bool) -> Self {
        let mut keys = vec![custodycap::evm_key(1, &MILADY)];
        if two {
            keys.push(custodycap::evm_key(1, &AZUKI));
        }
        let mut sale_receipts = HashMap::new();
        let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
        for (k, r) in v["receipts"].as_object().unwrap() {
            let h: [u8; 32] = hex::decode(&k[2..]).unwrap().try_into().unwrap();
            sale_receipts.insert(h, en::receipt_of(r).unwrap().unwrap());
        }
        let mut modules = HashMap::new();
        modules.insert(MODULE, Module::new());
        if two {
            let mut m = Module::new();
            m.accepted = 67 * E / 100;
            modules.insert(MODULE2, m);
        }
        Fake {
            now: NOW, paused: false, deposits_on: false, two, home_balance: 2_000_000_000_000_000, home_nonce: 0, modules, keeper: US,
            allocator: rc::RELAY_ALLOCATOR_4663, home_sent: HashMap::new(), home_calls: vec![], tickets: vec![], items: vec![], marked: vec![],
            eth_balance: 10_000_000_000_000_000, eth_nonce: 0, stale_nonce: None, finalized: ETH_PIN - 100, owners: HashMap::new(),
            eth_signed: vec![], receipts: HashMap::new(), receipt_err: vec![], listing_total: 45 * E / 100, transfer_err: None, sale_receipts,
            sale_err: vec![], deposit_txs: HashMap::new(), caps: custodycap::stage0(&keys), book: Book::default(), outflow: vec![],
            refuse_flow: None,
        }
    }

    fn m(&mut self, module: [u8; 20]) -> &mut Module {
        self.modules.get_mut(&module).unwrap()
    }

    fn cfg(&self) -> EthNftConfig {
        let col = |c: [u8; 20], w: [u8; 20], m: [u8; 20]| EthCollection { collection: format!("0x{}", hex::encode(c)),
            wrapper: format!("0x{}", hex::encode(w)), module: format!("0x{}", hex::encode(m)), max_purchase_wei: 2 * E, max_premium_bps: 500 };
        let mut collections = vec![col(MILADY, WRAPPER, MODULE)];
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
            if s == rc::ADVANCE {
                self.home_balance += arg(d, 0);
                self.m(to).transit = true;
            } else if s == rc::RETURN_ADVANCE {
                self.home_balance -= value;
                self.m(to).transit = false;
            } else if s == rc::SETTLE_ADVANCE {
                let m = self.m(to);
                m.transit = false;
                m.net_units -= E;
            } else if s == en::POST_FLOOR {
                let m = self.m(to);
                m.posted = Some((arg(d, 0), arg(d, 1) as u16, arg(d, 2) as u32));
                m.accepted = arg(d, 0);
                m.posted_at = now;
            } else {
                panic!("an unexpected module call");
            }
            return true;
        }
        match (to, sel(d)) {
            (WRAPPER | WRAPPER2, s) if s == en::MINT || s == en::MINT_DEPOSIT => self.items.push(d[4..36].try_into().unwrap()),
            (WRAPPER, s) if s == en::RECORD_DELIVERY => self.tickets[arg(d, 0) as usize].stage = Stage::Delivered,
            (WRAPPER, s) if s == en::MARK_UNDELIVERABLE => self.marked.push(arg(d, 0)),
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
                self.owners.insert(sp::word_u128(9_180), US);
            } else if tx.to == MILADY || tx.to == AZUKI {
                let dest: [u8; 20] = tx.data[4 + 32 + 12..4 + 64].try_into().unwrap();
                self.owners.insert(tx.data[4 + 64..4 + 96].try_into().unwrap(), dest);
            }
        }
        self.receipts.insert(hash, OurReceipt { block, success, fee });
    }

    fn plan_nonce(&mut self) -> u64 {
        self.stale_nonce.take().unwrap_or(self.eth_nonce)
    }
}

impl EthIo for Fake {
    // R2 adaptation: the trait gained home_finalized_time (H9 carried over); the home chain is taken as final at once
    async fn home_finalized_time(&mut self) -> Result<u64, String> { Ok(self.now + 86_400) }
    fn now(&self) -> u64 { self.now }
    fn home_chain(&self) -> u64 { 4663 }
    fn eth_chain(&self) -> u64 { 1 }
    fn config(&self) -> Option<EthNftConfig> { Some(self.cfg()) }
    fn paused(&self) -> bool { self.paused }
    fn eth_max_fee(&self) -> u128 { 50_000_000_000 }
    fn save(&mut self, _st: &EthState) {}
    fn with_caps<R>(&mut self, f: impl FnOnce(&CapPolicy, &mut Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _key: &str, _c: &str, flow: Flow, _amount: u128) -> Result<(), String> {
        if self.refuse_flow == Some(flow) {
            return Err(format!("outflow limit: {flow:?} over the day's global limit"));
        }
        Ok(())
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
                // R2 adaptation: the canister now reads floorBounds() before a post (D-H-2); no bound here
                else if s == ef::FLOOR_BOUNDS { [word(0), word(u128::MAX)].concat() }
                else { word(0) });
        }
        Ok(match to {
            WRAPPER => {
                if s == en::TICKET_COUNT { word(self.tickets.len() as u128) }
                else if s == en::TICKET {
                    let t = &self.tickets[arg(d, 0) as usize];
                    let st = match t.stage { Stage::None => 0, Stage::Pinned => 1, Stage::Resolved => 2, Stage::Delivered => 3, Stage::Refunded => 4 };
                    [word(st), addr_word(&ALICE), word(0), word(0), word(0), word(t.deliver_by as u128), word(0), word(t.item), word(0),
                        addr_word(&t.dest)].concat()
                } else if s == en::ITEM {
                    [self.items[arg(d, 0) as usize].to_vec(), word(1), word(0)].concat()
                } else if s == en::ITEM_OF {
                    let t: Word = d[4..36].try_into().unwrap();
                    word(self.items.iter().position(|x| *x == t).map_or(0, |i| i as u128 + 1))
                } else { word(0) }
            }
            WRAPPER2 => word(0),
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

    async fn eth_pin(&mut self) -> Result<EthPin, String> { Ok(EthPin { number: ETH_PIN, timestamp: self.now, base_fee: 100_000_000 }) }
    async fn eth_finalized(&mut self) -> Result<u64, String> { Ok(self.finalized) }
    async fn eth_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> {
        assert!(to == MILADY || to == AZUKI);
        assert_eq!(sel(d), sp::OWNER_OF);
        let t: Word = d[4..36].try_into().unwrap();
        Ok(addr_word(self.owners.get(&t).unwrap_or(&[0x0e; 20])))
    }
    async fn eth_balance(&mut self) -> Result<u128, String> { Ok(self.eth_balance) }
    async fn eth_nonce(&mut self) -> Result<u64, String> { Ok(self.eth_nonce) }
    async fn eth_receipt(&mut self, h: [u8; 32]) -> Result<Option<OurReceipt>, String> {
        if let Some(i) = self.receipt_err.iter().position(|x| *x == h) {
            self.receipt_err.remove(i);
            return Err("chain 1: providers disagree on eth_getTransactionReceipt; nothing is believed".into());
        }
        Ok(self.receipts.get(&h).copied())
    }
    async fn eth_plan_buy(&mut self, _order: &str, _collection: [u8; 20], cap: u128) -> Result<EthPlan, String> {
        if self.listing_total > cap {
            return Err(format!("refused (over cap): the order costs {} wei; the recorded cap is {cap}", self.listing_total));
        }
        let nonce = self.plan_nonce();
        Ok(EthPlan { to: sp::SEAPORT_1_6, value: self.listing_total, data: vec![0xfb, 0x0f, 0x3e, 0xe1], gas: 200_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce, token: sp::word_u128(9_180), total: self.listing_total })
    }
    async fn eth_plan_transfer(&mut self, collection: [u8; 20], token: Word, dest: [u8; 20]) -> Result<EthPlan, String> {
        if let Some(e) = self.transfer_err.take() {
            return Err(e);
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
    async fn eth_broadcast(&mut self, _raw: &[u8]) {}
    async fn eth_listing(&mut self, s: &str, _c: [u8; 20]) -> Result<Seen, String> {
        let (o, p) = s.split_once(':').ok_or("not a fake listing")?;
        Ok(Seen { offerer: [o.parse::<u8>().unwrap(); 20], token: sp::word_u128(o.parse().unwrap()), price: p.parse().unwrap() })
    }
    async fn eth_sale_receipt(&mut self, h: [u8; 32]) -> Result<Option<ReceiptV>, String> {
        if self.sale_err.contains(&h) {
            return Err("chain 1: eth_getTransactionReceipt: CallRejected(… Http body exceeds size limit of 200000 bytes)".into());
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
        println!("{l}");
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

/// Three honest listings (0.92, 0.93, 0.95 ETH from three offerers): the floor they say is 0.93.
fn listings() -> Vec<String> {
    vec![format!("1:{}", 92 * E / 100), format!("2:{}", 93 * E / 100), format!("3:{}", 95 * E / 100)]
}

/// A Seaport 1.6 `OrderFulfilled` in which `who` sells `token` of `collection` to `who` (itself) for `price` wei of native ETH:
/// a self-trade. It costs gas only: the ETH goes from the attacker's wallet to the attacker's wallet in one transaction.
fn wash_receipt(collection: [u8; 20], token: u128, price: u128, who: [u8; 20], block: u64) -> ReceiptV {
    let mut data = Vec::new();
    for w in [[0x77u8; 32].to_vec(), addr_word(&who), word(128), word(128 + 160), word(1), word(2), addr_word(&collection), word(token),
        word(1), word(1), word(0), word(0), word(0), word(price), addr_word(&who)] {
        data.extend(w);
    }
    ReceiptV { block, success: true, logs: vec![LogV { address: sp::SEAPORT_1_6,
        topics: vec![en::ORDER_FULFILLED, sp::word_addr(&who), sp::word_addr(&[0; 20])], data }] }
}

/// A plain transfer between two of the attacker's wallets: a "recent Transfer transaction" with no sale in it.
fn plain_transfer_receipt(collection: [u8; 20], token: u128, block: u64) -> ReceiptV {
    ReceiptV { block, success: true, logs: vec![LogV { address: collection,
        topics: vec![en::TRANSFER, sp::word_addr(&ATTACKER), sp::word_addr(&[0xef; 20]), sp::word_u128(token)], data: vec![] }] }
}

fn to_arrival(f: &mut Fake, st: &mut EthState) {
    tick(f, st);
    assert_eq!(phase(st), BPhase::Advancing);
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Funded { .. }));
    let id = st.buys.last().unwrap().id;
    ef::put_quote(st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(f, st);
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Bridging { .. }), "{:?}", phase(st));
    f.eth_balance += 498_443_962_449_863_321;
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Arrived { .. }), "{:?}", phase(st));
}

fn resolved(f: &mut Fake, token: u128, dest: [u8; 20], deliver_by: u64) {
    f.items.push(sp::word_u128(token));
    f.owners.insert(sp::word_u128(token), US);
    f.tickets.push(Ticket { stage: Stage::Resolved, item: (f.items.len() - 1) as u128, dest, deliver_by });
}

fn fill_custody(f: &mut Fake, collection: [u8; 20], n: u32) {
    let key = custodycap::evm_key(1, &collection);
    for i in 0..n {
        custodycap::admit(&f.caps, &mut f.book, &format!("{key}:{i}"), &key, E).unwrap();
    }
}

// ---- B-H-1: anyone steers the posted floor through sale evidence ----------------------------------------------------------------

#[test]
fn audit_nft_b_h1_the_sale_band_drives_the_post_away_from_every_real_listing() {
    let p = FloorParams::default();
    let seen: Vec<Seen> = [(1u8, 92 * E / 100), (2, 93 * E / 100), (3, 95 * E / 100)].iter()
        .map(|(o, x)| Seen { offerer: [*o; 20], token: sp::word_u128(*o as u128), price: *x }).collect();
    // three wash sales at 1.5 ETH: the post is 0.85 × 1.5 = 1.275 ETH, above every live listing, and the module takes it (5,000 ≥ 3,000)
    let up = en::floor_from(&seen, &r2_sales(3 * E / 2), &p).unwrap();
    assert_eq!(up.price, 1_275 * E / 1_000);
    assert!(up.price > 95 * E / 100 && up.confidence == 5_000);
    // three wash sales at 10 % of the listing floor (not "dust"): the post is 1.1 × 0.093 = 0.1023 ETH
    let down = en::floor_from(&seen, &r2_sales(93 * E / 1_000), &p).unwrap();
    assert_eq!(down.price, 1_023 * E / 10_000);
    assert_eq!(down.confidence, 5_000);
    println!("AUDIT-NFT-B: listings 0.92/0.93/0.95; wash sales at 1.5 → post {} wei; at 0.093 → post {} wei", up.price, down.price);
}

#[test]
fn audit_nft_b_h1_self_trades_handed_in_as_the_latest_transfer_txs_set_the_modules_floor() {
    for (price, want) in [(3 * E / 2, 1_275 * E / 1_000), (93 * E / 1_000, 1_023 * E / 10_000)] {
        let mut f = Fake::new(false);
        f.m(MODULE).net_units = 0;
        f.m(MODULE).posted_at = NOW - 2_000; // a post is due
        let mut st = EthState::default();
        // what the honest runner hands in: the collection's best listings, and the hashes of the 12 latest transactions that moved
        // a token of it (ops/src/ethnft.ts recentTransferTxs). The attacker made the latest 12: self-trades of one NFT at `price`.
        let mut hashes = Vec::new();
        for i in 0..12u8 {
            let h = [0x90 + i; 32];
            f.sale_receipts.insert(h, wash_receipt(MILADY, 4_242, price, ATTACKER, ETH_PIN - 20 + i as u64));
            hashes.push(h);
        }
        let cfg = f.cfg();
        ef::put_evidence(&mut st, &cfg, MILADY, listings(), hashes, f.now).unwrap();
        tick(&mut f, &mut st);
        let (posted, conf, _) = f.m(MODULE).posted.expect("posted");
        assert_eq!(posted, want, "the canister posts the attacker's number, not the listings' 0.93 ETH");
        assert_eq!(conf, 5_000);
        println!("AUDIT-NFT-B: wash sales at {price} → postFloor({posted}, {conf})");
    }
}

// ---- B-M-1: anyone freezes the floor (no post) through the same 12 slots --------------------------------------------------------

#[test]
fn audit_nft_b_m1_one_oversized_or_unreadable_sale_receipt_voids_the_whole_evidence_pass() {
    let mut f = Fake::new(false);
    f.m(MODULE).net_units = 0;
    f.m(MODULE).posted_at = NOW - 2_000;
    let mut st = EthState::default();
    let mut hashes = real_sales();
    let bad = [0x55; 32];
    f.sale_err.push(bad);
    hashes.insert(0, bad); // the attacker's receipt, padded past the 200 kB cap, is the newest transfer transaction
    let cfg = f.cfg();
    ef::put_evidence(&mut st, &cfg, MILADY, listings(), hashes, f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(f.m(MODULE).posted.is_none(), "five good sales and three good listings, and still no post");
    assert!(st.reading(&MILADY).is_none());
    assert!(l.iter().any(|x| x.contains("size limit")), "{l:?}");
    // and without any failure: 12 plain transfers crowd every sale out of the runner's 12 slots
    let mut f = Fake::new(false);
    f.m(MODULE).net_units = 0;
    f.m(MODULE).posted_at = NOW - 2_000;
    let mut st = EthState::default();
    let mut hashes = Vec::new();
    for i in 0..12u8 {
        let h = [0x60 + i; 32];
        f.sale_receipts.insert(h, plain_transfer_receipt(MILADY, 4_242, ETH_PIN - 20 + i as u64));
        hashes.push(h);
    }
    ef::put_evidence(&mut st, &cfg, MILADY, listings(), hashes, f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(f.m(MODULE).posted.is_none());
    assert!(l.iter().any(|x| x.contains("0 verified sales")), "{l:?}");
}

// ---- B-M-2: a transient outcall failure at the simulation marks a delivery undeliverable ------------------------------------------

#[test]
fn audit_nft_b_m2_a_transport_error_at_the_transfer_simulation_marks_the_ticket_undeliverable_a_week_early() {
    let mut f = Fake::new(false);
    f.m(MODULE).net_units = 0;
    let mut st = EthState::default();
    resolved(&mut f, 77, ALICE, NOW + 7 * 86_400);
    // exactly the text EthNet::fees produces when the eth_call outcall fails for ANY reason (ethnet.rs, `fees`): a 429 from one
    // provider, a consensus failure among replicas, a provider that disagrees
    f.transfer_err = Some("refused (simulation): it reverts at the pin: chain 1: eth_call: CallRejected(CallRejected { \
        raw_reject_code: 2, reject_message: \"SysTransient: HTTP 429 Too Many Requests\" })".into());
    tick(&mut f, &mut st);
    assert_eq!(f.marked, vec![0], "marked undeliverable at once, 7 days before its deadline");
    assert!(f.eth_signed.is_empty(), "nothing was ever tried again");
    assert!(matches!(st.deliveries[0].phase, DPhase::Marking { .. }));
}

// ---- B-M-3: a superseded line whose read-back errors once orphans the delivery (LostRace) --------------------------------------------

#[test]
fn audit_nft_b_m3_a_read_error_while_superseding_orphans_a_delivery_that_did_move_so_the_redeemer_keeps_nft_and_refund() {
    let mut f = Fake::new(false);
    f.m(MODULE).net_units = 0;
    let mut st = EthState::default();
    let deliver_by = NOW + 3 * 86_400;
    resolved(&mut f, 77, ALICE, deliver_by);
    tick(&mut f, &mut st);
    f.now = NOW + 2 * 86_400 + 1; // a day before its deadline, still unmined: cancelled
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }));
    // the transfer mines, not the cancel; at the tick that sees the nonce move, the transfer's receipt read fails once
    f.mine_eth(0, true, ETH_PIN - 3);
    let transfer = f.eth_signed[0].1;
    f.receipt_err.push(transfer);
    tick(&mut f, &mut st);
    assert!(st.line.is_none(), "the line was dropped before the read");
    // from here nothing ever looks at the delivery again
    f.finalized = ETH_PIN;
    for day in 0..40 {
        f.now = deliver_by + day * 86_400;
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }), "{:?}", st.deliveries[0].phase);
    assert_eq!(f.owners[&sp::word_u128(77)], ALICE, "the NFT is the redeemer's");
    assert_eq!(f.tickets[0].stage, Stage::Resolved, "never recorded: refundable once deliverBy + 30 days pass (LATE_DELIVERY_WINDOW)");
    assert!(f.marked.is_empty());
}

// ---- B-M-4: the Ethereum nonce read at a pin that moved backwards ------------------------------------------------------------------

#[test]
fn audit_nft_b_m4_a_nonce_read_at_a_lower_pin_strands_a_delivery_with_no_mark_and_no_retry() {
    let mut f = Fake::new(false);
    f.m(MODULE).net_units = 0;
    let mut st = EthState::default();
    resolved(&mut f, 77, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st); // delivery 0 signed at nonce 0
    f.mine_eth(0, true, ETH_PIN - 5); // mined in a block just under this pin
    tick(&mut f, &mut st); // seen, line free
    assert!(st.line.is_none());
    // a second ticket resolves; the next tick's pin is one bucket lower (a provider's head straddling a bucket), so the nonce read
    // there does not yet count the mined delivery
    resolved(&mut f, 78, ALICE, NOW + 7 * 86_400);
    f.stale_nonce = Some(0);
    tick(&mut f, &mut st);
    let DPhase::Sent { nonce, .. } = st.deliveries[1].phase.clone() else { panic!("{:?}", st.deliveries[1].phase) };
    assert_eq!(nonce, 0, "signed at a nonce already used");
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("not ours")), "{l:?}");
    // nothing re-signs it, cancels it or marks it, past its deadline and beyond
    for day in 1..12 {
        f.now = NOW + day * 86_400;
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.deliveries[1].phase, DPhase::Sent { .. }), "{:?}", st.deliveries[1].phase);
    assert!(f.marked.is_empty(), "never marked: the holder's refund waits for the 30-day late window");
    assert_eq!(f.eth_signed.len(), 2);
}

#[test]
fn audit_nft_b_m4_the_same_for_a_fill_strands_the_arrived_eth_and_every_later_buy() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    // first, one of our Ethereum transactions mines (a delivery), so nonce 0 is used
    f.m(MODULE).net_units = 0;
    resolved(&mut f, 77, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    f.m(MODULE).net_units = 2 * E;
    to_arrival(&mut f, &mut st);
    let cfg = f.cfg();
    ef::put_evidence(&mut st, &cfg, MILADY, listings(), real_sales(), f.now).unwrap();
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    f.stale_nonce = Some(0);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }));
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Stuck { .. }), "{:?}", phase(&st));
    assert!(st.buys[0].last.is_none());
    assert!(ef::give_back_now(&mut st.clone(), 0, 10 * E).is_err(), "no exit for the operator");
    for _ in 0..10 {
        tick(&mut f, &mut st);
    }
    assert_eq!(st.buys.len(), 1, "the module still wants items, and no buy ever starts again (one open buy canister-wide)");
    assert!(f.book.contains("eth:0"), "and its custody reservation is never settled");
}

// ---- B-M-5: cross-collection: one capped collection blocks every other collection's buys ------------------------------------------

#[test]
fn audit_nft_b_m5_a_collection_at_its_custody_cap_that_still_wants_blocks_every_other_collections_buys() {
    let mut f = Fake::new(true);
    fill_custody(&mut f, MILADY, 3); // Milady holds its stage-0 three; its module still wants a fourth
    let mut st = EthState::default();
    for _ in 0..20 {
        let l = tick(&mut f, &mut st);
        assert!(l.iter().any(|x| x.contains("custody cap")), "{l:?}");
    }
    assert!(st.buys.is_empty(), "Azuki's module wants two items and has room: none is ever started");
    // control: once Milady's module stops wanting, Azuki's buy starts at once
    f.m(MODULE).net_units = 0;
    tick(&mut f, &mut st);
    assert_eq!(st.buys.len(), 1);
    assert_eq!(st.buys[0].collection, AZUKI);
}

#[test]
fn audit_nft_b_m5_one_arrived_buy_that_cannot_fill_blocks_every_collection_and_has_no_exit() {
    let mut f = Fake::new(true);
    f.m(MODULE2).net_units = 0;
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    let cfg = f.cfg();
    // the market rose while the ETH was on its way: the cheapest listing is now over what arrived less the gas reserve
    f.listing_total = 496 * E / 1_000;
    f.m(MODULE2).net_units = 2 * E; // Azuki graduates and wants two items
    for _ in 0..30 {
        ef::put_evidence(&mut st, &cfg, MILADY, listings(), real_sales(), f.now).unwrap();
        ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
        tick(&mut f, &mut st);
        f.now += 600;
    }
    assert_eq!(st.buys.len(), 1, "no Azuki buy in five hours");
    assert!(matches!(phase(&st), BPhase::Arrived { .. }));
    assert!(ef::give_back_now(&mut st.clone(), 0, 10 * E).is_err(), "an Arrived buy cannot be given back (D4)");
}

// ---- B-H-2 (deposits ON): a deposit into a capped collection stops every collection's floor posts ------------------------------------

fn deposit_into(f: &mut Fake, collection: [u8; 20], token: u128, tx: [u8; 32]) {
    let holder = [0x33; 20];
    let recipient = [0x44; 20];
    let data = [&en::DEPOSIT_MAGIC[..], &addr_word(&recipient)].concat();
    let mut padded = data.clone();
    padded.resize(64, 0);
    let input = [&en::SAFE_TRANSFER_FROM_DATA[..], &addr_word(&holder), &addr_word(&US), &word(token), &word(0x80),
        &word(data.len() as u128), &padded].concat();
    let r = ReceiptV { block: ETH_PIN - 150, success: true, logs: vec![LogV { address: collection,
        topics: vec![en::TRANSFER, sp::word_addr(&holder), sp::word_addr(&US), sp::word_u128(token)], data: vec![] }] };
    f.deposit_txs.insert(tx, (holder, collection, input, r));
    f.owners.insert(sp::word_u128(token), US);
}

#[test]
fn audit_nft_b_h2_one_deposit_into_a_capped_collection_stops_the_floor_posts_of_every_collection() {
    for attack in [false, true] {
        let mut f = Fake::new(true);
        f.deposits_on = true;
        for m in [MODULE, MODULE2] {
            f.m(m).net_units = 0;
        }
        f.m(MODULE).posted_at = NOW - 2_000; // Milady's post is due
        let mut st = EthState::default();
        if attack {
            // Azuki is at its stage-0 cap; anyone deposits one more Azuki
            fill_custody(&mut f, AZUKI, 3);
            let tx = [0x42; 32];
            deposit_into(&mut f, AZUKI, 555, tx);
            ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
            tick(&mut f, &mut st);
            assert!(matches!(st.deposits[0].phase, XPhase::Mint), "{:?}", st.deposits[0].phase);
        }
        let cfg = f.cfg();
        let mut lines = Vec::new();
        for _ in 0..12 {
            ef::put_evidence(&mut st, &cfg, MILADY, listings(), real_sales(), f.now).unwrap();
            lines.extend(tick(&mut f, &mut st));
            f.now += 300;
        }
        let posted = f.m(MODULE).posted;
        if attack {
            assert!(posted.is_none(), "an hour of due readings and Milady is never posted: its module goes stale");
            assert!(lines.iter().any(|x| x.contains("custody cap")), "{lines:?}");
        } else {
            assert!(posted.is_some(), "control: without the deposit the post goes out");
        }
    }
}

#[test]
fn audit_nft_b_h2_the_days_global_mint_limit_reached_by_one_collection_stops_the_other_collections_floor_posts() {
    let mut f = Fake::new(true);
    f.m(MODULE2).net_units = 0;
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    let cfg = f.cfg();
    ef::put_evidence(&mut st, &cfg, MILADY, listings(), real_sales(), f.now).unwrap();
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    f.mine_eth(0, true, ETH_PIN - 10);
    tick(&mut f, &mut st);
    // H3's global mint limit (6 a day at stage 0, shared by every collection and by holder deposits) is reached
    f.refuse_flow = Some(Flow::Mint);
    f.finalized = ETH_PIN;
    // Azuki's evidence (honest sales at 0.67) arrives every 20 minutes and its post is due
    let mut hashes = Vec::new();
    for i in 0..5u8 {
        let h = [0xa0 + i; 32];
        f.sale_receipts.insert(h, wash_receipt(AZUKI, 100 + i as u128, 67 * E / 100, [0x10 + i; 20], ETH_PIN - 50));
        hashes.push(h);
    }
    f.m(MODULE2).posted_at = f.now - 2_000;
    let az = vec![format!("1:{}", 66 * E / 100), format!("2:{}", 67 * E / 100), format!("3:{}", 68 * E / 100)];
    for _ in 0..9 {
        ef::put_evidence(&mut st, &cfg, AZUKI, az.clone(), hashes.clone(), f.now).unwrap();
        let l = tick(&mut f, &mut st);
        assert!(l.iter().any(|x| x.contains("outflow limit")), "{l:?}");
        f.now += 400;
    }
    assert!(matches!(phase(&st), BPhase::Final { .. }));
    assert!(f.m(MODULE2).posted.is_none(), "an hour: Azuki's module is never posted and is stale (FLOOR_TTL 1 h)");
    // control: the limit window passes; the mint goes, and Azuki's post follows
    f.refuse_flow = None;
    for _ in 0..4 {
        ef::put_evidence(&mut st, &cfg, AZUKI, az.clone(), hashes.clone(), f.now).unwrap();
        tick(&mut f, &mut st);
    }
    assert!(f.m(MODULE2).posted.is_some());
}

// R2 adaptation: floor_from now takes Sales; the strongest wash for the attacker (distinct token, seller and buyer each)
fn r2_sales(price: u128) -> Vec<en::Sale> {
    (1..=3u8).map(|i| en::Sale { token: sp::word_u128(100 + i as u128), price, seller: [0x10 + i; 20], buyer: [0x20 + i; 20], block: 1 }).collect()
}
