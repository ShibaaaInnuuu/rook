//! Buying a Solana NFT into the canister's wallet (docs/v9/solana-nfts-canister.md §3): the marketplaces' listing accounts read at
//! `finalized`, the price each one really charges (fees and royalties included), the best ask under the canister's limit, the
//! purchase instruction built by hand, and the **judge** that every purchase transaction must pass before it is signed — whoever
//! built it (the canister, or a marketplace API whose transaction comes back co-signed).
//!
//! Venues (measured 2026-10-10, §3.1 of the doc):
//! - **Tensor Marketplace** (TComp, `TCMPhJdw…`): `buy_legacy` (legacy and pNFT) and `buy_core`. A listing escrows the item at its
//!   `ListState` PDA; it needs a co-signature only when `ListState.cosigner` is set (none of the listings read was). **Keyless.**
//! - **Magic Eden mmm** pools (`mmm3XBJg…`): `sol_fulfill_sell` (legacy) and `sol_mip1_fulfill_sell` (pNFT). Every fulfill requires
//!   the pool's `cosigner` to SIGN, and every Magic Eden pool's cosigner is Magic Eden's notary `NTYeYJ1w…`: **not keyless** (the
//!   co-signed transaction comes from Magic Eden's instruction API, which answers 401 without an API key). Built and judged here so
//!   that such a transaction can be judged the same way; tested with a stand-in cosigner.
//! - **Magic Eden M2** (`M2mx93ek…`, auction house `E8cU1WiR…`, `requires_notary = 1`): most of the cheapest asks. Read for the
//!   floor ([`decode_m2_listing`]), never bought keyless.
//!
//! Pure: no network, no keys, no clock (the caller passes `now`). Tested on real mainnet listings (`tests/solnft.rs`) and executed
//! against the marketplace programs dumped from mainnet in LiteSVM (`autobuyer/solnft-svm`).

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::solnft::{self as nft, Accounts, Member, Standard};
use crate::soltx::{self, key, meta, Borsh, Instruction, Pubkey, Reader};

pub const TENSOR_MARKETPLACE: &str = "TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp";
/// Tensor's fee program: the protocol fee goes to its `["fee_vault", shard]` PDA (shard = the state account's last byte).
pub const TENSOR_FEE: &str = "TFEEgwDP6nn1s8mMX2tTNPPz8j2VomkphLUmyxKm17A";
/// The marketplace's singleton, accepted by `assert_fee_account` in place of the shard vault.
pub const TCOMP_SINGLETON: &str = "q4s8z5dRAt2fKC2tLthBPatakZRXPMx1LfacckSXd4f";
pub const MMM: &str = "mmm3XBJg5gk8XJxEKBvdgptZz6SgK4tXvn36sodowMc";
pub const M2: &str = "M2mx93ekt1fmXSVkTrUL9xVFHkmME8HTUi5Cyc5aF7K";
pub const M2_AUCTION_HOUSE: &str = "E8cU1WiRWjanGxmn96ewBgk9vPTcL6AEZ1t6F6fkgUWe";
/// Magic Eden's notary: M2's auction house and every mmm pool name it as their co-signer.
pub const ME_NOTARY: &str = "NTYeYJ1wr4bpM5xo6zx5En44SvJFAd35zTxxNoERYqd";
pub const SYSVAR_RENT: &str = "SysvarRent111111111111111111111111111111111";

/// tensor-toolbox 0.8.1 `TAKER_FEE_BPS`: the taker pays 2 % on top of the listing (protocol + brokers).
pub const TENSOR_TAKER_FEE_BPS: u64 = 200;
/// mmm `MAX_REFERRAL_FEE_BP`: maker + taker fee, at most 5 %.
pub const MMM_MAX_FEE_BP: i16 = 500;
/// Rent the buyer pays besides the price: its ATA (165 bytes) and, for a pNFT, its token record (80 bytes).
pub const ATA_RENT: u64 = 2_039_280;
pub const TOKEN_RECORD_RENT: u64 = 1_447_680;

const LIST_STATE_BYTES: usize = 317;
const MMM_POOL_BYTES: usize = 849;
const SELL_STATE_BYTES: usize = 344;

// ---- the listing accounts -------------------------------------------------------------------------------------------------------------

/// Tensor Marketplace `ListState`.
#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub struct ListState {
    pub version: u8,
    pub owner: Pubkey,
    pub asset: Pubkey,
    pub amount: u64,
    /// `None`: SOL.
    pub currency: Option<Pubkey>,
    pub expiry: i64,
    pub private_taker: Option<Pubkey>,
    pub maker_broker: Option<Pubkey>,
    /// The owner when default.
    pub rent_payer: Pubkey,
    /// `None`: no co-signature needed.
    pub cosigner: Option<Pubkey>,
}

