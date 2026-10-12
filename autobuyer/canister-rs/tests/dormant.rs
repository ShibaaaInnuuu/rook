//! Dormant, wake-on-demand collections (docs/v7/dormant-collections.md) against a fake world: a 4663 with a `WakeDesk`, each
//! collection's wrapper, Floor Module, factory and curves; an Ethereum whose sale receipts are the real Milady ones. What is checked:
//! a DORMANT collection costs no read; the desk, the operator, a deposit and the watchdog wake one under every cap; no post, buy or
//! deposit mint before the 7-day backfill is read and the reading confirmed; ACTIVE on the first mined post; the timeout; the slow
//! cadence and its speed-up near a buy; the fresh read before a fill; the move back to DORMANT and each thing that holds it; the fill
//! of a graduation queued at a stale module; the lives decided at an upgrade; the status the site shows.

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custodycap::{self, Book, CapPolicy};
use etch_autobuyer::dormancy::{self as dm, Dormancy, Kind};
use etch_autobuyer::ethflow::{self as ef, BPhase, EthIo, EthPin, EthPlan, EthState, OurReceipt, XPhase};
use etch_autobuyer::ethnft::{self as en, EthCollection, EthNftConfig, ReceiptV, Seen};
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
const T: u128 = 1_000_000_000_000;
const SALES: &str = include_str!("fixtures/seaport/milady-sales-receipts.json");
const FILL: &str = include_str!("fixtures/seaport/eth-milady-fill-26055691.json");

fn fill_hex() -> String {
    let v: serde_json::Value = serde_json::from_str(FILL).unwrap();
    v["input"].as_str().unwrap().to_string()
}
const US: [u8; 20] = [0x76, 0x02, 0x0b, 0x70, 0x93, 0xbd, 0x35, 0xd2, 0x52, 0x0d, 0x47, 0xde, 0xd7, 0x64, 0xef, 0xc5, 0x54, 0x23, 0x7e, 0xfe];
const NOW: u64 = 1_791_117_628 - 7 * 86_400 + 5;
const ETH_PIN: u64 = 26_068_928;
const MILADY: [u8; 20] = [0x5a, 0xf0, 0xd9, 0x82, 0x7e, 0x0c, 0x53, 0xe4, 0x79, 0x9b, 0xb2, 0x26, 0x65, 0x5a, 0x1d, 0xe1, 0x52, 0xa4, 0x25, 0xa5];
const DOODLES: [u8; 20] = [0x8a, 0x90, 0xca, 0xb2, 0xb3, 0x8d, 0xba, 0x80, 0xc6, 0x4b, 0x77, 0x34, 0xe5, 0x8e, 0xe1, 0xdb, 0x38, 0xb8, 0x99, 0x2e];
const DESK: [u8; 20] = [0xde; 20];
const ALICE: [u8; 20] = [0xa1; 20];
const TICK: u64 = 600;

/// One collection on 4663: its wrapper, module, factory and coins.
#[derive(Clone)]
struct Col {
    collection: [u8; 20],
    wrapper: [u8; 20],
    module: [u8; 20],
    factory: [u8; 20],
    accepted: u128,
    posted_at: u64,
    net_units: u128,
    transit: bool,
    grad_eth: u128,
    grad_units: u128,
    float_units: u128,
    total_supply: u128,
    held: u128,
    /// `(coin, curve, state, filled)`: FrogCurve's state 0 Trading, 1 Graduating, 2 Graduated, 3 Refunding
    coins: Vec<([u8; 20], [u8; 20], u128, bool)>,
    /// A post moves the accepted floor at most this (bps of it) from a posted one: the module's bounds (DR-M-1); 0: none.
    step_bps: u128,
}

impl Col {
    fn new(collection: [u8; 20], tag: u8) -> Self {
        Col { collection, wrapper: [tag; 20], module: [tag + 1; 20], factory: [tag + 2; 20], accepted: 0, posted_at: 0, net_units: 0,
            transit: false, grad_eth: 0, grad_units: 0, float_units: 0, total_supply: 0, held: 0, coins: vec![], step_bps: 0 }
    }
}

