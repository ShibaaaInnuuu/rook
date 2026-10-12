//! Fix round 4 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round4.md): the canister PoCs of the round-4 re-review
//! (docs/audit/2026-09-27-nft/review4/poc/B, C and E) turned around. The harness is `tests/audit_nft_r3.rs`'s fake world, copied
//! whole, with the reviewers' switches: `home_back` (the 4663 pin one bucket lower: the balance and nonce there are those before our
//! last home transactions, and a transaction signed at a used nonce is refused by the providers; parts C and E) with the home pin's
//! number (`home_pin`, the canister's `EthIo::home_pin`), `big_block` (a block whose `Transfer` logs are over the largest response
//! class at any chunk) and `pin_now` (an advancing pin; part B), and `payouts` (transactions the payout judge reads; C4-M-1). Each test
//! FAILS on the code before this round (the PoC passed there) and passes now.

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
/// The 4663 pin the fake world shows (one bucket lower under `home_back`).
const HOME_PIN: u64 = 70_166_016;
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
    /// review round 4 (C4-H-1): the 4663 pin one bucket back: (what the balance there has more, how many of our home txs it does not show)
    home_back: Option<(u128, u64)>,
    /// round 4: the world says its home pin (the canister's does); false: it does not (only the nonce floor guards the line)
    home_pin_known: bool,
    /// review round 4 (B4-H-1): blocks whose Transfer logs are over the largest response class at any chunk
    big_blocks: Vec<u64>,
    /// failed eth_getLogs reads (each is 3 outcalls in production: 64 kB, 400 kB, 2 MB)
    log_fails: u32,
    /// the response bytes the log reads reserved (as EthNet counts them: every class tried, on both providers)
    logs_bytes: u64,
    /// review round 4: the pin, advancing
    pin_now: Option<u64>,
    /// round 4 (B4-H-1): the marketplace sale logs of a block (`eth_sale_moves`); a block not here fails the read
    sale_moves: HashMap<u64, Vec<en::Move>>,
    /// round 4 (C4-M-1): transactions the payout judge reads, `(on 4663, hash)` → the transaction
    payouts: HashMap<(bool, [u8; 32]), ef::PayoutTx>,
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
            fee_of: None, home_back: None, home_pin_known: true, big_blocks: vec![], log_fails: 0, logs_bytes: 0, pin_now: None,
            sale_moves: HashMap::new(), payouts: HashMap::new(),
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
/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for LowCycles<'_> {}

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
    async fn home_balance(&mut self) -> Result<u128, String> { Ok(self.home_balance + self.home_back.map_or(0, |b| b.0)) }
    async fn home_sign(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> {
        // the nonce as the tick's pin shows it (canister: `evm_nonce` at the pin), as `home_nonce` below
        let n = self.home_nonce - self.home_back.map_or(0, |b| b.1);
        let raw = [&to[..], &value.to_be_bytes(), &n.to_be_bytes(), data].concat();
        Ok((raw.clone(), keccak(&raw), n))
    }
    async fn home_broadcast(&mut self, raw: &[u8]) {
        let h = keccak(raw);
        if self.home_sent.contains_key(&h) {
            return;
        }
        // review round 4: a transaction at a nonce already used on chain is refused by every provider ("nonce too low")
        if u64::from_be_bytes(raw[36..44].try_into().unwrap()) < self.home_nonce {
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
    async fn home_nonce(&mut self) -> Result<u64, String> { Ok(self.home_nonce - self.home_back.map_or(0, |b| b.1)) }
    async fn home_pin(&mut self) -> Result<Option<u64>, String> {
        Ok(self.home_pin_known.then(|| HOME_PIN - self.home_back.map_or(0, |_| 1_024)))
    }
    async fn eth_sale_moves(&mut self, _collection: [u8; 20], block: u64) -> Result<Vec<en::Move>, String> {
        self.logs_bytes += 2 * 64_000;
        self.sale_moves.get(&block).cloned().ok_or_else(|| "chain 1: eth_getLogs: IC0406: SysTransient: Timeout expired".to_string())
    }
    fn logs_bytes(&self) -> u64 { self.logs_bytes }
    async fn payout_tx(&mut self, home: bool, hash: [u8; 32]) -> Result<Option<ef::PayoutTx>, String> {
        Ok(self.payouts.get(&(home, hash)).cloned())
    }
    async fn home_finalized_time(&mut self) -> Result<u64, String> { Ok(self.now.saturating_sub(self.home_final_lag).saturating_add(1)) }

    async fn eth_pin(&mut self) -> Result<EthPin, String> {
        Ok(EthPin { number: self.pin_now.unwrap_or(self.pin_back.map_or(ETH_PIN, |p| p.0)), timestamp: self.now, base_fee: 100_000_000 })
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
        if self.big_blocks.iter().any(|b| from <= *b && *b <= to) {
            self.log_fails += 1;
            self.logs_bytes += 2 * (64_000 + 400_000 + 2_000_000);
            return Err(format!("chain 1: eth_getLogs: {}Http body exceeds size limit of 2000000 bytes", etch_autobuyer::evmread::SIZE_REJECT));
        }
        self.logs_bytes += 2 * 64_000;
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


/// A Relay quote 4663 → Ethereum for `amount` at `now` (the measured fixture, its deadlines moved to now + 7 days and re-hashed:
/// every rule of `relayeth::judge` holds on it), as `back_quote` does for the way back (review4/poc/C).
fn fwd_quote(amount: u128, now: u64) -> Vec<u8> {
    let us = hex::encode(US);
    let mut v: serde_json::Value = serde_json::from_str(QUOTE).unwrap();
    let a = amount.to_string();
    v["steps"][0]["items"][0]["data"]["value"] = a.clone().into();
    {
        let o = &mut v["protocol"]["v2"]["orderData"];
        o["inputs"][0]["payment"]["amount"] = a.clone().into();
        for r in o["inputs"][0]["refunds"].as_array_mut().unwrap() {
            r["deadline"] = (now + 7 * 86_400).into();
        }
        o["output"]["deadline"] = (now + 7 * 86_400).into();
        o["output"]["payments"][0]["minimumAmount"] = (amount / 1000 * 977).to_string().into();
    }
    v["protocol"]["v2"]["paymentDetails"]["amount"] = a.into();
    let id = rc::order_id(&v["protocol"]["v2"]["orderData"]).unwrap();
    v["steps"][0]["items"][0]["data"]["data"] = format!("0x49290c1c{}{us}{}", "0".repeat(24), hex::encode(id)).into();
    v["protocol"]["v2"]["orderId"] = format!("0x{}", hex::encode(id)).into();
    serde_json::to_vec(&v).unwrap()
}

fn relay_deposits(f: &Fake) -> usize {
    f.home_calls.iter().filter(|(to, ..)| *to == rc::RELAY_DEPOSITORY_4663).count()
}

// =====================================================================================================================================
// C4-H-1: the 4663 pin never goes back; nothing is signed at a used nonce; a home transaction at a used nonce is dropped
// =====================================================================================================================================

#[test]
fn r4_c4_h1_a_4663_pin_one_bucket_back_neither_reads_a_phantom_refund_nor_signs_at_a_used_nonce() {
    // PoC `c4_h1_a_4663_pin_one_bucket_back_reads_the_deposit_as_refunded_and_wedges_the_home_line_for_good` (review4/poc/C): one tick
    // at a 4663 pin one bucket back read our own Relay deposit as Relay's refund, the next quote was signed at the deposit's own
    // nonce, and three days later the one 4663 line of the whole canister still waited on it ("waiting rather than guessing" 216 times)
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    assert!(matches!(phase(&st), BPhase::Funded { .. }), "{:?}", phase(&st));
    ef::put_quote(&mut st, id, fwd_quote(DEPOSIT, f.now), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Bridging { .. }), "{:?}", phase(&st));
    assert_eq!(relay_deposits(&f), 1);
    let calls = f.home_calls.len();
    // one provider's head falls back a bucket: the 4663 pin is one bucket lower, under the block our deposit mined in
    f.home_back = Some((DEPOSIT, 1));
    for _ in 0..3 {
        f.now += 60;
        let l = tick(&mut f, &mut st).join("\n");
        assert!(!l.contains("REFUNDED"), "our own deposit is never read as Relay's refund:\n{l}");
        assert!(l.contains("C4-H-1"), "the tick waits, and says why:\n{l}");
    }
    assert!(matches!(phase(&st), BPhase::Bridging { .. }) && st.buys[0].last.is_some(), "still watched: {:?}", phase(&st));
    assert!(st.pending.is_none() && f.home_calls.len() == calls, "nothing signed at the lower pin");
    // the provider catches up; Relay fills the order as ordered: it arrives, earmarked to its buy
    f.home_back = None;
    f.eth_balance += ARRIVED;
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buys[0].phase, BPhase::Arrived { .. })), "{:?}", phase(&st));
    assert_eq!(st.earmarked_except(u64::MAX), ARRIVED);
    // and the home line goes on: the next home act is signed at the next nonce and mines
    f.m(MODULE).posted_at = f.now - 3_000;
    evidence(&f, &mut st);
    let posts = f.m(MODULE).posted.len();
    assert!(until(&mut f, &mut st, 4, |_| true));
    for _ in 0..4 {
        f.now += 600;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    assert!(f.m(MODULE).posted.len() > posts, "floors are posted again");
    assert!(st.pending.as_ref().is_none_or(|p| p.nonce >= f.home_nonce - 1));
}

#[test]
fn r4_c4_h1b_a_home_act_at_a_lower_pin_after_a_mined_one_is_never_signed_at_the_used_nonce() {
    // PoC `c4_h1b_any_home_act_signed_at_a_lower_pin_after_a_mined_one_wedges_the_line_a_floor_post`: a floor post mined, the next one
    // was signed at the used nonce, and no floor was posted again in 200 ticks. Both guards are shown: with the world's home pin (the
    // pin floor) and without it (the nonce floor alone)
    for known in [true, false] {
        let mut f = Fake::new(false);
        f.home_pin_known = known;
        let mut st = EthState::default();
        f.m(MODULE).posted_at = f.now - 3_000;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
        assert!(until(&mut f, &mut st, 3, |s| s.pending.is_none()) && !f.m(MODULE).posted.is_empty(), "the first post mined");
        let posts = f.m(MODULE).posted.len();
        f.home_back = Some((0, 1));
        f.now += 60;
        f.m(MODULE).posted_at = f.now - 3_000;
        let moved = vec![format!("1:{}", 94 * E / 100), format!("2:{}", 95 * E / 100), format!("3:{}", 97 * E / 100)];
        ef::put_evidence(&mut st, &f.cfg(), MILADY, moved, real_sales(), f.now).unwrap();
        let l = tick(&mut f, &mut st).join("\n");
        assert!(st.pending.is_none(), "known {known}: nothing is signed at the used nonce:\n{l}");
        assert!(l.contains("C4-H-1"), "{l}");
        f.home_back = None;
        for _ in 0..6 {
            f.now += 600;
            f.m(MODULE).posted_at = f.now - 3_000;
            evidence(&f, &mut st);
            tick(&mut f, &mut st);
        }
        assert!(f.m(MODULE).posted.len() > posts, "known {known}: floors are posted again ({posts} → {})", f.m(MODULE).posted.len());
        assert_eq!(st.home_nonce_floor(), f.home_nonce - st.pending.as_ref().map_or(0, |_| 1));
    }
}

#[test]
fn r4_c4_h1c_a_home_transaction_already_at_a_used_nonce_is_dropped_and_its_step_done_again() {
    // a state saved by an earlier build, wedged as the PoC left it: a post signed at a nonce our previous post used. The line no longer
    // waits for ever: once the transaction has been seen nowhere for HOME_NONCE_TAKEN_SECS with its nonce used, it is dropped, its
    // step taken back, and done again at the next nonce
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).posted_at = f.now - 3_000;
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    assert!(until(&mut f, &mut st, 3, |s| s.pending.is_none()));
    let posts = f.m(MODULE).posted.len();
    let used = f.home_nonce - 1;
    let raw = [&MODULE[..], &0u128.to_be_bytes(), &used.to_be_bytes(), &en::post_floor_call(E, 9_000, 3)].concat();
    st.pending = Some(ef::HomeTx { step: ef::Step::Post { collection: MILADY }, hash: keccak(&raw), raw, nonce: used, used_since: None, pin: None, sent_at: None });
    st.home_floor = None; // the earlier build kept no floor
    let mut dropped = false;
    for _ in 0..12 {
        f.now += 600;
        f.m(MODULE).posted_at = f.now - 3_000;
        evidence(&f, &mut st);
        dropped |= tick(&mut f, &mut st).iter().any(|l| l.contains("DROPPED"));
    }
    assert!(dropped, "the transaction at a used nonce is dropped");
    assert!(f.m(MODULE).posted.len() > posts, "and the floor is posted again ({posts} → {})", f.m(MODULE).posted.len());
    assert!(st.pending.as_ref().is_none_or(|p| p.nonce + 1 >= f.home_nonce));
}

