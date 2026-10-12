use anchor_lang::prelude::*;

use crate::error::RookError;

pub const MAX_CONVERTERS: usize = 8;
pub const MAX_NAME: usize = 32;
pub const MAX_CUSTODY: usize = 64;
/// `ListingRegistry.CHANGE_DELAY`: roles and membership roots change only after seven public days.
pub const CHANGE_DELAY: i64 = 7 * crate::clock::DAY;

/// The registry (ListingRegistry + the guardian/keeper switches). PDA `["registry"]`.
/// rook_backing reads `owner` (its admin) by the shared layout `rook_layouts::registry`: keep `owner` the first field (the
/// test below fails otherwise).
/// `owner` is the Squads vault in production (§2.5); `keeper` is the posting EOA for Bitcoin listings and the prover of drand
/// rounds until Q4; `guardian` may only stop things.
#[account]
#[derive(InitSpace)]
pub struct Registry {
    pub owner: Pubkey,
    /// Ownable2Step: the proposed owner, who must accept (default: none).
    pub pending_owner: Pubkey,
    /// The keeper as named; it posts nothing while `keeper_paused`.
    pub keeper: Pubkey,
    pub pending_keeper: Pubkey,
    /// When the pending keeper may be accepted (0: none pending).
    pub pending_keeper_at: i64,
    pub guardian: Pubkey,
    pub keeper_paused: bool,
    pub keeper_resumed_at: i64,
    /// Program-derived signers allowed to call `convert` (rook_backing's PDA).
    pub converters: [Pubkey; MAX_CONVERTERS],
    pub converter_count: u8,
    /// A proposed converter and when it may be accepted (0: none pending; `CHANGE_DELAY`).
    pub pending_converter: Pubkey,
    pub pending_converter_at: i64,
    pub listing_count: u32,
    /// The Bitcoin network of every Inscriptions listing's custody address and redemption destinations, fixed at
    /// `init_registry` (the launch file names it: `Mainnet` in production; `Signet`/`Testnet4` only for a devnet rehearsal).
    pub bitcoin_network: BitcoinNetwork,
    pub bump: u8,
}

/// The Bitcoin network a registry serves (taproot prefixes: `bc1p…` on mainnet, `tb1p…` on testnet4 and signet, which share
/// their prefix). Default: mainnet.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub enum BitcoinNetwork {
    #[default]
    Mainnet,
    Testnet4,
    Signet,
}

impl BitcoinNetwork {
    /// The bech32 human-readable part of the network's addresses.
    pub fn hrp(self) -> &'static str {
        match self {
            BitcoinNetwork::Mainnet => "bc",
            BitcoinNetwork::Testnet4 | BitcoinNetwork::Signet => "tb",
        }
    }
}

impl Registry {
    /// `ListingRegistry.keeper()`: the named keeper, or none while paused.
    pub fn live_keeper(&self) -> Option<Pubkey> {
        if self.keeper_paused || self.keeper == Pubkey::default() {
            None
        } else {
            Some(self.keeper)
        }
    }

    pub fn require_owner(&self, who: &Pubkey) -> Result<()> {
        require_keys_eq!(*who, self.owner, RookError::NotOwner);
        Ok(())
    }

    /// `pauseKeeper` / `PosterSource.pause`: the owner, or a set guardian.
    pub fn require_owner_or_guardian(&self, who: &Pubkey) -> Result<()> {
        let g = self.guardian != Pubkey::default() && *who == self.guardian;
        require!(*who == self.owner || g, RookError::NotGuardian);
        Ok(())
    }

