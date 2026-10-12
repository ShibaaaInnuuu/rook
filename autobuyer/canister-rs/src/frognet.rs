//! The real world for item sinks, the mint desk and holder deposits (docs/v3/curve.md, "Canister: …"): the buyer's pinned
//! reads and one-node-sends through `CanisterIo`, ord and the explorers through the custodian's own `CustodianIo`, the float's
//! key on its own derivation path, and the endpoints. Nothing is decided here: `frogflow.rs` sequences, the pure modules judge.

use std::cell::RefCell;

use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs, HttpRequestResult, TransformArgs,
    TransformContext, TransformFunc};

use crate::canister::{cfg, only_operator, CanisterIo};
use crate::custody::{Contents, Location, Outpoint, Output, Utxo};
use crate::deposits::{DPhase, DepositRec, TxView};
use crate::frogbuy::{FPhase, FrogRec};
use crate::frogflow::{FrogIo, FrogState, Offer, ProtectedOffer};
use crate::itemsink::{Clock, Progress, SinkKind, SinkRec};
use crate::looper::{EvmView, Io};
use crate::mintdesk::ReqRec;
use crate::redeem::CustodyIo;

/// Below this many cycles the deposits pass is skipped (audit D, H-3): deliveries, which cannot be switched off, keep what is left.
pub const DEPOSITS_CYCLES_FLOOR: u128 = 8_000_000_000_000;

/// The float's Schnorr derivation path: frog buys for sinks and the desk are fronted from here, never from the vault buyer's
/// `b"btc"` (whose balance `state.rs` reads as "funded") and never from `b"custody"`.
pub(crate) const FLOAT_PATH: &[u8] = b"float";
/// The parent chain's block time the contracts' `Wait` uses (src/lib/Wait.sol).
const PARENT_BLOCK_SECONDS: u64 = 12;

thread_local! {
    /// Everything this side keeps. Saved across upgrades as the upgrade tuple's seventh element.
    pub(crate) static FROGS: RefCell<FrogState> = RefCell::new(FrogState::default());
    /// Offers handed in by the relayer, by frog record. Heap only, as `relay.rs`: after an upgrade they are handed in again.
    static OFFERS: RefCell<Vec<(u64, Offer)>> = const { RefCell::new(Vec::new()) };
    /// Protected Ordinals Wallet offers (docs/v8/ow-protected.md §3.4), beside `OFFERS` and like them heap only. A frog has at
    /// most one offer of either kind: a hand-in of one kind replaces the other.
    static PROTECTED: RefCell<Vec<(u64, ProtectedOffer)>> = const { RefCell::new(Vec::new()) };
    static FLOAT_SCRIPT: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// Relay quotes handed in by the runner, by frog, with when (docs/v6/relay-conversion.md §3.4). Heap only, like offers.
    static QUOTES: RefCell<Vec<(u64, Vec<u8>, u64)>> = const { RefCell::new(Vec::new()) };
}

// ---- outcalls -----------------------------------------------------------------------------------------------------------------

/// One replicated GET, reduced by `transform` to a canonical `ok …` / `err …` body.
async fn get(url: String, transform: &str, context: Vec<u8>, max: u64) -> Result<String, String> {
    let req = HttpRequestArgs {
        url,
        method: HttpMethod::GET,
        body: None,
        max_response_bytes: Some(max),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: transform.into() }),
            context,
        }),
        headers: vec![
            HttpHeader { name: "user-agent".into(), value: "etch-autobuyer".into() },
            HttpHeader { name: "accept".into(), value: "application/json".into() },
        ],
        is_replicated: Some(true),
    };
    let res = http_request(&req).await.map_err(|e| format!("{e:?}"))?;
    let body = String::from_utf8(res.body).map_err(|_| "a transformed body that is not text".to_string())?;
    match body.split_once(' ') {
        Some(("ok", rest)) => Ok(rest.to_string()),
        Some(("err", rest)) => Err(rest.to_string()),
        _ => Err(format!("an unexpected transformed body: {body:?}")),
    }
}

/// One replicated JSON-RPC POST through the custodian's block-clock transform.
async fn post(url: &str, body: String, transform: &str) -> Result<String, String> {
    let req = HttpRequestArgs {
        url: url.to_string(),
        method: HttpMethod::POST,
        body: Some(body.into_bytes()),
        max_response_bytes: Some(64_000),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: transform.into() }),
            context: vec![],
        }),
        headers: vec![HttpHeader { name: "content-type".into(), value: "application/json".into() }],
        is_replicated: Some(true),
    };
    let res = http_request(&req).await.map_err(|e| format!("{e:?}"))?;
    let body = String::from_utf8(res.body).map_err(|_| "a transformed body that is not text".to_string())?;
    match body.split_once(' ') {
        Some(("ok", rest)) => Ok(rest.to_string()),
        Some(("err", rest)) => Err(rest.to_string()),
        _ => Err(format!("an unexpected transformed body: {body:?}")),
    }
}

fn agree(label: &str, answers: Vec<String>) -> Result<String, String> {
    let first = answers.first().cloned().ok_or("no sources")?;
    if answers.iter().any(|a| *a != first) {
        return Err(format!("the sources disagree on {label}; nothing is believed"));
    }
    Ok(first)
}

fn reply(r: Result<String, String>) -> HttpRequestResult {
    let body = match r {
        Ok(s) => format!("ok {s}"),
        Err(e) => format!("err {e}"),
    };
    HttpRequestResult { status: 200u32.into(), body: body.into_bytes(), headers: vec![] }
}

fn status_of(a: &TransformArgs) -> u16 {
    a.response.status.to_string().parse().unwrap_or(0)
}

/// esplora's `/tx/<txid>`: the transaction reduced to what the deposit checks read (`deposits::TxView`), as JSON.
#[ic_cdk::query]
fn frog_tx_view(args: TransformArgs) -> HttpRequestResult {
    reply(crate::deposits::parse_esplora_tx(status_of(&args), &args.response.body).map(|t| serde_json::to_string(&t).unwrap_or_default()))
}

/// A bare number or hash (esplora's `/blocks/tip/height`, `/block-height/<h>`), trimmed and lower-cased.
#[ic_cdk::query]
fn frog_text(args: TransformArgs) -> HttpRequestResult {
    let s = String::from_utf8_lossy(&args.response.body).trim().to_ascii_lowercase();
    reply(if status_of(&args) == 200 && !s.is_empty() && s.len() <= 80 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(s)
    } else {
        Err(format!("HTTP {}", status_of(&args)))
    })
}

/// esplora's `/tx/<txid>/outspend/<vout>`: `none`, or `<spending txid> <confirmed>`.
#[ic_cdk::query]
fn frog_outspend(args: TransformArgs) -> HttpRequestResult {
    reply(crate::frogbuy::parse_outspend_full(status_of(&args), &args.response.body).map(|o| match o {
        None => "none".into(),
        Some((t, c)) => format!("{} {c}", hex::encode(t)),
    }))
}

/// Who spent `o` (display txid) and whether that is confirmed, every explorer agreeing; `None` unspent. The frog flow's and the
/// custodian's conflict-out (H8) read it the same way.
pub(crate) async fn outspend_of(o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
    let mut answers = Vec::new();
    for base in cfg().btc_api_urls {
        answers.push(get(format!("{base}/tx/{}/outspend/{}", hex::encode(o.txid), o.vout), "frog_outspend", vec![], 8_000).await
            .map_err(|e| format!("{base}: {e}"))?);
    }
    let a = agree("who spent the output", answers)?;
    if a == "none" {
        return Ok(None);
    }
    let (t, c) = a.split_once(' ').ok_or("an unreadable outspend")?;
    let txid: [u8; 32] = hex::decode(t).ok().and_then(|b| b.try_into().ok()).ok_or("an unreadable spending txid")?;
    Ok(Some((txid, c == "true")))
}

/// The pinned 4663 block's clocks, as the item flow reads them (shared with the Ethereum NFT side, `ethnet.rs`).
pub(crate) async fn home_clock(io: &mut CanisterIo) -> Result<Clock, String> {
    let pin = io.pin().await?;
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_getBlockByHash", "params": [pin.hash, false] }).to_string();
    let mut answers = Vec::new();
    for url in &cfg().rpc_urls {
        answers.push(post(url, body.clone(), "custody_block_clock").await?);
    }
    let a = agree("the pinned block's clocks", answers)?;
    let p: Vec<&str> = a.split(' ').collect();
    if p.len() != 3 || p[0] != pin.hash {
        return Err("the clocks are for a block other than the pin".into());
    }
    let n = |s: &str| s.parse::<u64>().map_err(|_| "an unreadable clock".to_string());
    let (ts, parent) = (n(p[1])?, n(p[2])?);
    // as the custodian does: the chain's time at least the canister's, the parent chain advanced by the lag at 12 s a block
    let now = ic_cdk::api::time() / 1_000_000_000;
    Ok(Clock { time: ts.max(now), parent_block: parent + now.saturating_sub(ts).div_ceil(PARENT_BLOCK_SECONDS) })
}

