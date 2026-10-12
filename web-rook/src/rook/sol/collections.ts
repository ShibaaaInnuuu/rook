/**
 * Rook's collections on Solana, in `listing_order` (deployments/launch/nft-pilot.json: the eleven Ethereum collections in file
 * order, then Bitcoin Frogs, then the further Ordinals collections of its `inscriptions` list; the site's listing ids 0…25, the
 * order of the site config's rows, src/config/solana.ts). Ids 12-25 are the further Ordinals (docs/v9/ordinals-collections.md,
 * `wrapping: "later"` in the launch file), pinned at the ceremony with the others. `tierSol` and `floorSol` are
 * deployments/launch/graduation-tiers.json's tier and day-1 floor for the listing: the two inputs of its graduation threshold in
 * whole wX, `T_wX = max(3, ⌈tierSol / floorSol⌉)` (docs/v9/canister-only-solana.md §3.2; src/rook/sol/graduation.ts computes it
 * exactly as the canister's launch pins do). Names and wX symbols are nft-pilot.json's; the logo is the collection's official picture,
 * served by the site itself from public/collections/ (scripts/collection-images.mjs fetches each from its marketplace and records the
 * source in public/collections/manifest.json; test/collectionimages.test.ts holds every logo to a file there), the initials mark as
 * the fallback.
 * test/sol.test.ts holds this list against nft-pilot.json, solana-collections.json, robinhood-collections.json and
 * graduation-tiers.json whenever the repository is beside the package (the Docker build context holds web-rook/ alone, so the list
 * is committed here).
 *
 * `COLLECTIONS` is every collection with a page, 41 (the canister's registry, autobuyer/canister-rs/src/coins/unlock.rs `ORDER`, in
 * another order): those 26 (`LISTED_COLLECTIONS`), then the 12 Solana collections of deployments/launch/solana-collections.json
 * (chain "Solana", ids 26…37), then the 3 Robinhood Chain collections of deployments/launch/robinhood-collections.json (chain
 * "Robinhood Chain", ids 38…40). Operator decision 2026-10-12 ("the people choose", no picking): the ceremony pins every collection
 * with a pipeline (`PINNED_COLLECTIONS`, 38: the 11 Ethereum ones, Bitcoin Frogs, the 12 Solana ones, the 14 Ordinals), and the canister opens one when a
 * creator first asks for it (`coins_request_open`). The Robinhood Chain ones are pinned after the ceremony, once it has read back
 * Seaport 1.6's code hash on chain 4663. QUOTRONS, an ERC-404 pair whose token transfers mint and burn its NFTs, is left out.
 */
/** "Robinhood Chain": an EthNft listing of chain 4663 (docs/v9/robinhood-everything.md §3), bought, held and delivered like an Ethereum one. */
export type Chain = "Ethereum" | "Bitcoin" | "Solana" | "Robinhood Chain";
export type SolCollection = {
  /** `listingId`: the site's own numbering (nft-pilot.json's listing order, then the Solana collections from 26); the canister's index
   *  is another order (`coins_collections`, found by wX symbol). */
  slug: string; listingId: number; name: string; wrappedName: string; wxSymbol: string;
  /** "Solana": a Solana NFT collection; its NFTs are held by the canister's own Solana wallet, `contract` is its collection key */
  chain: Chain; contract: string; logo: string | null;
  /** nft-pilot.json `wrapping: "later"`: the 14 Ordinals beyond Bitcoin Frogs (docs/v9/ordinals-collections.md §5). It no longer
   *  decides availability: they are pinned at the ceremony with the others (operator decision 2026-10-12, no picking). */
  wrapping?: "later";
  /** robinhood-collections.json `tokenBound`: an ERC-6551 collection (each NFT is an account holding tokenized stocks), priced and
   *  bought as the NFT only (StonkBrokers). */
  tokenBound?: true;
  /** a Solana collection's `threshold_wx` (solana-collections.json); the others compute it from `tierSol` and `floorSol` */
  thresholdWx?: number;
  /** graduation-tiers.json `tier`: the SOL the collection's graduation raise is worth at its day-1 floor (85 / 170 / 255 / 510). */
  tierSol: TierSol;
  /** graduation-tiers.json `floor_sol`: the collection's floor on 2026-10-10, in SOL, as the threshold was set from it. */
  floorSol: number;
};
/** A Solana NFT collection as solana-collections.json lists it: what its page and the thresholds table need (COLLECTIONS makes it a SolCollection). */
export type LaterCollection = { slug: string; name: string; wxSymbol: string; logo: string | null; tierSol: TierSol; floorSol: number; thresholdWx: number; supply: number };