pub fn decode_list_state(d: &[u8]) -> Result<ListState, String> {
    if d.len() != LIST_STATE_BYTES {
        return Err(format!("a ListState of {} bytes, not {LIST_STATE_BYTES}", d.len()));
    }
    let mut r = Reader::account(d, "ListState")?;
    let version = r.u8()?;
    r.u8()?; // bump
    let owner = r.key()?;
    let asset = r.key()?;
    let amount = r.u64()?;
    let currency = if r.bool()? { Some(r.key()?) } else { None };
    let expiry = r.i64()?;
    let private_taker = if r.bool()? { Some(r.key()?) } else { None };
    let maker_broker = if r.bool()? { Some(r.key()?) } else { None };
    let rent_payer = r.key()?;
    let cosigner = r.key()?;
    Ok(ListState { version, owner, asset, amount, currency, expiry, private_taker, maker_broker,
        rent_payer: if rent_payer == [0; 32] { owner } else { rent_payer }, cosigner: (cosigner != [0; 32]).then_some(cosigner) })
}

pub fn list_state_address(asset: &Pubkey) -> Pubkey {
    soltx::find_program_address(&[b"list_state", asset], &key(TENSOR_MARKETPLACE)).expect("a PDA").0
}

/// The Tensor fee vault for a state account (`["fee_vault", [state[31]]]` of the fee program).
pub fn tensor_fee_vault(state: &Pubkey) -> Pubkey {
    soltx::find_program_address(&[b"fee_vault", &[state[31]]], &key(TENSOR_FEE)).expect("a PDA").0
}

/// Magic Eden mmm `Pool`.
#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub struct MmmPool {
    pub spot_price: u64,
    /// 0 linear, 1 exponential.
    pub curve_type: u8,
    pub curve_delta: u64,
    pub reinvest_fulfill_sell: bool,
    pub expiry: i64,
    pub lp_fee_bp: u16,
    pub referral: Pubkey,
    pub buyside_creator_royalty_bp: u16,
    pub sellside_asset_amount: u64,
    pub owner: Pubkey,
    pub cosigner: Pubkey,
    pub uuid: Pubkey,
    pub payment_mint: Pubkey,
    /// `(kind, value)`: 1 first verified creator, 2 mint, 3 verified collection, 4 metadata, 6 Core collection, 255 any.
    pub allowlists: Vec<(u8, Pubkey)>,
    pub shared_escrow: Pubkey,
}

pub fn decode_mmm_pool(d: &[u8]) -> Result<MmmPool, String> {
    if d.len() != MMM_POOL_BYTES {
        return Err(format!("a Pool of {} bytes, not {MMM_POOL_BYTES}", d.len()));
    }
    let mut r = Reader::account(d, "Pool")?;
    let spot_price = r.u64()?;
    let curve_type = r.u8()?;
    let curve_delta = r.u64()?;
    r.bool()?; // reinvest_fulfill_buy
    let reinvest_fulfill_sell = r.bool()?;
    let expiry = r.i64()?;
    let lp_fee_bp = r.u16()?;
    let referral = r.key()?;
    r.u16()?; // referral_bp (deprecated)
    let buyside_creator_royalty_bp = r.u16()?;
    r.take(32)?; // cosigner_annotation
    let sellside_asset_amount = r.u64()?;
    r.u64()?; // lp_fee_earned
    let owner = r.key()?;
    let cosigner = r.key()?;
    let uuid = r.key()?;
    let payment_mint = r.key()?;
    let mut allowlists = Vec::new();
    for _ in 0..6 {
        let (k, v) = (r.u8()?, r.key()?);
        if k != 0 {
            allowlists.push((k, v));
        }
    }
    r.u64()?; // buyside_payment_amount
    let shared_escrow = r.key()?;
    Ok(MmmPool { spot_price, curve_type, curve_delta, reinvest_fulfill_sell, expiry, lp_fee_bp, referral, buyside_creator_royalty_bp,
        sellside_asset_amount, owner, cosigner, uuid, payment_mint, allowlists, shared_escrow })
}

/// Magic Eden mmm `SellState`: one asset deposited in a pool.
#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub struct SellState {
    pub pool: Pubkey,
    pub pool_owner: Pubkey,
    pub asset: Pubkey,
    pub amount: u64,
}

pub fn decode_sell_state(d: &[u8]) -> Result<SellState, String> {
    if d.len() != SELL_STATE_BYTES {
        return Err(format!("a SellState of {} bytes, not {SELL_STATE_BYTES}", d.len()));
    }
    let mut r = Reader::account(d, "SellState")?;
    Ok(SellState { pool: r.key()?, pool_owner: r.key()?, asset: r.key()?, amount: r.u64()? })
}

pub fn mmm_pool_address(owner: &Pubkey, uuid: &Pubkey) -> Pubkey {
    soltx::find_program_address(&[b"mmm_pool", owner, uuid], &key(MMM)).expect("a PDA").0
}

pub fn mmm_sell_state_address(pool: &Pubkey, asset: &Pubkey) -> Pubkey {
    soltx::find_program_address(&[b"mmm_sell_state", pool, asset], &key(MMM)).expect("a PDA").0
}

pub fn mmm_escrow_address(pool: &Pubkey) -> Pubkey {
    soltx::find_program_address(&[b"mmm_buyside_sol_escrow_account", pool], &key(MMM)).expect("a PDA").0
}

