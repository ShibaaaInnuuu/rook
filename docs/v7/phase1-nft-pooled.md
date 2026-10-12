# v7 Phase 1, addendum: per-collection graduation targets, pooled graduations, several collections at once (design)

Status: **design, written before its code**, 2026-09-27, branch `nft-pool` (from `nft-side`). It amends `docs/v7/phase1-nft.md`.
Nothing is deployed. The only calls made were read-only.

**Legend.** **[M]** measured (the floors are the operator's OpenSea reads of 2026-09-27 ~16:05 UTC). **[R]** read from this
repository. **[A]** assumption.

**The operator's asks (2026-09-27):**

1. "lets do the 9 eth for it [BAYC]": a per-collection graduation target, bounded, set at registration, and visible to
   golive-check, the monitor and the site.
2. "pooled graduations for initial nft tokens pool launch somehow": graduations whose budget is under one item pool until one
   item can be bought.
3. The pilot launches **five collections at once**: Milady, Azuki and MAYC at the default targets, plus Pudgy Penguins and BAYC.
   Pools and targets are strictly per collection, and the one Ethereum custody and gas address serves every collection's buys and
   deliveries without one blocking the others.

**Decisions already taken:** D1–D4 and D6–D9 as recommended. D5: holder deposits **on** at launch.

**J1 decided (2026-09-27):** BAYC graduates at **10 ETH** (normal = early, the `CurveFactory` ceiling) with `convBps` **1,000** and
`reserveBps` **0**: one graduation buys one ape, with **+8.26 %** of floor-rise headroom from the 6.79 floor (§1.1). D4 (the
automatic return of ETH stuck on Ethereum after 48 h, as built in round 2) confirmed the same day (`phase1-nft.md` §9).

---

## 0. Summary

- **Pooling is how the Floor Module already works [R], and it is the design.**
  - A graduation converts its whole item budget into **ETH-backed wX** at the module (`enqueue` → `_tryFill`). Each unit is backed
    by the ETH it paid (`backing`, plus the standing `reserve`).
  - The canister buys **one** item whenever the module's net convertible units reach one item plus the 5 % buffer
    (`floorsink::wanted`). A budget under one item is simply units that wait. The next graduations of the **same collection**
    add units at the **same module**, and the item is bought when the sum covers it.
  - Relay moves ETH **once per item** (`advance`, one open at a time per module), never once per graduation.
  - Every wX unit is 1:1 backed at all times, by ETH at its module or by an NFT in custody. That is the wrapper's ledger
    `totalSupply == (held − openTickets) × ONE + ethBacked` and the module's `C + R + T = E`. Both are asserted by the invariant
    suites. **No new contract, no new state and no new trust is needed for pooling.**
- **The graduation target is already per collection [R].** Every collection has its own `CurveFactory`, built with its own
  `Targets` (normal, early, max). The change is to make them **deployment parameters per collection**:
  - bounded in `StackChecks`;
  - written to the stack file;
  - read back by golive-check;
  - shown by the monitor and the site.

  The Floor Module's `convBps` and `reserveBps` become per-collection parameters the same way (§2). Other collections keep the
  defaults.
- **What changes in the canister is concurrency, not pooling.** `nft-side` keeps **one Ethereum buy open canister-wide**, so a
  BAYC buy waiting for a listing (for example, the floor ran up) would stop Milady's buys. Now:
  - one buy per collection;
  - **one Relay leg at a time canister-wide** (arrival and refund detection are balance-based);
  - any number of collections' buys past arrival;
  - each buy's spend bounded by its **own** arrived ETH less every other buy's earmark;
  - a line transaction stuck unmined is **re-priced at the same nonce** (every version is watched), so one stuck transaction cannot
    hold the address for ever.

  Deliveries go first; buys are round-robin across collections.

## 1. The numbers [M, R]

**The terms, all [R]:**
- **Item budget** = raise − min(5 %, 0.25 ETH) − 12.6 % (the shared pool's ETH side), from `CurveFactory.itemBudgetFor`.
- **Graduation price** = floor × (1 + `convBps`) (`FloorModule.gradPrice`). Default `convBps` = 300.
- **Reserve** = `reserveBps` = 2,500 of every intake, never converted.
- **A buy is due** at net convertible units ≥ 1.05.

So **items per graduation** = 0.75 × budget ÷ (1.03 × floor).

**Headroom** is the most the floor may rise between a graduation and the buy while that graduation's units can still pay the new
floor. It is ≈ `convBps` less Relay's 0.36 % and the gas margins. *(This table predates the round-2 advance gate and J1's decision:
its headroom column is superseded by §1.1, which works each step out as built.)*

| Collection (floor) | Target (early / normal) | Items per graduation (early / normal) | Graduations per item | Headroom |
|---|---|---|---|---|
| Milady (0.930) | 8 / 5 | 5.28 / 3.23 | < 1 | 2.3 % |
| Azuki (0.670) | 8 / 5 | 7.33 / 4.48 | < 1 | 2.2 % |
| MAYC (0.8735) | 8 / 5 | 5.62 / 3.43 | < 1 | 2.3 % |
| **Pudgy (3.353)** | 8 / 5 | **1.46 / 0.89** | early: 1; **normal: 2** (0.89 + 0.89 = 1.79 → one bought, 0.74 carried) | 2.5 % |
| **BAYC (6.79), operator's 9 ETH, defaults** | 9 / 9 (opening 9.26) | **0.82** | **2** (1.63 → one ape, 0.63 carried) | 2.6 % |
| BAYC, 10 ETH, defaults | 10 / 10 | 0.91 | 2 | 2.6 % |
| BAYC, 9 ETH, `convBps` 1,100, `reserveBps` 0 | 9 / 9 | 1.01 (under the 1.05 buffer) | 2 | 10.6 % |
| **BAYC, 10 ETH, `convBps` 1,100, `reserveBps` 0** | 10 / 10 | **1.13** | **1** | **10.6 %** |

**What the table says:**

- **9 ETH does not buy one ape per graduation**, in any configuration where the 5 % buffer holds. The reason is the standing
  reserve: a quarter of every graduation never buys an item.
  - At 9 ETH, BAYC pools two graduations per ape.
  - The raise cannot go past **10 ETH** without a contract change: `CurveFactory` refuses `earlyRaise > HARD_MAX_EARLY_RAISE`
    (10 ETH), and `normalRaise ≤ earlyRaise`.
- **A larger raise gives no floor-rise headroom.**
  - One item is bought from one unit's backing (`maxItemCost` ≤ backing per unit + revenue + surplus).
  - A unit's backing is fixed at its graduation price, so the headroom is `convBps`, whatever the raise.
  - Surviving a +10 % rise needs `convBps` ≥ 1,045. With the canister's +5 % listing premium on top, it needs ≈ 1,600.
- **The configuration that meets the operator's goal** — one graduation, one ape, still bought after a +10 % floor rise — is
  **BAYC at 10 ETH (normal = early), `convBps` 1,100, `reserveBps` 0**. This is a judgment call for the operator (J1), because it
  changes three things:
  - BAYC coin holders pay 11 % over the floor per wX at graduation. *(Corrected in fix round 2 of the 2026-09-27 review, A2-L-1:
    this said the part not spent on the ape "becomes the module's revenue, as every conversion's leftover does". On the advance
    path it does not: `advance` pays the whole item cap out, `settleAdvance(cost)` moves no ETH, and what the ape did not cost
    stays at the canister's Ethereum address as its gas pot, ≈ 0.41–0.75 ETH per ape at `convBps` 1,100, never returned. At the
    committed defaults (300) that leftover is the documented 1–3 %. Before the alternative is taken, the canister would advance
    only its buy bound, or return the leftover over the gas pot's target; neither is built.)*
  - BAYC keeps no standing ETH reserve for exits. Burns still pay at most each unit's backing.
  - The raise is the contract's ceiling.
- **Committed in this branch's launch config (until J1 was decided):** the operator's **9 ETH** with the defaults. That means
  pooling, two graduations per ape, and 2.6 % headroom. The 10 / 1,100 / 0 set was written beside it as the recommended
  alternative. **Superseded 2026-09-27:** the operator took J1 at 10 ETH; the conversion and reserve were then worked out from the
  as-built item-budget math as `convBps` 1,000 and `reserveBps` 0 (§1.1), and `deployments/launch/nft-pilot.json` carries them.
- **Stage 0** (3 items and 10 ETH a collection in custody) holds one BAYC ape and two Pudgys. Past that, the units stay ETH-backed
  and burnable until stage 1.
- **A pre-existing flaw, flagged for review (not changed here):** the factory's *opening* target is meant to let "a lone first
  coin buy one item on its own". It is sized as cap × 1.05 and ignores the module's reserve share. At the default 25 % reserve the
  opening coin converts 0.84 of an item (BAYC) or less, so the first item is always pooled with the next graduation. Pooling makes
  this harmless. Correcting the opening formula (dividing by 1 − `reserveBps`) is a `CurveFactory` change, left to the review.

### 1.1 J1 as decided (2026-09-27): BAYC at 10 ETH, and every collection's margin [R, M]

The operator's decisions: BAYC graduates at **10 ETH** (going above needs a `CurveFactory` change), and "find a percentage above
to use": the floor-headroom margin, from the real item-budget math, that lets **one** graduation buy **one** ape.

**What one graduation's ETH goes through, step by step [R].** A raise R, the floor F at the graduation, the floor F′ when the
canister buys:

1. **Item budget** B = R − min(5 % × R, 0.25 ETH) − 12.6 % × R (`CurveFactory.itemBudgetFor`): 10 ETH → **8.49**; 8 → 6.742;
   5 → 4.12.
2. **The module's share.** `reserveBps` of B and of the units stays as the standing reserve and is never converted. The rest
   becomes c = (1 − reserve) × B ÷ (F × (1 + conv)) convertible units (`gradPrice`), each backed by F × (1 + conv).
3. **A buy is due** at c ≥ 1.05 (`floorsink::wanted`, the 500 bps buffer). So a graduation buys an item alone while its floor is at
   most the **pool floor** F_pool = (1 − reserve) × B ÷ (1.05 × (1 + conv)). Above it, the collection pools.
4. **The advance** is the item cap: min(F′ × (1 + conv), the backing per unit + revenue + surplus, `max_purchase_wei`)
   (`maxItemCost`, `floorsink::view`). After a rise, the backing per unit, F × (1 + conv), is what binds.
5. **The canister's gate** (`ethflow::advance_covers`, round 2): cap − 0.0002 ETH (the 4663 gas keep) − 0.5 % × cap (Relay's
   estimated fee) ≥ F′ × 1.01 + 0.003 ETH (the floor + 1 % and the Ethereum gas reserve). So a graduation at F is still advanced
   while F′ ≤ (0.995 × cap − 0.0032) ÷ 1.01: the **post-graduation edge**.
6. **The buy** pays at most min(F′ × 1.05, what arrived − 0.003 ETH, `max_purchase_wei`) (`ethnft::buy_cap`). What arrives is
   cap − 0.0002 less Relay's actual fee (0.36 % measured). Whenever the gate passes, that covers a listing at F′ (and up to about
   F′ × 1.012), so step 5 is the one that binds.

