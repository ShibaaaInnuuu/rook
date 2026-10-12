//! Opening a collection (W6 a, `open_collection`): the wX mint with its Metaplex metadata, the DBC config with the pinned parameters, the
//! config's read-back (`solix2::guard::check_config`), and the collection's listing in the books. Paid from `K_main`'s float, which the fees
//! fill (§16), in the unlock order (`unlock`).
//!
//! **Level-triggered.** Every pass reads the chain side of the next collection (mint, metadata, config) and does what is missing, so a
//! crash, a lost transaction or a ceremony that already made some of it needs no record of progress: the chain is the record. The wX mint
//! is a `create_with_seed` account of `K_main` (so only `K_main` signs for the address: no throwaway key), its mint authority is `K_coll[i]`
//! from the start (no `SetAuthority`: the canister never signs one) and its freeze authority is none; the config's address is
//! `KeyPath::Config(i)` (DBC makes the new config account sign). When the launch file pins a mint or a config address (the ceremony made
//! it), the canister only reads it back.
use crate::books::{BooksError, ListingKind};
use crate::chainlink::driver::DeskIo;
use crate::desk::state as desk;
use crate::solhome::{LineOwner, LineStep};
use crate::solix2::decode::{DbcConfig, Mint};
use crate::solix2::guard::{self, ConfigPin};
use crate::solix2::{self, dbc, mpl, spl};
use crate::solkey::KeyPath;
use crate::soltx::{self, b58_encode, Instruction, Pubkey};

use super::state::*;
use super::step::CoinStep;
use super::unlock;

/// Lamports kept at `K_main` for the money flows: a step that pays rent waits below it (as the keeper's cranks do).
pub const RESERVE_LAMPORTS: u128 = 50_000_000;

/// The seed of collection `index`'s wX mint account (`create_with_seed` under `K_main`).
pub fn mint_seed(index: u32) -> String {
    format!("wx:{index}")
}

/// The address a canister-made wX mint has: `createWithSeed(K_main, "wx:<i>", the Token program)`.
pub fn wx_mint_of(k_main: &Pubkey, index: u32) -> Pubkey {
    soltx::address_with_seed(k_main, &mint_seed(index), &solix2::token_program()).expect("a short seed")
}

/// The addresses one collection's opening uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    pub main: Pubkey,
    pub coll: Pubkey,
    pub fc: Pubkey,
    pub cfg_key: Pubkey,
}

pub async fn keys_of<I: DeskIo>(io: &mut I, index: u32) -> Result<Keys, String> {
    Ok(Keys { main: io.key_of(&KeyPath::Main).await?, coll: io.key_of(&KeyPath::Coll(index)).await?, fc: io.key_of(&KeyPath::Claimer).await?,
        cfg_key: io.key_of(&KeyPath::Config(index)).await? })
}

/// The wX mint transaction (§2): create the account, `InitializeMint2` with `K_coll[i]` as the mint authority and no freeze authority,
/// and the metadata (name, symbol, uri; update authority `K_coll[i]`). Signers: `K_main` and `K_coll[i]`.
///
/// Audit C4-01: `held` is what the derived address holds already. Anyone can transfer lamports to it (its address is public), and
/// Solana's `CreateAccountWithSeed` refuses an address holding lamports (`AccountAlreadyInUse`); nobody but `K_main` can allocate or
/// assign it. So a pre-funded address is made in place, as the Associated Token Account program does: `AllocateWithSeed` (82 bytes,
/// owner the Token program, signed by `K_main` as the base) and a top-up `Transfer` to the rent when it holds less.
pub fn mint_tx(k: &Keys, mint: Pubkey, rent: u64, pin: &CollectionPin, held: u64) -> Vec<Instruction> {
    let seed = mint_seed(pin.index);
    let mut v = if held == 0 {
        vec![soltx::create_with_seed(k.main, mint, k.main, &seed, rent, spl::MINT_LEN, solix2::token_program())]
    } else {
        let mut v = vec![soltx::allocate_with_seed(mint, k.main, &seed, spl::MINT_LEN, solix2::token_program())];
        if held < rent {
            v.push(soltx::transfer(k.main, mint, rent - held));
        }
        v
    };
    v.push(spl::initialize_mint2(mint, k.coll, None, 9));
    v.push(mpl::create_metadata_v3(mint, k.coll, k.main, k.coll, &pin.name, &pin.symbol, &pin.uri));
    v
}

