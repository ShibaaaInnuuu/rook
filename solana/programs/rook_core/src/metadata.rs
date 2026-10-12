//! The wX mints' token metadata: Metaplex Token Metadata (classic), CPI built by hand (no mpl crate: two instructions, fixed
//! Borsh). Wallets and explorers read the `["metadata", MPL, mint]` account; without it a wX shows as "Unknown token".
//!
//! Authorities: the wrapper PDA is the mint authority (it signs `create_metadata_accounts_v3`) AND the metadata's update
//! authority. Updates go through rook_core's owner-only `set_wx_metadata`, which reads `Registry.owner` each time, so the
//! right to change a name, symbol or image follows a registry ownership transfer (`propose_owner`/`accept_owner`) with no
//! separate Metaplex hand-over that could be forgotten, and no key outside the registry can change a wX's identity.
//! `is_mutable = true`, so the image (uri) can be set after launch.

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, program::invoke_signed},
};

use crate::error::RookError;

/// Metaplex Token Metadata (mainnet and devnet).
pub const MPL_TOKEN_METADATA_ID: Pubkey = pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
pub const METADATA_SEED: &[u8] = b"metadata";
/// Metaplex's limits (`MAX_NAME_LENGTH`, `MAX_SYMBOL_LENGTH`, `MAX_URI_LENGTH`).
pub const MAX_META_NAME: usize = 32;
pub const MAX_META_SYMBOL: usize = 10;
pub const MAX_META_URI: usize = 200;

/// `MetadataInstruction::CreateMetadataAccountV3` / `UpdateMetadataAccountV2` (Borsh enum tags).
const IX_CREATE_V3: u8 = 33;
const IX_UPDATE_V2: u8 = 15;

/// The wX's display identity: name ("Wrapped Milady", ≤ 32 bytes), symbol ("wMILADY", ≤ 10), uri (the metadata JSON with the
/// image, ≤ 200; may be empty and set later).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct WxMetadata {
    pub name: String,
    pub symbol: String,
    pub uri: String,
}

impl WxMetadata {
    pub fn validate(&self) -> Result<()> {
        require!(
            !self.name.is_empty()
                && self.name.len() <= MAX_META_NAME
                && !self.symbol.is_empty()
                && self.symbol.len() <= MAX_META_SYMBOL
                && self.uri.len() <= MAX_META_URI,
            RookError::BadMetadata
        );
        Ok(())
    }

    /// Borsh `DataV2 { name, symbol, uri, seller_fee_basis_points: 0, creators: None, collection: None, uses: None }`.
    fn data_v2(&self, out: &mut Vec<u8>) {
        for s in [&self.name, &self.symbol, &self.uri] {
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0]);
    }
}

/// `create_metadata_accounts_v3`, signed by the wrapper PDA (mint authority and update authority), `is_mutable = true`.
#[allow(clippy::too_many_arguments)]
pub fn create_wx_metadata<'info>(
    program: &AccountInfo<'info>,
    metadata: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    wrapper: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    wrapper_seeds: &[&[u8]],
    m: &WxMetadata,
) -> Result<()> {
    m.validate()?;
    let mut data = vec![IX_CREATE_V3];
    m.data_v2(&mut data);
    data.push(1); // is_mutable
    data.push(0); // collection_details: None
    let ix = Instruction {
        program_id: MPL_TOKEN_METADATA_ID,
        accounts: vec![
            AccountMeta::new(metadata.key(), false),
            AccountMeta::new_readonly(mint.key(), false),
            AccountMeta::new_readonly(wrapper.key(), true), // mint authority
            AccountMeta::new(payer.key(), true),
            AccountMeta::new_readonly(wrapper.key(), true), // update authority
            AccountMeta::new_readonly(system_program.key(), false),
        ],
        data,
    };
    invoke_signed(
        &ix,
        &[metadata.clone(), mint.clone(), wrapper.clone(), payer.clone(), system_program.clone(), program.clone()],
        &[wrapper_seeds],
    )?;
    Ok(())
}

/// `update_metadata_account_v2`: new name, symbol and uri; the update authority (the wrapper PDA) and mutability unchanged.
pub fn update_wx_metadata<'info>(
    program: &AccountInfo<'info>,
    metadata: &AccountInfo<'info>,
    wrapper: &AccountInfo<'info>,
    wrapper_seeds: &[&[u8]],
    m: &WxMetadata,
) -> Result<()> {
    m.validate()?;
    let mut data = vec![IX_UPDATE_V2, 1];
    m.data_v2(&mut data);
    data.extend_from_slice(&[0, 0, 0]); // new_update_authority, primary_sale_happened, is_mutable: None
    let ix = Instruction {
        program_id: MPL_TOKEN_METADATA_ID,
        accounts: vec![AccountMeta::new(metadata.key(), false), AccountMeta::new_readonly(wrapper.key(), true)],
        data,
    };
    invoke_signed(&ix, &[metadata.clone(), wrapper.clone(), program.clone()], &[wrapper_seeds])?;
    Ok(())
}