    pub fn is_converter(&self, who: &Pubkey) -> bool {
        self.converters[..self.converter_count as usize].contains(who)
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum ListingKind {
    /// An ERC-721 collection on EVM chain `chain_id` (never this chain), held at the canister's threshold-ECDSA address.
    EthNft { chain_id: u64, collection: [u8; 20] },
    /// A Bitcoin inscription collection: membership by keccak sorted-pair Merkle root over inscription ids.
    Inscriptions { membership_root: [u8; 32] },
    /// A Solana NFT collection (docs/v9/solana-nfts-design.md), appended 2026-10-10: the NFTs are held by the wrapper's escrow
    /// PDA `["escrow", wrapper]`, deposited and redeemed on chain. `collection` is the Token Metadata collection mint or the
    /// Core collection account; `standard` the collection's standard (a Token Metadata listing accepts both legacy and
    /// programmable members).
    SolNft { collection: Pubkey, standard: NftStandard },
}

/// A Solana NFT's standard (`ListingKind::SolNft`, `NftRecord`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum NftStandard {
    /// Token Metadata `NonFungible` (or the pre-standard `None`) on classic SPL Token.
    Legacy,
    /// Token Metadata `ProgrammableNonFungible` (pNFT): moved through Token Metadata `TransferV1` and its rule set.
    Programmable,
    /// Metaplex Core `AssetV1`.
    Core,
}

/// One collection (ListingRegistry listing + PosterSource + CollectionDirectory entry). PDA `["listing", id u32 le]`.
/// The collection's wrapper and floor module are PDAs of this account, so the directory is the derivation.
#[account]
#[derive(InitSpace)]
pub struct Listing {
    pub id: u32,
    pub kind: ListingKind,
    /// The registry owner's instant switch: stops wrapper mints and conversions (`setPaused`).
    pub paused: bool,
    /// The canister's Ed25519 key: the only minter of items, the recorder of deliveries, the taker of advances (and their
    /// payee). Immutable (EthNftWrapper.custodian).
    pub custodian: Pubkey,
    /// The floor poster for an EthNft listing (PosterSource.poster: the canister). Ignored for Inscriptions (the registry
    /// keeper posts). Immutable.
    pub poster: Pubkey,
    /// This collection's own floor switch (PosterSource.paused, D-M-1): owner or guardian sets, owner clears.
    pub poster_paused: bool,
    pub poster_resumed_at: i64,
    /// The custody address on the collection's chain: `0x…` for EthNft, a taproot `bc1p…` for Inscriptions.
    #[max_len(MAX_CUSTODY)]
    pub custody: String,
    /// EthNft: the parsed custody address (redemptions may not name it); zero for Inscriptions.
    pub custody_evm: [u8; 20],
    #[max_len(MAX_NAME)]
    pub name: String,
    /// A proposed membership root and when it may be accepted (0: none).
    pub pending_root: [u8; 32],
    pub pending_root_at: i64,
    pub bump: u8,
}

impl Listing {
    pub fn is_eth(&self) -> bool {
        matches!(self.kind, ListingKind::EthNft { .. })
    }

    /// A Solana NFT listing: (collection, standard).
    pub fn sol_nft(&self) -> Option<(Pubkey, NftStandard)> {
        match self.kind {
            ListingKind::SolNft { collection, standard } => Some((collection, standard)),
            _ => None,
        }
    }

    /// The floor module's `_keeperDown()`: for an EthNft or SolNft listing (PosterSource) the poster's switch or the registry's; for an
    /// Inscriptions listing this collection's poster switch or the registry's keeper switch (audit A1-01: `pause_poster` stops
    /// one Inscriptions collection without the global `pause_keeper`; drand proving reads `live_keeper` and is not stopped).
    /// True means no word of the poster prices anything.
    pub fn keeper_down(&self, reg: &Registry) -> bool {
        match self.kind {
            ListingKind::EthNft { .. } | ListingKind::SolNft { .. } => {
                reg.keeper_paused || self.poster_paused || self.poster == Pubkey::default()
            }
            ListingKind::Inscriptions { .. } => self.poster_paused || reg.live_keeper().is_none(),
        }
    }

    /// Who may `post_floor` now (None while paused).
    pub fn live_poster(&self, reg: &Registry) -> Option<Pubkey> {
        if self.keeper_down(reg) {
            return None;
        }
        match self.kind {
            ListingKind::EthNft { .. } | ListingKind::SolNft { .. } => Some(self.poster),
            ListingKind::Inscriptions { .. } => reg.live_keeper(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_owner_matches_the_shared_layout() {
        let owner = Pubkey::new_from_array([7; 32]);
        let r = Registry {
            owner,
            pending_owner: Pubkey::new_from_array([1; 32]),
            keeper: Pubkey::new_from_array([2; 32]),
            pending_keeper: Pubkey::new_from_array([3; 32]),
            pending_keeper_at: 4,
            guardian: Pubkey::new_from_array([5; 32]),
            keeper_paused: true,
            keeper_resumed_at: 6,
            converters: [Pubkey::new_from_array([8; 32]); MAX_CONVERTERS],
            converter_count: 1,
            pending_converter: Pubkey::new_from_array([9; 32]),
            pending_converter_at: 10,
            listing_count: 11,
            bitcoin_network: BitcoinNetwork::Signet,
            bump: 12,
        };
        let mut d = Vec::new();
        r.try_serialize(&mut d).unwrap();
        assert_eq!(d.len(), 8 + Registry::INIT_SPACE);
        assert_eq!(Registry::DISCRIMINATOR, &rook_layouts::registry::DISCRIMINATOR[..]);
        assert_eq!(rook_layouts::registry::owner(&d), Some(owner.to_bytes()));
        assert_eq!(
            Pubkey::find_program_address(&[rook_layouts::registry::SEED], &crate::ID).0,
            Pubkey::find_program_address(&[crate::seeds::REGISTRY], &crate::ID).0
        );
        d[0] ^= 1;
        assert_eq!(rook_layouts::registry::owner(&d), None);
    }
}
