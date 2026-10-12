//! Fix round 3 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round3.md): the canister PoCs of the round-3 re-review
//! (docs/audit/2026-09-27-nft/review3/poc/B, C, E and the canister half of D) turned around. The harness is `tests/audit_nft_r2.rs`'s
//! fake world, copied whole, with the reviewers' switches: the canister's real H3 limiter (`real_outflow`: `outflow::Outflow::check`
//! and `record` as `canister::outflow_check` calls them, a freeze setting `paused`; part C), `pin_back` (the Ethereum pin one bucket
//! lower: the balance there is the one before a spend of ours that mined above it, receipts above it are not seen, the nonce is one
//! lower; part E) and `fee_of` (a mined transaction's gas; part E), and a log read that names each move's token and sender (round
//! 3's `eth_transfer_moves`; part B). Each test FAILS on the code before this round (the PoC passed there) and passes now.

#![allow(dead_code)] // the harness is tests/audit_nft.rs's, copied whole

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custodycap::{self, Book, CapPolicy};
use etch_autobuyer::ethflow::{self as ef, BPhase, EthIo, EthPin, EthPlan, EthState, OurReceipt};
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
    /// round 3 (C3-H-1): the canister's real H3 limiter
    real_outflow: Option<(etch_autobuyer::outflow::Outflow, etch_autobuyer::outflow::Limits)>,
    /// round 3 (E3-M-1): the pin moved back below a spend of ours: (the lower pin, what that spend took)
    pin_back: Option<(u64, u128)>,
    /// round 3 (E3-L-*): the next mined transaction's gas (else 21,000 gwei)
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
            receipt_err: HashMap::new(), plan_err: vec![], wait_prefix: vec![], broadcasts: vec![], listing_total: 45 * E / 100, sale_receipts, our_logs: HashMap::new(), noted: vec![],
            sale_err: vec![], sale_reads: 0, log_ranges: vec![], deposit_txs: HashMap::new(), caps: custodycap::stage0(&keys), book: Book::default(), outflow: vec![], drop_milady: false, real_outflow: None, pin_back: None,
            fee_of: None,
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
    fn outflow_check(&mut self, key: &str, c: &str, flow: Flow, amount: u128) -> Result<(), String> {
        let now = self.now;
        if let Some((o, l)) = self.real_outflow.as_mut() {
            use etch_autobuyer::outflow::Verdict;
            return match o.check(l, key, c, flow, amount, now) {
                Verdict::Allow => Ok(()),
                Verdict::Wait { why, until } => Err(format!("waits (outflow limit) until {until}: {why}")),
                // canister::outflow_check: the freeze sets PAUSED until k guardians resume the buyer
                Verdict::Frozen(cause) => {
                    self.paused = true;
                    Err(format!("refused: custody is frozen: {cause:?}"))
                }
            };
        }
        if self.wait_prefix.iter().any(|p| key.starts_with(p.as_str())) {
            Err(format!("waits (outflow limit) until {}: 3 of 3 in the last 24 h", self.now + 3_600))
        } else {
            Ok(())
        }
    }
    fn outflow_record(&mut self, key: &str, c: &str, flow: Flow, amount: u128) {
        let now = self.now;
        if let Some((o, _)) = self.real_outflow.as_mut() {
            o.record(key, c, flow, amount, now);
        }
        self.outflow.push((key.into(), flow, amount));
    }

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

    async fn eth_pin(&mut self) -> Result<EthPin, String> {
        Ok(EthPin { number: self.pin_back.map_or(ETH_PIN, |p| p.0), timestamp: self.now, base_fee: 100_000_000 })
    }
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
    /// Round 3 (B3-M-1): the same transactions, each with its token and sender (the least of its moves), as the canister's transform.
    async fn eth_transfer_moves(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<en::Move>, String> {
        self.log_ranges.push((from, to));
        let mut v: Vec<en::Move> = self.sale_receipts.iter().filter(|(_, r)| r.block >= from && r.block <= to).filter_map(|(h, r)| {
            r.logs.iter().filter(|l| l.address == collection && l.topics.len() == 4 && l.topics[0] == en::TRANSFER && l.topics[1] != l.topics[2]
                && l.topics[1] != [0; 32]).map(|l| (l.topics[1], l.topics[3])).min()
                .map(|(f, t)| en::Move { hash: *h, block: r.block, token: t, from: f[12..].try_into().unwrap() })
        }).collect();
        v.sort_by(|a, b| a.block.cmp(&b.block).then(a.hash.cmp(&b.hash)));
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
// Fix round 3: the round-3 PoCs turned around
// =====================================================================================================================================

/// The Ethereum chain's pin bucket (`chains.rs`, 8 blocks): a provider head that falls back across a bucket boundary moves the pin
/// back 8 blocks.
const BUCKET: u64 = 8;

/// Tick until `pred` holds, at most `n` times; true when it held.
fn until(f: &mut Fake, st: &mut EthState, n: usize, pred: impl Fn(&EthState) -> bool) -> bool {
    for _ in 0..n {
        if pred(st) {
            return true;
        }
        tick(f, st);
    }
    pred(st)
}

fn fresh_modules(f: &mut Fake) {
    let now = f.now;
    for m in [MODULE, MODULE2] {
        if let Some(x) = f.modules.get_mut(&m) {
            x.posted_at = now - 60;
        }
    }
}

/// Milady's buy, arrived and unable to fill, goes BackFunded after 48 h (the round-2 automatic return, D4).
fn to_back_funded(f: &mut Fake, st: &mut EthState) -> u64 {
    to_arrival(f, st);
    f.m(MODULE).net_units = 0;
    f.listing_total = 2 * E; // every listing is over what arrived
    f.now += ef::ARRIVED_BACK_SECS + 60;
    fresh_modules(f);
    tick(f, st);
    let b = buy_of(st, MILADY).unwrap();
    assert!(matches!(b.phase, BPhase::BackFunded { .. }), "{:?}", b.phase);
    b.id
}

/// … and on to BackBridging (the runner hands in the back quote, the deposit mines on Ethereum).
fn to_back_bridging(f: &mut Fake, st: &mut EthState, id: u64) -> u128 {
    let ask = st.buy(id).unwrap().ask.clone().unwrap();
    ef::put_quote(st, id, back_quote(ask.amount_wei, f.now), f.now).unwrap();
    tick(f, st);
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::BackSending { .. }), "{:?}", st.buy(id).unwrap().phase);
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    tick(f, st);
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::BackBridging { .. }), "{:?}", st.buy(id).unwrap().phase);
    ask.amount_wei
}

