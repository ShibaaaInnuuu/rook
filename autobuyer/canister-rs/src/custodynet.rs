//! The custodian's real world (docs/v3/wrapper.md, "Step 3, the flow"): the ordinals providers (two at a time, agreeing, through
//! `ordnet.rs`; docs/v6/ordinals-apis.md) and the explorers read replicated, the wrapper's tickets read at an agreed block, and
//! everything that signs or sends delegated to the buyer's own `CanisterIo`, so the frog delivery uses the same pinned reads,
//! the same threshold keys and the same one-node-sends, every-node-reads-back paths as a purchase. Nothing is decided here:
//! `redeem.rs` decides, `custody.rs` judges, this only translates.

use std::cell::RefCell;

use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs, HttpRequestResult, TransformArgs,
    TransformContext, TransformFunc};

use crate::canister::{cfg, only_operator, CanisterIo};
use crate::custody::{self, Contents, Location, Outpoint, Output, Utxo};
use crate::delivery::TicketRead;
use crate::looper::{BtcView, EvmView, Io};
use crate::pin::Pin;
use crate::redeem::{self, Custody, CustodyIo, Redemption};

thread_local! {
    /// The redemptions and the ticket cursor. Saved across upgrades with the rest of the buyer's state.
    pub(crate) static CUSTODY: RefCell<Custody> = RefCell::new(Custody::default());
    /// Custody's fee inputs (audit D, H-4). Saved as the upgrade tuple's twelfth element. Not part of `CUSTODY`, which a tick
    /// holds a copy of: the operator's top-up and the frog side's returns write here directly, and nothing overwrites them.
    pub(crate) static FEE_POOL: RefCell<custody::FeePool> = RefCell::new(custody::FeePool::default());
}

/// A fee input for later: custody's own change, or a conflict's output, once confirmed (H-4).
pub(crate) fn pool_add(o: Outpoint, value: u64, source: &str) {
    FEE_POOL.with(|p| {
        let _ = p.borrow_mut().add(o, value, source);
    });
}

pub(crate) fn pool_drop(o: &Outpoint) {
    FEE_POOL.with(|p| p.borrow_mut().drop_entry(o));
}

/// The parent chain's block time the contracts' `Wait` uses (src/lib/Wait.sol).
const PARENT_BLOCK_SECONDS: u64 = 12;

// ---- outcalls -----------------------------------------------------------------------------------------------------------------

/// One replicated GET, reduced by `transform` to a canonical `ok …` / `err …` body so a failure agrees as surely as a success.
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

/// One replicated JSON-RPC POST to one provider, reduced the same way.
async fn post(url: &str, body: String, transform: &str, context: Vec<u8>, max: u64) -> Result<String, String> {
    let req = HttpRequestArgs {
        url: url.to_string(),
        method: HttpMethod::POST,
        body: Some(body.into_bytes()),
        max_response_bytes: Some(max),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: transform.into() }),
            context,
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

/// The same answer from every source, or nothing is believed.
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

// ---- transforms: each reduces a reply to the one fact the flow uses ----------------------------------------------------------

/// esplora's `/tx/<txid>`, output `vout` (in the context, big-endian u32): the output as JSON.
#[ic_cdk::query]
fn custody_output(args: TransformArgs) -> HttpRequestResult {
    let vout = args.context.as_slice().try_into().map(u32::from_be_bytes).unwrap_or(u32::MAX);
    reply(custody::parse_esplora_output(status_of(&args), &args.response.body, vout).map(|o| serde_json::to_string(&o).unwrap_or_default()))
}

/// esplora's `/tx/<txid>/outspend/<vout>`: `true` or `false`.
#[ic_cdk::query]
fn custody_outspend(args: TransformArgs) -> HttpRequestResult {
    reply(custody::parse_outspend(status_of(&args), &args.response.body).map(|s| s.to_string()))
}

/// esplora's `/scripthash/<h>/utxo`: our CONFIRMED UTXOs, sorted, as JSON (a mempool entry moves between replicas).
#[ic_cdk::query]
fn custody_utxos(args: TransformArgs) -> HttpRequestResult {
    reply(custody::parse_esplora_utxos(status_of(&args), &args.response.body).map(|us| {
        let c: Vec<Utxo> = us.into_iter().filter(|u| u.confirmed).collect();
        serde_json::to_string(&c).unwrap_or_default()
    }))
}

