# Rook security review, area A: the bonding curve and launch path

Reviewer A, 2026-09-25. Scope: `src/curve/FrogCurve.sol`, `src/curve/CurveFactory.sol`, `src/curve/ItemQueue.sol`, `src/curve/MintDesk.sol`,
`src/pools/LiquidityLock.sol`, `src/pools/EtchRouter.sol`, and the graduation hand-off into `src/floor/FloorModule.sol`
(`enqueue` / `_tryFill` / `fill` / `release` / `cancel` / `fundReserve` / `fundLiquidity`), plus what these contracts call
(SweepVault `deposit`/`depositWrapped`/`sweepViaDesk`/`collectDesk`, the Uniswap v4 PoolManager).

- **Commit:** `e04f49b2bb94597e0107a607c5bc7ac3d3e3bbce` (origin/main, detached), checkout `/Users/jlohayon/Documents/Rook-audit-A`.
  The submodules were empty in this checkout. I populated `lib/` from `~/Documents/Rook/lib`, which is at the exact pinned
  commits (forge-std `bf647bd`, OZ `c64a1ed`, v4-core `46c6834`).
- **The chain the PoCs ran against:** the PoCs etch `test/fixtures/v4-pool-manager-4663.hex`. Its code hash
  (`0xbd388118…95626`) equals `keccak(cast code 0x8366a39C…0951)` read from `rpc.mainnet.chain.robinhood.com` (chain id 4663)
  today, so the PoCs ran against the live PoolManager bytecode.
- **What I changed:** I added PoC files under `test/audit/A/` and nothing else. I did not modify `src/`, `script/` or
  `docs/`, did not commit, and did not touch a key or send a transaction.

## 1. Methodology

I worked in passes, following these published sources:

- **Trail of Bits, Building Secure Contracts**, development guidelines and audit preparation
  (https://github.com/crytic/building-secure-contracts, `development-guidelines/guidelines.md`). Used for the design,
  implementation and dependency passes: are the docs consistent with the code, are the external dependencies (Uniswap v4
  semantics) understood, are the tests adequate.
- **OWASP Smart Contract Top 10 (2025)** (https://owasp.org/www-project-smart-contract-top-10/). Checked every category:
  - SC01 access control;
  - SC02 price-oracle manipulation (the keeper cap, spot-price adds);
  - SC03 logic errors;
  - SC04 input validation;
  - SC05 reentrancy (including cross-contract and read-only);
  - SC06 unchecked external calls;
  - SC07 flash-loan and same-block moves;
  - SC08 over/underflow and rounding;
  - SC09 randomness (not in this area);
  - SC10 DoS.
- **SWC registry** (https://swcregistry.io/): SWC-107 reentrancy, SWC-113 DoS with failed call, SWC-128 DoS with block gas
  limit, SWC-116 block values as time, SWC-132 unexpected Ether balance, SWC-101 arithmetic, SWC-114 transaction-order
  dependence.
- **Consensys Smart Contract Best Practices** (https://github.com/Consensys/smart-contract-best-practices, whose site now
  redirects). Used its attack list: reentrancy, front-running, timestamp dependence, insecure arithmetic, DoS, griefing and
  force-feeding.
- **Cyfrin / Solodit audit checklist** (https://github.com/Cyfrin/audit-checklist, `checklist.json` downloaded). Used its
  categories "Attacker's Mindset" (DoS, donation, front-running, griefing, price manipulation, sandwich), "Defi → AMM/Swap",
  "Integrations → Uniswap", "Basics → Math / Payment / Array-Loop" and "Token → ERC20".

The passes:

1. **Docs.** I read `docs/v6/phase1-scope.md` (which paths are ON), `docs/v6/deploy-stack.md` and `script/DeployStack.s.sol`
   (what is actually deployed and with which constants), and the NatSpec of every file in scope.
2. **Invariants and trust assumptions** (§3).
3. **Line-by-line review** against the checklists above.
4. **Breaking the invariants** with PoC and fuzz tests, against the real 4663 PoolManager runtime and the Phase-1 parameters.
   The factory's queue is the FloorModule, not ItemQueue. The template, targets and module params are copied from
   `DeployStack._template/_targets` and `StackChecks.defaultParams(1e18, 0)`.
5. **Gaps in the existing tests** (§7).
6. **Slither 0.11.6** over the Foundry project, triaged (§6).

## 2. Scope as deployed in Phase 1 (verified in `script/DeployStack.s.sol`)

- **Curves.** `CurveFactory(queue = FloorModule)` is ON and permissionless. The template is 1% fee (tiers 1–8%), a 50%
  launch fee decaying over 10 min, the 40/30/5/5/20 split, a 3-day window, and raises of 5 ETH normal / 8 ETH early (the
  30 ETH ceiling is `maxRaise`).
- **`ItemQueue` is not deployed** by `DeployStack` (the factory's queue is the module). I read it, but it is Phase-1
  unreachable.
- **`MintDesk` is deployed.** It is wired into all three vaults (`sweepViaDesk`). "OFF" means only that the canister never
  calls `fulfill`. No on-chain switch exists (A-05).
- **`LiquidityLock` and `EtchRouter` are deployed and ON.**

## 3. Invariants and trust assumptions (area A)

| # | Invariant | Holds? |
| --- | --- | --- |
| I1 | While Trading: `reserveEth == v − v0`; `v·t` never decreases; circulating coins `== t0 − t` | Yes (reviewed; the existing invariant suite covers it) |
| I2 | `address(curve).balance ≥ reserveEth + feesForVault + creatorOwed + platformOwed + reserveOwed + liquidityOwed` | **No:** a closing buy can leave it short by a few wei (A-02) |
| I3 | Nobody can take more ETH out of a curve than they put in, net of the curve price (no rounding extraction) | Yes: every rounding favours the curve; sell cannot drive `v` below `v0` |
| I4 | A Graduating curve always reaches Graduated or Refunding (**liveness of holders' funds**) | **No:** A-01 blocks `graduate()` for good, and `openRefunds` is refused once the module has filled (or always, for SWEEP_ONLY) |
| I5 | Graduation happens at most once; only the curve can enqueue, release or cancel its own coin | Yes (`state` guard; `_onlyCurve` via `factory.routerOf`) |
| I6 | The lock never removes liquidity; ETH leaves it only to the PoolManager, curves or the treasury | Yes |
| I7 | Graduation liquidity is added at a fair price | Only within `maxDeviation` (±~10%) of the keeper cap. That tolerance is monetisable atomically because `graduate()` is anyone's (A-04) |
| I8 | OFF paths cannot be used on chain | MintDesk: `fulfill` is custodian-only (canister policy), but `sweepViaDesk` is anyone's (A-05) |

Trust assumptions:

- The keeper posts honest caps and floors. `gradPrice` uses `max(price(), target)`, and `target` is the keeper's raw,
  unclamped post, so a high `target` shrinks every graduation's units.
- The custodian is honest about item costs, within `maxItemCost`.
- The PoolManager is canonical v4. Its code hash was checked.
- The sequencer can skew `block.timestamp`. Every wait in scope uses both clocks (`Wait.elapsed`, min of the two clocks for
  the launch-fee decay), which I checked.

## 4. Findings

| ID | Severity | Title | Location | Phase-1 reachable? | PoC |
| --- | --- | --- | --- | --- | --- |
| A-01 | **Critical** | Anyone can permanently freeze every graduation of a collection (and any single coin) for ~2e12 wei by saturating `maxLiquidityPerTick` at the full-range tick the lock always uses | `LiquidityLock.sol:490-501` (shared full range), `:453-461` (coin pool full range); impact via `FrogCurve.sol:318-343, 347-349`, `FloorModule.sol:565-575, 583-597` | **Yes** | `test/audit/A/TickSaturation.t.sol`, 4/4 PASS |
| A-02 | Low | The closing buy can book more fees than it received: the curve becomes insolvent by 1–2 wei and the last claimant is refused | `FrogCurve.sol:241-246, 271` | Yes | `test/audit/A/LastBuyAccounting.t.sol`: 2 PASS; the fuzz property FAILS on current code, as intended |
| A-03 | Medium | At stage 0 the module fills up after about 5–6 graduations. Every later curve of the collection is then bound to refund (fees kept), and a public mint can front-run a closing buy into that state | `FloorModule.sol:588`; `FrogCurve.sol:347`; params `StackChecks.sol` | Yes | none (arithmetic below, from measured units) |
| A-04 | Low | Atomic sandwich of `graduate()`: push the shared price to the `maxDeviation` edge, graduate someone else's coin, swap back. Net profit is about 0.011–0.013 ETH per graduation, taken from protocol-owned liquidity | `LiquidityLock.sol:298-324, 484-514`; `FrogCurve.sol:318` | Yes | `test/audit/A/GraduationSandwich.t.sol`, 3 PASS (measurement) |
| A-05 | Low | "MintDesk OFF" is not enforced on chain: anyone can park a coin's whole vault credit at the desk for 6 h, again and again, starving its sweeps | `SweepVault.sol:328-341`, `MintDesk.sol:65,98` | Yes | `test/audit/A/DeskOff.t.sol`, 1 PASS |
| A-06 | Info | `LiquidityLock.coinKey(coin)` names the COIN/**wX** pool for a SWEEP_ONLY coin until it is seeded | `LiquidityLock.sol:645-652` | Yes (view) | asserted inside the A-01 SWEEP_ONLY test |
| A-07 | Info | `buy`/`sell` on the curve take `minOut` but no deadline | `FrogCurve.sol:232, 275` | Yes | — |
| A-08 | Info | `MintDesk.collect` ignores `wrapper.transfer`'s return value (the wrapper reverts on failure, so this is harmless); inconsistent with `reclaimStray` | `MintDesk.sol:114` | Yes | — |
| A-09 | Info | `_coinPositions[PROTOCOL]` grows by up to 3 ids per distinct center; `claimShared(PROTOCOL)` pokes them all in one loop | `LiquidityLock.sol:369-374, 541-544` | Yes (slow) | — |
| A-10 | Info | Graduation units are priced at `max(price(), target)`, where `target` is the keeper's raw post and not the move-limited `accepted` | `FloorModule.sol:906-908` | Yes | — |

Counts: **1 Critical, 0 High, 1 Medium, 3 Low, 5 Informational.**

---

### A-01 (Critical): tick-liquidity saturation freezes graduations permanently

**Description.** Uniswap v4 caps the gross liquidity that may reference any single initialized tick:
`Pool.modifyLiquidity` reverts `TickLiquidityOverflow(tick)` when `liquidityGrossAfter > tickSpacingToMaxLiquidityPerTick(spacing)`
(v4-core `Pool.sol:164-172`). This applies to every positive add, whoever makes it.

The lock always adds a **full-range** position whose bounds are the fixed, public ticks
`TickMath.minUsableTick/maxUsableTick(spacing)`:

- **The shared ETH/wX pool.** `_cbAddShared` (`LiquidityLock.sol:490-501`) runs the i == 2 layer on `[minT, maxT]` on every
  graduation, and on protocol adds that carry wX.
- **Each coin's own pool.** `_cbSeedCoin` (`:453-461`) adds `[minUsableTick(200), maxUsableTick(200)]`.

Near those extreme ticks liquidity is almost free. A one-spacing position `[maxT − spacing, maxT]` lies above the price, so
it is ETH-only (currency0). The **whole** per-tick cap there costs about `L_max · 5.4e-20 · 0.003`:

- about **1.9e12 wei** for the shared pool (spacing 60), measured;
- about **2.1e13 wei** for a coin pool (spacing 200), measured.

Anyone can fill the cap. From then on, any full-range add by the lock reverts. Nobody but the attacker can remove the
position, and the lock has no owner and no fallback.

**Why this is a permanent freeze, not a delay:**

- `FrogCurve.graduate()` calls `seeder.seed` → `_deploy` → `_cbAddShared`, which reverts. For SWEEP_ONLY it calls `seedEth` →
  `_cbSeedCoin`, which reverts. `graduate()` is therefore unreachable.
- In Phase 1 the queue is the FloorModule, and `enqueue` fills **at once** (`_tryFill`, `FloorModule.sol:574, 589`), so
  `isFilled(coin)` is true from the closing buy onward. `openRefunds` requires `!queue.isFilled(token)` (`FrogCurve.sol:348`),
  so refunds can never open. For SWEEP_ONLY, `openRefunds` refuses outright (`sweepOnly` check), so there is no exit at all.
- `sell` and `refund` refuse in Graduating (`WrongState`).
- Holders are left with coins that cannot be redeemed or traded on the curve, and no pool exists.

The ETH is stuck in two places:

- **In the curve:** `poolEth + platformFee` + fees. That is 1.34 ETH in the PoC for an 8-ETH raise, or about 5.05 ETH, the
  whole raise, for a 5-ETH SWEEP_ONLY coin.
- **In the module:** the item budget, 6.742 ETH, as backing for `gradUnits` that are never released. Those units stay
  counted in `netUnits`, so the canister keeps converting them into frogs that coin holders never get.

**Blast radius.** **One** position in the collection's shared pool blocks every later non-SWEEP_ONLY graduation of that
collection. The PoC proves this for two coins in a row. Per-coin attacks on coin pools also work: the pool key is
deterministic from the coin address, which is known at launch. For a COIN/wX pool the attacker needs dust of the currency
on the chosen side.

By inspection (not in a PoC), the freeze reaches further:

- Once `pendingWrapped[PROTOCOL]` is non-zero (compounded wX fees), `addProtocolLiquidity` also reverts.
- That makes `FloorModule.fundLiquidity` revert, and with it every curve's `flushProtocolFees` (which also carries the
  reserve slice), and `claimShared(PROTOCOL)`.

It is also an extortion lever: the attacker can remove the position after being paid. Because the attack only needs to be
in place before the next graduation, it also works if the lock already has full-range positions: top the tick up to
`L_max − existing gross`.

**Exploit scenario.**

1. Mallory initializes the shared pool (permissionless; any price near the cap), or uses the existing one.
2. She calls `modifyLiquidity([887220−60, 887220], L_max(60))`, paying about 0.0000019 ETH.
3. Every coin that later sells out on the frog collection is frozen for ever with its raise.
4. For SWEEP_ONLY coins she can target each coin's COIN/ETH pool right after launch.

**PoC.** `test/audit/A/TickSaturation.t.sol`, Phase-1 stack, real 4663 PoolManager bytecode: **4/4 PASS.**

- `test_A01_sharedPoolSaturation_freezesEveryGraduation`: attack cost 1,878,783,905,959 wei. `graduate()` reverts
  `TickLiquidityOverflow(887220)` before and after the window. `openRefunds` → `WrongState`; `sell`/`refund` → `WrongState`.
  Frozen: 1.338808 ETH in the curve, 6.742 ETH item budget, 10.13 wX held in the module for the curve.
- `test_A01_onePositionBlocksEveryLaterCoin`: two later coins, both frozen.
- `test_A01_sweepOnlyCoinPoolSaturation_freezesTheRaise`: attack cost 2.1e13 wei; 5.0505 ETH frozen, with no refund path.
- `test_A01_control_graduatesWithoutTheAttack`: the same flow graduates without the attack.

**Recommended fix.** Do several of these, in order of importance:

1. **Never put a lock position's bound on a tick that is cheap to saturate.** Replace `[minUsableTick, maxUsableTick]` with a
   bounded "wide-wide" range around the center or seed price, for example ±~100k–200k ticks. Filling `L_max` at a tick that
   close to the price costs more than any attacker has: about 1e29 wei at ±100k ticks. Apply this to both the shared layer
   and `_cbSeedCoin`. (The near and wide bands already sit close to the price and are not economically saturable.)
2. **Make graduation independent of the shared add.** In `seed`, run `_deploy(token)` through a `try this.deployExternal(...)`
   (or have `_deploy` catch the `unlock` revert) and leave the amounts in `pendingEth/pendingWrapped` for a later `deploy()`.
3. **Make the coin-pool seed fail soft.** On a revert, keep the reserved coins and quote pending, and allow a retry with an
   alternative range.
4. **Add a liveness escape hatch to the curve.** If a curve is Graduating and `graduate()` has not succeeded by
   `deadline + X`, let `openRefunds` proceed even when filled. The module would `release` the units to the curve, which
   refunds `ETH + wX` pro rata; SWEEP_ONLY would refund its ETH.
5. **Add regression tests** for each of the above.

---

### A-02 (Low): the closing buy can over-book, leaving the curve insolvent by 1–2 wei

**Description.** In `buy` (`FrogCurve.sol:241-246`), when `net ≥ room` the code recomputes
`gross = ceil(room·BPS/(BPS−rate))` and `fee = gross − net`, but never checks `gross ≤ msg.value`. Because
`net = msg.value − ⌊msg.value·rate/BPS⌋` rounds in the buyer's favour, a `msg.value` in a narrow band closes the curve while
`gross > msg.value`. For example, rate 100: `msg.value = 101` gives net 100 = room and gross 102. The curve then books
`gross` (reserve + fees) but receives only `msg.value`; the `refund excess` branch (`:271`) is skipped. The books exceed the
balance by `gross − msg.value`, up to about `BPS/(BPS−rate)` wei (1–2 wei in practice).

**Impact.** Whoever claims last is refused `TransferFailed` in full, however large the claim: for example the platform's
0.266 ETH (graduation fee + trade fees) in the PoC. `receive()` accepts ETH only from the queue, so the only cure is a
SELFDESTRUCT force-send. Anyone can deliberately be the closing buyer and pick the value. Anyone can also claim the vault,
protocol and platform slices, so a griefer can choose who is left short. (`graduate()` itself stays reachable, because the
0.25 ETH graduation fee sits in the balance until it is claimed.)

**PoC.** `test/audit/A/LastBuyAccounting.t.sol`:

- `test_A02_lastBuyBooksMoreThanItReceives` PASS: owed − balance = 1 wei; `claimPlatformFees` reverts; a direct payment is
  refused; after a selfdestruct top-up it succeeds.
- `test_A02_duringLaunchFee_largerShortfall` PASS.
- `testFuzz_A02_closingBuyIsSolvent` **FAILS on the current code** (counterexample found within 17–19 runs on each of two runs). This is the property
  the fix must make pass.

**Fix.** After re-deriving `gross`, clamp it: `if (gross > msg.value) gross = msg.value; fee = gross − net;`. This is safe
because `net = room ≤ msg.value − ⌊msg.value·rate/BPS⌋ ≤ msg.value`. Add the fuzz property to the suite.

---

### A-03 (Medium): at stage 0 the module saturates, so later curves are bound to refund, and this can be forced

**Description.**

- `_tryFill` converts a graduation only if `ethBacked + units ≤ maxEthBacked` (40 units, not scaled by stage;
  `phase1-scope` §7.9).
- An 8-ETH graduation mints **10.13** units (measured in A-01). A 5-ETH one mints about 6.2 (budget 4.12 at about
  0.669 ETH per unit).
- At stage 0 custody holds at most 3 items, so at most about 3 units are ever converted out of `ethBacked`.
- The graduation wX then sits in never-withdrawable LP positions. Only units that traders buy out of the pools and burn at
  the module reduce `ethBacked` (5 an hour).

So after about 5–6 graduations of the collection, every later closing buy leaves its item budget **Queued**, and the curve
refunds after 3 days. The site keeps offering launches.

Buyers get `refundEthPool·amount/forSale`, the average price, so late buyers lose against early ones. Trading fees and the
up-to-50% launch fee are **not** refunded: they went to the vault, the creator, the platform and the module.

Public `mint` at the module (up to `publicCap`) can push `ethBacked` over the line in front of a specific closing buy,
forcing that curve into the refund path. `phase1-scope.md` §1 mentions that such a graduation "waits … and refunds", but
not that at stage 0 this becomes the steady state after a handful of coins, or that it can be forced.

**Fix.**

- Scale `maxEthBacked` and `maxPublicEthBacked` with the stage and with what custody can convert.
- Have the factory refuse (or the site hide) launches while `module.maxEthBacked − ethBacked < units for this raise`.
- Consider keeping public mints from taking room that queued graduations need (reserve headroom for Queued grads).

---

### A-04 (Low): atomic sandwich of permissionless `graduate()` against the shared pool

**Description.** The shared add prices the new liquidity at the pool's spot, accepted anywhere within `maxDeviation`
(1000 ticks) of the keeper cap (`_far`, `LiquidityLock.sol:309-315`). `graduate()` is anyone's, so a single transaction can:

1. swap the shared pool to about 990 ticks off the cap;
2. call `graduate()` on someone else's filled coin, so the lock adds about 1 ETH + about 1.5 wX of layers at the skewed
   price;
3. swap back.

**Measured** (`test/audit/A/GraduationSandwich.t.sol`, after one normal graduation):

- **Net profit** for the sandwich, valued at the cap: +0.01065 ETH pushing wX cheap, +0.01299 ETH pushing wX dear.
- **Control**, the same round trip with no graduation in between: −0.00372 ETH.

The same applies to `deploy(coin)` for deferred amounts and to protocol-liquidity adds. The loss falls on protocol-owned
liquidity, about 0.5–0.7% of each shared contribution.

**Fix.**

- Tighten `maxDeviation` for adds (for example ≤ 100–200 ticks).
- Or check the pool price against a TWAP or the module's `price()` as well as the cap.
- Or make the add wait (defer) unless the spot price is within a narrow band of the band's own center.

---

### A-05 (Low): MintDesk "OFF" is policy, not code; `sweepViaDesk` lets anyone lock a coin's vault credit

**Description.** Phase-1 scope says MintDesk is OFF because the canister is given no desk. On chain, every vault is
constructed with `desk`, and `sweepViaDesk` (`SweepVault.sol:328-341`) is permissionless. It needs only a fresh keeper cap,
which is always the case in normal operation.

A stranger can move a coin's entire `credit`, one cap at a time, into desk requests that nobody will fill. They stay there
until the 6-hour TTL. Then `collectDesk` (anyone) returns them, and the stranger repeats. Meanwhile `sweep` reverts
`Insufficient`, so the coin's RAFFLE/BURN buys, which are Phase-1 ON, never happen. The cost is gas only. The funds are not
lost.

`MintDesk.fulfill` is custodian-only, so no value can be extracted as long as the canister never fulfills. That part of
"OFF" holds.

**PoC.** `test/audit/A/DeskOff.t.sol::test_A05_strangerParksVaultCreditAtTheDesk`: PASS. 3 requests empty the credit, a sweep
reverts `Insufficient`, and credit is restored after the TTL.

**Fix.** For Phase 1, deploy the vaults with `desk = address(0)`: the constructor allows it when a venue exists. Otherwise,
restrict `sweepViaDesk` to the keeper or custodian, or add an owner-settable `deskEnabled` flag that defaults to false.

---

### A-06 to A-10 (Informational)

- **A-06.** `coinKey(coin)` reads `coins[coin].quoteIsEth`, which is false until seeding, so for a SWEEP_ONLY coin it returns
  the COIN/wX key before graduation. The site, the indexer or the router would look at the wrong pool pre-graduation. Pass
  the quote type explicitly, or read `factory` and the curve's `sweepOnly()`.
- **A-07.** Curve `buy`/`sell` have slippage protection but no deadline. A delayed transaction can still execute at a
  worse, in-bounds price. Add `deadline`.
- **A-08.** `MintDesk.collect` ignores `wrapper.transfer`'s boolean. The wrapper is OZ ERC20 and reverts, so this is benign,
  but use `safeTransfer` for consistency.
- **A-09.** Protocol positions accumulate as the center moves (at most once a day), and `claimShared(PROTOCOL)` loops over
  all of them. That is years away from the gas limit, but unbounded. Consider per-id claiming.
- **A-10.** `gradPrice()` uses `max(price(), target)`, so a keeper `target` far above `accepted` (not move-limited) shrinks
  the units a graduation receives. This is keeper trust. Consider using `accepted`.

### Checked and found sound (no finding)

- **Buy/sell math.** Every rounding favours the curve (`Math.Rounding.Ceil` on the post-trade reserve), `k` never
  decreases, sells cannot push `v` below `v0`, and `tEnd` is rounded up so the last buy takes exactly what is left.
- **Launch fee.** The decay uses min(timestamp, parent-block×12 s); the extra goes wholly to the vault; sells pay no launch
  fee, which is harmless.
- **Fee split.** The platform takes the remainder.
- **Reentrancy.** The ETH sends in `sell`/`buy`/`refund`/claims all follow CEI, and every entry point is `nonReentrant`.
  `onPoolFees` is unguarded by design: seeder-only, and it only books.
- **Double graduation or enqueue.** Impossible.
- **Enqueue/release/cancel.** Only the coin's own curve can call them.
- **Refund accounting.** `returned` is bounded by circulating supply (`forSale`).
- **Force-sent ETH.** It is harmless surplus (and see A-02).
- **Factory.** No owner. `wire`/`wireYoyo` are deployer-once with consistency checks. Fee tiers are enforced both in the
  factory and in the curve constructor.
- **EtchRouter.** It holds nothing between calls; `minOut` and `deadline` are enforced. The `to` callback on a module burn
  can only affect the caller's own trade.
- **Coin-pool pre-initialization by a third party.** The half-budget swap buys or sells only at prices on the lock's side of
  its own valuation (the existing tests cover this). Only the A-01 variant breaks the seed.

## 5. PoC files and results

The command was `forge test --match-path 'test/audit/A/*' -vv`. Full output is in
`scratchpad/rook-audit/A/poc-results.txt`.

| File | Test | Result |
| --- | --- | --- |
| `AuditABase.sol` | harness: the Phase-1 stack (module queue, DeployStack template/targets, stage-0 params, 4663 PoolManager) | — |
| `TickSaturation.t.sol` | 4 tests (A-01) | PASS ×4 (the attack works) |
| `LastBuyAccounting.t.sol` | 2 tests + 1 fuzz property (A-02) | PASS ×2; the fuzz property FAILS as expected (the bug) |
| `GraduationSandwich.t.sol` | 3 measurements (A-04) | PASS ×3 |
| `DeskOff.t.sol` | 1 test (A-05) | PASS |

## 6. Slither triage (0.11.6, `slither . --filter-paths "lib|test|script"`)

There are 457 results project-wide, 160 of them in area-A files. Raw output: `scratchpad/rook-audit/A/slither.json`, filtered
to `A/slither-mine.json`. **None is a real issue in this area, and none found A-01 to A-05.**

| Detector (count) | Verdict |
| --- | --- |
| arbitrary-send-erc20 (1): `EtchRouter` `safeTransferFrom(payer, …)` | False positive: `payer` is the outer `msg.sender`, encoded by `sell` itself |
| arbitrary-send-eth (1): `LiquidityLock.claimShared` → `treasury` | False positive: `treasury` is deployer-wired once |
| weak-prng (1): `FloorModule._volume` `day % 7` | False positive: a ring index |
| reentrancy-balance (1), unchecked-transfer (1): MintDesk `reclaimStray` / `collect` | Informational (A-08); trusted wrapper, custodian-only |
| reentrancy-eth (4), reentrancy-no-eth (5), reentrancy-benign (11), reentrancy-events (2) | False positive: the calls are to the trusted PoolManager inside `unlock`, with `nonReentrant` entry points; `rentIn` (OFF) calls the trusted wrapper |
| divide-before-multiply (3) | Intentional (`_floor` tick rounding; `pe` then the `2·pe` check); dust in `_revenue` |
| incorrect-equality (22), uninitialized-local (11), unused-return (21), timestamp (47), shadowing-local (4), missing-zero-check (1: the module's `treasury` may be zero by design), calls-loop (2), costly-loop (3, ItemQueue, not deployed) | Noise or by design; timestamp uses are paired with block-number clocks |

Note: Slither's crytic-compile runs `forge clean` + `forge build --force`, which wipes `out/`. Expect a full re-compile
afterwards.

## 7. Gaps in the existing tests

1. **No test adds third-party liquidity at the lock's full-range ticks**, or checks `maxLiquidityPerTick`. The front-running
   tests (`LiquidityLock.t.sol:47,62,167`) only pre-initialize pools or add near-price liquidity. **A-01 went uncaught.**
2. **`FrogCurve.invariant.t.sol` has the right invariant but cannot reach the bug.** `invariant_ethIsExactlyTheRaisePlusTheFeesOwed`
   would catch A-02, but the handler buys with random `uint96` amounts and essentially never lands in the 1–2-wei closing
   band. It also wires **ItemQueue**, not the FloorModule that Phase 1 deploys. Add a targeted "close exactly" handler action
   and a FloorModule-queue variant.
3. **No liveness invariant** of the form "every Graduating curve can reach Graduated or Refunding". This is the property A-01
   breaks.
4. **No test of `graduate()` bundled with pool manipulation** in the same transaction (A-04). The sandwich tests cover the
   router only.
5. **No test of stage-0 capacity:** repeated graduations until `maxEthBacked` binds, and whether later curves can still
   graduate (A-03).
6. **No test that the Phase-1 desk path is unreachable or harmless** (A-05). `MintDesk.t.sol` tests the desk working.
7. **No test of `coinKey` before seeding for SWEEP_ONLY** (A-06).

## 8. What I did not cover

- The canister (Rust), the keeper, the runner and off-chain services. So the claim "the canister never fulfills the desk or
  converts only within custody caps" is taken from the docs.
- Wrapper, WrapClaim, SweepVault internals beyond the calls the curve and desk make (`deposit`, `depositWrapped`,
  `sweepViaDesk`, `collectDesk`), ProtocolTreasury, OfferBoard, and the FloorModule's mint/burn/rental/price economics beyond
  the graduation lane. Those belong to other reviewers.
- ItemQueue in depth (not deployed in Phase 1). Note that it is **also** exposed to A-01: once `Filled`, `openRefunds` is
  refused.
- Uniswap's protocol fee (unset in the etched fixture), and gas measurements on 4663.
- A long-horizon invariant campaign (`FOUNDRY_PROFILE=deep`). I ran only the default fuzz runs for my property test.
