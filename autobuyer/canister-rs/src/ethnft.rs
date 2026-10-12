//! Phase 1 on Ethereum NFTs (docs/v7/phase1-nft.md): the rules, pure. The configuration and its validation; the floor from
//! verified Seaport listings bounded by verified on-chain sales (§4.2); the buy bound on top of `seaport.rs` (§4.3); the transfer
//! and deposit judges (§2.4, §4.4); the `EthNftWrapper` and `FloorModule` calls. No network, no keys, no clock: the flow
//! (`ethflow.rs`) feeds in what the chains said at their pins.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::abi::{self, Arg};
use crate::seaport::{self as sp, Word};

// ---- configuration ---------------------------------------------------------------------------------------------------------------

/// The floor's parameters (§4.2). `None` in the argument is [`FloorParams::default`].
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloorParams {
    /// Distinct offerers with a verified listing, at least.
    pub min_listings: u8,
    /// The floor is the `listing_rank`-th cheapest of them (1 = the cheapest).
    pub listing_rank: u8,
    /// Verified sales in the window, at least.
    pub min_sales: u8,
    /// The post is never above the sale median by more than this.
    pub up_cap_bps: u16,
    /// Nor below the sales' 25th percentile by more than this.
    pub down_cap_bps: u16,
}

impl Default for FloorParams {
    fn default() -> Self {
        FloorParams { min_listings: 3, listing_rank: 2, min_sales: 3, up_cap_bps: 1_000, down_cap_bps: 1_500 }
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EthCollection {
    /// The ERC-721 on `chain_id`.
    pub collection: String,
    /// Its `EthNftWrapper` on the home chain.
    pub wrapper: String,
    /// Its `FloorModule` on the home chain.
    pub module: String,
    /// The most one purchase may cost.
    pub max_purchase_wei: u128,
    /// A buy's total is at most our floor × (1 + this).
    pub max_premium_bps: u16,
    /// The EVM chain the ERC-721 lives on (2026-10-12, Robinhood Chain NFTs): one of `evm_chains`, and the chain of every step of this
    /// collection (its reads, its Seaport 1.6 fill, its gas pot, its Relay legs, its deliveries). `None`: `EthNftConfig.chain_id` (an
    /// argument from before it, or a collection on the config's own chain).
    #[serde(default)]
    pub chain_id: Option<u64>,
    /// 2026-10-12: each NFT of the collection is a token-bound account (ERC-6551: StonkBrokers on Robinhood Chain, whose accounts hold
    /// tokenized stocks). The canister prices and buys it as an NFT only: its floor is the NFT's Seaport listings and sales, a buy is the
    /// one ERC-721 at most the collection's own `max_purchase_wei`, never a valuation of what the account holds (its seller can drain it
    /// before the sale); it never calls, values or moves the account's assets; a delivery is the ERC-721 `transferFrom` alone (the
    /// account and whatever it holds then go with the NFT). `None`: false.
    #[serde(default)]
    pub token_bound: Option<bool>,
    /// Audit C3-01: for a token-bound collection, the share (bps, in (0, 10,000)) of its market price that is the NFT ALONE (an empty
    /// account). The listings and sales its floor is read from are prices of NFTs whose accounts hold what their sellers left in them,
    /// which no rule of the canister reads (a seller can drain the account before the fill), so every price of the collection is taken
    /// at this share before the floor is cut: the floor posted, the module's value and the buy bound are the NFT's, never the account's.
    /// A drained account listed at the floor of full ones is then over the bound; a full one is bought only as cheap as a bare NFT.
    /// Required with `token_bound`, refused without it. `None`: the whole price (an NFT with no account).
    #[serde(default)]
    pub nft_value_bps: Option<u16>,
}

impl EthCollection {
    /// Whether the collection's NFTs are token-bound accounts ([`EthCollection::token_bound`]).
    pub fn is_token_bound(&self) -> bool {
        self.token_bound.unwrap_or(false)
    }
    /// Audit C3-01: the share of a price the canister counts for this collection's NFT: `nft_value_bps` for a token-bound one (0, so
    /// nothing is ever bought, when it is missing: `validate` refuses that), the whole price otherwise.
    pub fn nft_bps(&self) -> u16 {
        if self.is_token_bound() { self.nft_value_bps.unwrap_or(0) } else { 10_000 }
    }
}

/// The install's `eth_nft` (§4.1). Operational: taken from every upgrade's argument.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EthNftConfig {
    pub chain_id: u64,
    pub collections: Vec<EthCollection>,
    pub relay_max_fee_bps: u16,
    pub deposits: bool,
    pub floor: Option<FloorParams>,
    /// A2-L-1 (2026-09-27): the gas pot's target at our Ethereum address; above it, the collections' leftover is swept back to their
    /// modules. `None`: `GAS_POT_TARGET_WEI`. Within [`GAS_POT_TARGET_MIN_WEI`, `GAS_POT_TARGET_MAX_WEI`].
    #[serde(default)]
    pub gas_pot_target_wei: Option<u128>,
    /// The widest `eth_getLogs` range the sale scan asks (the operator's Dwellir plan, 2026-09-27: 500). `None`:
    /// `SALE_SCAN_CHUNK_BLOCKS`.
    #[serde(default)]
    pub logs_max_blocks: Option<u64>,
    /// docs/v7/dormant-collections.md: the dormant / wake-on-demand life cycle, its caps and the `WakeDesk`. `None`: every collection
    /// is ACTIVE all the time (as before).
    #[serde(default)]
    pub dormancy: Option<crate::dormancy::Dormancy>,
}

/// One collection, parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Col {
    pub collection: [u8; 20],
    /// The wrapper and the module as the canister's records key them: their EVM addresses, or on the Solana home (the books) keys
    /// derived from the collection's address.
    pub wrapper: [u8; 20],
    pub module: [u8; 20],
    pub max_purchase_wei: u128,
    pub max_premium_bps: u16,
    /// W8: the canister keeps the books (`EthCollection.wrapper` and `.module` are both `"books"`, the only form the Solana home
    /// accepts: decision D-l): the record keys above are derived from the collection's address and the ledger is `desk::state`'s.
    pub books: bool,
    /// Audit C3-07: the chain the ERC-721 lives on (`EthNftConfig::chain_of`): the books' listing of the collection is the one naming
    /// this chain AND the address.
    pub chain_id: u64,
    /// Audit C3-01: the share of a listing's or a sale's price counted for the NFT (`EthCollection::nft_bps`): 10,000 unless token-bound.
    pub nft_bps: u16,
}

pub const MAX_PREMIUM_BPS: u16 = 1_000;
/// Listings per hand-in, at most (so one evidence pass's outcalls are bounded). `MAX_SALES` bounds the sale hashes a hand-in may
/// carry, which the canister no longer uses (audit 2026-09-27-nft round 2, A-M-1 / B2-M-1: it reads the collection's sales itself).
pub const MAX_LISTINGS: usize = 8;
pub const MAX_SALES: usize = 12;
/// A sale counts if its block is within this many blocks of the pin: 7 days of 12 s blocks.
pub const SALE_WINDOW_BLOCKS: u64 = 50_400;
/// Kept at our Ethereum address from each purchase: its own gas, and a floor for the gas pot (§4.3).
pub const ETH_GAS_RESERVE_WEI: u128 = 3_000_000_000_000_000;
/// A2-L-1 (the operator, 2026-09-27): the gas pot's default target (D3: seeded at 0.01 ETH, paged under 0.005), and its bounds.
pub const GAS_POT_TARGET_WEI: u128 = 50_000_000_000_000_000;
pub const GAS_POT_TARGET_MIN_WEI: u128 = 10_000_000_000_000_000;
pub const GAS_POT_TARGET_MAX_WEI: u128 = 1_000_000_000_000_000_000;
/// Our own floor reading is used for a buy bound only this long.
pub const READING_TTL_SECS: u64 = 3_600;
/// Post when the module's post is this old, or the price moved by `POST_MOVE_BPS`.
pub const POST_EVERY_SECS: u64 = 1_800;
pub const POST_MOVE_BPS: u128 = 100;
/// Deliveries are not signed this close to `deliverBy`; one still pending then is cancelled (§2.2).
pub const DELIVERY_STOP_SECS: u64 = 86_400;
/// A sale further than this from the listing floor `L` (either way) is not the same market (a dust side of a match, a trait sale,
/// a wash at a far price), and is not counted (audit 2026-09-27-nft, B-H-1).
pub const SALE_BAND_BPS: u128 = 3_000;
/// A deposit is minted only while the module's accepted floor is at most this far over the LOWER of the listing floor and the 25th
/// percentile of the sales the canister counted itself (audit 2026-09-27-nft A-M-1, round 2): above it, a deposit then a burn
/// would sell the module an NFT at a floor the market has not traded at, and the deposit waits (never refused, never returned).
pub const DEPOSIT_FLOOR_BAND_BPS: u128 = 500;
/// The confidence of a HELD post (audit 2026-09-27-nft round 2, B2-M-1): the listings are not confirmed by enough counted sales,
/// so the post keeps the module fresh without letting it rise. The module's `minConfidenceBps` is 3,000 (StackChecks' default):
/// a held post is a good post, priced with the module's low-confidence spread.
pub const HELD_CONFIDENCE: u16 = 5_000;
/// The canister's own sale sample (round 2, A-M-1 / B2-M-1 / D2-M-2): the collection's `Transfer` logs read by the canister, final
/// blocks only (`FINAL_DEPTH` under the pin), a chunk of blocks per read (halved on a failure, down to the minimum, and grown
/// back), a few chunks and receipts a pass, newest first; every final verdict kept, so each transaction is read once.
pub const SALE_FINAL_DEPTH_BLOCKS: u64 = 128;
/// The widest `eth_getLogs` range the sale scan asks, by default: the operator's Ethereum provider (Dwellir's plan, 2026-09-27)
/// serves at most 500 blocks a request. `eth_nft.logs_max_blocks` sets another, within [`SALE_SCAN_MIN_CHUNK_BLOCKS`,
/// `SALE_SCAN_MAX_CHUNK_BLOCKS`].
pub const SALE_SCAN_CHUNK_BLOCKS: u64 = 500;
pub const SALE_SCAN_MAX_CHUNK_BLOCKS: u64 = 2_000;
pub const SALE_SCAN_MIN_CHUNK_BLOCKS: u64 = 25;
pub const SALE_SCAN_CHUNKS_PER_PASS: usize = 13;
pub const SALE_RECEIPTS_PER_PASS: usize = 16;
pub const SALE_TODO_MAX: usize = 512;
/// Review round 4 (B4-H-1): each walk (the newest blocks, back to the window's start, a re-read of dropped blocks) stops for the pass
/// after this many failed reads. Review round 5 (B5-M-1): only a read that FAILED counts (a provider error, a disagreement); a range
/// too big to read is never a failure of the walk: it is read through the marketplaces' own sale logs, or deferred, and the walk goes
/// on past it, so a supply of oversized blocks cannot outpace the scan.
pub const SALE_SCAN_FAILS_PER_WALK: u32 = 2;
/// Review round 5 (B5-M-1): the refinement of deferred ranges (`SaleScan.bad`: read through the marketplaces' logs or not at all,
/// bisected later through their `Transfer` logs for every other sale) stops for the pass after this many too-big or failed reads.
/// It runs after the walks, newest range first, on what is left of the pass's budget: it never holds a walk up.
pub const SALE_REFINE_FAILS_PER_PASS: u32 = 2;
/// Review round 5 (B5-M-2): a read that fails this many times in a row at the same range start, whatever the error says, is taken as
/// a range the provider will not serve whole (a result cap, a timeout on a dense range, an error of its own): deferred like an
/// oversized one, never asked the same way for the rest of the window.
pub const SALE_SCAN_STUCK_FAILS: u32 = 6;
/// Review round 5 (B5-M-1): at most this many deferred ranges are kept (the oldest, whose sales leave the window first, go), and this
/// many oversized blocks already read through their marketplace logs are remembered, so a re-read or a reset never bisects one
/// again (B4-L-2).
pub const SALE_BAD_MAX: usize = 32;
pub const SALE_DONE_BIG_MAX: usize = 64;
/// Round 4 (B4-H-1), review round 5 (B5-L-1): the response bytes one collection's scan may reserve a pass (every size class tried, on
/// every provider), now a real bound: a log read is STARTED only when its worst case (`EthIo::logs_read_worst`: every class on every
/// provider, 4.928 MB on two) still fits, and the marketplace read inside one is checked the same way. 10 MB fits one oversized read
/// and its marketplace read (9.856 MB); the initial window read of a quiet collection (≈ 100 reads at the 64 kB class) fits a few
/// passes.
pub const SALE_SCAN_BYTES_PER_PASS: u64 = 10_000_000;
/// Review round 5 (B5-L-1): the receipts the scan reads a pass (both size classes on every provider: `EthIo::receipt_read_worst`,
/// 464 kB on two) have their own bound, so a pass costs at most 14 MB of reserved response in all (≈ 146 B cycles at 13 nodes), and
/// the log reads can never starve the judging of what was found.
pub const SALE_RECEIPT_BYTES_PER_PASS: u64 = 4_000_000;
/// Review round 5 (B5-L-1): the passes are the runner's (each floor hand-in), so the scan is also bounded per day: 72 passes' worth a
/// collection (the runner's 20-minute cadence), ≈ 10.5 T cycles a day a collection at worst; past it the scan waits for the next day
/// and the floor is still posted from what was read.
pub const SALE_SCAN_BYTES_PER_DAY: u64 = 72 * (SALE_SCAN_BYTES_PER_PASS + SALE_RECEIPT_BYTES_PER_PASS);
/// Round 4 (B4-H-1): one block over the largest class whose marketplace logs could not be read either is tried again this much later
/// (times the tries), at most `SALE_SKIP_TRIES` times; it is never read every pass.
pub const SALE_SKIP_RETRY_SECS: u64 = 6 * 3_600;
/// Review round 6 (B5-L-2): the retry interval grows with the tries up to this many times `SALE_SKIP_RETRY_SECS` (18 h), and a
/// skipped block is never given up while it is in the window (it was, after 3 tries: its sales lost for good). A rate limit is no try.
pub const SALE_SKIP_TRIES: u32 = 3;
/// Review round 6 (B6-M-1): a forward walk this far behind the final head (a day of blocks: the runner down, the cycles under the
/// evidence floor, a pause), or refused because the providers keep no logs that deep, re-anchors at the newest final blocks. The
/// blocks it passes over (`SaleScan.hole`) are read newest first by a walk of their own; what the providers can no longer serve is
/// recorded (`SaleScan.lost`) and paged. It never freezes waiting for the window to reset.
pub const SALE_REANCHOR_LAG_BLOCKS: u64 = 7_200;
/// Review round 6 (B6-L-1): a receipt that fails (a provider answers `null`, the providers disagree) is retried at once this many
/// times, then every `SALE_RECEIPT_RETRY_SECS` (times the tries past it, at most 4), and such retries take at most
/// `SALE_RECEIPT_RETRY_READS` of a pass's `SALE_RECEIPTS_PER_PASS`: an unreadable receipt can no longer hold every newer sale.
pub const SALE_RECEIPT_FAST_TRIES: u32 = 3;
pub const SALE_RECEIPT_RETRY_SECS: u64 = 6 * 3_600;
pub const SALE_RECEIPT_RETRY_READS: usize = 4;
/// Review round 7 (B7-L-1): a re-anchor's hole refused as deeper than the providers keep logs (both providers saying so) is read
/// again at once this many times, then every `SALE_DEEP_RETRY_SECS`, until it is read or leaves the window: never given up for good
/// on an answer. Paged (`SaleScan.hole_deep`).
pub const SALE_DEEP_FAST_TRIES: u32 = 3;
pub const SALE_DEEP_RETRY_SECS: u64 = 6 * 3_600;
/// Review round 7 (B7-L-2): a receipt answered as a rate limit is read again after `SALE_BUSY_RETRY_SECS` × 2^(answers − 1), at most
/// `SALE_BUSY_RETRY_MAX_SECS`, and only with the retries' share of a pass (`SALE_RECEIPT_RETRY_READS`): old busy receipts never hold a
/// newer sale. The monitor pages a receipt answered busy `SALE_BUSY_PAGE_TRIES` times in a row, and a scan that met a rate limit in
/// `SALE_BUSY_PAGE_PASSES` passes in a row (≈ 3 h at the runner's 20-minute cadence).
pub const SALE_BUSY_RETRY_SECS: u64 = 600;
pub const SALE_BUSY_RETRY_MAX_SECS: u64 = 6 * 3_600;
pub const SALE_BUSY_PAGE_TRIES: u32 = 6;
pub const SALE_BUSY_PAGE_PASSES: u32 = 9;
/// Review round 7 (B7-L-3): the monitor pages the scan's lag (the final head less `SaleScan.current`) SEV2 past this many final
/// blocks (8 h), SEV1 past `SALE_LAG_SEV1_BLOCKS` (a day). A healthy scan is at most a pass (≈ 100 blocks) plus a chunk behind.
pub const SALE_LAG_SEV2_BLOCKS: u64 = 2_400;
pub const SALE_LAG_SEV1_BLOCKS: u64 = 7_200;

