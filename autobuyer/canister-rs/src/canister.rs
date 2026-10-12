//! The canister: the threshold keys, the two chains, and the loop that drives [`crate::state`].
//!
//! Everything that decides is in the pure modules; this one only fetches and signs. It holds no secret of its own — both keys
//! are threshold keys held by the subnet, and neither is ever assembled anywhere.
//!
//! Three guards, because a program that signs for Bitcoin must not be casual about which network it is on:
//!
//! 1. the key name comes from an init argument, never a default, and naming the production key requires the installer to say
//!    so in a second field. **The canister cannot tell a local replica from mainnet**: dfx 0.32's local replica offers one
//!    BIP340 key and it is *named* `key_1`, exactly as production is, so the name carries no information about the network
//!    (found while building the spike). What the second field buys is that the choice is deliberate, recorded in the install
//!    arguments for anyone to read, and impossible to reach by leaving a default alone. That is all it buys, and pretending
//!    otherwise would be worse than not having it;
//! 2. the EVM chain id is checked against the one the canister was installed for, on every read;
//! 3. nothing spends until [`crate::state::Purchase::next`] says so, and that decision is made from the chains, not memory.

use candid::CandidType;
use ic_cdk::management_canister::{
    ecdsa_public_key, http_request, schnorr_public_key, sign_with_ecdsa, sign_with_schnorr, Bip341, EcdsaCurve,
    EcdsaKeyId, EcdsaPublicKeyArgs, HttpHeader, HttpMethod, HttpRequestArgs, HttpRequestResult, SchnorrAlgorithm,
    SchnorrAux, SchnorrKeyId, SchnorrPublicKeyArgs, SignWithEcdsaArgs, SignWithSchnorrArgs, TransformArgs,
    TransformContext, TransformFunc,
};
use serde::Deserialize;
use std::cell::RefCell;

use crate::evmsend::{chain_id_of, classify, tx_view, SendOutcome};
use crate::psbt::{Psbt, Quote};
use crate::sighash;
use crate::state::Purchase;

/// What the canister is installed with. Every one of these is public; none is a secret.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Init {
    /// The EVM chain the vault lives on. Checked against the RPC on every read.
    pub chain_id: u64,
    /// The vault contract.
    pub vault: String,
    /// JSON-RPC endpoints. At least two, and they must agree before anything is believed.
    pub rpc_urls: Vec<String>,
    /// Bitcoin explorers used for the read-back on testnet. At least two, and they must agree.
    pub btc_api_urls: Vec<String>,
    /// The threshold key's name.
    pub key_name: String,
    /// The installer's assertion that naming [`PRODUCTION_KEY`] is deliberate. It does **not** mean "this is mainnet": the
    /// canister has no way to know that, because the local replica's only key carries the production key's name. It means
    /// only that nobody reached this by leaving a field unset.
    pub accept_production_key_name: bool,
    /// Bridge contracts we will sign a transaction to. Anything else is refused.
    pub bridge_allowlist: Vec<String>,
    /// Which Bitcoin network this install is for. An explicit parameter with no default, because the alternative is a
    /// boolean that is false until the day somebody flips it. See [`REHEARSAL_NETWORK`].
    pub btc_network: String,
    /// Who runs the buyer once its only controller is the timelock (`docs/v2/timelock-controller.md`). Controllers change the
    /// code; the operator operates. Taken from every upgrade's argument, so changing it is itself a timelocked proposal whose
    /// argument is published during the delay. `None`: only controllers operate, as before the handover.
    pub operator: Option<candid::Principal>,
    /// The wrapper whose redeemed frogs this canister delivers (docs/v3/wrapper.md, "Step 3, the flow"). Identity, like the
    /// vault: an upgrade may set it once, from nothing, and never change it. `None`: no custody, no deliveries.
    pub wrapper: Option<String>,
    /// Other EVM chains the buyer may buy an NFT on through Seaport (docs/v4/eth-nft-canister.md §4). Operational, like
    /// `rpc_urls`: an upgrade takes it from its argument. `None` or empty: no NFT buying.
    pub evm_chains: Option<Vec<crate::chains::EvmChain>>,
    /// How much custody may hold (docs/v5/custody-hardening.md, H2). Taken from every upgrade's argument, but only through
    /// `custodycap::validate_change` against the running policy: a raise is at most ×4 per proposal. `None` is launch stage 0
    /// (docs/v5/launch-gates.md, G7): the wrapper's inscriptions at 3 items and 10 ETH, each `eth_nft` collection at 6 items and
    /// 15 ETH (J7, 2026-09-27), and in all 6 items and 10 ETH or the collections' sum when more (no binding total); no other
    /// collection (an unlisted collection is refused). An install may name an Ethereum collection at up to 24 items and 15 ETH.
    pub custody_caps: Option<crate::custodycap::CapPolicy>,
    /// Outbound limits and the tripwire (H3). `None` is stage 0 (`outflow::Limits::stage0`). Every collection's cap must be
    /// able to leave within the timelock's delay less a day at these limits (`outflow::exit_fits`), or the install is refused.
    pub outflow_limits: Option<crate::outflow::Limits>,
    /// Holder deposits (audit D, H-2 and H-3). `None` or `false`: the deposits pass does not run at all (no classification, no
    /// reads, no returns). Operational: taken from every upgrade's argument, so switching it on is a published, timelocked act.
    /// Phase 1 leaves it `None`.
    pub holder_deposits: Option<bool>,
    /// The first 4663 block the wrapper's `Redeemed`/`Resolved` events are read from when the scanner first runs and is not idle
    /// (audit D, H-1): the wrapper's deployment block. `None`: the finalized block less `ticketlog::FIRST_LOOKBACK_BLOCKS`.
    /// Read only until the scanner has a cursor.
    pub wrapper_from_block: Option<u64>,
    /// How a floor sink's frog is paid for (docs/v6/relay-conversion.md §3.5). `None` is Relay at 3 % with no operator float:
    /// the module advances the frog's budget, Relay converts it to BTC at the float, and the float buys the frog. Operational:
    /// taken from every upgrade's argument, so a change is a published, timelocked act. `max_fee_bps` must lie in [50, 500].
    pub frog_funding: Option<crate::relayconv::FrogFunding>,
    /// Phase 1 on Ethereum NFTs (docs/v7/phase1-nft.md §4.1): the collections, their wrappers and modules, the Relay bound, the
    /// deposits switch and the floor's parameters. Operational: taken from every upgrade's argument, so a change is a published,
    /// timelocked act. `None`: the Ethereum NFT side is inert.
    pub eth_nft: Option<crate::ethnft::EthNftConfig>,
    /// The home chain when it is not the EVM chain above (docs/v9/solana-migration.md §4). `None`: the EVM home (`chain_id`,
    /// `vault`, `rpc_urls`), as before. Phase A carries it beside the EVM fields and nothing reads it yet but the Solana layer
    /// (`sol*.rs`). Identity, like the wrapper: set once, from nothing; an upgrade may change only its `rpc_urls` and
    /// `blockhash_fallback` ([`restore_home`]).
    pub home: Option<Home>,
}

/// A non-EVM home chain.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Home {
    Solana {
        /// [`SOLANA_CLUSTERS`], spelled exactly.
        cluster: String,
        /// Solana JSON-RPC endpoints: at least two, distinct; every read is agreed at `finalized` (`solrpc::agreed`).
        rpc_urls: Vec<String>,
        /// `"books"`: the canister keeps the books itself (decision D-l, docs/v9/canister-only-solana.md §0: Rook deploys no Solana
        /// programs). The only value an install accepts; a program id is refused.
        rook_core: String,
        /// The seed of the home line's durable-nonce account: `create_with_seed(sol_address, seed, System Program)`.
        nonce_seed: String,
        /// `Some(true)`: sign on a recent blockhash instead of the durable nonce (`solsend::RecentBlockhash`, the fallback for the
        /// day durable nonces go). `None`/`false`: the durable nonce.
        blockhash_fallback: Option<bool>,
        /// Retired with rook_backing (decision D-l): an install names none (`null`); kept so an earlier argument still decodes.
        #[serde(default)]
        rook_backing: Option<String>,
        /// Decision D-i (2026-10-10): the drand HTTP relays the cranks ask for a redemption round's randomness (two must give the same
        /// bytes, none other ones: `drand::judge`). `None`: `drand::DEFAULT_RELAYS`. Operational, as `rpc_urls` is: an upgrade may
        /// change it.
        #[serde(default)]
        drand_urls: Option<Vec<String>>,
    },
}

/// Solana clusters an install may name: an allowlist, as [`KNOWN_NETWORKS`] is for Bitcoin.
pub const SOLANA_CLUSTERS: [&str; 3] = ["devnet", "testnet", "mainnet-beta"];

/// The Solana home's checks (pure): a known cluster, two distinct providers at least, the books home (D-l: no program), a usable nonce
/// seed.
pub fn validate_home(h: &Home) -> Result<(), String> {
    match h {
        Home::Solana { cluster, rpc_urls, rook_core, nonce_seed, rook_backing, drand_urls, .. } => {
            if let Some(d) = drand_urls {
                crate::drand::validate_relays(d)?;
            }
            if !SOLANA_CLUSTERS.contains(&cluster.as_str()) {
                return Err(format!("refused: home cluster {cluster:?}; this canister installs for {SOLANA_CLUSTERS:?}, spelled exactly so"));
            }
            if rpc_urls.len() < 2 || !distinct(rpc_urls) {
                return Err("refused: the Solana home needs two distinct RPC providers at least: one is not a check, and the same URL \
                    twice agrees with itself".into());
            }
            if rook_core != crate::solhome::SolHomeCfg::BOOKS {
                return Err(format!("refused: home rook_core {rook_core:?}: Rook deploys no Solana programs (decision D-l, \
                    docs/v9/canister-only-solana.md §0): the canister keeps the books; name \"books\""));
            }
            if rook_backing.is_some() {
                return Err("refused: home rook_backing: Rook deploys no Solana programs (decision D-l): rook_backing is retired; leave it null".into());
            }
            if nonce_seed.is_empty() || nonce_seed.len() > crate::soltx::MAX_SEED_LEN || !nonce_seed.bytes().all(|b| b.is_ascii_graphic()) {
                return Err("refused: home nonce_seed must be 1-32 printable ASCII bytes".into());
            }
            Ok(())
        }
    }
}

/// The frog wrapper as the install names it (`Init.wrapper`): an EVM address on the EVM home, or (phase B2) a Solana address in base58
/// on the Solana home (unused on the books home: the Inscriptions listing in the books is the frog collection). Returns the record key
/// the flows use (the address, or the Solana key's first 20 bytes) and the Solana address when it is one.
pub fn wrapper_key(w: &str) -> Result<([u8; 20], Option<crate::soltx::Pubkey>), String> {
    let w = w.trim();
    if w.starts_with("0x") {
        return Ok((crate::evmtx::parse_address(w)?, None));
    }
    let k = crate::soltx::pubkey(w).map_err(|e| format!("the wrapper {w:?} is neither an EVM address nor a Solana address: {e}"))?;
    Ok((k[..20].try_into().expect("20"), Some(k)))
}

/// The frog wrapper against the home (phase B2): an EVM address on the EVM home, a Solana address on the Solana home; holder deposits
/// (the EWD2 claim) are OFF on the Solana home at launch (docs/v9/solana-migration.md §2.7, Q7), as for Ethereum NFTs in B1.
pub fn validate_frog_home(wrapper: Option<&str>, holder_deposits: bool, solana: bool) -> Result<(), String> {
    if let Some(w) = wrapper {
        let (_, sol) = wrapper_key(w)?;
        if sol.is_some() != solana {
            return Err(format!("refused: the wrapper {w} is {} but the home is {}", if sol.is_some() { "a Solana address" } else { "an EVM address" },
                if solana { "Solana" } else { "the EVM chain" }));
        }
    }
    if solana && holder_deposits {
        return Err("refused: holder_deposits: holder deposits are OFF on the Solana home at launch (docs/v9/solana-migration.md §2.7; their \
            claim format changes to EWD2 with the first post-launch release, Q7)".into());
    }
    Ok(())
}

/// On the Solana home (the books home, `rook_core = "books"`: docs/v9/canister-only-solana.md O-11) there is no wake desk: a configured
/// `wake_desk` is refused; a dormant collection wakes by the operator's `eth_wake` only.
pub fn solana_desk_ok(d: &crate::dormancy::Dormancy) -> Result<(), String> {
    let Some(w) = &d.wake_desk else { return Ok(()) };
    Err(format!("eth_nft.dormancy.wake_desk {w}: the books home has no wake desk (canister-only-solana.md O-11: dropped for launch; a \
        dormant collection wakes by eth_wake); unset it"))
}

/// An upgrade's home: the saved one is identity (cluster, program, nonce seed), set once from nothing; its providers and the
/// lifetime switch are operational and come from the argument. An argument cannot remove a home.
pub fn restore_home(saved: Option<Home>, arg: Option<&Home>) -> Option<Home> {
    match (saved, arg) {
        (None, a) => a.cloned(),
        (Some(s), None) => Some(s),
        (Some(Home::Solana { cluster, rook_core, nonce_seed, rook_backing, .. }),
            Some(Home::Solana { rpc_urls, blockhash_fallback, rook_backing: arg_backing, drand_urls, .. })) => {
            // rook_backing is identity once set; an install from before it takes the argument's (set once, from nothing)
            let rook_backing = rook_backing.or_else(|| arg_backing.clone());
            Some(Home::Solana { cluster, rpc_urls: rpc_urls.clone(), rook_core, nonce_seed, blockhash_fallback: *blockhash_fallback, rook_backing,
                drand_urls: drand_urls.clone() })
        }
    }
}

thread_local! {
    /// What custody holds and is acquiring, as the caps count it (H2), and what left it in the last day (H3). Saved as the
    /// upgrade tuple's ninth element.
    pub(crate) static CUSTODY_BOOK: RefCell<crate::custodycap::Book> = RefCell::new(Default::default());
    pub(crate) static OUTFLOW: RefCell<crate::outflow::Outflow> = RefCell::new(Default::default());
    static INIT: RefCell<Option<Init>> = const { RefCell::new(None) };
    static PURCHASES: RefCell<Vec<Purchase>> = const { RefCell::new(Vec::new()) };
    /// Set by the owner's veto key. It stops the buyer acting; it can never move funds.
    static PAUSED: RefCell<bool> = const { RefCell::new(false) };
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Held for the length of one `tick`. See [`TickGuard`].
    static TICKING: RefCell<bool> = const { RefCell::new(false) };
    /// Review round 4 (C4-H-1): the highest pin and nonce read so far on the home chain (every 4663 read of every part of the
    /// canister goes through [`pin`]) and on the Ethereum NFT side (`ethnet::EthNet::pin`): no read goes back. Saved in the
    /// thirteenth element (`ethflow::EthState::pin_floors`), written there at `pre_upgrade`.
    pub(crate) static PIN_FLOORS: RefCell<crate::pin::PinFloors> = RefCell::new(Default::default());
}

/// The floors now (for the saved state and the state query).
pub(crate) fn pin_floors() -> crate::pin::PinFloors {
    PIN_FLOORS.with(|f| *f.borrow())
}

/// Every update method is for the operator, and for nobody else.
///
/// Before this there was no caller check anywhere in the canister, and the candid interface was the access-control policy:
/// every method in it was callable by any principal on the Internet Computer. Two of them sign. `rehearse_sign_sighash`
/// returns a BIP340 signature over any 32 bytes, and a BIP341 key-path sighash is 32 bytes; `rehearse_sign_evm` returns an
/// ECDSA signature over any 32 bytes, and an EVM transaction's signing hash is 32 bytes. So anyone could build a
/// transaction spending this canister's coins to themselves, hand over its digest, and be given the signature. The three
/// guards those methods already carried are about WHICH NETWORK the key derives for, not about who is asking — a distinction
/// that is easy to lose when both are called "guards". `set_paused`, whose own doc comment calls it "the owner's veto key",
/// could be toggled by anybody, which means an emergency pause could be lifted by the party it was pausing.
///
/// The design's claim is a key nobody holds. Without this it was a key anybody could use.
fn only_controller() {
    let caller = ic_cdk::api::msg_caller();
    if !ic_cdk::api::is_controller(&caller) {
        ic_cdk::trap(&format!("refused: {caller} is not a controller of this canister."));
    }
}

/// Operations: the operator named in the install argument, or a controller. After the handover the controller is the
/// timelock, which forwards nothing but the guardian's pause, so in practice this is the operator. What the operator can do
/// is bounded by this code; changing this code is a controller's act, and the controller is behind a public delay.
pub(crate) fn only_operator() {
    let caller = ic_cdk::api::msg_caller();
    let operator = INIT.with(|i| i.borrow().as_ref().and_then(|x| x.operator));
    if !may_operate(ic_cdk::api::is_controller(&caller), caller, operator) {
        ic_cdk::trap(&format!("refused: {caller} is neither the operator nor a controller of this canister."));
    }
}

fn may_operate(is_controller: bool, caller: candid::Principal, operator: Option<candid::Principal>) -> bool {
    is_controller || operator == Some(caller)
}

/// L-3 (audit D): ingress the canister would only refuse is not accepted at all, so a stranger cannot make it pay for update
/// calls that trap on `only_operator`. Controllers and the operator may call anything; anyone else only `submit_offer`, and
/// only while an offer board is configured (OFF in Phase 1), and never anonymously. This runs on one replica and is a cycles
/// guard, not the access control: every method keeps its own guard. Queries and inter-canister calls (the timelock's) never
/// pass through here.
#[ic_cdk::inspect_message]
fn inspect_message() {
    let caller = ic_cdk::api::msg_caller();
    let operator = INIT.with(|i| i.borrow().as_ref().and_then(|x| x.operator));
    let boards = crate::handinnet::HANDINS.with(|h| h.borrow().has_boards());
    let method = ic_cdk::api::msg_method_name();
    let is_controller = ic_cdk::api::is_controller(&caller);
    if !may_ingress(&method, is_controller, caller, operator, boards) {
        return;
    }
    // audit C4-05: a stranger's opening request is accepted only when it would queue the collection (not a no-op, not over the
    // queue's limits, C4-02), so a call that changes nothing costs the canister nothing
    if method == "coins_request_open" && !may_operate(is_controller, caller, operator)
        && !crate::coins::api::ingress_request(&ic_cdk::api::msg_arg_data(), caller.as_slice()) {
        return;
    }
    ic_cdk::api::accept_message();
}

/// Whether the buyer is paused (the cranks read it).
pub fn is_paused() -> bool {
    PAUSED.with(|p| *p.borrow())
}

fn may_ingress(method: &str, is_controller: bool, caller: candid::Principal, operator: Option<candid::Principal>, boards: bool) -> bool {
    may_operate(is_controller, caller, operator) || (method == "submit_offer" && boards && caller != candid::Principal::anonymous())
        || crate::desk::is_public(method) || crate::coins::api::is_public(method)
}

/// Refuses while the buyer is paused (docs/v5/custody-hardening.md, H1). The pause used to be read only inside `tick`, so a
/// stolen operator key could still sign a purchase or a Seaport fill after a guardian had paused the buyer. Called right after
/// `only_operator()` in every operator method that signs, sends, records a quote, stages a spend or changes what custody
/// holds; `every_operator_method_that_acts_refuses_while_paused` reads the source to keep it that way.
pub(crate) fn not_paused() -> Result<(), String> {
    if PAUSED.with(|p| *p.borrow()) {
        return Err("refused: the buyer is paused; k guardians resume it through the timelock".into());
    }
    Ok(())
}

/// The production Schnorr/ECDSA key. Naming it by accident on a local replica is the mistake this constant exists to catch.
const PRODUCTION_KEY: &str = "key_1";

/// Bitcoin mainnet, spelled exactly as every mainnet-only path matches it (`ordnet::providers`, `oracle::delivered`,
/// `btcsend::checkpoint`, `policy::hrp_for`). One constant, so that the install check and those paths cannot drift apart.
pub(crate) const BITCOIN_MAINNET: &str = "mainnet";

/// Bitcoin networks this canister will accept as an install argument: an allowlist, so a typo, an alias (`"bitcoin"`) or an empty
/// string is refused at install. Mainnet joined on 2026-09-25 (Phase-1 K2, launch gate C7) with its checkpoint in
/// `btcsend::checkpoint`, verified as `docs/deploy/canister-mainnet.md` ("The published C7 check") records. Every rehearsal
/// method stays dead on it: they allow [`REHEARSAL_NETWORK`] (and [`REHEARSAL_EVM_CHAIN`]) only.
const KNOWN_NETWORKS: [&str; 3] = ["signet", "testnet4", BITCOIN_MAINNET];

/// The one network [`rehearse_sign_sighash`] will sign for. An allowlist of exactly one, rather than "anything but mainnet":
/// a typo, an empty string and a network invented next year all refuse, where a denylist would have waved them through.
pub(crate) const REHEARSAL_NETWORK: &str = "signet";

/// The one EVM chain [`rehearse_sign_evm`] will sign for: Robinhood Chain **testnet**, which answers `eth_chainId` with
/// `0xb626`. An allowlist of exactly one, for the reason [`REHEARSAL_NETWORK`] is one: "anything but mainnet" accepts a
/// typo, and on this side a typo is a signature over a transaction for the chain the money is on.
const REHEARSAL_EVM_CHAIN: u64 = 46630;

/// Robinhood Chain **mainnet**, where `AssetVault` and the money are. Named here only so that a refusal can say what it
/// refused and why; nothing in this file ever treats it as a permitted value.
const PRODUCTION_EVM_CHAIN: u64 = 4663;

pub(crate) fn note(s: impl Into<String>) {
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        l.push(s.into());
        if l.len() > 500 {
            l.remove(0);
        }
    });
}

/// True when no entry repeats. Used on the provider lists, where a repeat would turn a consensus check into a tautology.
fn distinct(urls: &[String]) -> bool {
    urls.iter().enumerate().all(|(i, u)| !urls[..i].contains(u))
}

#[ic_cdk::init]
fn init(arg: Init) {
    validate(&arg);
    // stage 0 from nothing (docs/v5/launch-gates.md, G7): an install is a raise from no caps at all
    if let Err(e) = check_cap_change(None, &arg) {
        panic!("{e}");
    }
    INIT.with(|i| *i.borrow_mut() = Some(arg));
    note("installed");
    // W14b: the canister's own Solana timer (timers do not survive an install or an upgrade)
    crate::soltimer::arm_after_install();
}

/// Everything an install argument has to satisfy, whether it arrives at install or at upgrade.
/// What an upgrade keeps and what it takes from its argument. The buyer's identity -- the chain, the Bitcoin network, the
/// key, the vault, the bridge allowlist -- comes back exactly as saved: an upgrade is not a way to point it at a different
/// chain or key. Its endpoints and its operator follow the upgrade argument, which the timelock publishes in full for the
/// length of the delay, so changing an RPC or an explorer is as public as changing code. Found by the Bitcoin broadcast
/// rehearsal: a canister installed with a non-esplora explorer (UniSat's API) could never agree with mempool.space on a
/// read-back, and no upgrade could correct it.
fn restore_init(saved: Option<Init>, arg: &Init) -> Init {
    let saved_wrapper = saved.as_ref().and_then(|s| s.wrapper.clone());
    let saved_home = saved.as_ref().and_then(|s| s.home.clone());
    let mut kept = saved.unwrap_or_else(|| arg.clone());
    // the wrapper is identity: set once, from nothing, never changed
    kept.wrapper = saved_wrapper.or_else(|| arg.wrapper.clone());
    kept.operator = arg.operator;
    kept.rpc_urls = arg.rpc_urls.clone();
    kept.btc_api_urls = arg.btc_api_urls.clone();
    kept.evm_chains = arg.evm_chains.clone();
    // post_upgrade checks custodycap::validate_change(running, this) before it is kept
    kept.custody_caps = arg.custody_caps.clone();
    kept.outflow_limits = arg.outflow_limits.clone();
    kept.holder_deposits = arg.holder_deposits;
    kept.wrapper_from_block = arg.wrapper_from_block;
    kept.frog_funding = arg.frog_funding;
    kept.eth_nft = arg.eth_nft.clone();
    kept.home = restore_home(saved_home, arg.home.as_ref());
    kept
}

/// How frogs for floor sinks are funded (docs/v6/relay-conversion.md §3.5): the argument's, or Relay at 3 % with no float.
pub(crate) fn frog_funding() -> crate::relayconv::FrogFunding {
    INIT.with(|i| i.borrow().as_ref().and_then(|x| x.frog_funding)).unwrap_or_default()
}

/// Whether holder deposits are switched on (audit D, H-2): off unless the install argument says so.
pub(crate) fn deposits_on() -> bool {
    INIT.with(|i| i.borrow().as_ref().and_then(|x| x.holder_deposits)).unwrap_or(false)
}

/// The timelock's floor (`timelock-rs/src/rules.rs`): the least time holders have to leave before an upgrade lands.
const ASSUMED_DELAY_SECS: u64 = 72 * 3600;

/// The caps in force: the argument's, or stage 0 for the wrapper's inscriptions (G7).
pub(crate) fn caps_of(i: &Init) -> crate::custodycap::CapPolicy {
    i.custody_caps.clone().unwrap_or_else(|| {
        // phase B2: a Solana wrapper's inscriptions are keyed `sol:<wrapper>` (a Bitcoin wrapper's `btc:0x<wrapper>`)
        let mut listed: Vec<String> = i.wrapper.iter().filter_map(|w| wrapper_key(w).ok())
            .map(|(w, sol)| sol.map_or_else(|| crate::custodycap::btc_key(&w), |k| crate::custodycap::sol_key(&k))).collect();
        // Ordinals multi-collection (docs/v9/ordinals-collections.md §4): each Inscriptions listing of the books home at stage 0
        // (`sol:<wX mint>`), the global re-sized below by the exit rule. Audit C2-08: every listing in the books, live or asleep (default-
        // live: all of them live from install), so the operator's frog_wake / frog_sleep never change the caps
        if i.home.is_some() {
            for k in crate::frognet::listing_cap_keys() {
                if !listed.contains(&k) {
                    listed.push(k);
                }
            }
        }
        // docs/v7/phase1-nft.md §4.1: each Ethereum collection at stage 0 beside the wrapper's inscriptions
        if let Some(e) = &i.eth_nft {
            // 2026-10-12: each keyed by its own chain (a Robinhood Chain collection: `4663:0x…`, as `custodycap::evm_key` writes it: audit C3-13)
            listed.extend(e.cols().unwrap_or_default().iter().map(|c| crate::custodycap::evm_key(e.chain_of_addr(&c.collection).unwrap_or(e.chain_id),
                &c.collection)));
        }
        // audit C1-02: each Solana NFT listing of the books home at stage 0 (`sol:<collection>`), in the global caps and the exit rule
        if i.home.is_some() {
            for k in crate::solnftflow::live_cap_keys() {
                if !listed.contains(&k) {
                    listed.push(k);
                }
            }
        }
        let mut p = crate::custodycap::stage0(&listed);
        // audit C3-08: a collection on another chain than the config's own (Robinhood Chain's, deployments/launch/robinhood-collections.json)
        // starts at the launch file's bound, never at J7's Ethereum stage 0 (6 items, 15 ETH): FIRST_MAX_ITEMS items, each at most its own
        // max_purchase_wei, so the collection at most FIRST_MAX_ITEMS × max_purchase_wei (StonkBrokers 3 items, 5.52 ETH)
        if let Some(e) = &i.eth_nft {
            for c in e.cols().unwrap_or_default().iter().filter(|c| c.chain_id != e.chain_id) {
                let key = crate::custodycap::evm_key(c.chain_id, &c.collection);
                if let Some(x) = p.collections.iter_mut().find(|x| x.collection == key) {
                    x.max_items = crate::custodycap::FIRST_MAX_ITEMS;
                    x.max_item_wei = c.max_purchase_wei;
                    x.max_value_wei = c.max_purchase_wei.saturating_mul(crate::custodycap::FIRST_MAX_ITEMS as u128);
                }
            }
        }
        // review round 4 (A4-L-1): the DEFAULT install's total holds no more items than its outflow limits let out within the delay
        // (stage 0's 6 a day: 12), so the whole default custody can leave; an install that names its caps states its own total and
        // is held to the same rule (`validate_custody`)
        let room = exit_room(&limits_of(i));
        let largest = p.collections.iter().map(|c| c.max_items).max().unwrap_or(0);
        p.global_max_items = p.global_max_items.min(room.max(largest));
        p
    })
}

