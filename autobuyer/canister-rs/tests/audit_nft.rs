//! Audit 2026-09-27-nft, fix round 1 (docs/audit/2026-09-27-nft/fixes-round1.md): reviewer B's and reviewer C's proofs of concept
//! (docs/audit/2026-09-27-nft/poc/B/audit_nft_b.rs, poc/C/audit_nft_c.rs) as regressions. Each test drives the same world the PoC
//! did and asserts that the attack or the failure no longer works. The fake world is `tests/ethnft.rs`'s, generalised as the PoCs
//! generalised it: two collections (Milady and "Azuki") on one Ethereum address and line, each with its own module and wrapper,
//! and fault injection for the provider-facing reads.

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
            sale_err: vec![], sale_reads: 0, deposit_txs: HashMap::new(), caps: custodycap::stage0(&keys), book: Book::default(), outflow: vec![],
        }
    }
    fn m(&mut self, module: [u8; 20]) -> &mut Module {
        self.modules.get_mut(&module).unwrap()
    }
    fn cfg(&self) -> EthNftConfig {
        let col = |c: [u8; 20], w: [u8; 20], m: [u8; 20]| EthCollection { collection: hx(&c), wrapper: hx(&w), module: hx(&m),
            max_purchase_wei: 2 * E, max_premium_bps: 500, chain_id: None, token_bound: None, nft_value_bps: None };
        let mut collections = vec![col(MILADY, WRAPPER, MODULE)];
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

// ---- B-H-1: nobody steers the posted floor through the sale evidence ------------------------------------------------------------

#[test]
fn b_h_1_self_trades_as_the_latest_transfers_never_set_the_modules_floor() {
    // the PoC: 12 self-trades of one NFT at 1.5 ETH (and at 0.093 ETH) handed in as "the latest transfer transactions" made the
    // canister sign postFloor(1.275 ETH) (and 0.1023 ETH) at confidence 5,000. Round 1: no post at all (they are not sales). Round 2:
    // the canister reads the chain's sales itself (a hand-in is not used): the self-trades are no move between two wallets, the
    // market's five real sales are, and the post is the rank-2 listing, 0.93 ETH, neither steered nor stopped
    for price in [3 * E / 2, 93 * E / 1_000] {
        let mut f = Fake::new(false);
        f.m(MODULE).posted_at = NOW - 2_000;
        let mut st = EthState::default();
        let mut hashes = Vec::new();
        for i in 0..12u8 {
            let h = [0x90 + i; 32];
            f.sale_receipts.insert(h, sale_receipt(MILADY, 4_242, price, ATTACKER, ATTACKER, ETH_PIN - 20 + i as u64));
            hashes.push(h);
        }
        ef::put_evidence(&mut st, &f.cfg(), MILADY, listings(), hashes, f.now).unwrap();
        let l = tick(&mut f, &mut st);
        assert_eq!(f.m(MODULE).posted.iter().map(|p| p.0).collect::<Vec<_>>(), vec![93 * E / 100], "{l:?}");
    }
    // wash sales between twelve distinct wallets at 1.5 ETH, with the five real sales: far from every listing, they are no vote,
    // and the post is the rank-2 listing (0.93 ETH), never the band's edge
    let mut f = Fake::new(false);
    f.m(MODULE).posted_at = NOW - 2_000;
    let mut st = EthState::default();
    let mut hashes = real_sales();
    for i in 0..7u8 {
        let h = [0xa0 + i; 32];
        f.sale_receipts.insert(h, sale_receipt(MILADY, 5_000 + i as u128, 3 * E / 2, [0x10 + i; 20], [0x40 + i; 20], ETH_PIN - 5));
        hashes.insert(0, h);
    }
    ef::put_evidence(&mut st, &f.cfg(), MILADY, listings(), hashes, f.now).unwrap();
    tick(&mut f, &mut st);
    assert_eq!(f.m(MODULE).posted.iter().map(|p| p.0).collect::<Vec<_>>(), vec![93 * E / 100]);
}

// ---- B-M-1: one unreadable receipt no longer voids the pass --------------------------------------------------------------------

#[test]
fn b_m_1_an_oversized_or_unreadable_sale_receipt_is_not_a_sale_and_the_pass_goes_on() {
    let mut f = Fake::new(false);
    f.m(MODULE).posted_at = NOW - 2_000;
    let mut st = EthState::default();
    let mut hashes = real_sales();
    let bad = [0x55; 32];
    // a sale on the chain whose receipt cannot be read (over every size class)
    f.sale_receipts.insert(bad, sale_receipt(MILADY, 555, 93 * E / 100, [0x21; 20], [0x22; 20], ETH_PIN - 500));
    f.sale_err.push(bad);
    hashes.insert(0, bad);
    ef::put_evidence(&mut st, &f.cfg(), MILADY, listings(), hashes, f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert_eq!(f.m(MODULE).posted.first().map(|p| p.0), Some(93 * E / 100), "five good sales, three listings: posted ({l:?})");
    assert!(l.iter().any(|x| x.contains("1 receipt(s) unread")), "{l:?}");
}

// ---- B-M-7: final receipts are read once --------------------------------------------------------------------------------------------

#[test]
fn b_m_7_a_final_sale_receipt_is_read_once_and_listings_are_read_only_as_far_as_needed() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    let first = f.sale_reads;
    assert_eq!(first, 5);
    for _ in 0..3 {
        f.now += 1_200;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    assert_eq!(f.sale_reads, first, "every one of the five is final: never read again");
    assert!(st.reading(&MILADY).is_some_and(|r| r.post.sales == 5));
    // eight listings handed in, five distinct offerers are enough: the rest are not read
    let many: Vec<String> = (1..=8u8).map(|o| format!("{o}:{}", (90 + o as u128) * E / 100)).collect();
    let n0 = f.home_calls.len();
    ef::put_evidence(&mut st, &f.cfg(), MILADY, many, real_sales(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert_eq!(st.reading(&MILADY).unwrap().post.listings, 5, "min_listings + 2 verified, then the pass stops reading listings");
    let _ = n0;
}

// ---- D-H-2 (the canister's half): the post is clamped into floorBounds() ----------------------------------------------------------

#[test]
fn d_h_2_the_canister_posts_inside_the_modules_bounds_so_target_is_accepted() {
    let mut f = Fake::new(false);
    f.m(MODULE).posted_at = NOW - 2_000;
    f.m(MODULE).accepted = FLOOR0;
    f.m(MODULE).bounds = (8_775 * E / 10_000, 9_225 * E / 10_000); // ±2.5 % after half an hour
    let mut st = EthState::default();
    evidence(&f, &mut st);
    let l = tick(&mut f, &mut st);
    let (price, _, _) = f.m(MODULE).posted[0];
    assert_eq!(price, 9_225 * E / 10_000, "our reading is 0.93; the module allows 0.9225 now: that is what is posted ({l:?})");
    assert!(l.iter().any(|x| x.contains("clamped into the module's bounds")), "{l:?}");
}

// ---- B-M-2 / C-M-1: a transport failure is never "can never succeed" ------------------------------------------------------------

#[test]
fn b_m_2_c_m_1_a_transient_simulation_failure_waits_and_only_an_hour_of_agreed_reverts_marks() {
    // (a) a delivery: one timed-out eth_call is a wait; the next tick delivers
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, NOW + 7 * 86_400);
    f.plan_err = vec![TRANSIENT.into()];
    tick(&mut f, &mut st);
    assert_eq!(st.deliveries[0].phase, DPhase::Waiting, "not marked on a transport failure");
    assert!(f.marked.is_empty());
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }), "{:?}", st.deliveries[0].phase);
    // an agreed revert, once, then a pass: delivered, never marked
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, NOW + 7 * 86_400);
    f.plan_err = vec![REVERTS.into()];
    tick(&mut f, &mut st);
    assert!(f.marked.is_empty());
    f.now += 600;
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }));
    assert_eq!(st.deliveries[0].refused_since, None);
    // (b) a stray: a timed-out simulation leaves it on its return path; it goes back at the next tick
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    let tx = [0x51; 32];
    let r = ReceiptV { block: ETH_PIN - 2, success: true, logs: vec![LogV { address: MILADY,
        topics: vec![en::TRANSFER, sp::word_addr(&HOLDER), sp::word_addr(&US), tok(601)], data: vec![] }] };
    f.deposit_txs.insert(tx, (HOLDER, MILADY, en::transfer_from_call(&HOLDER, &US, tok(601)), r.clone()));
    f.sale_receipts.insert(tx, r);
    f.owners.insert((MILADY, tok(601)), US);
    f.finalized = ETH_PIN;
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    f.plan_err = vec![TRANSIENT.into()];
    tick(&mut f, &mut st);
    assert!(matches!(st.deposits[0].phase, XPhase::Return { .. }), "{:?}", st.deposits[0].phase);
    tick(&mut f, &mut st);
    assert!(matches!(st.deposits[0].phase, XPhase::Returning { .. }), "{:?}", st.deposits[0].phase);
}