/// Audit C3-02: the block counts above are ETHEREUM's (12 s blocks). Each chain's side cuts its sale window, its final depth, its
/// re-anchor lag, its lag pages and its scan budget in ITS OWN blocks, from these spans in seconds and the chain's block time
/// (`chains::block_ms`): on Robinhood Chain (0.104 s blocks) the 7-day window is 5,815,384 blocks, never Ethereum's 50,400 (87 minutes
/// there). For Ethereum they are exactly the constants above.
pub const SALE_WINDOW_SECS: u64 = 7 * 86_400;
pub const SALE_FINAL_DEPTH_SECS: u64 = 1_536;
pub const SALE_REANCHOR_LAG_SECS: u64 = 86_400;
pub const SALE_LAG_SEV2_SECS: u64 = 8 * 3_600;
pub const SALE_LAG_SEV1_SECS: u64 = 86_400;
/// Audit C3-02: the runner's evidence cadence (20 minutes): a pass's forward walk must cover at least `SALE_SCAN_PASS_COVER` times the
/// chain's blocks of one cadence, or it falls behind and re-anchors every pass.
pub const SALE_SCAN_PASS_SECS: u64 = 1_200;
pub const SALE_SCAN_PASS_COVER: u64 = 3;
/// Audit C3-02: a collection's FIRST post (the module's accepted floor 0: nothing to hold it to) needs its `min_sales` counted sales
/// (one per token and per wallet, no self-trade or round trip: `counted_sales`) spread over at least this long of the chain's blocks:
/// a burst of wash sales between fresh wallets inside one hour is never a first price, on any chain.
pub const FIRST_POST_SPAN_SECS: u64 = 86_400;

/// Audit C3-02: the floor's block counts on one chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaleBlocks {
    pub window: u64,
    pub final_depth: u64,
    pub reanchor_lag: u64,
    pub lag_sev2: u64,
    pub lag_sev1: u64,
    /// `SALE_SCAN_CHUNKS_PER_PASS` on Ethereum; on a faster chain enough chunks of `SALE_SCAN_MAX_CHUNK_BLOCKS` to cover the chain's
    /// blocks of `SALE_SCAN_PASS_COVER` cadences.
    pub chunks_per_pass: usize,
    /// The first post's minimum spread of its counted sales (`FIRST_POST_SPAN_SECS`).
    pub first_span: u64,
}

/// Audit C3-02: the floor's block counts on `chain` (an unknown chain is counted as Ethereum's; `validate` refuses one).
pub fn sale_blocks(chain: u64) -> SaleBlocks {
    let ms = crate::chains::block_ms(chain).unwrap_or(crate::chains::ETHEREUM_BLOCK_MS).max(1);
    let b = |secs: u64| (secs as u128 * 1_000 / ms as u128).min(u64::MAX as u128) as u64;
    let per_pass = b(SALE_SCAN_PASS_SECS).saturating_mul(SALE_SCAN_PASS_COVER).div_ceil(SALE_SCAN_MAX_CHUNK_BLOCKS) as usize;
    SaleBlocks { window: b(SALE_WINDOW_SECS), final_depth: b(SALE_FINAL_DEPTH_SECS), reanchor_lag: b(SALE_REANCHOR_LAG_SECS),
        lag_sev2: b(SALE_LAG_SEV2_SECS), lag_sev1: b(SALE_LAG_SEV1_SECS), chunks_per_pass: per_pass.max(SALE_SCAN_CHUNKS_PER_PASS),
        first_span: b(FIRST_POST_SPAN_SECS) }
}

impl EthNftConfig {
    pub fn floor_params(&self) -> FloorParams {
        self.floor.unwrap_or_default()
    }
    pub fn gas_pot_target(&self) -> u128 {
        self.gas_pot_target_wei.unwrap_or(GAS_POT_TARGET_WEI)
    }
    pub fn logs_max(&self) -> u64 {
        self.logs_max_blocks.unwrap_or(SALE_SCAN_CHUNK_BLOCKS)
    }
    pub fn cols(&self) -> Result<Vec<Col>, String> {
        self.collections.iter().map(|c| {
            if c.wrapper == crate::solhome::SolHomeCfg::BOOKS || c.module == crate::solhome::SolHomeCfg::BOOKS {
                if c.wrapper != c.module {
                    return Err(format!("{}'s wrapper and module: both are \"books\" or neither", c.collection));
                }
                let collection = crate::evmtx::parse_address(&c.collection)?;
                let key = |tag: &[u8]| -> [u8; 20] { crate::evmtx::keccak(&[tag, &collection].concat())[..20].try_into().expect("20") };
                return Ok(Col { collection, wrapper: key(b"rook:books:wrapper:"), module: key(b"rook:books:module:"), max_purchase_wei: c.max_purchase_wei,
                    max_premium_bps: c.max_premium_bps, books: true, chain_id: self.chain_of(c), nft_bps: c.nft_bps() });
            }
            Ok(Col {
                collection: crate::evmtx::parse_address(&c.collection)?,
                wrapper: crate::evmtx::parse_address(&c.wrapper)?,
                module: crate::evmtx::parse_address(&c.module)?,
                max_purchase_wei: c.max_purchase_wei,
                max_premium_bps: c.max_premium_bps,
                books: false,
                chain_id: self.chain_of(c),
                nft_bps: c.nft_bps(),
            })
        }).collect()
    }
    pub fn col(&self, collection: &[u8; 20]) -> Option<Col> {
        self.cols().ok()?.into_iter().find(|c| c.collection == *collection)
    }
    /// The chain a collection lives on: its own `chain_id`, or the config's.
    pub fn chain_of(&self, c: &EthCollection) -> u64 {
        c.chain_id.unwrap_or(self.chain_id)
    }
    /// The chain of the configured collection whose ERC-721 address is `collection`.
    pub fn chain_of_addr(&self, collection: &[u8; 20]) -> Option<u64> {
        self.collections.iter().find(|c| crate::evmtx::parse_address(&c.collection).ok().as_ref() == Some(collection)).map(|c| self.chain_of(c))
    }
    /// Every chain the collections live on: the config's own first (its side keeps the state saved before sides existed), then the
    /// others in the order their first collection is listed.
    pub fn chains(&self) -> Vec<u64> {
        let mut out = vec![self.chain_id];
        for c in &self.collections {
            let id = self.chain_of(c);
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
    /// The side of one chain (2026-10-12): the same config with `chain_id` that chain and only its collections. Each chain's side runs
    /// the whole flow on its own chain with its own state (`ethnet`); for the config's own chain with every collection on it, this is
    /// the config itself.
    pub fn on(&self, chain: u64) -> EthNftConfig {
        EthNftConfig { chain_id: chain, collections: self.collections.iter().filter(|c| self.chain_of(c) == chain).cloned().collect(), ..self.clone() }
    }
}

/// The collections against the home (docs/v9/solana-migration.md §4; decision D-l): on the EVM home every wrapper and module is an
/// EVM address; on the Solana home every one is `"books"` (the canister keeps the books: no program). A mixed configuration is refused
/// at install.
pub fn validate_home(c: &EthNftConfig, solana: bool) -> Result<(), String> {
    // 2026-10-12: the sides of several chains run on the books home only
    if !solana && c.chains().len() > 1 {
        return Err("eth_nft: collections on several chains need the Solana home (the books): on the EVM home Relay's legs are 4663's and \
            Ethereum's alone".into());
    }
    for x in c.cols().map_err(|e| format!("eth_nft: {e}"))? {
        if x.books != solana {
            return Err(format!("eth_nft: collection 0x{} names its wrapper and module {}, but the home is {}", hex::encode(x.collection),
                if x.books { "\"books\"" } else { "on an EVM chain" }, if solana { "Solana (the books: D-l)" } else { "the EVM chain" }));
        }
    }
    if solana && c.deposits {
        return Err("eth_nft.deposits: holder deposits are OFF on the Solana home at launch (docs/v9/solana-migration.md §2.7; their claim \
            format changes, EWD2)".into());
    }
    Ok(())
}

/// Every rule an install or upgrade argument's `eth_nft` must satisfy.
pub fn validate(c: &EthNftConfig, chains: &[crate::chains::EvmChain], home: u64) -> Result<(), String> {
    if c.chain_id == 0 || c.chain_id == home {
        return Err(format!("eth_nft.chain_id {} is zero or the home chain", c.chain_id));
    }
    if !chains.iter().any(|x| x.chain_id == c.chain_id) {
        return Err(format!("eth_nft.chain_id {} is not one of evm_chains: its providers, Seaport and fee caps come from there", c.chain_id));
    }
    // 2026-10-12: a collection on another chain (Robinhood Chain's own NFTs): that chain is listed in evm_chains (its providers, its
    // Seaport 1.6 and code hash, its fee caps), and Relay names it (its legs from and to Solana)
    for x in &c.collections {
        let id = c.chain_of(x);
        if id == 0 || id == home {
            return Err(format!("eth_nft: collection {} is on chain {id}: zero or the home chain", x.collection));
        }
        if !chains.iter().any(|y| y.chain_id == id) {
            return Err(format!("eth_nft: collection {} is on chain {id}, which is not one of evm_chains: its providers, Seaport and fee \
                caps come from there", x.collection));
        }
        if id != c.chain_id && crate::relayeth::relay_name(id).is_none() {
            return Err(format!("eth_nft: collection {} is on chain {id}, which this build's Relay legs do not name", x.collection));
        }
        // audit C3-01: a token-bound collection is priced as its NFT alone, at a share the argument states; nothing else has one
        match (x.is_token_bound(), x.nft_value_bps) {
            (true, Some(b)) if b > 0 && b < 10_000 => {}
            (true, _) => return Err(format!("eth_nft: collection {} is token-bound: its nft_value_bps (the share of a price that is the NFT \
                alone, in (0, 10,000)) must be named: no rule reads what its accounts hold (audit C3-01)", x.collection)),
            (false, Some(_)) => return Err(format!("eth_nft: collection {} names nft_value_bps without token_bound", x.collection)),
            (false, None) => {}
        }
    }
    // audit C3-14: each other chain's side counts its buys from `side_buy_base` (the chain id's low 24 bits, shifted): a chain whose low
    // bits are zero would count from the config's own side's ids, two chains equal in them would share one range
    let chains = c.chains();
    // audit C3-02: every block count of a side's floor is in its chain's blocks, from the chain's block time
    if let Some(id) = chains.iter().find(|id| crate::chains::block_ms(**id).is_none()) {
        return Err(format!("eth_nft: chain {id}'s block time is not known to this build: the floor's sale window cannot be cut in its blocks \
            (audit C3-02)"));
    }
    for (i, id) in chains.iter().enumerate().skip(1) {
        let low = id & 0xFF_FFFF;
        if low == 0 || chains[1..i].iter().any(|y| y & 0xFF_FFFF == low) {
            return Err(format!("eth_nft: chain {id}'s buy ids (its low 24 bits, shifted by 40) would meet another side's (audit C3-14)"));
        }
    }
    if c.chains().len() > 1 && c.dormancy.is_some() {
        return Err("eth_nft.dormancy: the dormancy counts one chain's collections; with collections on several chains it is off".into());
    }
    if c.collections.is_empty() {
        return Err("eth_nft names no collection".into());
    }
    if !(crate::relayeth::MIN_FEE_BPS..=crate::relayeth::MAX_FEE_BPS).contains(&c.relay_max_fee_bps) {
        return Err(format!("eth_nft.relay_max_fee_bps {} is outside [{}, {}]", c.relay_max_fee_bps, crate::relayeth::MIN_FEE_BPS,
            crate::relayeth::MAX_FEE_BPS));
    }
    let cols = c.cols().map_err(|e| format!("eth_nft: {e}"))?;
    for (i, x) in cols.iter().enumerate() {
        if x.collection == [0; 20] || x.wrapper == [0; 20] || x.module == [0; 20] {
            return Err("eth_nft: a zero address".into());
        }
        if cols[..i].iter().any(|y| y.collection == x.collection || y.wrapper == x.wrapper || y.module == x.module) {
            return Err(format!("eth_nft: collection 0x{} (or its wrapper or module) is listed twice", hex::encode(x.collection)));
        }
        if x.max_purchase_wei < crate::policy::MIN_PURCHASE_WEI || x.max_purchase_wei > crate::floortrigger::PURCHASE_LIMIT_CEILING_WEI {
            return Err(format!("eth_nft: max_purchase_wei {} is outside [{}, {}]", x.max_purchase_wei, crate::policy::MIN_PURCHASE_WEI,
                crate::floortrigger::PURCHASE_LIMIT_CEILING_WEI));
        }
        if x.max_premium_bps > MAX_PREMIUM_BPS {
            return Err(format!("eth_nft: max_premium_bps {} is over {MAX_PREMIUM_BPS}", x.max_premium_bps));
        }
    }
    if !(GAS_POT_TARGET_MIN_WEI..=GAS_POT_TARGET_MAX_WEI).contains(&c.gas_pot_target()) {
        return Err(format!("eth_nft.gas_pot_target_wei {} is outside [{GAS_POT_TARGET_MIN_WEI}, {GAS_POT_TARGET_MAX_WEI}]", c.gas_pot_target()));
    }
    if !(SALE_SCAN_MIN_CHUNK_BLOCKS..=SALE_SCAN_MAX_CHUNK_BLOCKS).contains(&c.logs_max()) {
        return Err(format!("eth_nft.logs_max_blocks {} is outside [{SALE_SCAN_MIN_CHUNK_BLOCKS}, {SALE_SCAN_MAX_CHUNK_BLOCKS}]", c.logs_max()));
    }
    if let Some(d) = &c.dormancy {
        crate::dormancy::validate(d, c.collections.len())?;
    }
    let p = c.floor_params();
    if p.min_listings < 2 || p.min_listings as usize > MAX_LISTINGS || p.listing_rank == 0 || p.listing_rank > p.min_listings {
        return Err("eth_nft.floor: 2 ≤ min_listings ≤ 8 and 1 ≤ listing_rank ≤ min_listings".into());
    }
    if p.min_sales == 0 || p.min_sales as usize > MAX_SALES || p.up_cap_bps > 2_000 || p.down_cap_bps > 3_000 {
        return Err("eth_nft.floor: 1 ≤ min_sales ≤ 12, up_cap_bps ≤ 2,000, down_cap_bps ≤ 3,000".into());
    }
    Ok(())
}

// ---- listings as floor evidence (§4.2 step 1) ------------------------------------------------------------------------------------

/// A handed-in Seaport listing, either encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listing {
    Basic(sp::BasicOrder),
    Advanced(sp::AdvancedOrder),
}

/// What a listing offers, as the floor rules see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListingTerms {
    pub offerer: [u8; 20],
    pub token: Word,
    /// What a buyer pays in all, fees included.
    pub price: u128,
    pub start: u64,
    pub end: u64,
    pub conduit_key: Word,
    /// Seaport's order type (0 full open, 1 partial open, 2 and 3 restricted: the zone decides, 4 contract) and the zone.
    pub order_type: u8,
    pub zone: [u8; 20],
}

