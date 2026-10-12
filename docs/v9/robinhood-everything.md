# v9: Rook on Robinhood Chain, with everything built in the Solana phase

**Operator decision 2026-10-10:** the Solana NFT collections (workstream 2 / phase R3's Solana part) are **paused** for now; R1, R1.5 and R2 proceed; Robinhood-native NFTs stay in R3.

Status: **design and effort plan, no code**, 2026-10-10, branch `golive-2026-10-08`. Nothing was signed, sent or deployed. A
separate track is finishing the Robinhood launch blockers: the canister rebuild, the Dwellir RPC, the dry run and the ceremony.
This page does not touch that work. It describes what comes after it, and what has to be decided **before** the R1 ceremony
(§1.3: the raises are fixed at deploy).

**Operator decision, 2026-10-10:** launch on Robinhood Chain (4663) after all, and "make sure we support Solana NFTs just like we
do here and everything we support here, including Robinhood NFTs".

**Legend.** **[R]** read from this repository. **[M]** measured or researched on 2026-10-10. **[A]** assumption or estimate.
Efforts are engineering days for one engineer who already knows the code, tests included. Audit time is listed separately.

Sources read: `robinhood-stop-2026-10-09.md`, `ref-robinhood-contracts.md`, `deployments/launch/nft-pilot.json`,
`ordinals-collections.md`, `graduation-tiers.md`, `solana-nfts-design.md`, `raffle-design.md`, and the code in
`autobuyer/canister-rs`, `script/DeployStack.s.sol`, `script/StackChecks.sol`, `keeper/service`, `indexer/src` and `web-rook/src`.

---

## 0. Summary

**Prior work was checked first.** On the operator's note ("some things may be already done — check"), these were searched before
anything was called missing:
- `git log --all` (745 commits) and every local branch;
- the worktrees `~/Documents/Rook-*`: nft-side, nft-pool, nftx-phase1, dormant-collections, cycles-from-fees, contract-fixes,
  invariant-fix, ideas-v8, ordinals-wave, p1-monitor, p1-deploy-site, site-v8 and the others;
- `src/`, `script/`, `test/`, `autobuyer/`, `ops/`, `keeper/`, `monitor/`, `indexer/`, `web-rook/` and `docs/v4-v9`.

Every local branch except four is already merged into `golive-2026-10-08`. Those four are:
- `cycles-from-fees` (a doc);
- `nftx-phase1` (a design for NFTX vTokens);
- `solana-wx-paired-wip` (parked WIP);
- an agent worktree carrying the Solana audit-verification docs.

None of them holds Robinhood or NFT work that is missing here. The status of each item:

| Item | Status | Where |
|---|---|---|
| Per-collection graduation targets (normal/early/max raise, convBps, reserveBps) | **already done** | commit `613173e` (DeployStack per-collection lists, golive-check), `docs/v7/phase1-nft-pooled.md`; tests `189dfdd` |
| Pooled graduations (several graduations fund one dear item) | **already done** (the FloorModule's own behaviour) | `phase1-nft-pooled.md` §0; tests `189dfdd`; site/indexer per-nest target `e370b12` |
| Several Ethereum collections on one canister address | **already done** | commit `13e01ea` |
| Mapping the approved tiers to ETH raises | **partly done**: the mechanism exists, the values must be chosen | §1.3 |
| Several Inscriptions wrappers in the canister | **decided 2026-09-28, not started** | `docs/v8/ordinals-wave.md` §0, §6 ("multi-wrapper support … its own review"); `ordinals-collections.md` §4 |
| A second Inscriptions wave on Robinhood (deploy, golive-check, services) | **partly done**: planned, and a second `deploy-stack` run works; the multiple-stack-file readers and the dry-run phase are not built | `ordinals-wave.md` §3 |
| One Relay leg canister-wide (B-L-6, needed before two sides run) | **already done** | commit "B-L-6 fixed — one Relay leg canister-wide" |
| Keeper floors for several Ordinals collections | **already done** (`KEEPER_OW_SLUGS=<stack slug>=<OW slug>,…`) | `keeper/service/config.ts:217` |
| Per-collection purchase **maximum** | **already done** (`set_purchase_limit`, up to 25 ETH) | `frogbuy::check_limits_for`, `floortrigger.rs` |
| Per-collection purchase **minimum** | **not started** | §1.2, item 2 |
| Bitcoin buy automation (OW protected, UniSat) | **already done** for one collection (runner steps) | `ops/src/owprotected.ts`, `unisatbuy.ts` |
| Robinhood-native NFT research and a keyless same-chain buy path | **partly done**: 12 native ERC-721s found with their addresses, Seaport 1.6 confirmed on 4663, `NftSweepVault` + `SeaportGuard` built and tested (a fee-funded raffle/burn sweep, not wX backing) | `docs/v4/nft.md` §1, `src/nft/NftSweepVault.sol`, `src/nft/SeaportGuard.sol`, `test/nft/*` |
| A same-chain wX wrapper (`LocalNftWrapper`) | **not started** (it reuses `SeaportGuard`) | §3.2 |
| A wrapper for Solana NFTs on an EVM chain (`SolNftWrapper`) | **not started** | §2.2 |
| Solana marketplace buys (Tensor / Magic Eden) | **not started** in either phase | §2.4 |
| Canister Solana primitives (threshold Ed25519, tx, RPC, send line, Relay Solana legs) | **already done**, as a **home** | `solkey/soltx/solsend/solrpc/solread/relaysol.rs` |
| Using Solana as a second custody chain beside an EVM home | **not started** | §2.3 |
| SolNft rules (membership, delegates, delivery checks) | **already done** in rook_core; **reusable as off-chain checks** | `solana/programs/rook_core/src/nft.rs`, `tests/suite/solnft.rs` |
| Raffle policy (`SweepVault` RAFFLE, drand verified on chain) with keeper draws and resolver | **already done** | `src/SweepVault.sol`, `keeper/service/draws.ts`, `resolver.ts`; commit `f07ba72` |
| BURN and YOYO policies | **already done**: BURN on; YOYO built, off at launch | `SweepVault.sol`, commit `4756b74` |
| Raffle panels on the Robinhood coin page | **already done** | `web-rook/src/ui/Raffles.tsx`, `pages/Coin.tsx:125` |
| Robinhood board (front page), coin page (chart, ticket, Trades / Holders / Info / Activity), launch with wake, redeem, draw, reserves | **already done, unrouted** | commit `468f2c5`; `web-rook/src/pages/{Home,Coin,Launch,Redeem,Draw,Reserves}.tsx`, `AppPages.tsx` |
| Robinhood indexer: `/v1/rook/board`, `/activity`, `/coins/:a/chart`, `/trades`, `/holders`, `/wallets/:a`, `/catalog` (curve plus v4 pool swaps) | **already done** | commit `468f2c5`; `indexer/src/rook/*` |
| Robinhood demo | **partly done**: fixture mode from a demo recording | `web-rook/src/rook/fixtures.ts`, `data.ts` |
| Solana-terminal look on Robinhood (terminal tabs, trending strip, tier display, official links) | **not started** on Robinhood; exists for Solana | `pages/SolHome.tsx`, `rook/sol/collectionLinks.ts` |
| Coin metadata registry (image, links, signed by the creator) | **specified, not built** | `docs/v6/site-design.md` §6.3; `Launch.tsx:151` says so |
| Monitor on 4663 (floors, caps, draws, guardian, holders, EthNft, canary) | **already done** | `monitor/src/{evm,draws,guardian,holders,ethnft,canary}.mjs`, commit `53ea373` |
| Monitor / reconciler for Solana custody of Solana NFTs | **not started** | §2.2 |
| X thread copy | **not started** (nothing committed) | §4.6 |

| # | Workstream | Already done / reused | Left to do | Effort (d) | Audit |
|---|---|---|---|---|---|
| 1 | 14 more Ordinals collections | contracts; roots in `nft-pilot.json`; per-collection targets; keeper multi-slug; per-collection maximum; B-L-6; the v8 wave plan | canister multi-wrapper (EVM home); per-collection minimum; one stack file or a list of them (treasury/directory reuse); services and dry run; cheap-collection `maxEthBacked` | **20-27** | internal + outside canister review (shared with #2) |
| 2 | 12 Solana NFT collections | canister Solana primitives; Relay Solana legs (Ethereum origin); rook_core's SolNft rules as a reference | `SolNftWrapper`; Solana custody line on an EVM home; Tensor / mmm buys; Relay 4663↔SOL; delivery; floor feed; data pinning; reconciler / monitor | **51-67** | outside contract audit + outside canister review |
| 3 | Robinhood-native NFTs | research, addresses, Seaport 1.6 on 4663, `SeaportGuard` / `NftSweepVault` with tests | `LocalNftWrapper` (reusing `SeaportGuard`); deploy kind; fresh measurement | **7-10** | bundled with #2's contract audit |
| 4 | Solana-phase features | Robinhood board, coin, launch, redeem, draw and reserves pages; `/v1/rook` indexer; `SweepVault` raffle with panels; monitor; fixture demo | re-route and restyle to the terminal; tier and links display; metadata registry; indexer list/detail/collections and a persisted fold; raffle ON for NFT collections; fees copy; thread | **17-24** | internal review only |
| | **Total** | | | **95-128** | |

The phased plan (§5): **R1** ships the 11 Ethereum collections and Bitcoin Frogs as originally planned, with the tier values of
§1.3, and with the Robinhood site pages that were already built (re-routed). **R1.5** restyles them into the terminal. **R2** adds the 14 Ordinals (≈ 5 weeks after
R1 including review). **R3** adds the 12 Solana collections and any native listing (≈ 10-12 weeks after R1, with the outside audit).

One fact makes Robinhood easier than Solana here: **a Robinhood collection is a fresh `deploy-stack` run that can happen any day
after launch.** Nothing like Solana's day-1 DBC configs commits the listing order. So a collection is listed only when the
canister can actually wrap it. No collection has to sit in a "coins launch, nothing is bought" state, as the 14 Ordinals would on
Solana day 1.

---

## 1. The 26 non-Solana collections on Robinhood

### 1.1 What exists [R]

- **The 11 Ethereum collections and Bitcoin Frogs** are the R1 launch set. Their files are `nft-pilot.json`, `DeployStack` (one
  btc kind plus `ETCH_NFT_*` lists) and runbook §9. The mixed dry run passed.
- **The 14 extra Ordinals** (`nft-pilot.json` `inscriptions[]`, ids 12-25 on Solana) carry their members files, roots, `owSlug`,
  `coingeckoId`, caps, `maxPurchaseLamports` and `minPurchaseSol`.
  - The roots use the same tree as Frogs: `keccak256(keccak256(id))`, sorted pairs. That is `ListingRegistry.isMember`'s rule, so
    **the same roots work on Robinhood unchanged**.
- **Contracts.** `Wrapper`, `WrapClaim`, `FloorModule`, `CurveFactory`, `SweepVault` and `LiquidityLock` take no
  collection-specific code. Every Inscriptions collection is one more `WrapClaim`, and with it one more `Wrapper`.
- **ListingRegistry** has `listInscriptionsAuto` (or `listInscriptions`) per collection. Each listing carries its own root, its
  custody address and its 7-day root-change delay.
- **Keeper.** `keeper/service/stack.ts` already parses N collections from the stack file and posts to each `floorModule`.
  Inscriptions floors on Robinhood are posted by the **keeper EOA**, not the canister (`ref-robinhood-contracts.md` §3).

### 1.2 What is missing, and the design

1. **The canister: several Inscriptions wrappers on the EVM home.** **Not started.** The operator decided it on 2026-09-28
   (`docs/v8/ordinals-wave.md` §6). The no-code alternative noted there, a second buyer canister, is heavier.
   - **Today [R]:** `Init.wrapper : opt text` is one wrapper. The tick runs the ordinals side for `cfg().wrapper` only. `FROGS`, the
     redemption cursor and the endpoints (`frog_proof`, `set_ow_creators`, `probe_frog`, `frog_offer(_protected)`) are singletons.
   - **The design** is `ordinals-collections.md` §4's seven parts, written once for both homes:
     - `Init.inscriptions : opt vec record { wrapper; ow_slug }`, with `wrapper` kept as entry 0;
     - per-wrapper `frogs` / `proofs` / `convs` / `protected` / `ow_creators` / redemption cursor;
     - a custody-wide classification. All wrappers share the one taproot custody, and each output is routed to the wrapper whose
       root its inscription proves against;
     - the tick loops over the wrappers;
     - every endpoint takes a wrapper argument;
     - dormancy covers Inscriptions;
     - one `CollectionCap` per awake listing, with `global_max_items` re-sized by the exit rule.
   - **What is EVM-only:** the home halves that Solana does not have. `ticketlog` reads each `Wrapper`'s tickets by state calls at
     a pin; `Wrapper.wrapDeposit(…, to = module)` and `settleAdvance` run per module; `frogflow`'s 4663 Relay legs run per sink.
     None of these change shape; they become per-wrapper.
   - **Effort:** 12-16 d. That is the doc's 1.5-2.5 weeks, plus ≈ 3 d for the EVM halves and their tests.
2. **A per-collection purchase minimum (EVM half of `ordinals-collections.md` §7).** **Not started.** The per-collection
   **maximum** is already done. In September `ordinals-wave.md` kept Runestone out rather than change this; the operator's
   2026-10-10 additions reverse that.
   - **Today [R]:** `policy::MIN_PURCHASE_WEI` is 0.05 ETH, global. `check_limits_for` already takes a per-collection **maximum**
     (`set_purchase_limit`, up to 25 ETH).
   - **The problem:** Runestone (≈ 0.046 ETH), Bitcoin Weirdos (≈ 0.022) and RSIC (≈ 0.025) are under that minimum, so every
     purchase would be refused.
   - **The fix:** `PurchaseLimit.min_wei: Option<u128>` with `#[serde(default)]`, a dust floor `ABS_MIN_PURCHASE_WEI` of
     ≈ 0.005 ETH, and `check_limits_for(cost, max, min)`.
   - **Also needed:** the default 0.5 ETH maximum is below NodeMonkes' Ordinals Wallet book (≈ 1.08 ETH), OMB (≈ 0.65), Taproot
     Wizards (≈ 1.56 by CoinGecko) and Bitcoin Wizards (book ≈ 1.84). Each needs `set_purchase_limit` at its wake. That is
     configuration only.
   - **Effort:** 2 d.
3. **`DeployStack`: N Inscriptions collections in one run, or in later runs against existing contracts.** **Partly done.**
   `ordinals-wave.md` §3.1 plans a second wave as a second `deploy-stack.sh` run (kind `btc`) against the same registry, which works
   today. Two things are left: readers of more than one stack file, and treasury/directory reuse.
   - **Today [R]:** each run deploys **one** `ProtocolTreasury` and **one** `CollectionDirectory`, plus a single btc collection
     (`ETCH_WRAPPER_LISTING`). A second run for one more collection therefore creates a second treasury and a second directory.
     The canister and the WakeDesk read one directory.
   - **The design:**
     - `ETCH_TREASURY` / `ETCH_DIRECTORY` to reuse what exists, read back and checked;
     - an `ETCH_INSCRIPTIONS` list (slug, name, symbol, listingId), looped like `_nftCollection`;
     - per-collection `CurveFactory.Targets` (§1.3);
     - a per-collection `maxEthBacked` / `maxPublicEthBacked` (§1.3, item 4);
     - the Safe calls printed per collection: `WrapClaim.initMintLimit` and `wireFloorModule`, `ProtocolTreasury.addCollection`,
       `CollectionDirectory.add`, `SweepVault.proposeCapCeiling`;
     - `StackChecks` bounds for every new input;
     - `deployments/stack-4663.json` appended, not rewritten (the keeper, indexer and monitor read it).
   - **Effort:** 2-3 d (the reuse inputs and one merged stack file; the per-collection loop already exists for NFT kinds).
4. **The keeper's floor feed, per collection.** **Mostly done.** `KEEPER_OW_SLUGS=<stack slug>=<OW slug>,…` already maps each
   stack collection to its Ordinals Wallet book (`keeper/service/config.ts:217`). What is left is configuration, the membership
   check per collection's members file, and the wake rule.
   - **What changes:** the feed reads each listing's Ordinals Wallet book (`owSlug`) and verifies the listed outpoints against
     that collection's members file. A collection that is not awake posts nothing.
   - **Two guards:** the D8 band against `coingeckoId`, and the 20 % wake check of `ordinals-collections.md` §5.
   - **Already available:** `ops/src/solfloor.ts` already reads every Inscriptions listing for the Solana runner and serves
     `GET /floors`. The same reading code serves the keeper.
   - **Effort:** 1-2 d.
5. **Runner, monitor, golive-check and dry run.**
   - The runner needs `RUNNER_INSCRIPTIONS=<wrapper>=<owSlug>:<membersFile>[:<unisat>],…`, the protected-listing and UniSat steps
     run per collection, and the members files must be in the runner image. Runestone's file is 8.1 MB.
   - The monitor needs `price_refs` per collection, the expected custody inflows and the D8 guard on each listing.
   - `golive-check` needs the N-collection stack, and the dry run gains a `mixed-N` phase.
   - **Effort:** 3-4 d (golive-check already loops by `kind`, `ordinals-wave.md` §3.2).
6. **Waking a collection** is unchanged from `ordinals-collections.md` §5:
   - rerun the members file and diff it;
   - the Ordinals Wallet book holds ≥ 3 valid listings within 20 % of CoinGecko's floor;
   - set the limits, the creators list and the caps;
   - confirm the UniSat collection.

   On 2026-10-10 only OMB passed the wake check. On Robinhood the R2 deploy can therefore list just the collections that pass
   that day, and list the rest later.

### 1.3 Graduation sizing: the approved tiers as Robinhood raises

**Status: partly done.**
- **Already done:** per-collection bounded targets and module conversion / reserve (commit `613173e`; `ETCH_NFT_NORMAL_RAISES`,
  `_EARLY_RAISES`, `_CONV_BPS_LIST`, …), and pooled graduations (`docs/v7/phase1-nft-pooled.md`; tests `189dfdd`; the site and
  indexer show each collection's own target, `e370b12`). On Robinhood a graduation smaller than one item **pools** with the next
  ones. That is the FloorModule's own behaviour, so nothing is stuck.
- **Left:** the operator's 2026-10-10 rule ("every graduation buys at least one NFT at once") is stricter than pooling. It needs
  values per collection; the mechanism is unchanged.

**How the two systems differ [R].**
- **Solana:** a graduation spends `0.364 × threshold` on NFTs (a 50 % migration fee, 25 % reserve, 3 % conversion).
- **Robinhood:** a coin's `itemBudget = raise − min(5 %·raise, 0.25 ETH) − 12.6 %·raise` goes to the module. The module converts
  it at `floor × (1 + convBps)` and keeps `reserveBps` as ETH.

  So the buying power is `BP(r) = itemBudget × (1 − reserveBps) / (1 + convBps)`. At the defaults (25 % reserve, 3 % conversion)
  that is `0.6365·r − 0.182` ETH for r ≥ 5 ETH.

**The operator's rule, unchanged:** the smallest raise whose `BP ≥ 1.33 × floor`.

**Bounds [R]:**
- `CurveFactory` refuses an `earlyRaise` above `HARD_MAX_EARLY_RAISE` (10 ETH), and early must be at least normal.
- `StackChecks.checkNftCollection` also bounds `normalRaise` to 1-10 ETH and `maxRaise` to at most 30.

So the 255 and 510 SOL tiers (11.2 and 22.5 ETH at 22.69 SOL/ETH) **cannot be mapped by value**. They clamp to 10 ETH.

| SOL tier (approved) | value in ETH | **Robinhood normal / early raise** | BP at normal | covers a floor up to | collections |
|---|---|---|---|---|---|
| 85 | 3.75 | **5 / 8 ETH** (the tested defaults) | 3.00 ETH | 2.26 ETH | 21 collections, Frogs included |
| 170 | 7.49 | **7.5 / 10 ETH** | 4.59 ETH | 3.45 ETH | Identity MD (2.58 NFTs/grad), Taproot Wizards (2.95) |
| 255 | 11.24 | **10 / 10 ETH** (clamped) | 6.18 ETH | 4.65 ETH | Pudgy Penguins (2.38) |
| 510 | 22.47 | **10 / 10 ETH, reserveBps 0, convBps 1000** (clamped; the tested BAYC setting) | 7.72 ETH | 5.80 ETH | BAYC (1.28 on value; buys one at once, see below) |

Floors are those of `graduation-tiers.md` §2 (2026-10-10), at 33.15 ETH/BTC [M].

**Why the 85 tier keeps 5 / 8 ETH.** 85 SOL is ≈ $9.3k; 5 ETH is ≈ $12.5k. A 4 / 7 mapping would be closer in dollars, and every
85-tier collection still passes at 4 ETH (BP 2.40, covering floors up to 1.80 ETH). But 5 / 8 is what the mixed dry run tested.
**Operator choice:** keep 5 / 8 (recommended for R1), or move to 4 / 7, which only changes environment inputs.

**What the launch file has today [R].** Every collection is on the 5 / 8 ETH defaults except BAYC, which is at 10 / 10 ETH with
`convBps 1000` and `reserveBps 0` (branch `nft-bayc10`, tested in `189dfdd`: "10 ETH / 11 % / no reserve buys one after a +10 %
rise").

**The two collections the 1.33 rule moves:**
- **Pudgy Penguins** at 5 ETH gets 1.15 units per graduation. That buys one at once, but with only 15 % headroom.
  **Recommendation: move it to 10 / 10 ETH** (2.38 ×). That is a per-collection entry in `nft-pilot.json`.
- **BAYC** at `convBps 1000` has a BP of 7.72 ETH, 1.28 × its 6.03 ETH floor. On the value rule that is below 1.33.
  - But its 10 % conversion margin already covers a +10 % floor rise: `maxItemCost` allows the item to cost the accepted floor
    × 1.10.
  - Its 1.28 units also clear the canister's 1.05 buy signal.
  - So the tested setting already meets "buys one at once", with the headroom carried in `convBps` instead of in the raise.
  - **Recommendation: keep the tested BAYC setting.** If the operator wants the value rule literally: (a) `convBps 300` gives
    1.37 ×, but only 3 % margin on a ≈ 6 ETH purchase; (b) `convBps 600` gives 1.33 ×; (c) raising `HARD_MAX_EARLY_RAISE` is a
    contract change, not recommended.

Re-measure BAYC and Pudgy on the morning of the ceremony.

**Identity MD** (1.78 ETH) passes at 5 ETH (1.69 ×). Its move to 7.5 / 10 ETH in the table above is optional, kept only for symmetry
with the approved 170 tier. **Recommendation: leave it at 5 / 8 ETH** (tested), and use 7.5 / 10 ETH for Taproot Wizards only if its
floor rises.

**Cheap collections overflow `maxEthBacked` [R], a Robinhood-only issue.**
- A graduation mints `BP / floor` ETH-backed units into the module. `maxEthBacked` is 40 units by default, and a graduation that
  does not fit refuses with `QueueFull`: the curve keeps trading and cannot close.
- At 5 / 8 ETH: Runestone gets 65 / 107 units, Bitcoin Weirdos 140 / 228, RSIC 121 / 199, Natcats 32 / 52, Bitcoin Punks 29 / 47
  and Frogs 23 / 37.
- **The design:** a "floor < 0.11 ETH" variant.
  - Runestone, Weirdos and RSIC: normal / early **2 / 3 ETH**.
  - Natcats and Bitcoin Punks: **3 / 4 ETH**.
  - Each module's `maxEthBacked` is ≥ 2.5 × the units of an early graduation: 120 for Weirdos and RSIC, 110 for Runestone, 60 for
    Natcats and Punks. `maxPublicEthBacked` stays ≤ 10 items.
  - Frogs stays as tested (5 / 8 ETH, 40 units).
- **Throughput.** At stage 0 a wrapper mints 3 items a day. A 63-unit Weirdos graduation therefore takes about three weeks to turn
  into NFTs, and is ETH-backed until then. That is the same limit as on Solana (`graduation-tiers.md` "Throughput").
- **Where the values live:**
  - `nft-pilot.json`, as new per-collection fields `normalRaiseWei`, `earlyRaiseWei` and `maxEthBackedItems`;
  - `ETCH_*_RAISES`;
  - a new `StackChecks` rule that every collection's units per early graduation are ≤ 40 % of its `maxEthBacked`.
- **Effort:** 1-2 d, included in the table under "graduation tiers" (§4).

### 1.4 Risk and audit

- **Risk.** Moderate. The canister change touches custody classification, deposits and redemptions, which is audited code.
  - A per-wrapper copy of the classification would refuse, and return, another collection's deposit. That is why the
    classification stays custody-wide.
  - Members-file drift (Quantum Cats, Taproot Wizards) is a 7-day root change on `ListingRegistry`.
- **Audit.** Internal review rounds in the style of the R1-R7 NFT rounds (≈ 5 d), plus the outside canister review shared with §2.
  The contracts are unchanged; `DeployStack`/`StackChecks` get script review and tamper tests (as `audit_a6_launch_tamper.sh`).

---

## 2. The 12 Solana NFT collections on Robinhood

The 12 collections are Mad Lads, SMB Gen2, Claynosaurz, Famous Fox Federation, DeGods, Galactic Geckos, Taiyo Robotics, Okay
Bears, SMB Gen3, Transdimensional Fox Federation, Pythenians and Cyber Frogs.

### 2.1 What exists [R]

- **rook_core's SolNft rules (`solana-nfts-design.md`):**
  - standards `Legacy` / `Programmable` / `Core`;
  - membership by the verified collection (Token Metadata) or `update_authority == Collection` (Core);
  - refusal of cNFTs, Token-2022 and permanent / foreign delegates;
  - wallet-to-wallet delivery.

  On Solana these checks are **on chain** at `mint_item` and `record_delivery`. On Robinhood they move into the canister as
  off-chain checks (below). The rules, the test fixtures (`mpl_core.so`, the rule sets) and the CU figures carry over.
- **Canister Solana primitives:**
  - `solkey` (threshold Ed25519, `sol_pubkey`; the canister's Solana address is an ordinary on-curve wallet);
  - `soltx` (base58, PDAs, ATAs, transfers, durable nonces);
  - `solsend` (a `Line`: prepare, simulate, send, durable-nonce lifecycle, incidents);
  - `solrpc` (several providers, agreement);
  - `solread` (rook_core decoders).
- **Relay:**
  - `relayeth.rs` `judge_to_solana` (Ethereum → Solana, rules 1-12);
  - `ops/src/relaysol.ts` (`eth-sol`, `sol-eth`, `sol-btc` legs, the depository program);
  - fixtures for an ETH→SOL 0.5 quote.
- **Not built in either phase:**
  - **any Solana marketplace buy** (no Tensor or Magic Eden code in the canister);
  - Solana NFT transfer builders (Token Metadata `TransferV1`, mpl-core `TransferV1`);
  - a Solana NFT floor feed;
  - the 12 collections as **data**: no launch-file entries with collection mints and standards. The site's `SolCollection.chain`
    is `"Ethereum" | "Bitcoin"`.

  The Solana phase was going to need all of this too. Building it for Robinhood is not wasted work if Solana returns.
- **Home assumption.** Everything Solana in the canister is a **home** (`Home::Solana`, `solhome.rs`). An upgrade that names a
  Solana home migrates the EVM state into it. Using Solana as a second **custody** chain beside an EVM home is new.

### 2.2 `SolNftWrapper` (new contract) — not started

An ERC-20 on 4663 where 1e18 = one Solana NFT held in the canister's Solana wallet. It is a sibling of `EthNftWrapper`, which stays
byte-for-byte unchanged because it is audited. It keeps the same ticket machine, invariants, H11 mint limits and module hooks.

| | `EthNftWrapper` | `SolNftWrapper` |
|---|---|---|
| Item key | `uint256 tokenId` | `bytes32 mint` (Token Metadata mint or Core asset) |
| Collection | `address collection`, `chainId` ≠ this chain | `bytes32 collection` (TM collection mint or Core collection), `uint8 standard` (0 Legacy, 1 Programmable, 2 Core), immutable |
| Custodian | `custodian` (EVM, no code), its ETH address holds the items | `custodian` (EVM, no code: the canister's 4663 address) **plus** immutable `bytes32 custodySol` (the canister's Ed25519 pubkey, the custody wallet) |
| Mint | `mint(tokenId, purchaseTx, to)`, `mintDeposit` | `mint(bytes32 mint, bytes32 txHead, to)` (`txHead` = the first 32 bytes of the Solana purchase signature; `source = keccak(mint ‖ txHead)`, once ever); `mintDeposit` the same (off at launch) |
| Redeem | `redeem(address ethDest)`, `redeemToBurn` | `redeem(bytes32 solDest)`: refuses 0 and `custodySol`. **No `redeemToBurn`** (Solana has no burn address; the BURN `SweepVault` policy is not offered for these collections; RAFFLE only) |
| Delivery | `recordDelivery(id, txHash)`, `noteSent`, `markUndeliverable` | `recordDelivery(id, bytes64 solSig)`: the full signature in the event, so anyone can look it up on Solana; the rest unchanged |
| Refunds, deadlines | 7 d pinned, deliverBy + 72 h, +30 d | unchanged |

`FloorModule`, `PosterSource`, `CurveFactory`, `LiquidityLock`, `EtchRouter`, `SweepVault` (RAFFLE) and `WakeDesk` need **no
change**. The module only calls `moduleMint` / `moduleBurn` and checks that the wrapper minted one unit to it before
`settleAdvance`.

**The trust downgrade, stated plainly.** On Solana, rook_core **refused** to mint wX for an NFT the custodian did not hold, and
refused to record a delivery that had not happened. An EVM contract cannot read Solana, so `SolNftWrapper` trusts the custodian's
attestation, exactly as `EthNftWrapper` trusts it for Ethereum. The mitigations:
- (i) The canister performs rook_core's checks itself, at Solana `finalized`, through ≥ 2 agreeing RPC providers (§2.4).
- (ii) The reconciler compares, on every pass, each wrapper's `held` set with the canister wallet's token accounts or Core assets.
  The monitor alerts on any difference and the guardian pauses (`PosterSource.pause`, `ListingRegistry.setPaused`).
- (iii) The site publishes a proof-of-custody page that lists each item, its owner on Solana and its wX ticket state.

The canister upgrade timelock (72 h) bounds an upgraded canister to 3 unbacked mints a day per wrapper, as today.

**Effort:**
- contract, tests and fuzz (the invariant `totalSupply = (held − openTickets)·1e18 + ethBacked`): 6-8 d;
- `DeployStack` "sol" kind, `StackChecks`, the stack file, keeper/indexer ABIs: 3 d.

### 2.3 The canister's Solana custody line (EVM home) — not started (the primitives are done)

- **A second `solsend::Line`,** keyed `custody`, separate from any home line. It has its own durable nonce (`nonce_account_of(me,
  "custody")`), a fee and rent float in SOL, and its own incident state.
- **Balance management.** The line keeps ≥ 0.05 SOL for fees, ATA rent (0.00204) and pNFT token-record rent. It tops up from the
  advance's Relay leg (the purchase quote is grossed up by ≈ 0.01 SOL). A refill below the floor raises a monitor alert.
- **Stable memory.** A new `#[serde(default)]` element in the upgrade tuple. The EVM home's state is untouched.
- **Effort:** 5-7 d.

### 2.4 Buying: ETH on Robinhood → SOL → Tensor / Magic Eden → mint wX on Robinhood — not started

The flow mirrors the Ethereum one (`ref-robinhood-contracts.md` §3), with Solana in place of Ethereum:

1. The canister sees `netUnits ≥ 1 + buffer` and calls `FloorModule.advance(≤ maxItemCost)`. The ETH arrives at the canister's
   4663 address.
2. **Relay 4663 → SOL** to `custodySol`.
   - The route needs a new judge pair: origin 4663, destination Solana (rules 1-12 of `judge_to_solana`, with origin 4663 instead of
     1), and the return pair **SOL → 4663** for `returnAdvance`.
   - The 4663→1 and 4663→BTC routes were proven live in September [R]. **4663→SOL is unmeasured: spike it first** (a keyless quote,
     then one small transfer).
3. **Choosing the item.**
   - The runner supplies candidate listings from Magic Eden's and Tensor's APIs.
   - The canister **verifies each candidate on chain** at `finalized` from ≥ 2 providers:
     - the listing or pool account (price, seller, expiry);
     - the NFT's metadata or Core asset, with rook_core's membership and delegate rules (§2.1);
     - the rule set, for a pNFT.
   - It picks the cheapest verified member with `price + fees ≤ advance − margin`.
4. **The keyless buy** into `custodySol`. The canister builds the instructions itself; no marketplace API key is in the signing
   path.
   - **Tensor:** TComp `buy_legacy` / `buy_core` for listings; TSWAP/TAMM buys from pools.
   - **Magic Eden:** `mmm` `sol_fulfill_sell`, buying from an mmm pool.
   - ME **M2** listings need ME's notary co-signature through its API. They are **left out** at first, so not every Magic Eden
     listing is reachable.
   - For a pNFT the marketplace program runs Token Metadata `TransferV1` under the rule set. For Core it runs mpl-core `TransferV1`.
5. **After Solana `finalized`:**
   - the canister re-reads the custody (the token account's owner is `custodySol` and `amount == 1`, or the Core `asset.owner`);
   - it then calls `SolNftWrapper.mint(mint, txHead, module)` and `FloorModule.settleAdvance(cost)` on 4663.

   **The cost is in ETH:** the advance less the ETH value of any SOL returned, so foreign-exchange drift lands in the advance's
   margin.
6. **On failure:** SOL → 4663 through Relay, then `returnAdvance`.

**Economics.** Two Relay legs on the round trip (only one on success) at ≈ 1-1.5 % each at 1-3 SOL sizes [A, from the SOL→BTC
quotes in `ordinals-collections.md` §7]. Add Solana fees (< 0.1 %) and the marketplace taker fee: Tensor ≈ 1.5-2 %, Magic Eden
≈ 2 % [A]. Royalties are optional or enforced depending on the collection (pNFT rule sets enforce them). That is ≈ 3-5 % on top of
the floor.
- **This does not fit `convBps` 300 (3 %).**
- **The design:** Solana collections use `convBps` **600** (within `StackChecks`' 300-1,600). Re-measure on real quotes before the
  deploy.
- Per-purchase minimum ≈ 0.5 SOL equivalent (Relay's flat part, as in §7 of the Ordinals doc); per-collection maximum through
  `set_purchase_limit`.

**Effort:**
- Tensor and mmm instruction builders and judges, including the pNFT token-record and rule-set account lists: 12-18 d;
- Relay 4663↔SOL judges and the spike: 3-4 d;
- the mint attestation flow (finality, custody re-read, the 4663 `mint` and `settleAdvance`): 3-4 d.

### 2.5 Delivery: Solana wallet to wallet, with a delivery proof — not started

1. `SolNftWrapper.redeem(solDest)` opens a ticket, which is resolved by drand as today. The canister reads tickets by state calls at
   a pin (`ticketlog`, per wrapper).
2. The canister sends **one Solana transaction**: ATA `CreateIdempotent` if needed, then the transfer (Token Metadata `TransferV1`
   for a pNFT with its rule set, SPL `Transfer` for legacy, mpl-core `TransferV1` for Core) from `custodySol` to `solDest`. The CU
   figures are in `solana-nfts-design.md` §10.
3. **The delivery proof.**
   - After `finalized`, the canister re-reads the destination's token account (`owner == solDest`, `mint`, `amount == 1`) or the
     Core asset's owner from ≥ 2 providers.
   - Only then does it call `noteSent` and `recordDelivery(id, solSig)` on 4663. The event carries the full 64-byte signature, and
     the reconciler re-verifies it.
4. **An off-curve `solDest`** (a PDA) may be refused by a pNFT rule set. The canister then calls `markUndeliverable`, and the holder
   refunds after `deliverBy + 72 h`, as for Ethereum.

   The site warns before redeeming to a non-wallet address. It can check this off-chain with `soltx::is_on_curve`.

**Effort:** 3-4 d.

### 2.6 Floor feeds — not started

- **The post.** As for Ethereum NFTs, the canister is the poster (through `PosterSource`).
  - It posts every 20 min from **verified** listings: §2.4 step 3, on ≥ 3 valid listings (`robustFloor`).
  - The post is bounded by recent sales, which the canister reads from marketplace sale transactions at `finalized` (the runner
    supplies the signatures, from ME's activity and Tensor's trades).
  - It is converted to ETH at the canister's SOL/ETH rate (the `rates.rs` median of 4 venues).
- **The monitor's D8 reference** uses two sources:
  - CoinGecko's NFT API, which lists the large Solana collections;
  - Magic Eden's keyless `collections/{symbol}/stats`.

  Tensor's API needs a key. It is optional as a third source.
- **Wake check:** the verified book's floor within 20 % of the D8 reference, as for Ordinals.
- **Effort:** 6-8 d, including the monitor.

### 2.7 The rest

- **Collection data.** For each of the 12, pin:
  - the collection mint (Token Metadata) or collection account (Core) and the dominant standard;
  - the rule set read from mainnet (custom rule sets are read before listing, `solana-nfts-design.md` §5);
  - for Core collections, a plugin check (no permanent delegates, no lifecycle or oracle adapters);
  - Magic Eden symbol, Tensor slug, CoinGecko id and official links;
  - floors, which give the tier (85-tier mapping for most, re-measured).

  A pinning script records all of this in `nft-pilot.json` `solana[]` with provenance: 2-3 d.
- **Dormancy and wake for SolNft listings** (`dormancy.rs` covers EthNft only): 3 d. The `WakeDesk` contract already serves any
  wrapper in the `CollectionDirectory`; the canister side is what is missing.
- **Holder deposits** stay off at launch, as for Ethereum and Bitcoin. Opening them later means the holder sends the NFT to
  `custodySol` with a memo naming the 4663 recipient, and the canister calls `mintDeposit`.
- **Rehearsal:**
  - devnet: Tensor and ME programs are not on devnet with real liquidity, so use a mainnet-fork LiteSVM or Surfpool with dumped
    program bytes and accounts, as the Solana phase did for Token Metadata;
  - one small funded mainnet purchase on the cheapest collection: 5 d.

### 2.8 Risk and audit

- **Risk: high**, the largest piece of new canister surface since the Seaport judge.
  - Marketplace program upgrades (Tensor and ME programs are upgradeable): pin program hashes and refuse on change.
  - pNFT rule-set revisions (`solana-nfts-design.md`: revision 9 is `Pass`; a stricter one could return).
  - Core update-authority plugins added later.
  - Relay 4663↔SOL route availability.
  - Exchange-rate drift between advance and settle.
  - The custody-trust downgrade (§2.2).
- **Audit:**
  - `SolNftWrapper` goes to an **outside contract audit**: ≈ 500 lines, mostly a diff against the audited `EthNftWrapper`, so a
    differential audit of ≈ 1 week;
  - the canister's custody line, buy judges and delivery go to **internal rounds plus an outside canister review** (≈ 2 weeks);
  - the monitor and reconciler custody comparison get internal review.

---

## 3. Robinhood-native NFTs

> **Built (2026-10-12, canister-only).** A Robinhood Chain NFT is an **EthNft listing on chain 4663 on the books home**, bought,
> held and delivered exactly like an Ethereum one; §3.2's `LocalNftWrapper` is not needed for it.
> - **The canister.** `EthCollection.chain_id` (an `opt`: none is `eth_nft.chain_id`) names each collection's chain, and
>   `EthCollection.token_bound` flags an ERC-6551 collection. `ethnet` runs one **side** per chain the collections live on. The config's
>   own chain keeps `ETH`; every other chain gets its own `EthState`, saved in the thirteenth element's `sides`. Each side has its own
>   reads, its chain's Seaport 1.6 (its pinned code hash), fee caps, gas pot, Ethereum line and nonce, pin floor, and its own buy ids
>   (`chain << 40`). Every side shares the one Solana money line (`LineOwner::Eth`; each takes only its own outcomes) and the one
>   Relay leg. The SOL leg pays out on `"robinhood"`, Relay's name for 4663, read from its real 4663 quotes. The way back is
>   `relayeth::ROBINHOOD_TO_SOLANA`, deposited on 4663 to the same depository. Custody is the same secp256k1 address on every chain;
>   caps are keyed `4663:0x…` (`custodycap::evm_key`; audit C3-13). The install refuses a collection whose chain is not in `evm_chains`,
>   collections on several chains on the EVM home, and the dormancy across chains. New endpoints: `eth_nft_state_of(chain)` and
>   `eth_line_clear_on(chain)`.
> - **Audit C3 fixes (2026-10-12, `docs/v9/audit-2026-10/c3-robinhood-chain.md`).**
>   - *Token-bound (C3-01).* A token-bound collection needs `EthCollection.nft_value_bps`: the share of a full account's price that is
>     the bare NFT. The install refuses one without it. Every listing and counted sale is taken at that share before the floor is cut
>     (`ethnft::nft_only`), so the floor posted, the module's value and the buy bound are the NFT's alone. A drained account listed at
>     the full floor is over the bound; nothing is paid for what an account holds.
>   - *Per-chain block counts (C3-02).* The sale window (7 days), the final depth (1,536 s), the re-anchor lag and the lag pages (a
>     day, 8 h) and the scan's chunks a pass are each chain's own blocks, from its block time (`chains::block_ms`: Ethereum 12 s,
>     Robinhood Chain 0.104 s; `ethnft::sale_blocks`). An NFT chain without a known block time is refused. A FIRST post (the module's
>     accepted floor 0) also needs its counted sales spread over a day of the chain's blocks, on every chain.
>   - *The 4663 Relay legs (C3-03).* The runner judges and hands in SOL → 4663 and 4663 → SOL (`relaysol.ts` `solLegOf`: `sol-rh`,
>     `rh-sol`). A `Funded` buy that no quote reaches for `FUNDED_QUOTE_SECS` (2 h) goes back to its module, freeing the one Relay leg.
>   - *Other fixes.*
>     - C3-04: a late payout on the side's own chain is the side's.
>     - C3-05: the monitor reads `eth_nft_state_of(chain)` for every side and pages it.
>     - C3-06: an upgrade that would orphan a side with work is refused.
>     - C3-07: the books listing is matched by chain and address.
>     - C3-08: stage 0 holds a 4663 collection to the launch file's 3 items at its `max_purchase_wei`.
>     - C3-09: the reconciler reads 4663 on two providers at least, every one answering, each checked by `eth_chainId`.
>     - C3-11: on 4663 the sale scan reads Seaport 1.6 only.
>     - C3-14: the install refuses chains whose buy-id ranges would meet.
> - **How to add one.**
>   1. The launch-file entry (`nft-pilot.json`, as `scripts/measure-robinhood-nfts.mjs` prints it: `chainId` 4663, the address,
>      `maxPurchaseWei`, `tokenBound`) and its `graduation-tiers.json` tier.
>   2. An `evm_chains` entry for 4663 with Seaport 1.6's code hash **on 4663**, which the ceremony reads back
>      (`cast keccak $(cast code 0x0000000000000068F116a894984e2DB1123eB395 --rpc-url https://rpc.mainnet.chain.robinhood.com)`).
>      Nobody has measured it offline; it differs from Ethereum's.
>   3. The `eth_nft` collection with `chain_id = opt 4663` (and for StonkBrokers `token_bound = opt true` with its
>      `nft_value_bps`, the bare NFT's share, set by the operator from sales of drained accounts: audit C3-01), `wrapper = module = "books"`.
>   4. The OpenSea slug in the runner's `ETH_NFT_SLUGS`.
>   5. The runner's env:
>      - `OPENSEA_CHAIN_4663`: OpenSea's API name for the chain. It is **not verified**; `robinhood` is the default.
>      - The runner must also read `eth_nft_state_of(4663)` (`EthNftDeps.sideStates`) and add `chain_id` and `token_bound` to its IDL
>        (`ops/src/ic.ts`). Done in the integration (2026-10-12): `runner.ts` `sideStatesOf` reads each other chain a collection names,
>        `OPENSEA_CHAIN_4663` (default `robinhood`) is passed as `openseaChains`, and the IDL carries both fields and both endpoints.
>   6. The reconciler's `RECONCILE_RH_RPC_URLS`. By default it uses the keyless 4663 pair; `ownerOf` is read on 4663. It needs two
>      providers, both answering (audit C3-09).
> - **StonkBrokers is included** (the operator, 2026-10-12): measured, **token-bound**, priced and bought as the NFT only (at its
>   `nft_value_bps` of the market's price, audit C3-01), with its own `max_purchase_wei` (≈ 1.25 × its floor, within the 25 ETH
>   ceiling) and its own tier above 85 SOL. The custody caps still bound the total: at stage 0, 3 items at most its `max_purchase_wei`
>   each (audit C3-08). Its risk is on the site's Risks page. The "do not list" below is the earlier view.
> - **The measurement.** `! OPENSEA_API_KEY=… node scripts/measure-robinhood-nfts.mjs` measures the twelve collections below on
>   OpenSea (slug by contract, stats, Seaport 1.6 listings, 30 days of sale events). It prints the criteria of
>   `ordinals-collections.md` §2 (≥ 30 single-item sales, ≥ 3 listings from ≥ 3 sellers, a floor of 1–10 SOL; a token-bound
>   collection uses its own cap instead), a verdict, and the `graduation-tiers.json` / `nft-pilot.json` entries:
>   - tier 85, or the next multiple of 85 that covers 1.33 × the floor;
>   - `T_wX = max(3, ⌈tier / floor⌉)`.
>
>   Tested offline: `node --test scripts/measure-robinhood-nfts.test.mjs`.

### 3.1 What is on chain 4663 [M, secondary sources; re-measure before deciding]

Robinhood Chain mainnet launched on 2026-07-01. It is an Arbitrum-based L2 and OpenSea supports it. Alchemy's NFT API covers
chain 4663. Coverage in August reported a summer burst of NFT activity, with daily NFT volume briefly around $1.05 M, above
Ethereum's on that day.

| Collection | Supply | Floor (mid-Aug 2026) | Notes |
|---|---|---|---|
| **StonkBrokers** (Clutch Markets) | 4,444 | ≈ 11-13.4 ETH | Free mint 2026-07-17; > $4.7 M volume. **Each NFT is an ERC-6551 account holding tokenized stocks** |
| **Chain Mancers** | 5,000 | ≈ 0.9 ETH | Released 2026-08-06; 3,750 minted by burning chain memecoins |
| **Cash Cats** | 10,000 (9,995) | ≈ 0.26 ETH | July 2026 |
| **Spritehood Wisps** | 44,444 | ≈ 0.0125 ETH | Sold out in ≈ 54 min on 2026-08-11 (≈ $1.28 M) |
| Robinhood Punks | ≈ 2,000 | n/a | ≈ 1,470 owners; the snapshot shows near-zero recent volume |
| Robinius and other small mints | 1,111+ | ≈ 0.0001 ETH | dust |

No source gives September or October figures. OpenSea's pages need its API (with a key) for live stats. **The figures above are
stale for a listing decision.** The operator's stop note ("no volume on Robinhood Chain") was about coin trading; the NFT side had
an August burst whose persistence is unknown.

**Already done (2026-09-23, `docs/v4/nft.md` §1):**
- **Seaport 1.6** is the only Seaport on 4663, at `0x0000000000000068F116a894984e2DB1123eB395`. The 1.5 address and the conduit
  controller have no code there.
- OpenSea is the only 4663 venue whose protocol has code on chain.
- **Nosh's `CollectionRegistry`** (`0x83B3…4B0f`) was read end to end. It lists 12 real ERC-721s on 4663, with their addresses:
  - StonkBrokers `0x539c…abf0`, Chain Mancers `0x797a…ca57`, CASHCAT `0xe3b3…54b1fa`;
  - Opencatz AI, The Royal Mechanica, Ape Hood, RH MACHINES, Rekt Tradooors, QUOTRONS, Hashcats, Rare Friends Genesis, AGNTS.

  Across all twelve, Nosh's vaults had swept 63 NFTs.

Use that list (with addresses) as the starting set for the fresh measurement. "Cash Cats" in the press is the same CASHCAT contract.

### 3.2 Same-chain wrapping: `LocalNftWrapper` (new contract; its buy path reuses built code)

**Status: partly done.**
- **Already built and tested:** `src/nft/NftSweepVault.sol` and **`src/nft/SeaportGuard.sol`**. That is a **permissionless,
  contract-checked Seaport 1.6 buy on 4663**: the guard decodes the calldata and checks every order, the vault accepts only the
  listed collection and only during a sweep, and nobody holds the money. The tests are `test/nft/NftSeaportGuard.t.sol` (29 KB),
  `NftSweepVault.invariant.t.sol`, `NftSecurity.t.sol` and `NftFuzz.t.sol`.
- **But:** that vault is a fee-funded raffle/burn sweep (the v4 design), not a wX wrapper behind a FloorModule. **Not started:** the
  wrapper itself.

`EthNftWrapper` cannot be reused either: its constructor requires `chainId ≠ block.chainid` and a code-less custodian. A same-chain
wrapper is simpler and **stronger**:

- **Custody is the contract itself.** `onERC721Received` from the listed collection mints 1e18 wX to the recipient that the
  transfer's `data` names. No custodian attestation is needed, so the trust is lower than for any other kind. Deposits can be **on
  at launch**.
- **The buy reuses `SeaportGuard`, and anyone can perform it.** After `FloorModule.advance`, the wrapper's `sweep(seaportCalldata)`
  fills an allow-listed Seaport 1.6 order into the wrapper itself. The guard checks the order, and the price must be ≤ the module's
  `maxItemCost`. The result is a `moduleMint` and `settleAdvance`. **No Relay leg and no canister signature are needed.**
  - **Today [R]:** `advance` is custodian-only and pays the custodian. The keyless form needs either a module-side "advance to the
    wrapper" or the canister calling `sweep` (it is permissionless) with ETH from an ordinary `advance`.
  - **Recommendation:** the canister path first, since the module stays unchanged. The canister's Seaport judge already allows
    "other EVM chains" (`Init.eth_nft.chains`).
- **Redemption.** `resolve` draws an item, and **`claim(id)` transfers it on chain** to the ticket's EVM destination. There is no
  `recordDelivery`, `noteSent` or refund timer. A burn ticket can send to `0x…dEaD` (as `NftSweepVault`'s burn).
- **Floor.** The canister posts from verified 4663 Seaport listings and sales, as on Ethereum.
- **Effort:**
  - contract 3-4 d (`SeaportGuard` and the `EthNftWrapper` ticket machine are reused);
  - canister same-chain buy 2-3 d;
  - deploy kind and measurement 2-3 d.

  Total **7-10 d**. Audit together with `SolNftWrapper`; it is ≈ 300 new lines.

### 3.3 Which ones, if any

Apply the criteria Rook already uses (`ordinals-collections.md` §2): ≥ 30 single-item sales in 30 days, ≥ 3 listings from ≥ 3
sellers on the buy venue, and a floor inside the purchase bounds.

- **StonkBrokers: do not list.** Each item's value is mostly the tokenized stocks in its ERC-6551 account.
  - A seller can drain that account before a sale, so the items are not interchangeable at a floor.
  - Custody of an NFT whose account holds securities tokens brings a securities question that Rook should not take on.
  - The account's assets would also move with any redemption.
- **Chain Mancers and Cash Cats: candidates** if a fresh 30-day measurement passes. Their floors (≈ 0.9 / 0.26 ETH) sit well in
  the 85-tier mapping.
- **Spritehood Wisps: a candidate only with the cheap-collection variant** (§1.3), since 0.0125 ETH is under the canister's 0.05
  ETH minimum. Its 44,444 supply gives depth.
- **Robinhood Punks and the dust mints: no.**

**Recommendation.** Build `LocalNftWrapper` in R3 (it is cheap, and the trust model is the best Rook has). List at most two native
collections, chosen by a measurement taken in the week of R3 with an OpenSea API key. Listing them sends a cheap, positive signal
on the chain itself.

---

## 4. Features from the Solana phase

### 4.1 Raffle

| | Robinhood `SweepVault` RAFFLE [R] | Solana raffle (`raffle-design.md`) |
|---|---|---|
| Funded by | 40 % of each coin's trade fees and shared-pool fees, per coin | Treasury reserve overflow (`raffle_bps` 50 %), per collection |
| Prize | Whole wX bought by `sweep` through `EtchRouter` at ≤ the keeper's posted cap | One whole wX minted through the module's public `mint` (`fund_prize`) |
| Who can win | Holders of **the coin that paid**, at a snapshot time drawn by drand inside the previous 24 h | Holders of **all graduated coins of the collection**, weighted by the backing they carry, at a recent slot |
| Cadence | Keeper-driven (`announceDraw` → 15-min challenge → `openDraw` → `requestDraw` → `resolve` → `claim`) | Weekly per collection |
| Randomness | drand evmnet **verified on chain** (`DrandEvmnet`, BN254 pairing) | drand round pinned, keeper-proved (on-chain verification was a Solana TODO) |
| Trust | Ticket list is the keeper's, with a 15-min recompute window and owner/guardian cancel | Snapshot is the keeper's, published for recomputation |
| State | Deployed per collection in R1; keeper `draws.ts` and resolver exist; `phase1-nft.md` leaves NFT-collection draws OFF in Phase 1 | Built, tested in LiteSVM |

**Status: already done.**
- Contracts: `SweepVault` RAFFLE / BURN / YOYO.
- Keeper: draws with a two-relay drand seed and the sentinel (`f07ba72`), and the keyless resolver.
- Monitor: `monitor/src/draws.mjs` recomputes draw roots on both RPCs.
- Site: the raffle section of the Robinhood coin page (`web-rook/src/ui/Raffles.tsx`, `pages/Coin.tsx:125`: announce, pin, resolve,
  claim, void, by anyone) and the redemption draw page (`Draw.tsx`).
- Indexer: draws are read by `indexer/src/wrapped/draws.ts`.

**Left:** only switching draws ON for NFT collections (`phase1-nft.md:352` has them off in Phase 1) and the caps' ceilings.

**Recommendation.** Robinhood's raffle is already built, audited and verifies drand on chain, which is better than the Solana one.
**Turn it ON for NFT collections in R1** (post caps, run draws). In the restyled terminal, show `Raffles.tsx`. The Solana-only
`RafflePanel.tsx` stays with the Solana build.

Porting the weekly backing-weighted collection raffle would need a `ProtocolTreasury` change (it has no raffle split) and a new
contract. **Not recommended**; the per-coin draw rewards the coin that paid the fees.

**Effort:** keeper and monitor configuration to turn NFT-collection draws ON, plus a drill: 1-2 d.

### 4.2 Graduation tiers

**Status: partly done.** The mechanism, golive-check's readout and the site/indexer per-collection target are built (`613173e`,
`e370b12`). The mapping is §1.3.
- **Left (configuration):**
  - Pudgy's entry in `nft-pilot.json`;
  - the cheap-collection `maxEthBacked` values for R2;
  - a `StackChecks` rule on units per graduation.
- **Site:** the old pages already read each collection's own target from the factory. Only the tier label in the terminal is new.
- **Effort:** 1 d. **Pudgy's change must land before the R1 ceremony** (the R1 targets are immutable).

### 4.3 The site (web-rook)

**What exists [R]:**
- **The Solana terminal** (`SolPages.tsx`: `SolHome`, `SolCoins`, `SolCoin`, `SolLaunch`, `SolCollections`, `SolTracked`), built
  on `src/rook/sol/*`:
  - `market.ts`, `board.ts`, `indexer.ts` (the `/v1/rook` client), `launchWizard.ts`, `demo.ts` / `demoMarket.ts` /
    `demoRaffle.ts`, `raffle.ts`;
  - `collections.ts` and `collectionLinks.ts` (official links);
  - Solana wallet adapter and transaction building.
- **The old Robinhood pages** (`AppPages.tsx`, unrouted): `Coins`, `Coin`, `Launch`, `Collection`, `Reserves`, `Redeem`, `Draw`,
  `Rent`, `Wallet`, with wagmi (`Web3.tsx`, chain 4663, `explorer.mainnet.chain.robinhood.com`).
  - **Already done (commit `468f2c5`):** the Robinhood **front page is a board** (tabs, sorts, search, king of the hoard, live
    tape), and the **coin page** has a lightweight-charts chart, a ticket, and Trades / Holders / Info / Activity tabs.
  - **Also done:** the launch page has the wake step and fee tiers (`rook/dormancy.ts`, `rook/tiers.ts`, `Launch.tsx`); the redeem
    and draw pages have proofs checked in the browser; there is a reserves page.
  - **Demo is partly done:** a fixture mode replays a demo recording (`rook/fixtures.ts`, `data.ts`).

  **So the Robinhood site needs re-routing and a restyle, not a rebuild.** Of the Solana phase's features, what has no Robinhood
  counterpart is:
  - the terminal layout (hero, trending strip, New / About to graduate / Graduated / Live trades);
  - official links per collection;
  - the tier label;
  - the synthetic demo market;
  - the coin metadata registry. It is specified in `docs/v6/site-design.md` §6.3 and **not built**; `Launch.tsx:151` says so on
    the page.

**The design: one UI, two chain adapters.** This is the larger option. The smaller option is to re-route `AppPages`, port the
terminal layout and links onto the old pages' data hooks (`rook/data.ts`), and leave `SolPages` untouched. **Recommended: the
smaller option** for R1.5. The adapter below is worth doing only if both chains will run at once.
1. Extract a `Market` interface from `src/rook/sol/market.ts` and `hooksMarket.ts`:
   - board rows, coin detail, chart, trades, holders;
   - a quote for a buy or sell;
   - `buildBuy` / `buildSell` / `buildLaunch`;
   - raffle state.

   The terminal components consume only this interface.
2. **A Robinhood adapter** (`src/rook/rh/*`):
   - viem reads of `CurveFactory` / `FrogCurve` / `LiquidityLock` / `FloorModule`;
   - wagmi writes: `FrogCurve.buy` / `sell` on the curve, and `EtchRouter.buy` / `sell` after graduation;
   - the indexer's `/v1/rook` for history.

   Amounts are ETH (18 decimals) and the progress bar reads the raise target. The old `pages/*.tsx` supply the tested call shapes.
3. **The launch wizard.** `CurveFactory.launch(name, symbol, policy, sweepOnly, feeBps)` has **no metadata URI**: a Robinhood coin
   has no on-chain image or description. Add an off-chain metadata record:
   - the creator signs `{coin, image hash, description, links}` (EIP-191) after the launch transaction;
   - the indexer stores the record and serves it, after checking `creatorOf(coin) == signer`;
   - the image is stored by content hash.

   The wizard keeps its steps: collection, name, symbol, policy (RAFFLE; BURN for EVM and Ordinals collections, **not** for
   Solana collections), first buy bounded by the collection's raise, and anti-sniper copy (Robinhood has a +50 % launch fee
   decaying over 10 min).
4. **Demo mode:** `demo.ts` / `demoMarket.ts` run in ETH units, with the 5 / 7.5 / 10 ETH raises and the Robinhood fee split.
5. **Official links:** `collectionLinks.ts` is chain-agnostic. Add Magic Eden and Tensor links for the 12 Solana collections and
   OpenSea (4663) links for the native ones.
6. **Wallets:** wagmi (injected, WalletConnect, Coinbase/Robinhood Wallet). Solana collections need a Solana address only as a
   **redeem destination**, a text field validated base58 and on-curve, so no Solana wallet connection is needed.
7. **Routing:** the Robinhood adapter becomes the default build, and `SolPages` stays in the tree. The Solana-only texts listed
   in `graduation-tiers.md` §4 become chain-aware.

**Effort (the recommended smaller option):**
- re-route `AppPages` and port the terminal layout onto `rook/data.ts`: 3-4 d;
- tier label, official links and Solana / native link sets: 1 d;
- the metadata registry, client and indexer halves (§6.3 of the site design): 3-4 d;
- demo: point the fixture set at the new collections, or port `demoMarket.ts` to ETH: 1-2 d.

Total **8-11 d**. The full two-adapter design would add ≈ 5 d.

### 4.4 The indexer

- **Status: mostly done.** The charts, trades and holders that the coin page needs are already served (commit `468f2c5`); each
  collection's own graduation target is served too (`e370b12`).
- **What exists (Robinhood) [R].** `indexer/src/rook/*` serves:
  - `/v1/rook/board`, `/activity`;
  - `/coins/:addr/chart` (1m-1d), `/trades`, `/holders`;
  - `/wallets/:addr`, `/catalog`.

  It reads the curve's `Bought` / `Sold`, the token's `Transfer`s, `Graduated` and, after graduation, the v4 `PoolManager.Swap`s of
  the coin's pool (`curve.ts`). `indexer/src/wrapped/*` serves the wrapper, floor module and draws.
- **What is missing, by comparison with the Solana indexer (`SOL_ENDPOINTS`):**
  - `/v1/rook/coins?sort=new|graduating|graduated|trending|mcap|active&collection=&q=`, `/coins/:addr` detail and
    `/collections`;
  - USD candles beside ETH. `usd.ts` exists; wire it into the chart response;
  - **A persisted fold.** `curve.ts` reads each coin's history **on demand** from its launch block. With Dwellir's ≤ 500-block
    `eth_getLogs` pages and a fast-block L2, that is thousands of calls per cold coin. Fold the Rook events into SQLite
    incrementally, as the v2 follower does;
  - the coin metadata store (§4.3, item 3);
  - raffle draws per coin for the panels (from `wrapped/draws.ts`);
  - the 4663 deploy file per R-phase, appended (§1.2, item 3);
  - `SolNftWrapper` and `LocalNftWrapper` ABIs.

  The list, detail and collections endpoints serve the terminal layout. The old pages work without them, from the board.
- **Effort:** 5-7 d. The persisted fold is the must-have (3-4 d); the rest is 2-3 d.

### 4.5 Fund and fees

- **No community deploy fund.** Deploying on 4663 costs gas in ETH (an L2, small), paid by the operator's deploy key. The Safe
  sends the owner steps.
  - Remove `fund.ts`, the fund page and `06-deploy-fund.md` from the Robinhood build.
  - Replace them with a short "who paid for the deploy" line.
- **Fees (Robinhood, [R]):**
  - 1 % trade fee by default (1-8 % tier at launch), split 40 % sweep vault (raffle or burn) / 30 % creator / 5 % collection
    reserve / 5 % protocol liquidity / 20 % treasury;
  - +50 % launch fee decaying over 10 min, all of it to the sweep vault;
  - graduation fee min(5 %, 0.25 ETH);
  - 12.6 % of the raise to the shared wX/ETH pool, all liquidity locked forever;
  - treasury 90 % reserves / 10 % ops.
- **Docs:** rewrite `content/docs/05-fees.md`, `02-how-it-works.md` and `03-launching.md` for these.
- **Effort:** 1-2 d.

### 4.6 The X thread

No thread copy is committed in the repository [R]. A draft to adapt:

1. Rook is live on Robinhood Chain. Launch a coin on a blue-chip NFT collection. When it graduates, the raise buys real NFTs,
   and your coin is backed by them.
2. R1 collections: Milady, Azuki, MAYC, Pudgy Penguins, BAYC, Doodles, Moonbirds, CloneX, Lil Pudgys, Checks VV, Identity MD,
   and Bitcoin Frogs on Bitcoin.
3. How it works: a bonding curve raises 5-10 ETH, depending on the collection's floor, so every graduation buys at least one NFT.
   The NFTs are held by an on-chain-governed canister on the Internet Computer. Each is wrapped 1:1 as wX on Robinhood Chain.
4. Any whole wX redeems for a real NFT, chosen by drand randomness and delivered to your wallet.
5. Fees: 1 % per trade. 40 % funds raffles for that coin's holders, 30 % goes to the creator, and the rest to reserves and
   locked liquidity. All liquidity is locked forever.
6. Next: 14 more Ordinals collections, 12 Solana collections (Mad Lads, SMB, Claynosaurz…), and native Robinhood Chain NFTs.
7. Links: site, docs, contracts on the explorer, audit reports.

**Effort:** 0.5 d. Have the operator review it, and check every claim against the deployed parameters.

---

## 5. Phased plan

Dates assume work starts on 2026-10-12 with one engineer per track (two tracks in parallel from R1), and that the parallel track
closes the R1 blockers in the week of 2026-10-12. **The G1 outside-audit decision is still open** (`robinhood-stop-2026-10-09.md`
§2) and gates every date.

| Phase | Ships | Work (d) | Target | Gate |
|---|---|---|---|---|
| **R1** | 11 Ethereum collections + Bitcoin Frogs, as planned. Tier values §1.3 (Pudgy 10 / 10; BAYC as tested). `SweepVault` RAFFLE ON. The **existing Robinhood pages re-routed** (board, coin chart / trades / holders, launch with wake, redeem, draw, reserves: already built, `468f2c5`) | tiers 1; raffle 1-2; re-route ≈ 1 | **≈ 2026-10-19** | the other track's blockers (canister build, Dwellir RPC, dry run, ceremony), G1 decision, BAYC and Pudgy re-measured on the ceremony morning |
| **R1.5** | Terminal layout over the Robinhood data hooks, official links, tier label, metadata registry, demo; indexer persisted fold, list / detail / collections, USD; fees copy; X thread | site 7-10 + indexer 5-7 + copy 1-2 | **≈ 2026-10-28** | internal review; a staging fork with real ABIs |
| **R2** | 14 Ordinals, the second wave already planned in `docs/v8/ordinals-wave.md`: deploy-stack run with the existing treasury/directory; list only the ones passing the wake check; the rest added later by new runs | 20-27 | code ≈ 2026-11-06; review → **≈ 2026-11-20** | internal rounds (5 d) + outside canister review (≈ 2 wk, overlapping R3 work); members-file reruns |
| **R3** | 12 Solana collections (`SolNftWrapper`, custody line, Tensor/mmm buys, Relay 4663↔SOL, delivery, floors) + `LocalNftWrapper` and ≤ 2 native collections | 58-77 | code ≈ 2026-12-18; audit → **≈ 2027-01-15** | outside contract audit (`SolNftWrapper` + `LocalNftWrapper`, ≈ 1-1.5 wk) + outside canister review of the Solana line (≈ 2 wk); one funded mainnet purchase per marketplace; the 4663→SOL Relay spike in week 1 |

**Order inside R3.**
1. The Relay 4663→SOL spike and the collection pinning, in week 1. If 4663→SOL has no Relay route, R3 needs a two-hop route
   (4663→Ethereum→SOL) and grows by ≈ 1 week.
2. `SolNftWrapper` and `LocalNftWrapper`, sent to audit early.
3. The custody line and the Tensor buy (most of the depth), then mmm.
4. Delivery and floors.
5. Rehearsal and one funded purchase.

### 5.1 Audit plan

| Item | New / changed | Lines (est.) | Review |
|---|---|---|---|
| `SolNftWrapper.sol` | new (sibling of `EthNftWrapper`) | ≈ 500 | outside, differential against `EthNftWrapper`; invariant fuzz in CI |
| `LocalNftWrapper.sol` | new | ≈ 300 | outside, same engagement |
| `DeployStack` / `StackChecks` (N Inscriptions, reuse, kinds sol/local, per-collection raises and caps) | changed scripts | ≈ 400 | internal + tamper tests; not deployed code, but it decides every immutable |
| Canister: multi-Inscriptions (EVM home) + per-collection minimum | changed audited code | ≈ 1,500-2,500 | internal rounds + outside canister review (with the next item, one engagement if timing allows) |
| Canister: Solana custody line, Tensor/mmm judges, Relay 4663↔SOL, delivery, SolNft dormancy | new | ≈ 4,000-6,000 | internal rounds + outside canister review; regression PoCs as in `docs/audit/2026-09-27-nft/` |
| Keeper multi-feed, runner, reconciler custody comparison, monitor D8 for Solana | changed / new | ≈ 1,500 | internal |
| Site and indexer | changed | — | internal; no keys, read-only |

**Unchanged and already reviewed:** `FloorModule`, `CurveFactory`/`FrogCurve`, `LiquidityLock`, `EtchRouter`, `SweepVault`,
`PosterSource`, `WakeDesk`, `ProtocolTreasury`, `Wrapper`/`WrapClaim`, `EthNftWrapper` and `ListingRegistry`. Nothing in this
plan edits them. The only contract change considered is raising `HARD_MAX_EARLY_RAISE` for BAYC (§1.3), and it is not recommended. `NftSweepVault` / `SeaportGuard` (v4, built and tested, not deployed) are reused by `LocalNftWrapper`; the guard is included in that audit's scope.

### 5.2 Decisions the operator owes

1. **Before R1:**
   - Pudgy Penguins to 10 / 10 ETH (recommended);
   - BAYC: keep the tested 10 / 10 / `convBps` 1000 / reserve 0 (recommended), or apply the 1.33 value rule literally;
   - the 85-tier raise: 5 / 8 (tested) or 4 / 7;
   - NFT-collection raffles ON in R1.
2. **R2:**
   - the cheap-collection variant (2-3 ETH raises, larger `maxEthBacked`) for Runestone, Weirdos, RSIC, Natcats and Punks;
   - per-collection purchase minimum values.
3. **R3:**
   - `convBps` 600 for Solana collections;
   - leaving out Magic Eden M2 listings at first;
   - BURN not offered on Solana collections;
   - which native collections, from a fresh measurement;
   - StonkBrokers excluded.
4. **Throughout:** accept the custody-trust downgrade for Solana NFTs on Robinhood (custodian-attested plus reconciler and public
   proof of custody, instead of rook_core's on-chain check).

---

## 6. Sources for §3 (researched 2026-10-10)

- Crypto Briefing, "Robinhood Chain posts $1.05 million in 24-hour NFT trading volume" (2026-08-13):
  https://cryptobriefing.com/robinhood-chain-surpasses-ethereum-nft-volume/
- KuCoin News, "NFT Summer Returns on Robinhood Chain: Key Projects to Watch" (2026-08-14):
  https://www.kucoin.com/news/flash/nft-summer-returns-on-robinhood-chain-key-projects-to-watch
- CryptoTicker, StonkBrokers and ERC-6551: https://cryptoticker.io/en/robinhood-chain-nfts-stonkbrokers-erc-6551/
- AMBCrypto, StonkBrokers floor: https://ambcrypto.com/stonkbrokers-nft-on-robinhood-hits-12-6k-in-2-weeks-can-momentum-hold/
- Alchemy, Robinhood Chain RPC (chain 4663, NFT API): https://www.alchemy.com/rpc/robinhood.md
- OpenSea, Robinhood Punks: https://opensea.io/collection/rhpunks/explore
