//! The multi-instruction transactions as spikes S1/S2 sized them (docs/v9/spikes/s1-s3-canister-only.md §2.3, §3.5).
//!
//! - **The desk's pool opening.** With a durable-nonce advance in front, "create the desk's ATAs + mint the seed + wrap + create
//!   the pool + memo" is 1,298 B, over the 1,232-byte limit. The fit: the desk's wX and wSOL ATAs are created in an earlier
//!   transaction ([`desk_atas`]; an ATA can exist before any supply), and the memo goes in the next one ([`open_pool_memo`]).
//!   [`open_pool`] is then mint + wrap + `initialize_customizable_pool` (≈ 1,195 B with the nonce and compute-budget
//!   instructions), and the anti-squat order of §5.1 holds: no wX exists before the pool does.
//! - **The site's own two-hop buy** SOL → wX (DAMM v2) → coin (DBC curve, or the graduated DAMM v2 pool), one legacy transaction
//!   a fresh wallet signs ([`two_hop_buy`]). Required: Jupiter drops a coin still on its curve after its grace period, and any
//!   wX whose pool fails its liquidity test, so this path must always work, with or without Jupiter.
use super::{damm, dbc, native_mint, spl};
use crate::soltx::{self, associated_token_address, Instruction, Pubkey};

/// The desk's wX and wSOL ATAs, created (idempotently) before the pool-opening transaction. `K_main` pays the rent.
pub fn desk_atas(k_main: Pubkey, k_desk: Pubkey, wx_mint: Pubkey) -> Vec<Instruction> {
    vec![spl::create_ata_idempotent(k_main, k_desk, wx_mint), spl::create_ata_idempotent(k_main, k_desk, native_mint())]
}

/// The pool opening (wX = token a, SOL = token b): `K_coll[i]` mints the seed wX to the desk, the desk wraps `seed_sol`, and the
/// desk creates the pool with the seed as its first position. Signers: the fee payer, `K_coll[i]`, `K_desk[i]`, the position-NFT
/// mint (a throwaway derived key, §1.1). The caller puts the nonce advance and compute-budget instructions in front.
pub fn open_pool(k_coll: Pubkey, k_desk: Pubkey, wx_mint: Pubkey, nft_mint: Pubkey, seed_wx: u64, seed_sol: u64, params: &damm::CustomizablePool) -> (damm::Pool, Vec<Instruction>) {
    let pool = damm::Pool::customizable(wx_mint, native_mint());
    let desk_wx = associated_token_address(&k_desk, &wx_mint);
    let desk_wsol = associated_token_address(&k_desk, &native_mint());
    let ixs = vec![
        spl::mint_to_checked(wx_mint, desk_wx, k_coll, seed_wx, 9),
        soltx::transfer(k_desk, desk_wsol, seed_sol),
        spl::sync_native(desk_wsol),
        damm::initialize_customizable_pool(&pool, k_desk, k_desk, nft_mint, desk_wx, desk_wsol, params),
    ];
    (pool, ixs)
}

/// The opening's memo, in the next transaction, signed by `K_coll[i]` so nobody else can forge it.
pub fn open_pool_memo(collection: u32, k_coll: Pubkey) -> Instruction {
    spl::memo(&format!("rook:v1:{collection}:pool:open"), &[k_coll])
}

/// A range `num/den` either side of `sqrt_price` in √price: (⌊s·den/num⌋, ⌊s·num/den⌋). `(11, 10)` is ±10 % in √price, i.e.
/// `[p/1.21, 1.21·p]`, the range spike S1e measured passing both Jupiter tests at 10 SOL a side. `None` on overflow or an empty
/// range.
pub fn range_around(sqrt_price: u128, num: u128, den: u128) -> Option<(u128, u128)> {
    if num <= den || den == 0 {
        return None;
    }
    let lo = sqrt_price.checked_mul(den)? / num;
    let hi = sqrt_price.checked_mul(num)? / den;
    (lo >= damm::MIN_SQRT_PRICE && hi <= damm::MAX_SQRT_PRICE && lo < sqrt_price && sqrt_price < hi).then_some((lo, hi))
}

/// The coin hop of the site's buy.
pub enum CoinHop<'a> {
    /// Still on its DBC curve (quote = wX): `swap2` exact in.
    Curve(&'a dbc::Curve),
    /// Graduated: the DAMM v2 COIN/wX pool, `swap` exact in.
    Graduated(&'a damm::Pool),
}

/// SOL → wX → coin in one legacy transaction (spikes §3.5: 1,033 B and ≈ 136 k CU on the curve; 903 B and ≈ 88 k CU graduated).
/// Wraps `sol_in`, buys at least `wx_min` wX on the wX/SOL pool, spends exactly `wx_min` on the coin hop for at least `coin_min`,
/// and closes the wSOL account. Any wX above `wx_min` stays in the user's wX account (swept on a later buy; or use exact out).
pub fn two_hop_buy(user: Pubkey, wx_pool: &damm::Pool, wx_mint: Pubkey, hop: CoinHop, sol_in: u64, wx_min: u64, coin_min: u64, cu_limit: u32, cu_price: u64) -> Vec<Instruction> {
    let wsol = native_mint();
    let coin = match &hop {
        CoinHop::Curve(c) => c.base_mint,
        CoinHop::Graduated(p) => if p.b == wx_mint { p.a } else { p.b },
    };
    let (u_wsol, u_wx, u_coin) = (associated_token_address(&user, &wsol), associated_token_address(&user, &wx_mint), associated_token_address(&user, &coin));
    let coin_hop = match hop {
        CoinHop::Curve(c) => dbc::swap2(c, user, u_wx, u_coin, wx_min, coin_min, 0),
        CoinHop::Graduated(p) => damm::swap(p, user, u_wx, u_coin, wx_min, coin_min),
    };
    vec![
        soltx::compute_unit_limit(cu_limit),
        soltx::compute_unit_price(cu_price),
        spl::create_ata_idempotent(user, user, wsol),
        soltx::transfer(user, u_wsol, sol_in),
        spl::sync_native(u_wsol),
        spl::create_ata_idempotent(user, user, wx_mint),
        damm::swap(wx_pool, user, u_wsol, u_wx, sol_in, wx_min),
        spl::create_ata_idempotent(user, user, coin),
        coin_hop,
        spl::close_account(u_wsol, user, user),
    ]
}
