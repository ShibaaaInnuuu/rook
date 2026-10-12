//! Round 3 of the 2026-09-27 NFT review, part E (arrival accounting, the deposits hold, concurrency, upgrade safety): PoCs on the
//! fake world of `tests/audit_nft_r2.rs` (copied whole), with two switches added to the fake: `pin_back` (the Ethereum pin moved
//! back one bucket: the balance there is the one before a spend of ours that mined above it, and receipts above it are not seen) and
//! `fee_of` (a mined transaction's gas, instead of the fake's fixed 21,000 gwei). Each test PASSES while the weakness is present.
#![allow(dead_code)] // the harness is tests/audit_nft.rs's, copied whole

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
/// Round 2 (A2-M-1, `ethflow::advance_covers`): the modules' floor before the tests post, one their item cap (the Relay fixture's
/// 0.5 ETH plus the 4663 gas keep) covers on Ethereum with the margin; a post then moves it to what the evidence says.
const FLOOR0: u128 = 47 * E / 100;
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
    /// the configuration no longer names Milady (an upgrade dropped it)
    drop_milady: bool,
    /// E3: the pin moved back below a spend of ours: (the lower pin, what that spend took, the block it mined in)
    pin_back: Option<(u64, u128)>,
    /// E3: the next mined transaction's gas (else 21,000 gwei)
    fee_of: Option<u128>,
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
        modules.insert(MODULE, Module::new(FLOOR0));
        let mut keys = vec![custodycap::evm_key(1, &MILADY)];
        if two {
            modules.insert(MODULE2, Module::new(FLOOR0));
            keys.push(custodycap::evm_key(1, &AZUKI));
        }
        Fake {
            now: NOW, paused: false, deposits_on: false, two, home_balance: 2_000_000_000_000_000, home_nonce: 0, modules, keeper: US,
            allocator: rc::RELAY_ALLOCATOR_4663, home_sent: HashMap::new(), home_calls: vec![], tickets: vec![], items: vec![], tickets2: vec![],
            items2: vec![], marked: vec![], marked2: vec![], home_final_lag: 0, eth_balance: 10_000_000_000_000_000, eth_nonce: 0,
            stale_nonce: None, finalized: ETH_PIN - 100, owners: HashMap::new(), eth_signed: vec![], receipts: HashMap::new(),
            receipt_err: HashMap::new(), plan_err: vec![], wait_prefix: vec![], broadcasts: vec![], listing_total: 45 * E / 100, sale_receipts,
            sale_err: vec![], sale_reads: 0, deposit_txs: HashMap::new(), caps: custodycap::stage0(&keys), book: Book::default(), outflow: vec![], drop_milady: false, pin_back: None, fee_of: None,
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
        let fee = self.fee_of.take().unwrap_or(21_000 * 1_000_000_000);
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

    async fn eth_pin(&mut self) -> Result<EthPin, String> { Ok(EthPin { number: self.pin_back.map_or(ETH_PIN, |p| p.0), timestamp: self.now, base_fee: 100_000_000 }) }
    async fn eth_finalized(&mut self) -> Result<u64, String> { Ok(self.finalized) }
    async fn eth_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> {
        if to == rc::RELAY_DEPOSITORY_4663 && sel(d) == rc::ALLOCATOR {
            return Ok(addr_word(&rc::RELAY_ALLOCATOR_4663)); // the depository and its allocator are the same on Ethereum
        }
        assert!(to == MILADY || to == AZUKI);
        assert_eq!(sel(d), sp::OWNER_OF);
        let t: Word = d[4..36].try_into().unwrap();
        Ok(addr_word(self.owners.get(&(to, t)).unwrap_or(&[0x0e; 20])))
    }
    async fn eth_balance(&mut self) -> Result<u128, String> { Ok(self.eth_balance + self.pin_back.map_or(0, |p| p.1)) }
    async fn eth_nonce(&mut self) -> Result<u64, String> { Ok(self.eth_nonce.saturating_sub(self.pin_back.map_or(0, |_| 1))) }
    async fn eth_receipt(&mut self, h: [u8; 32]) -> Result<Option<OurReceipt>, String> {
        if let Some(n) = self.receipt_err.get_mut(&h) {
            if *n > 0 {
                *n -= 1;
                return Err("chain 1: eth_getTransactionReceipt: HTTP 429 Too Many Requests".into());
            }
        }
        let pin = self.pin_back.map_or(ETH_PIN, |p| p.0);
        Ok(self.receipts.get(&h).copied().filter(|r| r.block <= pin))
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
    /// The chain's Transfer logs: every receipt the fake knows that moves a token of `collection` between two wallets (round 2).
    async fn eth_transfer_txs(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<([u8; 32], u64)>, String> {
        let mut v: Vec<([u8; 32], u64)> = self.sale_receipts.iter().filter(|(_, r)| r.block >= from && r.block <= to && r.logs.iter().any(|l|
            l.address == collection && l.topics.len() == 4 && l.topics[0] == en::TRANSFER && l.topics[1] != l.topics[2] && l.topics[1] != [0; 32]))
            .map(|(h, r)| (*h, r.block)).collect();
        v.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        Ok(v)
    }
    async fn eth_plan_call(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<EthPlan, String> {
        let nonce = self.plan_nonce();
        Ok(EthPlan { to, value, data: data.to_vec(), gas: 60_000, max_fee: 1_000_000_000, priority_fee: 100_000_000, nonce, token: [0; 32], total: value })
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

// =====================================================================================================================================
// Fix round 2: the round-2 PoCs turned around
// =====================================================================================================================================

const BACK: &str = include_str!("fixtures/relay-quote-1-4663-2026-09-27.json");
const OTHER: [u8; 20] = [0xef; 20];
const POT: u128 = 10_000_000_000_000_000; // the fake's starting Ethereum balance (the D3 gas pot)
/// A final block of the sale window (the canister reads its own sales at `SALE_FINAL_DEPTH_BLOCKS` under the pin).
const FINAL: u64 = ETH_PIN - 500;

fn buy_of(st: &EthState, c: [u8; 20]) -> Option<ef::EthBuy> {
    st.buys.iter().rev().find(|b| b.collection == c).cloned()
}

/// `n` wash sales on the chain, newest first, `price` each, at final blocks. `ring`: distinct token, seller and buyer per sale along
/// a chain of wallets (`w_i → w_{i+1}`); `pairs`: a pair of wallets and a token of their own per sale (the strongest wash: nothing
/// links two of them); otherwise ONE token ping-ponged between two wallets (ATTACKER ↔ OTHER).
fn washes(f: &mut Fake, first: u8, n: u8, price: u128, shape: &str) -> Vec<[u8; 32]> {
    let mut hs = Vec::new();
    for i in 0..n {
        let h = [first + i; 32];
        let (tok_id, s, b) = match shape {
            "ring" => (7_000 + i as u128, [0x50 + i; 20], [0x51 + i; 20]),
            "pairs" => (8_000 + i as u128, [0x80 + 2 * i; 20], [0x81 + 2 * i; 20]),
            _ => if i % 2 == 0 { (4_242, ATTACKER, OTHER) } else { (4_242, OTHER, ATTACKER) },
        };
        f.sale_receipts.insert(h, sale_receipt(MILADY, tok_id, price, s, b, FINAL - i as u64));
        hs.push(h);
    }
    hs
}

/// A Relay quote Ethereum → 4663 for `amount`, to our own address (the measured fixture, re-addressed and re-hashed: every rule of
/// `relayeth::judge` holds on it).
fn back_quote(amount: u128, now: u64) -> Vec<u8> {
    let us = hex::encode(US);
    let mut v: serde_json::Value = serde_json::from_str(&BACK.replace("2427857b22f64a7b9cb22148928f1e410c7aa108", &us)).unwrap();
    let a = amount.to_string();
    v["steps"][0]["items"][0]["data"]["value"] = a.clone().into();
    {
        let o = &mut v["protocol"]["v2"]["orderData"];
        o["inputs"][0]["payment"]["amount"] = a.clone().into();
        for r in o["inputs"][0]["refunds"].as_array_mut().unwrap() {
            r["deadline"] = (now + 7 * 86_400).into();
        }
        o["output"]["deadline"] = (now + 7 * 86_400).into();
        o["output"]["payments"][0]["minimumAmount"] = (amount / 100 * 98).to_string().into();
    }
    v["protocol"]["v2"]["paymentDetails"]["amount"] = a.into();
    let id = rc::order_id(&v["protocol"]["v2"]["orderData"]).unwrap();
    v["steps"][0]["items"][0]["data"]["data"] = format!("0x49290c1c{}{us}{}", "0".repeat(24), hex::encode(id)).into();
    v["protocol"]["v2"]["orderId"] = format!("0x{}", hex::encode(id)).into();
    serde_json::to_vec(&v).unwrap()
}

// =====================================================================================================================================
// Round 3, part E
// =====================================================================================================================================

/// The Ethereum chain's pin bucket (`chains.rs`, 8 blocks): `choose_pin` is the lowest provider head rounded down, less one bucket,
/// so a provider head that falls back across a bucket boundary moves the pin back 8 blocks.
const BUCKET: u64 = 8;

/// Milady arrives and its fill (at the buy's cap) is signed; meanwhile Azuki's buy starts. The fill mines 3 blocks under the pin and
/// is READ and COUNTED (eth_out) at the pin; Azuki is then Funded. Returns what the fill took from our Ethereum balance.
fn milady_fill_counted_azuki_funded(f: &mut Fake, st: &mut EthState) -> u128 {
    to_arrival(f, st);
    f.m(MODULE).net_units = 0;
    f.m(MODULE).accepted = 93 * E / 100;
    evidence(f, st);
    f.listing_total = ARRIVED - 3_000_000_000_000_000;
    ef::put_listing(st, 0, fill_hex(), f.now).unwrap();
    f.m(MODULE2).net_units = 2 * E;
    tick(f, st);
    assert!(matches!(buy_of(st, MILADY).unwrap().phase, BPhase::Buying { .. }));
    let i = f.eth_signed.len() - 1;
    let before = f.eth_balance;
    f.mine_eth(i, true, ETH_PIN - 3);
    let spent = before - f.eth_balance;
    let out0 = st.eth_out;
    tick(f, st);
    assert!(matches!(buy_of(st, MILADY).unwrap().phase, BPhase::Bought { .. }), "the fill is read and counted at the pin");
    assert_eq!(st.eth_out - out0, spent, "eth_out counts the fill");
    assert_eq!(st.last_mined.map(|x| x.1), Some(ETH_PIN - 3));
    assert!(matches!(buy_of(st, AZUKI).unwrap().phase, BPhase::Funded { .. }), "{:?}", buy_of(st, AZUKI).unwrap().phase);
    spent
}

// ---- E3-M-1 (a): a pin that moves back makes a COUNTED spend read as another buy's arrival ----------------------------------------------

#[test]
fn e3_m1_a_pin_that_moves_back_reads_a_counted_fill_as_another_collections_arrival() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let spent = milady_fill_counted_azuki_funded(&mut f, &mut st);
    // Azuki's quote is judged at the pin, every spend counted (unsettled is false): its watch is right
    let az = buy_of(&st, AZUKI).unwrap();
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Bridging { .. }), "{:?}", buy_of(&st, AZUKI).unwrap().phase);
    let real = f.eth_balance;
    // one provider's head falls back across a bucket boundary: the pin is one bucket lower, under the fill's block. There the
    // balance is the one before the fill; eth_out still counts it. No Relay payout reached Ethereum.
    f.pin_back = Some((ETH_PIN - BUCKET, spent));
    let l = tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    println!("AUDIT3-E E3-M-1a: Azuki {:?}; our real Ethereum balance {real}; Milady's fill took {spent}", az.phase);
    let BPhase::Arrived { wei, .. } = az.phase else { panic!("no phantom: {:?} {l:?}", az.phase) };
    assert_eq!(wei, spent, "Azuki's 'arrival' is Milady's spent fill");
    assert!(l.iter().any(|x| x.contains("below the block")), "eth_act refuses to SIGN at the lower pin (B-M-4) but the watch was read there: {l:?}");
    // the pin comes back: the earmark is ETH that is not there
    f.pin_back = None;
    tick(&mut f, &mut st);
    println!("AUDIT3-E E3-M-1a: earmarked {} wei; balance {} wei", st.earmarked_except(u64::MAX), f.eth_balance);
    assert!(st.earmarked_except(u64::MAX) > f.eth_balance, "the earmarks exceed the balance (the C2-M-1 invariant broken)");
    // Relay then refunds Azuki's order on 4663: nothing notices it (Azuki is no longer Bridging); the refund sits at our 4663 address
    let BPhase::Arrived { .. } = buy_of(&st, AZUKI).unwrap().phase else { panic!() };
    f.home_balance += 500_000_000_000_000_000;
    for _ in 0..3 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Arrived { .. }), "{:?}", buy_of(&st, AZUKI).unwrap().phase);
    // and 48 h on, its way back needs ETH that is not there: it waits for ever, and while it is BackFunded no buy starts anywhere
    f.now += ef::ARRIVED_BACK_SECS;
    tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    assert!(matches!(az.phase, BPhase::BackFunded { .. }), "{:?}", az.phase);
    let amount = az.ask.clone().unwrap().amount_wei;
    ef::put_quote(&mut st, az.id, back_quote(amount, f.now), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    println!("AUDIT3-E E3-M-1a: the way back: {:?}", l.iter().filter(|x| x.contains("way back")).collect::<Vec<_>>());
    assert!(l.iter().any(|x| x.contains("the way back needs")), "{l:?}");
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::BackFunded { .. }));
}