/// Round 4 (A4-L-1): the most items the outflow limits let out of custody in all within the timelock's delay less a day.
pub(crate) fn exit_room(l: &crate::outflow::Limits) -> u32 {
    let days = (ASSUMED_DELAY_SECS / crate::outflow::WINDOW_SECS).saturating_sub(1);
    (l.global_items as u64).saturating_mul(days).min(u32::MAX as u64) as u32
}

/// The outbound limits in force: the argument's, or stage 0 (G7).
pub(crate) fn limits_of(i: &Init) -> crate::outflow::Limits {
    i.outflow_limits.clone().unwrap_or_else(crate::outflow::Limits::stage0)
}

/// H2 and H3 at install and upgrade: the caps are well-formed, the limits too, and every collection's whole cap can leave
/// within the timelock's delay less a day, so a rate limit never traps holders behind an upgrade they can see coming.
fn validate_custody(i: &Init) -> Result<(), String> {
    let caps = caps_of(i);
    crate::custodycap::validate(&caps).map_err(|e| format!("refused: custody_caps: {e}"))?;
    let l = limits_of(i);
    crate::outflow::validate(&l).map_err(|e| format!("refused: outflow_limits: {e}"))?;
    for c in &caps.collections {
        if !crate::outflow::exit_fits(c.max_items, l.per_collection_items, ASSUMED_DELAY_SECS) {
            return Err(format!("refused: {} holds up to {} items and lets {} out a day: holders could not leave within the \
                timelock's delay", c.collection, c.max_items, l.per_collection_items));
        }
    }
    // review round 4 (A4-L-1): the whole custody too. A delivery counts against the global item limit, so holders of every
    // collection leaving at once (a bank run against a pending upgrade) need the total the caps can hold to fit the global limit
    // within the delay less a day; the per-collection check alone let 61 items stand against 48 that could leave
    let total = caps.collections.iter().fold(0u64, |a, c| a.saturating_add(c.max_items as u64)).min(caps.global_max_items as u64);
    let total = total.min(u32::MAX as u64) as u32;
    if !crate::outflow::exit_fits(total, l.global_items, ASSUMED_DELAY_SECS) {
        return Err(format!("refused: custody holds up to {total} items in all and lets {} out a day in all: {} can leave within the \
            timelock's delay less a day, so holders could not all leave (lower the collections' item caps or the global cap to {}, or \
            raise global_items to {} a day)", l.global_items, exit_room(&l), exit_room(&l),
            (total as u64).div_ceil((ASSUMED_DELAY_SECS / crate::outflow::WINDOW_SECS).saturating_sub(1).max(1))));
    }
    Ok(())
}

/// Review round 5 (A5-L-1): the exit rule on what custody HOLDS as well as on what the caps name: each collection's items held (or
/// its cap, the larger) and all of them must leave within the timelock's delay less a day at the limits in force. The caps alone let
/// one upgrade lower the caps and the outflow together while custody still held more than the new limits could let out before the next
/// proposal ran. Lowering the caps alone stays allowed (what is held still fits the unchanged outflow).
pub(crate) fn held_fits(caps: &crate::custodycap::CapPolicy, book: &crate::custodycap::Book, l: &crate::outflow::Limits) -> Result<(), String> {
    let held = |c: &str| book.entries.iter().filter(|e| e.collection == c).count().min(u32::MAX as usize) as u32;
    for c in &caps.collections {
        let n = c.max_items.max(held(&c.collection));
        if !crate::outflow::exit_fits(n, l.per_collection_items, ASSUMED_DELAY_SECS) {
            return Err(format!("refused: {} holds {} items (its cap {}) and would let {} out a day: they could not leave within the \
                timelock's delay less a day (A5-L-1)", c.collection, held(&c.collection), c.max_items, l.per_collection_items));
        }
    }
    let named = caps.collections.iter().fold(0u64, |a, c| a.saturating_add(c.max_items as u64)).min(caps.global_max_items as u64);
    let total = named.max(book.entries.len() as u64).min(u32::MAX as u64) as u32;
    if !crate::outflow::exit_fits(total, l.global_items, ASSUMED_DELAY_SECS) {
        return Err(format!("refused: custody holds {} items in all (its caps {named}) and would let {} out a day in all: they could not \
            leave within the timelock's delay less a day (A5-L-1)", book.entries.len(), l.global_items));
    }
    Ok(())
}

fn validate(arg: &Init) {
    if let Err(e) = validate_custody(arg) {
        panic!("{e}");
    }
    if let Some(w) = &arg.wrapper {
        assert!(wrapper_key(w).is_ok(), "the wrapper is not an address");
    }
    // phase B2: the frog wrapper names the home's chain; holder deposits stay off on Solana
    if let Err(e) = validate_frog_home(arg.wrapper.as_deref(), arg.holder_deposits == Some(true), arg.home.is_some()) {
        panic!("{e}");
    }
    if let Err(e) = arg.frog_funding.unwrap_or_default().validate() {
        panic!("refused: {e}");
    }
    // phase B2: on the Solana home no EVM chain is the home (`chains::evm_home`): every EVM chain may be listed, Ethereum included
    let evm_home = crate::chains::evm_home(arg.chain_id, arg.home.is_some());
    if let Err(e) = crate::chains::validate(arg.evm_chains.as_deref().unwrap_or_default(), evm_home) {
        panic!("refused: evm_chains: {e}");
    }
    if let Some(e) = &arg.eth_nft {
        if let Err(e) = crate::ethnft::validate(e, arg.evm_chains.as_deref().unwrap_or_default(), evm_home) {
            panic!("refused: {e}");
        }
    }
    assert!(arg.rpc_urls.len() >= 2, "two RPC providers at least: one is not a check");
    assert!(arg.btc_api_urls.len() >= 2, "two Bitcoin sources at least, for the same reason");
    // …and they must be two DIFFERENT providers. A count is not a check: `rpc()` demands that every provider return an
    // identical body, which the same URL listed twice satisfies trivially and always. Found while installing for the EVM
    // rehearsal, where chain 46630 has one obvious public RPC and listing it twice was the path of least resistance.
    assert!(distinct(&arg.rpc_urls), "the RPC providers must be distinct: the same URL twice agrees with itself");
    assert!(distinct(&arg.btc_api_urls), "the Bitcoin sources must be distinct, for the same reason");
    assert!(
        arg.key_name != PRODUCTION_KEY || arg.accept_production_key_name,
        "refused: {PRODUCTION_KEY} is the production threshold key's name. Install with accept_production_key_name = true \
         to say that is deliberate. (The local replica's only key has the same name, so this flag records intent; it cannot \
         tell you which network you are on.)"
    );
    if let Some(h) = &arg.home {
        if let Err(e) = validate_home(h) {
            panic!("{e}");
        }
    }
    // docs/v9/solana-migration.md §4: the Ethereum NFT side's collections name their wrappers and modules on the home's chain
    if let Some(e) = &arg.eth_nft {
        if let Err(e) = crate::ethnft::validate_home(e, arg.home.is_some()) {
            panic!("refused: {e}");
        }
        if let (Some(Home::Solana { .. }), Some(d)) = (&arg.home, &e.dormancy) {
            if let Err(e) = solana_desk_ok(d) {
                panic!("refused: {e}");
            }
        }
    }
    assert!(arg.chain_id != 0, "a chain id is required");
    assert!(
        arg.operator != Some(candid::Principal::anonymous()),
        "refused: the anonymous principal as operator is an operator anyone can be"
    );
    assert!(
        KNOWN_NETWORKS.contains(&arg.btc_network.as_str()),
        "refused: btc_network = {:?}. This canister installs for {:?} and nothing else, spelled exactly so. Adding a network \
         is a code change and a review (with its checkpoint), not an argument.",
        arg.btc_network,
        KNOWN_NETWORKS
    );
}

/// Everything the buyer knows, written to stable memory before the wasm is replaced.
///
/// `docs/v2/state-machine.md` rests on the sentence "a canister can stop at any point -- an upgrade, a subnet hiccup, a
/// trap, running out of cycles -- and resume with only what it wrote to stable memory", and its interruption table answers
/// "the canister dies here; what does it do on restart?" for every transition. None of that was true: `PURCHASES`, `PAUSED`
/// and `LOG` lived in `thread_local!`, which is heap; there was no `pre_upgrade`; and `post_upgrade` called `init`, so an
/// upgrade reset the buyer to nothing. Measured on a local replica before this was written: a paused canister with a
/// two-line log came back from `dfx deploy --mode upgrade` unpaused, with the log gone.
///
/// The sharpest case is the pause. An operator who hits the veto because something is wrong, and then ships the fix, was
/// restarting the buyer by deploying it -- at the one moment they were most certain it was stopped.
///
/// The `Signed` phase is why this is about money and not tidiness. It holds the raw bytes of a signed but unbroadcast
/// Bitcoin transaction, and the no-double-spend argument is "broadcast the SAVED bytes: same inputs, same txid". Bytes that
/// do not survive a restart cannot be rebroadcast, and a canister that re-requests a PSBT instead may be handed a different
/// transaction spending the same UTXOs -- two transactions, one of which may already be in a mempool.
#[ic_cdk::pre_upgrade]
fn pre_upgrade() {
    // L-2 (audit D): an upgrade that lands while a tick is awaiting outcalls drops their continuations (a cap reservation, a
    // return's bytes). The timelock stops the buyer first; this refuses the upgrade if anything upgrades it mid-tick anyway.
    if TICKING.with(|t| *t.borrow()) {
        ic_cdk::trap("refused: a tick is running; stop the canister before upgrading it");
    }
    // W4/W5: the desk's books, pending-ledger and intents ride in the lines' `desk_state` blob
    crate::desk::state::before_upgrade();
    // W6/W7: the coins and the pool desk ride in the lines' `coins_state` / `pool_state` blobs the same way
    crate::coins::state::before_upgrade();
    crate::pooldesk::state::before_upgrade();
    crate::cranks::before_upgrade();
    crate::solnftflow::before_upgrade();
    let saved: Saved = (
        INIT.with(|i| i.borrow().clone()),
        PURCHASES.with(|p| p.borrow().clone()),
        PAUSED.with(|p| *p.borrow()),
        LOG.with(|l| l.borrow().clone()),
        Some(crate::custodynet::CUSTODY.with(|c| c.borrow().clone())),
        Some(crate::nftnet::NFT.with(|n| n.borrow().clone())),
        Some(crate::frognet::FROGS.with(|f| f.borrow().clone())),
        Some(crate::floornet::FLOORS.with(|f| f.borrow().clone())),
        Some((CUSTODY_BOOK.with(|b| b.borrow().clone()), OUTFLOW.with(|o| o.borrow().clone()))),
        Some(crate::handinnet::HANDINS.with(|h| h.borrow().clone())),
        Some(crate::salenet::SALES.with(|s| s.borrow().clone())),
        Some(crate::custodynet::FEE_POOL.with(|p| p.borrow().clone())),
        // round 4 (C4-H-1): the pin floors ride in the thirteenth element
        // 2026-10-12: the other chains' NFT sides ride in it too (`EthState.sides`)
        Some(crate::ethnet::ETH.with(|e| crate::ethflow::EthState { pin_floors: Some(pin_floors()), sides: crate::ethnet::sides_blob(),
            ..e.borrow().clone() })),
        // phase B2: the canister-wide Solana line (docs/v9/solana-migration.md §2.4)
        // the raffle's published snapshots ride in the line's `raffle_book` slot (docs/v9/raffle-design.md §4.6)
        crate::solhome::LINE.with(|l| l.borrow().clone()).map(|mut l| {
            l.raffle_book = Some(crate::raffle::take_book());
            l
        }),
        // W3: the crank, desk and pool lines, the scheduling cursor and the timer (docs/v9/canister-only-solana.md §13.1)
        Some(crate::solhome::LINES.with(|l| l.borrow().clone())),
    );
    // Failing here would silently drop the state on the next upgrade, which is the bug this function exists to fix. Trap
    // instead: a refused upgrade is recoverable and a forgotten purchase is not.
    ic_cdk::storage::stable_save((saved,)).expect("could not write state to stable memory; refusing the upgrade");
}

/// The fifth element (docs/v3/wrapper.md, "Step 3, the flow") is optional so a state saved before it existed still restores.
/// The sixth (docs/v4/eth-nft-canister.md §7) likewise. The seventh (docs/v3/curve.md, "Canister: …": item sinks, the mint
/// desk and holder deposits) likewise. The eighth (docs/v5/floor-feed.md §6.4: floor sinks, per-collection purchase limits,
/// NFT wants) likewise, after the seven, nothing reordered. The ninth (docs/v5/custody-hardening.md, H2 and H3: the custody
/// cap book and the outbound limiter) likewise, after the eight. The tenth (docs/v6/offer-handins.md §6: the offer boards,
/// the hand-ins read, their bytes and verdicts) likewise, after the nine. The eleventh (docs/v6/canister-floor-followups.md
/// §6: the sale policies, the sales and their listings, the ticket cursor) likewise, after the ten. The twelfth (audit D, H-4:
/// custody's fee pool) likewise, after the eleven. The thirteenth (docs/v7/phase1-nft.md §4.6: the Ethereum NFT side) likewise,
/// after the twelve. The fourteenth (docs/v9/solana-migration.md §2.4, phase B2: the ONE Solana line canister-wide, which B1 kept in
/// the thirteenth's `sol`) likewise, after the thirteen. The fifteenth (docs/v9/canister-only-solana.md §13.1, W3: the crank, desk and
/// pool send lines beside the fourteenth, which stays the money line; `solhome::migrate_lines` at `post_upgrade`) likewise.
pub type Saved = (Option<Init>, Vec<Purchase>, bool, Vec<String>, Option<crate::redeem::Custody>, Option<crate::chains::NftState>,
    Option<crate::frogflow::FrogState>, Option<crate::floortrigger::FloorState>,
    Option<(crate::custodycap::Book, crate::outflow::Outflow)>, Option<crate::handin::HandinState>, Option<crate::sale::SaleState>,
    Option<crate::custody::FeePool>, Option<crate::ethflow::EthState>, Option<crate::solhome::SolLine>, Option<crate::solhome::SolLines>);

/// The saved tuple read with every optional element as `opt reserved`: what was SAVED, whatever its shape (audit 2026-09-27-nft
/// C-L-4, the M-2 leftover). Candid decodes an `opt` whose content does not fit the expected type as `null`, silently; comparing
/// with this probe turns that into the refusal M-2 intends.
pub type SavedProbe = (candid::Reserved, candid::Reserved, candid::Reserved, candid::Reserved, Option<candid::Reserved>, Option<candid::Reserved>,
    Option<candid::Reserved>, Option<candid::Reserved>, Option<candid::Reserved>, Option<candid::Reserved>, Option<candid::Reserved>,
    Option<candid::Reserved>, Option<candid::Reserved>, Option<candid::Reserved>, Option<candid::Reserved>);

/// Which optional elements (5th to 14th, as 0-based indexes 4..=13) were saved present but decoded as nothing. `decoded` names the
/// 5th to 13th; the 14th (phase B2's line) is `decoded_line`.
pub fn lost_elements(probe: &SavedProbe, decoded: [bool; 9]) -> Vec<usize> {
    lost_elements_with(probe, decoded, true)
}

/// [`lost_elements`] with the fourteenth element's decode.
pub fn lost_elements_with(probe: &SavedProbe, decoded: [bool; 9], decoded_line: bool) -> Vec<usize> {
    lost_elements_lines(probe, decoded, decoded_line, true)
}

/// [`lost_elements_with`] with the fifteenth element's decode (W3's lines).
pub fn lost_elements_lines(probe: &SavedProbe, decoded: [bool; 9], decoded_line: bool, decoded_lines: bool) -> Vec<usize> {
    let saved = [probe.4.is_some(), probe.5.is_some(), probe.6.is_some(), probe.7.is_some(), probe.8.is_some(), probe.9.is_some(),
        probe.10.is_some(), probe.11.is_some(), probe.12.is_some(), probe.13.is_some(), probe.14.is_some()];
    let mut d = decoded.to_vec();
    d.push(decoded_line);
    d.push(decoded_lines);
    (0..11).filter(|&i| saved[i] && !d[i]).map(|i| i + 5).collect()
}