/// What one asset costs from a pool's sell side (`get_sol_total_price_and_next_price(pool, 1, false)`): one step above spot.
pub fn mmm_sell_price(p: &MmmPool) -> Result<u64, String> {
    let price = match p.curve_type {
        0 => p.spot_price.checked_add(p.curve_delta),
        1 => (p.spot_price as u128).checked_mul(10_000 + p.curve_delta as u128).map(|x| (x / 10_000) as u64),
        c => return Err(format!("curve type {c}")),
    };
    match price {
        Some(x) if x > 0 => Ok(x),
        _ => Err("a pool price that overflows or is zero".into()),
    }
}

/// Magic Eden M2 `SellerTradeStateV2`: a listing in the seller's own wallet, sold only with the notary's signature.
#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub struct M2Listing {
    pub auction_house: Pubkey,
    pub seller: Pubkey,
    pub seller_referral: Pubkey,
    pub price: u64,
    pub mint: Pubkey,
    pub token_account: Pubkey,
    pub token_size: u64,
    pub expiry: i64,
    pub payment_mint: Pubkey,
}

pub fn decode_m2_listing(d: &[u8]) -> Result<M2Listing, String> {
    let mut r = Reader::account(d, "SellerTradeStateV2")?;
    let auction_house = r.key()?;
    let seller = r.key()?;
    let seller_referral = r.key()?;
    let price = r.u64()?;
    let mint = r.key()?;
    let token_account = r.key()?;
    let token_size = r.u64()?;
    r.u8()?; // bump
    let expiry = r.i64()?;
    let payment_mint = r.key()?;
    Ok(M2Listing { auction_house, seller, seller_referral, price, mint, token_account, token_size, expiry, payment_mint })
}

/// M2's escrow authority (`["m2", "signer"]`): listed items sit in its token accounts.
pub fn m2_program_as_signer() -> Pubkey {
    soltx::find_program_address(&[b"m2", b"signer"], &key(M2)).expect("a PDA").0
}

/// M2's auction house `requires_notary` flag (the reason M2 is never keyless).
pub fn m2_requires_notary(d: &[u8]) -> Result<(Pubkey, bool), String> {
    let mut r = Reader::account(d, "AuctionHouse")?;
    r.take(32 * 4)?;
    let notary = r.key()?;
    r.take(2 + 6)?;
    Ok((notary, r.bool()?))
}

// ---- asks ---------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub enum Venue {
    Tensor,
    Mmm,
    M2,
}

/// A listing an off-chain index named (untrusted: only its addresses are used, every fact is read on chain).
#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub struct Candidate {
    pub venue: Venue,
    /// Tensor: the `ListState`; mmm: the `SellState`; M2: the seller trade state.
    pub listing: Pubkey,
    pub asset: Pubkey,
}

#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub enum AskDetail {
    Tensor(ListState),
    Mmm { pool: Pubkey, state: MmmPool, sell: SellState, escrow_lamports: u64 },
    /// The listing and the auction house it names (S5: the purchase needs the house's treasury, authority and notary account).
    M2 { listing: M2Listing, house: Option<crate::solnftm2::AuctionHouse> },
}

/// An ask verified on chain: the listing exists, is live, is in SOL, and its escrow (or the seller) really holds the asset.
#[derive(Clone, Debug, PartialEq, Eq, CandidType, Serialize, Deserialize)]
pub struct Ask {
    pub venue: Venue,
    pub listing: Pubkey,
    pub asset: Pubkey,
    /// The seller (Tensor `owner`, the pool's owner, the M2 seller): the floor counts one ask per seller.
    pub seller: Pubkey,
    /// The listing's own price (Tensor `amount`, the pool's one-step sell price, M2 `buyer_price`).
    pub price: u64,
    /// Everything the buyer pays for the item: price, taker / LP fees and royalties (an upper bound; rent apart, [`Ask::rent`]).
    pub total: u64,
    /// A key that must co-sign the purchase (`None`: keyless).
    pub cosigner: Option<Pubkey>,
    pub detail: AskDetail,
}

impl Ask {
    pub fn keyless(&self) -> bool {
        self.cosigner.is_none() && self.venue != Venue::M2
    }
    /// The rent the purchase also costs the buyer (its ATA; a pNFT's token record), none for Core.
    pub fn rent(&self, m: &Member) -> u64 {
        match m.standard {
            Standard::Core => 0,
            Standard::Legacy => ATA_RENT,
            Standard::Programmable => ATA_RENT + TOKEN_RECORD_RENT,
        }
    }
}

fn bps(x: u64, b: u64) -> u64 {
    ((x as u128 * b as u128) / 10_000) as u64
}

/// Tensor: `amount` + 2 % taker fee + royalties (pNFT and Core: enforced, 100 %; legacy: none, `optional_royalty_pct = None`).
pub fn tensor_royalty(amount: u64, m: &Member) -> u64 {
    match m.standard {
        Standard::Legacy => 0,
        _ => bps(amount, m.royalty_bps as u64),
    }
}

