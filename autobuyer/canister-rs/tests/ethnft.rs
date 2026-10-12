//! The Ethereum NFT side (docs/v7/phase1-nft.md) against a fake world: a home chain whose Floor Module advances, settles and
//! takes back, and whose `EthNftWrapper` mints, resolves tickets and records deliveries; an Ethereum where Relay's ETH arrives
//! (or does not), fills and transfers mine, revert or are cancelled, and blocks become final; a runner that hands in quotes,
//! listings and evidence, good and bad. The Relay quote is the real one captured 2026-09-27; the floor's sales are real receipts.

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
/// The quote's own `user`: the fake canister's EVM address on both chains.
const US: [u8; 20] = [0x76, 0x02, 0x0b, 0x70, 0x93, 0xbd, 0x35, 0xd2, 0x52, 0x0d, 0x47, 0xde, 0xd7, 0x64, 0xef, 0xc5, 0x54, 0x23, 0x7e, 0xfe];
/// The quote's time: its deadlines are this + 7 days.
const NOW: u64 = 1_791_117_628 - 7 * 86_400 + 5;
const MILADY: [u8; 20] = [0x5a, 0xf0, 0xd9, 0x82, 0x7e, 0x0c, 0x53, 0xe4, 0x79, 0x9b, 0xb2, 0x26, 0x65, 0x5a, 0x1d, 0xe1, 0x52, 0xa4, 0x25, 0xa5];
const WRAPPER: [u8; 20] = [0xaa; 20];
const MODULE: [u8; 20] = [0xbb; 20];
const ALICE: [u8; 20] = [0xa1; 20];
/// The quote's deposit: 0.5 ETH, so the module advances 0.5 ETH plus the gas keep.
const DEPOSIT: u128 = 500_000_000_000_000_000;
const ETH_PIN: u64 = 26_068_928;
/// The second collection of the multi-collection tests (docs/v7/phase1-nft-pooled.md §4): its own wrapper and module.
const BAYC: [u8; 20] = [0xbc, 0x4c, 0xa0, 0xed, 0xa7, 0x64, 0x7a, 0x8a, 0xb7, 0xc2, 0x06, 0x1c, 0x2e, 0x11, 0x8a, 0x18, 0xa9, 0x36, 0xf1, 0x3d];
const WRAPPER2: [u8; 20] = [0xab; 20];
const MODULE2: [u8; 20] = [0xbd; 20];
/// The token a BAYC fill buys (Milady's is 9 180).
const APE: u128 = 4_444;
/// Round 2 (A2-M-1, `ethflow::advance_covers`): the modules' floor before the tests post, one their item cap (the Relay fixture's
/// 0.5 ETH plus the 4663 gas keep) covers on Ethereum with the margin; a post then moves it to what the evidence says.
const FLOOR0: u128 = 47 * E / 100;

/// The second collection's home-chain module and wrapper: nothing is shared with Milady's.
struct Second {
    net_units: u128,
    transit: bool,
    item_cap: u128,
    accepted: u128,
    posted_at: u64,
    settled: Option<u128>,
    items: Vec<Word>,
    listing_total: u128,
}

struct Ticket {
    stage: Stage,
    item: u128,
    dest: [u8; 20],
    deliver_by: u64,
}

