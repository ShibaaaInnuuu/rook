//! Coin redemption (W6 d; §7): a holder sends coins to a per-intent address (`K_int[n]`, the desk's `Kind::RedeemCoin`); the canister burns
//! them and pays the pro-rata share of the coin's vault in wX, rounded DOWN (rook_backing's `payout_never_overpays` property):
//!
//! `payout = ⌊V · a / S⌋`, with `V` the vault's wX balance, `a` the coins in the deposit account and `S` the coin mint's live supply, read at
//! finalized immediately before signing and still including the `a` coins (§7). The coins locked in the migrated pool are never burned, so
//! their share stays in the vault for the coins still out; the leftovers were burned at graduation, which raised every coin's backing per
//! unit. [`CoinsState::exclude_from_supply`] can leave accounts out of `S` (empty by default: see the module notes of the "as built" section).
//!
//! One redemption at a time on the crank line, so `V` and `S` are fresh for each. A redemption is a link (a memo `rook:v1:<coll>:coin:<id>:i<n>`
//! for the ledger) that changes no collection's money (no supply of wX, no lamports at `M[i]`): its `pre_sign` is the memo and the idempotence.
//! Only a backed coin (migration fee in the vault, leftovers burned) redeems: before that `S` would still hold the leftovers.
use crate::chainlink::driver::DeskIo;
use crate::chainlink::ledger::{Effects, OpKind, Scope, Spec};
use crate::desk::state as desk;
use crate::solix2::decode::{Mint, TokenAccount};
use crate::solkey::KeyPath;
use crate::soltx::{associated_token_address, Pubkey};

use super::build;
use super::state::*;
use super::step::{CoinStep, Planned};

/// A request for a coin that is not backed yet waits this long, then fails (the desk returns the coins).
pub const BACKED_WAIT_SECS: u64 = 6 * 3_600;
/// Attempts at one redemption before it fails.
pub const MAX_TRIES: u32 = 5;

/// `⌊V · a / S⌋` in u128. `None` when `S` is 0 or `a` exceeds `S`.
pub fn payout(vault: u64, supply: u64, amount: u64) -> Option<u64> {
    if supply == 0 || amount > supply {
        return None;
    }
    u64::try_from(vault as u128 * amount as u128 / supply as u128).ok()
}

/// What a redemption of `amount` coins would pay now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quote {
    pub vault: u64,
    pub supply: u64,
    pub amount: u64,
    pub payout: u64,
}

/// A redemption request (the desk calls this when its intent `n` holds coins): `recipient` is who is paid. Idempotent per `n`.
pub fn request(n: u64, coin: Pubkey, recipient: Pubkey, now: u64) -> Result<(), String> {
    with_mut(|s| {
        if s.redeems.contains_key(&n) {
            return Ok(());
        }
        let Some(rec) = s.coins.get(&coin) else {
            return Err(format!("{} is not a Rook coin", crate::soltx::b58_encode(&coin)));
        };
        let coll = rec.coll;
        s.redeems.insert(n, Redeem { n, coin, coll, recipient, amount: 0, payout: 0, state: RedeemState::Requested, signature: String::new(), at: now, tries: 0 });
        Ok(())
    })
}

pub fn status(n: u64) -> Option<Redeem> {
    with(|s| s.redeems.get(&n).cloned())
}

fn fail(n: u64, why: String) {
    with_mut(|s| {
        if let Some(r) = s.redeems.get_mut(&n) {
            r.state = RedeemState::Failed(why);
        }
    });
}

/// What the chain shows for a redemption of intent `n`'s deposit: the vault, the live supply (less the excluded accounts), the deposit.
pub async fn quote_of<I: DeskIo>(io: &mut I, coin: Pubkey, vault_owner: Pubkey, wx: Pubkey, a_ata: Pubkey, exclude: &[Pubkey]) -> Result<Quote, String> {
    let vault = associated_token_address(&vault_owner, &wx);
    let mut keys = vec![vault, coin, a_ata];
    keys.extend(exclude.iter().copied());
    let r = io.sol_accounts(&keys, 165).await?;
    let v = r[0].as_ref().ok_or("the coin's vault does not exist")?;
    let vault_bal = TokenAccount::decode(&v.owner, &v.data)?.amount;
    let m = r[1].as_ref().ok_or("the coin mint does not exist")?;
    let mut supply = Mint::decode(&m.owner, &m.data)?.supply;
    let a = r[2].as_ref().ok_or("the deposit account does not exist")?;
    let amount = TokenAccount::decode(&a.owner, &a.data)?.amount;
    for x in &r[3..] {
        if let Some(x) = x {
            supply = supply.saturating_sub(TokenAccount::decode(&x.owner, &x.data)?.amount);
        }
    }
    let p = payout(vault_bal, supply, amount).ok_or_else(|| format!("the supply {supply} does not cover the {amount} coins in the deposit account"))?;
    Ok(Quote { vault: vault_bal, supply, amount, payout: p })
}