/// Audit 2026-09-27-nft round 2, C2-L-3: every `opt` of the saved state, at ANY depth, that was saved present and would restore as
/// absent. Candid decodes an `opt` whose content does not fit this build's type as `null` at the innermost such `opt` (an older
/// `EthTx` inside `EthState.line` restores as `line = None` while the rest of the element decodes), so the element-level probe
/// (`lost_elements`) cannot see it. `saved`: the stable memory's bytes (the saved tuple, as `stable_save` wrote it); `restored`: the
/// state this build decoded from them, encoded again. Both are read as untyped Candid values and walked together; a field or
/// case the new type no longer has is not compared (Candid drops it by design). The paths found name what would be lost.
pub fn lost_opts(saved: &[u8], restored: &[u8]) -> Result<Vec<String>, String> {
    use candid::types::value::IDLValue;
    fn first(b: &[u8]) -> Result<IDLValue, String> {
        let mut de = candid::de::IDLDeserialize::new(b).map_err(|e| format!("{e}"))?;
        de.get_value::<IDLValue>().map_err(|e| format!("{e}"))
    }
    fn walk(a: &IDLValue, b: &IDLValue, path: &str, out: &mut Vec<String>) {
        match (a, b) {
            (IDLValue::Opt(x), IDLValue::Opt(y)) => walk(x, y, path, out),
            (IDLValue::Opt(_), IDLValue::None | IDLValue::Null | IDLValue::Reserved) => out.push(path.to_string()),
            (IDLValue::Record(fs), IDLValue::Record(gs)) => {
                for f in fs {
                    if let Some(g) = gs.iter().find(|g| g.id.get_id() == f.id.get_id()) {
                        walk(&f.val, &g.val, &format!("{path}.{}", f.id), out);
                    }
                }
            }
            (IDLValue::Variant(v), IDLValue::Variant(w)) if v.0.id.get_id() == w.0.id.get_id() => {
                walk(&v.0.val, &w.0.val, &format!("{path}::{}", v.0.id), out)
            }
            (IDLValue::Vec(xs), IDLValue::Vec(ys)) => {
                for (i, (x, y)) in xs.iter().zip(ys.iter()).enumerate() {
                    walk(x, y, &format!("{path}[{i}]"), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&first(saved)?, &first(restored)?, "saved", &mut out);
    Ok(out)
}

#[ic_cdk::post_upgrade]
fn post_upgrade(arg: Init) {
    // C-L-4, round 2 C2-L-3: an optional part of the state saved present and restored as nothing, at any depth, is a decode failure
    // in disguise: refuse the upgrade
    if let Ok((saved,)) = ic_cdk::storage::stable_restore::<(Saved,)>() {
        let again = candid::encode_args((saved,)).unwrap_or_else(|e| ic_cdk::trap(&format!("refused: the restored state does not encode ({e})")));
        match lost_opts(&ic_cdk::stable::stable_bytes(), &again) {
            Ok(lost) if lost.is_empty() => {}
            Ok(lost) => ic_cdk::trap(&format!("refused: saved part(s) {lost:?} of the upgrade tuple do not decode into this build's types (they \
                would restore as nothing); nothing was changed. Keep every saved field decodable (new fields optional, new cases appended).")),
            Err(e) => ic_cdk::trap(&format!("refused: the saved state could not be compared with what restores ({e}); nothing was changed")),
        }
    }
    match ic_cdk::storage::stable_restore::<(Saved,)>() {
        Ok(((saved_init, purchases, paused, log, custody, nft, frogs, floors, guard, handins, sales, fee_pool, eth, sol_line, sol_lines),)) => {
            // The install argument is still validated -- an upgrade must not be a way to smuggle in a mainnet network name
            // or a single RPC provider -- but what the buyer KNOWS comes back from before.
            validate(&arg);
            let kept = restore_init(saved_init.clone(), &arg);
            // audit C2-08: the cap comparison (H2) and the held-items check (A5-L-1) run once the books and the frog state are restored
            // (below, after the desk's `after_upgrade`): the Inscriptions listings' caps are named from the books, which read empty here
            INIT.with(|i| *i.borrow_mut() = Some(kept.clone()));
            let (book, outflow) = guard.unwrap_or_default();
            CUSTODY_BOOK.with(|b| *b.borrow_mut() = book.clone());
            OUTFLOW.with(|o| *o.borrow_mut() = outflow);
            PURCHASES.with(|p| *p.borrow_mut() = purchases);
            PAUSED.with(|p| *p.borrow_mut() = paused);
            LOG.with(|l| *l.borrow_mut() = log);
            crate::custodynet::CUSTODY.with(|c| *c.borrow_mut() = custody.unwrap_or_default());
            let nft = nft.unwrap_or_default();
            let listed: Vec<u64> = arg.evm_chains.iter().flatten().map(|c| c.chain_id).collect();
            if let Some(c) = nft.busy_chains().into_iter().find(|c| !listed.contains(c)) {
                ic_cdk::trap(&format!("refused: chain {c} has a purchase in flight and this upgrade drops it from evm_chains"));
            }
            crate::nftnet::NFT.with(|n| *n.borrow_mut() = nft);
            crate::frognet::FROGS.with(|f| *f.borrow_mut() = frogs.unwrap_or_default());
            crate::floornet::FLOORS.with(|f| *f.borrow_mut() = floors.unwrap_or_default());
            crate::handinnet::HANDINS.with(|h| *h.borrow_mut() = handins.unwrap_or_default());
            crate::salenet::SALES.with(|s| *s.borrow_mut() = sales.unwrap_or_default());
            crate::custodynet::FEE_POOL.with(|p| *p.borrow_mut() = fee_pool.unwrap_or_default());
            let mut eth = eth.unwrap_or_default();
            // 2026-10-12: the other chains' NFT sides come out of the thirteenth element (one that does not decode refuses the upgrade)
            let sides = crate::ethnet::sides_of(eth.sides.take()).unwrap_or_else(|e| ic_cdk::trap(&format!("refused: {e}")));
            // audit C3-06: a side on the chain this upgrade makes the config's own is never ticked again: refused while it holds work
            let sides = crate::ethnet::sides_kept(sides, arg.eth_nft.as_ref().map(|c| c.chain_id))
                .unwrap_or_else(|e| ic_cdk::trap(&format!("refused: {e}")));
            // round 2, C2-L-2: an upgrade that drops (or re-points) a collection with work still open would leave that work, and the
            // one Relay leg it may hold, waiting for ever (the config's own side against its chain's collections; each other side
            // against its chain's, or none when the upgrade drops its chain)
            let own = arg.eth_nft.as_ref().map(|c| c.on(c.chain_id));
            let mut stranded = crate::ethflow::stranded_by(&eth, own.as_ref().or(arg.eth_nft.as_ref()));
            for (chain, st) in &sides {
                let next = arg.eth_nft.as_ref().filter(|c| c.chains().contains(chain)).map(|c| c.on(*chain));
                stranded.extend(crate::ethflow::stranded_by(st, next.as_ref()).into_iter().map(|x| format!("chain {chain}: {x}")));
            }
            if !stranded.is_empty() {
                ic_cdk::trap(&format!("refused: this upgrade drops or re-points an Ethereum collection with work still open: {}", stranded.join("; ")));
            }
            crate::ethnet::restore_sides(sides);
            // audit 2026-09-27-nft B-L-5: a line, a pending home-chain transaction or an open buy is work only this side finishes;
            // an upgrade that drops or moves its chain would leave it (and every flow gated on it) waiting for ever
            if eth.line.is_some() || eth.home_busy() || eth.buys.iter().any(|b| !b.finished()) {
                let busy = saved_init.as_ref().and_then(|i| i.eth_nft.as_ref()).map(|e| e.chain_id);
                if busy.is_some() && arg.eth_nft.as_ref().map(|e| e.chain_id) != busy {
                    ic_cdk::trap("refused: the Ethereum NFT side has a transaction in flight, a home-chain transaction pending or an open \
                        buy, and this upgrade drops or moves its chain");
                }
            }
            // docs/v9/solana-migration.md §4: the home is set once, from nothing. Its amounts are wei on the EVM home and lamports on
            // Solana, so the Ethereum NFT side must be quiet when an upgrade first names a Solana home: no home transaction pending, no
            // open buy or delivery whose records would be read in the other currency
            if saved_init.as_ref().is_some_and(|i| i.home.is_none()) && arg.home.is_some()
                && (eth.pending.is_some() || eth.buys.iter().any(|b| !b.finished()) || eth.deliveries.iter().any(|d| !d.finished())) {
                ic_cdk::trap("refused: this upgrade first names a Solana home while the Ethereum NFT side has a home transaction pending, an \
                    open buy or an open delivery on the EVM home: finish them first (the home's amounts change currency)");
            }
            // phase B2: likewise the frog side: its amounts are wei on the EVM home and lamports on Solana, so nothing of it may be open
            // when an upgrade first names a Solana home (a frog wanted or bought, a conversion, an EVM transaction of it)
            if saved_init.as_ref().is_some_and(|i| i.home.is_none()) && arg.home.is_some() {
                let f = crate::frognet::FROGS.with(|f| f.borrow().clone());
                if f.pending.is_some() || f.frogs.iter().any(|x| !x.finished()) || f.convs().iter().any(|c| !c.finished()) {
                    ic_cdk::trap("refused: this upgrade first names a Solana home while the frog side has a frog, a conversion or a transaction \
                        open on the EVM home: finish them first (the home's amounts change currency)");
                }
            }
            // round 4 (C4-H-1): the pin floors come back with it (a state saved before them starts at nothing: the first tick sets them).
            // Review round 5 (F5-L-1): the Ethereum floor (and the Ethereum nonce floor) are the configured chain's: an upgrade that
            // moves eth_nft to another chain (allowed only while nothing is open) starts them afresh there
            let mut floors = eth.pin_floors.unwrap_or_default();
            let from = saved_init.as_ref().and_then(|i| i.eth_nft.as_ref()).map(|e| e.chain_id);
            if crate::ethflow::chain_moved(&mut eth, &mut floors, from, arg.eth_nft.as_ref().map(|e| e.chain_id)) {
                note("upgraded: eth_nft moved to another chain: its pin and nonce floors start afresh");
            }
            PIN_FLOORS.with(|f| *f.borrow_mut() = floors);
            // phase B2: B1's line (kept in EthState.sol) becomes the canister-wide one, once; its pending step stays the Ethereum side's
            let moved = eth.sol.is_some();
            let mut sol_line = sol_line;
            crate::raffle::restore_book(sol_line.as_mut().and_then(|l| l.raffle_book.take()).unwrap_or_default());
            let line = crate::solhome::migrate(sol_line, &mut eth).unwrap_or_else(|e| ic_cdk::trap(&e));
            if moved {
                note("upgraded: the Solana home line moved from the Ethereum side's state to the canister-wide slot (phase B2)");
            }
            // W3: the single line stays the money line (once)
            let first_lines = sol_lines.is_none() && line.is_some();
            let (line, lines) = crate::solhome::migrate_lines(line, sol_lines).unwrap_or_else(|e| ic_cdk::trap(&e));
            if first_lines {
                note("upgraded: four Solana send lines (money, crank, desk, pool) (W3)");
            }
            crate::solhome::LINE.with(|l| *l.borrow_mut() = line);
            crate::solhome::LINES.with(|l| *l.borrow_mut() = lines);
            // W4/W5: the desk comes out of the lines' blob (a blob that does not decode refuses the upgrade)
            crate::desk::state::after_upgrade().unwrap_or_else(|e| ic_cdk::trap(&format!("refused: {e}")));
            crate::coins::state::after_upgrade().unwrap_or_else(|e| ic_cdk::trap(&format!("refused: {e}")));
            crate::pooldesk::state::after_upgrade().unwrap_or_else(|e| ic_cdk::trap(&format!("refused: {e}")));
            crate::cranks::after_upgrade().unwrap_or_else(|e| ic_cdk::trap(&format!("refused: {e}")));
            // Ordinals multi-collection: the single frog and custodian states become the books' first Inscriptions listing's
            crate::frognet::ensure_migrated();
            // H2 at upgrade (audit C2-08: after the books and the frog state are restored, so both sides name the same listings): the
            // caps after against the caps before; then (review round 5, A5-L-1) what custody holds now must leave within the delay at
            // the limits kept
            if let Err(e) = check_cap_change(saved_init.as_ref(), &kept) {
                ic_cdk::trap(&e);
            }
            if let Err(e) = held_fits(&caps_of(&kept), &book, &limits_of(&kept)) {
                ic_cdk::trap(&e);
            }
            crate::solnftflow::after_upgrade().unwrap_or_else(|e| ic_cdk::trap(&format!("refused: {e}")));
            crate::ethnet::ETH.with(|e| *e.borrow_mut() = eth);
            note("upgraded: state restored");
            // W14b: the canister's own Solana timer (timers do not survive an upgrade)
            crate::soltimer::arm_after_install();
        }
        // M-2 (audit D): a state that does not decode is never a fresh install. Starting from nothing here would drop every
        // redemption and its signed bytes, UNPAUSE a paused buyer, and empty the cap book and the limiter. The IC's install is
        // atomic, so trapping refuses the upgrade and keeps everything as it was.
        Err(e) => ic_cdk::trap(&format!("refused: the saved state does not decode into this build's types ({e}); nothing was \
            changed. An upgrade must keep every saved field decodable (new fields optional, new cases appended).")),
    }
}

/// H2 at upgrade: the caps in force after it (the wrapper as kept) against the caps in force before it. Lowering is always
/// allowed; a raise is at most ×4, and a collection new to the policy starts at stage 0 at most. Also the H3 exit check on
/// what is kept (the wrapper may come from the saved state, not the argument).
fn check_cap_change(saved: Option<&Init>, kept: &Init) -> Result<(), String> {
    validate_custody(kept)?;
    let old = saved.map(caps_of).unwrap_or_default();
    crate::custodycap::validate_change(&old, &caps_of(kept)).map_err(|e| format!("refused: custody_caps: {e}"))?;
    // I-5 (audit D): the outbound limits follow the caps' ×4 rule; an install starts from stage 0's
    let old_limits = saved.map(limits_of).unwrap_or_else(crate::outflow::Limits::stage0);
    crate::outflow::validate_change(&old_limits, &limits_of(kept)).map_err(|e| format!("refused: outflow_limits: {e}"))
}

pub(crate) fn cfg() -> Init {
    INIT.with(|i| i.borrow().clone().expect("not installed"))
}

/// Audit C4-03: the chain ids of `evm_chains` (none before the install): an EthNft listing on a chain other than Ethereum opens only
/// once its chain is here (the Robinhood Chain collections after the ceremony's 4663 read-back).
pub(crate) fn evm_chain_ids() -> Vec<u64> {
    INIT.with(|i| i.borrow().as_ref().and_then(|x| x.evm_chains.as_ref()).map(|c| c.iter().map(|x| x.chain_id).collect()).unwrap_or_default())
}

/// Audit C4-11: whether the install has a home (the coins open nothing without the Solana one); false before the install.
pub(crate) fn has_home() -> bool {
    INIT.with(|i| i.borrow().as_ref().is_some_and(|x| x.home.is_some()))
}

// ---- the two addresses, derived from keys nobody holds ---------------------------------------------------------------------

fn schnorr_key() -> SchnorrKeyId {
    SchnorrKeyId { algorithm: SchnorrAlgorithm::Bip340secp256k1, name: cfg().key_name }
}

fn ecdsa_key() -> EcdsaKeyId {
    EcdsaKeyId { curve: EcdsaCurve::Secp256k1, name: cfg().key_name }
}

/// The buyer's taproot **internal** key, as a 32-byte x-only key. NOT the output key: `schnorr_public_key` returns the
/// canister's untweaked BIP340 key, and the subnet applies BIP341's tweak later, when it signs (`aux = #bip341`). The output
/// key is `Q = P + H_tapTweak(P)·G` and it is what goes in the scriptPubKey.
///
/// This doc comment used to say "output key" and it was wrong. An address built on that reading commits to P while every
/// signature is made under Q, so it can never be spent. Nothing in this repository could catch it -- a test that signs and
/// then verifies reuses the assumption it is testing -- and it was found by broadcasting to signet, where a node disagreed.
/// `signet-rehearsal.mjs::checkKeyKind` now settles the question by asking the subnet rather than the documentation.
/// See `docs/v2/2026-09-22-signet-rehearsal.md`.
#[ic_cdk::update]
pub async fn taproot_key() -> Result<String, String> {
    only_operator();
    let res = schnorr_public_key(&SchnorrPublicKeyArgs {
        canister_id: None,
        derivation_path: vec![b"btc".to_vec()],
        key_id: schnorr_key(),
    })
    .await
    .map_err(|e| format!("{e:?}"))?;
    // The reply is a 33-byte compressed key; taproot uses the x coordinate alone.
    Ok(hex::encode(&res.public_key[1..]))
}

/// The taproot **output** key: the internal key with BIP341's tweak applied.
///
/// [`taproot_key`] returns the key the subnet derives, which is the *internal* key. [`sign_taproot`] passes
/// `aux: Bip341 { merkle_root_hash: [] }`, so the subnet tweaks before signing and the signature verifies under a
/// different number entirely. The two are one function call apart and reading one for the other is the mistake that once
/// left a rehearsal's coins unspendable, so the tweaked key has its own name here rather than being a step every caller is
/// trusted to remember. This is what an address is built from, and what a verifier checks against.
#[ic_cdk::update]
pub async fn taproot_output_key() -> Result<String, String> {
    only_operator();
    let res = schnorr_public_key(&SchnorrPublicKeyArgs {
        canister_id: None,
        derivation_path: vec![b"btc".to_vec()],
        key_id: schnorr_key(),
    })
    .await
    .map_err(|e| format!("{e:?}"))?;
    let internal: [u8; 32] = res.public_key[1..33].try_into().map_err(|_| "the subnet's key is not 33 bytes".to_string())?;
    tweak_xonly(internal).map(hex::encode)
}

// ---- custody: a key of its own (docs/v3/wrapper.md, "Custody on its own key") ------------------------------------------------

/// The buyer's Schnorr derivation path. Purchases sign with this key only, for inputs paying its script only.
pub(crate) const BUYER_PATH: &[u8] = b"btc";
/// The custodian's: wrapped frogs are held at this key's address, and only deliveries sign with it.
pub(crate) const CUSTODY_PATH: &[u8] = b"custody";

/// The tweaked (BIP341 key-path) output key for `path`.
pub(crate) async fn output_key_for(path: &[u8]) -> Result<[u8; 32], String> {
    tweak_xonly(internal_key_for(path).await?)
}

/// The untweaked x-only **internal** key for `path`, as the subnet derives it (see [`taproot_key`]): what a PSBT's
/// `PSBT_IN_TAP_INTERNAL_KEY` names, never what an address is built from.
pub(crate) async fn internal_key_for(path: &[u8]) -> Result<[u8; 32], String> {
    let res = schnorr_public_key(&SchnorrPublicKeyArgs {
        canister_id: None,
        derivation_path: vec![path.to_vec()],
        key_id: schnorr_key(),
    })
    .await
    .map_err(|e| format!("{e:?}"))?;
    res.public_key.get(1..33).and_then(|k| k.try_into().ok()).ok_or_else(|| "the subnet's key is not 33 bytes".to_string())
}

/// A BIP341 key-path signature by `path`'s key over `sighash`.
pub(crate) async fn sign_taproot_with(path: &[u8], sighash: Vec<u8>) -> Result<Vec<u8>, String> {
    let res = sign_with_schnorr(&SignWithSchnorrArgs {
        message: sighash,
        derivation_path: vec![path.to_vec()],
        key_id: schnorr_key(),
        aux: Some(SchnorrAux::Bip341(Bip341 { merkle_root_hash: vec![] })),
    })
    .await
    .map_err(|e| format!("{e:?}"))?;
    Ok(res.signature)
}

thread_local! {
    static CUSTODY_SCRIPT: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// The custody address's script (`OP_1 <custody output key>`), derived once.
pub(crate) async fn custody_script() -> Result<Vec<u8>, String> {
    if let Some(s) = CUSTODY_SCRIPT.with(|c| c.borrow().clone()) {
        return Ok(s);
    }
    let s = [vec![0x51, 0x20], output_key_for(CUSTODY_PATH).await?.to_vec()].concat();
    CUSTODY_SCRIPT.with(|c| *c.borrow_mut() = Some(s.clone()));
    Ok(s)
}

/// The custody output key and address, for the wrapper's `custodyAddress` and for anyone checking where the frogs are. Its
/// derivation path differs from the buyer's, so its address differs: a purchase can never sign for a frog.
#[ic_cdk::update]
pub async fn custody_address() -> Result<String, String> {
    only_operator();
    let key = output_key_for(CUSTODY_PATH).await?;
    let hrp = crate::policy::hrp_for(&cfg().btc_network)?;
    Ok(format!("{} {}", crate::policy::taproot_address(&key, hrp), hex::encode(key)))
}

/// The buyer's taproot address, beside the custody one, so the two can be compared.
#[ic_cdk::update]
pub async fn buyer_address() -> Result<String, String> {
    only_operator();
    let key = output_key_for(BUYER_PATH).await?;
    let hrp = crate::policy::hrp_for(&cfg().btc_network)?;
    Ok(format!("{} {}", crate::policy::taproot_address(&key, hrp), hex::encode(key)))
}

/// `Q = P + int(tagged_hash("TapTweak", x(P))) * G`, with `P` the even-Y lift of `x`. BIP341, for a key-path-only output,
/// so the tweak commits to no script tree.
fn tweak_xonly(x: [u8; 32]) -> Result<[u8; 32], String> {
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    use k256::elliptic_curve::PrimeField;
    use k256::{ProjectivePoint, PublicKey, Scalar};

    let mut sec1 = [0u8; 33];
    sec1[0] = 0x02; // lift_x takes the even-Y point, which is what an x-only key means
    sec1[1..].copy_from_slice(&x);
    let p = PublicKey::from_sec1_bytes(&sec1).map_err(|e| format!("the internal key is not on the curve: {e}"))?;

    let t = sighash::tagged("TapTweak", &[&x]);
    let scalar = Option::<Scalar>::from(Scalar::from_repr(t.into()))
        .ok_or("the tweak is not a valid scalar, which has probability about 2^-128")?;

    let q = ProjectivePoint::from(*p.as_affine()) + ProjectivePoint::GENERATOR * scalar;
    let enc = q.to_affine().to_encoded_point(true);
    enc.as_bytes()[1..33].try_into().map_err(|_| "the tweaked point did not encode".to_string())
}

/// The buyer's EVM address: the low 20 bytes of the keccak of its uncompressed public key. The vault is constructed with this
/// address and can send to no other.
#[ic_cdk::update]
pub async fn evm_address() -> Result<String, String> {
    only_operator();
    let res = ecdsa_public_key(&EcdsaPublicKeyArgs {
        canister_id: None,
        derivation_path: vec![b"evm".to_vec()],
        key_id: ecdsa_key(),
    })
    .await
    .map_err(|e| format!("{e:?}"))?;
    Ok(format!("0x{}", hex::encode(&keccak(&decompress(&res.public_key)?)[12..])))
}

// ---- reading chain 4663: two providers, and they must agree ----------------------------------------------------------------

/// One JSON-RPC call, made by every node and reduced by consensus. A read is idempotent, so replication is free here and the
/// integrity is worth having. A `null` result is a result ("no receipt yet"), not an error; a reply with no `result` at all is.
async fn rpc_once(url: &str, body: String) -> Result<serde_json::Value, String> {
    rpc_once_with(url, body, "strip_volatile", vec![], 64_000).await
}

/// [`rpc_once`] through a named transform with its context, and a response cap sized to the call (a cap is paid for).
pub(crate) async fn rpc_once_with(url: &str, body: String, transform: &str, context: Vec<u8>, max_bytes: u64) -> Result<serde_json::Value, String> {
    let req = HttpRequestArgs {
        url: url.to_string(),
        method: HttpMethod::POST,
        body: Some(body.into_bytes()),
        max_response_bytes: Some(max_bytes),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: transform.into() }),
            context,
        }),
        headers: vec![HttpHeader { name: "content-type".into(), value: "application/json".into() }],
        is_replicated: Some(true),
    };
    let res: HttpRequestResult = http_request(&req).await.map_err(outcall_error)?;
    let parsed: serde_json::Value = serde_json::from_slice(&res.body).map_err(|e| format!("unreadable: {e}"))?;
    match parsed.get("result") {
        Some(r) => Ok(r.clone()),
        None => Err(format!("no result: {}", parsed.get("error").map(|e| e.to_string()).unwrap_or_default())),
    }
}

/// An HTTPS outcall's failure as text. The IC's own reject of an answer over the response cap (and only that) is marked with
/// `evmread::SIZE_REJECT` at the start, which the ticket-log scanner alone treats as a size problem (audit R3, R3-C2): a
/// provider's words never start the text (they arrive as `no result: …`), so no provider can pass for a size error.
pub(crate) fn outcall_error(e: ic_cdk::call::Error) -> String {
    if let ic_cdk::call::Error::CallRejected(r) = &e {
        if crate::evmread::ic_size_reject(r.reject_message()) {
            return format!("{}{}", crate::evmread::SIZE_REJECT, r.reject_message());
        }
    }
    format!("{e:?}")
}

/// Ask every configured provider and accept the answer only if they agree. Two providers is a weaker check than three;
/// Robinhood Chain has, as far as we know, two. Said plainly rather than glossed.
async fn rpc(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let c = cfg();
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let mut answers: Vec<serde_json::Value> = Vec::new();
    for url in &c.rpc_urls {
        answers.push(rpc_once(url, body.clone()).await?);
    }
    let first = answers.first().ok_or("no providers")?.clone();
    if answers.iter().any(|a| *a != first) {
        return Err(format!("providers disagree on {method}; nothing is believed"));
    }
    Ok(first)
}

/// [`rpc`], but the providers must agree only on what `view` keeps of each answer. For a receipt or a transaction, where a
/// provider's extra fields are presentation and not a disagreement about the chain (see [`receipt_view`]).
async fn rpc_view(method: &str, params: serde_json::Value, view: fn(&serde_json::Value) -> serde_json::Value)
    -> Result<serde_json::Value, String> {
    let c = cfg();
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let mut answers: Vec<serde_json::Value> = Vec::new();
    for url in &c.rpc_urls {
        answers.push(view(&rpc_once(url, body.clone()).await?));
    }
    let first = answers.first().ok_or("no providers")?.clone();
    if answers.iter().any(|a| *a != first) {
        return Err(format!("providers disagree on {method}; nothing is believed"));
    }
    Ok(first)
}

/// Every HTTPS response passes through here before consensus sees it. Headers carry dates and request ids that differ per
/// node, so they are dropped; the body is what the nodes must agree on.
#[ic_cdk::query]
fn strip_volatile(args: TransformArgs) -> HttpRequestResult {
    HttpRequestResult { status: args.response.status, body: args.response.body, headers: vec![] }
}

/// A JSON-RPC state read in one canonical shape (`pin::json_rpc_norm_body`, audit 2026-09-27-nft B-M-6): replicas agree on what
/// the chain said, not on how the provider wrote it, and a revert is told apart from a failure.
#[ic_cdk::query]
fn json_rpc_norm(args: TransformArgs) -> HttpRequestResult {
    HttpRequestResult { status: candid::Nat::from(200u32), body: crate::pin::json_rpc_norm_body(&args.response.body), headers: vec![] }
}

// ---- pinned reads (docs/v2/pinned-reads.md): every state read names one block every node and provider names ----------------

/// `eth_blockNumber`, rounded down to a multiple of the bucket in the context, so nodes asking moments apart agree.
#[ic_cdk::query]
fn head_bucket(args: TransformArgs) -> HttpRequestResult {
    let b = crate::pin::u64_of_context(&args.context).unwrap_or(0);
    HttpRequestResult { status: args.response.status, body: crate::pin::head_bucket_body(&args.response.body, b), headers: vec![] }
}

/// `eth_getBlockByNumber(pin)`, projected to `{number, hash, baseFeePerGas}`.
#[ic_cdk::query]
fn block_at(args: TransformArgs) -> HttpRequestResult {
    HttpRequestResult { status: args.response.status, body: crate::pin::block_body(&args.response.body), headers: vec![] }
}

/// `eth_getTransactionReceipt`, counted only at or below the pin in the context.
#[ic_cdk::query]
fn receipt_at_pin(args: TransformArgs) -> HttpRequestResult {
    let pin = crate::pin::u64_of_context(&args.context).unwrap_or(0);
    HttpRequestResult { status: args.response.status, body: crate::pin::receipt_at_pin_body(&args.response.body, pin), headers: vec![] }
}

/// Ask every provider through `transform` and believe the answer only if every body agrees.
async fn rpc_agreed(method: &str, params: serde_json::Value, transform: &str, context: Vec<u8>, max_bytes: u64)
    -> Result<Vec<serde_json::Value>, String> {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let mut answers = Vec::new();
    for url in &cfg().rpc_urls {
        answers.push(rpc_once_with(url, body.clone(), transform, context.clone(), max_bytes).await?);
    }
    Ok(answers)
}

/// One configured provider, over replicated outcalls and the transforms above (`evmread::Rpc`). The multi-provider logic (the
/// fast pin, finality, agreement) is `evmread.rs`, tested against fakes with each real provider's limits.
pub(crate) struct IcRpc {
    url: String,
}

/// Every configured provider.
pub(crate) fn ic_rpcs() -> Vec<IcRpc> {
    cfg().rpc_urls.into_iter().map(|url| IcRpc { url }).collect()
}

fn body_of(method: &str, params: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string()
}

impl crate::evmread::Rpc for IcRpc {
    fn name(&self) -> String {
        self.url.clone()
    }
    async fn head(&mut self, bucket: u64) -> Result<u64, String> {
        // the cap counts the provider's headers as well as the body: ordofi's exceed 1,000 bytes on a 60-byte answer (found live)
        let v = rpc_once_with(&self.url, body_of("eth_blockNumber", serde_json::json!([])), "head_bucket",
            crate::pin::context_u64(bucket), 8_000).await?;
        Ok(hex_u128(&v)? as u64)
    }
    async fn finalized(&mut self, bucket: u64) -> Result<u64, String> {
        let v = rpc_once_with(&self.url, body_of("eth_getBlockByNumber", serde_json::json!(["finalized", false])), "finalized_bucket",
            crate::pin::context_u64(bucket), 16_000).await?;
        Ok(hex_u128(&v)? as u64)
    }
    async fn block(&mut self, n: u64) -> Result<serde_json::Value, String> {
        rpc_once_with(&self.url, body_of("eth_getBlockByNumber", serde_json::json!([format!("0x{n:x}"), false])), "block_at", vec![], 64_000).await
    }
    async fn call(&mut self, to: &[u8; 20], data: &[u8], at: &crate::pin::Pin) -> Result<Vec<u8>, String> {
        hex_bytes(&rpc_once(&self.url, body_of("eth_call", serde_json::json!([{ "to": h0x(to), "data": h0x(data) }, crate::pin::at(at)]))).await?)
    }
    async fn logs(&mut self, address: &[u8; 20], topics: &serde_json::Value, from: u64, to: u64, max_bytes: u64)
        -> Result<Vec<crate::evmread::LogView>, String> {
        let filter = serde_json::json!([{ "address": h0x(address), "topics": topics, "fromBlock": format!("0x{from:x}"), "toBlock": format!("0x{to:x}") }]);
        // the scanner chooses the cap (`ticketlog::LOG_CAPS`): 48 kB holds ≈ 50 `Redeemed` logs with ordofi's ≈ 1 kB of headers;
        // a page over it is retried at a larger cap, then a smaller span, then by ticket (audit R2, N-1)
        let v = rpc_once_with(&self.url, body_of("eth_getLogs", filter), "logs_view", vec![],
            max_bytes.min(crate::evmread::MAX_RESPONSE_BYTES)).await?;
        crate::evmread::logs_of(&v)
    }
}

/// The pin (docs/v2/pinned-reads.md): every provider's head rounded to [`crate::pin::BUCKET`], one bucket below the lowest, and
/// that block agreed by hash on every provider (`evmread::fast_pin`). Each step fails closed: any error or disagreement and
/// nothing is believed this tick. Review round 4 (C4-H-1): never below a pin already read (`PIN_FLOORS.home`): every 4663 read of
/// every part of the canister (the vault loop, the custodian, the frog side, the Ethereum NFT side, the endpoints) is taken here, so
/// none of them reads or signs at a pin behind one of our own mined transactions.
async fn pin() -> Result<crate::pin::Pin, String> {
    let p = crate::evmread::fast_pin(&mut ic_rpcs()).await?;
    PIN_FLOORS.with(|f| f.borrow_mut().home.admit_pin("4663", p.number))?;
    Ok(p)
}

/// `eth_getBlockByNumber("finalized")`, projected to its number rounded down to the bucket in the context (H9).
#[ic_cdk::query]
fn finalized_bucket(args: TransformArgs) -> HttpRequestResult {
    let b = crate::pin::u64_of_context(&args.context).unwrap_or(0);
    HttpRequestResult { status: args.response.status, body: crate::pin::finalized_bucket_body(&args.response.body, b), headers: vec![] }
}

/// `eth_getLogs`, projected to what every provider agrees on, with each log's block (audit D, H-1).
#[ic_cdk::query]
fn logs_view(args: TransformArgs) -> HttpRequestResult {
    HttpRequestResult { status: args.response.status, body: crate::evmread::logs_body(&args.response.body), headers: vec![] }
}

// ---- sending to chain 4663: one node sends, every node reads back (audit B4; see `evmsend`) -------------------------------

/// One provider, one send, made by a **single** node (`is_replicated: false`), because a replicated send cannot reach consensus:
/// the first copy is accepted and every later one is "already known". Its reply is a report and is returned as one.
pub(crate) async fn send_raw_once(url: &str, raw_hex: &str, our_hash: &str) -> SendOutcome {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_sendRawTransaction", "params": [raw_hex] }).to_string();
    let req = HttpRequestArgs {
        url: url.to_string(),
        method: HttpMethod::POST,
        body: Some(body.into_bytes()),
        max_response_bytes: Some(8_000),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: "strip_volatile".into() }),
            context: vec![],
        }),
        headers: vec![HttpHeader { name: "content-type".into(), value: "application/json".into() }],
        is_replicated: Some(false),
    };
    match http_request(&req).await {
        Ok(res) => classify(res.status.to_string().parse().unwrap_or(0), &res.body, our_hash),
        Err(e) => SendOutcome::Unreadable(format!("the outcall itself failed: {e:?}")),
    }
}

/// What the chain says about a transaction, read replicated through [`rpc`]: every node, every provider, one answer.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum EvmSeen {
    /// No provider has it.
    NotYet,
    /// In a mempool, not in a block.
    Pending,
    /// In a block. `success` is the receipt's status: a mined transaction can still have reverted.
    Mined { block: u64, success: bool, from: String },
    /// The providers did not agree, or a read failed. Nothing is believed; ask again later.
    Unsure(String),
}

/// The read-back. This, not the send, is what the buyer acts on.
async fn evm_seen(hash: &str, pin: u64) -> EvmSeen {
    let receipt = rpc_agreed("eth_getTransactionReceipt", serde_json::json!([hash]), "receipt_at_pin", crate::pin::context_u64(pin), 64_000)
        .await
        .and_then(|rs| {
            let first = rs.first().cloned().ok_or("no providers")?;
            if rs.iter().any(|r| *r != first) { Err("providers disagree on eth_getTransactionReceipt at the pin; nothing is believed".to_string()) } else { Ok(first) }
        });
    match receipt {
        Err(e) => EvmSeen::Unsure(e),
        Ok(r) if !r.is_null() => {
            let hex = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("");
            let Ok(block) = u64::from_str_radix(hex("blockNumber").trim_start_matches("0x"), 16) else {
                return EvmSeen::Unsure("a receipt without a readable blockNumber".into());
            };
            EvmSeen::Mined { block, success: hex("status") == "0x1", from: hex("from").to_ascii_lowercase() }
        }
        Ok(_) => match rpc_view("eth_getTransactionByHash", serde_json::json!([hash]), tx_view).await {
            Err(e) => EvmSeen::Unsure(e),
            Ok(t) if t.is_null() => EvmSeen::NotYet,
            Ok(_) => EvmSeen::Pending,
        },
    }
}

/// Review round 4 (C4-M-1): a 4663 transaction for the payout judge (`ethflow::judge_payout`), read at a (floored) pin, every
/// provider agreeing: `None` when it is not mined at the pin.
pub(crate) async fn home_payout_tx(hash: [u8; 32]) -> Result<Option<crate::ethflow::PayoutTx>, String> {
    let p = pin().await?;
    let h = h0x(&hash);
    let rs = rpc_agreed("eth_getTransactionReceipt", serde_json::json!([h]), "receipt_at_pin", crate::pin::context_u64(p.number), 64_000).await?;
    let r = rs.first().cloned().ok_or("no providers")?;
    if rs.iter().any(|x| *x != r) {
        return Err("providers disagree on eth_getTransactionReceipt at the pin; nothing is believed".into());
    }
    if r.is_null() {
        return Ok(None);
    }
    let block = u64::from_str_radix(r.get("blockNumber").and_then(|x| x.as_str()).unwrap_or("").trim_start_matches("0x"), 16)
        .map_err(|_| "a receipt without a readable blockNumber")?;
    let success = r.get("status").and_then(|x| x.as_str()) == Some("0x1");
    let t = rpc_view("eth_getTransactionByHash", serde_json::json!([h]), crate::evmsend::payout_view).await?;
    if t.is_null() {
        return Err("the transaction's receipt is there, the transaction is not: nothing is believed".into());
    }
    crate::ethflow::PayoutTx::of(&t, block, success).map(Some)
}

