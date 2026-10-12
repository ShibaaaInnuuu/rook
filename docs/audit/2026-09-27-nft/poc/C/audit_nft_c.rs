//! Audit 2026-09-27-nft, reviewer C (canister flow, holder deposits, economics): proofs of concept against e90471c.
//!
//! The fake world is `tests/ethnft.rs`'s, copied (not modified) and extended with: a second collection (Azuki, its own wrapper
//! and module) on the same Ethereum address and line, as launch decision D1 has it; an Ethereum receipt read that can fail N
//! times for one hash (a transient provider error); a transfer plan that can fail once with the exact text `ethnet::fees` gives
//! any failed `eth_call` (a transient outcall error); outflow (H3) waits by key prefix; a log of broadcasts.
//!
//! Every test here asserts the WEAKNESS: it passes while the attack works, and turns red once fixed.

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custodycap::{self, Book, CapPolicy};
use etch_autobuyer::ethflow::{self as ef, BPhase, DPhase, EthIo, EthPin, EthPlan, EthState, OurReceipt, XPhase};
use etch_autobuyer::ethnft::{self as en, EthCollection, EthNftConfig, ReceiptV, Seen, Stage};
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
const DEPOSIT: u128 = 500_000_000_000_000_000;
const ETH_PIN: u64 = 26_068_928;
const HOLDER: [u8; 20] = [0x33; 20];
const RECIPIENT: [u8; 20] = [0x44; 20];
/// What `ethnet::EthNet::fees` returns for ANY failed agreed `eth_call` (an outcall that timed out, a 429, two providers that
/// disagree), not only for a revert: `.map_err(|e| format!("refused (simulation): it reverts at the pin: {e}"))`.
const TRANSIENT: &str = "refused (simulation): it reverts at the pin: chain 1: eth_call: IC0406: SysTransient: Timeout expired";

struct Ticket {
    stage: Stage,
    item: u128,
    dest: [u8; 20],
    deliver_by: u64,
}