pub fn tensor_total(amount: u64, m: &Member) -> u64 {
    amount + bps(amount, TENSOR_TAKER_FEE_BPS) + tensor_royalty(amount, m)
}

/// mmm `sol_*_fulfill_sell` with maker and taker fees 0 and, for legacy, `buyside_creator_royalty_bp` 0: the price, the LP fee
/// (charged only while the escrow covers the spot price) and the royalties (a pNFT's: enforced, 100 %).
pub fn mmm_total(price: u64, p: &MmmPool, escrow_lamports: u64, m: &Member) -> u64 {
    let lp = if p.sellside_asset_amount >= 1 && escrow_lamports >= p.spot_price { bps(price, p.lp_fee_bp as u64) } else { 0 };
    let royalty = if m.standard == Standard::Programmable { bps(price, m.royalty_bps as u64) } else { 0 };
    price + lp + royalty
}

/// The accounts [`verify_ask`] reads for a candidate (besides the membership ones): the listing, and what holds the item.
pub fn ask_keys(c: &Candidate, listing: Standard) -> Vec<Pubkey> {
    match c.venue {
        Venue::Tensor => {
            let mut v = vec![c.listing];
            if listing != Standard::Core {
                v.push(soltx::associated_token_address(&c.listing, &c.asset));
            }
            v
        }
        Venue::Mmm => vec![c.listing],
        Venue::M2 => vec![c.listing, key(M2_AUCTION_HOUSE)],
    }
}

/// The second round of keys for an mmm candidate (its pool, the pool's escrow and the pool's token account), known once the
/// `SellState` is read; Tensor and M2 need none ... M2: the seller's token account, named by the trade state.
pub fn ask_keys_then(c: &Candidate, a: &Accounts) -> Vec<Pubkey> {
    let Some(Some(acct)) = a.get(&c.listing) else { return vec![] };
    match c.venue {
        Venue::Mmm => match decode_sell_state(&acct.data) {
            Ok(s) => vec![s.pool, mmm_escrow_address(&s.pool), soltx::associated_token_address(&s.pool, &c.asset)],
            Err(_) => vec![],
        },
        Venue::M2 => decode_m2_listing(&acct.data).map(|l| vec![l.token_account]).unwrap_or_default(),
        Venue::Tensor => vec![],
    }
}

fn holds(a: &Accounts, ta: &Pubkey, owner: &Pubkey, mint: &Pubkey) -> Result<(), String> {
    let acct = match a.get(ta) {
        Some(Some(x)) => x,
        _ => return Err(format!("the token account {} is not there", soltx::b58_encode(ta))),
    };
    if acct.owner != key(soltx::TOKEN_PROGRAM) {
        return Err("the escrow token account is not SPL Token's".into());
    }
    let t = nft::decode_token_account(&acct.data)?;
    if t.mint != *mint || t.owner != *owner || t.amount != 1 {
        return Err(format!("the escrow holds {} of the mint for {}", t.amount, soltx::b58_encode(&t.owner)));
    }
    Ok(())
}

