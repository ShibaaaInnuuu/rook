//! Fake chain-4663 providers with the limits the real ones were measured to have (docs/audit/2026-09-26/fixes-D-canister.md §0,
//! 2026-09-26). The old fakes answered any read at any block, which is how a delivery path that no real provider pair could
//! serve passed every test (audit D, H-1). These refuse what the real ones refuse, with the errors they give:
//!
//! - **Robinhood** (`rpc.mainnet.chain.robinhood.com`): state only [`RH_STATE_BLOCKS`] behind its head
//!   (`-32000 historical state … is not available`); `eth_getLogs` over any span.
//! - **ordofi** (`rpc.ordofi.network`): state at any depth; `eth_getLogs` over at most [`ORDOFI_LOG_SPAN`] blocks
//!   (`-32005 the network is busy, please try again in a moment`).
//! - Both: `finalized` [`FIN_LAG_MIN`]–(`FIN_LAG_MIN` + [`FIN_STEP`]) behind the head, moving in steps of `FIN_STEP`.
//! - Both, as the canister reaches them: an `eth_getLogs` answer larger than the outcall's response cap fails as the IC fails it
//!   (`Http body exceeds size limit of N bytes`, behind `evmread::SIZE_REJECT` as `canister.rs` reports it), headers included ([`HEADER_BYTES`]; audit R2, N-1). Each log is sized as a Nitro
//!   node serves it, minified ([`raw_log_bytes`]).
#![allow(dead_code)]

/// The Solana provider pair (docs/v9/solana-migration.md §4).
pub mod sol;
/// The System Program and Relay's depository on it, executing what the canister signs (the books home: no program of Rook's).
pub mod sys;

use std::cell::RefCell;
use std::rc::Rc;

use etch_autobuyer::evmread::{LogView, Rpc};
use etch_autobuyer::pin::Pin;
use serde_json::Value;

/// Robinhood serves state this many blocks behind its head (bisected: 6,201 answers, 6,250 does not).
pub const RH_STATE_BLOCKS: u64 = 6_200;
/// ordofi answers `eth_getLogs` over at most this many blocks, inclusive.
pub const ORDOFI_LOG_SPAN: u64 = 10_000;
/// The least `head − finalized` measured, and the size of a finality step.
pub const FIN_LAG_MIN: u64 = 8_096;
pub const FIN_STEP: u64 = 3_900;
/// ≈ 10 blocks a second (9.9 measured).
pub const BLOCKS_PER_SEC: u64 = 10;
/// A provider's response headers and the JSON-RPC envelope, counted against the response cap (ordofi's headers measured ≈ 984
/// bytes on 2026-09-26).
pub const HEADER_BYTES: usize = 1_040;

/// One log's size in a provider's raw answer: the fields a Nitro node returns (address, blockHash, blockNumber, blockTimestamp,
/// data, logIndex, removed, topics, transactionHash, transactionIndex), minified. A `Redeemed` log is ≈ 900 bytes.
pub fn raw_log_bytes(l: &LogView) -> usize {
    let v = serde_json::json!({ "address": l.address, "topics": l.topics, "data": l.data, "blockNumber": format!("0x{:x}", l.block),
        "blockHash": format!("0x{}", "ab".repeat(32)), "blockTimestamp": "0x68d6a2c1", "transactionHash": format!("0x{}", "cd".repeat(32)),
        "transactionIndex": "0x1", "logIndex": "0x1f", "removed": false });
    serde_json::to_string(&v).unwrap().len() + 1
}

/// One chain every fake provider serves.
#[derive(Default)]
pub struct Chain {
    pub head: u64,
    /// Every log, any address.
    pub logs: Vec<LogView>,
}