struct Fake {
    now: u64,
    dormancy: Option<Dormancy>,
    deposits_on: bool,
    cols: Vec<Col>,
    /// the desk's requests: `(wrapper, requester, at, bond, closed)`
    desk: Vec<([u8; 20], [u8; 20], u64, u128, bool)>,
    home_nonce: u64,
    home_sent: HashMap<[u8; 32], (bool, Vec<u8>)>,
    home_calls: Vec<([u8; 20], Vec<u8>)>,
    home_reads: Vec<([u8; 20], [u8; 4])>,
    eth_reads: usize,
    listings_read: usize,
    sale_receipts: HashMap<[u8; 32], ReceiptV>,
    eth_signed: Vec<Tx1559>,
    listing_total: u128,
    caps: CapPolicy,
    book: Book,
    deposit_txs: HashMap<[u8; 32], ([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>,
    /// §11: the canister's liquid cycles balance (`None`: it cannot say)
    cycles: Option<u128>,
    /// DI-M-1: the desk's window, and what `markServed` recorded (request id → the module's post)
    desk_window: u64,
    desk_served: HashMap<u64, u64>,
    /// D2-L-2: how many of the next markServed / fill estimates fail as a provider that timed out (not a revert)
    flaky_mark: u32,
    flaky_fill: u32,
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
fn arg_addr(d: &[u8], i: usize) -> [u8; 20] {
    d[4 + 32 * i + 12..4 + 32 * i + 32].try_into().unwrap()
}

impl Fake {
    fn new(dormancy: Option<Dormancy>) -> Self {
        let mut sale_receipts = HashMap::new();
        let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
        for (k, r) in v["receipts"].as_object().unwrap() {
            let h: [u8; 32] = hex::decode(&k[2..]).unwrap().try_into().unwrap();
            sale_receipts.insert(h, en::receipt_of(r).unwrap().unwrap());
        }
        let cols = vec![Col::new(MILADY, 0x10), Col::new(DOODLES, 0x20)];
        let keys: Vec<String> = cols.iter().map(|c| custodycap::evm_key(1, &c.collection)).collect();
        Fake { now: NOW, dormancy, deposits_on: false, cols, desk: vec![], home_nonce: 0, home_sent: HashMap::new(), home_calls: vec![],
            home_reads: vec![], eth_reads: 0, listings_read: 0, sale_receipts, eth_signed: vec![], listing_total: 45 * E / 100,
            caps: custodycap::stage0(&keys), book: Book::default(), deposit_txs: HashMap::new(),
            cycles: Some(1_000 * T), desk_window: 7 * 86_400, desk_served: HashMap::new(), flaky_mark: 0, flaky_fill: 0 }
    }
    fn c(&mut self, x: [u8; 20]) -> &mut Col {
        self.cols.iter_mut().find(|c| c.collection == x).unwrap()
    }
    fn col_of(&self, a: [u8; 20]) -> Option<(usize, &'static str)> {
        for (i, c) in self.cols.iter().enumerate() {
            if a == c.module { return Some((i, "module")); }
            if a == c.wrapper { return Some((i, "wrapper")); }
            if a == c.factory { return Some((i, "factory")); }
            if c.coins.iter().any(|x| x.1 == a) { return Some((i, "curve")); }
        }
        None
    }
    fn cfg(&self) -> EthNftConfig {
        let collections = self.cols.iter().map(|c| EthCollection { collection: format!("0x{}", hex::encode(c.collection)),
            wrapper: format!("0x{}", hex::encode(c.wrapper)), module: format!("0x{}", hex::encode(c.module)), max_purchase_wei: 2 * E,
            max_premium_bps: 500, chain_id: None, token_bound: None, nft_value_bps: None }).collect();
        EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: self.deposits_on, floor: None, collections, gas_pot_target_wei: None,
            logs_max_blocks: None, dormancy: self.dormancy.clone() }
    }
    /// Reads of `a` since the last clear.
    fn reads_of(&self, a: [u8; 20]) -> usize {
        self.home_reads.iter().filter(|(x, _)| *x == a).count()
    }
    fn clear(&mut self) {
        self.home_reads.clear();
        self.eth_reads = 0;
        self.listings_read = 0;
    }
    /// The desk's `markServed(id)`: the module's latest post at or after the request, inside its window (D2-L-1: on the timestamp alone).
    fn mark_ok(&self, id: u64) -> Result<u64, String> {
        let (w, _, at, _, closed) = *self.desk.get(id as usize).ok_or("no such request")?;
        let c = self.cols.iter().find(|c| c.wrapper == w).ok_or("unlisted")?;
        if closed {
            return Err("Closed".into());
        }
        if c.posted_at < at || c.posted_at > at + self.desk_window {
            return Err("NotServed".into());
        }
        Ok(c.posted_at)
    }
    fn apply_home(&mut self, to: [u8; 20], value: u128, d: &[u8]) -> bool {
        self.home_calls.push((to, d.to_vec()));
        let now = self.now;
        if to == rc::RELAY_DEPOSITORY_4663 {
            return true;
        }
        if to == DESK {
            assert_eq!(sel(d), dm::MARK_SERVED, "the canister calls only markServed on the desk");
            let id = arg(d, 0) as u64;
            return match self.mark_ok(id) {
                Ok(p) => {
                    self.desk_served.entry(id).or_insert(p);
                    true
                }
                Err(_) => false,
            };
        }
        let Some((i, what)) = self.col_of(to) else { panic!("a home call to 0x{}", hex::encode(to)) };
        let c = &mut self.cols[i];
        match (what, sel(d)) {
            ("module", s) if s == en::POST_FLOOR => {
                let want = arg(d, 0);
                c.accepted = if c.step_bps == 0 || c.accepted == 0 { want } else {
                    want.clamp(c.accepted * (10_000 - c.step_bps) / 10_000, c.accepted * (10_000 + c.step_bps) / 10_000) };
                c.posted_at = now;
            }
            ("module", s) if s == rc::ADVANCE => {
                c.transit = true;
            }
            ("module", s) if s == dm::FILL => {
                let coin = arg_addr(d, 0);
                let x = c.coins.iter_mut().find(|x| x.0 == coin).expect("a known coin");
                if x.2 != 1 || x.3 || now >= c.posted_at + 3_600 {
                    return false;
                }
                x.3 = true;
                c.grad_eth = 0;
            }
            _ => panic!("an unexpected home call to the {what} 0x{}: {}", hex::encode(to), hex::encode(d)),
        }
        let _ = value;
        true
    }
}

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for Fake {}

impl EthIo for Fake {
    fn now(&self) -> u64 { self.now }
    fn home_chain(&self) -> u64 { 4663 }
    fn eth_chain(&self) -> u64 { 1 }
    fn config(&self) -> Option<EthNftConfig> { Some(self.cfg()) }
    fn paused(&self) -> bool { false }
    fn eth_max_fee(&self) -> u128 { 50_000_000_000 }
    fn save(&mut self, _st: &EthState) {}
    fn with_caps<R>(&mut self, f: impl FnOnce(&CapPolicy, &mut Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _key: &str, _c: &str, _flow: Flow, _amount: u128) -> Result<(), String> { Ok(()) }
    fn outflow_record(&mut self, _key: &str, _c: &str, _flow: Flow, _amount: u128) {}
    fn cycles_balance(&self) -> Option<u128> { self.cycles }

    async fn our_evm(&mut self) -> Result<[u8; 20], String> { Ok(US) }
    async fn home_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> {
        let s = sel(d);
        self.home_reads.push((to, s));
        if to == rc::RELAY_DEPOSITORY_4663 && s == rc::ALLOCATOR {
            return Ok(addr_word(&rc::RELAY_ALLOCATOR_4663));
        }
        if to == DESK {
            if s == dm::REQUEST_COUNT {
                return Ok(word(self.desk.len() as u128));
            }
            if s == dm::REQUEST_AT {
                let (w, r, at, bond, closed) = self.desk[arg(d, 0) as usize];
                return Ok([addr_word(&w), addr_word(&r), word(at as u128), word(bond), word(closed as u128)].concat());
            }
            if s == dm::WINDOW {
                return Ok(word(self.desk_window as u128));
            }
            if s == dm::SERVED_AT {
                return Ok(word(self.desk_served.get(&(arg(d, 0) as u64)).copied().unwrap_or(0) as u128));
            }
        }
        let Some((i, what)) = self.col_of(to) else { return Err(format!("no contract at 0x{}", hex::encode(to))) };
        let c = &self.cols[i];
        Ok(match what {
            "module" => {
                if s == etch_autobuyer::floorsink::NET_UNITS { word(c.net_units) }
                else if s == etch_autobuyer::floorsink::ESCROW { word(10 * E) }
                else if s == etch_autobuyer::floorsink::BUFFER { word(E / 20) }
                else if s == etch_autobuyer::floorsink::FLOOR { [word(c.accepted), word(c.posted_at as u128), word(3_600)].concat() }
                else if s == etch_autobuyer::floorsink::MAX_ITEM_COST { word(if c.transit || c.accepted == 0 { 0 } else { c.accepted * 110 / 100 + rc::GAS_KEEP_WEI }) }
                else if s == rc::TRANSIT_UNITS { word(if c.transit { E } else { 0 }) }
                else if s == en::ACCEPTED { word(c.accepted) }
                else if s == en::KEEPER { addr_word(&US) }
                else if s == ef::FLOOR_BOUNDS { [word(0), word(u128::MAX)].concat() }
                else if s == dm::GRAD_ETH { word(c.grad_eth) }
                else if s == dm::GRAD_UNITS { word(c.grad_units) }
                else if s == dm::FLOAT_UNITS { word(c.float_units) }
                else if s == dm::FACTORY { addr_word(&c.factory) }
                else if s == dm::IS_FILLED {
                    let coin = arg_addr(d, 0);
                    word(c.coins.iter().find(|x| x.0 == coin).is_some_and(|x| x.3) as u128)
                }
                else { word(0) }
            }
            "wrapper" => {
                if s == dm::TOTAL_SUPPLY { word(c.total_supply) }
                else if s == dm::HELD { word(c.held) }
                else { word(0) }
            }
            "factory" => {
                if s == dm::COIN_COUNT { word(c.coins.len() as u128) }
                else if s == dm::COINS { addr_word(&c.coins[arg(d, 0) as usize].0) }
                else if s == dm::ROUTER_OF {
                    let coin = arg_addr(d, 0);
                    addr_word(&c.coins.iter().find(|x| x.0 == coin).map_or([0; 20], |x| x.1))
                }
                else { word(0) }
            }
            "curve" => {
                if s == dm::STATE { word(c.coins.iter().find(|x| x.1 == to).unwrap().2) } else { word(0) }
            }
            _ => unreachable!(),
        })
    }
    async fn home_clock(&mut self) -> Result<Clock, String> { Ok(Clock { time: self.now, parent_block: 26_000_000 }) }
    async fn home_balance(&mut self) -> Result<u128, String> { Ok(2 * E) }
    async fn home_sign(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> {
        // D2-L-2: a provider that timed out on the estimate reads the same as a revert
        if to == DESK && self.flaky_mark > 0 {
            self.flaky_mark -= 1;
            return Err("would not succeed now: eth_estimateGas: HTTP outcall timed out".into());
        }
        if data.len() >= 4 && sel(data) == dm::FILL && self.flaky_fill > 0 {
            self.flaky_fill -= 1;
            return Err("would not succeed now: eth_estimateGas: HTTP outcall timed out".into());
        }
        // the estimate: a markServed that would revert does not succeed now
        if to == DESK {
            if let Err(e) = self.mark_ok(arg(data, 0) as u64) {
                return Err(format!("would not succeed now: markServed reverts ({e})"));
            }
        }
        // the estimate: a fill that would revert does not succeed now
        if data.len() >= 4 && sel(data) == dm::FILL {
            let coin = arg_addr(data, 0);
            let (i, _) = self.col_of(to).unwrap();
            let c = &self.cols[i];
            let x = c.coins.iter().find(|x| x.0 == coin).unwrap();
            if x.2 != 1 || x.3 || self.now >= c.posted_at + 3_600 {
                return Err("would not succeed now: fill reverts".into());
            }
        }
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
    async fn home_finalized_time(&mut self) -> Result<u64, String> { Ok(self.now) }

    async fn eth_pin(&mut self) -> Result<EthPin, String> {
        self.eth_reads += 1;
        Ok(EthPin { number: ETH_PIN, timestamp: self.now, base_fee: 100_000_000 })
    }
    async fn eth_finalized(&mut self) -> Result<u64, String> { Ok(ETH_PIN - 100) }
    async fn eth_read(&mut self, _to: [u8; 20], _d: &[u8]) -> Result<Vec<u8>, String> {
        self.eth_reads += 1;
        Ok(addr_word(&[0x0e; 20]))
    }
    async fn eth_balance(&mut self) -> Result<u128, String> { Ok(E) }
    async fn eth_nonce(&mut self) -> Result<u64, String> { Ok(0) }
    async fn eth_receipt(&mut self, _h: [u8; 32]) -> Result<Option<OurReceipt>, String> { Ok(None) }
    async fn eth_plan_buy(&mut self, _order: &str, _collection: [u8; 20], cap: u128) -> Result<EthPlan, String> {
        if self.listing_total > cap {
            return Err(format!("refused (over cap): {} > {cap}", self.listing_total));
        }
        Ok(EthPlan { to: sp::SEAPORT_1_6, value: self.listing_total, data: vec![0xfb, 0x0f, 0x3e, 0xe1], gas: 200_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce: 0, token: sp::word_u128(9_180), total: self.listing_total })
    }
    async fn eth_plan_transfer(&mut self, collection: [u8; 20], token: Word, dest: [u8; 20]) -> Result<EthPlan, String> {
        Ok(EthPlan { to: collection, value: 0, data: en::transfer_from_call(&US, &dest, token), gas: 60_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce: 0, token, total: 0 })
    }
    async fn eth_sign(&mut self, tx: &Tx1559) -> Result<Vec<u8>, String> {
        self.eth_signed.push(tx.clone());
        Ok(vec![1, 2, 3])
    }
    async fn eth_broadcast(&mut self, _raw: &[u8]) {}
    async fn eth_listing(&mut self, s: &str, _c: [u8; 20]) -> Result<Seen, String> {
        self.listings_read += 1;
        let (o, p) = s.split_once(':').ok_or("not a fake listing")?;
        Ok(Seen { offerer: [o.parse::<u8>().unwrap(); 20], token: sp::word_u128(o.parse().unwrap()), price: p.parse().unwrap() })
    }
    async fn eth_sale_receipt(&mut self, h: [u8; 32]) -> Result<Option<ReceiptV>, String> {
        self.eth_reads += 1;
        Ok(self.sale_receipts.get(&h).cloned())
    }
    async fn eth_deposit_tx(&mut self, h: [u8; 32]) -> Result<Option<([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>, String> {
        Ok(self.deposit_txs.get(&h).cloned())
    }
    async fn eth_transfer_txs(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<([u8; 32], u64)>, String> {
        self.eth_reads += 1;
        let mut v: Vec<([u8; 32], u64)> = self.sale_receipts.iter().filter(|(_, r)| r.block >= from && r.block <= to && r.logs.iter().any(|l|
            l.address == collection && l.topics.len() == 4 && l.topics[0] == en::TRANSFER && l.topics[1] != l.topics[2] && l.topics[1] != [0; 32]))
            .map(|(h, r)| (*h, r.block)).collect();
        v.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        Ok(v)
    }
    async fn eth_plan_call(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<EthPlan, String> {
        Ok(EthPlan { to, value, data: data.to_vec(), gas: 60_000, max_fee: 1_000_000_000, priority_fee: 100_000_000, nonce: 0, token: [0; 32], total: value })
    }
}

fn tick(f: &mut Fake, st: &mut EthState) -> Vec<String> {
    let lines = block_on(ef::tick(f, st, false));
    for l in &lines {
        println!("{l}");
    }
    lines
}

fn next(f: &mut Fake, st: &mut EthState) -> Vec<String> {
    f.now += TICK;
    tick(f, st)
}

/// The runner's hand-in of Milady's listings (the same three the Ethereum NFT suite uses: the rank-2 floor is 0.93).
fn evidence(f: &Fake, st: &mut EthState) -> Result<String, String> {
    ef::put_evidence(st, &f.cfg(), MILADY, vec![format!("1:{}", 92 * E / 100), format!("2:{}", 93 * E / 100), format!("3:{}", 95 * E / 100)],
        vec![], f.now)
}

fn launch(desk: bool) -> Dormancy {
    let mut d = Dormancy::launch(desk.then(|| format!("0x{}", hex::encode(DESK))));
    d.max_active = 2;
    d
}

fn kind(f: &Fake, st: &EthState, c: [u8; 20]) -> Kind {
    ef::kind_of(st, &f.cfg(), &c)
}

/// From DORMANT to ACTIVE through the desk: the request, the backfill (two passes), the first post, its mining.
fn wake_milady(f: &mut Fake, st: &mut EthState) {
    f.desk.push((f.cols[0].wrapper, ALICE, f.now, 3_000_000_000_000_000, false));
    next(f, st);
    assert_eq!(kind(f, st, MILADY), Kind::Waking);
    for _ in 0..4 {
        if kind(f, st, MILADY) == Kind::Active {
            return;
        }
        evidence(f, st).unwrap();
        next(f, st);
    }
    next(f, st);
    assert_eq!(kind(f, st, MILADY), Kind::Active, "{:?}", st.dorm);
}

// ---- DORMANT costs nothing -------------------------------------------------------------------------------------------------------

#[test]
fn allowlisted_collections_start_dormant_and_a_dormant_collection_costs_no_read() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("dormant: nothing is read")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Dormant);
    // after the one decision (accepted() once each), a day of ticks reads only the desk: no module, wrapper, factory or Ethereum read
    f.clear();
    for _ in 0..144 {
        next(&mut f, &mut st);
    }
    for c in f.cols.clone() {
        assert_eq!(f.reads_of(c.module) + f.reads_of(c.wrapper) + f.reads_of(c.factory), 0, "a dormant collection is never read");
    }
    assert_eq!(f.eth_reads, 0, "nothing on Ethereum");
    assert_eq!(f.reads_of(DESK), 144, "one requestCount() a tick");
    // its evidence is refused at the door, and one that slipped in is dropped without an outcall
    assert!(evidence(&f, &mut st).unwrap_err().contains("dormant"));
    st.inbox.evidence.push((MILADY, vec!["1:1".into()], vec![], f.now));
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("its evidence is not read")), "{l:?}");
    assert_eq!(f.listings_read, 0);
    assert!(f.home_calls.is_empty(), "no post");
    // the query shows it
    let v = ef::collections_view(&st, &f.cfg(), f.now);
    assert_eq!(v[0].state, "dormant");
    assert_eq!(v[0].status, "dormant: a launch wakes it");
}

#[test]
fn without_dormancy_every_collection_is_always_on_as_before() {
    let mut f = Fake::new(None);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    assert!(st.dorm.is_none());
    assert_eq!(kind(&f, &st, MILADY), Kind::Always);
    assert!(evidence(&f, &mut st).is_ok());
    assert!(ef::put_wake(&mut st, &f.cfg(), MILADY, f.now).unwrap_err().contains("not configured"));
    assert_eq!(ef::collections_view(&st, &f.cfg(), f.now)[0].state, "always-on");
}

// ---- the wake: backfill, no post before it, ACTIVE on the mined post -------------------------------------------------------------

#[test]
fn a_desk_request_wakes_it_and_it_is_active_only_after_the_backfill_and_its_first_confirmed_post() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    f.desk.push((f.cols[0].wrapper, ALICE, f.now, 3_000_000_000_000_000, false));
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("WAKING")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Dormant, "only what was asked for");
    let v = ef::collections_view(&st, &f.cfg(), f.now);
    assert_eq!(v[0].state, "waking");
    assert!(v[0].woken_by.as_deref().unwrap().contains("wake desk"));
    // pass 1: the newest blocks and half the window back; the sales are read, the reading is confirmed, the backfill is not complete
    evidence(&f, &mut st).unwrap();
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("WAKING: warming up (~")), "{l:?}");
    assert!(f.home_calls.is_empty(), "no post before the backfill is complete");
    assert!(st.readings.iter().any(|r| r.collection == MILADY && !r.due));
    let v = ef::collections_view(&st, &f.cfg(), f.now);
    assert!(v[0].status.starts_with("warming up (~") && v[0].eta_secs.is_some(), "{v:?}");
    // pass 2: the window is read down to its start: the first post goes out
    evidence(&f, &mut st).unwrap();
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("the first post is next")), "{l:?}");
    assert_eq!(f.home_calls.len(), 1);
    assert_eq!(sel(&f.home_calls[0].1), en::POST_FLOOR);
    assert_eq!(f.cols[0].accepted, 93 * E / 100);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking, "not active before the post is read mined");
    // its mining makes it ACTIVE
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("the collection is ACTIVE")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Active);
    assert_eq!(ef::collections_view(&st, &f.cfg(), f.now)[0].status, "active");
}

