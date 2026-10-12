//! The coins' transactions (§3.3, §7, §9.1): pure builders over `solix2` and `soltx`. Each is signed by `K_main` (fee payer) and the keys
//! that own what it moves (`K_fc` for the partner's claims, `K_coin[c]` for a vault, `K_int[n]` for a redemption's deposit, `K_coll[i]` where
//! `M[i]` pays). The keys of a builder are listed by its `*_keys` twin; every Solana address a builder names is an argument, nothing is read.
use crate::solix2::dbc::Curve;
use crate::solix2::{damm, dbc, spl};
use crate::solkey::KeyPath;
use crate::soltx::{associated_token_address, transfer, Instruction, Pubkey};

/// The addresses of one coin's operations.
#[derive(Clone, Copy, Debug)]
pub struct Addrs {
    pub main: Pubkey,
    /// `K_fc`: the claimer, the leftover receiver, the owner of the partner's LP position.
    pub fc: Pubkey,
    /// `K_coin[c]`: the owner of the coin's vault.
    pub vault_owner: Pubkey,
    pub wx: Pubkey,
    pub coin: Pubkey,
}

impl Addrs {
    pub fn vault(&self) -> Pubkey {
        associated_token_address(&self.vault_owner, &self.wx)
    }
    pub fn fc_wx(&self) -> Pubkey {
        associated_token_address(&self.fc, &self.wx)
    }
    pub fn fc_coin(&self) -> Pubkey {
        associated_token_address(&self.fc, &self.coin)
    }
}

/// `migration_damm_v2` by anyone: the payer and the two new position NFT mints (derived keys, so they sign) are the signers. The
/// second NFT is the PARTNER's position (`K_fc`'s: checked against the landed spike flow).
pub fn migrate_tx(curve: &Curve, payer: Pubkey, first_nft: Pubkey, second_nft: Pubkey) -> Vec<Instruction> {
    vec![dbc::migration_damm_v2(curve, payer, first_nft, second_nft)]
}
pub fn migrate_keys(seq: u64) -> Vec<KeyPath> {
    vec![KeyPath::PositionNft(2 * seq), KeyPath::PositionNft(2 * seq + 1)]
}

/// The partner's half of the raise (wX) from the curve straight into the coin's vault: `K_coin[c]`'s wX account is made first (the vault
/// is an ATA, so its address is known before it exists). Signers: `K_main`, `K_fc`.
pub fn migration_fee_tx(a: &Addrs, curve: &Curve) -> Vec<Instruction> {
    vec![spl::create_ata_idempotent(a.main, a.vault_owner, a.wx), dbc::withdraw_migration_fee(curve, a.fc, a.vault(), 0)]
}
pub fn fc_keys() -> Vec<KeyPath> {
    vec![KeyPath::Claimer]
}

/// The partner's surplus (the quote above the threshold the last buy left) to `K_fc`'s wX account. Signers: `K_main`, `K_fc`.
pub fn surplus_tx(a: &Addrs, curve: &Curve) -> Vec<Instruction> {
    vec![spl::create_ata_idempotent(a.main, a.fc, a.wx), dbc::partner_withdraw_surplus(curve, a.fc, a.fc_wx())]
}

/// `withdraw_leftover` (permissionless) after `K_fc`'s coin account exists (DBC requires it). Signer: `K_main`.
pub fn leftover_tx(a: &Addrs, curve: &Curve) -> Vec<Instruction> {
    vec![spl::create_ata_idempotent(a.main, a.fc, a.coin), dbc::withdraw_leftover(curve, a.fc)]
}

/// Burns `amount` coins `K_fc` holds (§3.3: the leftovers are burned, raising every coin's backing per unit). Coins have 6 decimals.
/// Signers: `K_main`, `K_fc`.
pub fn burn_coins_tx(a: &Addrs, amount: u64) -> Vec<Instruction> {
    vec![spl::burn_checked(a.fc_coin(), a.coin, a.fc, amount, crate::solix2::dbc::pins::COIN_DECIMALS)]
}