// ---- B-M-3 / C-M-3: superseding keeps the line until the outcome is read ------------------------------------------------------------

#[test]
fn b_m_3_c_m_3_a_read_error_as_a_transfer_beats_its_cancel_keeps_the_line_and_the_delivery_is_recorded() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let deliver_by = NOW + 3 * 86_400;
    resolved(&mut f, false, 77, ALICE, deliver_by);
    let key = custodycap::evm_key(1, &MILADY);
    custodycap::admit(&f.caps, &mut f.book, &held_id(MILADY, 77), &key, 0).unwrap();
    tick(&mut f, &mut st);
    let deliver_hash = f.eth_signed[0].1;
    f.now = NOW + 2 * 86_400 + 1;
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }));
    // the transfer, not the cancel, mines; the next read of its receipt fails once (a 429)
    f.mine_eth(0, true, ETH_PIN - 3);
    f.receipt_err.insert(deliver_hash, 1);
    resolved(&mut f, false, 78, BOB, NOW + 9 * 86_400);
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("429")), "{l:?}");
    assert!(st.line.is_some(), "the line is kept: nothing else is signed until the outcome is read");
    assert_eq!(f.eth_signed.len(), 2, "no new delivery was signed while the line was unresolved");
    f.finalized = ETH_PIN;
    for _ in 0..4 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.deliveries[0].phase, DPhase::Recorded { .. }), "{:?}", st.deliveries[0].phase);
    assert_eq!(f.tickets[0].stage, Stage::Delivered, "recorded: the redeemer can never refund it and keep the NFT");
    assert!(!f.book.contains(&held_id(MILADY, 77)), "its custody slot is released");
    assert!(st.last_out(&MILADY, &tok(77)).is_some(), "and `outbound` knows it left");
}

