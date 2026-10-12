//! Fix round 5 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round5.md): the canister PoCs of the round-5 re-review
//! (docs/audit/2026-09-27-nft/review5/poc/B, C, E and F) turned around. The harness is `tests/audit_nft_r4.rs`'s fake world, copied
//! whole, with the reviewers' switches (`hide`, `unsure`, `real_advance` of part F; `payout_balance_fail` of part C; `big_sale_blocks`,
//! `dense_blocks` of part B; `mint_revert`, `mint_log`, `item_of_err` and the estimate's `AlreadyHeld` of part E) and what this round
//! reads (`identify`: `EthIo::home_nonce_tx`; the module's `advanced()`, the wrapper's `sentTx` and `undeliverable`; the marketplaces'
//! logs over a range; the receipts' reserved bytes). Each test FAILS on the code before this round (its PoC passed there) and passes now.

#![allow(dead_code)] // the harness is tests/audit_nft_r4.rs's, copied whole

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
    /// review round 5: `advanced()` (the open advance's wei)
    advanced: u128,
    accepted: u128,
    posted_at: u64,
    /// `floorBounds()`
    bounds: (u128, u128),
    posted: Vec<(u128, u16, u32)>,
}

impl Module {
    fn new(accepted: u128) -> Self {
        Module { net_units: 0, item_cap: DEPOSIT + rc::GAS_KEEP_WEI, transit: false, advanced: 0, accepted, posted_at: NOW - 60, bounds: (0, u128::MAX), posted: vec![] }
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
    // ---- review round 5 (the reviewers' switches, review5/poc/B, C, E, F) ----------------------------------------------------------
    /// F / E: home transactions that MINED but that every 4663 provider fails to show (no receipt at the pin, no transaction by hash)
    hide: Vec<[u8; 32]>,
    /// F: the receipt read is Unsure (the providers disagree)
    unsure: bool,
    /// F: the module as FloorModule.sol is: maxItemCost stays above 0 while an advance is open; a second advance reverts AdvanceOpen and
    /// a returnAdvance with none open reverts NoAdvance at the estimate
    real_advance: bool,
    /// round 5 (F5-M-1): the world can tell which transaction of ours used a nonce (our nonce count at past blocks and the block's
    /// transactions: `EthIo::home_nonce_tx`); false: it cannot (a keyless provider keeps no state that deep)
    identify: bool,
    /// round 5 (F5-M-1): `noteSent`'s `sentTx(ticket)`, by wrapper
    sent_tx: HashMap<([u8; 20], u128), Word>,
    /// C: once the payout judge read a transaction, the Ethereum balance read after the next one fails once
    payout_balance_fail: bool,
    bal_fail: Option<u32>,
    /// B: blocks whose marketplace sale logs are over the largest class too
    big_sale_blocks: Vec<u64>,
    /// B: blocks of ~5,001 Transfer logs each: two or more in a range is over a provider's 10,000-results cap (a JSON-RPC error)
    dense_blocks: Vec<u64>,
    /// round 5 (B5-M-2): a range holding one of these blocks is refused by a provider with an error that names nothing (not a size)
    odd_blocks: Vec<u64>,
    /// round 5 (B5-L-1): the response bytes the sale receipts reserved (as EthNet counts them: both classes tried, both providers)
    receipt_bytes: u64,
    /// E: the next N wrapper mints revert; every wrapper mint that succeeded (wrapper, time); the next N itemOf reads fail
    mint_revert: u32,
    mint_log: Vec<([u8; 20], u64)>,
    item_of_err: u32,
    /// round 5: each wrapper item's `(sourceTx, deposit)`, as the wrapper's `item(id)` says
    item_src: HashMap<([u8; 20], usize), (Word, bool)>,
    /// fix round 6: a provider shows our transaction in its pool (`EvmView::Pending`)
    pool: bool,
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
            hide: vec![], unsure: false, real_advance: false, identify: false, sent_tx: HashMap::new(), payout_balance_fail: false,
            bal_fail: None, big_sale_blocks: vec![], dense_blocks: vec![], odd_blocks: vec![], receipt_bytes: 0, mint_revert: 0, mint_log: vec![], item_of_err: 0, item_src: HashMap::new(),
            pool: false,
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
                m.advanced = arg(d, 0);
                self.home_balance += arg(d, 0);
            } else if s == rc::RETURN_ADVANCE {
                m.transit = false;
                m.advanced = 0;
                self.home_balance -= value;
            } else if s == ef::FUND_RESERVE {
                // A2-L-1: a sweep pays into the module's surplus; no advance is involved
                assert!(!m.transit, "a sweep never meets an open advance at its module");
                self.home_balance -= value;
            } else if s == rc::SETTLE_ADVANCE {
                m.transit = false;
                m.advanced = 0;
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
        if (to == WRAPPER || to == WRAPPER2) && (sel(d) == en::MINT || sel(d) == en::MINT_DEPOSIT) {
            if self.mint_revert > 0 {
                self.mint_revert -= 1;
                return false; // reverted: nothing minted
            }
            self.mint_log.push((to, now));
            let n = if to == WRAPPER { self.items.len() } else { self.items2.len() };
            self.item_src.insert((to, n), (d[36..68].try_into().unwrap(), sel(d) == en::MINT_DEPOSIT));
        }
        if (to == WRAPPER || to == WRAPPER2) && sel(d) == en::NOTE_SENT {
            self.sent_tx.insert((to, arg(d, 0)), d[36..68].try_into().unwrap());
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
                else if s == fs::MAX_ITEM_COST { word(if m.transit && !self.real_advance { 0 } else { m.item_cap }) }
                else if s == fs::ITEMS_ARRIVED { word(0) }
                else if s == rc::TRANSIT_UNITS { word(if m.transit { E } else { 0 }) }
                else if s == en::ADVANCED { word(m.advanced) }
                else if s == en::ACCEPTED { word(m.accepted) }
                else if s == en::KEEPER { addr_word(&self.keeper) }
                else if s == ef::FLOOR_BOUNDS { [word(m.bounds.0), word(m.bounds.1)].concat() }
                else { word(0) });
        }
        if (to == WRAPPER || to == WRAPPER2) && s == en::ITEM_OF && self.item_of_err > 0 {
            self.item_of_err -= 1;
            return Err("chain 4663: eth_call: IC0406: SysTransient: Timeout expired".into());
        }
        if (to == WRAPPER || to == WRAPPER2) && s == en::ITEM {
            if let Some((src, dep)) = self.item_src.get(&(to, arg(d, 0) as usize)) {
                let t = if to == WRAPPER { self.items[arg(d, 0) as usize] } else { self.items2[arg(d, 0) as usize] };
                return Ok([t.to_vec(), src.to_vec(), word(*dep as u128)].concat());
            }
        }
        if (to == WRAPPER || to == WRAPPER2) && s == en::SENT_TX {
            return Ok(self.sent_tx.get(&(to, arg(d, 0))).map_or(vec![0; 32], |w| w.to_vec()));
        }
        if (to == WRAPPER || to == WRAPPER2) && s == en::UNDELIVERABLE {
            let m = if to == WRAPPER { &self.marked } else { &self.marked2 };
            return Ok(word(m.contains(&arg(d, 0)) as u128));
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
        if self.real_advance && data.len() >= 4 && sel(data) == rc::ADVANCE && self.modules.get(&to).is_some_and(|m| m.transit) {
            return Err("would not succeed now: execution reverted: AdvanceOpen()".into());
        }
        if self.real_advance && data.len() >= 4 && sel(data) == rc::RETURN_ADVANCE && self.modules.get(&to).is_some_and(|m| !m.transit) {
            return Err("would not succeed now: execution reverted: NoAdvance()".into());
        }
        // the real home_sign estimates first: a mint of a token the wrapper already holds reverts (AlreadyHeld / AlreadyMinted)
        if (to == WRAPPER || to == WRAPPER2) && data.len() >= 36 && (sel(data) == en::MINT || sel(data) == en::MINT_DEPOSIT) {
            let t: Word = data[4..36].try_into().unwrap();
            if (if to == WRAPPER { &self.items } else { &self.items2 }).contains(&t) {
                return Err("would not succeed now: execution reverted: AlreadyHeld()".into());
            }
        }
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
        if self.pool {
            return EvmView::Pending;
        }
        if self.unsure {
            return EvmView::Unsure("providers disagree on eth_getTransactionReceipt at the pin; nothing is believed".into());
        }
        if self.hide.contains(&h) {
            return EvmView::NotYet;
        }
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
        if self.big_sale_blocks.contains(&block) {
            self.log_fails += 1;
            self.logs_bytes += 2 * (64_000 + 400_000 + 2_000_000);
            return Err(format!("chain 1: eth_getLogs: {}Http body exceeds size limit of 2000000 bytes", etch_autobuyer::evmread::SIZE_REJECT));
        }
        self.logs_bytes += 2 * 64_000;
        self.sale_moves.get(&block).cloned().ok_or_else(|| "chain 1: eth_getLogs: IC0406: SysTransient: Timeout expired".to_string())
    }
    fn logs_bytes(&self) -> u64 { self.logs_bytes }
    fn receipt_bytes(&self) -> u64 { self.receipt_bytes }
    async fn home_nonce_tx(&mut self, nonce: u64, _from: Option<u64>) -> Result<Option<[u8; 32]>, String> {
        if !self.identify {
            return Err("chain 4663: eth_getTransactionCount: missing trie node (the provider keeps no state that deep)".into());
        }
        if self.home_nonce <= nonce {
            return Ok(None);
        }
        Ok(self.home_sent.iter().find(|(_, (_, raw))| u64::from_be_bytes(raw[36..44].try_into().unwrap()) == nonce).map(|(h, _)| *h))
    }
    async fn eth_sale_moves_range(&mut self, _collection: [u8; 20], from: u64, to: u64) -> Result<Vec<en::Move>, String> {
        if self.big_sale_blocks.iter().any(|b| from <= *b && *b <= to) {
            self.log_fails += 1;
            self.logs_bytes += 2 * (64_000 + 400_000 + 2_000_000);
            return Err(format!("chain 1: eth_getLogs: {}Http body exceeds size limit of 2000000 bytes", etch_autobuyer::evmread::SIZE_REJECT));
        }
        self.logs_bytes += 2 * 64_000;
        if from == to && !self.sale_moves.contains_key(&from) {
            return Err("chain 1: eth_getLogs: IC0406: SysTransient: Timeout expired".to_string());
        }
        // what the marketplaces' own logs show: the blocks' sale moves given, and every receipt the fake knows with a sale of the
        // collection on a marketplace (as `ethnft::sale_logs_body` projects them)
        let mut v: Vec<en::Move> = self.sale_moves.iter().filter(|(b, _)| from <= **b && **b <= to).flat_map(|(_, v)| v.clone()).collect();
        for (h, r) in &self.sale_receipts {
            if r.block >= from && r.block <= to {
                if let Some(x) = en::sales_in(r, &_collection).first() {
                    v.push(en::Move { hash: *h, block: r.block, token: x.token, from: x.seller });
                }
            }
        }
        v.sort_by(|a, b| a.block.cmp(&b.block).then(a.hash.cmp(&b.hash)));
        v.dedup_by(|a, b| a.hash == b.hash);
        Ok(v)
    }
    async fn payout_tx(&mut self, home: bool, hash: [u8; 32]) -> Result<Option<ef::PayoutTx>, String> {
        if self.payout_balance_fail {
            self.bal_fail = Some(1);
        }
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
    async fn eth_balance(&mut self) -> Result<u128, String> {
        match self.bal_fail {
            Some(0) => {
                self.bal_fail = None;
                return Err("chain 1: eth_getBalance: HTTP 429 Too Many Requests".into());
            }
            Some(n) => self.bal_fail = Some(n - 1),
            None => {}
        }
        Ok(self.eth_balance + self.pin_back.map_or(0, |p| p.1))
    }
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
        self.receipt_bytes += 2 * 32_000;
        if self.sale_err.contains(&h) {
            self.receipt_bytes += 2 * 200_000;
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
        if self.odd_blocks.iter().any(|b| from <= *b && *b <= to) {
            self.log_fails += 1;
            self.logs_bytes += 2 * 64_000;
            return Err("chain 1: eth_getLogs: no result: {\"code\":-32000,\"message\":\"internal error\"}".into());
        }
        let dense = self.dense_blocks.iter().filter(|b| from <= **b && **b <= to).count();
        if dense >= 2 {
            self.log_fails += 1;
            self.logs_bytes += 2 * 64_000;
            return Err("chain 1: eth_getLogs: no result: {\"code\":-32005,\"message\":\"query returned more than 10000 results\"}".into());
        }
        if dense == 1 || self.big_blocks.iter().any(|b| from <= *b && *b <= to) {
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

fn cached(st: &EthState, hs: &[[u8; 32]]) -> usize {
    st.sale_cache.iter().flatten().filter(|c| c.collection == MILADY && hs.contains(&c.hash) && !c.sales.is_empty()).count()
}
fn pass(f: &mut Fake, st: &mut EthState) {
    f.now += 1_200; // the runner hands evidence in every 1,200 s
    f.m(MODULE).posted_at = f.now - 60;
    evidence(f, st);
    tick(f, st);
}

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


// =====================================================================================================================================
// F5-M-1: a home transaction whose nonce is used and that no provider shows is never done again on that absence alone
// =====================================================================================================================================

fn home_calls_to(f: &Fake, to: [u8; 20], s: [u8; 4]) -> usize {
    f.home_calls.iter().filter(|(t, _, d)| *t == to && d.len() >= 4 && sel(d) == s).count()
}

/// Ticks `n` times `step` seconds apart; every line of every tick.
fn run_for(f: &mut Fake, st: &mut EthState, n: usize, step: u64) -> Vec<String> {
    let mut all = Vec::new();
    for _ in 0..n {
        f.now += step;
        fresh_modules(f);
        all.extend(tick(f, st));
    }
    all
}

/// A Milady buy funded on 4663 and its Relay deposit MINED, but every provider's index misses it (review5/poc/F `f5_m1_a`). `other`:
/// ETH of other flows at the shared 4663 address (the frog side's float, a late payout, the gas keeps).
fn hidden_deposit(f: &mut Fake, st: &mut EthState, other: u128) -> (u64, [u8; 32], u128) {
    f.m(MODULE).net_units = 2 * E;
    tick(f, st);
    tick(f, st);
    let id = st.buys[0].id;
    assert!(matches!(phase(st), BPhase::Funded { .. }), "{:?}", phase(st));
    f.home_balance += other;
    let start = f.home_balance;
    ef::put_quote(st, id, fwd_quote(DEPOSIT, f.now), f.now).unwrap();
    tick(f, st);
    let p = st.pending.clone().expect("the deposit is out");
    assert!(matches!(p.step, ef::Step::Deposit { .. }), "{:?}", p.step);
    assert_eq!(relay_deposits(f), 1, "the deposit mined: the ETH left for Relay");
    f.hide.push(p.hash);
    (id, p.hash, start)
}

#[test]
fn r5_f5_m1_a_a_mined_relay_deposit_seen_nowhere_is_never_deposited_again() {
    // PoC `f5_m1_a_a_mined_relay_deposit_seen_nowhere_for_an_hour_is_deposited_again`: dropped after the hour, taken back to Funded and
    // deposited AGAIN from the shared address's other ETH: 1.0 ETH spent for one 0.5 ETH buy, the first order's fill nobody's
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let (id, _, start) = hidden_deposit(&mut f, &mut st, E);
    // a whole day with no provider showing it and nothing that tells what used the nonce: it waits, paged (UNIDENTIFIED), never redone
    let lines = run_for(&mut f, &mut st, 6 * 24, 600);
    assert!(!lines.iter().any(|l| l.contains("DROPPED")), "never dropped on the absence of evidence");
    assert!(lines.iter().any(|l| l.contains("UNIDENTIFIED")), "it says why it waits");
    assert!(matches!(phase(&st), BPhase::Depositing { .. }) && st.pending.is_some(), "{:?}", phase(&st));
    assert_eq!(relay_deposits(&f), 1, "one Relay deposit for one buy");
    assert_eq!(start - f.home_balance, DEPOSIT, "the buy's value left once");
    // the chain tells what used the nonce (our nonce count at past blocks and that block's transactions): ours, so it MINED
    f.identify = true;
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("MINED") && l.contains("nonce count"), "{l}");
    assert!(matches!(phase(&st), BPhase::Bridging { .. }) && st.buys[0].last.is_some(), "watched as deposited: {:?}", phase(&st));
    // and the fill is this buy's arrival
    f.eth_balance += ARRIVED;
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buys[0].phase, BPhase::Arrived { .. })), "{:?}", phase(&st));
    assert_eq!(st.earmarked_except(u64::MAX), ARRIVED, "the first order's fill is its own");
    assert_eq!(relay_deposits(&f), 1);
    let _ = id;
}