#[test]
fn r4_e4_m1_a_home_pin_that_moves_back_no_longer_reads_our_own_relay_deposit_as_a_refund() {
    // PoC `e4_m1_a_home_pin_that_moves_back_reads_our_own_relay_deposit_as_a_refund` (review4/poc/E): the buy dropped its watch, the
    // real fill landed unwatched, the module booked the advance as a loss
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys.last().unwrap().id;
    st.used_orders.clear();
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    let before = f.home_balance;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Bridging { .. }));
    f.home_back = Some((before - f.home_balance, 1));
    tick(&mut f, &mut st);
    f.home_back = None;
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::Bridging { .. }) && st.buy(id).unwrap().last.is_some(), "no phantom refund");
    f.eth_balance += ARRIVED;
    tick(&mut f, &mut st);
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::Arrived { wei, .. } if wei == ARRIVED), "the real fill is its own arrival");
}

// =====================================================================================================================================
// E4-H-1: a waiting mint is one ask; honest deposits never trip the mint tripwire
// =====================================================================================================================================

/// The launch install's H3 (`nft-pilot.json` `_custody`: 12 / 24 items, 3 / 6 mints, 25 ETH, trip ×3) and J7's caps for the two
/// collections of the fake (Milady 16, Azuki 22 items; 15 ETH each; the global the sum).
fn launch_limits(f: &mut Fake) {
    let l = etch_autobuyer::outflow::Limits { per_collection_items: 12, global_items: 24, per_collection_mints: 3, global_mints: 6,
        global_value_wei: 25 * E, trip_factor: 3 };
    etch_autobuyer::outflow::validate(&l).unwrap();
    f.real_outflow = Some((Default::default(), l));
    f.caps = CapPolicy { collections: vec![
        custodycap::CollectionCap { collection: custodycap::evm_key(1, &MILADY), max_items: 16, max_value_wei: 15 * E, max_item_wei: 15 * E },
        custodycap::CollectionCap { collection: custodycap::evm_key(1, &AZUKI), max_items: 22, max_value_wei: 15 * E, max_item_wei: 15 * E }],
        global_max_items: 38, global_max_value_wei: 30 * E };
    custodycap::validate_change(&CapPolicy::default(), &f.caps).unwrap();
}

fn azuki_evidence(f: &Fake, st: &mut EthState, hashes: &[[u8; 32]]) {
    let az_ev = vec![format!("1:{}", 66 * E / 100), format!("2:{}", 67 * E / 100), format!("3:{}", 68 * E / 100)];
    ef::put_evidence(st, &f.cfg(), AZUKI, az_ev, hashes.to_vec(), f.now).unwrap();
}