impl ListingTerms {
    /// Floor evidence must be fillable by anyone (audit 2026-09-27-nft, B-L-1): an open order, or a restricted one only through a
    /// zone this chain allow-lists (the buy judge's own rule). A restricted order behind any other zone can refuse every fulfiller,
    /// and a contract order is the offerer's code: neither is a price anyone can pay.
    pub fn fillable(&self, zones: &[[u8; 20]]) -> Result<(), String> {
        match self.order_type {
            0 | 1 => Ok(()),
            2 | 3 if zones.contains(&self.zone) => Ok(()),
            2 | 3 => Err(format!("a restricted order behind zone 0x{}, not an allow-listed one: not fillable by anyone", hex::encode(self.zone))),
            n => Err(format!("order type {n}: not a listing anyone can fill")),
        }
    }
}

impl Listing {
    pub fn parse(b: &[u8]) -> Result<Listing, String> {
        if b.starts_with(&sp::FULFILL_ADVANCED_ORDER) {
            Ok(Listing::Advanced(sp::decode_advanced(b)?))
        } else {
            Ok(Listing::Basic(sp::decode(b)?))
        }
    }
    pub fn parse_hex(s: &str) -> Result<Listing, String> {
        Listing::parse(&hex::decode(s.trim().trim_start_matches("0x")).map_err(|e| format!("the order is not hex: {e}"))?)
    }
    pub fn offerer(&self) -> [u8; 20] {
        match self {
            Listing::Basic(o) => o.offerer,
            Listing::Advanced(o) => o.order.offerer,
        }
    }
    pub fn signature(&self) -> &[u8] {
        match self {
            Listing::Basic(o) => &o.signature,
            Listing::Advanced(o) => &o.signature,
        }
    }
    /// The order hash with the offerer's counter at the pin.
    pub fn order_hash(&self, counter: Word) -> Word {
        match self {
            Listing::Basic(o) => o.components(counter).hash(),
            Listing::Advanced(o) => sp::OrderComponents { counter, ..o.order.clone() }.hash(),
        }
    }
    /// ETH for exactly one ERC-721 of `collection`, and what it costs.
    pub fn terms(&self, collection: &[u8; 20]) -> Result<ListingTerms, String> {
        match self {
            Listing::Basic(o) => {
                if o.basic_order_type > 3 {
                    return Err(format!("basic order type {}: not ETH for an ERC-721", o.basic_order_type));
                }
                if o.consideration_token != [0; 20] || o.consideration_identifier != [0; 32] {
                    return Err("the price is not native ETH".into());
                }
                if o.offer_token != *collection || o.offer_amount != 1 {
                    return Err("not one token of this collection".into());
                }
                if o.total_original_additional_recipients as usize != o.additional_recipients.len() {
                    return Err("the order carries tips".into());
                }
                Ok(ListingTerms { offerer: o.offerer, token: o.offer_identifier, price: o.total().ok_or("the total overflows")?,
                    start: sp::secs(&o.start_time), end: sp::secs(&o.end_time), conduit_key: o.offerer_conduit_key,
                    order_type: o.basic_order_type % 4, zone: o.zone })
            }
            Listing::Advanced(a) => {
                let o = &a.order;
                if o.offer.len() != 1 {
                    return Err(format!("{} offer items: not one token", o.offer.len()));
                }
                let it = &o.offer[0];
                if it.item_type != 2 || it.token != *collection || it.start != sp::word_u128(1) || it.end != sp::word_u128(1) {
                    return Err("not one ERC-721 of this collection".into());
                }
                if a.total_original_consideration_items as usize != o.consideration.len() {
                    return Err("the order carries tips".into());
                }
                let mut price: u128 = 0;
                for c in &o.consideration {
                    if c.item_type != 0 || c.token != [0; 20] || c.identifier != [0; 32] || c.start != c.end {
                        return Err("a consideration item is not a fixed amount of native ETH".into());
                    }
                    if c.start[..16].iter().any(|x| *x != 0) {
                        return Err("an amount over 128 bits".into());
                    }
                    price = price.checked_add(u128::from_be_bytes(c.start[16..].try_into().expect("16"))).ok_or("the total overflows")?;
                }
                Ok(ListingTerms { offerer: o.offerer, token: it.identifier, price, start: sp::secs(&o.start_time),
                    end: sp::secs(&o.end_time), conduit_key: o.conduit_key, order_type: o.order_type, zone: o.zone })
            }
        }
    }
}

/// What the chain said about one listing at the pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListingFacts {
    pub pin_timestamp: u64,
    /// `ownerOf(token)`.
    pub owner: [u8; 20],
    /// `getOrderStatus(hash)`.
    pub cancelled: bool,
    pub filled: u128,
    pub size: u128,
    /// The offerer's conduit (or Seaport) may move the token: `isApprovedForAll` or `getApproved`.
    pub approved: bool,
    /// The signature recovered to the offerer here (an EOA or a bulk order). A contract wallet's (ERC-1271) is not evidence.
    pub signed: bool,
}

/// One verified listing: who offers which token, and for how much.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub offerer: [u8; 20],
    pub token: Word,
    pub price: u128,
}

/// A listing is floor evidence only if it is live and fillable at the pin.
pub fn judge_listing(t: &ListingTerms, f: &ListingFacts, min_life_s: u64) -> Result<Seen, String> {
    if t.price == 0 {
        return Err("a free listing".into());
    }
    if t.start > f.pin_timestamp || t.end <= f.pin_timestamp.saturating_add(min_life_s) {
        return Err(format!("not live for {min_life_s} s at the pin (start {}, end {}, pin {})", t.start, t.end, f.pin_timestamp));
    }
    if f.cancelled || (f.size > 0 && f.filled >= f.size) {
        return Err("cancelled or filled".into());
    }
    if f.owner != t.offerer {
        return Err(format!("the token's owner is 0x{}, not the offerer", hex::encode(f.owner)));
    }
    if !f.approved {
        return Err("the offerer's conduit is not approved to move the token".into());
    }
    if !f.signed {
        return Err("the signature is not an EOA's or a bulk order's recovering to the offerer".into());
    }
    Ok(Seen { offerer: t.offerer, token: t.token, price: t.price })
}

// ---- sales as floor evidence (§4.2 step 2) ---------------------------------------------------------------------------------------

pub const SEAPORT_1_5: [u8; 20] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xad, 0xc0, 0x4c, 0x56, 0xbf, 0x30, 0xac, 0x9d, 0x3c, 0x0a, 0xaf,
    0x14, 0xdc];
pub const BLUR_EXCHANGE_V2: [u8; 20] = [0xb2, 0xec, 0xfe, 0x4e, 0x4d, 0x61, 0xf8, 0x79, 0x0b, 0xbb, 0x9d, 0xe2, 0xd1, 0x25, 0x9b, 0x9e, 0x24, 0x10,
    0xce, 0xa5];

/// Audit C3-11: the marketplaces whose sale logs the scan reads on `chain`: `(the Seaports, Blur v2)`. Ethereum: Seaport 1.6 and 1.5 and
/// Blur v2 at their Ethereum addresses. Any other chain (Robinhood Chain: OpenSea's measurement shows Seaport 1.6 alone, at its canonical
/// address): Seaport 1.6 only, never a read of an Ethereum venue's address where it is not deployed.
pub fn sale_venues(chain: u64) -> (Vec<[u8; 20]>, bool) {
    if chain == 1 { (vec![sp::SEAPORT_1_6, SEAPORT_1_5], true) } else { (vec![sp::SEAPORT_1_6], false) }
}
pub const WETH: [u8; 20] = [0xc0, 0x2a, 0xaa, 0x39, 0xb2, 0x23, 0xfe, 0x8d, 0x0a, 0x0e, 0x5c, 0x4f, 0x27, 0xea, 0xd9, 0x08, 0x3c, 0x75, 0x6c, 0xc2];
pub const BLUR_POOL: [u8; 20] = [0x00, 0x00, 0x00, 0x00, 0x00, 0xa3, 0x9b, 0xb2, 0x72, 0xe7, 0x90, 0x75, 0xad, 0xe1, 0x25, 0xfd, 0x35, 0x18,
    0x87, 0xac];
/// `OrderFulfilled(bytes32,address,address,address,(uint8,address,uint256,uint256)[],(uint8,address,uint256,uint256,address)[])`
pub const ORDER_FULFILLED: [u8; 32] = hex32("9d9af8e38d66c62e2c12f0225249fd9d721c54b83f48d9352c97c6cacdcb6f31");
/// Blur v2: `Execution721Packed(bytes32,uint256,uint256)` and its taker-fee and maker-fee forms (a fourth word).
pub const BLUR_721: [u8; 32] = hex32("1d5e12b51dee5e4d34434576c3fb99714a85f57b0fd546ada4b0bddd736d12b2");
pub const BLUR_721_TAKER_FEE: [u8; 32] = hex32("0fcf17fac114131b10f37b183c6a60f905911e52802caeeb3e6ea210398b81ab");
pub const BLUR_721_MAKER_FEE: [u8; 32] = hex32("7dc5c0699ac8dd5250cbe368a2fc3b4a2daadb120ad07f6cccea29f83482686e");
/// ERC-721 `Transfer(address,address,uint256)`.
pub const TRANSFER: [u8; 32] = hex32("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");

const fn hex32(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nib(b[2 * i]) << 4) | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}
const fn nib(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => panic!("not lower-case hex"),
    }
}

/// One log of a receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogV {
    pub address: [u8; 20],
    pub topics: Vec<Word>,
    pub data: Vec<u8>,
}

/// A receipt as the transform keeps it: its block, its status, its logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptV {
    pub block: u64,
    pub success: bool,
    pub logs: Vec<LogV>,
}

fn hexb(v: &serde_json::Value, k: &str) -> Result<Vec<u8>, String> {
    let s = v.get(k).and_then(|x| x.as_str()).ok_or_else(|| format!("no {k}"))?;
    hex::decode(s.trim_start_matches("0x")).map_err(|_| format!("{k} is not hex"))
}

/// `eth_getTransactionReceipt`'s result (or the transform's projection of it) as a [`ReceiptV`]. `Ok(None)` for `null`.
pub fn receipt_of(v: &serde_json::Value) -> Result<Option<ReceiptV>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let block = u64::from_str_radix(v.get("blockNumber").and_then(|x| x.as_str()).ok_or("no blockNumber")?.trim_start_matches("0x"), 16)
        .map_err(|_| "an unreadable blockNumber")?;
    let success = v.get("status").and_then(|x| x.as_str()) == Some("0x1");
    let mut logs = Vec::new();
    for l in v.get("logs").and_then(|x| x.as_array()).ok_or("no logs")? {
        let address: [u8; 20] = hexb(l, "address")?.try_into().map_err(|_| "a log address is not 20 bytes")?;
        let mut topics = Vec::new();
        for t in l.get("topics").and_then(|x| x.as_array()).ok_or("a log has no topics")? {
            let b = hex::decode(t.as_str().unwrap_or("").trim_start_matches("0x")).map_err(|_| "a topic is not hex")?;
            topics.push(b.try_into().map_err(|_| "a topic is not 32 bytes")?);
        }
        logs.push(LogV { address, topics, data: hexb(l, "data")? });
    }
    Ok(Some(ReceiptV { block, success, logs }))
}

/// The projection every provider must agree on (a transform): `{blockNumber, status, logs: [{address, topics, data}]}`, the
/// logs' addresses lower-cased; `null` for no receipt or one above `pin`.
pub fn receipt_projection(result: &serde_json::Value, pin: u64) -> serde_json::Value {
    let Some(block) = result.get("blockNumber").and_then(|x| x.as_str()).and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok())
    else { return serde_json::Value::Null };
    if block > pin {
        return serde_json::Value::Null;
    }
    let logs: Vec<serde_json::Value> = result.get("logs").and_then(|x| x.as_array()).map(|ls| ls.iter().map(|l| serde_json::json!({
        "address": l.get("address").and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase(),
        "topics": l.get("topics").cloned().unwrap_or(serde_json::Value::Null),
        "data": l.get("data").cloned().unwrap_or(serde_json::Value::Null),
    })).collect()).unwrap_or_default();
    serde_json::json!({ "blockNumber": format!("0x{block:x}"), "status": result.get("status").cloned().unwrap_or(serde_json::Value::Null),
        "logs": logs })
}

/// The transform body for the canister's own sale scan (audit 2026-09-27-nft round 2, A-M-1 / B2-M-1): a collection's
/// `eth_getLogs` answer for `Transfer` projected to `{"result": [[txHash, blockNumber, tokenId, from], …]}`: every log with four
/// topics (ERC-721) whose `from` is neither zero (a mint) nor its `to`, not removed, one entry per transaction (the least sender and token
/// of its moves), sorted by block then hash, hex lower-cased. So replicas agree on which transactions moved a token, not on how
/// the provider wrote the logs. Round 3 (B3-M-1): the token and the sender let the scan read one transaction per token and per
/// sender before a second of either, so a flood of transfers cannot starve a real sale. A JSON-RPC error is `json_rpc_norm`'s
/// shape; anything else is the one unusable error.
pub fn transfer_logs_body(body: &[u8]) -> Vec<u8> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else { return crate::pin::json_rpc_norm_body(body) };
    let Some(arr) = v.get("result").and_then(|r| r.as_array()) else { return crate::pin::json_rpc_norm_body(body) };
    let mut out: Vec<(u64, String, String, String)> = Vec::new();
    for l in arr {
        if l.get("removed").and_then(|x| x.as_bool()) == Some(true) {
            continue;
        }
        let Some(t) = l.get("topics").and_then(|x| x.as_array()) else { continue };
        let tx = l.get("transactionHash").and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
        let block = l.get("blockNumber").and_then(|x| x.as_str()).and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok());
        let topic = |i: usize| t.get(i).and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
        let (from, to) = (topic(1), topic(2));
        let zero = from.trim_start_matches("0x").chars().all(|c| c == '0');
        if t.len() != 4 || topic(0) != format!("0x{}", hex::encode(TRANSFER)) || zero || from == to || tx.len() != 66 {
            continue;
        }
        let Some(b) = block else { continue };
        // one entry per transaction; of its moves, the least (sender, token), so the entry does not depend on the provider's order
        match out.iter_mut().find(|(_, x, ..)| *x == tx) {
            Some(e) => {
                if (from.clone(), topic(3)) < (e.3.clone(), e.2.clone()) {
                    e.2 = topic(3);
                    e.3 = from;
                }
            }
            None => out.push((b, tx, topic(3), from)),
        }
    }
    out.sort();
    let res: Vec<serde_json::Value> = out.into_iter().map(|(b, tx, token, from)| serde_json::json!([tx, format!("0x{b:x}"), token, from])).collect();
    serde_json::json!({ "result": res }).to_string().into_bytes()
}