#[test]
fn r5_f5_m1_a_operator_word_after_looking_decides_an_unidentified_deposit() {
    // where nothing on chain can tell (a keyless provider keeps no state that deep), the operator looks and says (eth_home_clear); refused
    // before the hour, and for another nonce
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let (_, _, _) = hidden_deposit(&mut f, &mut st, E);
    tick(&mut f, &mut st);
    let nonce = st.pending.as_ref().unwrap().nonce;
    assert!(ef::home_clear(&mut st, nonce, true, f.now).unwrap_err().contains("not yet seen nowhere"));
    run_for(&mut f, &mut st, 7, 600);
    assert!(ef::home_clear(&mut st, nonce + 1, true, f.now).unwrap_err().contains("not"));
    ef::home_clear(&mut st, nonce, true, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("the operator says it MINED"), "{l}");
    assert!(matches!(phase(&st), BPhase::Bridging { .. }), "{:?}", phase(&st));
    assert_eq!(relay_deposits(&f), 1);
}

#[test]
fn r5_f5_m1_a_control_another_transaction_at_the_nonce_is_dropped_and_redone() {
    // the drop's designed case (a transaction of ours signed at a nonce another of ours used, which an earlier build could leave): when
    // the chain shows ANOTHER transaction at the nonce, ours can never mine, and its step is taken back and done again
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).posted_at = f.now - 3_000;
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("a post is out");
    // swap what the chain shows at that nonce for another transaction of ours
    let (ok, raw) = f.home_sent.remove(&p.hash).unwrap();
    let mut other = raw.clone();
    other.push(0);
    f.home_sent.insert(keccak(&other), (ok, other));
    f.identify = true;
    f.now += 60;
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("DROPPED") && l.contains("another transaction of ours"), "{l}");
    assert!(st.pending.is_none() || st.pending.as_ref().unwrap().hash != p.hash);
}

#[test]
fn r5_f5_m1_b_a_mined_fund_reserve_seen_nowhere_is_never_paid_again() {
    // PoC `f5_m1_b_…`: 2 fundReserve calls, 0.6 ETH paid in for a 0.3 ETH sweep
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let wei = 3 * E / 10;
    st.buys.push(ef::EthBuy { id: 7, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: 0, attempts: 0,
        phase: BPhase::Returning { wei }, orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(),
        sweep: Some(true), arrived_at: None, back_at: None, ended_at: None, late: None });
    f.home_balance = E;
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("fundReserve is out");
    f.hide.push(p.hash);
    let lines = run_for(&mut f, &mut st, 6 * 24, 600);
    assert!(!lines.iter().any(|l| l.contains("DROPPED")) && lines.iter().any(|l| l.contains("UNIDENTIFIED")));
    assert_eq!(home_calls_to(&f, MODULE, ef::FUND_RESERVE), 1, "fundReserve went out once");
    assert_eq!(E - f.home_balance, wei);
    f.identify = true;
    tick(&mut f, &mut st);
    assert!(matches!(st.buys[0].phase, BPhase::Returned { .. }), "{:?}", st.buys[0].phase);
    assert_eq!(home_calls_to(&f, MODULE, ef::FUND_RESERVE), 1);
}

#[test]
fn r5_f5_m1_c_a_mined_advance_seen_nowhere_is_read_back_from_the_module() {
    // PoC `f5_m1_c_…`: the advance taken back (NeedAdvance, advanced 0), dropped, its ETH at our address tracked by no buy
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("the advance is out");
    assert!(matches!(p.step, ef::Step::Advance { .. }) && f.m(MODULE).transit);
    f.hide.push(p.hash);
    // no provider shows it and nothing tells what used the nonce, but its effect is on chain: the module's advance is open
    let l = run_for(&mut f, &mut st, 1, 60).join("\n");
    assert!(l.contains("its effect is on chain") && l.contains("MINED"), "{l}");
    assert!(matches!(phase(&st), BPhase::Funded { .. }) && st.buys[0].advanced > 0, "{:?}", phase(&st));
    assert_eq!(home_calls_to(&f, MODULE, rc::ADVANCE), 1);
}

#[test]
fn r5_f5_m1_c2_on_the_real_module_nothing_holds_the_relay_leg() {
    // PoC `f5_m1_c2_…`: on the real module the taken-back buy re-advanced into AdvanceOpen every tick, in NeedAdvance (holding the Relay
    // leg for every collection) with no operator exit
    let mut f = Fake::new(true);
    f.real_advance = true;
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("the advance is out");
    f.hide.push(p.hash);
    run_for(&mut f, &mut st, 2, 600);
    let m = buy_of(&st, MILADY).unwrap();
    assert!(matches!(m.phase, BPhase::Funded { .. }), "{:?}", m.phase);
    // Milady's buy goes on: the runner's quote, deposited, filled; then Azuki buys
    f.m(MODULE2).net_units = 2 * E;
    let ask = m.ask.clone().unwrap().amount_wei;
    ef::put_quote(&mut st, m.id, fwd_quote(ask, f.now), f.now).unwrap();
    assert!(until(&mut f, &mut st, 3, |s| matches!(buy_of(s, MILADY).unwrap().phase, BPhase::Bridging { .. })));
    f.eth_balance += ARRIVED;
    assert!(until(&mut f, &mut st, 3, |s| matches!(buy_of(s, MILADY).unwrap().phase, BPhase::Arrived { .. })));
    assert!(until(&mut f, &mut st, 6, |s| buy_of(s, AZUKI).is_some()), "Azuki buys");
}

#[test]
fn r5_f5_m1_c3_an_orphaned_advance_an_earlier_build_left_is_adopted_never_paid_again() {
    // the state an earlier build's drop left: the module's advance open, the buy taken back to NeedAdvance with nothing tracked
    let mut f = Fake::new(false);
    f.real_advance = true;
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    let p = st.pending.clone().unwrap();
    let advanced = st.buys[0].advanced;
    // what 1dc2a61 did an hour later: pending dropped, the buy back at NeedAdvance, advanced 0, the reservation settled
    st.pending = None;
    st.home_floor = Some((p.nonce + 1, st.home_floor.map_or(0, |x| x.1)));
    st.buys[0].phase = BPhase::NeedAdvance;
    st.buys[0].advanced = 0;
    let _ = f.with_caps(|_, bk| custodycap::settle(bk, &format!("eth:{}", st.buys[0].id), None));
    let l = run_for(&mut f, &mut st, 1, 60).join("\n");
    assert!(l.contains("adopted") && l.contains("nothing paid again"), "{l}");
    assert!(matches!(phase(&st), BPhase::Funded { .. }) && st.buys[0].advanced == advanced, "{:?}", phase(&st));
    assert_eq!(home_calls_to(&f, MODULE, rc::ADVANCE), 1, "never advanced twice");
}