#[test]
fn no_buy_and_no_deposit_mint_before_active() {
    let mut f = Fake::new(Some(launch(false)));
    let mut st = EthState::default();
    f.c(MILADY).net_units = 2 * E;               // the module wants an item
    tick(&mut f, &mut st);
    ef::put_wake(&mut st, &f.cfg(), MILADY, f.now).unwrap();
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
    f.c(MILADY).accepted = 90 * E / 100;          // even with a floor on the module (a once-active one), nothing buys while WAKING
    f.c(MILADY).posted_at = f.now;
    for _ in 0..3 {
        next(&mut f, &mut st);
    }
    assert!(st.buys.is_empty(), "no buy while WAKING");
    // a deposit judged while WAKING waits for ACTIVE (the mint is refused with the reason)
    st.deposits.push(ef::DepositRec { tx: [7; 32], collection: MILADY, token: [1; 32], recipient: ALICE, from: [9; 20], block: 1,
        phase: XPhase::Mint, refused_since: None });
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("the mint waits until it is active")), "{l:?}");
    assert!(!f.home_calls.iter().any(|(to, d)| *to == f.cols[0].wrapper && sel(d) == en::MINT_DEPOSIT));
}

#[test]
fn a_wake_that_cannot_confirm_times_out_and_the_cooldown_holds_the_next_request() {
    let mut f = Fake::new(Some(launch(false)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    ef::put_wake(&mut st, &f.cfg(), MILADY, f.now).unwrap();
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
    // no evidence ever comes (or it never confirms): 12 h later it is DORMANT again, with its scan dropped
    f.now += 43_200;
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("WAKE TIMED OUT")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    // asked again at once: queued behind its cooldown
    ef::put_wake(&mut st, &f.cfg(), MILADY, f.now).unwrap();
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("cooldown")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    assert!(ef::collections_view(&st, &f.cfg(), f.now)[0].status.starts_with("queued (#1)"));
    f.now += 86_400;
    tick(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking, "after the cooldown");
}

#[test]
fn max_active_and_max_waking_queue_the_rest() {
    let mut d = launch(false);
    d.max_active = 1;
    let mut f = Fake::new(Some(d));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    ef::put_wake(&mut st, &f.cfg(), MILADY, f.now).unwrap();
    ef::put_wake(&mut st, &f.cfg(), DOODLES, f.now + 1).unwrap();
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Queued);
    let v = ef::collections_view(&st, &f.cfg(), f.now);
    assert_eq!(v[1].queue_position, Some(1));
    for _ in 0..3 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert_eq!(kind(&f, &st, MILADY), Kind::Active);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Queued, "max_active 1: it waits until Milady sleeps");
}

#[test]
fn a_deposit_of_a_dormant_collection_asks_for_its_wake_owed() {
    let mut f = Fake::new(Some(launch(false)));
    f.deposits_on = true;
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    st.deposits.push(ef::DepositRec { tx: [7; 32], collection: DOODLES, token: [1; 32], recipient: ALICE, from: [9; 20], block: 1,
        phase: XPhase::Seen, refused_since: None });
    // an owed wake skips the cooldown
    st.dorm.as_mut().unwrap().life_mut(&DOODLES).unwrap().cooldown_until = f.now + 10 * 86_400;
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("a deposit")), "{l:?}");
    assert_eq!(kind(&f, &st, DOODLES), Kind::Waking);
}

// ---- ACTIVE: the slow cadence, hot near a buy ------------------------------------------------------------------------------------

#[test]
fn an_active_collection_is_read_on_the_slow_cadence_and_every_hand_in_when_a_buy_is_near() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    // evidence every tick (10 min): a pass only every 30 min (so a post about every 40 at the runner's 20)
    let mut passes = 0;
    for _ in 0..6 {
        evidence(&f, &mut st).unwrap();
        f.clear();
        next(&mut f, &mut st);
        if f.listings_read > 0 {
            passes += 1;
        }
    }
    assert_eq!(passes, 2, "an hour of hand-ins every 10 minutes: two passes");
    // the scan backs off when it counts no new sale
    let every = st.dorm.as_ref().unwrap().life(&MILADY).unwrap().scan_every;
    assert!(every > dm::SCAN_EVERY_SECS, "{every}");
    // the module view (a new buy) is read every 30 min, not every tick
    f.clear();
    for _ in 0..3 {
        next(&mut f, &mut st);
    }
    let views = f.home_reads.iter().filter(|(a, s)| *a == f.cols[0].module && *s == etch_autobuyer::floorsink::NET_UNITS).count();
    assert!(views <= 2, "{views}");
    // the module wants an item: hot, a buy starts, and every hand-in is a pass with a scan
    // the module wants an item: the next view makes it hot (read every tick), and the buy starts on a fresh post
    f.c(MILADY).net_units = 2 * E;
    for _ in 0..6 {
        evidence(&f, &mut st).unwrap();
        next(&mut f, &mut st);
        if !st.buys.is_empty() {
            break;
        }
    }
    assert!(!st.buys.is_empty(), "a buy started once ACTIVE");
    assert!(ef::is_hot(&st, &MILADY, f.now));
    let mut hot_passes = 0;
    for _ in 0..3 {
        evidence(&f, &mut st).unwrap();
        f.clear();
        next(&mut f, &mut st);
        if f.listings_read > 0 {
            hot_passes += 1;
        }
    }
    assert_eq!(hot_passes, 3, "hot: every hand-in");
}