struct Fake {
    now: u64,
    paused: bool,
    deposits_on: bool,
    two: bool,
    home_balance: u128,
    home_nonce: u64,
    item_cap: u128,
    net_units: u128,
    /// Azuki's module's net demand
    net_units2: u128,
    transit: bool,
    accepted: u128,
    posted_at: u64,
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
    eth_balance: u128,
    eth_nonce: u64,
    finalized: u64,
    owners: HashMap<([u8; 20], Word), [u8; 20]>,
    eth_signed: Vec<(Tx1559, [u8; 32])>,
    receipts: HashMap<[u8; 32], OurReceipt>,
    /// the receipt read of this hash fails this many more times
    receipt_err: HashMap<[u8; 32], u32>,
    /// the next transfer plan fails with this text (once)
    plan_err: Option<String>,
    /// outflow keys starting with one of these wait (H3's limit reached for that key's collection)
    wait_prefix: Vec<String>,
    broadcasts: Vec<Vec<u8>>,
    /// `postFloor` calls that landed, by module
    posts: Vec<[u8; 20]>,
    listing_total: u128,
    sale_receipts: HashMap<[u8; 32], ReceiptV>,
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

impl Fake {
    fn new() -> Self {
        let mut sale_receipts = HashMap::new();
        let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
        for (k, r) in v["receipts"].as_object().unwrap() {
            let h: [u8; 32] = hex::decode(&k[2..]).unwrap().try_into().unwrap();
            sale_receipts.insert(h, en::receipt_of(r).unwrap().unwrap());
        }
        Fake {
            now: NOW, paused: false, deposits_on: false, two: false,
            home_balance: 2_000_000_000_000_000, home_nonce: 0, item_cap: DEPOSIT + rc::GAS_KEEP_WEI, net_units: 2 * E, net_units2: 0, transit: false,
            accepted: 93 * E / 100, posted_at: NOW - 60, keeper: US, allocator: rc::RELAY_ALLOCATOR_4663, home_sent: HashMap::new(),
            home_calls: vec![], tickets: vec![], items: vec![], tickets2: vec![], items2: vec![], marked: vec![], marked2: vec![],
            eth_balance: 10_000_000_000_000_000, eth_nonce: 0, finalized: ETH_PIN - 100, owners: HashMap::new(), eth_signed: vec![],
            receipts: HashMap::new(), receipt_err: HashMap::new(), plan_err: None, wait_prefix: vec![], broadcasts: vec![], posts: vec![],
            listing_total: 45 * E / 100, sale_receipts, deposit_txs: HashMap::new(),
            // stage 0 as the default install builds it for the launch collections (D1)
            caps: custodycap::stage0(&[custodycap::evm_key(1, &MILADY), custodycap::evm_key(1, &AZUKI)]),
            book: Book::default(), outflow: vec![],
        }
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
        match (to, sel(d)) {
            (MODULE, s) if s == rc::ADVANCE => {
                self.home_balance += arg(d, 0);
                self.transit = true;
            }
            (MODULE2, s) if s == rc::ADVANCE => self.home_balance += arg(d, 0),
            (MODULE, s) if s == rc::RETURN_ADVANCE => {
                self.home_balance -= value;
                self.transit = false;
            }
            (MODULE, s) if s == rc::SETTLE_ADVANCE => {
                self.transit = false;
                self.net_units -= E;
            }
            (MODULE, s) | (MODULE2, s) if s == en::POST_FLOOR => {
                self.posts.push(to);
                self.accepted = arg(d, 0);
                self.posted_at = self.now;
            }
            (WRAPPER, s) if s == en::MINT || s == en::MINT_DEPOSIT => self.items.push(d[4..36].try_into().unwrap()),
            (WRAPPER2, s) if s == en::MINT || s == en::MINT_DEPOSIT => self.items2.push(d[4..36].try_into().unwrap()),
            (WRAPPER, s) if s == en::RECORD_DELIVERY => {
                let t = &mut self.tickets[arg(d, 0) as usize];
                if t.stage != Stage::Resolved {
                    return false; // WrongStage
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

    fn module_read(&self, s: [u8; 4], net_units: u128) -> Vec<u8> {
        if s == etch_autobuyer::floorsink::NET_UNITS { word(net_units) }
        else if s == etch_autobuyer::floorsink::ESCROW { word(10 * E) }
        else if s == etch_autobuyer::floorsink::BUFFER { word(E / 20) }
        else if s == etch_autobuyer::floorsink::FLOOR { [word(self.accepted), word(self.posted_at as u128), word(3_600)].concat() }
        else if s == etch_autobuyer::floorsink::MAX_ITEM_COST { word(if self.transit { 0 } else { self.item_cap }) }
        else if s == etch_autobuyer::floorsink::ITEMS_ARRIVED { word(0) }
        else if s == rc::TRANSIT_UNITS { word(if self.transit { E } else { 0 }) }
        else if s == en::ACCEPTED { word(self.accepted) }
        else if s == en::KEEPER { addr_word(&self.keeper) }
        else { word(0) }
    }

    fn wrapper_read(tickets: &[Ticket], items: &[Word], d: &[u8]) -> Vec<u8> {
        let s = sel(d);
        if s == en::TICKET_COUNT { word(tickets.len() as u128) }
        else if s == en::TICKET {
            let t = &tickets[arg(d, 0) as usize];
            let st = match t.stage { Stage::None => 0, Stage::Pinned => 1, Stage::Resolved => 2, Stage::Delivered => 3, Stage::Refunded => 4 };
            [word(st), addr_word(&ALICE), word(0), word(0), word(0), word(t.deliver_by as u128), word(0), word(t.item), word(0),
                addr_word(&t.dest)].concat()
        } else if s == en::ITEM {
            [items[arg(d, 0) as usize].to_vec(), word(1), word(0)].concat()
        } else if s == en::ITEM_OF {
            let t: Word = d[4..36].try_into().unwrap();
            word(items.iter().position(|x| *x == t).map_or(0, |i| i as u128 + 1))
        } else { word(0) }
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
        Ok(match to {
            MODULE => self.module_read(sel(d), self.net_units),
            MODULE2 => self.module_read(sel(d), self.net_units2),
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
    async fn eth_plan_buy(&mut self, _order: &str, collection: [u8; 20], cap: u128) -> Result<EthPlan, String> {
        assert_eq!(collection, MILADY);
        if self.listing_total > cap {
            return Err(format!("refused (over cap): {} > {cap}", self.listing_total));
        }
        Ok(EthPlan { to: sp::SEAPORT_1_6, value: self.listing_total, data: vec![0xfb, 0x0f, 0x3e, 0xe1], gas: 200_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce: self.eth_nonce, token: tok(9_180), total: self.listing_total })
    }
    async fn eth_plan_transfer(&mut self, collection: [u8; 20], token: Word, dest: [u8; 20]) -> Result<EthPlan, String> {
        if let Some(e) = self.plan_err.take() {
            return Err(e);
        }
        Ok(EthPlan { to: collection, value: 0, data: en::transfer_from_call(&US, &dest, token), gas: 60_000, max_fee: 1_000_000_000,
            priority_fee: 100_000_000, nonce: self.eth_nonce, token, total: 0 })
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
    async fn eth_sale_receipt(&mut self, h: [u8; 32]) -> Result<Option<ReceiptV>, String> { Ok(self.sale_receipts.get(&h).cloned()) }
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

fn sales() -> Vec<[u8; 32]> {
    let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
    v["receipts"].as_object().unwrap().keys().map(|k| hex::decode(&k[2..]).unwrap().try_into().unwrap()).collect()
}
fn fill_hex() -> String {
    let v: serde_json::Value = serde_json::from_str(FILL).unwrap();
    v["input"].as_str().unwrap().to_string()
}
fn phase(st: &EthState) -> BPhase {
    st.buys.last().unwrap().phase.clone()
}

/// A resolved ticket on `wrapper` for `token`, which we own on Ethereum.
fn resolved(f: &mut Fake, second: bool, token: u128, dest: [u8; 20], deliver_by: u64) {
    let (c, items, tickets) = if second { (AZUKI, &mut f.items2, &mut f.tickets2) } else { (MILADY, &mut f.items, &mut f.tickets) };
    items.push(tok(token));
    tickets.push(Ticket { stage: Stage::Resolved, item: (items.len() - 1) as u128, dest, deliver_by });
    f.owners.insert((c, tok(token)), US);
}
fn held_id(c: [u8; 20], token: u128) -> String {
    format!("{}:{}", custodycap::evm_key(1, &c), hex::encode(tok(token)))
}
fn hx(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

// ---------------------------------------------------------------------------------------------------------------------------------
// C-M-1: a transient error in the transfer's simulation is taken as "can never succeed": a good delivery is marked undeliverable,
// and a stray is held for good.
// ---------------------------------------------------------------------------------------------------------------------------------

#[test]
fn audit_nft_c_a_transient_read_error_at_the_simulation_marks_a_good_delivery_undeliverable_and_holds_a_stray_for_good() {
    // (a) a delivery
    let mut f = Fake::new();
    f.net_units = 0;
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, NOW + 7 * 86_400);
    f.plan_err = Some(TRANSIENT.into()); // one outcall times out, once
    tick(&mut f, &mut st);
    assert!(matches!(&st.deliveries[0].phase, DPhase::Mark { why } | DPhase::Marking { why } if why.contains("SysTransient")),
        "{:?}", st.deliveries[0].phase);
    assert_eq!(f.marked, vec![0], "markUndeliverable was sent in the same tick, six days before the deadline");
    tick(&mut f, &mut st);
    for _ in 0..5 {
        f.now += 3_600;
        tick(&mut f, &mut st); // the provider is healthy again: nothing is ever retried
    }
    assert!(matches!(st.deliveries[0].phase, DPhase::Marked { .. }));
    assert!(f.eth_signed.is_empty(), "the NFT was never sent");
    assert_eq!(f.owners[&(MILADY, tok(77))], US);
    println!("AUDIT-C C-M-1 (a): one timed-out eth_call → delivery #0 {:?}; marked {:?}; 0 transfers signed", st.deliveries[0].phase, f.marked);

    // (b) a stray (a plain transferFrom to us): held, never returned, and no endpoint returns it
    let mut f = Fake::new();
    f.net_units = 0;
    f.deposits_on = true;
    let mut st = EthState::default();
    let tx = [0x51; 32];
    let r = ReceiptV { block: ETH_PIN - 2, success: true, logs: vec![en::LogV { address: MILADY,
        topics: vec![en::TRANSFER, sp::word_addr(&HOLDER), sp::word_addr(&US), tok(601)], data: vec![] }] };
    f.deposit_txs.insert(tx, (HOLDER, MILADY, en::transfer_from_call(&HOLDER, &US, tok(601)), r.clone()));
    f.sale_receipts.insert(tx, r);
    f.owners.insert((MILADY, tok(601)), US);
    f.finalized = ETH_PIN;
    ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    f.plan_err = Some(TRANSIENT.into()); // the return's simulation times out once
    tick(&mut f, &mut st); // judged a stray (Return), final, and its return planned in the same tick
    assert!(matches!(&st.deposits[0].phase, XPhase::Held { why } if why.contains("SysTransient")), "{:?}", st.deposits[0].phase);
    for _ in 0..5 {
        f.now += 3_600;
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.deposits[0].phase, XPhase::Held { .. }));
    assert!(f.eth_signed.is_empty());
    println!("AUDIT-C C-M-1 (b): one timed-out eth_call → the stray is {:?} for good; 0 returns signed", st.deposits[0].phase);
}

// ---------------------------------------------------------------------------------------------------------------------------------
// C-M-2: an under-priced fill on the one Ethereum line holds it with no bump, cancel or timeout; every other collection's waiting
// delivery is neither sent nor marked, so its holder cannot refund before the 30-day late window.
// ---------------------------------------------------------------------------------------------------------------------------------

#[test]
fn audit_nft_c_an_unmined_fill_holds_the_one_ethereum_line_and_another_collections_delivery_is_never_sent_or_marked() {
    let mut f = Fake::new();
    f.two = true;
    let mut st = EthState::default();
    // a Milady buy up to its fill
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    ef::put_quote(&mut st, 0, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    f.eth_balance += 498_443_962_449_863_321;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Arrived { .. }), "{:?}", phase(&st));
    let cfg = f.cfg();
    ef::put_evidence(&mut st, &cfg, MILADY, vec![format!("1:{}", 92 * E / 100), format!("2:{}", 93 * E / 100), format!("3:{}", 95 * E / 100)],
        sales(), f.now).unwrap();
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }), "{:?}", phase(&st));
    // the base fee rises past the fill's max fee (4 × the pinned base, measured median 0.068 gwei, 1.5 gwei seen in one day):
    // the fill stays in mempools, unmined. An Azuki holder redeems.
    let deliver_by = f.now + 7 * 86_400;
    resolved(&mut f, true, 4_242, BOB, deliver_by);
    f.net_units = 0;
    let start = f.now;
    while f.now < deliver_by + 72 * 3_600 + 3_600 {
        f.now += 3_600;
        tick(&mut f, &mut st);
    }
    let d = st.deliveries.iter().find(|d| d.wrapper == WRAPPER2).unwrap();
    assert_eq!(d.phase, DPhase::Waiting, "never signed, never marked");
    assert!(f.marked2.is_empty(), "past deliverBy + 72 h: no markUndeliverable, so the refund waits for deliverBy + 30 days");
    assert!(matches!(st.line.as_ref().unwrap().kind, ef::EthKind::Buy { .. }), "the fill still holds the line");
    assert_eq!(f.eth_signed.len(), 1, "nothing but the fill was ever signed: no bump, no cancel");
    println!("AUDIT-C C-M-2: {} h with the Milady fill unmined; Azuki ticket (deliverBy {deliver_by}) is {:?}, marked {:?}; \
        signed on Ethereum: {}", (f.now - start) / 3_600, d.phase, f.marked2, f.eth_signed.len());
}

// ---------------------------------------------------------------------------------------------------------------------------------
// C-M-3: `superseded` clears the line before its own reads: one failed receipt read while a transfer beats its cancel orphans the
// delivery (Cancelling for ever). It is never recorded; past deliverBy + 30 days the redeemer refunds and keeps the NFT; the item
// stays in the wrapper as a phantom and its custody slot is never released. Meanwhile the line signs other transactions.
// ---------------------------------------------------------------------------------------------------------------------------------

#[test]
fn audit_nft_c_a_read_error_as_a_transfer_beats_its_cancel_orphans_the_delivery_and_the_redeemer_keeps_nft_and_refund() {
    let mut f = Fake::new();
    f.net_units = 0;
    let mut st = EthState::default();
    let deliver_by = NOW + 3 * 86_400;
    resolved(&mut f, false, 77, ALICE, deliver_by);
    // the token backs a unit: held in the custody book (as a buy's settle or a deposit's admit leaves it)
    let key = custodycap::evm_key(1, &MILADY);
    custodycap::admit(&f.caps, &mut f.book, &held_id(MILADY, 77), &key, 0).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Sent { .. }));
    let deliver_hash = f.eth_signed[0].1;
    f.now = NOW + 2 * 86_400 + 1; // still pending a day before the deadline: the cancel goes out
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }));
    // the transfer, not the cancel, mines; the next read of its receipt fails once (a 429)
    f.mine_eth(0, true, ETH_PIN - 3);
    f.receipt_err.insert(deliver_hash, 1);
    let l = tick(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("429")), "{l:?}");
    assert!(st.line.is_none(), "the line was cleared before the read that failed");
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }), "{:?}", st.deliveries[0].phase);
    // "look before anything else is signed": a second ticket is delivered at once all the same
    resolved(&mut f, false, 78, BOB, NOW + 9 * 86_400);
    f.finalized = ETH_PIN;
    for _ in 0..24 {
        f.now += 3_600;
        tick(&mut f, &mut st);
    }
    assert_eq!(f.eth_signed.len(), 3, "the transfer, its cancel, and a new delivery signed after the orphan");
    // the orphan is never looked at again: not recorded, not marked
    f.now = deliver_by + 30 * 86_400 + 1;
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    assert!(matches!(st.deliveries[0].phase, DPhase::Cancelling { .. }), "{:?}", st.deliveries[0].phase);
    assert_eq!(f.tickets[0].stage, Stage::Resolved, "never recorded");
    assert!(!f.marked.contains(&0));
    // anyone refunds the late window: the redeemer has the NFT AND their unit back
    f.tickets[0].stage = Stage::Refunded;
    assert_eq!(f.owners[&(MILADY, tok(77))], ALICE);
    assert!(f.book.contains(&held_id(MILADY, 77)), "the custody book still counts a token that left: one of 3 stage-0 slots, for good");
    assert!(st.last_out(&MILADY, &tok(77)).is_none(), "and `outbound` never learned it left");
    println!("AUDIT-C C-M-3: delivery #0 {:?} for ever; ticket refunded at deliverBy+30d; owner of #77 = {}; book still holds it: {}",
        st.deliveries[0].phase, hx(&f.owners[&(MILADY, tok(77))]), f.book.contains(&held_id(MILADY, 77)));
}

