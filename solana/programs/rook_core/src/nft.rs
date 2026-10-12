//! Solana NFT listings (docs/v9/solana-nfts-design.md): the NFTs sit in the canister's Solana wallet (the listing's
//! custodian), as EthNft items sit at its threshold-ECDSA address. The program moves no NFT; it **reads** Token Metadata and
//! Metaplex Core accounts to check, at `mint_item`, that the item is a member really held by the custodian and, at
//! `record_delivery`, that the destination now holds it. Layouts read by hand (fixed Borsh; no mpl crate, no CPI).

use anchor_lang::prelude::*;
use anchor_spl::token::spl_token;

use crate::error::RookError;
use crate::metadata::{METADATA_SEED, MPL_TOKEN_METADATA_ID};
use crate::state::NftStandard;

/// Metaplex Core (mainnet and devnet).
pub const MPL_CORE_ID: Pubkey = pubkey!("CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d");
const EDITION_SEED: &[u8] = b"edition";

// Token Metadata keys and token standards (mpl-token-metadata `Key`, `TokenStandard`).
const TM_KEY_MASTER_EDITION_V1: u8 = 2;
const TM_KEY_METADATA_V1: u8 = 4;
const TM_KEY_MASTER_EDITION_V2: u8 = 6;
const TS_NON_FUNGIBLE: u8 = 0;
const TS_PROGRAMMABLE_NON_FUNGIBLE: u8 = 4;

// Metaplex Core keys and plugin types (mpl-core `Key`, `PluginType`).
const CORE_KEY_ASSET_V1: u8 = 1;
const CORE_KEY_PLUGIN_HEADER_V1: u8 = 3;
const CORE_KEY_PLUGIN_REGISTRY_V1: u8 = 4;
const CORE_KEY_COLLECTION_V1: u8 = 5;
const CORE_UA_COLLECTION: u8 = 2;
const PT_FREEZE_DELEGATE: u8 = 1;
const PT_BURN_DELEGATE: u8 = 2;
const PT_TRANSFER_DELEGATE: u8 = 3;
const PT_PERMANENT_FREEZE: u8 = 5;
const PT_PERMANENT_TRANSFER: u8 = 7;
const PT_PERMANENT_BURN: u8 = 8;
const PT_PERMANENT_FREEZE_EXECUTE: u8 = 17;
/// `PluginAuthority::{None, Owner, UpdateAuthority, Address(Pubkey)}`.
const AUTH_OWNER: u8 = 1;
const AUTH_ADDRESS: u8 = 3;

/// A bounds-checked Borsh reader over account data.
struct Rd<'a> {
    d: &'a [u8],
    o: usize,
}

impl<'a> Rd<'a> {
    fn new(d: &'a [u8], o: usize) -> Self {
        Rd { d, o }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.o.checked_add(n).ok_or(RookError::NotAnNft)?;
        let s = self.d.get(self.o..end).ok_or(RookError::NotAnNft)?;
        self.o = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn key(&mut self) -> Result<Pubkey> {
        Ok(Pubkey::new_from_array(self.take(32)?.try_into().unwrap()))
    }
    fn skip_str(&mut self) -> Result<()> {
        let n = self.u32()? as usize;
        self.take(n).map(|_| ())
    }
    /// An `Option<T>` tag; `Ok(false)` past the end of the data (accounts written before a field existed end in zeros or
    /// stop short).
    fn opt(&mut self) -> Result<bool> {
        if self.o >= self.d.len() {
            return Ok(false);
        }
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => err!(RookError::NotAnNft),
        }
    }
}