// ---- the fresh read before a fill ----------------------------------------------------------------------------------------------

fn arrived_buy(f: &mut Fake, st: &mut EthState, floor: u128) {
    let id = st.next_buy;
    st.next_buy += 1;
    st.buys.push(ef::EthBuy { id, collection: MILADY, module: f.cols[0].module, wrapper: f.cols[0].wrapper, advanced: 6 * E / 10, attempts: 1,
        phase: BPhase::Arrived { wei: 55 * E / 100, since: f.now }, orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(),
        sweep: None, arrived_at: Some(f.now), back_at: None, ended_at: None, late: None });
    st.dorm.as_mut().unwrap().life_mut(&MILADY).unwrap().buy_floor = Some((id, floor));
}

#[test]
fn a_fill_needs_a_fresh_scanned_confirmed_reading_that_agrees_with_the_module_and_the_advance() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    arrived_buy(&mut f, &mut st, 93 * E / 100);
    let id = st.buys.last().unwrap().id;
    // 1. the reading is older than 15 minutes: nothing signed, the listing kept, the collection hot
    f.now += 1_000;
    ef::put_listing(&mut st, id, fill_hex(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("the fill needs one at most 900 s old") && x.contains("kept")), "{l:?}");
    assert!(f.eth_signed.is_empty());
    assert_eq!(st.inbox.listings.len(), 1, "kept for the next pass");
    assert!(ef::is_hot(&st, &MILADY, f.now));
    // 2. a fresh pass with its scan: the fill is signed
    evidence(&f, &mut st).unwrap();
    let l = next(&mut f, &mut st);
    assert_eq!(f.eth_signed.len(), 1, "{l:?}");
    assert!(matches!(st.buys.last().unwrap().phase, BPhase::Buying { .. }));
}

#[test]
fn a_fill_is_refused_on_a_mismatch_with_the_module_or_the_advance() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    // the advance was sized on a floor of 0.80: our fresh reading 0.93 is 16 % off: the listing is dropped, nothing signed
    arrived_buy(&mut f, &mut st, 80 * E / 100);
    let id = st.buys.last().unwrap().id;
    evidence(&f, &mut st).unwrap();
    f.now += TICK;
    ef::put_listing(&mut st, id, fill_hex(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("MISMATCH") && x.contains("advance was sized on") && x.contains("dropped")), "{l:?}");
    assert!(f.eth_signed.is_empty());
    assert!(st.inbox.listings.is_empty());
    // the module's accepted floor moved 10 % away from our reading: refused too
    st.dorm.as_mut().unwrap().life_mut(&MILADY).unwrap().buy_floor = Some((id, 93 * E / 100));
    f.c(MILADY).accepted = 83 * E / 100;
    evidence(&f, &mut st).unwrap();
    f.now += TICK;
    ef::put_listing(&mut st, id, fill_hex(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("MISMATCH") && x.contains("accepted floor")), "{l:?}");
    assert!(f.eth_signed.is_empty());
}

// ---- back to DORMANT ---------------------------------------------------------------------------------------------------------------

#[test]
fn an_idle_active_collection_goes_dormant_only_with_nothing_open_anywhere() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    let check = |f: &mut Fake, st: &mut EthState| {
        f.now += dm::SLEEP_CHECK_SECS;
        tick(f, st)
    };
    // wX is outstanding on 4663: a position, and the idle clock restarts at every check that sees it
    f.c(MILADY).total_supply = E;
    f.now += dm::NO_POSITION_SECS;
    let l = check(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("stays active: on 4663 it has wX supply")), "{l:?}");
    let seen = st.dorm.as_ref().unwrap().life(&MILADY).unwrap().position_at.unwrap();
    f.c(MILADY).total_supply = 0;
    // DR-H-1: a bare launch (a coin still trading, gas alone) is no position: it neither keeps the collection awake nor restarts the clock
    f.c(MILADY).coins.push(([0x61; 20], [0x62; 20], 0, false));
    let l = check(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("stays active: nothing open now; its last position was seen at")), "{l:?}");
    assert_eq!(st.dorm.as_ref().unwrap().life(&MILADY).unwrap().position_at, Some(seen));
    // an open buy in the canister is a position
    arrived_buy(&mut f, &mut st, 93 * E / 100);
    f.now += 3 * 86_400;
    let l = check(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("stays active: buy")), "{l:?}");
    st.buys.clear();
    let l = check(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("nothing open now")), "{l:?}");
    // idle_days (3) after the last position: asleep, the trading coin notwithstanding
    f.now += 3 * 86_400;
    let l = check(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("DORMANT: idle")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    assert!(st.readings.iter().all(|r| r.collection != MILADY) && st.sale_scan.iter().flatten().all(|s| s.collection != MILADY));
}

#[test]
fn dr_h_1_a_wake_nobody_uses_sleeps_six_hours_after_active_whatever_is_launched() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    // the griefer launches coins for gas alone (no buy, no graduation): nothing is a position
    for k in 0..5u8 {
        f.c(MILADY).coins.push(([0x90 + k; 20], [0xa0 + k; 20], 0, false));
    }
    f.now += dm::NO_POSITION_SECS;
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("DORMANT: idle")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant, "six hours after it became active, with no position: asleep");
}

#[test]
fn dr_m_1_after_a_long_dormancy_the_fill_waits_until_the_modules_clamped_floor_has_caught_up() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    f.now += dm::NO_POSITION_SECS + dm::SLEEP_CHECK_SECS;
    tick(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    // while it slept the floor fell to 0.93 from the 1.2 the module kept, and a graduation queued; each post moves it at most 10 %
    f.c(MILADY).accepted = 12 * E / 10;
    f.c(MILADY).step_bps = 1_000;
    let (coin, curve) = ([0x71; 20], [0x72; 20]);
    f.c(MILADY).coins.push((coin, curve, 1, false));
    f.c(MILADY).grad_eth = 4 * E;
    f.now += dm::SLEEP_CHECK_SECS;
    tick(&mut f, &mut st);
    for _ in 0..3 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert_eq!(kind(&f, &st, MILADY), Kind::Active);
    assert_eq!(f.cols[0].accepted, 108 * E / 100, "the first post is clamped");
    let mut waited = false;
    for _ in 0..4 {
        let l = next(&mut f, &mut st);
        waited |= l.iter().any(|x| x.contains("has not caught up"));
    }
    assert!(waited);
    assert!(!f.home_calls.iter().any(|(_, d)| sel(d) == dm::FILL), "no fill at a price the module has not caught up to");
    // the next posts walk it down; once within 5 % of our reading the graduation is filled
    for _ in 0..60 {
        if f.cols[0].coins[0].3 {
            break;
        }
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert!(dm::converged(93 * E / 100, f.cols[0].accepted), "{}", f.cols[0].accepted);
    assert!(f.cols[0].coins[0].3, "filled once converged");
}

#[test]
fn dr_h_2_a_deposit_whose_owed_wakes_keep_timing_out_goes_back_to_its_sender_and_never_holds_the_slot() {
    let mut f = Fake::new(Some(launch(false)));
    f.deposits_on = true;
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    st.deposits.push(ef::DepositRec { tx: [7; 32], collection: DOODLES, token: [1; 32], recipient: ALICE, from: [9; 20], block: 1,
        phase: XPhase::Seen, refused_since: None });
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Waking);
    // its wake never confirms (no evidence): it times out, backs off (1, 2 days), and after the third the NFT goes back
    let mut returned = false;
    for _ in 0..40 {
        f.now += 43_200;
        let l = tick(&mut f, &mut st);
        if l.iter().any(|x| x.contains("RETURNED to its sender")) {
            returned = true;
            break;
        }
    }
    assert!(returned, "{:?}", st.dorm.as_ref().unwrap().life(&DOODLES));
    assert_eq!(st.dorm.as_ref().unwrap().life(&DOODLES).unwrap().owed_fails, dm::OWED_FAILS_MAX);
    // on the return path (the fake's ownerOf names someone else, so the existing return judge refuses it: never minted either way)
    assert!(matches!(st.deposits[0].phase, XPhase::Return { .. } | XPhase::Returning { .. } | XPhase::Returned { .. } | XPhase::Refused { .. }),
        "{:?}", st.deposits[0].phase);
    assert!(st.dorm.as_ref().unwrap().life(&DOODLES).unwrap().queued.is_none(), "nothing asks for its wake any more");
    // the slot is free: a desk wake of Milady starts
    ef::put_wake(&mut st, &f.cfg(), MILADY, f.now).unwrap();
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
}

// ---- the watchdog and the fill -----------------------------------------------------------------------------------------------------

#[test]
fn the_watchdog_wakes_a_once_active_module_whose_graduation_queued_and_the_canister_fills_it() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    f.now += dm::NO_POSITION_SECS + dm::SLEEP_CHECK_SECS;
    tick(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    // while it sleeps, a coin is launched on its once-posted module and bought out: its graduation is queued at the stale module
    let (coin, curve) = ([0x71; 20], [0x72; 20]);
    f.c(MILADY).coins.push((coin, curve, 1, false));
    f.c(MILADY).grad_eth = 4 * E;
    f.now += dm::SLEEP_CHECK_SECS;
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("graduation queued")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking, "owed: past the cooldown");
    // it wakes (the backfill again, the post), then the hunt finds the Graduating curve and fills it
    for _ in 0..3 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert_eq!(kind(&f, &st, MILADY), Kind::Active);
    for _ in 0..4 {
        next(&mut f, &mut st);
    }
    assert!(f.home_calls.iter().any(|(_, d)| sel(d) == dm::FILL), "{:?}", f.home_calls.len());
    assert!(f.cols[0].coins[0].3, "filled");
    assert_eq!(f.cols[0].grad_eth, 0);
}