/// One candidate, verified on the accounts read at `finalized` ([`ask_keys`], [`ask_keys_then`], and the member's own): `m` is
/// the asset's membership (`nft::is_member`, already decided on the same read).
pub fn verify_ask(c: &Candidate, m: &Member, a: &Accounts, now: i64) -> Result<Ask, String> {
    if m.asset != c.asset {
        return Err("the member is another asset".into());
    }
    let acct = match a.get(&c.listing) {
        Some(Some(x)) => x,
        _ => return Err(format!("the listing {} is gone", soltx::b58_encode(&c.listing))),
    };
    match c.venue {
        Venue::Tensor => {
            if acct.owner != key(TENSOR_MARKETPLACE) {
                return Err("the listing is not Tensor Marketplace's".into());
            }
            let l = decode_list_state(&acct.data)?;
            if l.asset != c.asset || list_state_address(&c.asset) != c.listing {
                return Err("the ListState is not this asset's".into());
            }
            if l.version != 1 || l.currency.is_some() || l.expiry < now || l.private_taker.is_some() {
                return Err(format!("not takeable: version {}, currency {:?}, expiry {}, private {:?}", l.version, l.currency.is_some(),
                    l.expiry, l.private_taker.is_some()));
            }
            match m.standard {
                Standard::Core => {
                    if m.core_owner != Some(c.listing) {
                        return Err("the Core asset is not escrowed by the listing".into());
                    }
                }
                _ => holds(a, &soltx::associated_token_address(&c.listing, &c.asset), &c.listing, &c.asset)?,
            }
            Ok(Ask { venue: Venue::Tensor, listing: c.listing, asset: c.asset, seller: l.owner, price: l.amount,
                total: tensor_total(l.amount, m), cosigner: l.cosigner, detail: AskDetail::Tensor(l) })
        }
        Venue::Mmm => {
            if acct.owner != key(MMM) {
                return Err("the listing is not mmm's".into());
            }
            let s = decode_sell_state(&acct.data)?;
            if s.asset != c.asset || s.amount != 1 || mmm_sell_state_address(&s.pool, &c.asset) != c.listing {
                return Err("the SellState is not this asset's".into());
            }
            let pacct = match a.get(&s.pool) {
                Some(Some(x)) if x.owner == key(MMM) => x,
                _ => return Err("the pool is gone".into()),
            };
            let p = decode_mmm_pool(&pacct.data)?;
            if p.owner != s.pool_owner || mmm_pool_address(&p.owner, &p.uuid) != s.pool {
                return Err("the pool does not match the SellState".into());
            }
            if p.payment_mint != [0; 32] || (p.expiry != 0 && p.expiry <= now) || p.sellside_asset_amount == 0 {
                return Err("the pool does not sell for SOL now".into());
            }
            if m.standard == Standard::Core {
                return Err("mmm Core pools are not built here".into());
            }
            holds(a, &soltx::associated_token_address(&s.pool, &c.asset), &s.pool, &c.asset)?;
            let escrow = a.get(&mmm_escrow_address(&s.pool)).and_then(|x| x.as_ref()).map(|x| x.lamports).unwrap_or(0);
            let price = mmm_sell_price(&p)?;
            let total = mmm_total(price, &p, escrow, m);
            Ok(Ask { venue: Venue::Mmm, listing: c.listing, asset: c.asset, seller: p.owner, price, total, cosigner: Some(p.cosigner),
                detail: AskDetail::Mmm { pool: s.pool, state: p, sell: s, escrow_lamports: escrow } })
        }
        Venue::M2 => {
            if acct.owner != key(M2) {
                return Err("the listing is not M2's".into());
            }
            let l = decode_m2_listing(&acct.data)?;
            if l.mint != c.asset || l.auction_house != key(M2_AUCTION_HOUSE) || l.token_size != 1 || l.payment_mint != [0; 32] {
                return Err("the M2 listing is not a SOL listing of this asset".into());
            }
            if l.expiry > 0 && l.expiry < now {
                return Err("the M2 listing expired".into());
            }
            // M2 escrows the item in its program-as-signer's token account (pNFTs, recent listings), or leaves it in the seller's
            // with that PDA as delegate (older legacy listings)
            let signer = m2_program_as_signer();
            if holds(a, &l.token_account, &signer, &c.asset).is_err() {
                holds(a, &l.token_account, &l.seller, &c.asset)?;
                let t = nft::decode_token_account(&a[&l.token_account].as_ref().expect("held").data)?;
                if t.delegate != Some(signer) {
                    return Err("the seller holds the item without M2 as its delegate: not a live M2 listing".into());
                }
            }
            // the house is needed to BUY (`solnftm2::m2_buy`), not to count the ask in a floor
            let house = match a.get(&key(M2_AUCTION_HOUSE)) {
                Some(Some(x)) if x.owner == key(M2) => Some(crate::solnftm2::decode_auction_house(&x.data)?),
                _ => None,
            };
            // S5: a taker pays the price, M2's 2 % taker fee and the full royalty (`solnftm2::m2_total`)
            let total = crate::solnftm2::m2_total(l.price, m);
            Ok(Ask { venue: Venue::M2, listing: c.listing, asset: c.asset, seller: l.seller, price: l.price, total,
                cosigner: Some(key(ME_NOTARY)), detail: AskDetail::M2 { listing: l, house } })
        }
    }
}

/// The cheapest ask whose total and rent fit `limit`: keyless ones only, unless `cosigned` (an API that co-signs is configured
/// for that venue). M2 is never chosen (the canister builds no M2 purchase). Ties: the lower price, then the listing address (deterministic across replicas).
pub fn best_ask<'a>(asks: &'a [(Ask, Member)], limit: u64, cosigned: &[Venue]) -> Option<&'a (Ask, Member)> {
    asks.iter()
        .filter(|(x, _)| x.venue != Venue::M2 && (x.keyless() || cosigned.contains(&x.venue)))
        .filter(|(x, m)| x.total.saturating_add(x.rent(m)) <= limit)
        .min_by(|(x, _), (y, _)| (x.total, x.price, x.listing).cmp(&(y.total, y.price, y.listing)))
}

// ---- the instructions -----------------------------------------------------------------------------------------------------------------

fn w(k: Pubkey) -> soltx::AccountMeta {
    meta(k, false, true)
}
fn r(k: Pubkey) -> soltx::AccountMeta {
    meta(k, false, false)
}

/// The `max_amount` the canister signs for a Tensor ask: the listing plus the royalties the program adds (`price ≤ max_amount`);
/// the 2 % taker fee comes on top and is bounded by the listing (`amount` is read by the program from the `ListState`).
pub fn tensor_max_amount(ask: &Ask, m: &Member) -> u64 {
    ask.price + tensor_royalty(ask.price, m)
}