// ---- E3-M-1 (b): a watch taken at a pin that moved back never sees the real arrival: the one Relay leg is held for ever ---------------

#[test]
fn e3_m1_a_watch_taken_at_a_pin_that_moved_back_never_sees_the_real_arrival_and_holds_the_relay_leg() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let spent = milady_fill_counted_azuki_funded(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    // the quote is judged in a tick whose pin is one bucket back: unsettled is false (the line is empty), so the watch is taken there:
    // eth_before is the balance BEFORE Milady's fill, eth_out_before already counts it
    f.pin_back = Some((ETH_PIN - BUCKET, spent));
    tick(&mut f, &mut st);
    let w = buy_of(&st, AZUKI).unwrap().last.clone().expect("watched");
    println!("AUDIT3-E E3-M-1b: watch eth_before {} (real balance {}), eth_out_before {} (eth_out {})", w.eth_before, f.eth_balance, w.eth_out_before, st.eth_out);
    assert_eq!(w.eth_before, f.eth_balance + spent);
    f.pin_back = None;
    tick(&mut f, &mut st);
    let BPhase::Bridging { deadline, .. } = buy_of(&st, AZUKI).unwrap().phase else { panic!("{:?}", buy_of(&st, AZUKI).unwrap().phase) };
    // Relay's fill lands on Ethereum: the whole arrival
    f.eth_balance += ARRIVED;
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Bridging { .. }), "the real arrival is not seen");
    // past the deadline: Stuck, still watched (on the Relay leg), and the watch never reads the arrival
    f.now = deadline + ef::DEADLINE_GRACE_SECS + 60;
    for _ in 0..6 {
        f.now += 3_600;
        f.m(MODULE).posted_at = f.now - 60;
        tick(&mut f, &mut st);
    }
    let az = buy_of(&st, AZUKI).unwrap();
    println!("AUDIT3-E E3-M-1b: Azuki {:?}, watched {}, relay_busy {}, earmarked {}, balance {} ({} arrived, nobody's)", az.phase,
        az.last.is_some(), st.relay_busy(), st.earmarked_except(u64::MAX), f.eth_balance, ARRIVED);
    assert!(matches!(az.phase, BPhase::Stuck { .. }) && az.last.is_some() && st.relay_busy(), "the one Relay leg is held for ever");
    assert_eq!(st.earmarked_except(u64::MAX), 0, "the arrived ETH is nobody's");
    // the operator's exit for a watched Stuck buy returns 4663 ETH (what the home balance holds), not the ETH that arrived on Ethereum
    let mut st2 = st.clone();
    let r = ef::give_back_now(&mut st2, az.id, f.home_balance);
    println!("AUDIT3-E E3-M-1b: eth_give_back: {r:?} (home balance {})", f.home_balance);
    assert!(r.as_ref().is_err() || matches!(buy_of(&st2, AZUKI).unwrap().phase, BPhase::Returning { wei } if wei < ARRIVED / 10));
}