/** The SOL tiers a threshold is derived from (graduation-tiers.json `tiers_sol`; the rule picks the smallest whose buying power covers
 *  1.33 NFTs). The thresholds themselves are in whole wX: src/rook/sol/graduation.ts. */
export type TierSol = 85 | 170 | 255 | 510;

/** nft-pilot.json's 26 listings in `listing_order` (the site config's rows, src/config/solana.ts, follow it). */
export const LISTED_COLLECTIONS: readonly SolCollection[] = [
  { slug: "milady", listingId: 0, name: "Milady Maker", wrappedName: "Wrapped Milady", wxSymbol: "wMILADY", chain: "Ethereum", contract: "0x5af0d9827e0c53e4799bb226655a1de152a425a5", logo: "/collections/milady.webp", tierSol: 85, floorSol: 18.61 },
  { slug: "azuki", listingId: 1, name: "Azuki", wrappedName: "Wrapped Azuki", wxSymbol: "wAZUKI", chain: "Ethereum", contract: "0xed5af388653567af2f388e6224dc7c4b3241c544", logo: "/collections/azuki.webp", tierSol: 85, floorSol: 13.71 },
  { slug: "mayc", listingId: 2, name: "Mutant Ape Yacht Club", wrappedName: "Wrapped Mutant Ape", wxSymbol: "wMUTANTAPE", chain: "Ethereum", contract: "0x60e4d786628fea6478f785a6d7e704777c86a7c6", logo: "/collections/mayc.webp", tierSol: 85, floorSol: 18.56 },
  { slug: "pudgypenguins", listingId: 3, name: "Pudgy Penguins", wrappedName: "Wrapped Pudgy Penguin", wxSymbol: "wPUDGY", chain: "Ethereum", contract: "0xbd3531da5cf5857e7cfaa92426877b022e612cf8", logo: "/collections/pudgypenguins.webp", tierSol: 255, floorSol: 59.00 },
  { slug: "bayc", listingId: 4, name: "Bored Ape Yacht Club", wrappedName: "Wrapped Bored Ape", wxSymbol: "wBOREDAPE", chain: "Ethereum", contract: "0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d", logo: "/collections/bayc.webp", tierSol: 510, floorSol: 136.84 },
  { slug: "doodles", listingId: 5, name: "Doodles", wrappedName: "Wrapped Doodle", wxSymbol: "wDOODLE", chain: "Ethereum", contract: "0x8a90cab2b38dba80c64b7734e58ee1db38b8992e", logo: "/collections/doodles.webp", tierSol: 85, floorSol: 5.95 },
  { slug: "moonbirds", listingId: 6, name: "Moonbirds", wrappedName: "Wrapped Moonbird", wxSymbol: "wMOONBIRD", chain: "Ethereum", contract: "0x23581767a106ae21c074b2276d25e5c3e136a68b", logo: "/collections/moonbirds.webp", tierSol: 85, floorSol: 8.90 },
  { slug: "clonex", listingId: 7, name: "CloneX", wrappedName: "Wrapped CloneX", wxSymbol: "wCLONEX", chain: "Ethereum", contract: "0x49cf6f5d44e70224e2e23fdcdd2c053f30ada28b", logo: "/collections/clonex.webp", tierSol: 85, floorSol: 3.38 },
  { slug: "lilpudgys", listingId: 8, name: "Lil Pudgys", wrappedName: "Wrapped Lil Pudgy", wxSymbol: "wLILPUDGY", chain: "Ethereum", contract: "0x524cab2ec69124574082676e6f654a18df49a048", logo: "/collections/lilpudgys.webp", tierSol: 85, floorSol: 5.83 },
  { slug: "checks", listingId: 9, name: "Checks VV", wrappedName: "Wrapped Checks VV", wxSymbol: "wCHECKSVV", chain: "Ethereum", contract: "0x036721e5a769cc48b3189efbb9cce4471e8a48b1", logo: "/collections/checks.webp", tierSol: 85, floorSol: 3.47 },
  { slug: "identitymd", listingId: 10, name: "Identity MD", wrappedName: "Wrapped Identity MD", wxSymbol: "wIDENTITY", chain: "Ethereum", contract: "0x0000ec93127baa929e58e97dd0095a2bfb38ec1d", logo: "/collections/identitymd.webp", tierSol: 170, floorSol: 40.39 },
  { slug: "bitcoin-frogs", listingId: 11, name: "Bitcoin Frogs", wrappedName: "Wrapped Bitcoin Frog", wxSymbol: "wBFROG", chain: "Bitcoin", contract: "inscriptions (10,000 members)", logo: "/collections/bitcoin-frogs.webp", tierSol: 85, floorSol: 3.01 },
  { slug: "nodemonkes", listingId: 12, name: "NodeMonkes", wrappedName: "Wrapped NodeMonke", wxSymbol: "wNODEMONKE", chain: "Bitcoin", contract: "inscriptions (10,000 members)", logo: "/collections/nodemonkes.png", wrapping: "later" , tierSol: 85, floorSol: 17.44 },
  { slug: "omb", listingId: 13, name: "Ordinal Maxi Biz (OMB)", wrappedName: "Wrapped OMB", wxSymbol: "wOMB", chain: "Bitcoin", contract: "inscriptions (9,000 members)", logo: "/collections/omb.webp", wrapping: "later" , tierSol: 85, floorSol: 13.16 },
  { slug: "bitcoin-puppets", listingId: 14, name: "Bitcoin Puppets", wrappedName: "Wrapped Bitcoin Puppet", wxSymbol: "wPUPPET", chain: "Bitcoin", contract: "inscriptions (10,001 members)", logo: "/collections/bitcoin-puppets.webp", wrapping: "later" , tierSol: 85, floorSol: 7.70 },
  { slug: "bitcoin-punks", listingId: 15, name: "Bitcoin Punks", wrappedName: "Wrapped Bitcoin Punk", wxSymbol: "wBPUNK", chain: "Bitcoin", contract: "inscriptions (10,000 members)", logo: "/collections/bitcoin-punks.png", wrapping: "later" , tierSol: 85, floorSol: 2.36 },
  { slug: "natcats", listingId: 16, name: "Natcats", wrappedName: "Wrapped Natcat", wxSymbol: "wNATCAT", chain: "Bitcoin", contract: "inscriptions (8,060 members)", logo: "/collections/natcats.png", wrapping: "later" , tierSol: 85, floorSol: 2.14 },
  { slug: "runestone", listingId: 17, name: "Runestone", wrappedName: "Wrapped Runestone", wxSymbol: "wRUNESTONE", chain: "Bitcoin", contract: "inscriptions (112,384 members)", logo: "/collections/runestone.png", wrapping: "later", tierSol: 85, floorSol: 1.04 },
  { slug: "bitcoin-weirdos", listingId: 18, name: "Bitcoin Weirdos", wrappedName: "Wrapped Bitcoin Weirdo", wxSymbol: "wWEIRDO", chain: "Bitcoin", contract: "inscriptions (10,000 members)", logo: "/collections/bitcoin-weirdos.png", wrapping: "later", tierSol: 85, floorSol: 0.49 },
  { slug: "cents", listingId: 19, name: "CENTS", wrappedName: "Wrapped CENTS", wxSymbol: "wCENTS", chain: "Bitcoin", contract: "inscriptions (10,000 members)", logo: "/collections/cents.jpg", wrapping: "later", tierSol: 85, floorSol: 3.87 },
  { slug: "rsic", listingId: 20, name: "RSIC Metaprotocol", wrappedName: "Wrapped RSIC", wxSymbol: "wRSIC", chain: "Bitcoin", contract: "inscriptions (20,998 members)", logo: "/collections/rsic.png", wrapping: "later", tierSol: 85, floorSol: 0.56 },
  { slug: "pizza-ninjas", listingId: 21, name: "Pizza Ninjas", wrappedName: "Wrapped Pizza Ninja", wxSymbol: "wPIZZA", chain: "Bitcoin", contract: "inscriptions (2,920 members)", logo: "/collections/pizza-ninjas.gif", wrapping: "later", tierSol: 85, floorSol: 6.56 },
  { slug: "quantum-cats", listingId: 22, name: "Quantum Cats", wrappedName: "Wrapped Quantum Cat", wxSymbol: "wQCAT", chain: "Bitcoin", contract: "inscriptions (3,313 members)", logo: "/collections/quantum-cats.png", wrapping: "later", tierSol: 85, floorSol: 7.86 },
  { slug: "wizards-of-ord", listingId: 23, name: "The Wizards of Ord", wrappedName: "Wrapped Wizard of Ord", wxSymbol: "wWOO", chain: "Bitcoin", contract: "inscriptions (3,333 members)", logo: "/collections/wizards-of-ord.png", wrapping: "later", tierSol: 85, floorSol: 4.06 },
  { slug: "taproot-wizards", listingId: 24, name: "Taproot Wizards", wrappedName: "Wrapped Taproot Wizard", wxSymbol: "wTAPWIZ", chain: "Bitcoin", contract: "inscriptions (2,102 members)", logo: "/collections/taproot-wizards.webp", wrapping: "later", tierSol: 170, floorSol: 35.35 },
  { slug: "bitcoin-wizards", listingId: 25, name: "Bitcoin Wizards", wrappedName: "Wrapped Bitcoin Wizard", wxSymbol: "wBWIZ", chain: "Bitcoin", contract: "inscriptions (1,337 members)", logo: "/collections/bitcoin-wizards.webp", wrapping: "later", tierSol: 85, floorSol: 10.15 },
];
/** The twelve Solana NFT collections (deployments/launch/solana-collections.json; docs/v9/canister-only-solana.md §16), in the file's
 *  order. Pinned at the ceremony with the Ethereum collections and Bitcoin Frogs (operator decision 2026-10-12); each opens when a
 *  creator first asks for it. Their thresholds are the launch file's `threshold_wx`, the same rule on the floors it measured. */