// ---- the Io ---------------------------------------------------------------------------------------------------------------------

pub(crate) struct ItemIo {
    io: CanisterIo,
    custody: crate::custodynet::CustodianIo,
    wrapper: [u8; 20],
    /// Phase B2: the Solana home's world (its providers, our Ed25519 key, this tick's rates) and the frog wrapper's Solana address.
    sol: Option<crate::solhome::SolNet>,
    sol_wrapper: Option<crate::soltx::Pubkey>,
    /// Ordinals multi-collection: the Inscriptions listing (books id) this world serves; its state is `FROGS`' entry of it.
    coll: Option<u16>,
    /// Audit C2-07: the Ethereum side's turn on the Relay leg is due and the leg is free: this collection's pass starts no advance.
    yield_leg: bool,
}

impl ItemIo {
    fn new(wrapper: &str) -> Result<Self, String> {
        let (w, sol_wrapper) = crate::canister::wrapper_key(wrapper)?;
        Ok(ItemIo { io: CanisterIo::new()?, custody: crate::custodynet::CustodianIo::new(wrapper)?, wrapper: w,
            sol: crate::solhome::SolNet::of_install()?, sol_wrapper, coll: None, yield_leg: false })
    }
    /// The world of one Inscriptions listing on the books home: its wX mint names it (`frogsol::books::coll_for`).
    fn for_coll(id: u16, wx: crate::soltx::Pubkey) -> Result<Self, String> {
        Self::for_coll_with(id, wx, crate::solhome::SolNet::of_install()?)
    }
    /// [`ItemIo::for_coll`] on a Solana world handed in (audit C2-04: the tick's one world, its clock and rates read once for every
    /// listing); [`ItemIo::into_sol`] hands it on.
    fn for_coll_with(id: u16, wx: crate::soltx::Pubkey, sol: Option<crate::solhome::SolNet>) -> Result<Self, String> {
        Ok(ItemIo { io: CanisterIo::new()?, custody: crate::custodynet::CustodianIo::for_coll(id, wx)?, wrapper: crate::frogsol::multi::life_key(&wx),
            sol, sol_wrapper: Some(wx), coll: Some(id), yield_leg: false })
    }
    fn into_sol(self) -> Option<crate::solhome::SolNet> {
        self.sol
    }
}

/// Audit C2-04: how long one Solana world (its finalized clock, its SOL/ETH and SOL/BTC rates, each read at most once) serves every
/// Inscriptions listing's custodian and frog side: one canister tick (the custodian's pass, then the frog side's).
const TICK_SOL_SECS: u64 = 15;

thread_local! {
    /// Audit C2-04: the tick's Solana world, with when it was made, between the custodian's pass and the frog side's.
    static TICK_SOL: RefCell<Option<(u64, crate::solhome::SolNet)>> = const { RefCell::new(None) };
}

/// Audit C2-04: the tick's one Solana world: the one the custodian's pass of this tick read with (younger than [`TICK_SOL_SECS`]), or a
/// new one. Before the change each listing's world (custodian and frog side) re-read the clock (getSlot + getBlockTime, each agreed by
/// two providers: 4 HTTPS outcalls) and the rates: with 15 listings a quiet tick made about 120 Solana outcalls where there had been
/// 8. Shared, a quiet tick with 15 listings makes ONE clock read (4 outcalls) for every ticket count and sink read, and each rate at
/// most once (an oracle read each, only when a purchase is valued or a floor posted): 4 to 8 Solana outcalls, whatever the number of
/// listings (the per-listing reads that remain are the books', in memory).
pub(crate) fn tick_sol() -> Result<(u64, Option<crate::solhome::SolNet>), String> {
    let now = ic_cdk::api::time() / 1_000_000_000;
    if let Some((at, s)) = TICK_SOL.with(|t| t.borrow_mut().take()) {
        if now.saturating_sub(at) <= TICK_SOL_SECS {
            return Ok((at, Some(s)));
        }
    }
    Ok((now, crate::solhome::SolNet::of_install()?))
}

/// Audit C2-04: the tick's Solana world for the next listing's world: the one handed on, or (when a world could not be made and took
/// it with it) a new one, so no listing ever runs without the home's world.
pub(crate) fn next_sol(sol: &mut Option<crate::solhome::SolNet>) -> Result<Option<crate::solhome::SolNet>, String> {
    match sol.take() {
        Some(s) => Ok(Some(s)),
        None => crate::solhome::SolNet::of_install(),
    }
}

/// Audit C2-04: the tick's Solana world kept for the next pass of the same tick.
pub(crate) fn keep_tick_sol(at: u64, s: Option<crate::solhome::SolNet>) {
    if let Some(s) = s {
        TICK_SOL.with(|t| *t.borrow_mut() = Some((at, s)));
    }
}

crate::sol_io_via!(ItemIo, sol);

/// How deep a Bitcoin transaction is (display txid), as the frog side reads it: every explorer's status, the block at its height
/// still the one they named, the lowest tip (`frogflow::depth`). For the custodian's `markUndeliverable` (canister review2 N-2).
pub(crate) async fn depth_of(wrapper: &str, txid: [u8; 32]) -> Result<Option<u64>, String> {
    let mut io = ItemIo::new(wrapper)?;
    crate::frogflow::depth(&mut io, txid).await
}

/// [`depth_of`] for one Inscriptions listing's custodian (the books home).
pub(crate) async fn depth_of_coll(id: u16, wx: crate::soltx::Pubkey, txid: [u8; 32]) -> Result<Option<u64>, String> {
    let mut io = ItemIo::for_coll(id, wx)?;
    crate::frogflow::depth(&mut io, txid).await
}