/// The curve's partner fees: wX to `K_fc`'s wX account (and base fees, none under a quote-fee config, to its coin account).
pub fn claim_curve_fee_tx(a: &Addrs, curve: &Curve) -> Vec<Instruction> {
    vec![
        spl::create_ata_idempotent(a.main, a.fc, a.wx),
        spl::create_ata_idempotent(a.main, a.fc, a.coin),
        dbc::claim_trading_fee(curve, a.fc, a.fc_coin(), a.fc_wx(), u64::MAX, u64::MAX),
    ]
}

/// The migrated pool's LP fees on the partner position NFT `K_fc` holds. Signers: `K_main`, `K_fc`.
pub fn claim_lp_fee_tx(a: &Addrs, nft: Pubkey) -> Vec<Instruction> {
    let pool = damm::Pool::migrated(a.coin, a.wx);
    vec![
        spl::create_ata_idempotent(a.main, a.fc, a.wx),
        spl::create_ata_idempotent(a.main, a.fc, a.coin),
        damm::claim_position_fee(&pool, nft, a.fc, a.fc_coin(), a.fc_wx()),
    ]
}

/// Fee wX of a collection (§9.1, §5.3): `to_desk` wX moved to `K_desk[i]`'s wX account (made first), `burn` burned from `K_fc`'s wX account
/// at the module, the `paid` lamports leaving `M[i]` split between the treasury `K_tre` and (`desk_lamports`, the desk's share of the burn)
/// `K_desk[i]`, the link's memo. Signers: `K_main`, `K_fc`, `K_coll[i]`.
#[allow(clippy::too_many_arguments)]
pub fn cash_fees_tx(a: &Addrs, k_coll: Pubkey, k_tre: Pubkey, k_desk: Pubkey, to_desk: u64, burn: u64, to_treasury: u64, desk_lamports: u64, memo: &str) -> Vec<Instruction> {
    let mut v = vec![];
    if to_desk > 0 {
        v.push(spl::create_ata_idempotent(a.main, k_desk, a.wx));
        v.push(spl::transfer_checked(a.fc_wx(), a.wx, associated_token_address(&k_desk, &a.wx), a.fc, to_desk, 9));
    }
    v.push(spl::burn_checked(a.fc_wx(), a.wx, a.fc, burn, 9));
    if to_treasury > 0 {
        v.push(transfer(k_coll, k_tre, to_treasury));
    }
    if desk_lamports > 0 {
        v.push(transfer(k_coll, k_desk, desk_lamports));
    }
    v.push(spl::memo(memo, &[]));
    v
}
pub fn cash_fees_keys(coll: u32) -> Vec<KeyPath> {
    vec![KeyPath::Claimer, KeyPath::Coll(coll)]
}

/// A coin redemption (§7): the coins in `K_int[n]`'s coin account burned, `payout` wX from the vault to the recipient's wX account (made
/// first, paid by `K_main`), the emptied coin account closed (its rent to `K_main`, which paid the recipient's), the link's memo.
/// Signers: `K_main`, `K_int[n]`, `K_coin[c]`.
pub fn redeem_tx(a: &Addrs, k_int: Pubkey, recipient: Pubkey, amount: u64, payout: u64, memo: &str) -> Vec<Instruction> {
    let a_coin = associated_token_address(&k_int, &a.coin);
    vec![
        spl::create_ata_idempotent(a.main, recipient, a.wx),
        spl::burn_checked(a_coin, a.coin, k_int, amount, crate::solix2::dbc::pins::COIN_DECIMALS),
        spl::transfer_checked(a.vault(), a.wx, associated_token_address(&recipient, &a.wx), a.vault_owner, payout, 9),
        spl::close_account(a_coin, a.main, k_int),
        spl::memo(memo, &[]),
    ]
}
pub fn redeem_keys(n: u64, coin: Pubkey) -> Vec<KeyPath> {
    vec![KeyPath::Intent(n), KeyPath::Coin(coin)]
}