/// Tensor `buy_legacy` (legacy and pNFT) or `buy_core`, the canister (`payer`) buying for itself: no taker broker (the 1 % of
/// the fee a taker broker would get goes to Tensor's fee vault), the listing's maker broker, the creators as remaining accounts.
pub fn tensor_buy(payer: &Pubkey, ask: &Ask, m: &Member) -> Result<Instruction, String> {
    let AskDetail::Tensor(l) = &ask.detail else { return Err("not a Tensor ask".into()) };
    if let Some(c) = l.cosigner {
        return Err(format!("the listing needs {}'s co-signature", soltx::b58_encode(&c)));
    }
    let tcmp = key(TENSOR_MARKETPLACE);
    let none = || r(tcmp); // Anchor's "None" for an optional account: the program's own id
    let maker_broker = l.maker_broker.map(w).unwrap_or_else(none);
    let fee_vault = tensor_fee_vault(&ask.listing);
    let creators = m.creators.iter().map(|(c, _)| w(*c));
    if m.standard == Standard::Core {
        let accounts = vec![w(fee_vault), w(ask.listing), w(ask.asset), r(m.collection), r(*payer), meta(*payer, true, true), w(l.owner),
            none(), maker_broker, w(l.rent_payer), r(key(nft::MPL_CORE)), r(tcmp), r(soltx::SYSTEM_PROGRAM), none()];
        let data = Borsh::ix("buy_core").u64(tensor_max_amount(ask, m)).done();
        return Ok(Instruction { program: tcmp, accounts: accounts.into_iter().chain(creators).collect(), data });
    }
    let mint = ask.asset;
    let buyer_ta = soltx::associated_token_address(payer, &mint);
    let list_ta = soltx::associated_token_address(&ask.listing, &mint);
    let pnft = m.standard == Standard::Programmable;
    let opt = |k: Pubkey, writable: bool| if pnft { meta(k, false, writable) } else { none() };
    let mut accounts = vec![w(fee_vault), r(*payer), w(buyer_ta), w(list_ta), w(ask.listing), r(mint), w(l.owner), meta(*payer, true, true),
        none(), maker_broker, w(l.rent_payer), r(key(soltx::TOKEN_PROGRAM)), r(key(soltx::ASSOCIATED_TOKEN_PROGRAM)), r(tcmp),
        r(soltx::SYSTEM_PROGRAM), w(nft::metadata_address(&mint)), r(nft::edition_address(&mint)),
        opt(nft::token_record_address(&mint, &buyer_ta), true), opt(nft::token_record_address(&mint, &list_ta), true)];
    accounts.push(match (pnft, m.rule_set) {
        (true, Some(rs)) => r(rs),
        _ => none(),
    });
    accounts.push(if pnft && m.rule_set.is_some() { r(key(nft::TOKEN_AUTH_RULES)) } else { none() });
    accounts.push(opt(key(nft::TOKEN_METADATA), false));
    accounts.push(opt(key(nft::SYSVAR_INSTRUCTIONS), false));
    accounts.push(none()); // cosigner
    accounts.extend(creators);
    // max_amount, optional_royalty_pct: None (legacy pays no royalties; a pNFT's are enforced), authorization_data: None
    let data = Borsh::ix("buy_legacy").u64(tensor_max_amount(ask, m)).u8(0).u8(0).done();
    Ok(Instruction { program: tcmp, accounts, data })
}

/// mmm `sol_fulfill_sell` (legacy) or `sol_mip1_fulfill_sell` (pNFT), the canister (`payer`) buying one asset from the pool's sell
/// side, maker and taker fee 0, `max_payment_amount` = the ask's total. The pool's `cosigner` must also sign.
pub fn mmm_fulfill_sell(payer: &Pubkey, ask: &Ask, m: &Member) -> Result<Instruction, String> {
    let AskDetail::Mmm { pool, state: p, .. } = &ask.detail else { return Err("not an mmm ask".into()) };
    let mint = ask.asset;
    let pool_ta = soltx::associated_token_address(pool, &mint);
    let payer_ta = soltx::associated_token_address(payer, &mint);
    let head = vec![meta(*payer, true, true), w(p.owner), meta(p.cosigner, true, false), w(p.referral), w(*pool), w(mmm_escrow_address(pool))];
    let creators = m.creators.iter().map(|(c, _)| w(*c));
    match m.standard {
        Standard::Legacy => {
            let mut accounts = head;
            accounts.extend([r(nft::metadata_address(&mint)), r(nft::edition_address(&mint)), r(mint), w(pool_ta), w(payer_ta),
                r(soltx::SYSTEM_PROGRAM), w(ask.listing), r(soltx::SYSTEM_PROGRAM), r(key(soltx::TOKEN_PROGRAM)),
                r(key(soltx::ASSOCIATED_TOKEN_PROGRAM)), r(key(SYSVAR_RENT))]);
            accounts.extend(creators);
            let data = Borsh::ix("sol_fulfill_sell").u64(1).u64(ask.total).u16(0).u8(0).u16(0).u16(0).done();
            Ok(Instruction { program: key(MMM), accounts, data })
        }
        Standard::Programmable => {
            let rs = m.rule_set.unwrap_or(key(nft::TOKEN_METADATA));
            let mut accounts = head;
            accounts.extend([w(nft::metadata_address(&mint)), r(mint), r(nft::edition_address(&mint)), w(pool_ta), w(payer_ta),
                r(soltx::SYSTEM_PROGRAM), w(ask.listing), w(nft::token_record_address(&mint, &pool_ta)),
                w(nft::token_record_address(&mint, &payer_ta)), r(rs), r(key(nft::TOKEN_METADATA)), r(key(nft::TOKEN_AUTH_RULES)),
                r(key(nft::SYSVAR_INSTRUCTIONS)), r(soltx::SYSTEM_PROGRAM), r(key(soltx::TOKEN_PROGRAM)),
                r(key(soltx::ASSOCIATED_TOKEN_PROGRAM)), r(key(SYSVAR_RENT))]);
            accounts.extend(creators);
            let data = Borsh::ix("sol_mip1_fulfill_sell").u64(1).u64(ask.total).u8(0).u16(0).u16(0).done();
            Ok(Instruction { program: key(MMM), accounts, data })
        }
        Standard::Core => Err("mmm Core pools are not built here".into()),
    }
}