#[test]
fn r5_f5_m1_d_a_provider_that_shows_the_transaction_restarts_the_clock() {
    // PoC `f5_m1_d_…`: "seen nowhere for an hour" was two samples an hour apart; reads in between that showed it did not restart it.
    // Fix round 6 (F6-L-2): a provider that shows it in its POOL restarts the clock; providers that disagree on its receipt (Unsure)
    // with its nonce used no longer do (that is decided as "seen nowhere" is: `audit_nft_r6.rs` `r6_f6_l2_…`)
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).posted_at = f.now - 3_000;
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("a post is out");
    f.hide.push(p.hash);
    f.now += 60;
    tick(&mut f, &mut st);
    assert!(st.pending.as_ref().and_then(|x| x.used_since).is_some(), "the clock started");
    f.pool = true;
    f.now += 600;
    tick(&mut f, &mut st);
    assert_eq!(st.pending.as_ref().and_then(|x| x.used_since), None, "a provider that shows it restarts the clock");
    for _ in 0..5 {
        f.now += 600;
        tick(&mut f, &mut st);
    }
    f.pool = false;
    f.now += 60;
    let l = tick(&mut f, &mut st).join("\n");
    assert!(!l.contains("DROPPED"), "not dropped on the first unseen read after an hour of other views: {l}");
    // an hour of unseen reads in a row: a post (harmless to do again) goes then
    let l = run_for(&mut f, &mut st, 7, 600).join("\n");
    assert!(l.contains("DROPPED"), "{l}");
}

#[test]
fn r5_f5_m1_e_a_mined_return_advance_seen_nowhere_is_read_back_from_the_module() {
    // PoC `f5_m1_e_…`: the buy left Returning (holding the Relay leg), its estimate reverting NoAdvance, eth_give_back refused
    let mut f = Fake::new(true);
    f.real_advance = true;
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap();
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("returnAdvance is out");
    assert!(matches!(p.step, ef::Step::GiveBack { .. }) && !f.m(MODULE).transit);
    f.hide.push(p.hash);
    run_for(&mut f, &mut st, 2, 600);
    assert!(matches!(buy_of(&st, MILADY).unwrap().phase, BPhase::Returned { .. }), "{:?}", buy_of(&st, MILADY).unwrap().phase);
    assert_eq!(home_calls_to(&f, MODULE, rc::RETURN_ADVANCE), 1);
    f.m(MODULE2).net_units = 2 * E;
    assert!(until(&mut f, &mut st, 6, |s| buy_of(s, AZUKI).is_some()), "Azuki buys");
}

#[test]
fn r5_f5_m1_e2_a_return_an_earlier_build_took_back_is_read_back_never_paid_again() {
    // the state 1dc2a61 left: the returnAdvance mined, taken back as a revert, the buy Returning, the module's advance closed
    let mut f = Fake::new(false);
    f.real_advance = true;
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap();
    tick(&mut f, &mut st);
    let p = st.pending.clone().unwrap();
    st.pending = None;
    st.home_floor = Some((p.nonce + 1, st.home_floor.map_or(0, |x| x.1)));
    let l = run_for(&mut f, &mut st, 1, 60).join("\n");
    assert!(l.contains("already closed"), "{l}");
    assert!(matches!(st.buys[0].phase, BPhase::Returned { .. }));
    assert_eq!(home_calls_to(&f, MODULE, rc::RETURN_ADVANCE), 1);
}

#[test]
fn r5_f5_m1_record_seen_nowhere_is_recorded_and_its_item_leaves_the_book() {
    // the table's Record row: a dropped record went Gone (its ticket already Delivered) without releasing the item from custody
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let by = f.now + 7 * 86_400;
    resolved(&mut f, false, 9_180, ALICE, by);
    custodycap::admit(&f.caps, &mut f.book, &held_id(MILADY, 9_180), &custodycap::evm_key(1, &MILADY), 0).unwrap();
    for _ in 0..40 {
        if st.pending.as_ref().is_some_and(|p| matches!(p.step, ef::Step::Record { .. })) {
            break;
        }
        if let Some(i) = f.eth_signed.len().checked_sub(1) {
            if f.receipts.is_empty() {
                f.mine_eth(i, true, ETH_PIN - 10);
            }
        }
        f.finalized = ETH_PIN;
        run_for(&mut f, &mut st, 1, 600);
    }
    let p = st.pending.clone().expect("the record is out");
    assert!(matches!(p.step, ef::Step::Record { .. }), "{:?}", p.step);
    f.hide.push(p.hash);
    run_for(&mut f, &mut st, 2, 600);
    assert!(matches!(st.deliveries[0].phase, ef::DPhase::Recorded { .. }), "{:?}", st.deliveries[0].phase);
    assert!(!f.book.contains(&held_id(MILADY, 9_180)), "the item left the custody book");
}

// =====================================================================================================================================
// C5-M-1 / C5-L-1 / C5-L-3: a written-off order keeps the Relay leg's attribution; a payout is its own order's by what it names
// =====================================================================================================================================

const PAYOUT_TX: [u8; 32] = [0x5a; 32];

/// Milady's way back, neither filled nor refunded a day past its deadline and grace, written off by the operator (review5/poc/C).
fn milady_written_off(f: &mut Fake, st: &mut EthState) -> (u64, u128) {
    let mid = to_back_funded(f, st);
    let amount = to_back_bridging(f, st, mid);
    let BPhase::BackBridging { deadline, .. } = st.buy(mid).unwrap().phase else { unreachable!() };
    f.m(MODULE2).net_units = 2 * E;
    while f.now <= deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 60 {
        f.now += 1_200;
        fresh_modules(f);
        tick(f, st);
    }
    assert!(buy_of(st, AZUKI).is_none(), "no other order starts while Milady's order is watched");
    let l = ef::give_back_now(st, mid, f.home_balance, f.now).unwrap();
    assert!(l.contains("written off") && l.contains("holds the Relay leg"), "{l}");
    (mid, amount)
}

#[test]
fn r5_c5_m1_a_a_payout_after_the_write_off_is_never_read_as_another_orders() {
    // PoC `c5_m1_a_payout_after_the_write_off_is_still_read_as_another_collections_refund`: Azuki's watch read Milady's late payout as its
    // refund, bridged twice, Milady's credit was refused, 0.51 ETH at our Ethereum address was nobody's
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount) = milady_written_off(&mut f, &mut st);
    // the written-off order holds the Relay leg: no other order starts, so no watch is open to read its payout
    let l = run_for(&mut f, &mut st, 12, 1_200).join("\n");
    assert!(buy_of(&st, AZUKI).is_none() && st.relay_busy() && st.late_hold() == Some(mid), "{l}");
    assert!(l.contains("no new Relay order"), "it says why");
    // Relay pays Milady's way back after all: a fill on 4663 whose calldata names the order
    let oid = st.buy(mid).unwrap().late.as_ref().unwrap().watch.order_id;
    let payout = amount / 100 * 98;
    f.home_balance += payout;
    f.payouts.insert((true, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: payout, input: oid.to_vec(), block: HOME_PIN + 5, success: true });
    tick(&mut f, &mut st);
    assert!(buy_of(&st, AZUKI).is_none(), "still nobody reads it");
    ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED"), "{l}");
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(mid).unwrap().phase, BPhase::Returned { .. })));
    assert_eq!(returned_to(&f, MODULE, rc::RETURN_ADVANCE), payout, "Milady's module gets its ETH back");
    // credited: the leg is free; Azuki bridges once, its own ETH
    assert!(until(&mut f, &mut st, 8, |s| buy_of(s, AZUKI).is_some_and(|b| matches!(b.phase, BPhase::Funded { .. }))), "{:?}", buy_of(&st, AZUKI));
    let az = buy_of(&st, AZUKI).unwrap().id;
    let d0 = relay_deposits(&f);
    let ask = st.buy(az).unwrap().ask.clone().unwrap().amount_wei;
    ef::put_quote(&mut st, az, fwd_quote(ask, f.now), f.now).unwrap();
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Bridging { .. })));
    f.eth_balance += ARRIVED;
    assert!(until(&mut f, &mut st, 3, |s| matches!(s.buy(az).unwrap().phase, BPhase::Arrived { .. })));
    assert_eq!(relay_deposits(&f) - d0, 1, "Azuki bridged once");
}

#[test]
fn r5_c5_m1_b_released_by_the_operator_the_leg_is_free_and_a_later_payout_is_still_creditable() {
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, _) = milady_written_off(&mut f, &mut st);
    assert!(ef::late_release(&mut st, 99, f.now).is_err());
    ef::late_release(&mut st, mid, f.now).unwrap();
    assert!(ef::late_release(&mut st, mid, f.now).unwrap_err().contains("released already"));
    assert!(st.late_hold().is_none());
    assert!(until(&mut f, &mut st, 8, |s| buy_of(s, AZUKI).is_some()), "new orders start again");
    assert!(ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).is_ok(), "a payout of it may still be handed in");
}

#[test]
fn r5_c5_l1_a_another_relay_payment_from_the_shared_solver_is_not_this_orders() {
    // PoC `c5_l1_a_…`: a solver transfer of another request (mined before Milady's order existed) credited as Milady's, paid out of the
    // frog side's ETH
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    let (id, amount, _) = way_back_past(&mut f, &mut st, 2);
    f.now += 3_600;
    ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap();
    ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap();
    assert!(until(&mut f, &mut st, 4, |s| matches!(s.buy(id).unwrap().phase, BPhase::Returned { wei: 0 })));
    f.home_balance += 2 * E;
    let frog_refund = amount / 100 * 98;
    for (i, (input, block)) in [(vec![0x17; 32], 1), (vec![0x17; 32], HOME_PIN + 5), (vec![], HOME_PIN + 5)].into_iter().enumerate() {
        let tx = [0x60 + i as u8; 32];
        f.payouts.insert((true, tx), ef::PayoutTx { from: SOLVER, to: US, value: frog_refund, input, block, success: true });
        ef::put_payout(&mut st, id, true, tx, 0, f.now).unwrap();
        let l = tick(&mut f, &mut st).join("\n");
        assert!(l.contains("refused, nothing credited") && l.contains("does not name order"), "{l}");
    }
    // the order's own id, but mined before its deposit: not its payout either
    let oid = st.buy(id).unwrap().late.as_ref().unwrap().watch.order_id;
    f.payouts.insert((true, [0x6f; 32]), ef::PayoutTx { from: SOLVER, to: US, value: frog_refund, input: oid.to_vec(), block: 1, success: true });
    ef::put_payout(&mut st, id, true, [0x6f; 32], 0, f.now).unwrap();
    assert!(tick(&mut f, &mut st).join("\n").contains("not after the order was deposited"));
    assert_eq!(returned_to(&f, MODULE, ef::FUND_RESERVE), 0, "none of the frog side's ETH is paid into Milady's module");
}