/// Plans the first requested redemption that can go: reads the chain, books the link, builds the transaction. Fails a request that can never
/// go (not a coin, nothing deposited, the share is zero, too many attempts).
pub async fn plan<I: DeskIo>(io: &mut I, now: u64) -> Result<Option<Planned>, String> {
    let open: Vec<Redeem> = with(|s| s.redeems.values().filter(|r| r.state == RedeemState::Requested).cloned().collect());
    for r in open {
        let Some(rec) = with(|s| s.coins.get(&r.coin).cloned()) else {
            fail(r.n, "not a Rook coin".into());
            continue;
        };
        if r.tries >= MAX_TRIES {
            fail(r.n, format!("{MAX_TRIES} attempts did not land"));
            continue;
        }
        if rec.stage != CoinStage::Backed {
            if now.saturating_sub(r.at) > BACKED_WAIT_SECS {
                fail(r.n, "the coin is not backed (its migration fee or its leftovers are not done)".into());
            }
            continue;
        }
        let wx = with(|s| s.cols[&rec.coll].wx_mint);
        let a = grad::addrs_of(io, &rec, wx).await?;
        let k_int = io.key_of(&KeyPath::Intent(r.n)).await?;
        let a_ata = associated_token_address(&k_int, &r.coin);
        let exclude = with(|s| s.exclude_from_supply.clone());
        let q = match quote_of(io, r.coin, a.vault_owner, wx, a_ata, &exclude).await {
            Ok(q) => q,
            Err(e) => {
                with_mut(|s| s.wait(format!("redeem:{}", r.n), e.clone(), now));
                continue;
            }
        };
        if q.amount == 0 {
            fail(r.n, "no coins in the deposit account".into());
            continue;
        }
        if q.payout == 0 {
            fail(r.n, format!("the share of {} coins in a vault of {} wX over a supply of {} rounds down to nothing", q.amount, q.vault, q.supply));
            continue;
        }
        let op = with_mut(|s| {
            s.ops += 1;
            s.ops
        });
        let spec = Spec { key: format!("coin:{}:{op}", r.n), kind: OpKind::Coin, coll: Some(rec.coll as u16), intent: Some(r.n), scope: Scope::None };
        let begun = desk::with_mut(|s| s.link.begin(&mut s.books, spec, now, |_| Ok((Effects::default(), q.payout))));
        let (link, _) = match begun {
            Ok(x) => x,
            Err(e) => {
                with_mut(|s| s.wait(format!("redeem:{}", r.n), format!("the link is refused: {e}"), now));
                continue;
            }
        };
        let memo = desk::with(|s| s.link.get(link).map(|l| l.memo.clone()).expect("just begun"));
        let recipient = r.recipient;
        let ixs = build::redeem_tx(&a, k_int, recipient, q.amount, q.payout, &memo);
        let step = CoinStep::Redeem { n: r.n, coin: r.coin, amount: q.amount, payout: q.payout, link };
        with_mut(|s| {
            if let Some(x) = s.redeems.get_mut(&r.n) {
                x.amount = q.amount;
                x.payout = q.payout;
                x.tries += 1;
            }
            s.receipts.push(Receipt { link, step: step.clone(), coll: rec.coll, to_treasury: 0, desk_lamports: 0, desk_units: 0, burned: 0, desk_burned: 0, at: now });
        });
        return Ok(Some(Planned { step, ixs, keys: build::redeem_keys(r.n, r.coin), link: Some(link) }));
    }
    Ok(None)
}

/// The books of the coins after a redemption landed.
pub fn apply_landed(n: u64, coin: &Pubkey, amount: u64, payout: u64, signature: &str) {
    with_mut(|s| {
        if let Some(c) = s.coins.get_mut(coin) {
            if let Some(v) = c.vault.as_mut() {
                v.coins_burned = v.coins_burned.saturating_add(amount);
                v.wx_paid = v.wx_paid.saturating_add(payout);
            }
        }
        if let Some(r) = s.redeems.get_mut(&n) {
            r.state = RedeemState::Landed;
            r.signature = signature.to_string();
        }
    });
}

/// A redemption whose transaction was undone or failed is asked again (until [`MAX_TRIES`]).
pub fn apply_undone(n: u64) {
    with_mut(|s| {
        if let Some(r) = s.redeems.get_mut(&n) {
            if matches!(r.state, RedeemState::Signed { .. }) {
                r.state = RedeemState::Requested;
            }
        }
    });
}

use super::grad;