#[test]
fn b_m_3_a_nonce_taken_by_a_transaction_not_ours_halts_the_line_until_the_operator_clears_it() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    f.eth_nonce = 1; // something else mined at nonce 0
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("not ours")), "{l:?}");
    assert!(st.halted.is_some() && st.line.is_some(), "halted, the line kept");
    resolved(&mut f, false, 78, BOB, NOW + 7 * 86_400);
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    assert_eq!(f.eth_signed.len(), 1, "nothing more is signed on Ethereum while halted");
    // the operator looks, then clears: the delivery waits to be signed again, above the taken nonce
    assert!(ef::line_clear(&mut EthState::default()).is_err(), "refused unless halted");
    ef::line_clear(&mut st).unwrap();
    assert!(st.halted.is_none() && st.line.is_none());
    tick(&mut f, &mut st);
    let (t, _) = f.eth_signed.last().unwrap();
    assert!(t.nonce >= 1, "never at the taken nonce");
}

// ---- B-M-4: the nonce never goes back --------------------------------------------------------------------------------------------

#[test]
fn b_m_4_a_nonce_read_at_a_lower_pin_is_never_reused_for_a_delivery_or_a_fill() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    assert!(st.line.is_none());
    assert_eq!(st.last_mined, Some((0, ETH_PIN - 5)));
    resolved(&mut f, false, 78, ALICE, NOW + 7 * 86_400);
    f.stale_nonce = Some(0); // the next pin is one bucket lower and does not count the mined delivery
    tick(&mut f, &mut st);
    let DPhase::Sent { nonce, .. } = st.deliveries[1].phase.clone() else { panic!("{:?}", st.deliveries[1].phase) };
    assert_eq!(nonce, 1, "signed above every nonce of ours seen mined");
    assert_eq!(f.eth_signed[1].0.nonce, 1);
    // the same for a fill
    f.mine_eth(1, true, ETH_PIN - 4);
    tick(&mut f, &mut st);
    to_arrival(&mut f, &mut st);
    evidence(&f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    f.stale_nonce = Some(0);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }), "{:?}", phase(&st));
    assert_eq!(f.eth_signed.last().unwrap().0.nonce, 2);
    f.mine_eth(f.eth_signed.len() - 1, true, ETH_PIN - 3);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Bought { .. }), "{:?}", phase(&st));
}