export const SOLANA_COLLECTIONS: readonly LaterCollection[] = [
  { slug: "mad_lads", name: "Mad Lads", wxSymbol: "wMADLAD", logo: "/collections/mad_lads.png", tierSol: 85, floorSol: 8.352, thresholdWx: 11, supply: 9968 },
  { slug: "smb_gen2", name: "SMB Gen2", wxSymbol: "wSMBGEN2", logo: "/collections/smb_gen2.png", tierSol: 85, floorSol: 12.659, thresholdWx: 7, supply: 4993 },
  { slug: "claynosaurz", name: "Claynosaurz", wxSymbol: "wCLAYNO", logo: "/collections/claynosaurz.jpg", tierSol: 85, floorSol: 6.979, thresholdWx: 13, supply: 10232 },
  { slug: "famous_fox_federation", name: "Famous Fox Federation", wxSymbol: "wFFF", logo: "/collections/famous_fox_federation.gif", tierSol: 85, floorSol: 3.238, thresholdWx: 27, supply: 10000 },
  { slug: "degods", name: "DeGods (Solana)", wxSymbol: "wDEGOD", logo: "/collections/degods.png", tierSol: 85, floorSol: 3.482, thresholdWx: 25, supply: 6239 },
  { slug: "galactic_geckos", name: "Galactic Geckos", wxSymbol: "wGECKO", logo: "/collections/galactic_geckos.png", tierSol: 85, floorSol: 4.0816, thresholdWx: 21, supply: 10000 },
  { slug: "taiyo_robotics", name: "Taiyo Robotics", wxSymbol: "wTAIYO", logo: "/collections/taiyo_robotics.webp", tierSol: 85, floorSol: 14.9485, thresholdWx: 6, supply: 2348 },
  { slug: "okay_bears", name: "Okay Bears", wxSymbol: "wOKAYBEAR", logo: "/collections/okay_bears.png", tierSol: 85, floorSol: 1.2385, thresholdWx: 69, supply: 9858 },
  { slug: "smb_gen3", name: "SMB Gen3", wxSymbol: "wSMBGEN3", logo: "/collections/smb_gen3.png", tierSol: 85, floorSol: 2.3373, thresholdWx: 37, supply: 8333 },
  { slug: "transdimensional_fox_federation", name: "Transdimensional Fox Federation", wxSymbol: "wTFF", logo: "/collections/transdimensional_fox_federation.png", tierSol: 85, floorSol: 1.208, thresholdWx: 71, supply: 7779 },
  { slug: "pythenians", name: "Pythenians", wxSymbol: "wPYTHENIAN", logo: "/collections/pythenians.png", tierSol: 85, floorSol: 1.2, thresholdWx: 71, supply: 3686 },
  { slug: "cyber_frogs", name: "Cyber Frogs", wxSymbol: "wCYBERFROG", logo: "/collections/cyber_frogs.png", tierSol: 85, floorSol: 1.323, thresholdWx: 65, supply: 8088 },
];
/** Each Solana collection's collection key (solana-collections.json `collection_key`, the verified-collection mint its members name). */
export const SOLANA_COLLECTION_KEYS: Readonly<Record<string, string>> = {
  mad_lads: "J1S9H3QjnRtBbbuD4HjPV6RpRhwuk4zKbxsnCHuTgh9w", smb_gen2: "SMBtHCCC6RYRutFEPb4gZqeBLUZbMNhRKaMKZZLHi7W",
  claynosaurz: "6mszaj17KSfVqADrQj3o4W3zoLMTykgmV37W4QadCczK", famous_fox_federation: "BUjZjAS2vbbb65g7Z1Ca9ZRVYoJscURG5L3AkVvHP9ac",
  degods: "6XxjKYFbcndh2gDcsUrmZgVEsoDxXMnfsaGY6fpTJzNr", galactic_geckos: "J6RJFQfLgBTcoAt3KoZFiTFW9AbufsztBNDgZ7Znrp1Q",
  taiyo_robotics: "HJx4HRAT3RiFq7cy9fSrvP92usAmJ7bJgPccQTyroT2r", okay_bears: "3saAedkM9o5g1u5DCqsuMZuC4GRqPB4TuMkvSsSVvGQ3",
  smb_gen3: "8Rt3Ayqth4DAiPnW9MDFi63TiQJHmohfTWLMQFHi4KZH", transdimensional_fox_federation: "HbPtffcEzzSzZ1VJaTjaAJqTUQpfNeMkPqNG81dWE2Bi",
  pythenians: "pyTh2UtBKfuDW6KCdT3swospYeoLmmKaGujWA91Moru", cyber_frogs: "2kEAck1FyW8TxB5SprEnasb4gkaahTdDV83wPtxm9y32",
};