/// Audit C4-01: an account an outsider can leave at a derived address (a plain transfer: owned by the System program, no data). The
/// opening treats it as "not made yet" and makes its account over it; anything else there is still refused.
pub fn squatted(a: &crate::solrpc::Account) -> bool {
    a.owner == soltx::SYSTEM_PROGRAM && a.data.is_empty()
}

/// The DBC config transaction (§3.1): Rook's pinned parameters with the pinned curve, quote = wX, `K_fc` the claimer and the leftover
/// receiver. Signers: `K_main` (payer) and the config's own key.
pub fn config_tx(k: &Keys, wx_mint: Pubkey, pin: &CollectionPin) -> Vec<Instruction> {
    vec![dbc::create_config(k.cfg_key, k.fc, k.fc, wx_mint, k.main, &dbc::rook_wx_config(pin.t_wx, pin.sqrt_start_price, pin.curve.clone()))]
}

pub fn config_pin(k: &Keys, wx_mint: Pubkey, pin: &CollectionPin) -> ConfigPin {
    ConfigPin { wx_mint, k_fc: k.fc, t_wx: pin.t_wx, sqrt_start_price: pin.sqrt_start_price, curve: pin.curve.clone() }
}

/// The metadata account as the opening must leave it: its update authority `K_coll[i]` and its mint (Token Metadata: key u8, update
/// authority @1, mint @33).
pub fn check_metadata(owner: &Pubkey, data: &[u8], k_coll: &Pubkey, mint: &Pubkey) -> Result<(), String> {
    if *owner != solix2::id(solix2::METADATA_PROGRAM) {
        return Err(format!("the wX metadata is owned by {}, not Token Metadata", b58_encode(owner)));
    }
    if data.len() < 65 {
        return Err("the wX metadata account is too short".into());
    }
    if data[1..33] != k_coll[..] {
        return Err(format!("the wX metadata's update authority is {}, not K_coll", b58_encode(&<[u8; 32]>::try_from(&data[1..33]).expect("32"))));
    }
    if data[33..65] != mint[..] {
        return Err("the wX metadata is of another mint".into());
    }
    Ok(())
}

/// Pins one collection (the launch file, at install or by the operator). Refused when it is malformed or its index is taken.
pub fn register(pin: CollectionPin) -> Result<(), String> {
    if pin.curve.is_empty() || pin.curve.len() > crate::solix2::decode::MAX_CURVE_POINTS {
        return Err(format!("collection {}: the curve has {} points (1 to {})", pin.index, pin.curve.len(), crate::solix2::decode::MAX_CURVE_POINTS));
    }
    if pin.t_wx == 0 || pin.sqrt_start_price == 0 {
        return Err(format!("collection {}: a zero threshold or start price", pin.index));
    }
    // audit B3-10: the curve as DBC will check it (a refused create_config would otherwise wedge the unlock order behind this index):
    // strictly rising sqrt prices above the start, positive liquidity, and the whole curve raising at least the threshold
    let mut lo = pin.sqrt_start_price;
    for (i, (sqrt_price, liquidity)) in pin.curve.iter().enumerate() {
        if *sqrt_price <= lo {
            return Err(format!("collection {}: curve point {i} does not rise (sqrt price {sqrt_price} after {lo})", pin.index));
        }
        if *liquidity == 0 {
            return Err(format!("collection {}: curve point {i} has no liquidity", pin.index));
        }
        lo = *sqrt_price;
    }
    let threshold = pin.t_wx.saturating_mul(crate::solix2::dbc::pins::WX_ONE) as u128;
    let raised = crate::solix2::guard::quote_raised(pin.sqrt_start_price, &pin.curve, u128::MAX);
    if raised + threshold / 1_000 < threshold {
        return Err(format!("collection {}: the curve raises {raised} base units of wX in all, under the threshold T_wX {} = {threshold}", pin.index, pin.t_wx));
    }
    if pin.symbol.is_empty() || pin.symbol.len() > 10 || pin.name.is_empty() || pin.name.len() > 32 || pin.uri.len() > 200 {
        return Err(format!("collection {}: Metaplex wants a symbol of ≤ 10 bytes, a name of ≤ 32 and a uri of ≤ 200", pin.index));
    }
    if pin.index > u16::MAX as u32 {
        return Err(format!("collection {}: the books' ids are u16", pin.index));
    }
    crate::books::params::validate_params(&pin.params).map_err(|e| format!("collection {}: {e}", pin.index))?;
    with_mut(|s| {
        if s.pins.contains_key(&pin.index) {
            return Err(format!("collection {} is already pinned", pin.index));
        }
        if s.pins.values().any(|p| p.symbol == pin.symbol) {
            return Err(format!("symbol {} is already pinned", pin.symbol));
        }
        s.pins.insert(pin.index, pin);
        Ok(())
    })
}

