# v9: more Ordinals collections at launch (listings 12-25)

Status: **data and launch tooling done, canister support not built**, 2026-10-10, branch `golive-2026-10-08`. Listings 12-16 come
from the first pass. Listings 17-25 were appended the same day by operator decision (§8). Their minimum purchase needs a canister
change, described in §7. Graduation tiers are in `graduation-tiers.md`. Nothing was signed or sent. The only network calls were
read-only and keyless:
- Ordinals Wallet's `turbo` API (collection, stats, escrows, sales, inscriptions);
- ordinals.com (gallery pages, `/r/children`, `/rune/<name>`);
- CoinGecko's NFT API;
- GitHub raw files (TheWizardsOfOrd/ordinals-collections, ordinals-wallet/ordinals-collections);
- Coinbase tickers;
- one Solana mainnet `getMinimumBalanceForRentExemption`;
- for §7, Relay's `POST /quote/v2`, with no key and throwaway addresses.

Magic Eden's ordinals API answered `no healthy upstream` all day. UniSat's collection index refuses unauthenticated calls.

**Legend.** **[M]** measured on 2026-10-10. **[R]** read from this repository. **[A]** assumption or estimate.

## 0. Summary

1. **Five Ordinals collections join Bitcoin Frogs as rook_core `Inscriptions` listings 12-16, in this fixed order:**

   | id | slug | collection | wX (name / symbol) | members | members root |
   |---|---|---|---|---|---|
   | 12 | `nodemonkes` | NodeMonkes | Wrapped NodeMonke / `wNODEMONKE` | 10,000 | `0xe97c65d4…10126c09` |
   | 13 | `omb` | Ordinal Maxi Biz (OMB) | Wrapped OMB / `wOMB` | 9,000 | `0xb174d1b2…98dd7af6` |
   | 14 | `bitcoin-puppets` | Bitcoin Puppets | Wrapped Bitcoin Puppet / `wPUPPET` | 10,001 | `0x1aa1697e…2de25d47` |
   | 15 | `bitcoin-punks` | Bitcoin Punks | Wrapped Bitcoin Punk / `wBPUNK` | 10,000 | `0xcf39e608…fe91fda4` |
   | 16 | `natcats` | Natcats | Wrapped Natcat / `wNATCAT` | 8,060 | `0xe5a905f7…e0b925a3` |

   **Then, by operator decision of 2026-10-10 (§8), listings 17-25, most liquid first:**

   | id | slug | collection | wX (name / symbol) | members | members root | min purchase |
   |---|---|---|---|---|---|---|
   | 17 | `runestone` | Runestone | Wrapped Runestone / `wRUNESTONE` | 112,384 | `0x623653a2…89ee7752` | **0.25 SOL** |
   | 18 | `bitcoin-weirdos` | Bitcoin Weirdos | Wrapped Bitcoin Weirdo / `wWEIRDO` | 10,000 | `0xd2a7b78c…d008d76c` | **0.15 SOL** |
   | 19 | `cents` | CENTS | Wrapped CENTS / `wCENTS` | 10,000 | `0xa12652ab…5ce4849` | 1 SOL |
   | 20 | `rsic` | RSIC Metaprotocol | Wrapped RSIC / `wRSIC` | 20,998 | `0x5c441d67…25d82fee` | **0.25 SOL** |
   | 21 | `pizza-ninjas` | Pizza Ninjas | Wrapped Pizza Ninja / `wPIZZA` | 2,920 | `0x230aa6d2…cc847c55` | 1 SOL |
   | 22 | `quantum-cats` | Quantum Cats | Wrapped Quantum Cat / `wQCAT` | 3,313 | `0xb4c59b4a…03163094` | 1 SOL |
   | 23 | `wizards-of-ord` | The Wizards of Ord | Wrapped Wizard of Ord / `wWOO` | 3,333 | `0xee49af72…5d04a7d9` | 1 SOL |
   | 24 | `taproot-wizards` | Taproot Wizards | Wrapped Taproot Wizard / `wTAPWIZ` | 2,102 | `0x6ebe6bf0…a57a9b14` | 1 SOL |
   | 25 | `bitcoin-wizards` | Bitcoin Wizards | Wrapped Bitcoin Wizard / `wBWIZ` | 1,337 | `0x53daf6fc…1f56fb8c` | 1 SOL |

   The full roots are in `deployments/launch/nft-pilot.json` `inscriptions[]` and `ops/solana/day1-pins.json`.
