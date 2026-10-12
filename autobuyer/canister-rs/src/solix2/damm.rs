//! Meteora DAMM v2 (cp_amm 0.2.4, `cpamd…`), by hand from the official IDL: the wX/SOL pool the desk runs (§5) and the
//! migrated COIN/wX pools whose partner positions `K_fc` holds (§3.3).
use super::{damm_customizable_pool_address, damm_position_address, damm_position_nft_account, damm_vault_address, event_authority, id, ix, ro,
    rw, signer, DAMM_POOL_AUTHORITY, DAMM_PROGRAM, TOKEN_2022_PROGRAM};
use crate::soltx::{AccountMeta, Borsh, Instruction, Pubkey, SYSTEM_PROGRAM};

pub fn program() -> Pubkey {
    id(DAMM_PROGRAM)
}

/// `MIN_SQRT_PRICE` and `MAX_SQRT_PRICE` of cp_amm (a full-range position).
pub const MIN_SQRT_PRICE: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE: u128 = 79_226_673_521_066_979_257_578_248_091;
/// `collect_fee_mode` 1: fees in token b only (SOL on the wX/SOL pool).
pub const COLLECT_FEE_ONLY_B: u8 = 1;
pub const ACTIVATION_TIMESTAMP: u8 = 1;
/// Fee numerators are of 10⁹: 5,000,000 = 0.5 %.
pub const FEE_DENOMINATOR: u64 = 1_000_000_000;

/// A pool's addresses: token a and b (in the pool's order), their vaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    pub pool: Pubkey,
    pub a: Pubkey,
    pub b: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
}

impl Pool {
    pub fn at(pool: Pubkey, a: Pubkey, b: Pubkey) -> Pool {
        Pool { pool, a, b, vault_a: damm_vault_address(&a, &pool), vault_b: damm_vault_address(&b, &pool) }
    }
    /// The one customizable pool of the pair (`["cpool", max, min]`), with `a` and `b` in the order the creator gave them.
    pub fn customizable(a: Pubkey, b: Pubkey) -> Pool {
        Pool::at(damm_customizable_pool_address(&a, &b), a, b)
    }
    /// The COIN/wX pool DBC migrates a curve into (token a = the coin, token b = the quote).
    pub fn migrated(coin: Pubkey, quote: Pubkey) -> Pool {
        Pool::at(super::damm_config_pool_address(&id(super::DAMM_CONFIG_CUSTOMIZABLE), &coin, &quote), coin, quote)
    }
}

/// `BaseFeeParameters.data` (27 bytes) for a fixed fee: the cliff numerator, the scheduler fields zero.
pub fn fixed_base_fee(numerator: u64) -> [u8; 27] {
    let mut d = [0u8; 27];
    d[..8].copy_from_slice(&numerator.to_le_bytes());
    d
}

/// `InitializeCustomizablePoolParameters` (no dynamic fee, no alpha vault: Rook never uses either).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomizablePool {
    pub fee_numerator: u64,
    pub compounding_fee_bps: u16,
    pub sqrt_min_price: u128,
    pub sqrt_max_price: u128,
    pub liquidity: u128,
    pub sqrt_price: u128,
    pub activation_type: u8,
    pub collect_fee_mode: u8,
    pub activation_point: Option<u64>,
}

fn u128le(b: Borsh, v: u128) -> Borsh {
    b.bytes(&v.to_le_bytes())
}

/// `initialize_customizable_pool`: `payer` is also the authority over `payer_token_a/b` (DAMM v2 transfers with it), so the
/// token owner, `K_desk[i]`, pays the pool's rent (spikes §2.3: desk money). The position NFT mint signs (a throwaway key).
pub fn initialize_customizable_pool(p: &Pool, creator: Pubkey, payer: Pubkey, nft_mint: Pubkey, payer_token_a: Pubkey, payer_token_b: Pubkey, params: &CustomizablePool) -> Instruction {
    let d = program();
    let tp = super::token_program();
    let mut b = Borsh::ix("initialize_customizable_pool").bytes(&fixed_base_fee(params.fee_numerator)).u16(params.compounding_fee_bps).u8(0).u8(0);
    b = u128le(u128le(b, params.sqrt_min_price), params.sqrt_max_price).bool(false);
    b = u128le(u128le(b, params.liquidity), params.sqrt_price).u8(params.activation_type).u8(params.collect_fee_mode);
    b = match params.activation_point {
        None => b.u8(0),
        Some(t) => b.u8(1).u64(t),
    };
    ix(d, vec![ro(creator), signer(nft_mint, true), rw(damm_position_nft_account(&nft_mint)), signer(payer, true), ro(id(DAMM_POOL_AUTHORITY)), rw(p.pool),
        rw(damm_position_address(&nft_mint)), ro(p.a), ro(p.b), rw(p.vault_a), rw(p.vault_b), rw(payer_token_a), rw(payer_token_b), ro(tp), ro(tp),
        ro(id(TOKEN_2022_PROGRAM)), ro(SYSTEM_PROGRAM), ro(event_authority(&d)), ro(d)], b.done())
}