The **headroom** is min(F_pool, the post-graduation edge) ÷ today's floor − 1. It is the most the floor may rise from today, before
the graduation, after it, or split between the two, while one graduation still buys one item.

The numbers assume the module's revenue and surplus are 0 (a new module; the coins' reserve-fee slices, `fundReserve`, only add to
them and so to the headroom), that `price()` is the posted floor (no fills yet; fills inside the 3 % band move `gradPrice` with
them), and that the pool holds graduations only (a public mint is backed by about the floor plus its spread, not the conversion,
so mints into the pool lower the post-graduation edge). `Pooled.t.sol` reproduces both of BAYC's edges on the contracts: the
post-graduation gate passes at 7.3548 and refuses at 7.3550; a graduation at 7.3505 gives 1.05002 units and one at 7.3507 gives
1.04999.

**Advancing only the buy's bound does not change this table** (A2-L-1, §8.1): the gate still judges the pool (the item cap against
the module's floor), and the bound (floor × 1.05 and the fees) binds only while it is under the item cap, where the gate passes anyway.

**Choosing BAYC's conversion and reserve (R = 10 ETH, F = 6.79, within `StackChecks`: conv 300–1,600, reserve ≤ 2,500).** A
reserve lowers F_pool and does nothing for the gate, so it is **0**. The conversion trades the two edges against each other.
F_pool falls as 1 ÷ (1 + conv), and the post-graduation edge rises as ≈ 0.985 × (1 + conv). They meet at conv ≈ 997 (+8.29 %).
**1,000** is within 0.03 points of that and a round 10 %.