/// esplora's `/fee-estimates`: one bounded whole-number rate.
#[ic_cdk::query]
fn custody_fee(args: TransformArgs) -> HttpRequestResult {
    reply(custody::parse_fee_estimates(status_of(&args), &args.response.body).map(|r| r.to_string()))
}

/// `eth_getBlockByHash(pin)`: the block's hash, timestamp and parent-chain number (`l1BlockNumber` on Nitro; `number` on a
/// chain without one, where `block.number` is its own), as `<hash> <timestamp> <parent>`.
#[ic_cdk::query]
fn custody_block_clock(args: TransformArgs) -> HttpRequestResult {
    let v: serde_json::Value = serde_json::from_slice(&args.response.body).unwrap_or(serde_json::Value::Null);
    let r = v.get("result");
    let q = |k: &str| r.and_then(|b| b.get(k)).and_then(|x| x.as_str()).and_then(|x| x.strip_prefix("0x"))
        .and_then(|x| u64::from_str_radix(x, 16).ok());
    let hash = r.and_then(|b| b.get("hash")).and_then(|x| x.as_str()).map(str::to_ascii_lowercase);
    reply(match (hash, q("timestamp"), q("l1BlockNumber").or_else(|| q("number"))) {
        (Some(h), Some(t), Some(p)) => Ok(format!("{h} {t} {p}")),
        _ => Err("the pinned block has no readable clocks".into()),
    })
}

/// The pinned block's clocks, every provider agreeing, and the block must be the pin (same hash).
async fn block_clock(pin: &Pin) -> Result<(u64, u64), String> {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_getBlockByHash", "params": [pin.hash, false] }).to_string();
    let mut answers = Vec::new();
    for url in &cfg().rpc_urls {
        answers.push(post(url, body.clone(), "custody_block_clock", vec![], 64_000).await?);
    }
    let a = agree("the pinned block's clocks", answers)?;
    let parts: Vec<&str> = a.split(' ').collect();
    if parts.len() != 3 || parts[0] != pin.hash {
        return Err("the clocks are for a block other than the pin".into());
    }
    let n = |s: &str| s.parse::<u64>().map_err(|_| "an unreadable clock".to_string());
    Ok((n(parts[1])?, n(parts[2])?))
}

// ---- the Io -------------------------------------------------------------------------------------------------------------------

pub(crate) struct CustodianIo {
    io: CanisterIo,
    wrapper: [u8; 20],
    /// Phase B2: the Solana home's world and the frog wrapper's Solana address (tickets are the books' `Ticket`s there).
    sol: Option<crate::solhome::SolNet>,
    sol_wrapper: Option<crate::soltx::Pubkey>,
    /// Ordinals multi-collection: the Inscriptions listing (books id) this world serves; its redemptions are `CUSTODY`'s entry of it.
    coll: Option<u16>,
}

crate::sol_io_via!(CustodianIo, sol);

impl CustodianIo {
    pub(crate) fn new(wrapper: &str) -> Result<Self, String> {
        let (w, sol_wrapper) = crate::canister::wrapper_key(wrapper)?;
        Ok(CustodianIo { io: CanisterIo::new()?, wrapper: w, sol: crate::solhome::SolNet::of_install()?, sol_wrapper, coll: None })
    }
    /// The custodian of one Inscriptions listing on the books home: its wX mint names it (`frogsol::books::coll_for`).
    pub(crate) fn for_coll(id: u16, wx: crate::soltx::Pubkey) -> Result<Self, String> {
        Self::for_coll_with(id, wx, crate::solhome::SolNet::of_install()?)
    }
    /// [`CustodianIo::for_coll`] on the tick's one Solana world (audit C2-04: one clock read for every listing's ticket count).
    pub(crate) fn for_coll_with(id: u16, wx: crate::soltx::Pubkey, sol: Option<crate::solhome::SolNet>) -> Result<Self, String> {
        Ok(CustodianIo { io: CanisterIo::new()?, wrapper: crate::frogsol::multi::life_key(&wx), sol, sol_wrapper: Some(wx), coll: Some(id) })
    }
    /// Run `f` on this world's custodian state: its collection's entry, or the single one.
    fn with_custody<R>(&self, f: impl FnOnce(&mut Custody) -> R) -> R {
        CUSTODY.with(|c| {
            let mut c = c.borrow_mut();
            match self.coll {
                Some(id) => f(crate::frogsol::multi::custody_entry_mut(&mut c, id)),
                None => f(&mut c),
            }
        })
    }
    async fn word(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        self.io.eth_read(to, data).await
    }
    async fn registry(&mut self) -> Result<[u8; 20], String> {
        crate::abi::addr_at(&self.word(self.wrapper, &redeem::REGISTRY).await?, 0, 0)
    }
}