2. **Every one gets its day-1 DBC config with the others:** 26 configs and ROOK, **27 transactions**. That is **0.182161800 SOL**:
   - 0.128305080 for 17 configs and ROOK;
   - plus 9 × 0.005984080 (a config's rent, 0.005974080, and two signatures).

   The graduation tier changes the parameters, not the account size, so it does not change this cost. The listing ceremony grows to
   ≈ **1.41 SOL** (26 × 0.05394708, plus the treasury, the desk and fees). That is up ≈ 0.49 SOL from ≈ 0.93.

   The public fund target is the deploy plus the listing ceremony. With `fund-status.mjs`'s current program sizes it moves from
   ≈ 11.61 to ≈ **12.10 SOL**: deploy 10.69 + listing 1.41. The deploy part grew from ≈ 9.87 with the raffle agent's rook_core size;
   that agent owns those constants. Treasury slots used: 26 of `MAX_COLLECTIONS` 32.
3. **Wrapping waits for the canister.** The canister operates one Inscriptions wrapper (Bitcoin Frogs, §4). All fourteen are listed,
   their wrapper, module and partner created, and their coins launch from day 1. No floor is posted for them, so there is no wX
   mint and no advance, until the canister's multi-listing change ships and each collection passes the wake check (§5). Their
   backing share waits in DBC like every collection's before the deploy.
4. **Dropped by the first pass:** Runestone, Taproot Wizards, Quantum Cats, Bitcoin Wizards, The Wizards of Ord, Pizza Ninjas,
   Ordinal Punks and CENTS (§2 gives the reasons). The operator then added all of them except **Ordinal Punks**, which stays out
   (100 items, one sale in 30 days, nothing on OW). The operator also added RSIC and Bitcoin Weirdos. **Nothing the operator named
   was left out.**
5. **Minimum purchase.** Runestone, RSIC and Bitcoin Weirdos have floors under the canister's 1 SOL minimum. Each entry records a
   `minPurchaseSol`. The canister change is in §7 and is not built.
6. **Wake check (20 % band) today: none of the nine new collections passes** (§8). Of the first five, only OMB passed (§2).

## 1. What a listing commits on day 1

A collection's DBC config pays `claimer(listing(i))` for a listing id `i` that exists only after the deploy. Ids are given in
`list_collection` order, so the order is committed by the day-1 configs (`launch-day1-solana.md` §2). The five are **appended**
after Bitcoin Frogs:
- No existing id moves.
- The Ethereum collections stay 0-10 and Bitcoin Frogs stays 11.
- `nft-pilot.json`'s single `bitcoin` object is unchanged, so every EVM-era reader still finds exactly one: golive-check,
  golive-dryrun, audit2_golive_check, the monitor's launch config test, and dormant.test's 11 collections.

The new key `inscriptions[]` is read by every Solana-side order builder:
- `ops/solana/launch-params.mjs` `listingOrder()`;
- `rook-ceremony` `listing_order_from_pilot` (the pins and the order checks);
- `ops/solana/site-config.mjs` `listingOrder()`;
- `ops/test/audit_a6_day1_verify.py`;
- the site (`web-rook/src/rook/sol/collections.ts`, `src/config/solana.ts`).

`ops/solana/day1-pins.json` was regenerated with `rook-ceremony pins --write`:
- `listing_order` now has 26 entries, each with its `graduation_sol` (`graduation-tiers.md`); `listing_order_sha256` is
  `6faf8ed9…c74680`.
- The collection and ROOK parameter pins are unchanged (collection `8db45a9b…53ae1c`, ROOK `c50f8c33…a5fe96`).
- The new `tiers.<sol>` pin the 170 / 255 / 510 SOL configs.

`check_listing_order` now also refuses an order where anything but an Inscriptions listing follows Bitcoin Frogs, or Bitcoin Frogs
is not the first Inscriptions listing.

Limits checked [R]:
- **Treasury.** `MAX_COLLECTIONS` 32 modules, so 26 leaves 6.
  - `begin_distribution` near 32 needs an address lookup table (`tests/audit_a2_core.rs:269`).
  - With 26 module keys plus the treasury, the payer and the program, a legacy transaction is ≈ 1.07 KB of the 1,232 [A]. That is
    close: check the canister's `solcrank` builder before adding more.
- **rook_backing.** `MAX_CONFIGS` 8 is per partner, which is per listing (one config each), not a total.
- **rook_core.** No listing-count limit (`listing_count` is a u32).
- **Membership proof size.** rook_core has no proof cap for `mint_item`. Proof lengths:
  - 11-15 hashes for every collection but Runestone;
  - **17 for Runestone** (112,384 leaves, 544 B). That fits one `mint_item` transaction, where ≈ 25 hashes fit beside its 11
    accounts.

  The canister's hand-in (`handin.rs` `MAX_PROOF_LEN`) takes at most 32 hashes. The canister stores proofs per inscription
  (`frog_proof`), not member lists.
- **Members file size.** `runestone-members.json` is 8.1 MB and `rsic-members.json` 1.5 MB; the others are ≤ 0.7 MB. The runner
  loads one file into memory, which is fine. The runner image copies only the Frogs file today (`ops/Dockerfile.runner`); the
  multi-listing runner (§4.6) will need these files added to it.
- **Name and symbol limits.** rook_core `MAX_NAME` 32 and Metaplex symbol ≤ 10 bytes. The longest are "Wrapped Taproot Wizard" /
  "Wrapped Bitcoin Weirdo" / "Wrapped Bitcoin Wizard" (22) and `wNODEMONKE` / `wRUNESTONE` (10). All 26 symbols are distinct, and
  no inscription id is in two collections (223,448 ids checked).
- **Canister cycles.** The reads at rest scale per listing: ≈ 0.12 T a day for 12, ≈ 0.17 T for 17, so ≈ 0.26 T for 26 [A]
  (`solana-migration.md` §2.6). A dormant Inscriptions listing costs nothing more once §4 ships: no floor reads, no scan.

## 2. The choice (market data of 2026-10-10)

**Criteria.** A collection stays only if it meets all three:
- **Liquid:** at least 30 single-item sales across venues in 30 days, from Ordinals Wallet's sales feed. That feed covers OW,
  Magic Eden, Satflow, OrdSwap, Gamma, UniSat and others.
- **Buyable by the canister:** at least 3 listings from at least 3 sellers on **Ordinals Wallet's book**. That book is the
  canister's buy venue and the keeper's floor feed, and `robustFloor` gives no floor with fewer than 3 valid listings.
- **A floor inside the purchase bounds:** at least `MIN_PURCHASE_LAMPORTS` 1 SOL and at most the 250 SOL Relay ceiling
  (`frogsol.rs`).

At 1 SOL = 132,730 sats (Coinbase BTC 82,791 USD, SOL 109.89 USD).

| collection | OW listed (sellers) | OW lowest 3 (BTC) | sales 30 d | median sale (BTC) | CoinGecko floor (BTC), holders | verdict |
|---|---|---|---|---|---|---|
| Bitcoin Frogs (11, for reference) | 87 (63) | 0.00375 0.00411 0.00432 | 97 in 23 d | 0.00295 (7 d) | 0.0040, 5,143 | listed |
| **NodeMonkes** | 8 (8) | 0.03274 0.03274 0.03279 | 99 in 26 d | 0.02422 (7 d) | 0.02318, 4,499 | **in** |
| **OMB** | 9 (8) | 0.01953 0.0296 0.0605 | 97 in 20 d | 0.01635 (7 d) | 0.0175, 5,184 | **in** |
| **Bitcoin Puppets** | 16 (15) | 0.01438 0.01439 0.01871 | 95 in 18 d | 0.01183 (7 d) | 0.01024, 6,021 | **in** |
| **Bitcoin Punks** | 90 (67) | 0.00421 0.00514 0.00514 | 69 | 0.00376 | 0.00314, 5,437 | **in** |
| **Natcats** | 33 (23) | 0.00399 0.00452 0.00718 | 30 | 0.00286 | 0.00284, 3,048 | **in** |
| CENTS | 26 (**5**) | 0.00851 0.00859 0.00953 | 54 | 0.00341 | 0.00515, 2,109 | out: the book is 5 sellers, 2.5× the median sale |
| Quantum Cats | **1** | 0.01901 | 23 | 0.00541 (7 d) | 0.0104, 1,638 | out: one OW listing (no floor) |
| Taproot Wizards | **0** | none | 16 | 0.027 (7 d) | 0.047, 1,297 | out: nothing to buy on OW |
| Bitcoin Wizards | 11 (10) | 0.0555 0.411 0.432 | **7** | 0.0203 | 0.0135, 903 | out: illiquid, the book is 4× CoinGecko's floor |
| The Wizards of Ord | 3 | 0.0154 0.0180 0.0514 | 18, none in 7 d | none in 7 d | 0.0054, 2,174 | out: illiquid |
| Pizza Ninjas | **0** | none | 22 | 0.00367 (7 d) | 0.00872, 1,082 | out: nothing on OW; OW 2,920 vs ME 2,022 vs CG 1,500 items disagree |
| Ordinal Punks | **0** | none | 1 | none | 0.599, 82 | out: 100 items, one sale |
| Runestone | 183 (126) | 0.00086 0.00086 0.00092 | 93 in 7 d | 0.00074 (7 d) | 0.00139, 63,743 | out: floor ≈ 0.65 SOL < the 1 SOL minimum |

On Runestone:
- It is the most traded collection, but every purchase would be under `MIN_PURCHASE_LAMPORTS`.
- Its membership is the 112,384 children of `fdb2df5d…cbb5i0` (depth-17 proofs, 544 B: they would fit).
- If the operator lowers the Solana minimum to 0.5 SOL, it can be appended later (id 17+).
- RSIC (median sale 0.00066) and Bitcoin Weirdos (0.00047) fail the same way.

Also checked and not chosen:
- Pixel Pepes: 5 sales in 30 d.
- Bitcoin Bear Cubs: 6.
- MegaPunks: 5.
- Honey Badgers, The Royals, Ordinal Penguins, BTC Machines, Astral Babes and Pingoos are thin.

**A gap to watch: Ordinals Wallet's book sits above the market for all five.** The OW lowest listing against CoinGecko's floor:

| collection | OW lowest vs CoinGecko floor |
|---|---|
| NodeMonkes | +41 % |
| OMB | +12 % |
| Puppets | +40 % |
| Bitcoin Punks | +34 % |
| Natcats | +40 % |
| Bitcoin Frogs | −6 % |

Most of the trading happens elsewhere, as bid fills and Magic Eden sales. The canister posts the OW-book floor. A posted floor
above the market lets a holder deposit an item bought cheaper elsewhere and burn the wX at the higher posted price, and makes the
canister buy dear. That is why the wake check (§5) requires the OW book to sit inside the monitor's D8 band (20 % of CoinGecko's
floor) before a collection's first post. On 2026-10-10 only OMB passes it.