impl FrogIo for ItemIo {
    fn sol_wrapper(&self) -> Option<crate::soltx::Pubkey> {
        self.sol_wrapper
    }
    fn eth_relay_busy(&self) -> bool {
        // B-L-6: the one Relay leg is any chain's NFT side (Ethereum's, Robinhood Chain's), or (Ordinals multi-collection) another
        // Inscriptions collection's
        crate::ethnet::any_relay_busy() || self.yield_leg
            || self.coll.is_some_and(|me| FROGS.with(|f| f.try_borrow().map_or(true, |t| crate::frogsol::multi::held_elsewhere(&t, me))))
    }
    fn paused(&self) -> bool {
        crate::canister::not_paused().is_err()
    }
    fn now(&self) -> u64 {
        ic_cdk::api::time() / 1_000_000_000
    }
    fn chain_id(&self) -> u64 {
        Io::chain_id(&self.io)
    }
    fn wrapper(&self) -> [u8; 20] {
        self.wrapper
    }
    fn fresh(&mut self) {
        if let Ok(io) = CanisterIo::new() {
            self.io = io;
        }
    }
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        self.io.eth_read(to, data).await
    }
    async fn clock(&mut self) -> Result<Clock, String> {
        home_clock(&mut self.io).await
    }
    async fn our_evm(&mut self) -> Result<[u8; 20], String> {
        Ok(crate::canister::ours().await?.0)
    }
    async fn evm_nonce(&mut self) -> Result<u64, String> {
        self.io.evm_nonce().await
    }
    async fn base_fee(&mut self) -> Result<u128, String> {
        self.io.base_fee().await
    }
    async fn estimate_gas(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<u64, String> {
        self.io.estimate_gas(to, value, data).await
    }
    async fn sign_evm(&mut self, hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> {
        self.io.sign_evm(hash).await
    }
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String> {
        self.io.send_evm(raw).await
    }
    async fn evm_seen(&mut self, hash: [u8; 32]) -> EvmView {
        self.io.evm_seen(hash).await
    }
    async fn float_script(&mut self) -> Result<Vec<u8>, String> {
        float_script_of().await
    }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> {
        crate::canister::custody_script().await
    }
    async fn locate(&mut self, inscription: &str) -> Result<Option<Location>, String> {
        self.custody.locate(inscription).await
    }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> {
        self.custody.contents(o).await
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        self.custody.output(o).await
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        outspend_of(o).await
    }
    async fn tx_status(&mut self, txid: [u8; 32]) -> crate::btcnet::BtcSeen {
        let mut internal = txid;
        internal.reverse();
        crate::btcnet::btc_tx_seen(internal).await
    }
    async fn tip(&mut self) -> Result<u64, String> {
        // explorers a block apart are normal: the LOWEST tip is used, the side that counts fewer confirmations
        let mut lo = u64::MAX;
        for base in cfg().btc_api_urls {
            let t: u64 = get(format!("{base}/blocks/tip/height"), "frog_text", vec![], 2_000).await
                .map_err(|e| format!("{base}: {e}"))?.parse().map_err(|_| format!("{base}: an unreadable tip"))?;
            lo = lo.min(t);
        }
        Ok(lo)
    }
    async fn block_hash(&mut self, height: u64) -> Result<String, String> {
        let mut answers = Vec::new();
        for base in cfg().btc_api_urls {
            answers.push(get(format!("{base}/block-height/{height}"), "frog_text", vec![], 2_000).await.map_err(|e| format!("{base}: {e}"))?);
        }
        agree(&format!("the block at {height}"), answers)
    }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        crate::btcnet::broadcast_btc(raw).await.map(|_| ())
    }
    async fn custody_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        // finding holder deposits needs the listing itself (audit D, H-4: fees come from the pool, `fee_utxos`)
        let mut us = self.custody.list_custody_utxos().await?;
        us.retain(|u| u.confirmed);
        Ok(us)
    }
    fn deposits_on(&self) -> bool {
        // H-2: off unless the install argument switches them on; and below the cycles floor, optional work waits (H-3); phase B2: off
        // on the Solana home (the EWD2 claim comes with the first post-launch release)
        self.sol.is_none() && crate::canister::deposits_on() && ic_cdk::api::canister_cycle_balance() >= DEPOSITS_CYCLES_FLOOR
    }
    async fn fee_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        crate::redeem::CustodyIo::our_utxos(&mut self.custody).await
    }
    fn fee_pool_add(&mut self, o: Outpoint, value: u64) {
        crate::custodynet::pool_add(o, value, "change");
    }
    async fn buyer_script(&mut self) -> Result<Vec<u8>, String> {
        Ok(crate::canister::ours().await?.1)
    }
    async fn float_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        crate::custodynet::utxos_of(&float_script_of().await?).await
    }
    async fn tx_view(&mut self, txid: [u8; 32]) -> Result<TxView, String> {
        let mut answers = Vec::new();
        for base in cfg().btc_api_urls {
            answers.push(get(format!("{base}/tx/{}", hex::encode(txid)), "frog_tx_view", vec![], 200_000).await
                .map_err(|e| format!("{base}: {e}"))?);
        }
        serde_json::from_str(&agree("the deposit's transaction", answers)?).map_err(|_| "an unreadable transaction".into())
    }
    async fn fee_rate(&mut self) -> Result<u64, String> {
        self.custody.fee_rate().await
    }
    async fn rate(&mut self) -> Result<u64, String> {
        // phase B2: on the Solana home, satoshis per 10^18 lamports (SOL/BTC), so `frogbuy`'s arithmetic is the home's
        if let Some(s) = self.sol.as_mut() {
            return crate::rates::sats_per_home_unit(s.btc_rate().await?);
        }
        Ok(crate::oracle::read_rate().await?.sats_per_eth)
    }
    async fn input_clean(&mut self, o: Outpoint) -> Result<(), String> {
        crate::custodynet::buyer_input_clean(o).await
    }
    async fn sign_float(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        crate::canister::sign_taproot_with(FLOAT_PATH, sighash.to_vec()).await
    }
    async fn sign_custody(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        crate::canister::sign_taproot_with(crate::canister::CUSTODY_PATH, sighash.to_vec()).await
    }
    async fn float_address(&mut self) -> Result<String, String> {
        float_address_of().await
    }
    fn funding(&self) -> Option<crate::relayconv::FrogFunding> {
        Some(crate::canister::frog_funding())
    }
    fn take_quote(&mut self, frog: u64) -> Option<(Vec<u8>, u64)> {
        QUOTES.with(|q| {
            let mut q = q.borrow_mut();
            let i = q.iter().position(|(f, _, _)| *f == frog)?;
            let (_, body, at) = q.remove(i);
            Some((body, at))
        })
    }
    fn return_quote(&mut self, frog: u64, body: Vec<u8>, at: u64) {
        QUOTES.with(|q| {
            let mut q = q.borrow_mut();
            if !q.iter().any(|(f, _, _)| *f == frog) {
                q.push((frog, body, at));
            }
        })
    }
    fn take_offer(&mut self, frog: u64) -> Option<Offer> {
        OFFERS.with(|o| {
            let mut o = o.borrow_mut();
            let i = o.iter().position(|(f, _)| *f == frog)?;
            Some(o.remove(i).1)
        })
    }
    fn take_protected(&mut self, frog: u64) -> Option<ProtectedOffer> {
        PROTECTED.with(|o| {
            let mut o = o.borrow_mut();
            let i = o.iter().position(|(f, _)| *f == frog)?;
            Some(o.remove(i).1)
        })
    }
    async fn float_internal_key(&mut self) -> Result<[u8; 32], String> {
        crate::canister::internal_key_for(FLOAT_PATH).await
    }
    fn used_elsewhere(&self) -> Vec<[u8; 32]> {
        let Some(me) = self.coll else { return vec![] };
        FROGS.with(|f| {
            let top = f.borrow();
            let others = top.colls.iter().flatten().filter(|(c, _)| *c != me).map(|(_, s)| s);
            std::iter::once(&*top).chain(others).flat_map(|s| crate::relayconv::used_orders(s.convs())).collect()
        })
    }
    fn save(&mut self, st: &FrogState) {
        match self.coll {
            Some(id) => FROGS.with(|f| crate::frogsol::multi::put(&mut f.borrow_mut(), id, st)),
            None => FROGS.with(|f| *f.borrow_mut() = st.clone()),
        }
    }
    fn save_floor(&mut self, fl: &crate::floortrigger::FloorState) {
        crate::floornet::FLOORS.with(|f| *f.borrow_mut() = fl.clone());
    }
    fn save_handins(&mut self, hs: &crate::handin::HandinState) {
        crate::handinnet::HANDINS.with(|h| *h.borrow_mut() = hs.clone());
    }
    fn save_sales(&mut self, ss: &crate::sale::SaleState) {
        crate::salenet::SALES.with(|s| *s.borrow_mut() = ss.clone());
    }
    fn btc_hrp(&self) -> Result<&'static str, String> {
        Io::btc_hrp(&self.io)
    }
    async fn our_balance(&mut self) -> Result<u128, String> {
        // phase B2: on the Solana home, our Solana address's lamports at finalized (the refund rule reads it)
        if let Some(s) = self.sol.as_mut() {
            return s.balance().await;
        }
        self.io.evm_balance().await
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
    fn outbound_policy(&self, custody_script: &[u8]) -> crate::outbound::BtcPolicy {
        crate::custodynet::outbound_policy(custody_script)
    }
}

pub(crate) async fn float_script_of() -> Result<Vec<u8>, String> {
    if let Some(s) = FLOAT_SCRIPT.with(|c| c.borrow().clone()) {
        return Ok(s);
    }
    let s = [vec![0x51, 0x20], crate::canister::output_key_for(FLOAT_PATH).await?.to_vec()].concat();
    FLOAT_SCRIPT.with(|c| *c.borrow_mut() = Some(s.clone()));
    Ok(s)
}

/// The float's bech32m address on the installed network.
pub(crate) async fn float_address_of() -> Result<String, String> {
    let key = crate::canister::output_key_for(FLOAT_PATH).await?;
    let hrp = crate::policy::hrp_for(&cfg().btc_network)?;
    Ok(crate::policy::taproot_address(&key, hrp))
}

// ---- the tick's hooks ---------------------------------------------------------------------------------------------------------------

/// Whether our own EVM transaction is out (for the canister-wide one-transaction rule).
pub(crate) fn evm_pending() -> bool {
    FROGS.with(|f| crate::frogsol::multi::states(&f.borrow()).into_iter().any(|s| s.evm_pending()))
}

// ---- Ordinals multi-collection (docs/v9/ordinals-collections.md §4, on the books home) -------------------------------------------

/// The frog side is per Inscriptions listing on the books home (the Solana home: the install accepts only the books there); the EVM
/// home keeps its one wrapper and its one state.
pub(crate) fn per_collection() -> bool {
    cfg().home.is_some()
}

/// What the tick runs the frog side and the custodian for: the configured wrapper, or on the books home (where `Init.wrapper` is
/// unused) the books' Inscriptions listings themselves.
pub(crate) fn frog_wrapper() -> Option<String> {
    cfg().wrapper.or_else(|| per_collection().then(String::new))
}

