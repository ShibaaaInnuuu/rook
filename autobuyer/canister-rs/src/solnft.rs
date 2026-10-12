//! Solana NFTs held by the canister (docs/v9/solana-nfts-canister.md §2; the custodian model of docs/v9/solana-nfts-design.md, whose
//! on-chain checks become the canister's own): the Token Metadata and Metaplex Core account layouts, read by hand; **membership**
//! (`is_member`: a verified, non-compressed, one-of-one member of a listed collection that nobody else can freeze, take or burn)
//! and **custody** (`in_custody`: held by the canister's wallet with no delegate and no lock), decided on accounts read at
//! `finalized` through the two-provider agreement ([`read`]).
//!
//! Pure except [`read`] and [`check`]: no keys, no clock. Tested against real mainnet account bytes (`tests/solnft.rs`, fixtures in
//! `tests/fixtures/solnft/`, recorded by `autobuyer/solnft-svm/record_fixtures.py`).

use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::solrpc::{self, Account, Refused, SolRpc};
use crate::soltx::{self, key, Pubkey, Reader};

pub const TOKEN_METADATA: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
pub const MPL_CORE: &str = "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d";
pub const TOKEN_AUTH_RULES: &str = "auth9SigNpDKz4sJJ1DfCTuZrZNSAgh9sFD3rboVmgg";
pub const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const SYSVAR_INSTRUCTIONS: &str = "Sysvar1nstructions1111111111111111111111111";
/// The Metaplex Foundation rule set (its latest revision is `Pass` for every operation, read 2026-10-10).
pub const FOUNDATION_RULE_SET: &str = "eBJLFYPxJmMGKuFwpDWkzxZeUrad92kZRC5BJLpzyT9";
/// The Metaplex Compatibility rule set (SMB Gen3, Cyber Frogs).
pub const COMPATIBILITY_RULE_SET: &str = "AdH2Utn6Fus15ZhtenW4hZBQnvtLgM1YCW2MfVp7pYS5";

/// The largest account [`read`] accepts: Token Metadata accounts are ≤ 679 bytes; a Core asset with attributes and plugins a few
/// KiB. Rule sets (19 KB for the foundation's) are never read, only passed.
pub const MAX_NFT_ACCOUNT: usize = 10_000;

// Token Metadata `Key`
const TM_EDITION_V1: u8 = 1;
const TM_MASTER_EDITION_V1: u8 = 2;
const TM_METADATA_V1: u8 = 4;
const TM_MASTER_EDITION_V2: u8 = 6;
const TM_TOKEN_RECORD: u8 = 11;
// Token Metadata `TokenStandard`
pub const TS_NON_FUNGIBLE: u8 = 0;
pub const TS_PROGRAMMABLE_NON_FUNGIBLE: u8 = 4;
// mpl-core `Key`
const CORE_ASSET_V1: u8 = 1;
const CORE_HASHED_ASSET_V1: u8 = 2;
const CORE_PLUGIN_HEADER_V1: u8 = 3;
const CORE_PLUGIN_REGISTRY_V1: u8 = 4;
const CORE_COLLECTION_V1: u8 = 5;
// mpl-core `PluginType`
pub const PT_ROYALTIES: u8 = 0;
pub const PT_FREEZE_DELEGATE: u8 = 1;
pub const PT_BURN_DELEGATE: u8 = 2;
pub const PT_TRANSFER_DELEGATE: u8 = 3;
pub const PT_PERMANENT_FREEZE: u8 = 5;
pub const PT_PERMANENT_TRANSFER: u8 = 7;
pub const PT_PERMANENT_BURN: u8 = 8;
pub const PT_PERMANENT_FREEZE_EXECUTE: u8 = 17;
pub const PT_FREEZE_EXECUTE: u8 = 16;
// mpl-core `ExternalPluginAdapterType`: the ones that can veto a lifecycle event (a transfer)
pub const EXT_LIFECYCLE_HOOK: u8 = 0;
pub const EXT_ORACLE: u8 = 1;
pub const EXT_LINKED_LIFECYCLE_HOOK: u8 = 3;

/// The standard a listing is dominated by. A Token Metadata listing (`Legacy`, `Programmable`) accepts both kinds of member (some
/// collections migrated part of their items); a `Core` listing only Core assets.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standard {
    Legacy,
    Programmable,
    Core,
}

