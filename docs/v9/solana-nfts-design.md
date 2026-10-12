# Native Solana NFT collections (rook_core, 2026-10-10)

Operator decisions of 2026-10-10:

1. Rook lists **Solana NFT collections** next to the Ethereum and Ordinals ones. The first twelve: Mad Lads, SMB Gen2,
   Claynosaurz, Famous Fox Federation, DeGods (Solana), Galactic Geckos, Taiyo Robotics, Okay Bears, SMB Gen3,
   Transdimensional Fox Federation, Pythenians, Cyber Frogs. The program is generic over three standards.
2. **The NFTs are held in the canister's Solana wallet** (its threshold-Ed25519 address, an ordinary on-curve wallet), the
   same custodian model as EthNft and Inscriptions — **not** a program escrow. Reason given: most of these collections are
   pNFTs on the Metaplex Foundation rule set, and a rule set that only admits wallet-to-wallet transfers (or allow-listed
   programs) would refuse a program PDA. A wallet custodian works under every rule set.
3. The treasury's collection limit goes from 32 to 64.

This document is the contract the canister and the site build against. Program: `solana/programs/rook_core`
(`nft.rs`: the account reads; `instructions/wrap.rs`: `mint_item`, `redeem`, `record_delivery`; `instructions/treasury.rs`:
`weigh_distribution`). The program moves no NFT and makes **no CPI** to Token Metadata, Token Auth Rules or mpl-core: it only
**reads** their accounts.

> Note on the rule set (read from mainnet 2026-10-10, `eBJLFYPxJmMGKuFwpDWkzxZeUrad92kZRC5BJLpzyT9`): the account holds nine
> revisions. The first ones are the restrictive `ProgramOwnedList` of 14 marketplace programs (or `IsWallet` both sides);
> **the latest (9th) revision is `Pass` for every operation**, and Token Metadata applies the latest revision. Today a PDA
> could hold those pNFTs. The custodian model does not depend on it either way (the foundation could publish a stricter
> revision again); the tests run the real rule set and a wallet-to-wallet `TransferV1` through it.

| Standard | What it is | Membership (checked on chain at `mint_item`) | How the canister delivers |
|---|---|---|---|
| `Legacy` | Token Metadata `NonFungible` (or pre-standard `None`), classic SPL Token | metadata `collection == Some { verified: true, key: listing.collection }`; a master edition | SPL Token `Transfer` (or TM `TransferV1`) to the destination's ATA |
| `Programmable` (pNFT) | Token Metadata `ProgrammableNonFungible` | same | Token Metadata `TransferV1` with the NFT's rule set (wallet to wallet) |
| `Core` | Metaplex Core `AssetV1` | `asset.update_authority == Collection(listing.collection)` | mpl-core `TransferV1` |