// ---- B-M-5 / C-L-3: one collection never blocks another's buys ------------------------------------------------------------------------

#[test]
fn b_m_5_c_l_3_a_collection_at_its_cap_or_an_arrived_buy_never_blocks_another_collections_buy() {
    let mut f = Fake::new(true);
    fill_custody(&mut f, MILADY, custodycap::STAGE0_EVM_ITEMS);     // stage 0's items for an Ethereum collection (J7: 6)
    f.m(MODULE).net_units = 2 * E;
    f.m(MODULE2).net_units = 2 * E;
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    assert_eq!(st.buys.len(), 1);
    assert_eq!(st.buys[0].collection, AZUKI, "Milady is full; Azuki's buy starts at once");
    // an arrived Milady buy that cannot fill does not hold up Azuki's next buy
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    f.listing_total = 496 * E / 1_000;
    f.m(MODULE2).net_units = 2 * E;
    for _ in 0..3 {
        evidence(&f, &mut st);
        ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
        tick(&mut f, &mut st);
        f.now += 600;
    }
    assert!(st.buys.iter().any(|b| b.collection == AZUKI), "{:?}", st.buys.iter().map(|b| (b.collection, b.phase.clone())).collect::<Vec<_>>());
    assert!(matches!(st.buys[0].phase, BPhase::Arrived { .. }));
}

// ---- B-H-2 / C-H-1: no refusal elsewhere stops a floor post ------------------------------------------------------------------------------

#[test]
fn b_h_2_c_h_1_a_purchase_mint_waiting_on_the_limit_never_stops_any_floor_post() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    f.mine_eth(f.eth_signed.len() - 1, true, ETH_PIN - 10);
    tick(&mut f, &mut st);
    f.finalized = ETH_PIN;
    f.wait_prefix = vec!["mint:eth:".into()];
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Final { .. }), "{:?}", phase(&st));
    let az = vec![format!("1:{}", 66 * E / 100), format!("2:{}", 67 * E / 100), format!("3:{}", 68 * E / 100)];
    let mut hashes = Vec::new();
    for i in 0..5u8 {
        let h = [0xa0 + i; 32];
        f.sale_receipts.insert(h, sale_receipt(AZUKI, 100 + i as u128, 67 * E / 100, [0x10 + i; 20], [0x50 + i; 20], ETH_PIN - 50));
        hashes.push(h);
    }
    f.m(MODULE2).posted_at = f.now - 2_000;
    f.m(MODULE).posted_at = f.now - 2_000;
    for _ in 0..4 {
        evidence(&f, &mut st);
        ef::put_evidence(&mut st, &f.cfg(), AZUKI, az.clone(), hashes.clone(), f.now).unwrap();
        tick(&mut f, &mut st);
        f.now += 400;
    }
    assert!(!f.m(MODULE2).posted.is_empty(), "Azuki's floor is posted while Milady's mint waits on H3");
    assert!(!f.m(MODULE).posted.is_empty(), "and Milady's own");
}

