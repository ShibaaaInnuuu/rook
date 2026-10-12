//! Fee sweeps (W6 e; §3.3, §5.3, §9.1): the partner's curve fees and the migrated pools' LP fees are claimed to `K_fc`; the fee wX there are
//! split and cashed:
//!
//! - a share `q_pool` (25 %) goes to the collection's pool desk while its pool is under its target depth (`pooldesk::wants_fees`): half as
//!   wX (already backed units, moved to `K_desk[i]`), the other half burned at the module for SOL to `K_desk[i]`;
//! - the rest is burned at the module (the public burn, a [`chainlink`](crate::chainlink) link: booked, recorded, gated by `pre_sign`)
//!   for SOL to the treasury `K_tre`, booked as treasury income when the link lands (§9.1). Cashing raises nobody's backing and lowers
//!   nobody's: it is an ordinary burn. It waits (the parked D-j bound) while the burn would pay under 95 % of the accepted floor.
//!
//! Anything in `K_fc`'s wX account is cashed (a fee, a surplus, an unsolicited transfer: §4.4 "unbooked wX are swept").
use crate::books::ONE;
use crate::chainlink::driver::DeskIo;
use crate::chainlink::ledger::{Effects, OpKind, Scope, Spec};
use crate::desk::state as desk;
use crate::solix2::decode::{DammPool, DammPosition, DbcPool, TokenAccount, DAMM_POSITION_LEN, DBC_POOL_LEN};
use crate::solix2::{self, damm};
use crate::solkey::KeyPath;
use crate::soltx::{associated_token_address, b58_encode, Pubkey};

use super::build::{self, Addrs};
use super::grad::{self, curve_of, lp_fee_units};
use super::state::*;
use super::step::{CoinStep, Planned};

/// Token accounts of `K_fc` listed by one read of its position NFTs, at most.
pub const POSITIONS_LISTED: usize = 1_024;
/// The least a claim or a cash moves, in lamports of value (§3.3: a transaction costs ≈ 0.000255 SOL all in).
pub const MIN_FEE_LAMPORTS: u64 = 50_000_000;
/// The share of a collection's fee wX the pool desk gets while its pool is under target (§5.3, O-3), in bps.
pub const Q_POOL_BPS: u64 = 2_500;
/// A cash waits when the burn would pay under this share of the accepted floor (D-j's bound), in bps.
pub const CASH_FLOOR_BPS: u64 = 9_500;
/// How often each scan runs when it found nothing.
pub const CASH_EVERY: u64 = 3_600;
pub const CURVE_EVERY: u64 = 6 * 3_600;
pub const LP_EVERY: u64 = 24 * 3_600;

/// The least wX (base units) worth a transaction at an accepted floor (lamports per whole wX): `MIN_FEE_LAMPORTS` of value, never under
/// 0.001 wX. A floor of 0 (none posted) is the dust bound alone.
pub fn min_units(accepted: u64) -> u64 {
    let dust = ONE / 1_000;
    if accepted == 0 {
        return dust;
    }
    let v = (MIN_FEE_LAMPORTS as u128 * ONE as u128).div_ceil(accepted as u128);
    u64::try_from(v).unwrap_or(u64::MAX).max(dust)
}

/// How a batch of fee wX is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    /// wX moved to the pool desk as wX.
    pub to_desk: u64,
    /// wX burned for SOL to the pool desk.
    pub desk_burn: u64,
    /// wX burned for SOL to the treasury.
    pub cash: u64,
}

impl Split {
    pub fn burned(&self) -> u64 {
        self.desk_burn + self.cash
    }
}

/// §5.3: while the desk wants depth, `q_pool` of the batch goes to it, half as wX and half burned for SOL; the rest is cashed.
pub fn split(balance: u64, desk_wants: bool) -> Split {
    if !desk_wants {
        return Split { to_desk: 0, desk_burn: 0, cash: balance };
    }
    let q = (balance as u128 * Q_POOL_BPS as u128 / 10_000) as u64;
    let to_desk = q / 2;
    let desk_burn = q - to_desk;
    Split { to_desk, desk_burn, cash: balance - q }
}

/// Splits `paid` lamports of one burn of `burned` units between the treasury and the desk (the desk's burned share, rounded down).
pub fn split_paid(paid: u64, sp: &Split) -> (u64, u64) {
    let n = sp.burned();
    if n == 0 {
        return (paid, 0);
    }
    let desk = (paid as u128 * sp.desk_burn as u128 / n as u128) as u64;
    (paid - desk, desk)
}

fn amount_of(a: &Option<crate::solrpc::Account>) -> Option<u64> {
    a.as_ref().and_then(|x| TokenAccount::decode(&x.owner, &x.data).ok()).map(|t| t.amount)
}