/// The most distinct mint asks of one collection (or `*` for all) in the limiter's window, not acknowledged, now.
fn mint_demand(f: &Fake, col: Option<[u8; 20]>) -> usize {
    let (o, _) = f.real_outflow.as_ref().unwrap();
    let key = col.map(|c| custodycap::evm_key(1, &c));
    o.asked.iter().filter(|a| a.flow == Flow::Mint && key.as_ref().is_none_or(|k| a.collection == *k)).count()
}

#[test]
fn r4_e4_h1_nine_deposits_of_one_collection_in_a_day_never_trip_the_mint_tripwire() {
    // PoC `e4_h1_nine_deposits_of_one_collection_in_a_day_trip_the_mint_tripwire_and_pause_the_canister` (review4/poc/E): after four
    // ticks, 3 minted, 9 mint asks, `Tripwire { flow: Mint, demanded: 9, limit: 3 }`, the canister paused
    let mut f = Fake::new(true);
    launch_limits(&mut f);
    f.deposits_on = true;
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for k in 0..9u8 {
        let tx = [0x90 + k; 32];
        deposit_tx(&mut f, tx, MILADY, WRAPPER, 950 + k as u128, RECIPIENT, ETH_PIN - 20 - k as u64);
        ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
    }
    let mut most = 0;
    for _ in 0..(3 * 144) {
        f.now += 600;
        f.m(MODULE).posted_at = f.now - 60;
        f.m(MODULE2).posted_at = f.now - 60;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
        most = most.max(mint_demand(&f, Some(MILADY)));
        assert!(!f.paused, "never paused by honest deposits");
    }
    let (o, _) = f.real_outflow.as_ref().unwrap();
    println!("E4-H-1: 9 deposits over 3 days: {} minted; the most mint asks in a window {most}; frozen {:?}", f.items.len(), o.frozen());
    assert!(o.frozen().is_none());
    assert_eq!(f.items.len(), 9, "three a day: all nine minted within three days");
    assert!(most <= 4, "a collection's demand is what went out in the window plus the one waiting: {most}");
}

#[test]
fn r4_e4_h1_a_weeks_deposits_and_buys_at_j7s_caps_never_trip_any_h3_tripwire() {
    // the property over the Ethereum side's ask sites at the launch limits: Milady's 16 and Azuki's 22 items deposited at once (a
    // launch-week rush, D5 on) and redemptions delivered back out, for eight days: the tripwire never trips, and each collection's
    // mint demand in the window is at most what went out in it plus one
    let mut f = Fake::new(true);
    launch_limits(&mut f);
    f.deposits_on = true;
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    f.m(MODULE2).accepted = 67 * E / 100;
    let mut az_sales = Vec::new();
    for i in 0..5u8 {
        let h = [0xa0 + i; 32];
        f.sale_receipts.insert(h, sale_receipt(AZUKI, 100 + i as u128, 67 * E / 100, [0x10 + i; 20], [0x50 + i; 20], ETH_PIN - 500));
        az_sales.push(h);
    }
    let mut txs = Vec::new();
    for k in 0..16u8 {
        let tx = [0x10 + k; 32];
        deposit_tx(&mut f, tx, MILADY, WRAPPER, 2_000 + k as u128, RECIPIENT, ETH_PIN - 20);
        txs.push(tx);
    }
    for k in 0..22u8 {
        let tx = [0x40 + k; 32];
        deposit_tx(&mut f, tx, AZUKI, WRAPPER2, 3_000 + k as u128, RECIPIENT, ETH_PIN - 20);
        txs.push(tx);
    }
    let mut most = (0, 0, 0);
    for t in 0..(8 * 144) {
        f.now += 600;
        // claims trickle in as the runner hands them (16 wait at most)
        while st.inbox.deposits.len() < 16 {
            let Some(tx) = txs.pop() else { break };
            ef::put_deposit(&mut st, &f.cfg(), tx, f.now).unwrap();
        }
        fresh_modules(&mut f);
        evidence(&f, &mut st);
        azuki_evidence(&f, &mut st, &az_sales);
        if t % 72 == 0 {
            // a redemption now and then: a delivery out (its own H3 key)
            let n = f.items.len();
            if n > 0 {
                let token = u128::from_be_bytes(f.items[n - 1][16..].try_into().unwrap());
                f.tickets.push(Ticket { stage: Stage::Resolved, item: (n - 1) as u128, dest: ALICE, deliver_by: f.now + 7 * 86_400 });
                f.owners.insert((MILADY, tok(token)), US);
            }
        }
        tick(&mut f, &mut st);
        most = (most.0.max(mint_demand(&f, Some(MILADY))), most.1.max(mint_demand(&f, Some(AZUKI))), most.2.max(mint_demand(&f, None)));
        let (o, _) = f.real_outflow.as_ref().unwrap();
        assert!(o.frozen().is_none() && !f.paused, "tick {t}: honest demand tripped H3: {:?}", o.frozen());
    }
    println!("E4-H-1 (a week): minted {} Miladys and {} Azukis; the most mint asks in a window: Milady {}, Azuki {}, all {}", f.items.len(),
        f.items2.len(), most.0, most.1, most.2);
    assert!(most.0 <= 4 && most.1 <= 4, "{most:?}");
    assert!(most.2 < 18, "globally under the 6 × 3 that trip: {most:?}");
    assert!(f.items.len() + f.items2.len() >= 38 - 6, "the backlog drains at the limits: {} + {}", f.items.len(), f.items2.len());
}