#[test]
fn c_h_1_a_l_1_no_deposit_mints_while_a_buy_of_its_collection_holds_the_mint_room() {
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st); // bridging, then arrived: the buy holds the collection's mint room from its advance
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 20);
    ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
    f.finalized = ETH_PIN;
    for _ in 0..3 {
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    assert_eq!(st.deposits[0].phase, XPhase::Mint);
    assert!(f.items.is_empty(), "the deposit waits while the purchase is committed");
}

// ---- C-M-2: an unmined fill never keeps a holder from the 72 h refund; it is cancelled ------------------------------------------------

#[test]
fn c_m_2_with_a_fill_unmined_another_collections_delivery_is_still_marked_and_the_fill_is_cancelled() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }));
    f.m(MODULE).net_units = 0;
    let deliver_by = f.now + 7 * 86_400;
    resolved(&mut f, true, 4_242, BOB, deliver_by);
    // fees above the cap for a week: the fill cannot be re-priced or cancelled, and nothing mines
    f.home_final_lag = 0;
    let mut cancelled = false;
    while f.now < deliver_by + 3_600 {
        f.now += 3_600;
        let l = tick(&mut f, &mut st);
        cancelled |= l.iter().any(|x| x.contains("cancel") && x.contains("fill"));
    }
    assert!(cancelled, "the fill was cancelled at its nonce after FILL_CANCEL_SECS");
    let d = st.deliveries.iter().find(|d| d.wrapper == WRAPPER2).unwrap();
    assert!(f.marked2.contains(&0) || matches!(d.phase, DPhase::Sent { .. } | DPhase::Moved { .. }),
        "the Azuki holder is marked (72 h refund) or served, never left to the 30-day window: {:?}", d.phase);
}

// ---- C-L-1 / B-L-4: the paused pass -----------------------------------------------------------------------------------------------------

#[test]
fn c_l_1_while_paused_a_final_delivery_is_recorded_and_a_refunded_tickets_bytes_are_never_resent() {
    // (a) the delivery mined before the pause: recorded during it, long before the late window
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let deliver_by = NOW + 7 * 86_400;
    resolved(&mut f, false, 77, ALICE, deliver_by);
    let key = custodycap::evm_key(1, &MILADY);
    custodycap::admit(&f.caps, &mut f.book, &held_id(MILADY, 77), &key, 0).unwrap();
    tick(&mut f, &mut st);
    f.mine_eth(0, true, ETH_PIN - 5);
    f.paused = true;
    f.finalized = ETH_PIN;
    for _ in 0..4 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    assert_eq!(f.tickets[0].stage, Stage::Delivered, "recorded while paused");
    assert!(!f.book.contains(&held_id(MILADY, 77)));
    assert_eq!(f.eth_signed.len(), 1, "nothing new signed on Ethereum while paused");
    // (b) the delivery was pending when the pause began; the ticket is refunded after the late window: its bytes are never sent
    // again, and the pending transfer is cancelled
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, deliver_by);
    tick(&mut f, &mut st);
    let raw = st.line.as_ref().unwrap().raw.clone();
    f.paused = true;
    f.now = deliver_by + 30 * 86_400 + 1;
    f.tickets[0].stage = Stage::Refunded;
    let before = f.broadcasts.len();
    tick(&mut f, &mut st);
    f.paused = false;
    tick(&mut f, &mut st);
    assert!(!f.broadcasts[before..].contains(&raw), "the refunded ticket's delivery is never pushed again");
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }), "{:?}", st.deliveries[0].phase);
}