impl CustodyIo for CustodianIo {
    fn sol_wrapper(&self) -> Option<crate::soltx::Pubkey> {
        self.sol.as_ref().and(self.sol_wrapper)
    }
    fn now(&self) -> u64 {
        ic_cdk::api::time() / 1_000_000_000 // live: the rule measures the time since a read, including awaits in between
    }
    fn chain_id(&self) -> u64 {
        Io::chain_id(&self.io)
    }
    async fn ticket_count(&mut self) -> Result<(u64, u64), String> {
        // phase B2: the wrapper's `ticket_count` at finalized
        if let Some(w) = CustodyIo::sol_wrapper(self) {
            let hrp = self.btc_hrp()?;
            return crate::frogsol::ticket_count(self, w, hrp).await;
        }
        let pin = self.io.pin().await?;
        let n = crate::abi::uint_at(&self.word(self.wrapper, &redeem::TICKET_COUNT).await?, 0, 0, 64)? as u64;
        Ok((n, pin.number))
    }
    async fn read_ticket(&mut self, id: u64, fresh: bool) -> Result<TicketRead, String> {
        // phase B2: the `Ticket` account at finalized, unix clocks (every read is a fresh one)
        if let Some(w) = CustodyIo::sol_wrapper(self) {
            let now = self.now();
            let hrp = self.btc_hrp()?;
            return crate::frogsol::ticket_read(self, w, id, now, hrp).await;
        }
        if fresh {
            self.io = CanisterIo::new()?; // a new pin, now: the rule's read is "immediately before sending"
        }
        let pin = self.io.pin().await?;
        let t = redeem::decode_ticket(&self.word(self.wrapper, &redeem::call_uint(redeem::TICKET, id as u128)).await?)?;
        let (ts, parent) = block_clock(&pin).await?;
        let now = self.now();
        // The pin is a bucket behind the head. Take the chain's time as at least the canister's, and the parent chain's
        // number advanced by the lag at 12 s a block: both push the deadline NEARER, the safe direction for a rule that stops.
        let lag = now.saturating_sub(ts);
        Ok(TicketRead { stage: t.stage, acquisition_id: t.acquisition_id, destination: t.destination, deliver_by: t.deliver_by,
            deliver_by_block: t.deliver_by_block, chain_time: ts.max(now), parent_block: parent + lag.div_ceil(PARENT_BLOCK_SECONDS),
            read_at: now })
    }
    async fn inscription_of(&mut self, acquisition_id: u128) -> Result<String, String> {
        // phase B2: the ticket's item is the books' `Item`, which carries its inscription id
        if let Some(w) = CustodyIo::sol_wrapper(self) {
            return crate::frogsol::inscription_of(self, w, acquisition_id).await;
        }
        // a frog minted by `wrapDeposit` (bit 255 in the Wrapper, bit 127 here): its leaf, matched against what we minted
        if acquisition_id & crate::deposits::DEPOSIT_BIT != 0 {
            let call = crate::deposits::call_word(crate::deposits::LEAF_OF_FROG, crate::deposits::frog_id_word(acquisition_id));
            let leaf: [u8; 32] = crate::abi::word_at(&self.word(self.wrapper, &call).await?, 0, 0)?.try_into().map_err(|_| "a leaf is one word")?;
            return crate::frognet::inscription_for_leaf(&leaf)
                .ok_or_else(|| format!("deposit frog {} is none this canister minted: nothing is sent", acquisition_id & !crate::deposits::DEPOSIT_BIT));
        }
        let registry = self.registry().await?;
        let listing = crate::abi::uint_at(&self.word(self.wrapper, &redeem::LISTING_ID).await?, 0, 0, 128)?;
        let vault = crate::abi::addr_at(&self.word(registry, &redeem::call_uint(redeem::VAULT_OF, listing)).await?, 0, 0)?;
        let (amount, r) = redeem::decode_acquisition_ref(&self.word(vault, &redeem::call_uint(redeem::ACQUISITION, acquisition_id)).await?)?;
        if amount != 1 {
            return Err(format!("acquisition {acquisition_id} is {amount} units, not one inscription"));
        }
        Ok(r)
    }
    /// Where the inscription is, per two providers agreeing at one height (docs/v6/ordinals-apis.md §3); unverified is an error,
    /// and every flow treats an error here as "read again next tick".
    async fn locate(&mut self, inscription: &str) -> Result<Option<Location>, String> {
        crate::ordnet::locate(inscription).await
    }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> {
        crate::ordnet::contents(o).await
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        let txid = hex::encode(o.txid);
        let (mut outs, mut spent) = (Vec::new(), Vec::new());
        for base in cfg().btc_api_urls {
            outs.push(get(format!("{base}/tx/{txid}"), "custody_output", o.vout.to_be_bytes().to_vec(), 200_000).await
                .map_err(|e| format!("{base}: {e}"))?);
            spent.push(get(format!("{base}/tx/{txid}/outspend/{}", o.vout), "custody_outspend", vec![], 8_000).await
                .map_err(|e| format!("{base}: {e}"))?);
        }
        let out: Output = serde_json::from_str(&agree("the frog's output", outs)?).map_err(|_| "an unreadable output")?;
        Ok((out, agree("whether the frog's output is spent", spent)? == "true"))
    }
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        // H-4: the fee pool first; the address's listing only when the pool has nothing (a stranger can break the listing)
        let pooled = FEE_POOL.with(|p| p.borrow().utxos());
        let mut us = if pooled.is_empty() { self.list_custody_utxos().await? } else { pooled };
        // a fee input a refused deposit's return has claimed is not a delivery's to choose (docs/v3/curve.md, "Canister: …")
        let theirs = crate::frognet::reserved();
        us.retain(|u| !theirs.contains(&u.outpoint));
        Ok(us)
    }
    async fn fee_rate(&mut self) -> Result<u64, String> {
        // a report, bounded: the upper median of the explorers' estimates (audit R2, N-3), and a note when they disagree strongly
        let mut rates = Vec::new();
        for base in cfg().btc_api_urls {
            let r: u64 = get(format!("{base}/fee-estimates"), "custody_fee", vec![], 16_000).await
                .map_err(|e| format!("{base}: {e}"))?.parse().map_err(|_| "an unreadable fee rate")?;
            rates.push(r);
        }
        let (rate, alarm) = custody::choose_rate(&rates)?;
        if let Some(a) = alarm {
            crate::canister::note(format!("custody: {a} ({})", cfg().btc_api_urls.join(", ")));
        }
        Ok(rate)
    }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> {
        crate::canister::custody_script().await // the custody address, never the buyer's
    }
    fn btc_hrp(&self) -> Result<&'static str, String> {
        self.io.btc_hrp()
    }
    async fn sign_taproot(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        crate::canister::sign_taproot_with(crate::canister::CUSTODY_PATH, sighash.to_vec()).await // the custody key, never the buyer's
    }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        self.io.btc_send(raw).await
    }
    async fn btc_seen(&mut self, txid: [u8; 32]) -> BtcView {
        self.io.btc_seen(txid).await
    }
    async fn may_record(&mut self) -> Result<bool, String> {
        // only the wrapper's custodian may record a delivery (Wrapper.recordDelivery reverts NotCustodian for anyone else, the
        // keeper included: docs/audit/2026-09-26/fixes-B-C-keeper-draws.md B-03/C-05)
        if let Some(w) = CustodyIo::sol_wrapper(self) {
            return crate::frogsol::may_record(self, w).await;
        }
        let me = crate::canister::ours().await?.0;
        Ok(crate::abi::addr_at(&self.word(self.wrapper, &redeem::CUSTODIAN).await?, 0, 0)? == me)
    }
    fn wrapper(&self) -> [u8; 20] {
        self.wrapper
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
    fn save(&mut self, r: &Redemption) {
        self.with_custody(|c| match c.redemptions.iter_mut().find(|x| x.ticket == r.ticket) {
            Some(x) => *x = r.clone(),
            None => c.redemptions.push(r.clone()),
        });
    }
    fn save_cursor(&mut self, cursor: u64) {
        self.with_custody(|c| c.cursor = cursor);
    }
    fn ticket_key(&self, ticket: u64) -> String {
        // tickets are numbered per Inscriptions listing: two collections' ticket `n` are two deliveries
        match self.coll {
            Some(id) => format!("ticket:{id}:{ticket}"),
            None => format!("ticket:{ticket}"),
        }
    }
    fn reserved_elsewhere(&self) -> Vec<Outpoint> {
        // one custody address: another collection's delivery in flight holds its fee input
        let Some(me) = self.coll else { return vec![] };
        CUSTODY.with(|c| {
            let c = c.borrow();
            let others = c.colls.iter().flatten().filter(|(id, _)| *id != me).map(|(_, e)| e);
            c.redemptions.iter().chain(others.flat_map(|e| e.redemptions.iter())).flat_map(|r| r.reserved()).collect()
        })
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
    async fn scan_ticket_logs(&mut self, logs: &mut crate::ticketlog::TicketLogs, wanted: Option<&[u64]>) -> Result<String, String> {
        // H9 through events (audit D, H-1): the wrapper's Redeemed and Resolved logs, paged, to the providers' finality; phase B2: none on
        // the Solana home (the ticket accounts are final state)
        if CustodyIo::sol_wrapper(self).is_some() {
            return Ok("no event scan on the Solana home".into());
        }
        crate::ticketlog::advance(&mut crate::canister::ic_rpcs(), logs, &self.wrapper, cfg().wrapper_from_block, wanted).await
    }
    fn outbound_policy(&self, custody_script: &[u8]) -> crate::outbound::BtcPolicy {
        outbound_policy(custody_script)
    }
    fn paused(&self) -> bool {
        crate::canister::not_paused().is_err()
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        crate::frognet::outspend_of(o).await
    }
    fn fee_pool_add(&mut self, o: Outpoint, value: u64) {
        pool_add(o, value, "change");
    }
    fn fee_pool_drop(&mut self, o: &Outpoint) {
        pool_drop(o);
    }
    fn fee_pool_failed(&mut self, o: &Outpoint, why: &str) {
        let now = self.now();
        if let Some(line) = FEE_POOL.with(|p| p.borrow_mut().failed(o, why, now)) {
            crate::canister::note(format!("custody: {line}"));
        }
    }
    fn fee_pool_ok(&mut self, o: &Outpoint) {
        FEE_POOL.with(|p| p.borrow_mut().read_ok(o));
    }
    async fn listed_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        // audit R2, N-4: the pool had candidates and none was usable; with an empty pool `our_utxos` listed already
        if FEE_POOL.with(|p| p.borrow().entries.is_empty()) {
            return Ok(vec![]);
        }
        let mut us = self.list_custody_utxos().await?;
        let theirs = crate::frognet::reserved();
        us.retain(|u| !theirs.contains(&u.outpoint));
        Ok(us)
    }
    async fn btc_depth(&mut self, txid: [u8; 32]) -> Result<Option<u64>, String> {
        if let (Some(id), Some(wx)) = (self.coll, self.sol_wrapper) {
            return crate::frognet::depth_of_coll(id, wx, txid).await;
        }
        crate::frognet::depth_of(&cfg().wrapper.unwrap_or_else(|| format!("0x{}", hex::encode(self.wrapper))), txid).await
    }
    async fn not_destinations(&mut self) -> Result<Vec<Vec<u8>>, String> {
        Ok(vec![crate::frognet::float_script_of().await?, crate::canister::ours().await?.1])
    }
}