// ---- C3-H-1: an advance waiting on H3 is asked once, not once a tick; honest waiting never trips the freeze -----------------------

/// Stage 0's limits with the day's value scaled so that one Milady advance (0.5002 ETH) fits and a second does not (as one BAYC
/// advance of 7.35 ETH and a second against a 10 ETH day).
fn scaled_h3(f: &mut Fake) {
    let mut l = etch_autobuyer::outflow::Limits::stage0();
    l.global_value_wei = 9 * E / 10;
    f.real_outflow = Some((Default::default(), l));
}

fn value_asks(f: &Fake) -> Vec<String> {
    f.real_outflow.as_ref().unwrap().0.asked.iter().filter(|a| a.flow == Flow::Value).map(|a| a.key.clone()).collect()
}

#[test]
fn r3_c3_h1_a_waiting_advance_is_asked_once_and_never_trips_the_freeze() {
    // PoC `c3_h1_an_advance_waiting_on_the_value_limit_is_re_asked_under_a_new_key_every_tick_and_trips_the_freeze`: after Milady's
    // advance, a second advance that waits on the day's value was dropped and reopened every tick under `buy:eth:{id}`, each a new ask:
    // `Tripwire { flow: Value, demanded: 3.0012 ETH, limit: 0.9 ETH }` within four ticks, the canister paused. Now the key names the
    // collection and its count of advances: the wait is one ask however many ticks it lasts
    let mut f = Fake::new(true);
    scaled_h3(&mut f);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);          // Milady's advance went out (recorded) and arrived: the Relay leg is free
    assert_eq!(value_asks(&f).len(), 1);
    f.m(MODULE2).net_units = 2 * E;       // Azuki wants its item: its advance does not fit the day's value
    for _ in 0..200 {
        f.now += 60;
        fresh_modules(&mut f);
        tick(&mut f, &mut st);
    }
    let (o, _) = f.real_outflow.as_ref().unwrap();
    println!("C3-H-1: H3 asked {:?}; frozen {:?}", value_asks(&f), o.frozen());
    assert!(o.frozen().is_none() && !f.paused, "honest waiting never trips the freeze");
    assert_eq!(value_asks(&f).len(), 2, "Milady's advance and Azuki's one waiting ask, not one a tick: {:?}", value_asks(&f));
    let az_key = format!("advance:eth:{}:0", custodycap::evm_key(1, &AZUKI));
    assert!(value_asks(&f).contains(&az_key), "{:?}", value_asks(&f));
    assert!(!st.relay_busy() || buy_of(&st, AZUKI).is_some(), "the waiting buy never holds the Relay leg");
    // a day on, the window has slid: Azuki's advance goes out under the same key, counted once
    f.now += 86_400;
    fresh_modules(&mut f);
    assert!(until(&mut f, &mut st, 4, |s| buy_of(s, AZUKI).is_some_and(|b| b.committed())), "{:?}", buy_of(&st, AZUKI));
    let (o, _) = f.real_outflow.as_ref().unwrap();
    assert!(o.done.iter().any(|a| a.key == az_key) && o.frozen().is_none());
}