#[test]
fn r5_c5_l1_b_our_own_deposit_and_a_strangers_call_are_never_payouts() {
    // PoC `c5_l1_b_…`: our own depositNative(us, id) and a stranger's 0-value call carrying the id, both judged payouts of 5 ETH
    let w = ef::Watch { order_id: [0x0d; 32], value: 5 * E, min_out: 49 * E / 10, deadline: 1, balance_after: 0, eth_before: 0,
        eth_out_before: 0, pin: Some(26_000_000), solver: Some(SOLVER), request_id: Some([0x17; 32]), home_pin: Some(70_000_000) };
    let o = ef::LateOrder { watch: w.clone(), back: true, let_go_at: 2, credited: None, released_at: None };
    let dep = [rc::DEPOSIT_NATIVE.to_vec(), addr_word(&US), w.order_id.to_vec()].concat();
    let ours = ef::PayoutTx { from: US, to: rc::RELAY_DEPOSITORY_4663, value: 5 * E, input: dep.clone(), block: 26_000_001, success: true };
    assert!(ef::judge_payout(&o, &ours, &[], &[0x0a; 32], false, &US, 5 * E, &[], false).unwrap_err().contains("ours"));
    // a stranger's deposit into Relay naming the order is not a payout either
    let theirs = ef::PayoutTx { from: ATTACKER, ..ours.clone() };
    assert!(ef::judge_payout(&o, &theirs, &[], &[0x0a; 32], false, &US, 5 * E, &[], false).is_err());
    let junk = [vec![0xde, 0xad], w.order_id.to_vec(), vec![0xbe]].concat();
    let stranger = ef::PayoutTx { from: ATTACKER, to: ATTACKER, value: 0, input: junk, block: 70_000_001, success: true };
    assert!(ef::judge_payout(&o, &stranger, &[], &[0x0b; 32], true, &US, 5 * E, &[], false).is_err());
    // what IS a payout: Relay's fill naming the request id (its calldata), after the deposit, to us
    let fill = ef::PayoutTx { from: SOLVER, to: US, value: 49 * E / 10, input: vec![0x17; 32], block: 70_000_001, success: true };
    assert_eq!(ef::judge_payout(&o, &fill, &[], &[0x0c; 32], true, &US, 0, &[], false), Ok(49 * E / 10));
    // and a refund paid from inside Relay's depository (a call to it naming the order). Fix round 6 (C6-L-1): with Relay's own execution
    // record (the depository's RelayCallExecuted paying us) in its receipt, at that record's amount
    // fix round 7 (C7-L-1): the depository's real `execute` (here naming the order through its nonce) and its record as the contract
    // emits it: one topic, the id the request's struct hash
    let req = rc::CallRequest { calls: vec![rc::DepCall { to: US, data: vec![], value: 3 * E, allow_failure: false }], nonce: w.order_id,
        expiration: [0; 32] };
    let refund = ef::PayoutTx { from: [0x77; 20], to: rc::RELAY_DEPOSITORY_4663, value: 0, input: rc::encode_execute(&req, &[0x11; 65]),
        block: 26_000_002, success: true };
    let wd = |x: u128| sp::word_u128(x).to_vec();
    let exec = en::LogV { address: rc::RELAY_DEPOSITORY_4663, topics: vec![rc::RELAY_CALL_EXECUTED],
        data: [rc::call_request_hash(&req).to_vec(), wd(0x40), addr_word(&US), wd(0x80), wd(3 * E), wd(0), wd(0)].concat() };
    assert_eq!(ef::judge_payout(&o, &refund, &[exec], &[0x0d; 32], false, &US, 3 * E, &[], false), Ok(3 * E));
    assert!(ef::judge_payout(&o, &refund, &[], &[0x0d; 32], false, &US, 3 * E, &[], false).is_err(), "without it, no payout");
}

#[test]
fn r5_c5_l1_c_no_watch_is_open_whose_unread_refund_a_credit_could_take() {
    // PoC `c5_l1_c_…`: after the write-off Azuki's order was Bridging; its refund, handed in as Milady's (the solver path), was credited
    // to Milady out of Azuki's ETH, and Azuki then held the Relay leg on ETH that was gone. No order is watched now while Milady's is
    // written off, and the judge takes only what names Milady's order
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount) = milady_written_off(&mut f, &mut st);
    run_for(&mut f, &mut st, 6, 1_200);
    assert!(st.buys.iter().filter(|b| b.id != mid).all(|b| !b.in_relay()), "no other order on the leg");
    let refund = amount / 100 * 99;
    f.home_balance += refund;
    f.payouts.insert((true, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: refund, input: vec![0x42; 32], block: HOME_PIN + 5, success: true });
    ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("refused, nothing credited"), "{l}");
    assert_eq!(returned_to(&f, MODULE, rc::RETURN_ADVANCE), 0);
}

#[test]
fn r5_c5_l3_a_failed_balance_read_credits_nothing_and_the_payout_can_be_handed_in_again() {
    // PoC `c5_l3_a_…`: the credit was recorded before the (false, open) branch's second balance read, which failed: the payout consumed
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    st.used_orders.clear();
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let BPhase::Bridging { deadline, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    f.now = deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 60;
    fresh_modules(&mut f);
    tick(&mut f, &mut st);
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("written off"));
    let oid = st.buy(id).unwrap().late.as_ref().unwrap().watch.order_id;
    f.eth_balance += ARRIVED;
    // Relay fills it after all, on Ethereum, naming the order, after its deposit; final
    f.pin_now = Some(ETH_PIN + 300);
    f.finalized = ETH_PIN + 250;
    f.payouts.insert((false, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: ARRIVED, input: oid.to_vec(), block: ETH_PIN + 200, success: true });
    // the one balance read fails: nothing is written
    f.bal_fail = Some(0);
    ef::put_payout(&mut st, id, false, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("refused, nothing credited") && l.contains("429"), "{l}");
    let b = st.buy(id).unwrap();
    assert!(b.late.as_ref().unwrap().credited.is_none() && st.credited.as_ref().is_none_or(|v| !v.contains(&PAYOUT_TX)), "nothing recorded");
    // handed in again: credited, and it is the buy's arrival
    ef::put_payout(&mut st, id, false, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED"), "{l}");
    assert!(matches!(st.buy(id).unwrap().phase, BPhase::Arrived { .. }) && st.earmarked_except(u64::MAX) == ARRIVED);
}

// =====================================================================================================================================
// B5-M-1 / B5-M-2 / B5-L-1 / B4-L-2: the scan's progress cannot be starved; a too-big range, however it says so, never holds a walk;
// the pass's bytes are a real bound (review5/poc/B's harness: `run`, `show` as the reviewer wrote them)
// =====================================================================================================================================

fn quiet(f: &mut Fake, st: &mut EthState) -> Vec<String> {
    f.now += 1_200;
    f.m(MODULE).posted_at = f.now - 60;
    evidence(f, st);
    block_on(ef::tick(f, st, false))
}
fn scan_of(st: &EthState) -> ef::SaleScan {
    st.sale_scan.iter().flatten().find(|s| s.collection == MILADY).unwrap().clone()
}
/// `n` honest newer sales, one every `every` blocks from `first` (distinct tokens, sellers, buyers).
fn newer_sales(f: &mut Fake, first: u64, every: u64, n: u64) -> Vec<([u8; 32], u64)> {
    let mut v = vec![];
    for i in 0..n {
        let mut h = [0x5e; 32];
        h[..8].copy_from_slice(&(i + 1).to_be_bytes());
        let s = { let mut a = [0x71u8; 20]; a[12..].copy_from_slice(&i.to_be_bytes()); a };
        let b = { let mut a = [0x72u8; 20]; a[12..].copy_from_slice(&i.to_be_bytes()); a };
        let blk = first + every * i;
        f.sale_receipts.insert(h, sale_receipt(MILADY, 30_000 + i as u128, 94 * E / 100, s, b, blk));
        v.push((h, blk));
    }
    v
}
struct Run { read_day: usize, read_end: usize, final_end: usize, up_lag_day: u64, up_lag_end: u64, max_fails: u32, max_bytes: u64,
    total_fails: u32, max_bad: usize, first_read_pass: Option<usize>, past_attack_pass: Option<usize>, passes_held: usize,
    passes_no_reading: usize, sales_counted_end: u32, held_end: bool, posted_end: bool }

/// The window read first (4 passes at ETH_PIN), then the attacker's oversized blocks `attack` appear (their marketplace logs read fine
/// and hold no sale of the collection, as on mainnet; or are oversized too with `sale_logs_big`), and `passes` passes run with the
/// pin moving 100 blocks a pass (the runner's 1,200 s).
fn run(attack: &[u64], sale_logs_big: bool, sales: (u64, u64, u64), passes: usize) -> Run {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 { quiet(&mut f, &mut st); }
    assert_eq!(cached(&st, &real_sales()), 5);
    f.big_blocks = attack.to_vec();
    for b in attack {
        if sale_logs_big { f.big_sale_blocks.push(*b); } else { f.sale_moves.insert(*b, vec![]); }
    }
    let new = newer_sales(&mut f, sales.0, sales.1, sales.2);
    let hs: Vec<[u8; 32]> = new.iter().map(|x| x.0).collect();
    let (mut max_fails, mut max_bytes, mut max_bad) = (0, 0, 0);
    let (mut read_day, mut up_lag_day) = (0, 0);
    let (mut first_read_pass, mut past_attack_pass) = (None, None);
    let last_attack = attack.iter().copied().max().unwrap_or(0);
    let (mut passes_held, mut passes_no_reading) = (0, 0);
    let mut pin = ETH_PIN;
    for i in 0..passes {
        pin += 100;
        f.pin_now = Some(pin);
        f.finalized = pin;
        let (b0, l0) = (f.logs_bytes, f.log_fails);
        quiet(&mut f, &mut st);
        max_fails = max_fails.max(f.log_fails - l0);
        max_bytes = max_bytes.max(f.logs_bytes - b0);
        max_bad = max_bad.max(scan_of(&st).bad.map_or(0, |b| b.len()));
        match st.reading(&MILADY) {
            None => passes_no_reading += 1,
            Some(r) if r.post.held.is_some() => passes_held += 1,
            _ => {}
        }
        if first_read_pass.is_none() && cached(&st, &hs) > 0 { first_read_pass = Some(i + 1); }
        if past_attack_pass.is_none() && scan_of(&st).up >= last_attack { past_attack_pass = Some(i + 1); }
        if i + 1 == 72 {
            read_day = cached(&st, &hs);
            up_lag_day = (pin - en::SALE_FINAL_DEPTH_BLOCKS).saturating_sub(scan_of(&st).up);
        }
    }
    let top = pin - en::SALE_FINAL_DEPTH_BLOCKS;
    let sc = scan_of(&st);
    let r = st.reading(&MILADY);
    Run { read_day, read_end: cached(&st, &hs), final_end: new.iter().filter(|x| x.1 <= top).count(), up_lag_day,
        up_lag_end: top.saturating_sub(sc.up), max_fails, max_bytes, total_fails: f.log_fails, max_bad, first_read_pass, past_attack_pass,
        passes_held, passes_no_reading, sales_counted_end: r.map_or(0, |r| r.post.sales), held_end: r.is_some_and(|r| r.post.held.is_some()),
        posted_end: r.is_some() }
}
fn show(name: &str, attack_blocks: usize, r: &Run) {
    println!("{name}: {attack_blocks} oversized block(s): newer sales read after a day {} / at the end {} of {} final; forward walk behind \
        the final head by {} blocks after a day, {} at the end; failed log reads: at most {} a pass, {} in all; reserved bytes: at most {} a pass; \
        SaleScan.bad at most {} ranges; first newer sale read at pass {:?}; the walk past the last oversized block at pass {:?}; passes with a \
        HELD reading {}, with no reading {}; the last reading: {} sales counted, held {}, a reading {}",
        r.read_day, r.read_end, r.final_end, r.up_lag_day, r.up_lag_end, r.max_fails, r.total_fails, r.max_bytes, r.max_bad,
        r.first_read_pass, r.past_attack_pass, r.passes_held, r.passes_no_reading, r.sales_counted_end, r.held_end, r.posted_end);
}


#[test]
fn r5_b5_m1_a_a_burst_of_oversized_blocks_no_longer_holds_the_newer_sales() {
    // PoC `r5_b5_m1_a_…`: 126 oversized blocks one apart held every newer sale unread for 504 passes (a week)
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let sales = (top0 + 300, 100, 60);
    let burst: Vec<u64> = (0..126).map(|j| top0 + 1 + 2 * j).collect();
    let r = run(&burst, false, sales, 144);
    show("burst of 126 every other block (2 days)", burst.len(), &r);
    assert!(r.first_read_pass.is_some_and(|p| p <= 6), "a newer sale is read within a few passes: {:?}", r.first_read_pass);
    assert!(r.read_day + 2 >= r.final_end.min(24), "the day's newer sales are read: {} of {}", r.read_day, r.final_end);
    assert!(r.read_end + 2 >= r.final_end, "all of them in two days: {} of {}", r.read_end, r.final_end);
    assert!(r.up_lag_end <= 200, "the newest blocks' walk keeps up: {} behind", r.up_lag_end);
    assert_eq!(r.passes_held, 0, "no post HELD for want of reading");
    // control (the fix's case, one block)
    let one = run(&[top0 + 1], false, sales, 72);
    assert!(one.read_end + 2 >= one.final_end);
}

#[test]
fn r5_b5_m1_b_one_oversized_block_a_pass_no_longer_leaves_the_walk_behind() {
    // PoC `r5_b5_m1_b_…`: one a pass for a week: 30,406 blocks behind, 200 of 504 newer sales read
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let attack: Vec<u64> = (0..504).map(|j| top0 + 1 + 100 * j).collect();
    let r = run(&attack, false, (top0 + 50, 100, 504), 504);
    show("one oversized block a pass (a week)", attack.len(), &r);
    assert!(r.up_lag_day <= 200 && r.up_lag_end <= 200, "never behind: {} then {}", r.up_lag_day, r.up_lag_end);
    assert!(r.read_end + 5 >= r.final_end, "the newer sales are read: {} of {}", r.read_end, r.final_end);
}

#[test]
fn r5_b5_m1_c_with_no_anchor_the_first_read_lands_and_the_first_post_follows() {
    // PoC `r5_b5_m1_c_…`: one oversized block every 50 blocks kept the first read from ever landing: no sale read, no first post
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    for (name, attack) in [("one every 50 blocks, offset 17", (0..400u64).map(|j| top0 - 483 + 50 * j).collect::<Vec<_>>()),
        ("one every 50 blocks, offset 41", (0..400u64).map(|j| top0 - 459 + 50 * j).collect()),
        ("one every 10 blocks", (0..2_000u64).map(|j| top0 - 5_000 + 10 * j).collect())] {
        let mut f = Fake::new(false);
        let mut st = EthState::default();
        f.finalized = ETH_PIN;
        f.m(MODULE).accepted = 93 * E / 100;
        f.big_blocks = attack.clone();
        for b in &attack {
            f.sale_moves.insert(*b, vec![]);
        }
        let mut pin = ETH_PIN;
        let mut first = None;
        for i in 0..144 {
            pin += 100;
            f.pin_now = Some(pin);
            f.finalized = pin;
            quiet(&mut f, &mut st);
            if first.is_none() && st.reading(&MILADY).is_some_and(|r| r.post.sales >= 3) {
                first = Some(i + 1);
            }
        }
        let sc = scan_of(&st);
        println!("{name}: first reading with >= 3 counted sales at pass {first:?}; {} of 5 real sales read; scan up {}; {} failed reads",
            cached(&st, &real_sales()), sc.up, f.log_fails);
        assert!(sc.up != 0, "{name}: anchored");
        assert!(first.is_some_and(|p| p <= 72), "{name}: the first post within a day: {first:?}");
    }
}

#[test]
fn r5_b5_m2_a_provider_result_cap_error_is_taken_as_too_big_and_the_walk_goes_on() {
    // PoC `r5_b5_m2_a_…`: a range a provider refuses with an error of its own (a result cap) took round 2's memoryless halve-and-retry:
    // 0 of 40 newer sales read in a day, the walk stuck 4 blocks before the dense pair
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        quiet(&mut f, &mut st);
    }
    f.dense_blocks = vec![top0 + 60, top0 + 61];
    for b in f.dense_blocks.clone() {
        f.sale_moves.insert(b, vec![]);
    }
    let new = newer_sales(&mut f, top0 + 100, 100, 40);
    let hs: Vec<[u8; 32]> = new.iter().map(|x| x.0).collect();
    let mut pin = ETH_PIN;
    for _ in 0..72 {
        pin += 100;
        f.pin_now = Some(pin);
        f.finalized = pin;
        quiet(&mut f, &mut st);
    }
    let sc = scan_of(&st);
    let got = cached(&st, &hs);
    println!("B5-M-2 fixed: after a day {got} of the 40 newer sales read; scan up {} (the dense pair at {}), bad {:?}", sc.up, top0 + 60, sc.bad);
    assert!(sc.up > top0 + 61, "past the pair");
    assert!(got >= 38, "the newer sales are read: {got}");
}

