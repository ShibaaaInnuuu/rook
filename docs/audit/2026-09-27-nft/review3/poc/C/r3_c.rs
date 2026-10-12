//! Round 3 of the 2026-09-27 NFT review, part C (docs/audit/2026-09-27-nft/review3/R3-canister-wayback.md): PoCs against the
//! round-2 advance gate and the way back (Relay Ethereum -> 4663). The harness is tests/audit_nft_r2.rs's fake world, copied whole
//! (lines 1-546 at 4949da0). Each test PASSES while the weakness it names is present.

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
    /// round 3 (C3-H-1): the canister's real H3 limiter (`outflow::Outflow::check` / `record`, as `canister::outflow_check` calls it)
    real_outflow: Option<(etch_autobuyer::outflow::Outflow, etch_autobuyer::outflow::Limits)>,
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
            sale_err: vec![], sale_reads: 0, deposit_txs: HashMap::new(), caps: custodycap::stage0(&keys), book: Book::default(), outflow: vec![], drop_milady: false, real_outflow: None,
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
        let now = self.now;
        if let Some((o, l)) = self.real_outflow.as_mut() {
            use etch_autobuyer::outflow::Verdict;
            return match o.check(l, key, _c, _flow, _amount, now) {
                Verdict::Allow => Ok(()),
                Verdict::Wait { why, until } => Err(format!("waits (outflow limit) until {until}: {why}")),
                // canister::outflow_check: PAUSED = true, "the buyer is paused until k guardians resume it"
                Verdict::Frozen(cause) => { self.paused = true; Err(format!("refused: custody is frozen: {cause:?}")) }
            };
        }
        if self.wait_prefix.iter().any(|p| key.starts_with(p.as_str())) {
            Err(format!("waits (outflow limit) until {}: 3 of 3 in the last 24 h", self.now + 3_600))
        } else {
            Ok(())
        }
    }
    fn outflow_record(&mut self, key: &str, _c: &str, flow: Flow, amount: u128) {
        let now = self.now;
        if let Some((o, _)) = self.real_outflow.as_mut() {
            o.record(key, _c, flow, amount, now);
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

// ---- A-M-1: the deposit band is the canister's own sample, and washes cannot open it -------------------------------------------------


// =====================================================================================================================================
// Round 3, part C: the advance gate and the way back
// =====================================================================================================================================

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

/// Milady's buy, arrived and unable to fill, goes BackFunded after 48 h (the round-2 auto return).
fn to_back_funded(f: &mut Fake, st: &mut EthState) -> u64 {
    to_arrival(f, st);
    f.m(MODULE).net_units = 0;
    f.listing_total = 2 * E; // every listing is over what arrived
    f.now += ef::ARRIVED_BACK_SECS + 60;
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

fn fresh_modules(f: &mut Fake) {
    let now = f.now;
    for m in [MODULE, MODULE2] {
        if let Some(x) = f.modules.get_mut(&m) {
            x.posted_at = now - 60;
        }
    }
}

// ---- C3-M-1 (a): a way back that never gets a judgeable quote stops EVERY collection's buys, with no exit ---------------------------

#[test]
fn c3_m1_a_way_back_with_no_quote_stops_every_collections_buys_and_nothing_moves_it() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let id = to_back_funded(&mut f, &mut st);
    // Azuki now wants an item and has every room; the runner hands in no Ethereum -> 4663 quote (Relay has no route or solver
    // inventory for the amount, its fee is over relay_max_fee_bps, or the runner withholds it)
    f.m(MODULE2).net_units = 2 * E;
    let start = f.now;
    let mut refused_listing = String::new();
    for _ in 0..(7 * 24 * 3) {
        f.now += 1_200;
        fresh_modules(&mut f);
        // a listing under the cap reappears: it cannot be handed in any more (the buy is no longer Arrived)
        if let Err(e) = ef::put_listing(&mut st, id, fill_hex(), f.now) {
            refused_listing = e;
        }
        tick(&mut f, &mut st);
    }
    println!("C3-M-1a: after {} days: Milady {:?}; Azuki's buy {:?}", (f.now - start) / 86_400, st.buy(id).unwrap().phase,
        buy_of(&st, AZUKI).map(|b| b.phase));
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::BackFunded { .. }), "still waiting for a quote");
    assert!(buy_of(&st, AZUKI).is_none(), "Azuki wants an item and has room, but no buy of ANY collection starts");
    assert!(!st.relay_busy(), "BackFunded does not even hold the Relay leg: the block is buy_pass's BackFunded rule");
    assert!(refused_listing.contains("only once its ETH arrived"), "{refused_listing}");
    // no operator exit: eth_give_back refuses BackFunded (only Funded, Arrived, Stuck-while-bridging)
    let e = ef::give_back_now(&mut st.clone(), id, 10 * E).unwrap_err();
    assert!(e.contains("refused"), "{e}");
    assert!(ef::line_clear(&mut st.clone()).is_err(), "nor eth_line_clear (nothing is halted)");
    // control: the same world with Milady still Arrived (no way back) starts Azuki's buy at once
    let mut ctl = st.clone();
    ctl.buy_mut(id).unwrap().phase = BPhase::Arrived { wei: ARRIVED, since: f.now };
    tick(&mut f, &mut ctl);
    assert!(buy_of(&ctl, AZUKI).is_some(), "control: only the way back blocks Azuki");
}