/// Every H3 ask in the canister, read from its source: each is keyed by one logical outflow, so a wait is one ask however long it
/// lasts, and none is asked for an act a backlog repeats. A new ask site (or a changed key) fails here until it is reviewed and
/// listed with its reason.
#[test]
fn r4_every_h3_ask_is_keyed_per_logical_outflow() {
    // (file, the key expression as the source writes it, why it is one ask per logical outflow)
    let listed: &[(&str, &str, &str)] = &[
        ("ethflow.rs", "let key = format!(\"return:eth:{}:{}\", h(&d.tx), h(&d.token));", "a stray's return: one per claimed token; Return never trips"),
        ("ethflow.rs", "let key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);", "a delivery: one per ticket; bounded by custody (24 < 36; 61 + 6 < 72)"),
        ("ethflow.rs", "let key = mint_key(st, &col, &c.collection);", "a purchase's mint: the collection's next mint (E4-H-1)"),
        ("ethflow.rs", "let key = mint_key(st, &col, &d.collection);", "a deposit's mint: the collection's next mint, one candidate a pass (E4-H-1)"),
        ("ethflow.rs", "format!(\"advance:eth:{}:{n}\", key_of(&c, chain))", "an advance: the collection and its count (C3-H-1)"),
        ("convflow.rs", "format!(\"buy:frog:{}\", c.frog)", "a frog's conversion advance: one per frog, asked once the frog is wanted"),
        ("frogflow.rs", "let value_key = format!(\"buy:frog:{id}\");", "a frog buy's value: one per frog"),
        ("frogflow.rs", "let key = format!(\"return:{}\", d.outpoint.text());", "a refused Bitcoin deposit's return: Return never trips"),
        ("frogflow.rs", "io.outflow_check(&format!(\"claim:{}\", w.id), &key, Flow::Mint, 1)?;", "a floor claim: one per want, after the item is admitted to custody"),
        ("frogflow.rs", "let (key, collection) = (format!(\"mint:{}\", o.text()), custodycap::btc_key(&w));", "a Bitcoin deposit's mint: admitted to custody first (Ready), so bounded by its 3 items"),
        ("frogflow.rs", "let (key, collection) = (format!(\"mint:frog:{id}\"), crate::frogsol::collection_key(io));", "a bought frog's mint: one per frog, in custody (phase B2: the collection keyed sol:<wrapper> on the Solana home)"),
        ("handinflow.rs", "let key = format!(\"claim:handin:{}:{id}\", hex::encode(board));", "a hand-in's reward: one per hand-in"),
        ("nftnet.rs", "let fill = format!(\"fill:{}\", plan.order_hash);", "an operator's fill: one per order, asked at signing"),
        ("redeem.rs", "let key = io.ticket_key(r.ticket);", "a Bitcoin delivery: one per ticket (of its collection: `ticket:<coll>:<n>` when several Inscriptions listings), asked once its events are in"),
        ("saleflow.rs", "let key = format!(\"listing:{ticket}\");", "a sale's listing: one per ticket"),
        ("saleflow.rs", "let (key, collection) = (format!(\"mint:sale:{}\", s.id), custodycap::btc_key(&w));", "a sale's re-deposit mint: one per sale"),
        ("saleflow.rs", "let (key, collection) = (format!(\"sale:{}\", s.id), custodycap::btc_key(&io.wrapper()));", "a sale's proceeds: one per sale"),
    ];
    let files = [("ethflow.rs", include_str!("../src/ethflow.rs")), ("convflow.rs", include_str!("../src/convflow.rs")),
        ("frogflow.rs", include_str!("../src/frogflow.rs")), ("handinflow.rs", include_str!("../src/handinflow.rs")),
        ("nftnet.rs", include_str!("../src/nftnet.rs")), ("redeem.rs", include_str!("../src/redeem.rs")),
        ("saleflow.rs", include_str!("../src/saleflow.rs"))];
    let mut sites = 0;
    for (name, src) in files {
        let asks = src.matches("outflow_check(&").count() - src.matches("outflow_check(&mut self").count();
        sites += asks;
        let keys = listed.iter().filter(|(f, ..)| *f == name).count();
        assert_eq!(asks, keys, "{name}: {asks} H3 ask site(s), {keys} listed: review the new one and list it");
        for (_, key, why) in listed.iter().filter(|(f, ..)| *f == name) {
            assert!(src.contains(key), "{name}: the key `{key}` ({why}) is not in the source any more: review it");
        }
    }
    assert_eq!(sites, listed.len());
    // and no module outside these asks H3
    for (name, src) in [("canister.rs", include_str!("../src/canister.rs")), ("custodynet.rs", include_str!("../src/custodynet.rs")),
        ("frognet.rs", include_str!("../src/frognet.rs")), ("ethnet.rs", include_str!("../src/ethnet.rs"))] {
        assert_eq!(src.matches("outflow_check(&").count(), src.matches("outflow_check(&mut self").count(), "{name} asks H3 itself");
    }
}

// =====================================================================================================================================
// B4-H-1: an oversized log range costs a few failed reads once, never the scan
// =====================================================================================================================================

fn cached(st: &EthState, hs: &[[u8; 32]]) -> usize {
    st.sale_cache.iter().flatten().filter(|c| c.collection == MILADY && hs.contains(&c.hash) && !c.sales.is_empty()).count()
}
fn pass(f: &mut Fake, st: &mut EthState) {
    f.now += 1_200; // the runner hands evidence in every 1,200 s
    f.m(MODULE).posted_at = f.now - 60;
    evidence(f, st);
    tick(f, st);
}

#[test]
fn r4_b4_h1_a_one_oversized_block_no_longer_stalls_the_walk_back_or_burns_the_budget() {
    // PoC `b4_h1_a_one_oversized_block_stalls_the_walk_back_and_burns_the_log_budget_every_pass` (review4/poc/B): 3 of 5 real sales read
    // after a day, 51-52 failed reads (each three size classes) every pass
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    f.big_blocks = vec![26_060_000];
    let mut per_pass = vec![];
    let mut bytes = vec![];
    for _ in 0..72 {
        let (before, b0) = (f.log_fails, f.logs_bytes);
        pass(&mut f, &mut st);
        per_pass.push(f.log_fails - before);
        bytes.push(f.logs_bytes - b0);
    }
    let read = cached(&st, &real_sales());
    let sc = st.sale_scan.iter().flatten().find(|s| s.collection == MILADY).unwrap().clone();
    println!("B4-H-1 (a): {read} of 5 real sales read; failed reads per pass {:?} (total {}); bytes a pass at most {}; skipped {:?}",
        &per_pass[..8], f.log_fails, bytes.iter().max().unwrap(), sc.skipped);
    assert_eq!(read, 5, "the two older real sales are read past the oversized block");
    assert!(per_pass.iter().all(|n| *n <= 2 * en::SALE_SCAN_FAILS_PER_WALK), "a few failed reads a pass at most: {per_pass:?}");
    assert!(f.log_fails <= 20, "the oversized block costs its bisection once, not every pass: {}", f.log_fails);
    assert!(per_pass[24..].iter().all(|n| *n == 0), "once it is isolated nothing fails again: {per_pass:?}");
    assert!(bytes.iter().all(|b| *b <= en::SALE_SCAN_BYTES_PER_PASS + 2 * 2_464_000), "{bytes:?}");
    assert!(sc.skipped.as_ref().is_some_and(|s| s.iter().any(|(b, ..)| *b == 26_060_000)), "the block is recorded as skipped: {:?}", sc.skipped);
    assert!(sc.down <= ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS - en::SALE_WINDOW_BLOCKS, "the whole window is read");
}

#[test]
fn r4_b4_h1_b_an_oversized_new_block_no_longer_stops_every_newer_sale() {
    // PoC `b4_h1_b_an_oversized_new_block_stops_every_newer_sale_for_the_window`: 0 of 40 newer final sales read in a day, the whole
    // budget failing every pass
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        pass(&mut f, &mut st);
    }
    assert_eq!(cached(&st, &real_sales()), 5);
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    f.big_blocks = vec![top0 + 50];
    let mut new = vec![];
    for i in 0..40u64 {
        let mut h = [0x5e; 32];
        h[..8].copy_from_slice(&(i + 1).to_be_bytes());
        let s = { let mut a = [0x71u8; 20]; a[12..].copy_from_slice(&i.to_be_bytes()); a };
        let b = { let mut a = [0x72u8; 20]; a[12..].copy_from_slice(&i.to_be_bytes()); a };
        f.sale_receipts.insert(h, sale_receipt(MILADY, 30_000 + i as u128, 94 * E / 100, s, b, top0 + 100 + 100 * i));
        new.push(h);
    }
    let mut per_pass = vec![];
    let mut pin = ETH_PIN;
    for _ in 0..72 {
        pin += 100;
        f.pin_now = Some(pin);
        f.finalized = pin;
        let before = f.log_fails;
        pass(&mut f, &mut st);
        per_pass.push(f.log_fails - before);
    }
    let got = cached(&st, &new);
    println!("B4-H-1 (b): {got} of the 40 newer sales read in a day; failed reads per pass {:?} (total {})", &per_pass[..8], f.log_fails);
    assert!(got >= 38, "the newer sales are read past the oversized block: {got}");
    assert!(f.log_fails <= 20 && per_pass.iter().all(|n| *n <= 2 * en::SALE_SCAN_FAILS_PER_WALK), "{per_pass:?}");
}

#[test]
fn r4_b4_h1_c_a_sale_in_the_oversized_block_is_read_through_the_marketplaces_own_logs() {
    // the one block the flood blew up also holds a real sale: its Transfer logs cannot be read, its marketplace sale logs can
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    let blk = 26_060_000;
    f.big_blocks = vec![blk];
    let h = [0x6b; 32];
    f.sale_receipts.insert(h, sale_receipt(MILADY, 31_337, 94 * E / 100, [0x61; 20], [0x62; 20], blk));
    f.sale_moves.insert(blk, vec![en::Move { hash: h, block: blk, token: tok(31_337), from: [0x61; 20] }]);
    for _ in 0..30 {
        pass(&mut f, &mut st);
    }
    assert_eq!(cached(&st, &[h]), 1, "the sale in the oversized block is counted");
    assert_eq!(cached(&st, &real_sales()), 5);
    let sc = st.sale_scan.iter().flatten().find(|s| s.collection == MILADY).unwrap();
    assert!(sc.skipped.as_ref().is_none_or(|s| s.is_empty()), "nothing skipped: {:?}", sc.skipped);
}