/** The three Robinhood Chain (4663) collections that passed the measurement (deployments/launch/robinhood-collections.json, 2026-10-11),
 *  in the file's order: the launch file's `name` and `symbol` (≤ 10 bytes, the canister's limit), its ERC-721 address and OpenSea slug,
 *  and graduation-tiers.json's tier and floor. StonkBrokers is token-bound (ERC-6551). Pinned after the ceremony. */
export const ROBINHOOD_COLLECTIONS: readonly (Omit<SolCollection, "listingId" | "chain"> & { openseaSlug: string })[] = [
  { slug: "stonkbrokers", name: "StonkBrokers", wrappedName: "Wrapped StonkBrokers", wxSymbol: "wSTONKBROK", contract: "0x539cdd042c2f3d93ebc5be7dfff0c79f3b4fabf0", openseaSlug: "stonkbrokers-434284142", logo: "/collections/stonkbrokers.webp", tierSol: 170, floorSol: 33.54, tokenBound: true },
  { slug: "chainmancers", name: "Chain Mancers", wrappedName: "Wrapped Chain Mancers", wxSymbol: "wCHAINMANC", contract: "0x797a2e030b7e49107c8f07bf0300ea9cae88ca57", openseaSlug: "chain-mancers", logo: "/collections/chainmancers.webp", tierSol: 85, floorSol: 6.27 },
  { slug: "rekttradooors", name: "Rekt Tradooors", wrappedName: "Wrapped Rekt Tradooors", wxSymbol: "wREKTTRADO", contract: "0x7b3ecfa33657de415ff269dc97dfa82954cee706", openseaSlug: "rekt-tradooor", logo: "/collections/rekttradooors.webp", tierSol: 85, floorSol: 1.21 },
];

