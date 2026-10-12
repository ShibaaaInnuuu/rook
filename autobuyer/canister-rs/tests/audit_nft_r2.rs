//! Fix round 2 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round2.md): every canister PoC of the round-2
//! re-review (docs/audit/2026-09-27-nft/review2/poc/B and poc/C) turned around, on the fake world of `tests/audit_nft.rs` (copied, with
//! two additions: `drop_milady`, as an upgrade dropping Milady, and the Relay depository's allocator read on Ethereum). The fake's
//! chain serves the canister's own sale scan from the receipts it knows. Each test FAILS on the code before this round (the PoC
//! passed there) and passes now.

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
    /// round 3 (D3-H-1): our own mined transfers' receipts with their `Transfer` log (the canister judges a delivery by it)
    our_logs: HashMap<[u8; 32], ReceiptV>,
    /// round 3 (D3-H-1): tickets the canister noted sent (`noteSent`)
    noted: Vec<u128>,
    /// `eth_sale_receipt` of these hashes fails (over the size cap, providers disagree, …)
    sale_err: Vec<[u8; 32]>,
    sale_reads: u32,
    /// every eth_getLogs range the sale scan asked (the operator's Dwellir plan serves at most 500 blocks)
    log_ranges: Vec<(u64, u64)>,
    deposit_txs: HashMap<[u8; 32], ([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>,
    caps: CapPolicy,
    book: Book,
    outflow: Vec<(String, Flow, u128)>,
    /// the configuration no longer names Milady (an upgrade dropped it)
    drop_milady: bool,
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
            receipt_err: HashMap::new(), plan_err: vec![], wait_prefix: vec![], broadcasts: vec![], listing_total: 45 * E / 100, sale_receipts, our_logs: HashMap::new(), noted: vec![],
            sale_err: vec![], sale_reads: 0, log_ranges: vec![], deposit_txs: HashMap::new(), caps: custodycap::stage0(&keys), book: Book::default(), outflow: vec![], drop_milady: false,
        }
    }
    fn m(&mut self, module: [u8; 20]) -> &mut Module {
        self.modules.get_mut(&module).unwrap()
    }
    fn cfg(&self) -> EthNftConfig {
        let col = |c: [u8; 20], w: [u8; 20], m: [u8; 20]| EthCollection { collection: hx(&c), wrapper: hx(&w), module: hx(&m),
            max_purchase_wei: 2 * E, max_premium_bps: 500, chain_id: None, token_bound: None, nft_value_bps: None };
        let mut collections = if self.drop_milady { vec![] } else { vec![col(MILADY, WRAPPER, MODULE)] };
        if self.two {
            collections.push(col(AZUKI, WRAPPER2, MODULE2));
        }
        EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: self.deposits_on, floor: None, collections, gas_pot_target_wei: None, logs_max_blocks: None, dormancy: None }
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
            } else if s == ef::FUND_RESERVE {
                // A2-L-1: a sweep pays into the module's surplus; no advance is involved
                assert!(!m.transit, "a sweep never meets an open advance at its module");
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
            (WRAPPER, s) if s == en::NOTE_SENT => self.noted.push(arg(d, 0)),
            (WRAPPER2, s) if s == en::NOTE_SENT => self.noted.push(arg(d, 0)),
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
        if tx.data.len() == 4 + 96 && tx.data[..4] == en::TRANSFER_FROM {
            // round 3 (D3-H-1): transferFrom(us, dest, token): its receipt shows the collection's Transfer when it succeeded
            let logs = if success { vec![LogV { address: tx.to, topics: vec![en::TRANSFER, tx.data[4..36].try_into().unwrap(),
                tx.data[36..68].try_into().unwrap(), tx.data[68..100].try_into().unwrap()], data: vec![] }] } else { vec![] };
            self.our_logs.insert(hash, ReceiptV { block, success, logs });
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

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for Fake {}

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
        if to == rc::RELAY_DEPOSITORY_4663 && sel(d) == rc::ALLOCATOR {
            return Ok(addr_word(&rc::RELAY_ALLOCATOR_4663)); // the depository and its allocator are the same on Ethereum
        }
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
        if let Some(r) = self.our_logs.get(&h) {
            return Ok(Some(r.clone()));
        }
        self.sale_reads += 1;
        if self.sale_err.contains(&h) {
            return Err("chain 1: eth_getTransactionReceipt: IC reject, answer over the response cap: Http body exceeds size limit of 200000 bytes".into());
        }
        Ok(self.sale_receipts.get(&h).cloned())
    }
    /// The chain's Transfer logs: every receipt the fake knows that moves a token of `collection` between two wallets (round 2).
    async fn eth_transfer_txs(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<([u8; 32], u64)>, String> {
        self.log_ranges.push((from, to));
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

// ---- A-M-1: the deposit band is the canister's own sample, and washes cannot open it -------------------------------------------------

#[test]
fn r2_a_m1_wash_sales_no_longer_open_the_deposit_band_at_a_floor_over_the_market() {
    // PoC `audit2_a_m1_seven_in_band_wash_sales_open_the_deposit_band_at_a_floor_10_percent_over_the_market`: seven chain washes at
    // 1.20 ETH handed in with the real sales set the band's median to 1.20 and minted a deposit at accepted 1.00 (+10 % over the
    // real 0.91). Now the canister reads the chain's sales itself, a wallet is one voice, and the band is the LOWER of the listing
    // floor and the counted sales' 25th percentile: neither a ring nor seven disjoint pairs of wash wallets opens it.
    for shape in ["none", "ring", "pairs"] {
        let mut f = Fake::new(false);
        f.deposits_on = true;
        let mut st = EthState::default();
        deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 1);
        ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
        f.finalized = ETH_PIN;
        f.m(MODULE).accepted = E;
        f.m(MODULE).bounds = (E, E);
        let mut hashes = Vec::new();
        if shape != "none" {
            hashes = washes(&mut f, 0x70, 7, 120 * E / 100, shape);
        }
        hashes.extend(real_sales());
        for _ in 0..4 {
            f.now += 600;
            ef::put_evidence(&mut st, &f.cfg(), MILADY, listings(), hashes.clone(), f.now).unwrap();
            tick(&mut f, &mut st);
        }
        let r = st.reading(&MILADY).unwrap().post.clone();
        assert!(r.price <= 93 * E / 100, "{shape}: never above the listing floor");
        assert!(r.sale_p25 <= 92 * E / 100, "{shape}: the real sales hold the lower quartile: {}", r.sale_p25);
        assert!(f.items.is_empty() && st.deposits[0].phase == XPhase::Mint, "{shape}: the deposit waits at a floor 10 % over the market");
    }
    // the honest case still mints: the module at the market
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 1);
    ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        f.now += 600;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    assert_eq!(f.items, vec![tok(950)], "at the market, a deposit mints");
}

#[test]
fn r2_a_m1_the_burn_side_the_post_never_rises_past_what_the_counted_sales_support() {
    // mint time is not burn time: the burn pays the posted floor, and a post rises only as far as the canister's own counted sales
    // support (their 25th percentile + 10 %), never above a live listing: a depositor holding a unit cannot be paid a walked-up
    // floor that the market's own sales do not show
    let p = en::FloorParams::default();
    let sale = |i: u8, price: u128| en::Sale { token: sp::word_u128(100 + i as u128), price, seller: [0x10 + i; 20], buyer: [0x30 + i; 20], block: 1 };
    let honest: Vec<en::Sale> = [90u128, 91, 91, 92, 90].iter().enumerate().map(|(i, c)| sale(i as u8, c * E / 100)).collect();
    let dear = [Seen { offerer: [1; 20], token: tok(1), price: 110 * E / 100 }, Seen { offerer: [2; 20], token: tok(2), price: 112 * E / 100 },
        Seen { offerer: [3; 20], token: tok(3), price: 130 * E / 100 }];
    let post = en::floor_from(&dear, &honest, &p, 93 * E / 100).unwrap();
    assert!(post.price <= 90 * E / 100 * 11 / 10, "held at the counted sales' p25 + 10 %: {}", post.price);
    assert!(post.held.is_some() && post.confidence == en::HELD_CONFIDENCE);
}

// ---- B2-M-1 = D2-M-2: wash trading can no longer stop a post ------------------------------------------------------------------------

#[test]
fn r2_b2_m1_ping_pong_sales_no_longer_stop_the_floor_post() {
    // PoC `audit2_b2_m1_ten_dust_ping_pong_sales_stop_every_floor_post`: ten ping-pong sales of one Milady at 0.001 ETH crowded the
    // 12 newest out and nothing was posted for two hours. Now the sample is the canister's own, over the whole window: the round trip
    // counts for nothing, the five real sales count, and the module is posted every time a post is due
    for dust in [E / 1_000, 93 * E / 100] {
        let mut f = Fake::new(false);
        f.m(MODULE).posted_at = NOW - 2_000;
        let mut st = EthState::default();
        let mut hashes = washes(&mut f, 0x60, 10, dust, "ping-pong");
        hashes.extend(real_sales().into_iter().take(2));
        let mut lines = Vec::new();
        for _ in 0..6 {
            ef::put_evidence(&mut st, &f.cfg(), MILADY, listings(), hashes.clone(), f.now).unwrap();
            lines.extend(tick(&mut f, &mut st));
            f.now += 1_200;
        }
        let posted = &f.m(MODULE).posted;
        assert!(posted.len() >= 3, "posted all along: {posted:?}");
        assert!(posted.iter().all(|p| p.0 == 93 * E / 100 && p.1 >= en::HELD_CONFIDENCE), "{posted:?}");
        assert!(st.reading(&MILADY).is_some_and(|r| r.post.sales == 5 && r.post.held.is_none()), "{lines:?}");
    }
}

#[test]
fn r2_b2_m1_in_band_washes_can_hold_the_floor_but_never_stop_a_post() {
    // PoC `audit2_b2_m1_seven_in_band_washes_under_the_listings_veto_the_post_too`: seven sales at 0.70 made the post an error. Now,
    // even with seven DISJOINT wash pairs (each its own token and two wallets), the module keeps being posted: the floor is held
    let p = en::FloorParams::default();
    let seen: Vec<Seen> = [(1u8, 92u128), (2, 93), (3, 95)].iter().map(|(o, c)| Seen { offerer: [*o; 20], token: tok(*o as u128), price: c * E / 100 }).collect();
    let sale = |i: u8, price: u128| en::Sale { token: sp::word_u128(100 + i as u128), price, seller: [0x10 + i; 20], buyer: [0x30 + i; 20], block: 1 };
    let honest: Vec<en::Sale> = [90u128, 91, 91, 92, 90].iter().enumerate().map(|(i, c)| sale(i as u8, c * E / 100)).collect();
    let mut all: Vec<en::Sale> = (0..7u8).map(|i| sale(0x40 + i, 70 * E / 100)).collect();
    all.extend(honest);
    let post = en::floor_from(&seen, &all, &p, 93 * E / 100).expect("a post, never a veto");
    assert_eq!(post.price, 93 * E / 100, "held at the module's floor, not pushed down");
    assert!(post.held.is_some());
    // and in the flow, over two hours of such washes on the chain: posted every time one is due
    let mut f = Fake::new(false);
    f.m(MODULE).accepted = 93 * E / 100;
    f.m(MODULE).posted_at = NOW - 2_000;
    let mut st = EthState::default();
    washes(&mut f, 0x60, 7, 70 * E / 100, "pairs");
    for _ in 0..6 {
        ef::put_evidence(&mut st, &f.cfg(), MILADY, listings(), vec![], f.now).unwrap();
        tick(&mut f, &mut st);
        f.now += 1_200;
    }
    let posted = &f.m(MODULE).posted;
    assert!(posted.len() >= 3 && posted.iter().all(|p| p.0 == 93 * E / 100), "{posted:?}");
}

// ---- B2-L-1: a swept thin book and in-band washes no longer lift the post ------------------------------------------------------------

#[test]
fn r2_b2_l1_after_a_sweep_washes_no_longer_let_the_post_follow_the_attackers_listings() {
    // PoC `audit2_b2_l1_…`: seven washes at 1.30 made a swept book's 1.12 the post (+23 % over every real sale). The band is now the
    // counted sales' 25th percentile + 10 %: the real sales hold it
    let p = en::FloorParams::default();
    let sale = |i: u8, price: u128| en::Sale { token: sp::word_u128(100 + i as u128), price, seller: [0x10 + i; 20], buyer: [0x30 + i; 20], block: 1 };
    let honest: Vec<en::Sale> = [90u128, 91, 91, 92, 90].iter().enumerate().map(|(i, c)| sale(i as u8, c * E / 100)).collect();
    let book = vec![Seen { offerer: [0xa1; 20], token: tok(1), price: 110 * E / 100 }, Seen { offerer: [0xa2; 20], token: tok(2), price: 112 * E / 100 },
        Seen { offerer: [0x03; 20], token: tok(3), price: 130 * E / 100 }];
    let mut all: Vec<en::Sale> = (0..7u8).map(|i| sale(0x40 + i, 130 * E / 100)).collect();
    all.extend(honest);
    assert!(en::floor_from(&book, &all, &p, 0).is_err(), "no first post from it");
    let post = en::floor_from(&book, &all, &p, 93 * E / 100).unwrap();
    assert!(post.price < 112 * E / 100 && post.price <= 91 * E / 100 * 11 / 10, "held: {}", post.price);
}

// ---- B2-L-2: an oversized receipt is read once ----------------------------------------------------------------------------------------

#[test]
fn r2_b2_l2_an_oversized_final_receipt_is_read_once_and_kept_as_no_sale() {
    // PoC `audit2_b2_l2_oversized_receipts_are_re_reserved_every_pass`: 12 unreadable receipts were read on every pass. Now each is
    // read once: final and over the largest size class, it is kept as no sale and never paid for again
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let hashes = washes(&mut f, 0x80, 12, 93 * E / 100, "pairs");
    f.sale_err = hashes.clone();
    let mut per_pass = Vec::new();
    for _ in 0..4 {
        let before = f.sale_reads;
        ef::put_evidence(&mut st, &f.cfg(), MILADY, listings(), hashes.clone(), f.now).unwrap();
        tick(&mut f, &mut st);
        per_pass.push(f.sale_reads - before);
        f.now += 1_200;
    }
    // 17 transactions on the chain (12 oversized, 5 real): 16 receipts a pass, then the last, then nothing
    assert_eq!(per_pass, vec![en::SALE_RECEIPTS_PER_PASS as u32, 1, 0, 0]);
    assert!(st.reading(&MILADY).is_some_and(|r| r.post.sales == 5), "and the real sales are counted: a post");
}

// ---- A2-M-1: no advance under the floor, and an arrived buy that cannot fill goes back ----------------------------------------------

#[test]
fn r2_a2_m1_a_module_whose_item_cap_does_not_cover_its_floor_never_advances() {
    // the pooled case: the module's blended backing (its maxItemCost) is under what its floor needs on Ethereum
    let mut f = Fake::new(false);
    f.m(MODULE).net_units = 2 * E;
    f.m(MODULE).accepted = 93 * E / 100;     // the floor rose; the pool (0.5002 ETH) did not
    let mut st = EthState::default();
    let l = tick(&mut f, &mut st);
    assert!(buy_of(&st, MILADY).is_none(), "no buy starts");
    assert!(l.iter().any(|x| x.contains("no buy: an advance of")), "{l:?}");
    assert!(!f.m(MODULE).transit && f.home_calls.is_empty(), "nothing advanced");
    // a buy started while the pool covered the floor, then the floor rose before the advance: dropped, nothing paid
    let mut f = Fake::new(false);
    f.m(MODULE).net_units = 2 * E;
    let mut st = EthState::default();
    block_on(ef::tick(&mut f, &mut st, true)); // another home-chain transaction is out: the buy is created, its advance waits
    assert!(matches!(phase(&st), BPhase::NeedAdvance));
    f.m(MODULE).accepted = 93 * E / 100;
    let l = tick(&mut f, &mut st);
    assert!(st.buys.is_empty(), "dropped before its advance: {l:?}");
    assert!(!f.m(MODULE).transit);
    assert!(ef::advance_covers(DEPOSIT + rc::GAS_KEEP_WEI, FLOOR0).is_ok() && ef::advance_covers(DEPOSIT + rc::GAS_KEEP_WEI, 49 * E / 100).is_err());
}

#[test]
fn r2_a2_m1_an_arrived_buy_that_cannot_fill_goes_back_to_its_module_through_relay() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    f.m(MODULE).net_units = 0;
    f.listing_total = 2 * E; // every listing is over what arrived: the floor rose after the advance
    let id = st.buys[0].id;
    // under the bound it waits (D4)
    f.now += ef::ARRIVED_BACK_SECS - 60;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Arrived { .. }));
    // past it, it goes back: a Relay quote Ethereum → 4663 is asked
    f.now += 120;
    tick(&mut f, &mut st);
    let BPhase::BackFunded { wei, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    assert_eq!(wei, ARRIVED);
    let ask = st.buys[0].ask.clone().unwrap();
    assert_eq!((ask.origin_chain_id, ask.destination_chain_id, ask.amount_wei), (1, 4663, ARRIVED - en::ETH_GAS_RESERVE_WEI));
    // a quote that pays someone else is refused by the same judge as the forward leg
    let bad = String::from_utf8(back_quote(ask.amount_wei, f.now)).unwrap().replace(&hex::encode(US)[..], &hex::encode([0x66; 20]));
    ef::put_quote(&mut st, id, bad.into_bytes(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("the way back's quote is refused")), "{l:?}");
    // the real one: the deposit is signed on Ethereum to Relay's depository, for the ask, and the leg is watched
    ef::put_quote(&mut st, id, back_quote(ask.amount_wei, f.now), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::BackSending { .. }), "{:?}", phase(&st));
    let (tx, _) = f.eth_signed.last().unwrap().clone();
    assert_eq!((tx.to, tx.value), (rc::RELAY_DEPOSITORY_4663, ask.amount_wei));
    assert!(st.relay_busy(), "it holds the one Relay leg");
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::BackBridging { .. }), "{:?}", phase(&st));
    assert_eq!(st.earmarked_except(u64::MAX), 0, "the ETH left for Relay: nothing earmarked");
    // Relay pays out on 4663: the ETH goes back to the module (returnAdvance), the unit is ETH-backed again
    let out = ask.amount_wei / 1_000 * 997;
    f.home_balance += out;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Returned { wei } if wei == out), "{:?}", phase(&st));
    let (to, value, data) = f.home_calls.last().unwrap().clone();
    assert_eq!((to, value, sel(&data)), (MODULE, out, rc::RETURN_ADVANCE));
    assert!(!f.m(MODULE).transit && !st.relay_busy());
    assert!(!f.book.entries.iter().any(|e| e.id == format!("eth:{id}")), "its custody reservation is released");
}

