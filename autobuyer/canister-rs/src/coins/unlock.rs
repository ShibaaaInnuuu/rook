//! The unlock order of the collections (docs/v9/canister-only-solana.md §16, operator decisions 2026-10-10 and 2026-10-12): five collections at
//! launch, then the canister opens the rest one by one as fees cover each (≈ 0.023 SOL of wX mint, metadata and DBC config).
//!
//! The list is the design's order, written out once and CHECKED against `deployments/launch/nft-pilot.json` (the 11 Ethereum collections, the
//! Bitcoin Frogs and the 14 inscription collections: slug, symbol, name) and `deployments/launch/solana-collections.json` (the 12 Solana
//! collections: key, symbol, name) by `tests/coins_unlock.rs`, so a rename in the launch files fails a test instead of opening the wrong
//! coin. A collection's index is its position here (`K_coll[i]`, the books' collection id), unless the launch file pins another.
//!
//! Operator decision 2026-10-12 ("we don't select, the people do"; "no picking"): there is no launch set. The ceremony PINS every
//! collection with a pipeline, the first `LAUNCH_COUNT` entries (the 11 Ethereum collections, Bitcoin Frogs, the 12 Solana collections
//! and the 14 Ordinals, live by default), and a collection OPENS when a creator first asks for it (`coins_request_open`), oldest request
//! first; the three Robinhood Chain collections are pinned after the ceremony, once its 4663 read-back is done (audit C4-03: the
//! default pins never write them, and the canister refuses to open one before chain 4663 is in `evm_chains`). A collection's `Source` says which launch file (and which chain) it
//! comes from, so the launch pins (`examples/launch_pins.rs`) read its floor and custody from the right place whatever its position. The
//! order below only matters under the operator's `coins_set_auto_open` switch (index order) and as the registry of indices.
use candid::CandidType;
use serde::{Deserialize, Serialize};

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// `nft-pilot.json`'s `collections[]`: an ERC-721 on Ethereum mainnet (the floor and tier in `graduation-tiers.json`).
    Ethereum,
    /// `nft-pilot.json`'s `bitcoin`: Bitcoin Frogs, the Inscriptions collection whose floor feed and buy pipeline exist (§16).
    Bitcoin,
    /// `solana-collections.json`, in file order: a Solana NFT collection (its floor and `threshold_wx` in that file).
    Solana,
    /// The Ordinals of `nft-pilot.json`'s `inscriptions[]`: OMB first (the first to pass the 20 % floor-feed gate), then the file's order.
    Ordinals,
    /// `robinhood-collections.json` (Robinhood Chain, `chainId` 4663): the ones `scripts/measure-robinhood-nfts.mjs` passed
    /// (2026-10-11). Pinned after the ceremony, once the 4663 `evm_chains` entry (Seaport 1.6's code hash) is read back.
    Robinhood,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub slug: &'static str,
    pub symbol: &'static str,
    pub name: &'static str,
    pub source: Source,
}

const fn e(slug: &'static str, symbol: &'static str, name: &'static str, source: Source) -> Entry {
    Entry { slug, symbol, name, source }
}

use Source::{Bitcoin as Btc, Ethereum as Eth, Ordinals as Ord, Robinhood, Solana as Sol};