// ---- C3-M-1 (b): a way back Relay neither fills nor refunds holds the Relay leg for good, with no exit ------------------------------

#[test]
fn c3_m1_a_way_back_past_its_deadline_with_no_fill_or_refund_holds_the_relay_leg_for_good() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let id = to_back_funded(&mut f, &mut st);
    to_back_bridging(&mut f, &mut st, id);
    let BPhase::BackBridging { deadline, .. } = st.buy(id).unwrap().phase else { unreachable!() };
    f.m(MODULE2).net_units = 2 * E;
    let mut last = Vec::new();
    while f.now < deadline + 3 * 86_400 {
        f.now += 1_200;
        fresh_modules(&mut f);
        last = tick(&mut f, &mut st);
    }
    println!("C3-M-1b: {} days past the order's deadline: Milady {:?}; Azuki {:?}", (f.now - deadline) / 86_400,
        st.buy(id).unwrap().phase, buy_of(&st, AZUKI).map(|b| b.phase));
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::BackBridging { .. }));
    assert!(last.iter().any(|l| l.contains("the operator looks; the leg stays watched")), "{last:?}");
    assert!(st.relay_busy(), "the one Relay leg is held");
    assert!(buy_of(&st, AZUKI).is_none(), "no other collection ever buys");
    let e = ef::give_back_now(&mut st.clone(), id, 10 * E).unwrap_err();
    assert!(e.contains("refused"), "no operator exit for BackBridging: {e}");
    // the forward leg's analogue (Bridging past its deadline -> Stuck, still watched) HAS an exit
    let mut fwd = st.clone();
    let w = ef::Watch { order_id: [1; 32], value: DEPOSIT, min_out: ARRIVED, deadline, balance_after: 0, eth_before: 0, eth_out_before: 0 };
    let b = fwd.buy_mut(id).unwrap();
    b.phase = BPhase::Stuck { why: "past the deadline".into() };
    b.last = Some(w);
    assert!(ef::give_back_now(&mut fwd, id, 10 * E).is_ok(), "control: a stuck forward leg can be given back");
}

// ---- C3-M-2: the operator key (the runner's hot identity) churns a module's ETH through Relay round trips, each one a loss ----------