#[test]
fn r2_a2_m1_a_way_back_refunded_on_ethereum_waits_again_and_the_operator_can_send_it_back_early() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    f.m(MODULE).net_units = 0;
    f.listing_total = 2 * E;
    let id = st.buys[0].id;
    // the operator's exit (C2-M-2): an Arrived buy goes back early; round 3 (C3-L-2): once it has waited ARRIVED_HOLD_SECS, never at once
    assert!(ef::give_back_now(&mut st.clone(), id, 0, f.now).unwrap_err().contains("C3-L-2"));
    f.now += ef::ARRIVED_HOLD_SECS;
    f.m(MODULE).posted_at = f.now - 60;
    assert!(ef::give_back_now(&mut st, id, 0, f.now).unwrap().contains("goes back"));
    tick(&mut f, &mut st);
    let ask = st.buys[0].ask.clone().unwrap();
    ef::put_quote(&mut st, id, back_quote(ask.amount_wei, f.now), f.now).unwrap();
    tick(&mut f, &mut st);
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::BackBridging { .. }));
    // Relay refunds on Ethereum: the ETH is this buy's there again, earmarked, never more than the balance holds
    f.eth_balance += ask.amount_wei;
    tick(&mut f, &mut st);
    let BPhase::Arrived { wei, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    assert_eq!(wei, ask.amount_wei);
    assert!(st.earmarked_except(u64::MAX) <= f.eth_balance && !st.relay_busy());
}