struct Fake {
    /// B-L-6: the Bitcoin side holds / waits for the one Relay leg
    frog_busy: bool,
    frog_waiting: bool,
    now: u64,
    paused: bool,
    deposits_on: bool,
    // the home chain
    home_balance: u128,
    home_nonce: u64,
    item_cap: u128,
    net_units: u128,
    transit: bool,
    accepted: u128,
    posted_at: u64,
    keeper: [u8; 20],
    allocator: [u8; 20],
    home_sent: HashMap<[u8; 32], (bool, Vec<u8>)>,
    home_calls: Vec<([u8; 20], u128, Vec<u8>)>,
    home_revert_next: bool,
    tickets: Vec<Ticket>,
    items: Vec<Word>,
    settled: Option<u128>,
    returned: Option<u128>,
    posted: Option<(u128, u16, u32)>,
    marked: Vec<u128>,
    // Ethereum
    eth_balance: u128,
    eth_nonce: u64,
    finalized: u64,
    owners: HashMap<Word, [u8; 20]>,
    eth_signed: Vec<(Tx1559, [u8; 32])>,
    receipts: HashMap<[u8; 32], OurReceipt>,
    listing_total: u128,
    transfer_refused: bool,
    sale_receipts: HashMap<[u8; 32], ReceiptV>,
    /// round 3 (D3-H-1): our own mined transfers' receipts with their `Transfer` log (the canister judges a delivery by it)
    our_logs: HashMap<[u8; 32], ReceiptV>,
    /// round 3 (D3-H-1): tickets the canister noted sent (`noteSent`)
    noted: Vec<u128>,
    deposit_txs: HashMap<[u8; 32], ([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>,
    // guards
    caps: CapPolicy,
    book: Book,
    outflow: Vec<(String, Flow, u128)>,
    /// H3 says "wait" to every mint
    mint_wait: bool,
    saved: usize,
    bayc: Option<Second>,
    /// how far the home chain's finality is behind now (0: everything before now is final)
    home_final_lag: u64,
}

fn word(v: u128) -> Vec<u8> {
    etch_autobuyer::seaport::word_u128(v).to_vec()
}
fn addr_word(a: &[u8; 20]) -> Vec<u8> {
    etch_autobuyer::seaport::word_addr(a).to_vec()
}
fn sel(d: &[u8]) -> [u8; 4] {
    d[..4].try_into().unwrap()
}
fn arg(d: &[u8], i: usize) -> u128 {
    u128::from_be_bytes(d[4 + 32 * i + 16..4 + 32 * i + 32].try_into().unwrap())
}

impl Fake {
    fn new() -> Self {
        let key = custodycap::evm_key(1, &MILADY);
        let mut sale_receipts = HashMap::new();
        let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
        for (k, r) in v["receipts"].as_object().unwrap() {
            let h: [u8; 32] = hex::decode(&k[2..]).unwrap().try_into().unwrap();
            sale_receipts.insert(h, en::receipt_of(r).unwrap().unwrap());
        }
        Fake {
            frog_busy: false, frog_waiting: false,
            now: NOW, paused: false, deposits_on: false,
            home_balance: 2_000_000_000_000_000, home_nonce: 0, item_cap: DEPOSIT + rc::GAS_KEEP_WEI, net_units: 2 * E, transit: false,
            accepted: FLOOR0, posted_at: NOW - 60, keeper: US, allocator: rc::RELAY_ALLOCATOR_4663, home_sent: HashMap::new(),
            home_calls: vec![], home_revert_next: false, tickets: vec![], items: vec![], settled: None, returned: None, posted: None,
            marked: vec![],
            eth_balance: 10_000_000_000_000_000, eth_nonce: 0, finalized: ETH_PIN - 100, owners: HashMap::new(), eth_signed: vec![],
            receipts: HashMap::new(), listing_total: 45 * E / 100, transfer_refused: false, sale_receipts, our_logs: HashMap::new(), noted: vec![], deposit_txs: HashMap::new(),
            caps: custodycap::stage0(&[key]), book: Book::default(), outflow: vec![], mint_wait: false, saved: 0, bayc: None,
            home_final_lag: 0,
        }
    }

    /// Two collections on one Ethereum address: Milady first, BAYC second, both wanting an item.
    fn two() -> Self {
        let mut f = Fake::new();
        f.caps = custodycap::stage0(&[custodycap::evm_key(1, &MILADY), custodycap::evm_key(1, &BAYC)]);
        f.eth_balance += 10 * E / 1000; // gas for two collections
        f.bayc = Some(Second { net_units: 2 * E, transit: false, item_cap: DEPOSIT + rc::GAS_KEEP_WEI, accepted: FLOOR0,
            posted_at: NOW - 60, settled: None, items: vec![], listing_total: 45 * E / 100 });
        f
    }
    fn b(&mut self) -> &mut Second {
        self.bayc.as_mut().expect("a two-collection fake")
    }

    fn cfg(&self) -> EthNftConfig {
        let mut collections = vec![EthCollection {
            collection: format!("0x{}", hex::encode(MILADY)), wrapper: format!("0x{}", hex::encode(WRAPPER)),
            module: format!("0x{}", hex::encode(MODULE)), max_purchase_wei: 2 * E, max_premium_bps: 500, chain_id: None, token_bound: None, nft_value_bps: None }];
        if self.bayc.is_some() {
            collections.push(EthCollection { collection: format!("0x{}", hex::encode(BAYC)), wrapper: format!("0x{}", hex::encode(WRAPPER2)),
                module: format!("0x{}", hex::encode(MODULE2)), max_purchase_wei: 8 * E, max_premium_bps: 500, chain_id: None, token_bound: None, nft_value_bps: None });
        }
        EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: self.deposits_on, floor: None, collections, gas_pot_target_wei: None, logs_max_blocks: None, dormancy: None }
    }

    fn apply_home(&mut self, to: [u8; 20], value: u128, d: &[u8]) -> bool {
        self.home_calls.push((to, value, d.to_vec()));
        if self.home_revert_next {
            self.home_revert_next = false;
            return false;
        }
        if to == rc::RELAY_DEPOSITORY_4663 {
            self.home_balance -= value;
            return true;
        }
        match (to, sel(d)) {
            (MODULE, s) if s == rc::ADVANCE => {
                self.home_balance += arg(d, 0);
                self.transit = true;
            }
            (MODULE, s) if s == rc::RETURN_ADVANCE => {
                self.home_balance -= value;
                self.transit = false;
                self.returned = Some(value);
            }
            (MODULE, s) if s == rc::SETTLE_ADVANCE => {
                self.transit = false;
                self.settled = Some(arg(d, 0));
                self.net_units -= E;
            }
            (MODULE, s) if s == en::POST_FLOOR => {
                self.posted = Some((arg(d, 0), arg(d, 1) as u16, arg(d, 2) as u32));
                self.accepted = arg(d, 0);
                self.posted_at = self.now;
            }
            (WRAPPER, s) if s == en::MINT || s == en::MINT_DEPOSIT => {
                self.items.push(d[4..36].try_into().unwrap());
            }
            (WRAPPER, s) if s == en::RECORD_DELIVERY => self.tickets[arg(d, 0) as usize].stage = Stage::Delivered,
            (WRAPPER, s) if s == en::MARK_UNDELIVERABLE => self.marked.push(arg(d, 0)),
            (WRAPPER, s) if s == en::NOTE_SENT => self.noted.push(arg(d, 0)),
            (MODULE2, s) if s == rc::ADVANCE => {
                self.home_balance += arg(d, 0);
                self.b().transit = true;
            }
            (MODULE2, s) if s == rc::SETTLE_ADVANCE => {
                let b = self.b();
                b.transit = false;
                b.settled = Some(arg(d, 0));
                b.net_units -= E;
            }
            (MODULE2, s) if s == en::POST_FLOOR => {
                let now = self.now;
                let b = self.b();
                b.accepted = arg(d, 0);
                b.posted_at = now;
            }
            (WRAPPER2, s) if s == en::MINT || s == en::MINT_DEPOSIT => {
                let t: Word = d[4..36].try_into().unwrap();
                self.b().items.push(t);
            }
            _ => panic!("an unexpected home-chain call to 0x{}: {}", hex::encode(to), hex::encode(d)),
        }
        true
    }

    /// Mine an Ethereum transaction: it spends its value and fee; a fill moves the token to us, a transfer to its destination.
    fn mine_eth(&mut self, i: usize, success: bool, block: u64) {
        let (tx, hash) = self.eth_signed[i].clone();
        let fee = 21_000 * 1_000_000_000;
        self.eth_balance -= fee + if success { tx.value } else { 0 };
        self.eth_nonce = self.eth_nonce.max(tx.nonce + 1);
        if success {
            if tx.to == sp::SEAPORT_1_6 {
                let token = if tx.data[0] == 0xba { APE } else { 9_180 };
                self.owners.insert(sp::word_u128(token), US);
            } else if tx.to == MILADY || tx.to == BAYC {
                let dest: [u8; 20] = tx.data[4 + 32 + 12..4 + 64].try_into().unwrap();
                self.owners.insert(tx.data[4 + 64..4 + 96].try_into().unwrap(), dest);
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
}

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for Fake {}

impl EthIo for Fake {
    fn frog_relay_busy(&self) -> bool { self.frog_busy }
    fn frog_relay_waiting(&self) -> bool { self.frog_waiting }
    fn now(&self) -> u64 { self.now }
    fn home_chain(&self) -> u64 { 4663 }
    fn eth_chain(&self) -> u64 { 1 }
    fn config(&self) -> Option<EthNftConfig> { Some(self.cfg()) }
    fn paused(&self) -> bool { self.paused }
    fn eth_max_fee(&self) -> u128 { 50_000_000_000 }
    fn save(&mut self, _st: &EthState) { self.saved += 1; }
    fn with_caps<R>(&mut self, f: impl FnOnce(&CapPolicy, &mut Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _key: &str, _c: &str, flow: Flow, _amount: u128) -> Result<(), String> {
        if self.mint_wait && flow == Flow::Mint { Err("waits (outflow limit): 3 of 3 Mint in the last 24 h".into()) } else { Ok(()) }
    }
    fn outflow_record(&mut self, key: &str, _c: &str, flow: Flow, amount: u128) { self.outflow.push((key.into(), flow, amount)); }

    async fn our_evm(&mut self) -> Result<[u8; 20], String> { Ok(US) }
    async fn home_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> {
        if to == rc::RELAY_DEPOSITORY_4663 && sel(d) == rc::ALLOCATOR {
            return Ok(addr_word(&self.allocator));
        }
        let s = sel(d);
        Ok(match to {
            MODULE => {
                if s == etch_autobuyer::floorsink::NET_UNITS { word(self.net_units) }
                else if s == etch_autobuyer::floorsink::ESCROW { word(10 * E) }
                else if s == etch_autobuyer::floorsink::BUFFER { word(E / 20) }
                else if s == etch_autobuyer::floorsink::FLOOR { [word(self.accepted), word(self.posted_at as u128), word(3_600)].concat() }
                else if s == etch_autobuyer::floorsink::MAX_ITEM_COST { word(if self.transit { 0 } else { self.item_cap }) }
                else if s == etch_autobuyer::floorsink::ITEMS_ARRIVED { word(0) }
                else if s == rc::TRANSIT_UNITS { word(if self.transit { E } else { 0 }) }
                else if s == en::ACCEPTED { word(self.accepted) }
                else if s == en::KEEPER { addr_word(&self.keeper) }
                else if s == ef::FLOOR_BOUNDS { [word(0), word(u128::MAX)].concat() }
                else { word(0) }
            }
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
            MODULE2 => {
                let b = self.bayc.as_ref().ok_or("no second module")?;
                if s == etch_autobuyer::floorsink::NET_UNITS { word(b.net_units) }
                else if s == etch_autobuyer::floorsink::ESCROW { word(20 * E) }
                else if s == etch_autobuyer::floorsink::BUFFER { word(E / 20) }
                else if s == etch_autobuyer::floorsink::FLOOR { [word(b.accepted), word(b.posted_at as u128), word(3_600)].concat() }
                else if s == etch_autobuyer::floorsink::MAX_ITEM_COST { word(if b.transit { 0 } else { b.item_cap }) }
                else if s == rc::TRANSIT_UNITS { word(if b.transit { E } else { 0 }) }
                else if s == en::ACCEPTED { word(b.accepted) }
                else if s == en::KEEPER { addr_word(&self.keeper) }
                else if s == ef::FLOOR_BOUNDS { [word(0), word(u128::MAX)].concat() }
                else { word(0) }
            }
            WRAPPER2 => {
                let b = self.bayc.as_ref().ok_or("no second wrapper")?;
                if s == en::ITEM {
                    [b.items[arg(d, 0) as usize].to_vec(), word(1), word(0)].concat()
                } else if s == en::ITEM_OF {
                    let t: Word = d[4..36].try_into().unwrap();
                    word(b.items.iter().position(|x| *x == t).map_or(0, |i| i as u128 + 1))
                } else { word(0) }
            }
            _ => return Err(format!("no contract at 0x{}", hex::encode(to))),
        })
    }
    async fn home_clock(&mut self) -> Result<Clock, String> { Ok(Clock { time: self.now, parent_block: 26_000_000 }) }
    async fn home_balance(&mut self) -> Result<u128, String> { Ok(self.home_balance) }
    async fn home_sign(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> {
        let raw = [&to[..], &value.to_be_bytes(), &self.home_nonce.to_be_bytes(), data].concat();
        let h = keccak(&raw);
        Ok((raw, h, self.home_nonce))
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
        assert!(to == MILADY || to == BAYC, "an Ethereum read of 0x{}", hex::encode(to));
        assert_eq!(sel(d), sp::OWNER_OF);
        let t: Word = d[4..36].try_into().unwrap();
        Ok(addr_word(self.owners.get(&t).unwrap_or(&[0x0e; 20])))
    }
    async fn eth_balance(&mut self) -> Result<u128, String> { Ok(self.eth_balance) }
    async fn eth_nonce(&mut self) -> Result<u64, String> { Ok(self.eth_nonce) }
    async fn eth_receipt(&mut self, h: [u8; 32]) -> Result<Option<OurReceipt>, String> { Ok(self.receipts.get(&h).copied()) }
    async fn eth_plan_buy(&mut self, _order: &str, collection: [u8; 20], cap: u128) -> Result<EthPlan, String> {
        let (total, data, token) = match collection {
            MILADY => (self.listing_total, vec![0xfb, 0x0f, 0x3e, 0xe1], 9_180),
            BAYC => (self.bayc.as_ref().unwrap().listing_total, vec![0xba, 0x0f, 0x3e, 0xe1], APE),
            _ => panic!("a buy on 0x{}", hex::encode(collection)),
        };
        if total > cap {
            return Err(format!("refused (over cap): the order costs {total} wei; the recorded cap is {cap}"));
        }
        Ok(EthPlan { to: sp::SEAPORT_1_6, value: total, data, gas: 200_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce: self.eth_nonce, token: sp::word_u128(token), total })
    }
    async fn eth_plan_transfer(&mut self, collection: [u8; 20], token: Word, dest: [u8; 20]) -> Result<EthPlan, String> {
        if self.transfer_refused {
            return Err("refused (simulation): it reverts at the pin on every provider: blocked".into());
        }
        Ok(EthPlan { to: collection, value: 0, data: en::transfer_from_call(&US, &dest, token), gas: 60_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce: self.eth_nonce, token, total: 0 })
    }
    async fn eth_sign(&mut self, tx: &Tx1559) -> Result<Vec<u8>, String> {
        let raw = [&tx.nonce.to_be_bytes()[..], &tx.to, &tx.value.to_be_bytes(), &tx.max_fee.to_be_bytes(), &tx.data].concat();
        self.eth_signed.push((tx.clone(), keccak(&raw)));
        Ok(raw)
    }
    async fn eth_broadcast(&mut self, _raw: &[u8]) {}
    async fn eth_listing(&mut self, s: &str, _c: [u8; 20]) -> Result<Seen, String> {
        let (o, p) = s.split_once(':').ok_or("not a fake listing")?;
        if o == "bad" {
            return Err("cancelled or filled".into());
        }
        Ok(Seen { offerer: [o.parse::<u8>().unwrap(); 20], token: sp::word_u128(o.parse().unwrap()), price: p.parse().unwrap() })
    }
    async fn eth_sale_receipt(&mut self, h: [u8; 32]) -> Result<Option<ReceiptV>, String> {
        if let Some(r) = self.our_logs.get(&h) {
            return Ok(Some(r.clone()));
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
        let nonce = self.eth_nonce;
        Ok(EthPlan { to, value, data: data.to_vec(), gas: 60_000, max_fee: 1_000_000_000, priority_fee: 100_000_000, nonce, token: [0; 32], total: value })
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

fn sales() -> Vec<[u8; 32]> {
    let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
    v["receipts"].as_object().unwrap().keys().map(|k| hex::decode(&k[2..]).unwrap().try_into().unwrap()).collect()
}

fn fill_hex() -> String {
    let v: serde_json::Value = serde_json::from_str(FILL).unwrap();
    v["input"].as_str().unwrap().to_string()
}

/// Up to a buy's arrival on Ethereum: wanted → advance → quote judged → deposit → Relay pays our Ethereum address.
fn to_arrival(f: &mut Fake, st: &mut EthState) {
    tick(f, st);
    assert_eq!(phase(st), BPhase::Advancing, "the module wants an item: a buy starts and the module advances its budget");
    tick(f, st);
    let BPhase::Funded { wei, .. } = phase(st) else { panic!("{:?}", phase(st)) };
    assert_eq!(wei, DEPOSIT, "the advance less the gas keep");
    let ask = st.buys[0].ask.clone().unwrap();
    assert_eq!((ask.recipient.clone(), ask.origin_chain_id, ask.destination_chain_id), (format!("0x{}", hex::encode(US)), 4663, 1));
    ef::put_quote(st, 0, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Depositing { .. }), "the deposit is out: {:?}", phase(st));
    tick(f, st);
    assert!(matches!(phase(st), BPhase::Bridging { .. }), "{:?}", phase(st));
    assert_eq!(st.used_orders.len(), 1);
    // Relay pays 0.4984 ETH on Ethereum
    f.eth_balance += 498_443_962_449_863_321;
    tick(f, st);
    let BPhase::Arrived { wei, .. } = phase(st) else { panic!("{:?}", phase(st)) };
    assert_eq!(wei, 498_443_962_449_863_321);
}

fn evidence(f: &mut Fake, st: &mut EthState) {
    let cfg = f.cfg();
    ef::put_evidence(st, &cfg, MILADY, vec![format!("1:{}", 92 * E / 100), format!("2:{}", 93 * E / 100), format!("3:{}", 95 * E / 100)],
        sales(), f.now).unwrap();
}

#[test]
fn a_buy_end_to_end_advance_relay_seaport_finality_mint_settle() {
    let mut f = Fake::new();
    f.accepted = FLOOR0; // the module's floor, a little under what the evidence says: a post is due
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&mut f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    let r = st.reading(&MILADY).unwrap().clone();
    assert_eq!(r.post.listing, 93 * E / 100, "rank 2 of 3 offerers");
    assert_eq!(r.post.sales, 5, "five real sales in the window");
    let BPhase::Buying { total, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    assert_eq!(total, 45 * E / 100);
    assert_eq!(f.eth_signed.len(), 1);
    assert_eq!(f.eth_signed[0].0.value, 45 * E / 100);
    assert!(st.line.is_some(), "the fill's bytes are saved on the line before the send");
    // the floor post went out as the home-chain act of this tick (no other act was due)
    assert_eq!(f.posted.map(|p| p.0), Some(r.post.price));
    // mined, ours: bought; not final yet
    f.mine_eth(0, true, ETH_PIN - 10);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Bought { .. }), "{:?}", phase(&st));
    assert!(st.line.is_none());
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Bought { .. }), "not final: nothing is minted");
    assert!(f.items.is_empty());
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st); // final → the mint is the home act
    assert_eq!(f.items, vec![sp::word_u128(9_180)]);
    assert!(matches!(phase(&st), BPhase::Minting { .. }), "{:?}", phase(&st));
    tick(&mut f, &mut st); // mined → minted → settle
    let BPhase::Settling { cost, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    let want = en::cost_of(DEPOSIT + rc::GAS_KEEP_WEI, 45 * E / 100, 498_443_962_449_863_321).unwrap();
    assert_eq!(cost, want, "the item's share of the advance");
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Settled { .. }));
    assert_eq!(f.settled, Some(want));
    // the custody book holds the token under its own id; H3 counted the value at the advance and one mint
    assert!(f.book.contains(&format!("{}:{}", custodycap::evm_key(1, &MILADY), hex::encode(sp::word_u128(9_180)))));
    assert!(f.outflow.iter().any(|(_, fl, v)| *fl == Flow::Value && *v == DEPOSIT + rc::GAS_KEEP_WEI));
    assert!(f.outflow.iter().any(|(_, fl, _)| *fl == Flow::Mint));
}

#[test]
fn a_listing_over_the_cap_is_refused_and_nothing_is_signed() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&mut f, &mut st);
    f.listing_total = 496 * E / 1000; // over what arrived less the gas reserve (0.4954)
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("refused")), "{l:?}");
    assert!(f.eth_signed.is_empty());
    assert!(matches!(phase(&st), BPhase::Arrived { .. }));
    // without our own floor reading, no cap at all
    let mut st2 = EthState::default();
    let mut f2 = Fake::new();
    to_arrival(&mut f2, &mut st2);
    ef::put_listing(&mut st2, 0, fill_hex(), f2.now).unwrap();
    let l = tick(&mut f2, &mut st2);
    assert!(l.iter().any(|x| x.contains("no floor reading")), "{l:?}");
    assert!(f2.eth_signed.is_empty());
    // a stale hand-in is not used
    evidence(&mut f2, &mut st2);
    tick(&mut f2, &mut st2);
    ef::put_listing(&mut st2, 0, fill_hex(), f2.now).unwrap();
    f2.now += ef::HANDIN_TTL_SECS + 1;
    let l = tick(&mut f2, &mut st2);
    assert!(l.iter().any(|x| x.contains("stale")), "{l:?}");
    assert!(f2.eth_signed.is_empty());
}

