//! Discovery of Rook's coins, their graduation and their backing (W6 b, c; §3.3, §7): `getProgramAccounts` on DBC by config finds every pool
//! under a verified collection (a [`CoinRec`] and, once graduated, its vault book); a migrated curve is backed in a fixed order, each a step on
//! the crank line, each decided from what the chain shows now (level-triggered: a lost transaction or a step somebody else made needs no
//! record):
//!
//! 1. `migration_damm_v2`, if nobody has (permissionless; the canister's two position NFT mints sign);
//! 2. `withdraw_migration_fee`: the partner's half of the raise, wX, straight into `K_coin[c]`'s account: that is the coin's vault, and its
//!    balance is the book's `wx_in`. No books change: those wX were minted by the public mint, and stay SOL-backed units wherever they sit
//!    (§3.2: "every wX inside a DBC curve, a pool or a coin vault is a SOL-backed unit until the canister has bought NFTs for it");
//! 3. `withdraw_leftover` (permissionless) to `K_fc`'s coin account, then burn them (§3.3: raises every coin's backing per unit);
//! 4. the partner's surplus (wX above the threshold), and the curve's and the pool's fees (`fees`).
use crate::chainlink::driver::DeskIo;
use crate::solix2::decode::{DammPool, DammPosition, DbcPool, TokenAccount, DBC_POOL_LEN};
use crate::solix2::dbc::{self, Curve};
use crate::solix2::{self, damm};
use crate::solkey::KeyPath;
use crate::solrpc::MAX_KEYS;
use crate::soltx::{associated_token_address, b58_encode, Pubkey};

use super::build::{self, Addrs};
use super::state::*;
use super::step::{CoinStep, Planned};

/// DBC pools read by one discovery of one collection, at most (`solhome::POOLS_READ`): more is refused, loud.
pub const POOLS_PER_COLLECTION: usize = 1_000;
/// Discovery of new pools, and the graduation check of the coins still on a curve, every this many seconds.
pub const DISCOVER_EVERY: u64 = 900;
/// A surplus below this (wX base units, 0.001 wX) is not worth a transaction: marked done.
pub const MIN_SURPLUS_UNITS: u64 = 1_000_000;
/// Attempts at a step the simulation refuses before the coin's step is parked for the operator.
pub const MAX_REFUSED_TRIES: u32 = 3;

/// The coin's DBC pool, as a curve's addresses.
pub fn curve_of(config: &Pubkey, coin: &Pubkey, wx: &Pubkey) -> Curve {
    Curve::new(*config, *coin, *wx)
}

/// What the graduation of one coin needs next, from what the chain shows (pure).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// The curve completed and nobody ran the migration.
    Migrate,
    /// The partner's migration fee is not in the vault.
    MigrationFee,
    /// The curve's unsold coins are not withdrawn.
    Leftover,
    /// `K_fc` holds coins (the leftovers, or base fees): burn them.
    BurnCoins(u64),
    /// Wx above the threshold is the partner's.
    Surplus,
    /// The coin is backed.
    Backed,
    /// Still on its curve.
    NotYet,
}

/// What [`next_step`] reads.
pub struct View<'a> {
    pub rec: &'a CoinRec,
    pub pool: &'a DbcPool,
    /// The collection's `T_wX` in wX base units (the threshold the curve raises).
    pub threshold: u64,
    /// `K_coin[c]`'s wX account balance (`None`: it does not exist).
    pub vault: Option<u64>,
    /// `K_fc`'s coin account balance (`None`: it does not exist).
    pub fc_coin: Option<u64>,
}

/// The next step of a coin's graduation. Order (§3.3): migrate, the migration fee, the leftovers, their burn, the surplus.
pub fn next_step(v: &View) -> Next {
    let p = v.pool;
    if !p.is_migrated {
        // 0 pre-bonding-curve, 1 and 2 the curve is complete (2 with no vesting, as the spike flow showed), 3 created
        return if p.migration_progress >= 1 { Next::Migrate } else { Next::NotYet };
    }
    // the pool's own flag (bit 1: the partner's migration fee is withdrawn), never the vault's balance: anyone can send wX to the vault's
    // address, and a balance there must not skip the withdrawal (the coin would keep a dust backing)
    let fee_in = v.pool.migration_fee_withdraw_status & 0b10 != 0;
    if !fee_in {
        return Next::MigrationFee;
    }
    if !p.is_withdraw_leftover {
        return Next::Leftover;
    }
    if let Some(b) = v.fc_coin.filter(|b| *b > 0) {
        return Next::BurnCoins(b);
    }
    let surplus = p.quote_reserve.saturating_sub(v.threshold);
    if !p.is_partner_withdraw_surplus && surplus >= MIN_SURPLUS_UNITS && v.rec.surplus_tries < 3 {
        return Next::Surplus;
    }
    Next::Backed
}