/// Plans one cash: the first open collection whose `K_fc` wX account holds at least [`min_units`], books the burn as a link (idempotent
/// by a fresh key), and returns the step. `Ok(None)`: nothing to cash (or it waits).
pub async fn plan_cash<I: DeskIo>(io: &mut I, now: u64) -> Result<Option<Planned>, String> {
    let cols: Vec<(u32, Pubkey)> = with(|s| s.cols.iter().filter(|(_, c)| c.stage == ColStage::Open).map(|(i, c)| (*i, c.wx_mint)).collect());
    if cols.is_empty() {
        return Ok(None);
    }
    let fc = io.key_of(&KeyPath::Claimer).await?;
    let atas: Vec<Pubkey> = cols.iter().map(|(_, wx)| associated_token_address(&fc, wx)).collect();
    let mut balances = vec![];
    for page in atas.chunks(crate::solrpc::MAX_KEYS) {
        balances.extend(io.sol_accounts(page, 165).await?);
    }
    for ((index, wx), acct) in cols.iter().zip(&balances) {
        let Some(bal) = amount_of(acct).filter(|b| *b > 0) else { continue };
        let key = format!("cash_fees:{index}");
        if with(|s| s.waiting(&key, now)) {
            continue;
        }
        let coll = *index as u16;
        let (accepted, fresh_open) = desk::with(|s| {
            s.books.get(coll).map(|c| (c.module.accepted, c.module.burns_open(now as i64, c.kd(s.books.keeper_paused)))).unwrap_or((0, false))
        });
        if bal < min_units(accepted) {
            continue;
        }
        if !fresh_open {
            with_mut(|s| s.wait(key.clone(), "the module's burns are closed (a stale floor with the poster up)".into(), now));
            continue;
        }
        let wants = crate::pooldesk::wants_fees(*index);
        let sp = split(bal, wants);
        // the bound: the burn must pay at least 95 % of the accepted floor per unit
        let quote = desk::with(|s| s.books.quote_burn(coll, sp.burned(), now as i64));
        let quoted = match quote {
            Ok(q) => q,
            Err(e) => {
                with_mut(|s| s.wait(key.clone(), format!("the burn is refused now: {e}"), now));
                continue;
            }
        };
        let floor_value = (accepted as u128 * sp.burned() as u128 / ONE as u128) as u64;
        if (quoted as u128) * 10_000 < floor_value as u128 * CASH_FLOOR_BPS as u128 {
            with_mut(|s| s.wait(key.clone(), format!("the burn would pay {quoted} lamports for {} units, under {CASH_FLOOR_BPS} bps of the accepted floor", sp.burned()), now));
            continue;
        }
        // book it first (§4.4 rule 1)
        let op = with_mut(|s| {
            s.ops += 1;
            s.ops
        });
        let spec = Spec { key: format!("cash:{index}:{op}"), kind: OpKind::Burn, coll: Some(coll), intent: None, scope: Scope::Module };
        let burn = sp.burned();
        let begun = desk::with_mut(|s| {
            s.link.begin(&mut s.books, spec, now, |b| {
                let r = b.burn(coll, burn, 0, now as i64)?;
                Ok((Effects { burn_units: r.units, lamports_out: r.paid, ..Default::default() }, r.paid))
            })
        });
        let (link, _) = match begun {
            Ok(x) => x,
            Err(e) => {
                with_mut(|s| s.wait(key.clone(), format!("the books refused the cash: {e}"), now));
                continue;
            }
        };
        let (paid, memo) = desk::with(|s| s.link.get(link).map(|l| (l.aux, l.memo.clone())).expect("just begun"));
        let (to_treasury, desk_lamports) = split_paid(paid, &sp);
        let a = Addrs { main: io.key_of(&KeyPath::Main).await?, fc, vault_owner: [0; 32], wx: *wx, coin: [0; 32] };
        let (k_coll, k_tre, k_desk) = (io.key_of(&KeyPath::Coll(*index)).await?, io.key_of(&KeyPath::Treasury).await?, io.key_of(&KeyPath::Desk(*index)).await?);
        let ixs = build::cash_fees_tx(&a, k_coll, k_tre, k_desk, sp.to_desk, burn, to_treasury, desk_lamports, &memo);
        with_mut(|s| s.receipts.push(Receipt { link, step: CoinStep::CashFees { coll: *index, burn, to_desk: sp.to_desk, link }, coll: *index,
            to_treasury, desk_lamports, desk_units: sp.to_desk, burned: burn, desk_burned: sp.desk_burn, at: now }));
        return Ok(Some(Planned { step: CoinStep::CashFees { coll: *index, burn, to_desk: sp.to_desk, link }, ixs, keys: build::cash_fees_keys(*index), link: Some(link) }));
    }
    Ok(None)
}