// ---------------------------------------------------------------------------------------------------------------------------------
// C-L-1: nothing of the Ethereum side runs while paused — no record, no cancel. A pause that outlasts deliverBy + 30 days lets the
// redeemer refund a delivery that already mined (or that the canister itself re-broadcasts on resume).
// ---------------------------------------------------------------------------------------------------------------------------------

#[test]
fn audit_nft_c_a_pause_past_the_late_window_ends_with_nft_and_refund_and_the_resume_rebroadcasts_the_refunded_delivery() {
    // (a) the delivery mined just before the pause; nothing records it while paused
    let mut f = Fake::new();
    f.net_units = 0;
    let mut st = EthState::default();
    let deliver_by = NOW + 7 * 86_400;
    resolved(&mut f, false, 77, ALICE, deliver_by);
    let key = custodycap::evm_key(1, &MILADY);
    custodycap::admit(&f.caps, &mut f.book, &held_id(MILADY, 77), &key, 0).unwrap();
    tick(&mut f, &mut st);
    f.mine_eth(0, true, ETH_PIN - 5);
    tick(&mut f, &mut st);
    assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }));
    // PAUSED (guardian or H3 tripwire): canister.rs runs `ethnet::tick` only when not paused, so no tick of this side at all
    f.now = deliver_by + 30 * 86_400 + 1;
    f.tickets[0].stage = Stage::Refunded; // `refund(id)`: past deliverBy + 30 days, no mark needed
    // resumed
    f.finalized = ETH_PIN;
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    assert!(matches!(&st.deliveries[0].phase, DPhase::Gone { .. }), "{:?}", st.deliveries[0].phase);
    assert_eq!(f.owners[&(MILADY, tok(77))], ALICE);
    assert!(f.book.contains(&held_id(MILADY, 77)), "Gone after Moved never releases the slot");
    println!("AUDIT-C C-L-1 (a): NFT at ALICE, ticket refunded, delivery {:?}, slot still booked", st.deliveries[0].phase);

    // (b) the delivery was still pending when the pause began: on resume the canister's first act re-broadcasts it
    let mut f = Fake::new();
    f.net_units = 0;
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, deliver_by);
    tick(&mut f, &mut st);
    let raw = st.line.as_ref().unwrap().raw.clone();
    // PAUSED; the refund opens at deliverBy + 30 d and is taken
    f.now = deliver_by + 30 * 86_400 + 1;
    f.tickets[0].stage = Stage::Refunded;
    let before = f.broadcasts.len();
    tick(&mut f, &mut st); // resumed
    assert_eq!(f.broadcasts[before], raw, "the refunded ticket's delivery bytes are broadcast again before anything is decided");
    // the re-broadcast transfer lands before the cancel
    f.mine_eth(0, true, ETH_PIN - 1);
    f.finalized = ETH_PIN;
    for _ in 0..3 {
        tick(&mut f, &mut st);
    }
    assert_eq!(f.owners[&(MILADY, tok(77))], ALICE);
    assert!(matches!(&st.deliveries[0].phase, DPhase::Gone { .. }), "{:?}", st.deliveries[0].phase);
    println!("AUDIT-C C-L-1 (b): re-broadcast after the refund; NFT at ALICE; delivery {:?}", st.deliveries[0].phase);
}