/// `initialize_pool_with_dynamic_config`: the same parameters on a pool of a dynamic config whose `pool_creator_authority` (here
/// `K_desk[i]`) signs. **The price range belongs to the pool in DAMM v2** (positions share it), and the customizable pool is one per
/// pair, so re-centring the range means a new pool, `["pool", config, max, min]`: one per (config, pair). Dynamic configs are
/// created by Meteora's operator (`create_dynamic_config`), so this path needs Meteora to issue configs naming `K_desk` (open
/// item S4). Checked against the IDL-generated client only (no such config exists to send it against).
pub fn initialize_pool_with_dynamic_config(config: Pubkey, a: Pubkey, b: Pubkey, creator: Pubkey, payer: Pubkey, pool_creator_authority: Pubkey, nft_mint: Pubkey,
    payer_token_a: Pubkey, payer_token_b: Pubkey, params: &CustomizablePool) -> (Pool, Instruction) {
    let d = program();
    let tp = super::token_program();
    let p = Pool::at(super::damm_config_pool_address(&config, &a, &b), a, b);
    let mut b_ = Borsh::ix("initialize_pool_with_dynamic_config").bytes(&fixed_base_fee(params.fee_numerator)).u16(params.compounding_fee_bps).u8(0).u8(0);
    b_ = u128le(u128le(b_, params.sqrt_min_price), params.sqrt_max_price).bool(false);
    b_ = u128le(u128le(b_, params.liquidity), params.sqrt_price).u8(params.activation_type).u8(params.collect_fee_mode);
    b_ = match params.activation_point {
        None => b_.u8(0),
        Some(t) => b_.u8(1).u64(t),
    };
    let i = ix(d, vec![ro(creator), signer(nft_mint, true), rw(damm_position_nft_account(&nft_mint)), signer(payer, true), signer(pool_creator_authority, false),
        ro(config), ro(id(DAMM_POOL_AUTHORITY)), rw(p.pool), rw(damm_position_address(&nft_mint)), ro(p.a), ro(p.b), rw(p.vault_a), rw(p.vault_b),
        rw(payer_token_a), rw(payer_token_b), ro(tp), ro(tp), ro(id(TOKEN_2022_PROGRAM)), ro(SYSTEM_PROGRAM), ro(event_authority(&d)), ro(d)], b_.done());
    (p, i)
}

/// `create_position`: an empty position on an existing pool, inside the pool's own range (a new position cannot move the range;
/// see [`initialize_pool_with_dynamic_config`]); its NFT mint signs.
pub fn create_position(p: &Pool, owner: Pubkey, payer: Pubkey, nft_mint: Pubkey) -> Instruction {
    let d = program();
    ix(d, vec![ro(owner), signer(nft_mint, true), rw(damm_position_nft_account(&nft_mint)), rw(p.pool), rw(damm_position_address(&nft_mint)),
        ro(id(DAMM_POOL_AUTHORITY)), signer(payer, true), ro(id(TOKEN_2022_PROGRAM)), ro(SYSTEM_PROGRAM), ro(event_authority(&d)), ro(d)],
        Borsh::ix("create_position").done())
}

fn position_accounts(p: &Pool, with_authority: bool, nft_mint: Pubkey, owner: Pubkey, token_a: Pubkey, token_b: Pubkey, pool_writable: bool) -> Vec<AccountMeta> {
    let d = program();
    let tp = super::token_program();
    let mut v = Vec::with_capacity(15);
    if with_authority {
        v.push(ro(id(DAMM_POOL_AUTHORITY)));
    }
    v.push(if pool_writable { rw(p.pool) } else { ro(p.pool) });
    v.extend([rw(damm_position_address(&nft_mint)), rw(token_a), rw(token_b), rw(p.vault_a), rw(p.vault_b), ro(p.a), ro(p.b),
        ro(damm_position_nft_account(&nft_mint)), signer(owner, false), ro(tp), ro(tp), ro(event_authority(&d)), ro(d)]);
    v
}