#[test]
fn the_watchdog_ignores_a_never_posted_module_and_a_bare_launch_and_wakes_on_a_graduation() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    f.clear();
    f.now += 10 * dm::SLEEP_CHECK_SECS;
    tick(&mut f, &mut st);
    assert_eq!(f.reads_of(f.cols[1].module) + f.reads_of(f.cols[1].factory), 0, "never posted: no launch is possible, nothing is read");
    wake_milady(&mut f, &mut st);
    f.now += dm::NO_POSITION_SECS + dm::SLEEP_CHECK_SECS;
    tick(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    // DR-H-1: coins launched on the once-posted module (gas alone) wake nothing, and cost one read a watchdog period
    for k in 0..20u8 {
        f.c(MILADY).coins.push(([0x81; 20].map(|x: u8| x.wrapping_add(k)), [0x40 + k; 20], 0, false));
    }
    f.now += 2 * 86_400;
    f.clear();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("watchdog: no graduation queued")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    assert_eq!(f.reads_of(f.cols[0].module) + f.reads_of(f.cols[0].factory), 1, "the watchdog reads gradEth() alone");
    // one of them graduates: owed
    f.c(MILADY).coins[3].2 = 1;
    f.c(MILADY).grad_eth = 4 * E;
    f.now += dm::SLEEP_CHECK_SECS;
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("graduation queued")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
}

#[test]
fn dr_m_2_a_flood_of_newer_coins_hides_a_graduation_from_the_hunt_and_the_operators_hand_in_finds_it() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    // the graduating coin, then twenty newer coins launched for gas alone
    let (coin, curve) = ([0x71; 20], [0x72; 20]);
    f.c(MILADY).coins.push((coin, curve, 1, false));
    for k in 0..20u8 {
        f.c(MILADY).coins.push(([0x10 + k; 20], [0x50 + k; 20], 0, false));
    }
    f.c(MILADY).grad_eth = 4 * E;
    for _ in 0..6 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert!(!f.home_calls.iter().any(|(_, d)| sel(d) == dm::FILL), "the newest eight coins are not it");
    assert!(st.dorm.as_ref().unwrap().life(&MILADY).unwrap().note.contains("hand in eth_fill"));
    // the operator hands in the coin of the GradQueued event: filled at the next home act
    ef::put_fill(&mut st, &f.cfg(), MILADY, coin, f.now).unwrap();
    for _ in 0..3 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert!(f.home_calls.iter().any(|(_, d)| sel(d) == dm::FILL && arg_addr(d, 0) == coin), "filled from the hand-in");
    assert!(f.cols[0].coins[0].3);
    // a hand-in names only WHICH coin: a coin the factory never launched is dropped, nothing signed
    let n = f.home_calls.len();
    f.c(MILADY).grad_eth = E;
    ef::put_fill(&mut st, &f.cfg(), MILADY, [0xee; 20], f.now).unwrap();
    st.dorm.as_mut().unwrap().life_mut(&MILADY).unwrap().fill_hunt = Some(f.now);
    evidence(&f, &mut st).ok();
    next(&mut f, &mut st);
    assert_eq!(f.home_calls.iter().skip(n).filter(|(_, d)| sel(d) == dm::FILL).count(), 0);
    assert!(st.dorm.as_ref().unwrap().fills.is_empty());
}

// ---- the lives after an upgrade --------------------------------------------------------------------------------------------------

#[test]
fn an_upgrade_that_turns_dormancy_on_keeps_what_was_watched_active() {
    let mut f = Fake::new(None);
    let mut st = EthState::default();
    // Milady was watched and posted before (a reading, a posted module); Doodles was never posted
    f.c(MILADY).accepted = 93 * E / 100;
    f.c(MILADY).posted_at = f.now;
    evidence(&f, &mut st).unwrap();
    tick(&mut f, &mut st);
    assert!(st.dorm.is_none());
    f.dormancy = Some(launch(true));
    tick(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Active);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Dormant);
    // a posted module with nothing in the canister is ACTIVE too (it may have holders)
    let mut st2 = EthState::default();
    f.c(DOODLES).accepted = E / 4;
    tick(&mut f, &mut st2);
    assert_eq!(kind(&f, &st2, DOODLES), Kind::Active);
}

// ---- §11: the limits scale with the canister's own cycles balance ------------------------------------------------------------------

fn cap(st: &EthState) -> dm::Capacity {
    st.dorm.as_ref().unwrap().cap.clone().expect("the capacity is published each tick")
}

#[test]
fn scaling_a_low_balance_holds_a_desk_wake_until_a_top_up_and_the_capacity_is_published() {
    let mut f = Fake::new(Some(launch(true)));
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    assert_eq!(cap(&st).effective_max_active, 0);
    assert_eq!(cap(&st).status, "0 of 2 collections can be awake right now (0 awake)");
    f.desk.push((f.cols[0].wrapper, ALICE, f.now, 3_000_000_000_000_000, false));
    let l = next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    assert!(l.iter().any(|x| x.contains("keeps 0 awake for 7 days with the reserve")), "{l:?}");
    // just under the first wake's threshold (1.5 T + 30.6 T): still queued; at it: WAKING
    f.cycles = Some(32_100 * T / 1_000 - 1);
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    f.cycles = Some(32_100 * T / 1_000);
    let l = next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking, "{l:?}");
    let c = cap(&st);
    assert_eq!((c.awake, c.effective_max_active, c.wake_headroom), (1, 1, 0), "the backfill's cost still to come is counted: {c:?}");
    // the query reads the balance now
    let q = ef::capacity_view(&st, &f.cfg(), f.dormancy.as_ref().unwrap(), Some(100 * T), f.now).unwrap();
    assert_eq!((q.effective_max_active, q.balance), (2, 100 * T));
    assert!(q.status.starts_with("2 of 2 collections can be awake right now"), "{}", q.status);
}

/// Both collections ACTIVE from before the dormancy (their modules were posted), both counted as holding positions until their first check.
fn both_active(f: &mut Fake) -> EthState {
    let mut st = EthState::default();
    for c in [MILADY, DOODLES] {
        f.c(c).accepted = 93 * E / 100;
        f.c(c).posted_at = f.now;
    }
    tick(f, &mut st);
    assert_eq!((kind(f, &st, MILADY), kind(f, &st, DOODLES)), (Kind::Active, Kind::Active));
    assert_eq!(cap(&st).pinned, 2);
    st
}

#[test]
fn scaling_down_sleeps_an_idle_collection_at_its_check_but_never_one_with_a_position() {
    // the control: on a rich balance an idle collection waits out its idle period
    let mut f = Fake::new(Some(launch(true)));
    let mut st = both_active(&mut f);
    f.c(MILADY).total_supply = E;
    f.now += dm::SLEEP_CHECK_SECS;
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("stays active: nothing open now")), "{l:?}");
    assert_eq!(kind(&f, &st, DOODLES), Kind::Active);
    // the balance keeps one (30.6 T ≤ 35 T < 51.1 T): at the checks Milady (wX outstanding) stays, Doodles (nothing) sleeps now
    let mut f = Fake::new(Some(launch(true)));
    let mut st = both_active(&mut f);
    f.c(MILADY).total_supply = E;
    f.cycles = Some(35 * T);
    next(&mut f, &mut st);
    assert!(cap(&st).status.contains("1 of 2 collections can be awake on this balance, but 2 hold positions"), "{:?}", cap(&st));
    f.now += dm::SLEEP_CHECK_SECS;
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("stays active: on 4663 it has wX supply")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("DORMANT: scaled down")), "{l:?}");
    assert_eq!((kind(&f, &st, MILADY), kind(&f, &st, DOODLES)), (Kind::Active, Kind::Dormant));
    let c = cap(&st);
    assert_eq!((c.effective_max_active, c.awake, c.pinned), (1, 1, 1), "{c:?}");
    assert_eq!(c.status, "1 of 2 collections can be awake right now (1 awake)");
}

#[test]
fn scaling_positions_pin_collections_awake_whatever_the_balance_and_no_wake_starts_while_the_limit_is_exceeded() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = both_active(&mut f);
    f.c(MILADY).total_supply = E;
    f.c(DOODLES).held = 1;
    f.cycles = Some(5 * T);
    f.now += dm::SLEEP_CHECK_SECS;
    let l = tick(&mut f, &mut st);
    assert_eq!(l.iter().filter(|x| x.contains("stays active: on 4663 it has")).count(), 2, "{l:?}");
    assert_eq!((kind(&f, &st, MILADY), kind(&f, &st, DOODLES)), (Kind::Active, Kind::Active), "never stranded");
    let c = cap(&st);
    assert_eq!((c.effective_max_active, c.awake, c.pinned, c.wake_headroom), (0, 2, 2, 0));
    assert!(c.status.contains("0 of 2 collections can be awake on this balance, but 2 hold positions"), "{}", c.status);
    // days later, nothing has been put to sleep
    for _ in 0..4 {
        f.now += dm::SLEEP_CHECK_SECS;
        tick(&mut f, &mut st);
    }
    assert_eq!((kind(&f, &st, MILADY), kind(&f, &st, DOODLES)), (Kind::Active, Kind::Active));
}

#[test]
fn scaling_review_s_3_a_balance_that_cannot_be_read_puts_nothing_to_sleep_and_wakes_nothing() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = both_active(&mut f);
    f.c(MILADY).total_supply = E;
    f.cycles = None;
    f.now += dm::SLEEP_CHECK_SECS;
    let l = tick(&mut f, &mut st);
    assert!(!l.iter().any(|x| x.contains("scaled down")), "{l:?}");
    assert_eq!(kind(&f, &st, DOODLES), Kind::Active, "idle, but nothing is decided on an unread balance");
    assert!(cap(&st).status.contains("could not be read"));
}

#[test]
fn desk_requests_are_read_a_few_a_tick_closed_ones_and_unknown_wrappers_ignored() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    f.desk.push(([0x99; 20], ALICE, f.now, 1, false));                  // not served by this canister
    f.desk.push((f.cols[1].wrapper, ALICE, f.now, 1, true));             // closed (reclaimed or forfeited)
    for _ in 0..4 {
        f.desk.push((f.cols[0].wrapper, ALICE, f.now, 1, false));
    }
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("does not serve")), "{l:?}");
    assert_eq!(st.dorm.as_ref().unwrap().desk_cursor, dm::DESK_READS_PER_TICK);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Dormant, "a closed request wakes nothing");
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
    next(&mut f, &mut st);
    assert_eq!(st.dorm.as_ref().unwrap().desk_cursor, 6);
    assert_eq!(st.dorm.as_ref().unwrap().wakes.len(), 1, "the repeats of an awake collection start nothing");
}