/// The upgrade from the single state (once): the single frog state and the single custodian state move into the entry of the books'
/// first Inscriptions listing, the one the single-collection code served (`post_upgrade`, and lazily at the first use that finds it).
pub(crate) fn ensure_migrated() {
    if !per_collection() {
        return;
    }
    let Some(l) = crate::frogsol::multi::listings().into_iter().next() else { return };
    let now = ic_cdk::api::time() / 1_000_000_000;
    if FROGS.with(|f| crate::frogsol::multi::migrate(&mut f.borrow_mut(), &l, now)) {
        crate::canister::note(format!("upgraded: the frog state is per Inscriptions listing; the single state is collection {}'s", l.id));
    }
    if crate::custodynet::CUSTODY.with(|c| crate::frogsol::multi::migrate_custody(&mut c.borrow_mut(), l.id)) {
        crate::canister::note(format!("upgraded: the redemptions are per Inscriptions listing; the single cursor is collection {}'s", l.id));
    }
}

/// The custody caps' keys of the Inscriptions listings (`sol:<wX mint>`): stage 0 for each in the default caps (`canister::caps_of`),
/// the global re-sized there by the exit rule (A4-L-1). Audit C2-08: EVERY listing in the books, live or asleep, a repeated root
/// excepted (C2-14): the caps follow the books (the controller's pins), never the operator's `frog_wake` / `frog_sleep`, so a brake
/// neither adds a cap nor takes one away at run time, and `post_upgrade` compares the same set before and after.
pub(crate) fn listing_cap_keys() -> Vec<String> {
    // the caller names the home (`caps_of` reads its argument's): without Inscriptions listings in the books this is empty
    let all = crate::frogsol::multi::listings();
    all.iter().filter(|l| !crate::frogsol::multi::repeats_root(&all, l)).map(|l| crate::custodycap::sol_key(&l.wx_mint)).collect()
}

/// Run `f` on the frog state `coll` names: on the books home its Inscriptions listing's entry (`coll` must be one), on the EVM home
/// the one state (`coll` is not read there).
fn with_coll<R>(coll: u16, f: impl FnOnce(&mut FrogState) -> Result<R, String>) -> Result<R, String> {
    if !per_collection() {
        return FROGS.with(|s| f(&mut s.borrow_mut()));
    }
    crate::frogsol::multi::listing(coll)?;
    ensure_migrated();
    FROGS.with(|s| f(crate::frogsol::multi::entry_mut(&mut s.borrow_mut(), coll)))
}

/// Run `f` on the frog state that holds `frog` (frog ids are unique canister-wide): its collection's entry, or the one state.
fn with_frog<R>(frog: u64, f: impl FnOnce(&mut FrogState) -> R) -> R {
    FROGS.with(|s| {
        let mut top = s.borrow_mut();
        let id = top.colls.iter().flatten().find(|(_, e)| e.frogs.iter().any(|x| x.id == frog) || e.conv(frog).is_some()).map(|(c, _)| *c);
        match id {
            Some(c) => f(crate::frogsol::multi::entry_mut(&mut top, c)),
            None => f(&mut top),
        }
    })
}

/// The states a view reads: `coll`'s entry, or (none named) the one state and every collection's.
fn view<R>(coll: Option<u16>, f: impl Fn(&FrogState) -> Vec<R>) -> Vec<R> {
    FROGS.with(|s| {
        let top = s.borrow();
        match coll {
            Some(c) if per_collection() => crate::frogsol::multi::entry(&top, c).map(&f).unwrap_or_default(),
            _ => crate::frogsol::multi::states(&top).into_iter().flat_map(&f).collect(),
        }
    })
}

/// The money line's frog turn, where it is in the queue: one collection's pass that gives up the frog side's turn must not drop the
/// turn another collection queued for (they share `LineOwner::Frog`).
fn frog_turn() -> Option<(usize, u64)> {
    let l = crate::solhome::line_now();
    l.queue.iter().position(|(o, _)| *o == crate::solhome::LineOwner::Frog).map(|i| (i, l.queue[i].1))
}

fn keep_frog_turn(before: Option<(usize, u64)>) {
    let Some((i, at)) = before else { return };
    let mut l = crate::solhome::line_now();
    let taken = l.pending.as_ref().is_some_and(|p| p.step.owner() == crate::solhome::LineOwner::Frog);
    if !taken && !l.queue.iter().any(|(o, _)| *o == crate::solhome::LineOwner::Frog) {
        l.queue.insert(i.min(l.queue.len()), (crate::solhome::LineOwner::Frog, at));
        crate::solhome::line_put(&l);
    }
}

/// The books home's pass: custody classified once (each new output routed to the live collection whose root its inscription proves
/// against, a non-member refused once), then each LIVE Inscriptions listing with its own world and its own state (`next_frog`
/// canister-wide); the one Solana line, the Relay leg, the outbound limiter and the exit rule shared. A listing the operator put to
/// sleep is not read (default-live: every other one is).
///
/// Audit C2-04: every listing's world runs on the tick's ONE Solana world (`tick_sol`): one clock read and each rate at most once a
/// tick. Audit C2-07: the listings take their passes in the rotation's order (`multi::in_turn`, after the last that took the leg or the
/// line), and the Ethereum side's turn on a free leg is kept for it once per leg (`multi::eth_turn_now`). Default-live (audit C2 §4):
/// before a listing's pass, what the wake check waited for is set from the launch files where nothing is (`multi::launch_defaults`).
async fn tick_collections(others_pending: bool) -> Vec<String> {
    let all = crate::frogsol::multi::listings();
    if all.is_empty() {
        return vec![];
    }
    if let Err(e) = crate::btcnet::ensure_btc_network().await {
        return vec![format!("items: Bitcoin not read: {e}")];
    }
    ensure_migrated();
    let mut lines = Vec::new();
    let live = FROGS.with(|f| crate::frogsol::multi::live(&f.borrow(), &all));
    if live.is_empty() {
        return vec!["items: waits: every Inscriptions listing is asleep (frog_wake)".into()];
    }
    let (sol_at, mut sol) = match tick_sol() {
        Ok(x) => x,
        Err(e) => return vec![format!("items: not configured: {e}")],
    };
    // custody: one address, every new output classified once (optional work: below the cycles floor it waits, as deposits do)
    if ic_cdk::api::canister_cycle_balance() >= DEPOSITS_CYCLES_FLOOR {
        let roots: Vec<(u16, [u8; 32])> = live.iter().map(|l| (l.id, l.root)).collect();
        match next_sol(&mut sol).and_then(|s| ItemIo::for_coll_with(live[0].id, live[0].wx_mint, s)) {
            Ok(mut io) => {
                let top = FROGS.with(|f| f.borrow().clone());
                match crate::frogsol::multi::classify(&mut io, &top, &roots).await {
                    Ok((unspent, routes, failed)) => lines.extend(FROGS.with(|f| {
                        let mut top = f.borrow_mut();
                        let mut l = crate::frogsol::multi::apply(&mut top, &unspent, routes);
                        // audit C2-05: an output that cannot be read is skipped and, after a few tries, set aside: never the batch's end
                        l.extend(crate::frogsol::multi::set_aside(&mut top, failed));
                        l
                    })),
                    Err(e) => lines.push(format!("custody: waits: not classified: {e}")),
                }
                sol = io.into_sol();
            }
            Err(e) => lines.push(format!("custody: waits: {e}")),
        }
    }
    // audit C2-07: the Ethereum side's turn on a free leg, then the rotation
    let now = ic_cdk::api::time() / 1_000_000_000;
    let yield_leg = FROGS.with(|f| crate::frogsol::multi::eth_turn_now(&mut f.borrow_mut(), crate::ethnet::any_relay_busy(), now));
    if yield_leg {
        lines.push("items: the Relay leg is free and the Ethereum side's turn: no Inscriptions collection's advance starts this tick (C2-07)".into());
    }
    let live = FROGS.with(|f| crate::frogsol::multi::in_turn(&f.borrow(), live));
    for l in live {
        let mut io = match next_sol(&mut sol).and_then(|s| ItemIo::for_coll_with(l.id, l.wx_mint, s)) {
            Ok(io) => io,
            Err(e) => {
                lines.push(format!("collection {}: items: not configured: {e}", l.id));
                continue;
            }
        };
        io.yield_leg = yield_leg;
        // its floor sink as the controller configured it (`set_floor_sink`, the module's address), or (default-live) the launch files'
        // defaults where nothing is set: the collection's own entry reads it
        if let Ok(mk) = crate::frogsol::books::module_key(&mut io).await {
            let set = FROGS.with(|f| crate::floornet::FLOORS.with(|fl| {
                crate::frogsol::multi::launch_defaults(&mut f.borrow_mut(), &mut fl.borrow_mut(), &l, mk)
            }));
            if !set.is_empty() {
                lines.push(format!("collection {}: default-live: set from the launch files: {}", l.id, set.join(", ")));
            }
            FROGS.with(|f| {
                let mut top = f.borrow_mut();
                if let Some(rec) = top.sinks.iter().find(|s| s.sink == mk).cloned() {
                    let e = crate::frogsol::multi::entry_mut(&mut top, l.id);
                    if !e.sinks.iter().any(|s| s.sink == mk) {
                        e.sinks.push(rec);
                    }
                }
            });
        }
        let delivery_reserved = crate::custodynet::reserved();
        let mut st = FROGS.with(|f| crate::frogsol::multi::take(&f.borrow(), l.id));
        let (held_before, step_before) = (crate::convflow::holds_relay(&st), st.sol_step.is_some());
        let mut fl = crate::floornet::FLOORS.with(|f| f.borrow().clone());
        let mut hs = crate::handinnet::HANDINS.with(|h| h.borrow().clone());
        let mut ss = crate::salenet::SALES.with(|s| s.borrow().clone());
        let turn = frog_turn();
        let out = crate::frogflow::tick_all(&mut io, &mut st, &mut fl, &mut hs, &mut ss, others_pending, &delivery_reserved).await;
        keep_frog_turn(turn);
        let (took_leg, took_line) = (!held_before && crate::convflow::holds_relay(&st), !step_before && st.sol_step.is_some());
        FROGS.with(|f| {
            let mut top = f.borrow_mut();
            crate::frogsol::multi::put(&mut top, l.id, &st);
            if took_leg || took_line {
                crate::frogsol::multi::took(&mut top, l.id, took_leg);
            }
        });
        crate::floornet::FLOORS.with(|f| *f.borrow_mut() = fl);
        crate::handinnet::HANDINS.with(|h| *h.borrow_mut() = hs);
        crate::salenet::SALES.with(|s| *s.borrow_mut() = ss);
        lines.extend(out.into_iter().map(|s| format!("collection {}: {s}", l.id)));
        sol = io.into_sol();
    }
    keep_tick_sol(sol_at, sol);
    lines
}