/// Review round 6 (C6-L-1): a 4663 transaction's receipt with its logs at the pin (`ethnet`'s `eth_receipt_logs` projection), every
/// provider agreeing: the payout judge reads Relay's own execution record there.
pub(crate) async fn home_receipt_logs(hash: [u8; 32]) -> Result<Option<crate::ethnft::ReceiptV>, String> {
    let p = pin().await?;
    let rs = rpc_agreed("eth_getTransactionReceipt", serde_json::json!([h0x(&hash)]), "eth_receipt_logs", crate::pin::context_u64(p.number),
        200_000).await?;
    let r = rs.first().cloned().ok_or("no providers")?;
    if rs.iter().any(|x| *x != r) {
        return Err("providers disagree on eth_getTransactionReceipt at the pin; nothing is believed".into());
    }
    crate::ethnft::receipt_of(&r)
}

/// Review round 5 (F5-M-1): a block's transactions from the address in the context (`evmsend::our_txs_body`).
#[ic_cdk::query]
fn our_txs_view(args: TransformArgs) -> HttpRequestResult {
    let us: [u8; 20] = args.context.clone().try_into().unwrap_or([0; 20]);
    HttpRequestResult { status: args.response.status, body: crate::evmsend::our_txs_body(&args.response.body, &us), headers: vec![] }
}

async fn nonce_at(us: &[u8; 20], block: u64) -> Result<u64, String> {
    Ok(hex_u128(&rpc("eth_getTransactionCount", serde_json::json!([h0x(us), format!("0x{block:x}")])).await?)? as u64)
}

/// Review round 5 (F5-M-1): which transaction of ours used our 4663 nonce `nonce`, without the transaction index: our nonce count
/// at past blocks (bisected from `from`, a pin at which it was unused, to the pin), then that block's transactions from us, every
/// provider agreeing at each step. `Ok(None)`: the nonce is not used at the pin. An error: it cannot be told (a provider keeps no
/// state that deep, a disagreement), and the home line decides nothing on it.
pub(crate) async fn home_nonce_tx(nonce: u64, from: Option<u64>) -> Result<Option<[u8; 32]>, String> {
    let p = pin().await?;
    let us = ours().await?.0;
    if nonce_at(&us, p.number).await? <= nonce {
        return Ok(None);
    }
    // review round 6 (F6-L-1): 4663's first provider keeps state ≈ 6,200 blocks under its head (≈ 10 minutes). Review round 7
    // (F7-L-1): the search starts at the highest block where the nonce is known unused and state is still kept: the lowest provider's
    // rounded head at signing (a bucket above the transaction's own pin), or inside the reach for one mined late; the transaction's own
    // pin only if both are refused (`ethflow::home_search_starts`). A refusal of every start is an error: the step decides by its
    // effect, or the operator
    let mut lo = None;
    let mut why = String::new();
    for c in crate::ethflow::home_search_starts(from, p.number, crate::pin::BUCKET) {
        match nonce_at(&us, c).await {
            Ok(n) if n <= nonce => {
                lo = Some(c);
                break;
            }
            Ok(_) => why = format!("our nonce {nonce} was already used at block {c}, where the search would start"),
            Err(e) => why = format!("our nonce count at block {c} ({} blocks under the pin: 4663's providers keep state ≈ 6,200 blocks \
                under their head, ≈ 10 minutes) is not read: {e}", p.number - c),
        }
    }
    let Some(mut lo) = lo else { return Err(why) };
    let mut hi = p.number;
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if nonce_at(&us, mid).await? > nonce { hi = mid } else { lo = mid }
    }
    let got = rpc_agreed("eth_getBlockByNumber", serde_json::json!([format!("0x{hi:x}"), true]), "our_txs_view", us.to_vec(), 2_000_000).await?;
    let first = got.first().cloned().ok_or("no providers")?;
    if got.iter().any(|x| *x != first) {
        return Err(format!("providers disagree on block {hi}'s transactions; nothing is believed"));
    }
    for t in first.as_array().ok_or("an unusable block")? {
        let n = t.get(1).and_then(|x| x.as_str()).and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok());
        if n == Some(nonce) {
            let h = t.get(0).and_then(|x| x.as_str()).ok_or("a transaction without a hash")?;
            return hex::decode(h.trim_start_matches("0x")).ok().and_then(|b| b.try_into().ok()).map(Some)
                .ok_or_else(|| "a transaction hash that is not 32 bytes".into());
        }
    }
    Err(format!("block {hi} moved our nonce past {nonce} but holds no transaction of ours at it"))
}

/// What one broadcast did: each provider's report, and what the chain then said.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EvmSend {
    /// The keccak of the bytes, computed here: the only hash we ever watch.
    pub tx_hash: String,
    /// Each provider's reply to the single-node send. Reports only.
    pub providers: Vec<(String, SendOutcome)>,
    /// The replicated read-back straight after. `NotYet` or `Pending` here is normal; ask [`evm_tx_status`] later.
    pub seen: EvmSeen,
}

/// Send signed bytes to every configured provider, then read the chain back. Safe to repeat with the same bytes: one nonce,
/// so at most one of them is ever mined. Refuses bytes signed for a chain other than the one this canister was installed for,
/// and an RPC that is not that chain either.
async fn broadcast_evm(raw: &[u8]) -> Result<EvmSend, String> {
    let want = cfg().chain_id;
    let signed_for = chain_id_of(raw)?;
    if signed_for != want {
        return Err(format!("refused: these bytes are signed for chain {signed_for}; this canister sends only for chain {want}"));
    }
    assert_chain().await?;
    let tx_hash = format!("0x{}", hex::encode(keccak(raw)));
    let raw_hex = format!("0x{}", hex::encode(raw));
    let mut providers = Vec::new();
    for url in &cfg().rpc_urls {
        providers.push((url.clone(), send_raw_once(url, &raw_hex, &tx_hash).await));
    }
    let seen = match pin().await { Ok(p) => evm_seen(&tx_hash, p.number).await, Err(e) => EvmSeen::Unsure(e) };
    Ok(EvmSend { tx_hash, providers, seen })
}

/// The rehearsal's broadcast: the same [`broadcast_evm`] a real buyer uses, behind [`rehearse_sign_evm`]'s guards (installed
/// for the testnet, and the caller names it again) plus one more: the bytes themselves must be signed for the testnet.
#[ic_cdk::update]
pub async fn rehearse_broadcast_evm(chain_id: u64, raw_hex: String) -> Result<EvmSend, String> {
    only_operator();
    not_paused()?;
    check_evm_rehearsal_chain(cfg().chain_id, chain_id)?;
    let raw = hex::decode(raw_hex.trim().trim_start_matches("0x")).map_err(|e| format!("raw transaction is not hex: {e}"))?;
    broadcast_evm(&raw).await
}

/// Ask the chain again about a transaction this canister sent, replicated. Controller only: each call spends cycles.
#[ic_cdk::update]
pub async fn evm_tx_status(tx_hash: String) -> EvmSeen {
    only_operator();
    let h = tx_hash.trim().to_ascii_lowercase();
    if h.len() != 66 || !h.starts_with("0x") || hex::decode(&h[2..]).is_err() {
        return EvmSeen::Unsure("a transaction hash is 0x and 64 hex digits".into());
    }
    if let Err(e) = assert_chain().await {
        return EvmSeen::Unsure(e);
    }
    match pin().await {
        Ok(p) => evm_seen(&h, p.number).await,
        Err(e) => EvmSeen::Unsure(e),
    }
}

/// What a pinned read saw: the agreed block, and two state reads named by its hash.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PinnedRead {
    pub pin: u64,
    pub hash: String,
    pub base_fee: String,
    /// `eth_call(to, data)` at the pin, every provider agreeing.
    pub call: String,
    /// `eth_getBalance(to)` at the pin, every provider agreeing.
    pub balance: String,
}

/// Diagnostic, read-only: take a pin exactly as the loop does and read `to` at it — an `eth_call` with `data` and its balance.
/// It sends nothing and signs nothing; it is how the pinned path is shown working against a chain before the loop relies on it.
#[ic_cdk::update]
pub async fn pinned_read(to: String, data_hex: String) -> Result<PinnedRead, String> {
    only_operator();
    let to = crate::evmtx::parse_address(to.trim())?;
    let data = hex::decode(data_hex.trim().trim_start_matches("0x")).map_err(|e| format!("data is not hex: {e}"))?;
    assert_chain().await?;
    let p = pin().await?;
    let call = h0x(&eth_call(to, &data, &p).await?);
    let balance = rpc("eth_getBalance", serde_json::json!([h0x(&to), crate::pin::at(&p)])).await?
        .as_str().ok_or("balance is not a quantity")?.to_string();
    Ok(PinnedRead { pin: p.number, hash: p.hash, base_fee: p.base_fee.to_string(), call, balance })
}

/// Guard 2 for the tick, home-aware (phase B2): on the EVM home [`assert_chain`]; on the Solana home, every configured Solana provider
/// agrees the cluster's genesis hash is the installed cluster's (`solrpc::genesis_of`), the Solana form of `eth_chainId`.
pub(crate) async fn assert_home() -> Result<(), String> {
    let Some(h) = cfg().home else { return assert_chain().await };
    let Home::Solana { cluster, .. } = &h;
    let want = crate::solrpc::genesis_of(cluster).ok_or_else(|| format!("refused: no genesis hash is pinned for cluster {cluster:?}"))?;
    let got = crate::solhome::SolNet::new(&h)?.genesis().await?;
    if got != want {
        return Err(format!("refused: these Solana providers serve the cluster whose genesis is {got}, not {cluster} ({want}) we were installed for"));
    }
    Ok(())
}

/// Guard 2: the chain we are reading is the chain we were installed for.
async fn assert_chain() -> Result<(), String> {
    let got = rpc("eth_chainId", serde_json::json!([])).await?;
    let hex = got.as_str().ok_or("chainId not a string")?;
    let id = u64::from_str_radix(hex.trim_start_matches("0x"), 16).map_err(|e| e.to_string())?;
    let want = cfg().chain_id;
    if id != want {
        return Err(format!("refused: this RPC is chain {id}, not the {want} we were installed for"));
    }
    Ok(())
}

// ---- the loop --------------------------------------------------------------------------------------------------------------

// The loop itself is `looper.rs`, written against the `Io` trait and tested there against a fake world. This is the real
// world: replicated outcalls, the threshold keys and the canister's own memory, translated and nothing decided
// (`docs/v2/buyer-loop.md`).

thread_local! {
    /// Our EVM address and taproot script, derived once: a key's public half does not change.
    static OURS: RefCell<Option<([u8; 20], Vec<u8>)>> = const { RefCell::new(None) };
    /// Rehearsal fixtures for the venue and the bridge (`rehearse_fixtures`). Never on an install for chain 4663.
    static FIXTURES: RefCell<bool> = const { RefCell::new(false) };
}

pub(crate) fn hex_bytes(v: &serde_json::Value) -> Result<Vec<u8>, String> {
    let s = v.as_str().ok_or("not a hex string")?;
    hex::decode(s.trim_start_matches("0x")).map_err(|e| format!("not hex: {e}"))
}

pub(crate) fn hex_u128(v: &serde_json::Value) -> Result<u128, String> {
    let s = v.as_str().ok_or("not a hex quantity")?;
    u128::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("not a quantity: {e}"))
}

pub(crate) fn h0x(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

pub(crate) async fn ours() -> Result<([u8; 20], Vec<u8>), String> {
    if let Some(o) = OURS.with(|o| o.borrow().clone()) {
        return Ok(o);
    }
    let evm = crate::evmtx::parse_address(&evm_address().await?)?;
    let script = [vec![0x51, 0x20], hex::decode(taproot_output_key().await?).map_err(|e| e.to_string())?].concat();
    OURS.with(|o| *o.borrow_mut() = Some((evm, script.clone())));
    Ok((evm, script))
}

pub(crate) async fn eth_call(to: [u8; 20], data: &[u8], pin: &crate::pin::Pin) -> Result<Vec<u8>, String> {
    hex_bytes(&rpc("eth_call", serde_json::json!([{ "to": h0x(&to), "data": h0x(data) }, crate::pin::at(pin)])).await?)
}

/// One method, asked of every provider separately, each answer taken through `pick`; the highest is used. For the numbers
/// where a higher value is the safe side (a fee cap, a gas limit), so providers need not agree to the unit.
async fn rpc_max(method: &str, params: serde_json::Value, pick: fn(&serde_json::Value) -> Result<u128, String>)
    -> Result<u128, String> {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let mut best = 0u128;
    for url in &cfg().rpc_urls {
        best = best.max(pick(&rpc_once(url, body.clone()).await?)?);
    }
    Ok(best)
}

/// Our confirmed sats, from every Bitcoin explorer, which must agree. Read by script hash (esplora's `/scripthash/`), so no
/// address encoding is involved; mempool figures are stripped before consensus because they move between replicas.
async fn btc_confirmed_sats(script: &[u8]) -> Result<u64, String> {
    use sha2::{Digest, Sha256};
    let mut h: [u8; 32] = Sha256::digest(script).into();
    h.reverse();
    let mut seen: Vec<u64> = Vec::new();
    for base in &cfg().btc_api_urls {
        let req = HttpRequestArgs {
            url: format!("{base}/scripthash/{}", hex::encode(h)),
            method: HttpMethod::GET,
            body: None,
            max_response_bytes: Some(4_000),
            transform: Some(TransformContext {
                function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: "chain_stats_only".into() }),
                context: vec![],
            }),
            headers: vec![],
            is_replicated: Some(true),
        };
        let res = http_request(&req).await.map_err(|e| format!("{e:?}"))?;
        let v: serde_json::Value = serde_json::from_slice(&res.body).map_err(|e| format!("{base}: unreadable: {e}"))?;
        let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).ok_or_else(|| format!("{base}: no {k}"));
        seen.push(n("funded")?.checked_sub(n("spent")?).ok_or("spent exceeds funded")?);
    }
    let first = *seen.first().ok_or("no Bitcoin explorers")?;
    if seen.iter().any(|x| *x != first) {
        return Err("Bitcoin explorers disagree on our balance; nothing is believed".into());
    }
    Ok(first)
}

/// Keeps only what a balance is made of, confirmed: `{"funded": n, "spent": n}`. Everything else, mempool included, moves
/// between the moments different replicas ask.
#[ic_cdk::query]
fn chain_stats_only(args: TransformArgs) -> HttpRequestResult {
    let v: serde_json::Value = serde_json::from_slice(&args.response.body).unwrap_or(serde_json::Value::Null);
    let c = v.get("chain_stats");
    let f = |k: &str| c.and_then(|c| c.get(k)).and_then(|x| x.as_u64());
    let body = match (f("funded_txo_sum"), f("spent_txo_sum")) {
        (Some(a), Some(b)) => serde_json::json!({ "funded": a, "spent": b }).to_string().into_bytes(),
        _ => b"{}".to_vec(),
    };
    HttpRequestResult { status: args.response.status, body, headers: vec![] }
}

/// The rehearsal venue and bridge. They let the EVM half of the loop run end to end on a rehearsal install; they never
/// deliver anything, and a purchase that reaches the bridge with them waits for BTC that will not come and gives up.
fn fixtures() -> (crate::venue::Fixture, crate::bridge::Fixture) {
    let q = Quote { rune: crate::runestone::RuneId { block: 1, tx: 0 }, units: 1, max_spend_sats: 0, max_fee_sats: 0,
        recipient_script: vec![], our_script: vec![] };
    (crate::venue::Fixture { quote: q, psbt: vec![], to_sign: vec![] },
     crate::bridge::Fixture { quote: crate::bridge::BridgeQuote { send_wei: 0, expect_sats: 0, to: [0; 20], calldata: vec![],
        recipient_script: vec![] } })
}

pub(crate) struct CanisterIo {
    now: u64,
    chain_id: u64,
    vault: [u8; 20],
    venue: Option<crate::venue::Fixture>,
    bridge: Option<crate::bridge::Fixture>,
    /// On a real install, the venue is what the relayer handed in for the unfinished purchase (`relay.rs`).
    relayed: crate::relay::Relayed,
    /// This tick's pin, taken once at the first read: every read in one tick is of one block.
    pinned: Option<crate::pin::Pin>,
}

impl CanisterIo {
    pub(crate) async fn pin(&mut self) -> Result<crate::pin::Pin, String> {
        if let Some(p) = &self.pinned {
            return Ok(p.clone());
        }
        let p = pin().await?;
        self.pinned = Some(p.clone());
        Ok(p)
    }

    pub(crate) fn new() -> Result<Self, String> {
        let c = cfg();
        let (venue, bridge) = if FIXTURES.with(|f| *f.borrow()) { let (v, b) = fixtures(); (Some(v), Some(b)) } else { (None, None) };
        let unfinished = PURCHASES.with(|ps| ps.borrow().iter().find(|p| crate::looper::can_open(std::slice::from_ref(*p)).is_err())
            .map(|p| p.id));
        Ok(CanisterIo { now: ic_cdk::api::time() / 1_000_000_000, chain_id: c.chain_id,
            vault: crate::evmtx::parse_address(&c.vault)?, venue, bridge, relayed: crate::relay::Relayed::for_purchase(unfinished),
            pinned: None })
    }
}

impl crate::looper::Io for CanisterIo {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { self.chain_id }
    fn vault(&self) -> [u8; 20] { self.vault }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { Ok(ours().await?.1) }
    async fn purchase_count(&mut self) -> Result<u64, String> {
        let pin = self.pin().await?;
        crate::evmtx::decode_count(&eth_call(self.vault, &crate::evmtx::purchase_count(), &pin).await?)
    }
    async fn vault_purchase(&mut self, id: u64) -> Result<crate::evmtx::VaultPurchase, String> {
        let pin = self.pin().await?;
        crate::evmtx::decode_purchase(&eth_call(self.vault, &crate::evmtx::purchase(id), &pin).await?)
    }
    async fn evm_balance(&mut self) -> Result<u128, String> {
        let pin = self.pin().await?;
        hex_u128(&rpc("eth_getBalance", serde_json::json!([h0x(&ours().await?.0), crate::pin::at(&pin)])).await?)
    }
    async fn evm_nonce(&mut self) -> Result<u64, String> {
        // the nonce as of the pin; the loop's own saved in-flight transaction covers everything above it
        let pin = self.pin().await?;
        let n = hex_u128(&rpc("eth_getTransactionCount", serde_json::json!([h0x(&ours().await?.0), crate::pin::at(&pin)])).await?)? as u64;
        // round 4 (C4-H-1): never a nonce below one already read (the pin floor makes this hold; this is the second line)
        PIN_FLOORS.with(|f| f.borrow_mut().home.admit_nonce("4663", n))
    }
    async fn base_fee(&mut self) -> Result<u128, String> {
        // the pinned block's, agreed by every provider; the loop caps the fee at FEE_MULTIPLE times it
        Ok(self.pin().await?.base_fee)
    }
    async fn estimate_gas(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<u64, String> {
        let from = ours().await?.0;
        let pin = self.pin().await?;
        let call = serde_json::json!([{ "from": h0x(&from), "to": h0x(&to), "value": format!("0x{value:x}"), "data": h0x(data) },
            format!("0x{:x}", pin.number)]);
        Ok(rpc_max("eth_estimateGas", call, hex_u128).await? as u64)
    }
    async fn sign_evm(&mut self, hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> {
        let s = sign_evm_recoverable(hash).await?;
        let b = |h: &str| -> Result<[u8; 32], String> { hex::decode(h).map_err(|e| e.to_string())?.try_into().map_err(|_| "not 32 bytes".into()) };
        Ok((s.recovery_id, b(&s.r)?, b(&s.s)?))
    }
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String> {
        broadcast_evm(raw).await.map(|_| ())
    }
    async fn evm_seen(&mut self, hash: [u8; 32]) -> crate::looper::EvmView {
        use crate::looper::EvmView;
        let pin = match self.pin().await { Ok(p) => p, Err(e) => return crate::looper::EvmView::Unsure(e) };
        match evm_seen(&h0x(&hash), pin.number).await {
            EvmSeen::NotYet => EvmView::NotYet,
            EvmSeen::Pending => EvmView::Pending,
            EvmSeen::Mined { success, .. } => EvmView::Mined { success },
            EvmSeen::Unsure(e) => EvmView::Unsure(e),
        }
    }
    fn save(&mut self, p: &Purchase) {
        PURCHASES.with(|ps| {
            let mut ps = ps.borrow_mut();
            match ps.iter_mut().find(|x| x.id == p.id) {
                Some(x) => *x = p.clone(),
                None => ps.push(p.clone()),
            }
        });
    }
    async fn btc_sats(&mut self) -> Result<u64, String> {
        btc_confirmed_sats(&ours().await?.1).await
    }
    async fn btc_seen(&mut self, txid: [u8; 32]) -> crate::looper::BtcView {
        use crate::btcnet::BtcSeen;
        use crate::looper::BtcView;
        // the loop carries txids in display order (psbt::txid); the Bitcoin module takes internal order
        let mut internal = txid;
        internal.reverse();
        match crate::btcnet::btc_tx_seen(internal).await {
            BtcSeen::NotYet => BtcView::NotYet,
            BtcSeen::Mempool => BtcView::Mempool,
            BtcSeen::Confirmed { .. } => BtcView::Confirmed,
            BtcSeen::Unsure(e) => BtcView::Unsure(e),
        }
    }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        // `MissingInputs` and every other reply are reports; only `btc_seen` is believed
        crate::btcnet::broadcast_btc(raw).await.map(|_| ())
    }
    async fn delivered_units(&mut self, txid: [u8; 32], quote: &Quote) -> Result<u128, String> {
        crate::oracle::delivered(txid, quote).await // docs/v2/rates-and-indexer.md §2
    }
    async fn sign_taproot(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        sign_taproot(sighash.to_vec()).await
    }
    async fn input_clean(&mut self, o: crate::custody::Outpoint) -> Result<(), String> {
        crate::custodynet::buyer_input_clean(o).await
    }
    fn venue(&self) -> Option<&dyn crate::venue::Venue> {
        match &self.venue {
            Some(f) => Some(f as &dyn crate::venue::Venue),
            None => Some(&self.relayed as &dyn crate::venue::Venue),
        }
    }
    fn bridge(&self) -> Option<&dyn crate::bridge::Bridge> { self.bridge.as_ref().map(|b| b as &dyn crate::bridge::Bridge) }
    fn bridge_allowlist(&self) -> Vec<[u8; 20]> {
        cfg().bridge_allowlist.iter().filter_map(|a| crate::evmtx::parse_address(a).ok()).collect()
    }
    async fn reference_floor_sats(&mut self, send_wei: u128) -> Result<u64, String> {
        crate::oracle::reference_floor(send_wei).await // docs/v2/rates-and-indexer.md §1
    }
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
        // the policy's and the listing's reads are pinned like every other: one tick, one agreed block (docs/v2/pinned-reads.md)
        let p = self.pin().await?;
        eth_call(to, data, &p).await
    }
    fn btc_hrp(&self) -> Result<&'static str, String> {
        crate::policy::hrp_for(&cfg().btc_network)
    }
    /// `docs/v2/trust-points.md` §1: the cap on a quote's spend, by the same live rate as the bridge judge's floor. If the rate
    /// cannot be read (fewer than three sources, or too wide a spread), no quote is recorded and nothing is signed: fail closed.
    async fn reference_cap_sats(&mut self, wei: u128) -> Result<u64, String> {
        crate::oracle::reference_cap(wei).await
    }
}

/// Stops two ticks running through each other, and is released even if the callback that took it traps.
///
/// DFINITY's own guidance on inter-canister calls is the source: *"messages from interleaving calls do not have a reliable
/// execution ordering"*, which is a time-of-check-to-time-of-use hazard, and *"traps and panics roll back the canister state"*
/// only to the snapshot before the **callback**, not before the whole call. `tick` reads a purchase, `await`s two chains, and
/// then decides what to do about what it read — so a second `tick` entering during that await reads the same pre-act state and
/// reaches the same decision, and both act on it. For a state machine whose entire safety argument is "nothing that spends is
/// attempted until the intent to spend is durably recorded", two deciders on one record is the shape of a double spend.
///
/// The flag is a `Drop` guard rather than a boolean set and cleared by hand, because a hand-cleared boolean leaks on a trap:
/// the canister would be wedged, permanently, by one failed outcall. `Drop` runs through `ic0.call_on_cleanup`.
#[derive(Debug)]
pub(crate) struct TickGuard;
impl Drop for TickGuard {
    fn drop(&mut self) {
        TICKING.with(|t| *t.borrow_mut() = false);
    }
}

/// L-4 (audit D): a tick holds a copy of `FROGS`, `FLOORS`, `HANDINS`, `SALES` and `CUSTODY` across its awaits and writes it
/// back at the end, so a change made to them while it runs would be silently lost. Every update method that writes them calls
/// this first; the caller asks again once the tick is over (`every_method_that_writes_ticked_state_refuses_mid_tick`).
pub(crate) fn not_ticking() -> Result<(), String> {
    if TICKING.with(|t| *t.borrow()) {
        return Err("refused: a tick is running and would overwrite this change; ask again when it is over".into());
    }
    Ok(())
}

/// For an update method that writes ticked state across its own awaits (L-4): it holds the tick's flag, so no tick starts
/// meanwhile, and refuses if one is running.
pub(crate) fn hold_tick() -> Result<TickGuard, String> {
    begin_tick()
}

fn begin_tick() -> Result<TickGuard, String> {
    TICKING.with(|t| {
        let mut t = t.borrow_mut();
        if *t {
            return Err("a tick is already running: this one does nothing rather than decide twice from one reading".into());
        }
        *t = true;
        Ok(TickGuard)
    })
}