#[test]
fn r5_b5_m2_every_too_big_answer_takes_the_same_path_the_measured_shapes() {
    use ef::{scan_error, ScanErr};
    // Review round 6 (B6-I-1): the shapes go through the canister's real transforms (`eth_transfer_logs` / `eth_sale_logs`), which drop
    // a provider's words and keep their class, and `rpc_once_with`'s wording, exactly as the canister sees them. Measured 2026-09-28
    // (rounds 5 and 6): publicnode answers 74 MB for 1,000 dense blocks (the IC refuses it at 2 MB) and refuses logs older than about
    // 10,000 blocks; xrpc.cl answers -32005 "capacity temporarily busy" (an HTTP 429 rate limit, review round 6 B6-L-2) and -32000 when
    // its backend is syncing; MEV Blocker caps a read at 10,000 results (-32005), Tenderly at 20,000 (-32602 "invalid params", the words
    // in `data`) and rate-limits with -32005 "rate limit exceeded"
    let seen = |raw: &str| {
        let b = en::transfer_logs_body(raw.as_bytes());
        assert_eq!(b, en::sale_logs_body(raw.as_bytes(), &MILADY), "both transforms agree");
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        format!("chain 1: eth_getLogs: no result: {}", v.get("error").map(|e| e.to_string()).unwrap_or_default())
    };
    let size = format!("chain 1: eth_getLogs: {}Http body exceeds size limit of 2000000 bytes", etch_autobuyer::evmread::SIZE_REJECT);
    assert_eq!(scan_error(&size), ScanErr::TooBig);
    for (raw, want) in [
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"query returned more than 10000 results. Try with this block range [0x18DDA4A, 0x18DDA9A].","data":{"from":"0x18DDA4A","limit":10000}}}"#, ScanErr::TooBig),
        (r#"{"id":1,"jsonrpc":"2.0","error":{"code":-32602,"message":"invalid params","data":"Query returned more than 20000 results. Try with this block range [0x18dda4b, 0x18ddaf9]."}}"#, ScanErr::TooBig),
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Log response size exceeded. You can make eth_getLogs requests with up to a 2K block range"}}"#, ScanErr::TooBig),
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"block range is too wide"}}"#, ScanErr::TooBig),
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"query timeout exceeded"}}"#, ScanErr::TooBig),
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Archive requests require a personal token. Get one at: https://www.allnodes.com/publicnode"}}"#, ScanErr::Deep),
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"XRPC anonymous capacity temporarily busy"}}"#, ScanErr::Busy),
        (r#"{"id":1,"jsonrpc":"2.0","error":{"code":-32005,"message":"rate limit exceeded"}}"#, ScanErr::Busy),
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"XRPC Cluster: Backend eth node currently syncing and fallbacks busy"}}"#, ScanErr::Busy),
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32701,"message":"broker: node error"}}"#, ScanErr::Other),
        // a bare -32005 is both a rate limit and a result cap: it names nothing, and is deferred only when it persists
        (r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005}}"#, ScanErr::Other),
    ] {
        let s = seen(raw);
        assert_eq!(scan_error(&s), want, "{raw} -> {s}");
    }
    // an HTTP 429 page (MEV Blocker's Cloudflare: `error code: 1015`) is a rate limit (ethnet's transforms give it `pin::busy_body`)
    let page = format!("chain 1: eth_getLogs: no result: {}", String::from_utf8(etch_autobuyer::pin::busy_body()).unwrap()
        .trim_start_matches(r#"{"error":"#).trim_end_matches('}'));
    assert_eq!(scan_error(&page), ScanErr::Busy, "{page}");
    // the raw words, where a world keeps them, take the same path
    assert_eq!(scan_error(r#"no result: {"code":-32602,"message":"Archive requests require a personal token."}"#), ScanErr::Deep);
    assert_eq!(scan_error("chain 1: eth_getLogs: IC0406: SysTransient: Timeout expired"), ScanErr::TooBig, "a timeout may be a dense range");
    assert_eq!(scan_error("chain 1: providers disagree on eth_getLogs; nothing is believed"), ScanErr::Other);
}

#[test]
fn r5_b5_m2_b_any_error_repeated_at_one_start_is_deferred_after_a_bound() {
    // "treat persistence, not the message": a provider that keeps refusing one range with an error that names nothing
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        quiet(&mut f, &mut st);
    }
    f.odd_blocks = vec![top0 + 60];
    let new = newer_sales(&mut f, top0 + 100, 100, 10);
    let hs: Vec<[u8; 32]> = new.iter().map(|x| x.0).collect();
    let mut pin = ETH_PIN;
    for _ in 0..12 {
        pin += 100;
        f.pin_now = Some(pin);
        f.finalized = pin;
        quiet(&mut f, &mut st);
    }
    let sc = scan_of(&st);
    println!("B5-M-2 (persistence): {} of 10 newer sales read in 12 passes; up {} (the odd block {}); bad {:?}", cached(&st, &hs), sc.up, top0 + 60, sc.bad);
    assert!(sc.up > top0 + 60, "past it after at most {} failures in a row", en::SALE_SCAN_STUCK_FAILS);
    assert!(cached(&st, &hs) >= 6);
}

#[test]
fn r5_b5_l1_a_pass_never_reserves_more_than_its_bound() {
    // PoC `r5_b5_l1_…`: one pass reserved 19.71 MB (over the fix's own test bound), receipts uncounted
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let bound = en::SALE_SCAN_BYTES_PER_PASS;
    let burst: Vec<u64> = (0..12).map(|j| top0 + 1 + j).collect();
    let r = run(&burst, true, (top0 + 300, 100, 10), 72);
    show("(a) 12 consecutive oversized blocks, marketplace logs oversized too", burst.len(), &r);
    assert!(r.max_bytes <= bound, "a pass's log reads reserve at most {bound}: {}", r.max_bytes);
    assert!(r.read_end + 1 >= r.final_end);
    // (b) the reviewer's constructed pass
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 {
        quiet(&mut f, &mut st);
    }
    let d = 26_040_000u64;
    {
        let sc = st.sale_scan.as_mut().unwrap().iter_mut().find(|s| s.collection == MILADY).unwrap();
        sc.down = d;
        sc.bad = Some(vec![(d - 1, d - 1)]);
    }
    f.big_blocks = vec![top0 + 1, d - 1];
    f.big_sale_blocks = vec![d - 1];
    f.pin_now = Some(ETH_PIN + 100);
    f.finalized = ETH_PIN + 100;
    let (b0, r0) = (f.logs_bytes, f.receipt_bytes);
    quiet(&mut f, &mut st);
    let (logs, receipts) = (f.logs_bytes - b0, f.receipt_bytes - r0);
    println!("B5-L-1 fixed: (b) one pass reserved {logs} bytes of logs and {receipts} of receipts ({:.0} B cycles at 10,400 a byte)",
        (logs + receipts) as f64 * 10_400.0 / 1e9);
    assert!(logs <= bound && receipts <= en::SALE_RECEIPT_BYTES_PER_PASS);
    // (c) receipts: 40 transactions whose receipts are over the largest class: at most the receipts' bound a pass
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    for i in 0..40u64 {
        let mut h = [0x3c; 32];
        h[..8].copy_from_slice(&i.to_be_bytes());
        f.sale_receipts.insert(h, sale_receipt(MILADY, 60_000 + i as u128, 94 * E / 100, [0x3d; 20], [0x3e; 20], top0 - 50 - i));
        f.sale_err.push(h);
    }
    let mut most = 0;
    for _ in 0..6 {
        let r0 = f.receipt_bytes;
        quiet(&mut f, &mut st);
        most = most.max(f.receipt_bytes - r0);
    }
    assert!(most <= en::SALE_RECEIPT_BYTES_PER_PASS, "{most}");
}

#[test]
fn r5_b5_l1_the_day_has_a_bound_too() {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    quiet(&mut f, &mut st);
    let sc = st.sale_scan.as_mut().unwrap().iter_mut().find(|s| s.collection == MILADY).unwrap();
    sc.spent = Some((f.now, en::SALE_SCAN_BYTES_PER_DAY));
    let b0 = f.logs_bytes;
    let l = block_on(ef::tick(&mut f, &mut st, false)).join("\n");
    let _ = l;
    quiet(&mut f, &mut st);
    assert_eq!(f.logs_bytes, b0, "no log read once the day's bound is spent");
    f.now += 86_400;
    quiet(&mut f, &mut st);
    assert!(f.logs_bytes > b0, "the next day reads again");
}

#[test]
fn r5_b4_l2_a_rescan_hull_over_a_resolved_oversized_block_never_bisects_it_again() {
    // PoC `r5_b4_l2_a_…`: the hull re-read paid the whole bisection again (9 failed reads, 46 MB)
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    let blk = top0 - 3_000;
    f.big_blocks = vec![blk];
    f.sale_moves.insert(blk, vec![]);
    for _ in 0..30 {
        quiet(&mut f, &mut st);
    }
    let first = f.log_fails;
    assert!(scan_of(&st).done_big.is_some_and(|d| d.contains(&blk)), "remembered: {:?}", scan_of(&st));
    st.sale_scan.as_mut().unwrap().iter_mut().find(|s| s.collection == MILADY).unwrap().rescan = Some((blk - 4_000, blk + 2_000));
    let b0 = f.logs_bytes;
    for _ in 0..30 {
        quiet(&mut f, &mut st);
    }
    println!("B4-L-2 fixed: the first isolation cost {first} failed reads; the re-read of a hull over it {} more ({} bytes)",
        f.log_fails - first, f.logs_bytes - b0);
    assert_eq!(f.log_fails, first, "never read again");
}

// =====================================================================================================================================
// E5-I-2: every H3 ask site, read semantically: every file, every call, its key's binding in scope, comments out, the key functions pinned
// =====================================================================================================================================

/// The source with every `//` comment removed (a key kept only in a comment is not a key).
fn uncommented(src: &str) -> String {
    src.lines().map(|l| {
        let mut in_str = false;
        let b = l.as_bytes();
        let mut i = 0;
        while i + 1 < b.len() {
            match b[i] {
                b'"' if i == 0 || b[i - 1] != b'\\' => in_str = !in_str,
                b'/' if !in_str && b[i + 1] == b'/' => return l[..i].to_string(),
                _ => {}
            }
            i += 1;
        }
        l.to_string()
    }).collect::<Vec<_>>().join("\n")
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The top-level comma-separated parts of `s` (brackets and strings respected).
fn parts(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut in_str) = (Vec::new(), String::new(), 0i32, false);
    let mut prev = ' ';
    for ch in s.chars() {
        match ch {
            '"' if prev != '\\' => in_str = !in_str,
            '(' | '[' | '{' if !in_str => depth += 1,
            ')' | ']' | '}' if !in_str => depth -= 1,
            ',' if !in_str && depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
                prev = ch;
                continue;
            }
            _ => {}
        }
        cur.push(ch);
        prev = ch;
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// The text between the `(` at `open` and its matching `)`.
fn inside(s: &str, open: usize) -> &str {
    let (mut depth, mut in_str, mut prev) = (0i32, false, ' ');
    for (i, ch) in s[open..].char_indices() {
        match ch {
            '"' if prev != '\\' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return &s[open + 1..open + i];
                }
            }
            _ => {}
        }
        prev = ch;
    }
    &s[open + 1..]
}

/// What `name` is bound to by the nearest `let` before `at`: `let name = X;` or a tuple `let (.., name, ..) = (.., X, ..);`.
fn binding(src: &str, at: usize, name: &str) -> Option<String> {
    let before = &src[..at];
    let mut best: Option<(usize, String)> = None;
    for (i, _) in before.match_indices("let ") {
        let rest = &before[i + 4..];
        let Some(eq) = rest.find(" = ").filter(|e| !rest[..*e].contains('\n')) else { continue };
        let raw_pat = rest[..eq].trim().to_string();
        let pat = raw_pat.trim_start_matches("mut ").to_string();
        let rhs_start = i + 4 + eq + 3;
        let Some(end) = before[rhs_start..].find(";\n").map(|e| rhs_start + e) else { continue };
        let rhs = &before[rhs_start..end];
        let got = if pat == name {
            // fix round 6 (E6-I-1 (2)): a key bound `let mut` can be changed after its `let`: never a reviewed key
            Some(if raw_pat.starts_with("mut ") { format!("<let mut> {rhs}") } else { rhs.to_string() })
        } else if pat.starts_with('(') {
            let names = parts(pat.trim_start_matches('(').trim_end_matches(')'));
            names.iter().position(|n| n.trim_start_matches("mut ") == name).and_then(|k| {
                rhs.trim().strip_prefix('(').and_then(|r| r.strip_suffix(')')).map(parts).and_then(|v| v.get(k).cloned())
            })
        } else {
            None
        };
        if let Some(g) = got {
            best = Some((i, g));
        }
    }
    best.map(|(_, g)| norm(&g))
}

/// Every H3 ask in `files`, as `(file, the key it asks under, resolved to its binding)`, or why one cannot be read.
fn h3_asks(files: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    h3_sites(files, "outflow_check")
}

/// Fix round 6 (E6-I-1): every call of `call` (`outflow_check` or `outflow_record`) in `files`, its key resolved to its binding;
/// a key `mint_key(…, &col, …)` also has its `col` resolved (a per-deposit `col` is a per-deposit key); any other use of the name (a
/// path call `(I::outflow_check)(…)`, a `use … as` alias) is refused, as is a key bound `let mut`.
fn h3_sites(files: &[(String, String)], call: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (name, raw) in files {
        let src = uncommented(raw);
        for (at, _) in src.match_indices(call) {
            let prev = src[..at].chars().next_back().unwrap_or(' ');
            let next = src[at + call.len()..].chars().next().unwrap_or(' ');
            if prev.is_alphanumeric() || prev == '_' || next.is_alphanumeric() || next == '_' {
                continue; // a longer identifier
            }
            if next != '(' {
                return Err(format!("{name}: `{call}` used other than as a call (a path call, an alias): every H3 site must be read"));
            }
            let head = &src[at.saturating_sub(12)..at];
            // the trait's declarations and its adapters to the canister's one limiter are not asks
            if head.ends_with("fn ") || head.ends_with("pub(crate) fn ") {
                continue;
            }
            let args = parts(inside(&src, at + call.len()));
            let key = args.first().ok_or_else(|| format!("{name}: an H3 site with no key"))?;
            if ["key", "collection"].contains(&key.as_str()) && args.get(1).is_some_and(|c| c == "collection") && name != "canister.rs" {
                // `crate::canister::outflow_check(key, collection, …)`: an adapter passing its own parameters through
                let fnhead = src[..at].rfind(&format!("fn {call}(")).map(|f| &src[f..at]).unwrap_or("");
                if fnhead.contains("key: &str") {
                    continue;
                }
            }
            let key = key.trim_start_matches('&');
            let mut resolved = if key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                binding(&src, at, key).ok_or_else(|| format!("{name}: the key `{key}` of an H3 site has no `let` before it"))?
            } else {
                norm(key)
            };
            if resolved.starts_with("<let mut>") {
                return Err(format!("{name}: the key `{key}` of an H3 site is bound `let mut`: it can change after its `let`"));
            }
            if let Some(args) = resolved.strip_prefix("mint_key(").map(|r| parts(r.trim_end_matches(')'))) {
                let col = args.get(1).map(|c| c.trim_start_matches('&').to_string()).unwrap_or_default();
                let b = binding(&src, at, &col).ok_or_else(|| format!("{name}: mint_key's scope `{col}` has no `let` before it"))?;
                resolved = format!("{resolved} where {col} = {b}");
            }
            out.push((name.clone(), resolved));
        }
    }
    Ok(out)
}