// ---- E3-L-1: a fill its seller makes revert restarts the Arrived hold and the 48 h clock: deposits held without bound ----------------

#[test]
fn e3_l1_a_fill_that_reverts_restarts_the_arrived_hold_so_a_seller_can_hold_deposits_and_defer_the_way_back_indefinitely() {
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    let arrived_at = f.now;
    f.m(MODULE).net_units = 0;
    f.m(MODULE).accepted = 93 * E / 100;
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 20);
    ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
    f.finalized = ETH_PIN;
    let (mut reverts, mut waited) = (0, 0);
    // every 5 h (under ARRIVED_HOLD_SECS) the runner hands in the cheapest valid listing: the seller's; the canister signs the fill;
    // the seller cancels the order (or moves the token) before it mines: the fill reverts, and the buy is Arrived again, `since` now
    for step in 0..(5 * 24 * 3) {
        f.now += 1_200;
        f.m(MODULE).posted_at = f.now - 60;
        evidence(&f, &mut st);
        if step % 15 == 0 && matches!(phase(&st), BPhase::Arrived { .. }) {
            ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
        }
        for l in tick(&mut f, &mut st) {
            if l.contains("holds the day's mint room") {
                waited += 1;
            }
        }
        if let BPhase::Buying { .. } = phase(&st) {
            let i = f.eth_signed.len() - 1;
            f.mine_eth(i, false, ETH_PIN - 1);
            reverts += 1;
        }
    }
    let BPhase::Arrived { wei, since } = phase(&st) else { panic!("{:?}", phase(&st)) };
    println!("AUDIT3-E E3-L-1: after 5 days: {reverts} reverted fills; the deposit {:?} (waited {waited} ticks); the buy Arrived {{ wei {wei}, \
        since +{} s }}; never went back", st.deposits[0].phase, since - arrived_at);
    assert_eq!(st.deposits[0].phase, XPhase::Mint, "the holder's deposit is still unminted after 5 days");
    assert!(f.items.is_empty());
    assert!(f.now - arrived_at > ef::ARRIVED_BACK_SECS * 2 && !st.buys.iter().any(|b| matches!(b.phase, BPhase::BackFunded { .. }
        | BPhase::BackSending { .. } | BPhase::BackBridging { .. })), "the 48 h way back never started");
    assert!(reverts >= 20);
}