## 3. Members, roots and provenance

**Generator.** `ops/solana/ordinals-members.mjs` (new; read-only).
- `fetch` takes the inscription ids from two independent sources:
  - Ordinals Wallet `turbo /collection/<owSlug>/inscriptions`, the canister's buy venue;
  - the collection's **on-chain ord gallery inscription** (`ordGallery` in nft-pilot.json, as named by TheWizardsOfOrd's curated
    `collections.json`), read page by page from `ordinals.com/gallery/<id>`.
- It keeps the ids **both** have, ordered by inscription number ascending, as `bitcoin-frogs-members.json` is.
- References are compared and recorded, but are not part of the agreement:
  - Magic Eden's legacy export (TheWizardsOfOrd `legacy/collections/<slug>.json`);
  - Ordinals Wallet's 2024 GitHub registry.
- `<members>.provenance.json` records every URL, count, and the ids each source alone had.
- `root` prints a file's count and root. `check` recomputes every Inscriptions entry of nft-pilot.json, Frogs included.

**The tree is Frogs'.** Leaves in list order, `keccak256(keccak256(id))`, sorted pairs, an odd node moves up. This is rook_core
`hashing.rs`, `ops/src/frogmath.ts` `proofFor` and `script/Deploy.s.sol`. The method was checked against Bitcoin Frogs:
- The OW list, Frogs' on-chain gallery (`659e684f…5a62i0`) and Magic Eden's export all give exactly the committed 10,000 ids.
- The generator reproduces `0x801f71bb…acfae418e`.

| collection | OW | on-chain gallery | agreed (members) | left out | references |
|---|---|---|---|---|---|
| NodeMonkes | 10,000 | 10,000 (`2ebbd9b9…ba04i0`) | 10,000 | none | ME 9,985 (15 missing, none extra) |
| OMB | 9,001 | 9,000 (`557a0782…9bb4i0`) | 9,000 | 1 OW-only id | ME 8,998, OW GitHub 2,094 (stale) |
| Bitcoin Puppets | 10,001 | 10,001 (`4dc81b56…6adfi0`) | 10,001 | none | ME 10,001 (identical) |
| Bitcoin Punks | 10,000 | 10,000 (`d4d69e8d…040di0`) | 10,000 | none | OW GitHub 10,000 (identical) |
| Natcats | 8,060 | 8,064 (`dca25d6d…3428i0`) | 8,060 | 4 gallery-only ids | ME 8,063 (has the same 4; lacks 1 member) |

**Rule: an id only one source has is left out, never added.** A member nobody else recognises is what a holder deposit could abuse.
An item left out cannot be wrapped. Adding it later is a rook_core root change: `propose_root`, a 7-day wait, `apply_root`. Before a
collection wakes, the operator reruns `fetch` and diffs it (`deployments/launch/btc/README.md`).

**Price references, as Frogs'.**
- `owSlug`: the keeper's book and the runner's protected-listing step.
- `coingeckoId`: the monitor's D8 reference. `nodemonkes`, `ordinal-maxi-biz-omb`, `bitcoin-puppets`, `bitcoin-punks`, `natcats`
  are all listed on CoinGecko [M].
- `unisatCollection` is `null` until it is confirmed with the UniSat key.

**Caps** (named at the wake, not at the install):
- `custodyItems` 3 and `custodyWei` 10 ETH: Frogs' stage 0. At the 2026-10-10 floors, 3 NodeMonkes ≈ 3.3 ETH.
- `maxPurchaseLamports` (`set_purchase_limit` on the module): max(10 SOL, ≈ 1.5 × the OW floor), at most 250 SOL. That is:

  | collection | limit |
  |---|---|
  | NodeMonkes | 40 SOL (floor ≈ 24.7) |
  | OMB | 25 SOL (≈ 14.7) |
  | Puppets | 20 SOL (≈ 10.8) |
  | Bitcoin Punks | 10 SOL (≈ 3.2) |
  | Natcats | 10 SOL (≈ 3.0) |

  Every one of these floors is above the 1 SOL minimum.
