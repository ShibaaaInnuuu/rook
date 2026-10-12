//! The canister's address lookup table for Magic Eden M2 purchases (2026-10-12, audit C1-03 "fixed further"; docs/v9/solana-nfts-canister.md
//! §9). An M2 pNFT purchase names ~30 accounts (`solnftm2::m2_buy`: `deposit`, `buy_v2`, `mip1_execute_sale_v2` + creators); as the flow
//! sends it (the durable nonce's advance, the advance `M[i] → K_main` with its second signature, the purchase, the memo) it is 1,395–1,461
//! bytes in a legacy message, over Solana's 1,232. Two designs were weighed:
//!
//! - (a) two transactions, the advance alone first and then the purchase signed by `K_main` alone: measured on the fixtures, the
//!   purchase alone with its nonce and memo is still 1,282–1,348 bytes for every pNFT (Mad Lads 1,315; even without its compute-limit
//!   instruction 1,242–1,308). It does not fit, and it would split the one atomic transaction the advance's undo relies on.
//! - (b) **a v0 message loading M2's fixed accounts from a lookup table the canister owns** (chosen): the same ONE transaction, signed
//!   by the same two keys, judged on the same instructions (`buy::judge_tx`, `solnftm2::judge`: only `SetComputeUnitLimit` beside it,
//!   B2-01), only encoded with one-byte indexes for the table's keys: Mad Lads 1,428 → 1,061 bytes, every M2 pNFT fixture 1,028–1,094
//!   (≥ 138 bytes of margin), the legacy M2 one 980. Tensor and MMM purchases are unchanged (legacy messages).
//!
//! The table ([`fixed_addresses`]): Magic Eden's auction house and the three keys it names (notary, authority, treasury, as read on
//! chain at finalized with the ask), M2's `["m2", "signer"]` PDA, `K_main`'s M2 escrow PDA, the SPL Token, Token Metadata, Token Auth
//! Rules and Associated Token programs, the Instructions and Rent sysvars, Metaplex's two common rule sets. Nothing of a seller, a
//! listing or an item: every key of the hand-in stays a static key of the message. Never the durable nonce's accounts (its advance
//! stays exactly as in every other transaction).
//!
//! Its life: ONE table for the canister, created (`CreateLookupTable` on a finalized slot, `K_main` its authority and payer, ≈ 0.0044 SOL
//! of rent) and filled (`ExtendLookupTable`) in one transaction on the money line ([`pass`], a step of this flow, `SolNftStep::Table`)
//! the first time an M2 ask is to be bought and no table is there; undone (forgotten, made again on a new slot) if it fails or never
//! lands. Before EVERY use it is read at finalized ([`usable`]): owned by the ALT program, `K_main` its authority, never deactivated,
//! holding exactly the addresses it was made with, its last extension older than the finalized slot. The canister never deactivates,
//! freezes, closes or extends it again, and only `K_main` can: what the indexes of a signed message point to cannot change before it
//! lands. `solsend::compile_with` also refuses a v0 message that does not decompile, against the table as read, to the judged
//! instructions. An M2 ask seen while no usable table is there waits for it (skipped, not remembered as refused); one that does not
//! fit even with it is refused as before (`Cache::refused`, C1-03).
use std::cell::RefCell;

use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::*;
use crate::solhome::{LineId, LineOwner, LineStep};
use crate::solnftm2::{self as m2, AuctionHouse};
use crate::soltx::{self, key, Instruction, LookupTable};

/// The canister's table, as created.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub key: Pubkey,
    pub recent_slot: u64,
    /// What it was filled with ([`fixed_addresses`]), in order: what it must hold when read.
    pub addresses: Vec<Pubkey>,
    /// Its creation landed (the line's verdict); `false`: signed and out.
    pub landed: bool,
}

thread_local! {
    /// An M2 ask waited for the table this pass: the collection and the auction house as read with it (transient).
    static WANTED: RefCell<Option<(u16, AuctionHouse)>> = const { RefCell::new(None) };
}

/// `buy::plan_and_sign`: an M2 ask of `coll` is to be bought and no usable table is there.
pub fn want(coll: u16, house: &AuctionHouse) {
    WANTED.with(|w| *w.borrow_mut() = Some((coll, house.clone())));
}

/// Nothing waits for the table (a new pass, a reset).
pub fn unwant() {
    WANTED.with(|w| *w.borrow_mut() = None);
}

