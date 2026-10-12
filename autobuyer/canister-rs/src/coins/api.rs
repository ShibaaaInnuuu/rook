//! The coins on the canister's surface: the IC world of the tick, the operator's pins, and the public queries.
use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::chainlink::driver::DeskIo;
use crate::solkey::KeyPath;
use crate::soltx::{b58_encode, Pubkey};

use super::state::*;

pub(crate) struct CoinsNet {
    sol: Option<crate::solhome::SolNet>,
}

crate::sol_io_via!(CoinsNet, sol);

impl DeskIo for CoinsNet {
    async fn key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String> {
        crate::solkey::derive_key(p).await
    }
}

fn now_secs() -> u64 {
    ic_cdk::api::time() / 1_000_000_000
}

/// The coins' pass for the canister's timer and the runner's tick (the tick's flag held by the caller). Nothing without a Solana home.
pub(crate) async fn tick() -> Vec<String> {
    if crate::canister::cfg().home.is_none() {
        return vec![];
    }
    match crate::solhome::SolNet::of_install() {
        Ok(Some(sol)) => super::run::tick(&mut CoinsNet { sol: Some(sol) }, now_secs()).await,
        Ok(None) => vec![],
        Err(e) => vec![format!("coins: waits: {e}")],
    }
}

/// One collection as the operator and the site see it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CollectionView {
    pub index: u32,
    pub symbol: String,
    pub stage: String,
    pub wx_mint: String,
    pub config: String,
    /// `M[i]`, the module's address (empty before the collection was ever looked at).
    pub module: String,
    pub coins: u32,
    pub backed: u32,
}

/// One graduated coin's vault book and address.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct VaultView {
    pub coll: u32,
    pub coin: String,
    pub pool: String,
    pub stage: String,
    pub vault: Option<String>,
    pub wx_in: u64,
    pub coins_burned: u64,
    pub wx_paid: u64,
    pub backed_at: u64,
}

pub fn collections() -> Vec<CollectionView> {
    with(|s| {
        s.pins
            .values()
            .map(|p| {
                let col = s.cols.get(&p.index);
                let coins = s.coins.values().filter(|c| c.coll == p.index);
                // a pinned collection a creator asked to open shows `Requested` until its first step is signed (then `Opening`)
                let requested = s.requested.iter().any(|(i, _)| *i == p.index);
                CollectionView {
                    index: p.index,
                    symbol: p.symbol.clone(),
                    stage: match col.map(|c| &c.stage) {
                        Some(ColStage::Pinned) | None if requested => "Requested".into(),
                        Some(st) => format!("{st:?}"),
                        None => "Pinned".into(),
                    },
                    wx_mint: col.map(|c| b58_encode(&c.wx_mint)).unwrap_or_default(),
                    config: col.map(|c| b58_encode(&c.config)).unwrap_or_default(),
                    module: col.filter(|c| c.module != [0; 32]).map(|c| b58_encode(&c.module)).unwrap_or_default(),
                    coins: coins.clone().count() as u32,
                    backed: coins.filter(|c| c.stage == CoinStage::Backed).count() as u32,
                }
            })
            .collect()
    })
}

pub fn vault_view(c: &CoinRec) -> VaultView {
    let v = c.vault.clone().unwrap_or_default();
    VaultView { coll: c.coll, coin: b58_encode(&c.coin), pool: b58_encode(&c.pool), stage: format!("{:?}", c.stage), vault: c.vault.as_ref().map(|v| b58_encode(&v.address)),
        wx_in: v.wx_in, coins_burned: v.coins_burned, wx_paid: v.wx_paid, backed_at: v.backed_at }
}

pub fn coin_vaults(coll: u32) -> Vec<VaultView> {
    with(|s| s.coins.values().filter(|c| c.coll == coll).map(vault_view).collect())
}

pub fn coin_vault(coin: &Pubkey) -> Option<VaultView> {
    with(|s| s.coins.get(coin).map(vault_view))
}

