//! Magic Eden M2 without the notary (docs/v9/spikes/s5-magic-eden-m2.md): the purchase the canister builds itself, and its judge.
//!
//! M2 (`M2mx93ek…`) is closed source; its interface is `programs/m2_interface/m2.json` in `me-foundation/mmm`. A purchase is three
//! instructions in one transaction: `deposit` (the buyer's escrow PDA receives price + taker fee + royalty), `buy_v2` (writes the
//! buyer trade state) and `execute_sale_v2` (legacy) or `mip1_execute_sale_v2` (pNFT), which moves the item from the seller's token
//! account, pays the seller, the creators and the treasury out of the escrow, and closes the trade states. Only the buyer signs.
//! The `notary` account is passed (Magic Eden's key, as the buyer-only transactions on chain do) but not as a signer.
//!
//! **When M2 wants the notary** (spike S5, doc §2): only to LIST. Every `sell` / `mip1_sell` / `core_sell` read on chain (148 of
//! 148 in a random sample of 500 M2 transactions) is co-signed by the notary; buys are not (405 of 407 `buy_v2`). The execute
//! instructions check the notary ACCOUNT against the auction house (`has_one`, error 6024 `InvalidNotary` for any other key, signer
//! or not), never its signature: in LiteSVM, with M2's mainnet bytes, a buyer-only purchase lands for 22 of 22 live listings of the
//! 12 collections, whatever `requires_notary` and `nprob` say and whatever the clock. So an M2 listing is keyless to TAKE.
//!
//! Pure: no network, no keys, no clock.

use crate::solnft::{self as nft, Member, Standard};
use crate::solnftbuy::{self as buy, Ask, AskDetail, Expect, Venue};
use crate::soltx::{self, key, meta, Borsh, Instruction, Pubkey, Reader};

pub use crate::solnftbuy::{ME_NOTARY, M2, M2_AUCTION_HOUSE};

/// M2's ["m2", "signer"] PDA: the escrow authority of listed items.
pub fn program_as_signer() -> (Pubkey, u8) {
    pda(&[b"m2", b"signer"])
}

/// M2's taker fee as Magic Eden's own transactions pass it (`taker_fee_bp` of `execute_sale_v2`): 2 % of the price, to the treasury.
pub const M2_TAKER_FEE_BP: u16 = 200;
/// `buyer_creator_royalty_bp` as Magic Eden's own transactions pass it: 100 % of the item's royalty.
pub const M2_ROYALTY_BP: u16 = 10_000;
/// The compute-unit limit of a purchase: a pNFT with three creators and a rule set took 0.33–0.38 M units in LiteSVM, varying with
/// the buyer's PDA bumps (some buyer keys passed 0.4 M); no unit price is set, so the headroom costs nothing.
pub const M2_BUY_CU: u32 = 600_000;

/// The auction house account (`E8cU1WiR…`): the facts the purchase and the notary rule depend on.
#[derive(Clone, Debug, PartialEq, Eq, candid::CandidType, serde::Serialize, serde::Deserialize)]
pub struct AuctionHouse {
    pub treasury: Pubkey,
    pub treasury_withdrawal_destination: Pubkey,
    pub authority: Pubkey,
    pub creator: Pubkey,
    pub notary: Pubkey,
    pub bump: u8,
    pub treasury_bump: u8,
    pub seller_fee_bp: u16,
    pub buyer_referral_bp: u16,
    pub seller_referral_bp: u16,
    pub requires_notary: bool,
    pub nprob: u8,
}

pub fn decode_auction_house(d: &[u8]) -> Result<AuctionHouse, String> {
    let mut r = Reader::account(d, "AuctionHouse")?;
    Ok(AuctionHouse { treasury: r.key()?, treasury_withdrawal_destination: r.key()?, authority: r.key()?, creator: r.key()?, notary: r.key()?,
        bump: r.u8()?, treasury_bump: r.u8()?, seller_fee_bp: r.u16()?, buyer_referral_bp: r.u16()?, seller_referral_bp: r.u16()?,
        requires_notary: r.bool()?, nprob: r.u8()? })
}

fn pda(seeds: &[&[u8]]) -> (Pubkey, u8) {
    soltx::find_program_address(seeds, &key(M2)).expect("a PDA")
}

/// The buyer's escrow payment account: `["m2", auction_house, wallet]`.
pub fn escrow_payment(ah: &Pubkey, wallet: &Pubkey) -> (Pubkey, u8) {
    pda(&[b"m2", ah, wallet])
}

/// The buyer's trade state (V2): `["m2", wallet, auction_house, mint]`.
pub fn buyer_trade_state(wallet: &Pubkey, ah: &Pubkey, mint: &Pubkey) -> Pubkey {
    pda(&[b"m2", wallet, ah, mint]).0
}