| BAYC at 10 ETH | Units per graduation at 6.79 | Pools above (floor at graduation) | Still advanced after a graduation at 6.79, up to | Headroom |
|---|---|---|---|---|
| defaults (conv 300, reserve 2,500) | 0.91 | 5.888 (−13.3 %: pooled today) | 6.887 (+1.42 %) | pooled |
| conv 700, reserve 500 (the best with a 5 % reserve) | 1.11 | 7.179 (+5.73 %) | 7.154 (+5.36 %) | +5.4 % |
| conv 1,000, reserve 500 | 1.08 | 6.983 (+2.84 %) | 7.355 (+8.32 %) | +2.8 % |
| **conv 1,000, reserve 0 (decided)** | **1.137** | **7.3506 (+8.26 %)** | **7.3549 (+8.32 %)** | **+8.26 %** |
| conv 1,100, reserve 0 (the earlier recommendation) | 1.127 | 7.284 (+7.28 %) | 7.422 (+9.30 %) | +7.28 % |
| conv 1,600, reserve 0 (the bound) | 1.078 | 6.970 (+2.66 %) | 7.756 (+14.2 %) | +2.66 % |

With a reserve, the best is: 250 bps → conv 860, +6.9 %; 500 → 720, +5.5 %; 1,000 → 435, +2.7 %. At 9 ETH, conv 1,000 and
reserve 0 give 1.02 units: pooled today.

**BAYC as decided:**

- **One graduation, one ape.** 1.137 units at 6.79. 0.137 of them stay ETH-backed for the next graduation.
- **The floor at which BAYC starts needing to pool: 7.3506 ETH** at the graduation (+8.26 %). Above it, two graduations per ape.
- **A graduation at 6.79 is still bought after a rise to 7.3549 ETH** (+8.32 %). Past that, its ETH waits on 4663, ETH-backed and
  burnable, until a later graduation at the new floor lifts the pool's backing. The monitor pages after 24 h.
- **The purchase limit stays 8 ETH.** It covers the item cap 1.10 × 6.79 = 7.47 and a buy at the edge, 7.35 × 1.05 = 7.72. At a
  7.35 graduation the item cap is 8.09. The canister advances min(item cap, 8 ETH); the 0.09 left stays in the module as revenue,
  and 8 ETH still reaches Ethereum covering a floor up to 7.88.
- **The opening target.** The first coin's is (1.05 × item cap + 0.25) ÷ 0.874: 9.26 at 6.79, so 10 ETH. With no reserve it buys
  exactly the 1.05 units it is sized for, so J5's opening flaw does not apply to BAYC. Above a 7.35 floor, the first coin's target
  rises past 10 ETH (up to `maxRaise`, 30) and still buys one ape.

**Why a zero reserve leaves the BAYC holders' ETH side safe.**

- Every ETH-backed unit is still backed 1:1 by the ETH it paid. A burn pays at most its tier's backing per unit, from the
  convertible pot when there are no reserve units. The ledger and the books hold (`_books` in each of the J1 tests).
- Nothing in the module needs a reserve: `_split`, `beta`, `_tierPrices` and `_release` all handle `reserveUnits` = 0.
- A returned advance's loss (Relay both ways, 48 h, D4) is covered first by revenue and surplus. Only what is left is shared,
  and it is shared by all the ETH-backed units rather than by the convertible ones alone. *(Fix round 3, A3-L-1: a round trip costs
  Relay's fee both ways, the 0.003 ETH gas reserve and gas, about 1 % of an item at the measured fees; the canister now waits a day
  after a collection's advance came back before it advances again, so repeated round trips cannot eat a pool. A way back Relay
  never pays or refunds is closed by the operator as a loss of that advance (`returnAdvance` of 0), taken the same way.)*
- What a zero reserve gives up is a standing ETH pot for exits that conversion never touches. After an ape, about 0.14 units per
  graduation stay ETH-backed. Holders exit by selling in the coins' pools, burning what is ETH-backed, or redeeming a whole unit
  for an ape. That is exit liquidity, not backing.

**What the 10 % conversion costs (stated for the review).**