#[test]
fn c3_m2_the_runners_operator_key_churns_a_modules_eth_through_relay_round_trips_at_will() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let t0 = f.now;
    let (mut advanced, mut returned) = (0u128, 0u128);
    for cycle in 0..3 {
        // the module wants an item (it keeps wanting: nothing is ever bought): advance, Relay 4663 -> Ethereum, arrival
        f.m(MODULE).net_units = 2 * E;
        assert!(until(&mut f, &mut st, 4, |s| s.buys.last().is_some_and(|b| matches!(b.phase, BPhase::Funded { .. }))), "cycle {cycle}");
        let id = st.buys.last().unwrap().id;
        st.used_orders.clear(); // harness: the one measured forward quote is re-used each cycle (rule 12 would want a fresh one)
        ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
        assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(id).unwrap().phase, BPhase::Bridging { .. })));
        f.eth_balance += ARRIVED;
        assert!(until(&mut f, &mut st, 2, |s| matches!(s.buy(id).unwrap().phase, BPhase::Arrived { .. })));
        // no listing is handed in; the runner, which holds the operator identity (OPERATOR_IDENTITY_PEM), calls eth_give_back at once
        ef::give_back_now(&mut st, id, 0).unwrap();
        tick(&mut f, &mut st);
        let amount = to_back_bridging(&mut f, &mut st, id);
        f.home_balance += amount / 10_000 * 9_994; // the measured Ethereum -> 4663 quote pays 99.94 %
        assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { .. })));
        let BPhase::Returned { wei } = st.buy(id).unwrap().phase else { unreachable!() };
        let adv = st.buy(id).unwrap().advanced;
        println!("C3-M-2 cycle {cycle}: advanced {adv}, returned {wei}: the module is short {} wei", adv - wei);
        advanced += adv;
        returned += wei;
        f.now += 60;
    }
    let lost = advanced - returned;
    println!("C3-M-2: 3 round trips in {} s of canister time, no NFT bought, no listing ever handed in: {lost} wei short at the module \
        ({:.2} % of what it advanced)", f.now - t0, lost as f64 * 100.0 / advanced as f64);
    assert!(f.items.is_empty(), "nothing was bought");
    assert!(f.now - t0 < 3_600, "no 48 h wait: the operator key triggers each round trip at once");
    assert!(lost * 100 >= advanced, "each round trip costs the module about 1 % of the item (Relay twice, the 0.003 ETH reserve, gas)");
}

// ---- C3-L-1: the gate assumes 0.5 % for Relay; the judged quote may take up to relay_max_fee_bps (3 %) -------------------------------

#[test]
fn c3_l1_a_quote_the_judge_accepts_arrives_under_the_floor_the_gate_passed() {
    let mut f = Fake::new(false);
    let edge = 4_896 * E / 10_000; // a module floor at the gate's edge for Milady's 0.5002 ETH item cap
    f.m(MODULE).accepted = edge;
    assert!(ef::advance_covers(DEPOSIT + rc::GAS_KEEP_WEI, edge).is_ok(), "the gate passes");
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let BPhase::Bridging { value, min_out, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    println!("C3-L-1: the judged quote promises {min_out} of {value} ({} bps under); the gate assumed 50", (value - min_out) * 10_000 / value);
    // Relay's solver fills at the order's minimum (what the quote, and the judge, allow)
    f.eth_balance += min_out;
    tick(&mut f, &mut st);
    let BPhase::Arrived { wei, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    assert!(wei - en::ETH_GAS_RESERVE_WEI < edge, "what arrived, less the gas reserve, is under the floor the gate passed");
    // a listing AT the module's floor (the rank-2 listing the post was made from), our own fresh reading at the same floor
    st.readings.push(ef::Reading { collection: MILADY, post: en::FloorPost { price: edge, confidence: 5_000, depth: 3, listing: edge,
        sale_median: edge, sale_p25: edge, listings: 3, sales: 5, clamped: false, held: None }, at: f.now, pin: ETH_PIN, due: false,
        module_posted_at: Some(f.now - 60) });
    f.listing_total = edge;
    ef::put_listing(&mut st, id, fill_hex(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Arrived { .. }), "{:?}", phase(&st));
    assert!(l.iter().any(|x| x.contains("the listing is refused") && x.contains("over cap")), "{l:?}");
    // then it waits 48 h and goes back (two Relay fees and the reserve from the module), and the gate lets it advance again
    f.now += ef::ARRIVED_BACK_SECS + 60;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::BackFunded { .. }));
    assert!(ef::advance_covers(DEPOSIT + rc::GAS_KEEP_WEI, edge).is_ok(), "nothing in the gate learns from it");
}

// ---- C3-L-2: any 4663 inflow during the way back is read as its arrival and returned to the buy's module ----------------------------