// ---- C2-M-1: a mined fill whose outcome was not read is never another buy's arrival -------------------------------------------------

#[test]
fn r2_c2_m1_a_fill_mined_but_unread_is_never_read_as_another_collections_arrival() {
    // PoC `c2_a_fill_mined_at_the_pin_but_not_yet_counted_…`: Azuki went Arrived on Milady's spent 0.4954 ETH fill. Now the quote waits
    // while the line's outcome is unread, the watch starts with every spend counted, and nothing arrives that Relay did not send
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    f.m(MODULE).net_units = 0;
    f.m(MODULE).accepted = 93 * E / 100; // the module's floor once the buy is under way
    evidence(&f, &mut st);
    f.listing_total = ARRIVED - 3_000_000_000_000_000;
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    f.m(MODULE2).net_units = 2 * E;
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, MILADY).unwrap().phase, BPhase::Buying { .. }));
    tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    assert!(matches!(az.phase, BPhase::Funded { .. }), "{:?}", az.phase);
    let i = f.eth_signed.len() - 1;
    let fill_hash = f.eth_signed[i].1;
    f.mine_eth(i, true, ETH_PIN - 5);
    f.receipt_err.insert(fill_hash, 1);
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Funded { .. }), "the quote waits: {l:?}");
    assert!(l.iter().any(|x| x.contains("C2-M-1")), "{l:?}");
    // the receipt is read: the fill is counted; the quote is judged with every spend counted
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Depositing { .. } | BPhase::Bridging { .. }), "{:?}", buy_of(&st, AZUKI).unwrap().phase);
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    assert!(matches!(az.phase, BPhase::Bridging { .. }), "no Relay fill reached Ethereum: no arrival ({:?})", az.phase);
    assert!(st.earmarked_except(u64::MAX) <= f.eth_balance);
    // Relay's own fill lands: that is the arrival, of what Relay sent
    f.eth_balance += ARRIVED;
    tick(&mut f, &mut st);
    let BPhase::Arrived { wei, .. } = buy_of(&st, AZUKI).unwrap().phase else { panic!() };
    assert_eq!(wei, ARRIVED);
    assert!(buy_of(&st, AZUKI).unwrap().last.is_none(), "the watch ends with the arrival");
    assert!(st.earmarked_except(u64::MAX) <= f.eth_balance);
}