/// The table's addresses: M2's fixed accounts for a purchase by `me` (none of a seller, a listing or an item).
pub fn fixed_addresses(me: &Pubkey, house: &AuctionHouse) -> Vec<Pubkey> {
    let ahk = key(m2::M2_AUCTION_HOUSE);
    let mut v = vec![ahk, house.notary, house.authority, house.treasury, m2::program_as_signer().0, m2::escrow_payment(&ahk, me).0,
        key(soltx::TOKEN_PROGRAM), key(crate::solnft::TOKEN_METADATA), key(crate::solnft::TOKEN_AUTH_RULES),
        key(crate::solnft::SYSVAR_INSTRUCTIONS), key(soltx::ASSOCIATED_TOKEN_PROGRAM), key(soltx::SYSVAR_RENT),
        key(crate::solnft::FOUNDATION_RULE_SET), key(crate::solnft::COMPATIBILITY_RULE_SET)];
    let mut seen = vec![];
    v.retain(|k| {
        let fresh = !seen.contains(k);
        seen.push(*k);
        fresh
    });
    v
}

/// The table's creation and filling, one transaction signed by `me` alone: `CreateLookupTable` on `recent_slot`, `ExtendLookupTable`.
pub fn create_instructions(me: &Pubkey, recent_slot: u64, addresses: &[Pubkey]) -> (Vec<Instruction>, Pubkey) {
    let (create, table) = soltx::create_lookup_table(*me, *me, recent_slot);
    (vec![create, soltx::extend_lookup_table(table, *me, *me, addresses)], table)
}

/// The table if it can be used now: read at finalized and checked (see the module's header). `Ok(None)`: none created or landed yet.
pub async fn usable<I: SolNftIo>(io: &mut I, me: &Pubkey) -> Result<Option<LookupTable>, String> {
    let Some(t) = with(|s| s.table.clone()).filter(|t| t.landed) else { return Ok(None) };
    let (_, slot) = io.sol_clock().await?;
    let acc = read(io, &[t.key]).await?;
    let a = acc.get(&t.key).cloned().flatten().ok_or_else(|| format!("the lookup table {} is not on chain", b58_encode(&t.key)))?;
    if a.owner != key(soltx::ADDRESS_LOOKUP_TABLE_PROGRAM) {
        return Err(format!("the lookup table {} is not the ALT program's", b58_encode(&t.key)));
    }
    let st = soltx::decode_lookup_table(&a.data)?;
    if st.authority != Some(*me) || st.deactivation_slot != u64::MAX {
        return Err(format!("the lookup table {} is not K_main's, or is deactivated: never used (OPERATOR)", b58_encode(&t.key)));
    }
    if st.addresses != t.addresses {
        return Err(format!("the lookup table {} does not hold what it was made with: never used (OPERATOR)", b58_encode(&t.key)));
    }
    if st.last_extended_slot >= slot {
        return Ok(None);
    }
    Ok(Some(LookupTable { key: t.key, addresses: st.addresses }))
}

/// The table's creation, signed as this flow's step when an M2 ask waited for it this pass and none is there (or out). `None`: nothing
/// to do.
pub async fn pass<I: SolNftIo>(io: &mut I, now: u64) -> Option<String> {
    let (coll, house) = WANTED.with(|w| w.borrow_mut().take())?;
    if with(|s| s.table.is_some()) {
        return None;
    }
    let me = match io.key_of(&crate::solkey::KeyPath::Main).await {
        Ok(m) => m,
        Err(e) => return Some(format!("solnft: the lookup table waits: {e}")),
    };
    let slot = match io.sol_clock().await {
        Ok((_, s)) => s,
        Err(e) => return Some(format!("solnft: the lookup table waits: {e}")),
    };
    let addresses = fixed_addresses(&me, &house);
    let (ixs, key) = create_instructions(&me, slot, &addresses);
    let step = SolNftStep::Table { coll, key };
    let label = step.label();
    match crate::coins::link::sign_plain(io, LineOwner::SolNft, LineStep::SolNft(step), ixs, vec![], now).await {
        Ok(sig) => {
            let p = io.sol_line_of(LineId::Money).pending.clone();
            with_mut(|s| {
                s.table = Some(Table { key, recent_slot: slot, addresses, landed: false });
                s.out = p;
                s.signed_today += 1;
            });
            Some(format!("{label}: signed and sent ({sig})"))
        }
        Err(e) => Some(format!("{label}: waits: {e}")),
    }
}

/// The creation's outcome (`run::settle`): landed, the table is read before each use; failed or never landed, it is forgotten (made
/// again on a new slot when an M2 ask next waits for it).
pub fn landed(ok: Option<bool>, signature: &str) -> String {
    with_mut(|s| match ok {
        Some(true) => {
            if let Some(t) = s.table.as_mut() {
                t.landed = true;
            }
            format!("solnft: the lookup table landed ({signature}): read at finalized before each M2 purchase")
        }
        _ => {
            s.table = None;
            format!("solnft: the lookup table's creation {} ({signature}): made again when an M2 ask waits for it",
                if ok == Some(false) { "failed on chain" } else { "never landed" })
        }
    })
}