#[test]
fn c3_l2_a_4663_inflow_during_the_way_back_is_returned_as_the_arrival_and_the_real_arrival_is_orphaned() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let id = to_back_funded(&mut f, &mut st);
    let amount = to_back_bridging(&mut f, &mut st, id);
    let BPhase::BackBridging { min_out, .. } = st.buy(id).unwrap().phase else { unreachable!() };
    // ETH that is not Relay's reaches our 4663 address first: a frog-side advance (B-L-6, both sides on one canister), a donation
    let other = min_out / 2 + 1;
    f.home_balance += other;
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { .. })));
    let BPhase::Returned { wei } = st.buy(id).unwrap().phase else { unreachable!() };
    println!("C3-L-2: {other} wei of someone else's ETH was returned to Milady's module as the way back ({amount} sent back)");
    assert_eq!(wei, other);
    let (to, value, _) = f.home_calls.last().unwrap().clone();
    assert_eq!((to, value), (MODULE, other));
    // Relay's real payout lands next: nothing watches it any more; it stays at our 4663 address, unattributed
    let before = f.home_balance;
    f.home_balance += amount / 10_000 * 9_994;
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    assert_eq!(f.home_balance, before + amount / 10_000 * 9_994, "orphaned at our address");
    assert!(st.buys.iter().all(|b| b.finished()));
}

// ---- C3-H-1: an advance waiting on H3's value limit asks under a NEW key every tick; the tripwire freezes (pauses) the canister ------

#[test]
fn c3_h1_an_advance_waiting_on_the_value_limit_is_re_asked_under_a_new_key_every_tick_and_trips_the_freeze() {
    let mut f = Fake::new(false);
    // H3 exactly as the canister runs it (outflow::Outflow::check / record, stage 0's tripwire at 3 x a limit), with the day's value
    // limit scaled so that one Milady advance (0.5002 ETH) fits and a second does not: at launch, one BAYC advance (7 ETH) and one
    // Pudgy (3.4 ETH) against stage 0's 10 ETH, or any second advance of a module whose first came back the same day
    let mut l = etch_autobuyer::outflow::Limits::stage0();
    l.global_value_wei = 9 * E / 10;
    f.real_outflow = Some((Default::default(), l.clone()));
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    assert!(until(&mut f, &mut st, 4, |s| s.buys.last().is_some_and(|b| matches!(b.phase, BPhase::Funded { .. }))));
    let id = st.buys[0].id;
    // Relay cannot fill the order and refunds on 4663, three times (the threat model's "Relay failing to fill"): the advance goes back
    for _ in 0..3 {
        st.used_orders.clear(); // harness: the one measured quote, re-used
        ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
        assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(id).unwrap().phase, BPhase::Bridging { .. })), "{:?}", phase(&st));
        f.home_balance += DEPOSIT; // Relay's refund
        assert!(until(&mut f, &mut st, 2, |s| matches!(s.buy(id).unwrap().phase, BPhase::Funded { .. } | BPhase::Returning { .. } | BPhase::Returned { .. })));
        f.now += 600;
    }
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { .. })), "{:?}", phase(&st));
    // the module still wants its item: every tick buy_pass opens a NEW buy, home_act asks H3 for its advance under "buy:eth:{id}" (a
    // new key), is told to wait, and drops it; each new key is a new ask in the tripwire's demand
    let first_new = st.next_buy;
    let mut ticks = 0;
    while !f.paused && ticks < 100 {
        f.now += 60;
        tick(&mut f, &mut st);
        ticks += 1;
    }
    let (o, _) = f.real_outflow.as_ref().unwrap();
    println!("C3-H-1: {ticks} ticks after the return, {} buys opened and dropped; H3 asked {:?}; frozen: {:?}", st.next_buy - first_new,
        o.asked.iter().map(|a| a.key.clone()).collect::<Vec<_>>(), o.frozen());
    assert!(o.frozen().is_some(), "the tripwire froze custody on demand the canister itself inflated");
    assert!(f.paused, "canister::outflow_check pauses the buyer until k guardians resume it");
    assert!(ticks <= 10, "within minutes of ticks");
    assert!(o.asked.len() as u64 >= 1 + (st.next_buy - first_new) - 1, "one ask per re-opened buy");
    // what the pause stops: nothing new is signed on either chain, the floor is no longer posted (the module goes stale within its
    // 1 h FLOOR_TTL and refuses mints and burns), and no delivery is signed
    f.now += 2 * 3_600;
    let lines = tick(&mut f, &mut st);
    assert!(lines.iter().any(|x| x.contains("paused: nothing new is signed")), "{lines:?}");
}