/// The Token Metadata checks of a legacy or programmable member of `collection` (docs §5): the mint (classic SPL Token,
/// supply 1, decimals 0), its metadata (TM, `MetadataV1`, this mint, verified collection), its master edition (the PDA by
/// the metadata's `edition_nonce`, `MasterEditionV1/V2`) and its token standard.
pub fn inspect_tm(mint: &AccountInfo, metadata: &AccountInfo, edition: &AccountInfo, collection: &Pubkey) -> Result<NftStandard> {
    require_keys_eq!(*mint.owner, spl_token::ID, RookError::NotAnNft);
    {
        let m = mint.try_borrow_data()?;
        require!(m.len() == 82 && m[45] == 1, RookError::NotAnNft);
        let supply = u64::from_le_bytes(m[36..44].try_into().unwrap());
        require!(supply == 1 && m[44] == 0, RookError::NotAnNft);
    }
    require_keys_eq!(*metadata.owner, MPL_TOKEN_METADATA_ID, RookError::NotAnNft);
    let d = metadata.try_borrow_data()?;
    let mut r = Rd::new(&d, 0);
    require!(r.u8()? == TM_KEY_METADATA_V1, RookError::NotAnNft);
    r.take(32)?; // update authority
    require_keys_eq!(r.key()?, mint.key(), RookError::NotAnNft);
    r.skip_str()?; // name
    r.skip_str()?; // symbol
    r.skip_str()?; // uri
    r.take(2)?; // seller fee
    if r.opt()? {
        let n = r.u32()? as usize;
        r.take(n.checked_mul(34).ok_or(RookError::NotAnNft)?)?;
    }
    r.take(2)?; // primary sale happened, is mutable
    let nonce = if r.opt()? { Some(r.u8()?) } else { None };
    let ts = if r.opt()? { Some(r.u8()?) } else { None };
    let standard = match ts {
        None | Some(TS_NON_FUNGIBLE) => NftStandard::Legacy,
        Some(TS_PROGRAMMABLE_NON_FUNGIBLE) => NftStandard::Programmable,
        _ => return err!(RookError::NotAnNft),
    };
    let member = if r.opt()? {
        let verified = r.u8()? == 1;
        verified && r.key()? == *collection
    } else {
        false
    };
    require!(member, RookError::NotInCollection);
    drop(d);
    // the master edition of this mint: print editions and fungibles are refused
    let seeds: &[&[u8]] = &[METADATA_SEED, MPL_TOKEN_METADATA_ID.as_ref(), mint.key.as_ref(), EDITION_SEED];
    let expected = match nonce {
        Some(b) => {
            let bump = [b];
            let mut s = seeds.to_vec();
            s.push(&bump);
            Pubkey::create_program_address(&s, &MPL_TOKEN_METADATA_ID).map_err(|_| error!(RookError::NotAnNft))?
        }
        None => Pubkey::find_program_address(seeds, &MPL_TOKEN_METADATA_ID).0,
    };
    require_keys_eq!(edition.key(), expected, RookError::NotAnNft);
    require_keys_eq!(*edition.owner, MPL_TOKEN_METADATA_ID, RookError::NotAnNft);
    let k = edition.try_borrow_data()?.first().copied().unwrap_or(0);
    require!(k == TM_KEY_MASTER_EDITION_V1 || k == TM_KEY_MASTER_EDITION_V2, RookError::NotAnNft);
    Ok(standard)
}

/// The Core membership checks before a transfer: an `AssetV1` (not compressed) of this listing's collection, and the
/// collection account itself.
pub fn inspect_core(asset: &AccountInfo, coll: &AccountInfo, collection: &Pubkey) -> Result<()> {
    require_keys_eq!(*asset.owner, MPL_CORE_ID, RookError::NotAnNft);
    {
        let d = asset.try_borrow_data()?;
        require!(d.len() >= 66 && d[0] == CORE_KEY_ASSET_V1, RookError::NotAnNft);
        require!(d[33] == CORE_UA_COLLECTION && d[34..66] == collection.to_bytes(), RookError::NotInCollection);
    }
    require_keys_eq!(coll.key(), *collection, RookError::NotInCollection);
    require_keys_eq!(*coll.owner, MPL_CORE_ID, RookError::NotInCollection);
    require!(coll.try_borrow_data()?.first() == Some(&CORE_KEY_COLLECTION_V1), RookError::NotInCollection);
    Ok(())
}

/// A Core asset's owner.
pub fn core_owner(asset: &AccountInfo) -> Result<Pubkey> {
    let d = asset.try_borrow_data()?;
    require!(d.len() >= 33, RookError::NotAnNft);
    Ok(Pubkey::new_from_array(d[1..33].try_into().unwrap()))
}

/// Held by the custodian: neither the asset nor its collection may carry a permanent delegate or an external plugin
/// with lifecycle checks, and the asset no owner-managed delegate whose authority is not its owner (docs §5).
pub fn check_core_plugins(asset: &AccountInfo, coll: &AccountInfo) -> Result<()> {
    let d = asset.try_borrow_data()?;
    let mut r = Rd::new(&d, 33);
    if r.u8()? != 0 {
        r.take(32)?;
    }
    r.skip_str()?; // name
    r.skip_str()?; // uri
    if r.opt()? {
        r.take(8)?; // seq
    }
    check_registry(&d, r.o, true)?;
    let c = coll.try_borrow_data()?;
    let mut r = Rd::new(&c, 33);
    r.skip_str()?;
    r.skip_str()?;
    r.take(8)?; // num_minted, current_size
    check_registry(&c, r.o, false)
}