// ---- E3-L-2: C2-I-1's drift + the round-2 arrival cap: an arrival waits on an earmark that waits on the Relay leg (deadlock) ----------

#[test]
fn e3_l2_gas_drift_into_an_earmark_deadlocks_the_next_arrival_with_the_way_back() {
    let mut f = Fake::new(true);
    f.eth_balance = 1_000_000_000_000_000; // a gas pot of 0.001 ETH
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    f.m(MODULE).net_units = 0;
    f.m(MODULE).accepted = 93 * E / 100;
    f.listing_total = 2 * E; // Milady cannot fill (the floor rose: A2-M-1's case), it waits, then goes back
    // a holder's delivery: signed on the shared balance (deliveries check only the total, C2-I-1); its gas (60k at ~66 gwei) is
    // more than the pot
    let by = f.now + 7 * 86_400 - 60;
    resolved(&mut f, false, 77, ALICE, by);
    tick(&mut f, &mut st);
    assert!(st.line.as_ref().is_some_and(|l| matches!(l.kind, ef::EthKind::Deliver { .. })), "the delivery is signed");
    f.fee_of = Some(4_000_000_000_000_000);
    let i = f.eth_signed.len() - 1;
    f.mine_eth(i, true, ETH_PIN - 1);
    f.m(MODULE2).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).expect("Azuki's buy");
    assert!(matches!(az.phase, BPhase::Funded { .. }), "{:?}", az.phase);
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Bridging { .. }));
    // Relay pays Azuki's whole arrival on Ethereum
    f.eth_balance += ARRIVED;
    let mut refusals = 0;
    for _ in 0..(10 * 24 * 3) {
        f.now += 1_200;
        f.m(MODULE).posted_at = f.now - 60;
        f.m(MODULE2).posted_at = f.now - 60;
        if let Some(b) = buy_of(&st, MILADY).filter(|b| matches!(b.phase, BPhase::BackFunded { .. }) && b.ask.is_some()) {
            if !st.inbox.quotes.iter().any(|(id, ..)| *id == b.id) {
                ef::put_quote(&mut st, b.id, back_quote(b.ask.clone().unwrap().amount_wei, f.now), f.now).unwrap();
            }
        }
        for l in tick(&mut f, &mut st) {
            if l.contains("nothing is earmarked on ETH that is not there") {
                refusals += 1;
            }
        }
    }
    let (m, a) = (buy_of(&st, MILADY).unwrap(), buy_of(&st, AZUKI).unwrap());
    println!("AUDIT3-E E3-L-2: after 10 days: Milady {:?} (note: {}); Azuki {:?} (watched {}); {refusals} arrival refusals; balance {} vs \
        earmarks {}; relay_busy {}", m.phase, m.note, a.phase, a.last.is_some(), f.eth_balance, st.earmarked_except(u64::MAX), st.relay_busy());
    assert!(matches!(m.phase, BPhase::BackFunded { .. }) && m.note.contains("another buy holds the Relay leg"), "Milady waits for the leg");
    assert!(!matches!(a.phase, BPhase::Arrived { .. }) && a.last.is_some(), "Azuki waits for room only Milady's leaving frees");
    assert!(st.relay_busy() && refusals > 100);
    assert!(f.eth_balance > ARRIVED, "both buys' ETH is at our Ethereum address; neither moves");
    // the refused arrival returns before the deadline check: Azuki stays Bridging past its deadline (never even Stuck), and a Bridging
    // buy holds its collection's deposit mints: Azuki's deposits wait too, without bound
    assert!(matches!(a.phase, BPhase::Bridging { deadline, .. } if f.now > deadline + ef::DEADLINE_GRACE_SECS), "{:?}", a.phase);
    assert!(st.buy_holds_mint(&AZUKI, f.now));
    // the only way out: someone sends ETH to our Ethereum address (the operator topping up the pot); then Azuki arrives, Milady goes
    f.eth_balance += 10_000_000_000_000_000;
    tick(&mut f, &mut st);
    let a = buy_of(&st, AZUKI).unwrap();
    println!("AUDIT3-E E3-L-2: after a 0.01 ETH top-up: Azuki {:?}", a.phase);
    assert!(matches!(a.phase, BPhase::Arrived { .. }), "control: the deadlock is the earmark cap, nothing else");
}