/** Every Rook collection with a page: nft-pilot.json's 26, then the 12 Solana collections (chain "Solana", site ids 26…37), then the 3
 *  Robinhood Chain collections (site ids 38…40). */
export const COLLECTIONS: readonly SolCollection[] = [
  ...LISTED_COLLECTIONS,
  ...SOLANA_COLLECTIONS.map((c, i): SolCollection => ({ slug: c.slug, listingId: LISTED_COLLECTIONS.length + i, name: c.name, wrappedName: `Wrapped ${c.name}`,
    wxSymbol: c.wxSymbol, chain: "Solana", contract: SOLANA_COLLECTION_KEYS[c.slug], logo: c.logo, tierSol: c.tierSol, floorSol: c.floorSol, thresholdWx: c.thresholdWx })),
  ...ROBINHOOD_COLLECTIONS.map(({ openseaSlug: _os, ...c }, i): SolCollection => ({ ...c, listingId: LISTED_COLLECTIONS.length + SOLANA_COLLECTIONS.length + i, chain: "Robinhood Chain" })),
];

/** Where a collection stands before anyone launches on it:
 *  - "pinned": pinned at the ceremony (operator decision 2026-10-12, "the people choose", no picking: every collection with a
 *    pipeline, the 14 Ordinals included); it opens when a creator first asks for it;
 *  - "after-ceremony": a Robinhood Chain collection, pinned once the ceremony has read back Seaport 1.6's code hash on chain 4663.
 *  Once a canister is read, `coins_collections` is the truth (src/rook/sol/opening.ts `launchableOf`). */