// ---------------------------------------------------------------------------------------------------------------------------------
// C-L-2: one collection's delivery waiting on its H3 limit returns from `eth_act` with `?`: every other collection's deliveries,
// the buy and the stray returns wait behind it. Likewise one collection's deposit mint at the head of the list holds up every
// other collection's deposit mints.
// ---------------------------------------------------------------------------------------------------------------------------------

#[test]
fn audit_nft_c_one_collections_limit_blocks_every_other_collections_deliveries_and_deposit_mints() {
    let mut f = Fake::new();
    f.two = true;
    f.net_units = 0;
    let mut st = EthState::default();
    resolved(&mut f, false, 77, ALICE, NOW + 7 * 86_400); // Milady: its day's 3 deliveries are used up
    resolved(&mut f, true, 4_242, BOB, NOW + 7 * 86_400); // Azuki: nothing out today
    f.wait_prefix = vec![format!("deliver:{}", hx(&WRAPPER))];
    for _ in 0..12 {
        tick(&mut f, &mut st);
        f.now += 3_600;
    }
    let az = st.deliveries.iter().find(|d| d.wrapper == WRAPPER2).unwrap();
    assert_eq!(az.phase, DPhase::Waiting, "Azuki's delivery waits on Milady's limit");
    assert!(f.eth_signed.is_empty());
    println!("AUDIT-C C-L-2 (a): 12 h, Milady's delivery waits on its own limit; Azuki's is {:?}; 0 signed", az.phase);

    // deposits: a Milady deposit at the head of the mint list waits (its limit, its listing paused); the Azuki one never mints
    let mut f = Fake::new();
    f.two = true;
    f.net_units = 0;
    f.deposits_on = true;
    let mut st = EthState::default();
    for (tx, c, w, t) in [([0x91; 32], MILADY, WRAPPER, 950u128), ([0x92; 32], AZUKI, WRAPPER2, 951u128)] {
        let mut padded = en::deposit_data(4663, &w, &RECIPIENT);
        padded.resize(64, 0);
        let input = [&en::SAFE_TRANSFER_FROM_DATA[..], &addr_word(&HOLDER), &addr_word(&US), &word(t), &word(0x80), &word(48), &padded].concat();
        let r = ReceiptV { block: ETH_PIN - 1, success: true, logs: vec![en::LogV { address: c,
            topics: vec![en::TRANSFER, sp::word_addr(&HOLDER), sp::word_addr(&US), tok(t)], data: vec![] }] };
        f.deposit_txs.insert(tx, (HOLDER, c, input, r.clone()));
        f.sale_receipts.insert(tx, r);
        f.owners.insert((c, tok(t)), US);
        ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    }
    f.finalized = ETH_PIN;
    f.wait_prefix = vec![format!("mint:deposit:{}", hx(&[0x91; 32]))];
    for _ in 0..12 {
        tick(&mut f, &mut st);
        f.now += 3_600;
    }
    assert_eq!(st.deposits[1].phase, XPhase::Mint, "{:?}", st.deposits[1].phase);
    assert!(f.items2.is_empty(), "the Azuki deposit is never minted while the Milady one waits");
    println!("AUDIT-C C-L-2 (b): Milady deposit waits; Azuki deposit {:?}, minted {:?}", st.deposits[1].phase, f.items2);
}