/// The purchase: a compute-unit limit, then the venue's instruction (it creates the canister's ATA itself).
pub fn buy_instructions(payer: &Pubkey, ask: &Ask, m: &Member) -> Result<Vec<Instruction>, String> {
    let ix = match ask.venue {
        Venue::Tensor => tensor_buy(payer, ask, m)?,
        Venue::Mmm => mmm_fulfill_sell(payer, ask, m)?,
        Venue::M2 => return Err("M2 is never bought by the canister (notary-gated)".into()),
    };
    Ok(vec![soltx::compute_unit_limit(BUY_CU), ix])
}

/// Measured in LiteSVM (doc §3.4): the pNFT paths cost ≈ 150–230k CU; 400k leaves room for longer creator lists.
pub const BUY_CU: u32 = 400_000;

// ---- the judge ----------------------------------------------------------------------------------------------------------------------

/// What a purchase must do, from chain-verified facts only.
#[derive(Clone, Debug)]
pub struct Expect<'a> {
    /// The canister's wallet: the only payer, and where the NFT lands.
    pub payer: Pubkey,
    pub ask: &'a Ask,
    /// The asset's membership, decided on the same `finalized` read as the ask.
    pub member: &'a Member,
    /// The listed collection.
    pub collection: Pubkey,
    /// The most the canister may pay for this item (its floor bound), rent included.
    pub limit: u64,
}

/// The judge: refuse to sign unless the transaction's instructions (decompiled; whoever built them) buy exactly this ask for the
/// canister, at most at `limit`:
/// - J1 programs: only Compute Budget and the ask's venue; exactly one venue instruction;
/// - J2 the venue instruction is the canonical one for this ask: the program id, the instruction (discriminator) for the asset's
///   standard, and EVERY account (and its signer / writable flags) equal to [`buy_instructions`]' — so the destination is the
///   canister's own token account (or, Core, the canister), the fee vault the protocol's, no taker broker, the maker broker and
///   the seller the listing's, the creators the metadata's, the rule set the asset's;
/// - J3 price: the signed maximum (`max_amount` / `max_payment_amount`) is at most the ask's (front-running cannot raise it) and
///   the most the instruction can spend (max + Tensor's taker fee) plus rent is at most `limit`; mmm fees within the program's 5 %;
/// - J4 membership: the member is this asset, of this collection.
pub fn judge(ixs: &[Instruction], e: &Expect) -> Result<(), String> {
    let (ask, m) = (e.ask, e.member);
    if m.asset != ask.asset || m.collection != e.collection {
        return Err("J4: the member is not this asset of this collection".into());
    }
    let venue_program = key(match ask.venue {
        Venue::Tensor => TENSOR_MARKETPLACE,
        Venue::Mmm => MMM,
        Venue::M2 => return Err("J1: M2 is never signed".into()),
    });
    let mut venue_ix = None;
    for ix in ixs {
        if ix.program == key(soltx::COMPUTE_BUDGET_PROGRAM) {
            soltx::compute_budget_ok(ix).map_err(|e| format!("J1: {e}"))?;
            continue;
        }
        if ix.program != venue_program {
            return Err(format!("J1: an instruction of {}", soltx::b58_encode(&ix.program)));
        }
        if venue_ix.replace(ix).is_some() {
            return Err("J1: two venue instructions".into());
        }
    }
    let ix = venue_ix.ok_or("J1: no venue instruction")?;
    let want = buy_instructions(&e.payer, ask, m)?.pop().expect("the venue instruction");
    if ix.data.get(..8) != want.data.get(..8) {
        return Err("J2: not the instruction for this asset's standard".into());
    }
    if ix.accounts != want.accounts {
        let at = ix.accounts.iter().zip(&want.accounts).position(|(x, y)| x != y).unwrap_or(ix.accounts.len().min(want.accounts.len()));
        return Err(format!("J2: the accounts differ from the canonical purchase (first at #{at} of {} vs {})", ix.accounts.len(),
            want.accounts.len()));
    }
    let mut rd = Reader { data: &ix.data, at: 8 };
    let spend = match ask.venue {
        Venue::Tensor => {
            let max = rd.u64()?;
            if m.standard != Standard::Core {
                // optional_royalty_pct: None; authorization_data: None
                if rd.u8()? != 0 || rd.u8()? != 0 {
                    return Err("J3: unexpected royalty or authorization arguments".into());
                }
            }
            if rd.at != ix.data.len() {
                return Err("J3: trailing instruction data".into());
            }
            if max > tensor_max_amount(ask, m) {
                return Err(format!("J3: max_amount {max} over the ask's {}", tensor_max_amount(ask, m)));
            }
            max + bps(ask.price, TENSOR_TAKER_FEE_BPS)
        }
        Venue::Mmm => {
            if rd.u64()? != 1 {
                return Err("J3: not one asset".into());
            }
            let max = rd.u64()?;
            if m.standard == Standard::Legacy && rd.u16()? != 0 {
                return Err("J3: a buyside royalty for a legacy item".into());
            }
            if rd.u8()? != 0 {
                return Err("J3: an allowlist_aux".into());
            }
            let (maker, taker) = (rd.u16()? as i16, rd.u16()? as i16);
            if rd.at != ix.data.len() {
                return Err("J3: trailing instruction data".into());
            }
            if !(0..=MMM_MAX_FEE_BP).contains(&taker) || !(-MMM_MAX_FEE_BP..=MMM_MAX_FEE_BP).contains(&maker) || maker + taker > MMM_MAX_FEE_BP {
                return Err("J3: mmm fees out of range".into());
            }
            if max > ask.total + bps(ask.price, taker as u64) {
                return Err(format!("J3: max_payment_amount {max} over the ask's {} (+ taker fee)", ask.total));
            }
            max
        }
        Venue::M2 => unreachable!(),
    };
    let all_in = spend.saturating_add(ask.rent(m));
    if all_in > e.limit {
        return Err(format!("J3: up to {all_in} lamports (rent included) over the limit {}", e.limit));
    }
    Ok(())
}

