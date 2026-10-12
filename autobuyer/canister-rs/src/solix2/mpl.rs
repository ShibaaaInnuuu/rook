//! Metaplex Token Metadata, by hand (Borsh after a one-byte instruction index): the wX metadata of §2.
use super::{id, ix, metadata_address, ro, rw, signer, METADATA_PROGRAM};
use crate::soltx::{Borsh, Instruction, Pubkey, SYSTEM_PROGRAM};

pub const CREATE_METADATA_ACCOUNT_V3: u8 = 33;
pub const UPDATE_METADATA_ACCOUNT_V2: u8 = 15;

/// `DataV2` as wX uses it: no creators, no collection, no uses, 0 royalty.
fn data_v2(b: Borsh, name: &str, symbol: &str, uri: &str) -> Borsh {
    b.string(name).string(symbol).string(uri).u16(0).u8(0).u8(0).u8(0)
}

/// `CreateMetadataAccountV3`: mutable, no collection details. The mint authority signs; `update_authority` is only named (the
/// ceremony names `K_coll[i]`, which never signs the creation).
pub fn create_metadata_v3(mint: Pubkey, mint_authority: Pubkey, payer: Pubkey, update_authority: Pubkey, name: &str, symbol: &str, uri: &str) -> Instruction {
    let d = data_v2(Borsh(vec![CREATE_METADATA_ACCOUNT_V3]), name, symbol, uri).bool(true).u8(0).done();
    ix(id(METADATA_PROGRAM), vec![rw(metadata_address(&mint)), ro(mint), signer(mint_authority, false), signer(payer, true),
        ro(update_authority), ro(SYSTEM_PROGRAM)], d)
}

/// Audit B2-02: is this `UpdateMetadataAccountV2` data exactly a rename as [`update_metadata_v2`] builds it? `Some(DataV2 { name, symbol,
/// uri, seller_fee 0, creators None, collection None, uses None })`, then `update_authority`, `primary_sale_happened`, `is_mutable` all
/// `None`: the three strings are walked by their lengths, and what follows must be exactly eight zero bytes (no trailing bytes, no key).
pub fn is_rename_only(data: &[u8]) -> bool {
    if data.len() < 2 || data[0] != UPDATE_METADATA_ACCOUNT_V2 || data[1] != 1 {
        return false;
    }
    let mut at = 2usize;
    for _ in 0..3 {
        let Some(len) = data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize) else { return false };
        at += 4;
        if data.len() < at + len {
            return false;
        }
        at += len;
    }
    data[at..] == [0u8; 8]
}

/// `UpdateMetadataAccountV2` that only renames (name, symbol, uri): `update_authority`, `primary_sale_happened` and
/// `is_mutable` stay `None`, so the update authority can never move and the metadata never freezes through this builder.
pub fn update_metadata_v2(mint: Pubkey, update_authority: Pubkey, name: &str, symbol: &str, uri: &str) -> Instruction {
    let d = data_v2(Borsh(vec![UPDATE_METADATA_ACCOUNT_V2, 1]), name, symbol, uri).u8(0).u8(0).u8(0).done();
    ix(id(METADATA_PROGRAM), vec![rw(metadata_address(&mint)), signer(update_authority, false)], d)
}