#[test]
fn r3_c3_h1_an_advance_that_came_back_is_not_re_asked_every_tick_either() {
    // the PoC's own path: Relay refunds three times, the advance goes back (Returned) while the day's record stays, and the module
    // still wants its item. The collection now waits a day (A3-L-1), and when it asks again it asks once
    let mut f = Fake::new(false);
    scaled_h3(&mut f);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    assert!(until(&mut f, &mut st, 4, |s| s.buys.last().is_some_and(|b| matches!(b.phase, BPhase::Funded { .. }))));
    let id = st.buys[0].id;
    for _ in 0..3 {
        st.used_orders.clear(); // harness: the one measured quote, re-used
        ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
        assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(id).unwrap().phase, BPhase::Bridging { .. })), "{:?}", phase(&st));
        f.home_balance += DEPOSIT; // Relay's refund
        assert!(until(&mut f, &mut st, 2, |s| matches!(s.buy(id).unwrap().phase, BPhase::Funded { .. } | BPhase::Returning { .. } | BPhase::Returned { .. })));
        f.now += 600;
    }
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { .. })), "{:?}", phase(&st));
    let mut waited = false;
    for _ in 0..100 {
        f.now += 60;
        fresh_modules(&mut f);
        waited |= tick(&mut f, &mut st).iter().any(|l| l.contains("no buy until"));
    }
    let (o, _) = f.real_outflow.as_ref().unwrap();
    assert!(o.frozen().is_none() && !f.paused, "not frozen: {:?}", o.frozen());
    assert!(waited && st.buys.len() == 1, "the collection waits out the cooldown instead of re-asking every tick");
    assert_eq!(value_asks(&f).len(), 1);
}

// ---- C3-M-1: the way back has bounded exits; one collection's stuck return never blocks the others ---------------------------------

#[test]
fn r3_c3_m1_a_way_back_with_no_quote_frees_the_other_collections_within_its_window() {
    // PoC `c3_m1_a_way_back_with_no_quote_stops_every_collections_buys_and_nothing_moves_it`: 7 days BackFunded, Azuki never bought,
    // put_listing, eth_give_back and eth_line_clear all refused
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let id = to_back_funded(&mut f, &mut st);
    let went = f.now;
    // the operator's exit exists now: a way back with no deposit waits at Arrived again
    let mut op = st.clone();
    assert!(ef::give_back_now(&mut op, id, 10 * E, f.now).unwrap().contains("stopped"));
    assert!(matches!(op.buy(id).unwrap().phase, BPhase::Arrived { .. }));
    // and without it: Azuki wants an item; within the way back's quote window nothing starts (it is next on the leg), after it Azuki buys
    f.m(MODULE2).net_units = 2 * E;
    let mut az_started = None;
    for _ in 0..(12 * 3) {
        f.now += 1_200;
        fresh_modules(&mut f);
        tick(&mut f, &mut st);
        if az_started.is_none() && buy_of(&st, AZUKI).is_some() {
            az_started = Some(f.now);
        }
    }
    let t = az_started.expect("Azuki's buy starts");
    assert!(t >= went + ef::BACK_QUOTE_SECS && t <= went + ef::BACK_QUOTE_SECS + 2 * 1_200, "at the end of the window: {}", t - went);
    let m = st.buy(id).unwrap().clone();
    assert!(matches!(m.phase, BPhase::Arrived { .. }), "{:?}", m.phase);
    assert!(m.back_at.unwrap() >= went + ef::BACK_QUOTE_SECS + ef::BACK_RETRY_SECS, "it goes back again a day later");
    assert!(ef::put_listing(&mut st, id, fill_hex(), f.now).is_ok(), "and it may take a listing again meanwhile");
    assert!(st.earmarked_except(u64::MAX) <= f.eth_balance, "its ETH is still its own, and nothing was signed twice");
}