/// Why an asset is refused. Every refusal names what was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NftError {
    /// An account the decision needs does not exist (a compressed NFT has none at all).
    Missing(String),
    /// Not a one-of-one NFT of a supported program: fungible, print edition, Token-2022, compressed, malformed.
    NotAnNft(String),
    /// A real NFT, but not a verified member of this collection.
    NotInCollection(String),
    /// A member the canister must not hold: a permanent or foreign delegate, a lifecycle veto, a frozen asset.
    Unsupported(String),
    /// Not held by the wallet that must hold it, or held with a delegate or a lock.
    NotInCustody(String),
}

impl std::fmt::Display for NftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NftError::Missing(s) => write!(f, "missing account: {s}"),
            NftError::NotAnNft(s) => write!(f, "not an NFT: {s}"),
            NftError::NotInCollection(s) => write!(f, "not in the collection: {s}"),
            NftError::Unsupported(s) => write!(f, "unsupported NFT: {s}"),
            NftError::NotInCustody(s) => write!(f, "not in custody: {s}"),
        }
    }
}

impl From<NftError> for String {
    fn from(e: NftError) -> String {
        e.to_string()
    }
}

fn bad(e: String) -> NftError {
    NftError::NotAnNft(e)
}

/// The accounts a decision was made on: address → the account at `finalized`, `None` when it does not exist.
pub type Accounts = BTreeMap<Pubkey, Option<Account>>;

fn get<'a>(a: &'a Accounts, k: &Pubkey, what: &str) -> Result<&'a Account, NftError> {
    match a.get(k) {
        Some(Some(x)) => Ok(x),
        Some(None) => Err(NftError::Missing(format!("{what} {} does not exist", soltx::b58_encode(k)))),
        None => Err(NftError::Missing(format!("{what} {} was not read", soltx::b58_encode(k)))),
    }
}

// ---- addresses ----------------------------------------------------------------------------------------------------------------------

fn tm_pda(seeds: &[&[u8]]) -> Pubkey {
    let tm = key(TOKEN_METADATA);
    let mut s: Vec<&[u8]> = vec![b"metadata", &tm];
    s.extend_from_slice(seeds);
    soltx::find_program_address(&s, &tm).expect("a Token Metadata PDA").0
}

pub fn metadata_address(mint: &Pubkey) -> Pubkey {
    tm_pda(&[mint])
}

pub fn edition_address(mint: &Pubkey) -> Pubkey {
    tm_pda(&[mint, b"edition"])
}

/// A pNFT's token record for one token account.
pub fn token_record_address(mint: &Pubkey, token_account: &Pubkey) -> Pubkey {
    tm_pda(&[mint, b"token_record", token_account])
}

// ---- SPL Token ------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mint {
    pub mint_authority: Option<Pubkey>,
    pub supply: u64,
    pub decimals: u8,
    pub initialized: bool,
    pub freeze_authority: Option<Pubkey>,
}

fn coption(d: &[u8]) -> Result<Option<Pubkey>, String> {
    match u32::from_le_bytes(d[..4].try_into().expect("4")) {
        0 => Ok(None),
        1 => Ok(Some(d[4..36].try_into().expect("32"))),
        t => Err(format!("a COption tag of {t}")),
    }
}