#[test]
fn r2_c2_m1_an_arrival_is_never_earmarked_over_the_balance() {
    // whatever the delta says, an arrival is at most what the order sent and what the balance holds beside other earmarks
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    f.m(MODULE).net_units = 0;
    f.m(MODULE2).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Bridging { .. }));
    // the watch misread (eth_out moved without the balance): a delta over the order's value is never an arrival over it
    st.eth_out += 2 * E;
    let l = tick(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    assert!(!matches!(az.phase, BPhase::Arrived { .. }), "{:?} {l:?}", az.phase);
    assert!(st.earmarked_except(u64::MAX) <= f.eth_balance);
}

// ---- C2-M-2: an arrived buy holds its collection's deposits six hours at most, and goes back after 48 -------------------------------

#[test]
fn r2_c2_m2_an_arrived_buy_that_cannot_fill_holds_deposits_six_hours_at_most_then_goes_back() {
    // PoC `c2_an_arrived_buy_that_cannot_fill_keeps_every_deposit_of_its_collection_unminted_indefinitely`: three days, the deposit
    // still `Mint`, no exit
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    let arrived_at = f.now;
    f.m(MODULE).net_units = 0;
    f.listing_total = 2 * E;
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 20);
    ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
    f.finalized = ETH_PIN;
    let (mut minted_at, mut back_at) = (None, None);
    for _ in 0..(3 * 24 * 3) {
        f.now += 1_200;
        f.m(MODULE).posted_at = f.now - 60;
        evidence(&f, &mut st);
        if matches!(phase(&st), BPhase::Arrived { .. }) {
            ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
        }
        tick(&mut f, &mut st);
        if minted_at.is_none() && !f.items.is_empty() {
            minted_at = Some(f.now);
        }
        if back_at.is_none() && matches!(phase(&st), BPhase::BackFunded { .. }) {
            back_at = Some(f.now);
        }
    }
    let m = minted_at.expect("the deposit is minted");
    assert!(m >= arrived_at + ef::ARRIVED_HOLD_SECS && m <= arrived_at + ef::ARRIVED_HOLD_SECS + 2 * 1_200, "minted once the hold ended");
    assert_eq!(f.items, vec![tok(950)]);
    let b = back_at.expect("the buy went back after 48 h");
    assert!(b >= arrived_at + ef::ARRIVED_BACK_SECS && b <= arrived_at + ef::ARRIVED_BACK_SECS + 2 * 1_200, "at 48 h");
    // round 3 (C3-M-1): with no quote handed in, its quote window ends after 2 h: it waits at Arrived again and goes back again later
    assert!(matches!(phase(&st), BPhase::Arrived { .. }), "{:?}", phase(&st));
    assert!(st.buys[0].back_at.unwrap() >= b + ef::BACK_QUOTE_SECS + ef::BACK_RETRY_SECS);
}

