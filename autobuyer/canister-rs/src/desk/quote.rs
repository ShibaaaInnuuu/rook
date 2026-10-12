//! Quotes (§6.1): what a deposit would do now, from the books alone (no chain read), with the desk's fee and minimums.
use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::intent::{FEE_LAMPORTS, MIN_BURN_UNITS, MIN_MINT_LAMPORTS};
use crate::books::{Books, BooksError, ONE};

/// The rent of a new associated token account (165 bytes), charged to a mint's deposit when the recipient has none.
pub const ATA_RENT_LAMPORTS: u64 = 2_039_280;

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MintQuote {
    pub coll: u16,
    /// The SOL deposit the quote is for.
    pub deposit: u64,
    pub fee: u64,
    /// The token account's rent, assumed charged (the recipient has none): `0` once it exists.
    pub ata_rent: u64,
    /// Lamports that reach `M[i]`.
    pub to_module: u64,
    /// wX base units minted.
    pub units: u64,
    /// Lamports per whole wX at this price.
    pub unit_price: u64,
    pub min_deposit: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BurnQuote {
    pub coll: u16,
    /// The wX deposit (base units) the quote is for.
    pub units: u64,
    /// What the books pay for it, before the fee.
    pub paid: u64,
    pub fee: u64,
    /// What reaches the recipient.
    pub net: u64,
    pub min_units: u64,
}

/// The mint of `deposit` lamports now. `ata_rent`: what the recipient's token account costs (0 when it exists).
pub fn quote_mint(books: &Books, coll: u16, deposit: u64, ata_rent: u64, now: i64) -> Result<MintQuote, BooksError> {
    let to_module = deposit.checked_sub(FEE_LAMPORTS).and_then(|d| d.checked_sub(ata_rent)).ok_or(BooksError::BadInput)?;
    let units = books.quote_mint(coll, to_module, now)?;
    let unit_price = u64::try_from((to_module as u128).saturating_mul(ONE as u128) / (units.max(1) as u128)).unwrap_or(u64::MAX);
    Ok(MintQuote { coll, deposit, fee: FEE_LAMPORTS, ata_rent, to_module, units, unit_price, min_deposit: MIN_MINT_LAMPORTS })
}

/// The burn of `units` wX now.
pub fn quote_burn(books: &Books, coll: u16, units: u64, now: i64) -> Result<BurnQuote, BooksError> {
    let paid = books.quote_burn(coll, units, now)?;
    Ok(BurnQuote { coll, units, paid, fee: FEE_LAMPORTS, net: paid.saturating_sub(FEE_LAMPORTS), min_units: MIN_BURN_UNITS })
}