/// `add_liquidity { liquidity_delta, token_a_amount_threshold, token_b_amount_threshold }` (the thresholds are maxima in).
pub fn add_liquidity(p: &Pool, nft_mint: Pubkey, owner: Pubkey, token_a: Pubkey, token_b: Pubkey, liquidity_delta: u128, max_a: u64, max_b: u64) -> Instruction {
    let b = u128le(Borsh::ix("add_liquidity"), liquidity_delta).u64(max_a).u64(max_b);
    ix(program(), position_accounts(p, false, nft_mint, owner, token_a, token_b, true), b.done())
}

/// `remove_liquidity { liquidity_delta, token_a_amount_threshold, token_b_amount_threshold }` (the thresholds are minima out).
pub fn remove_liquidity(p: &Pool, nft_mint: Pubkey, owner: Pubkey, token_a: Pubkey, token_b: Pubkey, liquidity_delta: u128, min_a: u64, min_b: u64) -> Instruction {
    let b = u128le(Borsh::ix("remove_liquidity"), liquidity_delta).u64(min_a).u64(min_b);
    ix(program(), position_accounts(p, true, nft_mint, owner, token_a, token_b, true), b.done())
}

/// `remove_all_liquidity(token_a_amount_threshold, token_b_amount_threshold)`: the position's whole unlocked liquidity.
pub fn remove_all_liquidity(p: &Pool, nft_mint: Pubkey, owner: Pubkey, token_a: Pubkey, token_b: Pubkey, min_a: u64, min_b: u64) -> Instruction {
    ix(program(), position_accounts(p, true, nft_mint, owner, token_a, token_b, true), Borsh::ix("remove_all_liquidity").u64(min_a).u64(min_b).done())
}

/// `claim_position_fee`: a position's fees (the pool is read-only here).
pub fn claim_position_fee(p: &Pool, nft_mint: Pubkey, owner: Pubkey, token_a: Pubkey, token_b: Pubkey) -> Instruction {
    ix(program(), position_accounts(p, true, nft_mint, owner, token_a, token_b, false), Borsh::ix("claim_position_fee").done())
}

/// `close_position`: an empty position; its rent goes to `rent_receiver` (the position NFT is Token-2022).
pub fn close_position(p: &Pool, nft_mint: Pubkey, owner: Pubkey, rent_receiver: Pubkey) -> Instruction {
    let d = program();
    ix(d, vec![rw(nft_mint), rw(damm_position_nft_account(&nft_mint)), rw(p.pool), rw(damm_position_address(&nft_mint)), ro(id(DAMM_POOL_AUTHORITY)),
        rw(rent_receiver), signer(owner, false), ro(id(TOKEN_2022_PROGRAM)), ro(event_authority(&d)), ro(d)], Borsh::ix("close_position").done())
}

fn swap_accounts(p: &Pool, payer: Pubkey, input: Pubkey, output: Pubkey) -> Vec<AccountMeta> {
    let d = program();
    let tp = super::token_program();
    vec![ro(id(DAMM_POOL_AUTHORITY)), rw(p.pool), rw(input), rw(output), rw(p.vault_a), rw(p.vault_b), ro(p.a), ro(p.b), signer(payer, false), ro(tp), ro(tp),
        ro(d), ro(event_authority(&d)), ro(d)]
}

/// `swap { amount_in, minimum_amount_out }`: direction follows `input` / `output`.
pub fn swap(p: &Pool, payer: Pubkey, input: Pubkey, output: Pubkey, amount_in: u64, minimum_amount_out: u64) -> Instruction {
    ix(program(), swap_accounts(p, payer, input, output), Borsh::ix("swap").u64(amount_in).u64(minimum_amount_out).done())
}

/// `swap2 { amount_0, amount_1, swap_mode }`: 0 exact in, 1 partial fill, 2 exact out (`amount_0` out, `amount_1` maximum in;
/// the site's hop 1 in exact-out leaves no wX dust, spikes §3.5).
pub fn swap2(p: &Pool, payer: Pubkey, input: Pubkey, output: Pubkey, amount_0: u64, amount_1: u64, swap_mode: u8) -> Instruction {
    ix(program(), swap_accounts(p, payer, input, output), Borsh::ix("swap2").u64(amount_0).u64(amount_1).u8(swap_mode).done())
}