/// Plans one claim: the curve's partner fees of a coin whose DBC pool holds at least [`min_units`], or the LP fees of a migrated coin's
/// partner position. `curve`: the curve scan is due; `lp`: the LP scan is due.
pub async fn plan_claims<I: DeskIo>(io: &mut I, now: u64, curve: bool, lp: bool) -> Result<Option<Planned>, String> {
    let recs: Vec<CoinRec> = with(|s| s.coins.values().cloned().collect());
    if recs.is_empty() {
        return Ok(None);
    }
    if curve {
        let pools: Vec<Pubkey> = recs.iter().map(|r| r.pool).collect();
        let mut accts = vec![];
        for page in pools.chunks(crate::solrpc::MAX_KEYS) {
            accts.extend(io.sol_accounts(page, DBC_POOL_LEN).await?);
        }
        for (rec, a) in recs.iter().zip(accts) {
            let Some(a) = a else { continue };
            let Ok(pool) = DbcPool::decode(&a.owner, &a.data) else { continue };
            let (cfg, wx, accepted) = coin_ctx(rec);
            let units = grad::curve_fee_units(&pool);
            let step = CoinStep::ClaimCurveFee { coin: rec.coin, units };
            if units >= min_units(accepted) && !with(|s| s.waiting(&step.key(), now)) {
                let ad = grad::addrs_of(io, rec, wx).await?;
                return Ok(Some(Planned { ixs: build::claim_curve_fee_tx(&ad, &curve_of(&cfg, &rec.coin, &wx)), keys: build::fc_keys(), step, link: None }));
            }
        }
    }
    if lp {
        for rec in recs.iter().filter(|r| r.stage != CoinStage::Curve) {
            let (_, wx, accepted) = coin_ctx(rec);
            let Some(nft) = grad::find_partner_nft(io, rec, wx).await? else { continue };
            let pool_key = damm::Pool::migrated(rec.coin, wx).pool;
            let r = io.sol_accounts(&[pool_key, solix2::damm_position_address(&nft)], DAMM_POSITION_LEN.max(crate::solix2::decode::DAMM_POOL_LEN)).await?;
            let (Some(pa), Some(xa)) = (&r[0], &r[1]) else { continue };
            let (Ok(pool), Ok(pos)) = (DammPool::decode(&pa.owner, &pa.data), DammPosition::decode(&xa.owner, &xa.data)) else { continue };
            let units = u64::try_from(lp_fee_units(&pool, &pos)).unwrap_or(u64::MAX);
            let step = CoinStep::ClaimLpFee { coin: rec.coin, units };
            if units >= min_units(accepted) && !with(|s| s.waiting(&step.key(), now)) {
                let ad = grad::addrs_of(io, rec, wx).await?;
                return Ok(Some(Planned { ixs: build::claim_lp_fee_tx(&ad, nft), keys: build::fc_keys(), step, link: None }));
            }
        }
    }
    Ok(None)
}

fn coin_ctx(rec: &CoinRec) -> (Pubkey, Pubkey, u64) {
    let (cfg, wx) = with(|s| (s.cols[&rec.coll].config, s.cols[&rec.coll].wx_mint));
    let accepted = desk::with(|s| s.books.get(rec.coll as u16).map(|c| c.module.accepted).unwrap_or(0));
    (cfg, wx, accepted)
}

/// A landed claim: the fee book of its collection.
pub fn apply_claim(step: &CoinStep) {
    with_mut(|s| match step {
        CoinStep::ClaimCurveFee { coin, units } => {
            if let Some(c) = s.coins.get(coin).map(|c| c.coll) {
                s.fees.entry(c).or_default().curve_wx = s.fees.get(&c).map(|f| f.curve_wx).unwrap_or(0).saturating_add(*units);
            }
        }
        CoinStep::ClaimLpFee { coin, units } => {
            if let Some(c) = s.coins.get(coin).map(|c| c.coll) {
                let f = s.fees.entry(c).or_default();
                f.lp_wx = f.lp_wx.saturating_add(*units);
            }
        }
        _ => {}
    });
}

/// A landed cash: the treasury's income is booked, the fee book and the desk's credit move. Called once per receipt (the caller removes it).
pub fn apply_cash(r: &Receipt) {
    desk::with_mut(|s| {
        let _ = s.books.receive(r.to_treasury);
    });
    with_mut(|s| {
        let f = s.fees.entry(r.coll).or_default();
        f.cashed_wx = f.cashed_wx.saturating_add(r.burned.saturating_sub(r.desk_burned));
        f.to_desk_burned_wx = f.to_desk_burned_wx.saturating_add(r.desk_burned);
        f.cashed_lamports = f.cashed_lamports.saturating_add(r.to_treasury);
        f.to_desk_wx = f.to_desk_wx.saturating_add(r.desk_units);
        f.to_desk_lamports = f.to_desk_lamports.saturating_add(r.desk_lamports);
    });
    crate::pooldesk::note_fee_income(r.coll, r.desk_lamports, r.desk_units);
}

/// The wX base units a coin's DBC partner fee shows, for the readers.
pub fn describe(rec: &CoinRec) -> String {
    format!("coin {} of collection {}", b58_encode(&rec.coin), rec.coll)
}