/// What a pass of the opening found and did.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// Nothing is left to open (or nothing is pinned).
    Nothing,
    /// A step was signed (its signature).
    Signed(String),
    /// The collection is open now (listed in the books).
    Open(u32),
    /// Not now: the reason.
    Waits(String),
}

/// Ethereum mainnet: its EthNft listings need no more than the install (the Ethereum pipeline is the canister's own).
pub const ETHEREUM: u64 = 1;

/// Audit C4-03: whether a pin's pipeline is ready to open on. An `EthNft` listing on a chain other than Ethereum (the Robinhood Chain
/// collections, chain 4663) needs that chain's `evm_chains` entry, which the ceremony adds only after its 4663 read-back; an
/// `Inscriptions` listing needs nothing more (every Ordinals listing is live by default, operator decision 2026-10-12); a `SolNft`
/// collection is the Solana NFT flow's.
pub fn pipeline_ready(kind: &ListingKind, evm_chains: &[u64]) -> Result<(), String> {
    match kind {
        ListingKind::EthNft { chain_id, .. } if *chain_id != ETHEREUM && !evm_chains.contains(chain_id) => Err(format!(
            "its chain {chain_id} is not in the canister's evm_chains yet (a {} collection opens after the chain's read-back)",
            listing_label(kind))),
        _ => Ok(()),
    }
}

/// Whether a pinned collection still waits to be opened (neither Open nor Refused) and its pipeline is ready (audit C4-03).
fn openable(s: &CoinsState, i: u32, chains: &[u64]) -> bool {
    s.pins.get(&i).is_some_and(|p| pipeline_ready(&p.kind, chains).is_ok())
        && !matches!(s.cols.get(&i).map(|c| &c.stage), Some(ColStage::Open) | Some(ColStage::Refused(_)))
}

/// Audit C4-04: a requested collection the pass waited on for a reason of its own this long is skipped for the requests behind it; it
/// keeps its place and is looked at again every [`STALLED_LOOK_EVERY`].
pub const HEAD_PATIENCE: u64 = 1_800;
pub const STALLED_LOOK_EVERY: u64 = 3_600;

/// Audit C4-02: the opening queue's bounds. At most [`MAX_QUEUED`] requests pending (a day of openings at the pace), at most
/// [`REQUESTS_PER_HOUR`] accepted an hour from everyone, [`ANON_REQUESTS_PER_HOUR`] of them from the anonymous principal (the site's
/// visitors, all one caller), and from a named principal one pending request at a time and [`CALLER_REQUESTS_PER_HOUR`] an hour.
pub const MAX_QUEUED: usize = OPENS_PER_DAY as usize;
pub const REQUESTS_PER_HOUR: usize = 4;
pub const ANON_REQUESTS_PER_HOUR: usize = 2;
pub const CALLER_REQUESTS_PER_HOUR: usize = 2;

/// The anonymous principal's bytes (`Principal::anonymous()`).
pub const ANONYMOUS: &[u8] = &[4];

/// Whether a stall is skipped now: its own wait, older than the patience, looked at less than [`STALLED_LOOK_EVERY`] ago.
pub fn is_skipped(st: &Stall, now: u64) -> bool {
    st.own && now.saturating_sub(st.since) >= HEAD_PATIENCE && now.saturating_sub(st.last_look) < STALLED_LOOK_EVERY
}