/// The seller's trade state (V2): `["m2", seller, auction_house, token_account, mint]`.
pub fn seller_trade_state(seller: &Pubkey, ah: &Pubkey, token_account: &Pubkey, mint: &Pubkey) -> Pubkey {
    pda(&[b"m2", seller, ah, token_account, mint]).0
}

fn bps(x: u64, b: u64) -> u64 {
    ((x as u128 * b as u128) / 10_000) as u64
}

/// What M2 takes from the buyer's escrow besides the price: the taker fee and the royalty (a legacy item's too).
pub fn m2_royalty(price: u64, m: &Member) -> u64 {
    bps(price, m.royalty_bps as u64)
}

pub fn m2_taker_fee(price: u64) -> u64 {
    bps(price, M2_TAKER_FEE_BP as u64)
}

/// Everything the buyer pays for the item: the price, the 2 % taker fee and the royalty (rent apart, [`Ask::rent`]).
pub fn m2_total(price: u64, m: &Member) -> u64 {
    price + m2_taker_fee(price) + m2_royalty(price, m)
}

/// The lamports the buyer puts in escrow: exactly what the sale takes out (a surplus could only be taken back with the notary).
pub fn deposit_amount(price: u64, m: &Member) -> u64 {
    m2_total(price, m)
}

/// The three instructions of the purchase for `ask` (an M2 ask): `deposit`, `buy_v2`, then the execute.
pub fn m2_buy(payer: &Pubkey, ask: &Ask, m: &Member) -> Result<Vec<Instruction>, String> {
    let AskDetail::M2 { listing: l, house: Some(ah) } = &ask.detail else { return Err("not an M2 ask with its auction house".into()) };
    if m.standard == Standard::Core {
        return Err("M2 sells no Core assets here".into());
    }
    let ahk = key(M2_AUCTION_HOUSE);
    let (escrow, escrow_bump) = escrow_payment(&ahk, payer);
    let bts = buyer_trade_state(payer, &ahk, &l.mint);
    let sts = seller_trade_state(&l.seller, &ahk, &l.token_account, &l.mint);
    if sts != ask.listing {
        return Err("the listing is not this seller's trade state".into());
    }
    let (signer, signer_bump) = program_as_signer();
    let notary = ah.notary;
    let metadata = nft::metadata_address(&l.mint);
    let buyer_ta = soltx::associated_token_address(payer, &l.mint);
    let w = |k: Pubkey| meta(k, false, true);
    let r = |k: Pubkey| meta(k, false, false);

    let deposit = Instruction { program: key(M2), accounts: vec![meta(*payer, true, true), r(notary), w(escrow), r(ah.authority), r(ahk),
        r(soltx::SYSTEM_PROGRAM)], data: Borsh::ix("deposit").u8(escrow_bump).u64(deposit_amount(l.price, m)).done() };
    // buy_v2(buyer_price, token_size, buyer_state_expiry, buyer_creator_royalty_bp, extra_args: empty bytes)
    let buy_v2 = Instruction { program: key(M2), accounts: vec![meta(*payer, true, true), r(notary), r(l.mint), r(metadata), w(escrow),
        r(ah.authority), r(ahk), w(bts), r(ah.authority), r(key(soltx::TOKEN_PROGRAM)), r(soltx::SYSTEM_PROGRAM)],
        data: Borsh::ix("buy_v2").u64(l.price).u64(1).i64(0).u16(M2_ROYALTY_BP).u32(0).done() };
    let creators = m.creators.iter().map(|(c, _)| w(*c));
    let exec = if m.standard == Standard::Programmable {
        let rs = m.rule_set.unwrap_or(key(nft::TOKEN_METADATA));
        let mut accounts = vec![meta(*payer, true, true), w(*payer), w(l.seller), r(notary), r(signer), w(l.token_account), w(buyer_ta), r(l.mint),
            w(metadata), r(ahk), w(ah.treasury), w(sts), w(bts), w(escrow), w(ah.authority), w(l.seller_referral), r(key(nft::TOKEN_METADATA)),
            r(nft::edition_address(&l.mint)), r(key(nft::TOKEN_AUTH_RULES)), r(rs), w(nft::token_record_address(&l.mint, &l.token_account)),
            w(nft::token_record_address(&l.mint, &buyer_ta)), r(key(nft::SYSVAR_INSTRUCTIONS)), r(key(soltx::ASSOCIATED_TOKEN_PROGRAM)),
            r(key(soltx::TOKEN_PROGRAM)), r(soltx::SYSTEM_PROGRAM), r(key(buy::SYSVAR_RENT))];
        accounts.extend(creators);
        // price, maker_fee_bp (i16), taker_fee_bp (u16)
        Instruction { program: key(M2), accounts, data: Borsh::ix("mip1_execute_sale_v2").u64(l.price).u16(0).u16(M2_TAKER_FEE_BP).done() }
    } else {
        let mut accounts = vec![w(*payer), w(l.seller), r(notary), w(l.token_account), r(l.mint), r(metadata), w(escrow), w(buyer_ta),
            r(ah.authority), r(ahk), w(ah.treasury), w(bts), w(ah.authority), w(sts), w(l.seller_referral), r(key(soltx::TOKEN_PROGRAM)),
            r(soltx::SYSTEM_PROGRAM), r(key(soltx::ASSOCIATED_TOKEN_PROGRAM)), r(signer), r(key(buy::SYSVAR_RENT))];
        accounts.extend(creators);
        // escrow_payment_bump, program_as_signer_bump, buyer_price, token_size, buyer_state_expiry, seller_state_expiry, maker_fee_bp, taker_fee_bp
        Instruction { program: key(M2), accounts, data: Borsh::ix("execute_sale_v2").u8(escrow_bump).u8(signer_bump).u64(l.price).u64(1)
            .i64(0).i64(l.expiry).u16(0).u16(M2_TAKER_FEE_BP).done() }
    };
    Ok(vec![deposit, buy_v2, exec])
}