#[test]
fn r3_c3_m1_a_way_back_relay_neither_fills_nor_refunds_frees_the_leg_and_the_operator_closes_it() {
    // PoC `c3_m1_a_way_back_past_its_deadline_with_no_fill_or_refund_holds_the_relay_leg_for_good`: 3 days past the deadline still
    // BackBridging, relay_busy, Azuki never bought, eth_give_back refused
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let id = to_back_funded(&mut f, &mut st);
    to_back_bridging(&mut f, &mut st, id);
    let BPhase::BackBridging { deadline, .. } = st.buy(id).unwrap().phase else { unreachable!() };
    // the operator may write it off once the canister has watched it past its deadline, the grace and a day, not before
    assert!(ef::give_back_now(&mut st.clone(), id, 10 * E, f.now).unwrap_err().contains("watched until"));
    f.m(MODULE2).net_units = 2 * E;
    // review round 4 (C4-M-1): round 3 let the way back go by itself at deadline + grace + BACK_LATE_SECS, and a payout later than that
    // was read as the next order's refund or arrival. It now stays watched (it holds the Relay leg) until its payout is read or the
    // operator writes it off
    while f.now < deadline + 3 * 86_400 {
        f.now += 1_200;
        fresh_modules(&mut f);
        tick(&mut f, &mut st);
        assert!(matches!(st.buy(id).unwrap().phase, BPhase::BackBridging { .. }) && st.relay_busy(), "still watched");
    }
    assert!(buy_of(&st, AZUKI).is_none(), "no other order starts while its payout could still land");
    // the operator writes it off (Relay says nothing more will come): off the leg, its watch kept for a late payout
    assert!(ef::give_back_now(&mut st, id, 10 * E, f.now).unwrap().contains("written off"));
    assert!(st.buy(id).unwrap().last.is_none() && st.buy(id).unwrap().late.is_some(), "no longer watched; the order kept");
    // review round 5 (C5-M-1): the written-off order still holds the Relay leg for new orders until a payout of it is credited or the
    // operator releases it (its payout could land and be read as the next order's)
    assert!(!until(&mut f, &mut st, 6, |s| buy_of(s, AZUKI).is_some()) && st.relay_busy(), "no new order while it may still be paid");
    ef::late_release(&mut st, id, f.now).unwrap();
    assert!(until(&mut f, &mut st, 6, |s| buy_of(s, AZUKI).is_some()), "Azuki buys again once the operator released it");
    // the operator closes it once Relay says the order is dead: the advance is returned as a loss, the module's transit ends
    let l = ef::give_back_now(&mut st, id, 10 * E, f.now).unwrap();
    assert!(l.contains("closed as a loss"), "{l}");
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { wei: 0 })), "{:?}", st.buy(id).unwrap().phase);
    assert!(f.home_calls.iter().any(|(to, v, d)| *to == MODULE && *v == 0 && sel(d) == rc::RETURN_ADVANCE));
    assert!(!f.m(MODULE).transit);
    assert!(ef::stranded_by(&st, Some(&f.cfg())).is_empty());
}

#[test]
fn r3_c3_m1_the_operator_may_stop_watching_a_late_way_back_at_once() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let id = to_back_funded(&mut f, &mut st);
    to_back_bridging(&mut f, &mut st, id);
    let BPhase::BackBridging { deadline, .. } = st.buy(id).unwrap().phase else { unreachable!() };
    // review round 4 (C4-M-1): not at the deadline and the grace any more (Relay refunds after the deadline, and a refund landing after
    // the write-off was read as the next order's), but once the canister has watched it a day past them
    f.now = deadline + ef::DEADLINE_GRACE_SECS + 1;
    assert!(ef::give_back_now(&mut st.clone(), id, 10 * E, f.now).unwrap_err().contains("watched until"));
    f.now = deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 1;
    assert!(ef::give_back_now(&mut st, id, 10 * E, f.now).unwrap().contains("no longer watched"));
    // review round 5 (C5-M-1): off its watch, but still holding the Relay leg for new orders until credited or released
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::Stuck { .. }) && st.relay_busy() && st.late_hold() == Some(id));
    ef::late_release(&mut st, id, f.now).unwrap();
    assert!(!st.relay_busy());
}

// ---- E3-M-1: every arrival read has the pin floor --------------------------------------------------------------------------------

/// Milady arrives and its fill is signed; meanwhile Azuki's buy starts. The fill mines 3 blocks under the pin and is READ and COUNTED
/// (eth_out) at the pin; Azuki is then Funded. Returns what the fill took from our Ethereum balance (review3/poc/E's).
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
    tick(f, st);
    assert!(matches!(buy_of(st, MILADY).unwrap().phase, BPhase::Bought { .. }), "the fill is read and counted at the pin");
    assert_eq!(st.last_mined.map(|x| x.1), Some(ETH_PIN - 3));
    assert!(matches!(buy_of(st, AZUKI).unwrap().phase, BPhase::Funded { .. }), "{:?}", buy_of(st, AZUKI).unwrap().phase);
    spent
}