/// A script's confirmed UTXOs as every explorer lists them, agreeing (the float's, for a stale buy's conflict-out: audit D, L-5).
pub(crate) async fn utxos_of(script: &[u8]) -> Result<Vec<Utxo>, String> {
    use sha2::{Digest, Sha256};
    let mut h: [u8; 32] = Sha256::digest(script).into();
    h.reverse();
    let mut answers = Vec::new();
    for base in cfg().btc_api_urls {
        answers.push(get(format!("{base}/scripthash/{}/utxo", hex::encode(h)), "custody_utxos", vec![], 400_000).await
            .map_err(|e| format!("{base}: {e}"))?);
    }
    serde_json::from_str(&agree("the UTXOs", answers)?).map_err(|_| "unreadable UTXOs".to_string())
}

impl CustodianIo {
    /// Custody's confirmed UTXOs as every explorer lists them, agreeing. Only for when the fee pool is empty, and for finding
    /// holder deposits: more than 500 and mempool.space refuses the listing (measured 2026-09-26).
    pub(crate) async fn list_custody_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        utxos_of(&self.our_script().await?).await
    }
}

/// The outbound verifier's rules for custody's spends (H4): both launch-gate requirements on (C4).
pub(crate) fn outbound_policy(custody_script: &[u8]) -> crate::outbound::BtcPolicy {
    crate::outbound::BtcPolicy { custody_script: custody_script.to_vec(), max_fee_sats: crate::outbound::MAX_FEE_SATS,
        require_finalized: true, require_event: true }
}