/// The registry: BAYC, Milady, Mad Lads, SMB Gen2, Bitcoin Frogs, Pudgy Penguins, Azuki, Mutant Apes, Doodles, Lil Pudgys, Moonbirds,
/// CloneX, Checks VV, Identity MD, the ten other Solana collections in their file's order, then the Ordinals (OMB first): the 38
/// pinned at the ceremony; then the three Robinhood Chain collections, pinned after its 4663 read-back.
pub const ORDER: [Entry; 41] = [
    e("bayc", "wBOREDAPE", "Wrapped Bored Ape", Eth),
    e("milady", "wMILADY", "Wrapped Milady", Eth),
    e("mad_lads", "wMADLAD", "Wrapped Mad Lads", Sol),
    e("smb_gen2", "wSMBGEN2", "Wrapped SMB Gen2", Sol),
    e("bitcoin-frogs", "wBFROG", "Wrapped Bitcoin Frog", Btc),
    e("pudgypenguins", "wPUDGY", "Wrapped Pudgy Penguin", Eth),
    e("azuki", "wAZUKI", "Wrapped Azuki", Eth),
    e("mayc", "wMUTANTAPE", "Wrapped Mutant Ape", Eth),
    e("doodles", "wDOODLE", "Wrapped Doodle", Eth),
    e("lilpudgys", "wLILPUDGY", "Wrapped Lil Pudgy", Eth),
    e("moonbirds", "wMOONBIRD", "Wrapped Moonbird", Eth),
    e("clonex", "wCLONEX", "Wrapped CloneX", Eth),
    e("checks", "wCHECKSVV", "Wrapped Checks VV", Eth),
    e("identitymd", "wIDENTITY", "Wrapped Identity MD", Eth),
    e("claynosaurz", "wCLAYNO", "Wrapped Claynosaurz", Sol),
    e("famous_fox_federation", "wFFF", "Wrapped Famous Fox Federation", Sol),
    e("degods", "wDEGOD", "Wrapped DeGods (Solana)", Sol),
    e("galactic_geckos", "wGECKO", "Wrapped Galactic Geckos", Sol),
    e("taiyo_robotics", "wTAIYO", "Wrapped Taiyo Robotics", Sol),
    e("okay_bears", "wOKAYBEAR", "Wrapped Okay Bears", Sol),
    e("smb_gen3", "wSMBGEN3", "Wrapped SMB Gen3", Sol),
    e("transdimensional_fox_federation", "wTFF", "Wrapped Transdim Fox Federation", Sol),
    e("pythenians", "wPYTHENIAN", "Wrapped Pythenians", Sol),
    e("cyber_frogs", "wCYBERFROG", "Wrapped Cyber Frogs", Sol),
    e("omb", "wOMB", "Wrapped OMB", Ord),
    e("nodemonkes", "wNODEMONKE", "Wrapped NodeMonke", Ord),
    e("bitcoin-puppets", "wPUPPET", "Wrapped Bitcoin Puppet", Ord),
    e("bitcoin-punks", "wBPUNK", "Wrapped Bitcoin Punk", Ord),
    e("natcats", "wNATCAT", "Wrapped Natcat", Ord),
    e("runestone", "wRUNESTONE", "Wrapped Runestone", Ord),
    e("bitcoin-weirdos", "wWEIRDO", "Wrapped Bitcoin Weirdo", Ord),
    e("cents", "wCENTS", "Wrapped CENTS", Ord),
    e("rsic", "wRSIC", "Wrapped RSIC", Ord),
    e("pizza-ninjas", "wPIZZA", "Wrapped Pizza Ninja", Ord),
    e("quantum-cats", "wQCAT", "Wrapped Quantum Cat", Ord),
    e("wizards-of-ord", "wWOO", "Wrapped Wizard of Ord", Ord),
    e("taproot-wizards", "wTAPWIZ", "Wrapped Taproot Wizard", Ord),
    e("bitcoin-wizards", "wBWIZ", "Wrapped Bitcoin Wizard", Ord),
    e("stonkbrokers", "wSTONKBROK", "Wrapped StonkBrokers", Robinhood),
    e("chainmancers", "wCHAINMANC", "Wrapped Chain Mancers", Robinhood),
    e("rekttradooors", "wREKTTRADO", "Wrapped Rekt Tradooors", Robinhood),
];

/// How many collections the ceremony pins (`examples/launch_pins.rs --count`, the default): the first `LAUNCH_COUNT` of `ORDER`, every
/// collection with a pipeline — the 11 Ethereum collections, Bitcoin Frogs, the 12 Solana collections and the 14 Ordinals (operator
/// decision 2026-10-12, "no picking"; every Inscriptions listing is live by default). It used to be the five opened at launch, then 24;
/// since the decision of 2026-10-12 nothing opens unasked, so "launch" here means "pinned at the ceremony". The three Robinhood Chain
/// collections after them are pinned only after the ceremony's 4663 read-back (`launch_pins --include-robinhood`, audit C4-03).
pub const LAUNCH_COUNT: usize = 38;

/// The collections pinned at the ceremony.
pub fn pinned_at_ceremony() -> &'static [Entry] {
    &ORDER[..LAUNCH_COUNT]
}

/// Whether an index is pinned at the ceremony (the rest are pinned later).
pub const fn is_pinned_at_ceremony(index: u32) -> bool {
    (index as usize) < LAUNCH_COUNT
}