/// Review round 4 (B4-H-1): the transform body for the marketplace sale logs (Seaport 1.5 / 1.6 `OrderFulfilled`, Blur v2
/// `Execution721*`) of a block or (round 5, B5-M-1) a range: the logs that sell a token of `collection` (`seaport_sale`,
/// `blur_sale`), projected like `transfer_logs_body`, one entry per transaction `[txHash, blockNumber, tokenId, seller-side address]`
/// (the order's offerer; Blur's trader), sorted, hex lower-cased. The route around a range whose `Transfer` logs are too big to read:
/// a flood of transfers does not hide a sale from it. (Review round 5, B5-L-2: a fill padded with thousands of consideration items
/// can make one marketplace's logs that big too; `EthNet` reads each marketplace on its own, so that hides nothing on the other.) Its
/// transactions' receipts are then judged as any other (`sales_in`).
pub fn sale_logs_body(body: &[u8], collection: &[u8; 20]) -> Vec<u8> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else { return crate::pin::json_rpc_norm_body(body) };
    let Some(arr) = v.get("result").and_then(|r| r.as_array()) else { return crate::pin::json_rpc_norm_body(body) };
    let mut out: Vec<(u64, String, String, String)> = Vec::new();
    for l in arr {
        if l.get("removed").and_then(|x| x.as_bool()) == Some(true) {
            continue;
        }
        let tx = l.get("transactionHash").and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
        let Some(b) = l.get("blockNumber").and_then(|x| x.as_str()).and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok()) else { continue };
        let Ok(address) = hexb(l, "address") else { continue };
        let Ok(data) = hexb(l, "data") else { continue };
        let topics: Vec<Word> = l.get("topics").and_then(|x| x.as_array()).map(|t| t.iter().filter_map(|x| hex::decode(x.as_str().unwrap_or("")
            .trim_start_matches("0x")).ok().and_then(|b| b.try_into().ok())).collect()).unwrap_or_default();
        let (Some(t0), true) = (topics.first(), tx.len() == 66) else { continue };
        let got = if (address == sp::SEAPORT_1_6 || address == SEAPORT_1_5) && *t0 == ORDER_FULFILLED {
            seaport_sale(&data, collection).map(|(tok, _)| (tok, topics.get(1).map(addr_w).unwrap_or([0; 20])))
        } else if address == BLUR_EXCHANGE_V2 && (*t0 == BLUR_721 || *t0 == BLUR_721_TAKER_FEE || *t0 == BLUR_721_MAKER_FEE) {
            blur_sale(&data, collection).map(|(tok, _)| (tok, word(&data, 1).map(|w| addr_w(&w)).unwrap_or([0; 20])))
        } else {
            None
        };
        let Some((token, from)) = got else { continue };
        let (token, from) = (format!("0x{}", hex::encode(token)), format!("0x{}{}", "0".repeat(24), hex::encode(from)));
        match out.iter_mut().find(|(_, x, ..)| *x == tx) {
            Some(e) => {
                if (from.clone(), token.clone()) < (e.3.clone(), e.2.clone()) {
                    e.2 = token;
                    e.3 = from;
                }
            }
            None => out.push((b, tx, token, from)),
        }
    }
    out.sort();
    let res: Vec<serde_json::Value> = out.into_iter().map(|(b, tx, token, from)| serde_json::json!([tx, format!("0x{b:x}"), token, from])).collect();
    serde_json::json!({ "result": res }).to_string().into_bytes()
}

/// `transfer_logs_body`'s result as `(tx hash, block)`.
pub fn transfer_txs_of(v: &serde_json::Value) -> Result<Vec<([u8; 32], u64)>, String> {
    Ok(transfer_moves_of(v)?.into_iter().map(|m| (m.hash, m.block)).collect())
}

/// One transaction that moved a token of a collection between two wallets (round 3, B3-M-1): its first such `Transfer`'s token and
/// sender.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub hash: [u8; 32],
    pub block: u64,
    pub token: Word,
    pub from: [u8; 20],
}

/// `transfer_logs_body`'s result as [`Move`]s. An entry without the token and sender (the round-2 projection) gets a token and a
/// sender of its own (its hash), so it is never grouped with another.
pub fn transfer_moves_of(v: &serde_json::Value) -> Result<Vec<Move>, String> {
    let arr = v.as_array().ok_or("the transfer logs are not a list")?;
    arr.iter().map(|x| {
        let hx = |i: usize| hex::decode(x.get(i).and_then(|y| y.as_str()).unwrap_or("").trim_start_matches("0x"));
        let h: [u8; 32] = hx(0).map_err(|_| "a transaction hash is not hex")?.try_into().map_err(|_| "a transaction hash is not 32 bytes")?;
        let block = u64::from_str_radix(x.get(1).and_then(|y| y.as_str()).unwrap_or("").trim_start_matches("0x"), 16).map_err(|_| "a block is not hex")?;
        let token: Word = match x.get(2) {
            Some(_) => hx(2).map_err(|_| "a token is not hex")?.try_into().map_err(|_| "a token is not 32 bytes")?,
            None => h,
        };
        let from: [u8; 20] = match x.get(3) {
            Some(_) => {
                let w: Word = hx(3).map_err(|_| "a sender is not hex")?.try_into().map_err(|_| "a sender is not 32 bytes")?;
                addr_w(&w)
            }
            None => h[..20].try_into().expect("20"),
        };
        Ok(Move { hash: h, block, token, from })
    }).collect()
}

/// Round 3 (D3-H-1): our delivery's own receipt shows it delivered: it succeeded and the collection's `Transfer` of `token` from
/// `from` (our address) to `to` (the ticket's destination) is in it. Where the token went AFTER that is not the delivery's question.
pub fn delivered_in(r: &ReceiptV, collection: &[u8; 20], from: &[u8; 20], to: &[u8; 20], token: &Word) -> bool {
    r.success && r.logs.iter().any(|l| l.address == *collection && l.topics.len() == 4 && l.topics[0] == TRANSFER
        && addr_w(&l.topics[1]) == *from && addr_w(&l.topics[2]) == *to && l.topics[1][..12] == [0; 12] && l.topics[2][..12] == [0; 12]
        && l.topics[3] == *token)
}

fn word(d: &[u8], i: usize) -> Option<Word> {
    d.get(i * 32..i * 32 + 32).map(|s| s.try_into().expect("32"))
}
fn small(w: &Word) -> Option<u128> {
    if w[..16].iter().any(|x| *x != 0) { None } else { Some(u128::from_be_bytes(w[16..].try_into().expect("16"))) }
}
fn addr_w(w: &Word) -> [u8; 20] {
    w[12..].try_into().expect("20")
}
fn money(item_type: u128, token: &[u8; 20]) -> bool {
    item_type == 0 && *token == [0; 20] || item_type == 1 && (*token == WETH || *token == BLUR_POOL)
}

/// One Seaport `OrderFulfilled`: a sale of exactly one token of `collection` for ETH, WETH or Blur Pool ETH.
fn seaport_sale(data: &[u8], collection: &[u8; 20]) -> Option<(Word, u128)> {
    let arr = |off: u128, n: usize| -> Option<Vec<Vec<Word>>> {
        let i = (off / 32) as usize;
        let count = small(&word(data, i)?)? as usize;
        if count > 64 {
            return None;
        }
        (0..count).map(|k| (0..n).map(|j| word(data, i + 1 + k * n + j)).collect::<Option<Vec<_>>>()).collect()
    };
    let offer = arr(small(&word(data, 2)?)?, 4)?;
    let cons = arr(small(&word(data, 3)?)?, 5)?;
    let is_nft = |it: &Vec<Word>| matches!(small(&it[0]), Some(2) | Some(4)) && addr_w(&it[1]) == *collection;
    let is_money = |it: &Vec<Word>| small(&it[0]).is_some_and(|t| money(t, &addr_w(&it[1])));
    let nft_off: Vec<&Vec<Word>> = offer.iter().filter(|x| is_nft(x)).collect();
    let nft_con: Vec<&Vec<Word>> = cons.iter().filter(|x| is_nft(x)).collect();
    if nft_off.len() == 1 && offer.len() == 1 && nft_con.is_empty() && cons.iter().all(is_money) {
        // a listing filled: the buyer's payments are the consideration
        let price = cons.iter().try_fold(0u128, |a, c| a.checked_add(small(&c[3])?))?;
        return Some((nft_off[0][2], price));
    }
    if nft_con.len() == 1 && !offer.is_empty() && offer.iter().all(is_money) && cons.iter().all(|c| is_nft(c) || is_money(c)) {
        // a bid accepted: the bidder's offer is the price
        let price = offer.iter().try_fold(0u128, |a, o| a.checked_add(small(&o[3])?))?;
        return Some((nft_con[0][2], price));
    }
    None
}

/// One Blur v2 `Execution721*Packed`: `tokenIdListingIndexTrader = tokenId << 168 | listingIndex << 160 | trader` and
/// `collectionPriceSide = side << 248 | price << 160 | collection` (measured on real receipts, 2026-09-27).
fn blur_sale(data: &[u8], collection: &[u8; 20]) -> Option<(Word, u128)> {
    let tlt = word(data, 1)?;
    let cps = word(data, 2)?;
    if cps[12..] != collection[..] {
        return None;
    }
    let mut price = [0u8; 16];
    price[5..].copy_from_slice(&cps[1..12]);   // 88 bits between the side byte and the collection
    let mut token = [0u8; 32];
    token[21..].copy_from_slice(&tlt[..11]);   // the top 88 bits: tokenIdListingIndexTrader >> 168
    Some((token, u128::from_be_bytes(price)))
}

/// One sale of one token of a collection, as its receipt shows it: the price, and who sold and who bought (the `from` of the token's
/// first `Transfer` in the transaction and the `to` of its last: the marketplace moves the NFT from the seller to the buyer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sale {
    pub token: Word,
    pub price: u128,
    pub seller: [u8; 20],
    pub buyer: [u8; 20],
    pub block: u64,
}

/// The sales of `collection` in one receipt, one per token (the lowest price, when one transaction shows the same token twice:
/// a listing matched to a bid is one movement, not two sales). A sale counts only if the collection's own `Transfer` of that token
/// is in the receipt (who sold it, who bought it).
pub fn sales_in(r: &ReceiptV, collection: &[u8; 20]) -> Vec<Sale> {
    if !r.success {
        return vec![];
    }
    let mut out: Vec<(Word, u128)> = Vec::new();
    for l in &r.logs {
        let t0 = l.topics.first();
        let got = if (l.address == sp::SEAPORT_1_6 || l.address == SEAPORT_1_5) && t0 == Some(&ORDER_FULFILLED) {
            seaport_sale(&l.data, collection)
        } else if l.address == BLUR_EXCHANGE_V2 && matches!(t0, Some(t) if *t == BLUR_721 || *t == BLUR_721_TAKER_FEE || *t == BLUR_721_MAKER_FEE) {
            blur_sale(&l.data, collection)
        } else {
            None
        };
        if let Some((tok, p)) = got {
            match out.iter_mut().find(|(t, _)| *t == tok) {
                Some(x) => x.1 = x.1.min(p),
                None => out.push((tok, p)),
            }
        }
    }
    out.into_iter().filter_map(|(token, price)| {
        let moves: Vec<&LogV> = r.logs.iter().filter(|l| l.address == *collection && l.topics.len() == 4 && l.topics[0] == TRANSFER
            && l.topics[3] == token).collect();
        let (first, last) = (moves.first()?, moves.last()?);
        Some(Sale { token, price, seller: addr_w(&first.topics[1]), buyer: addr_w(&last.topics[2]), block: r.block })
    }).collect()
}

/// The sales in the window from receipts read at the pin, newest block first. The caller reads each distinct transaction once.
pub fn sales_window(receipts: &[ReceiptV], collection: &[u8; 20], pin: u64) -> Vec<Sale> {
    let mut v: Vec<Sale> = receipts.iter().filter(|r| r.block <= pin && r.block + SALE_WINDOW_BLOCKS >= pin)
        .flat_map(|r| sales_in(r, collection)).collect();
    v.sort_by(|a, b| b.block.cmp(&a.block));
    v
}

/// Tokens that went back to a wallet that held them earlier in the window (A → B → A, or round a longer ring): every sale of such
/// a token is a wash and none is counted (audit 2026-09-27-nft round 2, B2-M-1). `sales` newest first.
fn round_trips(sales: &[Sale]) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut tokens: Vec<Word> = sales.iter().map(|s| s.token).collect();
    tokens.sort();
    tokens.dedup();
    for t in tokens {
        let mut holders: Vec<[u8; 20]> = Vec::new();
        for s in sales.iter().rev().filter(|s| s.token == t) {
            if holders.contains(&s.buyer) {
                out.push(t);
                break;
            }
            holders.push(s.seller);
            holders.push(s.buyer);
        }
    }
    out
}

/// The sales that may bound a floor (audit 2026-09-27-nft B-H-1; round 2 B2-M-1, B2-L-1). Newest first, from the canister's OWN
/// read of the collection's sales over the window (never a hand-in: nobody chooses which sales count). Never a self-trade, never a
/// token that went back to an earlier holder (a ping-pong or a ring), within `SALE_BAND_BPS` of the listing floor `l`, one per
/// token, and one per WALLET: a wallet that sold or bought in a counted sale is never a party to another (so a chain of wallets
/// counts every other link, and a Blur bidder buying from ten sellers is one voice). A wash vote therefore needs its own token
/// and two wallets that trade nothing else in the window, and the sample is the whole window, so no volume of washes crowds a
/// real sale out.
pub fn counted_sales(sales: &[Sale], l: u128) -> Vec<u128> {
    counted(sales, l).into_iter().map(|s| s.price).collect()
}

/// [`counted_sales`], the sales themselves (audit C3-02: the first post reads their blocks).
pub fn counted(sales: &[Sale], l: u128) -> Vec<&Sale> {
    let lo = l.saturating_mul(10_000 - SALE_BAND_BPS) / 10_000;
    let hi = l.saturating_mul(10_000 + SALE_BAND_BPS) / 10_000;
    let washed = round_trips(sales);
    let (mut tokens, mut wallets, mut out) = (Vec::new(), Vec::new(), Vec::new());
    for s in sales {
        if s.seller == s.buyer || s.seller == [0; 20] || washed.contains(&s.token) || s.price < lo || s.price > hi {
            continue;
        }
        if tokens.contains(&s.token) || wallets.contains(&s.seller) || wallets.contains(&s.buyer) {
            continue;
        }
        tokens.push(s.token);
        wallets.push(s.seller);
        wallets.push(s.buyer);
        out.push(s);
    }
    out
}

// ---- the floor (§4.2) ---------------------------------------------------------------------------------------------------------------

/// What the canister would post, and why.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FloorPost {
    pub price: u128,
    pub confidence: u16,
    pub depth: u32,
    /// The listing floor `L`, the counted sales' median `S` and 25th percentile `S25` it came from (0 with no counted sale).
    pub listing: u128,
    pub sale_median: u128,
    pub sale_p25: u128,
    pub listings: u32,
    pub sales: u32,
    pub clamped: bool,
    /// Why the post is HELD under the listing floor (round 2, B2-M-1): the listings are not confirmed by `min_sales` counted
    /// sales, or run over them. `None` for a confirmed post.
    #[serde(default)]
    pub held: Option<String>,
}