A Token Metadata listing (`Legacy` or `Programmable`) accepts both legacy and programmable members of its collection (some
collections migrated part of their items); `standard` tells the site and the canister which is dominant. A `Core` listing
accepts only Core assets. Compressed NFTs (Bubblegum cNFTs; Core's ledger-state `HashedAssetV1`) are refused: there is no
account to read.

## 1. Listing: `ListingKind::SolNft { collection: Pubkey, standard: NftStandard }`

Appended as variant 2 of `ListingKind`; `NftStandard` is `0 Legacy`, `1 Programmable`, `2 Core`. `Listing` grows by one byte
(285 bytes, rent 0.00287448 SOL; the variant is 33 bytes, the largest) for every listing created by this program version.

`list_collection(args)` (owner) with `args.kind = SolNft { collection, standard }`:

- `collection ≠ 0` — the Token Metadata collection mint, or the Core collection account (`BadInput`);
- `custody` must be **empty** (`Empty`): the custody wallet **is `listing.custodian`** (the canister's Ed25519 key is its
  Solana wallet). Any other kind still needs its custody address (`Empty` if missing);
- `custodian ≠ 0` (mints items, records deliveries, takes advances);
- `poster ≠ 0` (`BadInput`): the canister posts the floor, as for EthNft (`keeper_down` / `live_poster` are EthNft's).

Everything else is kind-agnostic and unchanged: `init_wrapper`, `init_module`, `post_floor`, `mint`, `burn`, `convert`,
`advance`, `settle_advance`, `item_arrived`, `return_advance`, `snapshot_cap`, `skim`, raffles, the treasury, H11 (both
the wrapper's mint limit and the module's arrivals), `MAX_HELD`.

## 2. `mint_item(args)` on a SolNft listing — the custodian

Accounts unchanged (`custodian` (w, s), `listing`, `wrapper` (w), `wx_mint` (w), `item_set` (w), `item` (w, init
`["item", wrapper, item_count]`), `held` (w, `["held", wrapper, key]`), `source` (w, `["source", wrapper, source]`), `dest`
(w), `token_program`, `system_program`) **plus `remaining_accounts`** (read-only):

| Listing standard | `remaining_accounts`, in order |
|---|---|
| `Legacy` / `Programmable` | `mint`, `metadata` (`["metadata", TM, mint]`), `edition` (`["metadata", TM, mint, "edition"]`), the custodian's token account for `mint` (normally its ATA) |
| `Core` | `asset`, `collection` (`= listing.collection`) |

Arguments (`MintItemArgs`): `key` = the mint / asset address (32 bytes); `tx` = 32 non-zero bytes naming the transaction
that brought it into custody (the canister uses the first 32 bytes of the purchase transaction's signature); `source =
keccak(key ‖ tx)` (as EthNft: once per (item, transaction) ever); `vout = 0`; `inscription_id` empty; `proof` empty;
`deposit` as for the other kinds.

Checks, on top of the unchanged ones (custodian, not paused, `AlreadyHeld`, `AlreadyMinted`, H11, `CustodyFull`):

- `remaining_accounts[0] == key` (`BadInput`); a missing account → `MissingAccount`;
- **Token Metadata**: the mint is owned by **classic SPL Token** (Token-2022 refused), initialized, `supply == 1`,
  `decimals == 0`; the metadata is owned by TM, key `MetadataV1`, `metadata.mint == mint`; `token_standard` is `None` /
  `NonFungible` / `ProgrammableNonFungible` (editions and fungibles → `NotAnNft`); `collection == Some { verified: true,
  key: listing.collection }` (`NotInCollection`); the edition account is the mint's edition PDA (derived with the metadata's
  `edition_nonce`), owned by TM, key `MasterEditionV1/V2` (`NotAnNft`); the token account is owned by SPL Token, its `mint ==
  key`, its `owner == listing.custodian`, `amount == 1` (`NotInCustody`; a pNFT's token account is frozen, which is fine);
- **Core**: the asset is owned by mpl-core, key `AssetV1` (`NotAnNft`), `update_authority == Collection(listing.collection)`
  and the collection account is that collection, owned by mpl-core, key `CollectionV1` (`NotInCollection`); `asset.owner ==
  listing.custodian` (`NotInCustody`); and neither the asset nor the collection carries a **permanent** delegate
  (`PermanentFreezeDelegate`, `PermanentTransferDelegate`, `PermanentBurnDelegate`, `PermanentFreezeExecute`), the asset
  no `FreezeDelegate` / `TransferDelegate` / `BurnDelegate` whose authority is not its owner, and neither an external plugin
  adapter with lifecycle checks (`LifecycleHook`, `Oracle`, `LinkedLifecycleHook`) (`UnsupportedNft`): those could freeze,
  take or burn an item in custody, or veto its delivery.

Effects as for EthNft: one item recorded (`Item.key` = the mint / asset), the held marker set, one `ONE` minted to `dest`,
the item pushed to the pending draw set. Event `Wrapped`.

## 3. The canister's buying path

Unchanged in the program; only the marketplace differs:

1. `advance(amount)` — one convertible unit in transit, `amount ≤ max_item_cost` lamports to the custodian (its wallet);
2. **buy the cheapest member** keyless into the custodian wallet: Tensor (TComp listings, TSWAP/TAMM pools) or Magic Eden
   (M2 listings, `mmm` pools). For a pNFT the marketplace program performs the Token Metadata transfer to the canister's
   ATA under the rule set; for Core, an mpl-core transfer;
3. `mint_item` with `dest = module_wx` and the remaining accounts above (custody checked on chain), then `settle_advance(cost)`
   — both may go in one Solana transaction. The older pay-on-arrival path (`item_arrived(cost)`) works the same way.

If the purchase fails: `return_advance`, unchanged.

**Holder deposits**: off at launch, as for Ethereum and Bitcoin (open question Q7 of `solana-migration.md`: "first
post-launch release"). When they open, they go through the custodian as for the other kinds: the holder sends the NFT to
the canister's wallet and the canister calls `mint_item` with `deposit = true` and the holder's wX account as `dest`. The
program already allows it; the canister decides.

## 4. Redemption: tickets, delivered wallet to wallet

`redeem(destination)` (anyone, unchanged accounts): a SolNft listing takes only the new **`Destination::Sol(Pubkey)`**
(appended as variant 3; the `Ticket` size is unchanged, 241 bytes):

- `Sol(a)` with `a ≠ 0` (`BadDestination`) and `a ≠ listing.custodian` (`CustodyDestination`, as A-M-2 / A2-03);
- `Burn`, `Eth`, `Taproot` → `BadDestination` (no burn address on Solana; a burn would need the custodian to burn through
  Token Metadata or Core — not offered). `Sol(_)` on an EthNft or Inscriptions listing → `BadDestination`.

The ticket is pinned, proven and resolved by drand exactly as for the other kinds (`resolve`), then delivered:

**`record_delivery(tx)`** (custodian, unchanged accounts) **plus `remaining_accounts`** for a SolNft listing:

| Listing standard | `remaining_accounts[0]` |
|---|---|
| `Legacy` / `Programmable` | the destination's token account for the item's mint (its ATA): `owner == destination`, `mint == Item.key`, `amount == 1` |
| `Core` | the asset (`== Item.key`): owned by mpl-core, `owner == destination` |

Otherwise `NotInCustody` (`MissingAccount` if absent). **The canister sends the transfer and `record_delivery` in the same
transaction** (ATA `CreateIdempotent` if needed → TM `TransferV1` / SPL `Transfer` / Core `TransferV1` → `record_delivery`),
so the check always sees the NFT where the transfer left it and nobody can move it in between. `note_sent`,
`mark_undeliverable`, the deadlines and `refund` are unchanged.

## 5. Risks that stay (check per collection before listing it)

| Standard | Risk |
|---|---|
| `Programmable` | The rule set governs the canister's own transfers too; wallet-to-wallet is the case every Metaplex rule set allows. A collection with a custom rule set must be read before listing. |
| `Core` | The collection's update authority can add a `Royalties` rule set or an `Oracle` adapter later that refuses some transfers (only permanent plugins are fixed at creation and checked on chain). Wallet-to-wallet transfers are the case they normally allow. |
| all | The custodian is trusted as for EthNft: it may move an item out of its wallet without a ticket. The on-chain checks make it impossible to mint wX for an item it does not hold, or to record a delivery that did not happen. |
| all | cNFTs refused. Supporting them needs Bubblegum proofs; not planned. |

## 6. Treasury: 64 collections

`MAX_COLLECTIONS` 32 → 64: `collections` (Vec, max 64), `round_weights: [u64; 64]`, `round_paid: u64` (one bit per index),
and two fields appended after `total_to_raffles`: `weigh_next: u8`, `weigh_total: u64`. **2,778 bytes** (was 1,485), rent
0.02022576 SOL; created in one instruction (under the 10 KiB PDA limit).

A transaction locks at most 64 accounts, so `begin_distribution` (every module in one instruction) fits up to 61 modules
(treasury + fee payer + program + 61, with an address lookup table: a legacy transaction carries about 30 keys). For more,
the paged form:

**`weigh_distribution(start: u8)`** (anyone) — accounts: `treasury` (w); `remaining_accounts` = modules
`collections[start .. start + k]` in order (read-only), `k ≥ 1`, `start + k ≤ len` (`BadInput`), each the registered one
(`NotCollection`).

- `start == 0` begins a weighing: refused (`RoundOpen`) while a round is open or another weighing is younger than
  `ROUND_TIMEOUT` (1 day), unless they are `ROUND_TIMEOUT` old; clears the round (`round_total = 0`: nothing is owed while
  weighing) and sets `round_started_at = now`;
- `start > 0` continues: a weighing must be in progress (`NothingPending`) and `start == weigh_next` (`OutOfOrder`);
- each module's `volume_7d(now)` becomes its weight; `weigh_next = start + k`;
- after the last module: `round_pot = reserves_pot`, `round_total = weigh_total` (`NothingToDo` if 0), `round_paid = 0`,
  `weigh_next = 0` — the round is open exactly as `begin_distribution` opens it.

`begin_distribution` cancels a weighing only once it is `ROUND_TIMEOUT` old. `distribute(index)` is unchanged.

The canister: with ≤ 61 collections nothing changes; above, `weigh_distribution(0)` with the first page and further pages in
order (16 modules per legacy transaction, ~53,700 CU for 16), then `distribute` per index.

rook_backing has no per-collection cap (its 8-entry `configs` list is DBC curve configs).

## 7. Invariants

Unchanged: `supply == (held − open_tickets) · ONE + eth_backed`; the module vault holds its books. Added: every held SolNft
item was, at its mint, a verified member owned by the custodian; every delivered one was, at its delivery, owned by the
ticket's destination.

## 8. Errors (appended after `RaffleNotExpired`)

`NotAnNft`, `UnsupportedNft`, `NotInCustody`, `MissingAccount`. No new event (`Wrapped`, `Redeemed`, `Delivered` carry it).

## 9. Canister decoder changes (`autobuyer/canister-rs/src/solread.rs`)

- `LISTING_BYTES` 284 → 285; `ListingKind::SolNft { collection, standard: NftStandard }` (variant 2);
- `Destination::Sol(Pubkey)` (variant 3);
- `TREASURY_BYTES` 1,485 → 2,778, `MAX_COLLECTIONS` 64, `round_paid: u64`, then `weigh_next: u8`, `weigh_total: u64`.

## 10. Measurements (LiteSVM, 2026-10-10)

| What | Value |
|---|---|
| rook_core size (SBPF v2 build) | 1,132,880 bytes, +41,160 over HEAD (1,091,720) ≈ +0.29 SOL of program rent (6,960 lamports/byte) |
| rook_core size (`anchor build`, SBPF v3) | 1,067,856 bytes |
| `mint_item`, Legacy (custody + membership read) | 54,895 CU |
| `mint_item`, pNFT | 45,898 – 50,398 CU |
| `mint_item`, Core (with the plugin checks) | 43,291 CU |
| ATA create + SPL transfer + `record_delivery` (Legacy) | 46,977 CU |
| TM `TransferV1` (pNFT, foundation / compatibility / no rule set) + `record_delivery` | 132,624 / 128,124 / 142,627 CU |
| Core `TransferV1` + `record_delivery` | 21,702 CU |
| `weigh_distribution`, 16 modules | 53,700 CU |

Tests: `programs/rook_core/tests/suite/solnft.rs` against the real Token Metadata, Token Auth Rules and mpl-core programs
and the Foundation and Compatibility rule set accounts, dumped from mainnet into `solana/tests/fixtures`
(`mpl_core.so`, `mpl_token_auth_rules.so`, `rule_set_metaplex_*.json`; `mpl_token_metadata.so` is byte-identical to
mainnet's today).