fn check_registry(d: &[u8], header: usize, asset: bool) -> Result<()> {
    if header >= d.len() {
        return Ok(());
    }
    let mut r = Rd::new(d, header);
    require!(r.u8()? == CORE_KEY_PLUGIN_HEADER_V1, RookError::UnsupportedNft);
    let at = usize::try_from(r.u64()?).map_err(|_| error!(RookError::UnsupportedNft))?;
    let mut r = Rd::new(d, at);
    require!(r.u8()? == CORE_KEY_PLUGIN_REGISTRY_V1, RookError::UnsupportedNft);
    for _ in 0..r.u32()? {
        let t = r.u8()?;
        let a = r.u8()?;
        if a == AUTH_ADDRESS {
            r.take(32)?;
        }
        r.take(8)?;
        let permanent = matches!(t, PT_PERMANENT_FREEZE | PT_PERMANENT_TRANSFER | PT_PERMANENT_BURN | PT_PERMANENT_FREEZE_EXECUTE);
        let delegated = asset && matches!(t, PT_FREEZE_DELEGATE | PT_BURN_DELEGATE | PT_TRANSFER_DELEGATE) && a != AUTH_OWNER;
        require!(!permanent && !delegated, RookError::UnsupportedNft);
    }
    for _ in 0..r.u32()? {
        r.u8()?;
        if r.u8()? == AUTH_ADDRESS {
            r.take(32)?;
        }
        if r.opt()? {
            require!(r.u32()? == 0, RookError::UnsupportedNft);
        }
        r.take(8)?;
        if r.opt()? {
            r.take(8)?;
        }
        if r.opt()? {
            r.take(8)?;
        }
    }
    Ok(())
}

/// A token account's (owner, mint, amount, state).
pub fn token_facts(ta: &AccountInfo) -> Result<(Pubkey, Pubkey, u64, u8)> {
    require_keys_eq!(*ta.owner, spl_token::ID, RookError::NotInCustody);
    let d = ta.try_borrow_data()?;
    require!(d.len() == 165, RookError::NotInCustody);
    Ok((
        Pubkey::new_from_array(d[32..64].try_into().unwrap()),
        Pubkey::new_from_array(d[0..32].try_into().unwrap()),
        u64::from_le_bytes(d[64..72].try_into().unwrap()),
        d[108],
    ))
}

/// The standard-specific accounts of a `SolNft` item, in `remaining_accounts` (docs §4): Token Metadata listings
/// `[mint, metadata, edition, token_account]`, Core listings `[asset, collection]` at `mint_item`; `[token_account]` (Token
/// Metadata) or `[asset]` (Core) at `record_delivery`.
fn at<'a, 'info>(rest: &'a [AccountInfo<'info>], i: usize) -> Result<&'a AccountInfo<'info>> {
    rest.get(i).ok_or_else(|| error!(RookError::MissingAccount))
}

/// `mint_item` on a `SolNft` listing: the item `key` (the mint or the Core asset) is a member of `collection` and is held by
/// `holder` (the custodian's wallet) now.
pub fn check_custody(
    rest: &[AccountInfo],
    standard: NftStandard,
    collection: &Pubkey,
    key: &[u8; 32],
    holder: &Pubkey,
) -> Result<()> {
    let asset = at(rest, 0)?;
    require!(asset.key.to_bytes() == *key, RookError::BadInput);
    if standard == NftStandard::Core {
        let coll = at(rest, 1)?;
        inspect_core(asset, coll, collection)?;
        require_keys_eq!(core_owner(asset)?, *holder, RookError::NotInCustody);
        return check_core_plugins(asset, coll);
    }
    inspect_tm(asset, at(rest, 1)?, at(rest, 2)?, collection)?;
    check_token_holds(at(rest, 3)?, key, holder)
}

/// `record_delivery` on a `SolNft` listing: `holder` (the ticket's destination) now holds the item.
pub fn check_delivered(rest: &[AccountInfo], standard: NftStandard, key: &[u8; 32], holder: &Pubkey) -> Result<()> {
    let a = at(rest, 0)?;
    if standard == NftStandard::Core {
        require!(a.key.to_bytes() == *key, RookError::BadInput);
        require_keys_eq!(*a.owner, MPL_CORE_ID, RookError::NotInCustody);
        require_keys_eq!(core_owner(a)?, *holder, RookError::NotInCustody);
        return Ok(());
    }
    check_token_holds(a, key, holder)
}

fn check_token_holds(ta: &AccountInfo, key: &[u8; 32], holder: &Pubkey) -> Result<()> {
    let (owner, mint, amount, _) = token_facts(ta)?;
    require!(owner == *holder && mint.to_bytes() == *key && amount == 1, RookError::NotInCustody);
    Ok(())
}