// ---- C2-L-1: a Stuck after arrival holds nothing and re-arrives nothing ----------------------------------------------------------------

#[test]
fn r2_c2_l1_a_stuck_after_arrival_no_longer_holds_the_relay_leg_or_re_arrives_spent_eth() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }));
    let i = f.eth_signed.len() - 1;
    f.mine_eth(i, true, ETH_PIN - 10);
    f.owners.insert((MILADY, tok(9_180)), [0x0e; 20]);
    f.m(MODULE2).net_units = 2 * E;
    tick(&mut f, &mut st);
    let m = buy_of(&st, MILADY).unwrap();
    assert!(matches!(m.phase, BPhase::Stuck { .. }), "{:?}", m.phase);
    assert!(m.last.is_none() && !m.in_relay(), "the watch ended at the arrival: the Stuck buy is off the Relay leg");
    assert!(st.earmarked_except(u64::MAX) <= f.eth_balance, "no phantom earmark");
    // L-2 (its Stuck branch is live again): the reservation of a buy that can no longer land is released
    assert!(!f.book.entries.iter().any(|e| e.id == format!("eth:{}", m.id)), "{:?}", f.book.entries);
    // Azuki's buy runs, arrives, and spends its own ETH
    for _ in 0..2 {
        tick(&mut f, &mut st);
    }
    let az = buy_of(&st, AZUKI).expect("Azuki's buy started");
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    f.eth_balance += ARRIVED;
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Arrived { .. }));
    let azl = vec![format!("1:{}", 66 * E / 100), format!("2:{}", 67 * E / 100), format!("3:{}", 68 * E / 100)];
    for k in 0..5u8 {
        f.sale_receipts.insert([0xa0 + k; 32], sale_receipt(AZUKI, 100 + k as u128, 67 * E / 100, [0x10 + k; 20], [0x50 + k; 20], FINAL));
    }
    f.listing_total = 45 * E / 100;
    let n = f.eth_signed.len();
    ef::put_evidence(&mut st, &f.cfg(), AZUKI, azl, vec![], f.now).unwrap();
    ef::put_listing(&mut st, az.id, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert_eq!(f.eth_signed.len(), n + 1, "Azuki's own arrived ETH buys");
}

// ---- C2-L-2: an upgrade dropping a collection with open work ---------------------------------------------------------------------------

#[test]
fn r2_c2_l2_a_dropped_collections_buy_gives_its_eth_back_and_the_upgrade_is_refused() {
    let mut f = Fake::new(true);
    f.m(MODULE).net_units = 2 * E;
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    // post_upgrade refuses an upgrade that drops Milady now
    let mut g = Fake::new(true);
    g.drop_milady = true;
    assert!(!ef::stranded_by(&st, Some(&g.cfg())).is_empty(), "the upgrade is refused");
    assert!(ef::stranded_by(&st, Some(&f.cfg())).is_empty(), "an upgrade that keeps it is not");
    // and if the collection is gone anyway, its buy gives the advance back to its own module, and the Relay leg is free
    f.drop_milady = true;
    f.m(MODULE2).net_units = 2 * E;
    for _ in 0..6 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    let m = buy_of(&st, MILADY).unwrap();
    assert!(matches!(m.phase, BPhase::Returned { .. }), "{:?}", m.phase);
    assert!(f.home_calls.iter().any(|(to, _, d)| *to == MODULE && sel(d) == rc::RETURN_ADVANCE));
    assert!(buy_of(&st, AZUKI).is_some(), "Azuki's buy starts");
}