/// Fee units a claim from the curve would take: the partner's unclaimed quote fee.
pub fn curve_fee_units(p: &DbcPool) -> u64 {
    p.partner_quote_fee
}

fn u256_sub_sat(a: &[u8; 32], b: &[u8; 32]) -> u128 {
    // little-endian U256 difference, saturated to u128 (an accumulator never goes back; a delta past 2^128 is just "a lot")
    let (mut lo_a, mut hi_a, mut lo_b, mut hi_b) = ([0u8; 16], [0u8; 16], [0u8; 16], [0u8; 16]);
    lo_a.copy_from_slice(&a[..16]);
    hi_a.copy_from_slice(&a[16..]);
    lo_b.copy_from_slice(&b[..16]);
    hi_b.copy_from_slice(&b[16..]);
    let (la, ha, lb, hb) = (u128::from_le_bytes(lo_a), u128::from_le_bytes(hi_a), u128::from_le_bytes(lo_b), u128::from_le_bytes(hi_b));
    if (ha, la) <= (hb, lb) {
        return 0;
    }
    let (d_lo, borrow) = la.overflowing_sub(lb);
    let d_hi = ha - hb - borrow as u128;
    if d_hi != 0 {
        u128::MAX
    } else {
        d_lo
    }
}

/// The wX (token b) a DAMM v2 position can claim now: `fee_b_pending + ⌊liquidity · (pool.fee_b_per_liquidity − checkpoint) / 2^128⌋`.
pub fn lp_fee_units(pool: &DammPool, pos: &DammPosition) -> u128 {
    let delta = u256_sub_sat(&pool.fee_b_per_liquidity, &pos.fee_b_per_token_checkpoint);
    let (l, d) = (pos.liquidity(), delta);
    // the high 128 bits of the 256-bit product
    let (a0, a1) = (l as u64 as u128, l >> 64);
    let (b0, b1) = (d as u64 as u128, d >> 64);
    let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    let mid = (p00 >> 64) + (p01 as u64 as u128) + (p10 as u64 as u128);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    hi.saturating_add(pos.fee_b_pending as u128)
}

/// `K_fc`'s addresses for coin `rec` and the transactions' accounts.
pub async fn addrs_of<I: DeskIo>(io: &mut I, rec: &CoinRec, wx: Pubkey) -> Result<Addrs, String> {
    Ok(Addrs { main: io.key_of(&KeyPath::Main).await?, fc: io.key_of(&KeyPath::Claimer).await?, vault_owner: io.key_of(&KeyPath::Coin(rec.coin)).await?, wx, coin: rec.coin })
}