#[test]
fn r3_e3_m1_a_pin_that_moves_back_no_longer_reads_a_counted_fill_as_another_collections_arrival() {
    // PoC `e3_m1_a_pin_that_moves_back_reads_a_counted_fill_as_another_collections_arrival`: Azuki Arrived on Milady's spent fill,
    // earmarks over the balance, the 4663 refund unseen, the way back stuck
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let spent = milady_fill_counted_azuki_funded(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Bridging { .. }), "{:?}", buy_of(&st, AZUKI).unwrap().phase);
    f.pin_back = Some((ETH_PIN - BUCKET, spent));
    let l = tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Bridging { .. }), "no phantom arrival: {:?}", buy_of(&st, AZUKI).unwrap().phase);
    assert!(l.iter().any(|x| x.contains("E3-M-1")), "{l:?}");
    f.pin_back = None;
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Bridging { .. }));
    assert!(st.earmarked_except(u64::MAX) <= f.eth_balance, "the earmarks never exceed the balance");
    // Relay's real fill arrives and is Azuki's, at its size
    f.eth_balance += ARRIVED;
    tick(&mut f, &mut st);
    let BPhase::Arrived { wei, .. } = buy_of(&st, AZUKI).unwrap().phase else { panic!("{:?}", buy_of(&st, AZUKI).unwrap().phase) };
    assert!(wei >= ARRIVED - 1_000_000_000_000_000 && wei <= ARRIVED + 1_000_000_000_000_000, "{wei}");
    assert!(st.earmarked_except(u64::MAX) <= f.eth_balance);
}

#[test]
fn r3_e3_m1_a_watch_is_never_taken_at_a_pin_that_moved_back() {
    // PoC `e3_m1_a_watch_taken_at_a_pin_that_moved_back_never_sees_the_real_arrival_and_holds_the_relay_leg`
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let spent = milady_fill_counted_azuki_funded(&mut f, &mut st);
    let az = buy_of(&st, AZUKI).unwrap();
    st.used_orders.clear();
    ef::put_quote(&mut st, az.id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    f.pin_back = Some((ETH_PIN - BUCKET, spent));
    let l = tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Funded { .. }), "the quote waits: {:?}", buy_of(&st, AZUKI).unwrap().phase);
    assert!(st.inbox.quotes.iter().any(|(id, ..)| *id == az.id) && l.iter().any(|x| x.contains("E3-M-1")), "{l:?}");
    f.pin_back = None;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let b = buy_of(&st, AZUKI).unwrap();
    assert!(matches!(b.phase, BPhase::Bridging { .. }), "{:?}", b.phase);
    let w = b.last.clone().unwrap();
    assert_eq!((w.eth_before, w.pin), (f.eth_balance, Some(ETH_PIN)), "the watch starts at the real balance, at the pin");
    f.eth_balance += ARRIVED;
    tick(&mut f, &mut st);
    assert!(matches!(buy_of(&st, AZUKI).unwrap().phase, BPhase::Arrived { .. }), "the real arrival is seen");
    assert!(!st.relay_busy());
}

// ---- B3-M-1: a flood of plain transfers no longer starves real sales ---------------------------------------------------------------

/// A plain ERC-721 Transfer of `collection` (no marketplace log): not a sale, but a `Transfer` log the scan finds (review3/poc/B's).
fn transfer_receipt(collection: [u8; 20], token: u128, from: [u8; 20], to: [u8; 20], block: u64) -> ReceiptV {
    ReceiptV { block, success: true, logs: vec![
        LogV { address: collection, topics: vec![en::TRANSFER, sp::word_addr(&from), sp::word_addr(&to), tok(token)], data: vec![] },
    ] }
}

/// `n` plain transfers just under the pin (all newer than every real sale). `shape`: "pingpong" (one token between two wallets, the
/// PoC's), "one-wallet" (many tokens out of one wallet), "chain" (one token passed along fresh wallets), "fresh" (a fresh token and a
/// fresh sender every time: n NFTs, the strongest).
fn flood(f: &mut Fake, first_byte: u8, n: usize, shape: &str, newer: bool) -> Vec<[u8; 32]> {
    let top = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let mut hs = Vec::new();
    for i in 0..n {
        // distinct in the first 8 bytes too (the scan keeps a judged transaction by its first 8 bytes)
        let mut h = [first_byte; 32];
        h[..4].copy_from_slice(&(i as u32).to_be_bytes());
        h[28..].copy_from_slice(&(i as u32).to_be_bytes());
        let block = if newer { top - (i as u64 % 1_999) } else { 26_055_000 - i as u64 };
        let w = |k: usize| { let mut a = [0x60u8; 20]; a[16..].copy_from_slice(&(k as u32).to_be_bytes()); a };
        let r = match shape {
            "pingpong" => if i % 2 == 0 { transfer_receipt(MILADY, 4_242, ATTACKER, OTHER, block) } else { transfer_receipt(MILADY, 4_242, OTHER, ATTACKER, block) },
            "one-wallet" => transfer_receipt(MILADY, 10_000 + i as u128, ATTACKER, w(i), block),
            "chain" => transfer_receipt(MILADY, 4_243, w(i), w(i + 1), block),
            _ => transfer_receipt(MILADY, 20_000 + i as u128, w(i), w(100_000 + i), block),
        };
        f.sale_receipts.insert(h, r);
        hs.push(h);
    }
    hs
}