// ---- C-L-2 / B-L-8: one collection's limit never holds up another's deliveries or deposit mints --------------------------------------

#[test]
fn c_l_2_b_l_8_one_collections_limit_never_blocks_another_collections_delivery_or_deposit_mint() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, NOW + 7 * 86_400);
    resolved(&mut f, true, 4_242, BOB, NOW + 7 * 86_400);
    f.wait_prefix = vec![format!("deliver:{}", hx(&WRAPPER))];
    tick(&mut f, &mut st);
    let az = st.deliveries.iter().find(|d| d.wrapper == WRAPPER2).unwrap();
    assert!(matches!(az.phase, DPhase::Sent { .. }), "Azuki's delivery goes while Milady's waits: {:?}", az.phase);
    // deposits
    let mut f = Fake::new(true);
    f.deposits_on = true;
    let mut st = EthState::default();
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 1);
    deposit_tx(&mut f, [0x92; 32], AZUKI, WRAPPER2, 951, RECIPIENT, ETH_PIN - 1);
    for t in [[0x91; 32], [0x92; 32]] {
        ef::put_deposit(&mut st, &f.cfg(), t, f.now).unwrap();
    }
    f.finalized = ETH_PIN;
    // round 4 (E4-H-1): the key of Milady's next mint (the collection and its count), not the deposit's own
    f.wait_prefix = vec![format!("mint:eth:{}:", etch_autobuyer::custodycap::evm_key(1, &MILADY))];
    let az_ev = vec![format!("1:{}", 66 * E / 100), format!("2:{}", 67 * E / 100), format!("3:{}", 68 * E / 100)];
    let mut hashes = Vec::new();
    for i in 0..5u8 {
        let h = [0xa0 + i; 32];
        f.sale_receipts.insert(h, sale_receipt(AZUKI, 100 + i as u128, 67 * E / 100, [0x10 + i; 20], [0x50 + i; 20], ETH_PIN - 500));
        hashes.push(h);
    }
    for _ in 0..4 {
        evidence(&f, &mut st);
        ef::put_evidence(&mut st, &f.cfg(), AZUKI, az_ev.clone(), hashes.clone(), f.now).unwrap();
        tick(&mut f, &mut st);
    }
    assert_eq!(f.items2, vec![tok(951)], "the Azuki deposit is minted while the Milady one waits on its limit");
    assert_eq!(st.deposits[0].phase, XPhase::Mint);
}

// ---- C-L-4: an optional saved element of another shape refuses the upgrade -----------------------------------------------------------

#[derive(candid::CandidType, serde::Deserialize, Clone)]
struct OldEthState {
    pub buys: Vec<ef::EthBuy>,
    pub next_buy: u64,
    pub deliveries: Vec<u8>, // not the saved type's `vec Delivery`: an incompatible shape
    pub rr: u64,
}