impl Chain {
    pub fn finalized_at(head: u64) -> u64 {
        head.saturating_sub(FIN_LAG_MIN) / FIN_STEP * FIN_STEP
    }
    pub fn finalized(&self) -> u64 {
        Self::finalized_at(self.head)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Robinhood,
    Ordofi,
}

pub struct FakeRpc {
    pub kind: Kind,
    pub chain: Rc<RefCell<Chain>>,
    /// Blocks this provider's head is behind the chain's.
    pub lag: u64,
    /// Finality steps this provider is behind (one, for ≈ 20 s after a step, as measured).
    pub fin_steps_behind: u64,
    /// Finality this provider claims instead (a liar or a broken node).
    pub fin_override: Option<u64>,
    /// Logs this provider adds to every answer (a provider that disagrees).
    pub extra_logs: Vec<LogView>,
    /// Every request, for the tests' assertions: `(method, from, to)`.
    pub asked: Rc<RefCell<Vec<(String, u64, u64)>>>,
    /// The response cap of every `eth_getLogs`, in order.
    pub caps: Rc<RefCell<Vec<u64>>>,
}

impl FakeRpc {
    pub fn new(kind: Kind, chain: &Rc<RefCell<Chain>>) -> Self {
        FakeRpc { kind, chain: chain.clone(), lag: 0, fin_steps_behind: 0, fin_override: None, extra_logs: vec![],
            asked: Rc::new(RefCell::new(vec![])), caps: Rc::new(RefCell::new(vec![])) }
    }
    fn head_now(&self) -> u64 {
        self.chain.borrow().head.saturating_sub(self.lag)
    }
    fn fin_now(&self) -> u64 {
        self.fin_override.unwrap_or_else(|| Chain::finalized_at(self.head_now()).saturating_sub(self.fin_steps_behind * FIN_STEP))
    }
}

/// The measured pair: Robinhood and ordofi, over one chain.
pub fn pair(chain: &Rc<RefCell<Chain>>) -> Vec<FakeRpc> {
    vec![FakeRpc::new(Kind::Robinhood, chain), FakeRpc::new(Kind::Ordofi, chain)]
}

pub fn hash_of(n: u64) -> String {
    format!("0x{n:064x}")
}

impl Rpc for FakeRpc {
    fn name(&self) -> String {
        format!("{:?}", self.kind)
    }
    async fn head(&mut self, bucket: u64) -> Result<u64, String> {
        self.asked.borrow_mut().push(("eth_blockNumber".into(), 0, 0));
        Ok(self.head_now() / bucket * bucket)
    }
    async fn finalized(&mut self, bucket: u64) -> Result<u64, String> {
        self.asked.borrow_mut().push(("finalized".into(), 0, 0));
        Ok(self.fin_now() / bucket * bucket)
    }
    async fn block(&mut self, n: u64) -> Result<Value, String> {
        if n > self.head_now() {
            return Ok(Value::Null);
        }
        Ok(serde_json::json!({ "number": format!("0x{n:x}"), "hash": hash_of(n), "baseFeePerGas": "0x3669b00" }))
    }
    async fn call(&mut self, _to: &[u8; 20], _data: &[u8], at: &Pin) -> Result<Vec<u8>, String> {
        self.asked.borrow_mut().push(("eth_call".into(), at.number, at.number));
        if at.hash != hash_of(at.number) {
            return Err("-32000 header not found".into());
        }
        if self.kind == Kind::Robinhood && self.head_now().saturating_sub(at.number) > RH_STATE_BLOCKS {
            return Err(format!("-32000 historical state {} is not available", &at.hash[2..10]));
        }
        Ok(vec![0u8; 32])
    }
    async fn logs(&mut self, address: &[u8; 20], topics: &Value, from: u64, to: u64, max_bytes: u64) -> Result<Vec<LogView>, String> {
        self.asked.borrow_mut().push(("eth_getLogs".into(), from, to));
        self.caps.borrow_mut().push(max_bytes);
        if self.kind == Kind::Ordofi && to.saturating_sub(from) + 1 > ORDOFI_LOG_SPAN {
            return Err("-32005 the network is busy, please try again in a moment".into());
        }
        let want = format!("0x{}", hex::encode(address));
        let any_of = |i: usize| -> Vec<String> {
            topics.get(i).and_then(|t| t.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        let (t0, t1) = (any_of(0), any_of(1));
        let head = self.head_now();
        let c = self.chain.borrow();
        let out: Vec<LogView> = c.logs.iter().chain(self.extra_logs.iter())
            .filter(|l| l.address == want && l.block >= from && l.block <= to.min(head))
            .filter(|l| t0.is_empty() || l.topics.first().is_some_and(|x| t0.contains(x)))
            .filter(|l| t1.is_empty() || l.topics.get(1).is_some_and(|x| t1.contains(x)))
            .cloned().collect();
        // the IC refuses an answer over the cap it was asked with, headers included (audit R2, N-1)
        let body = HEADER_BYTES + out.iter().map(raw_log_bytes).sum::<usize>();
        if body as u64 > max_bytes {
            // as `canister.rs` reports the IC's reject: the fixed prefix, then the IC's own words (audit R3, R3-C2)
            return Err(format!("{}Http body exceeds size limit of {max_bytes} bytes.", etch_autobuyer::evmread::SIZE_REJECT));
        }
        Ok(out)
    }
}

/// The wrapper's two events, as a Nitro node serves them.
pub fn word(v: u128) -> String {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    format!("0x{}", hex::encode(w))
}

/// `Redeemed(ticket, owner, round, destination)`: the destination ABI-encoded after the round, as the contract emits it.
pub fn redeemed(wrapper: &[u8; 20], ticket: u64, dest: &str, block: u64) -> LogView {
    let topic0 = format!("0x{}", hex::encode(etch_autobuyer::redeem::REDEEMED_TOPIC));
    let mut data = Vec::new();
    data.extend(hex::decode(&word(3)[2..]).unwrap());
    data.extend(hex::decode(&word(64)[2..]).unwrap());
    data.extend(hex::decode(&word(dest.len() as u128)[2..]).unwrap());
    let mut s = dest.as_bytes().to_vec();
    s.resize(dest.len().div_ceil(32) * 32, 0);
    data.extend(s);
    LogView { address: format!("0x{}", hex::encode(wrapper)), topics: vec![topic0, word(ticket as u128), word(0)],
        data: format!("0x{}", hex::encode(data)), block, removed: false }
}

/// `Resolved(ticket, acquisitionId, deliverBy)`.
pub fn resolved(wrapper: &[u8; 20], ticket: u64, acq: u128, block: u64) -> LogView {
    let topic0 = format!("0x{}", hex::encode(etch_autobuyer::ticketlog::resolved_topic()));
    LogView { address: format!("0x{}", hex::encode(wrapper)), topics: vec![topic0, word(ticket as u128), word(acq)],
        data: word(1_790_604_800), block, removed: false }
}