// ---------------------------------------------------------------------------------------------------------------------------------
// C-L-3: a new buy is refused for EVERY collection when the first collection in the round robin whose module wants an item has
// no custody room: `buy_pass` returns on the cap check with `?` instead of trying the next collection, and `rr` does not move.
// Three deposited Milady (legitimate: the depositor gets its units) stop Azuki and MAYC from ever buying while Milady's module
// wants an item.
// ---------------------------------------------------------------------------------------------------------------------------------

#[test]
fn audit_nft_c_a_full_collection_whose_module_wants_an_item_stops_every_other_collection_from_buying() {
    let mut f = Fake::new();
    f.two = true;
    f.net_units = 2 * E; // Milady's module wants an item (a Milady graduation)
    f.net_units2 = 2 * E; // so does Azuki's
    let key = custodycap::evm_key(1, &MILADY);
    for t in 0..3u128 {
        custodycap::admit(&f.caps, &mut f.book, &held_id(MILADY, 10_000 + t), &key, 0).unwrap(); // three Milady deposits, minted
    }
    let mut st = EthState::default();
    let mut refused = 0;
    for _ in 0..24 {
        let l = tick(&mut f, &mut st);
        refused += l.iter().filter(|x| x.contains("custody cap")).count();
        f.now += 60;
        f.posted_at = f.now - 60; // the modules' floors stay fresh (the canister posts them)
    }
    assert!(st.buys.is_empty(), "Azuki has room and wants an item, but no buy starts: {:?}", st.buys.iter().map(|b| b.collection).collect::<Vec<_>>());
    assert!(f.home_calls.is_empty(), "no advance for anyone");
    assert!(refused >= 24);
    // Milady stops wanting: Azuki's buy starts at once
    f.net_units = 0;
    tick(&mut f, &mut st);
    assert_eq!(st.buys.len(), 1);
    assert_eq!(st.buys[0].collection, AZUKI);
    println!("AUDIT-C C-L-3: 24 ticks, {refused} refusals on Milady's cap, 0 Azuki buys; once Milady's module stops wanting, buy 0 is {}",
        hx(&st.buys[0].collection));
}