#[test]
fn c_l_4_an_eth_state_of_another_shape_is_detected_rather_than_restored_as_nothing() {
    use etch_autobuyer::canister::{lost_elements, Saved, SavedProbe};
    let old = OldEthState { buys: vec![], next_buy: 7, deliveries: vec![1, 2], rr: 1 };
    type Written = (Option<etch_autobuyer::canister::Init>, Vec<etch_autobuyer::state::Purchase>, bool, Vec<String>,
        Option<etch_autobuyer::redeem::Custody>, Option<etch_autobuyer::chains::NftState>, Option<etch_autobuyer::frogflow::FrogState>,
        Option<etch_autobuyer::floortrigger::FloorState>, Option<(u8,)>, Option<u8>, Option<u8>, Option<u8>, Option<OldEthState>);
    let w: Written = (None, vec![], false, vec![], None, None, None, None, None, None, None, None, Some(old));
    let bytes = candid::encode_args(((w,),)).unwrap();
    let bytes = {
        // as stable_save writes `(saved,)`
        let (x,): ((Written,),) = candid::decode_args(&bytes).unwrap();
        candid::encode_args((x.0,)).unwrap()
    };
    let (saved,): (Saved,) = candid::decode_args(&bytes).expect("decodes: no error for M-2's trap");
    assert!(saved.12.is_none(), "Candid's opt rule: the saved Ethereum side comes back as nothing");
    let (probe,): (SavedProbe,) = candid::decode_args(&bytes).unwrap();
    let decoded = [saved.4.is_some(), saved.5.is_some(), saved.6.is_some(), saved.7.is_some(), saved.8.is_some(), saved.9.is_some(),
        saved.10.is_some(), saved.11.is_some(), saved.12.is_some()];
    assert_eq!(lost_elements(&probe, decoded), vec![13], "post_upgrade now traps: element 13 was saved and would restore as nothing");
    // an absent element is not lost
    let absent: Written = (None, vec![], false, vec![], None, None, None, None, None, None, None, None, None);
    let b2 = candid::encode_args((absent,)).unwrap();
    let (s2,): (Saved,) = candid::decode_args(&b2).unwrap();
    let (p2,): (SavedProbe,) = candid::decode_args(&b2).unwrap();
    assert!(lost_elements(&p2, [s2.4.is_some(), s2.5.is_some(), s2.6.is_some(), s2.7.is_some(), s2.8.is_some(), s2.9.is_some(),
        s2.10.is_some(), s2.11.is_some(), s2.12.is_some()]).is_empty());
    // and this round's new fields are all `opt`: a state saved without them restores
    let cur = EthState::default();
    let (back,): (Option<EthState>,) = candid::decode_args(&candid::encode_args((Some(cur.clone()),)).unwrap()).unwrap();
    assert_eq!(back, Some(cur));
}

// ---- A-M-1: deposits mint only against a floor the market has traded at ------------------------------------------------------------

#[test]
fn a_m_1_a_deposit_waits_while_the_accepted_floor_is_over_the_verified_sales() {
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 1);
    ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
    f.finalized = ETH_PIN;
    // the module's accepted floor walked up to +10 % over the sales (the PoC's inflated floor, 1.00 ETH against a sales median of
    // 0.91), and the module cannot come down faster than its bounds
    f.m(MODULE).accepted = E;
    f.m(MODULE).bounds = (E, E);
    for _ in 0..4 {
        f.now += 600;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    let median = st.reading(&MILADY).unwrap().post.sale_median;
    assert_eq!(median, 91 * E / 100);
    assert!(f.items.is_empty(), "no unit is minted to be burned at a floor the market has not traded at");
    assert_eq!(st.deposits[0].phase, XPhase::Mint, "the deposit waits: never refused, never returned");
    // the floor comes back into range: minted
    f.m(MODULE).accepted = median;
    f.m(MODULE).bounds = (0, u128::MAX);
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    assert_eq!(f.items, vec![tok(950)]);
}

// ---- A-I-3: a claim naming a module of ours is a stray ----------------------------------------------------------------------------------

#[test]
fn a_i_3_a_deposit_naming_a_module_or_wrapper_of_ours_goes_back() {
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, MODULE, ETH_PIN - 1);
    ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(&st.deposits[0].phase, XPhase::Return { why } if why.contains("Floor Module")), "{:?}", st.deposits[0].phase);
}

// ---- B-L-2 / C-L-6: a delivery waits for its ticket's finality on the home chain ---------------------------------------------------

