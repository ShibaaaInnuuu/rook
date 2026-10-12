//! The real world for the Ethereum NFT side (docs/v7/phase1-nft.md §4): the home chain through `CanisterIo`'s pinned reads and
//! one-node sends, Ethereum through `nftnet`'s per-chain pins (every provider agreeing), the threshold key, and the endpoints.
//! Nothing is decided here: `ethflow.rs` sequences, `ethnft.rs`, `relayeth.rs` and `seaport.rs` judge.

use std::cell::RefCell;

use ic_cdk::management_canister::{HttpRequestResult, TransformArgs};

use crate::canister::{cfg, h0x, hex_bytes, hex_u128, only_operator, CanisterIo};
use crate::chains::EvmChain;
use crate::ethflow::{EthIo, EthPin, EthPlan, EthState, OurReceipt};
use crate::ethnft::{self as en, EthNftConfig, ReceiptV, Seen};
use crate::evmtx::Tx1559;
use crate::itemsink::Clock;
use crate::looper::{EvmView, Io};
use crate::nftnet as nn;
use crate::seaport::{self as sp, Word};

thread_local! {
    /// Everything this side keeps. Saved across upgrades as the upgrade tuple's thirteenth element.
    pub(crate) static ETH: RefCell<EthState> = RefCell::new(EthState::default());
    /// 2026-10-12 (Robinhood Chain NFTs): the side of every other chain the collections live on (`EthNftConfig::chains`, past the
    /// config's own), by chain: the same flow with its own state, its own Ethereum line, gas pot and leftovers on its chain. Saved
    /// across upgrades inside the thirteenth element (`EthState.sides`).
    pub(crate) static SIDES: RefCell<Vec<(u64, EthState)>> = const { RefCell::new(Vec::new()) };
    /// Each side's pin floor on its own chain, as `canister::PIN_FLOORS.eth` is the config's own chain's (a 4663 block number is no
    /// floor for Ethereum's, nor the other way).
    pub(crate) static SIDE_FLOORS: RefCell<Vec<(u64, crate::pin::PinFloor)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn config() -> Option<EthNftConfig> {
    cfg().eth_nft
}

/// 2026-10-12: whether any side, the config's own or another chain's, holds the one Relay leg (`EthState::relay_busy`), but `except`.
/// A side that cannot be read says busy.
pub(crate) fn relay_busy_except(except: Option<u64>) -> bool {
    let own = except.is_some() && ETH.with(|e| e.try_borrow().map_or(true, |s| s.relay_busy()));
    own || SIDES.with(|s| s.try_borrow().map_or(true, |v| v.iter().any(|(c, st)| Some(*c) != except && st.relay_busy())))
}

/// Whether any side holds the one Relay leg (the frog side's `eth_relay_busy`).
pub(crate) fn any_relay_busy() -> bool {
    ETH.with(|e| e.try_borrow().map_or(true, |s| s.relay_busy())) || relay_busy_except(None)
}

/// The chain of the side that holds buy `buy`, when it is another chain's (`None`: the config's own side).
fn side_of_buy(buy: u64) -> Option<u64> {
    SIDES.with(|s| s.borrow().iter().find(|(_, st)| st.buy(buy).is_some()).map(|(c, _)| *c))
}

/// `f` on the state of the side of `chain` (`None`, or the config's own chain: `ETH`); a side not ticked yet starts empty.
fn with_side<R>(chain: Option<u64>, f: impl FnOnce(&mut EthState) -> R) -> R {
    let own = config().map(|c| c.chain_id);
    match chain.filter(|c| Some(*c) != own) {
        None => ETH.with(|e| f(&mut e.borrow_mut())),
        Some(c) => SIDES.with(|s| {
            let mut v = s.borrow_mut();
            if !v.iter().any(|(x, _)| *x == c) {
                v.push((c, crate::ethflow::new_side(c)));
            }
            f(&mut v.iter_mut().find(|(x, _)| *x == c).expect("just pushed").1)
        }),
    }
}

/// `f` on the state of the side that holds buy `buy`.
fn with_buy<R>(buy: u64, f: impl FnOnce(&mut EthState) -> R) -> R {
    with_side(side_of_buy(buy), f)
}

/// `pre_upgrade`: the other chains' sides, each with its own pin floor, encoded for `EthState.sides`. `None` with none.
pub(crate) fn sides_blob() -> Option<Vec<(u64, Vec<u8>)>> {
    let v = SIDES.with(|s| s.borrow().clone());
    if v.is_empty() {
        return None;
    }
    let floors = SIDE_FLOORS.with(|f| f.borrow().clone());
    Some(v.into_iter().map(|(c, st)| {
        let eth = floors.iter().find(|(x, _)| *x == c).map(|(_, f)| *f).unwrap_or_default();
        let st = EthState { pin_floors: Some(crate::pin::PinFloors { home: Default::default(), eth }), ..st };
        (c, candid::encode_one(&st).expect("an EthState encodes"))
    }).collect())
}

/// `post_upgrade`: the sides out of `EthState.sides` (a side that does not decode refuses the upgrade: it is never a fresh start).
pub(crate) fn sides_of(blob: Option<Vec<(u64, Vec<u8>)>>) -> Result<Vec<(u64, EthState)>, String> {
    blob.unwrap_or_default().into_iter().map(|(c, b)| candid::decode_one::<EthState>(&b).map(|st| (c, st))
        .map_err(|e| format!("the saved side of chain {c} does not decode: {e}"))).collect()
}

/// Audit C3-06, `post_upgrade`: the saved sides against the new config's own chain `own`. A side on that chain would never be ticked
/// again (`with_side` sends the config's own chain to `ETH`) and its open work would hold the Relay leg for good: refused while it holds
/// any (a line, a home transaction, an unfinished buy or delivery, the Relay leg), dropped when idle.
pub(crate) fn sides_kept(v: Vec<(u64, EthState)>, own: Option<u64>) -> Result<Vec<(u64, EthState)>, String> {
    let mut out = Vec::new();
    for (c, st) in v {
        if Some(c) != own {
            out.push((c, st));
            continue;
        }
        if st.line.is_some() || st.home_busy() || st.relay_busy() || st.buys.iter().any(|b| !b.finished()) || st.deliveries.iter().any(|d| !d.finished()) {
            return Err(format!("the side of chain {c} has work open (a line, a home transaction, an open buy or delivery, or the Relay leg), and \
                this upgrade makes {c} the config's own chain: that side would never be ticked again (audit C3-06)"));
        }
    }
    Ok(out)
}

/// `post_upgrade`: the sides restored, their pin floors apart.
pub(crate) fn restore_sides(v: Vec<(u64, EthState)>) {
    let floors = v.iter().map(|(c, st)| (*c, st.pin_floors.map(|f| f.eth).unwrap_or_default())).collect();
    SIDE_FLOORS.with(|f| *f.borrow_mut() = floors);
    SIDES.with(|s| *s.borrow_mut() = v.into_iter().map(|(c, st)| (c, EthState { pin_floors: None, ..st })).collect());
}

/// Admit a pin (or a nonce) of the side `side`'s chain against its floor: `PIN_FLOORS.eth` for the config's own chain.
fn with_floor<R>(side: Option<u64>, f: impl FnOnce(&mut crate::pin::PinFloor) -> R) -> R {
    match side {
        None => crate::canister::PIN_FLOORS.with(|p| f(&mut p.borrow_mut().eth)),
        Some(c) => SIDE_FLOORS.with(|s| {
            let mut v = s.borrow_mut();
            if !v.iter().any(|(x, _)| *x == c) {
                v.push((c, Default::default()));
            }
            f(&mut v.iter_mut().find(|(x, _)| *x == c).expect("just pushed").1)
        }),
    }
}

/// Whether our own home-chain transaction is out (for the canister-wide one-transaction rule).
pub(crate) fn evm_pending() -> bool {
    ETH.with(|e| e.borrow().evm_pending())
}

// ---- transforms ---------------------------------------------------------------------------------------------------------------------

/// A receipt at or below the pin in the context, projected to `{blockNumber, status, logs}` (`ethnft::receipt_projection`).
#[ic_cdk::query]
fn eth_receipt_logs(args: TransformArgs) -> HttpRequestResult {
    let pin = crate::pin::u64_of_context(&args.context).unwrap_or(0);
    let body = match serde_json::from_slice::<serde_json::Value>(&args.response.body).ok().and_then(|v| v.get("result").cloned()) {
        Some(r) => serde_json::json!({ "result": en::receipt_projection(&r, pin) }).to_string().into_bytes(),
        // review round 6 (B6-L-1): a rate limit is told apart (the scan does not count it against the receipt), as the log reads'
        None if rate_limited(&args) => crate::pin::busy_body(),
        None => br#"{"error":"unusable reply"}"#.to_vec(),
    };
    HttpRequestResult { status: args.response.status, body, headers: vec![] }
}

/// Review round 6 (B6-L-2): an HTTP 429 answer with no JSON-RPC result, or an error whose words say it is a rate limit.
fn rate_limited(args: &TransformArgs) -> bool {
    let no_result = serde_json::from_slice::<serde_json::Value>(&args.response.body).ok().and_then(|v| v.get("result").cloned()).is_none();
    no_result && (args.response.status == candid::Nat::from(429u32)
        || String::from_utf8(crate::pin::json_rpc_norm_body(&args.response.body)).unwrap_or_default().contains(r#""class":"busy""#))
}

/// A collection's `Transfer` logs projected to `[[txHash, blockNumber, tokenId, from], …]` (`ethnft::transfer_logs_body`, audit 2026-09-27-nft round 2:
/// the canister's own sale scan).
#[ic_cdk::query]
fn eth_transfer_logs(args: TransformArgs) -> HttpRequestResult {
    // review round 6 (B6-L-2): a 429 page is a rate limit (MEV Blocker's Cloudflare answers `error code: 1015`), never "unusable"
    let body = if rate_limited(&args) { crate::pin::busy_body() } else { en::transfer_logs_body(&args.response.body) };
    HttpRequestResult { status: candid::Nat::from(200u32), body, headers: vec![] }
}

/// Round 4 (B4-H-1): one block's marketplace sale logs that name the collection in the context (`ethnft::sale_logs_body`).
#[ic_cdk::query]
fn eth_sale_logs(args: TransformArgs) -> HttpRequestResult {
    let c: [u8; 20] = args.context.clone().try_into().unwrap_or([0; 20]);
    let body = if rate_limited(&args) { crate::pin::busy_body() } else { en::sale_logs_body(&args.response.body, &c) };
    HttpRequestResult { status: candid::Nat::from(200u32), body, headers: vec![] }
}

/// Round 4 (C4-M-1): a transaction projected for the payout judge (`evmsend::payout_view`).
#[ic_cdk::query]
fn eth_payout_view(args: TransformArgs) -> HttpRequestResult {
    let body = match serde_json::from_slice::<serde_json::Value>(&args.response.body).ok().and_then(|v| v.get("result").cloned()) {
        Some(r) => serde_json::json!({ "result": crate::evmsend::payout_view(&r) }).to_string().into_bytes(),
        None => br#"{"error":"unusable reply"}"#.to_vec(),
    };
    HttpRequestResult { status: args.response.status, body, headers: vec![] }
}

/// Our own receipt at or below the pin: `{blockNumber, status, gasUsed, effectiveGasPrice}`.
#[ic_cdk::query]
fn eth_our_receipt(args: TransformArgs) -> HttpRequestResult {
    let pin = crate::pin::u64_of_context(&args.context).unwrap_or(0);
    let body = match serde_json::from_slice::<serde_json::Value>(&args.response.body).ok().and_then(|v| v.get("result").cloned()) {
        Some(r) => {
            let block = r.get("blockNumber").and_then(|x| x.as_str()).and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok());
            let v = match block {
                Some(b) if b <= pin => serde_json::json!({ "blockNumber": r["blockNumber"], "status": r["status"], "gasUsed": r["gasUsed"],
                    "effectiveGasPrice": r["effectiveGasPrice"] }),
                _ => serde_json::Value::Null,
            };
            serde_json::json!({ "result": v }).to_string().into_bytes()
        }
        None => br#"{"error":"unusable reply"}"#.to_vec(),
    };
    HttpRequestResult { status: args.response.status, body, headers: vec![] }
}

/// The home chain's finalized block's timestamp, rounded down to the minute (`pin::finalized_time_body`, the H9 rule on this side).
#[ic_cdk::query]
fn finalized_time(args: TransformArgs) -> HttpRequestResult {
    let s = crate::pin::u64_of_context(&args.context).unwrap_or(60);
    HttpRequestResult { status: args.response.status, body: crate::pin::finalized_time_body(&args.response.body, s), headers: vec![] }
}

/// A transaction projected to `{from, to, input, blockNumber}`, lower-cased.
#[ic_cdk::query]
fn eth_tx_view(args: TransformArgs) -> HttpRequestResult {
    let body = match serde_json::from_slice::<serde_json::Value>(&args.response.body).ok().and_then(|v| v.get("result").cloned()) {
        Some(r) if r.is_null() => br#"{"result":null}"#.to_vec(),
        Some(r) => {
            let f = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
            serde_json::json!({ "result": { "from": f("from"), "to": f("to"), "input": f("input"), "blockNumber": f("blockNumber") } })
                .to_string().into_bytes()
        }
        None => br#"{"error":"unusable reply"}"#.to_vec(),
    };
    HttpRequestResult { status: args.response.status, body, headers: vec![] }
}

// ---- the Io ------------------------------------------------------------------------------------------------------------------------

/// Response caps (B-M-7): a few-word `eth_call`, and the two receipt size classes.
pub const SMALL_READ_BYTES: u64 = 8_000;
pub const RECEIPT_SMALL_BYTES: u64 = 32_000;
pub const RECEIPT_LARGE_BYTES: u64 = 200_000;
/// The sale scan's `eth_getLogs` size classes (round 2): a chunk is asked at the small class, the larger only on the IC's own size
/// reject; over the largest, the flow halves the chunk.
pub const LOGS_BYTES: [u64; 3] = [64_000, 400_000, 2_000_000];

pub(crate) struct EthNet {
    home: CanisterIo,
    chain: EvmChain,
    cfg: EthNftConfig,
    pinned: Option<(crate::pin::Pin, u64)>,
    /// Round 4 (B4-H-1): the response bytes this tick's log reads reserved (every size class tried), the sale scan's budget.
    logs_bytes: u64,
    /// Review round 5 (B5-L-1): the same for the receipts read (both classes tried, every provider).
    receipt_bytes: u64,
    /// docs/v9/solana-migration.md §4: the Solana home, when `Init.home` names one (every home step and read goes there).
    sol: Option<crate::solhome::SolNet>,
    /// 2026-10-12: the side's chain when it is not the config's own (`SIDES`); `None`: the config's own side (`ETH`).
    side: Option<u64>,
    /// The whole configuration (every chain's collections), for the other sides' steps on the shared Solana line.
    all: EthNftConfig,
}

impl EthNet {
    /// The side of `chain`: the configuration of its collections, its chain's providers, Seaport and fee caps.
    fn new_on(chain: u64) -> Result<Self, String> {
        let all = config().ok_or("eth_nft is not configured")?;
        let cfg = all.on(chain);
        let side = (chain != all.chain_id).then_some(chain);
        let sol = crate::canister::cfg().home.as_ref().map(crate::solhome::SolNet::new).transpose()?;
        Ok(EthNet { home: CanisterIo::new()?, chain: nn::chain(chain)?, cfg, pinned: None, logs_bytes: 0, receipt_bytes: 0, sol, side, all })
    }
    /// Round 4 (C4-H-1): the Ethereum pin is never below one already read (`canister::PIN_FLOORS.eth`), so no read of the flow goes
    /// back across a bucket (the balance floors of E3-M-1 stay: they also name the block of our own last mined transaction).
    async fn pin(&mut self) -> Result<(crate::pin::Pin, u64), String> {
        if let Some(p) = &self.pinned {
            return Ok(p.clone());
        }
        let p = nn::pin_on(&self.chain).await?;
        let what = if self.side.is_some() { format!("chain {}", self.chain.chain_id) } else { "Ethereum".to_string() };
        with_floor(self.side, |f| f.admit_pin(&what, p.0.number))?;
        self.pinned = Some(p.clone());
        Ok(p)
    }
    /// One `eth_getLogs` through the size classes, counting what each class reserved (B4-H-1).
    async fn logs_classes(&mut self, params: serde_json::Value, transform: &str, context: Vec<u8>) -> Result<Vec<serde_json::Value>, String> {
        let mut got = Err(String::new());
        for max in LOGS_BYTES {
            self.logs_bytes = self.logs_bytes.saturating_add(max.saturating_mul(self.chain.rpc_urls.len() as u64));
            // review round 7 (B7-L-1): `deep` only when every provider says it
            got = nn::ask_scan(&self.chain, "eth_getLogs", params.clone(), transform, context.clone(), max).await;
            match &got {
                Err(e) if e.contains(crate::evmread::SIZE_REJECT) => continue,
                _ => break,
            }
        }
        got
    }
    async fn us(&self) -> Result<[u8; 20], String> {
        Ok(crate::canister::ours().await?.0)
    }
    async fn call(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        let (p, _) = self.pin().await?;
        nn::call_at(&self.chain, &to, data, &p).await
    }
    /// A read whose answer is a few words (a counter, a status, an owner, an approval): a smaller response cap, and so fewer cycles
    /// (audit 2026-09-27-nft, B-M-7; the cap counts the provider's headers too).
    async fn call_small(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        let (p, _) = self.pin().await?;
        hex_bytes(&nn::agreed(&self.chain, "eth_call", serde_json::json!([{ "to": h0x(&to), "data": h0x(data) }, crate::pin::at(&p)]),
            SMALL_READ_BYTES).await?)
    }
    async fn fees(&mut self, from: [u8; 20], to: [u8; 20], value: u128, data: &[u8]) -> Result<(u128, u128, u64, u64), String> {
        let (p, _) = self.pin().await?;
        let tx = serde_json::json!({ "from": h0x(&from), "to": h0x(&to), "value": format!("0x{value:x}"), "data": h0x(data) });
        // audit 2026-09-27-nft B-M-2 / C-M-1: a revert is told apart from a failure. Only when EVERY provider answers that the call
        // reverts (a JSON-RPC error saying so, through the normalizing transform) is it "refused (simulation)"; a transport error,
        // a rate limit, an unusable body or providers that disagree is "unread", which the flow takes as a wait, never a decision
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_call", "params": [tx.clone(), crate::pin::at(&p)] }).to_string();
        let (mut ok, mut reverts, mut fails) = (Vec::new(), Vec::new(), Vec::new());
        for url in &self.chain.rpc_urls {
            match crate::canister::rpc_once_with(url, body.clone(), "json_rpc_norm", vec![], 16_000).await {
                Ok(v) => ok.push(v),
                Err(e) if crate::pin::said_reverted(&e) => reverts.push(e),
                Err(e) => fails.push(e),
            }
        }
        if reverts.len() == self.chain.rpc_urls.len() {
            return Err(format!("refused (simulation): it reverts at the pin on every provider: {}", reverts.join("; ")));
        }
        if !reverts.is_empty() || !fails.is_empty() || ok.iter().any(|a| *a != ok[0]) {
            return Err(format!("unread (simulation): the providers did not all answer alike ({} ok, {} revert, {} failed: {}): nothing is \
                decided", ok.len(), reverts.len(), fails.len(), fails.join("; ")));
        }
        let mut estimate = 0u128;
        for a in nn::ask(&self.chain, "eth_estimateGas", serde_json::json!([tx, format!("0x{:x}", p.number)]), "json_rpc_norm", vec![], 8_000).await? {
            estimate = estimate.max(hex_u128(&a)?);
        }
        let (max_fee, prio, gas) = crate::chains::fees_and_gas(&self.chain, p.base_fee, estimate as u64)?;
        let nonce = nn::quantity_at(&self.chain, "eth_getTransactionCount", &from, &p).await? as u64;
        Ok((max_fee, prio, gas, nonce))
    }
}

impl EthIo for EthNet {
    /// The one Relay leg is held by a Bitcoin-side conversion of any Inscriptions collection (B-L-6, Ordinals multi-collection), or (2026-10-12) by another chain's side: Relay's payouts and refunds land
    /// in the same Solana balance, so one order is watched at a time canister-wide.
    fn frog_relay_busy(&self) -> bool {
        crate::frognet::FROGS.with(|f| f.try_borrow().map_or(true, |s| crate::frogsol::multi::holds_relay(&s))) || relay_busy_except(self.side)
    }
    fn other_side_step(&self, step: &crate::ethflow::Step) -> bool {
        let others = self.all.chains().into_iter().filter(|c| *c != self.cfg.chain_id);
        others.into_iter().any(|c| {
            let side = (c != self.all.chain_id).then_some(c);
            let cfg = self.all.on(c);
            match side {
                None => ETH.with(|e| e.try_borrow().map_or(true, |st| crate::ethflow::owns_step(&st, &cfg, step))),
                Some(c) => SIDES.with(|s| s.try_borrow().map_or(true, |v| v.iter().filter(|(x, _)| *x == c)
                    .any(|(_, st)| crate::ethflow::owns_step(st, &cfg, step)))),
            }
        })
    }
    fn frog_relay_waiting(&self) -> bool {
        crate::frognet::FROGS.with(|f| f.try_borrow().map_or(true, |s| crate::frogsol::multi::wants_relay(&s)))
    }
    fn frog_reimbursement_owed(&self) -> bool {
        crate::frognet::FROGS.with(|f| crate::floornet::FLOORS.with(|l| match (f.try_borrow(), l.try_borrow()) {
            (Ok(s), Ok(fl)) => crate::frogsol::multi::states(&s).into_iter().any(|s| crate::convflow::reimbursement_owed(s, &fl).is_some()),
            _ => true,
        }))
    }
    fn now(&self) -> u64 {
        ic_cdk::api::time() / 1_000_000_000
    }
    fn home_chain(&self) -> u64 {
        if self.sol.is_some() {
            return crate::relaysol::RELAY_SOLANA;
        }
        Io::chain_id(&self.home)
    }
    fn eth_chain(&self) -> u64 {
        self.cfg.chain_id
    }
    fn config(&self) -> Option<EthNftConfig> {
        Some(self.cfg.clone())
    }
    fn paused(&self) -> bool {
        crate::canister::not_paused().is_err()
    }
    fn eth_max_fee(&self) -> u128 {
        self.chain.max_fee_wei as u128
    }
    fn cycles_ok(&self) -> bool {
        ic_cdk::api::canister_cycle_balance() >= crate::ethflow::EVIDENCE_CYCLES_FLOOR
    }
    /// docs/v7/dormant-collections.md §11: what the canister can spend (the freezing reserve excluded). Nobody can take cycles out of the
    /// canister: a balance only falls by what the canister itself spends, and only rises by a top-up.
    fn cycles_balance(&self) -> Option<u128> {
        Some(ic_cdk::api::canister_liquid_cycle_balance())
    }
    fn save(&mut self, st: &EthState) {
        with_side(self.side, |e| *e = st.clone());
    }
    fn with_caps<R>(&mut self, f: impl FnOnce(&crate::custodycap::CapPolicy, &mut crate::custodycap::Book) -> R) -> R {
        crate::canister::with_caps(f)
    }
    fn outflow_check(&mut self, key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) -> Result<(), String> {
        crate::canister::outflow_check(key, collection, flow, amount)
    }
    fn outflow_record(&mut self, key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) {
        crate::canister::outflow_record(key, collection, flow, amount)
    }

    async fn our_evm(&mut self) -> Result<[u8; 20], String> {
        self.us().await
    }
    async fn home_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        if self.sol.is_some() {
            return Err("the Solana home has no EVM reads".into());
        }
        self.home.eth_read(to, data).await
    }
    async fn home_clock(&mut self) -> Result<Clock, String> {
        if let Some(s) = self.sol.as_mut() {
            let (time, slot) = s.clock().await?;
            return Ok(Clock { time, parent_block: slot });
        }
        crate::frognet::home_clock(&mut self.home).await
    }
    async fn home_balance(&mut self) -> Result<u128, String> {
        if let Some(s) = self.sol.as_mut() {
            return s.balance().await;
        }
        self.home.evm_balance().await
    }
    async fn home_sign(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> {
        let nonce = self.home.evm_nonce().await?;
        let base = self.home.base_fee().await?;
        let est = self.home.estimate_gas(to, value, data).await.map_err(|e| format!("would not succeed now: {e}"))?;
        let tx = Tx1559 { chain_id: Io::chain_id(&self.home), nonce, max_priority_fee: 0,
            max_fee: base.saturating_mul(crate::looper::FEE_MULTIPLE), gas: est + est * crate::looper::GAS_MARGIN_PCT / 100, to, value,
            data: data.to_vec() };
        let (y, r, s) = self.home.sign_evm(tx.signing_hash()).await?;
        let raw = tx.signed(y, &r, &s);
        Ok((raw.clone(), crate::evmtx::keccak(&raw), nonce))
    }
    async fn home_broadcast(&mut self, raw: &[u8]) {
        let _ = self.home.send_evm(raw).await;
    }
    async fn home_seen(&mut self, hash: [u8; 32]) -> EvmView {
        self.home.evm_seen(hash).await
    }
    async fn home_nonce(&mut self) -> Result<u64, String> {
        self.home.evm_nonce().await
    }
    async fn home_finalized_time(&mut self) -> Result<u64, String> {
        if let Some(s) = self.sol.as_mut() {
            return Ok(s.clock().await?.0);
        }
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_getBlockByNumber", "params": ["finalized", false] }).to_string();
        let mut lo = u64::MAX;
        for url in cfg().rpc_urls {
            let v = crate::canister::rpc_once_with(&url, body.clone(), "finalized_time", crate::pin::context_u64(60), 16_000).await?;
            lo = lo.min(hex_u128(&v)? as u64);
        }
        if lo == u64::MAX {
            return Err("no home-chain provider".into());
        }
        Ok(lo)
    }

    async fn eth_pin(&mut self) -> Result<EthPin, String> {
        let (p, ts) = self.pin().await?;
        Ok(EthPin { number: p.number, timestamp: ts, base_fee: p.base_fee })
    }
    async fn eth_finalized(&mut self) -> Result<u64, String> {
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_getBlockByNumber", "params": ["finalized", false] }).to_string();
        let mut lo = u64::MAX;
        for url in &self.chain.rpc_urls {
            let v = crate::canister::rpc_once_with(url, body.clone(), "finalized_bucket", crate::pin::context_u64(self.chain.bucket),
                nn::block_view_bytes(self.chain.chain_id)).await?;
            lo = lo.min(hex_u128(&v)? as u64);
        }
        Ok(lo)
    }
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        self.call(to, data).await
    }
    async fn eth_balance(&mut self) -> Result<u128, String> {
        let (p, _) = self.pin().await?;
        let us = self.us().await?;
        nn::quantity_at(&self.chain, "eth_getBalance", &us, &p).await
    }
    async fn eth_nonce(&mut self) -> Result<u64, String> {
        let (p, _) = self.pin().await?;
        let us = self.us().await?;
        let n = nn::quantity_at(&self.chain, "eth_getTransactionCount", &us, &p).await? as u64;
        let what = if self.side.is_some() { format!("chain {}", self.chain.chain_id) } else { "Ethereum".to_string() };
        with_floor(self.side, |f| f.admit_nonce(&what, n))
    }
    async fn eth_receipt(&mut self, hash: [u8; 32]) -> Result<Option<OurReceipt>, String> {
        let (p, _) = self.pin().await?;
        let r = nn::one(&self.chain, "eth_getTransactionReceipt", nn::ask(&self.chain, "eth_getTransactionReceipt", serde_json::json!([h0x(&hash)]),
            "eth_our_receipt", crate::pin::context_u64(p.number), 64_000).await?)?;
        if r.is_null() {
            return Ok(None);
        }
        let n = |k: &str| hex_u128(&r[k]);
        Ok(Some(OurReceipt { block: n("blockNumber")? as u64, success: r["status"] == "0x1",
            fee: n("gasUsed")?.saturating_mul(n("effectiveGasPrice")?) }))
    }
    async fn eth_plan_buy(&mut self, order_hex: &str, collection: [u8; 20], cap: u128) -> Result<EthPlan, String> {
        let (plan, token) = nn::check_any(&self.chain, collection, cap, order_hex).await?;
        let p = |s: &str| s.parse::<u128>().map_err(|_| "an unreadable plan".to_string());
        Ok(EthPlan { to: crate::evmtx::parse_address(&plan.to)?, value: p(&plan.total_wei)?, data: hex_bytes(&serde_json::json!(plan.calldata))?,
            gas: plan.gas, max_fee: p(&plan.max_fee_wei)?, priority_fee: p(&plan.priority_fee_wei)?, nonce: plan.nonce, token,
            total: p(&plan.total_wei)? })
    }
    async fn eth_plan_transfer(&mut self, collection: [u8; 20], token: Word, dest: [u8; 20]) -> Result<EthPlan, String> {
        let us = self.us().await?;
        let owner = sp::decode_address(&self.call(collection, &sp::call1(sp::OWNER_OF, token)).await?)?;
        if owner != us {
            return Err(format!("the token's owner at the pin is 0x{}, not us", hex::encode(owner)));
        }
        let data = en::transfer_from_call(&us, &dest, token);
        let (max_fee, prio, gas, nonce) = self.fees(us, collection, 0, &data).await?;
        let (p, _) = self.pin().await?;
        let bal = nn::quantity_at(&self.chain, "eth_getBalance", &us, &p).await?;
        if bal < max_fee.saturating_mul(gas as u128) {
            return Err(format!("our Ethereum balance {bal} does not cover the transfer's gas (the gas pot is empty)"));
        }
        Ok(EthPlan { to: collection, value: 0, data, gas, max_fee, priority_fee: prio, nonce, token, total: 0 })
    }
    async fn eth_sign(&mut self, tx: &Tx1559) -> Result<Vec<u8>, String> {
        let s = crate::canister::sign_evm_recoverable(tx.signing_hash()).await?;
        let b = |h: &str| -> Result<[u8; 32], String> { hex::decode(h).map_err(|e| e.to_string())?.try_into().map_err(|_| "not 32 bytes".into()) };
        Ok(tx.signed(s.recovery_id, &b(&s.r)?, &b(&s.s)?))
    }
    async fn eth_broadcast(&mut self, raw: &[u8]) {
        let hash = h0x(&crate::evmtx::keccak(raw));
        let raw_hex = h0x(raw);
        for url in &self.chain.rpc_urls {
            let _ = crate::canister::send_raw_once(url, &raw_hex, &hash).await;
        }
    }
    async fn eth_listing(&mut self, order_hex: &str, collection: [u8; 20]) -> Result<Seen, String> {
        let l = en::Listing::parse_hex(order_hex)?;
        let t = l.terms(&collection)?;
        // B-L-1: evidence is a listing anyone can fill (open, or restricted through an allow-listed zone)
        let (zones, _) = self.chain.zones()?;
        t.fillable(&zones.iter().map(|(z, _)| *z).collect::<Vec<_>>())?;
        let (_, ts) = self.pin().await?;
        let seaport = self.chain.seaport_addr()?;
        let counter: Word = self.call_small(seaport, &sp::call1(sp::GET_COUNTER, sp::word_addr(&t.offerer))).await?
            .get(..32).ok_or("getCounter returned nothing")?.try_into().expect("32");
        let hash = l.order_hash(counter);
        let (_, cancelled, filled, size) = sp::decode_order_status(&self.call_small(seaport, &sp::call1(sp::GET_ORDER_STATUS, hash)).await?)?;
        let owner = sp::decode_address(&self.call_small(collection, &sp::call1(sp::OWNER_OF, t.token)).await?)?;
        let operator = if t.conduit_key == [0; 32] {
            seaport
        } else {
            let r = self.call_small(en::CONDUIT_CONTROLLER, &sp::call1(en::GET_CONDUIT, t.conduit_key)).await?;
            if !crate::abi::bool_at(&r, 0, 1)? {
                return Err("the order's conduit does not exist".into());
            }
            crate::abi::addr_at(&r, 0, 0)?
        };
        let all = self.call_small(collection, &crate::abi::encode_call(en::IS_APPROVED_FOR_ALL, &[crate::abi::Arg::Word(sp::word_addr(&t.offerer)),
            crate::abi::Arg::Word(sp::word_addr(&operator))])).await.ok().and_then(|r| crate::abi::bool_at(&r, 0, 0).ok()).unwrap_or(false);
        // the single approval is read only when the operator is not approved for all (one read fewer, B-M-7)
        let one = !all && self.call_small(collection, &sp::call1(en::GET_APPROVED, t.token)).await.ok().and_then(|r| crate::abi::addr_at(&r, 0, 0).ok()) == Some(operator);
        let signed = sp::offerer_signed(&hash, &t.offerer, l.signature(), self.chain.chain_id, &seaport).unwrap_or(false);
        en::judge_listing(&t, &en::ListingFacts { pin_timestamp: ts, owner, cancelled, filled, size, approved: all || one, signed },
            self.chain.min_order_life_s)
    }
    async fn eth_sale_receipt(&mut self, hash: [u8; 32]) -> Result<Option<ReceiptV>, String> {
        let (p, _) = self.pin().await?;
        // B-M-7: reserved by size class, 32 kB first (a real sale's receipt measured 4,161 B), the 200 kB class only when the IC
        // itself refused the answer for its size
        let mut got = Err(String::new());
        for max in [RECEIPT_SMALL_BYTES, RECEIPT_LARGE_BYTES] {
            self.receipt_bytes = self.receipt_bytes.saturating_add(max.saturating_mul(self.chain.rpc_urls.len() as u64));
            got = nn::ask(&self.chain, "eth_getTransactionReceipt", serde_json::json!([h0x(&hash)]), "eth_receipt_logs",
                crate::pin::context_u64(p.number), max).await;
            match &got {
                Err(e) if e.contains(crate::evmread::SIZE_REJECT) => continue,
                _ => break,
            }
        }
        let r = nn::one(&self.chain, "eth_getTransactionReceipt", got?)?;
        en::receipt_of(&r)
    }
    async fn eth_transfer_txs(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<([u8; 32], u64)>, String> {
        Ok(self.eth_transfer_moves(collection, from, to).await?.into_iter().map(|m| (m.hash, m.block)).collect())
    }
    async fn eth_transfer_moves(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<en::Move>, String> {
        let (p, _) = self.pin().await?;
        if to > p.number || from > to {
            return Err(format!("blocks {from}..{to} are not under the pin {}", p.number));
        }
        let params = serde_json::json!([{ "address": h0x(&collection), "topics": [h0x(&en::TRANSFER)], "fromBlock": format!("0x{from:x}"),
            "toBlock": format!("0x{to:x}") }]);
        let got = self.logs_classes(params, "eth_transfer_logs", vec![]).await;
        en::transfer_moves_of(&nn::one(&self.chain, "eth_getLogs", got?)?)
    }
    async fn eth_sale_moves(&mut self, collection: [u8; 20], block: u64) -> Result<Vec<en::Move>, String> {
        self.eth_sale_moves_range(collection, block, block).await
    }
    /// Review round 5 (B5-M-1 / B5-L-2): the marketplaces' sale logs of `[from, to]` that name `collection`, one read per marketplace
    /// (Seaport 1.5 / 1.6 `OrderFulfilled`; Blur v2 `Execution721*`), each filtered by address and topic: a padded fill on one
    /// marketplace no longer hides the other's sales. An error only when neither could be read.
    async fn eth_sale_moves_range(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<en::Move>, String> {
        Ok(self.eth_sale_moves_whole(collection, from, to).await?.0)
    }
    /// Review round 6 (B6-L-2): the same, and whether BOTH marketplaces answered: a block whose Seaport read was refused (a rate
    /// limit, or Seaport's global volume over the response cap) while Blur's answered is not read yet, and is never remembered as read.
    async fn eth_sale_moves_whole(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<(Vec<en::Move>, bool), String> {
        let (p, _) = self.pin().await?;
        if to > p.number || from > to {
            return Err(format!("blocks {from}..{to} are not under the pin {}", p.number));
        }
        let t = |x: &[u8]| h0x(x);
        // audit C3-11: only the venues deployed on this side's chain (`ethnft::sale_venues`): on Robinhood Chain, Seaport 1.6 alone
        let (seaports, blur) = en::sale_venues(self.chain.chain_id);
        let mut queries = vec![serde_json::json!([{ "address": seaports.iter().map(|a| t(a)).collect::<Vec<_>>(), "topics": [[t(&en::ORDER_FULFILLED)]],
            "fromBlock": format!("0x{from:x}"), "toBlock": format!("0x{to:x}") }])];
        if blur {
            queries.push(serde_json::json!([{ "address": t(&en::BLUR_EXCHANGE_V2), "topics": [[t(&en::BLUR_721), t(&en::BLUR_721_TAKER_FEE),
                t(&en::BLUR_721_MAKER_FEE)]], "fromBlock": format!("0x{from:x}"), "toBlock": format!("0x{to:x}") }]));
        }
        let n = queries.len();
        let (mut all, mut errs) = (Vec::new(), Vec::new());
        for q in queries {
            let got = self.logs_classes(q, "eth_sale_logs", collection.to_vec()).await;
            match got.and_then(|g| nn::one(&self.chain, "eth_getLogs", g)).and_then(|v| en::transfer_moves_of(&v)) {
                Ok(v) => all.extend(v),
                Err(e) => errs.push(e),
            }
        }
        if errs.len() == n {
            return Err(errs.join("; "));
        }
        all.sort_by(|a: &en::Move, b: &en::Move| a.block.cmp(&b.block).then(a.hash.cmp(&b.hash)));
        all.dedup_by(|a, b| a.hash == b.hash);
        Ok((all, errs.is_empty()))
    }
    fn logs_bytes(&self) -> u64 {
        self.logs_bytes
    }
    fn logs_read_worst(&self) -> u64 {
        LOGS_BYTES.iter().sum::<u64>().saturating_mul(self.chain.rpc_urls.len() as u64)
    }
    fn receipt_bytes(&self) -> u64 {
        self.receipt_bytes
    }
    fn receipt_read_worst(&self) -> u64 {
        (RECEIPT_SMALL_BYTES + RECEIPT_LARGE_BYTES).saturating_mul(self.chain.rpc_urls.len() as u64)
    }
    async fn home_nonce_tx(&mut self, nonce: u64, from: Option<u64>) -> Result<Option<[u8; 32]>, String> {
        crate::canister::home_nonce_tx(nonce, from).await
    }
    async fn home_pin(&mut self) -> Result<Option<u64>, String> {
        if self.sol.is_some() {
            return Ok(None);
        }
        Ok(Some(self.home.pin().await?.number))
    }
    async fn payout_logs(&mut self, home: bool, hash: [u8; 32]) -> Result<Vec<en::LogV>, String> {
        let r = if home { crate::canister::home_receipt_logs(hash).await? } else { self.eth_sale_receipt(hash).await? };
        Ok(r.ok_or("the payout's receipt is not at the pin")?.logs)
    }
    async fn payout_tx(&mut self, home: bool, hash: [u8; 32]) -> Result<Option<crate::ethflow::PayoutTx>, String> {
        if home && self.sol.is_some() {
            return Err("a late payout on the Solana home is not read by this build (phase B2): its order stays written off".into());
        }
        if home {
            return crate::canister::home_payout_tx(hash).await;
        }
        let Some(r) = self.eth_receipt(hash).await? else { return Ok(None) };
        let t = nn::one(&self.chain, "eth_getTransactionByHash", nn::ask(&self.chain, "eth_getTransactionByHash", serde_json::json!([h0x(&hash)]),
            "eth_payout_view", vec![], 64_000).await?)?;
        if t.is_null() {
            return Err("the transaction's receipt is there, the transaction is not: nothing is believed".into());
        }
        crate::ethflow::PayoutTx::of(&t, r.block, r.success).map(Some)
    }
    async fn eth_plan_call(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<EthPlan, String> {
        let us = self.us().await?;
        let (max_fee, prio, gas, nonce) = self.fees(us, to, value, data).await?;
        let (p, _) = self.pin().await?;
        let bal = nn::quantity_at(&self.chain, "eth_getBalance", &us, &p).await?;
        if bal < value.saturating_add(max_fee.saturating_mul(gas as u128)) {
            return Err(format!("our Ethereum balance {bal} does not cover {value} wei and the call's gas"));
        }
        Ok(EthPlan { to, value, data: data.to_vec(), gas, max_fee, priority_fee: prio, nonce, token: [0; 32], total: value })
    }
    async fn eth_deposit_tx(&mut self, hash: [u8; 32]) -> Result<Option<([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>, String> {
        let t = nn::one(&self.chain, "eth_getTransactionByHash", nn::ask(&self.chain, "eth_getTransactionByHash", serde_json::json!([h0x(&hash)]),
            "eth_tx_view", vec![], 64_000).await?)?;
        if t.is_null() {
            return Ok(None);
        }
        let Some(r) = self.eth_sale_receipt(hash).await? else { return Ok(None) };
        let a = |k: &str| crate::evmtx::parse_address(t[k].as_str().unwrap_or(""));
        Ok(Some((a("from")?, a("to")?, hex_bytes(&t["input"])?, r)))
    }
}


crate::sol_io_via!(EthNet, sol);

// ---- the tick's hook ----------------------------------------------------------------------------------------------------------------

/// The pass, run by `tick` after the frog side's when `eth_nft` is configured: the config's own chain's side, then (2026-10-12) each
/// other chain's side in the order its first collection is listed.
pub(crate) async fn tick(others_pending: bool) -> Vec<String> {
    let chains = match config() {
        Some(c) => c.chains(),
        None => return vec!["eth: not configured: eth_nft is not configured".into()],
    };
    let mut out = Vec::new();
    for chain in chains {
        let mut io = match EthNet::new_on(chain) {
            Ok(io) => io,
            Err(e) => {
                out.push(format!("eth: not configured: {e}"));
                continue;
            }
        };
        let side = io.side;
        let mut st = with_side(side, |e| e.clone());
        let lines = crate::ethflow::tick(&mut io, &mut st, others_pending).await;
        with_side(side, |e| *e = st);
        match side {
            None => out.extend(lines),
            Some(c) => out.extend(lines.into_iter().map(|l| format!("chain {c}: {l}"))),
        }
    }
    out
}

// ---- endpoints --------------------------------------------------------------------------------------------------------------------

fn now() -> u64 {
    ic_cdk::api::time() / 1_000_000_000
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    hex::decode(s.trim().trim_start_matches("0x")).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| format!("{s:?} is not 32 bytes of hex"))
}

/// The runner hands in Relay's quote for a funded buy (4663 → Ethereum, to our own address). Judged at the next tick
/// (`relayeth::judge`); nothing in it is trusted that is not recomputed.
#[ic_cdk::update]
pub fn eth_relay_quote(buy: u64, quote_json: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    with_buy(buy, |st| crate::ethflow::put_quote(st, buy, quote_json.into_bytes(), now()))
}

/// The runner hands in a Seaport listing for an arrived buy (`fulfillBasicOrder` / `fulfillAdvancedOrder` calldata). Judged at
/// the next tick against our floor, what arrived, and every Seaport rule at an Ethereum pin.
#[ic_cdk::update]
pub fn eth_listing(buy: u64, order_hex: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    with_buy(buy, |st| crate::ethflow::put_listing(st, buy, order_hex, now()))
}

/// The runner hands in floor evidence for a collection: up to 8 Seaport listings. Sale hashes (up to 12) are still accepted for the
/// interface's sake and NOT used (audit 2026-09-27-nft round 2, A-M-1 / B2-M-1): the canister reads the collection's sales itself.
#[ic_cdk::update]
pub fn eth_floor_evidence(collection: String, listings: Vec<String>, sales: Vec<String>) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let all = config().ok_or("refused: eth_nft is not configured")?;
    let c = crate::evmtx::parse_address(&collection)?;
    let sales = sales.iter().map(|s| hex32(s)).collect::<Result<Vec<_>, _>>()?;
    // 2026-10-12: the evidence goes to the side of the collection's chain
    let chain = all.chain_of_addr(&c).unwrap_or(all.chain_id);
    let cfg = all.on(chain);
    with_side(Some(chain), |st| crate::ethflow::put_evidence(st, &cfg, c, listings, sales, now()))
}

/// A holder's deposit claim (docs §2.4): refused while `eth_nft.deposits` is off.
#[ic_cdk::update]
pub fn eth_deposit(tx_hash: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let cfg = config().ok_or("refused: eth_nft is not configured")?;
    let tx = hex32(&tx_hash)?;
    ETH.with(|e| crate::ethflow::put_deposit(&mut e.borrow_mut(), &cfg, tx, now()))
}

/// Our balance on the home chain and its gas keep: wei on the EVM home; on the Solana home (docs/v9 §4) lamports at our Solana address.
async fn home_balance_and_keep() -> Result<(u128, u128), String> {
    match crate::canister::cfg().home {
        Some(h) => Ok((crate::solhome::SolNet::new(&h)?.balance().await?, crate::solhome::SOL_GAS_KEEP_LAMPORTS)),
        None => Ok((CanisterIo::new()?.evm_balance().await?, crate::ethflow::GAS_KEEP_WEI)),
    }
}

/// The operator's exit: a funded (or stuck while bridging) buy's ETH on the home chain goes back to its module.
#[ic_cdk::update]
pub async fn eth_give_back(buy: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    let (balance, keep) = home_balance_and_keep().await?;
    crate::canister::not_ticking()?;
    with_buy(buy, |st| crate::ethflow::give_back_now_keeping(st, buy, balance, now(), keep))
}

/// Review round 4 (C4-M-1): the operator hands in the transaction that paid out a written-off Relay order late (Relay's status names
/// it), on chain 4663 or on Ethereum (`chain_id`). `wei` is what it paid only when the transaction itself does not show it (a refund
/// paid from inside the depository's call; 0 otherwise). Judged at the next tick against that order alone (`ethflow::judge_payout`)
/// and credited to its own buy and collection.
#[ic_cdk::update]
pub fn eth_credit_payout(buy: u64, chain_id: u64, tx_hash: String, wei: candid::Nat) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let cfg = config().ok_or("refused: eth_nft is not configured")?;
    let home = crate::canister::cfg().chain_id;
    let solana = crate::canister::cfg().home.is_some() && chain_id == crate::relaysol::RELAY_SOLANA;
    // 2026-10-12: the NFT chain is the buy's side's (Ethereum, or Robinhood Chain for its own collections)
    let side = side_of_buy(buy).unwrap_or(cfg.chain_id);
    if chain_id != home && chain_id != side && !solana {
        return Err(format!("refused: buy {buy}'s payout is on chain {home} or {side}, not {chain_id}"));
    }
    let wei: u128 = wei.0.try_into().map_err(|_| "refused: the amount does not fit".to_string())?;
    // phase B2: on the Solana home a payout on Solana is named by its base58 signature (chain id: Relay's Solana id), its amount in lamports
    if crate::canister::cfg().home.is_some() && chain_id == crate::relaysol::RELAY_SOLANA {
        return with_buy(buy, |st| crate::ethflow::put_sol_payout(st, buy, &tx_hash, wei, now()));
    }
    let tx = hex32(&tx_hash)?;
    let home_pay = payout_is_home(chain_id, home, side, crate::canister::cfg().home.is_some())?;
    with_buy(buy, |st| crate::ethflow::put_payout(st, buy, home_pay, tx, wei, now()))
}

/// Audit C3-04: whether a late payout on `chain_id` is read on the HOME chain, decided after the side's chain: on the books home the
/// install's `chain_id` is still 4663, which is also the Robinhood Chain side's, and a payout on the side's own chain is the side's
/// (read on its chain, credited to its buy), never the home's. On the Solana home a payout on the install's chain that is not the side's
/// is refused (that home reads none there).
pub(crate) fn payout_is_home(chain_id: u64, home: u64, side: u64, solana_home: bool) -> Result<bool, String> {
    if chain_id == side {
        return Ok(false);
    }
    if chain_id == home && solana_home {
        return Err(format!("refused: chain {chain_id} is neither the buy's NFT chain ({side}) nor Solana: the Solana home reads no payout there"));
    }
    Ok(chain_id == home)
}

/// Review round 5 (C5-M-1): the operator releases a written-off Relay order's hold on the Relay leg, once Relay's own records show where
/// its ETH went (`ethflow::late_release`). New Relay orders start again.
#[ic_cdk::update]
pub fn eth_late_release(buy: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    with_buy(buy, |st| crate::ethflow::late_release(st, buy, now()))
}

/// Review round 5 (F5-M-1): the operator's word, after looking, on the pending 4663 transaction that no provider shows, whose nonce is
/// used and that the canister could not identify (`ethflow::home_clear`): `mined` or not. Applied at the next tick unless the chain
/// shows it first.
#[ic_cdk::update]
pub fn eth_home_clear(nonce: u64, mined: bool) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    ETH.with(|e| crate::ethflow::home_clear(&mut e.borrow_mut(), nonce, mined, now()))
}

/// The operator's exit from a halted Ethereum line (a transaction that is not ours took its nonce), after looking: the record goes
/// back to before its signature and is judged again from the start (`ethflow::line_clear`).
#[ic_cdk::update]
pub fn eth_line_clear() -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    ETH.with(|e| crate::ethflow::line_clear(&mut e.borrow_mut()))
}

/// 2026-10-12: `eth_line_clear` for the side of `chain_id` (a Robinhood Chain collection's line is its own, on 4663).
#[ic_cdk::update]
pub fn eth_line_clear_on(chain_id: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let all = config().ok_or("refused: eth_nft is not configured")?;
    if !all.chains().contains(&chain_id) {
        return Err(format!("refused: no collection is on chain {chain_id}"));
    }
    with_side(Some(chain_id), crate::ethflow::line_clear)
}

/// A stray held for the operator goes back to its return path, judged again with every check (`ethflow::rearm_stray`).
#[ic_cdk::update]
pub fn eth_rearm_stray(tx_hash: String, token_hex: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let all = config().ok_or("refused: eth_nft is not configured")?;
    let (tx, token) = (hex32(&tx_hash)?, hex32(&token_hex)?);
    // 2026-10-12: the stray is held by the side of the chain it came in on: each side is asked, the config's own first
    let mut first = None;
    for chain in all.chains() {
        let cfg = all.on(chain);
        match with_side(Some(chain), |st| crate::ethflow::rearm_stray(st, &cfg, tx, token)) {
            Ok(t) => return Ok(t),
            Err(e) => {
                first.get_or_insert(e);
            }
        }
    }
    Err(first.unwrap_or_else(|| "refused: no side holds it".into()))
}

/// docs/v7/dormant-collections.md §3.1: the operator asks for a collection's wake. It is queued; the caps (`max_active`, `max_waking`,
/// `wakes_per_day`, the cooldown) decide when it runs, as for a bonded request on the WakeDesk.
#[ic_cdk::update]
pub fn eth_wake(collection: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let cfg = config().ok_or("refused: eth_nft is not configured")?;
    let c = crate::evmtx::parse_address(&collection)?;
    ETH.with(|e| crate::ethflow::put_wake(&mut e.borrow_mut(), &cfg, c, now()))
}

/// docs/v7/dormant-collections.md §5 (dormant-review DR-M-2): the operator names the coin of a graduation queued at an ACTIVE collection's
/// module (its `GradQueued` event), for the fill hunt to try first. The factory, the module and the curve are read before `fill` is signed.
#[ic_cdk::update]
pub fn eth_fill(collection: String, coin: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let cfg = config().ok_or("refused: eth_nft is not configured")?;
    let (c, k) = (crate::evmtx::parse_address(&collection)?, crate::evmtx::parse_address(&coin)?);
    ETH.with(|e| crate::ethflow::put_fill(&mut e.borrow_mut(), &cfg, c, k, now()))
}

/// docs/v7/dormant-collections.md §4.2: every configured collection's life (`always-on`, `dormant`, `queued`, `waking`, `active`), with
/// the status the site shows ("warming up (~N min)"). For the runner (which hands in evidence only for what is awake), the monitor and
/// the site.
#[ic_cdk::query]
pub fn eth_nft_collections() -> Vec<crate::dormancy::CollectionLife> {
    let Some(cfg) = config() else { return vec![] };
    ETH.with(|e| crate::ethflow::collections_view(&e.borrow(), &cfg, now()))
}

/// docs/v7/dormant-collections.md §11: how many collections can be awake right now on the canister's own cycles balance (read now), how
/// many are and how many hold positions, and the wakes that fit. `None` without a dormancy.
#[ic_cdk::query]
pub fn eth_nft_capacity() -> Option<crate::dormancy::Capacity> {
    let cfg = config()?;
    let d = cfg.dormancy.clone()?;
    ETH.with(|e| crate::ethflow::capacity_view(&e.borrow(), &cfg, &d, Some(ic_cdk::api::canister_liquid_cycle_balance()), now()))
}

/// Everything this side keeps: the buys, deliveries, deposits, floor readings, both lines. For the runner, the monitor and the
/// reserves page.
#[ic_cdk::query]
pub fn eth_nft_state() -> EthState {
    ETH.with(|e| {
        let mut s = e.borrow().clone();
        // the inbox's bodies are the runner's own; the signed bytes are kept (the monitor matches them)
        s.inbox = Default::default();
        // round 4 (C4-H-1): the canister-wide pin floors, as they are now
        s.pin_floors = Some(crate::canister::pin_floors());
        s
    })
}

/// 2026-10-12: `eth_nft_state` of the side of `chain_id` (the config's own chain: `eth_nft_state` itself). `None`: no collection there.
#[ic_cdk::query]
pub fn eth_nft_state_of(chain_id: u64) -> Option<EthState> {
    let all = config()?;
    if chain_id == all.chain_id {
        return Some(eth_nft_state());
    }
    if !all.chains().contains(&chain_id) {
        return None;
    }
    let mut s = SIDES.with(|v| v.borrow().iter().find(|(c, _)| *c == chain_id).map(|(_, st)| st.clone())).unwrap_or_else(|| crate::ethflow::new_side(chain_id));
    s.inbox = Default::default();
    let eth = SIDE_FLOORS.with(|f| f.borrow().iter().find(|(c, _)| *c == chain_id).map(|(_, f)| *f)).unwrap_or_default();
    s.pin_floors = Some(crate::pin::PinFloors { home: crate::canister::pin_floors().home, eth });
    Some(s)
}

#[ic_cdk::query]
pub fn eth_nft_config() -> Option<EthNftConfig> {
    config()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-12: the other chains' sides cross an upgrade inside the thirteenth element, each with its own pin floor, and come back
    /// as they were; one that does not decode refuses. Any side's Relay leg is the one leg canister-wide.
    #[test]
    fn the_sides_cross_an_upgrade_with_their_own_pin_floors_and_share_the_one_relay_leg() {
        assert_eq!(sides_blob(), None, "no side: nothing saved (the thirteenth element is as before)");
        let mut rh = crate::ethflow::new_side(4663);
        rh.rr = 3;
        SIDES.with(|s| *s.borrow_mut() = vec![(4663, rh.clone())]);
        SIDE_FLOORS.with(|f| *f.borrow_mut() = vec![(4663, crate::pin::PinFloor { pin: 900_000_000, nonce: 4 })]);
        let blob = sides_blob().expect("one side");
        SIDES.with(|s| s.borrow_mut().clear());
        SIDE_FLOORS.with(|f| f.borrow_mut().clear());
        restore_sides(sides_of(Some(blob.clone())).unwrap());
        assert_eq!(SIDES.with(|s| s.borrow().clone()), vec![(4663, rh)]);
        assert_eq!(SIDE_FLOORS.with(|f| f.borrow().clone()), vec![(4663, crate::pin::PinFloor { pin: 900_000_000, nonce: 4 })]);
        assert!(sides_of(Some(vec![(4663, vec![1, 2, 3])])).unwrap_err().contains("chain 4663"));
        // the 4663 side's floor is its own: Ethereum's is untouched by a 4663 pin, and a 4663 pin under its floor is refused
        assert!(with_floor(Some(4663), |f| f.admit_pin("chain 4663", 899_999_999)).is_err());
        assert_eq!(crate::canister::PIN_FLOORS.with(|p| p.borrow().eth), Default::default());
        // the Relay leg: a 4663 buy in flight holds it for the config's own side and for the frog side; not for itself
        assert!(!relay_busy_except(None) && !any_relay_busy());
        SIDES.with(|s| s.borrow_mut()[0].1.buys.push(crate::ethflow::EthBuy { id: crate::ethflow::side_buy_base(4663), collection: [1; 20],
            module: [0; 20], wrapper: [0; 20], advanced: 0, attempts: 0, phase: crate::ethflow::BPhase::Depositing { order_id: [0; 32], value: 1,
            min_out: 1, deadline: 1 }, orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(), sweep: None, arrived_at: None,
            back_at: None, ended_at: None, late: None }));
        assert!(relay_busy_except(None) && any_relay_busy() && !relay_busy_except(Some(4663)));
        assert_eq!(side_of_buy(crate::ethflow::side_buy_base(4663)), Some(4663));
        assert_eq!(side_of_buy(0), None, "the config's own side");
        SIDES.with(|s| s.borrow_mut().clear());
        SIDE_FLOORS.with(|f| f.borrow_mut().clear());
    }

    /// Audit C3-04: on the books home (install chain 4663) a 4663 side's late payout on 4663 is the side's, read on its chain; an
    /// Ethereum side's payout on 4663 is refused there (no home payout is read on the Solana home); the EVM home is unchanged.
    #[test]
    fn fixed_c3_04_a_4663_sides_late_payout_on_4663_is_the_sides_not_the_homes() {
        assert_eq!(payout_is_home(4663, 4663, 4663, true), Ok(false), "the 4663 side's own chain");
        assert_eq!(payout_is_home(1, 4663, 1, true), Ok(false), "the Ethereum side's own chain");
        assert!(payout_is_home(4663, 4663, 1, true).unwrap_err().contains("Solana home"));
        assert_eq!(payout_is_home(4663, 4663, 1, false), Ok(true), "the EVM home: a payout on 4663 is the home's, as before");
        assert_eq!(payout_is_home(1, 4663, 1, false), Ok(false));
    }

    /// Audit C3-06: a saved side on the chain an upgrade makes the config's own is refused while it holds work, dropped when idle.
    #[test]
    fn fixed_c3_06_an_upgrade_onto_a_sides_chain_is_refused_while_it_holds_work() {
        let idle = crate::ethflow::new_side(4663);
        let mut busy = crate::ethflow::new_side(4663);
        busy.buys.push(crate::ethflow::EthBuy { id: crate::ethflow::side_buy_base(4663), collection: [1; 20], module: [0; 20], wrapper: [0; 20],
            advanced: 1, attempts: 0, phase: crate::ethflow::BPhase::Funded { wei: 1, since: 1 }, orders: vec![], deposits: vec![], ask: None,
            last: None, note: String::new(), sweep: None, arrived_at: None, back_at: None, ended_at: None, late: None });
        assert_eq!(sides_kept(vec![(4663, idle.clone())], Some(1)), Ok(vec![(4663, idle.clone())]), "the config stays on 1: kept");
        assert_eq!(sides_kept(vec![(4663, idle)], Some(4663)), Ok(vec![]), "idle and now the config's own chain: dropped");
        assert!(sides_kept(vec![(4663, busy)], Some(4663)).unwrap_err().contains("chain 4663"));
    }

    #[test]
    fn every_update_method_here_is_operator_only_and_the_writers_refuse_mid_tick() {
        let src = include_str!("ethnet.rs");
        let parts: Vec<&str> = src.split(concat!("#[ic_cdk::", "update]")).skip(1).collect();
        assert_eq!(parts.len(), 13, "every update is counted");
        for p in parts {
            let head: String = p.chars().take(500).collect();
            assert!(head.contains("only_operator();"), "unguarded: {head}");
            assert!(head.contains("not_paused()?"), "a hand-in or exit while paused: {head}");
            assert!(head.contains("not_ticking()?"), "a write to ticked state mid-tick: {head}");
        }
    }
}