// ---- the surface ----------------------------------------------------------------------------------------------------------------

/// Every pinned collection: its stage, its wX mint and DBC config, its coins (§10.1).
#[ic_cdk::query]
pub fn coins_collections() -> Vec<crate::coins::api::CollectionView> {
    collections()
}

/// The vault books of a collection's coins, and one coin's (§10.1 `coin_vaults(coll)`, `coin_vault(coin)`).
#[ic_cdk::query]
pub fn coins_vaults(coll: u32) -> Vec<crate::coins::api::VaultView> {
    coin_vaults(coll)
}

#[ic_cdk::query]
pub fn coins_vault(coin: String) -> Result<Option<crate::coins::api::VaultView>, String> {
    Ok(coin_vault(&crate::soltx::pubkey(&coin)?))
}

/// The coins' fee books, waits, last landings, and the opening queue (`(collection, asked at)`, oldest first: `coins_request_open`).
#[ic_cdk::query]
pub fn coins_log() -> (Vec<(u32, crate::coins::state::FeeBook)>, Vec<crate::coins::state::Wait>, Vec<(String, u64, String)>, Vec<(u32, u64)>) {
    with(|s| (s.fees.iter().map(|(c, f)| (*c, f.clone())).collect(), s.waits.clone(), s.last.clone(), s.requested.clone()))
}

/// The operator pins a collection (its wX name, its curve, its books parameters): opened when a creator first asks for it
/// (`coins_request_open`), or in index order under `coins_set_auto_open(true)`, as the float allows.
#[ic_cdk::update]
pub fn coins_pin(pin: crate::coins::state::CollectionPin) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    let i = pin.index;
    super::open::register(pin)?;
    Ok(format!("collection {i} pinned"))
}

/// Anyone (the anonymous principal too) asks a pinned collection to open: the first creator to launch a coin on it (operator decision
/// 2026-10-12, "we don't select, the people do"). Idempotent; refused for an index that is not pinned, open or refused, whose pipeline
/// is not ready (audit C4-03), while the module is off or the install has no home (C4-11), and beyond the queue's bounds (C4-02:
/// `open::check_request`). The canister opens the requested collections oldest first, ≈ 0.023 SOL from the float each, on its next
/// ticks; `coins_collections` shows `Requested`, then `Opening`, then `Open`; `coins_open_queue` the queue and what it waits on.
#[ic_cdk::update]
pub fn coins_request_open(index: u32) -> Result<String, String> {
    crate::canister::not_paused()?;
    if !crate::canister::has_home() {
        return Err("the canister has no Solana home: no collection opens".into());
    }
    super::open::request_open(index, ic_cdk::api::msg_caller().as_slice(), now_secs())
}

/// The methods anyone may call (the ingress filter admits them: `canister::may_ingress`).
pub fn is_public(method: &str) -> bool {
    method == "coins_request_open"
}

/// Audit C4-05: whether a stranger's `coins_request_open` ingress is worth accepting: its argument decodes as one `nat32`, and the call
/// would queue the collection (`open::check_request` gives `Ask::New`: not a no-op, not refused by the limits). `inspect_message`
/// accepts nothing else, so a call that would change nothing costs the canister nothing.
pub fn may_request(arg: &[u8], caller: &[u8], chains: &[u64], now: u64) -> bool {
    let Ok(index) = candid::decode_one::<u32>(arg) else { return false };
    with(|s| matches!(super::open::check_request(s, index, caller, chains, now), Ok(super::open::Ask::New { .. })))
}

/// `inspect_message`'s check of a stranger's `coins_request_open` (audit C4-05): the pause, the home, then [`may_request`].
pub(crate) fn ingress_request(arg: &[u8], caller: &[u8]) -> bool {
    !crate::canister::is_paused() && crate::canister::has_home() && may_request(arg, caller, &crate::canister::evm_chain_ids(), now_secs())
}

