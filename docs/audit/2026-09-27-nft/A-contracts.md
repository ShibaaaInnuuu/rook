# Rook security review, 2026-09-27 (nft-side), area A: the contracts

Reviewer A (contracts), 2026-09-27. Checkout `/Users/jlohayon/Documents/Rook-audit-nft-A`, a detached worktree at
**`b48b3ba`** (`nft-side`, "Docs: phase1-nft as built …"); the diff base is GitHub `main` = **`47a9b94`**. Nothing under `src/`,
`script/`, canister code or `docs/` was modified in either worktree. The only additions are PoC tests under `test/audit_nft_a/` in
my own worktree, copied here to `poc/A/` (they are not part of any build). The worktree was removed afterwards. I made no commit,
push, deploy, signature or transaction, and I used no key. No chain was contacted: every run was a local forge test.

**Mid-review scope change (operator, 2026-09-27):** the pilot launches **Milady, Azuki and MAYC together** in one NFT-only stack
(one `EthNftWrapper` + `PosterSource` + `FloorModule` per collection, one registry, one canister custodian). This supersedes D1.
§6 is the cross-collection isolation section that change asked for, and A-L-3 and A-L-4 come from it.

## 1. Scope and commit

`git diff 47a9b94..b48b3ba -- src/ script/`: 8 files, +783/−61 lines.