/// Belt and braces for the BUYER: the providers' account of an output it is about to sign as its own. Where providers exist
/// (Bitcoin mainnet, or an ord the operator added) an output whose contents do not VERIFY refuses the signature (fail closed).
/// On a test network with no provider at all the check is not made, the log says so, and custody's separate key is the only
/// defence (docs/v3/wrapper.md, "Custody on its own key").
pub(crate) async fn buyer_input_clean(o: Outpoint) -> Result<(), String> {
    let ps = crate::ordnet::providers();
    if ps.is_empty() {
        crate::canister::note(format!("{}: not checked for inscriptions or runes: no ordinals provider serves {}", o.text(), cfg().btc_network));
        return Ok(());
    }
    let c = crate::ordnet::contents_with(&ps, &o).await
        .map_err(|e| format!("refused: {} could not be verified ({e}), so it is not signed", o.text()))?;
    custody::buyer_may_sign(&o, c.as_ref())
}

// ---- the tick's hook, and what anyone may read ----------------------------------------------------------------------------------

/// Whether a delivery's own EVM transaction is out (for the buyer's one-EVM-transaction-at-a-time rule).
pub(crate) fn evm_pending() -> bool {
    CUSTODY.with(|c| crate::frogsol::multi::custodies(&c.borrow()).into_iter().any(|c| c.redemptions.iter().any(|r| r.evm_pending())))
}