/// The next collection to open (operator decision 2026-10-12, "the people choose"): the OLDEST REQUESTED collection first
/// (`request_open`), whatever its index; with no request pending, nothing — unless the operator's `auto_open` switch is on, which opens
/// the lowest pinned index that is neither open nor refused, as the earlier design did. Entries that are open or refused by now are
/// dropped from the queue here. Audit C4-04: a request the pass has waited on for a reason of its own for [`HEAD_PATIENCE`] is passed
/// over for the next (it keeps its place, and is looked at again every [`STALLED_LOOK_EVERY`]).
fn next_pinned(chains: &[u64], now: u64) -> Option<u32> {
    with_mut(|s| {
        let pending: Vec<(u32, u64)> = s.requested.iter().copied().filter(|(i, _)| s.pins.contains_key(i)
            && !matches!(s.cols.get(i).map(|c| &c.stage), Some(ColStage::Open) | Some(ColStage::Refused(_)))).collect();
        s.requested = pending;
        let kept: Vec<u32> = s.requested.iter().map(|(i, _)| *i).collect();
        s.requesters.retain(|i, _| kept.contains(i));
        let stale: Vec<u32> = s.stalls.keys().copied().filter(|i| !kept.contains(i) && !(s.auto_open && openable(s, *i, chains))).collect();
        for i in stale {
            s.stalls.remove(&i);
        }
        if let Some((i, _)) = s.requested.iter().find(|(i, _)| openable(s, *i, chains) && !s.stalls.get(i).is_some_and(|st| is_skipped(st, now))) {
            return Some(*i);
        }
        if !s.requested.is_empty() || !s.auto_open {
            return None;
        }
        s.pins.keys().copied().find(|i| openable(s, *i, chains) && !s.stalls.get(i).is_some_and(|st| is_skipped(st, now)))
    })
}

/// What a request would do: queue the collection, or nothing (it is queued already, at this position).
#[derive(Debug, PartialEq, Eq)]
pub enum Ask {
    New { ahead: usize },
    Already { ahead: usize },
}

/// Every check of a request (audit C4-02, C4-03, C4-05, C4-11), reading only: `request_open` applies it, `inspect_message` refuses the
/// ingress of anything but [`Ask::New`] (a call that would change nothing costs the canister nothing).
pub fn check_request(s: &CoinsState, index: u32, caller: &[u8], chains: &[u64], now: u64) -> Result<Ask, String> {
    if s.off {
        return Err("the coins module is off: no collection opens for now".into());
    }
    let Some(pin) = s.pins.get(&index) else { return Err(format!("collection {index} is not pinned")) };
    let symbol = &pin.symbol;
    match s.cols.get(&index).map(|c| &c.stage) {
        Some(ColStage::Open) => return Err(format!("collection {index} ({symbol}) is open already")),
        Some(ColStage::Refused(why)) => return Err(format!("collection {index} ({symbol}) was refused ({why}): the operator must look at it")),
        _ => {}
    }
    pipeline_ready(&pin.kind, chains).map_err(|e| format!("collection {index} ({symbol}) cannot open yet: {e}"))?;
    if let Some(ahead) = s.requested.iter().position(|(i, _)| *i == index) {
        return Ok(Ask::Already { ahead });
    }
    // audit C4-02: the bounds, in the order a spammer meets them
    if s.requested.len() >= MAX_QUEUED {
        return Err(format!("{MAX_QUEUED} collections are queued to open already (a day of openings): ask again once one has opened"));
    }
    let hour: Vec<&(Vec<u8>, u64)> = s.request_log.iter().filter(|(_, at)| at + 3_600 > now).collect();
    if hour.len() >= REQUESTS_PER_HOUR {
        return Err(format!("{REQUESTS_PER_HOUR} opening requests were accepted this hour: ask again later"));
    }
    if caller == ANONYMOUS {
        if hour.iter().filter(|(c, _)| c == caller).count() >= ANON_REQUESTS_PER_HOUR {
            return Err(format!("{ANON_REQUESTS_PER_HOUR} anonymous opening requests were accepted this hour: ask again later"));
        }
    } else {
        if let Some((i, _)) = s.requesters.iter().find(|(_, c)| c.as_slice() == caller) {
            return Err(format!("you asked for collection {i} already: one pending request a caller"));
        }
        if hour.iter().filter(|(c, _)| c == caller).count() >= CALLER_REQUESTS_PER_HOUR {
            return Err(format!("{CALLER_REQUESTS_PER_HOUR} opening requests an hour a caller: ask again later"));
        }
    }
    Ok(Ask::New { ahead: s.requested.len() })
}

