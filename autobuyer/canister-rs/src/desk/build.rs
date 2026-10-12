//! The desk's transactions (§6.1): pure builders over `solix2` and `soltx`. Each is signed by `K_main` (fee payer), the intent's key
//! `K_int[n]` and, where `M[i]` pays or wX is minted, `K_coll[i]` ([`Keys`] names them). Every transaction ends with the link's Memo.
use crate::solix2::spl;
use crate::solkey::KeyPath;
use crate::soltx::{associated_token_address, transfer, Instruction, Pubkey};

/// The addresses of one operation.
#[derive(Clone, Copy, Debug)]
pub struct Addrs {
    /// `K_main`: fee payer, and where the fee goes.
    pub main: Pubkey,
    /// `A = K_int[n]`.
    pub intent: Pubkey,
    /// `K_coll[i]`: the module address `M[i]` and the wX mint authority.
    pub module: Pubkey,
    /// The wX mint (or the coin mint of a coin redemption).
    pub mint: Pubkey,
}

/// A mint: `A` pays the new token account's rent (when it is created), `to_module` into `M[i]`, the fee to `K_main`, `units` wX to
/// the recipient's token account. `A` pays out exactly what it holds when `deposit = ata_rent + to_module + fee`.
pub fn mint_tx(a: &Addrs, recipient: Pubkey, units: u64, to_module: u64, fee: u64, create_ata: bool, memo: &str) -> Vec<Instruction> {
    let ata = associated_token_address(&recipient, &a.mint);
    let mut v = vec![];
    if create_ata {
        v.push(spl::create_ata_idempotent(a.intent, recipient, a.mint));
    }
    v.push(transfer(a.intent, a.module, to_module));
    v.push(transfer(a.intent, a.main, fee));
    v.push(spl::mint_to(a.mint, ata, a.module, units));
    v.push(spl::memo(memo, &[]));
    v
}

pub fn mint_keys(n: u64, coll: u16) -> Vec<KeyPath> {
    vec![KeyPath::Intent(n), KeyPath::Coll(coll as u32)]
}

/// A burn: the deposited `units` burned from `A`'s token account, `net` from `M[i]` to the recipient and the fee to `K_main`. Audit
/// B1-04: no `CloseAccount` here. It needs an empty account, and `units` is what was read before signing: one base unit sent to `A` by
/// anyone in between would fail the whole burn. The emptied account is closed later by [`close_tx`], a transaction of its own.
pub fn burn_tx(a: &Addrs, recipient: Pubkey, units: u64, net: u64, fee: u64, memo: &str) -> Vec<Instruction> {
    let ata = associated_token_address(&a.intent, &a.mint);
    vec![
        spl::burn(ata, a.mint, a.intent, units),
        transfer(a.module, recipient, net),
        transfer(a.module, a.main, fee),
        spl::memo(memo, &[]),
    ]
}

pub fn burn_keys(n: u64, coll: u16) -> Vec<KeyPath> {
    mint_keys(n, coll)
}

/// A redemption's burn of `units` (one wX) from `A`'s token account.
pub fn redeem_tx(a: &Addrs, units: u64, memo: &str) -> Vec<Instruction> {
    let ata = associated_token_address(&a.intent, &a.mint);
    vec![spl::burn(ata, a.mint, a.intent, units), spl::memo(memo, &[])]
}

pub fn redeem_keys(n: u64) -> Vec<KeyPath> {
    vec![KeyPath::Intent(n)]
}

/// A SOL deposit returned: `net` to `to`, the fee to `K_main` (`A` is emptied).
pub fn refund_sol_tx(a: &Addrs, to: Pubkey, net: u64, fee: u64, memo: &str) -> Vec<Instruction> {
    vec![transfer(a.intent, to, net), transfer(a.intent, a.main, fee), spl::memo(memo, &[])]
}

/// A token deposit returned: `units` to `to`'s token account (created by `K_main` when absent). Audit B1-04: no `CloseAccount` here
/// (dust sent to `A` between the read and the landing would fail the refund); [`close_tx`] closes `A`'s emptied account afterwards, to
/// `K_main`: its rent is still the token refund's fee (audit B1-02), and `run` lets `K_main` create the destination only while that close
/// has never failed for the intent.
pub fn refund_token_tx(a: &Addrs, to: Pubkey, units: u64, memo: &str) -> Vec<Instruction> {
    let from = associated_token_address(&a.intent, &a.mint);
    let dest = associated_token_address(&to, &a.mint);
    vec![spl::create_ata_idempotent(a.main, to, a.mint), spl::transfer(from, dest, a.intent, units), spl::memo(memo, &[])]
}

/// Audit B1-04: `A`'s token account, read empty at finalized, closed to `K_main` (a burn's or a token refund's last step, best effort:
/// its rent pays for this transaction and, for a refund, for the destination account `K_main` created). If a stranger's dust lands
/// first only this transaction fails; after [`super::intent::MAX_CLOSE_TRIES`] the account is left open and its rent stays at `A`.
pub fn close_tx(a: &Addrs, memo: &str) -> Vec<Instruction> {
    let ata = associated_token_address(&a.intent, &a.mint);
    vec![spl::close_account(ata, a.main, a.intent), spl::memo(memo, &[])]
}

pub fn refund_keys(n: u64) -> Vec<KeyPath> {
    vec![KeyPath::Intent(n)]
}