/// Reads `keys` in pages of `MAX_KEYS`.
async fn read_paged<I: DeskIo>(io: &mut I, keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<crate::solrpc::Account>>, String> {
    let mut out = Vec::with_capacity(keys.len());
    for page in keys.chunks(MAX_KEYS) {
        out.extend(io.sol_accounts(page, max_data).await?);
    }
    Ok(out)
}

/// Finds the pools under every open collection's config that the books of the coins do not know yet (W6 b): a [`CoinRec`] each ("open_vault"
/// without a transaction: a vault book costs no rent, §3.3). Returns the log.
pub async fn discover<I: DeskIo>(io: &mut I, now: u64) -> Vec<String> {
    let mut log = vec![];
    let cols: Vec<(u32, Pubkey)> = with(|s| s.cols.iter().filter(|(_, c)| c.stage == ColStage::Open).map(|(i, c)| (*i, c.config)).collect());
    for (index, config) in cols {
        let listed = io.sol_program_slices(&dbc::program(), DBC_POOL_LEN, &[(72, config.to_vec())], (104, 64), POOLS_PER_COLLECTION + 1).await;
        let listed = match listed {
            Ok(l) => l,
            Err(e) => {
                log.push(format!("coins: discovery under collection {index} waits: {e}"));
                continue;
            }
        };
        if listed.len() > POOLS_PER_COLLECTION {
            log.push(format!("coins: collection {index} has over {POOLS_PER_COLLECTION} DBC pools: a sharded listing is needed (the first {POOLS_PER_COLLECTION} are read)"));
        }
        let fresh: Vec<Pubkey> = with(|s| {
            listed.iter().filter(|(a, o, _)| *o == dbc::program() && !s.coins.values().any(|c| c.pool == *a)).map(|(a, ..)| *a).take(POOLS_PER_COLLECTION).collect()
        });
        if fresh.is_empty() {
            continue;
        }
        let accts = match read_paged(io, &fresh, DBC_POOL_LEN).await {
            Ok(a) => a,
            Err(e) => {
                log.push(format!("coins: the new pools of collection {index} are not read: {e}"));
                continue;
            }
        };
        for (addr, a) in fresh.iter().zip(accts) {
            let Some(a) = a else { continue };
            let pool = match DbcPool::decode(&a.owner, &a.data) {
                Ok(p) => p,
                Err(e) => {
                    log.push(format!("coins: pool {} under collection {index} is not a DBC pool: {e}", b58_encode(addr)));
                    continue;
                }
            };
            if pool.config != config {
                continue;
            }
            with_mut(|s| {
                let seq = s.next_seq;
                s.next_seq += 1;
                s.coins.insert(pool.base_mint, CoinRec { coll: index, coin: pool.base_mint, pool: *addr, creator: pool.creator, launched_at: pool.activation_point, seq,
                    stage: if pool.is_migrated { CoinStage::Migrated } else { CoinStage::Curve }, vault: None, damm_pool: None, partner_nft: None, surplus_tries: 0 });
            });
            log.push(format!("coins: coin {} of collection {index} found (DBC pool {}, launched at {})", b58_encode(&pool.base_mint), b58_encode(addr), pool.activation_point));
        }
    }
    with_mut(|s| s.set_next("discover", now + DISCOVER_EVERY));
    log
}

/// The coins that still need graduation work: on a curve, or migrated and not yet backed.
fn pending_coins() -> Vec<CoinRec> {
    with(|s| s.coins.values().filter(|c| c.stage != CoinStage::Backed).cloned().collect())
}

/// Reads the coins in `recs` and returns the first graduation step (by [`next_step`]'s order across coins: a migration anywhere first, then
/// fees into vaults, then leftovers). Updates each [`CoinRec`] (stage, vault book, partner NFT) from what it read.
pub async fn plan_graduation<I: DeskIo>(io: &mut I, now: u64) -> Result<Option<Planned>, String> {
    let recs = pending_coins();
    if recs.is_empty() {
        return Ok(None);
    }
    let pools: Vec<Pubkey> = recs.iter().map(|r| r.pool).collect();
    let pool_accts = read_paged(io, &pools, DBC_POOL_LEN).await?;
    // the accounts of the migrated ones
    let mut best: Option<(u8, Planned)> = None;
    for (rec, pa) in recs.iter().zip(pool_accts) {
        let Some(pa) = pa else { continue };
        let pool = DbcPool::decode(&pa.owner, &pa.data)?;
        let (cfg, wx, threshold) = with(|s| {
            let c = &s.cols[&rec.coll];
            (c.config, c.wx_mint, s.pins[&rec.coll].t_wx.saturating_mul(dbc::pins::WX_ONE))
        });
        let a = addrs_of(io, rec, wx).await?;
        let (vault, fc_coin) = if pool.is_migrated {
            let r = io.sol_accounts(&[a.vault(), a.fc_coin()], 165).await?;
            let bal = |x: &Option<crate::solrpc::Account>| x.as_ref().and_then(|x| TokenAccount::decode(&x.owner, &x.data).ok()).map(|t| t.amount);
            (bal(&r[0]), bal(&r[1]))
        } else {
            (None, None)
        };
        // keep the record current
        with_mut(|s| {
            if let Some(c) = s.coins.get_mut(&rec.coin) {
                if pool.is_migrated && c.stage == CoinStage::Curve {
                    c.stage = CoinStage::Migrated;
                }
                if let Some(v) = vault.filter(|v| *v > 0) {
                    let vb = c.vault.get_or_insert_with(|| Vault { address: a.vault(), owner: a.vault_owner, ..Default::default() });
                    if vb.wx_in == 0 {
                        vb.wx_in = v;
                        vb.backed_at = 0;
                    }
                }
            }
        });
        let rec_now = with(|s| s.coins[&rec.coin].clone());
        let view = View { rec: &rec_now, pool: &pool, threshold, vault, fc_coin };
        let next = next_step(&view);
        let curve = curve_of(&cfg, &rec.coin, &wx);
        let (prio, step, ixs, keys) = match next {
            Next::NotYet => continue,
            Next::Backed => {
                with_mut(|s| {
                    if let Some(c) = s.coins.get_mut(&rec.coin) {
                        c.stage = CoinStage::Backed;
                        if let Some(v) = c.vault.as_mut() {
                            v.backed_at = now;
                        }
                    }
                    s.landed("backed", now, format!("coin {} is backed", b58_encode(&rec.coin)));
                });
                continue;
            }
            Next::Migrate => {
                let nfts = (io.key_of(&KeyPath::PositionNft(2 * rec.seq)).await?, io.key_of(&KeyPath::PositionNft(2 * rec.seq + 1)).await?);
                with_mut(|s| {
                    if let Some(c) = s.coins.get_mut(&rec.coin) {
                        c.partner_nft = Some(nfts.1);
                        c.damm_pool = Some(damm::Pool::migrated(rec.coin, wx).pool);
                    }
                });
                (0u8, CoinStep::Migrate { coin: rec.coin }, build::migrate_tx(&curve, a.main, nfts.0, nfts.1), build::migrate_keys(rec.seq))
            }
            Next::MigrationFee => (1, CoinStep::MigrationFee { coin: rec.coin }, build::migration_fee_tx(&a, &curve), build::fc_keys()),
            Next::Leftover => (2, CoinStep::Leftover { coin: rec.coin }, build::leftover_tx(&a, &curve), vec![]),
            Next::BurnCoins(n) => (3, CoinStep::BurnLeftover { coin: rec.coin, amount: n }, build::burn_coins_tx(&a, n), build::fc_keys()),
            Next::Surplus => (4, CoinStep::Surplus { coin: rec.coin }, build::surplus_tx(&a, &curve), build::fc_keys()),
        };
        if with(|s| s.waiting(&step.key(), now)) {
            continue;
        }
        if best.as_ref().is_none_or(|(p, _)| prio < *p) {
            best = Some((prio, Planned { step, ixs, keys, link: None }));
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// What a landed graduation step did to the books of the coins (called from `run::settle`).
pub fn apply_landed(step: &CoinStep, now: u64) {
    with_mut(|s| match step {
        CoinStep::Migrate { coin } => {
            if let Some(c) = s.coins.get_mut(coin) {
                c.stage = CoinStage::Migrated;
            }
        }
        CoinStep::Surplus { coin } => {
            if let Some(c) = s.coins.get_mut(coin) {
                c.surplus_tries = c.surplus_tries.saturating_add(1);
            }
        }
        CoinStep::BurnLeftover { coin, amount } => {
            if let Some(c) = s.coins.get_mut(coin) {
                if let Some(v) = c.vault.as_mut() {
                    v.coins_burned = v.coins_burned.saturating_add(*amount);
                }
            }
        }
        _ => {}
    });
    let _ = now;
}

/// The partner position NFT `K_fc` holds in the migrated pool of `rec`, when the canister did not run the migration itself: `K_fc`'s
/// Token-2022 accounts (the NFTs), each one's DAMM v2 position, the one whose pool is the coin's migrated pool.
pub async fn find_partner_nft<I: DeskIo>(io: &mut I, rec: &CoinRec, wx: Pubkey) -> Result<Option<Pubkey>, String> {
    if let Some(n) = rec.partner_nft {
        return Ok(Some(n));
    }
    let fc = io.key_of(&KeyPath::Claimer).await?;
    let pool = damm::Pool::migrated(rec.coin, wx).pool;
    let held = io.sol_token_accounts(&fc, &solix2::id(solix2::TOKEN_2022_PROGRAM), (0, 32), super::fees::POSITIONS_LISTED).await?;
    let nfts: Vec<Pubkey> = held.iter().filter_map(|(_, a)| (a.data.len() >= 32).then(|| a.data[..32].try_into().expect("32"))).collect();
    let positions: Vec<Pubkey> = nfts.iter().map(solix2::damm_position_address).collect();
    let accts = read_paged(io, &positions, crate::solix2::decode::DAMM_POSITION_LEN).await?;
    for (nft, a) in nfts.iter().zip(accts) {
        if let Some(a) = a {
            if DammPosition::decode(&a.owner, &a.data).is_ok_and(|p| p.pool == pool) {
                with_mut(|s| {
                    if let Some(c) = s.coins.get_mut(&rec.coin) {
                        c.partner_nft = Some(*nft);
                    }
                });
                return Ok(Some(*nft));
            }
        }
    }
    Ok(None)
}

/// The ATA of `owner` for `mint` (a convenience for the tests and the readers).
pub fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    associated_token_address(owner, mint)
}