// ---- the read -------------------------------------------------------------------------------------------------------------------------

/// The first round of keys [`verified_asks`] reads for `cands`: every member's accounts and every listing's ([`ask_keys`]).
pub fn ask_keys_all(cands: &[Candidate], collection: &Pubkey, listing: Standard) -> Vec<Pubkey> {
    let mut keys: Vec<Pubkey> = Vec::new();
    for c in cands {
        keys.extend(nft::member_keys(&c.asset, collection, listing));
        keys.extend(ask_keys(c, listing));
    }
    keys
}

/// Every candidate decided on accounts already read (both rounds: [`ask_keys_all`], then [`ask_keys_then`] of each): the asks that
/// hold, each with its membership, and why each other candidate was dropped. One asset counts once: listed on two venues (an
/// aggregator re-listing the same escrow), the cheaper ask stands. Pure.
pub fn verify_all(a: &Accounts, cands: &[Candidate], collection: &Pubkey, listing: Standard, now: i64) -> (Vec<(Ask, Member)>, Vec<(Candidate, String)>) {
    let (mut good, mut bad): (Vec<(Ask, Member)>, Vec<(Candidate, String)>) = (vec![], vec![]);
    for c in cands {
        match nft::is_member(&c.asset, collection, listing, a).map_err(String::from).and_then(|m| verify_ask(c, &m, a, now).map(|x| (x, m))) {
            Ok(x) => good.push(x),
            Err(e) => bad.push((c.clone(), e)),
        }
    }
    good.sort_by(|(x, _), (y, _)| (x.asset, x.total, x.listing).cmp(&(y.asset, y.total, y.listing)));
    good.dedup_by(|later, first| later.0.asset == first.0.asset);
    (good, bad)
}

/// Every candidate verified at `finalized` (two rounds of agreed `getMultipleAccounts`: the listings and the members, then what the
/// listings name): [`verify_all`] on the read.
pub async fn verified_asks<P: crate::solrpc::SolRpc>(ps: &mut [P], cands: &[Candidate], collection: &Pubkey, listing: Standard, now: i64)
    -> Result<(Vec<(Ask, Member)>, Vec<(Candidate, String)>), crate::solrpc::Refused> {
    let mut a = read_all(ps, &ask_keys_all(cands, collection, listing)).await?;
    let more: Vec<Pubkey> = cands.iter().flat_map(|c| ask_keys_then(c, &a)).filter(|k| !a.contains_key(k)).collect();
    if !more.is_empty() {
        a.extend(read_all(ps, &more).await?);
    }
    Ok(verify_all(&a, cands, collection, listing, now))
}

async fn read_all<P: crate::solrpc::SolRpc>(ps: &mut [P], keys: &[Pubkey]) -> Result<Accounts, crate::solrpc::Refused> {
    let mut keys = keys.to_vec();
    keys.sort_unstable();
    keys.dedup();
    let mut out = Accounts::new();
    for chunk in keys.chunks(crate::solrpc::MAX_KEYS) {
        out.extend(nft::read(ps, chunk).await?);
    }
    Ok(out)
}