// ---- independent review 1 (docs/audit/2026-09-27-nft/dormant-review-independent-1.md): its PoCs, fixed --------------------------------

const MALLORY: [u8; 20] = [0x66; 20];
const BOND: u128 = 3_000_000_000_000_000;

fn deposit_of(c: [u8; 20]) -> ef::DepositRec {
    ef::DepositRec { tx: [7; 32], collection: c, token: [1; 32], recipient: ALICE, from: [9; 20], block: 1, phase: XPhase::Seen, refused_since: None }
}

fn life(st: &EthState, c: [u8; 20]) -> dm::Life {
    st.dorm.as_ref().unwrap().life(&c).unwrap().clone()
}

/// Milady woken, then asleep (idle): once active, so launches on its module are possible while it sleeps.
fn milady_once_active_and_asleep(f: &mut Fake, st: &mut EthState) {
    tick(f, st);
    wake_milady(f, st);
    f.now += dm::NO_POSITION_SECS + dm::SLEEP_CHECK_SECS;
    tick(f, st);
    assert_eq!(kind(f, st, MILADY), Kind::Dormant);
}

/// DI-H-1 control (the reviewer's `poc_di_1_control`): at 20 T, a balance that keeps no desk wake, a deposit wakes Doodles owed.
#[test]
fn di_h_1_control_a_deposit_wakes_owed_on_a_balance_that_keeps_no_desk_wake() {
    let mut f = Fake::new(Some(launch(true)));
    f.deposits_on = true;
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    st.deposits.push(deposit_of(DOODLES));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Waking);
}

/// DI-H-1 (the reviewer's `poc_di_1_a_queued_desk_request_downgrades_a_later_deposit_and_strands_it`): Mallory's desk request queues
/// first on the same balance. The deposit no longer finds the collection "not DORMANT": it upgrades the queued desk wake to owed (the
/// owed reserve and the owed path hold for it), and Doodles wakes at once instead of stranding the NFT.
#[test]
fn di_h_1_a_deposit_behind_a_queued_desk_request_wakes_owed_and_is_not_stranded() {
    let mut f = Fake::new(Some(launch(true)));
    f.deposits_on = true;
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    f.desk.push((f.cols[1].wrapper, MALLORY, f.now, BOND, false));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Queued, "no desk wake fits this balance");
    st.deposits.push(deposit_of(DOODLES));
    let l = next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Waking, "{l:?}");
    assert!(matches!(life(&st, DOODLES).by, Some(dm::WakeBy::Deposit { .. })), "owed: {:?}", life(&st, DOODLES).by);
    // Mallory's request is still held: the owed wake's first post serves it (her bond is decided by that post)
    assert_eq!(life(&st, DOODLES).desk.map(|a| a.id), Some(0));
}

/// DI-H-1 (the reviewer's `poc_di_1_a_queued_desk_request_blinds_the_graduation_watchdog`): Milady once active and asleep, a desk request
/// queued on a balance that keeps no desk wake, then a graduation queued at its module. The watchdog still reads it, and the owed wake
/// starts (the curve is filled before its refund window, not left to `openRefunds`).
#[test]
fn di_h_1_a_queued_desk_request_no_longer_blinds_the_graduation_watchdog() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    milady_once_active_and_asleep(&mut f, &mut st);
    f.cycles = Some(20 * T);
    f.c(MILADY).posted_at = f.now - 2 * 3_600;
    f.desk.push((f.cols[0].wrapper, MALLORY, f.now, BOND, false));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    let (coin, curve) = ([0x71; 20], [0x72; 20]);
    f.c(MILADY).coins.push((coin, curve, 1, false));
    f.c(MILADY).grad_eth = 4 * E;
    f.clear();
    for _ in 0..(dm::SLEEP_CHECK_SECS / TICK + 1) {
        next(&mut f, &mut st);
        if kind(&f, &st, MILADY) == Kind::Waking {
            break;
        }
    }
    assert!(f.home_reads.iter().any(|(a, s)| *a == f.cols[0].module && *s == dm::GRAD_ETH), "the watchdog read gradEth()");
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
    assert_eq!(life(&st, MILADY).by, Some(dm::WakeBy::Graduation));
}

/// DI-M-1 (the reviewer's `poc_di_2_a_desk_request_closed_and_refunded_on_chain_is_still_served_later`): a request queued behind a low
/// balance is dropped from the queue once a wake could no longer post inside its window, and is never started later: after its window
/// and grace (closed and refunded on chain) a top-up wakes nothing, so it can neither be served free nor hold the queue's head.
#[test]
fn di_m_1_a_desk_request_past_its_window_is_dropped_and_never_served_later() {
    let mut f = Fake::new(Some(launch(true)));
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    let at = f.now;
    f.desk.push((f.cols[0].wrapper, MALLORY, at, BOND, false));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    // the last moment a wake could start and still post inside the window (the timeout and the margin before its end)
    let deadline = at + f.desk_window - launch(true).wake_timeout_secs - dm::DESK_START_MARGIN_SECS;
    while f.now + TICK <= deadline {
        next(&mut f, &mut st);
    }
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued, "still servable in time");
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("dropped from the queue")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    // the window and the grace pass; anyone forfeits: refunded and closed on chain
    while f.now < at + 8 * 86_400 + 1 {
        next(&mut f, &mut st);
    }
    f.desk[0].4 = true;
    next(&mut f, &mut st);
    f.cycles = Some(100 * T);
    for _ in 0..3 {
        next(&mut f, &mut st);
    }
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant, "a closed, refunded request wakes nothing");
    assert!(f.home_calls.iter().all(|(a, _)| *a != f.cols[0].module), "nothing posted for it");
}

/// DI-M-1: a queued desk request closed on the desk before its wake starts (its bond reclaimed) is read again before the wake and dropped.
#[test]
fn di_m_1_a_queued_desk_request_closed_on_the_desk_is_read_again_and_dropped_before_its_wake() {
    let mut f = Fake::new(Some(launch(true)));
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    f.desk.push((f.cols[0].wrapper, MALLORY, f.now, BOND, false));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    f.desk[0].4 = true;
    f.cycles = Some(100 * T);
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("was closed on the desk before its wake started")), "{l:?}");
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    assert!(life(&st, MILADY).desk.is_none());
    // an open one is read again too, and starts
    f.desk.push((f.cols[0].wrapper, ALICE, f.now, BOND, false));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
}

/// DI-M-1, the treasury's side: the post that served a desk request is recorded on the desk (`markServed`) inside its window, so later
/// posts past the window never make a served wake look unserved.
#[test]
fn di_m_1_the_post_that_served_a_desk_request_is_recorded_on_the_desk_in_its_window() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    assert!(life(&st, MILADY).desk.as_ref().is_some_and(|a| a.served.is_some()), "{:?}", life(&st, MILADY).desk);
    for _ in 0..4 {
        next(&mut f, &mut st);
    }
    assert!(f.home_calls.iter().any(|(a, d)| *a == DESK && sel(d) == dm::MARK_SERVED && arg(d, 0) == 0));
    assert!(f.desk_served.contains_key(&0), "recorded on the desk");
    assert!(f.desk_served[&0] >= f.desk[0].2 && f.desk_served[&0] <= f.desk[0].2 + f.desk_window);
    next(&mut f, &mut st);
    assert!(life(&st, MILADY).desk.is_none(), "recorded: nothing more to do");
    let marks = f.home_calls.iter().filter(|(a, _)| *a == DESK).count();
    for _ in 0..6 {
        next(&mut f, &mut st);
    }
    assert_eq!(f.home_calls.iter().filter(|(a, _)| *a == DESK).count(), marks, "recorded once");
}

/// DI-M-2 (the reviewer's `poc_di_3_...`): a request read while the collection is ACTIVE is no longer dropped. On chain the desk now
/// refuses a request while the module is fresh; one it takes (the module stale while ACTIVE, here: no hand-in, no post) is kept, and
/// the collection sleeping before any post serves it queues its wake, which starts after the cooldown, inside the request's window.
#[test]
fn di_m_2_a_request_read_while_active_is_kept_and_wakes_the_collection_if_it_sleeps_unserved() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    f.now += 2 * 3_600;                           // no hand-in since: the module is stale, so the desk takes a request
    let at = f.now;
    f.desk.push((f.cols[0].wrapper, MALLORY, at, BOND, false));
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("is already active") && x.contains("request #1")), "{l:?}");
    assert_eq!(life(&st, MILADY).desk.as_ref().map(|a| a.id), Some(1));
    // no post, no position: it sleeps; the unserved request queues its wake
    for _ in 0..(dm::SLEEP_CHECK_SECS / TICK + 1) {
        next(&mut f, &mut st);
        if kind(&f, &st, MILADY) != Kind::Active {
            break;
        }
    }
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued, "{:?}", life(&st, MILADY));
    // the cooldown, then its wake (six days later it is not DORMANT with nothing asked, as in the PoC)
    for _ in 0..(2 * 86_400 / TICK) {
        next(&mut f, &mut st);
        if kind(&f, &st, MILADY) == Kind::Waking {
            break;
        }
    }
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
    assert!(matches!(life(&st, MILADY).by, Some(dm::WakeBy::Desk { id: 1, .. })));
    assert!(f.now < at + f.desk_window);
}

/// DI-M-2: a request read while the collection is WAKING is served by that wake's first post, and recorded.
#[test]
fn di_m_2_a_request_read_while_waking_is_served_by_the_wakes_first_post() {
    let mut f = Fake::new(Some(launch(true)));
    f.deposits_on = true;
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    st.deposits.push(deposit_of(MILADY));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
    f.desk.push((f.cols[0].wrapper, MALLORY, f.now, BOND, false));
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("already warming up")), "{l:?}");
    for _ in 0..12 {
        if f.desk_served.contains_key(&0) {
            break;
        }
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert_eq!(kind(&f, &st, MILADY), Kind::Active);
    assert!(f.desk_served.contains_key(&0), "recorded as served by the wake's first post");
}