/// The answer's "when": next, or behind `ahead` requests at the pace of [`OPENS_PER_DAY`] a day (audit C4-02: the position against
/// the pace, as the float allows).
fn when(ahead: usize) -> String {
    if ahead == 0 {
        "next".to_string()
    } else {
        format!("after {ahead} other request(s): at most {OPENS_PER_DAY} open a day, as the float allows")
    }
}

/// A creator asks for a pinned collection to open (anyone may; idempotent; bounded by the pins and, audit C4-02, by
/// [`check_request`]'s limits). Refused for an index that is not pinned, already open or refused, or whose pipeline is not ready
/// (C4-03), and while the module is off (C4-11). The queue is FIFO: the opening pass takes the oldest request at its next look,
/// which this brings forward to the next tick. `caller`: the caller's principal bytes ([`ANONYMOUS`] for the anonymous principal).
pub fn request_open(index: u32, caller: &[u8], now: u64) -> Result<String, String> {
    request_open_with(index, caller, &crate::canister::evm_chain_ids(), now)
}

/// [`request_open`] with the install's `evm_chains` given (tests).
pub fn request_open_with(index: u32, caller: &[u8], chains: &[u64], now: u64) -> Result<String, String> {
    with_mut(|s| {
        let ask = check_request(s, index, caller, chains, now)?;
        let symbol = s.pins[&index].symbol.clone();
        let ahead = match ask {
            Ask::Already { ahead } => return Ok(format!("collection {index} ({symbol}) is requested already: {ahead} ahead of it in the queue")),
            Ask::New { ahead } => ahead,
        };
        s.request_log.retain(|(_, at)| at + 3_600 > now);
        s.request_log.push((caller.to_vec(), now));
        s.requested.push((index, now));
        s.requesters.insert(index, caller.to_vec());
        // brings the next look forward, never back (a later request must not delay an earlier one)
        let due = s.next.get("open").copied().unwrap_or(0).min(now);
        s.set_next("open", due);
        Ok(format!("collection {index} ({symbol}) requested: it opens {} (a wX mint, its metadata and a DBC config, ≈ {} SOL from the float)",
            when(ahead), unlock::OPEN_COST as f64 / 1e9))
    })
}

/// Audit C4-01: the operator's way out of `Refused`: the collection is pinned again under its own index and symbol (nothing about the
/// chain is assumed: the pass reads it afresh, and refuses again what is still wrong), its steps' waits are cleared, and it is queued
/// at the back as a request (outside the callers' limits).
pub fn retry_refused(index: u32, now: u64) -> Result<String, String> {
    with_mut(|s| {
        let Some(c) = s.cols.get_mut(&index) else { return Err(format!("collection {index} was never looked at: nothing to retry")) };
        let ColStage::Refused(why) = c.stage.clone() else { return Err(format!("collection {index} is not refused ({:?})", c.stage)) };
        c.stage = ColStage::Pinned;
        s.clear_wait(&format!("open_mint:{index}"));
        s.clear_wait(&format!("open_config:{index}"));
        s.stalls.remove(&index);
        if !s.requested.iter().any(|(i, _)| *i == index) {
            s.requested.push((index, now));
            s.requesters.insert(index, vec![]);
        }
        let due = s.next.get("open").copied().unwrap_or(0).min(now);
        s.set_next("open", due);
        Ok(format!("collection {index} is pinned again (it was refused: {why}) and queued to open"))
    })
}

/// The operator's switch: open the pinned collections in index order without a request (the float left over), or only on request.
pub fn set_auto_open(on: bool, now: u64) {
    with_mut(|s| {
        s.auto_open = on;
        let due = s.next.get("open").copied().unwrap_or(0).min(now);
        s.set_next("open", due);
    });
}

