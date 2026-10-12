//! Fix round 6 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round6.md): the canister PoCs of the round-6 re-review
//! (docs/audit/2026-09-27-nft/review6/poc/B, C and F) turned around. The harness is `tests/audit_nft_r5.rs`'s fake world, copied
//! whole up to its first test, with the reviewers' switches (part B: `log_depth` / `depth_raw`, `receipt_disagree`, `busy_pct`,
//! `seaport_range_cap`; part F: `live_pin`, `state_depth`, `mint_estimate_refuse`) and what this round reads (`depth_plain`: a
//! refusal whose words name nothing; `partial` and `eth_sale_moves_whole`: a marketplace read answered for one marketplace only). A
//! provider's error reaches the fake's callers as the canister sees it: through the real log transform (`seen_as`). Each test FAILS
//! on the code before this round (its PoC passed there) and passes now.


#![allow(dead_code)] // the harness is tests/audit_nft_r5.rs's, copied whole

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
    // ---- review round 6, part B (poc/B) ----
    /// logs older than this many blocks under the pin are refused (publicnode's keyless endpoint: about 10,000)
    log_depth: Option<u64>,
    /// the refusal as the provider says it (true), or as the canister sees it after its transform (false: the message is dropped)
    depth_raw: bool,
    /// these receipts' reads fail with a disagreement (one provider answers null: publicnode prunes receipts ~20-30k blocks deep)
    receipt_disagree: Vec<[u8; 32]>,
    /// percent of log reads (each marketplace query on its own) answered -32005 "anonymous capacity temporarily busy" (xrpc.cl, measured)
    busy_pct: u64,
    busy_seed: u64,
    busy_hits: u32,
    /// a marketplace range read wider than this: Seaport's half is over 2 MB (every collection's OrderFulfilled: 1.9 MB per 500 blocks,
    /// 9.9 MB per 2,000 measured), so EthNet returns Blur's half alone (Ok)
    seaport_range_cap: Option<u64>,
    /// fix round 6: a refusal for depth whose words name no class (only the code reaches the canister)
    depth_plain: bool,
    /// fix round 6 (B6-L-2): the last marketplace read answered for one marketplace only (`EthNet::eth_sale_moves_whole`)
    partial: bool,
    /// fix round 6 (C6-L-1): a payout transaction's receipt logs (Relay's own execution record, or none)
    payout_logs: HashMap<(bool, [u8; 32]), Vec<LogV>>,
    // ---- review round 6, part F (review6/poc/F) ----
    /// F6: the 4663 pin advances with the clock at 10 blocks a second (measured: 1,000 blocks = 100 s) from this time
    live_pin: Option<u64>,
    /// F6: the providers keep historical STATE this many blocks back (rpc.mainnet.chain.robinhood.com: ~6,200); `home_nonce_tx` needs
    /// our nonce count at `from` (HomeTx.pin), so it fails once the pin is further than this past it
    state_depth: Option<u64>,
    /// F6: every wrapper mintDeposit's estimate reverts (a paused listing: `ListingPaused`; or `MintRateLimited`)
    mint_estimate_refuse: bool,
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