#[test]
fn b_l_2_c_l_6_a_delivery_is_signed_only_once_its_tickets_resolution_is_final_on_the_home_chain() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, NOW + 7 * 86_400); // resolved NOW
    f.home_final_lag = 20 * 60; // 4663 finality ~20 min behind
    let l = tick(&mut f, &mut st);
    assert!(f.eth_signed.is_empty(), "{l:?}");
    assert!(l.iter().any(|x| x.contains("finality")), "{l:?}");
    f.now += 20 * 60 + 60;
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }), "{:?}", st.deliveries[0].phase);
}

// ---- C-L-5: a held stray can be re-armed (judged again) -----------------------------------------------------------------------------------

#[test]
fn c_l_5_a_held_stray_goes_back_to_its_return_path_and_is_judged_again() {
    let mut f = Fake::new(false);
    f.deposits_on = true;
    let mut st = EthState::default();
    let tx = [0x81; 32];
    let r = ReceiptV { block: ETH_PIN - 1, success: true, logs: vec![LogV { address: MILADY,
        topics: vec![en::TRANSFER, sp::word_addr(&HOLDER), sp::word_addr(&US), tok(900)], data: vec![] }] };
    f.deposit_txs.insert(tx, (HOLDER, MILADY, en::transfer_from_call(&HOLDER, &US, tok(900)), r.clone()));
    f.sale_receipts.insert(tx, r);
    f.owners.insert((MILADY, tok(900)), US);
    f.finalized = ETH_PIN;
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    f.plan_err = vec![REVERTS.into(), REVERTS.into()];
    tick(&mut f, &mut st);
    f.now += ef::SIM_REFUSAL_SECS;
    tick(&mut f, &mut st);
    assert!(matches!(st.deposits[0].phase, XPhase::Held { .. }), "{:?}", st.deposits[0].phase);
    assert!(ef::rearm_stray(&mut st, &f.cfg(), [0x99; 32], tok(900)).is_err());
    ef::rearm_stray(&mut st, &f.cfg(), tx, tok(900)).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(st.deposits[0].phase, XPhase::Returning { .. }), "{:?}", st.deposits[0].phase);
    assert_eq!(f.eth_signed.last().unwrap().0.data, en::transfer_from_call(&US, &HOLDER, tok(900)));
}

// ---- B-L-7: a Relay refund on Ethereum under the minimum out is an arrival, not a stuck buy holding the Relay leg ---------------

#[test]
fn b_l_7_a_refund_on_ethereum_under_the_minimum_frees_the_relay_leg() {
    let mut f = Fake::new(true);
    f.m(MODULE).net_units = 2 * E;
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    st.used_orders.clear();
    ef::put_quote(&mut st, 0, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    f.now += 8 * 86_400;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Stuck { .. }), "{:?}", phase(&st));
    assert!(st.relay_busy(), "a watched stuck buy holds the Relay leg");
    f.eth_balance += DEPOSIT * 6 / 10; // Relay refunds 60 % on Ethereum, under the order's minimum out
    f.m(MODULE2).net_units = 2 * E;
    let now = f.now;
    f.m(MODULE2).posted_at = now; // Azuki's floor is fresh (the canister posts it)
    tick(&mut f, &mut st);
    assert!(matches!(st.buys[0].phase, BPhase::Arrived { .. }), "{:?}", st.buys[0].phase);
    assert!(st.buys.iter().any(|b| b.collection == AZUKI), "the Relay leg is free: Azuki's buy starts");
}

// ---- B-I-1: the NFT side's chain never lists -------------------------------------------------------------------------------------------------

#[test]
fn b_i_1_sale_nft_listing_refuses_the_eth_nft_chain() {
    let src = include_str!("../src/salenet.rs");
    let f = src.split("pub async fn sale_nft_listing(").nth(1).unwrap();
    let head: String = f.chars().take(900).collect();
    assert!(head.contains("cfg.eth_nft.as_ref().is_some_and(|e| e.chains().contains(&chain_id))"), "refused before any signature (every chain of eth_nft)");
    let _ = XPhase::Seen;
}