fn cached_reals(st: &EthState) -> usize {
    let reals = real_sales();
    st.sale_cache.iter().flatten().filter(|c| c.collection == MILADY && reals.contains(&c.hash) && !c.sales.is_empty()).count()
}

#[test]
fn r3_b3_m1_a_plain_transfer_flood_no_longer_starves_real_sales_or_deposits() {
    // PoC `b3_m1_a_plain_transfer_flood_permanently_starves_real_sales_holds_the_floor_and_stops_deposit_mints`: 520 plain transfers
    // newer than the real sales filled the 512 queue; the real sales were truncated out and never read again
    for shape in ["pingpong", "one-wallet", "chain", "fresh"] {
        let mut f = Fake::new(false);
        f.deposits_on = true;
        let mut st = EthState::default();
        deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 1);
        ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
        f.finalized = ETH_PIN;
        f.m(MODULE).accepted = 93 * E / 100;
        f.m(MODULE).bounds = (0, u128::MAX);
        flood(&mut f, 0xc0, 520, shape, true);
        let mut passes = 0;
        while cached_reals(&st) < 5 && passes < 60 {
            f.now += 600;
            f.m(MODULE).posted_at = f.now - 60;
            evidence(&f, &mut st);
            tick(&mut f, &mut st);
            passes += 1;
        }
        for _ in 0..3 {
            f.now += 600;
            f.m(MODULE).posted_at = f.now - 60;
            evidence(&f, &mut st);
            tick(&mut f, &mut st);
        }
        let r = st.reading(&MILADY).unwrap().post.clone();
        println!("B3-M-1 {shape}: every real sale read in {passes} passes; counted {}, held {:?}, minted {:?}", r.sales, r.held, f.items);
        assert_eq!(cached_reals(&st), 5, "{shape}: every real sale is read");
        // the flood's newer transactions come after them in the order: one per token and per sender first, oldest first
        assert!(passes <= if shape == "fresh" { 4 } else { 3 }, "{shape}: read before the flood, within a few passes: {passes}");
        assert_eq!(r.sales, 5, "{shape}");
        assert!(r.held.is_none(), "{shape}: the floor is confirmed");
        assert_eq!(f.items, vec![tok(950)], "{shape}: the deposit mints");
    }
}