// ---------------------------------------------------------------------------------------------------------------------------------
// C-L-4 (M-2 residual): the thirteenth upgrade element is `opt EthState`. Candid decodes an `opt` whose content does not fit as
// `null` instead of failing, so an EthState of another shape comes back as nothing and `post_upgrade` restores an EMPTY
// Ethereum side (line, buys, deliveries, cursors, used orders) instead of trapping. b48b3ba → e90471c is such a change.
// ---------------------------------------------------------------------------------------------------------------------------------

#[derive(candid::CandidType, serde::Deserialize, Clone)]
struct OldEthState {
    // e90471c's EthState without `outbound` (as b48b3ba's was), everything else as now
    pub buys: Vec<ef::EthBuy>,
    pub next_buy: u64,
    pub deliveries: Vec<ef::Delivery>,
    pub cursors: Vec<([u8; 20], u128)>,
    pub deposits: Vec<ef::DepositRec>,
    pub readings: Vec<ef::Reading>,
    pub line: Option<ef::EthTx>,
    pub eth_out: u128,
    pub pending: Option<ef::HomeTx>,
    pub used_orders: Vec<[u8; 32]>,
    pub inbox: ef::Inbox,
    pub rr: u64,
}

#[test]
fn audit_nft_c_an_eth_state_of_another_shape_decodes_as_nothing_rather_than_refusing_the_upgrade() {
    let old = OldEthState { buys: vec![], next_buy: 7, deliveries: vec![], cursors: vec![(WRAPPER, 42)], deposits: vec![], readings: vec![],
        line: Some(ef::EthTx { kind: ef::EthKind::Deliver { wrapper: WRAPPER, ticket: 3 }, nonce: 9, hash: [1; 32], raw: vec![0xde, 0xad],
            value: 0, max_fee: 1, priority_fee: 1, signed_at: 1 }),
        eth_out: 5, pending: None, used_orders: vec![[7; 32]], inbox: ef::Inbox::default(), rr: 1 };
    // as `stable_save` writes the thirteenth element
    let bytes = candid::encode_args((Some(old.clone()),)).unwrap();
    let (got,): (Option<EthState>,) = candid::decode_args(&bytes).expect("decodes: no error for post_upgrade to trap on");
    assert!(got.is_none(), "the saved Ethereum side is silently gone");
    // the same bytes decoded without the `opt` do refuse, as M-2 intends
    let bytes2 = candid::encode_args((old,)).unwrap();
    assert!(candid::decode_args::<(EthState,)>(&bytes2).is_err());
    println!("AUDIT-C C-L-4: Some(saved state with a signed line at nonce 9, cursor 42, a used order) → decoded as {:?}", got.map(|_| "Some"));
}