#[test]
fn r4_b4_h1_d_under_the_evidence_cycles_floor_the_floor_is_still_posted() {
    // B4-H-1's effect at EVIDENCE_CYCLES_FLOOR: no evidence verified, no floor posted, every module stale. Now only the canister's own
    // sale scan stops; the listings are verified and the floor is posted from them and the sales already read
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        pass(&mut f, &mut st);
    }
    let posted = f.m(MODULE).posted.len();
    let reads = f.log_ranges.len();
    // a world whose cycles are under the floor
    for _ in 0..4 {
        f.now += 1_200;
        f.m(MODULE).posted_at = f.now - ef::URGENT_POST_SECS - 60;
        evidence(&f, &mut st);
        let lines = block_on(ef::tick(&mut LowCycles(&mut f), &mut st, false));
        assert!(lines.iter().any(|l| l.contains("did not run")), "{lines:?}");
    }
    assert!(f.m(MODULE).posted.len() > posted, "posted under the cycles floor");
    assert_eq!(f.log_ranges.len(), reads, "and the sale scan did not run");
}

/// The fake world with its cycles under `EVIDENCE_CYCLES_FLOOR`.
struct LowCycles<'a>(&'a mut Fake);
impl EthIo for LowCycles<'_> {
    fn now(&self) -> u64 { self.0.now() }
    fn home_chain(&self) -> u64 { self.0.home_chain() }
    fn eth_chain(&self) -> u64 { self.0.eth_chain() }
    fn config(&self) -> Option<EthNftConfig> { self.0.config() }
    fn paused(&self) -> bool { self.0.paused() }
    fn eth_max_fee(&self) -> u128 { self.0.eth_max_fee() }
    fn save(&mut self, st: &EthState) { self.0.save(st) }
    fn with_caps<R>(&mut self, f: impl FnOnce(&CapPolicy, &mut Book) -> R) -> R { self.0.with_caps(f) }
    fn outflow_check(&mut self, k: &str, c: &str, fl: Flow, a: u128) -> Result<(), String> { self.0.outflow_check(k, c, fl, a) }
    fn outflow_record(&mut self, k: &str, c: &str, fl: Flow, a: u128) { self.0.outflow_record(k, c, fl, a) }
    fn cycles_ok(&self) -> bool { false }
    async fn our_evm(&mut self) -> Result<[u8; 20], String> { self.0.our_evm().await }
    async fn home_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> { self.0.home_read(to, d).await }
    async fn home_clock(&mut self) -> Result<Clock, String> { self.0.home_clock().await }
    async fn home_balance(&mut self) -> Result<u128, String> { self.0.home_balance().await }
    async fn home_sign(&mut self, to: [u8; 20], v: u128, d: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> { self.0.home_sign(to, v, d).await }
    async fn home_broadcast(&mut self, raw: &[u8]) { self.0.home_broadcast(raw).await }
    async fn home_seen(&mut self, h: [u8; 32]) -> EvmView { self.0.home_seen(h).await }
    async fn home_nonce(&mut self) -> Result<u64, String> { self.0.home_nonce().await }
    async fn home_finalized_time(&mut self) -> Result<u64, String> { self.0.home_finalized_time().await }
    async fn eth_pin(&mut self) -> Result<EthPin, String> { self.0.eth_pin().await }
    async fn eth_finalized(&mut self) -> Result<u64, String> { self.0.eth_finalized().await }
    async fn eth_read(&mut self, to: [u8; 20], d: &[u8]) -> Result<Vec<u8>, String> { self.0.eth_read(to, d).await }
    async fn eth_balance(&mut self) -> Result<u128, String> { self.0.eth_balance().await }
    async fn eth_nonce(&mut self) -> Result<u64, String> { self.0.eth_nonce().await }
    async fn eth_receipt(&mut self, h: [u8; 32]) -> Result<Option<OurReceipt>, String> { self.0.eth_receipt(h).await }
    async fn eth_plan_buy(&mut self, o: &str, c: [u8; 20], cap: u128) -> Result<EthPlan, String> { self.0.eth_plan_buy(o, c, cap).await }
    async fn eth_plan_transfer(&mut self, c: [u8; 20], t: Word, d: [u8; 20]) -> Result<EthPlan, String> { self.0.eth_plan_transfer(c, t, d).await }
    async fn eth_sign(&mut self, tx: &Tx1559) -> Result<Vec<u8>, String> { self.0.eth_sign(tx).await }
    async fn eth_broadcast(&mut self, raw: &[u8]) { self.0.eth_broadcast(raw).await }
    async fn eth_listing(&mut self, s: &str, c: [u8; 20]) -> Result<Seen, String> { self.0.eth_listing(s, c).await }
    async fn eth_sale_receipt(&mut self, h: [u8; 32]) -> Result<Option<ReceiptV>, String> { self.0.eth_sale_receipt(h).await }
    async fn eth_deposit_tx(&mut self, h: [u8; 32]) -> Result<Option<([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>, String> { self.0.eth_deposit_tx(h).await }
    async fn eth_transfer_txs(&mut self, c: [u8; 20], a: u64, b: u64) -> Result<Vec<([u8; 32], u64)>, String> { self.0.eth_transfer_txs(c, a, b).await }
    async fn eth_transfer_moves(&mut self, c: [u8; 20], a: u64, b: u64) -> Result<Vec<en::Move>, String> { self.0.eth_transfer_moves(c, a, b).await }
    async fn eth_plan_call(&mut self, to: [u8; 20], v: u128, d: &[u8]) -> Result<EthPlan, String> { self.0.eth_plan_call(to, v, d).await }
    async fn home_pin(&mut self) -> Result<Option<u64>, String> { self.0.home_pin().await }
}

#[test]
fn r4_b4_i3_a_pin_that_moves_back_no_longer_resets_the_scan() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        pass(&mut f, &mut st);
    }
    let up = st.sale_scan.iter().flatten().find(|s| s.collection == MILADY).unwrap().up;
    let reads = f.log_ranges.len();
    f.pin_now = Some(ETH_PIN - 8);
    pass(&mut f, &mut st);
    let sc = st.sale_scan.iter().flatten().find(|s| s.collection == MILADY).unwrap();
    assert_eq!(sc.up, up, "kept: the newest blocks wait for the pin to pass them again");
    // no window re-read: at most the 8 blocks the window's start moved down by
    let bottom = ETH_PIN - 8 - en::SALE_FINAL_DEPTH_BLOCKS - en::SALE_WINDOW_BLOCKS;
    assert!(f.log_ranges[reads..].iter().all(|(a, b)| *a >= bottom && *b < bottom + 8), "{:?}", &f.log_ranges[reads..]);
}