/// One pass: for each purchase, ask the chains, ask the state machine, and do the one thing it says (`looper::step`).
#[ic_cdk::update]
pub async fn tick() -> Result<Vec<String>, String> {
    only_operator();
    let _tick = begin_tick()?;
    if PAUSED.with(|p| *p.borrow()) {
        // audit R2, M-1 (c): nothing new leaves while paused, but custody keeps protecting what already left (a delivery in
        // flight is conflicted out when the rule refuses it, a confirmed one is recorded): `redeem::tick`'s paused pass
        let mut lines = vec!["paused by the owner's key: the buyer does nothing new".to_string()];
        // audit R3, R3-C3: the restricted pass acts on chain reads too (it records confirmed deliveries), so the RPC must be the
        // chain this canister was installed for, as in the unpaused tick (phase B2: on the Solana home, its cluster's genesis hash)
        if let Err(e) = assert_home().await {
            lines.push(format!("custody: the paused pass does not run: {e}"));
            return Ok(lines);
        }
        if let Some(w) = crate::frognet::frog_wrapper() {
            let pending = PURCHASES.with(|ps| ps.borrow().iter().any(|x| x.pending_evm.is_some())) || crate::frognet::evm_pending()
                || crate::ethnet::evm_pending();
            for line in crate::custodynet::tick(&w, pending).await {
                if !line.contains(": waits: ") {
                    note(line.clone());
                }
                lines.push(line);
            }
        }
        // audit 2026-09-27-nft B-L-4 / C-L-1: the Ethereum NFT side's paused pass (`ethflow::tick` while paused): the line is read,
        // a final delivery recorded, a delivery in its last day cancelled; nothing new is signed
        if cfg().eth_nft.is_some() {
            let others = PURCHASES.with(|ps| ps.borrow().iter().any(|x| x.pending_evm.is_some())) || crate::custodynet::evm_pending()
                || crate::frognet::evm_pending();
            for line in crate::ethnet::tick(others).await {
                if !line.contains("waits") {
                    note(line.clone());
                }
                lines.push(line);
            }
        }
        return Ok(lines);
    }
    assert_home().await?;
    let mut io = CanisterIo::new()?;
    let mut lines = Vec::new();
    // The purchase policy (docs/v2/venue-bridge-policy.md §1): with nothing unfinished, open what the vault's own state says is
    // owed. Its reasons for waiting are reported, not noted, so an idle buyer does not fill the log. Phase B2: the AutoBuyerVault is the
    // EVM home's (docs/v9 §4: the v2 vault buyer is not carried to Solana): nothing is opened on the Solana home.
    let all = PURCHASES.with(|p| p.borrow().clone());
    let opened = if cfg().home.is_some() { Ok(None) } else { crate::looper::maybe_open(&mut io, &all).await };
    match opened {
        Ok(Some(p)) => {
            crate::relay::clear();
            crate::looper::Io::save(&mut io, &p);
            let line = format!("policy opened purchase {}: {} wei for {} units", p.id, p.amount_wei, p.order.as_ref().map_or(0, |o| o.units));
            note(line.clone());
            lines.push(line);
            io = CanisterIo::new()?; // the venue now serves the new purchase
        }
        Ok(None) => {}
        Err(e) => lines.push(format!("policy: waits: {e}")),
    }
    let ids: Vec<u64> = PURCHASES.with(|p| p.borrow().iter().map(|x| x.id).collect());
    for id in ids {
        if PAUSED.with(|p| *p.borrow()) {
            lines.push("paused mid-tick: stopping here".into());
            break;
        }
        let Some(mut p) = PURCHASES.with(|ps| ps.borrow().iter().find(|x| x.id == id).cloned()) else { continue };
        let other = PURCHASES.with(|ps| ps.borrow().iter().any(|x| x.id != id && x.pending_evm.is_some())) || crate::custodynet::evm_pending()
            || crate::frognet::evm_pending() || crate::ethnet::evm_pending();
        let line = crate::looper::step(&mut io, &mut p, other).await;
        crate::looper::Io::save(&mut io, &p);
        if !line.contains(": waits (") {
            note(line.clone());
        }
        lines.push(line);
    }
    // The custodian (docs/v3/wrapper.md, "Step 3, the flow"): redeemed frogs, after the purchases, one EVM transaction of ours
    // at a time across both.
    if let (Some(w), false) = (crate::frognet::frog_wrapper(), PAUSED.with(|p| *p.borrow())) {
        let purchases_pending = PURCHASES.with(|ps| ps.borrow().iter().any(|x| x.pending_evm.is_some()));
        for line in crate::custodynet::tick(&w, purchases_pending || crate::frognet::evm_pending() || crate::ethnet::evm_pending()).await {
            if !line.contains(": waits: ") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    // Item sinks, the mint desk and holder deposits (docs/v3/curve.md, "Canister: …"): after the custodian, one EVM
    // transaction of ours at a time across all of it.
    if let (Some(w), false) = (crate::frognet::frog_wrapper(), PAUSED.with(|p| *p.borrow())) {
        let others = PURCHASES.with(|ps| ps.borrow().iter().any(|x| x.pending_evm.is_some())) || crate::custodynet::evm_pending()
            || crate::ethnet::evm_pending();
        for line in crate::frognet::tick(&w, others).await {
            if !line.contains("waits") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    // The Ethereum NFT side (docs/v7/phase1-nft.md): after the rest, one home-chain transaction of ours at a time across all of it.
    if cfg().eth_nft.is_some() && !PAUSED.with(|p| *p.borrow()) {
        let others = PURCHASES.with(|ps| ps.borrow().iter().any(|x| x.pending_evm.is_some())) || crate::custodynet::evm_pending()
            || crate::frognet::evm_pending();
        for line in crate::ethnet::tick(others).await {
            if !line.contains("waits") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    // W3: the four Solana send lines, once each in fair order (the canister's own timer does the same between ticks: soltimer.rs)
    if cfg().home.is_some() && !PAUSED.with(|p| *p.borrow()) {
        for line in crate::soltimer::lines_pass().await.0 {
            if !line.contains("waits") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    // W5: the intent desk (deposits found, mints, burns, redemptions and refunds on its own line), after the lines have landed
    if cfg().home.is_some() && !PAUSED.with(|p| *p.borrow()) {
        for line in crate::desk::api::tick().await {
            if !line.contains("waits") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    // W6: the coins (openings, graduations, fee claims and cashes, coin redemptions) on the crank line, only while it is free
    if cfg().home.is_some() && !PAUSED.with(|p| *p.borrow()) {
        for line in crate::coins::api::tick().await {
            if !line.contains("waits") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    // W7: the pool desk (opening, deposits, arbs, re-centres, fee claims, sweeps) on the pool line
    if cfg().home.is_some() && !PAUSED.with(|p| *p.borrow()) {
        for line in crate::pooldesk::api::tick().await {
            if !line.contains("waits") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    // W8c: the Solana NFT flow (the floor from Magic Eden's listings verified on chain, the advance + purchase, deliveries) on the money line
    if cfg().home.is_some() && !PAUSED.with(|p| *p.borrow()) {
        for line in crate::solnftflow::api::tick_ic().await {
            if !line.contains("waits") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    // W9: the treasury's cranks (the draws, the raffles, the distributions), LAST, on the crank line (W3: one a tick)
    if cfg().home.is_some() && !PAUSED.with(|p| *p.borrow()) {
        for line in crate::cranks::tick_ic().await {
            if !line.contains("waits") {
                note(line.clone());
            }
            lines.push(line);
        }
    }
    Ok(lines)
}

/// Open a purchase for the loop: what to buy, for whom, and at most how much ETH. The id is the vault's next one, read now;
/// nothing is announced here -- the next `tick` does that, and only with a venue and a bridge configured. Refused while any
/// other purchase is unfinished (one at a time) and while no venue or bridge is configured.
#[ic_cdk::update]
pub async fn open_purchase(amount_wei: candid::Nat, rune: String, units: candid::Nat, recipient_script_hex: String,
    cap_ref: String) -> Result<u64, String> {
    only_operator();
    not_paused()?;
    // Where the money is, only the policy opens purchases (docs/v2/venue-bridge-policy.md §1): the operator has no door.
    if cfg().chain_id == PRODUCTION_EVM_CHAIN {
        return Err(format!("refused: on chain {PRODUCTION_EVM_CHAIN} purchases are opened by the policy only"));
    }
    let _tick = begin_tick()?; // no tick may decide while a purchase is being added
    let n = |x: &candid::Nat, what: &str| -> Result<u128, String> { x.0.to_string().parse::<u128>().map_err(|_| format!("{what} too large")) };
    let (amount_wei, units) = (n(&amount_wei, "amount_wei")?, n(&units, "units")?);
    if amount_wei == 0 || units == 0 {
        return Err("refused: a purchase of nothing".into());
    }
    crate::looper::parse_rune(&rune)?;
    let recipient_script = hex::decode(recipient_script_hex.trim()).map_err(|e| format!("recipient script is not hex: {e}"))?;
    if recipient_script.is_empty() {
        return Err("refused: no recipient".into());
    }
    PURCHASES.with(|ps| crate::looper::can_open(&ps.borrow()))?;
    let mut io = CanisterIo::new()?;
    crate::looper::configured(&io)?;
    assert_chain().await?;
    // docs/v2/trust-points.md §1: the rune and the recipient are the listing's, read from chain, not the operator's choice
    let listing = crate::looper::read_listing(&mut io).await?;
    crate::listing::check_order(&rune, &recipient_script, &listing, &io.vault, crate::policy::hrp_for(&cfg().btc_network)?)?;
    let id = crate::looper::Io::purchase_count(&mut io).await?;
    PURCHASES.with(|ps| crate::looper::can_open(&ps.borrow()))?; // again: the read was an await
    let p = Purchase { id, phase: crate::state::Phase::Watching, amount_wei,
        order: Some(crate::state::Order { rune, units, recipient_script, cap_ref }), pending_evm: None };
    crate::looper::Io::save(&mut io, &p);
    note(format!("purchase {id} opened: {amount_wei} wei for {units} units"));
    Ok(id)
}

/// The relayer hands in the venue's quote for the unfinished purchase (`relay.rs`). It is not recorded here: the loop checks it
/// against the purchase's order and our holdings, and `record_quote`'s checks, before anything is recorded.
#[ic_cdk::update]
pub fn relay_venue_quote(purchase_id: u64, quote: Quote) -> Result<String, String> {
    only_operator();
    not_paused()?;
    with_purchase(purchase_id, |p| match p.phase {
        crate::state::Phase::Funded | crate::state::Phase::Pulled { .. } | crate::state::Phase::Bridging { .. } => Ok(()),
        ref other => Err(format!("refused: purchase {purchase_id} is {other:?}; a quote is handed in only before it is recorded")),
    })?;
    crate::relay::put_quote(purchase_id, quote);
    Ok(format!("quote held for purchase {purchase_id}"))
}

/// The relayer hands in the venue's PSBT and the inputs it asks us to sign. Judged by `psbt.rs` against the RECORDED quote when
/// the loop reaches it; a PSBT for anything else is refused there.
#[ic_cdk::update]
pub fn relay_venue_psbt(purchase_id: u64, psbt_hex: String, to_sign: Vec<u32>) -> Result<String, String> {
    only_operator();
    not_paused()?;
    let psbt = hex::decode(psbt_hex.trim()).map_err(|e| format!("the psbt is not hex: {e}"))?;
    Psbt::parse(&psbt).map_err(|e| format!("refused: the psbt does not parse: {e:?}"))?;
    with_purchase(purchase_id, |p| match p.phase {
        crate::state::Phase::Quoted { .. } => Ok(()),
        ref other => Err(format!("refused: purchase {purchase_id} is {other:?}; a PSBT is handed in only once its quote is recorded")),
    })?;
    crate::relay::put_psbt(purchase_id, psbt, to_sign.into_iter().map(|i| i as usize).collect());
    Ok(format!("psbt held for purchase {purchase_id}"))
}

/// Give a purchase up before any BTC is spent: its ETH goes back to the vault (or, never pulled, the announcement is
/// cancelled) on the next ticks. Refused once it has bridged, and while one of its EVM transactions is out.
#[ic_cdk::update]
pub fn abandon(id: u64) -> Result<String, String> {
    only_operator();
    with_purchase(id, |p| {
        if p.pending_evm.is_some() {
            return Err(format!("refused: purchase {id} has a transaction out; ask again once it resolves"));
        }
        match p.phase {
            crate::state::Phase::Announced { .. } | crate::state::Phase::Pulled { .. } => {
                p.phase = crate::state::Phase::Returning;
                Ok(format!("purchase {id} returning"))
            }
            ref other => Err(format!("refused: purchase {id} is {other:?}; only an announced or pulled purchase is given up here")),
        }
    })
}

/// Turn the rehearsal venue and bridge on or off. Only on an install that is NOT for chain 4663 and is for the rehearsal
/// Bitcoin network: they exist so the EVM half of the loop can run end to end against a local chain, never where money is.
#[ic_cdk::update]
pub fn rehearse_fixtures(on: bool) -> Result<String, String> {
    only_operator();
    not_paused()?;
    let c = cfg();
    if !fixtures_allowed(c.chain_id, &c.btc_network) {
        return Err(format!("refused: rehearsal fixtures exist only off chain {PRODUCTION_EVM_CHAIN} and on {REHEARSAL_NETWORK:?}"));
    }
    FIXTURES.with(|f| *f.borrow_mut() = on);
    note(format!("rehearsal fixtures {}", if on { "on" } else { "off" }));
    Ok(format!("rehearsal fixtures {}", if on { "on" } else { "off" }))
}

/// Whether the rehearsal venue and bridge may be switched on: never on chain 4663, and only on [`REHEARSAL_NETWORK`] (so never on
/// Bitcoin mainnet).
fn fixtures_allowed(chain_id: u64, btc_network: &str) -> bool {
    chain_id != PRODUCTION_EVM_CHAIN && btc_network == REHEARSAL_NETWORK
}

/// The network half of every Bitcoin rehearsal guard (`rehearse_sign_sighash`, `rehearse_broadcast_btc`): the install AND the
/// call must both name [`REHEARSAL_NETWORK`]. On an install for Bitcoin mainnet the first half refuses whatever the caller says.
pub(crate) fn check_btc_rehearsal(installed: &str, asked: &str, what: &str) -> Result<(), String> {
    if installed != REHEARSAL_NETWORK {
        return Err(format!("refused: this canister was installed for {installed:?}; the rehearsal {what} only for {REHEARSAL_NETWORK:?}"));
    }
    if asked != REHEARSAL_NETWORK {
        return Err(format!(
            "refused: caller asked to {what} for {asked:?}. The only accepted value is {REHEARSAL_NETWORK:?}, and it is an \
             allowlist of one on purpose -- 'anything but mainnet' would accept a typo."
        ));
    }
    Ok(())
}

/// The owner's veto key stops the buyer. It cannot move funds, redirect them, or change what the buyer would do next — only
/// whether it acts at all.
#[ic_cdk::update]
pub fn set_paused(p: bool) {
    // Anyone trusted may stop it; only a controller may start it again. After the handover that controller is the timelock,
    // which forwards only the guardian's `pause_target`, so an operator cannot lift the guardian's veto.
    if p {
        only_operator();
    } else {
        only_controller();
    }
    PAUSED.with(|x| *x.borrow_mut() = p);
    // H3: the pause freezes custody's outflow, and only a controller's resume (after the handover, k guardians through the
    // timelock's `resume_target`) lifts it -- a tripwire freeze included
    let now = ic_cdk::api::time() / 1_000_000_000;
    OUTFLOW.with(|o| {
        let mut o = o.borrow_mut();
        if p { o.freeze("paused", now) } else { o.unfreeze(true).expect("the resume branch is a controller's") }
    });
    note(if p { "paused" } else { "resumed" });
}

/// Ask the outbound limiter (H3) whether one act may go out now. A trip pauses the buyer. Every outbound hook calls this before
/// it signs, then [`outflow_record`] once the signed bytes are saved.
pub(crate) fn outflow_check(key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) -> Result<(), String> {
    let l = limits_of(&cfg());
    let now = ic_cdk::api::time() / 1_000_000_000;
    match OUTFLOW.with(|o| o.borrow_mut().check(&l, key, collection, flow, amount, now)) {
        crate::outflow::Verdict::Allow => Ok(()),
        crate::outflow::Verdict::Wait { why, until } => Err(format!("waits (outflow limit) until {until}: {why}")),
        crate::outflow::Verdict::Frozen(cause) => {
            if !PAUSED.with(|x| *x.borrow()) {
                PAUSED.with(|x| *x.borrow_mut() = true);
                note(format!("CUSTODY FROZEN: {cause:?}; the buyer is paused until k guardians resume it"));
            }
            Err(format!("refused: custody is frozen: {cause:?}"))
        }
    }
}

/// Count an act that went out (H3). Idempotent per key.
pub(crate) fn outflow_record(key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) {
    let now = ic_cdk::api::time() / 1_000_000_000;
    OUTFLOW.with(|o| o.borrow_mut().record(key, collection, flow, amount, now));
}

/// Run `f` on the caps in force and the book (H2).
pub(crate) fn with_caps<R>(f: impl FnOnce(&crate::custodycap::CapPolicy, &mut crate::custodycap::Book) -> R) -> R {
    let p = caps_of(&cfg());
    // audit C1-02: the Solana NFT flow's items count in every flow's global checks (added for the call, never kept: the books hold them)
    let extra = crate::solnftflow::cap_entries(None);
    CUSTODY_BOOK.with(|b| {
        let mut b = b.borrow_mut();
        b.entries.extend(extra);
        let r = f(&p, &mut b);
        b.entries.retain(|e| !e.id.starts_with(crate::solnftflow::CAP_ID_PREFIX));
        r
    })
}

/// What custody may hold and holds, for anyone (the proof-of-reserves page reads it): the caps in force, and per collection
/// what is held, in flight, and left.
#[ic_cdk::query]
pub fn custody_caps() -> (crate::custodycap::CapPolicy, Vec<crate::custodycap::Headroom>) {
    let p = caps_of(&cfg());
    let h = with_caps(|_, b| b.headroom(&p));
    (p, h)
}

/// The outbound limits in force and the limiter (what went out and asked to in the last day, and whether custody is frozen),
/// for anyone: the watcher alerts on a freeze.
#[ic_cdk::query]
pub fn custody_outflow() -> (crate::outflow::Limits, crate::outflow::Outflow) {
    (limits_of(&cfg()), OUTFLOW.with(|o| o.borrow().clone()))
}

/// Whether the veto is on. There was no way to read this: an operator who wanted to know whether the buyer was paused had
/// to infer it from the last "paused"/"resumed" line in a log that holds only the most recent 500 entries.
/// Below this many cycles the watcher alerts (`docs/v2/trust-points.md` §2): about two days of a loop that makes one replicated
/// read a minute on a 13-node subnet (`docs/deploy/canister-mainnet.md`, "Cycles").
pub const LOW_CYCLES: u128 = 5_000_000_000_000;

#[derive(CandidType, Deserialize)]
pub struct Cycles {
    pub balance: candid::Nat,
    pub low_threshold: candid::Nat,
}

/// The buyer's balance, for anyone. After the handover its only controller is the timelock, so `canister_status` answers
/// nobody else; this is how a holder, or the watcher, sees it running low. Anyone may top it up.
#[ic_cdk::query]
pub fn cycles() -> Cycles {
    Cycles { balance: ic_cdk::api::canister_cycle_balance().into(), low_threshold: LOW_CYCLES.into() }
}

#[ic_cdk::query]
pub fn paused() -> bool {
    PAUSED.with(|p| *p.borrow())
}

#[ic_cdk::query]
pub fn log() -> Vec<String> {
    LOG.with(|l| l.borrow().clone())
}

/// What the buyer believes, for anyone watching. Belief is a cache of two public chains, so publishing it costs nothing.
#[ic_cdk::query]
pub fn purchases() -> Vec<(u64, String)> {
    PURCHASES.with(|p| p.borrow().iter().map(|x| (x.id, format!("{:?}", x.phase))).collect())
}

/// Sign a taproot key-path spend with the threshold key. `merkle_root` is empty for a key-path-only output, which is what the
/// buyer uses: there is no script path, so there is no script anyone could be tricked into taking.
pub async fn sign_taproot(sighash: Vec<u8>) -> Result<Vec<u8>, String> {
    let res = sign_with_schnorr(&SignWithSchnorrArgs {
        message: sighash,
        derivation_path: vec![b"btc".to_vec()],
        key_id: schnorr_key(),
        // A key-path-only output has no script tree, so the merkle root is empty. The subnet applies BIP341's tweak itself:
        // the spike proved the resulting signature verifies under the tweaked output key and NOT under the internal one.
        aux: Some(SchnorrAux::Bip341(Bip341 { merkle_root_hash: vec![] })),
    })
    .await
    .map_err(|e| format!("{e:?}"))?;
    Ok(res.signature)
}

/// The signet rehearsal's one entry point: sign a 32-byte sighash with the buyer's own taproot key, through
/// [`sign_taproot`] and therefore through exactly the path a real purchase would take.
///
/// This is a blind signing oracle and it is named like one. It exists so that a Bitcoin node -- not a test in this
/// process -- can be the thing that accepts a signature from this key. Three guards, none of which is a boolean default:
///
/// 1. the install must have named [`REHEARSAL_NETWORK`], which `init` already restricted to [`KNOWN_NETWORKS`];
/// 2. the caller must name the network again, in the call, and it must be the same one. A caller that does not know which
///    network it is building for does not get a signature;
/// 3. the sighash must be exactly 32 bytes of hex.
///
/// What none of them can do is tell where the resulting 64 bytes are broadcast. The canister signs a hash; it never sees a
/// network. That limit is real and is written down in `docs/v2/2026-09-22-signet-rehearsal.md` rather than glossed.
#[ic_cdk::update]
pub async fn rehearse_sign_sighash(network: String, sighash_hex: String) -> Result<String, String> {
    only_operator();
    not_paused()?;
    check_btc_rehearsal(&cfg().btc_network, &network, "signs")?;
    let sighash = hex::decode(sighash_hex.trim()).map_err(|e| format!("sighash is not hex: {e}"))?;
    if sighash.len() != 32 {
        return Err(format!("a BIP341 sighash is 32 bytes; got {}", sighash.len()));
    }
    Ok(hex::encode(sign_taproot(sighash).await?))
}

/// Sign one of our inputs in a purchase — **after** reading the transaction, not instead of reading it.
///
/// This is the closing half of `docs/security/audit-btc-v1.md` B3. Until now the only way to get a signature out of this
/// canister was [`rehearse_sign_sighash`], which takes thirty-two bytes and signs them: the digest was computed in
/// JavaScript, on the operator's machine, from a transaction the canister never saw, while `psbt.rs`'s 716 lines of checks
/// sat with no non-test caller. The security argument and the signature were in different places.
///
/// Here they are the same act. The PSBT is parsed, verified against the quote, and the digest is computed **from the bytes
/// that were verified** — and because BIP341 commits to every input's amount and script and to every output, a digest
/// derived that way cannot describe a different transaction from the one the checks passed. A venue that lies about what
/// our own input is worth produces a signature that is invalid for the real UTXO rather than a valid one for a transaction
/// we did not agree to.
///
/// The quote is the purchase's **recorded** one ([`record_quote`]), never an argument: it was fixed before any PSBT
/// existed, so the venue cannot influence what it is checked against (`docs/v2/state-machine.md`; the half of audit B3 that
/// was left open). And the first signature binds the purchase to its transaction: one quote pays for one transaction.
#[ic_cdk::update]
pub async fn sign_purchase(purchase_id: u64, psbt_hex: String, input_index: u32) -> Result<SignedInput, String> {
    only_operator();
    not_paused()?;
    let bytes = hex::decode(psbt_hex.trim()).map_err(|e| format!("the psbt is not hex: {e}"))?;
    let psbt = Psbt::parse(&bytes).map_err(|e| format!("refused: the psbt does not parse: {e:?}"))?;
    let txid = psbt_txid(&psbt);
    let quote = with_purchase(purchase_id, |p| p.quote_to_sign(&txid))?;
    // the recorded quote, against the chain as it is NOW: a listing paused or re-addressed since signs nothing
    check_on_chain_unless_rehearsal(purchase_id, &quote).await?;

    // every check in psbt.rs, on the path that signs
    let ours = psbt
        .verify(&quote, &[input_index as usize])
        .map_err(|e| format!("refused: {e:?}"))?;
    if !ours.contains(&(input_index as usize)) {
        return Err(format!("refused: input {input_index} is not ours to sign"));
    }
    // belt and braces: none of our inputs in this transaction carries an inscription or a rune (docs/v3/wrapper.md)
    for &i in &ours {
        crate::custodynet::buyer_input_clean(crate::custody::outpoint_of(&psbt.tx.inputs[i])).await?;
    }

    // the digest, from the bytes just verified
    let sighash = sighash::taproot_key_path(&psbt, input_index as usize)
        .map_err(|e| format!("refused: the digest cannot be computed: {e:?}"))?;

    // bound before the await: a second call in flight for a different transaction now fails `quote_to_sign`
    with_purchase(purchase_id, |p| {
        p.quote_to_sign(&txid)?;
        p.bind(txid);
        Ok(())
    })?;
    let signature = sign_taproot(sighash.to_vec()).await?;
    note(format!("purchase {purchase_id}: signed input {input_index} of {}", hex::encode(txid)));
    Ok(SignedInput {
        sighash: hex::encode(sighash),
        signature: hex::encode(signature),
        txid: hex::encode(txid),
        ours: ours.iter().map(|i| *i as u32).collect(),
    })
}

/// `docs/v2/trust-points.md` §1: a quote moves money only as the chain says — the listing's rune and address, the vault's
/// announced units, the released ETH at our reference rate. The one exception is a rehearsal purchase (no order, no vault behind
/// it, created by `rehearse_open_purchase`), and only on a canister installed for [`REHEARSAL_NETWORK`]: an upgrade cannot change
/// the network, so on any other install there is no rehearsal purchase and no exception.
async fn check_on_chain_unless_rehearsal(purchase_id: u64, quote: &Quote) -> Result<(), String> {
    let has_order = with_purchase(purchase_id, |p| Ok(p.order.is_some()))?;
    if !has_order {
        if cfg().btc_network == REHEARSAL_NETWORK {
            return Ok(());
        }
        return Err(format!("refused: purchase {purchase_id} has no order, and outside {REHEARSAL_NETWORK} every quote is checked against one"));
    }
    let mut io = CanisterIo::new()?;
    crate::looper::check_quote_on_chain(&mut io, purchase_id, quote).await
}

/// Run `f` on purchase `id`, or refuse if there is none. The buyer never invents an id.
fn with_purchase<T>(id: u64, f: impl FnOnce(&mut Purchase) -> Result<T, String>) -> Result<T, String> {
    PURCHASES.with(|ps| {
        let mut ps = ps.borrow_mut();
        let p = ps.iter_mut().find(|p| p.id == id).ok_or_else(|| format!("refused: there is no purchase {id}"))?;
        f(p)
    })
}

/// Record the quote purchase `id` will be signed against: once, and only once it is funded. Today the controller supplies
/// it; when the loop is wired, `Act::GetQuote`'s performer calls the same [`Purchase::record_quote`] with the venue's quote.
/// Either way it is fixed **before** a PSBT is asked for, and cannot be replaced after.
#[ic_cdk::update]
pub async fn record_quote(purchase_id: u64, quote: Quote) -> Result<String, String> {
    only_operator();
    not_paused()?;
    check_on_chain_unless_rehearsal(purchase_id, &quote).await?;
    with_purchase(purchase_id, |p| p.record_quote(quote))?;
    note(format!("purchase {purchase_id}: quote recorded"));
    Ok(format!("purchase {purchase_id} quoted"))
}

/// A funded purchase for the rehearsal to sign against. Real purchases are created from the vault's own ids by the loop,
/// which is not wired yet; this exists only on a canister installed for [`REHEARSAL_NETWORK`], so it cannot invent an id on
/// the network where the money is.
#[ic_cdk::update]
pub fn rehearse_open_purchase(purchase_id: u64, amount_wei: u128) -> Result<String, String> {
    only_operator();
    not_paused()?;
    let installed = cfg().btc_network;
    if installed != REHEARSAL_NETWORK {
        return Err(format!("refused: this canister was installed for {installed:?}; rehearsal purchases exist only on {REHEARSAL_NETWORK:?}"));
    }
    PURCHASES.with(|ps| {
        let mut ps = ps.borrow_mut();
        if ps.iter().any(|p| p.id == purchase_id) {
            return Err(format!("refused: purchase {purchase_id} already exists"));
        }
        ps.push(Purchase { id: purchase_id, phase: crate::state::Phase::Funded, amount_wei, order: None, pending_evm: None });
        Ok(())
    })?;
    note(format!("rehearsal purchase {purchase_id} opened, funded"));
    Ok(format!("purchase {purchase_id} funded"))
}

/// What was signed, and what it was signed for. The digest is returned so a caller can check it against its own
/// computation — and so that a disagreement is visible rather than silent.
#[derive(candid::CandidType, serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct SignedInput {
    pub sighash: String,
    pub signature: String,
    /// The transaction this input belongs to, as Bitcoin will identify it: the double-SHA of the unsigned serialisation.
    /// A key-path witness is not covered by the txid, so this is known before anything is broadcast, which is what makes a
    /// re-broadcast idempotent (`docs/v2/state-machine.md`).
    pub txid: String,
    /// Every input of ours in this transaction, so a caller signing more than one knows the whole set it must cover.
    pub ours: Vec<u32>,
}

/// The transaction's id. One implementation, in `psbt.rs`, shared with the loop.
fn psbt_txid(psbt: &Psbt) -> [u8; 32] {
    psbt.txid()
}

// ---- the runestone rehearsal: the buyer's own encoder and decoder, reachable from outside -----------------------------------

/// Encode a runestone with the canister's own [`crate::runestone::Plan`], and return the scriptPubKey as hex.
///
/// Pure: no key, no network, no state. It is exposed so that the transaction the runes rehearsal puts on signet carries
/// **the buyer's encoding** rather than a second implementation written in the driver that would only ever agree with
/// itself. `docs/v2/2026-09-22-evm-and-runes-rehearsals.md` is why.
///
/// The spec is JSON: `{"mint":"189444:15","pointer":0,"edicts":[{"id":"189444:15","amount":"1","output":0}]}`. Amounts are
/// strings because a rune amount is a `u128` and JSON numbers are not.
#[ic_cdk::query]
pub fn rehearse_runestone_encode(spec_json: String) -> Result<String, String> {
    use crate::runestone::{Edict, Plan, RuneId};
    let v: serde_json::Value = serde_json::from_str(&spec_json).map_err(|e| format!("spec is not JSON: {e}"))?;

    let id_of = |s: &serde_json::Value| -> Result<RuneId, String> {
        let s = s.as_str().ok_or("a rune id is a \"block:tx\" string")?;
        RuneId::parse(s).ok_or_else(|| format!("{s:?} is not a block:tx rune id"))
    };

    let mint = match v.get("mint") {
        None | Some(serde_json::Value::Null) => None,
        Some(m) => Some(id_of(m)?),
    };
    let pointer = match v.get("pointer") {
        None | Some(serde_json::Value::Null) => None,
        Some(p) => Some(u32::try_from(p.as_u64().ok_or("pointer must be a number")?).map_err(|_| "pointer too large")?),
    };
    let mut edicts = Vec::new();
    if let Some(list) = v.get("edicts") {
        for e in list.as_array().ok_or("edicts must be a list")? {
            let amount: u128 = e
                .get("amount")
                .and_then(|a| a.as_str())
                .ok_or("an edict amount is a decimal STRING, because a rune amount is a u128")?
                .parse()
                .map_err(|_| "amount is not a u128")?;
            let output = u32::try_from(e.get("output").and_then(|o| o.as_u64()).ok_or("output must be a number")?)
                .map_err(|_| "output too large")?;
            edicts.push(Edict { id: id_of(e.get("id").ok_or("an edict needs an id")?)?, amount, output });
        }
    }
    if mint.is_none() && edicts.is_empty() {
        return Err("refused: a runestone that neither mints nor moves anything is not worth a transaction".into());
    }
    Ok(hex::encode(Plan { edicts, pointer, mint }.script()))
}

/// Decode a transaction's output scripts with the canister's own [`crate::runestone::Runestone`], and say what it found.
///
/// The input is a JSON array of hex scriptPubKeys **in output order**, because an edict's output index only means anything
/// against that order. The rehearsal feeds this the scripts of a transaction re-read from a block, so that what is decoded
/// is what a chain holds rather than what the driver remembered.
#[ic_cdk::query]
pub fn rehearse_runestone_decode(scripts_json: String) -> Result<String, String> {
    use crate::runestone::Runestone;
    let list: Vec<String> = serde_json::from_str(&scripts_json).map_err(|e| format!("not a JSON array: {e}"))?;
    let scripts: Vec<Vec<u8>> = list
        .iter()
        .map(|h| hex::decode(h.trim_start_matches("0x")).map_err(|e| format!("{h:?} is not hex: {e}")))
        .collect::<Result<_, _>>()?;
    let r = Runestone::decode(&scripts).map_err(|e| format!("{e:?}"))?;
    Ok(serde_json::json!({
        "cenotaph": r.cenotaph,
        "pointer": r.pointer,
        "has_spread": r.has_spread(scripts.len() as u32),
        "edicts": r.edicts.iter().map(|e| serde_json::json!({
            "id": e.id.to_string(),
            "amount": e.amount.to_string(),
            "output": e.output,
        })).collect::<Vec<_>>(),
    })
    .to_string())
}

/// Sign a 4663 transaction hash with the threshold ECDSA key.
///
/// This returns what the subnet returns and nothing more: **64 bytes, `r‖s`, with no recovery parameter**. It is not enough
/// to put in a transaction. [`sign_evm_recoverable`] is what a caller wants; this stays separate so that the thing the
/// subnet actually hands back stays visible rather than folded into a helper.
pub async fn sign_evm(hash: Vec<u8>) -> Result<Vec<u8>, String> {
    let res = sign_with_ecdsa(&SignWithEcdsaArgs {
        message_hash: hash,
        derivation_path: vec![b"evm".to_vec()],
        key_id: ecdsa_key(),
    })
    .await
    .map_err(|e| format!("{e:?}"))?;
    Ok(res.signature)
}

/// A signature an EVM chain can recover a sender from. Every field is public; none is a secret.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EvmSignature {
    /// 32 bytes, hex, no `0x`.
    pub r: String,
    /// 32 bytes, hex. **After** low-`s` normalisation, which is what goes in the transaction.
    pub s: String,
    /// What the subnet actually returned for `s`, before normalisation. Recorded so that "does the IC return a low `s`?"
    /// is measured on every call rather than assumed once and written in a comment.
    pub s_from_subnet: String,
    /// True when `s_from_subnet` was in the upper half of the curve order and had to be negated (EIP-2).
    pub s_was_high: bool,
    /// 0 or 1. **Derived**, by recovering the public key under each parity and keeping the one whose address is ours -- not
    /// chosen, not guessed, not defaulted.
    pub recovery_id: u8,
    /// The address the canister recovered from its own signature. A caller should check this equals [`evm_address`]; a node
    /// will check it whether the caller does or not.
    pub signer: String,
}

/// Recover the EVM address that `(r, s, recovery_id)` implies over `hash`. `None` when that combination recovers nothing.
fn recover_evm_address(hash: &[u8; 32], r: &[u8; 32], s: &[u8; 32], recovery_id: u8) -> Option<String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let sig = Signature::from_scalars(*r, *s).ok()?;
    let rid = RecoveryId::from_byte(recovery_id)?;
    let vk = VerifyingKey::recover_from_prehash(hash, &sig, rid).ok()?;
    let point = vk.to_encoded_point(false);
    Some(format!("0x{}", hex::encode(&keccak(&point.as_bytes()[1..])[12..])))
}

/// Sign an EVM transaction hash and return something a chain can actually use.
///
/// **This is the gap the signet rehearsal named.** `sign_with_ecdsa` gives back `r‖s`. An EVM transaction carries a third
/// number -- `v`, the recovery parameter -- because a chain does not know the sender's public key: it *recovers* one from
/// the signature and charges whatever account comes out. Two public keys satisfy any `(r, s)`; `v` says which. Get it wrong
/// and the transaction is perfectly well-formed and belongs to **somebody else's address**, which has no balance, no nonce
/// and nobody to complain.
///
/// Nothing in this repository could have caught that, for the same reason nothing could catch the `taproot_key()` mislabel:
/// a test that verifies a signature against the key it signed with agrees with itself under either parity. So the parity is
/// not asserted here -- it is **searched for and then checked against [`evm_address`]**, and if neither candidate
/// reproduces our own address this returns an error rather than picking one.
///
/// Two things happen in this order, and the order is itself a bug that would otherwise hide here: `s` is normalised to the
/// lower half of the curve order **first** (EIP-2 -- a chain rejects the high half, and `(r, n-s)` is an equally valid
/// signature for the same message), and the parity is searched for **afterwards**, because negating `s` flips it.
/// Normalise after choosing the parity and `v` is wrong in a way that shows up only once the signature is somewhere real.
pub async fn sign_evm_recoverable(hash: [u8; 32]) -> Result<EvmSignature, String> {
    use k256::ecdsa::Signature;

    let ours = evm_address().await?;
    let raw = sign_evm(hash.to_vec()).await?;
    if raw.len() != 64 {
        return Err(format!("sign_with_ecdsa returned {} bytes; r||s is 64", raw.len()));
    }
    let r: [u8; 32] = raw[..32].try_into().expect("32 bytes");
    let s_from_subnet: [u8; 32] = raw[32..].try_into().expect("32 bytes");

    let sig = Signature::from_scalars(r, s_from_subnet).map_err(|e| format!("not a signature: {e}"))?;
    let (sig, s_was_high) = match sig.normalize_s() {
        Some(low) => (low, true),
        None => (sig, false),
    };
    let s: [u8; 32] = sig.s().to_bytes().into();

    for recovery_id in 0u8..=1 {
        if recover_evm_address(&hash, &r, &s, recovery_id).as_deref() == Some(ours.as_str()) {
            return Ok(EvmSignature {
                r: hex::encode(r),
                s: hex::encode(s),
                s_from_subnet: hex::encode(s_from_subnet),
                s_was_high,
                recovery_id,
                signer: ours,
            });
        }
    }
    Err(format!(
        "refused: neither parity of this signature recovers {ours}. Either the signature is not this canister's or the \
         hash signed is not the hash given. Nothing is returned, because returning one of two wrong answers is exactly how \
         a transaction ends up charged to an address nobody holds."
    ))
}

/// The rehearsal's EVM entry point: sign a 32-byte transaction hash through [`sign_evm_recoverable`], and therefore through
/// exactly the path a real buyer would take.
///
/// The guards are [`rehearse_sign_sighash`]'s, transposed. An **allowlist of exactly one chain id**, named in both the
/// install and the call, because "anything but 4663" accepts a typo and 4663 is where the money is:
///
/// 1. the canister must have been installed for [`REHEARSAL_EVM_CHAIN`];
/// 2. the caller must name it again, in the call. A caller that does not know which chain it is building for does not get a
///    signature;
/// 3. the hash must be exactly 32 bytes of hex.
///
/// And the same limit as on the Bitcoin side, restated because it does not go away: **the canister signs a hash and never
/// sees a network.** Nothing here can tell where the result is broadcast.
/// The chain-id half of [`rehearse_sign_evm`]'s guard, pulled out so it can be tested without a replica. Both the install
/// and the call must name [`REHEARSAL_EVM_CHAIN`]; there is no third accepted value and no default.
fn check_evm_rehearsal_chain(installed: u64, asked: u64) -> Result<(), String> {
    if installed != REHEARSAL_EVM_CHAIN {
        return Err(format!(
            "refused: this canister was installed for chain {installed}; the EVM rehearsal signs only for chain \
             {REHEARSAL_EVM_CHAIN}."
        ));
    }
    if asked != REHEARSAL_EVM_CHAIN {
        return Err(format!(
            "refused: caller asked to sign for chain {asked}. The only accepted value is {REHEARSAL_EVM_CHAIN} \
             (Robinhood Chain testnet), and it is an allowlist of one on purpose -- 'anything but \
             {PRODUCTION_EVM_CHAIN}' would accept a typo.{}",
            if asked == PRODUCTION_EVM_CHAIN {
                " That is Robinhood Chain MAINNET, where the vault and the money are. Never."
            } else {
                ""
            }
        ));
    }
    Ok(())
}

#[ic_cdk::update]
pub async fn rehearse_sign_evm(chain_id: u64, hash_hex: String) -> Result<EvmSignature, String> {
    only_operator();
    not_paused()?;
    check_evm_rehearsal_chain(cfg().chain_id, chain_id)?;
    let hash = hex::decode(hash_hex.trim()).map_err(|e| format!("hash is not hex: {e}"))?;
    let n = hash.len();
    let hash: [u8; 32] = hash
        .try_into()
        .map_err(|_| format!("an EVM signing hash is 32 bytes; got {n}"))?;
    sign_evm_recoverable(hash).await
}

/// Expand a compressed secp256k1 point to the 64 bytes an EVM address is derived from (x‖y, no prefix).
fn decompress(compressed: &[u8]) -> Result<Vec<u8>, String> {
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    let point = k256::PublicKey::from_sec1_bytes(compressed).map_err(|e| format!("not a public key: {e}"))?;
    Ok(point.to_encoded_point(false).as_bytes()[1..].to_vec())
}

fn keccak(b: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    let mut h = Keccak256::new();
    h.update(b);
    h.finalize().into()
}

#[cfg(test)]
pub(crate) mod tests {

    #[test]
    fn an_upgrade_keeps_the_buyers_identity_and_takes_its_endpoints_and_operator_from_the_argument() {
        let saved = Init {
            chain_id: 46630, vault: "0x01".into(), rpc_urls: vec!["https://rpc-a".into(), "https://rpc-b".into()],
            btc_api_urls: vec!["https://mempool.space/signet/api".into(), "https://open-api-signet.unisat.io".into()],
            key_name: "key_1".into(), accept_production_key_name: true, bridge_allowlist: vec!["0xbb".into()],
            btc_network: "signet".into(), operator: None, wrapper: None, evm_chains: None, custody_caps: None, outflow_limits: None,
            holder_deposits: None, wrapper_from_block: None, frog_funding: None, eth_nft: None, home: None,
        };
        let op = candid::Principal::from_slice(&[7; 10]);
        let arg = Init {
            chain_id: 4663, vault: "0x02".into(), rpc_urls: vec!["https://rpc-c".into(), "https://rpc-d".into()],
            btc_api_urls: vec!["https://mempool.space/signet/api".into(), "https://blockstream.info/signet/api".into()],
            key_name: "other".into(), accept_production_key_name: false, bridge_allowlist: vec![],
            btc_network: "testnet4".into(), operator: Some(op), wrapper: Some("0x00000000000000000000000000000000000000cc".into()), evm_chains: None,
            custody_caps: None, outflow_limits: None, holder_deposits: None, wrapper_from_block: None, frog_funding: None, eth_nft: None, home: None,
        };
        let got = restore_init(Some(saved.clone()), &arg);
        // identity: exactly as saved, whatever the argument says
        assert_eq!((got.chain_id, got.vault.as_str(), got.key_name.as_str(), got.btc_network.as_str()), (46630, "0x01", "key_1", "signet"));
        assert_eq!((got.accept_production_key_name, got.bridge_allowlist.clone()), (true, vec!["0xbb".to_string()]));
        // endpoints and operator: from the argument
        assert_eq!(got.rpc_urls, arg.rpc_urls);
        assert_eq!(got.btc_api_urls, arg.btc_api_urls);
        assert_eq!(got.operator, Some(op));
        // the wrapper: set once from nothing...
        assert_eq!(got.wrapper.as_deref(), Some("0x00000000000000000000000000000000000000cc"));
        // ...and never changed after
        let set = Init { wrapper: Some("0x00000000000000000000000000000000000000aa".into()), ..saved.clone() };
        assert_eq!(restore_init(Some(set), &arg).wrapper.as_deref(), Some("0x00000000000000000000000000000000000000aa"));
        // nothing saved: the argument, whole
        assert_eq!(restore_init(None, &arg).chain_id, 4663);
    }


    /// Every update method is the operator's, proved by reading this file rather than by remembering to.
    ///
    /// This is DFINITY's "verify that exported canister endpoints are legitimate" recommendation, made cheap: the failure it
    /// guards against is one somebody adds later. A newly added update endpoint without the guard is a control lever, or a signing
    /// oracle over 32 bytes, open to every principal on the IC — and that is exactly how the first one got there. The needle is
    /// built at runtime so that this test's own source does not match itself.
    #[test]
    fn every_update_method_takes_the_controller_guard() {
        let src = include_str!("canister.rs");
        let needle = format!("#[ic_cdk::{}]", "update");
        let mut unguarded = Vec::new();
        let mut seen = 0;
        for part in src.split(needle.as_str()).skip(1) {
            seen += 1;
            let head: String = part.chars().take(500).collect();
            let name = head.split("fn ").nth(1).and_then(|s| s.split('(').next()).unwrap_or("?").trim().to_string();
            if !head.contains("only_controller();") && !head.contains("only_operator();") {
                unguarded.push(name);
            }
        }
        assert!(seen >= 6, "the scan found {seen} update methods, which is fewer than this canister has");
        assert!(unguarded.is_empty(), "these update methods have no caller guard: {unguarded:?}");
    }

    /// H1 (docs/v5/custody-hardening.md): every operator method that signs, sends, records a quote, stages a spend or changes
    /// what custody holds refuses while paused. Read from the source of EVERY file in `src/`, so an operator method added later,
    /// in any module, fails this test until it either checks the pause or is argued onto the read-only list below.
    #[test]
    fn every_operator_method_that_acts_refuses_while_paused() {
        // Read-only (derive a public key, read a chain, check without signing), or safe while paused by design: `abandon` only
        // returns ETH to the vault; `tick` reads the pause itself and does nothing; `set_paused` is the pause. The ordinals
        // providers' configuration (docs/v6/ordinals-apis.md §3) is exempt too: a key or an ord added while paused moves
        // nothing, every provider must still agree, and a pause is exactly when an operator repairs a read.
        // §10.4: the guardian's stop-only pauses and the modules' off switches stop things, as `set_paused` does: safe while paused
        const EXEMPT: [&str; 31] = ["taproot_key", "taproot_output_key", "custody_address", "buyer_address", "evm_address", "sol_address",
            "sol_address_of", "pause_collection", "pause_desk", "pause_all_mints", "pool_set_off", "coins_set_off", "cranks_set_off", "sol_nft_set_off",
            "evm_tx_status", "pinned_read", "btc_tx_status", "probe_frog", "reference_rate", "probe_delivered", "probe_rune_name",
            "nft_preview", "nft_chain_status", "float_address", "abandon", "tick", "set_paused", "set_unisat_key", "clear_unisat_key",
            "add_ord_provider", "float_internal_key"];
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let needle = format!("#[ic_cdk::{}]", "update");
        let guard = format!("{}_paused()?;", "not");
        let (mut gated, mut seen_exempt, mut missing) = (Vec::new(), Vec::new(), Vec::new());
        // every `.rs` under `src/`, its subdirectories included (src/books/, …)
        fn files(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(d).unwrap().filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_dir() { files(&p, out) } else if p.extension().is_some_and(|x| x == "rs") { out.push(p) }
            }
        }
        let mut all = Vec::new();
        files(std::path::Path::new(dir), &mut all);
        for path in all {
            let src = std::fs::read_to_string(path).unwrap();
            let code = src.split(concat!("#[cfg(", "test)]")).next().unwrap(); // the methods, not the tests that name them
            for part in code.split(needle.as_str()).skip(1) {
                let head: String = part.chars().take(600).collect();
                let name = head.split("fn ").nth(1).and_then(|s| s.split('(').next()).unwrap_or("?").trim().to_string();
                if !head.contains("only_operator();") {
                    continue; // controller-only configuration: the controller can lift the pause anyway
                }
                if EXEMPT.contains(&name.as_str()) {
                    seen_exempt.push(name);
                    continue;
                }
                if head.contains(&guard) { gated.push(name) } else { missing.push(name) }
            }
        }
        assert!(missing.is_empty(), "these operator methods act and do not check the pause: {missing:?}");
        for n in EXEMPT {
            assert!(seen_exempt.iter().any(|x| x == n), "{n} is on the read-only list but no longer exists: remove it");
        }
        // the fourteen of the H1 diff, and what was added since (the floor-feed merge's two operator endpoints among them)
        for n in ["open_purchase", "relay_venue_quote", "relay_venue_psbt", "sign_purchase", "record_quote", "rehearse_sign_sighash",
            "rehearse_sign_evm", "rehearse_broadcast_evm", "rehearse_broadcast_btc", "nft_set_target", "nft_sign", "nft_send",
            "frog_offer", "refuse_deposit", "rehearse_open_purchase", "rehearse_fixtures", "nft_status", "frog_proof",
            "floor_nft_sign", "floor_nft_arrived", "frog_offer_protected"] {
            assert!(gated.iter().any(|x| x == n), "{n} is not gated");
        }
    }

    /// The pause refuses, and lifting it lets the same call through.
    #[test]
    fn not_paused_refuses_exactly_while_paused() {
        PAUSED.with(|p| *p.borrow_mut() = true);
        assert!(not_paused().unwrap_err().contains("paused"));
        PAUSED.with(|p| *p.borrow_mut() = false);
        assert_eq!(not_paused(), Ok(()));
    }

    /// Operations are the operator's or a controller's; nobody else's, and never the anonymous principal's by default.
    #[test]
    fn the_operator_operates_and_nobody_else_does() {
        let (op, other) = (candid::Principal::from_slice(&[7; 10]), candid::Principal::from_slice(&[8; 10]));
        assert!(may_operate(true, other, None), "a controller operates, as before the handover");
        assert!(may_operate(false, op, Some(op)), "the named operator operates without being a controller");
        assert!(!may_operate(false, other, Some(op)), "a stranger does not");
        assert!(!may_operate(false, candid::Principal::anonymous(), None), "no operator named: only controllers");
        assert!(!may_operate(false, candid::Principal::anonymous(), Some(op)));
    }

    /// The Candid interface shipped (`etch_autobuyer.did`) is the code's: every endpoint and type, as `ic_cdk::export_candid!` writes
    /// it (run with `WRITE_FIXTURES=1` to rewrite the file after adding or changing an endpoint; `candid-extractor` on the wasm gives
    /// the same text).
    #[test]
    fn the_did_file_is_the_interface_the_code_exports() {
        let now = __export_service();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/etch_autobuyer.did");
        if std::env::var("WRITE_FIXTURES").is_ok() {
            std::fs::write(path, format!("{now}\n")).unwrap();
        }
        let file = std::fs::read_to_string(path).unwrap();
        assert_eq!(now.trim(), file.trim(), "etch_autobuyer.did is stale: WRITE_FIXTURES=1 cargo test --lib the_did_file");
    }

    /// Resuming is a controller's act only: an operator who can pause cannot lift the guardian's veto.
    #[test]
    fn set_paused_lets_the_operator_stop_but_only_a_controller_resume() {
        let src = include_str!("canister.rs");
        let body = src.split("pub fn set_paused(p: bool) {").nth(1).expect("set_paused exists");
        let body: String = body.chars().take(700).collect();
        let (pause, resume) = body.split_once("} else {").expect("two branches");
        assert!(pause.contains("only_operator();") && !pause.contains("only_controller();"));
        assert!(resume.contains("only_controller();") && !resume.contains("only_operator();"));
    }

    /// The guard refuses a second entry and is released when it drops, so one failed tick does not wedge the canister.
    #[test]
    fn a_second_tick_is_refused_while_one_is_running_and_the_flag_is_released_on_drop() {
        {
            let _first = begin_tick().expect("the first tick takes the flag");
            let second = begin_tick().expect_err("the second is refused");
            assert!(second.contains("already running"), "{second}");
        } // _first drops here, exactly as it would through ic0.call_on_cleanup after a trap
        let _again = begin_tick().expect("the flag was released, so a later tick may run");
    }
    use super::*;

    /// Audit D, M-2: a golden fixture saved by the audited build (`e04f49b`, every record kind in flight: signed, sent and
    /// conflicting deliveries, a signed buy, returning and refused deposits, the cap book with a reservation, a frozen limiter,
    /// paused) decodes into this build's types with every record kept, and the new fields read as absent.
    #[test]
    fn the_state_the_audited_build_saved_decodes_into_this_build_with_every_record_kept() {
        let bytes = hex::decode(include_str!("../tests/fixtures/state/saved-e04f49b.hex").trim()).unwrap();
        let (s,): (Saved,) = candid::decode_args(&bytes).expect("e04f49b's saved state decodes into this build");
        assert!(s.2, "still paused");
        assert_eq!(s.3, vec!["installed".to_string(), "paused".to_string()]);
        let init = s.0.clone().unwrap();
        assert_eq!((init.holder_deposits, init.wrapper_from_block), (None, None), "deposits stay off");
        assert_eq!(init.wrapper.as_deref(), Some("0x00000000000000000000000000000000000000cc"));
        let c = s.4.clone().unwrap();
        assert_eq!((c.cursor, c.redemptions.len(), c.logs.clone()), (6, 6, None));
        use crate::redeem::RPhase;
        assert!(matches!(c.redemptions[1].phase, RPhase::Signed { ref raw, bump: None, .. } if *raw == vec![1, 2, 3]));
        assert!(matches!(c.redemptions[2].phase, RPhase::Sent { bump: None, .. }));
        let RPhase::Conflicting { ref conflict, .. } = c.redemptions[3].phase else { panic!("{:?}", c.redemptions[3].phase) };
        assert_eq!((conflict.earlier.clone(), conflict.leaked_fee, conflict.tx.as_ref().map(|t| t.fee)), (None, None, Some(869)));
        let f = s.6.clone().unwrap();
        assert!(matches!(f.frogs[0].phase, crate::frogbuy::FPhase::Signed { .. }));
        assert_eq!(f.deposits.len(), 4);
        assert!(f.deposits.iter().all(|d| d.next_check.is_none() && d.tries.is_none()));
        assert!(matches!(f.deposits[1].phase, crate::deposits::DPhase::Pending { ref why } if why.contains("awaiting a membership proof")));
        assert_eq!((f.unread.clone(), f.buy_clock.clone()), (None, None));
        let (book, out) = s.8.clone().unwrap();
        assert!(book.entries.iter().any(|e| e.id == "frog:0" && e.in_flight));
        assert!(out.frozen().is_some() && out.acked.is_none());
        assert_eq!(s.11, None, "no fee pool before it existed");
        // and this build's state round-trips whole
        let (again,): (Saved,) = candid::decode_args(&candid::encode_args((s.clone(),)).unwrap()).unwrap();
        assert_eq!((again.4, again.6, again.8), (s.4, s.6, s.8));
    }

    /// Audit D, M-2 and L-2: a state that does not decode refuses the upgrade (it is never a fresh install, which would unpause
    /// and forget every redemption), and an upgrade that lands mid-tick is refused too.
    #[test]
    fn an_upgrade_whose_state_does_not_decode_is_refused_and_one_mid_tick_is_refused() {
        let src = include_str!("canister.rs");
        let post = src.split("fn post_upgrade(arg: Init) {").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(!post.contains(concat!("init(", "arg)")), "no fall-back to a fresh install");
        assert!(post.contains("Err(e) => ic_cdk::trap("), "a decode error traps");
        let pre = src.split("fn pre_upgrade() {").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(pre.contains("TICKING") && pre.contains("ic_cdk::trap(\"refused: a tick is running"));
        assert!(candid::decode_args::<(Saved,)>(b"DIDL\x00\x01\x7f").is_err(), "a different shape does not decode");
    }

    /// Audit R3, R3-C3: the paused tick's restricted custodian pass (it records confirmed deliveries on chain) first checks that
    /// the RPC is the chain this canister was installed for, as the unpaused tick does; and only the IC's own size reject is marked
    /// as one for the ticket-log scanner (R3-C2).
    #[test]
    fn the_paused_pass_asserts_the_chain_first_and_only_the_ics_size_reject_is_marked() {
        let src = include_str!("canister.rs");
        let tick = src.split("pub async fn tick() -> Result<Vec<String>, String> {").nth(1).unwrap();
        let paused = tick.split("if PAUSED.with(|p| *p.borrow()) {").nth(1).unwrap().split("        return Ok(lines);\n    }").next().unwrap();
        let (check, pass) = (paused.find("assert_home().await").expect("asserted"), paused.find("custodynet::tick(").expect("the pass"));
        assert!(check < pass, "the chain is asserted before the paused pass");
        let rpc = src.split("pub(crate) async fn rpc_once_with(").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(rpc.contains("http_request(&req).await.map_err(outcall_error)"), "{rpc}");
        let oe = src.split("pub(crate) fn outcall_error(").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(oe.contains("Error::CallRejected(r)") && oe.contains("ic_size_reject(r.reject_message())"), "{oe}");
    }

    /// Audit D, L-3: ingress from strangers is not accepted, except `submit_offer` while a board exists, never anonymously.
    #[test]
    fn only_the_operator_and_controllers_are_admitted_and_strangers_only_to_an_open_board() {
        let (op, stranger) = (candid::Principal::from_slice(&[7; 29]), candid::Principal::from_slice(&[9; 29]));
        let anon = candid::Principal::anonymous();
        assert!(may_ingress("tick", true, stranger, None, false), "a controller");
        assert!(may_ingress("tick", false, op, Some(op), false), "the operator");
        assert!(!may_ingress("tick", false, stranger, Some(op), true));
        assert!(!may_ingress("submit_offer", false, stranger, Some(op), false), "no board: closed");
        assert!(may_ingress("submit_offer", false, stranger, Some(op), true));
        assert!(!may_ingress("submit_offer", false, anon, Some(op), true));
        assert!(!may_ingress("set_paused", false, anon, None, true));
        // the public desk and coins methods: anyone, the anonymous principal too (an opening request is free; audit C4-05: inspect_message
        // then accepts a stranger's request only when it would queue the collection, `coins::api::may_request`)
        assert!(may_ingress("open_intent", false, anon, Some(op), false));
        assert!(may_ingress("coins_request_open", false, anon, Some(op), false));
        assert!(may_ingress("coins_request_open", false, stranger, None, false));
        assert!(!may_ingress("coins_set_auto_open", false, stranger, Some(op), false), "the switch is the operator's");
    }

    /// Audit D, L-4: every update method that writes state a tick holds a copy of refuses while a tick runs (or holds the tick's
    /// flag for its own awaits), so a change is never silently overwritten.
    #[test]
    fn every_method_that_writes_ticked_state_refuses_mid_tick() {
        let files = [("frognet.rs", include_str!("frognet.rs")), ("floornet.rs", include_str!("floornet.rs")),
            ("handinnet.rs", include_str!("handinnet.rs")), ("salenet.rs", include_str!("salenet.rs")),
            ("custodynet.rs", include_str!("custodynet.rs")), ("ethnet.rs", include_str!("ethnet.rs")), ("solhome.rs", include_str!("solhome.rs"))];
        let ticked = ["FROGS.with", "FLOORS.with", "HANDINS.with", "SALES.with", "CUSTODY.with", "FEE_POOL.with", "ETH.with"];
        let mut checked = 0;
        for (name, src) in files {
            for part in src.split(concat!("#[ic_cdk::", "update]")).skip(1) {
                let body = part.split("\n}\n").next().unwrap_or("");
                let fname = body.split("fn ").nth(1).and_then(|x| x.split('(').next()).unwrap_or("?");
                // a write: a ticked static's `with` whose closure borrows it mutably (a read of it, beside a write to heap-only
                // state such as frog_offer's OFFERS, is not one)
                let writes = ticked.iter().any(|t| body.match_indices(t).any(|(i, _)| {
                    let end = body.len().min(i + 160);
                    body.get(i..end).is_some_and(|w| w.contains("borrow_mut()"))
                }));
                if writes {
                    checked += 1;
                    assert!(body.contains("not_ticking()?") || body.contains("hold_tick()?"), "{name}: {fname} writes ticked state unguarded");
                }
            }
        }
        assert!(checked >= 14, "{checked} methods checked");
    }

    #[test]
    fn a_state_saved_before_custody_existed_still_restores_and_custody_round_trips() {
        // what the canister now deployed saves: four elements, no custody
        let old: (Option<Init>, Vec<Purchase>, bool, Vec<String>) = (Some(good()), vec![], true, vec!["installed".into()]);
        let bytes = candid::encode_args((old,)).unwrap();
        let (restored,): (Saved,) = candid::decode_args(&bytes).expect("an older state decodes into the new shape");
        assert_eq!((restored.2, restored.3.clone(), restored.4.clone()), (true, vec!["installed".to_string()], None));
        assert_eq!(restored.0.unwrap().wrapper, None, "an Init saved before the wrapper existed reads as no wrapper");
        // and a custody record survives an upgrade whole
        let c = crate::redeem::Custody { cursor: 3, redemptions: vec![crate::redeem::Redemption { ticket: 2, acquisition_id: 7,
            destination: "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr".into(),
            phase: crate::redeem::RPhase::Signed { txid: [9; 32], raw: vec![1, 2, 3], fee_input: crate::custody::Outpoint { txid: [8; 32], vout: 1 }, bump: None },
            inscription: Some(format!("{}i0", "07".repeat(32))), attest: None, noted: None }], logs: None, colls: None };
        let now: Saved = (Some(good()), vec![], false, vec![], Some(c.clone()), None, None, None, None, None, None, None, None, None, None);
        let (back,): (Saved,) = candid::decode_args(&candid::encode_args((now,)).unwrap()).unwrap();
        assert_eq!(back.4, Some(c));
    }

    #[test]
    fn a_state_saved_before_nft_chains_existed_still_restores_and_an_nft_line_round_trips() {
        // what f24e234 saves: five elements, and an Init with no evm_chains
        #[derive(candid::CandidType)]
        struct OldInit { chain_id: u64, vault: String, rpc_urls: Vec<String>, btc_api_urls: Vec<String>, key_name: String,
            accept_production_key_name: bool, bridge_allowlist: Vec<String>, btc_network: String,
            operator: Option<candid::Principal>, wrapper: Option<String> }
        let g = good();
        let old_init = OldInit { chain_id: g.chain_id, vault: g.vault, rpc_urls: g.rpc_urls, btc_api_urls: g.btc_api_urls,
            key_name: g.key_name, accept_production_key_name: false, bridge_allowlist: vec![], btc_network: g.btc_network,
            operator: None, wrapper: None };
        let old: (Option<OldInit>, Vec<Purchase>, bool, Vec<String>, Option<crate::redeem::Custody>) =
            (Some(old_init), vec![], false, vec![], None);
        let (restored,): (Saved,) = candid::decode_args(&candid::encode_args((old,)).unwrap()).expect("f24e234's state decodes");
        assert_eq!((restored.0.unwrap().evm_chains, restored.5), (None, None));
        // and a line with a signed purchase in it survives an upgrade whole: the saved bytes are what gets rebroadcast
        let mut nft = crate::chains::NftState::default();
        nft.line_mut(1).in_flight = Some(crate::chains::InFlight { nonce: 3, tx_hash: "0xab".into(), raw: vec![2, 1, 0],
            collection: "0x01".into(), token_id: "9820".into(), total_wei: "53000000000000000".into(), order_hash: "0xcd".into() });
        let now: Saved = (Some(good()), vec![], false, vec![], None, Some(nft.clone()), None, None, None, None, None, None, None, None, None);
        let (back,): (Saved,) = candid::decode_args(&candid::encode_args((now,)).unwrap()).unwrap();
        assert_eq!(back.5, Some(nft));
    }

    #[test]
    fn a_state_saved_before_item_sinks_existed_still_restores_and_the_item_state_round_trips() {
        // what 6eb96e6 saves: six elements
        let old: (Option<Init>, Vec<Purchase>, bool, Vec<String>, Option<crate::redeem::Custody>, Option<crate::chains::NftState>) =
            (Some(good()), vec![], true, vec![], None, Some(crate::chains::NftState::default()));
        let (restored,): (Saved,) = candid::decode_args(&candid::encode_args((old,)).unwrap()).expect("6eb96e6's state decodes");
        assert_eq!((restored.2, restored.6.clone()), (true, None));
        // a frog signed and not yet seen, a deposit returning, and our EVM transaction out: all survive whole
        let buy = crate::frogbuy::Bought { inscription: "ab".repeat(32) + "i0", outpoint: crate::custody::Outpoint { txid: [1; 32], vout: 1 },
            seller: crate::custody::Outpoint { txid: [2; 32], vout: 0 }, spend_sats: 903_000, fee_sats: 3_000, rate: 3_190_000,
            cost_wei: 288_733_542_319_749_217, raw: vec![2, 0, 1], setup_txid: None };
        let mut st = crate::frogflow::FrogState { desk: Some([5; 20]), ..Default::default() };
        st.frogs.push(crate::frogbuy::FrogRec { id: 0, target: Some(crate::frogbuy::Target::Sink([7; 20])),
            phase: crate::frogbuy::FPhase::Signed { buy } });
        st.deposits.push(crate::deposits::DepositRec { outpoint: crate::custody::Outpoint { txid: [3; 32], vout: 0 }, first_seen: 9,
            phase: crate::deposits::DPhase::Returning { reason: crate::deposits::Reason::Runes(vec!["X".into()]), txid: [4; 32],
                raw: vec![9], fee_input: crate::custody::Outpoint { txid: [5; 32], vout: 2 }, unseen_since: Some(77) }, admitted: None, next_check: None, tries: None });
        st.pending = Some(crate::frogflow::FrogEvm { step: crate::frogflow::FStep::Mint { frog: 0 }, raw: vec![2, 2], hash: [6; 32], nonce: 4 });
        let now: Saved = (Some(good()), vec![], false, vec![], None, None, Some(st.clone()), None, None, None, None, None, None, None, None);
        let (back,): (Saved,) = candid::decode_args(&candid::encode_args((now,)).unwrap()).unwrap();
        assert_eq!(back.6, Some(st));
    }

    /// H8 (K1): a return saved before `unseen_since` existed restores with its clock unset, and both conflict phases -- a
    /// redemption's `Conflicting` and a deposit's `ReturnConflicting`, with their signed conflict -- survive the upgrade encoding.
    #[test]
    fn a_return_saved_before_h8_restores_and_both_conflict_phases_round_trip_an_upgrade() {
        use crate::custody::Outpoint;
        use crate::deposits::{DPhase, DepositRec, Reason};
        #[derive(candid::CandidType)]
        enum OldDPhase { Returning { reason: Reason, txid: [u8; 32], raw: Vec<u8>, fee_input: Outpoint } }
        #[derive(candid::CandidType)]
        struct OldRec { outpoint: Outpoint, first_seen: u64, phase: OldDPhase, admitted: Option<String> }
        let o = Outpoint { txid: [3; 32], vout: 0 };
        let fee = Outpoint { txid: [5; 32], vout: 2 };
        let old = OldRec { outpoint: o, first_seen: 9, phase: OldDPhase::Returning { reason: Reason::ZeroRecipient, txid: [4; 32],
            raw: vec![9], fee_input: fee }, admitted: None };
        let back: DepositRec = candid::decode_one(&candid::encode_one(&old).unwrap()).expect("a pre-H8 return decodes");
        assert_eq!(back.phase, DPhase::Returning { reason: Reason::ZeroRecipient, txid: [4; 32], raw: vec![9], fee_input: fee,
            unseen_since: None });
        let conflict = crate::conflict::Conflict { leaked_txid: [4; 32], raw: vec![1, 2], fee_input: fee, item: o,
            tx: Some(crate::conflict::ConflictTx { txid: [6; 32], raw: vec![3], inputs: vec![fee, Outpoint { txid: [7; 32], vout: 1 }], fee: 869,
                signed_at: Some(5), earlier: Some(vec![[5; 32]]) }), earlier: None, leaked_fee: None, held_since: Some(7) };
        let custody = crate::redeem::Custody { cursor: 2, redemptions: vec![crate::redeem::Redemption { ticket: 1, acquisition_id: 7,
            destination: "bc1p".into(), phase: crate::redeem::RPhase::Conflicting { reason: "InsideMargin; …".into(), conflict: conflict.clone() },
            inscription: Some("ab".repeat(32) + "i0"), attest: Some(crate::redeem::Attest::Due { deep: Some([6; 32]) }), noted: None }], logs: None, colls: None };
        let mut st = crate::frogflow::FrogState::default();
        st.deposits.push(DepositRec { outpoint: o, first_seen: 9, phase: DPhase::ReturnConflicting { reason: Reason::ZeroRecipient, conflict },
            admitted: None, next_check: None, tries: None });
        let now: Saved = (Some(good()), vec![], true, vec![], Some(custody.clone()), None, Some(st.clone()), None, None, None, None, None, None, None, None);
        let (back,): (Saved,) = candid::decode_args(&candid::encode_args((now,)).unwrap()).unwrap();
        assert_eq!((back.4, back.6, back.2), (Some(custody), Some(st), true));
    }

    #[test]
    fn a_state_saved_before_floor_sinks_existed_still_restores_and_the_floor_state_round_trips() {
        // what 9022920 saves: seven elements, with a queue whose view has no `price_expiry` (the SinkView of then)
        #[derive(candid::CandidType)]
        enum OldKind { Queue, Curve }
        #[derive(candid::CandidType)]
        struct OldClock { time: u64, parent_block: u64 }
        #[derive(candid::CandidType)]
        struct OldView { kind: OldKind, status: String, open: bool, done: bool, recorded: u64, target: Option<u64>, want: u64,
            budget_left: u128, item_cap: Option<u128>, deadline: Option<(u64, u64)>, clock: OldClock, drive_refunds: bool, drive_settle: bool }
        #[derive(candid::CandidType)]
        struct OldRec { kind: OldKind, sink: [u8; 20], coin: [u8; 20], view: Option<OldView>, refused: Option<String>,
            drove_refunds_at: Option<u64>, drove_settle: bool }
        #[derive(candid::CandidType)]
        struct OldFrogs { factory: Option<[u8; 20]>, desk: Option<[u8; 20]>, coin_cursor: u64, sinks: Vec<OldRec>, poll: u64,
            request_cursor: u64, requests: Vec<crate::mintdesk::ReqRec>, frogs: Vec<crate::frogbuy::FrogRec>, next_frog: u64,
            deposits: Vec<crate::deposits::DepositRec>, seen: Vec<crate::custody::Outpoint>, unmarked: Vec<crate::custody::Outpoint>,
            proofs: Vec<(String, Vec<[u8; 32]>)>, mint_times: Vec<u64>, return_times: Vec<u64>, pending: Option<crate::frogflow::FrogEvm> }
        let _ = OldKind::Curve;
        let view = OldView { kind: OldKind::Queue, status: "open".into(), open: true, done: false, recorded: 3, target: None, want: 1,
            budget_left: 7, item_cap: Some(5), deadline: Some((9, 10)), clock: OldClock { time: 1, parent_block: 2 }, drive_refunds: false,
            drive_settle: false };
        let frogs = OldFrogs { factory: None, desk: None, coin_cursor: 0, sinks: vec![OldRec { kind: OldKind::Queue, sink: [7; 20],
            coin: [0; 20], view: Some(view), refused: None, drove_refunds_at: None, drove_settle: false }], poll: 0, request_cursor: 0,
            requests: vec![], frogs: vec![], next_frog: 0, deposits: vec![], seen: vec![], unmarked: vec![], proofs: vec![],
            mint_times: vec![], return_times: vec![], pending: None };
        let old = (Some(good()), Vec::<Purchase>::new(), false, Vec::<String>::new(), None::<crate::redeem::Custody>,
            None::<crate::chains::NftState>, Some(frogs));
        let (restored,): (Saved,) = candid::decode_args(&candid::encode_args((old,)).unwrap()).expect("9022920's state decodes");
        assert_eq!(restored.7, None, "no floor state before it existed");
        let st = restored.6.unwrap();
        let v = st.sinks[0].view.clone().unwrap();
        assert_eq!((st.sinks[0].kind, v.recorded, v.price_expiry), (crate::itemsink::SinkKind::Queue, 3, None),
            "an old queue view decodes with no price expiry: its behaviour is unchanged");
        // and a floor state survives an upgrade whole
        let mut fl = crate::floortrigger::FloorState::default();
        fl.add_sink([9; 20], crate::floortrigger::FloorItem::Evm { chain_id: 1, collection: [0xbc; 20], wrapper: [0xa9; 20] }).unwrap();
        fl.set_limit([9; 20], 6_000_000_000_000_000_000).unwrap();
        fl.wants.push(crate::floortrigger::NftWant { id: 0, module: [9; 20], chain_id: 1, collection: [0xbc; 20], cap_wei: 5,
            phase: crate::floortrigger::WPhase::Arrived { token_id: "9".into(), tx_hash: "0xab".into(), cost_wei: 5 } });
        let now: Saved = (Some(good()), vec![], false, vec![], None, None, None, Some(fl.clone()), None, None, None, None, None, None, None);
        let (back,): (Saved,) = candid::decode_args(&candid::encode_args((now,)).unwrap()).unwrap();
        assert_eq!(back.7, Some(fl));
    }

    #[test]
    fn a_state_saved_before_the_custody_guard_existed_still_restores_and_the_book_and_limiter_round_trip() {
        // what ac3cb58 saves: eight elements, the floor state last, and an Init with no custody_caps or outflow_limits
        #[derive(candid::CandidType)]
        struct OldInit { chain_id: u64, vault: String, rpc_urls: Vec<String>, btc_api_urls: Vec<String>, key_name: String,
            accept_production_key_name: bool, bridge_allowlist: Vec<String>, btc_network: String,
            operator: Option<candid::Principal>, wrapper: Option<String>, evm_chains: Option<Vec<crate::chains::EvmChain>> }
        let g = good();
        let old_init = OldInit { chain_id: g.chain_id, vault: g.vault, rpc_urls: g.rpc_urls, btc_api_urls: g.btc_api_urls,
            key_name: g.key_name, accept_production_key_name: false, bridge_allowlist: vec![], btc_network: g.btc_network,
            operator: None, wrapper: Some("0x00000000000000000000000000000000000000cc".into()), evm_chains: None };
        let mut fl = crate::floortrigger::FloorState::default();
        fl.add_sink([9; 20], crate::floortrigger::FloorItem::Bitcoin).unwrap();
        let old = (Some(old_init), Vec::<Purchase>::new(), true, vec!["paused".to_string()], None::<crate::redeem::Custody>,
            None::<crate::chains::NftState>, None::<crate::frogflow::FrogState>, Some(fl.clone()));
        let (restored,): (Saved,) = candid::decode_args(&candid::encode_args((old,)).unwrap()).expect("ac3cb58's state decodes");
        assert_eq!((restored.2, restored.7.clone(), restored.8.clone()), (true, Some(fl), None), "no guard before it existed");
        let init = restored.0.unwrap();
        assert_eq!((init.custody_caps.clone(), init.outflow_limits.clone()), (None, None));
        // ...which reads as stage 0 for the wrapper's inscriptions, and an upgrade from it may raise by x4 at most
        let caps = caps_of(&init);
        assert_eq!(caps, crate::custodycap::stage0(&["btc:0x00000000000000000000000000000000000000cc".to_string()]));
        assert_eq!(limits_of(&init), crate::outflow::Limits::stage0());
        // the book and the limiter survive an upgrade whole, frozen state included
        let mut book = crate::custodycap::Book::default();
        crate::custodycap::admit(&caps, &mut book, &format!("{}i0", "ab".repeat(32)), &caps.collections[0].collection, 5).unwrap();
        let mut out = crate::outflow::Outflow::default();
        out.record("ticket:0", &caps.collections[0].collection, crate::outflow::Flow::Item, 1, 1_790_000_000);
        out.freeze("paused", 1_790_000_001);
        let now: Saved = (Some(good()), vec![], true, vec![], None, None, None, None, Some((book.clone(), out.clone())), None, None, None, None, None, None);
        let (back,): (Saved,) = candid::decode_args(&candid::encode_args((now,)).unwrap()).unwrap();
        assert_eq!(back.8, Some((book, out)));
    }

    #[test]
    fn a_state_saved_before_offer_boards_existed_still_restores_and_the_hand_in_state_round_trips() {
        // what feb928b saves: nine elements, the guard last
        let old = (Some(good()), Vec::<Purchase>::new(), false, Vec::<String>::new(), None::<crate::redeem::Custody>,
            None::<crate::chains::NftState>, None::<crate::frogflow::FrogState>, None::<crate::floortrigger::FloorState>,
            Some((crate::custodycap::Book::default(), crate::outflow::Outflow::default())));
        let (restored,): (Saved,) = candid::decode_args(&candid::encode_args((old,)).unwrap()).expect("feb928b's state decodes");
        assert_eq!(restored.9, None, "no boards before they existed");
        // a board, a submission read, its bytes, and a verdict survive an upgrade whole
        let mut hs = crate::handin::HandinState::default();
        hs.add_board([0x0b; 20]).unwrap();
        hs.board_mut(&[0x0b; 20]).unwrap().params = Some(crate::handin::Params { bond_wei: 1, reward_wei: 2, slash_delay: 3, expiry: 4,
            reward_day_cap_wei: 5, max_open: 6 });
        hs.seen.push(crate::handin::Seen { board: [0x0b; 20], id: 7, hash: [8; 32], submitter: [9; 20], at: 10, at_block: 11, ask_sats: 12,
            inscription: "x".into(), open: true });
        hs.bytes.push(crate::handin::Bytes { hash: [8; 32], psbt: vec![1, 2, 3], to_sign: vec![0], proof: vec![[4; 32]], received_at: 13 });
        hs.set_verdict([0x0b; 20], 7, crate::handin::Verdict::Bought { frog: 2 }, 14);
        let now: Saved = (Some(good()), vec![], false, vec![], None, None, None, None, None, Some(hs.clone()), None, None, None, None, None);
        let (back,): (Saved,) = candid::decode_args(&candid::encode_args((now,)).unwrap()).unwrap();
        assert_eq!(back.9, Some(hs));
    }

    #[test]
    fn a_state_saved_before_sales_existed_still_restores_and_the_sale_state_round_trips() {
        // what 9c6df31 saves: ten elements, the hand-ins last
        let old = (Some(good()), Vec::<Purchase>::new(), false, Vec::<String>::new(), None::<crate::redeem::Custody>,
            None::<crate::chains::NftState>, None::<crate::frogflow::FrogState>, None::<crate::floortrigger::FloorState>,
            None::<(crate::custodycap::Book, crate::outflow::Outflow)>, Some(crate::handin::HandinState::default()));
        let (restored,): (Saved,) = candid::decode_args(&candid::encode_args((old,)).unwrap()).expect("9c6df31's state decodes");
        assert_eq!((restored.9.clone(), restored.10.clone()), (Some(crate::handin::HandinState::default()), None), "no sales before they existed");
        // a policy, a listed sale with its signed listing, a sold relist and the ticket cursor survive an upgrade whole
        let mut ss = crate::sale::SaleState::default();
        ss.set_policy(crate::sale::SalePolicy { module: [0x56; 20], reserve_floor_wei: 5, markup_bps: 500, max_open: 1, fee_bps: 0 }, None).unwrap();
        let id = ss.plan([0x56; 20], [0xaa; 20]);
        ss.sale_mut(id).unwrap().ticket = Some(3);
        ss.sale_mut(id).unwrap().phase = crate::sale::SPhase::Listed { inscription: format!("{}i0", "07".repeat(32)),
            outpoint: crate::custody::Outpoint { txid: [9; 32], vout: 1 }, ask_sats: 950_000, ask_wei: 7, rate: 3_190_000, listing: vec![1, 2, 3], hash: [4; 32] };
        let r = ss.adopt_relist([0xaa; 20], 4, [0x55; 20], format!("{}i0", "08".repeat(32)), 9).unwrap();
        ss.sale_mut(r).unwrap().phase = crate::sale::SPhase::Sold { inscription: format!("{}i0", "08".repeat(32)), fill_txid: [5; 32], sold_sats: 1,
            rate: 2, proceeds_wei: 3, ask_wei: 9, hash: [6; 32] };
        ss.ticket_cursor = 5;
        ss.set_extra([0x56; 20], crate::floorsink::FloorExtra { price: 1, float_units: 2, min_sale_proceeds: 3, reserve: 4 });
        let now: Saved = (Some(good()), vec![], false, vec![], None, None, None, None, None, None, Some(ss.clone()), None, None, None, None);
        let (back,): (Saved,) = candid::decode_args(&candid::encode_args((now,)).unwrap()).unwrap();
        assert_eq!(back.10, Some(ss));
    }

    fn with_wrapper() -> Init {
        Init { wrapper: Some("0x00000000000000000000000000000000000000cc".into()), ..good() }
    }

    fn stage(k: u128) -> crate::custodycap::CapPolicy {
        let mut p = crate::custodycap::stage0(&["btc:0x00000000000000000000000000000000000000cc".to_string()]);
        let c = &mut p.collections[0];
        c.max_items *= k as u32;
        c.max_value_wei *= k;
        c.max_item_wei *= k;
        p.global_max_items *= k as u32;
        p.global_max_value_wei *= k;
        p
    }

    /// Review round 4 (A4-L-1): the install's exit rule held to the whole custody, not each collection alone. The PoC
    /// (review4/poc/A/a4_exit_fits_global.rs): J7's launch caps (16 + 22 + 17 + 4 + 2 = 61 items, the global their sum) at the
    /// launch's 24 deliveries a day in all passed the install check, collection by collection, and 13 items could not leave within
    /// the timelock's 72 h less a day.
    #[test]
    fn a5_l1_a_lowering_that_what_custody_holds_would_not_fit_is_refused() {
        // review round 5, A5-L-1 (poc/A `weakness_lowering_caps_and_outflow_together_passes_while_custody_holds_more`): 61 held at 12 / 31,
        // one upgrade to caps 3/3/3/2/1 at 3 / 6 a day passed the rule on the named caps, and 12 of 61 could leave in the next proposal's
        // usable days
        let cols = ["a", "b", "c", "d", "e"];
        let caps = |items: [u32; 5]| crate::custodycap::CapPolicy { collections: cols.iter().zip(items).map(|(c, n)| crate::custodycap::CollectionCap {
            collection: format!("1:{c}"), max_items: n, max_value_wei: 15, max_item_wei: 15 }).collect(), global_max_items: items.iter().sum(),
            global_max_value_wei: 75 };
        let lim = |per: u32, glob: u32| crate::outflow::Limits { per_collection_items: per, global_items: glob, per_collection_mints: 3,
            global_mints: 6, global_value_wei: 25, trip_factor: 3 };
        let mut book = crate::custodycap::Book::default();
        for (c, n) in cols.iter().zip([16u32, 22, 17, 4, 2]) {
            for k in 0..n {
                book.entries.push(crate::custodycap::Entry { id: format!("1:{c}:{k}"), collection: format!("1:{c}"), value_wei: 0, in_flight: false });
            }
        }
        assert_eq!(held_fits(&caps([16, 22, 17, 4, 2]), &book, &lim(12, 31)), Ok(()), "J7 at 12 / 31: fits");
        let e = held_fits(&caps([3, 3, 3, 2, 1]), &book, &lim(3, 6)).unwrap_err();
        assert!(e.contains("A5-L-1"), "{e}");
        assert_eq!(held_fits(&caps([3, 3, 3, 2, 1]), &book, &lim(12, 31)), Ok(()), "lowering the caps alone stays allowed");
        let src = include_str!("canister.rs");
        let post = src.split("fn post_upgrade(").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(post.contains("held_fits(&caps_of(&kept), &book, &limits_of(&kept))"), "it runs at every upgrade, on the restored book");
        // audit C2-08: the cap comparison and the held check run after the books (the desk) and the frog state are restored
        let (desk, cmp, held) = (post.find("crate::desk::state::after_upgrade()").unwrap(), post.find("check_cap_change(saved_init.as_ref(), &kept)").unwrap(),
            post.find("held_fits(&caps_of(&kept)").unwrap());
        let frogs = post.find("crate::frognet::FROGS.with(|f| *f.borrow_mut() = frogs").unwrap();
        assert!(desk < cmp && frogs < cmp && cmp < held, "the caps are compared on the restored books and frog state");
    }

    #[test]
    fn a4_l1_the_whole_custody_must_leave_within_the_delay_at_the_global_item_limit() {
        const E: u128 = 1_000_000_000_000_000_000;
        let cols = ["5af0d9827e0c53e4799bb226655a1de152a425a5", "ed5af388653567af2f388e6224dc7c4b3241c544",
            "60e4d786628fea6478f785a6d7e704777c86a7c6", "bd3531da5cf5857e7cfaa92426877b022e612cf8", "bc4ca0eda7647a8ab7c2061c2e118a18a936f13d"];
        let caps = |items: [u32; 5]| {
            let collections: Vec<crate::custodycap::CollectionCap> = cols.iter().zip(items).map(|(c, n)| crate::custodycap::CollectionCap {
                collection: format!("1:0x{c}"), max_items: n, max_value_wei: 15 * E, max_item_wei: 15 * E }).collect();
            let total = collections.iter().map(|c| c.max_items).sum();
            crate::custodycap::CapPolicy { collections, global_max_items: total, global_max_value_wei: 75 * E }
        };
        let launch = crate::outflow::Limits { per_collection_items: 12, global_items: 24, per_collection_mints: 3, global_mints: 6,
            global_value_wei: 25 * E, trip_factor: 3 };
        let j7 = Init { custody_caps: Some(caps([16, 22, 17, 4, 2])), outflow_limits: Some(launch.clone()), ..good() };
        let e = validate_custody(&j7).unwrap_err();
        assert!(e.contains("61 items in all") && e.contains("48 can leave") && e.contains("31 a day"), "{e}");
        // what fits: 48 items in all at 24 a day (the operator's options: fewer items, or a global cap of 48, or more a day later)
        let fits = Init { custody_caps: Some(caps([13, 16, 13, 4, 2])), outflow_limits: Some(launch.clone()), ..good() };
        assert_eq!(validate_custody(&fits), Ok(()));
        let capped = { let mut p = caps([16, 22, 17, 4, 2]); p.global_max_items = 48; Init { custody_caps: Some(p), outflow_limits: Some(launch), ..good() } };
        assert_eq!(validate_custody(&capped), Ok(()), "a global cap of 48 holds the same 61 per-collection caps");
        // the default install (no caps named: stage 0 for the wrapper and five Ethereum collections, 33 items) holds what stage 0's 6
        // a day let out: its total is 12
        let eth = crate::ethnft::EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None,
            logs_max_blocks: None, dormancy: None, collections: cols.iter().enumerate().map(|(i, c)| crate::ethnft::EthCollection { collection: format!("0x{c}"),
                wrapper: format!("0x{:040x}", 0xa0 + i), module: format!("0x{:040x}", 0xb0 + i), max_purchase_wei: 2 * E, max_premium_bps: 500, chain_id: None, token_bound: None, nft_value_bps: None }).collect() };
        let default = Init { eth_nft: Some(eth), ..with_wrapper() };
        assert_eq!(caps_of(&default).global_max_items, 12);
        assert_eq!(validate_custody(&default), Ok(()));
    }

    /// Audit C3-08: with no caps named, a Robinhood Chain collection starts at the launch file's bound (deployments/launch/
    /// robinhood-collections.json: custodyItems and custodyWei, one item at most its maxPurchaseWei), never at J7's Ethereum stage 0 (6 items,
    /// 15 ETH); an Ethereum collection beside it keeps J7's.
    #[test]
    fn fixed_c3_08_a_4663_collection_starts_at_the_launch_files_custody_bound() {
        let launch: serde_json::Value = serde_json::from_str(include_str!("../../../deployments/launch/robinhood-collections.json")).unwrap();
        let milady = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
        let mut cols = vec![crate::ethnft::EthCollection { collection: milady.into(), wrapper: "0x00000000000000000000000000000000000000a0".into(),
            module: "0x00000000000000000000000000000000000000b0".into(), max_purchase_wei: 2_000_000_000_000_000_000, max_premium_bps: 500, chain_id: None, token_bound: None,
            nft_value_bps: None }];
        for (i, c) in launch["collections"].as_array().unwrap().iter().enumerate() {
            let tb = c["tokenBound"].as_bool() == Some(true);
            cols.push(crate::ethnft::EthCollection { collection: c["collection"].as_str().unwrap().into(), wrapper: format!("0x{:040x}", 0xc0 + i),
                module: format!("0x{:040x}", 0xd0 + i), max_purchase_wei: c["maxPurchaseWei"].as_str().unwrap().parse().unwrap(), max_premium_bps: 500,
                chain_id: Some(4663), token_bound: tb.then_some(true), nft_value_bps: c["nftValueBps"].as_u64().map(|x| x as u16) });
        }
        let eth = crate::ethnft::EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None,
            logs_max_blocks: None, dormancy: None, collections: cols };
        let p = caps_of(&Init { eth_nft: Some(eth), ..with_wrapper() });
        for c in launch["collections"].as_array().unwrap() {
            let key = format!("4663:{}", c["collection"].as_str().unwrap());
            let cap = p.cap(&key).unwrap_or_else(|| panic!("{key} has a cap"));
            let want: (u32, u128, u128) = (c["custodyItems"].as_u64().unwrap() as u32, c["custodyWei"].as_str().unwrap().parse().unwrap(),
                c["maxPurchaseWei"].as_str().unwrap().parse().unwrap());
            assert_eq!((cap.max_items, cap.max_value_wei, cap.max_item_wei), want, "{key}");
        }
        let m = p.cap(&format!("1:{milady}")).unwrap();
        assert_eq!((m.max_items, m.max_value_wei), (crate::custodycap::STAGE0_EVM_ITEMS, crate::custodycap::FIRST_EVM_MAX_VALUE_WEI));
    }

    #[test]
    fn an_install_starts_at_stage_zero_at_most_and_an_upgrade_raises_by_four_at_most() {
        // stage 0's limits (3/6/3/6/10 ETH) raised ×2 then ×4: within the ×4 rule the limits follow too (audit D, I-5)
        let limits = |per: u32| crate::outflow::Limits { per_collection_items: per, global_items: 2 * per, per_collection_mints: per,
            global_mints: 2 * per, global_value_wei: per as u128 * crate::custodycap::FIRST_MAX_VALUE_WEI * 2 / 3, trip_factor: 3 };
        // install: stage 0 (by default or said), never more
        assert_eq!(check_cap_change(None, &with_wrapper()), Ok(()));
        let s1 = Init { custody_caps: Some(stage(4)), outflow_limits: Some(limits(6)), ..with_wrapper() };
        assert!(check_cap_change(None, &s1).unwrap_err().contains("stage 0"));
        // upgrade: x4 per proposal (stage 0 -> 1 -> 2), never x16 at once, and lowering is always allowed
        let s0 = with_wrapper();
        assert_eq!(check_cap_change(Some(&s0), &s1), Ok(()));
        let s2 = Init { custody_caps: Some(stage(16)), outflow_limits: Some(limits(24)), ..with_wrapper() };
        assert_eq!(check_cap_change(Some(&s1), &s2), Ok(()));
        assert!(check_cap_change(Some(&s0), &s2).unwrap_err().contains("×4"));
        assert_eq!(check_cap_change(Some(&s2), &s0), Ok(()), "stepping down is immediate and always allowed");
    }

    #[test]
    fn caps_that_holders_could_not_leave_within_the_delay_are_refused() {
        // stage 1's 12 items at stage 0's 3 a day: four days to leave, and the timelock gives three less one
        let slow = Init { custody_caps: Some(stage(4)), outflow_limits: None, ..with_wrapper() };
        assert!(validate_custody(&slow).unwrap_err().contains("could not leave"));
        let fast = Init { outflow_limits: Some(crate::outflow::Limits { per_collection_items: 6, global_items: 12,
            per_collection_mints: 6, global_mints: 12, global_value_wei: 20 * crate::custodycap::FIRST_MAX_VALUE_WEI / 10,
            trip_factor: 3 }), ..slow };
        assert_eq!(validate_custody(&fast), Ok(()));
        // and a malformed policy or limit is refused at install
        let dup = Init { custody_caps: Some(crate::custodycap::CapPolicy { collections: vec![stage(1).collections[0].clone(),
            stage(1).collections[0].clone()], ..stage(1) }), ..with_wrapper() };
        assert!(validate_custody(&dup).unwrap_err().contains("twice"));
        let trip1 = Init { outflow_limits: Some(crate::outflow::Limits { trip_factor: 1, ..crate::outflow::Limits::stage0() }), ..with_wrapper() };
        assert!(validate_custody(&trip1).unwrap_err().contains("trip factor"));
    }

    #[test]
    #[should_panic(expected = "vault's own chain")]
    fn an_install_that_lists_the_vaults_own_chain_as_an_nft_chain_is_refused() {
        let mut home = crate::chains::tests::chain(4663);
        home.chain_id = 4663;
        validate(&Init { evm_chains: Some(vec![home]), ..good() });
    }

    pub(crate) fn good() -> Init {
        Init {
            chain_id: 4663,
            vault: "0x0000000000000000000000000000000000000001".into(),
            rpc_urls: vec!["https://a".into(), "https://b".into()],
            btc_api_urls: vec!["https://c".into(), "https://d".into()],
            key_name: "dfx_test_key".into(),
            accept_production_key_name: false,
            bridge_allowlist: vec![],
            btc_network: "signet".into(),
            operator: None,
            wrapper: None,
            evm_chains: None,
            custody_caps: None,
            outflow_limits: None,
            holder_deposits: None,
            wrapper_from_block: None, frog_funding: None, eth_nft: None, home: None,
        }
    }

    #[test]
    #[should_panic(expected = "an operator anyone can be")]
    fn the_anonymous_principal_is_refused_as_operator() {
        validate(&Init { operator: Some(candid::Principal::anonymous()), ..good() });
    }

    #[test]
    #[should_panic(expected = "production threshold key's name")]
    fn the_production_key_name_is_refused_unless_the_installer_said_it_was_deliberate() {
        let mut a = good();
        a.key_name = PRODUCTION_KEY.into();
        init(a);
    }

    #[test]
    fn keccak_matches_the_empty_string_vector_every_implementation_agrees_on() {
        assert_eq!(
            hex::encode(keccak(b"")),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn an_evm_address_is_derived_from_a_decompressed_point() {
        // secp256k1's generator, compressed. Its uncompressed form is the well-known Gx‖Gy.
        let g = hex::decode("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798").unwrap();
        let full = decompress(&g).unwrap();
        assert_eq!(full.len(), 64, "x and y, with no prefix byte");
        assert_eq!(
            hex::encode(&full[..32]),
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        );
        assert_eq!(
            hex::encode(&full[32..]),
            "483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"
        );
        // and the address is the last 20 bytes of its keccak, which is what evm_address() returns
        assert_eq!(hex::encode(&keccak(&full)[12..]).len(), 40);
        assert!(decompress(b"not a point").is_err());
    }

    /// The production install argument (docs/v6/phase1-scope.md §3.2, K2): chain 4663, Bitcoin mainnet, `key_1` said to be
    /// deliberate, two providers each side, stage 0. It validates; so do the rehearsal and testnet installs, as before.
    #[test]
    fn the_production_install_for_bitcoin_mainnet_validates_and_the_test_networks_still_do() {
        let prod = Init { chain_id: PRODUCTION_EVM_CHAIN, btc_network: BITCOIN_MAINNET.into(), key_name: PRODUCTION_KEY.into(),
            accept_production_key_name: true,
            rpc_urls: vec!["https://rpc.mainnet.chain.robinhood.com".into(), "https://rpc.ordofi.network".into()],
            btc_api_urls: vec!["https://mempool.space/api".into(), "https://blockstream.info/api".into()], ..good() };
        validate(&prod);
        for n in ["signet", "testnet4"] {
            validate(&Init { btc_network: n.into(), ..good() });
            validate(&Init { btc_network: n.into(), chain_id: REHEARSAL_EVM_CHAIN, ..good() });
        }
        // every accepted network has an address prefix and a checkpoint; nothing is sent or believed on one without
        for n in KNOWN_NETWORKS {
            assert!(crate::policy::hrp_for(n).is_ok() && crate::btcsend::checkpoint(n).is_some(), "{n}");
        }
        assert_eq!(crate::policy::hrp_for(BITCOIN_MAINNET), Ok("bc"));
        // the mainnet-only paths match the very string the install accepts
        for (file, src) in [("ordnet.rs", include_str!("ordnet.rs")), ("oracle.rs", include_str!("oracle.rs"))] {
            assert!(src.contains("crate::canister::BITCOIN_MAINNET"), "{file} names mainnet through the constant");
            assert!(!src.contains("btc_network == \"mainnet\"") && !src.contains("network != \"mainnet\""), "{file}: no second spelling");
        }
    }

    #[test]
    fn a_near_miss_for_mainnet_is_refused_at_install() {
        for n in ["bitcoin", "Mainnet", "MAINNET", "main", "mainnet ", " mainnet", "testnet", "regtest", ""] {
            let r = std::panic::catch_unwind(|| validate(&Init { btc_network: n.into(), ..good() }));
            assert!(r.is_err(), "{n:?} must be refused");
        }
    }

    /// Every rehearsal path is dead on a production install (4663 and Bitcoin mainnet), whatever the caller names; and still
    /// alive on the rehearsal install it exists for.
    #[test]
    fn every_rehearsal_guard_refuses_a_production_install() {
        // the Bitcoin half: rehearse_sign_sighash and rehearse_broadcast_btc
        for asked in [BITCOIN_MAINNET, REHEARSAL_NETWORK, "testnet4", ""] {
            let e = check_btc_rehearsal(BITCOIN_MAINNET, asked, "signs").unwrap_err();
            assert!(e.contains("installed for \"mainnet\""), "{e}");
        }
        assert!(check_btc_rehearsal(REHEARSAL_NETWORK, BITCOIN_MAINNET, "signs").is_err(), "the caller naming mainnet");
        assert!(check_btc_rehearsal("testnet4", REHEARSAL_NETWORK, "sends").is_err());
        assert_eq!(check_btc_rehearsal(REHEARSAL_NETWORK, REHEARSAL_NETWORK, "signs"), Ok(()), "the rehearsal still works");
        // the EVM half: rehearse_sign_evm and rehearse_broadcast_evm
        for asked in [PRODUCTION_EVM_CHAIN, REHEARSAL_EVM_CHAIN] {
            assert!(check_evm_rehearsal_chain(PRODUCTION_EVM_CHAIN, asked).is_err());
        }
        assert_eq!(check_evm_rehearsal_chain(REHEARSAL_EVM_CHAIN, REHEARSAL_EVM_CHAIN), Ok(()));
        // the fixtures
        assert!(!fixtures_allowed(PRODUCTION_EVM_CHAIN, BITCOIN_MAINNET));
        assert!(!fixtures_allowed(REHEARSAL_EVM_CHAIN, BITCOIN_MAINNET), "mainnet Bitcoin alone refuses them");
        assert!(!fixtures_allowed(PRODUCTION_EVM_CHAIN, REHEARSAL_NETWORK), "4663 alone refuses them");
        assert!(fixtures_allowed(REHEARSAL_EVM_CHAIN, REHEARSAL_NETWORK));
        // and every rehearsal update method in the canister actually calls one of those guards, before anything else it does
        let sources = [("canister.rs", include_str!("canister.rs")), ("btcnet.rs", include_str!("btcnet.rs"))];
        let mut seen = 0;
        for (file, src) in sources {
            for part in src.split(concat!("#[ic_cdk::", "update]")).skip(1) {
                let head: String = part.chars().take(700).collect();
                let Some(name) = head.split("fn ").nth(1).and_then(|x| x.split('(').next()) else { continue };
                if !name.starts_with("rehearse_") {
                    continue;
                }
                seen += 1;
                let guarded = ["check_btc_rehearsal(", "check_evm_rehearsal_chain(", "fixtures_allowed(", "!= REHEARSAL_NETWORK"]
                    .iter().any(|g| head.contains(g));
                assert!(guarded, "{file}: {name} has no network guard in its first lines");
            }
        }
        assert_eq!(seen, 6, "rehearse_fixtures, _sign_sighash, _open_purchase, _sign_evm, _broadcast_evm, _broadcast_btc");
    }

    #[test]
    #[should_panic(expected = "installs for")]
    fn an_install_that_names_no_network_at_all_is_refused_too() {
        let mut a = good();
        a.btc_network = String::new();
        init(a);
    }

    #[test]
    fn the_rehearsal_network_is_an_allowlist_of_one_and_is_not_the_production_key_name() {
        // The guard is positive, not negative: the only string that passes is the one we named.
        assert!(KNOWN_NETWORKS.contains(&REHEARSAL_NETWORK));
        assert_ne!(REHEARSAL_NETWORK, BITCOIN_MAINNET, "the rehearsal network is never mainnet");
        assert!(!KNOWN_NETWORKS.contains(&"bitcoin"));
        assert!(!KNOWN_NETWORKS.contains(&""));
        // a near-miss a denylist would have waved through
        assert!(!KNOWN_NETWORKS.contains(&"signet "));
        assert_ne!(REHEARSAL_NETWORK, PRODUCTION_KEY);
    }


    // ---- the recovery parameter, which nothing in this repository used to have ---------------------------------------------

    /// The EIP-155 example key. Its address is stated in the EIP itself, so this vector comes from outside this repository.
    const EIP155_KEY: [u8; 32] = [0x46; 32];
    const EIP155_ADDRESS: &str = "0x9d8a62f656a8d1615c1294fd71e9cfb3e4855a4f";

    fn sign_locally(key: &[u8; 32], hash: &[u8; 32]) -> ([u8; 32], [u8; 32], u8) {
        use k256::ecdsa::SigningKey;
        let sk = SigningKey::from_bytes(key.into()).unwrap();
        let (sig, rid) = sk.sign_prehash_recoverable(hash).unwrap();
        (sig.r().to_bytes().into(), sig.s().to_bytes().into(), rid.to_byte() & 1)
    }

    #[test]
    fn the_recovery_id_picks_a_public_key_and_the_other_parity_picks_a_different_one() {
        // The whole reason `v` exists: two keys satisfy any (r, s), and only one of them is the signer. A wrong `v` does not
        // fail -- it succeeds, at somebody else's address.
        let hash = keccak(b"a transaction that has to come from us");
        let (r, s, rid) = sign_locally(&EIP155_KEY, &hash);
        assert_eq!(
            recover_evm_address(&hash, &r, &s, rid).as_deref(),
            Some(EIP155_ADDRESS),
            "the right parity recovers the key that signed"
        );
        let other = recover_evm_address(&hash, &r, &s, rid ^ 1);
        assert_ne!(
            other.as_deref(),
            Some(EIP155_ADDRESS),
            "the wrong parity must NOT recover the signer -- if it did, `v` would carry no information"
        );
        assert!(other.is_some(), "and it recovers a perfectly valid other address, which is exactly the danger");
    }

    #[test]
    fn a_hardcoded_recovery_id_would_be_wrong_about_half_the_time() {
        // This is the test that says why the parity must be searched for rather than written down. If `v` were a constant,
        // roughly half of all transactions would be charged to an address nobody holds -- and every test that verified the
        // signature against the key it signed with would still pass.
        let mut zero = 0usize;
        let mut one = 0usize;
        for i in 0u32..64 {
            let hash = keccak(&i.to_be_bytes());
            let (r, s, rid) = sign_locally(&EIP155_KEY, &hash);
            assert_eq!(recover_evm_address(&hash, &r, &s, rid).as_deref(), Some(EIP155_ADDRESS));
            if rid == 0 { zero += 1 } else { one += 1 }
        }
        assert!(zero > 8 && one > 8, "both parities occur in the wild: {zero} zeros, {one} ones out of 64");
    }

    #[test]
    fn a_high_s_signature_recovers_nothing_at_all_which_is_why_normalisation_comes_first() {
        use k256::ecdsa::Signature;
        // Measured, not assumed. For a signature whose `s` is in the upper half of the curve order, k256's recovery
        // refuses **both** parities -- it does not quietly hand back the other key. So the ordering inside
        // `sign_evm_recoverable` has a specific consequence worth writing down: if the subnet ever returns a high `s` and
        // we failed to normalise it, the parity search would find nothing and the call would return the "neither parity
        // recovers" error. That is a loud failure rather than a wrong address, which is the right way round.
        //
        // What is NOT safe is the reverse order. `s` and `n - s` are both valid signatures for the same message under
        // opposite parities, so negating `s` after fixing `v` leaves a signature that recovers -- to somebody else. That
        // case is the one the test above pins down, and it is live: both of the low-`s` parities recover successfully and
        // only one of them is us.
        let hash = keccak(b"order of operations");
        let (r, s_low, rid) = sign_locally(&EIP155_KEY, &hash);
        let sig = Signature::from_scalars(r, s_low).unwrap();
        assert!(sig.normalize_s().is_none(), "k256 signs low already; this is the baseline");

        let s_high: [u8; 32] = (-*sig.s()).to_bytes().into();
        let high = Signature::from_scalars(r, s_high).unwrap();
        assert!(high.normalize_s().is_some(), "and this one is high, so normalisation has something to do");
        assert_eq!(
            <[u8; 32]>::from(high.normalize_s().unwrap().s().to_bytes()),
            s_low,
            "normalising the high form gives back exactly the low form: no information is lost, only malleability"
        );

        for parity in 0u8..=1 {
            assert_eq!(
                recover_evm_address(&hash, &r, &s_high, parity),
                None,
                "a high-s signature recovers nothing under parity {parity}"
            );
        }
        // …while the normalised form recovers under both, and only one of the two is us. That asymmetry is the danger.
        assert!(recover_evm_address(&hash, &r, &s_low, 0).is_some());
        assert!(recover_evm_address(&hash, &r, &s_low, 1).is_some());
        assert_eq!(recover_evm_address(&hash, &r, &s_low, rid).as_deref(), Some(EIP155_ADDRESS));
        assert_ne!(recover_evm_address(&hash, &r, &s_low, rid ^ 1).as_deref(), Some(EIP155_ADDRESS));
    }

    #[test]
    fn a_signature_that_is_not_ours_yields_no_address_rather_than_a_plausible_one() {
        // `sign_evm_recoverable` refuses when neither parity is ours. This is the arithmetic behind that refusal.
        let hash = keccak(b"whose signature is this");
        let (r, s, _) = sign_locally(&EIP155_KEY, &hash);
        let someone_else = keccak(b"a different message entirely");
        for rid in 0u8..=1 {
            assert_ne!(
                recover_evm_address(&someone_else, &r, &s, rid).as_deref(),
                Some(EIP155_ADDRESS),
                "a signature over one hash must not recover our address over another"
            );
        }
        assert_eq!(recover_evm_address(&hash, &r, &s, 2), None, "there are two parities, not four");
        assert_eq!(recover_evm_address(&hash, &[0u8; 32], &s, 0), None, "r = 0 is not a signature");
    }

    #[test]
    fn the_evm_rehearsal_chain_is_an_allowlist_of_exactly_one() {
        assert_eq!(check_evm_rehearsal_chain(REHEARSAL_EVM_CHAIN, REHEARSAL_EVM_CHAIN), Ok(()));

        // Mainnet, by name, from either side.
        let by_caller = check_evm_rehearsal_chain(REHEARSAL_EVM_CHAIN, PRODUCTION_EVM_CHAIN).unwrap_err();
        assert!(by_caller.contains("MAINNET"), "{by_caller}");
        assert!(by_caller.contains("Never"), "{by_caller}");
        assert!(check_evm_rehearsal_chain(PRODUCTION_EVM_CHAIN, REHEARSAL_EVM_CHAIN).is_err(), "install side too");

        // And every near miss a denylist of "not 4663" would have waved straight through.
        for typo in [0u64, 1, 4663, 4630, 46_63, 46631, 46629, 466_30_0, u64::MAX] {
            if typo == REHEARSAL_EVM_CHAIN {
                continue;
            }
            assert!(
                check_evm_rehearsal_chain(REHEARSAL_EVM_CHAIN, typo).is_err(),
                "chain {typo} must be refused; only {REHEARSAL_EVM_CHAIN} is accepted"
            );
        }
        assert_ne!(REHEARSAL_EVM_CHAIN, PRODUCTION_EVM_CHAIN);
    }

    #[test]
    #[should_panic(expected = "must be distinct")]
    fn the_same_rpc_provider_listed_twice_is_not_two_providers() {
        // `rpc()` accepts an answer only when every provider returns an identical body. One URL listed twice always
        // satisfies that, so the count passed while the check it stands for did not exist.
        let mut a = good();
        a.rpc_urls = vec!["https://a".into(), "https://a".into()];
        init(a);
    }

    #[test]
    fn distinct_is_about_values_not_lengths() {
        assert!(distinct(&["a".to_string(), "b".to_string()]));
        assert!(!distinct(&["a".to_string(), "a".to_string()]));
        assert!(!distinct(&["a".to_string(), "b".to_string(), "a".to_string()]));
        assert!(distinct::<>(&[]));
    }

    #[test]
    #[should_panic(expected = "two RPC providers")]
    fn one_rpc_provider_is_not_a_check() {
        let mut a = good();
        a.rpc_urls = vec!["https://only-one".into()];
        init(a);
    }

    /// `taptree::output_key` with no script tree is exactly the tweak the float and custody keys get, which it generalises.
    #[test]
    fn the_taptree_key_path_tweak_is_tweak_xonly() {
        for x in ["79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798", crate::taptree::NUMS.map(|b| format!("{b:02x}")).concat().as_str()] {
            let x: [u8; 32] = hex::decode(x).unwrap().try_into().unwrap();
            assert_eq!(crate::taptree::output_key(&x, None).unwrap().0, super::tweak_xonly(x).unwrap());
        }
    }
}


// The Candid interface of every endpoint (`ic_cdk::export_candid!` at the end of this file spells each signature's types as written
// in its module): the names other modules' endpoints use unqualified, in scope here.
#[allow(unused_imports)]
use crate::{
    books::{self, ledger::ListingKind, params::Params},
    btcnet::{BtcSeen, BtcSend},
    chainlink::recon::Report,
    custody,
    desk::api::{DeskStatus, IntentView, LinkView, OpenKind, Opened},
    desk::quote::{BurnQuote, MintQuote},
    ethflow::EthState,
    ethnft::EthNftConfig,
    floortrigger::FloorState,
    frogbuy::FrogRec,
    frognet::{FrogCollection, ProtectedListing, ProtectedSigned},
    handin::{BoardView, VerdictRec},
    itemsink::Progress,
    mintdesk::ReqRec,
    nftnet::{NftChainStatus, NftPlan},
    raffle::SnapshotPage,
    redeem::Redemption,
    sale::SaleState,
    solhome::{LineId, SolLine, TimerSeen},
};
#[allow(unused_imports)]
use crate::deposits::DepositRec;
#[allow(unused_imports)]
use crate::rates::Rate;

// The Candid interface of every endpoint, for `candid-extractor` (scripts: `candid-extractor target/wasm32-unknown-unknown/release/etch_autobuyer.wasm`).
ic_cdk::export_candid!();