/// Doodles ACTIVE, its module fresh and converged with our reading, a Graduating coin queued at it and its hunt started.
fn doodles_ready_to_fill(f: &mut Fake, st: &mut EthState) {
    let (coin, curve) = ([0x81; 20], [0x82; 20]);
    f.c(DOODLES).accepted = 25 * E / 100;
    f.c(DOODLES).posted_at = f.now;
    f.c(DOODLES).coins.push((coin, curve, 1, false));
    f.c(DOODLES).grad_eth = 2 * E;
    st.readings.retain(|r| r.collection != DOODLES);
    st.readings.push(ef::Reading { collection: DOODLES, post: en::FloorPost { price: 25 * E / 100, confidence: 9_000, depth: 10, listing: 25 * E / 100,
        sale_median: 25 * E / 100, sale_p25: 25 * E / 100, listings: 3, sales: 5, clamped: false, held: None }, at: f.now, pin: ETH_PIN, due: false,
        module_posted_at: Some(f.now) });
    let d = st.dorm.as_mut().unwrap();
    let l = d.life_mut(&DOODLES).unwrap();
    assert!(l.is_active());
    l.fill_hunt = Some(f.now);
}

/// Milady asleep a long time, a graduation queued, woken: its module's clamped floor walks 1 % a post towards our reading (DR-M-1).
fn milady_catching_up(f: &mut Fake, st: &mut EthState) {
    milady_once_active_and_asleep(f, st);
    f.c(MILADY).accepted = 2 * E;
    f.c(MILADY).step_bps = 100;
    f.c(MILADY).coins.push(([0x71; 20], [0x72; 20], 1, false));
    f.c(MILADY).grad_eth = 4 * E;
    f.now += dm::SLEEP_CHECK_SECS;
    tick(f, st);
    for _ in 0..4 {
        evidence(f, st).ok();
        next(f, st);
    }
    assert_eq!(kind(f, st, MILADY), Kind::Active);
    assert!(!dm::converged(93 * E / 100, f.cols[0].accepted));
}

/// DI-M-3 control (the reviewer's `poc_di_4_control`): Doodles alone is filled.
#[test]
fn di_m_3_control_doodles_fills_when_no_other_collection_waits() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = both_active(&mut f);
    doodles_ready_to_fill(&mut f, &mut st);
    for _ in 0..4 {
        f.c(DOODLES).posted_at = f.now;
        next(&mut f, &mut st);
    }
    assert!(f.cols[1].coins[0].3, "filled");
}

/// DI-M-3 (the reviewer's `poc_di_4_one_collection_catching_up_blocks_every_other_collections_fill`): Milady, earlier in the
/// configuration, waits for its clamped floor to catch up; Doodles' graduation is filled all the same, inside the day.
#[test]
fn di_m_3_one_collection_catching_up_no_longer_blocks_another_collections_fill() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    milady_catching_up(&mut f, &mut st);
    // Doodles: ACTIVE (its module posted), fresh and converged, its own graduation queued
    f.c(DOODLES).accepted = 25 * E / 100;
    f.c(DOODLES).posted_at = f.now;
    st.dorm.as_mut().unwrap().life_mut(&DOODLES).unwrap().phase = dm::Phase::Active { since: f.now };
    doodles_ready_to_fill(&mut f, &mut st);
    // Milady's hunt is due first (earlier in the configuration), and waits
    st.dorm.as_mut().unwrap().life_mut(&MILADY).unwrap().fill_hunt = Some(f.now);
    let t0 = f.now + TICK;
    for _ in 0..8 {
        f.c(DOODLES).posted_at = f.now;
        f.c(MILADY).posted_at = f.now;
        next(&mut f, &mut st);
        if f.cols[1].coins[0].3 {
            break;
        }
    }
    assert!(life(&st, MILADY).fill_hunt.is_some_and(|t| t >= t0 + dm::FILL_RETRY_SECS), "Milady was tried, waited, and is tried again later");
    assert!(!dm::converged(93 * E / 100, f.cols[0].accepted));
    assert!(f.cols[1].coins[0].3, "Doodles' graduation is filled while Milady waits");
    assert!(!f.cols[0].coins[0].3);
}

// ---- the services decode it ------------------------------------------------------------------------------------------------------

/// A state with every life (dormant with a queued desk request and a cooldown, waking with its estimate, active with a fill hunt) and a
/// pending `Fill`, as `eth_nft_state()` returns it. `tests/fixtures/eth-state-dormant.hex` is these bytes: the monitor's and the runner's
/// tests decode that file with their own IDL.
fn dormant_state() -> EthState {
    let mut st = EthState::default();
    let mut d = dm::Dorm::default();
    let mut a = dm::Life::new(MILADY, dm::Phase::Active { since: 5 }, 5);
    a.fill_hunt = Some(2);
    a.position_at = Some(6);
    a.buy_floor = Some((1, 7));
    a.read_scan = Some((8, true, 9));
    let mut w = dm::Life::new(DOODLES, dm::Phase::Waking { since: 10, backfilled_at: Some(11) }, 10);
    w.by = Some(dm::WakeBy::Desk { id: 4, requester: ALICE });
    w.eta = Some((2_400, 12));
    // independent review DI-M-1 / DI-M-2: its desk request as held, served by a post and not yet recorded on the desk
    w.desk = Some(dm::DeskAsk { id: 4, requester: ALICE, at: 9, until: 9 + 7 * 86_400, tried: true, served: Some(17), marked: false });
    let mut z = dm::Life::new([0x33; 20], dm::Phase::Dormant, 13);
    z.queued = Some((dm::WakeBy::Deposit { tx: [7; 32] }, 14));
    z.cooldown_until = 15;
    z.note = "the wake timed out after 43200 s".into();
    d.lives = vec![a, w, z];
    d.desk_cursor = 5;
    d.desk = Some(DESK);
    d.wakes = vec![10];
    d.fills = vec![(MILADY, [0x61; 20], 7)];
    d.desk_window = Some(7 * 86_400);
    // §11: the capacity as of the last tick (fewer can be awake than hold positions: the monitor pages it)
    d.lives[0].pinned = true;
    d.cap = Some(dm::Capacity { at: 16, balance: 20 * T, scaling: true, collections: 10, ceiling: 10, wakes_per_day: 12, effective_max_active: 0,
        awake: 2, pinned: 1, wake_headroom: 0, wakes_left_today: 11, next_wake_at: 54_100 * T / 1_000,
        status: "0 of 10 collections can be awake on this balance, but 1 hold positions: they stay awake; no new wake until a top-up".into() });
    st.dorm = Some(d);
    st.pending = Some(ef::HomeTx { step: ef::Step::Fill { collection: MILADY, coin: [0x61; 20] }, raw: vec![1], hash: [2; 32], nonce: 3,
        used_since: None, pin: None, sent_at: Some(4) });
    st
}

#[test]
fn the_dormant_state_fixture_is_what_the_canister_encodes() {
    let hex_now = hex::encode(candid::encode_one(dormant_state()).unwrap());
    let fixture = include_str!("fixtures/eth-state-dormant.hex").trim();
    assert_eq!(hex_now, fixture, "tests/fixtures/eth-state-dormant.hex is stale: write the new bytes there (and run the monitor's and ops' tests)");
    let back: EthState = candid::decode_one(&hex::decode(fixture).unwrap()).unwrap();
    assert_eq!(back, dormant_state());
}

/// Independent review DI-M-1: the same state with a pending `markServed` (`Step::Served`): `tests/fixtures/eth-state-dormant-served.hex`,
/// decoded by the monitor's test (a state it cannot decode would blind every check).
#[test]
fn the_dormant_state_with_a_pending_mark_is_what_the_canister_encodes() {
    let mut st = dormant_state();
    st.pending.as_mut().unwrap().step = ef::Step::Served { collection: DOODLES, id: 4 };
    let hex_now = hex::encode(candid::encode_one(&st).unwrap());
    let fixture = include_str!("fixtures/eth-state-dormant-served.hex").trim();
    assert_eq!(hex_now, fixture, "tests/fixtures/eth-state-dormant-served.hex is stale: write the new bytes there (and run the monitor's tests)");
}

// ---- independent review 2 (docs/audit/2026-09-27-nft/dormant-review-independent-2.md) ------------------------------------------------

/// D2-L-2, the reviewer's PoC (`poc_d2_one_transient_estimate_failure_abandons_mark_served_for_good`): one estimate that failed as a
/// provider timeout no longer gives up `markServed`. The chain shows it can be taken (the request open, nothing recorded, the latest post
/// inside the window), so it is tried again at the next tick and recorded, days before the window ends.
#[test]
fn d2_l_2_one_transient_estimate_failure_no_longer_abandons_mark_served() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    let at = f.now;
    f.flaky_mark = 1;
    wake_milady(&mut f, &mut st);
    for _ in 0..18 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert!(f.now + 5 * 86_400 < at + f.desk_window);
    assert_eq!(f.flaky_mark, 0, "the one failure happened (read as a wait: \"markServed(0) was not estimated ... tried again next tick\")");
    assert!(f.home_calls.iter().any(|(a, d)| *a == DESK && sel(d) == dm::MARK_SERVED && arg(d, 0) == 0), "markServed sent again");
    assert!(f.desk_served.contains_key(&0), "recorded on the desk");
    assert!(f.desk_served[&0] >= at && f.desk_served[&0] <= at + f.desk_window);
}

/// D2-L-2: a real refusal still ends the ask, once the chain confirms it: a request closed on the desk (reclaimed) is given up.
#[test]
fn d2_l_2_a_mark_is_given_up_only_when_the_chain_shows_the_request_closed() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    f.flaky_mark = 1_000;                         // the estimate never answers
    wake_milady(&mut f, &mut st);
    for _ in 0..3 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert!(life(&st, MILADY).desk.as_ref().is_some_and(|a| !a.marked), "kept while the chain says it can be taken");
    f.desk[0].4 = true;                           // reclaimed on chain
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("markServed(0) would not succeed: the request is closed on the desk")), "{l:?}");
    next(&mut f, &mut st);
    assert!(life(&st, MILADY).desk.is_none(), "given up: nothing more to do");
    assert!(f.desk_served.is_empty());
}