- A graduation's wX is priced at 1.10 × the floor. A burn pays the lower of the floor less the spread and the backing, so a
  holder who burns at once gets about the floor.
- **A2-L-1 is closed for it (the operator, 2026-09-27; both parts built, §8.1).**
  - *The advance is the buy's bound* (`ethflow::advance_bound`): the floor + 5 % (our fresh reading, never above the module's
    accepted floor), within `max_purchase_wei`, plus the 0.003 ETH gas reserve, grossed up for the Relay fee the judge accepts (3 %)
    and the 0.0002 ETH gas keep, never above the item cap. At 6.79 that is **7.353 ETH, not 7.469**: 0.116 ETH per ape never leaves
    the module (`advance` books it as BAYC's revenue, which tops up its later conversions).
  - *The gas-pot sweep* (`ethflow::sweep_pass`): what the ape did not cost is credited to BAYC's leftover (≈ 0.54 ETH at a listing
    at the floor, ≈ 0.20 at the floor + 5 %, less the fill's gas). Once the canister's Ethereum balance that is nobody's is over
    the target (0.05 ETH, `eth_nft.gas_pot_target_wei`), it goes back through the way back (Relay Ethereum → 4663, judged by
    `relayeth::judge`) to **BAYC's** module, `fundReserve`: that collection's surplus, which tops up its conversions (and so its
    headroom after a floor rise) and never pays a burn.
  - For the four collections at conv 300 the item cap (floor × 1.03) is under the bound, so their advance is unchanged; their
    leftover (the documented 1–3 %) is swept the same way.
- It takes nothing from any unit's backing: the converted unit released exactly its own. A returned advance loses only Relay's
  fees on what was advanced (the kept revenue restores the rest; `Pooled.t.sol`).

**Every collection at its current target** (floors the operator's, 2026-09-27; conv 300 and reserve 2,500 unless named):

| Collection (floor) | Target early / normal | Items per graduation early / normal | Pools above (early / normal target) | Still advanced after a graduation, up to | One-graduation headroom |
|---|---|---|---|---|---|
| Milady (0.930) | 8 / 5 | 5.28 / 3.23 | 4.676 (+403 %) / 2.857 (+207 %) | +1.13 % | +1.13 % |
| Azuki (0.670) | 8 / 5 | 7.33 / 4.48 | 4.676 (+598 %) / 2.857 (+326 %) | +1.00 % | +1.00 % |
| MAYC (0.8735) | 8 / 5 | 5.62 / 3.43 | 4.676 (+435 %) / 2.857 (+227 %) | +1.11 % | +1.11 % |
| Pudgy (3.353) | 8 / 5 | 1.46 / **0.89** | 4.676 (+39 %) / 2.857 (**pooled today**: two graduations per item) | +1.38 % | early +1.38 %; normal: pooled |
| **BAYC (6.79)**, conv 1,000, reserve 0 | 10 / 10 | **1.137** | 7.3506 (+8.26 %) | **+8.32 %** | **+8.26 %** |

**What the table says:**

- **The four collections at conv 300 have about 1 % of rise after a graduation**, the same for every item that graduation buys.
  The gate needs the floor + 1 % + 0.5 %, and a 3 % conversion leaves about 1.5 % minus the fixed 0.0032 ETH. Past it, that
  graduation's remaining items wait on 4663, ETH-backed, until later graduations at the new floor, or the fee slices in the
  surplus, lift the pool's backing per unit; the monitor pages after 24 h.
- Their pooling floors are far off, except Pudgy at the normal target, which pools already.
- Nothing about them is changed here. A wider margin for them would be their own `convBps`, which is the operator's call.

## 2. Per-collection parameters (contracts unchanged, deploy and checks)

`DeployStack`, for each EVM collection, takes these values from the environment. Each is bounded in
`StackChecks.checkNftCollection`; anything outside the bounds is refused before any broadcast.

| Parameter | Default | Bounds | Where it lives |
|---|---|---|---|
| `normalRaise` | 5 ETH | [1, 10] ETH | the collection's `CurveFactory` (immutable) |
| `earlyRaise` | 8 ETH | [`normalRaise`, 10] ETH | the factory (the contract caps it at 10 ETH) |
| `earlyUntilEth` | 2 ETH | [0, 20] ETH | the factory |
| `maxRaise` | 30 ETH | [`earlyRaise`, 30] ETH | the factory |
| `convBps` | 300 | [300, 1,600] | the collection's `FloorModule` (immutable) |
| `reserveBps` | 2,500 | [0, 2,500] | the module |

- **The stack file** records `targets` and `moduleParams` per collection.
- **golive-check** reads the factory's `normalRaise`, `earlyRaise`, `earlyUntilEth` and `maxRaise`, and the module's `convBps`
  and `reserveBps`, against the stack file (a FAIL on any difference). It prints the collection's **items per graduation** at the
  module's accepted floor, and says "pooled: N graduations per item" when that is under one.
- **The canister's `eth_nft` collection** gains nothing. Its `max_purchase_wei` must cover one item at the collection's
  `convBps`: BAYC 8 ETH, Pudgy 4 ETH, under the 10 ETH custody per-item cap.
- **The monitor** reads the pool and pages when it covers an item but nothing has been bought for 24 h (§3.4).
- **The site's collection page** shows the target and the pool.

**Several NFT collections in one stack.** `ETCH_STACK_KIND=nft` with the reviewer's comma lists (`ETCH_NFT_COLLECTIONS`,
`_NAMES`, `_SYMBOLS`, `_SLUGS`) deploys every collection in one run: one treasury and directory, and per collection a wrapper,
poster source, module, lock, factory, router and vaults. This branch adds the per-collection lists `ETCH_NFT_NORMAL_RAISES`,
`ETCH_NFT_EARLY_RAISES`, `ETCH_NFT_CONV_BPS_LIST` and `ETCH_NFT_RESERVE_BPS_LIST` in the same order (unset: the defaults; one
collection: `ETCH_NFT_NORMAL_RAISE` …). The five launch collections, with BAYC at 10 ETH, `convBps` 1,000 and `reserveBps` 0 (J1,
§1.1), are `deployments/launch/nft-pilot.json`; the dry run builds all four lists from it.

## 3. Pooled graduations: the semantics

### 3.1 While pooled, what backs each unit, and where the ETH is

- The ETH is **at the collection's Floor Module on 4663**, in `backing` (convertible) and `reserve`. Each ETH-backed wX is backed
  by what it paid; the module never pays a burn more than that.
- **Nothing is at Relay or on Ethereum while pooled.** The module advances only when net convertible units ≥ 1.05 and
  `maxItemCost()` is non-zero, and it advances exactly one item's budget (at most the floor + `convBps`, never above one unit's
  backing + revenue + surplus).