/// One pass: read the chain side of the next collection and make the next missing piece, or list it. Pays from `K_main`'s float.
pub async fn pass<I: DeskIo>(io: &mut I, now: u64) -> Opened {
    let chains = crate::canister::evm_chain_ids();
    let Some(index) = next_pinned(&chains, now) else { return Opened::Nothing };
    let (out, own) = pass_one(io, index, now).await;
    // audit C4-04, C4-12: what the pass waits on, per collection (the queue's view and the skip); anything else ends the wait
    with_mut(|s| match &out {
        Opened::Waits(why) => {
            let st = s.stalls.entry(index).or_insert(Stall { since: now, last_look: now, why: String::new(), own });
            if st.own != own {
                st.since = now;
            }
            st.last_look = now;
            st.why = why.clone();
            st.own = own;
        }
        _ => {
            s.stalls.remove(&index);
        }
    });
    out
}

/// The pass on one collection: what it did, and whether a wait is the collection's own (`true`) or shared by every request (the
/// float, the pace, the keys, the reads: `false`).
async fn pass_one<I: DeskIo>(io: &mut I, index: u32, now: u64) -> (Opened, bool) {
    let pin = with(|s| s.pins[&index].clone());
    let k = match keys_of(io, index).await {
        Ok(k) => k,
        Err(e) => return (Opened::Waits(format!("the keys of collection {index} are not derived: {e}")), false),
    };
    let mint = pin.wx_mint.unwrap_or_else(|| wx_mint_of(&k.main, index));
    let cfg = pin.config.unwrap_or(k.cfg_key);
    let accts = match io.sol_accounts(&[mint, solix2::metadata_address(&mint), cfg], 1_100).await {
        Ok(a) => a,
        Err(e) => return (Opened::Waits(format!("collection {index}: its chain side is not read: {e}")), false),
    };
    with_mut(|s| {
        let c = s.cols.entry(index).or_insert_with(|| ColRec { index, stage: ColStage::Pinned, wx_mint: mint, config: cfg, opened_at: 0, checked_at: 0, module: k.coll });
        c.checked_at = now;
        c.module = k.coll;
    });
    let refuse = |why: String| {
        with_mut(|s| {
            if let Some(c) = s.cols.get_mut(&index) {
                c.stage = ColStage::Refused(why.clone());
            }
        });
        (Opened::Waits(format!("collection {index} REFUSED: {why}")), true)
    };
    // the mint; audit C4-01: a pre-funded, data-less System account at the DERIVED address is not a mint yet, it is made over
    let held = match &accts[0] {
        None => Some(0),
        Some(m) if pin.wx_mint.is_none() && squatted(m) => Some(m.lamports),
        Some(_) => None,
    };
    if let Some(held) = held {
        if pin.wx_mint.is_some() {
            return (Opened::Waits(format!("collection {index}: the pinned wX mint {} does not exist yet (the ceremony makes it)", b58_encode(&mint))), true);
        }
        return step_open_mint(io, &k, mint, &pin, held, now).await;
    }
    let m = accts[0].as_ref().expect("read above");
    match Mint::decode(&m.owner, &m.data).and_then(|d| guard::check_wx_mint(&d, &k.coll).map(|_| d)) {
        Ok(d) => {
            let listed = desk::with(|s| s.books.collections.contains_key(&(index as u16)));
            if d.supply != 0 && !listed {
                return refuse(format!("the wX mint already has a supply of {} before the collection is listed: the books would not own it", d.supply));
            }
        }
        Err(e) => return refuse(e),
    }
    // the metadata
    match &accts[1] {
        Some(md) => {
            if let Err(e) = check_metadata(&md.owner, &md.data, &k.coll, &mint) {
                return refuse(e);
            }
        }
        None => return (Opened::Waits(format!("collection {index}: the wX metadata does not exist yet")), true),
    }
    // the config; audit C4-01: a pre-funded, data-less System account at the derived config address is not a config yet: DBC's
    // `create_config` initialises over it (Anchor's `init` tops up, allocates and assigns an account that holds lamports)
    let made = match &accts[2] {
        None => false,
        Some(c) if pin.config.is_none() && squatted(c) => false,
        Some(_) => true,
    };
    if !made {
        if pin.config.is_some() {
            return (Opened::Waits(format!("collection {index}: the pinned DBC config {} does not exist yet (the ceremony makes it)", b58_encode(&cfg))), true);
        }
        return step_open_config(io, &k, mint, &pin, now).await;
    }
    let c = accts[2].as_ref().expect("read above");
    let read = DbcConfig::decode(&c.owner, &c.data).and_then(|d| guard::check_config(&d, &config_pin(&k, mint, &pin)));
    if let Err(e) = read {
        return refuse(e);
    }
    // verified: list it in the books (idempotent) and open it
    let listed = desk::with_mut(|s| {
        if s.books.collections.contains_key(&(index as u16)) {
            return Ok(false);
        }
        s.books.add_collection(index as u16, pin.kind.clone(), mint, pin.params).map(|_| true)
    });
    match listed {
        Ok(_) => {}
        Err(BooksError::AlreadyCollection) => {}
        Err(e) => return refuse(format!("the books refused the collection: {e}")),
    }
    with_mut(|s| {
        if let Some(c) = s.cols.get_mut(&index) {
            c.stage = ColStage::Open;
            c.opened_at = now;
        }
        s.clear_wait(&format!("open_mint:{index}"));
        s.clear_wait(&format!("open_config:{index}"));
        s.landed("open", now, format!("collection {index} ({}) is open", pin.symbol));
    });
    (Opened::Open(index), false)
}