- The exit rule (`outflow::exit_fits`, every item out within the timelock's 72 h less a day) must be re-sized at each wake. Each
  awake Ordinals collection adds 3 items to the global, so `global_items` × 2 must cover it.

## 4. The canister: what it supports today, and the change

**Built (2026-10-12, on the books home).** The seven parts below were written for the programs era; on the books home there are no
programs, and a collection is a books listing of kind `ListingKind::Inscriptions { membership_root, custody }` pinned through
`coins_pin`. They landed in `autobuyer/canister-rs` (`src/frogsol/multi.rs` is new) as follows:
1. **Init → the books.** No `Init.inscriptions`: every Inscriptions listing in the books is a frog collection, keyed by its books id
   (`u16`). `Init.wrapper` stays unused on the books home, and the tick runs the frog side and the custodian without it. Each
   collection's world names its listing by its wX mint (`frogsol::books::coll_for`). A world that names none, while the books hold
   exactly one Inscriptions listing, is served by that one (the single-collection world of before). `frog_funding` stays global.
2. **State.**
   - The per-collection parts (`frogs`, `proofs`, `convs`, `protected`, `ow_creators`, the floor reading, `values`, the floor sink's
     view) live in `FrogState.colls`, one entry per books id.
   - Redemptions live in `Custody.colls`: the cursor, the redemptions and the outbound key `ticket:<coll>:<n>`, since tickets are
     numbered per listing.
   - Kept single, custody-wide: `seen`, `unmarked`, the deposits and their clocks, the fee pool, the float, the one taproot custody
     address, `next_frog` (frog ids stay unique), the floor sinks as configured, the one Solana line, the one Relay leg and the
     Relay orders already used (rule S12 is checked against every collection's).
   - Custody classifies each new output once per tick (`multi::classify` / `route` / `apply`):
     - a purchase goes to its collection;
     - an output with one inscription goes to the live collection whose root it proves against; every proof handed in is tried
       against every live root;
     - a non-member of every root is refused once, at the top, never once per collection.
   - `frog_proof` names its collection and is checked against that root at hand-in. That is why the runner knows the members file
     it proved from, and an id is in no two collections (launch-params test).
   - Fee inputs are reserved across collections (`CustodyIo::reserved_elsewhere`).
   - Stable memory: both maps are `#[serde(default)]` inside the existing upgrade elements. `post_upgrade` (and lazily the first
     use) moves the single state into the entry of the books' first Inscriptions listing, the one the single-collection code
     served. That collection stays live. Candid encodes the recursive maps.
3. **The tick.**
   - The frog pass loops over the live Inscriptions listings, one `ItemIo` / `CustodianIo` per collection (`for_coll`), with
     `next_frog` taken from the top and written back.
   - Book outcomes on the line name their collection (`books:<id>`), and each collection's `resolve` takes only its own
     (`frogsol::owns`).
   - One collection's idle pass keeps another's queued frog turn.
   - The custodian loops over every Inscriptions listing, dormant ones included, since a holder's exit is never stopped.
   - Shared and unchanged: the Relay leg (B-L-6, `held_elsewhere`, read by both the Ethereum side and every other collection), the
     outbound limiter and the A4-L-1 exit rule.
4. **Endpoints.**
   - A leading `coll : nat16` on `frog_floor_reading`, `frog_proof`, `set_ow_creators`, `probe_frog`, `frog_offer`,
     `frog_offer_protected` and `relay_give_back`.
   - The views `frog_buys`, `frog_conversions`, `item_sinks` and `unmarked_custody` take `opt nat16`; null reads every collection,
     so an old caller still decodes.
   - `relay_quote`, `relay_arrived_exit`, `relay_late_release` and `frog_protected_signed` find the collection by the frog id.
   - New: `frog_collections` (query), `frog_wake` and `frog_sleep`.
   - The Candid names are unchanged; the `EXEMPT` list is unchanged.
5. **Dormancy** (superseded by default-live, below: a listing with no life is LIVE).
   - `dormancy.rs`'s `Life` is kept per listing (`FrogState.dorm`, keyed by the wX mint's first 20 bytes). As first built, a listing
     with no life was DORMANT, not always-on.
   - A dormant listing gets no floor reading (`frog_floor_reading` refuses), no book read and no buy (the tick skips it).
   - The operator's `frog_wake(coll)` makes a listing ACTIVE at once, in the `eth_wake` style; the wake desk is dropped (O-11).
   - `frog_sleep(coll)` is refused while anything of the collection is open.
6. **Custody caps.** The default caps (`caps_of`) name each listing at stage 0 (`sol:<wX mint>`, 3 items, 10 ETH): as first built
   each LIVE listing; since the C2 fix round (C2-08) EVERY Inscriptions listing in the books, live or asleep. The global is re-sized
   by the exit rule: 15 listings name 45 items, and stage 0's limits let 12 leave, so the global is 12. Caps that the install names
   explicitly stay the controller's statement.
7. **Runner.** The runner (`ops/`, not part of this change) now sends the collection with every call above. Built in the
   integration (2026-10-12): it reads `frog_collections()` and pairs each collection with its `nft-pilot.json` listing by members
   root; every LIVE one gets `frog_floor_reading(coll, …)` inside §5's 20 % band of CoinGecko (`src/solfloor.ts`; out of the band the
   reading is held), a holder deposit awaiting its proof gets `frog_proof(coll, …)` from that collection's members file
   (`src/inscriptions.ts`), and the buying steps run per `RUNNER_INSCRIPTIONS=<slug>=<owSlug>:<membersFile>[:<unisat>],…` entry
   (the old scalars read as Bitcoin Frogs'), one Bitcoin hand-in per cycle across them. The offer tool takes `--coll`.

**Default-live and the C2 fix round (operator decision 2026-10-12; `docs/v9/audit-2026-10/c2-ordinals-multi.md`).** No collection is
picked or woken: every Inscriptions listing starts LIVE, and a collection simply cannot mint until a floor reading is accepted (a
module never posted wants nothing: `books_sink_view`'s `open = accepted > 0`).
- **The life cycle flipped.** A listing with no life is live (`multi::is_live`), so `live`, `live_cap_keys` (now `listing_cap_keys`,
  every listing) and the classification's roots all read the new default. `frog_sleep(coll)` (the operator's per-collection brake)
  gives a listing a DORMANT life; `frog_wake(coll)` only lifts that brake. The ceremony calls neither.
- **What the wake check waited for is set from the launch files**, before a listing's first reading is posted and only where nothing
  is set (`multi::launch_defaults`, the table `frogsol::launch::LAUNCH`, held to `nft-pilot.json` and `graduation-tiers.json` by a
  test): its floor sink (its module, derived by the canister), its purchase limit and minimum (`maxPurchaseLamports`,
  `minPurchaseSol`; Bitcoin Frogs: the home's 10 SOL / 1 SOL), its Ordinals Wallet creators (none pinned: a protected sale with a
  royalty is refused, P-01's default). A controller's `set_floor_sink` / `set_purchase_limit` / `set_ow_creators` still win.
- **The first floor is bounded** (C2-12): with no accepted floor to clamp it to, a listing's first reading is refused unless it is
  within ±50 % of the floor its launch pin was sized from (`graduation-tiers.json` `floor_sol`, `launch::first_floor_ok`); later
  readings are clamped by the module's move bound as before. A listing whose members root no launch file names posts no first floor.
- **The fix round's other parts:** `sweep_stale` releases only its own collection's reservations and an arrival is always booked
  (C2-01); `frog_sleep` is refused while the collection holds the Relay leg or has an unread outcome on the line (C2-02); book outcomes
  are trimmed on their own count, never a landed advance's (C2-03); one Solana world (clock, rates) per tick for every listing's
  custodian and frog side (C2-04: a quiet tick with 15 listings makes one clock read, 4 outcalls, not about 120); an unreadable custody
  output is set aside after three tries, never stopping the classification (C2-05); the outputs refused custody-wide are routed again
  when a proof is handed in or the roots change (C2-06); the collections take their passes in a rotation and the Ethereum side's turn
  on a free leg is kept once per leg (C2-07); the caps are compared at an upgrade after the books and the frog state are restored
  (C2-08); a listing repeating a lower id's members root is never live (C2-14).

§7's minimum purchase is built with it:
- `PurchaseLimit.min_wei` (`#[serde(default)]`).
- `set_purchase_limit(sink, max, opt min)`: Solana home only, `min` within [0.15 SOL, 1 SOL] and at most `max`.
- `check_limits_min` / `bounds_min` at the advance (`convflow`) and at `cost_checked`.
- The dust floor is `ABS_MIN_PURCHASE_LAMPORTS` = 0.15 SOL, the smallest order Relay was measured filling, not the 0.1 SOL
  proposed below. A SOL to BTC deposit under 0.15 SOL less the gas keep is refused (`RELAY_SOL_BTC_MIN_LAMPORTS`).
- Tests: `frogsol::multi::tests` and the `multi_*` tests in `tests/frogs_sol.rs`. The fixtures are real roots and proofs
  (`tests/fixtures/ordinals/multi.json`).

The text below is the original design, kept for the record.

**Today [R]. One Inscriptions wrapper.**
- `Init.wrapper : opt text` (`etch_autobuyer.did:11`) is the single wrapper. On the Solana home it is Bitcoin Frogs' Wrapper PDA.
- The tick runs the ordinals side for `cfg().wrapper` only (`canister.rs:1730`, `:1792`, `:1803`).
- `frogsol.rs` derives one root, one module, one custody-cap key and one floor reading from `io.sol_wrapper()`.
- `FROGS` is a singleton `FrogState`, and its floor reading is one `Option`.
- `RedeemState` has one ticket cursor.
- The endpoints `frog_floor_reading`, `frog_proof`, `set_ow_creators`, `probe_frog`, `frog_offer(_protected)` take no listing.
- The runner (`ops/src/runner.ts`) has one `RUNNER_OW_SLUG` / `RUNNER_OW_MEMBERS` / `RUNNER_UNISAT_COLLECTION` (since replaced:
  part 7 of "Built" above).

**Already generic [R]:**
- the Solana programs;
- `solcrank` (it reads every listing up to `listing_count`, and every vault and partner);
- the custody-cap book (`CapPolicy.collections` is a list keyed `sol:<wrapper>`);
- the OW and ord buy judges (`owbuy`, `frogbuy`), which hold no slug: membership is a proof against a root;
- `solread::Listing`.

**The change** (not built: it touches custody, deposits and redemptions, which are audited code; estimate 1.5-2.5 weeks with its
tests and an audit pass). There are seven parts:

1. **Init.** Add `inscriptions : opt vec record { wrapper : text; ow_slug : text }` beside `wrapper`, which stays as entry 0 for
   back-compatibility. `validate_frog_home` and `wrapper_key` iterate over the entries. `frog_funding` stays global.
2. **State.**
   - Per-listing parts are kept per wrapper: `frogs`, `proofs`, `convs`, `protected`, `ow_creators`, `floor_reading`, `values`, and
     the redemption cursor.
   - **Custody-wide parts stay single:** `seen`, `unmarked`, `deposits`' classification, fee and float UTXOs, `sol_step` on the one
     Solana line.
   - All listings share the canister's one taproot custody (the ceremony's `btc_custody` default). Custody classifies an output
     **once**, and routes it to the wrapper whose root its inscription proves against. A non-member of every root is refused once,
     not once per wrapper. A per-wrapper copy of the classification would refuse, and return, another collection's deposit.
   - Stable memory: the upgrade tuple's element gains `#[serde(default)]` maps, migrated from the single state into entry 0.
3. **The tick and the Io objects.** Loop over the wrappers. `ItemIo::new(wrapper)` / `CustodianIo::new(wrapper)` already take
   one. `frogsol::view`, `collection_key`, `root`, `read_sink` and `identity` then follow `io.sol_wrapper()`.
4. **Endpoints.** Add a wrapper or listing-id argument to `frog_floor_reading`, `frog_proof`, `set_ow_creators`, `probe_frog`,
   `frog_offer(_protected)`, `relay_give_back`, and the state views. Keep `next_frog` global so frog ids stay unique.
5. **Dormancy for Inscriptions.** Reuse `dormancy.rs`'s life cycle, which covers EthNft only today. A dormant Inscriptions listing
   gets no floor reading, no book read and no buy. Its wake is the operator's (§5) until a bonded wake exists on Solana.
6. **Runner.** Replace the scalars with `RUNNER_INSCRIPTIONS=<wrapper>=<owSlug>:<membersFile>[:<unisat>],…`. Loop the protected
   and UniSat steps. Add the missing Solana floor hand-in per listing: nothing in `ops/src` calls `frog_floor_reading` today, only
   the devnet rehearsal `drive.py` does.
7. **Custody caps.** One `CollectionCap { collection: "sol:<wrapper>" }` per awake listing, with `global_max_items` re-sized by
   the exit rule.

None of this touches rook_core or rook_backing. Until it ships, listings 12-16 behave as dormant collections:
- **No floor post.** `mint` and `burn` refuse a stale module (`keeper_down`), and nothing is advanced.
- **Coins launch on their configs.** DBC fees and the 42.5 SOL migration fee accrue to the listing's claimer PDA.
- **`back` waits** for a live module.

The site marks them `wrapping: "later"` (`web-rook/src/rook/sol/collections.ts`) and its docs say so.

## 5. Waking one (after §4 ships)

**Superseded (2026-10-12): default-live (§4).** No collection waits for a wake: the purchase limit, minimum and creators come from the
launch files, the cap from the books, and the first floor is bounded by the launch floor in the canister; the runner's 20 % CoinGecko
band (item 2) still gates every reading it hands in. The list below is kept for the record and as the checklist for a collection the
operator puts to sleep and wakes again.

A collection's first floor post needs all of the following:
1. The members file reproduced: rerun `ordinals-members.mjs fetch` and diff it. A changed set is a root change (7-day wait), and
   the first post waits for it.
2. The OW book has ≥ 3 valid listings (`robustFloor`), and its floor is within **20 %** of CoinGecko's. This is the monitor's D8
   band, which would pause the collection otherwise.
3. `set_purchase_limit(<module>, maxPurchaseLamports)`, `set_ow_creators(<wrapper>, …)` (protected-listing review P-01), its
   `CollectionCap`, and `outflow_limits` re-sized by the exit rule.
4. `unisatCollection` confirmed with the key, or the UniSat step off for that collection.
5. The monitor config gains the collection's `price_refs` (OW + CoinGecko) and its expected custody inflows.

## 6. Files

- `deployments/launch/nft-pilot.json`: `inscriptions[]` and `_inscriptions`.
- `deployments/launch/btc/*-members.json` and `*.provenance.json`, plus the README.
- `ops/solana/ordinals-members.mjs` (new).
- `ops/solana/launch-params.mjs`, `site-config.mjs`, `fund-status.mjs` (`LISTINGS` = the order's length), `day1-pins.json`
  (regenerated), and the `deploy-later.sh` comment.
- `ops/rehearsal/sol-devnet/tool/src/ceremony_checks.rs` (the order from `inscriptions`, the stricter order check, tests),
  `ceremony.rs` (tests: 17 listings, claimer PDAs of ids 12 and 16), and the `ceremony_list.rs` comment.
- `ops/test/launch_params.test.ts` (new test: order, limits, every root recomputed, no id in two collections),
  `audit_a6_day1_verify.py`, `audit_a6_launch_tamper.sh` (frogs-first now moves id 11; a swapped pair of Ordinals listings is
  refused).
- web-rook:
  - `src/rook/sol/collections.ts` (5 rows, `wrapping: "later"`);
  - `src/config/solana.ts` (17 rows, rendered by `site-config.mjs`);
  - `src/rook/sol/fund.ts` (listings = `COLLECTIONS.length`);
  - `src/rook/sol/demo.ts` (a demo coin on each new collection);
  - copy ("seventeen", ≈ 0.9 SOL to list, ≈ 10.8 SOL target);
  - `src/content/docs/01-what-is-rook.md` and `04-backing.md`;
  - tests (`sol`, `demo`, `tracked`; the day-1 plan fixture gains five rows, derived as rook-ceremony derives them).
- Docs: `launch-day1-solana.md` (order, costs, payer amount), `solana-migration.md` (cycles), `rook-core-as-built.md` (symbols).

## 7. Minimum purchase below 1 SOL: the canister change needed, and Relay at small sizes

**Today [R].** `autobuyer/canister-rs/src/frogsol.rs:37` `MIN_PURCHASE_LAMPORTS = 1_000_000_000` (1 SOL, decision D-b) is one
global constant. Three things use it:
- `frogsol::bounds(true)`, then `frogsol::check_limits(cost, limit, solana)`. That is called by `convflow.rs:386` (the advance) and
  `frogflow.rs:1434-1439` (`cost_checked`, where a cost under the minimum is `Why::Gone`).
- `floornet::set_purchase_limit`, through `FloorState::set_limit_in(sink, max, min, ceiling)` (`floortrigger.rs:114`). A limit
  under the minimum is refused.
- `launch-params.mjs` `FIXED.frog_purchase` (review only).

So the canister would refuse every purchase of a Runestone (floor ≈ 0.65-1.04 SOL), a RSIC (≈ 0.56 SOL by CoinGecko) or a Bitcoin
Weirdo (≈ 0.38-0.49 SOL).

**The change** (not built in this task; it touches audited purchase rules). There are four parts:
1. `floortrigger.rs` `PurchaseLimit { sink, max_wei }` gains `min_wei: Option<u128>` with `#[serde(default)]`, so stable memory
   upgrades without migration. `FloorState::min_for(sink) -> Option<u128>` sits beside `limit_for`.
2. `floornet::set_purchase_limit(sink, max, min: opt nat)` stays controller-only (a timelocked act). It accepts a `min` in
   `[ABS_MIN_PURCHASE_LAMPORTS, MIN_PURCHASE_LAMPORTS]`, proposed as 0.1 SOL .. 1 SOL, with `min ≤ max`. Add
   `pub const ABS_MIN_PURCHASE_LAMPORTS: u128 = 100_000_000` to `frogsol.rs` as the dust floor that no collection goes under.
3. `frogsol::check_limits(cost, limit, min, solana)` and `bounds()` use `min.unwrap_or(MIN_PURCHASE_LAMPORTS)`. The two call sites
   pass `fl.min_for(&module)`:
   - `convflow::advance` (`let limit = fl.limit_for(&c.module)` is already there);
   - `frogflow::cost_checked`: its `limit` comes from the same `FloorState`.

   The EVM home is unchanged: `crate::frogbuy::check_limits_for` and `policy::MIN_PURCHASE_WEI`.
4. Tests:
   - `check_limits` with and without a collection minimum;
   - `set_purchase_limit` refusing `min < 0.1 SOL`, `min > 1 SOL` and `min > max`;
   - an advance and a `cost_checked` at 0.3 SOL accepted only for a sink with `min ≤ 0.3`;
   - the upgrade round-trip of a `FloorState` without `min_wei`.

   Then `launch-params.mjs` `FIXED.frog_purchase` gains the per-collection note.

At each wake the operator calls `set_purchase_limit(<module>, maxPurchaseLamports, minPurchaseSol × 10^9)` (§5 step 3). The values
are in `nft-pilot.json`:

| collection | minPurchaseSol | why |
|---|---|---|
| Runestone | 0.25 | ≈ ½ × its 30-day median sale (0.00074 BTC ≈ 0.56 SOL), rounded down to 0.05 |
| RSIC | 0.25 | ≈ ½ × its median sale (0.00066 BTC ≈ 0.50 SOL) |
| Bitcoin Weirdos | 0.15 | ≈ ½ × its median sale (0.00047 BTC ≈ 0.35 SOL) |
| every other | 1 (D-b) | floors ≥ 2 SOL |

The simpler alternative is to lower the global constant to 0.15 SOL. That is a one-line change, but it removes the 1 SOL guard for
every collection. The per-collection minimum keeps D-b for all the others.

**Relay SOL→BTC at small sizes [M].** These are keyless `POST https://api.relay.link/quote/v2` quotes, 2026-10-10 ≈ 07:35-07:42 UTC.
They used the request shape of the Q5 fixtures (`ops/rehearsal/relay-sol/fixtures/quotes/sol-btc-1.request.json`, throwaway
addresses) and are saved in the scratch directory only. "Impact" is Relay's `totalImpact`, which includes the relayer fee.

| in | out (BTC) | Relay impact | relayer fee (gas + service) | time estimate |
|---|---|---|---|---|
| 0.15 SOL ($16.46) | 0.00019438 | −2.67 % | $0.36 | 328 s |
| 0.25 SOL ($27.44) | 0.00032661 | −1.88 % | $0.39 | 328 s |
| 0.30 SOL ($32.92) | 0.00038859 | −2.75 % | $0.75 (0.66 + 0.09) | 652 s |
| 0.45 SOL ($49.38) | 0.00058703 | −2.05 % | $0.79 (0.66 + 0.12) | 652 s |
| 0.60 SOL ($65.84) | 0.00078549 | −1.70 % | $0.82 (0.66 + 0.16) | 652 s |
| 1 SOL ($109.74) | 0.00131224 | −1.47 % | $1.12 (0.87 + 0.25) | 652 s |
| 3 SOL ($329.21) | 0.0039557 | −0.99 % | $1.78 | 652 s |

The relayer gas part is nearly flat, between $0.36 and $0.87. Relay's share therefore grows as the size shrinks: ≈ 2-2.8 % at 0.15-0.6
SOL, against ≈ 1-1.5 % at 1-3 SOL. Today's 0.30 and 0.15 quotes came minutes apart with different gas estimates.

What a 0.3-0.6 SOL purchase costs beyond the price:

| cost | at 0.3 SOL | at 0.45 SOL | at 0.6 SOL | source |
|---|---|---|---|---|
| Relay impact (incl. relayer fee) | 2.75 % | 2.05 % | 1.70 % | measured above |
| the advance's gas keep (`solhome.rs` `SOL_GAS_KEEP_LAMPORTS` 0.002 SOL) | 0.67 % | 0.44 % | 0.33 % | [R] |
| Bitcoin miner fee for the purchase and the move to custody, ≈ 500-2,000 sats at 2-5 sat/vB | ≈ 1.3-5.0 % | ≈ 0.9-3.4 % | ≈ 0.6-2.5 % | [A] |
| **total** | **≈ 4.7-8.4 %** | **≈ 3.4-5.9 %** | **≈ 2.7-4.5 %** | |

Three bounds apply [R]:
- The Relay bound is `frog_funding.max_fee_bps` 400 (4 %, `nft-pilot.json`). Relay alone stays inside it at every size quoted.
- rook_core's `conv_bps` 3 % is the margin an advance carries for conversion. Relay plus the gas keep exceeds it at 0.3 SOL
  (3.4 %). It fits at 0.45 SOL (2.5 %) and 0.6 SOL (2.0 %).
- The miner fee comes out of the custody fee pool (`custodyFeePoolMinSats`), not the advance.

**Recommendation for the operator:**
- A 0.25 SOL minimum for Runestone and RSIC is workable. Their actual purchases are ≈ 0.5-1 SOL, where costs are ≈ 2-4 %.
- Bitcoin Weirdos (purchases ≈ 0.35-0.5 SOL) is marginal. Expect some advances to fall short of the 3 % margin and retry later
  (`Why::Later`), and ≈ 4-6 % total cost per item.

Either way the decision is the canister change above plus the values in `nft-pilot.json`. Until it ships these three are listed,
and their coins trade and graduate, but nothing is bought.

## 8. Listings 17-25 (operator decision 2026-10-10)

**Decision.** Add every collection the first pass dropped, except Ordinal Punks: Pizza Ninjas, Quantum Cats, Taproot Wizards,
Runestone, Bitcoin Wizards, The Wizards of Ord, CENTS, RSIC (metaprotocol) and Bitcoin Weirdos. They are appended as listings 17+ in
a fixed order, most liquid first, and no existing id moves.

**Order: most liquid first.** The measure is single-item sales per day in Ordinals Wallet's cross-venue sales feed (its last ≤ 100
sales) [M]. The tie between Quantum Cats and The Wizards of Ord (0.59 a day each) is broken by 7-day sales: 9 against 4.

| id | collection | sales / day | OW listed (sellers) | OW lowest 3 (BTC) | 30-d median (BTC) | CoinGecko floor (BTC), holders | OW lowest vs CoinGecko | wake check (§5) |
|---|---|---|---|---|---|---|---|---|
| 17 | Runestone | 131 (93 in 0.7 d) | 183 (126) | 0.00086 0.00086 0.00092 | 0.00074 (7 d) | 0.001388, 63,743 | −38 % | **fails** (book far under) |
| 18 | Bitcoin Weirdos | 5.3 | 22 (9) | 0.0005 0.00065 0.0008 | 0.00047 | 0.00065, 1,860 | −23 % | **fails** |
| 19 | CENTS | 2.2 | 26 (5) | 0.00851 0.00859 0.00953 | 0.00367 | 0.00515, 2,109 | +65 % | **fails** |
| 20 | RSIC | 1.7 | 23 (17) | 0.00257 0.00257 0.00257 | 0.00066 | 0.000745, 6,865 | +245 % | **fails** |
| 21 | Pizza Ninjas | 0.74 | 0 | none | 0.004 | 0.00872, 1,082 | no book | **fails** |
| 22 | Quantum Cats | 0.59 | 1 (1) | 0.01901 | 0.00551 | 0.01045, 1,638 | one listing | **fails** |
| 23 | The Wizards of Ord | 0.59 | 3 (3) | 0.01542 0.01799 0.05139 | 0.00509 | 0.0054, 2,174 | +186 % | **fails** |
| 24 | Taproot Wizards | 0.34 | 0 | none | 0.0316 | 0.047, 1,297 | no book | **fails** |
| 25 | Bitcoin Wizards | 0.32 | 11 (10) | 0.0555 0.411 0.432 | 0.021 | 0.0135, 903 | +311 % | **fails** |

**None of the nine passes the 20 % band today**, so none may get a first floor post. Each is listed with its day-1 config. Its coins
launch and graduate, and its backing share waits as §4 describes. It wakes only when its Ordinals Wallet book has ≥ 3 valid
listings within 20 % of CoinGecko's floor.

**Price references:**

| collection | `owSlug` | `coingeckoId` |
|---|---|---|
| Runestone | `runestone` | `runestone` |
| Bitcoin Weirdos | `bitcoin-weirdos` | `bitcoin-weirdos` |
| CENTS | `cents` | `cents` |
| RSIC | `rsic` | `rsic-metaprotocol` |
| Pizza Ninjas | `pizza-ninjas` | `pizza-ninjas` |
| Quantum Cats | `quantum_cats` | `quantum-cats` |
| The Wizards of Ord | `wizards` | `the-wizards-of-ord` |
| Taproot Wizards | `taproot-wizards` | `taproot-wizards` |
| Bitcoin Wizards | `bitcoin-wizards` | `bitcoin-wizards` |

`unisatCollection` is `null`, as for the first five.

**Caps.**
- `custodyItems` 3 and `custodyWei` 10 ETH, as the others.
- `maxPurchaseLamports` is ≈ 1.5 × the OW floor rounded up to 5 SOL, at least 10, using CoinGecko's floor where OW has no
  listing. That gives:
  - 10 SOL for Runestone, Weirdos, CENTS, RSIC and Pizza Ninjas;
  - 25 for Quantum Cats;
  - 20 for The Wizards of Ord;
  - 55 for Taproot Wizards (CoinGecko 35.4 SOL);
  - 65 for Bitcoin Wizards (OW 41.7 SOL; CoinGecko is 10.2, and the wake check holds it until the book comes down).
- `minPurchaseSol`: §7.

**Members, roots and provenance [M].** The generator, the rule and the tree are §3's. Two options are new:
- `--parent` takes the children of the collection's parent inscription.
- `--grandparent` takes the children of every child of an inscription.

Each `<slug>-members.provenance.json` records the sources, counts and left-out ids.

| collection | OW | on-chain source | agreed (members) | proof | references |
|---|---|---|---|---|---|
| Runestone | 112,384 | parent `fdb2df5d…cbb5i0`: 112,384 | **112,384** | 17 | no ME export |
| Bitcoin Weirdos | 10,000 | gallery `ee502230…23b6i0`: 10,000 | **10,000** | 14 | ME 10,000 (identical) |
| CENTS | 10,000 | gallery `c871da40…7f71i0`: 10,001 | **10,000** (1 gallery-only out) | 14 | ME 10,001 (has the same extra) |
| RSIC | 20,998 | grandparent `aa84eca1…1fb0i0` → 10 sub-parents `bdd321fc…9218i0`-`i9` × 2,100: 21,000 | **20,998** (2 on-chain-only out) | 15 | ME 19,244 (stale) |
| Pizza Ninjas | 2,920 | parent `80da3cec…82e1i0`: 2,920 | **2,920** | 12 | ME 2,022 (a subset) |
| Quantum Cats | 3,313 | parent `0e383b8a…ea41i0`: 3,334 | **3,313** (21 parent-only out) | 12 | ME 3,333 (20 of the 21) |
| The Wizards of Ord | 3,333 | gallery `b8a6c9e9…e64ci0`: 3,333 | **3,333** | 12 | ME 3,333 (identical) |
| Taproot Wizards | 2,106 | gallery `13bc906c…a7fbi0`: 2,102 | **2,102** (4 OW-only out) | 12 | ME 2,102 (identical); OW GitHub 2,106 |
| Bitcoin Wizards | 1,337 | gallery `da62a85d…2ac0i0`: 1,338 | **1,337** (1 gallery-only out) | 11 | ME 1,338; OW GitHub 1,337 |

**Pizza Ninjas: 2,920, 2,022 or 1,500.** The on-chain parent decides.
- The collection's parent inscription is `80da3cec…82e1i0`, the one TheWizardsOfOrd's curated registry names as its parent.
- On ordinals.com it has `child_count` **2,920**, and its children are exactly Ordinals Wallet's 2,920 ids.
- Magic Eden's 2,022 is its 2024 legacy export, a strict subset: it lacks 898 and has nothing extra. It is a snapshot from before the
  rest were inscribed.
- CoinGecko's 1,500 is a `total_supply` display figure, not a list.

Membership is **2,920**.

**Taproot Wizards: no team parent exists.**
- The wizards were inscribed in early 2023 (lowest inscription number 652), before ord had parent/child provenance. No inscription
  signed by the team holds them as children.
- The gallery TheWizardsOfOrd's registry names (`13bc906c…a7fbi0`) holds 2,102 items. Its own description says "2121 Taproot
  Wizards were inscribed".
- That gallery is a child of `c77ff02f…2600i0`, a 2026 curator inscription. The same inscription also parents the Bitcoin Puppets
  gallery used in §3 and the Bitcoin Wizards gallery. So it is a curated gallery, not a team-signed list.
- Magic Eden's export agrees with it exactly (2,102). Ordinals Wallet has 4 more ids, which its 2024 GitHub registry also has. They
  are in neither the gallery nor ME, so they are left out.
- 19 of the claimed 2,121 are in no source.

Membership is **2,102**, the ids the curated gallery, Magic Eden and Ordinals Wallet all share. The 4 OW-only ids, and any of the
missing 19 that turn up with provenance, are a later root change (7-day wait).

**Quantum Cats.** The parent holds 21 ids that Ordinals Wallet lacks, and Magic Eden has 20 of them. By the rule they are left out.
If Ordinals Wallet lists them, they are a root change before the wake.

**RSIC and Runestone are large but fit:**
- Proofs are 15 and 17 hashes (480 / 544 B): under the canister's 32 and inside one `mint_item` transaction.
- The files are 1.5 MB and 8.1 MB (`deployments/launch/btc/README.md`).
- Runestone's 112,384 ids are about half of all 223,448 ids across the 15 Inscriptions listings. No id is in two collections.

**Not added:** Ordinal Punks, by the operator's decision. Everything else the operator named was added.

## 9. Files (second pass: listings 17-25 and the graduation tiers)

- `deployments/launch/nft-pilot.json`:
  - nine `inscriptions[]` entries, each with `ordParent` / `ordGrandparent` / `ordGallery`, `minPurchaseSol` and
    `wrapping: "later"`;
  - `_inscriptions` updated.
- `deployments/launch/btc/`: nine `*-members.json` and `*.provenance.json` files, and a README section.
- `deployments/launch/graduation-tiers.json` (new) and `docs/v9/graduation-tiers.md` (new).
- `ops/solana/ordinals-members.mjs`:
  - `--parent` takes a list;
  - `--grandparent` is new;
  - large lists no longer overflow the stack.
- `ops/solana/launch-params.mjs`: `GRADUATION_TIERS`, `tierFor`, `meteora.tiers`, and `graduation_sol` in `listingOrder()`.
- `ops/solana/day1-pins.json` (`rook-ceremony pins --write`): 26 listings, plus `tiers`.
- `ops/solana/site-config.mjs`: `graduationSol` per row, the threshold check, and `TIER_SOL`.
- `ops/rehearsal/sol-devnet/tool/src/`:
  - `ceremony_checks.rs`: the tiers file, `listing_threshold`, `tier_parameters_json`, `check_pinned_tiers`, a tier-aware
    `WantConfig`, `cmd_pins` writing `tiers`, and tests;
  - `ceremony.rs`: `check_params` and the tier configs in `day1`, the plan's `graduation_sol`, `preflight` per-tier, and tests for
    26 listings, ids 17 / 25 and a tier carrying another tier's curve;
  - `ceremony_list.rs`: `check_written` with the row's threshold.
- `ops/test/`:
  - `launch_params.test.ts`: 26 listings, the 17-25 order, min / max purchase, and a new tier test;
  - `audit_a6_day1_verify.py`: 26 listings, and the parameters checked per tier;
  - `audit_a6_launch_tamper.sh`: 6 new tampered files.
- web-rook:
  - `src/rook/sol/collections.ts`: nine rows, `graduationSol` on every row, `GRADUATION_TIERS_SOL`;
  - `src/config/solana.ts`: 26 rows with `graduationSol`;
  - `test/sol.test.ts` and `test/tracked.test.ts`: the count pins (17 → 26), and the tiers held equal across the files;
  - `test/fixtures/sol/day1-plan.rehearsal.json`: nine rows derived as rook-ceremony derives them, plus `graduation_sol`.