/// A classic SPL Token mint (82 bytes).
pub fn decode_mint(d: &[u8]) -> Result<Mint, String> {
    if d.len() != 82 {
        return Err(format!("a mint of {} bytes, not 82", d.len()));
    }
    Ok(Mint {
        mint_authority: coption(&d[0..36])?,
        supply: u64::from_le_bytes(d[36..44].try_into().expect("8")),
        decimals: d[44],
        initialized: d[45] == 1,
        freeze_authority: coption(&d[46..82])?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenAccount {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub delegate: Option<Pubkey>,
    /// 0 uninitialized, 1 initialized, 2 frozen.
    pub state: u8,
    pub delegated_amount: u64,
    pub close_authority: Option<Pubkey>,
}

/// A classic SPL Token account (165 bytes).
pub fn decode_token_account(d: &[u8]) -> Result<TokenAccount, String> {
    if d.len() != 165 {
        return Err(format!("a token account of {} bytes, not 165", d.len()));
    }
    Ok(TokenAccount {
        mint: d[0..32].try_into().expect("32"),
        owner: d[32..64].try_into().expect("32"),
        amount: u64::from_le_bytes(d[64..72].try_into().expect("8")),
        delegate: coption(&d[72..108])?,
        state: d[108],
        delegated_amount: u64::from_le_bytes(d[121..129].try_into().expect("8")),
        close_authority: coption(&d[129..165])?,
    })
}

// ---- Token Metadata -------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub struct Creator {
    pub address: Pubkey,
    pub verified: bool,
    /// Percent (0–100), not basis points.
    pub share: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub update_authority: Pubkey,
    pub mint: Pubkey,
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub seller_fee_bps: u16,
    pub creators: Vec<Creator>,
    pub primary_sale_happened: bool,
    pub is_mutable: bool,
    pub edition_nonce: Option<u8>,
    pub token_standard: Option<u8>,
    /// `(verified, key)`.
    pub collection: Option<(bool, Pubkey)>,
    /// A collection's own metadata (`CollectionDetails`): set on the collection NFT, never on a member.
    pub is_collection: bool,
    /// `ProgrammableConfig::V1 { rule_set }` (`Some(None)`: programmable without a rule set).
    pub programmable: Option<Option<Pubkey>>,
}

/// `MetadataV1`. Fields added after an account was written are absent or zero-filled at its end: both read as `None`.
pub fn decode_metadata(d: &[u8]) -> Result<Metadata, String> {
    let mut r = Reader::new(d);
    if r.u8()? != TM_METADATA_V1 {
        return Err("not a MetadataV1 account".into());
    }
    let update_authority = r.key()?;
    let mint = r.key()?;
    let trim = |s: String| s.trim_end_matches('\0').to_string();
    let name = trim(r.string(200)?);
    let symbol = trim(r.string(200)?);
    let uri = trim(r.string(400)?);
    let seller_fee_bps = r.u16()?;
    let mut creators = Vec::new();
    if r.bool()? {
        for _ in 0..r.len(5)? {
            creators.push(Creator { address: r.key()?, verified: r.bool()?, share: r.u8()? });
        }
    }
    let primary_sale_happened = r.bool()?;
    let is_mutable = r.bool()?;
    let opt = |r: &mut Reader| -> Result<bool, String> { if r.at >= r.data.len() { Ok(false) } else { r.bool() } };
    let edition_nonce = if opt(&mut r)? { Some(r.u8()?) } else { None };
    let token_standard = if opt(&mut r)? { Some(r.u8()?) } else { None };
    let collection = if opt(&mut r)? { Some((r.bool()?, r.key()?)) } else { None };
    if opt(&mut r)? {
        r.take(17)?; // uses: method, remaining, total
    }
    let is_collection = if opt(&mut r)? {
        r.u8()?; // V1 { size } / V2 { padding }
        r.take(8)?;
        true
    } else {
        false
    };
    let programmable = if opt(&mut r)? {
        if r.u8()? != 0 {
            return Err("an unknown ProgrammableConfig".into());
        }
        Some(if opt(&mut r)? { Some(r.key()?) } else { None })
    } else {
        None
    };
    Ok(Metadata { update_authority, mint, name, symbol, uri, seller_fee_bps, creators, primary_sale_happened, is_mutable, edition_nonce,
        token_standard, collection, is_collection, programmable })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edition {
    Master { supply: u64, max_supply: Option<u64> },
    /// A print: one of many, refused.
    Print { parent: Pubkey, edition: u64 },
}

pub fn decode_edition(d: &[u8]) -> Result<Edition, String> {
    let mut r = Reader::new(d);
    match r.u8()? {
        TM_MASTER_EDITION_V1 | TM_MASTER_EDITION_V2 => {
            let supply = r.u64()?;
            let max_supply = if r.bool()? { Some(r.u64()?) } else { None };
            Ok(Edition::Master { supply, max_supply })
        }
        TM_EDITION_V1 => Ok(Edition::Print { parent: r.key()?, edition: r.u64()? }),
        k => Err(format!("not an edition account (key {k})")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenState {
    Unlocked,
    Locked,
    Listed,
}

/// A pNFT's per-token-account record: its lock state and its one delegate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRecord {
    pub state: TokenState,
    pub rule_set_revision: Option<u64>,
    pub delegate: Option<Pubkey>,
    pub delegate_role: Option<u8>,
    pub locked_transfer: Option<Pubkey>,
}

pub fn decode_token_record(d: &[u8]) -> Result<TokenRecord, String> {
    let mut r = Reader::new(d);
    if r.u8()? != TM_TOKEN_RECORD {
        return Err("not a TokenRecord account".into());
    }
    r.u8()?; // bump
    let state = match r.u8()? {
        0 => TokenState::Unlocked,
        1 => TokenState::Locked,
        2 => TokenState::Listed,
        s => return Err(format!("a token state of {s}")),
    };
    let rule_set_revision = if r.bool()? { Some(r.u64()?) } else { None };
    let delegate = if r.bool()? { Some(r.key()?) } else { None };
    let delegate_role = if r.bool()? { Some(r.u8()?) } else { None };
    let locked_transfer = if r.at < d.len() && r.bool()? { Some(r.key()?) } else { None };
    Ok(TokenRecord { state, rule_set_revision, delegate, delegate_role, locked_transfer })
}

// ---- Metaplex Core --------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authority {
    None,
    Owner,
    UpdateAuthority,
    Address(Pubkey),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    pub kind: u8,
    pub authority: Authority,
    /// The plugin's data offset in the account.
    pub offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalPlugin {
    pub kind: u8,
    pub authority: Authority,
    pub lifecycle_checks: usize,
}

/// `Royalties { basis_points, creators: [(address, percentage)], rule_set }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Royalties {
    pub basis_points: u16,
    pub creators: Vec<(Pubkey, u8)>,
    /// 0 none, 1 program allow-list, 2 program deny-list.
    pub rule: u8,
    pub programs: Vec<Pubkey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreUpdateAuthority {
    None,
    Address(Pubkey),
    Collection(Pubkey),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreAsset {
    pub owner: Pubkey,
    pub update_authority: CoreUpdateAuthority,
    pub name: String,
    pub plugins: Vec<Plugin>,
    pub externals: Vec<ExternalPlugin>,
    pub royalties: Option<Royalties>,
    /// `FreezeDelegate { frozen }` is set and true.
    pub frozen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreCollection {
    pub update_authority: Pubkey,
    pub name: String,
    pub num_minted: u32,
    pub current_size: u32,
    pub plugins: Vec<Plugin>,
    pub externals: Vec<ExternalPlugin>,
    pub royalties: Option<Royalties>,
}

fn authority(r: &mut Reader) -> Result<Authority, String> {
    Ok(match r.u8()? {
        0 => Authority::None,
        1 => Authority::Owner,
        2 => Authority::UpdateAuthority,
        3 => Authority::Address(r.key()?),
        a => return Err(format!("a plugin authority of {a}")),
    })
}

/// The plugin header at `at` (absent when the account ends at the base): `(plugins, externals)`.
fn plugins(d: &[u8], at: usize) -> Result<(Vec<Plugin>, Vec<ExternalPlugin>), String> {
    if at >= d.len() {
        return Ok((vec![], vec![]));
    }
    let mut r = Reader { data: d, at };
    if r.u8()? != CORE_PLUGIN_HEADER_V1 {
        return Err("no PluginHeaderV1 after the base".into());
    }
    let reg = usize::try_from(r.u64()?).map_err(|_| "a registry offset over usize")?;
    let mut r = Reader { data: d, at: reg };
    if r.u8()? != CORE_PLUGIN_REGISTRY_V1 {
        return Err("no PluginRegistryV1 at the header's offset".into());
    }
    let mut ps = Vec::new();
    for _ in 0..r.len(64)? {
        let kind = r.u8()?;
        let authority = authority(&mut r)?;
        let offset = r.u64()?;
        ps.push(Plugin { kind, authority, offset });
    }
    let mut xs = Vec::new();
    for _ in 0..r.len(64)? {
        let kind = r.u8()?;
        let authority = authority(&mut r)?;
        let lifecycle_checks = if r.bool()? {
            let n = r.len(64)?;
            r.take(n * 5)?; // (HookableLifecycleEvent u8, ExternalCheckResult u32)
            n
        } else {
            0
        };
        r.u64()?; // offset
        if r.bool()? {
            r.u64()?; // data offset
        }
        if r.bool()? {
            r.u64()?; // data len
        }
        xs.push(ExternalPlugin { kind, authority, lifecycle_checks });
    }
    Ok((ps, xs))
}

fn royalties_of(d: &[u8], ps: &[Plugin]) -> Result<Option<Royalties>, String> {
    let Some(p) = ps.iter().find(|p| p.kind == PT_ROYALTIES) else { return Ok(None) };
    let mut r = Reader { data: d, at: usize::try_from(p.offset).map_err(|_| "offset")? };
    if r.u8()? != PT_ROYALTIES {
        return Err("the Royalties offset holds another plugin".into());
    }
    let basis_points = r.u16()?;
    let mut creators = Vec::new();
    for _ in 0..r.len(16)? {
        creators.push((r.key()?, r.u8()?));
    }
    let rule = r.u8()?;
    let mut programs = Vec::new();
    if rule == 1 || rule == 2 {
        for _ in 0..r.len(64)? {
            programs.push(r.key()?);
        }
    }
    Ok(Some(Royalties { basis_points, creators, rule, programs }))
}

fn frozen_of(d: &[u8], ps: &[Plugin]) -> Result<bool, String> {
    match ps.iter().find(|p| p.kind == PT_FREEZE_DELEGATE) {
        None => Ok(false),
        Some(p) => {
            let at = usize::try_from(p.offset).map_err(|_| "offset")?;
            match d.get(at..at + 2) {
                Some([PT_FREEZE_DELEGATE, f]) => Ok(*f == 1),
                _ => Err("the FreezeDelegate offset holds another plugin".into()),
            }
        }
    }
}

/// `AssetV1` (a `HashedAssetV1`, compressed, is refused by the caller).
pub fn decode_core_asset(d: &[u8]) -> Result<CoreAsset, String> {
    let mut r = Reader::new(d);
    match r.u8()? {
        CORE_ASSET_V1 => {}
        CORE_HASHED_ASSET_V1 => return Err("a HashedAssetV1 (compressed: no account state to read)".into()),
        k => return Err(format!("not an AssetV1 (key {k})")),
    }
    let owner = r.key()?;
    let update_authority = match r.u8()? {
        0 => CoreUpdateAuthority::None,
        1 => CoreUpdateAuthority::Address(r.key()?),
        2 => CoreUpdateAuthority::Collection(r.key()?),
        u => return Err(format!("an update authority of {u}")),
    };
    let name = r.string(1_000)?;
    r.string(1_000)?; // uri
    if r.bool()? {
        r.u64()?; // seq
    }
    let (plugins, externals) = plugins(d, r.at)?;
    let royalties = royalties_of(d, &plugins)?;
    let frozen = frozen_of(d, &plugins)?;
    Ok(CoreAsset { owner, update_authority, name, plugins, externals, royalties, frozen })
}

pub fn decode_core_collection(d: &[u8]) -> Result<CoreCollection, String> {
    let mut r = Reader::new(d);
    if r.u8()? != CORE_COLLECTION_V1 {
        return Err("not a CollectionV1".into());
    }
    let update_authority = r.key()?;
    let name = r.string(1_000)?;
    r.string(1_000)?;
    let num_minted = r.u32()?;
    let current_size = r.u32()?;
    let (plugins, externals) = plugins(d, r.at)?;
    let royalties = royalties_of(d, &plugins)?;
    Ok(CoreCollection { update_authority, name, num_minted, current_size, plugins, externals, royalties })
}

/// Nothing on the asset or its collection can freeze, take or burn an item in custody, or veto its delivery
/// (solana-nfts-design.md §2): no permanent delegate on either, no owner-managed delegate on the asset whose authority is not its
/// owner, and no external plugin that can veto a lifecycle event.
fn core_plugins_safe(owner: &Pubkey, asset: &CoreAsset, coll: &CoreCollection) -> Result<(), NftError> {
    let permanent = |k: u8| matches!(k, PT_PERMANENT_FREEZE | PT_PERMANENT_TRANSFER | PT_PERMANENT_BURN | PT_PERMANENT_FREEZE_EXECUTE);
    for (who, ps) in [("asset", &asset.plugins), ("collection", &coll.plugins)] {
        if let Some(p) = ps.iter().find(|p| permanent(p.kind)) {
            return Err(NftError::Unsupported(format!("the {who} carries a permanent delegate plugin (type {})", p.kind)));
        }
    }
    for p in &asset.plugins {
        let delegated = matches!(p.kind, PT_FREEZE_DELEGATE | PT_BURN_DELEGATE | PT_TRANSFER_DELEGATE | PT_FREEZE_EXECUTE);
        let ours = match &p.authority {
            Authority::Owner => true,
            Authority::Address(a) => a == owner,
            _ => false,
        };
        if delegated && !ours {
            return Err(NftError::Unsupported(format!("the asset's delegate plugin (type {}) is held by {:?}, not its owner", p.kind,
                p.authority)));
        }
    }
    for (who, xs) in [("asset", &asset.externals), ("collection", &coll.externals)] {
        if let Some(x) = xs.iter().find(|x| x.lifecycle_checks > 0 || matches!(x.kind, EXT_LIFECYCLE_HOOK | EXT_ORACLE | EXT_LINKED_LIFECYCLE_HOOK)) {
            return Err(NftError::Unsupported(format!("the {who} carries an external plugin with lifecycle checks (type {})", x.kind)));
        }
    }
    if let Some(r) = asset.royalties.as_ref().or(coll.royalties.as_ref()) {
        if r.rule == 1 && !r.programs.is_empty() {
            // an allow-list: a wallet-to-wallet transfer (the System Program as the "program") must be on it
            if !r.programs.contains(&soltx::SYSTEM_PROGRAM) {
                return Err(NftError::Unsupported("the royalties rule is a program allow-list without the System Program: a wallet \
                    transfer could be refused".into()));
            }
        }
    }
    Ok(())
}

// ---- membership -------------------------------------------------------------------------------------------------------------------------

/// A verified member, and what a buy or a delivery needs to know about it.
#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub struct Member {
    pub asset: Pubkey,
    /// What the item itself is (a Token Metadata listing may hold legacy and programmable members).
    pub standard: Standard,
    pub collection: Pubkey,
    /// Royalties: Token Metadata `seller_fee_basis_points`, or Core's `Royalties` (asset first, then collection).
    pub royalty_bps: u16,
    /// `(address, percent)`, in the order the programs pay them (they are passed as remaining accounts in this order).
    pub creators: Vec<(Pubkey, u8)>,
    /// A pNFT's rule set (Token Metadata), `None` for legacy and Core.
    pub rule_set: Option<Pubkey>,
    /// Core: the owner now; Token Metadata: unknown here ([`in_custody`] reads the token account).
    pub core_owner: Option<Pubkey>,
}

/// The accounts [`is_member`] reads for `asset` (and `collection` for Core), in one `getMultipleAccounts`.
pub fn member_keys(asset: &Pubkey, collection: &Pubkey, listing: Standard) -> Vec<Pubkey> {
    match listing {
        Standard::Core => vec![*asset, *collection],
        _ => vec![*asset, metadata_address(asset), edition_address(asset)],
    }
}

/// `asset` is a member of `collection` under a listing of standard `listing`: a verified, non-compressed, one-of-one item that
/// nobody but its owner can move (solana-nfts-design.md §2, now checked by the canister on accounts read at `finalized`):
/// - Token Metadata: the mint is classic SPL Token, initialized, supply 1, decimals 0; the metadata is `MetadataV1` of this mint,
///   standard `None` / `NonFungible` / `ProgrammableNonFungible`, `collection == Some { verified: true, key: collection }`, and not
///   itself a collection; its edition is the mint's PDA (by the metadata's nonce), a master edition (prints and fungibles refused);
/// - Core: an `AssetV1` (not `HashedAssetV1`), `update_authority == Collection(collection)`, the collection a `CollectionV1`; no
///   permanent delegate on either, no foreign owner-managed delegate on the asset, no lifecycle-veto external plugin.
pub fn is_member(asset: &Pubkey, collection: &Pubkey, listing: Standard, a: &Accounts) -> Result<Member, NftError> {
    if listing == Standard::Core {
        return core_member(asset, collection, a);
    }
    let tm = key(TOKEN_METADATA);
    let mint_acct = get(a, asset, "mint")?;
    if mint_acct.owner == key(TOKEN_2022) {
        return Err(bad("a Token-2022 mint (not supported)".into()));
    }
    if mint_acct.owner == key(MPL_CORE) {
        return Err(bad("a Core asset under a Token Metadata listing".into()));
    }
    if mint_acct.owner != key(soltx::TOKEN_PROGRAM) {
        return Err(bad(format!("owned by {}, not SPL Token", soltx::b58_encode(&mint_acct.owner))));
    }
    let mint = decode_mint(&mint_acct.data).map_err(bad)?;
    if !mint.initialized || mint.supply != 1 || mint.decimals != 0 {
        return Err(bad(format!("supply {} decimals {}: not a one-of-one", mint.supply, mint.decimals)));
    }
    let md_acct = get(a, &metadata_address(asset), "metadata")?;
    if md_acct.owner != tm {
        return Err(bad("the metadata is not owned by Token Metadata".into()));
    }
    let md = decode_metadata(&md_acct.data).map_err(bad)?;
    if md.mint != *asset {
        return Err(bad("the metadata names another mint".into()));
    }
    let standard = match md.token_standard {
        None | Some(TS_NON_FUNGIBLE) => Standard::Legacy,
        Some(TS_PROGRAMMABLE_NON_FUNGIBLE) => Standard::Programmable,
        Some(t) => return Err(bad(format!("token standard {t} (fungible or a print edition)"))),
    };
    if md.is_collection {
        return Err(NftError::NotInCollection("a collection NFT itself, not a member".into()));
    }
    match md.collection {
        Some((true, k)) if k == *collection => {}
        Some((false, k)) if k == *collection => return Err(NftError::NotInCollection("the collection is not verified".into())),
        Some((_, k)) => return Err(NftError::NotInCollection(format!("collection {}", soltx::b58_encode(&k)))),
        None => return Err(NftError::NotInCollection("no collection".into())),
    }
    let ed_key = edition_address(asset);
    if let Some(n) = md.edition_nonce {
        let want = soltx::create_program_address(&[b"metadata", &tm, asset, b"edition", &[n]], &tm);
        if want != Some(ed_key) {
            return Err(bad("the metadata's edition nonce does not derive the edition".into()));
        }
    }
    let ed_acct = get(a, &ed_key, "edition")?;
    if ed_acct.owner != tm {
        return Err(bad("the edition is not owned by Token Metadata".into()));
    }
    match decode_edition(&ed_acct.data).map_err(bad)? {
        Edition::Master { .. } => {}
        Edition::Print { .. } => return Err(bad("a print edition".into())),
    }
    let rule_set = if standard == Standard::Programmable { md.programmable.flatten() } else { None };
    Ok(Member {
        asset: *asset,
        standard,
        collection: *collection,
        royalty_bps: md.seller_fee_bps,
        creators: md.creators.iter().map(|c| (c.address, c.share)).collect(),
        rule_set,
        core_owner: None,
    })
}

fn core_member(asset: &Pubkey, collection: &Pubkey, a: &Accounts) -> Result<Member, NftError> {
    let core = key(MPL_CORE);
    let acct = get(a, asset, "asset")?;
    if acct.owner != core {
        return Err(bad(format!("owned by {}, not mpl-core", soltx::b58_encode(&acct.owner))));
    }
    let x = decode_core_asset(&acct.data).map_err(bad)?;
    match &x.update_authority {
        CoreUpdateAuthority::Collection(c) if c == collection => {}
        u => return Err(NftError::NotInCollection(format!("update authority {u:?}"))),
    }
    let cacct = get(a, collection, "collection")?;
    if cacct.owner != core {
        return Err(NftError::NotInCollection("the collection is not owned by mpl-core".into()));
    }
    let c = decode_core_collection(&cacct.data).map_err(|e| NftError::NotInCollection(e))?;
    core_plugins_safe(&x.owner, &x, &c)?;
    let r = x.royalties.as_ref().or(c.royalties.as_ref());
    Ok(Member {
        asset: *asset,
        standard: Standard::Core,
        collection: *collection,
        royalty_bps: r.map(|r| r.basis_points).unwrap_or(0),
        creators: r.map(|r| r.creators.clone()).unwrap_or_default(),
        rule_set: None,
        core_owner: Some(x.owner),
    })
}

// ---- custody -----------------------------------------------------------------------------------------------------------------------

/// What [`in_custody`] saw: the token account (Token Metadata) or the asset (Core) that holds the item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Custody {
    pub asset: Pubkey,
    pub holder: Pubkey,
    /// The token account (Token Metadata; normally the holder's ATA) or the asset itself (Core).
    pub account: Pubkey,
}

/// The accounts [`in_custody`] reads beyond [`member_keys`]: the holder's ATA and, for a pNFT, its token record.
pub fn custody_keys(asset: &Pubkey, holder: &Pubkey, listing: Standard) -> Vec<Pubkey> {
    match listing {
        Standard::Core => vec![],
        _ => {
            let ta = soltx::associated_token_address(holder, asset);
            vec![ta, token_record_address(asset, &ta)]
        }
    }
}

/// `holder` holds the member `m` and nobody else can move it: Token Metadata — its ATA is SPL Token, this mint, owned by
/// `holder`, amount 1, no delegate, no close authority; a legacy token account is not frozen; a pNFT's token record is `Unlocked`
/// with no delegate and no locked transfer (its token account is always frozen by Token Metadata, which is fine). Core — the
/// asset's owner is `holder` and it is not frozen (the plugins were checked by [`is_member`]).
pub fn in_custody(m: &Member, holder: &Pubkey, a: &Accounts) -> Result<Custody, NftError> {
    let out = |s: String| NftError::NotInCustody(s);
    if m.standard == Standard::Core {
        let x = decode_core_asset(&get(a, &m.asset, "asset")?.data).map_err(bad)?;
        if x.owner != *holder {
            return Err(out(format!("the asset is owned by {}", soltx::b58_encode(&x.owner))));
        }
        if x.frozen {
            return Err(NftError::Unsupported("the asset is frozen".into()));
        }
        return Ok(Custody { asset: m.asset, holder: *holder, account: m.asset });
    }
    let ta_key = soltx::associated_token_address(holder, &m.asset);
    let ta_acct = get(a, &ta_key, "token account").map_err(|e| out(e.to_string()))?;
    if ta_acct.owner != key(soltx::TOKEN_PROGRAM) {
        return Err(out("the token account is not SPL Token's".into()));
    }
    let ta = decode_token_account(&ta_acct.data).map_err(out)?;
    if ta.mint != m.asset || ta.owner != *holder || ta.amount != 1 {
        return Err(out(format!("the token account holds {} of {} for {}", ta.amount, soltx::b58_encode(&ta.mint),
            soltx::b58_encode(&ta.owner))));
    }
    if ta.delegate.is_some() || ta.close_authority.is_some() {
        return Err(out("the token account has a delegate or a close authority".into()));
    }
    match m.standard {
        Standard::Legacy if ta.state != 1 => return Err(out(format!("the token account's state is {} (frozen?)", ta.state))),
        Standard::Programmable => {
            let rec = decode_token_record(&get(a, &token_record_address(&m.asset, &ta_key), "token record")?.data).map_err(bad)?;
            if rec.state != TokenState::Unlocked || rec.delegate.is_some() || rec.locked_transfer.is_some() {
                return Err(out(format!("the token record is {:?} with delegate {:?}", rec.state, rec.delegate.map(|d| soltx::b58_encode(&d)))));
            }
        }
        _ => {}
    }
    Ok(Custody { asset: m.asset, holder: *holder, account: ta_key })
}

// ---- the reads ------------------------------------------------------------------------------------------------------------------------

/// The accounts `keys` at one `finalized` slot, as every provider agrees (at least two; one is not a check).
pub async fn read<P: SolRpc>(ps: &mut [P], keys: &[Pubkey]) -> Result<Accounts, Refused> {
    let mut keys = keys.to_vec();
    keys.sort_unstable();
    keys.dedup();
    let got = solrpc::accounts(ps, &keys, MAX_NFT_ACCOUNT).await?;
    Ok(keys.into_iter().zip(got).collect())
}

/// The accounts [`check`] reads: the member's and the holder's.
pub fn check_keys(asset: &Pubkey, collection: &Pubkey, listing: Standard, holder: &Pubkey) -> Vec<Pubkey> {
    let mut keys = member_keys(asset, collection, listing);
    keys.extend(custody_keys(asset, holder, listing));
    keys
}

/// Both decisions on accounts already read ([`check_keys`]): `asset` is a member of `collection` and `holder` holds it. Pure.
pub fn check_on(a: &Accounts, asset: &Pubkey, collection: &Pubkey, listing: Standard, holder: &Pubkey) -> Result<(Member, Custody), NftError> {
    let m = is_member(asset, collection, listing, a)?;
    let c = in_custody(&m, holder, a)?;
    Ok((m, c))
}

/// One read, both decisions: `asset` is a member of `collection` and `holder` holds it, at `finalized`, agreed by every provider.
pub async fn check<P: SolRpc>(ps: &mut [P], asset: &Pubkey, collection: &Pubkey, listing: Standard, holder: &Pubkey)
    -> Result<(Member, Custody), String> {
    let a = read(ps, &check_keys(asset, collection, listing, holder)).await?;
    Ok(check_on(&a, asset, collection, listing, holder)?)
}