// ---- C2-L-3: the upgrade probe sees an older type nested in the state -------------------------------------------------------------------

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
fn r2_c2_l3_the_upgrade_probe_refuses_an_older_eth_tx_inside_the_line() {
    use etch_autobuyer::canister::{lost_elements, lost_opts, Saved, SavedProbe};
    type Written = (Option<etch_autobuyer::canister::Init>, Vec<etch_autobuyer::state::Purchase>, bool, Vec<String>,
        Option<etch_autobuyer::redeem::Custody>, Option<etch_autobuyer::chains::NftState>, Option<etch_autobuyer::frogflow::FrogState>,
        Option<etch_autobuyer::floortrigger::FloorState>, Option<(u8,)>, Option<u8>, Option<u8>, Option<u8>, Option<SideEthState>);
    let d = ef::Delivery { wrapper: WRAPPER, collection: MILADY, ticket: 3, token: tok(77), destination: ALICE, deliver_by: NOW,
        phase: DPhase::Sent { tx: [1; 32], nonce: 9 }, note: String::new(), refused_since: None, sent: None };
    let side = SideEthState { buys: vec![], next_buy: 1, deliveries: vec![d], cursors: vec![(WRAPPER, 4)], deposits: vec![], readings: vec![],
        line: Some(SideEthTx { kind: ef::EthKind::Deliver { wrapper: WRAPPER, ticket: 3 }, nonce: 9, hash: [1; 32], raw: vec![0xde, 0xad],
            value: 0, max_fee: 1, priority_fee: 1, signed_at: 1 }), eth_out: 5, pending: None, used_orders: vec![[7; 32]], outbound: vec![],
        inbox: ef::Inbox::default(), rr: 0 };
    let w: Written = (None, vec![], false, vec![], None, None, None, None, None, None, None, None, Some(side));
    let bytes = candid::encode_args((w,)).unwrap(); // as stable_save writes `(saved,)`
    let (saved,): (Saved,) = candid::decode_args(&bytes).unwrap();
    let s = saved.12.clone().expect("the element decodes");
    assert!(s.line.is_none() && matches!(s.deliveries[0].phase, DPhase::Sent { .. }), "Candid alone forgets the signed line");
    // the element-level probe (round 1) does not see it
    let (probe,): (SavedProbe,) = candid::decode_args(&bytes).unwrap();
    assert!(lost_elements(&probe, [saved.4.is_some(), saved.5.is_some(), saved.6.is_some(), saved.7.is_some(), saved.8.is_some(),
        saved.9.is_some(), saved.10.is_some(), saved.11.is_some(), saved.12.is_some()]).is_empty());
    // the round-2 probe, at any depth, does: post_upgrade traps
    let again = candid::encode_args((saved,)).unwrap();
    let lost = lost_opts(&bytes, &again).unwrap();
    assert_eq!(lost.len(), 1, "{lost:?}");
    // and a state of this build's own shape, nested options and all, loses nothing
    let mut cur = EthState::default();
    cur.line = Some(ef::EthTx { kind: ef::EthKind::BackDeposit { buy: 2 }, nonce: 3, hash: [2; 32], raw: vec![1], value: 5, max_fee: 1,
        priority_fee: 1, signed_at: 1, to: US, data: vec![], gas: 21_000, earlier: vec![], rivals: vec![], first_signed_at: Some(1) });
    cur.last_mined = Some((2, 3));
    cur.sale_scan = Some(vec![]);
    let w2: Saved = (None, vec![], false, vec![], None, None, None, None, None, None, None, None, Some(cur), None, None);
    let b2 = candid::encode_args((w2,)).unwrap();
    let (s2,): (Saved,) = candid::decode_args(&b2).unwrap();
    assert!(lost_opts(&b2, &candid::encode_args((s2,)).unwrap()).unwrap().is_empty());
}

// ---- the services' Candid types decode this round's state (every new case) -------------------------------------------------------------

/// An `EthState` with every case this round added (the way back's three phases and its line, a held reading, the sale scan), as
/// `eth_nft_state()` returns it. `tests/fixtures/eth-state-round2.hex` is these bytes: the monitor and the runner decode that file with
/// their own IDL (an unknown variant tag is an error there), so a case added here and not there fails their tests.
fn round2_state() -> EthState {
    let buy = |id: u64, phase: BPhase| ef::EthBuy { id, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: 1, attempts: 1, phase,
        orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(), sweep: None, arrived_at: None, back_at: None, ended_at: None, late: None };
    let mut st = EthState::default();
    st.buys = vec![buy(1, BPhase::BackFunded { wei: 5, since: 6 }),
        buy(2, BPhase::BackSending { wei: 5, order_id: [1; 32], value: 4, min_out: 3, deadline: 9, tx: [2; 32] }),
        buy(3, BPhase::BackBridging { order_id: [1; 32], value: 4, min_out: 3, deadline: 9, since: 7 })];
    st.line = Some(ef::EthTx { kind: ef::EthKind::BackDeposit { buy: 2 }, nonce: 3, hash: [2; 32], raw: vec![1], value: 4, max_fee: 1,
        priority_fee: 1, signed_at: 1, to: US, data: vec![], gas: 21_000, earlier: vec![], rivals: vec![], first_signed_at: Some(1) });
    st.readings = vec![ef::Reading { collection: MILADY, post: en::FloorPost { price: 1, confidence: en::HELD_CONFIDENCE, depth: 3, listing: 2,
        sale_median: 1, sale_p25: 1, listings: 3, sales: 2, clamped: false, held: Some("held".into()) }, at: 1, pin: 2, due: true, module_posted_at: None }];
    st.sale_scan = Some(vec![ef::SaleScan { collection: MILADY, down: 1, up: 2, chunk: 25, todo: vec![([3; 32], 2)], pend: None, rescan: None, judged: None, bad: None, skipped: None, done_big: None, stuck: None, spent: None, hole: None, lost: None, behind: None, rfail: None, raw: None, current: None, hole_deep: None, rbusy: None, busy: None }]);
    // A2-L-1 (2026-09-27): a gas-pot sweep on its way to fundReserve, the per-collection leftover and the sweep cooldown
    let mut sweep = buy(4, BPhase::Returning { wei: 5 });
    sweep.sweep = Some(true);
    sweep.advanced = 0;
    st.buys.push(sweep);
    st.leftover = Some(vec![(MILADY, 7)]);
    st.sweep_after = Some(8);
    st
}

#[test]
fn r2_the_round_2_state_fixture_is_what_the_canister_encodes() {
    let hex_now = hex::encode(candid::encode_one(round2_state()).unwrap());
    let fixture = include_str!("fixtures/eth-state-round2.hex").trim();
    assert_eq!(hex_now, fixture, "tests/fixtures/eth-state-round2.hex is stale: write the new bytes there (and run the monitor's and ops' tests)");
}

// ---- A2-L-1 (the operator, 2026-09-27): the advance is the buy's bound; the gas pot's leftover goes back to its own module ----------