// ---- E3-L-3: a Stuck buy with no watch never finishes and has no exit: its collection never buys again, its module stays in transit,
// and no upgrade may drop or re-point the collection --------------------------------------------------------------------------------

#[test]
fn e3_l3_a_post_arrival_stuck_buy_is_open_for_ever_with_no_exit() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    let i = f.eth_signed.len() - 1;
    f.mine_eth(i, true, ETH_PIN - 10);
    f.owners.insert((MILADY, tok(9_180)), [0x0e; 20]); // the fill mined, the token is not ours (C2-L-1's case)
    tick(&mut f, &mut st);
    let m = buy_of(&st, MILADY).unwrap();
    assert!(matches!(m.phase, BPhase::Stuck { .. }) && m.last.is_none());
    // Milady's module wants another item; days pass
    f.m(MODULE).net_units = 2 * E;
    for _ in 0..(3 * 24) {
        f.now += 3_600;
        f.m(MODULE).posted_at = f.now - 60;
        tick(&mut f, &mut st);
    }
    let milady_buys = st.buys.iter().filter(|b| b.collection == MILADY).count();
    let give = ef::give_back_now(&mut st.clone(), m.id, f.home_balance);
    let mut g = Fake::new(true);
    g.drop_milady = true;
    let stranded = ef::stranded_by(&st, Some(&g.cfg()));
    println!("AUDIT3-E E3-L-3: after 3 days: Milady buys {milady_buys} (the Stuck one {:?}); module in transit {}; eth_give_back: {give:?}; \
        an upgrade dropping Milady: {stranded:?}", buy_of(&st, MILADY).unwrap().phase, f.m(MODULE).transit);
    assert_eq!(milady_buys, 1, "no new Milady buy ever starts (its Stuck buy is its open buy)");
    assert!(f.m(MODULE).transit, "the module's advance is never settled or returned");
    assert!(give.is_err(), "no operator exit");
    assert!(!stranded.is_empty(), "and no upgrade of this build may drop or re-point Milady");
}