const H3_ASKS: &[(&str, &str, &str)] = &[
    ("ethflow.rs", "format!(\"return:eth:{}:{}\", h(&d.tx), h(&d.token))", "a stray's return: one per claimed token; Return never trips"),
    ("ethflow.rs", "format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket)", "a delivery: one per ticket; bounded by custody plus the window's mints"),
    ("ethflow.rs", "mint_key(st, &col, &c.collection) where col = key_of(&c, chain)", "a purchase's mint: the collection's next mint (E4-H-1)"),
    ("ethflow.rs", "mint_key(st, &col, &d.collection) where col = key_of(&c, chain)", "a deposit's mint: the collection's next mint (E4-H-1)"),
    ("ethflow.rs", "format!(\"advance:eth:{}:{n}\", key_of(&c, chain))", "an advance: the collection and its count (C3-H-1)"),
    ("convflow.rs", "format!(\"buy:frog:{}\", c.frog)", "a frog's conversion advance: one per frog"),
    ("frogflow.rs", "format!(\"buy:frog:{id}\")", "a frog buy's value: one per frog"),
    ("frogflow.rs", "format!(\"return:{}\", d.outpoint.text())", "a refused Bitcoin deposit's return: Return never trips"),
    ("frogflow.rs", "format!(\"claim:{}\", w.id)", "a floor claim: one per want, after the item is admitted to custody"),
    ("frogflow.rs", "format!(\"mint:{}\", o.text())", "a Bitcoin deposit's mint: admitted to custody first, so bounded by its 3 items"),
    ("frogflow.rs", "format!(\"mint:frog:{id}\")", "a bought frog's mint: one per frog, in custody"),
    ("handinflow.rs", "format!(\"claim:handin:{}:{id}\", hex::encode(board))", "a hand-in's reward: one per hand-in"),
    ("nftnet.rs", "format!(\"fill:{}\", plan.order_hash)", "an operator's fill: one per order, asked at signing"),
    ("redeem.rs", "io.ticket_key(r.ticket)", "a Bitcoin delivery: one per ticket of its collection"),
    ("saleflow.rs", "format!(\"listing:{ticket}\")", "a sale's listing: one per ticket"),
    ("saleflow.rs", "format!(\"mint:sale:{}\", s.id)", "a sale's re-deposit mint: one per sale"),
    ("saleflow.rs", "format!(\"sale:{}\", s.id)", "a sale's proceeds: one per sale"),
    ("solnftflow/deliver.rs", "format!(\"solnft:{coll}:deliver:{ticket}\")", "a Solana NFT delivery: one per ticket (audit C1-02)"),
];

/// The two keys whose meaning is a count, pinned: the collection's next mint (E4-H-1) and the collection's next advance (C3-H-1).
const MINT_KEY_BODY: &str = "let buys = st.buys.iter().filter(|b| b.collection == *c && !b.is_sweep() && matches!(b.phase, BPhase::Minting { .. } | \
    BPhase::Minted { .. } | BPhase::Settling { .. } | BPhase::Settled { .. })).count(); let deposits = st.deposits.iter().filter(|d| d.collection \
    == *c && matches!(d.phase, XPhase::Minting | XPhase::Minted)).count(); format!(\"mint:eth:{col}:{}\", buys + deposits)";
const ADVANCE_N: &str = "st.buys.iter().filter(|x| x.collection == b.collection && !x.is_sweep() && x.committed()).count()";