/// Custody outputs a return in flight, or a delist's move in flight, has claimed as its fee: a redemption delivery must not
/// choose them.
pub(crate) fn reserved() -> Vec<Outpoint> {
    let mut r = FROGS.with(|f| f.borrow().reserved());
    r.extend(crate::salenet::reserved());
    r
}

/// The inscription of a frog this canister minted through `wrapDeposit`, by its registry leaf.
pub(crate) fn inscription_for_leaf(leaf: &[u8; 32]) -> Option<String> {
    FROGS.with(|f| f.borrow().inscription_for_leaf(leaf))
}

/// The pass, run by `tick` after the custodian's when a wrapper is configured.
pub(crate) async fn tick(wrapper: &str, others_pending: bool) -> Vec<String> {
    if per_collection() {
        return tick_collections(others_pending).await;
    }
    let mut io = match ItemIo::new(wrapper) {
        Ok(io) => io,
        Err(e) => return vec![format!("items: not configured: {e}")],
    };
    // I-3 (audit R2, N-9): every explorer shows the installed network's checkpoint before the frog pass (buys, abandon
    // conflicts, deposits, returns) believes a read from it, as the custodian's pass already required
    if let Err(e) = crate::btcnet::ensure_btc_network().await {
        return vec![format!("items: Bitcoin not read: {e}")];
    }
    let delivery_reserved: Vec<Outpoint> = crate::custodynet::reserved();
    let mut st = FROGS.with(|f| f.borrow().clone());
    let mut fl = crate::floornet::FLOORS.with(|f| f.borrow().clone());
    let mut hs = crate::handinnet::HANDINS.with(|h| h.borrow().clone());
    let mut ss = crate::salenet::SALES.with(|s| s.borrow().clone());
    let lines = crate::frogflow::tick_all(&mut io, &mut st, &mut fl, &mut hs, &mut ss, others_pending, &delivery_reserved).await;
    FROGS.with(|f| *f.borrow_mut() = st);
    crate::floornet::FLOORS.with(|f| *f.borrow_mut() = fl);
    crate::handinnet::HANDINS.with(|h| *h.borrow_mut() = hs);
    crate::salenet::SALES.with(|s| *s.borrow_mut() = ss);
    lines
}

// ---- endpoints ---------------------------------------------------------------------------------------------------------------------

fn only_controller() {
    let caller = ic_cdk::api::msg_caller();
    if !ic_cdk::api::is_controller(&caller) {
        ic_cdk::trap(&format!("refused: {caller} is not a controller of this canister."));
    }
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    hex::decode(s.trim().trim_start_matches("0x")).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| format!("{s:?} is not 32 bytes of hex"))
}

/// Protected-listing review P-01 (2026-10-09): the frog collection's creator addresses, the only recipients a protected Ordinals
/// Wallet sale may pay a royalty to (the listing's own word is never believed). Replaces the list; empty pays no royalty. A
/// controller's act (after the handover, the timelock's), never the operator's.
#[ic_cdk::update]
pub fn set_ow_creators(coll: u16, creators: Vec<String>) -> Result<String, String> {
    only_controller();
    crate::canister::not_ticking()?;
    with_coll(coll, |st| st.set_ow_creators(creators))
}

/// Ordinals multi-collection (docs/v9/ordinals-collections.md §5): the operator wakes an Inscriptions listing it put to sleep: from the
/// next tick its floor reading is taken, its book read and its frogs bought. Default-live (operator decision 2026-10-12): every listing
/// is live from install, so this only lifts the operator's brake (`frog_sleep`); no listing needs it to start. Audit C2-06: the outputs
/// refused custody-wide are routed again (the live roots changed).
#[ic_cdk::update]
pub fn frog_wake(coll: u16) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    if !per_collection() {
        return Err("refused: the EVM home has one wrapper and no Inscriptions listings to wake".into());
    }
    let l = crate::frogsol::multi::listing(coll)?;
    ensure_migrated();
    let now = ic_cdk::api::time() / 1_000_000_000;
    let r = FROGS.with(|f| {
        let mut top = f.borrow_mut();
        let r = crate::frogsol::multi::wake(&mut top, &l, now)?;
        crate::frogsol::multi::reopen(&mut top);
        Ok::<String, String>(r)
    })?;
    crate::canister::note(format!("frogs: {r}"));
    Ok(r)
}

/// The operator puts an Inscriptions listing to sleep (default-live: the per-collection brake): no floor reading, no book read, no buy;
/// refused while anything of it is open. Its redemptions go on. Audit C2-02: also refused while it holds the one Relay leg (a written-off
/// order still watched) or has an outcome on the Solana line it has not read yet (a dormant listing is never ticked, so neither would
/// ever be read).
#[ic_cdk::update]
pub fn frog_sleep(coll: u16) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    if !per_collection() {
        return Err("refused: the EVM home has one wrapper and no Inscriptions listings to put to sleep".into());
    }
    let l = crate::frogsol::multi::listing(coll)?;
    ensure_migrated();
    let now = ic_cdk::api::time() / 1_000_000_000;
    let line = crate::solhome::line_now();
    let unread = FROGS.with(|f| {
        let top = f.borrow();
        let e = crate::frogsol::multi::take(&top, l.id);
        line.outcomes.iter().filter(|o| crate::frogsol::owns(&e, Some(l.id), o)).count()
    });
    if unread > 0 {
        return Err(format!("refused: collection {coll} has {unread} outcome(s) on the Solana line it has not read yet; it sleeps once they are read \
            (the next tick)"));
    }
    let r = FROGS.with(|f| {
        let mut top = f.borrow_mut();
        let r = crate::frogsol::multi::sleep(&mut top, &l, now)?;
        crate::frogsol::multi::reopen(&mut top);
        Ok::<String, String>(r)
    })?;
    crate::canister::note(format!("frogs: {r}"));
    Ok(r)
}

/// One Inscriptions listing as the runner and the monitor read it.
#[derive(Debug, Clone, candid::CandidType, serde::Deserialize)]
pub struct FrogCollection {
    pub coll: u16,
    /// The wX mint, base58.
    pub wx_mint: String,
    /// Its members root, hex.
    pub root: String,
    pub live: bool,
    pub frogs_open: u64,
    pub floor_reading: Option<crate::frogsol::FrogReading>,
}