| Path | Status | Reviewed |
| --- | --- | --- |
| `src/nft/EthNftWrapper.sol` (446 lines, new) | ON (the Phase-1 collections) | Line by line, and against `src/Wrapper.sol` for every earlier fix |
| `src/floor/PosterSource.sol` (35 lines, new) | ON (each NFT module's `IKeeperSource`) | Line by line; every `registry.*` read in `FloorModule` |
| `src/floor/FloorModule.sol` (+121: `advance` / `settleAdvance` / `returnAdvance`, `transitUnits`, `_liveUnits`) | ON | The diff line by line; `burn`, `_tierPrices`, `_release`, `_pauseFloor`, `_burnTarget`, `price`, `gradPrice`, `maxItemCost`, `_capOrSnapshot`, `hasRoomFor`, `holdsRefunds` and `itemArrived` re-read because they now run on the canister's floor |
| `script/DeployStack.s.sol` (NFT-only mode, `ETCH_STACK_KIND=nft`) | Deploy | Diff, plus `run()` and `_collection` / `_readBack` / `_ownerSteps` in full |
| `script/StackChecks.sol` (`nftOnly`) | Deploy | Diff and `defaultParams`, `stageOf` |
| `script/StackRehearsal.s.sol` (phase `advance`), `script/deploy-stack.sh` (`forKeeper` / `forCanister`), `script/golive-check.sh` (NFT checks) | Deploy / go-live | Diffs |

**Launch configuration assumed** (operator decisions, 2026-09-27):

- D1, as updated: Milady, Azuki and MAYC at once.
- D2: drand-random redemption.
- D3: 0.01 ETH gas seed.
- D4: stranded ETH waits.
- **D5: holder deposits ON** (`EthNftWrapper.mintDeposit` is live).
- D6: an OpenSea key in the runner.
- D7: floor = rank 2 of ≥ 3 listings, ≥ 3 sales, +10 %/−15 % band.
- D8: the guardian pauses the keeper when a floor is > 20 % off both references.
- D9: Blur listings are not bought.
- The module parameters are `StackChecks.defaultParams(1e18, 0)`: reserve 25 %, 5 units an hour each way, public cap 10, `maxEthBacked` 40, `maxItemsPerDay` 3. The wrapper's `initMintLimit` is 3.

**Signed messages.** No contract in scope verifies a signature. Every canister action is a transaction from its threshold-ECDSA
EOA, gated by `msg.sender == custodian` (the wrapper, and the module's conversion calls) or `== registry.keeper()` (the
`PosterSource`: the same address). The drand proof goes through the beacon, unchanged.

## 2. Methodology

The same sources as the 2026-09-26 reviews (areas B and C), used as checklists, pass by pass:

- Trail of Bits *Building Secure Contracts* (guidelines, workflow);
- OWASP Smart Contract Top 10 (2025), SC01–SC10;
- the SWC registry (107, 104, 105/106, 113/128, 116, 120, 132);
- Consensys best practices (oracles, force-fed ETH, circuit breakers, rotation);
- the Cyfrin/Solodit checklist (DoS, griefing, oracle/price, rounding, centralization).

Passes:

1. **Docs.** `docs/v7/phase1-nft.md` (§0–§3, §5, §7, §9, §10, §11) and `docs/v6/relay-conversion.md` §0–§2. For the audited
   baseline: `docs/audit/2026-09-26/README.md`, `B-floor.md` and `C-treasury-roles-deploy.md` in full, `fixes-B-C-keeper-draws.md`,
   `review2/R2-contracts.md`, `review2/fixes-R3-contracts.md`, `review3/fixes-R4-contracts.md`, `review4/R4-check.md` and
   `review4/fixes-golive-lows.md` (contract parts). `A-curve.md` was skimmed.
2. **Invariants and trust** (§3).
3. **Line by line.** `EthNftWrapper` against `Wrapper.sol` (§7 lists every earlier fix and its status). `PosterSource` and every
   place the module reads its `registry`. The `FloorModule` diff. The DeployStack NFT path.
4. **Attacks.** Forge PoCs at the **stage-0 parameters** (not the suite's `EthNftBase`, which runs `maxItemsPerDay` 40, a mint
   limit of 1,000 and reserve 0). Includes a deposit-aware fuzz of the ledger.
5. **Canister cross-reads, only where the contract side depends on them** (the other reviewer owns the canister):
   - `ethnft.rs:507-542` `floor_from`;
   - `ethnft.rs:684-697` `judge_transfer`;
   - `ethnft.rs:747-749` the deposit recipient rule;
   - `ethflow.rs:1371-1374` when deposits are minted.
6. **Suites.** The existing suites, then mine (§9).

Slither was **not run**: it is not installed on this machine, and I did not install software for this review. The Ethereum fork
proof (`test/fork/EthNftSeaport.fork.t.sol`) was not run, because it needs an archive-RPC anvil fork.

## 3. Threat model and invariants

**Trust assumptions (on 4663):**

- **The canister** (one threshold-ECDSA address) is now the custodian of every NFT collection, the recorder of deliveries, the
  payee of every advance **and** the floor poster of every NFT module. It is fully trusted for mints (bounded by H11), conversions
  (≤ `maxItemCost`, 3 a day) and, now, the floor (±5 %/h, ±20 %/day, pausable).
- **The runner** (ops, holding the OpenSea key) is **not** trusted. It hands in listings, sales, quotes and deposit claims, which
  the canister verifies, but it chooses *which* real listings and sales to hand in.
- **The Safe** owns the registry: pause, unpause, rotation, guardian.
- **The guardian** (monitor host) can only pause, and D8 automates that.
- **The deploy EOA**: see A-L-2.

| # | Invariant (claimed by code or docs) | Status |
| --- | --- | --- |
| W1 | `totalSupply == (held − openTickets) × ONE + ethBacked` (`EthNftWrapper.sol:34`) | **Holds**, with deposits, all three redeem kinds, pinned and resolved refunds, and advance/settle/return interleaved (fuzz, 512 runs + 20-seed non-vacuity) |
| W3 | A ticket needs an unpromised item: `held ≥ openTickets + 1` (`:326`); every open ticket has an item | **Holds** (fuzz) |
| W4 | A ticket draws only among items free when it was burned (pending stamps, `:300`, `:351`); a deposit after a burn is never drawn by it | **Holds** (the fuzz asserts every deposit's stamp = the ticket count at its mint) |
| W5 | One token id held once (`itemOf`), one `(tokenId, tx)` source mints once ever (`sourceUsed`) (`:291-295`) | **Holds** |
| W6 | Only the custodian mints, records and marks; only the wired module mints/burns ETH-backed units | **Holds** |
| W7 | No redraw (C-02); a pinned ticket refunds after `PIN_TIMEOUT` and the order skips it (E-03); a resolved refund needs `REFUND_GRACE` + mark, or `LATE_DELIVERY_WINDOW`, on both clocks (D M-1 / canister N-2) | **Holds**: the same code as `Wrapper.sol:486-617`, re-keyed (§7) |
| W8 | Redemption cannot be switched off (phase1-nft §7 "What does not change") | **Violated in effect** by a free, repeatable relist lock (**A-M-2**) |
| M2 | Module solvency, `C + R + T = E` with a unit in transit | **Holds** (`_books()` in every PoC and in the fuzz) |
| M4 | A burn never pays more than the drawn tier's backing per unit, β | **Holds**. But β is itself set by the posted floor (graduations at `gradPrice`), see A-M-1 |
| F1 | "A high floor drains nothing: burns cannot pay more than a unit's backing" (`phase1-nft.md:507`) | **Violated** once deposits are ON (**A-M-1**) |
| F2 | The poster is fenced by ±5 %/h and ±20 %/day, and the registry's pause zeroes it at once | **Holds** (existing `PosterSource` tests; `test_A_L3_…`) |
| F3 | An NFT-only stack leaves nothing with the deploy key ("The deployer owns nothing", `DeployStack.s.sol:202`) | **Violated** (**A-L-2**) |
| H11 | Items a day bounded by the wrapper's mint limit and the module's `maxItemsPerDay` | **Holds** per wrapper, but purchases and deposits share one pool (**A-L-1**) |
| X1 | One collection's ETH, NFTs or units cannot back or redeem another's | **Holds** on chain (§6a) |
| X2 | One collection's floor incident is contained to that collection | **Violated**: the only floor switch is registry-wide (**A-L-3**) |

## 4. Findings

| ID | Severity | Title | Where (at `b48b3ba`) | Reachable in launch config? | PoC |
| --- | --- | --- | --- | --- | --- |
| A-M-1 | **Medium** | Deposits (ON) plus module burns turn a floor posted above the market, within the module's limits, into ETH paid out of the module. The design's "a high floor drains nothing" is false | `EthNftWrapper.sol:282-302`; `FloorModule.sol:501-521, 525-541, 1244-1246, 1329-1333`; claims `docs/v7/phase1-nft.md:507`, `FloorModule.sol:50-51` | **Yes** (D5; needs only a runner that withholds, or a rank-1 listing under rank 2) | `AuditNftA_M1_DepositBurn`, 6 tests: **PASS (attack works)**. +0.272 ETH/day at +10 %, +0.523 at +19 % |
| A-M-2 | **Medium** | `redeemToRelist` is a free, repeatable option: a holder of `held − openTickets` units keeps every NFT on its own tickets, and redemption is closed to everyone else | `EthNftWrapper.sol:320-323, 325-326, 407-430`; `ethnft.rs:688` | **Yes** (3 units at stage 0) | `AuditNftA_M2_RelistFreeze`, 2 tests: **PASS** (30 days frozen, 0 cost) |
| A-L-1 | Low | Purchases and deposits share the wrapper's H11 mint room. Deposits minted while a purchase is bridging or waiting for a listing hold its advance open (`AdvanceOpen` for everything else) for up to 24 h | `EthNftWrapper.sol:214-225, 287-302`; `FloorModule.sol:684-686`; `ethflow.rs:1371-1374` | Yes (D5) | `AuditNftA_L1_MintRoomStarvation`: **PASS** |
| A-L-2 | Low | The `EthNftWrapper`'s `deployer` is the deploy EOA for good: after launch it alone cuts the mint rate at once or proposes raises, and a stage-1 raise needs it. The deploy log says "The deployer owns nothing"; golive-check does not read it | `EthNftWrapper.sol:174, 189-211`; `DeployStack.s.sol:166-167, 202, 259-260` | Yes | `AuditNftA_L2_DeployerKeepsMintLimit`: **PASS** |
| A-L-3 | Low | One floor switch for every collection. D8's pause for one collection's floor zeroes every `PosterSource` and the registry keeper: every module's mints, conversions and curve closes stop. There is no per-collection floor pause, and the listing pause gates only wrapper mints | `PosterSource.sol:26-28`; `FloorModule.sol:864-866, 876-877, 1142-1144, 1335-1337` | **Yes** (multi-collection launch) | `AuditNftA_L3_PauseIsGlobal`: **PASS** |
| A-L-4 | Low | DeployStack's NFT mode deploys one EVM collection per run, each with its own treasury and directory, and refuses a second run on 4663 while the stack file exists. The three-collection launch is not deployable or checkable as one stack | `DeployStack.s.sol:113, 129-131, 159-176, 235-237`; `deploy-stack.sh:34`; `golive-check.sh` `ethcheck` | Yes (multi-collection launch) | analysis |
| A-I-1 | Info | The NFT modules' floor role cannot be rotated (the poster is immutable). The E-02/C-11 fix (registry-read, 7-day rotatable keeper) does not reach them | `PosterSource.sol:14-23` | Yes | — |
| A-I-2 | Info | An open advance has no on-chain timeout: a stranded one (D4) blocks every later conversion of its module. `advanced − cost` leaves the module for good; it is 13.3 % of each item at a +10 % floor | `FloorModule.sol:684-712, 715-731` | Yes | numbers in A-M-1 |
| A-I-3 | Info | B-06 carries into `settleAdvance` (absolute wX balance). A deposit may name the module as its recipient | `FloorModule.sol:720`; `ethnft.rs:748` | Custodian-only | — |
| A-I-4 | Info | Deposits make the item set adversely selected. Uniform draws let a depositor swap the worst NFT they own for the set's average; `Item.deposit` is recorded and never used | `EthNftWrapper.sol:82-86, 356` | Yes (D5) | — |
| A-I-5 | Info | Every NFT collection's wrapper reads the stack's one listing pause by default (`ETCH_NFT_LISTING`), and no module reads a listing pause (B-09 carried) | `DeployStack.s.sol:165`; `EthNftWrapper.sol:289` | Yes | shown in `test_A_L3_…` |
| A-I-6 | Info | A3-I1 carried, and likelier now: an ordinary stale module (the runner or its OpenSea key down, too few verified listings) refuses burns for up to 7 days unless someone pauses | `FloorModule.sol:538` | Yes | — |

**Counts:** Critical 0, High 0, Medium 2, Low 4, Informational 6.

**What I tried to break and could not:**

- mint a unit without the custodian or the wired module;
- redeem more items than are held;
- have a ticket draw an item minted or refunded after its burn;
- refund a resolved ticket before `REFUND_GRACE` with the mark, or before `LATE_DELIVERY_WINDOW` without it (both clocks);
- re-roll a pinned ticket;
- make the module insolvent across advance / settle / return / burns;
- burn the unit in transit;
- post a floor from any address but the canister;
- post while the registry pauses the keeper;
- move one collection's ETH, NFTs or units into another's books on chain.

The pause floor (A3-M1 / A4-L1), the N-1a burn target and the stale escape behave as before with a `PosterSource` (§7).

---

### A-M-1 (Medium): deposits plus burns pay out a high floor

**Description.** `mintDeposit` mints one whole unit per deposited NFT (`EthNftWrapper.sol:282-302`). The unit is fungible with
every other unit, so its holder can `FloorModule.burn` it against the module's ETH-backed units (`:501-521`; the only bound is
`units ≤ _liveUnits()`). A fresh burn pays `min(price(), _burnTarget()) × (1 − sBurn)`, capped at each tier's backing per unit
β (`:525-541`). Before any fill in the 24 h window, `price()` is the accepted floor itself (`:1196-1200`).

β is **not** an independent bound. It is what the units paid in, and graduations pay `gradPrice() = max(price(), upTarget) × 1.03`
(`:1244-1246`), **the posted floor**. So a floor posted above the market does three things:

- **Graduations overpay.** At +10 % a 4.12 ETH budget buys 3.910 units, not 4.301 (−9.1 %). β rises to 1.1337 × F.
- **Burns pay the high floor**, up to that β. With deposits ON, anyone can buy the NFT at the real floor on Ethereum, deposit it
  (≈ 15–35 minutes to finality), and burn the unit at the posted floor. That turns the overpayment into ETH taken from the module.
- **Advances take `maxItemCost = accepted × 1.03`** (`:1329-1333`). That is 13.3 % over the market per item at +10 %: a seller of
  the runner's choosing gets it, or the canister's Ethereum gas pot does (A-I-2).

**Who can post high, within the module's limits.** The canister clamps its reading at the sales median + 10 %
(`ethnft.rs:533-535`). But the runner chooses which real listings and which real sales to hand in, and sales are only filtered
from below (dust, `:527`). The contract bound is ±5 %/h and ±20 %/day, and D8 pauses only above 20 % off *both* references. So
+10 % is within the canister's own band, and anything up to about +19.9 % is unguarded on chain.

**It also works at an honest floor.** The floor is the **rank-2** listing (D7). An NFT bought at the rank-1 listing, 2 % under
it, deposited and burned before any conversion fill enters the window, profits 0.052 ETH on 3 items
(`test_A_M1d_…`). A fill closes that gap only gradually, because `price()` is a 24 h time-weighted average:

- 3 h after a fill the gap is still open;
- by 23 h after it the gap is closed (`test_A_M1e_…`).

**Exploit (stage 0, Milady F = 0.93 ETH).**

1. The runner withholds the cheap listings. The canister posts 1.1 F; the module accepts it within 2 h (5 %/h).
2. A 4.12 ETH graduation fills at 1.0537 ETH per unit.
3. Mallory buys 3 Miladys on OpenSea at F (2.79 ETH), deposits them (the wrapper's whole day of H11), and burns 3 units:
   **3.062 ETH**.

**Profit: 0.272 ETH per day** (PoC). The profit grows with the inflation and with the graduation price:

| Case | Profit per day |
| --- | --- |
| +10 %, graduation during the inflation | 0.272 ETH |
| +10 %, graduation before the inflation (β = 1.03 F caps the burn) | 0.084 ETH |
| +19 % (just under D8) | 0.523 ETH |
| Honest floor, NFT bought at the posted floor (control) | a loss (−0.004 ETH) |

The day is bounded by the wrapper's H11 (3 mints, shared with purchases, A-L-1), the canister's custody room (3 items per
collection) and `burnPerHour` (5). The effect scales with F. At +10 %, Pudgy (3.353) would give ≈ 0.98 ETH/day and BAYC (6.79)
≈ 1.99 ETH/day.

Who loses:

- the curves that graduated at the high `gradPrice` (their pools received fewer wX);
- the module's revenue, which would otherwise keep `β − paid`;
- and, on advances, the module's ETH above the market price.

No holder's β falls (M4 holds), which is why the module's books look fine throughout.

**PoC.** `poc/A/AuditNftA.t.sol` `AuditNftA_M1_DepositBurn`, all **PASS**:

- `test_A_M1a_…` (+0.2719 ETH);
- `test_A_M1b_…` (+0.0837);
- `test_A_M1c_…` (+0.5235);
- `test_A_M1d_…` (+0.0516 at an honest rank-2 floor);
- `test_A_M1e_…` (the fill window);
- the control `test_A_M1_control_…` (a loss).

**Recommended fix.**

1. **Make a floor rise pay out only once it has stood.** On the module side, cap the fresh burn price by the **lowest accepted
   floor over the trailing 24 h** (a two-bucket minimum of `accepted`, kept like `_mintLow`). A floor walked up inside the day is
   then not burnable into until it has held for a day, by which time the monitor and D8 have seen it. Burns at a falling floor
   are unchanged. A wrapper-side alternative: a deposit-minted unit may not be burned at the module for a `DEPOSIT_COOLDOWN` of
   at least 24 h (both clocks). That needs a per-holder lock (fixes-R3 §4 option 2) and does not cover pool-bought units.
2. Or **keep deposits OFF** until (1) exists. The design recommended OFF (§2.4); D5 turned it on.
3. Lower D8's threshold for the upward direction, or compare with the **sales median** the canister itself verified rather than
   OpenSea/CoinGecko alone.
4. Correct `phase1-nft.md:507` and the Trust page: a high floor is paid out, bounded per day by the numbers above.

---

### A-M-2 (Medium): the relist ticket is a free option that closes redemption

**Description.** `EthNftWrapper.redeemToRelist()` (`:320-323`) burns one unit for a ticket whose destination is the custodian.
The canister never delivers such a ticket: `judge_transfer` refuses it with "a relist ticket: never delivered", `ethnft.rs:688`,
and the as-built notes say it is marked undeliverable at once. So it refunds at `deliverBy + REFUND_GRACE`, about 10 days after
the burn (`:407-430`). Without the mark it refunds after 37 days.

While the ticket is open its NFT is promised (`held ≥ openTickets + 1`, `:326`), so every other `redeem` reverts `NoItem`. The
refund mints the unit back, and it can be re-burned in the same transaction. Holding `held − openTickets` units (3 at stage 0) is
therefore enough to keep every NFT in custody promised to one's own tickets, indefinitely:

- the only cost is gas and the capital's price exposure;
- `refund` is anyone's, so an honest redeemer can race the refund-and-reburn at each unlock, but only against a bot that knows the
  exact moment.

YOYO is OFF and the canister never relists an NFT, so the function serves no Phase-1 purpose. It exists for `IWrappedCollection`.

**Impact.** Item-backed holders' only exit besides the pools is closed. Burns reach only ETH-backed units. SweepVault BURN's
`retire` (`redeemToBurn`) also reverts `NoItem`. This contradicts "redemption is never pausable" (phase1-nft §7). The same
function exists in `Wrapper.sol:468-471` (frogs, now Phase 2); B-13 listed it as Info while the frog canister refunded it after 7
days.

**PoC.** `AuditNftA_M2_RelistFreeze`, both **PASS**:

- `test_A_M2_…`: 3 units, three 10-day cycles; Bob is refused `NoItem` in every cycle, and Mallory ends with her 3 units.
- `test_A_M2b_…`: no mark, a 37-day cycle.

**Recommended fix.** Make `EthNftWrapper.redeemToRelist` revert (`NotSupported`). The NFT collection's YOYO vault is OFF, and its
`relist` would then revert too, which is correct. If a relist exit is ever wanted, the custodian should have to list, and the
ticket should not be refundable to its burner.

---

### A-L-1 (Low): one H11 pool for purchases and deposits

The wrapper's `_countMint` (`:222-225`) is one rolling window of `maxMintsPerDay` (3 at stage 0) for `mint` **and**
`mintDeposit`. The module counts its own H11 at `advance` (`:703`), so an advance can be open while the wrapper's room is gone.
The canister skips deposit mints only while its buy is `Buying`…`Minting` (`ethflow.rs:1371-1374`). Deposits final during
`NeedAdvance`…`Arrived` (Relay, waiting for a listing) are minted first. The purchase's `mint` then reverts `MintRateLimited`, and
the advance stays open:

- no second advance (`AdvanceOpen`);
- the transit unit stays out of every burn;
- the wait is until the oldest of the day's mints is 24 h old.

PoC `test_A_L1_…` **PASS**: still open after 23 h, settled after 24 h. A depositor that lands 3 deposits a day around each buy
roughly halves the collection's conversions, at the cost of the spreads. No loss.

**Fix.** Reserve the purchase's mint when its advance is taken. In the canister: do not mint deposits while any buy of that
collection is past `NeedAdvance`. Or on chain: `mintDeposit` refuses if `mintRoom` would leave no room while
`module.transitUnits() != 0`.

---

### A-L-2 (Low): the deploy key keeps the NFT wrapper's mint limit

`DeployStack` creates the `EthNftWrapper` from the deploy EOA (`:166-167`) and calls `wireFloorModule` / `initMintLimit` itself
(`:259-260`), so `deployer` (`EthNftWrapper.sol:174`) is that EOA for good. Afterwards it alone can:

- lower `maxMintsPerDay` at once (1 = one item a day for purchases and deposits together);
- propose a raise that anyone accepts after 7 days (`:189-211`).

For Bitcoin collections this power was moved to the Safe through `WrapClaim` (deploy-stack.md §8.2). The NFT path is the one
still open, and it is now ON. The deploy log prints "The deployer owns nothing" (`:202`), and golive-check reads neither
`deployer()` nor `maxMintsPerDay()` for NFT wrappers (it only fails a pending raise). The stage-1 raise (6 a day) needs the deploy
key to still exist.

PoC `test_A_L2_…` **PASS**.

**Fix.** Deploy the wrapper through a Safe-owned passthrough (as `WrapClaim`), or take the deployer from an argument (the Safe).
Have golive-check require `deployer() == <safe or passthrough>` and `maxMintsPerDay() == stage`.

---

### A-L-3 (Low): one pause for every collection

`PosterSource.keeper()` returns `registry.keeperPaused() ? 0 : poster` (`:26-28`). The module's `_keeperDown()` (`:1142-1144`)
reads `keeperPaused()` through it. D8's guardian pause therefore zeroes:

- **every** NFT module's poster;
- the registry's keeper EOA (caps, draws, `requestDraw`).

On every module, at once:

- mints, advances and conversions stop (`maxItemCost` 0);
- closing buys revert (`hasRoomFor` false, `:864-866`);
- refunds are held (`:876-877`);
- the canister cannot post a *good* floor elsewhere.

Only the Safe (2-of-2) unpauses. No per-collection floor switch exists:

- `PosterSource` has no setter;
- the listing pause gates only the wrapper's mints, and the default listing is shared (A-I-5);
- the module ignores it (B-09).

A module cannot be moved by another collection's floor (each has its own `PosterSource`, bounds and anchor). The coupling is only
the switch, but with three collections D8's false-positive surface (a genuine 20 % crash that the 5 %/h move limit lags behind)
is tripled, and each trip halts all three. Burns continue at each module's pause floor, and there is no loss.

PoC `test_A_L3_…` **PASS**: Azuki's module stops minting and closing when Milady's floor is paused; the listing pause does not
stop a module.

**Fix.** A per-collection pause in `PosterSource`:

- `pause()` by the registry's owner or guardian;
- `unpause()` by the owner;
- `keeperPaused() = registry.keeperPaused() || paused`;
- its own `keeperResumedAt` / `keeperResumedAtBlock` (the max of its own and the registry's), so A3-L1's grace holds per
  collection.

D8 then pauses only the collection that tripped it. Keep the registry pause for a canister-wide incident.

---

### A-L-4 (Low): DeployStack cannot build the multi-collection NFT stack

`run()` reads one `ETCH_NFT_COLLECTION` / `ETCH_NFT_CHAIN_ID` (`:89-91`) and deploys a new `ProtocolTreasury` and
`CollectionDirectory` every run (`:129-131`). On 4663 it refuses to run while `deployments/stack-4663.json` exists
(`:113`; `deploy-stack.sh:34`).

Three collections therefore mean three runs, with the file moved aside by hand each time:

- three treasuries and three directories;
- three stack files, of which the services, golive-check and the canister's `forCanister` list read one;
- the `ethcheck` cross-check of the canister's `eth_nft_config` against the stack is only a note without `--canister`.

Each run's read-backs hold per collection. What nothing checks is the set: for example, that the three collections' modules all
use distinct `PosterSource`s over the one registry, or that no collection address is repeated.

**Fix.** Accept a list of NFT collections in one run (one treasury, one directory, one file). In `--phase1`, make `ethcheck` a
FAIL without `--canister`, and require every configured collection to be in the stack file and vice versa.

---

### Informational

- **A-I-1. The poster is immutable** (`PosterSource.sol:14-23`). The 2026-09-26 root fix (E-02 / C-11 / B-04: the keeper read
  from the registry, rotatable in 7 public days) does not reach NFT modules. A compromised or replaced canister can only be paused,
  after which the module's exits are burns at the pause floor and, after 7 days without a good post, at β. Documented
  (phase1-nft §3.2), and the canister already holds the NFTs, so this adds little. It should be on the Trust page's key table.
- **A-I-2. An open advance has no timeout** (`:684-712`). Only the custodian can settle or return it. A stranded advance (D4:
  ETH on Ethereum, no listing within the bound) blocks every later conversion of that module (`AdvanceOpen`). After the stale
  escape, the transit unit's wX is the one unit nobody can burn (its backing left). `advanced − cost` is never returned
  (`settleAdvance` moves no ETH, `:715-731`). It accumulates at the canister's Ethereum address: about 1–3 % of each item at an
  honest floor (design §5), and 13.3 % at a +10 % floor (A-M-1). State it on the reserves page. Consider bounding the advance by
  the canister's own buy bound rather than `maxItemCost`.
- **A-I-3. B-06 in `settleAdvance`** (`:720`): the check is the module's absolute wX balance. Any wX sent to the module satisfies
  `NoWrapped` without the purchase's mint. That includes a deposit whose claim names the module as recipient: the canister refuses
  only zero, the wrapper and itself (`ethnft.rs:748`). Custodian-only and self-correcting (the later purchase mint becomes a stray
  for `reclaimStray`). Refuse the module (and every module) as a deposit recipient.
- **A-I-4. Adverse selection.** Redemption draws uniformly (`:356`), and deposits bring the NFTs their owners value least
  (sub-floor, or flagged on OpenSea and so unsellable there). The set's value drifts below the posted floor, and deposit+burn sells
  such items to the module at the floor (A-M-1). `Item.deposit` (`:85`) is never read. Document it, and consider a deposit fee or
  keeping deposit items out of burns' reach.
- **A-I-5. The listing pause is shared by default** (`DeployStack.s.sol:165`: `ETCH_NFT_LISTING` defaults to the stack's
  listing). Every NFT wrapper then reads the same `isPaused` flag, so the only other switch is also cross-collection. B-09 still
  holds: no `FloorModule` reads a listing pause.
- **A-I-6. A3-I1, more likely now.** The canister posts only with ≥ 3 verified listings and ≥ 3 sales (`ethnft.rs:521-529`), and
  the runner holds the OpenSea key. A runner outage makes the module stale, and burns then revert `Stale` (`:538`) until someone
  pauses or 7 days pass. Have the monitor pause on a sustained no-good-post for NFT modules too (R4 §4 recommended it for the frog
  keeper).

## 5. The canister as floor poster: what the contract bounds (quantified)

The poster can move `accepted` at most 5 % an hour (`floorBounds`, `:419-435`) and 20 % from the day's anchor (`:424-427`). It is
zeroed at once by the registry pause. Within those limits, with stage-0 parameters and Milady F = 0.93 ETH:

| Lever | Contract bound | One day at +10 % | One day at +19 % (under D8) | One day at −15 % / −20 % |
| --- | --- | --- | --- | --- |
| Deposits + burns (A-M-1) | `burnPerHour` 5; wrapper H11 3 a day (shared); β = what units paid | **0.272 ETH** (PoC) | **0.523 ETH** (PoC) | no gain (burns pay less) |
| Advances (`maxItemCost = accepted × 1.03`) | 3 a day (`maxItemsPerDay`), one at a time | ≤ 3 × 0.124 = **0.371 ETH** above market leaves the module | ≤ 0.63 ETH | none |
| Graduations (`gradPrice`) | per graduation | 9.1 % fewer units: ≈ 0.36 ETH of value per 4.12 ETH graduation | ≈ 16 % fewer | more units per ETH: the curve gains, the module's β is lower |
| Public mints | `publicCap` 10 (and `depthItems`, the poster's), 5 an hour | none (dearer to mint) | none | ≈ 10 × F × 20 % = **1.86 ETH/day** through cheap mints (the documented keeper bound, `FloorModule.sol:49-51`) |
| Redemption with cheap wX | `held − openTickets` (≤ 3) | — | — | 3 × 0.2 F ≈ 0.56 ETH, within the line above |

- Advances, deposits and purchases compete for the same 3 wrapper mints and 3 custody slots per collection a day. The upward day
  is therefore about **0.37 ETH (+10 %) to 0.63 ETH (+19 %)** per collection, not the sum.
- The downward day is the documented `maxPublicEthBacked × F × 20 %`.
- Per collection, scale by F (Azuki 0.71×, MAYC 0.93×; later Pudgy 3.6×, BAYC 7.3×).
- The pause bounds everything above from the moment of the pause: no mint, advance or conversion; burns at the pause floor.

## 6. Cross-collection isolation (operator update, 2026-09-27)

**(a) Per-collection accounting on 4663: holds.**

- Each wrapper has one module, wired once by B-05 (`wireFloorModule` requires `module.wrapped() == this`, `:235`). Each module
  has one immutable wrapper.
- `burn` and `release` move only that wrapper's token.
- `settleAdvance` / `itemArrived` count only that module's own wX balance.
- `enqueue` / `release` / `cancel` are gated by that module's factory (`routerOf`), and DeployStack reads back
  `factory.queue() == module`, `module.wrapped() == wrapper`, `lock.wrapped()` and `router`.
- Deposits name their wrapper in the claim data, and the canister refuses another wrapper's (`ethnft.rs:744-746`).
- No contract lets one collection's ETH, units or NFTs back or redeem another's.

What **is** shared rests on the canister's word:

- One custodian and payee address for every collection. A canister configuration that swaps (collection, wrapper, module) would
  mint one collection's NFT into another's wrapper. `EthNftWrapper.mint` takes only a token id: the collection is an immutable of
  the wrapper, not checked per mint. That mapping is checked only by golive-check's `ethcheck`, and only with `--canister`
  (A-L-4). I did not find a runtime read of `wrapper.collection()` in `ethflow.rs` / `ethnft.rs`: the `COLLECTION` selector at
  `ethnft.rs:603` is defined but not referenced. **For reviewer B to confirm.**
- One Ethereum address holds every collection's NFTs and one ETH pot: the gas pot and every advance's leftover, across collections.
- `returnAdvance{value}` accepts any ETH, so the canister alone decides which module gets which returned ETH.

**(b) Floor posting and pause.**

- Posting is per module: each has its own `PosterSource`, and bounds and anchor per module. A bad floor for one collection cannot
  move another module's `accepted`.
- The pause is **registry-wide**: one D8 trip halts every collection and the keeper EOA (**A-L-3**, PoC).
- There is no per-collection floor pause. The listing pause gates wrapper mints only, and by default the listing is shared
  (A-I-5).
- My view: halting all on one bad floor is acceptable as the *canister-wide* incident switch, but not as D8's *routine* floor
  guard across three (later five) collections. Hence Low with the per-`PosterSource` pause fix.

**(c) Anything else shared.**

| Shared thing | Effect |
| --- | --- |
| **ProtocolTreasury / CollectionDirectory** | Not shared across DeployStack runs (A-L-4: one each per run). If they are put in one treasury by hand, B-11 (`volume7d` churn shifts `distribute`'s weights) becomes live, and 90 % of income is allocated by it |
| **H11** | Per wrapper (3 a day each) and per module (3 a day each), so 9 a day in all for three collections. The canister's custody caps are per collection (3 items, 10 ETH) **and global** (6 items, 10 ETH, phase1-nft §5). So a deposit in one collection can take room another collection's purchase needed (A-L-1, across collections; the canister's side) |
| **Guardian, owner, keeper EOA** | One set for all collections |
| **Cap ceiling** | `ETCH_CAP_CEILING` is one wei value per DeployStack run, and SweepVault caps are in wei per item, so each collection's run needs its own. Sweeps are OFF for NFT collections in Phase 1 |
| **Beacon** | drand, shared and harmless |

## 7. Regression check of the 2026-09-26 audit's fixed contract findings

| Finding (fix) | Where this branch touches it | Status at `b48b3ba` |
| --- | --- | --- |
| **C-02** (no redraw; the pinned round is final) | `EthNftWrapper` has no `redraw`; `resolve` `:342-368` | **Holds** |
| **E-03** (pinned refund after `PIN_TIMEOUT`, both clocks; `_skipRefunded`) | `:409-419`, `:370-375` | **Holds** (identical to `Wrapper.sol:589-599, 540-545`) |
| **D M-1** (`REFUND_GRACE` 72 h past `deliverBy`, both clocks) | `:420-422` | **Holds** |
| **Canister N-2 (review2)** (`markUndeliverable`, custodian-only, Resolved only; else `LATE_DELIVERY_WINDOW` 30 d) | `:397-402`, `:423-424` | **Holds** |
| **B-03 / C-05** (`recordDelivery` custodian-only) | `:379-392` | **Holds** |
| **B-05** (module must name this wrapper) | `:231-239` | **Holds** |
| **H11** (mint rate; init once before any mint; lower at once, raise in 7 d) | `:180-225` | **Holds** (the deployer is the deploy EOA: A-L-2; the pool is shared: A-L-1) |
| **W1 / W3** (ledger, unpromised item) | `:287-338`, no `openReturns` (no rentals) | **Holds** (fuzz) |
| **B-01** (burns span tiers) | `burn` / `_release` now divide by `_liveUnits()` | **Holds** (`C + R + T = E`; tier prices exclude the transit unit) |
| **B-02** (surplus apart; reserve burns ≤ basis; day band) | `advance` draws a shortfall from revenue, then surplus, as `itemArrived` does | **Holds** as designed. The day band is what bounds A-M-1 |
| **B-04** (stale escape at β) | `_tierPrices` via `_liveUnits` | **Holds** (the transit unit is outside it: A-I-2) |
| **B-06** (absolute balance) | Not fixed; carried into `settleAdvance` | **Not fixed** (A-I-3) |
| **B-07** (`_gradRoom` counts queued budgets) | Uses `_w.ethBacked()` (transit included, conservative) | **Holds** |
| **B-08** (burn race before `itemArrived`) | The advance takes the unit into transit *before* the item exists | **Improved**: no burn can take `C` below one unit between the wrap and the settle |
| **B-09 / N-1b → A3-M1 / A4-L1** (pause zeroes the keeper; burns at the pause floor) | `_keeperDown` reads `PosterSource.keeperPaused()` → the registry's | **Holds** (the pause floor is unchanged; per collection impossible: A-L-3) |
| **N-1a** (bounded burn target) | Unchanged | **Holds** |
| **N-3 / A3-L1** (close gate and held refunds while paused; `RESUME_GRACE` from `keeperResumedAt`) | `PosterSource` forwards `keeperResumedAt` / `Block` | **Holds** |
| **N-7** (item cap 0 and snapshots ignored while paused) | `advance` is bounded by `maxItemCost` → `_capOrSnapshot` returns 0 while paused | **Holds** (advances refused while paused; `settleAdvance` / `returnAdvance` still allowed, correctly) |
| **E-02 / C-11** (keeper read from the registry, rotatable) | NFT modules read an immutable poster | **Not applicable by design** (A-I-1) |
| **C-01** (cap ceiling active at deploy) | NFT-only `_ownerSteps` proposes ×3 and reads back raffle and burn | **Holds** |
| **C-07 / C-12** (treasury defaults, `try fundReserve`) | Unchanged | **Holds** |
| **B-12** (`snapshotCap` anyone's, 2 h) | Now also extends `advance` | Unchanged (Info) |

## 8. `EthNftWrapper` vs `Wrapper`, line by line: what changed, and whether it matters

| Area | Change | Verdict |
| --- | --- | --- |
| Identity | Taproot custody string → `chainId` + `collection` + `custodian`, where the custodian is also the NFT holder | OK. The constructor refuses this chain, a coded custodian and zeros (`:162-166`) |
| Mint | `wrap` (Distributor, OFF) / `wrapDeposit` (membership proof, outpoint) → `mint` / `mintDeposit`, keyed by `(tokenId, tx)`, no membership proof (any token id of the collection) | OK: the canister judges the collection on Ethereum. The mapping to this wrapper is its word (§6a) |
| Item ids | `acquisitionId` / `DEPOSIT_FLAG` → a dense `_items` array | OK. Redelivered-then-redeposited tokens get a new item id; `itemOf` is cleared at delivery (`:387`) |
| Redeem | Taproot check → `destination ≠ 0, ≠ custodian`; `redeemToBurn` → `DEAD`; `redeemToRelist` → custodian | `redeemToRelist`: **A-M-2** |
| Rentals | Removed (`NoRentals`, `openReturns` / `rentedHeld` 0, `rentalState` None) | OK: the module's `rentIn` reverts inside `admitRental`; rentals are OFF |
| Pause | The listing pause gates `mint` / `mintDeposit` (as `wrapDeposit`) | OK. Shared listing: A-I-5 |
| Deployer | The claim contract (Safe) → the deploy EOA | **A-L-2** |
| Events | `Resolved` gains the token id; `Delivered` gains the destination | OK (indexer and resolver updated per §11.3; not verified here) |

## 9. Tests run

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` at `b48b3ba` (baseline) | **1,088 passed, 0 failed** (124 suites) |
| The same, with `test/audit_nft_a/` added | **1,101 passed, 0 failed** (130 suites) |
| `forge test --match-path "test/audit_nft_a/*"` | **13 passed**, table below |
| Fork proof, Slither | Not run (§2) |

Each test is in `poc/A/AuditNftA.t.sol`, with its base in `poc/A/AuditNftABase.sol` (stage-0 params, a stand-in factory for
graduations). "PASS" means the attack works, except where the Finding column says "holds".

| Test | Finding | Result |
| --- | --- | --- |
| `test_A_M1a_graduationDuringTheInflation_depositsAndBurnsTakeTheGap` | A-M-1 | PASS: +0.2719 ETH / 3 items; `maxItemCost` 1.0537 vs F 0.93 |
| `test_A_M1b_graduationAtTheTrueFloor_theBetaCapLimitsTheGap` | A-M-1 | PASS: +0.0837 ETH |
| `test_A_M1c_atTheGuardiansThreshold_plus19pct` | A-M-1 | PASS: +0.5235 ETH |
| `test_A_M1d_honestRank2Floor_rank1ListingTwoPctUnder_stillProfits` | A-M-1 | PASS: +0.0516 ETH at an honest floor |
| `test_A_M1e_aFillClosesTheHonestGapOnlyOverItsWindow` | A-M-1 | PASS: the gap is open 3 h after a fill, closed at 23 h |
| `test_A_M1_control_anHonestFloorLoses` | A-M-1 control | PASS: a loss (2.7858 vs 2.79) |
| `test_A_M2_threeUnitsFreezeEveryRedemptionIndefinitely_atNoCost` | A-M-2 | PASS: 3 cycles, 30 days, 0 cost |
| `test_A_M2b_withoutTheMarkTheCycleIsLonger_stillFree` | A-M-2 | PASS |
| `test_A_L1_threeDepositsHoldAnOpenAdvanceForADay` | A-L-1 | PASS |
| `test_A_L2_theDeployKeyCutsMintsAfterLaunch` | A-L-2 | PASS |
| `test_A_L3_oneCollectionsPauseHaltsAnotherCollection` | A-L-3 | PASS |
| `testFuzz_A_ledgerHoldsWithDepositsRelistAndAdvances` (512 runs; also 2,000) | W1, W3, W4, M2 | holds |
| `test_A_ledgerHolds_theRunReachesEveryAction` (non-vacuity, 20 seeds) | — | every action taken: deposits 84, advances 49, settles 37, returns 11, mints 106, burns 50, relists 9, resolves 8, delivers/marks 9, refunds 15 |

## 10. Test gaps

1. **The NFT suites run the wrong configuration again** (B-floor §6.1's gap, repeated). `test/ethnft/EthNftBase.sol` uses
   `maxItemsPerDay = 40` and `initMintLimit(1_000)`, and reserve 0 in the unit tests. The deployed stage 0 is 3, 3 and 25 %. The
   shared H11 pool (A-L-1) and the stage-0 day bounds are therefore never exercised.
2. **The invariant handler has no `mintDeposit`, `redeemToRelist`, `redeemToBurn` or pinned refund**
   (`test/ethnft/EthNft.invariant.t.sol`), although deposits are ON at launch. My fuzz adds them.
3. **No test posts a high floor and then burns** a deposit-minted unit (A-M-1). No test checks a burn against the market, only
   against β.
4. **No test of a second collection on the same registry** (A-L-3), and no DeployStack test of more than one NFT collection
   (A-L-4).
5. **No golive-check assertion on `EthNftWrapper.deployer()` / `maxMintsPerDay()`** (A-L-2).

## 11. What I did not cover

- The canister's Rust beyond the cross-reads in §2 pass 5: `relayeth`, the buy judge, finality, the cancel path and
  `markUndeliverable`'s conditions are reviewer B's.
- The runner, monitor (the D8 implementation), indexer, site and keeper.
- The fork proof (needs an Ethereum archive fork).
- Slither.
- `FrogCurve`, `LiquidityLock`, `EtchRouter` and the sweep vaults beyond their calls into the module.
- The deep invariant profile.
- The `nft-pool` branch (Pudgy, BAYC and pooled graduations): out of scope. Its thin books (BAYC: 5 listings from 3 sellers
  within 5 %; Pudgy: 7 from 4) make the rank-2 rule and A-M-1's per-unit gap larger, and the bounds in §5 scale by F.