/// May a step that pays `rent` lamports be signed: the pace, and `K_main`'s float above the reserve.
async fn can_pay<I: DeskIo>(io: &mut I, rent: u64, now: u64) -> Result<(), String> {
    let (opened, signed) = with_mut(|s| {
        s.roll_day(now);
        (s.opened_today, s.signed_today)
    });
    if opened >= OPENS_PER_DAY {
        return Err(format!("{OPENS_PER_DAY} collections opened today already (the unlock pace)"));
    }
    if signed >= STEPS_PER_DAY {
        return Err(format!("{STEPS_PER_DAY} coin steps signed today already"));
    }
    let balance = io.sol_balance().await?;
    if balance < RESERVE_LAMPORTS + rent as u128 {
        return Err(format!("it costs {rent} lamports and K_main holds {balance} (the money flows keep {RESERVE_LAMPORTS}): the fees have not covered it yet"));
    }
    Ok(())
}

/// The mint step (`held`: what the derived address holds already, audit C4-01). Its waits: the refusal's retry is the collection's own
/// (C4-04), the rent read and the float are everyone's.
async fn step_open_mint<I: DeskIo>(io: &mut I, k: &Keys, mint: Pubkey, pin: &CollectionPin, held: u64, now: u64) -> (Opened, bool) {
    let key = format!("open_mint:{}", pin.index);
    if with(|s| s.waiting(&key, now)) {
        return (Opened::Waits(format!("collection {}: its mint step waits (a refusal, retried later)", pin.index)), true);
    }
    let rent = match io.sol_rent(spl::MINT_LEN).await {
        Ok(r) => r,
        Err(e) => return (Opened::Waits(format!("the mint's rent is not read: {e}")), false),
    };
    let topup = rent.saturating_sub(held);
    if let Err(e) = can_pay(io, topup + unlock::METADATA_RENT + unlock::METAPLEX_FEE + unlock::CONFIG_RENT + unlock::OPEN_TX_FEES, now).await {
        return (Opened::Waits(format!("collection {}: {e}", pin.index)), false);
    }
    let ixs = mint_tx(k, mint, rent, pin, held);
    sign(io, CoinStep::OpenMint { coll: pin.index }, ixs, vec![KeyPath::Coll(pin.index)], now).await
}

async fn step_open_config<I: DeskIo>(io: &mut I, k: &Keys, mint: Pubkey, pin: &CollectionPin, now: u64) -> (Opened, bool) {
    let key = format!("open_config:{}", pin.index);
    if with(|s| s.waiting(&key, now)) {
        return (Opened::Waits(format!("collection {}: its config step waits (a refusal, retried later)", pin.index)), true);
    }
    if let Err(e) = can_pay(io, unlock::CONFIG_RENT + unlock::OPEN_TX_FEES, now).await {
        return (Opened::Waits(format!("collection {}: {e}", pin.index)), false);
    }
    let ixs = config_tx(k, mint, pin);
    sign(io, CoinStep::OpenConfig { coll: pin.index }, ixs, vec![KeyPath::Config(pin.index)], now).await
}