#[test]
fn r4_b4_h1_the_marketplace_sale_logs_of_a_block_keep_only_the_collections_sales() {
    // `ethnft::sale_logs_body`, the transform of `eth_sale_moves`: a Seaport sale of Milady kept (its tx, block, token and offerer),
    // one of another collection dropped, a removed log dropped
    let js = |r: &ReceiptV, tx: &str, removed: bool| r.logs.iter().filter(|l| l.address == sp::SEAPORT_1_6).map(|l| serde_json::json!({
        "address": hx(&l.address), "topics": l.topics.iter().map(|t| hx(t)).collect::<Vec<_>>(), "data": hx(&l.data),
        "blockNumber": format!("0x{:x}", r.block), "transactionHash": tx, "removed": removed })).collect::<Vec<_>>();
    let a = sale_receipt(MILADY, 77, 94 * E / 100, [0x61; 20], [0x62; 20], 26_060_000);
    let b = sale_receipt(AZUKI, 78, 67 * E / 100, [0x63; 20], [0x64; 20], 26_060_000);
    let (ta, tb, tc) = (hx(&[0xaa; 32]), hx(&[0xbb; 32]), hx(&[0xcc; 32]));
    let logs: Vec<serde_json::Value> = [js(&a, &ta, false), js(&b, &tb, false), js(&a, &tc, true)].concat();
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": logs }).to_string();
    let out: serde_json::Value = serde_json::from_slice(&en::sale_logs_body(body.as_bytes(), &MILADY)).unwrap();
    let moves = en::transfer_moves_of(&out["result"]).unwrap();
    assert_eq!(moves, vec![en::Move { hash: [0xaa; 32], block: 26_060_000, token: tok(77), from: [0x61; 20] }]);
    assert!(en::sale_logs_body(b"<html>", &MILADY).starts_with(b"{\"error\""));
}

// =====================================================================================================================================
// C4-M-1: a late payout is its own order's: read by the order's still-open watch, or credited by its transaction after a write-off
// =====================================================================================================================================

/// Relay's solver in the measured quotes (`orderData.solver`, bound by the order id).
const SOLVER: [u8; 20] = [0xf7, 0x0d, 0xa9, 0x78, 0x12, 0xcb, 0x96, 0xac, 0xdf, 0x81, 0x07, 0x12, 0xaa, 0x56, 0x2d, 0xb8, 0xdf, 0xa3, 0xdb, 0xef];

/// Milady's way back, Relay neither filling nor refunding, `days` past its deadline: still watched (BackBridging). Returns (buy, what
/// the way back sent, its deadline).
fn way_back_past(f: &mut Fake, st: &mut EthState, days: u64) -> (u64, u128, u64) {
    let id = to_back_funded(f, st);
    let amount = to_back_bridging(f, st, id);
    let BPhase::BackBridging { deadline, .. } = st.buy(id).unwrap().phase else { unreachable!() };
    while f.now < deadline + days * 86_400 {
        f.now += 1_200;
        fresh_modules(f);
        tick(f, st);
    }
    (id, amount, deadline)
}

fn returned_to(f: &Fake, module: [u8; 20], sel_: [u8; 4]) -> u128 {
    f.home_calls.iter().filter(|(to, _, d)| *to == module && sel(d) == sel_).map(|(_, v, _)| *v).sum()
}

#[test]
fn r4_c4_m1_a_late_way_back_payout_is_read_by_its_own_still_open_watch() {
    // PoC `c4_m1_a_late_way_back_payout_is_nobodys_after_the_operator_closes_it_as_a_loss` (review4/poc/C): let go 25 h past its deadline,
    // the payout that came later was nobody's and the module booked its whole advance as a loss. The way back is now watched until its
    // payout is read or the operator writes it off
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let (id, amount, _) = way_back_past(&mut f, &mut st, 3);
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::BackBridging { .. }) && st.relay_busy(), "{:?}", st.buy(id).unwrap().phase);
    // Relay pays the way back late (after its refund process), to our 4663 address, as the order said
    let payout = amount / 100 * 98;
    f.home_balance += payout;
    assert!(until(&mut f, &mut st, 6, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { .. })), "{:?}", st.buy(id).unwrap().phase);
    let back = returned_to(&f, MODULE, rc::RETURN_ADVANCE);
    println!("C4-M-1: the late payout {payout} came back to the module: returnAdvance of {back}");
    assert_eq!(back, payout, "the module gets its ETH back, not a loss");
}

#[test]
fn r4_c4_m1b_a_late_payout_is_never_read_as_another_collections_refund() {
    // PoC `c4_m1b_a_late_way_back_payout_is_read_as_another_collections_refund_and_orphans_its_real_arrival`: Azuki's order read
    // Milady's late payout as its refund and bridged twice; one arrival on Ethereum was nobody's
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let mid = to_back_funded(&mut f, &mut st);
    let amount = to_back_bridging(&mut f, &mut st, mid);
    let BPhase::BackBridging { deadline, .. } = st.buy(mid).unwrap().phase else { unreachable!() };
    f.m(MODULE2).net_units = 2 * E;
    while f.now < deadline + 2 * 86_400 {
        f.now += 1_200;
        fresh_modules(&mut f);
        tick(&mut f, &mut st);
    }
    assert!(buy_of(&st, AZUKI).is_none(), "no other order starts while Milady's payout could still land");
    // Milady's payout lands late: it is Milady's
    f.home_balance += amount / 100 * 98;
    assert!(until(&mut f, &mut st, 6, |s| matches!(s.buy(mid).unwrap().phase, BPhase::Returned { .. })));
    assert_eq!(returned_to(&f, MODULE, rc::RETURN_ADVANCE), amount / 100 * 98);
    // then Azuki buys, and bridges once
    assert!(until(&mut f, &mut st, 8, |s| buy_of(s, AZUKI).is_some_and(|b| matches!(b.phase, BPhase::Funded { .. }))), "{:?}", buy_of(&st, AZUKI));
    let az = buy_of(&st, AZUKI).unwrap().id;
    let deposits0 = relay_deposits(&f);
    let ask = st.buy(az).unwrap().ask.clone().unwrap().amount_wei;
    ef::put_quote(&mut st, az, fwd_quote(ask, f.now), f.now).unwrap();
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Bridging { .. })));
    f.eth_balance += ARRIVED;
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Arrived { .. })));
    assert_eq!(relay_deposits(&f) - deposits0, 1, "Azuki bridged once, its own ETH");
    assert_eq!(st.earmarked_except(u64::MAX), ARRIVED.min(st.buy(az).unwrap().earmark()), "its arrival is its own");
}

#[test]
fn r4_c4_m1c_a_payout_after_the_write_off_is_credited_to_its_own_order_by_its_transaction() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let (id, amount, _) = way_back_past(&mut f, &mut st, 2);
    // Relay says the order is dead: the operator writes it off, then closes it as a loss
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("written off"));
    // review round 5 (C5-M-1): the written-off order holds the Relay leg for new orders until it is credited or released
    assert!(st.buy(id).unwrap().late.is_some() && st.relay_busy());
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("closed as a loss"));
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { wei: 0 })));
    // Relay pays it after all, to our 4663 address, from its solver
    let payout = amount / 100 * 98;
    f.home_balance += payout;
    // review round 5 (C5-L-1): a payout names its order (Relay's fill carries the id in its calldata); the solver alone no longer does
    let oid = st.buy(id).unwrap().late.as_ref().unwrap().watch.order_id;
    let pay = |from: [u8; 20], value: u128| ef::PayoutTx { from, to: US, value, input: oid.to_vec(), block: HOME_PIN + 10, success: true };
    // a stranger's transfer of the same amount (not naming the order) is not this order's payout
    f.payouts.insert((true, [0x51; 32]), ef::PayoutTx { from: [0x99; 20], to: US, value: payout, input: vec![], block: 70_000_000, success: true });
    ef::put_payout(&mut st, id, true, [0x51; 32], 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("refused, nothing credited") && l.contains("does not name order"), "{l}");
    // more than the order sent is not its payout either
    f.payouts.insert((true, [0x52; 32]), pay(SOLVER, amount + 1));
    ef::put_payout(&mut st, id, true, [0x52; 32], 0, f.now).unwrap();
    assert!(tick(&mut f, &mut st).join("\n").contains("more than the order sent"));
    // the solver's payment of it is credited: the module had booked the loss, so the ETH joins its surplus (fundReserve)
    f.payouts.insert((true, [0x53; 32]), pay(SOLVER, payout));
    ef::put_payout(&mut st, id, true, [0x53; 32], 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED"), "{l}");
    for _ in 0..4 {
        tick(&mut f, &mut st);
    }
    assert_eq!(returned_to(&f, MODULE, ef::FUND_RESERVE), payout, "the late payout reached its own module");
    assert!(ef::put_payout(&mut st, id, true, [0x53; 32], 0, f.now).is_err(), "never credited twice");
    assert_eq!(st.buy(id).unwrap().late.as_ref().unwrap().credited.map(|c| c.1), Some(payout));
    assert!(!st.relay_busy(), "credited: the Relay leg is free again");
}