/// D2-L-2, the reviewer's PoC (`poc_d2_one_transient_estimate_failure_drops_the_operators_fill_hand_in`): one estimate that failed as a
/// provider timeout no longer drops the operator's hand-in. The chain still shows the graduation queued and the curve Graduating, so
/// the hand-in is kept, and the hunt fills the coin FILL_RETRY_SECS later.
#[test]
fn d2_l_2_one_transient_estimate_failure_no_longer_drops_the_operators_fill_hand_in() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    let (coin, curve) = ([0x71; 20], [0x72; 20]);
    f.c(MILADY).coins.push((coin, curve, 1, false));
    for k in 0..20u8 {
        f.c(MILADY).coins.push(([0x10 + k; 20], [0x50 + k; 20], 0, false));
    }
    f.c(MILADY).grad_eth = 4 * E;
    for _ in 0..3 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    f.flaky_fill = 1;
    ef::put_fill(&mut st, &f.cfg(), MILADY, coin, f.now).unwrap();
    let mut kept = false;
    for _ in 0..72 {
        evidence(&f, &mut st).ok();
        let l = next(&mut f, &mut st);
        kept |= l.iter().any(|x| x.contains("the operator's hand-in is kept"));
        if f.cols[0].coins[0].3 {
            break;
        }
    }
    assert_eq!(f.flaky_fill, 0, "the one failure happened");
    assert!(kept, "{:?}", st.dorm.as_ref().map(|d| &d.fills));
    assert!(f.cols[0].coins[0].3, "filled after the retry");
    assert!(f.home_calls.iter().any(|(_, d)| sel(d) == dm::FILL && arg_addr(d, 0) == coin));
    evidence(&f, &mut st).ok();
    next(&mut f, &mut st);
    next(&mut f, &mut st);
    assert!(st.dorm.as_ref().unwrap().fills.is_empty(), "dropped once the chain shows it filled");
}

/// D2-L-2: a hand-in is still dropped once the chain shows its curve left Graduating (refunds opened), whatever the estimate said.
#[test]
fn d2_l_2_a_kept_hand_in_is_dropped_once_the_curve_is_no_longer_graduating() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    let (coin, curve) = ([0x71; 20], [0x72; 20]);
    f.c(MILADY).coins.push((coin, curve, 1, false));
    for k in 0..20u8 {
        f.c(MILADY).coins.push(([0x10 + k; 20], [0x50 + k; 20], 0, false));
    }
    f.c(MILADY).grad_eth = 4 * E;
    for _ in 0..3 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    f.flaky_fill = 1_000;
    ef::put_fill(&mut st, &f.cfg(), MILADY, coin, f.now).unwrap();
    for _ in 0..8 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert_eq!(st.dorm.as_ref().unwrap().fills.len(), 1, "kept while the chain shows it fillable");
    f.cols[0].coins[0].2 = 3;                     // Refunding
    for _ in 0..8 {
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert!(st.dorm.as_ref().unwrap().fills.is_empty(), "dropped: the chain shows it can no longer be filled");
}

/// D2-L-3, the reviewer's PoC (`poc_d2_a_request_dropped_as_unserved_is_then_marked_served_by_the_canister`). The desk's rule is kept
/// (any post inside the window serves the request, and anyone may record it), and the canister's own words now say so: the drop no
/// longer promises an unconditional refund, and the post that came from another wake inside the window is recorded, as the desk would.
#[test]
fn d2_l_3_a_dropped_request_is_worded_as_the_desk_decides_it_and_a_post_in_its_window_still_serves_it() {
    let mut f = Fake::new(Some(launch(true)));
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    let at = f.now;
    f.desk.push((f.cols[0].wrapper, ALICE, at, BOND, false));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    let deadline = at + f.desk_window - launch(true).wake_timeout_secs - dm::DESK_START_MARGIN_SECS;
    while f.now + TICK <= deadline {
        next(&mut f, &mut st);
    }
    let l = next(&mut f, &mut st);
    assert!(!l.iter().any(|x| x.contains("goes back unserved")), "no promise the desk does not keep: {l:?}");
    assert!(l.iter().any(|x| x.contains("dropped from the queue")
        && x.contains("its bond goes back unless the collection is posted inside the window by another wake")), "{l:?}");
    assert!(life(&st, MILADY).note.contains("refunded unless another wake posts the collection inside it"), "{}", life(&st, MILADY).note);
    assert_eq!(kind(&f, &st, MILADY), Kind::Dormant);
    // the operator's wake posts the collection inside the window: the desk's rule serves the request, and the canister records it
    f.cycles = Some(100 * T);
    ef::put_wake(&mut st, &f.cfg(), MILADY, f.now).unwrap();
    for _ in 0..16 {
        if f.desk_served.contains_key(&0) {
            break;
        }
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert!(f.now < at + f.desk_window);
    assert!(f.desk_served.contains_key(&0), "served inside its window, as the desk and the words both say");
}

/// D2-I-2, the reviewer's PoC (`poc_d2_a_new_desk_request_inherits_a_closed_requests_queue_time`): a new desk request no longer takes
/// the queue place of a closed request of the same collection. It queues at its own read time, behind another collection's older request.
#[test]
fn d2_i_2_a_new_desk_request_queues_at_its_own_time_not_a_closed_requests() {
    let mut f = Fake::new(Some(launch(true)));
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    let t0 = f.now;
    f.desk.push((f.cols[0].wrapper, MALLORY, t0, BOND, false));
    next(&mut f, &mut st);
    f.desk.push((f.cols[1].wrapper, [0xb0; 20], f.now, BOND, false));
    next(&mut f, &mut st);
    for _ in 0..6 {
        next(&mut f, &mut st);
    }
    f.desk[0].4 = true;                                   // #0 reclaimed on chain
    f.desk.push((f.cols[0].wrapper, ALICE, f.now, BOND, false));
    next(&mut f, &mut st);
    let q = life(&st, MILADY).queued.clone().unwrap();
    assert!(matches!(q.0, dm::WakeBy::Desk { id: 2, .. }));
    assert_eq!(q.1, f.now, "request #2 queues at its own read time");
    assert!(q.1 > t0 + TICK);
    let d = st.dorm.as_ref().unwrap();
    assert!(d.position(&DOODLES).unwrap() < d.position(&MILADY).unwrap(), "Doodles' older #1 is ahead");
}


/// An upgrade argument naming the dormancy (and the desk), as the timelock publishes it: `tests/fixtures/init-dormant.hex`. The
/// monitor's strict decode (every label known) must take it, and its argument diff must word the dormancy.
fn dormant_init() -> etch_autobuyer::canister::Init {
    let f = Fake::new(Some(Dormancy::launch(Some(format!("0x{}", hex::encode(DESK))))));
    etch_autobuyer::canister::Init {
        chain_id: 4663,
        vault: "0x00000000000000000000000000000000000000cc".into(),
        rpc_urls: vec!["https://rpc.mainnet.chain.robinhood.com".into(), "https://rpc.ordofi.network".into()],
        btc_api_urls: vec!["https://mempool.space/api".into(), "https://blockstream.info/api".into()],
        key_name: "key_1".into(),
        accept_production_key_name: true,
        bridge_allowlist: vec![],
        btc_network: "mainnet".into(),
        operator: None,
        wrapper: None,
        evm_chains: None,
        custody_caps: None,
        outflow_limits: None,
        holder_deposits: None,
        wrapper_from_block: None,
        frog_funding: None,
        eth_nft: Some(f.cfg()),
        home: None,
    }
}

/// The fixture is what `dormant_init()` encodes, `home` label included (docs/v9/solana-migration.md §4): the monitor knows `home` since
/// phase B1 (`monitor/src/candid.mjs`), so its strict decode (`KNOWN_INIT_LABELS`) is held to these bytes. The bytes published before
/// `home` existed (no such label) still decode, to `home: None` (Candid's opt subtyping): `tests/fixtures/state/init-dormant-pre-home.hex`.
#[test]
fn the_dormant_init_fixture_is_what_the_canister_encodes() {
    let want = dormant_init();
    let fixture = include_str!("fixtures/init-dormant.hex").trim();
    assert_eq!(hex::encode(candid::encode_one(&want).unwrap()), fixture, "tests/fixtures/init-dormant.hex is stale: write the new bytes \
        there (and run the monitor's tests)");
    let old: etch_autobuyer::canister::Init = candid::decode_one(&hex::decode(include_str!("fixtures/state/init-dormant-pre-home.hex").trim()).unwrap())
        .expect("an Init from before `home` still decodes");
    assert_eq!(old.home, None);
    assert_eq!(candid::encode_one(&old).unwrap(), candid::encode_one(&want).unwrap(), "the old bytes decode to the same Init");
}

/// docs/v9/solana-migration.md §4: an upgrade argument naming the Solana home, as the timelock would publish it:
/// `tests/fixtures/init-solana.hex`, decoded by the monitor's strict decode (every label known: the `Solana` tag and its fields).
#[test]
fn the_solana_home_init_fixture_is_what_the_canister_encodes() {
    let mut i = dormant_init();
    i.home = Some(etch_autobuyer::canister::Home::Solana { cluster: "mainnet-beta".into(),
        rpc_urls: vec!["https://mainnet.helius-rpc.com".into(), "https://solana-rpc.publicnode.com".into()],
        // decision D-l: the books home, the only one an install accepts; rook_backing retired (null)
        rook_core: "books".into(), nonce_seed: "rook-home-0".into(), blockhash_fallback: None, rook_backing: None, drand_urls: None });
    let fixture = include_str!("fixtures/init-solana.hex").trim();
    assert_eq!(hex::encode(candid::encode_one(&i).unwrap()), fixture, "tests/fixtures/init-solana.hex is stale: write the new bytes there \
        (and run the monitor's tests)");
    let back: etch_autobuyer::canister::Init = candid::decode_one(&hex::decode(fixture).unwrap()).unwrap();
    assert_eq!(back.home, i.home);
}