/// BAYC as J1 decided it: a 6.79 floor, the item cap 1.10 × 6.79 = 7.469, the purchase limit 8 ETH, a 5 % premium and Relay's
/// judged bound of 3 %: the advance is (7.1295 + 0.003) / 0.97 + 0.0002 = 7.35334… ETH, not 7.469; with a fresh reading of 6.70
/// under the accepted floor the buy's bound is lower still; the item cap stays the ceiling.
#[test]
fn a2_l_1_the_advance_is_the_buys_bound_not_the_item_cap() {
    let (cap, acc) = (7_469 * E / 1_000, 6_790 * E / 1_000);
    let (a, floor) = ef::advance_bound(cap, acc, None, NOW, 500, 8 * E, 300);
    assert_eq!(floor, acc);
    assert_eq!(a, (acc * 10_500 / 10_000 + en::ETH_GAS_RESERVE_WEI) * 10_000 / 9_700 + 1 + rc::GAS_KEEP_WEI);
    assert!(a < cap && cap - a > 115 * E / 1_000, "≈ 0.116 ETH of the item cap never leaves the module");
    // what arrives after the judged worst fee still pays the buy's most and keeps the gas reserve
    let arrive = (a - rc::GAS_KEEP_WEI) * 9_700 / 10_000;
    assert!(arrive >= acc * 10_500 / 10_000 + en::ETH_GAS_RESERVE_WEI - 1);
    assert_eq!(ef::advance_covers(a, floor), Ok(()), "the round-2 gate holds for what is advanced");
    // our fresh reading under the accepted floor bounds the buy, so it bounds the advance; a stale one does not count
    let (b, f2) = ef::advance_bound(cap, acc, Some((6_700 * E / 1_000, NOW - 60)), NOW, 500, 8 * E, 300);
    assert!(f2 == 6_700 * E / 1_000 && b < a);
    assert_eq!(ef::advance_bound(cap, acc, Some((6_700 * E / 1_000, NOW - en::READING_TTL_SECS - 1)), NOW, 500, 8 * E, 300).0, a);
    // the purchase limit bounds it; the item cap is the ceiling (after a floor rise the pool's cap is what binds, as before)
    assert!(ef::advance_bound(cap, acc, None, NOW, 500, 7 * E, 300).0 < a);
    assert_eq!(ef::advance_bound(7 * E, 7_300 * E / 1_000, None, NOW, 500, 8 * E, 300).0, 7 * E);
}

#[test]
fn a2_l_1_the_canister_advances_only_the_bound_and_reserves_custody_for_it() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).item_cap = E;                 // a pool whose item cap is far over the buy's bound (0.47 floor)
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let (to, _, data) = f.home_calls.iter().find(|(_, _, d)| sel(d) == rc::ADVANCE).cloned().expect("an advance");
    let (want, _) = ef::advance_bound(E, FLOOR0, st.readings.first().map(|r| (r.post.price, r.at)), f.now, 500, 2 * E, 300);
    assert_eq!((to, arg(&data, 0)), (MODULE, want));
    assert!(want < E / 10 * 6, "about 0.51 ETH, not the 1 ETH item cap");
    assert_eq!(st.buys[0].advanced, want);
    let r = f.book.entries.iter().find(|e| e.id == format!("eth:{}", st.buys[0].id)).expect("reserved");
    assert_eq!(r.value_wei, want, "custody counts what left, not the item cap");
}

/// The fill's leftover is its collection's; the sweep sends it to that collection's module only, the largest first; the kept
/// gas reserve of a way back's deposit is credited too; `fundReserve` is the call, with no advance open at the module.
#[test]
fn a2_l_1_the_sweep_returns_each_collections_leftover_to_its_own_module() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    st.credit_leftover(&MILADY, 300 * E / 1_000);
    st.credit_leftover(&AZUKI, 100 * E / 1_000);
    f.eth_balance = 500 * E / 1_000;
    tick(&mut f, &mut st);
    let s = st.buys.last().unwrap().clone();
    assert!(s.is_sweep() && s.collection == MILADY && s.module == MODULE && s.advanced == 0);
    let BPhase::BackFunded { wei, .. } = s.phase else { panic!("{:?}", s.phase) };
    assert_eq!(wei, 300 * E / 1_000, "Milady's leftover, the larger");
    assert_eq!((st.leftover_of(&MILADY), st.leftover_of(&AZUKI)), (0, 100 * E / 1_000), "Azuki's is untouched");
    let ask = s.ask.clone().unwrap();
    assert_eq!((ask.origin_chain_id, ask.destination_chain_id, ask.amount_wei), (1, 4663, wei - en::ETH_GAS_RESERVE_WEI));
    assert!(st.open_buy_for(&AZUKI).is_none());
    // the way back's judge: a quote paying someone else is refused
    let bad = String::from_utf8(back_quote(ask.amount_wei, f.now)).unwrap().replace(&hex::encode(US)[..], &hex::encode([0x66; 20]));
    ef::put_quote(&mut st, s.id, bad.into_bytes(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("the way back's quote is refused")), "{l:?}");
    ef::put_quote(&mut st, s.id, back_quote(ask.amount_wei, f.now), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::BackSending { .. }), "{:?}", phase(&st));
    let (tx, _) = f.eth_signed.last().unwrap().clone();
    assert_eq!((tx.to, tx.value), (rc::RELAY_DEPOSITORY_4663, ask.amount_wei));
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::BackBridging { .. }));
    assert_eq!(st.leftover_of(&MILADY), en::ETH_GAS_RESERVE_WEI - 21_000 * 1_000_000_000, "the kept reserve less the deposit's gas");
    let out = ask.amount_wei / 1_000 * 997;
    f.home_balance += out;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(st.buys[0].phase, BPhase::Returned { wei } if wei == out), "{:?}", st.buys[0].phase);
    let (to, value, data) = f.home_calls.last().unwrap().clone();
    assert_eq!((to, value, sel(&data)), (MODULE, out, ef::FUND_RESERVE), "Milady's module, fundReserve");
    assert!(!f.home_calls.iter().any(|(t, _, d)| *t == MODULE2 && sel(d) == ef::FUND_RESERVE), "nothing of it went to Azuki's module");
    // then Azuki's (started in the same tick, once Milady's sweep had ended), to Azuki's module
    let s2 = st.buys.last().unwrap().clone();
    assert!(s2.is_sweep() && s2.collection == AZUKI && s2.module == MODULE2, "{s2:?}");
    assert!(matches!(s2.phase, BPhase::BackFunded { wei, .. } if wei == 100 * E / 1_000));
}