/// Every Inscriptions listing in the books, live or asleep (the runner feeds each live one: `frog_floor_reading(coll, …)`; default-live:
/// every listing the operator has not put to sleep).
#[ic_cdk::query]
pub fn frog_collections() -> Vec<FrogCollection> {
    let all = crate::frogsol::multi::listings();
    FROGS.with(|f| {
        let top = f.borrow();
        all.iter().map(|l| {
            let e = crate::frogsol::multi::entry(&top, l.id);
            FrogCollection { coll: l.id, wx_mint: crate::soltx::b58_encode(&l.wx_mint), root: hex::encode(l.root),
                live: crate::frogsol::multi::is_live(&top, &l.wx_mint) && !crate::frogsol::multi::repeats_root(&all, l), frogs_open: e.map_or(0, |s| s.frogs.iter().filter(|x| !x.finished()).count() as u64),
                floor_reading: e.and_then(|s| s.floor_reading.clone()) }
        }).collect()
    })
}

/// The contracts this side serves. Identity, like the wrapper: the factory and the desk are set once each, from nothing; a
/// queue is added and never removed. A controller's act (after the handover, the timelock's), never the operator's.
#[ic_cdk::update]
pub fn set_item_config(factory: Option<String>, desk: Option<String>, queues: Vec<String>) -> Result<String, String> {
    only_controller();
    crate::canister::not_ticking()?;
    // phase B2: on the Solana home there is no curve factory (DBC), no mint desk and no buy queue (docs/v9 §2.7): the frog module is
    // the only sink, configured with set_floor_sink
    if cfg().home.is_some() && (factory.is_some() || desk.is_some() || !queues.is_empty()) {
        return Err("refused: the Solana home has no curve factory, mint desk or buy queue; the frog listing's module is its one sink".into());
    }
    FROGS.with(|f| {
        let mut st = f.borrow_mut();
        let parse = |s: &str| crate::evmtx::parse_address(s);
        if let Some(a) = factory {
            let a = parse(&a)?;
            match st.factory {
                None => st.factory = Some(a),
                Some(x) if x == a => {}
                Some(x) => return Err(format!("refused: the factory is 0x{} and is never changed", hex::encode(x))),
            }
        }
        if let Some(a) = desk {
            let a = parse(&a)?;
            match st.desk {
                None => st.desk = Some(a),
                Some(x) if x == a => {}
                Some(x) => return Err(format!("refused: the desk is 0x{} and is never changed", hex::encode(x))),
            }
        }
        for q in queues {
            let a = parse(&q)?;
            if !st.sinks.iter().any(|s| s.sink == a) {
                st.sinks.push(SinkRec { kind: SinkKind::Queue, sink: a, coin: [0; 20], view: None, refused: None,
                    drove_refunds_at: None, drove_settle: false });
            }
        }
        Ok(format!("factory {:?}, desk {:?}, {} sink(s)", st.factory.map(hex::encode), st.desk.map(hex::encode), st.sinks.len()))
    })
}

/// The float's address and output key: fund it with plain sats (one frog's worth) to front purchases.
#[ic_cdk::update]
pub async fn float_address() -> Result<String, String> {
    only_operator();
    let key = crate::canister::output_key_for(FLOAT_PATH).await?;
    let hrp = crate::policy::hrp_for(&cfg().btc_network)?;
    Ok(format!("{} {}", crate::policy::taproot_address(&key, hrp), hex::encode(key)))
}

/// The relayer hands in a marketplace offer for a wanted frog record. Judged in full at the next tick; a bad offer wastes a
/// tick and nothing else.
#[ic_cdk::update]
pub fn frog_offer(coll: u16, frog_id: u64, inscription_id: String, psbt_hex: String, to_sign: Vec<u32>, proof_hex: Vec<String>)
    -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::custody::check_inscription_id(&inscription_id)?;
    let wanted = with_coll(coll, |st| Ok(st.frogs.iter().any(|x| x.id == frog_id && matches!(x.phase, FPhase::Wanted { .. }))))?;
    if !wanted {
        return Err(format!("refused: frog {frog_id} of collection {coll} is not waiting for an offer"));
    }
    let psbt = hex::decode(psbt_hex.trim()).map_err(|e| format!("the PSBT is not hex: {e}"))?;
    let proof = proof_hex.iter().map(|p| hex32(p)).collect::<Result<Vec<_>, _>>()?;
    let offer = Offer { inscription: inscription_id, psbt, to_sign: to_sign.into_iter().map(|i| i as usize).collect(), proof };
    OFFERS.with(|o| {
        let mut o = o.borrow_mut();
        o.retain(|(f, _)| *f != frog_id);
        o.push((frog_id, offer));
    });
    PROTECTED.with(|o| o.borrow_mut().retain(|(f, _)| *f != frog_id));
    Ok(format!("offer for frog {frog_id} held for the next tick"))
}

/// A protected Ordinals Wallet listing as the runner hands it in (docs/v8/ow-protected.md §3.4, `GET /market/escrow/:id`).
#[derive(Debug, Clone, candid::CandidType, serde::Deserialize)]
pub struct ProtectedListing {
    pub outpoint: String,
    pub seller_address: String,
    pub satoshi_price: u64,
    /// What the seller listed for. Protected-listing review P-05: optional, so an unknown price reaches the judge's
    /// `EscrowPriceUnknown` instead of being guessed by the runner.
    pub escrow_price: Option<u64>,
    pub creator_address: Option<String>,
}

/// The signed PSBTs of a handed frog, for the runner to submit to Ordinals Wallet.
#[derive(Debug, Clone, candid::CandidType, serde::Deserialize)]
pub struct ProtectedSigned {
    /// Display order hex.
    pub sale_txid: String,
    /// Hex.
    pub sale_psbt: String,
    pub setup_psbt: Option<String>,
}

/// The runner hands in a protected Ordinals Wallet sale for a wanted frog record (docs/v8/ow-protected.md §3.4): the build's
/// sale PSBT, its parent (the passthrough's raw bytes), the listing, the setup PSBT when the build made one, and the membership
/// proof. Nothing is judged here: the next tick judges it in full (`owbuy::judge_protected`), signs the float's inputs and hands
/// the PSBTs back (`frog_protected_signed`). It replaces any earlier offer for that frog, of either kind.
#[ic_cdk::update]
pub fn frog_offer_protected(coll: u16, frog_id: u64, inscription_id: String, sale_psbt_hex: String, parent_hex: String,
    listing: ProtectedListing, setup_psbt_hex: Option<String>, proof_hex: Vec<String>) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::custody::check_inscription_id(&inscription_id)?;
    let wanted = with_coll(coll, |st| Ok(st.frogs.iter().any(|x| x.id == frog_id && matches!(x.phase, FPhase::Wanted { .. }))))?;
    if !wanted {
        return Err(format!("refused: frog {frog_id} of collection {coll} is not waiting for an offer"));
    }
    let offer = protected_offer(inscription_id, &sale_psbt_hex, &parent_hex, listing, setup_psbt_hex.as_deref(), &proof_hex)?;
    PROTECTED.with(|o| {
        let mut o = o.borrow_mut();
        o.retain(|(f, _)| *f != frog_id);
        o.push((frog_id, offer));
    });
    OFFERS.with(|o| o.borrow_mut().retain(|(f, _)| *f != frog_id));
    Ok(format!("protected offer for frog {frog_id} held for the next tick"))
}

/// The hand-in decoded: hex to bytes, the listing's outpoint parsed. No judgement.
pub fn protected_offer(inscription: String, sale_psbt_hex: &str, parent_hex: &str, listing: ProtectedListing, setup_psbt_hex: Option<&str>,
    proof_hex: &[String]) -> Result<ProtectedOffer, String> {
    let sale_psbt = hex::decode(sale_psbt_hex.trim()).map_err(|e| format!("the sale PSBT is not hex: {e}"))?;
    let parent = hex::decode(parent_hex.trim()).map_err(|e| format!("the parent is not hex: {e}"))?;
    let setup_psbt = setup_psbt_hex.map(|h| hex::decode(h.trim()).map_err(|e| format!("the setup PSBT is not hex: {e}"))).transpose()?;
    let proof = proof_hex.iter().map(|p| hex32(p)).collect::<Result<Vec<_>, _>>()?;
    let listing = crate::owbuy::Listing {
        outpoint: Outpoint::parse(listing.outpoint.trim())?,
        seller_address: listing.seller_address,
        satoshi_price: listing.satoshi_price,
        escrow_price: listing.escrow_price,
        creator_address: listing.creator_address,
    };
    Ok(ProtectedOffer { inscription, sale_psbt, parent, listing, setup_psbt, proof })
}