export type Availability = "pinned" | "after-ceremony";
export const availabilityOf = (c: Pick<SolCollection, "chain">): Availability => (c.chain === "Robinhood Chain" ? "after-ceremony" : "pinned");
/** What the collections page says of a collection that is not open, by its availability. */
export const AVAILABILITY_TEXT: Record<Availability, string> = {
  pinned: "opens on its first coin",
  "after-ceremony": "coming: pinned after the Robinhood Chain check",
};

/** Operator decision 2026-10-12 ("the people choose", no picking): the 38 collections the ceremony pins, every one with a pipeline (the
 *  11 Ethereum collections, Bitcoin Frogs, the 12 Solana collections, the 14 Ordinals; not the Robinhood Chain ones, pinned after the
 *  ceremony's 4663 readback). Each opens when a creator first asks for it. The launch wizard lists these while no canister is read. */
export const PINNED_COLLECTIONS: readonly SolCollection[] = COLLECTIONS.filter((c) => availabilityOf(c) === "pinned");

/** The canister's registry (autobuyer/canister-rs/src/coins/unlock.rs `ORDER`): a collection's index there is its position in this list,
 *  41 slugs: BAYC, Milady, Mad Lads, SMB Gen2, Bitcoin Frogs, the other Ethereum and Solana collections, the 14 Ordinals (these 38 pinned
 *  at the ceremony, `LAUNCH_COUNT`), then the 3 Robinhood Chain collections. The live site never relies on it (it finds a collection's
 *  index by wX symbol in `coins_collections`); the demo's canister numbers its collections by it. test/sol.test.ts holds it against
 *  unlock.rs when the repository is beside the package. */
export const REGISTRY_ORDER: readonly string[] = [
  "bayc", "milady", "mad_lads", "smb_gen2", "bitcoin-frogs", "pudgypenguins", "azuki", "mayc", "doodles", "lilpudgys", "moonbirds", "clonex",
  "checks", "identitymd", "claynosaurz", "famous_fox_federation", "degods", "galactic_geckos", "taiyo_robotics", "okay_bears", "smb_gen3",
  "transdimensional_fox_federation", "pythenians", "cyber_frogs", "omb", "nodemonkes", "bitcoin-puppets", "bitcoin-punks", "natcats", "runestone",
  "bitcoin-weirdos", "cents", "rsic", "pizza-ninjas", "quantum-cats", "wizards-of-ord", "taproot-wizards", "bitcoin-wizards", "stonkbrokers",
  "chainmancers", "rekttradooors",
];
/** How many of the registry's first entries the ceremony pins (unlock.rs `LAUNCH_COUNT`). */
export const LAUNCH_COUNT = 38;

/** The chains in the order the collections page groups them. */
export const CHAINS: readonly Chain[] = ["Ethereum", "Solana", "Bitcoin", "Robinhood Chain"];

/** Where the canister holds a collection's items (docs/v9/canister-only-solana.md §8, robinhood-everything.md §3): one secp256k1 address
 *  on every EVM chain, the taproot custody on Bitcoin, its own wallet (`K_main`) on Solana. */
export function custodyOf(chain: Chain): string {
  return chain === "Solana" ? "the canister's own Solana wallet" : chain === "Bitcoin" ? "the canister's taproot custody address"
    : chain === "Robinhood Chain" ? "the canister's EVM address, on Robinhood Chain" : "the canister's EVM address, on Ethereum";
}
/** What a redemption of one of the collection's wX must name as its destination (the canister checks it when the intent opens). */
export function destinationOf(chain: Chain): string {
  return chain === "Solana" ? "a Solana wallet" : chain === "Bitcoin" ? "a Bitcoin taproot address (bc1p…)"
    : chain === "Robinhood Chain" ? "an address on Robinhood Chain (0x…)" : "an Ethereum address (0x…)";
}

export const collectionBySlug = (slug: string | undefined | null): SolCollection | null => COLLECTIONS.find((c) => c.slug === slug) ?? null;