#[test]
fn a_tampered_quote_signs_nothing_and_the_allocator_is_pinned() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let mut v: serde_json::Value = serde_json::from_str(QUOTE).unwrap();
    v["protocol"]["v2"]["orderData"]["output"]["payments"][0]["recipient"] = "0x1111111111111111111111111111111111111111".into();
    ef::put_quote(&mut st, 0, serde_json::to_vec(&v).unwrap(), f.now).unwrap();
    let before = f.home_calls.len();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("quote refused")), "{l:?}");
    assert_eq!(f.home_calls.len(), before, "no deposit");
    assert!(matches!(phase(&st), BPhase::Funded { .. }));
    // the real quote, but Relay rotated its allocator
    f.allocator = [9; 20];
    ef::put_quote(&mut st, 0, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("allocator")), "{l:?}");
    assert_eq!(f.home_calls.len(), before);
    // a quote while not funded is refused at the door
    let mut st2 = EthState::default();
    assert!(ef::put_quote(&mut st2, 0, vec![], f.now).is_err());
}

#[test]
fn refunds_retry_then_the_eth_goes_back_to_the_module() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    for attempt in 1..=3u32 {
        // Relay's quote: its id is new each time only in the real world; here the order id is reused, which rule 12 refuses
        st.used_orders.clear();
        ef::put_quote(&mut st, 0, QUOTE.as_bytes().to_vec(), f.now).unwrap();
        tick(&mut f, &mut st);
        tick(&mut f, &mut st);
        assert!(matches!(phase(&st), BPhase::Bridging { .. }), "attempt {attempt}: {:?}", phase(&st));
        // Relay refunds on 4663
        f.home_balance += DEPOSIT - 1_000;
        tick(&mut f, &mut st);
        if attempt < 3 {
            let BPhase::Funded { wei, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
            assert!(wei <= DEPOSIT);
            // keep the ask at the fixture's amount for the next attempt
            st.buys[0].phase = BPhase::Funded { wei: DEPOSIT, since: f.now };
        }
    }
    assert!(matches!(phase(&st), BPhase::Returning { .. }), "{:?}", phase(&st));
    tick(&mut f, &mut st);
    assert!(matches!(st.buys[0].phase, BPhase::Returned { .. }), "{:?}", st.buys[0].phase);
    assert!(f.returned.is_some());
    assert!(!f.book.contains("eth:0"), "the first buy's reservation is released");
}