/// Fix round 6 (E6-I-1 (3)): what may be RECORDED under a key besides the reviewed asks' keys (each records what its own ask let out).
const H3_RECORD_ONLY: &[(&str, &str, &str)] = &[
    ("ethflow.rs", "format!(\"advance:eth:{}:{n}\", key_of(c, chain))", "an adopted advance, under the advance's own key (F5-M-1)"),
];

fn h3_check(files: &[(String, String)]) -> Result<(), String> {
    let asks = h3_asks(files)?;
    let mut left: Vec<(&str, &str)> = H3_ASKS.iter().map(|(f, k, _)| (*f, *k)).collect();
    for (f, k) in &asks {
        match left.iter().position(|(lf, lk)| lf == f && norm(lk) == *k) {
            Some(i) => {
                left.remove(i);
            }
            None => return Err(format!("{f}: an H3 ask under `{k}` is not reviewed: list it with why it is one ask per logical outflow")),
        }
    }
    if !left.is_empty() {
        return Err(format!("reviewed asks no longer in the source: {left:?}"));
    }
    // fix round 6 (E6-I-1 (3)): every record is under a reviewed ask's key of its file (the record side's keying is H3's too: a record
    // under one constant key counts only the window's first outflow)
    for (f, k) in h3_sites(files, "outflow_record")? {
        let ok = H3_ASKS.iter().chain(H3_RECORD_ONLY).any(|(lf, lk, _)| *lf == f && norm(lk) == k);
        if !ok {
            return Err(format!("{f}: an H3 record under `{k}` is not a reviewed key"));
        }
    }
    let ethflow = files.iter().find(|(f, _)| f == "ethflow.rs").map(|(_, s)| uncommented(s)).ok_or("no ethflow.rs")?;
    let mk = ethflow.find("fn mint_key(").ok_or("no mint_key")?;
    let body_start = mk + ethflow[mk..].find('{').ok_or("mint_key")? + 1;
    let body_end = body_start + ethflow[body_start..].find("\n}").ok_or("mint_key")?;
    if norm(&ethflow[body_start..body_end]) != norm(MINT_KEY_BODY) {
        return Err(format!("mint_key's body changed: {}", norm(&ethflow[body_start..body_end])));
    }
    let adv = ethflow.find("format!(\"advance:eth:{}:{n}\", key_of(&c, chain))").ok_or("no advance key")?;
    if binding(&ethflow, adv, "n").as_deref() != Some(ADVANCE_N) {
        return Err(format!("the advance key's count changed: {:?}", binding(&ethflow, adv, "n")));
    }
    Ok(())
}