#[test]
fn r4_c4_m1d_eth_give_back_on_a_watched_forward_stuck_pays_only_its_measured_refund() {
    // the forward leg's exit returned the whole 4663 balance less one keep (capped at the advance), whatever it was: another order's
    // late payout, the frog side's ETH, the gas pot
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    // 2 ETH at our 4663 address that are not this order's (say the frog side's, B-L-6), there before its deposit
    f.home_balance += 2 * E;
    st.used_orders.clear();
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let BPhase::Bridging { deadline, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    f.now = deadline + ef::DEADLINE_GRACE_SECS + 60;
    tick(&mut f, &mut st);
    assert!(matches!(phase(&st), BPhase::Stuck { .. }) && st.buys[0].last.is_some(), "watched: {:?}", phase(&st));
    // nothing of the 2 ETH is given back (before round 4: the balance less one keep, capped at the advance: 0.5002 ETH of them)
    let e = ef::give_back_now(&mut st.clone(), id, f.home_balance, f.now).unwrap_err();
    assert!(e.contains("shows no refund") && e.contains("watched until"), "{e}");
    let mut late = st.clone();
    let l = ef::give_back_now(&mut late, id, f.home_balance, deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 1).unwrap();
    assert!(l.contains("written off") && matches!(late.buys[0].phase, BPhase::Stuck { .. }) && late.buys[0].late.is_some(), "{l}");
    // with the order's own refund at our address: exactly the rise over the watch's start goes back
    let w = st.buys[0].last.clone().unwrap();
    let refunded = w.balance_after + DEPOSIT;
    assert!(ef::give_back_now(&mut st, id, refunded, f.now).unwrap().contains("its order's refund"));
    assert_eq!(st.buys[0].phase, BPhase::Returning { wei: DEPOSIT.min(st.buys[0].advanced) });
}

#[test]
fn r4_c4_m1_the_payout_judge() {
    let w = ef::Watch { order_id: [0x0d; 32], value: 5 * E, min_out: 49 * E / 10, deadline: 1, balance_after: 0, eth_before: 0,
        eth_out_before: 0, pin: None, solver: Some(SOLVER), request_id: None, home_pin: None };
    let o = ef::LateOrder { watch: w, back: true, let_go_at: 2, credited: None, released_at: None };
    let t = |from: [u8; 20], to: [u8; 20], value: u128, input: Vec<u8>, success: bool| ef::PayoutTx { from, to, value, input, block: 9, success };
    let id = [0x0d; 32];
    // review round 5 (C5-L-1): a plain transfer to us naming the order (Relay's fill carries the id in its calldata): its value; the
    // solver alone no longer names it (it is every order's)
    assert_eq!(ef::judge_payout(&o, &t(SOLVER, US, 49 * E / 10, id.to_vec(), true), &[], &[1; 32], true, &US, 0, &[], false), Ok(49 * E / 10));
    assert!(ef::judge_payout(&o, &t(SOLVER, US, 49 * E / 10, vec![], true), &[], &[1; 32], true, &US, 0, &[], false).unwrap_err().contains("does not name"));
    // a call naming the order id (a refund from the depository). Fix round 6 (C6-L-1): only with Relay's own execution record in its
    // receipt (the depository's RelayCallExecuted paying us), at that record's amount, never over the order's value
    // fix round 7 (C7-L-1): the depository's real `execute`, its records' id the request's struct hash (here its nonce is the order id,
    // so the input names the order)
    let req = rc::CallRequest { calls: vec![rc::DepCall { to: US, data: vec![], value: 3 * E, allow_failure: false }], nonce: id, expiration: [0; 32] };
    let call = rc::encode_execute(&req, &[0x11; 65]);
    let sh = rc::call_request_hash(&req);
    let exec = |to: [u8; 20], value: u128| {
        let w = |x: u128| sp::word_u128(x).to_vec();
        en::LogV { address: rc::RELAY_DEPOSITORY_4663, topics: vec![rc::RELAY_CALL_EXECUTED],
            data: [sh.to_vec(), w(0x40), [vec![0; 12], to.to_vec()].concat(), w(0x80), w(value), w(0), w(0)].concat() }
    };
    let dep = |logs: &[en::LogV], named: u128| ef::judge_payout(&o, &t([0x77; 20], rc::RELAY_DEPOSITORY_4663, 0, call.clone(), true), logs, &[2; 32],
        false, &US, named, &[], false);
    assert!(dep(&[], 3 * E).unwrap_err().contains("RelayCallExecuted"), "a call that only names the order is no payout");
    assert_eq!(dep(&[exec(US, 3 * E)], 3 * E), Ok(3 * E));
    assert_eq!(dep(&[exec(US, 3 * E)], 0), Ok(3 * E), "the record's amount, not the operator's");
    assert!(dep(&[exec(US, 3 * E)], 4 * E).unwrap_err().contains("stated"));
    assert!(dep(&[exec([0x55; 20], 3 * E)], 3 * E).is_err(), "paying someone else");
    assert!(dep(&[exec(US, 6 * E)], 0).is_err(), "over its value");
    // refusals
    assert!(ef::judge_payout(&o, &t([0x99; 20], US, E, vec![], true), &[], &[3; 32], true, &US, 0, &[], false).unwrap_err().contains("does not name"));
    let n = || id.to_vec();
    assert!(ef::judge_payout(&o, &t(SOLVER, US, E, n(), false), &[], &[3; 32], true, &US, 0, &[], false).unwrap_err().contains("reverted"));
    assert!(ef::judge_payout(&o, &t(SOLVER, US, E, n(), true), &[], &[3; 32], true, &US, 0, &[], false).unwrap_err().contains("under half"));
    assert!(ef::judge_payout(&o, &t(SOLVER, US, 49 * E / 10, n(), true), &[], &[3; 32], true, &US, 0, &[[3; 32]], false).unwrap_err().contains("already"));
    assert!(ef::judge_payout(&o, &t(SOLVER, [0x55; 20], 49 * E / 10, n(), true), &[], &[3; 32], true, &US, 0, &[], false).is_err(), "not to us");
    assert!(ef::judge_payout(&o, &t(SOLVER, US, 49 * E / 10, n(), true), &[], &[3; 32], true, &US, 4 * E, &[], false).unwrap_err().contains("not the"));
}

// =====================================================================================================================================
// Upgrade safety of this round's state: states saved by the old builds themselves restore, nothing lost, the new fields absent
// =====================================================================================================================================

fn paged(b: &[u8]) -> Vec<u8> {
    let mut v = b.to_vec();
    v.resize(b.len().div_ceil(65_536) * 65_536, 0);
    v
}

#[test]
fn r4_states_saved_by_the_old_builds_restore_with_nothing_lost_and_the_new_fields_absent() {
    use etch_autobuyer::canister::{lost_opts, Saved};
    // 4949da0 (= 55d8d32's canister) and a37f54b: review4/poc/E's, written by those builds; 1eb5788 (round 3, every one of its new
    // fields present, a pending noteSent): written by that build with review4/poc/E's dumper (docs/audit/2026-09-27-nft/fixes-round4-poc/)
    for (name, hex_) in [("e04f49b", include_str!("fixtures/state/saved-e04f49b.hex")),
        ("4949da0-rich", include_str!("fixtures/state/saved-4949da0-rich.hex")), ("a37f54b-rich", include_str!("fixtures/state/saved-a37f54b-rich.hex")),
        ("1eb5788-rich", include_str!("fixtures/state/saved-1eb5788-rich.hex"))] {
        let bytes = paged(&hex::decode(hex_.trim()).unwrap());
        let (saved,): (Saved,) = candid::decode_args(&bytes).or_else(|_| {
            let mut de = candid::de::IDLDeserialize::new(&bytes).map_err(|e| e.to_string())?;
            Ok::<_, String>((de.get_value::<Saved>().map_err(|e| e.to_string())?,))
        }).unwrap();
        let again = candid::encode_args((saved.clone(),)).unwrap();
        assert_eq!(lost_opts(&bytes, &again).unwrap(), Vec::<String>::new(), "{name}: an honest upgrade would be refused");
        let Some(e) = saved.12 else { continue };
        assert!(e.home_floor.is_none() && e.pin_floors.is_none() && e.credited.is_none(), "{name}");
        assert!(e.buys.iter().all(|b| b.late.is_none() && b.last.as_ref().is_none_or(|w| w.solver.is_none())), "{name}");
        assert!(e.pending.as_ref().is_none_or(|p| p.used_since.is_none()), "{name}");
        assert!(e.sale_scan.iter().flatten().all(|s| s.bad.is_none() && s.skipped.is_none()), "{name}");
        assert!(e.inbox.payouts.is_none(), "{name}");
        if name == "1eb5788-rich" {
            assert!(matches!(e.pending.as_ref().unwrap().step, ef::Step::Sent { .. }), "round 3's Step::Sent restores");
            assert!(e.deliveries.iter().all(|d| d.sent == Some(true)) && e.buys.iter().all(|b| b.arrived_at == Some(10)));
            let sc = &e.sale_scan.as_ref().unwrap()[0];
            assert!(sc.pend.as_ref().is_some_and(|p| p.len() == 1) && sc.rescan == Some((12, 14)) && sc.judged.is_some());
            assert!(e.buys.iter().all(|b| b.last.as_ref().is_some_and(|w| w.pin == Some(4))));
        }
    }
}

/// The state this round writes, with every one of its new fields present, decodes back as itself (the `.did` is checked against it by
/// the services' tests, `tests/fixtures/eth-state-round4.hex`: E4-I-4).
fn round4_state() -> EthState {
    let w = ef::Watch { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, balance_after: 1, eth_before: 2, eth_out_before: 3, pin: Some(4),
        solver: Some([0xf7; 20]), request_id: None, home_pin: None };
    let b = |id: u64, phase: BPhase| ef::EthBuy { id, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: 1, attempts: 1, phase,
        orders: vec![], deposits: vec![], ask: None, last: Some(w.clone()), note: String::new(), sweep: Some(false), arrived_at: Some(3),
        back_at: Some(4), ended_at: None, late: None };
    let mut st = EthState::default();
    let mut stuck = b(1, BPhase::Stuck { why: "written off".into() });
    stuck.last = None;
    stuck.late = Some(ef::LateOrder { watch: w.clone(), back: true, let_go_at: 5, credited: Some(([2; 32], 4, true)), released_at: None });
    st.buys = vec![b(0, BPhase::BackBridging { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, since: 2 }), stuck];
    st.deliveries = vec![ef::Delivery { wrapper: WRAPPER, collection: MILADY, ticket: 0, token: tok(77), destination: ALICE, deliver_by: 9,
        phase: ef::DPhase::Record { tx: [3; 32] }, note: String::new(), refused_since: None, sent: Some(true) }];
    st.pending = Some(ef::HomeTx { step: ef::Step::Sent { wrapper: WRAPPER, ticket: 0 }, raw: vec![1], hash: [4; 32], nonce: 7, used_since: Some(6), pin: None, sent_at: None });
    st.sale_scan = Some(vec![ef::SaleScan { collection: MILADY, down: 1, up: 9, chunk: 500, todo: vec![],
        pend: Some(vec![en::Move { hash: [5; 32], block: 8, token: tok(1), from: ALICE }]), rescan: Some((2, 3)), judged: Some(vec![(1, 2)]),
        bad: Some(vec![(4, 5)]), skipped: Some(vec![(6, 7, 1)]), done_big: None, stuck: None, spent: None, hole: None, lost: None, behind: None, rfail: None, raw: None, current: None, hole_deep: None, rbusy: None, busy: None }]);
    st.home_floor = Some((8, 70_166_016));
    st.pin_floors = Some(etch_autobuyer::pin::PinFloors { home: etch_autobuyer::pin::PinFloor { pin: 70_166_016, nonce: 8 },
        eth: etch_autobuyer::pin::PinFloor { pin: ETH_PIN, nonce: 3 } });
    st.credited = Some(vec![[2; 32]]);
    st.leftover = Some(vec![(MILADY, 7)]);
    st
}

#[test]
fn r4_the_round_4_state_fixture_is_what_the_canister_encodes() {
    let st = round4_state();
    let bytes = candid::encode_one(&st).unwrap();
    assert_eq!(candid::decode_one::<EthState>(&bytes).unwrap(), st);
    let fixture = include_str!("fixtures/eth-state-round4.hex").trim();
    assert_eq!(hex::encode(&bytes), fixture, "tests/fixtures/eth-state-round4.hex is stale: write the new bytes there (and run the monitor's and ops' tests)");
}

#[test]
fn r4_b4_l1_a_dropped_range_is_read_again_even_while_a_flood_keeps_the_queue_full() {
    // review round 4, B4-L-1: the re-read of blocks dropped from a full queue waited for the queue to be under half full, which a
    // cheap flood (one token ping-ponged) keeps from happening; one chunk a pass is now re-read whatever the queue
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        pass(&mut f, &mut st);
    }
    let top = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    // an honest sale an overflow dropped (its block is to be read again), and 400 flood moves of one token in the queue
    let blk = top - 3_000;
    let real = [0x7a; 32];
    f.sale_receipts.insert(real, sale_receipt(MILADY, 51_000, 94 * E / 100, [0x3a; 20], [0x3b; 20], blk));
    let flood: Vec<en::Move> = (0..400u32).map(|i| {
        let mut h = [0x9f; 32];
        h[..4].copy_from_slice(&i.to_be_bytes());
        en::Move { hash: h, block: top - 2_000 + i as u64, token: tok(4_242), from: if i % 2 == 0 { ATTACKER } else { OTHER } }
    }).collect();
    let sc = st.sale_scan.as_mut().unwrap().iter_mut().find(|s| s.collection == MILADY).unwrap();
    sc.pend = Some(flood);
    sc.rescan = Some((blk, blk));
    for _ in 0..3 {
        pass(&mut f, &mut st);
    }
    assert_eq!(cached(&st, &[real]), 1, "the dropped sale is read again and counted");
}

#[test]
fn r4_e4_i2_no_deposit_mints_before_its_module_has_a_floor() {
    // review round 4, E4-I-2 / A4-I-6: a module that never posted (its keeper is not the canister, so no post lands) has an accepted
    // floor of 0, and a deposit minted then was admitted to custody at 0 wei
    let mut f = Fake::new(false);
    f.deposits_on = true;
    f.keeper = [0x66; 20];
    f.m(MODULE).accepted = 0;
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    deposit_tx(&mut f, [0x91; 32], MILADY, WRAPPER, 950, RECIPIENT, ETH_PIN - 20);
    ef::put_deposit(&mut st, &f.cfg(), [0x91; 32], f.now).unwrap();
    for _ in 0..6 {
        f.now += 600;
        evidence(&f, &mut st);
        tick(&mut f, &mut st);
    }
    assert!(f.items.is_empty(), "not minted against no floor");
    assert_eq!(st.deposits[0].phase, ef::XPhase::Mint, "it waits for the module's first post");
}