#[test]
fn no_arrival_and_no_refund_past_the_deadline_is_stuck_and_a_late_fill_still_counts() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    ef::put_quote(&mut st, 0, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    f.now += 8 * 86_400;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Stuck { .. }), "{:?}", phase(&st));
    assert!(ef::give_back_now(&mut st.clone(), 0, 0, f.now).is_err(), "nothing at our address to give back");
    f.eth_balance += 490_000_000_000_000_000;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Arrived { .. }), "a late fill is still an arrival");
}

fn resolved(f: &mut Fake, dest: [u8; 20], deliver_by: u64) {
    f.items.push(sp::word_u128(77));
    f.owners.insert(sp::word_u128(77), US);
    f.tickets.push(Ticket { stage: Stage::Resolved, item: (f.items.len() - 1) as u128, dest, deliver_by });
    f.net_units = 0; // no buy wanted in these tests
}

#[test]
fn a_redemption_is_delivered_final_and_recorded() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }), "{:?}", st.deliveries[0].phase);
    let (tx, _) = &f.eth_signed[0];
    assert_eq!((tx.to, tx.value), (MILADY, 0));
    assert_eq!(tx.data, en::transfer_from_call(&US, &ALICE, sp::word_u128(77)));
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }));
    assert_eq!(f.tickets[0].stage, Stage::Resolved, "not recorded before finality");
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st); // final → noteSent (review round 4, A4-L-2: noted at finality, from the same receipt as the record)
    assert_eq!(f.noted, vec![0]);
    tick(&mut f, &mut st); // → record
    assert_eq!(f.tickets[0].stage, Stage::Delivered);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Recorded { .. }), "{:?}", st.deliveries[0].phase);
    assert!(f.outflow.iter().any(|(_, fl, n)| *fl == Flow::Item && *n == 1), "H3 counted one item out");
    assert!(f.marked.is_empty());
}

#[test]
fn a_delivery_pending_a_day_before_its_deadline_is_cancelled_then_marked_once_final() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 3 * 86_400);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }));
    f.now = NOW + 2 * 86_400 + 1; // within a day of deliverBy, still unmined
    tick(&mut f, &mut st);
    let DPhase::Cancelling { nonce, .. } = st.deliveries[0].phase.clone() else { panic!("{:?}", st.deliveries[0].phase) };
    let (cancel, _) = &f.eth_signed[1];
    assert_eq!((cancel.nonce, cancel.to, cancel.value, cancel.data.len()), (nonce, US, 0, 0), "a 0-value self-send at the same nonce");
    assert!(cancel.max_fee > f.eth_signed[0].0.max_fee && cancel.max_priority_fee > f.eth_signed[0].0.max_priority_fee, "fees bumped");
    f.mine_eth(1, true, ETH_PIN - 3);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::MarkAfter { .. }), "{:?}", st.deliveries[0].phase);
    assert!(f.marked.is_empty(), "not marked before the cancel is final");
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert_eq!(f.marked, vec![0]);
    assert!(matches!(st.deliveries[0].phase, DPhase::Marked { .. }));
}

#[test]
fn if_the_transfer_wins_the_race_with_its_cancel_it_is_a_delivery() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 3 * 86_400);
    tick(&mut f, &mut st);
    f.now = NOW + 2 * 86_400 + 1;
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }));
    f.mine_eth(0, true, ETH_PIN - 3); // the transfer, not the cancel
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }), "{:?}", st.deliveries[0].phase);
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st); // noteSent at finality (round 4, A4-L-2), then the record
    tick(&mut f, &mut st);
    assert_eq!(f.tickets[0].stage, Stage::Delivered);
    assert!(f.marked.is_empty(), "never marked: it was delivered");
}

#[test]
fn a_transfer_that_cannot_succeed_is_never_sent_and_is_marked() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    f.transfer_refused = true;
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    assert!(f.eth_signed.is_empty());
    tick(&mut f, &mut st);
    assert!(f.marked.is_empty(), "one agreed revert is not believed at once (audit 2026-09-27-nft B-M-2)");
    f.now += ef::SIM_REFUSAL_SECS;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert_eq!(f.marked, vec![0], "an agreed revert that stands for an hour: never sent, marked");
    // a reverted transfer is marked only once its block is final
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    f.mine_eth(0, false, ETH_PIN - 1);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::MarkAfter { .. }));
    assert!(f.marked.is_empty());
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert_eq!(f.marked, vec![0]);
}

#[test]
fn a_relist_ticket_is_never_delivered() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, US, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(f.eth_signed.is_empty());
    assert_eq!(f.marked, vec![0]);
}

#[test]
fn the_floor_is_posted_only_as_the_modules_keeper_and_only_when_due() {
    let mut f = Fake::new();
    f.net_units = 0;
    f.accepted = FLOOR0;
    let mut st = EthState::default();
    evidence(&mut f, &mut st);
    f.keeper = [0; 20]; // the registry paused the keeper: PosterSource says nobody
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("not us")), "{l:?}");
    assert!(f.posted.is_none());
    f.keeper = US;
    evidence(&mut f, &mut st);
    tick(&mut f, &mut st);
    let (price, conf, depth) = f.posted.unwrap();
    assert_eq!(price, 93 * E / 100);
    assert!(conf >= 7_000);
    assert!(depth >= 2);
    // the same again a minute later: not due
    f.posted = None;
    f.now += 60;
    evidence(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(f.posted.is_none(), "not due: the price did not move and the post is fresh");
    // too few verified listings: no post, the module goes stale by design
    let cfg = f.cfg();
    f.now += 3_600;
    ef::put_evidence(&mut st, &cfg, MILADY, vec!["1:1".into(), "bad:1".into()], sales(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("no post")), "{l:?}");
    assert!(f.posted.is_none());
}

const HOLDER: [u8; 20] = [0x33; 20];
const RECIPIENT: [u8; 20] = [0x44; 20];

/// `safeTransferFrom(holder, us, token, data)` as the holder's own transaction input.
fn safe_input(holder: [u8; 20], token: u128, data: &[u8]) -> Vec<u8> {
    let mut padded = data.to_vec();
    padded.resize(data.len().div_ceil(32) * 32, 0);
    [&en::SAFE_TRANSFER_FROM_DATA[..], &addr_word(&holder), &addr_word(&US), &word(token), &word(0x80), &word(data.len() as u128), &padded].concat()
}
fn transfer_log(collection: [u8; 20], from: [u8; 20], to: [u8; 20], token: u128) -> en::LogV {
    en::LogV { address: collection, topics: vec![en::TRANSFER, sp::word_addr(&from), sp::word_addr(&to), sp::word_u128(token)], data: vec![] }
}
/// A mined transaction that moved `token` from the holder to us, with `input` as its calldata; the NFT is ours from then.
fn arrive(f: &mut Fake, tx: [u8; 32], token: u128, input: Vec<u8>, block: u64) {
    let r = ReceiptV { block, success: true, logs: vec![transfer_log(MILADY, HOLDER, US, token)] };
    f.deposit_txs.insert(tx, (HOLDER, MILADY, input, r.clone()));
    f.sale_receipts.insert(tx, r);
    f.owners.insert(sp::word_u128(token), US);
}
fn deposits_on() -> (Fake, EthState) {
    let mut f = Fake::new();
    f.net_units = 0; // no buy wanted
    f.deposits_on = true;
    (f, EthState::default())
}