/// The signed sale (and setup) PSBTs of a frog handed back to the runner; `None` unless the frog is `Handed`. The operator's.
#[ic_cdk::query]
pub fn frog_protected_signed(frog_id: u64) -> Option<ProtectedSigned> {
    only_operator();
    FROGS.with(|f| crate::frogsol::multi::states(&f.borrow()).into_iter().find_map(|s| s.protected_signed(frog_id).map(|r| ProtectedSigned {
        sale_txid: hex::encode(r.sale_txid), sale_psbt: hex::encode(&r.sale_psbt), setup_psbt: r.setup_psbt.as_ref().map(hex::encode) })))
}

/// The float's untweaked x-only internal key, hex: what Ordinals Wallet's build asks for as `public_key` (the address is built
/// from the tweaked output key, `float_address`).
#[ic_cdk::update]
pub async fn float_internal_key() -> Result<String, String> {
    only_operator();
    crate::canister::internal_key_for(FLOAT_PATH).await.map(hex::encode)
}

/// The runner hands in Relay's quote for a funded conversion (docs/v6/relay-conversion.md §3.4). Nothing is trusted here: the
/// next tick judges it against the canister's own reads (`relayconv::judge`) and signs the deposit only if every rule holds.
#[ic_cdk::update]
pub fn relay_quote(frog_id: u64, quote_json: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    if quote_json.len() > crate::relayconv::MAX_QUOTE_BYTES {
        return Err(format!("refused: the quote is over {} bytes", crate::relayconv::MAX_QUOTE_BYTES));
    }
    let funded = with_frog(frog_id, |st| st.conv(frog_id).is_some_and(|c| matches!(c.phase, crate::relayconv::CPhase::Funded { .. })));
    if !funded {
        return Err(format!("refused: frog {frog_id}'s conversion is not waiting for a quote"));
    }
    let now = ic_cdk::api::time() / 1_000_000_000;
    QUOTES.with(|q| {
        let mut q = q.borrow_mut();
        q.retain(|(f, _, _)| *f != frog_id);
        q.push((frog_id, quote_json.into_bytes(), now));
    });
    Ok(format!("quote for frog {frog_id} held; judged at the next tick (within {} s)", crate::relayconv::QUOTE_TTL_SECS))
}

/// The operator's exit (docs/v6/relay-conversion.md §3.4): a funded or stuck conversion's ETH goes back to its module at the next
/// tick (`returnAdvance`). It can send ETH nowhere else.
/// Our home balance and the home's gas keep: 4663's wei, or (phase B2) our Solana address's lamports.
async fn home_balance() -> Result<(u128, u128), String> {
    match crate::solhome::SolNet::of_install()? {
        Some(mut s) => Ok((s.balance().await?, crate::solhome::SOL_GAS_KEEP_LAMPORTS)),
        None => Ok((crate::canister::CanisterIo::new()?.evm_balance().await?, crate::relayconv::GAS_KEEP_WEI)),
    }
}

/// Phase B2, the Solana home: a BTC floor reading for the frog listing (satoshis per frog, the reader's confidence and depth), posted at
/// the next tick as `post_floor` in the books, in lamports at the canister's own SOL/BTC rate, clamped into the module's bounds (the
/// canister is the poster; while the poster is paused the reading is dropped, loud). Replaces a reading not yet posted. Operator-only.
#[ic_cdk::update]
pub fn frog_floor_reading(coll: u16, sats_per_item: u64, confidence_bps: u16, depth_items: u32) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    if cfg().home.is_none() {
        return Err("refused: on the EVM home the keeper service posts the frog floor (in wei); this is the Solana home's".into());
    }
    if sats_per_item == 0 || confidence_bps > 10_000 {
        return Err("refused: a zero price or a confidence over 10,000 bps".into());
    }
    // a collection put to sleep gets no floor reading (docs/v9/ordinals-collections.md §4, part 5); default-live: every other one does
    let l = crate::frogsol::multi::listing(coll)?;
    ensure_migrated();
    let all = crate::frogsol::multi::listings();
    if !FROGS.with(|f| crate::frogsol::multi::live(&f.borrow(), &all).contains(&l)) {
        return Err(format!("refused: collection {coll} is asleep (frog_sleep) or repeats another listing's members root: no floor reading"));
    }
    let at = ic_cdk::api::time() / 1_000_000_000;
    with_coll(coll, |st| {
        st.floor_reading = Some(crate::frogsol::FrogReading { sats: sats_per_item, confidence_bps, depth_items, at });
        Ok(())
    })?;
    Ok(format!("collection {coll}: a frog floor reading of {sats_per_item} sats ({confidence_bps} bps, depth {depth_items}) is posted in the \
        books at the next tick"))
}

#[ic_cdk::update]
pub async fn relay_give_back(coll: u16, frog_id: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    let (balance, keep) = home_balance().await?;
    crate::canister::not_ticking()?; // after the await: a tick that started meanwhile would write its own copy back
    let now = ic_cdk::api::time() / 1_000_000_000;
    with_coll(coll, |st| crate::convflow::give_back_now_keeping(st, frog_id, balance, now, keep))
}

/// Conversion review C-01: the operator gives up an arrival that has funded no buy for `relayconv::ARRIVED_EXIT_SECS` (7 days): what
/// is at our 4663 address (less the gas keep, at most the advance) goes back to the module with `returnAdvance` at the next tick, the
/// frog fails, the leg is freed, and the arrived sats stay in the float, recorded as the conversion's `abandoned_sats`
/// (`convflow::arrived_exit_now`). It can send ETH nowhere but the module.
#[ic_cdk::update]
pub async fn relay_arrived_exit(frog_id: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    let (balance, keep) = home_balance().await?;
    crate::canister::not_ticking()?; // after the await, as relay_give_back
    let now = ic_cdk::api::time() / 1_000_000_000;
    with_frog(frog_id, |st| crate::convflow::arrived_exit_now_keeping(st, frog_id, balance, now, keep))
}

/// Conversion review C-03: the operator releases a written-off order's hold on the Relay leg, once Relay's own records show where its
/// ETH went (`convflow::late_release_now`; the Ethereum side's `eth_late_release`).
#[ic_cdk::update]
pub fn relay_late_release(frog_id: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let now = ic_cdk::api::time() / 1_000_000_000;
    with_frog(frog_id, |st| crate::convflow::late_release_now(st, frog_id, now))
}

/// Every Relay conversion, for the runner, the monitor and the reserves page: `coll`'s (an Inscriptions listing on the books home), or
/// every collection's when none is named.
#[ic_cdk::query]
pub fn frog_conversions(coll: Option<u16>) -> Vec<crate::relayconv::ConvRec> {
    view(coll, |s| s.convs().to_vec())
}

/// A membership proof for an inscription (a holder deposit's, typically): verified against the listing root at every use.
///
/// Ordinals multi-collection: the proof names its collection (`coll`, the members file the runner proved it from): on the books home
/// it is checked against that listing's root at once (a non-member is refused here, not at a later use) and kept in that collection;
/// custody's classification still tries every proof handed in against every live root (`frogsol::multi::route`).
#[ic_cdk::update]
pub fn frog_proof(coll: u16, inscription_id: String, proof_hex: Vec<String>) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    crate::custody::check_inscription_id(&inscription_id)?;
    let proof = proof_hex.iter().map(|p| hex32(p)).collect::<Result<Vec<_>, _>>()?;
    if per_collection() {
        let l = crate::frogsol::multi::listing(coll)?;
        if !crate::frogbuy::verify_proof(&proof, &l.root, &crate::frogbuy::leaf_of(&inscription_id)) {
            return Err(format!("refused: the proof for {inscription_id} does not verify against collection {coll}'s root"));
        }
    }
    with_coll(coll, |st| {
        st.put_proof(inscription_id.clone(), proof);
        Ok(())
    })?;
    FROGS.with(|f| {
        let mut st = f.borrow_mut();
        // audit C2-06: a member whose proof comes after its output was seen is routed again (the outputs refused custody-wide)
        if per_collection() {
            crate::frogsol::multi::reopen(&mut st);
        }
        // a deposit parked or waiting for this proof is looked at again at the next tick (audit D, H-3)
        for d in st.deposits.iter_mut() {
            match &d.phase {
                DPhase::AwaitingProof { inscription } | DPhase::Parked { inscription } if *inscription == inscription_id => {
                    d.phase = DPhase::AwaitingProof { inscription: inscription_id.clone() };
                    d.next_check = None;
                    d.tries = None;
                }
                _ => {}
            }
        }
    });
    Ok(format!("proof for {inscription_id} kept"))
}