/// The index of a collection by slug.
pub fn index_of(slug: &str) -> Option<u32> {
    ORDER.iter().position(|x| x.slug == slug).map(|i| i as u32)
}

/// The next index to open: the first of `ORDER` not in `open` (an index the launch file pinned out of order is skipped).
pub fn next_to_open(open: &dyn Fn(u32) -> bool) -> Option<u32> {
    (0..ORDER.len() as u32).find(|i| !open(*i))
}

/// What opening one collection costs the canister, in lamports (§2, §12.2 [M]): the wX mint (82 bytes), its Metaplex metadata (rent and
/// the 0.01 SOL Metaplex fee), the DBC config (rent), and the transactions' fees (two transactions, three signatures).
pub const MINT_RENT: u64 = 1_461_600;
pub const METADATA_RENT: u64 = 5_115_600;
pub const METAPLEX_FEE: u64 = 10_000_000;
pub const CONFIG_RENT: u64 = 5_984_080;
pub const OPEN_TX_FEES: u64 = 25_000;
pub const OPEN_COST: u64 = MINT_RENT + METADATA_RENT + METAPLEX_FEE + CONFIG_RENT + OPEN_TX_FEES;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_is_the_designs() {
        assert_eq!(ORDER.len(), 41, "38 + the three Robinhood Chain collections of robinhood-collections.json");
        // the 38 pinned at the ceremony: every collection with a pipeline (Ethereum, Bitcoin Frogs, Solana, the Ordinals); the three
        // Robinhood Chain collections after, pinned once the ceremony read chain 4663 back (audit C4-03)
        assert_eq!(LAUNCH_COUNT, 38);
        assert!(pinned_at_ceremony().iter().all(|x| x.source != Robinhood));
        assert!(ORDER[LAUNCH_COUNT..].iter().all(|x| x.source == Robinhood));
        assert!((0..LAUNCH_COUNT as u32).all(is_pinned_at_ceremony) && !is_pinned_at_ceremony(LAUNCH_COUNT as u32));
        let head: Vec<&str> = ORDER.iter().take(7).map(|x| x.slug).collect();
        assert_eq!(head, ["bayc", "milady", "mad_lads", "smb_gen2", "bitcoin-frogs", "pudgypenguins", "azuki"]);
        assert_eq!(ORDER.iter().take(5).map(|x| x.source).collect::<Vec<_>>(), [Eth, Eth, Sol, Sol, Btc]);
        let next: Vec<&str> = ORDER.iter().skip(7).take(7).map(|x| x.slug).collect();
        assert_eq!(next, ["mayc", "doodles", "lilpudgys", "moonbirds", "clonex", "checks", "identitymd"]);
        assert!(ORDER[5..14].iter().all(|x| x.source == Eth));
        assert_eq!(ORDER.iter().filter(|x| x.source == Eth).count(), 11);
        // then the ten other Solana collections, then the Ordinals, OMB first
        assert_eq!(ORDER[14].slug, "claynosaurz");
        assert!(ORDER[14..24].iter().all(|x| x.source == Sol));
        assert_eq!(ORDER[24].slug, "omb", "the Ordinals follow the Solana collections, OMB first");
        assert!(ORDER[24..].iter().all(|x| x.source == Ord || x.source == Robinhood));
        assert_eq!(ORDER.iter().filter(|x| x.source == Sol).count(), 12);
        assert_eq!(ORDER.iter().filter(|x| x.source == Btc).count(), 1);
        // slugs and symbols are unique, the wrapped symbols fit Metaplex's 10 bytes, the names its 32
        for (i, a) in ORDER.iter().enumerate() {
            assert!(a.symbol.len() <= 10 && a.name.len() <= 32, "{}", a.slug);
            for b in &ORDER[i + 1..] {
                assert!(a.slug != b.slug && a.symbol != b.symbol, "{} / {}", a.slug, b.slug);
            }
        }
        assert_eq!(index_of("mad_lads"), Some(2));
        assert_eq!(index_of("azuki"), Some(6));
        assert_eq!(next_to_open(&|i| i < 5), Some(5));
        assert_eq!(next_to_open(&|_| true), None);
        assert_eq!(OPEN_COST, 22_586_280, "≈ 0.0226 SOL (§16 says ≈ 0.023)");
    }
}