#[test]
fn deposits_are_on_at_launch_minted_only_past_finality_and_bound_to_this_wrapper() {
    // D5 (2026-09-27): holder deposits are live; with the switch off the claim is refused at the door
    let mut f = Fake::new();
    assert!(ef::put_deposit(&mut EthState::default(), &f.cfg(), [0x42; 32], f.now).unwrap_err().contains("off"));
    let (mut f2, mut st) = deposits_on();
    f = std::mem::replace(&mut f2, Fake::new());
    let tx = [0x42; 32];
    arrive(&mut f, tx, 555, safe_input(HOLDER, 555, &en::deposit_data(4663, &WRAPPER, &RECIPIENT)), ETH_PIN - 1);
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    assert!(ef::put_deposit(&mut st, &f.cfg(), tx, f.now).is_err(), "claimed once");
    tick(&mut f, &mut st);
    assert_eq!(st.deposits[0].phase, XPhase::Seen);
    assert_eq!((st.deposits[0].recipient, st.deposits[0].from, st.deposits[0].block), (RECIPIENT, HOLDER, ETH_PIN - 1));
    tick(&mut f, &mut st);
    assert!(f.items.is_empty(), "not final: nothing is minted");
    f.finalized = ETH_PIN;
    evidence(&mut f, &mut st); // a deposit mints only against a fresh, verified floor (audit 2026-09-27-nft A-M-1)
    tick(&mut f, &mut st);
    assert_eq!(f.items, vec![sp::word_u128(555)]);
    let (_, _, call) = f.home_calls.last().unwrap().clone();
    assert_eq!(call, en::mint_call(true, sp::word_u128(555), tx, &RECIPIENT), "mintDeposit to the recipient the holder signed");
    tick(&mut f, &mut st);
    assert_eq!(st.deposits[0].phase, XPhase::Minted);
    assert!(f.book.contains(&format!("{}:{}", custodycap::evm_key(1, &MILADY), hex::encode(sp::word_u128(555)))));
    // round 4 (E4-H-1): a mint's H3 key is its collection's next mint
    assert!(f.outflow.iter().any(|(k, fl, _)| *fl == Flow::Mint && *k == format!("mint:eth:{}:0", custodycap::evm_key(1, &MILADY))));
    assert!(f.eth_signed.is_empty(), "nothing is returned");
}

#[test]
fn strays_go_back_to_their_sender_once_final_and_are_never_minted() {
    let (mut f, mut st) = deposits_on();
    // a plain transferFrom (no claim data), and a deposit naming another wrapper
    let (t1, t2) = ([0x51; 32], [0x52; 32]);
    arrive(&mut f, t1, 601, en::transfer_from_call(&HOLDER, &US, sp::word_u128(601)), ETH_PIN - 2);
    arrive(&mut f, t2, 602, safe_input(HOLDER, 602, &en::deposit_data(4663, &[0xcc; 20], &RECIPIENT)), ETH_PIN - 2);
    ef::put_deposit(&mut st, &f.cfg(), t1, f.now).unwrap();
    ef::put_deposit(&mut st, &f.cfg(), t2, f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(&st.deposits[0].phase, XPhase::Return { why } if why.contains("safeTransferFrom")), "{:?}", st.deposits[0].phase);
    assert!(matches!(&st.deposits[1].phase, XPhase::Return { why } if why.contains("wrapper")), "{:?}", st.deposits[1].phase);
    assert!(f.eth_signed.is_empty(), "not before its block is final");
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st);
    let (tx, _) = f.eth_signed[0].clone();
    assert_eq!((tx.to, tx.value), (MILADY, 0));
    assert_eq!(tx.data, en::transfer_from_call(&US, &HOLDER, sp::word_u128(601)), "back to its own Transfer's sender, by transferFrom");
    assert!(matches!(st.deposits[0].phase, XPhase::Returning { .. }));
    assert!(matches!(st.line.as_ref().unwrap().kind, ef::EthKind::Return { .. }), "saved on the line before the send");
    assert!(f.outflow.iter().any(|(k, fl, n)| *fl == Flow::Return && *n == 1 && k.starts_with("return:eth:")), "its own budget, never the freeze");
    f.mine_eth(0, true, ETH_PIN - 1);
    tick(&mut f, &mut st); // mined → returned; the second goes out
    assert!(matches!(st.deposits[0].phase, XPhase::Returned { .. }));
    assert_eq!(st.last_out(&MILADY, &sp::word_u128(601)), Some(ETH_PIN - 1));
    assert!(matches!(st.deposits[1].phase, XPhase::Returning { .. }));
    assert!(f.items.is_empty(), "no stray is ever minted");
}

#[test]
fn a_claim_not_readable_yet_waits_unrecorded_and_is_dropped_after_its_window() {
    let (mut f, mut st) = deposits_on();
    let tx = [0x61; 32];
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("tried again")), "{l:?}");
    assert!(st.deposits.is_empty(), "a transaction not seen at the pin is not refused for good");
    assert_eq!(st.inbox.deposits.len(), 1);
    // it mines: judged at the next tick
    arrive(&mut f, tx, 700, safe_input(HOLDER, 700, &en::deposit_data(4663, &WRAPPER, &RECIPIENT)), ETH_PIN - 1);
    tick(&mut f, &mut st);
    assert_eq!(st.deposits[0].phase, XPhase::Seen);
    // one never seen is dropped after CLAIM_RETRY_SECS, unrecorded: it may be claimed again
    let t2 = [0x62; 32];
    ef::put_deposit(&mut st, &f.cfg(), t2, f.now).unwrap();
    f.now += ef::CLAIM_RETRY_SECS + 1;
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("dropped")), "{l:?}");
    assert!(st.inbox.deposits.is_empty() && !st.deposits.iter().any(|d| d.tx == t2));
    ef::put_deposit(&mut st, &f.cfg(), t2, f.now).unwrap();
}

#[test]
fn a_deposit_with_no_custody_room_goes_back_and_another_collection_is_never_touched() {
    let (mut f, mut st) = deposits_on();
    // stage 0 holds 6 items an Ethereum collection (J7): fill it
    let key = custodycap::evm_key(1, &MILADY);
    for i in 0..custodycap::STAGE0_EVM_ITEMS as u128 {
        custodycap::admit(&f.caps, &mut f.book, &format!("{key}:{i}"), &key, 0).unwrap();
    }
    let tx = [0x71; 32];
    arrive(&mut f, tx, 800, safe_input(HOLDER, 800, &en::deposit_data(4663, &WRAPPER, &RECIPIENT)), ETH_PIN - 1);
    f.finalized = ETH_PIN;
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    tick(&mut f, &mut st); // judged
    tick(&mut f, &mut st); // final: no room → return; then signed
    assert!(matches!(&st.deposits[0].phase, XPhase::Return { why } if why.contains("custody room")) || matches!(st.deposits[0].phase, XPhase::Returning { .. }),
        "{:?}", st.deposits[0].phase);
    tick(&mut f, &mut st);
    assert!(matches!(st.deposits[0].phase, XPhase::Returning { .. }), "{:?}", st.deposits[0].phase);
    assert!(f.items.is_empty());
    // an NFT of a collection we do not serve: nothing is recorded to return, and no call is ever made to that contract
    let t2 = [0x72; 32];
    let other = [0x77; 20];
    let r = ReceiptV { block: ETH_PIN - 1, success: true, logs: vec![transfer_log(other, HOLDER, US, 5)] };
    f.deposit_txs.insert(t2, (HOLDER, other, en::transfer_from_call(&HOLDER, &US, sp::word_u128(5)), r));
    ef::put_deposit(&mut st, &f.cfg(), t2, f.now).unwrap();
    tick(&mut f, &mut st);
    let d = st.deposits.iter().find(|d| d.tx == t2).unwrap();
    assert!(matches!(&d.phase, XPhase::Refused { why } if why.contains("configured collection")), "{:?}", d.phase);
    // our own transaction (the runner's discovery sees our Seaport fills arrive too): never a claim, never returned
    let t3 = [0x73; 32];
    let r = ReceiptV { block: ETH_PIN - 1, success: true, logs: vec![transfer_log(MILADY, [0x0f; 20], US, 9_180)] };
    f.deposit_txs.insert(t3, (US, sp::SEAPORT_1_6, vec![0xfb, 0x0f, 0x3e, 0xe1], r));
    ef::put_deposit(&mut st, &f.cfg(), t3, f.now).unwrap();
    tick(&mut f, &mut st);
    let d = st.deposits.iter().find(|d| d.tx == t3).unwrap();
    assert!(matches!(&d.phase, XPhase::Refused { why } if why.contains("our own")), "{:?}", d.phase);
}