/// The sweep never spends what is anyone's: an arrived buy's ETH, an open delivery, a stray going back, the line; only the balance
/// over the target, and nothing under SWEEP_MIN.
#[test]
fn a2_l_1_the_sweep_never_touches_earmarked_eth() {
    // an arrived buy holds its ETH: no sweep, however large the leftover
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    f.listing_total = 2 * E;
    st.credit_leftover(&MILADY, E);
    f.eth_balance += E;
    tick(&mut f, &mut st);
    assert!(!st.buys.iter().any(|b| b.is_sweep()), "an arrived buy's ETH is never swept");
    // only the balance over the target goes, and the rest of the leftover waits
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    st.credit_leftover(&MILADY, E);
    f.eth_balance = 150 * E / 1_000;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::BackFunded { wei, .. } if wei == 100 * E / 1_000), "{:?}", phase(&st));
    assert_eq!(st.leftover_of(&MILADY), 900 * E / 1_000);
    // under SWEEP_MIN over the target: nothing
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    st.credit_leftover(&MILADY, E);
    f.eth_balance = en::GAS_POT_TARGET_WEI + ef::SWEEP_MIN_WEI - 1;
    tick(&mut f, &mut st);
    assert!(st.buys.is_empty());
    // an open delivery holds the pot for its gas: nothing
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    st.credit_leftover(&MILADY, E);
    f.eth_balance = E;
    st.deliveries.push(ef::Delivery { wrapper: WRAPPER, collection: MILADY, ticket: 0, token: tok(1), destination: ALICE, deliver_by: f.now + 7 * 86_400,
        phase: ef::DPhase::Moved { tx: [9; 32], block: ETH_PIN }, note: String::new(), refused_since: None, sent: Some(true) });
    tick(&mut f, &mut st);
    assert!(!st.buys.iter().any(|b| b.is_sweep()), "an open delivery: no sweep");
}

/// A sweep that gets no quote is abandoned (it would hold every buy back), and one Relay refunds is never an arrival: both credit the
/// leftover back and wait out the cooldown; the operator's give-back refuses a sweep.
#[test]
fn a2_l_1_an_unquoted_or_refunded_sweep_ends_and_credits_its_leftover_back() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    st.credit_leftover(&MILADY, 300 * E / 1_000);
    f.eth_balance = 500 * E / 1_000;
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    assert!(ef::give_back_now(&mut st, id, 0, f.now).unwrap_err().contains("sweep"));
    f.now += ef::SWEEP_QUOTE_SECS + 1;
    tick(&mut f, &mut st);
    assert!(matches!(st.buys[0].phase, BPhase::Returned { wei: 0 }) && st.buys[0].finished());
    assert_eq!(st.leftover_of(&MILADY), 300 * E / 1_000);
    tick(&mut f, &mut st);
    assert_eq!(st.buys.len(), 1, "no new sweep inside the cooldown");
    f.now += ef::SWEEP_COOLDOWN_SECS;
    tick(&mut f, &mut st);
    let s = st.buys.last().unwrap().clone();
    assert!(s.is_sweep() && s.id != id);
    // refunded on Ethereum: it ends, never Arrived (nothing is bought with it)
    let ask = s.ask.clone().unwrap();
    ef::put_quote(&mut st, s.id, back_quote(ask.amount_wei, f.now), f.now).unwrap();
    tick(&mut f, &mut st);
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::BackBridging { .. }));
    f.eth_balance += ask.amount_wei;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Returned { wei: 0 }), "{:?}", phase(&st));
    assert_eq!(st.leftover_of(&MILADY), ask.amount_wei + en::ETH_GAS_RESERVE_WEI - 21_000 * 1_000_000_000);
    assert!(!st.buys.iter().any(|b| matches!(b.phase, BPhase::Arrived { .. })));
}

/// A fill's leftover (what arrived less the price and the fill's gas) is credited to its collection when the fill mines.
#[test]
fn a2_l_1_a_fills_leftover_is_its_collections() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    let BPhase::Buying { arrived, total, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Bought { .. }), "{:?}", phase(&st));
    assert_eq!(st.leftover_of(&MILADY), arrived - total - 21_000 * 1_000_000_000);
}

/// The operator's Ethereum provider (Dwellir, 2026-09-27) serves eth_getLogs over at most 500 blocks a request: the sale scan never asks
/// more (`eth_nft.logs_max_blocks`, 500 by default), and a chunk saved at the old 2,000 is cut to it.
#[test]
fn dwellir_the_sale_scan_never_asks_more_than_500_blocks() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    evidence(&f, &mut st);
    st.sale_scan.as_mut().unwrap()[0].chunk = 2_000;     // saved by the round-2 build
    tick(&mut f, &mut st);
    assert!(!f.log_ranges.is_empty());
    assert!(f.log_ranges.iter().all(|(a, b)| b - a + 1 <= en::SALE_SCAN_CHUNK_BLOCKS), "{:?}", f.log_ranges);
    assert_eq!(en::SALE_SCAN_CHUNK_BLOCKS, 500);
    // the bound is validated
    let mut c = f.cfg();
    c.logs_max_blocks = Some(10);
    assert!(en::validate(&c, &[chain1()], 4663).unwrap_err().contains("logs_max_blocks"));
    c.logs_max_blocks = Some(2_001);
    assert!(en::validate(&c, &[chain1()], 4663).is_err());
    c.logs_max_blocks = Some(250);
    assert_eq!(en::validate(&c, &[chain1()], 4663), Ok(()));
}

fn chain1() -> etch_autobuyer::chains::EvmChain {
    etch_autobuyer::chains::EvmChain { chain_id: 1, rpc_urls: vec!["https://a".into(), "https://b".into()], bucket: 8,
        seaport: "0x0000000000000068F116a894984e2DB1123eB395".into(),
        seaport_codehash: "0x74499ac0cce14428e4b41541d5e44f28f5a6882a1051d0118867c2a93cd5aec0".into(), max_fee_wei: 200_000_000_000,
        priority_fee_wei: 1_000_000_000, gas_cap: 400_000, fee_recipients: vec!["0x0000a26b00c1F0DF003000390027140000fAa719".into()],
        max_fee_bps: 1_500, min_order_life_s: 300, signed_zones: None, zone_registries: None }
}