/// Fee inputs every collection's deliveries in flight have claimed (one custody address).
pub(crate) fn reserved() -> Vec<Outpoint> {
    CUSTODY.with(|c| crate::frogsol::multi::custodies(&c.borrow()).into_iter().flat_map(|c| c.redemptions.iter().flat_map(|r| r.reserved()))
        .collect())
}

/// The custodian's pass, run by `tick` after the purchases when a wrapper is configured. On the books home: every Inscriptions
/// listing's tickets, live or dormant (a holder's exit is never a collection's to stop), one world each.
pub(crate) async fn tick(wrapper: &str, purchases_evm_pending: bool) -> Vec<String> {
    if crate::frognet::per_collection() {
        return tick_collections(purchases_evm_pending).await;
    }
    let mut io = match CustodianIo::new(wrapper) {
        Ok(io) => io,
        Err(e) => return vec![format!("custody: not configured: {e}")],
    };
    // I-3: every explorer shows the installed network's checkpoint before custody believes a read from it
    if let Err(e) = crate::btcnet::ensure_btc_network().await {
        return vec![format!("custody: Bitcoin not read: {e}")];
    }
    let mut c = CUSTODY.with(|c| c.borrow().clone());
    let lines = redeem::tick(&mut io, &mut c, purchases_evm_pending).await;
    CUSTODY.with(|x| *x.borrow_mut() = c);
    lines
}