#[test]
fn a_return_is_never_signed_for_a_held_token_an_old_transfer_or_under_the_gas_kept_for_deliveries() {
    // a token the wrapper holds (it backs a unit): held, never returned
    let (mut f, mut st) = deposits_on();
    let tx = [0x81; 32];
    arrive(&mut f, tx, 900, en::transfer_from_call(&HOLDER, &US, sp::word_u128(900)), ETH_PIN - 1);
    f.items.push(sp::word_u128(900));
    f.finalized = ETH_PIN;
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(&st.deposits[0].phase, XPhase::Held { why } if why.contains("wrapper holds")), "{:?}", st.deposits[0].phase);
    assert!(f.eth_signed.is_empty());
    // an old transfer: the token left us (a delivery) after it came in, and is back by another transaction
    let (mut f, mut st) = deposits_on();
    let tx = [0x82; 32];
    arrive(&mut f, tx, 901, en::transfer_from_call(&HOLDER, &US, sp::word_u128(901)), ETH_PIN - 50);
    st.outbound.push((MILADY, sp::word_u128(901), ETH_PIN - 40));
    f.finalized = ETH_PIN;
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(&st.deposits[0].phase, XPhase::Held { why } if why.contains("left us")), "{:?}", st.deposits[0].phase);
    assert!(f.eth_signed.is_empty());
    // the gas pot is low: the return waits, nothing is signed
    let (mut f, mut st) = deposits_on();
    let tx = [0x83; 32];
    arrive(&mut f, tx, 902, en::transfer_from_call(&HOLDER, &US, sp::word_u128(902)), ETH_PIN - 1);
    f.finalized = ETH_PIN;
    f.eth_balance = ef::RETURN_GAS_KEEP_WEI;
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    tick(&mut f, &mut st);
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("kept for deliveries")), "{l:?}");
    assert!(f.eth_signed.is_empty());
    assert!(matches!(st.deposits[0].phase, XPhase::Return { .. }));
    // a transfer that cannot succeed (every provider agrees, for an hour): held for the operator
    f.eth_balance = E / 100;
    f.transfer_refused = true;
    tick(&mut f, &mut st);
    assert!(matches!(st.deposits[0].phase, XPhase::Return { .. }), "one agreed revert is not believed at once: {:?}", st.deposits[0].phase);
    f.now += ef::SIM_REFUSAL_SECS;
    tick(&mut f, &mut st);
    assert!(matches!(&st.deposits[0].phase, XPhase::Held { why } if why.contains("cannot succeed")), "{:?}", st.deposits[0].phase);
}

#[test]
fn a_deposit_waiting_on_the_mint_limit_never_holds_up_the_floor_and_a_reorg_before_finality_is_followed() {
    let (mut f, mut st) = deposits_on();
    f.accepted = FLOOR0;
    f.mint_wait = true;
    let tx = [0x91; 32];
    arrive(&mut f, tx, 950, safe_input(HOLDER, 950, &en::deposit_data(4663, &WRAPPER, &RECIPIENT)), ETH_PIN - 30);
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    tick(&mut f, &mut st);
    // before finality the transaction was re-mined in a later block: the deposit follows it
    let mut r = f.sale_receipts[&tx].clone();
    r.block = ETH_PIN - 5;
    f.sale_receipts.insert(tx, r);
    f.finalized = ETH_PIN - 20;
    tick(&mut f, &mut st);
    assert_eq!((st.deposits[0].phase.clone(), st.deposits[0].block), (XPhase::Seen, ETH_PIN - 5), "the block it is final in");
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st);
    assert_eq!(st.deposits[0].phase, XPhase::Mint);
    evidence(&mut f, &mut st);
    let l = tick(&mut f, &mut st);
    assert!(f.posted.is_some(), "the floor is posted while the deposit waits on H3: {l:?}");
    assert!(f.items.is_empty());
    f.mint_wait = false;
    tick(&mut f, &mut st);
    assert_eq!(f.items, vec![sp::word_u128(950)]);
}

#[test]
fn a_deposit_is_not_minted_while_a_purchase_is_between_its_fill_and_its_mint() {
    let mut f = Fake::new();
    f.deposits_on = true;
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&mut f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }));
    let tx = [0xa1; 32];
    arrive(&mut f, tx, 960, safe_input(HOLDER, 960, &en::deposit_data(4663, &WRAPPER, &RECIPIENT)), ETH_PIN - 20);
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    f.mine_eth(0, true, ETH_PIN - 10);
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st); // bought; the deposit is judged
    tick(&mut f, &mut st); // final both: the purchase mints first
    assert_eq!(f.items, vec![sp::word_u128(9_180)], "the purchase's mint, not the deposit's");
    assert_eq!(st.deposits[0].phase, XPhase::Mint);
    tick(&mut f, &mut st); // minted → settle
    tick(&mut f, &mut st); // settled; now the deposit
    tick(&mut f, &mut st);
    assert!(f.items.contains(&sp::word_u128(960)), "{:?}", f.items);
}

#[test]
fn one_home_transaction_at_a_time_and_nothing_signed_while_paused() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    let lines = block_on(ef::tick(&mut f, &mut st, true));
    assert!(lines.iter().any(|l| l.contains("another transaction of ours is out")), "{lines:?}");
    assert!(f.home_calls.is_empty());
    assert_eq!(phase(&st), BPhase::NeedAdvance);
    f.paused = true;
    block_on(ef::tick(&mut f, &mut st, false));
    assert!(f.home_calls.is_empty());
    f.paused = false;
    tick(&mut f, &mut st);
    assert_eq!(f.home_calls.len(), 1, "the advance");
}

#[test]
fn a_module_that_no_longer_wants_the_item_drops_the_buy_before_anything_is_paid() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    let before = f.home_calls.len();
    block_on(ef::tick(&mut f, &mut st, true)); // the buy starts; the act waits
    assert_eq!(phase(&st), BPhase::NeedAdvance);
    f.net_units = 0;
    tick(&mut f, &mut st);
    assert!(st.buys.is_empty());
    assert_eq!(f.home_calls.len(), before);
}

// ---- several collections on one Ethereum address (docs/v7/phase1-nft-pooled.md §4) ------------------------------------------------

const ARRIVED: u128 = 498_443_962_449_863_321;

fn ph(st: &EthState, id: u64) -> BPhase {
    st.buy(id).unwrap().phase.clone()
}