/// What the canister sees of a provider's error body: through its log transforms (`eth_transfer_logs`), then `rpc_once_with`'s words.
fn seen_as(raw: &str) -> String {
    let v: serde_json::Value = serde_json::from_slice(&en::transfer_logs_body(raw.as_bytes())).unwrap();
    format!("chain 1: eth_getLogs: no result: {}", v.get("error").map(|e| e.to_string()).unwrap_or_default())
}
/// xrpc.cl's anonymous rate limit (HTTP 429, measured), as the canister sees it
fn busy_t() -> String {
    seen_as(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"XRPC anonymous capacity temporarily busy"}}"#)
}
impl Fake {
    fn busy(&mut self) -> bool {
        if self.busy_pct == 0 { return false; }
        self.busy_seed = self.busy_seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let b = (self.busy_seed >> 33) % 100 < self.busy_pct;
        if b { self.busy_hits += 1; }
        b
    }
    fn too_deep(&mut self, from: u64) -> Option<String> {
        let pin = self.pin_now.unwrap_or(ETH_PIN);
        let d = self.log_depth?;
        if from + d >= pin { return None; }
        self.log_fails += 1;
        self.logs_bytes += 2 * 64_000;
        let raw = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Archive requests require a personal token. Get one at: https://www.allnodes.com/publicnode"}}"#;
        Some(if self.depth_raw {
            format!("chain 1: eth_getLogs: no result: {raw}")
        } else if self.depth_plain {
            // fix round 6: a provider whose refusal names nothing the canister knows (its words give no class): only the code survives
            "chain 1: eth_getLogs: no result: {\"code\":-32602,\"reverted\":false}".to_string()
        } else {
            // what `rpc_once_with` makes of the transform's output: the code and (fix round 6) the words' class
            seen_as(raw)
        })
    }
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
            log_depth: None, depth_raw: false, receipt_disagree: vec![], busy_pct: 0, busy_seed: 0x1234_5678, busy_hits: 0, seaport_range_cap: None, depth_plain: false, partial: false, payout_logs: HashMap::new(),
            live_pin: None, state_depth: None, mint_estimate_refuse: false,
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
        if self.mint_estimate_refuse && (to == WRAPPER || to == WRAPPER2) && data.len() >= 4 && sel(data) == en::MINT_DEPOSIT {
            return Err("would not succeed now: execution reverted: ListingPaused()".into());
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
        if let Some(t0) = self.live_pin {
            return Ok(Some(HOME_PIN + self.now.saturating_sub(t0) * 10));
        }
        Ok(self.home_pin_known.then(|| HOME_PIN - self.home_back.map_or(0, |_| 1_024)))
    }
    async fn eth_sale_moves(&mut self, _collection: [u8; 20], block: u64) -> Result<Vec<en::Move>, String> {
        if let Some(e) = self.too_deep(block) { return Err(e); }
        if self.busy_pct > 0 {
            let (s, b) = (self.busy(), self.busy());
            self.logs_bytes += 2 * 64_000;
            if s && b { self.log_fails += 1; return Err(busy_t()); }
            if s { self.partial = true; return Ok(vec![]); } // Seaport's half busy, Blur's read: EthNet answers Blur's (none), not whole
        }
        if self.big_sale_blocks.contains(&block) {
            self.log_fails += 1;
            self.logs_bytes += 2 * (64_000 + 400_000 + 2_000_000);
            return Err(format!("chain 1: eth_getLogs: {}Http body exceeds size limit of 2000000 bytes", etch_autobuyer::evmread::SIZE_REJECT));
        }
        self.logs_bytes += 2 * 64_000;
        self.sale_moves.get(&block).cloned().ok_or_else(|| "chain 1: eth_getLogs: IC0406: SysTransient: Timeout expired".to_string())
    }
    /// fix round 6 (B6-L-2): as EthNet, whether both marketplaces answered
    async fn eth_sale_moves_whole(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<(Vec<en::Move>, bool), String> {
        self.partial = false;
        let v = if from == to { self.eth_sale_moves(collection, from).await? } else { self.eth_sale_moves_range(collection, from, to).await? };
        Ok((v, !self.partial))
    }
    fn logs_bytes(&self) -> u64 { self.logs_bytes }
    fn receipt_bytes(&self) -> u64 { self.receipt_bytes }
    async fn home_nonce_tx(&mut self, nonce: u64, _from: Option<u64>) -> Result<Option<[u8; 32]>, String> {
        if let (Some(d), Some(t0)) = (self.state_depth, self.live_pin) {
            let pin = HOME_PIN + self.now.saturating_sub(t0) * 10;
            let lo = _from.unwrap_or(pin.saturating_sub(ef::HOME_NONCE_SEARCH_BLOCKS));
            if pin.saturating_sub(lo) > d {
                return Err(format!("chain 4663: eth_getTransactionCount at block {lo}: historical state not available ({} blocks back, the provider keeps {d})", pin - lo));
            }
        }
        if !self.identify {
            return Err("chain 4663: eth_getTransactionCount: missing trie node (the provider keeps no state that deep)".into());
        }
        if self.home_nonce <= nonce {
            return Ok(None);
        }
        Ok(self.home_sent.iter().find(|(_, (_, raw))| u64::from_be_bytes(raw[36..44].try_into().unwrap()) == nonce).map(|(h, _)| *h))
    }
    async fn eth_sale_moves_range(&mut self, _collection: [u8; 20], from: u64, to: u64) -> Result<Vec<en::Move>, String> {
        if let Some(e) = self.too_deep(from) { return Err(e); }
        let mut seaport_out = false;
        if self.busy_pct > 0 {
            let (s, b) = (self.busy(), self.busy());
            if s && b { self.log_fails += 1; self.logs_bytes += 2 * 2 * 64_000; return Err(busy_t()); }
            seaport_out = s;
        }
        if self.seaport_range_cap.is_some_and(|c| to - from + 1 > c) {
            self.logs_bytes += 2 * (64_000 + 400_000 + 2_000_000);
            seaport_out = true;
        }
        if seaport_out { self.logs_bytes += 2 * 64_000; self.partial = true; return Ok(vec![]); }
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
    async fn payout_logs(&mut self, home: bool, hash: [u8; 32]) -> Result<Vec<LogV>, String> {
        Ok(self.payout_logs.get(&(home, hash)).cloned().unwrap_or_default())
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
        if self.receipt_disagree.contains(&h) {
            return Err("chain 1: providers disagree on eth_getTransactionReceipt; nothing is believed".into());
        }
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
        if let Some(e) = self.too_deep(from) { return Err(e); }
        if self.busy() { self.log_fails += 1; self.logs_bytes += 2 * 64_000; return Err(busy_t()); }
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

// =====================================================================================================================================
// Part B (review6/poc/B): the sale scan's provider failures
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
fn newer_sales(f: &mut Fake, first: u64, every: u64, n: u64, tag: u8) -> Vec<([u8; 32], u64)> {
    let mut v = vec![];
    for i in 0..n {
        let mut h = [tag; 32];
        h[..8].copy_from_slice(&(i + 1).to_be_bytes());
        let s = { let mut a = [0x71u8; 20]; a[0] = tag; a[12..].copy_from_slice(&i.to_be_bytes()); a };
        let b = { let mut a = [0x72u8; 20]; a[0] = tag; a[12..].copy_from_slice(&i.to_be_bytes()); a };
        let blk = first + every * i;
        f.sale_receipts.insert(h, sale_receipt(MILADY, (tag as u128) * 1_000_000 + i as u128, 94 * E / 100, s, b, blk));
        v.push((h, blk));
    }
    v
}

struct Gap {
    first: Option<usize>,
    day: usize,
    week: usize,
    fin: usize,
    lag_day: u64,
    fails: u32,
    scan: ef::SaleScan,
    lines: Vec<String>,
}

/// The scan after a gap of `gap` blocks with no pass (the runner down, the cycles under the floor, ...): the window read at ETH_PIN,
/// then the gap, then `passes` passes 100 blocks apart; honest sales one every 100 blocks after the gap.
fn after_gap(depth: Option<u64>, raw: bool, plain: bool, gap: u64, passes: usize) -> Gap {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.log_depth = depth;
    f.depth_raw = raw;
    f.depth_plain = plain;
    f.finalized = ETH_PIN;
    f.pin_now = Some(ETH_PIN);
    f.m(MODULE).accepted = 93 * E / 100;
    for _ in 0..4 { quiet(&mut f, &mut st); }
    let mut pin = ETH_PIN + gap;
    f.now += gap * 12;
    let top_gap = pin - en::SALE_FINAL_DEPTH_BLOCKS;
    let new = newer_sales(&mut f, top_gap + 50, 100, passes as u64, 0x5e);
    let hs: Vec<[u8; 32]> = new.iter().map(|x| x.0).collect();
    let (mut first, mut day, mut lag_day, mut lines) = (None, 0, 0, vec![]);
    for i in 0..passes {
        pin += 100;
        f.pin_now = Some(pin);
        f.finalized = pin;
        lines.extend(quiet(&mut f, &mut st));
        if first.is_none() && cached(&st, &hs) > 0 { first = Some(i + 1); }
        if i + 1 == 72 { day = cached(&st, &hs); lag_day = (pin - en::SALE_FINAL_DEPTH_BLOCKS).saturating_sub(scan_of(&st).up); }
    }
    let top = pin - en::SALE_FINAL_DEPTH_BLOCKS;
    Gap { first, day, week: cached(&st, &hs), fin: new.iter().filter(|x| x.1 <= top).count(), lag_day, fails: f.log_fails, scan: scan_of(&st), lines }
}

#[test]
fn r6_b6_m1_a_gap_past_the_providers_log_depth_no_longer_freezes_the_forward_walk() {
    // PoC `r6_b_m1_a_…`: after a 12,000-block gap with a provider keeping ~10,000 blocks of logs, the first newer sale was read at pass
    // 385-395 of 504 (the window's reset), none after a day, the walk 19,200 blocks behind. Now: the refusal keeps its class through
    // the transform (Deep), the walk re-anchors at the newest blocks, the blocks it passed over are read newest first, and what the
    // provider can no longer serve is recorded (lost) for the monitor
    let c = after_gap(None, false, false, 12_000, 72);
    assert!(c.first.is_some_and(|p| p <= 3), "control: {:?}", c.first);
    for (raw, plain, name) in [(false, false, "as the canister sees it (the class kept: Deep)"), (true, false, "raw words"),
        (false, true, "a refusal whose words name nothing (only the code): the lag re-anchors it")] {
        let r = after_gap(Some(10_000), raw, plain, 12_000, 144);
        println!("log depth 10,000, {name}: first newer sale at pass {:?}; after a day {} of the newer sales, lag {} blocks; after two days {} \
            of {}; lost {:?}; hole {:?}; behind {:?}; {} failed log reads", r.first, r.day, r.lag_day, r.week, r.fin, r.scan.lost, r.scan.hole,
            r.scan.behind, r.fails);
        assert!(r.first.is_some_and(|p| p <= 3), "a newer sale is read within a few passes: {:?}", r.first);
        assert!(r.day >= 65, "a day's newer sales are read: {}", r.day);
        assert!(r.lag_day <= en::SALE_SCAN_CHUNK_BLOCKS, "the walk is at the head: {}", r.lag_day);
        assert!(r.week + 2 >= r.fin, "{} of {}", r.week, r.fin);
        assert!(r.scan.lost.is_some() || r.scan.hole.is_some(), "what the provider cannot serve is recorded for the page");
        assert!(r.lines.iter().any(|l| l.contains("re-anchored")), "the note says so");
    }
}

#[test]
fn r6_b6_m1_b_the_transforms_keep_the_class_so_deep_and_the_size_refusals_classify() {
    use ef::{scan_error, ScanErr};
    // PoC `r6_b_m1_b_…`: every provider refusal but -32005 was `Other` as the canister saw it (the words dropped), and a rate limit
    // (-32005) was TooBig. Measured 2026-09-28 shapes, through the real transforms
    let got: Vec<ScanErr> = [
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Archive requests require a personal token. Get one at: https://www.allnodes.com/publicnode"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Log response size exceeded. You can make eth_getLogs requests with up to a 2K block range"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"block range is too wide"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"query timeout exceeded"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32701,"message":"broker: node error"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"query returned more than 10000 results"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"XRPC anonymous capacity temporarily busy"}}"#,
        r#"{"id":1,"jsonrpc":"2.0","error":{"code":-32602,"message":"invalid params","data":"Query returned more than 20000 results. Try with this block range [0x18dda4b, 0x18ddaf9]."}}"#,
        r#"{"id":1,"jsonrpc":"2.0","error":{"code":-32005,"message":"rate limit exceeded"}}"#,
    ].iter().map(|raw| scan_error(&seen_as(raw))).collect();
    assert_eq!(got, vec![ScanErr::Deep, ScanErr::TooBig, ScanErr::TooBig, ScanErr::TooBig, ScanErr::Other, ScanErr::TooBig, ScanErr::Busy,
        ScanErr::TooBig, ScanErr::Busy]);
    // the class is the same on every replica: the same words, the same body
    let a = en::transfer_logs_body(br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"query returned more than 10000 results. Try with this block range [0x1, 0x2]."}}"#);
    let b = en::transfer_logs_body(br#"{"id":7,"error":{"message":"query returned more than 10000 results. Try with this block range [0x3, 0x4].","code":-32005},"jsonrpc":"2.0"}"#);
    assert_eq!(a, b);
}

/// A week at xrpc.cl's measured rate limit: `pct` % of log reads (each marketplace query on its own) answered -32005 busy.
fn busy_week(pct: u64, cap: Option<u64>, passes: usize) -> (usize, usize, usize, usize, usize, usize, u32) {
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.pin_now = Some(ETH_PIN);
    f.m(MODULE).accepted = 93 * E / 100;
    f.seaport_range_cap = cap;
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let old = newer_sales(&mut f, top0 - en::SALE_WINDOW_BLOCKS + 200, 400, 120, 0x6a);
    let new = newer_sales(&mut f, top0 + 50, 100, passes as u64, 0x5e);
    f.busy_pct = pct;
    let mut pin = ETH_PIN;
    let mut max_bad = 0;
    for _ in 0..passes {
        pin += 100;
        f.pin_now = Some(pin);
        f.finalized = pin;
        quiet(&mut f, &mut st);
        max_bad = max_bad.max(scan_of(&st).bad.map_or(0, |b| b.len()));
    }
    let top = pin - en::SALE_FINAL_DEPTH_BLOCKS;
    let bottom = top - en::SALE_WINDOW_BLOCKS;
    let sc = scan_of(&st);
    let due: Vec<[u8; 32]> = old.iter().chain(new.iter()).filter(|x| x.1 >= bottom && x.1 + 7_200 <= top).map(|x| x.0).collect();
    let read = cached(&st, &due);
    // lost for good: in a block the scan will never read again (remembered as read, or given up), or in no range, queue or skip
    let never: Vec<u64> = sc.done_big.iter().flatten().copied().chain(sc.skipped.iter().flatten().filter(|s| s.1 == u64::MAX).map(|s| s.0)).collect();
    let blk_of: std::collections::HashMap<[u8; 32], u64> = old.iter().chain(new.iter()).map(|x| (x.0, x.1)).collect();
    let nowhere = due.iter().filter(|h| {
        let b = blk_of[*h];
        cached(&st, &[**h]) == 0
            && !sc.bad.iter().flatten().any(|(x, y)| *x <= b && b <= *y)
            && !sc.skipped.iter().flatten().any(|s| s.0 == b && s.1 != u64::MAX)
            && !sc.pend.iter().flatten().any(|m| m.hash == **h)
            && !sc.hole.is_some_and(|(x, y)| x <= b && b <= y)
    }).count();
    let lost = due.iter().filter(|h| never.contains(&blk_of[*h]) && cached(&st, &[**h]) == 0).count() + nowhere;
    (read, due.len(), lost, max_bad, sc.bad.map_or(0, |b| b.len()), sc.done_big.map_or(0, |d| d.len()), f.busy_hits)
}

#[test]
fn r6_b6_l2_a_rate_limit_is_retried_never_taken_as_too_big_and_no_sale_is_lost_for_good() {
    // PoC `r6_b_m2_a_…`: at 50 % busy, 20 of 432 due honest sales were lost for good (a busy Seaport half put the block in done_big,
    // ranges were evicted unread) and SaleScan.bad grew to 529. Now: a rate limit backs off and is read again; a block is remembered
    // as read only when both marketplaces answered; the refinement is kept to the bound
    for (pct, cap) in [(0, None), (0, Some(520)), (30, Some(520)), (50, Some(520))] {
        let r = busy_week(pct, cap, 504);
        println!("busy {pct} %, Seaport range cap {cap:?}: a week: {} of {} due sales counted; lost for good {}; SaleScan.bad at most {} ({} at \
            the end); done_big {}; busy answers {}", r.0, r.1, r.2, r.3, r.4, r.5, r.6);
        assert_eq!(r.2, 0, "no due sale is lost for good");
        assert!(r.3 <= 2 * en::SALE_BAD_MAX, "the deferred ranges stay bounded: {}", r.3);
        if pct <= 30 {
            assert!(r.0 + 1 >= r.1, "{} of {}", r.0, r.1);
        } else {
            assert!(r.0 * 100 >= r.1 * 95, "at 50 % busy, at least 95 % counted within the week: {} of {}", r.0, r.1);
        }
    }
}

#[test]
fn r6_b6_l1_unreadable_old_receipts_no_longer_hold_every_newer_sale() {
    // PoC `r6_b_l_sixteen_…`: 16 honest moves 30,000 blocks back whose receipts one provider nulls took every pass's 16 reads: 0 of 144
    // newer final sales read in two days. Now their retries are bounded and kept behind newer moves
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.pin_now = Some(ETH_PIN);
    f.m(MODULE).accepted = 93 * E / 100;
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let old = newer_sales(&mut f, top0 - 30_000, 10, 16, 0x6b);
    f.receipt_disagree = old.iter().map(|x| x.0).collect();
    let mut lines = vec![];
    for _ in 0..8 { lines.extend(quiet(&mut f, &mut st)); }
    let new = newer_sales(&mut f, top0 + 50, 100, 144, 0x5e);
    let hs: Vec<[u8; 32]> = new.iter().map(|x| x.0).collect();
    let mut pin = ETH_PIN;
    for _ in 0..144 {
        pin += 100;
        f.pin_now = Some(pin);
        f.finalized = pin;
        lines.extend(quiet(&mut f, &mut st));
    }
    let top = pin - en::SALE_FINAL_DEPTH_BLOCKS;
    let fin = new.iter().filter(|x| x.1 <= top).count();
    println!("16 old receipts unreadable: after two days {} of {} final newer sales read; rfail {:?}", cached(&st, &hs), fin,
        scan_of(&st).rfail.map(|v| v.len()));
    let unread: Vec<u64> = new.iter().filter(|x| cached(&st, &[x.0]) == 0).map(|x| x.1).collect();
    println!("unread newer: {:?} top {top}; pend {}", unread, scan_of(&st).pend.map_or(0, |p| p.len()));
    assert!(cached(&st, &hs) + 2 >= fin, "{} of {}", cached(&st, &hs), fin);
    assert!(scan_of(&st).rfail.is_some_and(|v| v.len() == 16), "the unreadable ones are still kept (retried, behind the newer)");
    assert!(lines.iter().any(|l| l.contains("unreadable after 3 tries")), "and noted");
    // once the provider serves them again, they are read
    f.receipt_disagree.clear();
    for _ in 0..(6 * 24 / 20 + 20) {
        pin += 100;
        f.pin_now = Some(pin);
        f.finalized = pin;
        f.now += 6 * 3_600;
        quiet(&mut f, &mut st);
    }
    assert_eq!(cached(&st, &old.iter().map(|x| x.0).collect::<Vec<_>>()), 16, "read once served (none lost)");
}

// =====================================================================================================================================
// Part C (review6/poc/C): the payout judge and the credit
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

/// Relay's own record of a payment out of its depository: `RelayCallExecuted(id, (to, data, value, allowFailure))`. Fix round 7 (C7-L-1):
/// as the contract emits it, ONE topic and the id in the data (round 6 indexed it, a shape the contract never emits).
fn relay_exec(id: [u8; 32], to: [u8; 20], value: u128) -> LogV {
    LogV { address: rc::RELAY_DEPOSITORY_4663, topics: vec![rc::RELAY_CALL_EXECUTED],
        data: [id.to_vec(), word(0x40), addr_word(&to), word(0x80), word(value), word(0), word(0)].concat() }
}

/// Fix round 7 (C7-L-1): the depository's real `execute` paying `to` `value`, its nonce `nonce` (here the order id, so the input names
/// the order), and its record (id = the request's struct hash).
fn relay_execute(to: [u8; 20], value: u128, nonce: [u8; 32]) -> (Vec<u8>, LogV) {
    let r = rc::CallRequest { calls: vec![rc::DepCall { to, data: vec![], value, allow_failure: false }], nonce, expiration: [0; 32] };
    (rc::encode_execute(&r, &[0x11; 65]), relay_exec(rc::call_request_hash(&r), to, value))
}

#[test]
fn r6_c6_l1_a_a_strangers_gas_only_depository_call_is_never_credited_relays_own_execution_is() {
    // PoC `r6_c_l1_a_…`: a stranger's allocator() + order id call to Relay's depository (0 value, gas only), handed in with an amount,
    // was CREDITED: returnAdvance paid the stated amount out of other flows' ETH, the hold ended and Azuki's order started
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount) = milady_written_off(&mut f, &mut st);
    run_for(&mut f, &mut st, 2, 1_200);
    f.home_balance += 2 * E;
    let oid = st.buy(mid).unwrap().late.as_ref().unwrap().watch.order_id;
    let crafted = [rc::ALLOCATOR.to_vec(), oid.to_vec()].concat();
    let tx = [0x5c; 32];
    f.payouts.insert((true, tx), ef::PayoutTx { from: ATTACKER, to: rc::RELAY_DEPOSITORY_4663, value: 0, input: crafted.clone(), block: HOME_PIN + 5, success: true });
    let named = amount / 100 * 98;
    ef::put_payout(&mut st, mid, true, tx, named, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(!l.contains("CREDITED") && (l.contains("RelayCallExecuted") || l.contains("not its execute")), "the stranger's call is refused: {l}");
    assert_eq!(st.late_hold(), Some(mid), "the C5-M-1 hold stands");
    run_for(&mut f, &mut st, 4, 1_200);
    assert_eq!(returned_to(&f, MODULE, rc::RETURN_ADVANCE), 0, "nothing paid out of the other flows' ETH");
    assert!(buy_of(&st, AZUKI).is_none(), "no new Relay order");
    // Relay's own execution out of its depository (its receipt carries RelayCallExecuted paying us): credited at the record's amount
    let real = [0x5d; 32];
    let (input, rec) = relay_execute(US, named, oid);
    f.payouts.insert((true, real), ef::PayoutTx { from: SOLVER, to: rc::RELAY_DEPOSITORY_4663, value: 0, input, block: HOME_PIN + 6, success: true });
    f.payout_logs.insert((true, real), vec![rec]);
    ef::put_payout(&mut st, mid, true, real, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED"), "{l}");
    assert!(st.late_hold().is_none());
}

#[test]
fn r6_c6_l1_b_no_depository_selector_and_no_address_like_request_id_names_the_order() {
    // PoC `r6_c_l1_b_…`: an ERC-20 transfer(us, 1) sent to the depository "named" the order through a runner-chosen request id equal to
    // our address word, and was credited at 3 ETH stated
    let us_word: [u8; 32] = addr_word(&US).try_into().unwrap();
    let w = ef::Watch { order_id: [0x0d; 32], value: 5 * E, min_out: 49 * E / 10, deadline: 1, balance_after: 0, eth_before: 0,
        eth_out_before: 0, pin: Some(26_000_000), solver: Some(SOLVER), request_id: Some(us_word), home_pin: Some(70_000_000) };
    let o = ef::LateOrder { watch: w.clone(), back: false, let_go_at: 2, credited: None, released_at: None };
    let erc20 = [vec![0xa9, 0x05, 0x9c, 0xbb], addr_word(&US), word(1)].concat();
    let t = ef::PayoutTx { from: ATTACKER, to: rc::RELAY_DEPOSITORY_4663, value: 0, input: erc20, block: 70_000_001, success: true };
    assert!(ef::judge_payout(&o, &t, &[], &[0x01; 32], true, &US, 3 * E, &[], false).is_err(), "no longer credited");
    // even with a depository execution record paying us, an address-like request id names nothing
    assert!(ef::judge_payout(&o, &t, &[relay_exec([0x33; 32], US, 3 * E)], &[0x01; 32], true, &US, 0, &[], false).unwrap_err().contains("does not name"));
    // Relay's execute naming the order, with its execution record paying us, is a payout, at its amount (fix round 7: the real encoding)
    let (input, rec) = relay_execute(US, 3 * E, w.order_id);
    let tx = ef::PayoutTx { input, from: SOLVER, ..t.clone() };
    assert_eq!(ef::judge_payout(&o, &tx, &[rec.clone()], &[0x01; 32], true, &US, 0, &[], false), Ok(3 * E));
    // a record from another contract, or paying someone else, is not
    let mut forged = rec.clone();
    forged.address = ATTACKER;
    assert!(ef::judge_payout(&o, &tx, &[forged], &[0x01; 32], true, &US, 0, &[], false).is_err());
    let (input2, rec2) = relay_execute(ATTACKER, 3 * E, w.order_id);
    assert!(ef::judge_payout(&o, &ef::PayoutTx { input: input2, ..tx.clone() }, &[rec2], &[0x01; 32], true, &US, 0, &[], false).is_err());
    // a plain ETH transfer to us naming the order stays a payout at its value: it is real ETH (fix round 6, judgment call)
    let t2 = ef::PayoutTx { from: ATTACKER, to: US, value: 5 * E / 2, input: w.order_id.to_vec(), block: 70_000_001, success: true };
    assert_eq!(ef::judge_payout(&o, &t2, &[], &[0x02; 32], true, &US, 0, &[], false), Ok(5 * E / 2));
}

#[test]
fn r6_c6_l2_a_credit_waits_while_the_loss_return_is_out_and_is_then_paid() {
    // PoC `r6_c_l2_…`: credited while the loss-closing returnAdvance(0) was unresolved: recorded Returned{payout}, the module paid 0, the
    // credit spent. Now the claim waits while that transaction is out; once it resolves, the payout goes to the module's surplus
    let mut f = Fake::new(true);
    let mut st = EthState::default();
    let (mid, amount) = milady_written_off(&mut f, &mut st);
    let l = ef::give_back_now(&mut st, mid, f.home_balance, f.now).unwrap();
    assert!(l.contains("closed as a loss"), "{l}");
    f.now += 600;
    tick(&mut f, &mut st);
    assert!(matches!(st.pending.as_ref().map(|p| &p.step), Some(ef::Step::GiveBack { .. })), "returnAdvance(0) is out");
    let oid = st.buy(mid).unwrap().late.as_ref().unwrap().watch.order_id;
    let payout = amount / 100 * 98;
    f.home_balance += payout;
    f.payouts.insert((true, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: payout, input: oid.to_vec(), block: HOME_PIN + 5, success: true });
    ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).unwrap();
    // the returnAdvance(0) is not yet seen at the pin (not mined yet there, or a provider that has not caught up): it stays out
    let hash = st.pending.as_ref().unwrap().hash;
    f.hide.push(hash);
    f.home_nonce -= 1;
    f.now += 300;
    let l = tick(&mut f, &mut st).join("\n");
    assert!(st.pending.is_some(), "{l}");
    assert!(!l.contains("CREDITED") && l.contains("waits"), "{l}");
    assert!(st.inbox.payouts.as_ref().is_some_and(|v| v.len() == 1), "the claim is kept");
    assert!(st.buy(mid).unwrap().late.as_ref().unwrap().credited.is_none());
    f.hide.clear();
    f.home_nonce += 1;
    run_for(&mut f, &mut st, 2, 120);
    assert!(st.pending.is_none() || !matches!(st.pending.as_ref().map(|p| &p.step), Some(ef::Step::GiveBack { buy }) if *buy == mid));
    assert!(matches!(st.buy(mid).unwrap().phase, BPhase::Returned { wei: 0 }), "the loss closed with nothing: {:?}", st.buy(mid).unwrap().phase);
    // the operator hands it in again (or the kept claim is judged): credited to the module's surplus, never recorded as returned unpaid
    if st.buy(mid).unwrap().late.as_ref().unwrap().credited.is_none() {
        ef::put_payout(&mut st, mid, true, PAYOUT_TX, 0, f.now).unwrap();
    }
    let l = run_for(&mut f, &mut st, 6, 600).join("\n");
    assert!(st.buy(mid).unwrap().late.as_ref().unwrap().credited.is_some(), "{l}");
    let paid = returned_to(&f, MODULE, ef::FUND_RESERVE) + returned_to(&f, MODULE, rc::RETURN_ADVANCE);
    assert_eq!(paid, payout, "the module is paid the payout: {l}");
}

#[test]
fn r6_c6_l3_a_credited_forward_payout_keeps_its_custody_reservation() {
    // PoC `r6_c_l3_a_…`: the write-off released `eth:{id}`; the credited late fill then minted an item the custody book never counted
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    let flight = format!("eth:{id}");
    assert!(f.book.entries.iter().any(|e| e.id == flight && e.in_flight), "the advance reserved its room");
    st.used_orders.clear();
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let BPhase::Bridging { deadline, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    f.now = deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 60;
    fresh_modules(&mut f);
    tick(&mut f, &mut st);
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("written off"));
    f.now += 600;
    fresh_modules(&mut f);
    let l = tick(&mut f, &mut st).join("\n");
    assert!(f.book.entries.iter().any(|e| e.id == flight && e.in_flight), "the open written-off order keeps its room: {l}");
    let oid = st.buy(id).unwrap().late.as_ref().unwrap().watch.order_id;
    f.eth_balance += ARRIVED;
    f.pin_now = Some(ETH_PIN + 300);
    f.finalized = ETH_PIN + 250;
    f.payouts.insert((false, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: ARRIVED, input: oid.to_vec(), block: ETH_PIN + 200, success: true });
    ef::put_payout(&mut st, id, false, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED") && matches!(phase(&st), BPhase::Arrived { .. }), "{l}");
    f.pin_now = None;
    f.finalized = 0;
    evidence(&f, &mut st);
    ef::put_listing(&mut st, id, fill_hex(), f.now).unwrap();
    let mut all = tick(&mut f, &mut st);
    f.mine_eth(f.eth_signed.len() - 1, true, ETH_PIN - 10);
    all.extend(tick(&mut f, &mut st));
    f.finalized = ETH_PIN;
    for _ in 0..6 {
        f.now += 600;
        fresh_modules(&mut f);
        all.extend(tick(&mut f, &mut st));
    }
    let all = all.join("\n");
    assert!(matches!(phase(&st), BPhase::Minted { .. } | BPhase::Settling { .. } | BPhase::Settled { .. }), "{:?}\n{all}", phase(&st));
    assert!(f.book.contains(&held_id(MILADY, 9_180)), "the item in custody is in the custody book");
    assert!(!all.contains("not in flight"), "{all}");
}

#[test]
fn r6_c6_l3_b_a_released_order_credited_after_all_takes_its_custody_room_again() {
    // C6-L-3, the released case: the operator released the order (its room goes with it); a fill credited afterwards reserves the room
    // again before it arrives, so its item is in the custody book too
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).net_units = 2 * E;
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let id = st.buys[0].id;
    let flight = format!("eth:{id}");
    assert!(f.book.entries.iter().any(|e| e.id == flight && e.in_flight), "the advance reserved its room");
    st.used_orders.clear();
    ef::put_quote(&mut st, id, QUOTE.as_bytes().to_vec(), f.now).unwrap();
    tick(&mut f, &mut st);
    tick(&mut f, &mut st);
    let BPhase::Bridging { deadline, .. } = phase(&st) else { panic!("{:?}", phase(&st)) };
    f.now = deadline + ef::DEADLINE_GRACE_SECS + ef::BACK_LATE_SECS + 60;
    fresh_modules(&mut f);
    tick(&mut f, &mut st);
    assert!(ef::give_back_now(&mut st, id, f.home_balance, f.now).unwrap().contains("written off"));
    f.now += 600;
    fresh_modules(&mut f);
    let l = tick(&mut f, &mut st).join("\n");
    assert!(f.book.entries.iter().any(|e| e.id == flight && e.in_flight), "the open written-off order keeps its room: {l}");
    ef::late_release(&mut st, id, f.now).unwrap();
    f.now += 600;
    fresh_modules(&mut f);
    let l = tick(&mut f, &mut st).join("\n");
    assert!(!f.book.entries.iter().any(|e| e.id == flight), "released: its room goes: {l}");
    let oid = st.buy(id).unwrap().late.as_ref().unwrap().watch.order_id;
    f.eth_balance += ARRIVED;
    f.pin_now = Some(ETH_PIN + 300);
    f.finalized = ETH_PIN + 250;
    f.payouts.insert((false, PAYOUT_TX), ef::PayoutTx { from: SOLVER, to: US, value: ARRIVED, input: oid.to_vec(), block: ETH_PIN + 200, success: true });
    ef::put_payout(&mut st, id, false, PAYOUT_TX, 0, f.now).unwrap();
    let l = tick(&mut f, &mut st).join("\n");
    assert!(l.contains("CREDITED") && matches!(phase(&st), BPhase::Arrived { .. }), "{l}");
    f.pin_now = None;
    f.finalized = 0;
    evidence(&f, &mut st);
    ef::put_listing(&mut st, id, fill_hex(), f.now).unwrap();
    let mut all = tick(&mut f, &mut st);
    f.mine_eth(f.eth_signed.len() - 1, true, ETH_PIN - 10);
    all.extend(tick(&mut f, &mut st));
    f.finalized = ETH_PIN;
    for _ in 0..6 {
        f.now += 600;
        fresh_modules(&mut f);
        all.extend(tick(&mut f, &mut st));
    }
    let all = all.join("\n");
    assert!(matches!(phase(&st), BPhase::Minted { .. } | BPhase::Settling { .. } | BPhase::Settled { .. }), "{:?}\n{all}", phase(&st));
    assert!(f.book.contains(&held_id(MILADY, 9_180)), "the item in custody is in the custody book");
    assert!(!all.contains("not in flight"), "{all}");
}

// =====================================================================================================================================
// Part F (review6/poc/F) and part D's canister half: the home line's exits
// =====================================================================================================================================

/// hidden_deposit with the 4663 pin advancing at 10 blocks a second from now, the chain able to tell what used the nonce (`identify`)
/// but its STATE kept only `depth` blocks back (None: archive).
fn hidden_deposit_live(depth: Option<u64>) -> (Fake, EthState) {
    let mut f = Fake::new(false);
    f.live_pin = Some(f.now);
    f.state_depth = depth;
    f.identify = true;
    let mut st = EthState::default();
    hidden_deposit(&mut f, &mut st, E);
    (f, st)
}

#[test]
fn r6_f6_l1_at_the_runners_pending_cadence_the_identification_is_inside_the_providers_state_depth() {
    // PoC `r6_f6_l1_at_the_runners_cadence_…`: at the runner's 600 s sleep plus a 60 s cycle, the first resolving tick's pin was 6,600
    // blocks past HomeTx.pin, beyond Robinhood's ~6,200 blocks of state: a hidden deposit was UNIDENTIFIED for a day. The runner now
    // sleeps RUNNER_PENDING_EVERY_S (240 s) while a home transaction is out (ops/src/runner.ts `sleepAfter`): 300 s = 3,000 blocks
    let (mut f, mut st) = hidden_deposit_live(Some(6_200));
    let l = run_for(&mut f, &mut st, 1, 240 + 60).join("\n");
    assert!(l.contains("MINED") && l.contains("nonce count"), "{l}");
    assert!(matches!(phase(&st), BPhase::Bridging { .. }), "{:?}", phase(&st));
    // the old cadence, for the record: out of reach, the operator's word decides (the page names eth_home_clear for this step)
    let (mut f, mut st) = hidden_deposit_live(Some(6_200));
    let l = run_for(&mut f, &mut st, 7, 660).join("\n");
    assert!(l.contains("UNIDENTIFIED") && !l.contains("nonce count"), "{l}");
    assert!(ef::home_clear_applies(&st, &st.pending.as_ref().unwrap().step));
}

#[test]
fn r6_f6_l2_one_provider_that_disagrees_on_the_receipt_no_longer_halts_the_home_line() {
    // PoC `r6_f6_l2_…`: a mined deposit whose receipt the providers disagree on (Unsure) for a day: used_since never set,
    // eth_home_clear refused, no post, mint or record signed, identification never tried. Now: with its nonce used it is decided as
    // "seen nowhere" is: identified at once (here, archive state)
    let (mut f, mut st) = hidden_deposit_live(None);
    f.unsure = true;
    f.m(MODULE).posted_at = f.now;
    let l = run_for(&mut f, &mut st, 1, 600).join("\n");
    assert!(l.contains("MINED") && l.contains("nonce count"), "identified although the providers disagree on the receipt: {l}");
    assert!(st.pending.as_ref().is_none_or(|p| !matches!(p.step, ef::Step::Deposit { .. })));
    assert_eq!(relay_deposits(&f), 1, "never deposited again");
    // and when it cannot be identified (no state that deep), the clock runs and the operator's exit opens after the hour
    let (mut f, mut st) = hidden_deposit_live(Some(6_200));
    f.unsure = true;
    let nonce = st.pending.as_ref().unwrap().nonce;
    run_for(&mut f, &mut st, 7, 660);
    assert!(st.pending.as_ref().is_some_and(|p| p.used_since.is_some()), "the clock runs under the disagreement");
    assert!(ef::home_clear(&mut st, nonce, true, f.now).is_ok(), "the operator's exit opens");
    let l = run_for(&mut f, &mut st, 1, 60).join("\n");
    assert!(l.contains("the operator says it MINED"), "{l}");
    assert_eq!(relay_deposits(&f), 1);
}

#[test]
fn r6_d6_l1_eth_home_clear_refuses_the_steps_it_is_never_applied_to() {
    // PoC `D6-L-1` / F6-I-2: `home_clear` accepted any pending step ("taken as MINED at the next tick") but the canister applies the word
    // only to a Relay deposit or a sweep's fundReserve: for every other step it silently did nothing
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.m(MODULE).posted_at = f.now - 3_000;
    evidence(&f, &mut st);
    tick(&mut f, &mut st);
    let p = st.pending.clone().expect("a post is out");
    assert!(matches!(p.step, ef::Step::Post { .. }));
    st.pending.as_mut().unwrap().used_since = Some(f.now - 4_000);
    let e = ef::home_clear(&mut st, p.nonce, true, f.now).unwrap_err();
    assert!(e.contains("decided by its own effect"), "{e}");
    assert!(st.inbox.home_clear.is_none());
    // a deposit takes it
    let (f2, mut st2) = hidden_deposit_live(None);
    let n = st2.pending.as_ref().unwrap().nonce;
    st2.pending.as_mut().unwrap().used_since = Some(f2.now - 4_000);
    assert!(ef::home_clear(&mut st2, n, true, f2.now).is_ok());
}

#[test]
fn r6_d6_l2_a_home_transaction_says_when_it_was_signed() {
    // D6-L-2: Pending and Unsure reset `used_since`, so a line held by a stale pool entry never paged. `HomeTx.sent_at` is kept from the
    // signature; the monitor pages on its age whatever the providers show (monitor/test/audit_nft_d_r6.test.mjs)
    let (f, st) = hidden_deposit_live(None);
    let p = st.pending.as_ref().unwrap();
    assert!(p.sent_at.is_some_and(|t| t <= f.now && t + 3_600 > f.now), "{:?}", p.sent_at);
}

#[test]
fn r6_b5_l2_a_skipped_block_is_never_given_up_and_one_sided_reads_are_not_remembered() {
    // B5-L-2 (carried, made permanent in round 5): a block over the largest class whose marketplace logs could not be read either was
    // given up after 3 tries (its sales never counted), and a block whose Seaport half failed while Blur's answered was remembered as
    // read (done_big). Now it is retried, backing off, for as long as it is in the window, and remembered only when both answered
    let top0 = ETH_PIN - en::SALE_FINAL_DEPTH_BLOCKS;
    let mut f = Fake::new(false);
    let mut st = EthState::default();
    f.finalized = ETH_PIN;
    f.m(MODULE).accepted = 93 * E / 100;
    let blk = top0 - 3_000;
    f.big_blocks = vec![blk];
    f.big_sale_blocks = vec![blk];
    let sale = newer_sales(&mut f, blk, 1, 1, 0x7a);
    let mv = en::Move { hash: sale[0].0, block: blk, token: tok(0x7a * 1_000_000), from: [0x7a; 20] };
    f.sale_moves.insert(blk, vec![mv]);
    for _ in 0..30 {
        quiet(&mut f, &mut st);
    }
    for _ in 0..8 {
        f.now += 18 * 3_600;
        quiet(&mut f, &mut st);
    }
    let sk = scan_of(&st).skipped.unwrap_or_default();
    assert!(sk.iter().any(|(b, at, t)| *b == blk && *at != u64::MAX && *t >= en::SALE_SKIP_TRIES), "still tried after {} tries: {sk:?}",
        en::SALE_SKIP_TRIES);
    assert!(!scan_of(&st).done_big.unwrap_or_default().contains(&blk));
    // the marketplace logs answer again: read and counted, then remembered
    f.big_sale_blocks.clear();
    for _ in 0..4 {
        f.now += 18 * 3_600;
        quiet(&mut f, &mut st);
    }
    assert!(scan_of(&st).done_big.unwrap_or_default().contains(&blk));
    assert_eq!(cached(&st, &[sale[0].0]), 1, "its sale is counted");
}

// =====================================================================================================================================
// Upgrade safety: the state the round-5 build wrote restores; the round-6 state the services decode
// =====================================================================================================================================

#[test]
fn r6_the_state_the_round_5_build_wrote_restores_with_nothing_lost_and_the_new_fields_absent() {
    use etch_autobuyer::canister::lost_opts;
    // tests/fixtures/state/eth-state-e04fa16.hex: the bytes e04fa16's own `r5_the_round_5_state_fixture_is_what_the_canister_encodes`
    // held equal to what that build encodes (every round-5 field present)
    let bytes = hex::decode(include_str!("fixtures/state/eth-state-e04fa16.hex").trim()).unwrap();
    let e: EthState = candid::decode_one(&bytes).unwrap();
    let again = candid::encode_one(&e).unwrap();
    assert_eq!(lost_opts(&bytes, &again).unwrap(), Vec::<String>::new(), "an honest upgrade would be refused");
    assert!(e.pending.as_ref().is_some_and(|p| p.pin.is_some() && p.sent_at.is_none()));
    assert!(e.sale_scan.iter().flatten().all(|s| s.done_big.is_some() && s.hole.is_none() && s.lost.is_none() && s.behind.is_none()
        && s.rfail.is_none() && s.raw.is_none()));
    assert!(e.inbox.home_clear.is_some() && e.inbox.payouts.is_some(), "round 5's own fields kept");
}

/// The state this round writes, every one of its new fields present (`HomeTx.sent_at`; `SaleScan.hole` / `lost` / `behind` / `rfail` /
/// `raw`): the services decode it (`tests/fixtures/eth-state-round6.hex`, read by the monitor's and the runner's tests).
fn round6_state() -> EthState {
    let w = ef::Watch { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, balance_after: 1, eth_before: 2, eth_out_before: 3, pin: Some(4),
        solver: Some([0xf7; 20]), request_id: Some([0x17; 32]), home_pin: Some(70_166_016) };
    let b = |id: u64, phase: BPhase| ef::EthBuy { id, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: 1, attempts: 1, phase,
        orders: vec![], deposits: vec![], ask: None, last: Some(w.clone()), note: String::new(), sweep: Some(false), arrived_at: Some(3),
        back_at: Some(4), ended_at: None, late: None };
    let mut st = EthState::default();
    let mut held = b(1, BPhase::Returned { wei: 0 });
    held.last = None;
    held.late = Some(ef::LateOrder { watch: w.clone(), back: true, let_go_at: 5, credited: None, released_at: None });
    st.buys = vec![b(0, BPhase::Bridging { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, since: 2 }), held];
    st.pending = Some(ef::HomeTx { step: ef::Step::Deposit { buy: 0 }, raw: vec![1], hash: [4; 32], nonce: 7, used_since: None, pin: Some(70_166_000),
        sent_at: Some(1_791_000_000) });
    st.sale_scan = Some(vec![ef::SaleScan { collection: MILADY, down: 1, up: 9, chunk: 500, todo: vec![], pend: Some(vec![]), rescan: None,
        judged: Some(vec![]), bad: Some(vec![(4, 5)]), skipped: Some(vec![(6, 7, 1)]), done_big: Some(vec![8]), stuck: None,
        spent: Some((1, 2)), hole: Some((10, 20)), lost: Some((2, 3)), behind: Some(9_000), rfail: Some(vec![([5; 32], 3, 99)]),
        raw: Some(vec![(4, 5)]), current: None, hole_deep: None, rbusy: None, busy: None }]);
    st.inbox.payouts = Some(vec![ef::PayoutClaim { buy: 1, home: true, tx: [9; 32], wei: 0, at: 5 }]);
    st.credited = Some(vec![]);
    st
}

#[test]
fn r6_the_round_6_state_fixture_is_what_the_canister_encodes() {
    let st = round6_state();
    let bytes = candid::encode_one(&st).unwrap();
    assert_eq!(candid::decode_one::<EthState>(&bytes).unwrap(), st);
    let fixture = include_str!("fixtures/eth-state-round6.hex").trim();
    assert_eq!(hex::encode(&bytes), fixture, "tests/fixtures/eth-state-round6.hex is stale: write the new bytes there (and run the monitor's and ops' tests)");
}