- Relay carries one item's budget at a time, once, after the pool is sufficient.

### 3.2 Who gets the NFT once it is bought

- **No one coin, and no one holder.** wX is one fungible token per collection. The graduations' units went into each coin's own
  locked pool (`release` → the curve → `LiquidityLock`).
- When the NFT arrives, the canister mints one unit to the module and `settleAdvance` burns one ETH-backed unit: one unit's
  backing turns from ETH into the NFT. The whole supply is still `(held − open) × 1 + ethBacked`.
- Every holder's wX, pooled or not, is one unit of the same thing. A whole unit redeems a random NFT of the collection.
  Fractions trade in the coins' pools, or burn at the module for their backing.
- "Pro rata" is therefore exact **in aggregate**, which is what fungibility means. A per-coin earmark would need per-coin wrappers
  and would split liquidity. It is rejected (J2).

### 3.3 Redemptions and refunds while pooled

- **Before the collection's first item** (and with no deposits), `redeem` reverts `NoItem`: nothing is in custody. Holders exit by
  burning at the module (at most their backing), or by selling in the pools.
- **With holder deposits on (D5)**, deposited NFTs are redeemable at once by any whole unit, pooled ETH-backed units included.
  The ledger still holds: the redeemed unit counts against the item, and `ethBacked` is unchanged, so the pool's ETH still backs
  exactly the ETH-backed units. Deposits do **not** count toward the pool: they mint item-backed units to their depositor and
  leave `netUnits` unchanged.
- **Curve refunds** are unchanged (A-03, the 3-day window and the escape hatch). They concern a graduation the module could not
  take in, not pooling. **A filled graduation has no refund**: its units are wX with a backing. There is no pool timeout (J3): a
  unit that never becomes an NFT is still exactly redeemable for its ETH at the module.

### 3.4 The floor moves while the pool fills

- **The floor rises.**
  - New graduations are priced at the new floor, so their units carry more backing.
  - `maxItemCost` uses the **average** backing per convertible unit, so the pool recovers as it grows.
  - While the average backing plus revenue and surplus cannot pay the cheapest acceptable listing, **nothing is advanced**.
    *(Corrected in fix round 2 of the 2026-09-27 review, A2-M-1: `maxItemCost` did NOT gate this; it caps the advance at the blend
    and pays it out, and the canister advanced exactly that, so a rise of more than ≈ 5 % between two graduations sent a pool that
    could not pay the floor to Ethereum, where it waited with no way back.)* As built after round 2: the **canister** gates it
    (`ethflow::advance_covers`): no buy starts, and no advance is signed, unless `maxItemCost` less Relay's estimated fee
    (50 bps) and the 4663 gas keep reaches Ethereum as at least the module's accepted floor + 1 % and the Ethereum gas reserve
    (0.003 ETH). The module itself has no floor gate (a frog's advance is one inscription's price, which may be under the
    floor). So at BAYC's 9 ETH with the defaults (its configuration before J1; as decided, one graduation per ape, §1.1) a rise of
    about 1–2 % between the two graduations already keeps the pool on 4663
    (`Pooled.t.sol` `test_A2_M1_…`: +1 % converts, +3 % waits); it converts once a later graduation lifts the blend or the floor
    comes back.
  - The monitor pages SEV2 when the pool has covered an item for 24 h with no buy. The message shows `maxItemCost` against the
    module's floor.
  - Holders may burn at their backing at any time.
- **The floor falls.** The item costs less. The unit releases its full backing, and the difference is the module's revenue (the
  module's existing rule).
- **The rise happens after the advance** (in the seconds of Relay, or while the canister waits for a listing). This is D4, as
  before: the ETH waits at the canister's Ethereum address for a listing within its own bound, and pages at 24 h. *(Round 2,
  A2-M-1 / C2-M-2: after 48 h without a fill it goes back: a Relay order Ethereum → 4663 judged like the forward one
  (`relayeth::FROM_ETHEREUM`), then `returnAdvance` to its module; the operator's `eth_give_back` does it at once. Meanwhile the
  buy holds its collection's deposit mints for 6 h at most.)* *(Round 3, E3-L-1 / C3-M-1: both clocks run from the first arrival;
  a way back that gets no acceptable quote within 2 h waits for a listing again and retries a day later, and no longer stops the
  other collections' buys.)* *(Round 4, C4-M-1: a way back Relay neither pays nor refunds stays watched past its deadline (the
  other collections' buys wait meanwhile, as for a forward order), so a late payout is always its own; the operator writes it off a
  day after the deadline at the earliest, and a payout after that is credited to the same collection by its transaction.)*