fn quantile(sorted: &[u128], num: usize, den: usize) -> u128 {
    // the lower value at the position (n − 1) × num / den: a conservative percentile
    sorted[(sorted.len() - 1) * num / den]
}

fn median(sorted: &[u128]) -> u128 {
    let n = sorted.len();
    if n % 2 == 1 { sorted[n / 2] } else { sorted[n / 2 - 1] / 2 + sorted[n / 2] / 2 + (sorted[n / 2 - 1] % 2 + sorted[n / 2] % 2) / 2 }
}

/// The post from verified listings and the counted sales, or why there is none. `accepted`: the module's accepted floor now (0
/// before its first post).
///
/// Audit 2026-09-27-nft B-H-1: the post is never above the `listing_rank`-th cheapest verified live listing `L` (one per offerer):
/// it never leaves the range of listings anyone can fill. Round 2 (B2-M-1 / D2-M-2, B2-L-1, A-M-1): sales can HOLD the floor back,
/// never stop a post and never push the floor up:
/// - **confirmed** (`min_sales` counted sales and `L ≤ S25 × (1 + up_cap)`): the post is `L`;
/// - **held** (fewer counted sales, or `L` over that band): the post is `min(L, max(accepted, S25 × (1 + up_cap)))`, at
///   `HELD_CONFIDENCE`: it follows the listings DOWN (a fall anyone can fill; the module's ±5 %/h and ±20 %/day bound its pace)
///   and rises only as far as the counted sales support. Washes can therefore hold a rise back, never make the module stale;
/// - `L` under `S25 × (1 − down_cap)`: posted as it is, at `HELD_CONFIDENCE`, flagged (D7's −15 % no longer refuses a post);
/// - the FIRST post (accepted 0) is confirmed only: there is nothing yet to hold.
///
/// The band is taken over the 25th percentile, not the median (B2-L-1): lifting it takes wash votes outnumbering three quarters of
/// the window's real distinct-wallet sales. There is no downward veto any more: a listing floor below the market's sales is a
/// price anyone can fill (and burns price at the posted target, never above it); it is only flagged.
pub fn floor_from(listings: &[Seen], sales: &[Sale], p: &FloorParams, accepted: u128) -> Result<FloorPost, String> {
    floor_from_on(listings, sales, p, accepted, &sale_blocks(1))
}

/// Audit C3-01: a token-bound collection's listings and sales at the share of their price that is the NFT alone (`Col::nft_bps`), so
/// the floor cut from them, the module's value and the buy bound are the NFT's; 10,000 leaves them as they are.
pub fn nft_only(listings: &mut [Seen], sales: &mut [Sale], bps: u16) {
    if bps >= 10_000 {
        return;
    }
    let at = |x: u128| x.saturating_mul(bps as u128) / 10_000;
    listings.iter_mut().for_each(|s| s.price = at(s.price));
    sales.iter_mut().for_each(|s| s.price = at(s.price));
}

/// [`floor_from`] on one chain's blocks (audit C3-02): the first post (`accepted` 0) is confirmed only when its counted sales also span
/// `b.first_span` blocks (a day of the chain's): a burst of wash sales between fresh wallets is never a first price, on any chain.
pub fn floor_from_on(listings: &[Seen], sales: &[Sale], p: &FloorParams, accepted: u128, b: &SaleBlocks) -> Result<FloorPost, String> {
    // the cheapest per offerer, one token once
    let mut best: Vec<([u8; 20], u128)> = Vec::new();
    let mut tokens: Vec<Word> = Vec::new();
    for s in listings {
        if tokens.contains(&s.token) {
            continue;
        }
        tokens.push(s.token);
        match best.iter_mut().find(|(o, _)| *o == s.offerer) {
            Some(x) => x.1 = x.1.min(s.price),
            None => best.push((s.offerer, s.price)),
        }
    }
    if best.len() < p.min_listings as usize {
        return Err(format!("{} distinct offerers with a verified listing; {} are needed", best.len(), p.min_listings));
    }
    let mut prices: Vec<u128> = best.iter().map(|(_, x)| *x).collect();
    prices.sort_unstable();
    let l = prices[p.listing_rank as usize - 1];
    let counted = counted(sales, l);
    let span = match (counted.iter().map(|x| x.block).min(), counted.iter().map(|x| x.block).max()) {
        (Some(lo), Some(hi)) => hi - lo,
        _ => 0,
    };
    let mut s: Vec<u128> = counted.iter().map(|x| x.price).collect();
    s.sort_unstable();
    let (med, p25) = if s.is_empty() { (0, 0) } else { (median(&s), quantile(&s, 1, 4)) };
    // audit C3-02: a first post's sample also spans a day of the chain's blocks
    let spread = accepted != 0 || span >= b.first_span;
    let quorum = s.len() >= p.min_sales as usize && spread;
    let hi = p25.saturating_mul(10_000 + p.up_cap_bps as u128) / 10_000;
    let lo = p25.saturating_mul(10_000 - p.down_cap_bps as u128).div_ceil(10_000);
    // the book's depth at the listing floor: distinct offerers within 5 % (B-I-5: one offerer's several listings are one)
    let depth = best.iter().filter(|(_, x)| x.saturating_mul(100) <= l.saturating_mul(105)).count() as u32;
    let post = |price: u128, confidence: u16, held: Option<String>| FloorPost { price, confidence, depth, listing: l, sale_median: med,
        sale_p25: p25, listings: best.len() as u32, sales: s.len() as u32, clamped: false, held };
    if quorum && l <= hi && l >= lo {
        let gap = l.abs_diff(med).saturating_mul(10_000) / med.max(1);
        let confidence = if gap <= 500 { 10_000 } else if gap <= 1_500 { 7_000 } else { 5_000 };
        return Ok(post(l, confidence, None));
    }
    if quorum && l < lo {
        // a listing floor further under the counted sales' lower quarter than `down_cap` (a crash the sales do not show yet, or a
        // burst of cheap listings): it is a price anyone can fill, so it IS posted (never "no post", never held up), at the held
        // confidence, flagged; the module's ±5 %/h and ±20 %/day pace it
        return Ok(post(l, HELD_CONFIDENCE, Some(format!("the listing floor {l} is more than {} % under the counted sales' 25th percentile \
            {p25}: posted at the listings, flagged", p.down_cap_bps / 100))));
    }
    let why = if quorum {
        format!("the listing floor {l} is more than {} % over the counted sales' 25th percentile {p25}: held at {} at most", p.up_cap_bps / 100, hi.max(accepted))
    } else if !spread && s.len() >= p.min_sales as usize {
        format!("{} counted sales span {span} blocks; a first post needs them over at least {} (a day of this chain's blocks: audit C3-02)",
            s.len(), b.first_span)
    } else {
        format!("{} counted sales (one per token and per wallet, no self-trade or round trip, within {} % of the listing floor); {} are \
            needed to let the floor rise", s.len(), SALE_BAND_BPS / 100, p.min_sales)
    };
    if accepted == 0 {
        return Err(format!("no first post until the listings are confirmed: {why}"));
    }
    let cap = if quorum { accepted.max(hi) } else { accepted };
    Ok(post(l.min(cap), HELD_CONFIDENCE, Some(why)))
}

/// Whether to post now: the module's post is old, or the price moved enough.
pub fn should_post(price: u128, accepted: u128, target: u128, posted_at: u64, now: u64) -> bool {
    if accepted == 0 || now >= posted_at.saturating_add(POST_EVERY_SECS) {
        return true;
    }
    let reference = if target != 0 { target } else { accepted };
    price.abs_diff(reference).saturating_mul(10_000) > reference.saturating_mul(POST_MOVE_BPS)
}

pub const POST_FLOOR: [u8; 4] = [0x39, 0xfa, 0x36, 0x5e];
pub const ACCEPTED: [u8; 4] = [0x67, 0xfe, 0x5a, 0x0c];
pub const KEEPER: [u8; 4] = [0xac, 0xed, 0x16, 0x61];

pub fn post_floor_call(price: u128, confidence: u16, depth: u32) -> Vec<u8> {
    abi::encode_call(POST_FLOOR, &[Arg::Word(abi::uint_word(price)), Arg::Word(abi::uint_word(confidence as u128)),
        Arg::Word(abi::uint_word(depth as u128))])
}

// ---- the buy bound (§4.3) -------------------------------------------------------------------------------------------------------

/// The most a buy may cost: our floor (our own reading while fresh, never above the module's accepted floor) plus the premium,
/// what arrived less the gas reserve, and the collection's maximum. An error when there is no usable floor.
pub fn buy_cap(own: Option<(u128, u64)>, module_accepted: u128, now: u64, premium_bps: u16, available_wei: u128, max_purchase_wei: u128)
    -> Result<u128, String> {
    let (reading, at) = own.ok_or("no floor reading of ours yet")?;
    if now > at.saturating_add(READING_TTL_SECS) {
        return Err(format!("our floor reading is {} s old, over {READING_TTL_SECS}", now - at));
    }
    if module_accepted == 0 {
        return Err("the module has no accepted floor".into());
    }
    let floor = reading.min(module_accepted);
    let by_floor = floor.saturating_mul(10_000 + premium_bps as u128) / 10_000;
    let by_money = available_wei.checked_sub(ETH_GAS_RESERVE_WEI).ok_or("what arrived does not cover the gas reserve")?;
    Ok(by_floor.min(by_money).min(max_purchase_wei))
}

/// The item's share of the advance: `⌈advanced × paid / arrived⌉`, never above `advanced` (as `relayconv::cost_of`).
pub fn cost_of(advanced: u128, paid: u128, arrived: u128) -> Result<u128, String> {
    if arrived == 0 || paid == 0 {
        return Err("nothing arrived or nothing paid".into());
    }
    if paid > arrived {
        return Err(format!("{paid} wei paid is over the {arrived} that arrived"));
    }
    Ok(advanced.checked_mul(paid).ok_or("cost: overflow")?.div_ceil(arrived).min(advanced))
}

// ---- the wrapper's ABI -----------------------------------------------------------------------------------------------------------

pub const TICKET_COUNT: [u8; 4] = [0xcf, 0xbd, 0x90, 0x0f];
pub const TICKET: [u8; 4] = [0xc1, 0x29, 0xff, 0x32];
pub const ITEM: [u8; 4] = [0x89, 0x8e, 0x62, 0x19];
pub const MINT: [u8; 4] = [0xee, 0xb7, 0x30, 0x71];
pub const MINT_DEPOSIT: [u8; 4] = [0x36, 0xeb, 0x4e, 0xeb];
pub const RECORD_DELIVERY: [u8; 4] = [0x19, 0xa8, 0x35, 0xd9];
pub const MARK_UNDELIVERABLE: [u8; 4] = [0xac, 0xf7, 0x56, 0xcc];
/// Round 3 (D3-H-1): `noteSent(uint256,bytes32)`: the custodian's word that a delivery of this resolved ticket MINED on Ethereum; the
/// wrapper's late window no longer refunds it (only `markUndeliverable` does).
pub const NOTE_SENT: [u8; 4] = [0x6d, 0x65, 0xf7, 0x20];
pub const ITEM_OF: [u8; 4] = [0xfb, 0x04, 0x87, 0x05];
/// Review round 5 (F5-M-1): `sentTx(uint256)` and `undeliverable(uint256)`: what `noteSent` and `markUndeliverable` leave on the
/// wrapper, read back before either is taken as not having happened.
pub const SENT_TX: [u8; 4] = [0x43, 0x76, 0xb4, 0x5d];
pub const UNDELIVERABLE: [u8; 4] = [0xb3, 0x21, 0xa0, 0xaa];
/// Review round 5 (F5-M-1): `FloorModule.advanced()`: the wei the open advance paid (0 when none is open).
pub const ADVANCED: [u8; 4] = [0xa0, 0xdd, 0x2c, 0x03];
pub const CHAIN_ID: [u8; 4] = [0x9a, 0x8a, 0x05, 0x92];
pub const COLLECTION: [u8; 4] = [0x7d, 0xe1, 0xe5, 0x36];
pub const CUSTODIAN: [u8; 4] = [0x37, 0x5b, 0x74, 0xc3];

/// `EthNftWrapper.Stage`.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    None,
    Pinned,
    Resolved,
    Delivered,
    Refunded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TicketV {
    pub stage: Stage,
    pub owner: [u8; 20],
    pub deliver_by: u64,
    pub item_id: u128,
    pub destination: [u8; 20],
}

/// `ticket(id)`: a struct of static fields, returned inline (ten words).
pub fn decode_ticket(r: &[u8]) -> Result<TicketV, String> {
    let stage = match abi::uint_at(r, 0, 0, 8)? {
        0 => Stage::None,
        1 => Stage::Pinned,
        2 => Stage::Resolved,
        3 => Stage::Delivered,
        4 => Stage::Refunded,
        n => return Err(format!("stage {n}")),
    };
    Ok(TicketV { stage, owner: abi::addr_at(r, 0, 1)?, deliver_by: abi::uint_at(r, 0, 5, 64)? as u64, item_id: abi::uint_at(r, 0, 7, 128)?,
        destination: abi::addr_at(r, 0, 9)? })
}

/// Review round 5 (F5-M-1): `ticket(id)`'s `deliveryTx` (word 8): the delivery `recordDelivery` recorded (zero before).
pub fn decode_ticket_tx(r: &[u8]) -> Result<Word, String> {
    Ok(abi::word_at(r, 0, 8)?.try_into().expect("32"))
}

/// `item(id)`: `(tokenId, sourceTx, deposit)`.
pub fn decode_item(r: &[u8]) -> Result<(Word, Word, bool), String> {
    let t: Word = abi::word_at(r, 0, 0)?.try_into().expect("32");
    let s: Word = abi::word_at(r, 0, 1)?.try_into().expect("32");
    Ok((t, s, abi::bool_at(r, 0, 2)?))
}

pub fn call_word(sel: [u8; 4], w: Word) -> Vec<u8> {
    abi::encode_call(sel, &[Arg::Word(w)])
}
pub fn mint_call(deposit: bool, token: Word, tx: Word, to: &[u8; 20]) -> Vec<u8> {
    abi::encode_call(if deposit { MINT_DEPOSIT } else { MINT }, &[Arg::Word(token), Arg::Word(tx), Arg::Word(abi::addr_word(to))])
}
pub fn record_delivery_call(ticket: u128, tx: Word) -> Vec<u8> {
    abi::encode_call(RECORD_DELIVERY, &[Arg::Word(abi::uint_word(ticket)), Arg::Word(tx)])
}
pub fn mark_call(ticket: u128) -> Vec<u8> {
    call_word(MARK_UNDELIVERABLE, abi::uint_word(ticket))
}
pub fn note_sent_call(ticket: u128, tx: Word) -> Vec<u8> {
    abi::encode_call(NOTE_SENT, &[Arg::Word(abi::uint_word(ticket)), Arg::Word(tx)])
}

// ---- the transfer (§4.4) and the deposit (§2.4) --------------------------------------------------------------------------------