/// The books home: each Inscriptions listing's redemptions with its own world and its own entry of `CUSTODY`.
async fn tick_collections(purchases_evm_pending: bool) -> Vec<String> {
    let all = crate::frogsol::multi::listings();
    if all.is_empty() {
        return vec![];
    }
    if let Err(e) = crate::btcnet::ensure_btc_network().await {
        return vec![format!("custody: Bitcoin not read: {e}")];
    }
    crate::frognet::ensure_migrated();
    let mut lines = Vec::new();
    // audit C2-04: one Solana world for every listing's custodian (one clock read a tick, not one per listing), handed on to the frog
    // side's pass of the same tick
    let (sol_at, mut sol) = match crate::frognet::tick_sol() {
        Ok(x) => x,
        Err(e) => return vec![format!("custody: not configured: {e}")],
    };
    for l in all {
        let mut io = match crate::frognet::next_sol(&mut sol).and_then(|s| CustodianIo::for_coll_with(l.id, l.wx_mint, s)) {
            Ok(io) => io,
            Err(e) => {
                lines.push(format!("custody: collection {}: not configured: {e}", l.id));
                continue;
            }
        };
        let mut c = CUSTODY.with(|c| crate::frogsol::multi::custody_entry(&c.borrow(), l.id));
        let out = redeem::tick(&mut io, &mut c, purchases_evm_pending).await;
        CUSTODY.with(|x| crate::frogsol::multi::put_custody(&mut x.borrow_mut(), l.id, c));
        lines.extend(out.into_iter().map(|s| format!("collection {}: {s}", l.id)));
        sol = io.sol.take();
    }
    crate::frognet::keep_tick_sol(sol_at, sol);
    lines
}

/// Every redemption and where it is (every Inscriptions listing's on the books home; each names its ticket in its listing).
#[ic_cdk::query]
pub fn redemptions() -> Vec<Redemption> {
    CUSTODY.with(|c| crate::frogsol::multi::custodies(&c.borrow()).into_iter().flat_map(|c| c.redemptions.clone()).collect())
}

/// The ticket-log scanner (audit D, H-1): how far it has read, finality at the last read, and whether it is failing. The watcher
/// alerts on `failures`.
#[ic_cdk::query]
pub fn ticket_logs() -> Option<crate::ticketlog::TicketLogs> {
    CUSTODY.with(|c| c.borrow().logs.clone())
}

/// Custody's fee inputs as the canister knows them (audit D, H-4).
#[ic_cdk::query]
pub fn custody_fee_pool() -> custody::FeePool {
    FEE_POOL.with(|p| p.borrow().clone())
}

/// The operator registers a plain-sat custody output as a fee input (audit D, H-4). Only a claim: every use checks it on every
/// explorer (custody's script, confirmed, unspent, this value) and on ord (carrying nothing) before it pays a fee.
#[ic_cdk::update]
pub fn add_custody_fee_input(outpoint: String, value: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let o = Outpoint::parse(outpoint.trim())?;
    FEE_POOL.with(|p| p.borrow_mut().add(o, value, "operator"))?;
    Ok(format!("{} ({value} sats) is a fee input candidate; it is verified at use", o.text()))
}

/// The operator removes a fee input from the pool, a candidate or a quarantined one (audit R2, N-4): a mistyped top-up, or one
/// spent elsewhere. Adding it again restores it.
#[ic_cdk::update]
pub fn remove_custody_fee_input(outpoint: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let o = Outpoint::parse(outpoint.trim())?;
    if FEE_POOL.with(|p| p.borrow_mut().remove(&o)) {
        Ok(format!("{} is no longer a fee input candidate", o.text()))
    } else {
        Err(format!("{} is not in the fee pool", o.text()))
    }
}