### 3.5 Relay batching

One advance equals one item (the module refuses a second while one is open, `AdvanceOpen`). Graduations never move ETH across
chains by themselves.

### 3.6 Strictly per collection

- **Contracts.** Each collection has its own wrapper, module, factory, lock and vaults. A module's `backing` pays only its own
  `advance` to the canister, and only its own wrapper's unit settles it (`settleAdvance` checks the module's own wX balance).
  Nothing is shared but the treasury (fees), the registry (the pause) and the poster.
- **Canister.** A buy records its collection, module and wrapper at creation and keeps them. Its arrived ETH is **earmarked** to
  it. Another buy's spend is capped at its own arrived wei **and** at the address's balance less every other open buy's earmark.
  The mint goes to its own wrapper and module; the settlement goes to its own module.

  Tests hold each of these (§5).

## 4. The canister: several collections on one Ethereum address

| Rule | nft-side | nft-pool |
|---|---|---|
| Open buys | one canister-wide | **one per collection** |
| The Relay leg (NeedAdvance → Advancing → Funded → Depositing → Bridging, and Returning, and a Stuck buy still watched) | one | **one canister-wide**: arrival is a rise in our one Ethereum balance and a refund a rise in our one 4663 balance, so two legs at once could not be told apart |
| Past arrival (Arrived → Buying → Bought → Final → Minting → Minted → Settling) | — | **any number**, one per collection |
| A buy's spend | its arrived wei | its arrived wei, and ≤ balance − Σ other open buys' earmarks − the gas reserve |
| The Ethereum line | one transaction in flight | one transaction in flight, **re-priced at the same nonce** after 30 min unmined (+25 % tip and max fee, capped by the chain's fee cap); every version's hash is kept and watched, and whichever mines settles it |
| Order of Ethereum acts | deliveries, then the buy | deliveries (nearest deadline first), then arrived buys round-robin across collections |
| Home-chain acts | one at a time | unchanged; records and marks first, then each collection's buy steps, round-robin |

- **Nonce ordering is strict:** one transaction in flight means nonce n + 1 is never signed before n is mined.
- **The stuck cases:**
  - a fill or transfer under-priced for a spike is re-priced every 30 min up to the cap, and a delivery near its deadline is
    cancelled (unchanged);
  - a nonce used by something not ours stops the line loudly (unchanged);
  - a buy waiting for a listing does **not** hold the line (it signs nothing), so other collections go on.

## 5. Tests (to be written with the code)

- **forge (`test/ethnft/Pooled.t.sol`)**, with real curves and a real factory per collection:
  - two BAYC graduations at 9 ETH: after the first, no buy is due (`maxItemCost` 0 at 0.82 convertible units); after the second
    it is due, and exactly one advance is possible;
  - after the item settles, 0.63 units stay ETH-backed; the ledger and the books hold throughout;
  - Pudgy at 5 ETH: the same, at 0.89 per graduation;
  - two collections: BAYC's pooled ETH never funds Milady's advance, and Milady's settle never uses BAYC's units;
  - a floor rise after the first graduation: no advance while the average backing is short, and the next graduation at the new
    price makes it sufficient;
  - a deposit while pooled: redeemable, `netUnits` unchanged, the ledger holds;
  - per-collection factory and module parameters read back; `StackChecks` bounds each.
- **Canister (`tests/ethnft.rs`):**
  - two collections at once: BAYC arrived and waiting (listing over cap) while Milady's buy completes Relay → buy → mint → settle;
  - only one Relay leg at a time;
  - earmarks: BAYC's buy cannot spend Milady's arrived ETH;
  - a stuck delivery is re-priced at the same nonce, and the replacement's receipt settles it;
  - a stuck fill is re-priced, and the original mining later still settles;
  - deliveries go before buys, and round-robin across collections.
- **Dry run:** the NFT-only stack with two collections (one at 9 ETH targets) deploys and passes golive-check.
- **Monitor:** the pooled-but-not-buying page.
- **Site:** the target and the pool on the collection page.

## 6. Judgment calls for the operator

- **J1 — BAYC's parameters. DECIDED 2026-09-27: 10 ETH (normal = early), `convBps` 1,000, `reserveBps` 0** (§1.1): one
  graduation per ape; it pools above a 7.3506 floor at graduation (+8.26 %); a graduation at 6.79 is still advanced after a rise
  to 7.3549 (+8.32 %); A2-L-1's leftover is closed by the operator's decision of the same day (the advance is the buy's bound, and
  the leftover is swept back to BAYC's module, §1.1). *What this item said before:*
  Committed: the operator's 9 ETH with the defaults (two graduations per ape, 2.6 % floor-rise
  headroom before Relay; since fix round 2 of the 2026-09-27 review the canister advances the pool only while it covers the floor
  + 1 % after Relay's fee, so about 1–2 % between the two graduations, §3.4). Recommended alternative: **10 ETH, `convBps` 1,100, `reserveBps` 0**: one graduation per ape, bought after a floor
  rise of up to ~10 %. More than 10 ETH needs a `CurveFactory` change (`HARD_MAX_EARLY_RAISE`).
- **J2 — No per-coin earmark.** A graduation's units are the collection's fungible wX; a bought NFT backs the collection, not the
  coin whose budget completed the pool.
- **J3 — No pool timeout or refund.** A pooled unit stays ETH-backed and burnable at its backing until an item is bought. A
  floor that stays above the pool's average backing means "wait and page", never "force a buy" or "refund".
- **J4 — Custody room is shared by buys and deposits** per collection (stage 0: 3 items). With deposits on, a burst of deposits
  can take the room a pooled buy needs; the buy then waits. Alternative: reserve one slot for buys (a canister rule). **Not
  built.**
- **J5 — The opening-target flaw** (§1) is left to the review: pooling covers it. (BAYC at reserve 0 is not affected: its
  opening coin buys the 1.05 units it is sized for, §1.1.)
- **J6 — Re-pricing a stuck line transaction** every 30 min at +25 % up to the chain's cap (50 gwei). The alternative is manual
  only.
- **J7 — Stage-0 custody caps. DECIDED 2026-09-27: 15 ETH per Ethereum collection, no canister-wide total.** As built (§8.1):
  one item up to 15 ETH; each collection's item count is what 15 ETH buys at its floor (Milady 16, Azuki 22, MAYC 17, Pudgy 4,
  BAYC 2: `deployments/launch/nft-pilot.json` `custodyItems`); the global caps are the collections' sum (61 items, 75 ETH), so
  they never bind before the collections' own; a deposit counts at the module's accepted floor (review round 3 A3-L-4). The
  canister accepts an Ethereum collection from nothing at up to 24 items and 15 ETH (`custodycap::FIRST_EVM_*`). Because every item
  must be able to leave within the timelock's delay (`outflow::exit_fits`), 22 items need 11 deliveries a day: the launch install
  names `outflow_limits` of 12 / 24 items a day. With it, the operator raised stage 0's H3 value a day from 10 to 25 ETH
  (`outflow::STAGE0_VALUE_WEI`). **Open again (review round 4, A4-L-1):** the exit rule holds for the whole custody too, and at 24
  deliveries a day in all only 48 of the 61 items can leave within the timelock's 72 h less a day. The canister now refuses that
  install and golive-check refuses the pair, printing the options (`fixes-round4.md` §6): item caps summing to 48, a global cap of
  48 (a binding total, which changes this decision), a later raise of `global_items` to 31 a day, or a longer delay. **DECIDED
  2026-09-28: option (c)** (`fixes-round5.md` §6, `deployments/launch/nft-pilot.json` `stages`). **Step 1**, the launch install: J7's
  counts cut in proportion to 48 (Milady 13, Azuki 17, MAYC 13, Pudgy 3, BAYC 2), 15 ETH each, at 12 / 24 a day. **Step 2**: a
  timelocked upgrade, proposed only after step 1 has executed and the handover is done, that raises `global_items` alone to 31 a day
  (`golive-check --proposal --stage 2` holds its argument; the monitor pages it as EXPECTED, not SEV1; any other upgrade executed in
  its 72 h makes it stale and it is queued again). **Step 3**, after two clean weeks: a second upgrade putting the item counts back
  to J7's 61 (caps only; its argument carries `global_items` 31, since every argument is a whole `Init`). Deliveries go backlog
  first (`ethflow::delivery_order`, review round 5 A5-L-2), so step 2 need not raise `per_collection_items`; the canister also
  checks, at every upgrade, that what custody HOLDS can leave at the new limits (A5-L-1). *What this
  item said before:* **Stage-0 custody caps against BAYC and Pudgy (found while
  building).** The canister's stage-0 custody policy is 3 items
  and 10 ETH per collection, and 6 items and 10 ETH across all collections (`custodycap::stage0`). One ape at 6.79 ETH takes
  two thirds of the global 10 ETH; Pudgy holds at most 2 items (6.7 ETH). With five collections live, a BAYC buy can wait on
  the global value cap while the others hold items (the buy then waits for room, and the monitor's pool page fires after a day).
  Nothing is built for this: the operator decides whether BAYC launches at stage 0, or the value caps are raised for the pilot
  through the timelock (G7). *(Since J1, 2026-09-27: a BAYC buy reserves its item cap, `custodycap::check_acquire` at `item_cap`, which is 7.47 ETH at 6.79 and
  up to the 8 ETH purchase limit, against 6.99 at conv 300. So J7 is unchanged and slightly tighter.)*

## 7. As built (2026-09-27)

Rebased on nft-side's `57160b0` (the reviewer's launch configuration: Milady, Azuki, MAYC). The reviewer's comma-list form of
several collections is kept; this branch layers the per-collection targets onto it (§2), adds Pudgy Penguins and BAYC to the
golive-check launch list, and has the dry run deploy all five from `deployments/launch/nft-pilot.json`.

- **Contracts:** unchanged. `test/ethnft/Pooled.t.sol` proves §3 on real curves and factories.
- **Deploy:** `DeployStack` reads the per-collection lists, checks each collection with `StackChecks.checkNftCollection`, and
  reads the factory's targets and the module's `convBps` / `reserveBps` back. The stack file records `targets` and
  `moduleParams`. golive-check compares them (`targetcheck`) and prints items per graduation, with `POOLED: N graduations per
  item` below one.
- **Canister (`ethflow.rs`):** as §4. There is one open buy per collection and one Relay leg canister-wide. Earmarks
  (`earmarked_except`) keep each buy to its own ETH. The line re-prices after `LINE_BUMP_SECS`; `EthTx` gains `to`, `data`,
  `gas`, `earlier` and `rivals` (all `#[serde(default)]`; correction from fix round 1 of the 2026-09-27 review, C-L-4: Candid does
  not fill a missing non-`opt` field, so an `nft-side`-era saved `EthTx` does not decode into this shape; nothing was installed
  from `nft-side`, and the decode probe now refuses such an upgrade instead of restoring the Ethereum side as nothing). An arrived buy whose listing is refused
  records the reason and the next collection's buy is tried in the same tick. A conservative consequence of the earmarks: an
  over-read arrival also holds back the other collections.
- **Tests on a two-collection fake (`tests/ethnft.rs`):**
  - one Relay leg at a time;
  - BAYC waiting with no floor reading while Milady's buy settles;
  - pools never crossing wrappers or modules;
  - earmark isolation;
  - deliveries by nearest deadline, then both buys, on consecutive nonces;
  - a stuck delivery re-priced while BAYC's home side goes on;
  - a re-priced fill whose first version mines, minted against that hash;
  - no re-price over the chain's cap.
- **Monitor:** `poolAlerts` pages (SEV2) when a pool is due a buy and no buy has been open for it for 24 h.
- **Indexer:** reads the factory's targets (`graduation`) and the module's `buffer()` (`pool`) into the collection.
- **Site:** the collection page shows the graduation target and the pool's progress.

**Results:**

| Suite | Result |
|---|---|
| forge, fork directory excluded | 1,096 passed |
| Seaport fork proof (block 26055690) | 2 passed |
| canister `cargo test` | 550 passed (`ethnft` 26) |
| timelock | 15 passed |
| ops | 45 passed |
| monitor | 154 passed |
| keeper | 150 passed |
| indexer | 135 passed |
| web-rook | 72 passed |
| staging and tools/snapshot | typecheck clean |
| `scripts/golive-dryrun.sh stack` | ALL PASS: five NFT collections, BAYC's factory at 9 ETH, "pools two graduations per ape" read at a 6.79 floor |

Two Docker builds, the second with `--no-cache`, gave the same hashes:
- `buyer.wasm` `acaba6a221e0054abafd4e206a7047c514a20c10ea57b1e57b8f590970111a5e`;
- `timelock.wasm` `b23a1903…edc41`;
- `timelock-rehearsal.wasm` `2145099e…62f4`.

## 8. J1 as decided (2026-09-27, branch `nft-bayc10` from `nft-side` `4949da0`)

- **Launch file:** `deployments/launch/nft-pilot.json`: BAYC at `normalRaise` = `earlyRaise` = 10 ETH, `convBps` 1,000,
  `reserveBps` 0. `maxPurchaseWei` stays 8 ETH. `_baycAlternative` is gone.
- **Dry run:** it now passes `ETCH_NFT_CONV_BPS_LIST` and `ETCH_NFT_RESERVE_BPS_LIST` from the launch file. Before, it passed
  only the raise lists, which was harmless while every collection had the defaults. It reads BAYC's factory (10 / 10 ETH) and
  module (1,000 / 0) back. It expects golive-check to say that one BAYC graduation converts 1.14 items with no pooled note, and
  that Pudgy pools two graduations per item at 5 ETH. The shifted-list refusal now moves BAYC's four values onto Pudgy.
- **golive-check:** comments only; `--launch` already holds all four values per collection.
- **Tests:** `Pooled.t.sol` has three J1 tests on the contracts: one graduation buys one ape at 6.79; a rise after the graduation
  up to +8.30 % is still advanced and bought, and +8.54 % waits; a rise before it up to +8.25 % still buys one ape (advancing the
  8 ETH limit), and +8.39 % pools two graduations. The 9 ETH pooled tests stay as the pooled-collection case. `StackChecks.t.sol`
  bounds the decided set and the 1,600 edge.
- **Contracts and canister:** unchanged, so `buyer.wasm` is not rebuilt.

### 8.1 The rest of the operator's decisions of 2026-09-27, as built on this branch

- **J7 (custody caps)** and **H3 at 25 ETH a day**: §6 J7. Canister `custodycap.rs` (`FIRST_EVM_*`, `STAGE0_EVM_ITEMS`,
  `first_of`, `first_global`), `outflow.rs` (`STAGE0_VALUE_WEI`), deposits valued at the accepted floor (`ethflow.rs`, A3-L-4);
  the launch file's `custodyItems` / `custodyWei` and `_custody`; golive-check holds each Ethereum collection's cap against the
  launch file and FAILs a total that binds before the collections' caps; the monitor (`argdiff.mjs`) and the reconciler
  (`ops/src/stage.ts`) mirror the new stage 0; the Trust page and `docs/v5/launch-gates.md` say it.
- **A2-L-1 (the leftover)**: §1.1. Canister `advance_bound` (the advance), `sweep_pass` (the sweep: it takes the one Relay leg,
  never while any buy, delivery, return or line transaction holds ETH at the address; abandoned after 2 h without a quote, then a
  24 h cooldown), a per-collection `leftover` credited when a fill mines (what arrived less the price and the gas) and when a way
  back's deposit mines (the kept reserve less its gas); `eth_nft.gas_pot_target_wei` (0.05 ETH, within [0.01, 1]); `EthBuy.sweep`;
  the sweep's 4663 call is `FloorModule.fundReserve()` (surplus: no contract change). The monitor never reads a sweep as an advance.
- **The Ethereum provider (Dwellir)**: `deployments/launch/monitor-nft.json` names
  `https://api-ethereum-mainnet.n.dwellir.com/${DWELLIR_API_KEY}` first and `xrpc.cl/eth` second; the key is expanded from the
  monitor's environment and scrubbed from alerts and status. Every `eth_getLogs` is at most 500 blocks: the monitor's deposit scan
  (`eth_nft.logs_max_blocks`, and only new blocks after the first poll), the runner's discovery (`ETH_LOGS_MAX_BLOCKS`), the
  canister's sale scan (`eth_nft.logs_max_blocks`: the same blocks a pass as before, in narrower chunks). The canister keeps a
  keyless provider pair: its install argument is published with every timelock proposal.
- `docs/audit/2026-09-27-nft/bayc10-decisions.md` has each decision's change, tests and results, what is left for the round-3
  fixer, and the diff list.