pub const TRANSFER_FROM: [u8; 4] = [0x23, 0xb8, 0x72, 0xdd];
pub const SAFE_TRANSFER_FROM_DATA: [u8; 4] = [0xb8, 0x8d, 0x4f, 0xde];
pub const IS_APPROVED_FOR_ALL: [u8; 4] = [0xe9, 0x85, 0xe9, 0xc5];
pub const GET_APPROVED: [u8; 4] = [0x08, 0x18, 0x12, 0xfc];
pub const GET_CONDUIT: [u8; 4] = [0x6e, 0x9b, 0xfd, 0x9f];
/// Seaport's ConduitController (every chain).
pub const CONDUIT_CONTROLLER: [u8; 20] = [0x00, 0x00, 0x00, 0x00, 0xf9, 0x49, 0x00, 0x04, 0xc1, 0x1c, 0xef, 0x24, 0x3f, 0x54, 0x00, 0x49, 0x3c,
    0x00, 0xad, 0x63];
/// The first four bytes of a deposit's `safeTransferFrom` data (as `NftWrapper.DEPOSIT_MAGIC`).
pub const DEPOSIT_MAGIC: [u8; 4] = *b"EWN1";
/// A deposit's data is exactly `"EWN1" ‖ uint32 BE home chain ‖ wrapper (20 bytes) ‖ recipient (20 bytes)` (docs/v4/nft-wrapper.md
/// §4, docs/v7/phase1-nft.md §2.4): the holder's own signature names the chain and the wrapper as well as the recipient.
pub const DEPOSIT_DATA_LEN: usize = 48;
/// A claimed transaction's transfers to us from one collection that are handled (returned) at most; more is left to the operator.
pub const MAX_TRANSFERS_PER_CLAIM: usize = 4;

/// A delivery: `transferFrom(us, destination, token)`. No hook runs in our transaction.
pub fn transfer_from_call(from: &[u8; 20], to: &[u8; 20], token: Word) -> Vec<u8> {
    abi::encode_call(TRANSFER_FROM, &[Arg::Word(abi::addr_word(from)), Arg::Word(abi::addr_word(to)), Arg::Word(token)])
}

/// A delivery may be signed now: resolved, its destination sane, not within a day of its deadline, and we still own the token.
pub fn judge_transfer(t: &TicketV, owner_now: &[u8; 20], us: &[u8; 20], now: u64) -> Result<(), String> {
    if t.stage != Stage::Resolved {
        return Err(format!("the ticket is {:?}, not resolved", t.stage));
    }
    if t.destination == [0; 20] || t.destination == *us {
        return Err("the ticket's destination is nobody or us (a relist ticket): never delivered".into());
    }
    if now.saturating_add(DELIVERY_STOP_SECS) >= t.deliver_by {
        return Err(format!("within a day of its deadline {}: not signed (it refunds after the canister marks it)", t.deliver_by));
    }
    if owner_now != us {
        return Err(format!("the token's owner is 0x{}, not us", hex::encode(owner_now)));
    }
    Ok(())
}

/// The data a holder puts in their deposit's `safeTransferFrom` (the site and the docs build exactly this).
pub fn deposit_data(home_chain: u64, wrapper: &[u8; 20], recipient: &[u8; 20]) -> Vec<u8> {
    [&DEPOSIT_MAGIC[..], &(home_chain as u32).to_be_bytes(), &wrapper[..], &recipient[..]].concat()
}

/// Where a deposit's units go, and the deployment it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepositTarget {
    pub home_chain: u64,
    pub wrapper: [u8; 20],
    pub us: [u8; 20],
}

/// A holder's deposit, from the transaction and its receipt: `safeTransferFrom(from, us, id, "EWN1" ‖ chain ‖ wrapper ‖ recipient)`
/// to the collection, sent by `from` itself, succeeded, with exactly one `Transfer(from, us, id)` from the collection, and its
/// data naming this home chain and this collection's wrapper.
pub fn judge_deposit(tx_from: &[u8; 20], tx_to: &[u8; 20], input: &[u8], r: &ReceiptV, collection: &[u8; 20], t: &DepositTarget)
    -> Result<(Word, [u8; 20]), String> {
    let us = &t.us;
    if tx_to != collection {
        return Err("the transaction is not to the collection".into());
    }
    if !r.success {
        return Err("the transaction reverted".into());
    }
    if input.len() < 4 || input[..4] != SAFE_TRANSFER_FROM_DATA {
        return Err("not safeTransferFrom(address,address,uint256,bytes)".into());
    }
    let args = &input[4..];
    let from = abi::addr_at(args, 0, 0)?;
    let to = abi::addr_at(args, 0, 1)?;
    let token: Word = abi::word_at(args, 0, 2)?.try_into().expect("32");
    let data = abi::bytes_at(args, 0, 3)?;
    if from != *tx_from {
        return Err("the NFT's owner did not send the transaction themselves".into());
    }
    if to != *us {
        return Err("the NFT was not sent to us".into());
    }
    if data.len() != DEPOSIT_DATA_LEN || data[..4] != DEPOSIT_MAGIC {
        return Err("the transfer's data is not \"EWN1\" ‖ chain ‖ wrapper ‖ recipient (48 bytes)".into());
    }
    if u32::from_be_bytes(data[4..8].try_into().expect("4")) as u64 != t.home_chain {
        return Err(format!("the deposit names chain {}, not {}", u32::from_be_bytes(data[4..8].try_into().expect("4")), t.home_chain));
    }
    if data[8..28] != t.wrapper[..] {
        return Err(format!("the deposit names wrapper 0x{}, not this collection's 0x{}", hex::encode(&data[8..28]), hex::encode(t.wrapper)));
    }
    let recipient: [u8; 20] = data[28..48].try_into().expect("20");
    if recipient == [0; 20] || recipient == t.wrapper || recipient == *us {
        return Err("the recipient is nobody, the wrapper or the custodian".into());
    }
    let ours: Vec<&LogV> = r.logs.iter().filter(|l| l.address == *collection && l.topics.first() == Some(&TRANSFER)).collect();
    if ours.len() != 1 {
        return Err(format!("{} Transfer logs from the collection, not one", ours.len()));
    }
    let l = ours[0];
    if l.topics.len() != 4 || addr_w(&l.topics[1]) != from || addr_w(&l.topics[2]) != *us || l.topics[3] != token {
        return Err("the Transfer log is not (from, us, id)".into());
    }
    Ok((token, recipient))
}

/// Every ERC-721 `Transfer(from, us, id)` the collection logged in a receipt, in log order: `(from, id)`.
pub fn inbound_transfers(r: &ReceiptV, collection: &[u8; 20], us: &[u8; 20]) -> Vec<([u8; 20], Word)> {
    r.logs.iter()
        .filter(|l| l.address == *collection && l.topics.len() == 4 && l.topics[0] == TRANSFER && l.topics[1][..12] == [0; 12]
            && l.topics[2][..12] == [0; 12] && addr_w(&l.topics[2]) == *us)
        .map(|l| (addr_w(&l.topics[1]), l.topics[3]))
        .collect()
}

/// What one claimed transaction is, for one configured collection (docs/v7/phase1-nft.md §2.4, D5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// A valid deposit: one unit of `token` to `recipient`, once final.
    Deposit { token: Word, recipient: [u8; 20], from: [u8; 20] },
    /// NFTs of the collection that reached us without a valid claim (no data, garbled data, another wrapper, an approved operator
    /// or a contract moved them): each goes back to the `from` of its own `Transfer`, once final. `why` is the deposit judge's.
    Strays { transfers: Vec<([u8; 20], Word)>, why: String },
    /// Nothing of this collection reached us in this transaction (or it reverted).
    Nothing,
}

/// The claim judge: a valid deposit, or strays to return, or nothing. Pure; the caller reads the transaction and its receipt at
/// the pin.
pub fn judge_claim(tx_from: &[u8; 20], tx_to: &[u8; 20], input: &[u8], r: &ReceiptV, collection: &[u8; 20], t: &DepositTarget) -> Claim {
    if !r.success {
        return Claim::Nothing;
    }
    let inbound = inbound_transfers(r, collection, &t.us);
    if inbound.is_empty() {
        return Claim::Nothing;
    }
    match judge_deposit(tx_from, tx_to, input, r, collection, t) {
        Ok((token, recipient)) if inbound.len() == 1 && inbound[0].1 == token => Claim::Deposit { token, recipient, from: inbound[0].0 },
        Ok(_) => Claim::Strays { transfers: inbound, why: "more than one transfer to us".into() },
        Err(why) => Claim::Strays { transfers: inbound, why },
    }
}

/// What the chains say about a stray before its return is signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReturnFacts {
    /// `ownerOf(token)` at the Ethereum pin.
    pub owner: [u8; 20],
    /// The wrapper's `itemOf(token)` at the home-chain pin: non-zero while the wrapper holds it.
    pub item_of: u128,
    /// The token is part of an open buy of ours, or of another claim not yet settled.
    pub busy: bool,
    /// The last block in which a transfer of ours moved this token out (a delivery or an earlier return), if any.
    pub last_out: Option<u64>,
}