/// A buy from its advance to its arrival on Ethereum. The fixture's order id is reused, so rule 12's list is cleared first (in the
/// real world every quote is a new order).
fn relay_leg(f: &mut Fake, st: &mut EthState, id: u64) {
    for _ in 0..4 {
        if st.buy(id).is_some_and(|b| matches!(b.phase, BPhase::Funded { .. })) {
            break;
        }
        tick(f, st);
    }
    assert!(matches!(ph(st, id), BPhase::Funded { .. }), "buy {id}: {:?}", ph(st, id));
    st.used_orders.clear();
    ef::put_quote(st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    for _ in 0..4 {
        if matches!(ph(st, id), BPhase::Bridging { .. }) {
            break;
        }
        tick(f, st);
    }
    assert!(matches!(ph(st, id), BPhase::Bridging { .. }), "buy {id}: {:?}", ph(st, id));
    f.eth_balance += ARRIVED;
    tick(f, st);
    assert!(matches!(ph(st, id), BPhase::Arrived { wei: ARRIVED, .. }), "buy {id}: {:?}", ph(st, id));
}

/// Our own floor reading of BAYC (as `put_evidence` would leave it from real BAYC sales), so its fills have a cap.
fn bayc_reading(st: &mut EthState, now: u64) {
    let mut r = st.reading(&MILADY).expect("Milady's reading").clone();
    r.collection = BAYC;
    r.post.price = 679 * E / 100;
    r.post.listing = 679 * E / 100;
    r.at = now;
    r.due = false;
    st.readings.retain(|x| x.collection != BAYC);
    st.readings.push(r);
}

/// Both collections' buys arrived on Ethereum: Milady's (buy 0) and BAYC's (buy 1).
fn both_arrived(f: &mut Fake, st: &mut EthState) {
    relay_leg(f, st, 0);
    assert_eq!(st.buys.len(), 2, "Milady's ETH arrived: the Relay leg is free and BAYC's buy starts in the same tick");
    assert_eq!(st.buys[1].collection, BAYC);
    relay_leg(f, st, 1);
}

#[test]
fn one_relay_leg_at_a_time_and_each_collection_its_own_buy() {
    let mut f = Fake::two();
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    assert_eq!(st.buys.len(), 1, "both modules want an item; one buy starts");
    assert_eq!((st.buys[0].collection, st.buys[0].module), (MILADY, MODULE));
    for _ in 0..2 {
        tick(&mut f, &mut st);
        assert_eq!(st.buys.len(), 1, "BAYC waits while Milady's buy holds the Relay leg: {:?}", ph(&st, 0));
    }
    assert!(st.relay_busy());
    relay_leg(&mut f, &mut st, 0);
    assert_eq!(st.buys.len(), 2);
    assert_eq!((st.buys[1].collection, st.buys[1].module, st.buys[1].wrapper), (BAYC, MODULE2, WRAPPER2));
    assert!(st.open_buy_for(&MILADY).is_some() && st.open_buy_for(&BAYC).is_some());
    // BAYC's advance came from BAYC's module; Milady's module was asked for exactly one advance
    let adv = |m: [u8; 20]| f.home_calls.iter().filter(|(to, _, d)| *to == m && sel(d) == rc::ADVANCE).count();
    assert_eq!((adv(MODULE), adv(MODULE2)), (1, 1));
    // no second Milady buy while its own is open, however many units its pool holds
    f.net_units = 5 * E;
    tick(&mut f, &mut st);
    assert_eq!(st.buys.iter().filter(|b| b.collection == MILADY).count(), 1);
}

#[test]
fn a_collection_waiting_on_ethereum_never_holds_up_another_and_pools_never_cross() {
    let mut f = Fake::two();
    f.accepted = FLOOR0;
    let mut st = EthState::default();
    both_arrived(&mut f, &mut st);
    // BAYC's listing is handed in first but it has no floor reading of ours: it waits, and Milady's fill goes out in the same tick
    ef::put_listing(&mut st, 1, fill_hex(), f.now).unwrap();
    evidence(&mut f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    st.rr = 1; // BAYC's turn first
    let l = tick(&mut f, &mut st);
    assert!(st.buy(1).unwrap().note.contains("no floor reading"), "{l:?}");
    assert!(matches!(ph(&st, 0), BPhase::Buying { .. }), "{:?}", ph(&st, 0));
    assert!(matches!(ph(&st, 1), BPhase::Arrived { .. }));
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 10);
    f.finalized = ETH_PIN;
    for _ in 0..5 {
        tick(&mut f, &mut st);
    }
    assert!(matches!(ph(&st, 0), BPhase::Settled { .. }), "Milady's buy completed while BAYC's waited: {:?}", ph(&st, 0));
    assert!(matches!(ph(&st, 1), BPhase::Arrived { .. }), "{:?}", ph(&st, 1));
    assert_eq!(f.items, vec![sp::word_u128(9_180)], "the Milady wrapper got the Milady token");
    assert!(f.settled.is_some());
    {
        let b = f.b();
        assert!(b.items.is_empty() && b.settled.is_none(), "nothing reached BAYC's wrapper or module");
        assert_eq!(b.net_units, 2 * E, "BAYC's pool is untouched");
        assert!(b.transit, "BAYC's advance is still out, its own");
    }
    // BAYC gets a reading of its own: its fill goes out and settles on its own module
    bayc_reading(&mut st, f.now);
    ef::put_listing(&mut st, 1, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    let BPhase::Buying { token, .. } = ph(&st, 1) else { panic!("{:?}", ph(&st, 1)) };
    assert_eq!(token, sp::word_u128(APE));
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    for _ in 0..5 {
        tick(&mut f, &mut st);
    }
    assert!(matches!(ph(&st, 1), BPhase::Settled { .. }), "{:?}", ph(&st, 1));
    let mint2 = f.home_calls.iter().find(|(to, _, d)| *to == WRAPPER2 && sel(d) == en::MINT).expect("BAYC's mint").2.clone();
    assert_eq!(mint2[4..36], sp::word_u128(APE), "the ape to BAYC's wrapper");
    assert_eq!(f.b().items, vec![sp::word_u128(APE)]);
    assert_eq!(f.items, vec![sp::word_u128(9_180)], "and nothing more to Milady's");
    assert!(f.b().settled.is_some());
    assert_eq!(f.b().net_units, E);
    assert!(f.book.contains(&format!("{}:{}", custodycap::evm_key(1, &BAYC), hex::encode(sp::word_u128(APE)))));
}

#[test]
fn a_buy_never_spends_another_collections_arrived_eth() {
    let mut f = Fake::two();
    f.accepted = FLOOR0;
    let mut st = EthState::default();
    both_arrived(&mut f, &mut st);
    f.b().accepted = 679 * E / 100; // BAYC's floor once both buys are under way (the advance gate is behind them)
    evidence(&mut f, &mut st);
    tick(&mut f, &mut st);
    bayc_reading(&mut st, f.now);
    assert_eq!(st.earmarked_except(1), ARRIVED, "Milady's arrived ETH is Milady's");
    assert_eq!(st.earmarked_except(0), ARRIVED);
    // suppose BAYC's arrival were over-read by 0.3 ETH: its cap is still what is not Milady's
    if let Some(b) = st.buy_mut(1) {
        b.phase = BPhase::Arrived { wei: ARRIVED + 3 * E / 10, since: f.now };
    }
    f.b().listing_total = 7 * E / 10;
    let bal = f.eth_balance;
    ef::put_listing(&mut st, 1, fill_hex(), f.now).unwrap();
    let l = tick(&mut f, &mut st);
    let cap = bal - ARRIVED - en::ETH_GAS_RESERVE_WEI;
    assert!(l.iter().any(|x| x.contains(&format!("the recorded cap is {cap}"))), "cap {cap}: {l:?}");
    assert!(f.eth_signed.iter().all(|(t, _)| t.data[0] != 0xba), "nothing signed for BAYC");
    // (an over-read is conservative both ways: while it stood, Milady's cap was also held under the balance less BAYC's earmark)
    if let Some(b) = st.buy_mut(1) {
        b.phase = BPhase::Arrived { wei: ARRIVED, since: f.now };
    }
    // Milady's fill still has all of its own
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(ph(&st, 0), BPhase::Buying { arrived: ARRIVED, .. }), "{:?}", ph(&st, 0));
    assert_eq!(st.earmarked_except(1), ARRIVED, "earmarked until its fill is mined");
    let n = f.eth_signed.len();
    f.mine_eth(n - 1, true, ETH_PIN - 1);
    tick(&mut f, &mut st);
    assert_eq!(st.earmarked_except(1), 0, "mined: what is left is no longer Milady's to hold");
}

#[test]
fn deliveries_go_first_nearest_deadline_first_then_arrived_buys_in_turn() {
    let mut f = Fake::two();
    f.accepted = FLOOR0;
    let mut st = EthState::default();
    both_arrived(&mut f, &mut st);
    evidence(&mut f, &mut st);
    tick(&mut f, &mut st);
    bayc_reading(&mut st, f.now);
    // two redemptions of Milady items: the later ticket has the nearer deadline
    for (tok, by) in [(77u128, NOW + 7 * 86_400), (78, NOW + 3 * 86_400)] {
        f.items.push(sp::word_u128(tok));
        f.owners.insert(sp::word_u128(tok), US);
        f.tickets.push(Ticket { stage: Stage::Resolved, item: (f.items.len() - 1) as u128, dest: ALICE, deliver_by: by });
    }
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    ef::put_listing(&mut st, 1, fill_hex(), f.now).unwrap();
    let mut order = vec![];
    for _ in 0..6 {
        tick(&mut f, &mut st);
        if let Some(t) = st.line.clone() {
            order.push(match t.kind {
                ef::EthKind::Deliver { ticket, .. } => format!("deliver #{ticket}"),
                ef::EthKind::Buy { buy } => format!("buy {buy}"),
                k => format!("{k:?}"),
            });
            let i = f.eth_signed.iter().position(|(_, h)| *h == t.hash).unwrap();
            f.mine_eth(i, true, ETH_PIN - 1);
        }
    }
    assert_eq!(&order[..2], &["deliver #1".to_string(), "deliver #0".to_string()], "{order:?}");
    let mut buys = order[2..4].to_vec();
    buys.sort();
    assert_eq!(buys, vec!["buy 0".to_string(), "buy 1".to_string()], "both arrived buys fill, one after the other: {order:?}");
    // one nonce each, in order: no gap, no reuse
    let nonces: Vec<u64> = f.eth_signed.iter().map(|(t, _)| t.nonce).collect();
    assert_eq!(nonces, vec![0, 1, 2, 3]);
}

#[test]
fn a_stuck_delivery_is_repriced_at_its_nonce_and_the_home_side_goes_on_meanwhile() {
    let mut f = Fake::two();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400); // Milady wants nothing; BAYC does
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }));
    assert_eq!(st.buys.len(), 1);
    assert_eq!(st.buys[0].collection, BAYC);
    let first = f.eth_signed[0].clone();
    f.now += ef::LINE_BUMP_SECS;
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("re-priced")), "{l:?}");
    let (re, rh) = f.eth_signed[1].clone();
    assert_eq!((re.nonce, re.to, re.value, re.data.clone(), re.gas), (first.0.nonce, first.0.to, 0, first.0.data.clone(), first.0.gas),
        "the same transfer at the same nonce");
    assert!(re.max_fee > first.0.max_fee && re.max_priority_fee > first.0.max_priority_fee);
    assert!(re.max_fee <= 50_000_000_000, "never over the chain's cap");
    let line = st.line.clone().unwrap();
    assert_eq!((line.hash, line.earlier.clone()), (rh, vec![first.1]), "every version watched");
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { tx, .. } if tx == rh));
    // meanwhile BAYC's buy went on: its advance, its Relay leg
    relay_leg(&mut f, &mut st, 0);
    // the replacement mines: the delivery moves under its hash
    f.mine_eth(1, true, ETH_PIN - 2);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { tx, .. } if tx == rh), "{:?}", st.deliveries[0].phase);
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert_eq!(f.tickets[0].stage, Stage::Delivered);
}