/// For monitoring: `(ticket, txid)` for every frog delivered on Bitcoin and not yet recorded on chain, txid in display order. Only
/// the wrapper's custodian (this canister) can record one; until it does, the ticket can be refunded after `deliverBy` + grace.
#[ic_cdk::query]
pub fn pending_delivery_records() -> Vec<(u64, String)> {
    CUSTODY.with(|c| crate::frogsol::multi::custodies(&c.borrow()).into_iter().flat_map(|c| c.redemptions.iter())
        .filter_map(|r| r.record_owed().map(|t| (r.ticket, hex::encode(t)))).collect())
}

/// Read-only rehearsal on Bitcoin MAINNET, whatever this canister is installed for: where mainnet's providers agree
/// `inscription_id` is, what every mainnet explorer says about that output, and whether this canister could send it from there
/// (`custody::frog_at` against our own taproot script). Nothing is signed or sent.
#[ic_cdk::update]
pub async fn probe_frog(coll: u16, inscription_id: String) -> Result<String, String> {
    only_operator();
    custody::check_inscription_id(&inscription_id)?;
    // the books home: the collection must be an Inscriptions listing; whether a proof handed in for it verifies against its root
    let member = if crate::frognet::per_collection() {
        let l = crate::frogsol::multi::listing(coll)?;
        let proof = crate::frognet::FROGS.with(|f| crate::frogsol::multi::entry(&f.borrow(), coll).and_then(|s| s.proof(&inscription_id)));
        match proof {
            Some(p) if crate::frogbuy::verify_proof(&p, &l.root, &crate::frogbuy::leaf_of(&inscription_id)) => format!("collection {coll}: its proof verifies"),
            Some(_) => format!("collection {coll}: its proof does NOT verify against the root"),
            None => format!("collection {coll}: no proof handed in"),
        }
    } else {
        String::new()
    };
    let ps = crate::ordnet::mainnet_now();
    let Some(loc) = crate::ordnet::locate_with(&ps, &inscription_id).await? else {
        return Ok(format!("no provider knows {inscription_id}"));
    };
    let contents = crate::ordnet::contents_with(&ps, &loc.output).await?;
    let txid = hex::encode(loc.output.txid);
    let (mut outs, mut spent) = (Vec::new(), Vec::new());
    for base in ["https://mempool.space/api", "https://blockstream.info/api"] {
        outs.push(get(format!("{base}/tx/{txid}"), "custody_output", loc.output.vout.to_be_bytes().to_vec(), 200_000).await?);
        spent.push(get(format!("{base}/tx/{txid}/outspend/{}", loc.output.vout), "custody_outspend", vec![], 8_000).await?);
    }
    let out: Output = serde_json::from_str(&agree("the output", outs)?).map_err(|_| "output")?;
    let is_spent = agree("spent", spent)? == "true";
    let ours = crate::canister::custody_script().await?;
    let verdict = match custody::frog_at(&inscription_id, &loc, contents.as_ref(), &out, is_spent, &ours) {
        Ok(f) => format!("SENDABLE from {} ({} sats)", f.output.text(), f.value),
        Err(custody::NotSendable::Wait(w)) => format!("WAIT: {w}"),
        Err(custody::NotSendable::Stuck(s)) => format!("NOT SENDABLE: {s}"),
    };
    Ok(format!("{inscription_id}{}: {} agree: at {} offset {} value {}; carries {:?}; explorers agree: value {} confirmed {} spent {} \
        script {}; ours {}; verdict: {verdict}", if member.is_empty() { String::new() } else { format!(" ({member})") }, ps.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join("+"), loc.output.text(), loc.offset,
        loc.value, contents, out.value, out.confirmed, is_spent, hex::encode(&out.script), hex::encode(&ours)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_update_method_here_is_operator_only() {
        let src = include_str!("custodynet.rs");
        let parts: Vec<&str> = src.split(concat!("#[ic_cdk::update]", "\npub async fn")).skip(1).collect();
        assert!(!parts.is_empty());
        for part in parts {
            let head: String = part.chars().take(300).collect();
            assert!(head.contains("only_operator();"), "unguarded: {head}");
        }
    }
}