/// Audit B3-10: a config step refused by simulation this many times (one a [`crate::coins::state::WAIT_RETRY_SECS`]) refuses its collection.
pub const CONFIG_REFUSALS: u32 = 3;

/// Signs an opening step on the crank line; a refusal by simulation is the step's to wait out (no line error, no alert), and the
/// collection's own wait (audit C4-04: the requests behind it move on after [`HEAD_PATIENCE`]).
async fn sign<I: DeskIo>(io: &mut I, step: CoinStep, ixs: Vec<Instruction>, keys: Vec<KeyPath>, now: u64) -> (Opened, bool) {
    let key = step.key();
    let label = step.label();
    let before = io.sol_line_of(crate::solhome::LineId::Crank).line.error.clone();
    match super::link::sign_plain(io, LineOwner::Coin, LineStep::Coin(step.clone()), ixs, keys, now).await {
        Ok(sig) => {
            with_mut(|s| {
                s.signed_today += 1;
                if let CoinStep::OpenMint { coll } | CoinStep::OpenConfig { coll } = &step {
                    if matches!(step, CoinStep::OpenMint { .. }) {
                        s.opened_today += 1;
                    }
                    if let Some(c) = s.cols.get_mut(coll) {
                        c.stage = ColStage::Opening;
                    }
                }
            });
            (Opened::Signed(sig), false)
        }
        Err(e) if e.contains(crate::solsend::SIMULATION_REFUSED) && !e.contains("insufficient_funds") => {
            let mut l = io.sol_line_of(crate::solhome::LineId::Crank);
            l.line.error = before;
            io.sol_line_save_of(crate::solhome::LineId::Crank, &l);
            let tries = with_mut(|s| {
                s.wait(key.clone(), e.clone(), now);
                s.waits.iter().find(|w| w.key == key).map(|w| w.tries).unwrap_or(1)
            });
            // audit B3-10: a config DBC refuses for good (a curve `register` could not see through) must not hold the unlock order:
            // after CONFIG_REFUSALS the collection is refused and the next index opens; the operator looks, and `coins_retry` (audit
            // C4-01) queues it again under its own index once the cause is gone
            if let CoinStep::OpenConfig { coll } = &step {
                if tries >= CONFIG_REFUSALS {
                    let why = format!("its config was refused by simulation {tries} times: {e}");
                    with_mut(|s| {
                        if let Some(c) = s.cols.get_mut(coll) {
                            c.stage = ColStage::Refused(why.clone());
                        }
                    });
                    return (Opened::Waits(format!("collection {coll} REFUSED: {why}")), true);
                }
            }
            (Opened::Waits(format!("{label} is refused by simulation, nothing signed: {e}")), true)
        }
        Err(e) => (Opened::Waits(format!("{label} waits: {e}")), false),
    }
}

/// Marks the kind of listing a pin has (for the readers: Ethereum, Robinhood Chain, Ordinals, Solana). An EthNft listing of chain 4663
/// is a Robinhood Chain collection (docs/v9/robinhood-everything.md §3), not an Ethereum one.
pub fn listing_label(k: &ListingKind) -> &'static str {
    match k {
        ListingKind::EthNft { chain_id: 4663, .. } => "robinhood",
        ListingKind::EthNft { .. } => "ethereum",
        ListingKind::Inscriptions { .. } => "inscriptions",
        ListingKind::SolNft { .. } => "solana",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_4663_listing_is_labelled_robinhood() {
        let eth = |chain_id| ListingKind::EthNft { chain_id, collection: [1; 20], custody: [2; 20] };
        assert_eq!(listing_label(&eth(1)), "ethereum");
        assert_eq!(listing_label(&eth(4663)), "robinhood");
        assert_eq!(listing_label(&ListingKind::Inscriptions { membership_root: [0; 32], custody: "bc1p".into() }), "inscriptions");
        assert_eq!(listing_label(&ListingKind::SolNft { collection: [3; 32], custodian: [4; 32] }), "solana");
    }
}
