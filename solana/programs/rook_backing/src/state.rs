use anchor_lang::prelude::*;

use crate::constants::MAX_CONFIGS;

/// Program-wide settings, PDA ["settings"]. Created once by the program's upgrade authority.
/// There is no stored admin: every admin instruction reads rook_core's `Registry` (at `registry`) and requires its `owner`
/// to sign, so a registry ownership change (propose_owner / accept_owner) carries over at once.
#[account]
#[derive(InitSpace)]
pub struct Settings {
    /// rook_core's registry PDA ["registry"], derived at init; its `owner` is rook_backing's admin.
    pub registry: Pubkey,
    /// rook_core's treasury PDA ["treasury"]; the only destination of `sweep_*`.
    pub treasury: Pubkey,
    pub bump: u8,
    /// This program's ["converter"] PDA (rook_core's converter for every collection) and its bump, derived at init so `back`
    /// checks it by address and signs without a `find_program_address`.
    pub converter: Pubkey,
    pub converter_bump: u8,
}

/// One per collection, PDA ["partner", listing]. Rook is the DBC "partner" of every config listed here.
#[account]
#[derive(InitSpace)]
pub struct Partner {
    /// rook_core `Listing` of the collection (the collection's identity).
    pub listing: Pubkey,
    /// rook_core accounts `convert` needs, pinned at registration so a cranker cannot substitute them.
    pub registry: Pubkey,
    pub module: Pubkey,
    pub module_vault: Pubkey,
    pub wrapper: Pubkey,
    pub wx_mint: Pubkey,
    /// The DBC `fee_claimer` of every config: PDA ["claimer", listing] of this program (system-owned, no data).
    pub claimer: Pubkey,
    pub claimer_bump: u8,
    pub bump: u8,
    /// The DBC config keys Rook created for this collection.
    #[max_len(MAX_CONFIGS)]
    pub configs: Vec<Pubkey>,
    /// Totals, for the indexer.
    pub vaults: u64,
    pub lamports_backed: u64,
    pub lamports_swept: u64,
}

/// One per coin, PDA ["vault", coin_mint].
#[account]
#[derive(InitSpace)]
pub struct Vault {
    pub partner: Pubkey,
    pub coin_mint: Pubkey,
    pub dbc_pool: Pubkey,
    pub dbc_config: Pubkey,
    /// The holder of DBC's (transferable) creator role when the coin was backed (`back`; `open_vault` writes the holder at
    /// opening until then). Not necessarily who launched the coin: DBC's `transfer_pool_creator` moves the role (audit A3-01).
    /// rook_core's wake `reclaim` credits this key.
    pub creator: Pubkey,
    /// The vault's wX token account, PDA ["vault_wx", coin_mint], authority = this vault.
    pub wx_account: Pubkey,
    pub graduated: bool,
    /// SOL withdrawn from DBC as the migration fee and converted.
    pub lamports_backed: u64,
    /// wX units credited by `back` (the account balance is the live figure; this is the historical one).
    pub wx_units: u64,
    /// The coin's mint supply when it was backed.
    pub coin_supply_at_backing: u64,
    pub backed_at: i64,
    /// When the coin launched: the DBC pool's own `activation_point` (unix seconds: Rook configs use timestamp activation,
    /// and DBC 0.2.1 sets it to the pool's creation time), copied at `open_vault`. rook_core's wake `reclaim` reads it.
    pub launched_at: i64,
    /// Redemption totals.
    pub coins_burned: u64,
    pub wx_paid: u64,
    pub bump: u8,
    pub wx_bump: u8,
}

/// The layout contract with rook_core (`rook_layouts::backing_vault`): rook_core's wake desk parses this account by those
/// offsets, so this test fails if a field of `Vault` moves.
#[cfg(test)]
mod tests {
    use super::*;
    use rook_layouts::backing_vault as L;

    fn k(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    #[test]
    fn vault_matches_the_shared_layout() {
        let v = Vault {
            partner: k(1),
            coin_mint: k(2),
            dbc_pool: k(3),
            dbc_config: k(4),
            creator: k(5),
            wx_account: k(6),
            graduated: true,
            lamports_backed: 0x0101_0101_0101_0107,
            wx_units: 0x0101_0101_0101_0108,
            coin_supply_at_backing: 0x0101_0101_0101_0109,
            backed_at: 1_791_504_123,
            launched_at: 1_791_500_007,
            coins_burned: 0x0101_0101_0101_010a,
            wx_paid: 0x0101_0101_0101_010b,
            bump: 0xfe,
            wx_bump: 0xfd,
        };
        let mut d = Vec::new();
        v.try_serialize(&mut d).unwrap();
        assert_eq!(d.len(), L::LEN);
        assert_eq!(d.len(), 8 + Vault::INIT_SPACE);
        assert_eq!(Vault::DISCRIMINATOR, &L::DISCRIMINATOR[..]);
        let got = L::parse(&d).expect("parses");
        assert_eq!(got.partner, k(1).to_bytes());
        assert_eq!(got.coin_mint, k(2).to_bytes());
        assert_eq!(got.creator, k(5).to_bytes());
        assert!(got.graduated);
        assert_eq!(got.backed_at, 1_791_504_123);
        assert_eq!(got.launched_at, 1_791_500_007);
        // every offset the crate names
        let u64_at = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().unwrap());
        assert_eq!(&d[L::DBC_POOL..L::DBC_POOL + 32], k(3).as_ref());
        assert_eq!(&d[L::DBC_CONFIG..L::DBC_CONFIG + 32], k(4).as_ref());
        assert_eq!(&d[L::WX_ACCOUNT..L::WX_ACCOUNT + 32], k(6).as_ref());
        assert_eq!(u64_at(L::LAMPORTS_BACKED), v.lamports_backed);
        assert_eq!(u64_at(L::WX_UNITS), v.wx_units);
        assert_eq!(u64_at(L::COIN_SUPPLY_AT_BACKING), v.coin_supply_at_backing);
        assert_eq!(u64_at(L::COINS_BURNED), v.coins_burned);
        assert_eq!(u64_at(L::WX_PAID), v.wx_paid);
        assert_eq!((d[L::BUMP], d[L::WX_BUMP]), (0xfe, 0xfd));
        // anything else is refused
        assert!(L::parse(&d[..L::LEN - 1]).is_none());
        let mut bad = d.clone();
        bad[0] ^= 1;
        assert!(L::parse(&bad).is_none());
    }
}