// ---------------------------------------------------------------------------------------------------------------------------------
// C-H-1 (B-H-2's buy path, still open at e90471c, now fed by deposits): a purchase whose mint waits on H3's mint limit returns
// from `home_act` with `?` before the floor step, every tick, until the limit's window slides. Three deposits of the purchase's
// collection (legitimate: the depositor gets its units) use up the day's 3 mints while the purchase is bridging or waiting for a
// listing; then no module's floor is posted for up to 24 h, and every module goes stale (FLOOR_TTL 1 h): its mints and burns stop.
// ---------------------------------------------------------------------------------------------------------------------------------

#[test]
fn audit_nft_c_a_purchase_mint_waiting_on_the_limit_deposits_used_up_stops_every_collections_floor_post() {
    let mut f = Fake::new();
    f.two = true;
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    ef::put_quote(&mut st, 0, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    f.eth_balance += 498_443_962_449_863_321;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Arrived { .. }));
    // (while the buy waited for its ETH or a listing, three Milady deposits were minted: H3's 3 Milady mints of the day are used)
    let ev = |f: &Fake, st: &mut EthState, c: [u8; 20], p: u128| {
        let cfg = f.cfg();
        ef::put_evidence(st, &cfg, c, vec![format!("1:{}", p - E / 100), format!("2:{p}"), format!("3:{}", p + E / 100)], sales(), f.now).unwrap();
    };
    ev(&f, &mut st, MILADY, 93 * E / 100);
    ef::put_listing(&mut st, 0, fill_hex(), f.now).unwrap();
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Buying { .. }), "{:?}", phase(&st));
    f.mine_eth(0, true, ETH_PIN - 10);
    tick(&mut f, &mut st);
    f.finalized = ETH_PIN;
    f.wait_prefix = vec!["mint:eth:".into()]; // "waits (outflow limit): 3 of 3 Mint in the last 24 h"
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Final { .. }), "{:?}", phase(&st));
    // the modules' floors move; fresh evidence for both collections every 20 minutes, for two hours
    let posts_before = f.posts.len();
    for i in 0..6u128 {
        f.now += 1_200;
        f.accepted = 80 * E / 100; // a post is due for both (price moved > 1 %, and the post is older than 30 min)
        ev(&f, &mut st, MILADY, 93 * E / 100 + i);
        ev(&f, &mut st, AZUKI, 93 * E / 100 + i);
        tick(&mut f, &mut st);
    }
    assert_eq!(f.posts.len(), posts_before, "no postFloor for any collection while the purchase's mint waits");
    assert!(st.readings.iter().all(|r| r.due), "both readings are due and never sent");
    // control: the limit frees; the mint goes, then the posts
    f.wait_prefix.clear();
    for _ in 0..4 {
        ev(&f, &mut st, MILADY, 93 * E / 100);
        ev(&f, &mut st, AZUKI, 93 * E / 100);
        tick(&mut f, &mut st);
    }
    assert!(f.posts.len() > posts_before, "once the mint is out, posts resume");
    println!("AUDIT-C C-H-1: 2 h with the purchase's mint waiting on H3: 0 floor posts for Milady and Azuki (both due); after: {}",
        f.posts.len() - posts_before);
}