#[test]
fn r3_b3_m1_a_flood_at_a_collections_first_post_no_longer_keeps_it_unposted() {
    // PoC `b3_m1b_at_first_post_accepted_zero_the_flood_blocks_the_floor_post_entirely_module_never_gets_a_floor`
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 0;
    f.m(MODULE).bounds = (0, u128::MAX);
    flood(&mut f, 0xd0, 520, "pingpong", true);
    for _ in 0..8 {
        f.now += 600;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    assert_eq!(cached_reals(&st), 5);
    assert!(!f.m(MODULE).posted.is_empty(), "the module gets its first floor");
}

#[test]
fn r3_b3_m1_judged_transfers_without_a_sale_never_push_a_sale_out_of_the_cache() {
    // the PoC's second route: the cache kept the newest 2,048 judged transactions, a sale or not, so judged flood entries pushed
    // cached real sales out. Now a sale is never evicted by a transaction without one
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        f.now += 600;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    assert_eq!(cached_reals(&st), 5);
    // 2,100 judged no-sale entries newer than every real sale (round 2's shape: kept in sale_cache)
    let top = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let c = st.sale_cache.get_or_insert_with(Vec::new);
    for i in 0..2_100u32 {
        let mut h = [0xe0u8; 32];
        h[..4].copy_from_slice(&i.to_be_bytes());
        c.push(ef::CachedSales { collection: MILADY, hash: h, block: top - (i as u64 % 2_000), sales: vec![] });
    }
    f.now += 600;
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    assert_eq!(cached_reals(&st), 5, "every cached real sale is kept");
    assert_eq!(st.reading(&MILADY).unwrap().post.sales, 5);
}

#[test]
fn r3_b3_m1_a_sale_dropped_from_a_full_queue_is_read_again() {
    // "rescan truncated ranges": a queue over SALE_TODO_MAX drops the last in the order and reads their blocks again once it has room.
    // 600 transactions older than the real sales, each a fresh token and sender (600 NFTs), rank ahead of them; the real sales past the
    // 512th are dropped, and read later from the blocks read again
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    let hs = flood(&mut f, 0xa0, 600, "fresh", false);
    let top = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let mut pend: Vec<en::Move> = hs.iter().map(|h| {
        let r = &f.sale_receipts[h];
        en::Move { hash: *h, block: r.block, token: r.logs[0].topics[3], from: r.logs[0].topics[1][12..].try_into().unwrap() }
    }).collect();
    for h in real_sales() {
        let r = &f.sale_receipts[&h];
        let l = r.logs.iter().find(|l| l.address == MILADY && l.topics.len() == 4 && l.topics[0] == en::TRANSFER).unwrap();
        pend.push(en::Move { hash: h, block: r.block, token: l.topics[3], from: l.topics[1][12..].try_into().unwrap() });
    }
    // the window's blocks are read already; everything found is queued
    st.sale_scan = Some(vec![ef::SaleScan { collection: MILADY, down: top - en::SALE_WINDOW_BLOCKS, up: top, chunk: 500, todo: vec![],
        pend: Some(pend), rescan: None, judged: None, bad: None, skipped: None, done_big: None, stuck: None, spent: None, hole: None, lost: None, behind: None, rfail: None, raw: None, current: None, hole_deep: None, rbusy: None, busy: None }]);
    let (mut dropped, mut again) = (false, false);
    let mut passes = 0;
    while cached_reals(&st) < 5 && passes < 80 {
        f.now += 600;
        evidence(&f, &mut st);
        for l in tick(&mut f, &mut st) {
            dropped |= l.contains("dropped: blocks");
            again |= l.contains("to read again");
        }
        passes += 1;
    }
    println!("B3-M-1 rescan: every real sale read after {passes} passes (dropped {dropped}, read again {again})");
    assert!(dropped && again, "the queue was over its bound and its dropped blocks were read again");
    assert_eq!(cached_reals(&st), 5, "no real sale is lost for good");
    assert!(st.sale_scan.as_ref().unwrap()[0].rescan.is_none() || passes < 80);
}

#[test]
fn r3_b3_m1_the_order_reads_one_move_per_token_and_sender_first_oldest_first() {
    let m = |b: u64, t: u8, f: u8, h: u8| en::Move { hash: [h; 32], block: b, token: [t; 32], from: [f; 20] };
    // a flood of one token (t=9) and one sender (f=9), newer and older than a real sale (t=1, f=1) and another (t=2, f=2)
    let v = vec![m(10, 9, 9, 1), m(11, 9, 8, 2), m(12, 7, 9, 3), m(50, 1, 1, 4), m(5, 2, 2, 5), m(60, 9, 9, 6)];
    let o: Vec<u8> = ef::sale_order(v).iter().map(|x| x.hash[0]).collect();
    // rank 1: the oldest of each token and sender: (5: t2 f2), (10: t9 f9), (50: t1 f1); then rank 2: (11: t9#2), (12: f9#2); rank 3: 60
    assert_eq!(o, vec![5, 1, 4, 2, 3, 6]);
}

// ---- C3-L-3: the way back's arrival is the order's minimum, never half of it -------------------------------------------------------

#[test]
fn r3_c3_l3_a_4663_inflow_during_the_way_back_is_not_its_arrival() {
    // PoC `c3_l2_a_4663_inflow_during_the_way_back_is_returned_as_the_arrival_and_the_real_arrival_is_orphaned`
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let id = to_back_funded(&mut f, &mut st);
    let amount = to_back_bridging(&mut f, &mut st, id);
    let BPhase::BackBridging { min_out, .. } = st.buy(id).unwrap().phase else { unreachable!() };
    let other = min_out / 2 + 1;
    f.home_balance += other;
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::BackBridging { .. }), "a donation of half the minimum is not the arrival");
    let payout = amount / 10_000 * 9_994;
    f.home_balance += payout;
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { .. })));
    let BPhase::Returned { wei } = st.buy(id).unwrap().phase else { unreachable!() };
    assert_eq!(wei, amount, "Relay's payout (with the donation) is returned, never more than the order sent");
}

// ---- A3-L-1 / C3-L-2: no churn of a module's ETH through Relay round trips ----------------------------------------------------------

#[test]
fn r3_a3_l1_c3_l2_a_returned_advance_waits_a_day_and_the_operator_cannot_send_an_arrival_back_at_once() {
    // PoCs `a3_m1_the_way_back_repeats_without_bound_…` and `c3_m2_the_runners_operator_key_churns_…`: 3 to 5 round trips in minutes
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    let id = st.buys[0].id;
    assert!(ef::give_back_now(&mut st.clone(), id, 0, f.now).unwrap_err().contains("C3-L-2"), "not at once");
    f.now += ef::ARRIVED_HOLD_SECS;
    fresh_modules(&mut f);
    ef::give_back_now(&mut st, id, 0, f.now).unwrap();
    tick(&mut f, &mut st);
    let amount = to_back_bridging(&mut f, &mut st, id);
    f.home_balance += amount / 10_000 * 9_994;
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { .. })));
    let back = f.now;
    f.m(MODULE).net_units = 2 * E; // the module still wants its item
    for _ in 0..(23 * 3) {
        f.now += 1_200;
        fresh_modules(&mut f);
        tick(&mut f, &mut st);
    }
    assert_eq!(st.buys.len(), 1, "no new advance within the day");
    f.now = back + ef::RETURN_COOLDOWN_SECS + 60;
    fresh_modules(&mut f);
    tick(&mut f, &mut st);
    assert_eq!(st.buys.len(), 2, "a day later the collection buys again");
}

