//! Delivering a Solana NFT from the canister's wallet to a redeemer's (docs/v9/solana-nfts-canister.md §4): wallet to wallet, the
//! one transfer every Metaplex rule set allows.
//! - **legacy**: ATA `CreateIdempotent` for the destination, SPL Token `TransferChecked` (1, 0 decimals), then `CloseAccount` of
//!   the canister's emptied token account (its 0.00204 SOL of rent comes back);
//! - **pNFT**: Token Metadata `TransferV1` with both token records and the item's rule set (Token Metadata creates the destination
//!   ATA and its token record; the canister pays both rents);
//! - **Core**: mpl-core `TransferV1` (with the collection).
//!
//! [`judge_send`] rebuilds the canonical instructions and refuses anything else. Pure; executed against the real programs and the
//! Metaplex Foundation / Compatibility rule sets in LiteSVM (`autobuyer/solnft-svm`).

use crate::solnft::{self as nft, Member, Standard};
use crate::soltx::{self, key, meta, Instruction, Pubkey};

/// Measured in LiteSVM (doc §4): a pNFT `TransferV1` through the foundation rule set ≈ 90–130k CU; 250k leaves room.
pub const SEND_CU: u32 = 250_000;

fn w(k: Pubkey) -> soltx::AccountMeta {
    meta(k, false, true)
}
fn r(k: Pubkey) -> soltx::AccountMeta {
    meta(k, false, false)
}

/// Associated Token `CreateIdempotent` (tag 1): `payer` funds `owner`'s ATA for `mint` if it does not exist yet.
pub fn create_ata_idempotent(payer: &Pubkey, owner: &Pubkey, mint: &Pubkey) -> Instruction {
    Instruction {
        program: key(soltx::ASSOCIATED_TOKEN_PROGRAM),
        accounts: vec![meta(*payer, true, true), w(soltx::associated_token_address(owner, mint)), r(*owner), r(*mint),
            r(soltx::SYSTEM_PROGRAM), r(key(soltx::TOKEN_PROGRAM))],
        data: vec![1],
    }
}

/// SPL Token `TransferChecked` (tag 12) of one 0-decimal token.
pub fn spl_transfer_checked(source: &Pubkey, mint: &Pubkey, dest: &Pubkey, authority: &Pubkey) -> Instruction {
    let mut data = vec![12u8];
    data.extend(1u64.to_le_bytes());
    data.push(0);
    Instruction { program: key(soltx::TOKEN_PROGRAM), accounts: vec![w(*source), r(*mint), w(*dest), meta(*authority, true, false)], data }
}

/// SPL Token `CloseAccount` (tag 9): an empty token account's rent back to its owner.
pub fn spl_close(account: &Pubkey, owner: &Pubkey) -> Instruction {
    Instruction { program: key(soltx::TOKEN_PROGRAM), accounts: vec![w(*account), w(*owner), meta(*owner, true, false)], data: vec![9] }
}

/// Token Metadata `TransferV1` (discriminator 49, `TransferArgs::V1 { amount: 1, authorization_data: None }`), `from` the owner,
/// authority and payer. The optional accounts take Token Metadata's id when absent.
pub fn tm_transfer_v1(from: &Pubkey, to: &Pubkey, mint: &Pubkey, programmable: bool, rule_set: Option<Pubkey>) -> Instruction {
    let tm = key(nft::TOKEN_METADATA);
    let (src, dst) = (soltx::associated_token_address(from, mint), soltx::associated_token_address(to, mint));
    let mut data = vec![49u8, 0];
    data.extend(1u64.to_le_bytes());
    data.push(0);
    Instruction {
        program: tm,
        accounts: vec![
            w(src),
            r(*from),
            w(dst),
            r(*to),
            r(*mint),
            w(nft::metadata_address(mint)),
            r(nft::edition_address(mint)),
            if programmable { w(nft::token_record_address(mint, &src)) } else { r(tm) },
            if programmable { w(nft::token_record_address(mint, &dst)) } else { r(tm) },
            meta(*from, true, false),
            meta(*from, true, true),
            r(soltx::SYSTEM_PROGRAM),
            r(key(nft::SYSVAR_INSTRUCTIONS)),
            r(key(soltx::TOKEN_PROGRAM)),
            r(key(soltx::ASSOCIATED_TOKEN_PROGRAM)),
            r(if rule_set.is_some() { key(nft::TOKEN_AUTH_RULES) } else { tm }),
            r(rule_set.unwrap_or(tm)),
        ],
        data,
    }
}

/// mpl-core `TransferV1` (discriminator 14, `compression_proof: None`), `from` the owner, authority and payer.
pub fn core_transfer_v1(from: &Pubkey, to: &Pubkey, asset: &Pubkey, collection: &Pubkey) -> Instruction {
    let core = key(nft::MPL_CORE);
    Instruction {
        program: core,
        accounts: vec![w(*asset), r(*collection), meta(*from, true, true), meta(*from, true, false), r(*to), r(soltx::SYSTEM_PROGRAM), r(core)],
        data: vec![14, 0],
    }
}

/// The delivery of member `m` (held by `from`, checked by `nft::in_custody`) to `to`: a compute-unit limit, then the standard's
/// transfer. `to` must be a real address other than the canister's own (A-M-2: a delivery to custody is no delivery).
pub fn send_instructions(from: &Pubkey, to: &Pubkey, m: &Member) -> Result<Vec<Instruction>, String> {
    if *to == [0; 32] || to == from {
        return Err("the destination is empty or the canister itself".into());
    }
    if !soltx::is_on_curve(to) {
        // an off-curve destination is a program's PDA: a pNFT rule set may refuse it, and a person cannot sign for it
        return Err("the destination is not a wallet (off the ed25519 curve)".into());
    }
    let mut v = vec![soltx::compute_unit_limit(SEND_CU)];
    match m.standard {
        Standard::Legacy => {
            let src = soltx::associated_token_address(from, &m.asset);
            v.push(create_ata_idempotent(from, to, &m.asset));
            v.push(spl_transfer_checked(&src, &m.asset, &soltx::associated_token_address(to, &m.asset), from));
            v.push(spl_close(&src, from));
        }
        Standard::Programmable => v.push(tm_transfer_v1(from, to, &m.asset, true, m.rule_set)),
        Standard::Core => v.push(core_transfer_v1(from, to, &m.asset, &m.collection)),
    }
    Ok(v)
}

/// Refuse to sign a delivery unless it is exactly [`send_instructions`]' for this item and this destination (compute-budget
/// instructions aside): one item leaves, to `to`, and nothing else moves.
pub fn judge_send(ixs: &[Instruction], from: &Pubkey, to: &Pubkey, m: &Member) -> Result<(), String> {
    let cb = key(soltx::COMPUTE_BUDGET_PROGRAM);
    for i in ixs {
        soltx::compute_budget_ok(i)?;
    }
    let want: Vec<Instruction> = send_instructions(from, to, m)?.into_iter().filter(|i| i.program != cb).collect();
    let got: Vec<&Instruction> = ixs.iter().filter(|i| i.program != cb).collect();
    if got.len() != want.len() || got.iter().zip(&want).any(|(g, w)| *g != w) {
        return Err(format!("the delivery is not the canonical transfer of {} to {}", soltx::b58_encode(&m.asset), soltx::b58_encode(to)));
    }
    Ok(())
}