/// One pending opening request as the site shows it (audit C4-12): its place, and what the canister waits on for it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OpenStatus {
    pub index: u32,
    /// 0: opens next.
    pub position: u32,
    pub asked_at: u64,
    /// Since when the opening pass waits on it (0: it does not); `why`: the pass's last reason ("" when none).
    pub waiting_since: u64,
    pub why: String,
    /// Audit C4-04: the pass moved past it to later requests (it waits on something of its own); it keeps its place.
    pub skipped: bool,
}

pub fn open_queue(now: u64) -> Vec<OpenStatus> {
    with(|s| {
        s.requested
            .iter()
            .enumerate()
            .map(|(p, (i, at))| {
                let st = s.stalls.get(i);
                OpenStatus { index: *i, position: p as u32, asked_at: *at, waiting_since: st.map(|x| x.since).unwrap_or(0),
                    why: st.map(|x| x.why.clone()).unwrap_or_default(), skipped: st.is_some_and(|x| super::open::is_skipped(x, now)) }
            })
            .collect()
    })
}

/// The opening queue, oldest request first, with what the canister waits on for each (audit C4-12: the site shows the truth).
#[ic_cdk::query]
pub fn coins_open_queue() -> Vec<crate::coins::api::OpenStatus> {
    open_queue(now_secs())
}

/// Audit C4-01: the operator clears a collection's refusal (once the cause is looked at) and queues it again under its own index and
/// symbol; the pass reads its chain side afresh and refuses again what is still wrong.
#[ic_cdk::update]
pub fn coins_retry(index: u32) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    super::open::retry_refused(index, now_secs())
}

/// The operator's switch: with no request pending, open the pinned collections in index order anyway (the float left over), or (the
/// default, off) only on request. Audit C4-11: switching it on starts spending, so it waits for the pause like any act; switching it
/// off only stops, and stays allowed while paused.
#[ic_cdk::update]
pub fn coins_set_auto_open(on: bool) -> Result<String, String> {
    crate::canister::only_operator();
    if on { crate::canister::not_paused()?; }
    super::open::set_auto_open(on, now_secs());
    Ok(format!("auto-open is {}: {}", if on { "on" } else { "off" }, if on { "pinned collections open in index order as the float allows" } else { "a collection opens when someone first asks for it" }))
}

/// The operator switches the coins module off (nothing signs) or on again.
#[ic_cdk::update]
pub fn coins_set_off(off: bool) -> Result<String, String> {
    crate::canister::only_operator();
    with_mut(|s| s.off = off);
    Ok(format!("the coins module is {}", if off { "off" } else { "on" }))
}

/// The operator leaves an account's coins out of the supply a redemption divides by (§7), or counts it again: applied by the coins'
/// pass a day later (audit B3-11: it moves every redemption's value, as the treasury's setters do, so it waits as they do); asked
/// again before then, the later ask replaces the earlier and the clock restarts.
#[ic_cdk::update]
pub fn coins_exclude_from_supply(account: String, excluded: bool) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    let k = crate::soltx::pubkey(&account)?;
    let at = ic_cdk::api::time() / 1_000_000_000 + crate::books::treasury::TREASURY_DELAY as u64;
    with_mut(|s| {
        s.pending_exclusions.retain(|(x, ..)| *x != k);
        s.pending_exclusions.push((k, excluded, at));
    });
    Ok(format!("{account} is {} from {at} (unix), unless asked otherwise before", if excluded { "left out of the supply" } else { "counted in the supply" }))
}

/// The accounts left out of a redemption's supply, and the changes waiting their day (`(account, excluded, when)`), base58.
#[ic_cdk::query]
pub fn coins_exclusions() -> (Vec<String>, Vec<(String, bool, u64)>) {
    with(|s| (s.exclude_from_supply.iter().map(|k| crate::soltx::b58_encode(k)).collect(),
        s.pending_exclusions.iter().map(|(k, e, at)| (crate::soltx::b58_encode(k), *e, *at)).collect()))
}