/// A stray may go back to `dest` (its own `Transfer`'s `from`) only if it is ours now, the wrapper does not hold it, nothing of
/// ours is using it, and it has not left us since it came in (else the transfer we were shown is not the one that brought it).
pub fn judge_return(dest: &[u8; 20], came_in: u64, f: &ReturnFacts, us: &[u8; 20]) -> Result<(), String> {
    if *dest == [0; 20] || dest == us {
        return Err("no sender to return it to (minted to us, or sent by ourselves)".into());
    }
    if f.owner != *us {
        return Err(format!("the token's owner is 0x{}, not us", hex::encode(f.owner)));
    }
    if f.item_of != 0 {
        return Err("the wrapper holds this token: it backs a unit and is never returned".into());
    }
    if f.busy {
        return Err("the token is part of an open buy or claim of ours".into());
    }
    if f.last_out.is_some_and(|b| b >= came_in) {
        return Err(format!("the token left us in block {} after it came in (block {came_in}): this is not the transfer that brought it",
            f.last_out.unwrap_or(0)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f5_m1_the_read_back_selectors() {
        let k = |x: &str| crate::evmtx::keccak(x.as_bytes())[..4].to_vec();
        assert_eq!(SENT_TX.to_vec(), k("sentTx(uint256)"));
        assert_eq!(UNDELIVERABLE.to_vec(), k("undeliverable(uint256)"));
        assert_eq!(ADVANCED.to_vec(), k("advanced()"));
        let mut t = vec![0u8; 320];
        t[8 * 32..9 * 32].copy_from_slice(&[7u8; 32]);
        assert_eq!(decode_ticket_tx(&t), Ok([7u8; 32]));
    }

    const MILADY: [u8; 20] = [0x5a, 0xf0, 0xd9, 0x82, 0x7e, 0x0c, 0x53, 0xe4, 0x79, 0x9b, 0xb2, 0x26, 0x65, 0x5a, 0x1d, 0xe1, 0x52, 0xa4, 0x25, 0xa5];
    const FILL: &str = include_str!("../tests/fixtures/seaport/eth-milady-fill-26055691.json");
    const SALES: &str = include_str!("../tests/fixtures/seaport/milady-sales-receipts.json");
    const E: u128 = 1_000_000_000_000_000_000;

    fn receipts() -> Vec<(String, ReceiptV)> {
        let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
        v["receipts"].as_object().unwrap().iter().map(|(k, r)| (k.clone(), receipt_of(r).unwrap().unwrap())).collect()
    }
    fn rec(h: &str) -> ReceiptV {
        receipts().into_iter().find(|(k, _)| k.starts_with(h)).unwrap().1
    }
    fn fill_listing() -> Listing {
        let v: serde_json::Value = serde_json::from_str(FILL).unwrap();
        Listing::parse_hex(v["input"].as_str().unwrap()).unwrap()
    }

    fn tp(v: Vec<Sale>) -> Vec<(Word, u128)> {
        v.into_iter().map(|s| (s.token, s.price)).collect()
    }

    #[test]
    fn real_sales_are_read_from_real_receipts() {
        // a Seaport basic listing fill: 0.999899999 ETH for #9180 (the offerer's 0.9899 plus OpenSea's 1 %)
        let f = sales_in(&rec("0x404e2c74"), &MILADY);
        assert_eq!(tp(f.clone()), vec![(sp::word_u128(9_180), 999_899_999_000_000_000)]);
        // who sold and who bought, from the collection's own Transfer: the listing's offerer to the fulfiller
        let v: serde_json::Value = serde_json::from_str(FILL).unwrap();
        assert_eq!(format!("0x{}", hex::encode(f[0].seller)), v["offerer"].as_str().unwrap());
        assert_eq!(format!("0x{}", hex::encode(f[0].buyer)), v["fulfiller"].as_str().unwrap());
        assert_ne!(f[0].seller, f[0].buyer);
        // a listing matched to a bid in one transaction: one token, the lower of the two
        let m = sales_in(&rec("0xf499e73d"), &MILADY);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].price, 891_990_000_000_000_000);
        // Blur v2, a listing and a bid
        assert_eq!(sales_in(&rec("0x8b4ae48b"), &MILADY)[0].price, 947_801_150_000_000_000);
        assert_eq!(sales_in(&rec("0x54a344cf"), &MILADY)[0].price, 890_000_000_000_000_000);
        // a Seaport bid
        assert_eq!(sales_in(&rec("0xa578a2da"), &MILADY)[0].price, 910_000_000_000_000_000);
        // every real sale names a seller and a buyer, different ones
        for (_, r) in receipts() {
            for x in sales_in(&r, &MILADY) {
                assert!(x.seller != [0; 20] && x.buyer != [0; 20] && x.seller != x.buyer, "{x:?}");
            }
        }
        // another collection's lens sees nothing; a reverted receipt nothing
        assert!(sales_in(&rec("0x404e2c74"), &[1; 20]).is_empty());
        let mut r = rec("0x404e2c74");
        r.success = false;
        assert!(sales_in(&r, &MILADY).is_empty());
        // no Transfer of the token in the receipt: not a sale (an OrderFulfilled alone moves nothing)
        let mut r = rec("0x404e2c74");
        r.logs.retain(|l| l.topics.first() != Some(&TRANSFER));
        assert!(sales_in(&r, &MILADY).is_empty());
        // the window, newest first
        let all: Vec<ReceiptV> = receipts().into_iter().map(|(_, r)| r).collect();
        let w = sales_window(&all, &MILADY, 26_068_928);
        assert_eq!(w.len(), 5);
        assert!(w.windows(2).all(|x| x[0].block >= x[1].block));
        assert_eq!(sales_window(&all, &MILADY, 26_062_135 - 1).len(), 2, "blocks above the pin are not believed");
        assert_eq!(sales_window(&all, &MILADY, 26_055_691 + SALE_WINDOW_BLOCKS + 1).len(), 4, "older than 7 days: out");
    }

    #[test]
    fn the_projection_keeps_what_providers_agree_on_and_nothing_above_the_pin() {
        let v: serde_json::Value = serde_json::from_str(SALES).unwrap();
        let (_, raw) = v["receipts"].as_object().unwrap().iter().next().unwrap();
        let p = receipt_projection(raw, 26_068_928);
        assert_eq!(receipt_of(&p).unwrap().unwrap(), receipt_of(raw).unwrap().unwrap());
        assert!(receipt_projection(raw, 26_000_000).is_null());
    }

    #[test]
    fn a_real_listing_is_evidence_and_each_lie_is_refused() {
        let l = fill_listing();
        let t = l.terms(&MILADY).unwrap();
        assert_eq!(t.price, 999_899_999_000_000_000);
        assert_eq!(t.token, sp::word_u128(9_180));
        let v: serde_json::Value = serde_json::from_str(FILL).unwrap();
        let hash = l.order_hash([0; 32]);
        assert_eq!(format!("0x{}", hex::encode(hash)), v["order_hash"].as_str().unwrap(), "our order hash is Seaport's");
        let signed = sp::offerer_signed(&hash, &l.offerer(), l.signature(), 1, &sp::SEAPORT_1_6).unwrap();
        let facts = ListingFacts { pin_timestamp: v["fork_block_timestamp"].as_u64().unwrap(), owner: l.offerer(), cancelled: false,
            filled: 0, size: 0, approved: true, signed };
        assert!(signed, "an EOA signature, recovered here");
        let s = judge_listing(&t, &facts, 60).unwrap();
        assert_eq!(s.price, t.price);
        assert!(judge_listing(&t, &ListingFacts { owner: [1; 20], ..facts.clone() }, 60).unwrap_err().contains("owner"));
        assert!(judge_listing(&t, &ListingFacts { cancelled: true, ..facts.clone() }, 60).unwrap_err().contains("cancelled"));
        assert!(judge_listing(&t, &ListingFacts { filled: 1, size: 1, ..facts.clone() }, 60).unwrap_err().contains("filled"));
        assert!(judge_listing(&t, &ListingFacts { approved: false, ..facts.clone() }, 60).unwrap_err().contains("approved"));
        assert!(judge_listing(&t, &ListingFacts { signed: false, ..facts.clone() }, 60).unwrap_err().contains("signature"));
        assert!(judge_listing(&t, &ListingFacts { pin_timestamp: t.end, ..facts.clone() }, 60).unwrap_err().contains("not live"));
        assert!(judge_listing(&t, &ListingFacts { pin_timestamp: t.start - 1, ..facts.clone() }, 60).unwrap_err().contains("not live"));
        assert!(l.terms(&[1; 20]).unwrap_err().contains("this collection"));
        // a tampered price no longer recovers to the offerer
        let Listing::Basic(mut o) = l.clone() else { panic!() };
        o.consideration_amount -= 1;
        assert!(sp::offerer_signed(&Listing::Basic(o).order_hash([0; 32]), &l.offerer(), l.signature(), 1, &sp::SEAPORT_1_6).is_err());
    }

    fn seen(o: u8, t: u128, p: u128) -> Seen {
        Seen { offerer: [o; 20], token: sp::word_u128(t), price: p }
    }
    /// A sale of token `t` by seller `[s; 20]` to buyer `[s + 100; 20]` (distinct wallets) at `p`.
    fn sale(s: u8, t: u128, p: u128) -> Sale {
        Sale { token: sp::word_u128(t), price: p, seller: [s; 20], buyer: [s.wrapping_add(100); 20], block: 1 }
    }
    /// Sales a day apart (audit C3-02: a first post's counted sales span a day of the chain's blocks).
    fn sales_at(ps: &[u128]) -> Vec<Sale> {
        ps.iter().enumerate().map(|(i, p)| Sale { block: 1 + i as u64 * 7_200, ..sale(i as u8 + 1, 100 + i as u128, *p) }).collect()
    }

    #[test]
    fn the_floor_is_the_ranked_listing_and_sales_only_hold_it_back() {
        let p = FloorParams::default();
        let sales = sales_at(&[91 * E / 100, 90 * E / 100, 95 * E / 100, 89 * E / 100, 91 * E / 100]);
        let ls = [seen(1, 1, 93 * E / 100), seen(2, 2, 94 * E / 100), seen(3, 3, 97 * E / 100), seen(4, 4, 99 * E / 100)];
        let f = floor_from(&ls, &sales, &p, 0).unwrap();
        assert_eq!(f.listing, 94 * E / 100, "rank 2: one listing does not set the floor");
        assert_eq!(f.price, 94 * E / 100);
        assert_eq!(f.sale_median, 91 * E / 100);
        assert_eq!(f.sale_p25, 90 * E / 100);
        assert_eq!(f.confidence, 10_000, "3.3 % from the median");
        assert_eq!(f.depth, 3, "distinct offerers at most 5 % over the post (0.93, 0.94, 0.97; not 0.99)");
        assert!(!f.clamped && f.held.is_none());
        // the cheapest per offerer: one offerer's three listings are one voice
        let sybil = [seen(1, 1, 50 * E / 100), seen(1, 2, 51 * E / 100), seen(1, 3, 52 * E / 100), seen(2, 4, 94 * E / 100)];
        assert!(floor_from(&sybil, &sales, &p, 93 * E / 100).unwrap_err().contains("distinct offerers"));
        // a token listed twice counts once
        let twice = [seen(1, 1, 93 * E / 100), seen(2, 1, 50 * E / 100), seen(3, 3, 97 * E / 100)];
        assert!(floor_from(&twice, &sales, &p, 93 * E / 100).is_err());
        // withheld cheap listings: a listing floor more than 10 % over the counted sales' 25th percentile (0.99) is HELD: the
        // module keeps its floor, fresh, and rises at most to 0.99; a first post is not made at all
        let dear = [seen(1, 1, 105 * E / 100), seen(2, 2, 106 * E / 100), seen(3, 3, 107 * E / 100)];
        assert!(floor_from(&dear, &sales, &p, 0).unwrap_err().contains("no first post"));
        let held = floor_from(&dear, &sales, &p, 93 * E / 100).unwrap();
        assert_eq!(held.price, 99 * E / 100);
        assert_eq!(held.confidence, HELD_CONFIDENCE);
        assert!(held.held.as_ref().unwrap().contains("25th percentile"));
        assert_eq!(floor_from(&dear, &sales, &p, E).unwrap().price, E, "never lower than the module's floor because of a veto");
        // a listing floor below the market's sales is a price anyone can fill: it is posted (there is no downward veto any more)
        let cheap = [seen(1, 1, 74 * E / 100), seen(2, 2, 75 * E / 100), seen(3, 3, 76 * E / 100)];
        let f = floor_from(&cheap, &sales, &p, 93 * E / 100).unwrap();
        assert_eq!((f.price, f.confidence), (75 * E / 100, HELD_CONFIDENCE));
        assert!(f.held.as_ref().unwrap().contains("under the counted sales' 25th percentile"), "posted, flagged");
        // too few counted sales: held (never up, down with the listings), and no first post
        assert!(floor_from(&ls, &sales[..2], &p, 0).unwrap_err().contains("counted sales"));
        assert_eq!(floor_from(&ls, &sales[..2], &p, 90 * E / 100).unwrap().price, 90 * E / 100, "the floor does not rise");
        assert_eq!(floor_from(&ls, &sales[..2], &p, E).unwrap().price, 94 * E / 100, "but it follows the listings down");
        assert!(floor_from(&ls[..2], &sales, &p, 93 * E / 100).is_err());
        // in between: 7,000
        let mid = [seen(1, 1, 96 * E / 100), seen(2, 2, 97 * E / 100), seen(3, 3, E)];
        assert_eq!(floor_from(&mid, &sales, &p, 0).unwrap().confidence, 7_000);
    }

    #[test]
    fn b_h_1_wash_sales_never_steer_the_post_they_can_only_hold_it() {
        // audit 2026-09-27-nft B-H-1's PoC: with listings at 0.92 / 0.93 / 0.95 ETH, three sales at 1.5 ETH posted 1.275 ETH and three
        // at 0.093 ETH posted 0.1023 ETH. Now the post is at most the listing floor 0.93, and never above the module's floor unless the
        // counted sales confirm it.
        let p = FloorParams::default();
        let ls = [seen(1, 1, 92 * E / 100), seen(2, 2, 93 * E / 100), seen(3, 3, 95 * E / 100)];
        for x in [3 * E / 2, 93 * E / 1_000] {
            assert!(floor_from(&ls, &sales_at(&[x, x, x]), &p, 0).is_err(), "far wash sales are no vote: no first post");
            assert_eq!(floor_from(&ls, &sales_at(&[x, x, x]), &p, 90 * E / 100).unwrap().price, 90 * E / 100, "held, never steered");
        }
        // self-trades (the seller is the buyer) never count, whatever the price
        let selfs: Vec<Sale> = (0..12u8).map(|i| Sale { buyer: [i + 1; 20], ..sale(i + 1, 50 + i as u128, 92 * E / 100) }).collect();
        assert!(floor_from(&ls, &selfs, &p, 0).unwrap_err().contains("counted sales"));
        // one wallet selling (or buying) again and again is one sale
        let one_seller: Vec<Sale> = (0..12u8).map(|i| Sale { buyer: [200 - i; 20], ..sale(9, 50 + i as u128, 92 * E / 100) }).collect();
        assert!(floor_from(&ls, &one_seller, &p, 0).is_err());
        let one_buyer: Vec<Sale> = (0..12u8).map(|i| Sale { buyer: [250; 20], ..sale(i + 1, 50 + i as u128, 92 * E / 100) }).collect();
        assert!(floor_from(&ls, &one_buyer, &p, 0).is_err());
        // wash sales near the listings (distinct wallets) can at most hold the floor back; a post is never above the listings
        for x in [80 * E / 100, 88 * E / 100, 93 * E / 100, 99 * E / 100, 110 * E / 100, 120 * E / 100] {
            for acc in [0, 85 * E / 100, 93 * E / 100, E] {
                if let Ok(f) = floor_from(&ls, &sales_at(&[x, x, x, x]), &p, acc) {
                    assert!(f.price <= 93 * E / 100, "the post never leaves the listings");
                    assert!(f.price >= acc.min(93 * E / 100), "a veto never pushes the floor down");
                }
            }
        }
    }

    #[test]
    fn round_2_a_wallet_is_one_voice_and_a_round_trip_is_none() {
        let l = 93 * E / 100;
        let s = |t: u128, from: u8, to: u8, p: u128, block: u64| Sale { token: sp::word_u128(t), price: p, seller: [from; 20], buyer: [to; 20], block };
        // one token ping-ponged between two wallets: a round trip, never counted
        let ping: Vec<Sale> = (0..10u64).map(|i| if i % 2 == 0 { s(7, 1, 2, l, 100 - i) } else { s(7, 2, 1, l, 100 - i) }).collect();
        assert!(counted_sales(&ping, l).is_empty());
        // a ring A → B → C → A of one token: none counted
        assert!(counted_sales(&[s(8, 3, 1, l, 3), s(8, 2, 3, l, 2), s(8, 1, 2, l, 1)], l).is_empty());
        // a chain of seven wallets and seven tokens: every other link (a wallet is a party to one counted sale)
        let chain: Vec<Sale> = (0..7u8).map(|i| s(100 + i as u128, 0x50 + i, 0x51 + i, l, 100 - i as u64)).collect();
        assert_eq!(counted_sales(&chain, l).len(), 4);
        // disjoint pairs each count
        let pairs: Vec<Sale> = (0..7u8).map(|i| s(200 + i as u128, 0x60 + 2 * i, 0x61 + 2 * i, l, 100 - i as u64)).collect();
        assert_eq!(counted_sales(&pairs, l).len(), 7);
        // a bidder buying from five sellers is one voice
        let bidder: Vec<Sale> = (0..5u8).map(|i| s(300 + i as u128, 0x10 + i, 0xbb, l, 100 - i as u64)).collect();
        assert_eq!(counted_sales(&bidder, l).len(), 1);
    }

    #[test]
    fn round_2_the_transfer_logs_transform_keeps_moves_between_two_wallets() {
        let t = format!("0x{}", hex::encode(TRANSFER));
        let w = |a: u8| format!("0x{}{}", "00".repeat(12), format!("{a:02x}").repeat(20));
        let tx = |a: u8| format!("0x{}", format!("{a:02X}").repeat(32));
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": [
            { "transactionHash": tx(2), "blockNumber": "0x20", "topics": [t, w(1), w(2), w(9)], "logIndex": "0x1", "removed": false },
            { "transactionHash": tx(1), "blockNumber": "0x10", "topics": [t, w(1), w(3), w(9)] },
            { "transactionHash": tx(1), "blockNumber": "0x10", "topics": [t, w(3), w(4), w(9)] },
            { "transactionHash": tx(3), "blockNumber": "0x11", "topics": [t, w(0), w(4), w(9)] },
            { "transactionHash": tx(4), "blockNumber": "0x12", "topics": [t, w(5), w(5), w(9)] },
            { "transactionHash": tx(5), "blockNumber": "0x13", "topics": [t, w(5), w(6)] },
            { "transactionHash": tx(6), "blockNumber": "0x14", "topics": [t, w(5), w(6), w(9)], "removed": true }
        ] });
        let out: serde_json::Value = serde_json::from_slice(&transfer_logs_body(body.to_string().as_bytes())).unwrap();
        let v = transfer_txs_of(&out["result"]).unwrap();
        assert_eq!(v, vec![([1; 32], 0x10), ([2; 32], 0x20)], "a mint, a self-move, an ERC-20 and a removed log are not moves; one per tx");
        // round 3 (B3-M-1): each move names its token and its sender (for a transaction with several, the least sender)
        let m = transfer_moves_of(&out["result"]).unwrap();
        let mut nine = [0u8; 32];
        nine[12..].fill(9);
        assert_eq!((m[0].token, m[0].from, m[1].from), (nine, [1; 20], [1; 20]));
        let rev = serde_json::json!({ "result": [
            { "transactionHash": tx(1), "blockNumber": "0x10", "topics": [t, w(3), w(4), w(9)] },
            { "transactionHash": tx(1), "blockNumber": "0x10", "topics": [t, w(1), w(3), w(9)] },
            { "transactionHash": tx(2), "blockNumber": "0x20", "topics": [t, w(1), w(2), w(9)] } ] });
        assert_eq!(transfer_logs_body(body.to_string().as_bytes()), transfer_logs_body(rev.to_string().as_bytes()), "the provider's log order does not matter");
        // a round-2 entry (hash and block only) is its own group
        let old = transfer_moves_of(&serde_json::json!([[tx(7), "0x5"]])).unwrap();
        assert_eq!((old[0].token, old[0].from), ([7; 32], [7; 20]));
        // another provider's field order and case give the same bytes
        let body2 = serde_json::json!({ "result": [
            { "removed": false, "topics": [t.to_uppercase().replace("0X", "0x"), w(1), w(3), w(9)], "blockNumber": "0x10", "transactionHash": tx(1).to_lowercase() },
            { "topics": [t, w(1), w(2), w(9)], "blockNumber": "0x20", "transactionHash": tx(2) } ], "id": 7, "jsonrpc": "2.0" });
        assert_eq!(transfer_logs_body(body.to_string().as_bytes()), transfer_logs_body(body2.to_string().as_bytes()));
        assert!(String::from_utf8(transfer_logs_body(b"<html>busy</html>")).unwrap().contains("unusable"));
    }

    #[test]
    fn posting_cadence() {
        assert!(should_post(E, 0, 0, 0, 100));
        assert!(!should_post(E, E, E, 1_000, 1_000 + POST_EVERY_SECS - 1));
        assert!(should_post(E, E, E, 1_000, 1_000 + POST_EVERY_SECS));
        assert!(should_post(E * 102 / 100, E, E, 1_000, 1_001));
        assert!(!should_post(E * 1_005 / 1_000, E, E, 1_000, 1_001));
        assert_eq!(hex::encode(&post_floor_call(1, 2, 3)[..4]), "39fa365e");
    }

    #[test]
    fn the_buy_cap() {
        let now = 10_000;
        // our reading 0.93, the module 0.94: the lower, plus 5 %
        assert_eq!(buy_cap(Some((93 * E / 100, now)), 94 * E / 100, now, 500, 10 * E, 25 * E).unwrap(), 93 * E / 100 * 10_500 / 10_000);
        // what arrived binds, less the reserve
        assert_eq!(buy_cap(Some((93 * E / 100, now)), 94 * E / 100, now, 500, E / 2, 25 * E).unwrap(), E / 2 - ETH_GAS_RESERVE_WEI);
        // the collection's maximum binds
        assert_eq!(buy_cap(Some((93 * E / 100, now)), 94 * E / 100, now, 500, 10 * E, E / 2).unwrap(), E / 2);
        assert!(buy_cap(None, E, now, 500, E, E).is_err());
        assert!(buy_cap(Some((E, now - READING_TTL_SECS - 1)), E, now, 500, E, E).unwrap_err().contains("old"));
        assert!(buy_cap(Some((E, now)), 0, now, 500, E, E).is_err());
        assert!(buy_cap(Some((E, now)), E, now, 500, 1, E).is_err());
        assert_eq!(cost_of(E, 99 * E / 100, E), Ok(99 * E / 100));
        assert_eq!(cost_of(E, 1, 3), Ok(E.div_ceil(3)));
        assert!(cost_of(E, 2, 1).is_err());
    }

    #[test]
    fn the_wrapper_calls_match_the_contract() {
        assert_eq!(hex::encode(&mint_call(false, [0; 32], [0; 32], &[0; 20])[..4]), "eeb73071");
        assert_eq!(hex::encode(&mint_call(true, [0; 32], [0; 32], &[0; 20])[..4]), "36eb4eeb");
        assert_eq!(hex::encode(&record_delivery_call(1, [0; 32])[..4]), "19a835d9");
        assert_eq!(hex::encode(&mark_call(1)[..4]), "acf756cc");
        assert_eq!(hex::encode(&transfer_from_call(&[1; 20], &[2; 20], sp::word_u128(9)))[..8], *"23b872dd");
        let mut t = vec![0u8; 320];
        t[31] = 2;
        t[44..64].copy_from_slice(&[7; 20]);
        t[5 * 32 + 31] = 99;
        t[7 * 32 + 31] = 3;
        t[9 * 32 + 12..9 * 32 + 32].copy_from_slice(&[8; 20]);
        let v = decode_ticket(&t).unwrap();
        assert_eq!((v.stage, v.owner, v.deliver_by, v.item_id, v.destination), (Stage::Resolved, [7; 20], 99, 3, [8; 20]));
        t[31] = 9;
        assert!(decode_ticket(&t).is_err());
    }

    #[test]
    fn a_transfer_is_judged() {
        let us = [5; 20];
        let t = TicketV { stage: Stage::Resolved, owner: [1; 20], deliver_by: 1_000_000, item_id: 0, destination: [2; 20] };
        assert_eq!(judge_transfer(&t, &us, &us, 1_000), Ok(()));
        assert!(judge_transfer(&TicketV { stage: Stage::Pinned, ..t.clone() }, &us, &us, 1_000).is_err());
        assert!(judge_transfer(&TicketV { destination: us, ..t.clone() }, &us, &us, 1_000).is_err());
        assert!(judge_transfer(&t, &us, &us, 1_000_000 - DELIVERY_STOP_SECS).unwrap_err().contains("deadline"));
        assert!(judge_transfer(&t, &[9; 20], &us, 1_000).unwrap_err().contains("owner"));
    }

    fn deposit_tx(from: [u8; 20], to: [u8; 20], token: u128, data: &[u8]) -> Vec<u8> {
        abi::encode_call(SAFE_TRANSFER_FROM_DATA, &[Arg::Word(abi::addr_word(&from)), Arg::Word(abi::addr_word(&to)),
            Arg::Word(sp::word_u128(token)), Arg::Bytes(data)])
    }
    fn transfer_log(from: [u8; 20], to: [u8; 20], token: u128) -> LogV {
        LogV { address: MILADY, topics: vec![TRANSFER, abi::addr_word(&from), abi::addr_word(&to), sp::word_u128(token)], data: vec![] }
    }

    #[test]
    fn a_deposit_is_judged_from_the_holders_own_transaction() {
        let (holder, us, rcpt, wrapper) = ([1; 20], [5; 20], [9; 20], [0xaa; 20]);
        let t = DepositTarget { home_chain: 4663, wrapper, us };
        let data = deposit_data(4663, &wrapper, &rcpt);
        assert_eq!(data.len(), DEPOSIT_DATA_LEN);
        let r = ReceiptV { block: 1, success: true, logs: vec![transfer_log(holder, us, 42)] };
        let input = deposit_tx(holder, us, 42, &data);
        assert_eq!(judge_deposit(&holder, &MILADY, &input, &r, &MILADY, &t), Ok((sp::word_u128(42), rcpt)));
        assert!(judge_deposit(&[2; 20], &MILADY, &input, &r, &MILADY, &t).unwrap_err().contains("owner"));
        assert!(judge_deposit(&holder, &[3; 20], &input, &r, &MILADY, &t).is_err());
        assert!(judge_deposit(&holder, &MILADY, &deposit_tx(holder, [6; 20], 42, &data), &r, &MILADY, &t).is_err());
        assert!(judge_deposit(&holder, &MILADY, &deposit_tx(holder, us, 42, b"nope"), &r, &MILADY, &t).is_err());
        // the old 36-byte form (a recipient alone) names no chain or wrapper: refused
        let old = [&DEPOSIT_MAGIC[..], &abi::addr_word(&rcpt)].concat();
        assert!(judge_deposit(&holder, &MILADY, &deposit_tx(holder, us, 42, &old), &r, &MILADY, &t).unwrap_err().contains("48 bytes"));
        // another chain, another wrapper, or a recipient that would lock the unit
        assert!(judge_deposit(&holder, &MILADY, &deposit_tx(holder, us, 42, &deposit_data(1, &wrapper, &rcpt)), &r, &MILADY, &t)
            .unwrap_err().contains("chain"));
        assert!(judge_deposit(&holder, &MILADY, &deposit_tx(holder, us, 42, &deposit_data(4663, &[0xbb; 20], &rcpt)), &r, &MILADY, &t)
            .unwrap_err().contains("wrapper"));
        for bad in [[0; 20], wrapper, us] {
            assert!(judge_deposit(&holder, &MILADY, &deposit_tx(holder, us, 42, &deposit_data(4663, &wrapper, &bad)), &r, &MILADY, &t).is_err());
        }
        let r2 = ReceiptV { success: false, ..r.clone() };
        assert!(judge_deposit(&holder, &MILADY, &input, &r2, &MILADY, &t).is_err());
        let r3 = ReceiptV { logs: vec![transfer_log(holder, us, 42), transfer_log(holder, us, 43)], ..r.clone() };
        assert!(judge_deposit(&holder, &MILADY, &input, &r3, &MILADY, &t).is_err());
        let r4 = ReceiptV { logs: vec![transfer_log(holder, us, 43)], ..r.clone() };
        assert!(judge_deposit(&holder, &MILADY, &input, &r4, &MILADY, &t).is_err());
        let plain = abi::encode_call(TRANSFER_FROM, &[Arg::Word(abi::addr_word(&holder)), Arg::Word(abi::addr_word(&us)), Arg::Word(sp::word_u128(42))]);
        assert!(judge_deposit(&holder, &MILADY, &plain, &r, &MILADY, &t).is_err(), "no claim data: a stray, never minted");
    }

    #[test]
    fn a_claim_is_a_deposit_strays_to_return_or_nothing() {
        let (holder, us, rcpt, wrapper) = ([1; 20], [5; 20], [9; 20], [0xaa; 20]);
        let t = DepositTarget { home_chain: 4663, wrapper, us };
        let data = deposit_data(4663, &wrapper, &rcpt);
        let r = ReceiptV { block: 1, success: true, logs: vec![transfer_log(holder, us, 42)] };
        assert_eq!(judge_claim(&holder, &MILADY, &deposit_tx(holder, us, 42, &data), &r, &MILADY, &t),
            Claim::Deposit { token: sp::word_u128(42), recipient: rcpt, from: holder });
        // a plain transferFrom: one stray, back to its sender
        let plain = abi::encode_call(TRANSFER_FROM, &[Arg::Word(abi::addr_word(&holder)), Arg::Word(abi::addr_word(&us)), Arg::Word(sp::word_u128(42))]);
        let Claim::Strays { transfers, .. } = judge_claim(&holder, &MILADY, &plain, &r, &MILADY, &t) else { panic!() };
        assert_eq!(transfers, vec![(holder, sp::word_u128(42))]);
        // moved by an approved operator (a marketplace, a Safe): the return goes to the owner in the log, not the sender
        let Claim::Strays { transfers, .. } = judge_claim(&[7; 20], &[8; 20], &[0xde, 0xad], &r, &MILADY, &t) else { panic!() };
        assert_eq!(transfers, vec![(holder, sp::word_u128(42))]);
        // a batch: every transfer to us, each to its own sender
        let rb = ReceiptV { logs: vec![transfer_log(holder, us, 42), transfer_log([3; 20], us, 43), transfer_log(holder, [6; 20], 44)], ..r.clone() };
        let Claim::Strays { transfers, .. } = judge_claim(&holder, &[8; 20], &[], &rb, &MILADY, &t) else { panic!() };
        assert_eq!(transfers, vec![(holder, sp::word_u128(42)), ([3; 20], sp::word_u128(43))]);
        // another collection's lens, a transfer away from us, a revert: nothing
        assert_eq!(judge_claim(&holder, &MILADY, &plain, &r, &[2; 20], &t), Claim::Nothing);
        let away = ReceiptV { logs: vec![transfer_log(us, holder, 42)], ..r.clone() };
        assert_eq!(judge_claim(&holder, &MILADY, &plain, &away, &MILADY, &t), Claim::Nothing);
        assert_eq!(judge_claim(&holder, &MILADY, &plain, &ReceiptV { success: false, ..r.clone() }, &MILADY, &t), Claim::Nothing);
    }

    #[test]
    fn a_stray_goes_back_only_if_it_is_ours_unheld_unused_and_this_transfer_brought_it() {
        let us = [5; 20];
        let ok = ReturnFacts { owner: us, item_of: 0, busy: false, last_out: None };
        assert_eq!(judge_return(&[1; 20], 100, &ok, &us), Ok(()));
        assert_eq!(judge_return(&[1; 20], 100, &ReturnFacts { last_out: Some(99), ..ok.clone() }, &us), Ok(()));
        assert!(judge_return(&[0; 20], 100, &ok, &us).is_err(), "minted to us: nobody to return to");
        assert!(judge_return(&us, 100, &ok, &us).is_err());
        assert!(judge_return(&[1; 20], 100, &ReturnFacts { owner: [1; 20], ..ok.clone() }, &us).unwrap_err().contains("owner"));
        assert!(judge_return(&[1; 20], 100, &ReturnFacts { item_of: 3, ..ok.clone() }, &us).unwrap_err().contains("wrapper holds"));
        assert!(judge_return(&[1; 20], 100, &ReturnFacts { busy: true, ..ok.clone() }, &us).unwrap_err().contains("open buy"));
        assert!(judge_return(&[1; 20], 100, &ReturnFacts { last_out: Some(100), ..ok.clone() }, &us).unwrap_err().contains("left us"));
    }

    /// The calldata the canister signs for the real Milady #9180 listing, as the forge fork test replays it on an Ethereum fork
    /// (`test/fork/EthNftSeaport.fork.t.sol` reads `test/fixtures/ethnft/milady-9180-canister-fill.json`). The fixture is written
    /// with `WRITE_FIXTURES=1` and checked byte for byte otherwise.
    #[test]
    fn the_canisters_own_calldata_for_the_real_listing_is_the_forge_fixture() {
        let v: serde_json::Value = serde_json::from_str(FILL).unwrap();
        let Listing::Basic(o) = fill_listing() else { panic!("a basic order") };
        let us: [u8; 20] = [0xcb, 0x78, 0x7b, 0xae, 0x73, 0x2a, 0x52, 0x0d, 0x3c, 0x37, 0x36, 0xc6, 0x1f, 0xc9, 0x50, 0xb9, 0x67, 0xf8, 0xee, 0x6e];
        let t = sp::Target { collection: MILADY, token_id: sp::word_u128(9_180), cap_wei: E };
        let rules = sp::Rules { fee_recipients: vec![hex::decode("0000a26b00c1f0df003000390027140000faa719").unwrap().try_into().unwrap()],
            max_fee_bps: 250, min_order_life_s: 300 };
        let facts = sp::ChainFacts { ours: us, pin_timestamp: v["fork_block_timestamp"].as_u64().unwrap(), royalty_receiver: None };
        let c = sp::verify(&o, &t, &rules, &facts).unwrap();
        assert_eq!(c.total_wei, 999_899_999_000_000_000);
        assert_eq!(&c.calldata[..4], &sp::FULFILL_BASIC_ORDER, "our own selector, not the taker's alias");
        let fixture = serde_json::json!({
            "note": "written by ethnft::tests::the_canisters_own_calldata_for_the_real_listing_is_the_forge_fixture: the canister's re-encoding of the real Milady #9180 listing (fulfiller conduit key zeroed, nothing appended)",
            "chainId": 1, "forkBlock": v["fork_block"], "fillTx": v["tx"], "collection": format!("0x{}", hex::encode(MILADY)), "tokenId": 9180,
            "canister": format!("0x{}", hex::encode(us)), "valueWei": c.total_wei.to_string(), "calldata": format!("0x{}", hex::encode(&c.calldata)),
            "offerer": v["offerer"], "orderHash": v["order_hash"] });
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/ethnft/milady-9180-canister-fill.json");
        if std::env::var("WRITE_FIXTURES").is_ok() {
            std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
            std::fs::write(path, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
        }
        let on_disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("the forge fixture exists")).unwrap();
        assert_eq!(on_disk, fixture, "the forge fork test replays exactly what the canister signs");
    }

    #[test]
    fn the_config_is_validated() {
        let chain = crate::chains::tests::chain(1);
        let good = EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: false, floor: None, gas_pot_target_wei: None, logs_max_blocks: None, dormancy: None, collections: vec![EthCollection {
            collection: "0x5af0d9827e0c53e4799bb226655a1de152a425a5".into(), wrapper: "0x00000000000000000000000000000000000000aa".into(),
            module: "0x00000000000000000000000000000000000000bb".into(), max_purchase_wei: E, max_premium_bps: 500, chain_id: None, token_bound: None, nft_value_bps: None }] };
        assert_eq!(validate(&good, std::slice::from_ref(&chain), 4663), Ok(()));
        let bad = |f: &dyn Fn(&mut EthNftConfig)| {
            let mut c = good.clone();
            f(&mut c);
            validate(&c, std::slice::from_ref(&chain), 4663).unwrap_err()
        };
        assert!(bad(&|c| c.chain_id = 4663).contains("home"));
        assert!(bad(&|c| c.chain_id = 8453).contains("evm_chains"));
        assert!(bad(&|c| c.collections.clear()).contains("no collection"));
        assert!(bad(&|c| c.relay_max_fee_bps = 10).contains("relay_max_fee_bps"));
        assert!(bad(&|c| { let x = c.collections[0].clone(); c.collections.push(x); }).contains("twice"));
        assert!(bad(&|c| c.collections[0].max_purchase_wei = 26 * E).contains("max_purchase_wei"));
        assert!(bad(&|c| c.collections[0].max_premium_bps = 1_001).contains("premium"));
        assert!(bad(&|c| c.floor = Some(FloorParams { listing_rank: 4, ..FloorParams::default() })).contains("listing_rank"));
        assert!(bad(&|c| c.floor = Some(FloorParams { min_sales: 0, ..FloorParams::default() })).contains("min_sales"));
        assert!(bad(&|c| c.collections[0].wrapper = "nope".into()).contains("eth_nft"));
    }
}