/// Every `.rs` file under `src/`, sub-directories included (fix round 6, E6-I-1 (5)), named by its path under `src/`.
fn src_files() -> Vec<(String, String)> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, v: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, v);
            } else if p.extension().is_some_and(|x| x == "rs") {
                v.push((p.strip_prefix(root).unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let mut v = Vec::new();
    walk(&root, &root, &mut v);
    v.sort();
    v
}

#[test]
fn r5_e5_i2_every_h3_ask_is_keyed_per_logical_outflow_read_semantically() {
    let files = src_files();
    assert!(files.len() > 40, "every file under src/ is read");
    h3_check(&files).unwrap();
    let with = |f: &str, from: &str, to: &str| -> Vec<(String, String)> {
        let mut v = files.clone();
        let x = v.iter_mut().find(|(n, _)| n == f).unwrap();
        assert!(x.1.contains(from), "{from}");
        x.1 = x.1.replacen(from, to, 1);
        v
    };
    // (a) mint_key made per-backlog again: caught
    let a = with("ethflow.rs", "format!(\"mint:eth:{col}:{}\", buys + deposits)",
        "format!(\"mint:eth:{col}:{}:{}\", buys + deposits, st.deposits.iter().filter(|d| d.phase == XPhase::Mint).count())");
    assert!(h3_check(&a).unwrap_err().contains("mint_key"), "(a)");
    // (b) the listed key kept only in a comment, the call site asking per deposit: caught
    let b = with("ethflow.rs", "            let key = mint_key(st, &col, &d.collection);\n",
        "            // let key = mint_key(st, &col, &d.collection);\n            let key = format!(\"mint:deposit:{}\", h(&d.tx));\n");
    assert!(h3_check(&b).unwrap_err().contains("not reviewed"), "(b)");
    // (c) a new site in a file the round-4 test did not read: caught
    let mut c = files.clone();
    c.iter_mut().find(|(n, _)| n == "looper.rs").unwrap().1.push_str(
        "\nfn ask(k: &str) -> Result<(), String> { crate::canister::outflow_check(&format!(\"loop:{k}\"), \"c\", crate::outflow::Flow::Item, 1) }\n");
    assert!(h3_check(&c).unwrap_err().contains("looper.rs"), "(c)");
    // (d) a new site without `&`: caught
    let mut d = files.clone();
    d.iter_mut().find(|(n, _)| n == "ethflow.rs").unwrap().1.push_str(
        "\nasync fn ask_again<I: EthIo>(io: &mut I, key: String) -> Result<(), String> { let k2 = format!(\"x:{key}\"); io.outflow_check(k2.as_str(), \"c\", Flow::Mint, 1) }\n");
    assert!(h3_check(&d).is_err(), "(d)");
    // (e) the advance's count keyed by the buy again (C3-H-1's shape): caught
    let e = with("ethflow.rs", ADVANCE_N, "st.buys.iter().filter(|x| x.id == b.id).count()");
    assert!(h3_check(&e).unwrap_err().contains("count"), "(e)");
    // fix round 6 (E6-I-1): review6/poc/E's five mutations, each passed the round-5 guard; each is caught now
    // (1) the deposit mint's `col` made per deposit (mint_key's text unchanged)
    let m1 = with("ethflow.rs", "            let col = key_of(&c, chain);\n            let key = mint_key(st, &col, &d.collection);",
        "            let col = format!(\"{}:{}\", key_of(&c, chain), h(&d.tx));\n            let key = mint_key(st, &col, &d.collection);");
    assert!(h3_check(&m1).unwrap_err().contains("not reviewed"), "(1)");
    // (2) the delivery key changed after a `let mut`
    let m2 = with("ethflow.rs", "            let key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);\n",
        "            let mut key = format!(\"deliver:{}:{}\", h(&d.wrapper), d.ticket);\n            key.push_str(&format!(\":{}\", io.now()));\n");
    assert!(h3_check(&m2).unwrap_err().contains("let mut"), "(2)");
    // (3) the record side under one constant key
    let m3 = with("ethflow.rs", "            io.outflow_record(&key, &col, Flow::Item, 1);\n            let nonce = st.line",
        "            io.outflow_record(\"deliver\", &col, Flow::Item, 1);\n            let nonce = st.line");
    assert!(h3_check(&m3).unwrap_err().contains("record"), "(3)");
    // (4) a new ask through a path call
    let mut m4 = files.clone();
    m4.iter_mut().find(|(n, _)| n == "ethflow.rs").unwrap().1.push_str(
        "\nasync fn ask_again<I: EthIo>(io: &mut I) -> Result<(), String> { let k = format!(\"x:{}\", io.now()); (I::outflow_check)(io, &k, \"c\", Flow::Mint, 1) }\n");
    assert!(h3_check(&m4).unwrap_err().contains("other than as a call"), "(4)");
    // ... and an alias
    let mut m4b = files.clone();
    m4b.iter_mut().find(|(n, _)| n == "looper.rs").unwrap().1.push_str("\nuse crate::canister::outflow_check as ask;\n");
    assert!(h3_check(&m4b).is_err(), "(4b)");
    // (5) a new site in a file under a sub-directory of src/ (read in memory: nothing is written to the tree)
    let mut m5 = files.clone();
    m5.push(("r6extra/mod.rs".into(), "pub fn ask(n: u64) -> Result<(), String> { crate::canister::outflow_check(&format!(\"loop:{n}\"), \"c\", crate::outflow::Flow::Item, 1) }\n".into()));
    assert!(h3_check(&m5).unwrap_err().contains("r6extra/mod.rs"), "(5)");
}

// =====================================================================================================================================
// E5-L-1: a deposit whose mint went out but was read as not minted no longer wedges its collection
// =====================================================================================================================================

fn milady_deposits(f: &mut Fake, st: &mut EthState, n: u8, first: u8) -> Vec<[u8; 32]> {
    let mut v = Vec::new();
    for k in 0..n {
        let tx = [first + k; 32];
        deposit_tx(f, tx, MILADY, WRAPPER, 5_000 + (first + k) as u128, RECIPIENT, ETH_PIN - 20 - k as u64);
        ef::put_deposit(st, &f.cfg(), tx, f.now).unwrap();
        v.push(tx);
    }
    v
}

fn step(f: &mut Fake, st: &mut EthState, secs: u64) -> Vec<String> {
    f.now += secs;
    fresh_modules(f);
    evidence(f, st);
    tick(f, st)
}

#[test]
fn r5_e5_l1_a_mint_misread_as_not_minted_no_longer_wedges_its_collection() {
    // PoC `e5_l1_a_…`: a mintDeposit that mined, seen nowhere for an hour, its one itemOf read at the drop failing: back at Mint, its
    // estimate reverting AlreadyHeld every pass, and (round 4's `waited`) every later deposit of the collection held for good
    let mut f = Fake::new(true);
    launch_limits(&mut f);
    f.deposits_on = true;
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    f.m(MODULE2).accepted = 67 * E / 100;
    let mut az_sales = Vec::new();
    for i in 0..5u8 {
        let h = [0xa0 + i; 32];
        f.sale_receipts.insert(h, sale_receipt(AZUKI, 100 + i as u128, 67 * E / 100, [0x10 + i; 20], [0x50 + i; 20], ETH_PIN - 500));
        az_sales.push(h);
    }
    let mut st = EthState::default();
    let txs = milady_deposits(&mut f, &mut st, 2, 0x90);
    let mut hidden = false;
    for _ in 0..30 {
        step(&mut f, &mut st, 600);
        azuki_evidence(&f, &mut st, &az_sales);
        if !hidden {
            if let Some(p) = st.pending.clone() {
                if matches!(p.step, ef::Step::MintDeposit { tx } if tx == txs[0]) {
                    f.hide.push(p.hash);
                    hidden = true;
                    step(&mut f, &mut st, 600);
                    f.now += ef::HOME_NONCE_TAKEN_SECS;
                    f.item_of_err = 1; // the one itemOf read fails: now a wait, never "not minted"
                    step(&mut f, &mut st, 60);
                }
            }
        }
    }
    assert!(hidden);
    for _ in 0..(3 * 24) {
        step(&mut f, &mut st, 600);
    }
    let a = st.deposits.iter().find(|d| d.tx == [0x90; 32]).unwrap().phase.clone();
    let b = st.deposits.iter().find(|d| d.tx == [0x91; 32]).unwrap().phase.clone();
    assert_eq!(a, ef::XPhase::Minted, "A is minted (its effect was read back)");
    assert_eq!(b, ef::XPhase::Minted, "B minted too");
    assert!(f.items.contains(&tok(5_000 + 0x91)) && f.items.iter().filter(|t| **t == tok(5_000 + 0x90)).count() == 1, "never minted twice");
}

#[test]
fn r5_e5_l1_b_a_deposit_already_on_the_wrapper_is_adopted_and_the_next_deposit_still_mints() {
    // the state an earlier build could leave: A back at Mint while its token is on the wrapper from its own transaction; A is adopted as
    // minted when its mint's estimate refuses, and B is not held behind it
    let mut f = Fake::new(false);
    launch_limits(&mut f);
    f.deposits_on = true;
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    let mut st = EthState::default();
    // two final deposits at Mint; A's mint went out and mined (the wrapper holds its token from A's own transaction), but its record
    // says Mint
    for (k, t) in [(0x90u8, 5_000 + 0x90u128), (0x91, 5_000 + 0x91)] {
        deposit_tx(&mut f, [k; 32], MILADY, WRAPPER, t, RECIPIENT, ETH_PIN - 20);
        st.deposits.push(ef::DepositRec { tx: [k; 32], collection: MILADY, token: tok(t), recipient: RECIPIENT, from: HOLDER, block: ETH_PIN - 20,
            phase: ef::XPhase::Mint, refused_since: None });
    }
    f.items.push(tok(5_000 + 0x90));
    f.item_src.insert((WRAPPER, 0), ([0x90; 32], true));
    for _ in 0..(2 * 24) {
        step(&mut f, &mut st, 600);
    }
    let phases: Vec<ef::XPhase> = st.deposits.iter().map(|d| d.phase.clone()).collect();
    assert!(phases.iter().all(|p| *p == ef::XPhase::Minted), "{phases:?}");
    assert_eq!(f.items.iter().filter(|t| **t == tok(5_000 + 0x90)).count(), 1, "A never minted twice");
    assert!(f.items.contains(&tok(5_000 + 0x91)), "B minted");
}

// =====================================================================================================================================
// F5-L-1: the Ethereum floors are the configured chain's
// =====================================================================================================================================

#[test]
fn r5_f5_l1_a_chain_move_starts_the_ethereum_floors_afresh() {
    // PoC `f5_l1_…`: the one Ethereum floor survived a move of eth_nft to another chain and refused its pins or nonces for good
    let mut fl = etch_autobuyer::pin::PinFloors::default();
    fl.eth.admit_pin("Ethereum", ETH_PIN).unwrap();
    fl.eth.admit_nonce("Ethereum", 40).unwrap();
    fl.home.admit_pin("4663", HOME_PIN).unwrap();
    let mut st = EthState::default();
    st.last_mined = Some((39, ETH_PIN - 5));
    assert!(!ef::chain_moved(&mut st, &mut fl, Some(1), Some(1)), "the same chain keeps them");
    assert_eq!(fl.eth.pin, ETH_PIN);
    assert!(ef::chain_moved(&mut st, &mut fl, Some(1), Some(8453)));
    fl.eth.admit_pin("Ethereum", 9_300_000).unwrap();
    fl.eth.admit_nonce("Ethereum", 0).unwrap();
    assert!(st.last_mined.is_none() && fl.home.pin == HOME_PIN, "the home chain's floor is untouched");
    let src = include_str!("../src/canister.rs");
    let post = src.split("fn post_upgrade(").nth(1).unwrap().split("\n}\n").next().unwrap();
    assert!(post.contains("crate::ethflow::chain_moved(&mut eth, &mut floors, from, arg.eth_nft.as_ref().map(|e| e.chain_id))"));
}

// =====================================================================================================================================
// A5-L-2: backlog-first delivery drains every collection within the exit rule's two usable days
// =====================================================================================================================================

/// Deliveries of five collections (`held`), their tickets resolved in `order` (the earlier, the nearer the deadline), one signed a
/// 10-minute tick (the first the real H3 limiter allows, in `pick`'s order), for `hours`: what is left per collection.
fn drain(held: &[u32], order: &[usize], l: &etch_autobuyer::outflow::Limits, hours: u64, pick: impl Fn(&[ef::Delivery], u64) -> Vec<usize>) -> Vec<u32> {
    use etch_autobuyer::outflow::{Outflow, Verdict};
    let cols: Vec<[u8; 20]> = (0..held.len()).map(|k| [0xc0 + k as u8; 20]).collect();
    let mut ds = Vec::new();
    let mut deadline = NOW + 7 * 86_400;
    for &k in order {
        for n in 0..held[k] {
            deadline += 1;
            ds.push(ef::Delivery { wrapper: cols[k], collection: cols[k], ticket: n as u128, token: tok(n as u128), destination: ALICE,
                deliver_by: deadline, phase: ef::DPhase::Waiting, note: String::new(), refused_since: None, sent: None });
        }
    }
    let mut o = Outflow::default();
    let mut now = NOW;
    while now < NOW + hours * 3_600 {
        for i in pick(&ds, now) {
            let d = &ds[i];
            let (key, col) = (format!("deliver:{}:{}", hx(&d.wrapper), d.ticket), custodycap::evm_key(1, &d.collection));
            if o.check(l, &key, &col, Flow::Item, 1, now) == Verdict::Allow {
                o.record(&key, &col, Flow::Item, 1, now);
                ds[i].phase = ef::DPhase::Moved { tx: [0; 32], block: 0 };
                break;
            }
        }
        now += 600;
    }
    cols.iter().map(|c| ds.iter().filter(|d| d.collection == *c && d.phase == ef::DPhase::Waiting).count() as u32).collect()
}

#[test]
fn r5_a5_l2_backlog_first_delivery_leaves_nothing_for_the_margin_day() {
    use etch_autobuyer::outflow::{exit_fits, Limits};
    let lim = |per: u32, glob: u32| Limits { per_collection_items: per, global_items: glob, per_collection_mints: 3, global_mints: 6,
        global_value_wei: 25 * E, trip_factor: 3 };
    // Milady, Azuki, MAYC, Pudgy, BAYC; Azuki's tickets resolved last (the reviewer's worst order)
    let worst = [0usize, 2, 3, 4, 1];
    let deadline_first = |ds: &[ef::Delivery], _now: u64| {
        let mut w: Vec<usize> = (0..ds.len()).filter(|&i| ds[i].phase == ef::DPhase::Waiting).collect();
        w.sort_by_key(|&i| ds[i].deliver_by);
        w
    };
    for (name, held, l, was) in [("(a) the launch split 13/17/13/3/2 @ 12/24", [13u32, 17, 13, 3, 2], lim(12, 24), 5),
        ("(c) step 2: 13/17/13/3/2 @ 12/31", [13, 17, 13, 3, 2], lim(12, 31), 3),
        ("(c) step 3: J7's 16/22/17/4/2 @ 12/31", [16, 22, 17, 4, 2], lim(12, 31), 9)] {
        let total: u32 = held.iter().sum();
        assert!(held.iter().all(|&n| exit_fits(n, l.per_collection_items, 72 * 3_600)) && exit_fits(total, l.global_items, 72 * 3_600), "{name}");
        let before: u32 = drain(&held, &worst, &l, 48, deadline_first).iter().sum();
        let now: u32 = drain(&held, &worst, &l, 48, |ds, t| ef::delivery_order(ds, t)).iter().sum();
        println!("A5-L-2 {name}: deadline-first left {before} for the margin day (review 5: {was}); backlog-first leaves {now}");
        assert_eq!(before, was, "the sim reproduces the reviewer's count");
        assert_eq!(now, 0, "{name}: every item out within the two usable days");
    }
    // a delivery within two days of its deadline still goes before any backlog
    let mut ds: Vec<ef::Delivery> = (0..3u128).map(|n| ef::Delivery { wrapper: [0xc1; 20], collection: [0xc1; 20], ticket: n, token: tok(n),
        destination: ALICE, deliver_by: NOW + 7 * 86_400, phase: ef::DPhase::Waiting, note: String::new(), refused_since: None, sent: None }).collect();
    ds.push(ef::Delivery { collection: [0xc2; 20], wrapper: [0xc2; 20], deliver_by: NOW + 86_400, ..ds[0].clone() });
    assert_eq!(ef::delivery_order(&ds, NOW)[0], 3);
}

// =====================================================================================================================================
// Upgrade safety: the state the round-4 build wrote restores with nothing lost; this round's fixture holds every new field
// =====================================================================================================================================

#[test]
fn r5_the_state_the_round_4_build_wrote_restores_with_nothing_lost_and_the_new_fields_absent() {
    use etch_autobuyer::canister::lost_opts;
    // tests/fixtures/state/eth-state-1dc2a61.hex: the bytes 1dc2a61's own `r4_the_round_4_state_fixture_is_what_the_canister_encodes`
    // held equal to what that build encodes (every round-4 field present)
    let bytes = hex::decode(include_str!("fixtures/state/eth-state-1dc2a61.hex").trim()).unwrap();
    let e: EthState = candid::decode_one(&bytes).unwrap();
    let again = candid::encode_one(&e).unwrap();
    assert_eq!(lost_opts(&bytes, &again).unwrap(), Vec::<String>::new(), "an honest upgrade would be refused");
    assert!(e.buys.iter().all(|b| b.late.as_ref().is_none_or(|l| l.released_at.is_none() && l.watch.request_id.is_none())));
    assert!(e.buys.iter().all(|b| b.last.as_ref().is_none_or(|w| w.request_id.is_none() && w.home_pin.is_none())));
    assert!(e.pending.as_ref().is_some_and(|p| p.pin.is_none() && p.used_since == Some(6)));
    assert!(e.sale_scan.iter().flatten().all(|s| s.done_big.is_none() && s.stuck.is_none() && s.spent.is_none() && s.bad.is_some()));
    assert!(e.inbox.home_clear.is_none());
    assert!(e.buys.iter().any(|b| b.late.as_ref().is_some_and(|l| l.credited.is_some())) && e.credited.is_some(), "round 4's own fields kept");
}

/// The state this round writes, every one of its new fields present, and `Inbox.payouts` too (E5-I-3): the services decode it
/// (`tests/fixtures/eth-state-round5.hex`, read by the monitor's and the runner's tests).
fn round5_state() -> EthState {
    let w = ef::Watch { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, balance_after: 1, eth_before: 2, eth_out_before: 3, pin: Some(4),
        solver: Some([0xf7; 20]), request_id: Some([0x17; 32]), home_pin: Some(70_166_016) };
    let b = |id: u64, phase: BPhase| ef::EthBuy { id, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: 1, attempts: 1, phase,
        orders: vec![], deposits: vec![], ask: None, last: Some(w.clone()), note: String::new(), sweep: Some(false), arrived_at: Some(3),
        back_at: Some(4), ended_at: None, late: None };
    let mut st = EthState::default();
    let mut held = b(1, BPhase::Stuck { why: "written off".into() });
    held.last = None;
    held.late = Some(ef::LateOrder { watch: w.clone(), back: true, let_go_at: 5, credited: None, released_at: None });
    let mut released = b(2, BPhase::Returned { wei: 0 });
    released.last = None;
    released.late = Some(ef::LateOrder { watch: w.clone(), back: false, let_go_at: 5, credited: None, released_at: Some(6) });
    st.buys = vec![b(0, BPhase::Bridging { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, since: 2 }), held, released];
    st.pending = Some(ef::HomeTx { step: ef::Step::Deposit { buy: 0 }, raw: vec![1], hash: [4; 32], nonce: 7, used_since: Some(6), pin: Some(70_166_000), sent_at: None });
    st.sale_scan = Some(vec![ef::SaleScan { collection: MILADY, down: 1, up: 9, chunk: 500, todo: vec![], pend: Some(vec![]), rescan: None,
        judged: Some(vec![]), bad: Some(vec![(4, 5)]), skipped: Some(vec![(6, 7, 1)]), done_big: Some(vec![8]), stuck: Some((2, 3)),
        spent: Some((1, 2)), hole: None, lost: None, behind: None, rfail: None, raw: None, current: None, hole_deep: None, rbusy: None, busy: None }]);
    st.inbox.payouts = Some(vec![ef::PayoutClaim { buy: 1, home: true, tx: [9; 32], wei: 0, at: 5 }]);
    st.inbox.home_clear = Some((7, true, 8));
    st.credited = Some(vec![]);
    st
}

#[test]
fn r5_the_round_5_state_fixture_is_what_the_canister_encodes() {
    let st = round5_state();
    let bytes = candid::encode_one(&st).unwrap();
    assert_eq!(candid::decode_one::<EthState>(&bytes).unwrap(), st);
    let fixture = include_str!("fixtures/eth-state-round5.hex").trim();
    assert_eq!(hex::encode(&bytes), fixture, "tests/fixtures/eth-state-round5.hex is stale: write the new bytes there (and run the monitor's and ops' tests)");
}