#[test]
fn a_repriced_fill_whose_first_version_mines_is_still_the_buy() {
    let mut f = Fake::new();
    f.accepted = FLOOR0;
    let mut st = EthState::default();
    to_arrival(&mut f, &mut st);
    evidence(&mut f, &mut st);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }));
    f.now += ef::LINE_BUMP_SECS;
    tick(&mut f, &mut st);
    assert_eq!(f.eth_signed.len(), 2, "re-priced");
    assert_eq!((f.eth_signed[1].0.nonce, f.eth_signed[1].0.value), (f.eth_signed[0].0.nonce, 45 * E / 100));
    // a second re-price 30 minutes later: three versions watched
    f.now += ef::LINE_BUMP_SECS;
    tick(&mut f, &mut st);
    assert_eq!(st.line.as_ref().unwrap().earlier.len(), 2);
    // the FIRST version mines
    f.mine_eth(0, true, ETH_PIN - 10);
    let spent_before = st.eth_out;
    tick(&mut f, &mut st);
    let BPhase::Bought { tx, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    assert_eq!(tx, f.eth_signed[0].1, "the buy names the version that mined");
    assert!(st.line.is_none());
    assert_eq!(st.eth_out - spent_before, 45 * E / 100 + 21_000 * 1_000_000_000, "spent counted once");
    f.finalized = ETH_PIN;
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    assert!(matches!(phase(&st), BPhase::Settled { .. }), "{:?}", phase(&st));
    let (_, _, mint) = f.home_calls.iter().find(|(to, _, d)| *to == WRAPPER && sel(d) == en::MINT).unwrap().clone();
    assert_eq!(mint, en::mint_call(false, sp::word_u128(9_180), f.eth_signed[0].1, &MODULE), "minted against the mined fill");
    // a re-price that would go over the chain's cap waits instead
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    for _ in 0..12 {
        f.now += ef::LINE_BUMP_SECS;
        tick(&mut f, &mut st);
    }
    assert!(f.eth_signed.iter().all(|(t, _)| t.max_fee <= 50_000_000_000));
    assert!(st.line.is_some());
}

// ---- fix round 3 (D3-H-1): a delivery is judged by its own receipt, whoever owns the token afterwards ------------------------------

/// PoC `d3_h1_a_redeemer_who_moves_the_nft_before_finality_is_never_recorded_and_the_ticket_stays_refundable`
/// (review3/poc/D/canister): ALICE moved token 77 on to BOB before finality, and 40 days later the delivery was still `Moved`
/// ("not the destination"), never recorded, never marked: `refund()` would mint the unit back. Now the delivery is recorded from its
/// own receipt's `Transfer(us → ALICE, 77)`, and it was noted sent on the wrapper as soon as it mined (the wrapper's late window can
/// no longer refund it, whatever happens to the record).
#[test]
fn r3_d3_h1_a_redeemer_who_moves_the_nft_on_before_finality_is_still_recorded_and_noted_sent() {
    const BOB: [u8; 20] = [0xb0; 20];
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }), "{:?}", st.deliveries[0].phase);
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }));
    // review round 4 (A4-L-2): noted sent only at finality, from the receipt (a transfer a reorg drops before finality is never noted)
    tick(&mut f, &mut st);
    assert!(f.noted.is_empty() && st.deliveries[0].sent.is_none(), "not noted before finality");
    assert_eq!(f.tickets[0].stage, Stage::Resolved);
    // ALICE moves token 77 to her other wallet BOB (or sells it into a bid) in the next blocks; then finality
    f.owners.insert(sp::word_u128(77), BOB);
    f.finalized = ETH_PIN;
    for _ in 0..3 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.deliveries[0].phase, DPhase::Recorded { .. }), "{:?} ({})", st.deliveries[0].phase, st.deliveries[0].note);
    assert_eq!(f.tickets[0].stage, Stage::Delivered, "recordDelivery: the ticket can never refund");
    assert!(f.marked.is_empty());
    assert_eq!(f.noted, vec![0], "noted once, at finality, before the record");
}

/// The control: a mined delivery whose receipt does not show the token going from us to the destination is never recorded (and never
/// marked: it was noted sent, so only the operator's look ends it).
#[test]
fn r3_d3_h1_a_receipt_without_the_transfer_to_the_destination_is_never_recorded() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    f.mine_eth(0, true, ETH_PIN - 5);
    let hash = f.eth_signed[0].1;
    f.our_logs.get_mut(&hash).unwrap().logs.clear();
    tick(&mut f, &mut st);
    f.finalized = ETH_PIN;
    for _ in 0..3 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }), "{:?}", st.deliveries[0].phase);
    assert!(st.deliveries[0].note.contains("shows no Transfer"), "{}", st.deliveries[0].note);
    assert_eq!(f.tickets[0].stage, Stage::Resolved);
}

/// While paused, a mined delivery is still noted sent (it already left) and a final one still recorded.
#[test]
fn r3_d3_h1_a_mined_delivery_is_noted_sent_while_paused() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    f.paused = true;
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(f.noted.is_empty(), "round 4 (A4-L-2): not before finality");
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert_eq!(f.noted, vec![0], "noted at finality while paused");
    tick(&mut f, &mut st);
    assert_eq!(f.tickets[0].stage, Stage::Delivered);
}

/// Review round 4 (A4-L-2): the PoC `a4_l2_a_noted_delivery_dropped_by_a_reorg_before_finality_has_no_way_to_a_mark`
/// (review4/poc/A): noteSent was signed as soon as the transfer was seen MINED; a reorg then dropped it for good, and 30 days on the
/// delivery was still `Moved`, never marked, and the noted ticket could never refund (`DeliverySentNotRecorded`). The note now waits
/// for finality and the delivery's own receipt, so a transfer dropped before finality is never noted: the wrapper's late window still
/// refunds its ticket, and the delivery is then gone.
#[test]
fn r4_a4_l2_a_delivery_a_reorg_drops_before_finality_is_never_noted_and_its_ticket_still_refunds() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 7 * 86_400);
    tick(&mut f, &mut st);
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }));
    assert!(st.deliveries[0].sent.is_none() && f.noted.is_empty(), "not noted before finality");
    // the reorg: the transfer is gone from the canonical chain and never re-included; the token is ours again
    let hash = f.eth_signed[0].1;
    f.our_logs.remove(&hash);
    f.receipts.remove(&hash);
    f.owners.insert(sp::word_u128(77), US);
    f.finalized = ETH_PIN;
    for _ in 0..(30 * 144) {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    assert!(f.noted.is_empty(), "never noted sent: the wrapper's late window is still open for it");
    assert!(st.deliveries[0].note.contains("never noted sent"), "{}", st.deliveries[0].note);
    // the holder takes the late window's refund (refund() on the wrapper): the canister sees the ticket gone and stops
    f.tickets[0].stage = Stage::Refunded;
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Gone { .. }), "{:?}", st.deliveries[0].phase);
}

/// Round 4 (A4-L-2): a mark (a delivery that can never confirm) is signed while paused too: it signs nothing on Ethereum and only lets
/// its holder be refunded after the grace.
#[test]
fn r4_a4_l2_a_mark_goes_out_while_paused() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    resolved(&mut f, ALICE, NOW + 3 * 86_400);
    tick(&mut f, &mut st);
    f.now = NOW + 2 * 86_400 + 1;
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }));
    f.mine_eth(1, true, ETH_PIN - 3);
    f.paused = true;
    tick(&mut f, &mut st);
    f.finalized = ETH_PIN;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert_eq!(f.marked, vec![0], "marked while paused");
}

/// B-L-6 (docs/audit/2026-09-27-nft/B-canister-judges.md): both sides read a Relay refund as a rise of the same 4663 balance, so one
/// Relay leg canister-wide. No Ethereum buy starts while a Bitcoin conversion holds the leg, nor while one waits for it (fairness).
#[test]
fn b_l_6_no_ethereum_buy_starts_while_the_bitcoin_side_holds_or_waits_for_the_relay_leg() {
    let mut f = Fake::new();
    let mut st = EthState::default();
    f.frog_busy = true;
    tick(&mut f, &mut st);
    assert!(st.buys.iter().all(|b| b.finished()), "the Bitcoin side holds the leg: no buy starts ({:?})", st.buys.first().map(|b| &b.phase));
    f.frog_busy = false;
    f.frog_waiting = true;
    tick(&mut f, &mut st);
    assert!(st.buys.iter().all(|b| b.finished()), "a Bitcoin conversion waits for the leg: the Ethereum side yields");
    f.frog_waiting = false;
    tick(&mut f, &mut st);
    assert_eq!(phase(&st), BPhase::Advancing, "the leg is free: the buy starts");
}