/// The operator's one say over a deposit: one waiting for a membership proof may be refused as not in the collection, which
/// only ever leads to its return. Nothing else about a deposit is the operator's.
#[ic_cdk::update]
pub fn refuse_deposit(outpoint: String, reason: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let o = Outpoint::parse(outpoint.trim())?;
    FROGS.with(|f| {
        let mut st = f.borrow_mut();
        let d = st.deposits.iter_mut().find(|d| d.outpoint == o).ok_or("no such deposit")?;
        match &d.phase {
            DPhase::AwaitingProof { .. } | DPhase::Parked { .. } => {
                d.phase = DPhase::Refused { reason: crate::deposits::Reason::NotInCollection(reason) };
                d.next_check = None;
                d.tries = None;
                Ok(format!("{} refused as not in the collection; it is returned at the next tick", o.text()))
            }
            DPhase::Pending { why } if why.contains("awaiting a membership proof") => {
                d.phase = DPhase::Refused { reason: crate::deposits::Reason::NotInCollection(reason) };
                Ok(format!("{} refused as not in the collection; it is returned at the next tick", o.text()))
            }
            p => Err(format!("refused: {} is {p:?}; only a deposit waiting for its proof may be refused", o.text())),
        }
    })
}

/// "k of N" (or a queue's count), per sink, as of the last tick's pinned read: `coll`'s, or every collection's.
#[ic_cdk::query]
pub fn item_sinks(coll: Option<u16>) -> Vec<Progress> {
    let progress = |st: &FrogState| st.sinks.iter().map(|s| crate::itemsink::progress(s, &st.frogs)).collect::<Vec<_>>();
    if per_collection() && coll.is_none() {
        // the books home: each collection's own sink (the top-level list is the configuration, not read)
        return FROGS.with(|f| f.borrow().colls.iter().flatten().flat_map(|(_, s)| progress(s)).collect());
    }
    view(coll, progress)
}

/// One sink's progress, by address.
#[ic_cdk::query]
pub fn item_progress(sink: String) -> Option<Progress> {
    let a = crate::evmtx::parse_address(sink.trim()).ok()?;
    view(None, |st| st.sinks.iter().filter(|s| s.sink == a).map(|s| crate::itemsink::progress(s, &st.frogs)).collect()).into_iter().last()
}

/// The frog buys: `coll`'s (an Inscriptions listing on the books home), or every collection's when none is named.
#[ic_cdk::query]
pub fn frog_buys(coll: Option<u16>) -> Vec<FrogRec> {
    view(coll, |s| s.frogs.clone())
}

#[ic_cdk::query]
pub fn holder_deposits() -> Vec<DepositRec> {
    FROGS.with(|f| f.borrow().deposits.clone())
}

#[ic_cdk::query]
pub fn desk_requests() -> Vec<ReqRec> {
    FROGS.with(|f| f.borrow().requests.clone())
}

/// Inscription-bearing custody outputs with no deposit marker: for the daily reconciliation, never acted on here. `coll`: those
/// routed to that collection (its items we did not buy); none named: every one, the custody-wide non-members included.
#[ic_cdk::query]
pub fn unmarked_custody(coll: Option<u16>) -> Vec<String> {
    view(coll, |s| s.unmarked.iter().map(|o| o.text()).collect())
}

#[cfg(test)]
mod tests {
    use crate::custody::Outpoint;
    use crate::deposits::DepositRec;
    use crate::itemsink::SinkRec;
    use crate::mintdesk::ReqRec;

    #[test]
    fn every_update_method_here_is_guarded() {
        let src = include_str!("frognet.rs");
        for kind in ["\npub fn", "\npub async fn"] {
            for part in src.split(&format!("{}{kind}", concat!("#[ic_cdk::", "update]"))).skip(1) {
                let head: String = part.chars().take(400).collect();
                assert!(head.contains("only_operator();") || head.contains("only_controller();"), "unguarded: {head}");
            }
        }
        assert_eq!(src.matches(concat!("#[ic_cdk::", "update]")).count(), 15, "every update is counted above (phase B2: frog_floor_reading; \
            Ordinals multi-collection: frog_wake, frog_sleep)");
        // the one query that is the operator's: the signed PSBTs of a handed frog (docs/v8/ow-protected.md §3.4)
        let q = src.split("pub fn frog_protected_signed(").nth(1).unwrap();
        assert!(q.chars().take(80).collect::<String>().contains("only_operator();"));
    }

    #[test]
    fn a_protected_hand_in_is_decoded_and_nothing_else() {
        use super::{protected_offer, ProtectedListing};
        let l = |outpoint: &str| ProtectedListing { outpoint: outpoint.into(), seller_address: "bc1pseller".into(), satoshi_price: 51_350,
            escrow_price: Some(50_000), creator_address: None };
        let op = format!("{}:0", "aa".repeat(32));
        let o = protected_offer("x".into(), " 70736274ff ", "0200", l(&op), Some("01"), &["11".repeat(32)]).unwrap();
        assert_eq!((o.sale_psbt, o.parent, o.setup_psbt, o.proof), (vec![0x70, 0x73, 0x62, 0x74, 0xff], vec![2, 0], Some(vec![1]), vec![[0x11; 32]]));
        assert_eq!((o.listing.escrow_price, o.listing.satoshi_price, o.listing.outpoint.vout), (Some(50_000), 51_350, 0));
        // review P-05: an unknown escrow price is carried as unknown (the judge refuses it), never replaced
        let unknown = ProtectedListing { escrow_price: None, ..l(&op) };
        assert_eq!(protected_offer("x".into(), "70", "02", unknown, None, &[]).unwrap().listing.escrow_price, None);
        assert!(protected_offer("x".into(), "zz", "02", l(&op), None, &[]).is_err());
        assert!(protected_offer("x".into(), "70", "0g", l(&op), None, &[]).is_err());
        assert!(protected_offer("x".into(), "70", "02", l(&op), Some("x"), &[]).is_err());
        assert!(protected_offer("x".into(), "70", "02", l("not an outpoint"), None, &[]).is_err());
        assert!(protected_offer("x".into(), "70", "02", l(&op), None, &["12".into()]).is_err());
    }

    /// A frog state saved before protected buys existed restores (`protected` = None); one with a handed frog and its record
    /// round-trips through Candid, the upgrade's encoding.
    #[test]
    fn frog_state_with_and_without_protected_buys_restores() {
        use crate::frogbuy::{Bought, FPhase, FrogRec};
        use crate::frogflow::{FrogState, ProtectedRec};
        // FrogState as it was before `protected`, field for field
        #[derive(candid::CandidType, Default)]
        struct Old { factory: Option<[u8; 20]>, desk: Option<[u8; 20]>, coin_cursor: u64, sinks: Vec<SinkRec>, poll: u64, request_cursor: u64,
            requests: Vec<ReqRec>, frogs: Vec<FrogRec>, next_frog: u64, deposits: Vec<DepositRec>, seen: Vec<Outpoint>, unmarked: Vec<Outpoint>,
            proofs: Vec<(String, Vec<[u8; 32]>)>, mint_times: Vec<u64>, return_times: Vec<u64>, pending: Option<crate::frogflow::FrogEvm>,
            unread: Option<Vec<Outpoint>>, read_failures: Option<Vec<(Outpoint, u32)>>, buy_clock: Option<Vec<(u64, u64)>>,
            convs: Option<Vec<crate::relayconv::ConvRec>> }
        let back: FrogState = candid::decode_one(&candid::encode_one(Old { next_frog: 3, ..Default::default() }).unwrap())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((back.protected, back.next_frog), (None, 3));
        let o = Outpoint { txid: [5; 32], vout: 2 };
        let buy = Bought { inscription: "i".into(), outpoint: o, seller: o, spend_sats: 1, fee_sats: 2, rate: 3, cost_wei: 4, raw: vec![], setup_txid: None };
        let st = FrogState {
            frogs: vec![FrogRec { id: 0, target: None, phase: FPhase::Handed { buy, sale_txid: hex::encode([5; 32]), at: 9 } }],
            protected: Some(vec![ProtectedRec { frog: 0, sale_txid: [5; 32], sale_psbt: vec![1], setup_psbt: Some(vec![2]),
                setup_txid: Some([6; 32]), sale_inputs: vec![(o, 7)], setup_inputs: vec![], sale_fee: 8, setup_fee: 9, escrow: o, at: 9,
                listing: Some(o), seen: Some(true) }]),
            ..Default::default()
        };
        let again: FrogState = candid::decode_one(&candid::encode_one(&st).unwrap()).unwrap();
        assert_eq!(again, st);
        assert_eq!(again.protected_signed(0).map(|r| r.sale_psbt.clone()), Some(vec![1]));
    }
}