// ---- E3-L-1: the deposit hold and the 48 h are counted from the first arrival ------------------------------------------------------

#[test]
fn r3_e3_l1_a_fill_that_reverts_no_longer_restarts_the_hold_or_the_way_back() {
    // PoC `e3_l1_a_fill_that_reverts_restarts_the_arrived_hold_so_a_seller_can_hold_deposits_and_defer_the_way_back_indefinitely`
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
    let (mut reverts, mut minted_at, mut back_at) = (0, None, None);
    for step in 0..(5 * 24 * 3) {
        f.now += 1_200;
        f.m(MODULE).posted_at = f.now - 60;
        evidence(&f, &mut st);
        if step % 15 == 0 && matches!(phase(&st), BPhase::Arrived { .. }) {
            ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
        }
        tick(&mut f, &mut st);
        if let BPhase::Buying { .. } = phase(&st) {
            let i = f.eth_signed.len() - 1;
            f.mine_eth(i, false, ETH_PIN - 1);
            reverts += 1;
        }
        if minted_at.is_none() && !f.items.is_empty() {
            minted_at = Some(f.now);
        }
        if back_at.is_none() && matches!(phase(&st), BPhase::BackFunded { .. }) {
            back_at = Some(f.now);
        }
    }
    println!("E3-L-1: {reverts} reverted fills; minted at +{:?} s; went back at +{:?} s", minted_at.map(|t| t - arrived_at), back_at.map(|t| t - arrived_at));
    assert!(reverts >= 3);
    let m = minted_at.expect("the holder's deposit mints");
    assert!(m <= arrived_at + ef::ARRIVED_HOLD_SECS + 2 * 1_200, "within the hold of the FIRST arrival");
    let b = back_at.expect("the way back starts");
    assert!(b >= arrived_at + ef::ARRIVED_BACK_SECS && b <= arrived_at + ef::ARRIVED_BACK_SECS + 5 * 3_600, "48 h from the first arrival");
}

// ---- E3-L-3 / C3-I-3: a post-arrival Stuck buy, and dust, close the books ----------------------------------------------------------

#[test]
fn r3_e3_l3_a_post_arrival_stuck_buy_is_closed_by_the_operator() {
    // PoC `e3_l3_a_post_arrival_stuck_buy_is_open_for_ever_with_no_exit`
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    let i = f.eth_signed.len() - 1;
    f.mine_eth(i, true, ETH_PIN - 10);
    f.owners.insert((MILADY, tok(9_180)), [0x0e; 20]); // the fill mined, the token is not ours
    tick(&mut f, &mut st);
    let id = buy_of(&st, MILADY).unwrap().id;
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::Stuck { .. }) && st.buy(id).unwrap().last.is_none());
    assert!(!ef::stranded_by(&st, None).is_empty());
    assert!(ef::give_back_now(&mut st, id, 0, f.now).unwrap().contains("closed as a loss"));
    assert!(until(&mut f, &mut st, 4, |s| s.buy(id).unwrap().finished()), "{:?}", st.buy(id).unwrap().phase);
    assert!(!f.m(MODULE).transit, "the module's transit ended");
    f.drop_milady = true;
    assert!(ef::stranded_by(&st, Some(&f.cfg())).iter().all(|x| !x.contains(&format!("buy {id} "))), "no longer pins its collection");
}

#[test]
fn r3_c3_i3_dust_never_wedges_the_way_back() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    let id = st.buys[0].id;
    f.now += ef::ARRIVED_HOLD_SECS;
    if let BPhase::Arrived { wei, .. } = &mut st.buys[0].phase {
        *wei = en::ETH_GAS_RESERVE_WEI;
    }
    assert!(ef::give_back_now(&mut st.clone(), id, 0, f.now).unwrap_err().contains("gas reserve"), "the operator cannot wedge it");
    // at 48 h dust is kept as leftover and the advance returned as the loss it is
    f.now += ef::ARRIVED_BACK_SECS;
    fresh_modules(&mut f);
    assert!(until(&mut f, &mut st, 4, |s| s.buy(id).unwrap().finished()), "{:?}", st.buy(id).unwrap().phase);
    assert_eq!(st.leftover_of(&MILADY), en::ETH_GAS_RESERVE_WEI);
}