/// The purchase as the canister sends it: the compute limit, then [`m2_buy`].
pub fn buy_instructions(payer: &Pubkey, ask: &Ask, m: &Member) -> Result<Vec<Instruction>, String> {
    let mut v = vec![soltx::compute_unit_limit(M2_BUY_CU)];
    v.extend(m2_buy(payer, ask, m)?);
    Ok(v)
}

/// The judge for an M2 purchase: refuse unless the instructions buy exactly this ask for the canister, at most at `limit`.
/// - J1: only Compute Budget and M2, exactly `deposit`, `buy_v2`, the execute, in that order;
/// - J2: each is the canonical one for this ask (program, discriminator, every account and its flags): the escrow, the trade states
///   and the destination are the canister's own, the seller, the token account, the treasury and the creators the listing's;
/// - J3: the price in `buy_v2` and the execute is the listing's, the fees are the standard ones, the deposit is exactly what the sale
///   takes (price + 2 % + royalty), no trailing data, and price + fee + royalty + rent is at most `limit`;
/// - J4: the member is this asset of this collection.
pub fn judge(ixs: &[Instruction], e: &Expect) -> Result<(), String> {
    let (ask, m) = (e.ask, e.member);
    if ask.venue != Venue::M2 {
        return Err("J1: not an M2 ask".into());
    }
    if m.asset != ask.asset || m.collection != e.collection {
        return Err("J4: the member is not this asset of this collection".into());
    }
    let mut venue: Vec<&Instruction> = Vec::new();
    for ix in ixs {
        if ix.program == key(soltx::COMPUTE_BUDGET_PROGRAM) {
            soltx::compute_budget_ok(ix).map_err(|e| format!("J1: {e}"))?;
            continue;
        }
        if ix.program != key(M2) {
            return Err(format!("J1: an instruction of {}", soltx::b58_encode(&ix.program)));
        }
        venue.push(ix);
    }
    let want = m2_buy(&e.payer, ask, m)?;
    if venue.len() != want.len() {
        return Err(format!("J1: {} M2 instructions, not {}", venue.len(), want.len()));
    }
    for (n, (got, want)) in venue.iter().zip(&want).enumerate() {
        if got.data.get(..8) != want.data.get(..8) {
            return Err(format!("J2: instruction #{n} is not the canonical one (discriminator)"));
        }
        if got.accounts != want.accounts {
            let at = got.accounts.iter().zip(&want.accounts).position(|(x, y)| x != y).unwrap_or(got.accounts.len().min(want.accounts.len()));
            return Err(format!("J2: instruction #{n}: the accounts differ from the canonical purchase (first at #{at})"));
        }
        // every argument is the canonical one: the program's own rules then decide nothing the canister did not intend
        if got.data != want.data {
            return Err(format!("J3: instruction #{n}: the arguments differ from the canonical purchase"));
        }
    }
    let price = match &ask.detail {
        AskDetail::M2 { listing, .. } => listing.price,
        _ => return Err("J1: not an M2 ask".into()),
    };
    let all_in = m2_total(price, m).saturating_add(ask.rent(m));
    if all_in > e.limit {
        return Err(format!("J3: {all_in} lamports (fee, royalty and rent included) over the limit {}", e.limit));
    }
    Ok(())
}
