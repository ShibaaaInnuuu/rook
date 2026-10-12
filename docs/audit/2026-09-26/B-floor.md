# Rook security review, area B: the floor module, wrapping and custody bookkeeping (EVM side)

Reviewer B, 2026-09-25. Checkout `/Users/jlohayon/Documents/Rook-audit-B`, detached at `origin/main` =
**`e04f49b2bb94597e0107a607c5bc7ac3d3e3bbce`** ("Merge branch 'p1-keeper'"). Nothing under `src/`, `script/`, canister code or
`docs/` was modified. PoC tests were added at `test/audit/B/FloorAuditB.t.sol` (not committed). Chain 4663 was read once over
public RPC (chain id, head, `l1BlockNumber`), and nothing else.

## 1. Methodology

These sources were used as checklists, pass by pass:

- **Trail of Bits, Building Secure Contracts.** The development guidelines (design, implementation and deployment:
  https://github.com/crytic/building-secure-contracts/blob/master/development-guidelines/guidelines.md) and the audit-preparation
  / workflow guidance (https://secure-contracts.com/development-guidelines/workflow.html). They were used for pass 1 (the
  documented intent and invariants), for the Slither pass, and for the deployment/incident-response lens (a key lost or
  compromised, what monitoring can actually stop).
- **OWASP Smart Contract Top 10 (2025)** (https://owasp.org/www-project-smart-contract-top-10/): SC01 access control, SC02 price
  oracle manipulation, SC03 logic errors, SC04 input validation, SC05 reentrancy, SC06 unchecked calls, SC07 flash loans, SC08
  arithmetic, SC09 randomness, SC10 DoS. Each was walked against every external function in scope.
- **SWC registry** (https://swcregistry.io/): SWC-107 reentrancy, 104 unchecked call, 105/106 access control, 113/128 DoS,
  116 timestamp dependence, 120 weak randomness, 121/117 signature replay and malleability, 132 unexpected ether.
- **Consensys smart-contract best practices** (https://consensys.github.io/smart-contract-best-practices/): the oracle,
  force-fed ether, pull-over-push, checks-effects-interactions and "prepare for failure" (circuit breakers, upgrade and
  rotation paths) sections.
- **Community checklist.** Solodit/Cyfrin aggregated audit checklist (https://github.com/Cyfrin/audit-checklist,
  https://solodit.cyfrin.io/checklist), mainly the DoS, griefing, oracle/price, rounding and centralization items.

The work went in five passes:

1. **Docs.** `docs/v6/phase1-scope.md` first, then `docs/v5/launch-gates.md`, `docs/v6/deploy-stack.md`,
   `docs/v5/floor-module.md` (§4 books, §5 floor, §6 defences, §12 non-goals, §13 reserve and treasury),
   `docs/trust-model.md` and `docs/v5/custody-hardening.md`.
2. **Invariants and trust assumptions** (§3 below).
3. **Line-by-line review** of `FloorModule.sol`, `Wrapper.sol`, `WrapClaim.sol`, `ListingRegistry.sol`,
   `AutoBuyerVault.sol` and `Distributor.sol`, plus the deploy wiring in `script/DeployStack.s.sol` and `script/StackChecks.sol`.
   `AssetVault.sol` was skimmed only: it is v1 and unreachable in Phase 1. The interfaces touched were
   `ProtocolTreasury.sol`, `FrogCurve.sol` (the queue calls) and `EtchRouter._wrappedToEth`.
4. **Attacks.** PoC and fuzz tests for each invariant I could break or wanted to stress.
5. **Existing tests.** The floor suites and the invariant handler were checked for gaps.

## 2. Scope and what is ON in Phase 1

All of the following was checked in code:

| Path | Phase 1 | Reachable on chain? |
| --- | --- | --- |
| `FloorModule.mint` / `burn` | ON | Yes, anyone, whenever the keeper's post is fresh |
| Graduation lane (`enqueue` / `fill` / `release` / `cancel`) | ON | Yes, via any `CurveFactory` curve (permissionless launch) |
| `itemArrived`, `snapshotCap`, `reclaimStray` | ON | Custodian (the canister). `snapshotCap` is anyone's |
| `fundReserve` / `fundLiquidity` | ON | Anyone. Fed by curves' fee slices and by `ProtocolTreasury.distribute` (90% to reserves in Phase 1) |
| `Wrapper.wrapDeposit` | ON | Custodian only (conversions to the module; holder deposits are an operator-procedure OFF) |
| `Wrapper.redeem` / `redeemToBurn` / `resolve` / `refund` / `recordDelivery` | ON | Anyone, except `recordDelivery` (keeper **or** custodian). There is no pause |
| `Wrapper.redeemToRelist` | OFF by canister policy | **Reachable on chain.** The ticket refunds after 7 days (documented) |
| `fundFloat`, `itemSold`, `bootstrap` | OFF | `fundFloat` is treasury-only and floats are 0%. `itemSold` needs `floatUnits` (0). The bootstrap loan is 0 |
| Rentals (`rentIn`, `endRental`, `settleRental`, `payDebt`, `claim`) | OFF | `rentIn` is custodian-only and needs a `rentalRoot` the K3 keeper never posts. The canister has no path |
| `Wrapper.wrap` / `WrapClaim` open/claim / `Distributor` | OFF | No `AutoBuyerVault` acquisition can exist (`open_purchase` is refused on 4663), so `wrap` and every `Distributor` lot revert |
| `AutoBuyerVault.deposit` | ON (inflow only) | Pons launches deposit fees. Nothing takes them out (documented, phase1-scope §1) |
| `AssetVault` | OFF | Only created by `listInscriptions` / `listRune`, which Phase 1 does not call |

**Signed messages.** The EVM contracts in scope verify **no signatures** (no `ecrecover`/EIP-712 anywhere in these files). Every
canister action is a transaction from the canister's tECDSA EOA, gated by `msg.sender == custodian`/`buyer`. Replay and domain
separation therefore rest on the EOA nonce plus EIP-1559 `chainId`. `autobuyer/canister-rs/src/evmtx.rs:178-205` signs
`0x02 ‖ rlp([chainId, …])`, with chain id 4663 from the install argument. There is nothing further to replay on this side.

## 3. Invariants and trust assumptions

**Invariants the contracts claim**, checked or attacked:

- **W1.** `totalSupply == (held − openTickets) × 1e18 + ethBacked`. **Holds.** Checked in every PoC via `_books()`.
- **W3.** A redemption needs an unpromised item: `held ≥ openTickets + openReturns + 1` (`Wrapper.sol:449`). **Holds.** ETH-backed
  wX can redeem a frog, since the token is fungible. This is intended.
- **Minting.** Only the custodian (`wrapDeposit`) and the one wired module (`moduleMint`) can mint. `wrap` is unreachable.
  **Holds.** Nobody but the Safe (through `WrapClaim`) can wire the module, and nothing can mint before it is wired, so the
  wiring cannot be front-run.
- **M2 (solvency).** `balance ≥ backing + reserve + revenue + gradEth + rentalEscrow + renterPool + owedTotal`. **Holds.** Every
  pot movement was traced (mint, burn in both tiers, itemArrived, enqueue/fill/cancel, fund*, skim, rentals). Force-fed ETH is
  ignored safely: there is no `receive`, and the accounting uses `≥`.
- **M4 / M10.** Burns never pay more than the drawn tier's backing per unit, and rounding always favours the pot (`units` floor,
  `mintPrice` ceil, `released` floor). **Holds.** It is fuzzed in `testFuzz_B_roundTripNeverProfitsWithSurplus`.
- **Liveness (implicit, not stated as an invariant anywhere).** While the floor is fresh, a holder of ETH-backed wX can burn.
  **Broken by B-01.**
- **The keeper bound (floor-module.md §6.3, §12; `FloorModule.sol:38-39`).** "A compromised keeper costs at most
  `maxPublicEthBacked × F × drop` … nothing through burns." **Broken by B-02** once the reserve holds fee or treasury surplus,
  which is exactly Phase 1's configuration.

**Trust assumptions:**

- The **custodian** (canister) is fully trusted on this side: `wrapDeposit` mints against its word, and `itemArrived` pays it up
  to `itemCap` from backing plus revenue.
- The **keeper** is trusted for the floor, confidence and depth, within ±5%/h. It is a **hot EOA** (K3 on Railway) and is
  **immutable in the module** (`FloorModule.sol:112`).
- The **Safe** (registry owner) is trusted for listing pause, roles, `WrapClaim` wiring and the treasury shares.

## 4. Findings

| ID | Severity | Title | Location | Phase-1 reachable? | PoC |
| --- | --- | --- | --- | --- | --- |
| B-01 | **Medium** | A 4-wei mint (or an odd-sized burn) leaves a dust reserve tier that makes every module burn revert | `FloorModule.sol:401-405, 409-427, 431-434` | Yes | `test_B01_dustMintFreezesEveryBurn`, `test_B01b_griefIsReArmableAfterEveryDrain`: **PASS (exploit reproduced)** |
| B-02 | **Medium** | A compromised keeper drains the reserve's fee/treasury surplus through burns, and nothing on chain can stop it | `FloorModule.sol:330-350, 898-903, 912-917, 748-753`; claim at `:38-39` | Yes (needs keeper key) | `test_B02_compromisedKeeperDrainsTheReserveSurplusThroughBurns`: **PASS** (1 unit burned for 1.314 ETH against a 0.640 ETH basis) |
| B-03 | **Medium** | The keeper can record a fake delivery for any resolved ticket: the redeemer loses the token and the refund, and the frog leaves the books | `Wrapper.sol:524-543, 548-551` | Yes (needs keeper key) | `test_B03_keeperForgesADelivery_redeemerLosesTheTokenAndTheRefund`: **PASS** |
| B-04 | **Medium** | The keeper is immutable and the module has no owner or escape: if the keeper is lost, ETH-backed backing and the reserve are locked for good | `FloorModule.sol:112, 373, 410, 830-833` | Yes | `test_B04_keeperLossLocksTheBackingForever`: **PASS** |
| B-05 | Low | A skipped `wireFloorModule` is only a "note" in the go-live check; once the keeper posts, a graduating buy then reverts | `Wrapper.sol:254-261`, `golive-check.sh:230-235`, `FloorModule.sol:588-595` | Setup ordering | analysis |
| B-06 | Low | `itemArrived` checks the module's absolute wX balance, not a per-deposit credit | `FloorModule.sol:465` | Custodian-only | analysis |
| B-07 | Low | Public mints (~13 units, fully recoverable) can hold the ETH-backed room and push graduations into their 3-day refund | `FloorModule.sol:379, 588` | Yes | analysis |
| B-08 | Low | A race lets burners take `C` below one unit between the canister's `wrapDeposit` and `itemArrived`: the canister is then paid in a stray wX, not ETH | `FloorModule.sol:466-467, 548-554` | Yes | analysis |
| B-09 | Info | The module ignores the registry's listing pause, so the owner has no circuit breaker for mint, burn or fill | `FloorModule.sol` (no pause) | — | shown inside B-02 |
| B-10 | Info | Doc/natspec drift: `itemArrived` says "from the reserve", the code uses `revenue`; floor-module.md §5's interface is stale | `FloorModule.sol:457-458`; `floor-module.md` §5 | — | — |
| B-11 | Info | `volume7d` is gameable by mint/burn churn (matters only with more than one collection) | `FloorModule.sol:509-514, 798-803` | No (one collection) | — |
| B-12 | Info | `snapshotCap` is anyone's and extends the custodian's allowance 2 h past staleness | `FloorModule.sol:517-524, 980-984` | Yes | — |
| B-13 | Info | OFF paths confirmed unreachable; Pons fees in `AutoBuyerVault` have no exit (documented) | `Wrapper.sol:369-390`, `AutoBuyerVault.sol` | No | — |

**Counts:** Critical 0, High 0, Medium 4, Low 4, Informational 5.

**What I tried to break and could not.** I found no way to:

- mint wX without an item arriving (except through the custodian or keeper trust);
- redeem more items than are held;
- make the module insolvent;
- get a round-trip profit in a calm market (fuzzed);
- sandwich or flash-loan-move the module price. Its price never reads pool state, and the fills price can only widen spreads.
- exploit reentrancy. It is all `nonReentrant`, and ETH is sent last.
- exploit the Arbitrum clocks. Both-clock `Wait` and `_slowElapsed`/`_fastElapsed` are correct. `block.number` is the parent
  (Ethereum) number: I read `l1BlockNumber` 26,058,720 at L2 head 72,736,867.

---

### B-01 (Medium): a dust reserve tier freezes every module burn

**Description.** Burns draw the reserve tier first, and a burn may take **at most `reserveUnits`** while it is non-zero
(`_burnTier`, `:431-434`; `burn` reverts `NotEnoughBacked` above it, `:413`). A burn must also pay at least 1 wei
(`eth == 0 → Slippage`, `:420`). `_split` (`:401-405`) gives a mint a reserve share as soon as both `eth*2500/1e4` and
`units*2500/1e4` are ≥ 1. So a **4-wei mint** at a 0.63 ETH floor gives 6 units, of which 1 wei is a reserve unit. When the
reserve tier is empty, that sets `reserveUnits = 1`. From then on:

- `burn(u > 1)` reverts with `NotEnoughBacked`;
- `burn(1)` pays `⌊1 × 0.63e18 / 1e18⌋ = 0` wei and reverts with `Slippage`;
- `burnCapacity()` reports room = 1 wei, and `EtchRouter._wrappedToEth` (`EtchRouter.sol:217`) skips the module because
  `mulDiv(1, price, 1e18) == 0`. Every router sale goes to the pool.

The state persists until somebody **mints** a real amount. The grief costs 4 wei plus gas. It can be re-armed every time a burn
empties the reserve tier, which is exactly what a bank run does. The same state also arises **with no attacker**: a holder who
burns `reserveUnits − 1` leaves a 1-wei tier behind.

**Exploit scenario.** The frog floor dumps and holders queue to exit at the module (5 units an hour). Each time the reserve tier
is emptied, Mallory back-runs with `mint{value: 4}`. The module's ETH exit, the thing ETH-backed wX exists for, is closed. Holders
are pushed into thin pools, while up to `maxEthBacked` units' worth of backing (≈ 25 ETH) sits idle. Only sophisticated users with
a helper contract can get through, by minting about 8 wei, burning the 3-wei tier and then burning the convertible tier in one
transaction. Mints also need room under `publicCap` and a fresh floor.

**PoC.** `test/audit/B/FloorAuditB.t.sol::test_B01_dustMintFreezesEveryBurn` (PASS). After the grief, Alice's 3 ETH-backed units
(1.90 ETH of backing) cannot be burned, and 24 hours of fresh posts later the module is still frozen.
`test_B01b_griefIsReArmableAfterEveryDrain` (PASS) shows the grief being re-armed after an honest mint thaws it.

**Recommendation.** Make a burn span the tiers: take `min(units, R)` from the reserve tier at `β_S` and the rest from the
convertible tier at `β_B` in the same call. Alternatively, treat `R` below a dust threshold (for example `< 1e15`) as zero by
folding it into `C`, and make `_split` give no reserve share below a minimum number of units. Add a liveness invariant: while
fresh and under the hourly cap, a holder can always burn `min(balance, E, cap)` units.

---

### B-02 (Medium): a compromised keeper drains the reserve surplus through burns, and nothing on chain can stop it

**Description.** Floor-module §13 routes three things into the standing reserve `S` **without units**:

- the curves' reserve slice (`reserveShareBps = 500`);
- `ProtocolTreasury.distribute`'s reserves pot, which is **90% of all protocol income** in Phase 1 (`setShares(9000,0,0,1000)`);
- `itemSold` proceeds.

`β_S = S / R` therefore rises well above what the reserve units paid in. The reserve-tier burn price is
`min(price(), target) × (1 − s)`, capped at `β_S` (`:898-903`, `:912-917`), so once `β_S` is large, **only the keeper's floor
bounds the payout**.

A keeper that walks the accepted floor up 5% an hour, re-posting flat once for an hour so the volatility term decays, lets anyone
burn the reserve units at the inflated floor. This pays out the surplus.

The code (`:38-39`) and the docs (floor-module.md §6.3 "Push-up drains nothing (β cap)", §12 "nothing through burns") state the
opposite. The existing test `test_compromisedKeeper_cannotDrainThroughBurns_evenOverADay` runs with `reserveBps = 0` and no
surplus. `test_pushUpThenBurn_theReservePaysAtMostWhatItHoldsPerUnit` has no `fundReserve`. Neither covers the Phase 1
configuration.

**Nothing can stop it once the key is stolen:**

- the module has **no owner, no pause, and an immutable keeper** (`:112`);
- it does not read the registry pause (the PoC pauses the listing and the drain continues);
- rotating the registry keeper does not rotate the module's keeper.

"A watcher bounds `h`" (§6.3) is false: noticing does not stop anything.

The same compromised key can also:

- walk the floor down, mint up to `publicCap` cheaply, and **redeem the held frogs** with that cheap wX (stage 0: ≤ 3);
- dilute `β` of honest ETH-backed holders, then walk the floor back up and burn at the diluted `β`;
- make graduations cheap relative to existing holders.

**Exploit scenario.** In the PoC:

1. The reserve holds 1 unit's basis (0.64 ETH) plus 3 ETH of fees.
2. Mallory holds pool-bought wX at F = 0.633.
3. The stolen keeper posts `F×100` hourly for 15 hours. The accepted floor passes 2F, and the owner's `setPaused` does nothing.
4. Mallory burns the reserve unit for **1.314 ETH**, more than 2× the unit's paid-in 0.640 ETH. The reserve falls from 3.63 to
   2.32 ETH.

Continued over more hours (5 units/h, while `R > 0` and `β_S` covers it), the whole surplus goes. Honest burners of reserve
units also take it, at the expense of the protocol's reserve.

**PoC.** `test_B02_compromisedKeeperDrainsTheReserveSurplusThroughBurns` (PASS). The numbers above are from its logs.

**Recommendation.**

- Keep the reserve's **basis** (`Σ rEth` of its units) apart from its **surplus** (`fundReserve` and `itemSold`). Cap
  reserve-tier burns at `basis / R`, the paid-in average, and spend the surplus only on conversions or top-ups that do not
  depend on the keeper's floor.
- Give the keeper a **rotatable forwarder** owned by the timelock or Safe, as floor-module.md §5 itself suggests ("a small
  forwarder contract if it must rotate"). Give it a guardian **pause for mint and fill**. Burns at `≤ β_basis` can stay
  unpausable.
- Correct `:38-39`, §6.3 and §12, and state the real bound on the Trust page.

---

### B-03 (Medium): the keeper can forge a delivery and void any redemption

**Description.** `Wrapper.recordDelivery` accepts `registry.keeper()` **or** the custodian (`:525`) and needs only a non-zero
txid. It moves a `Resolved` ticket to `Delivered`, decrements `held` and `openTickets`, and clears `inscriptionHeld` (`:529-533`).

`refund` accepts only `Resolved` tickets (`:550`), so once a delivery is recorded the redeemer can never be refunded. The canister
checks the ticket immediately before sending, sees `Delivered`, and does not send. The frog stays at the custody address but is
off the books. It cannot be re-wrapped either: its outpoint is already `depositMinted`.

In Phase 1 the canister records its own deliveries (`redeem.rs:41`, `frogflow.rs:55`), and the production keeper sender
(`keeper/service`) never calls `recordDelivery`. The keeper's power is **unused, but live on a hot key**. The registry keeper
rotates only after 7 days (`ListingRegistry.proposeRoles`), and the attacker keeps the power for that whole week.

**Exploit scenario.** An attacker holding the K3 key watches `Resolved` events and calls `recordDelivery(id, junk)` on every one.
Each redeemer loses a frog's worth of wX (≈ 0.63 ETH), and redemption, the path that "cannot be switched off", becomes useless for
at least the week the rotation takes. The K6 canary does not fire, because nothing left custody. K7's reconciliation would show
custody holding more frogs than `held`.

**PoC.** `test_B03_keeperForgesADelivery_redeemerLosesTheTokenAndTheRefund` (PASS). After the fake record, `refund` reverts
`WrongStage` 8 days later. Alice has neither the frog nor her token, and `held == 0`.

**Recommendation.** For this wrapper, which is not deployed yet, allow only the **custodian** in `Wrapper.recordDelivery`: the
party that actually signs the Bitcoin spend. If the keeper must keep the power, add a dispute window, or require the custodian to
co-sign within the window before the ticket leaves `Resolved`.

---

### B-04 (Medium): a lost keeper locks the backing and the reserve permanently

**Description.** `mint` and `burn` (`:373`, `:410`) need `fresh()` (`:830-833`): a post less than an hour old. `itemArrived`
needs a fresh `itemCap` or a 2-hour snapshot. The keeper is **immutable** (`:112`), the module has **no owner**, and **no stale
path releases ETH**. The ETH could come out only through `skim` (revenue beyond 1 ETH) or `cancel` (queued graduation budgets).

If the keeper key is lost, or its sender stops for good (a Railway account lost, the UniSat key revoked so "no frog floor is
posted", phase1-scope §9), then:

- `backing + reserve` (up to ~40 items' worth, ≈ 25 ETH per phase1-scope §0) is locked for good;
- ETH-backed wX is backed by ETH nobody can reach, and can only be sold into the pools or used to redeem the ≤ 3 held frogs.

phase1-scope §1 says holders "have only the pools". It does not say the backing is unrecoverable.

**PoC.** `test_B04_keeperLossLocksTheBackingForever` (PASS). A year after the last post, `burn` reverts `Stale`, `itemArrived`
reverts `CapStale`, and every wei is still in the module.

**Recommendation.** Add a stale-escape. After, say, 7 days stale on both clocks, allow `burn` at the drawn tier's `β` (backing per
unit) with the base spread only and no floor price. This keeps M4, M6 and M10 by construction, and needs no key. Combined with
B-02's rotatable forwarder, a keeper loss becomes a liveness incident, not a loss.

---

### B-05 (Low): setup ordering, and the unwired module is only a note

`Wrapper.wireFloorModule` (`:254-261`) checks only that the target has code; it does not check that `module.wrapper() == this`.
`golive-check.sh:230-233` treats an unwired wrapper as a **note**. Phase-1 go-live step 8 runs after step 7 and is run again after
step 9, but the check never fails.

If step 7 is skipped and the keeper starts at step 11:

- `FloorModule.mint` reverts with `NotModule` (harmless);
- `_tryFill` → `moduleMint` reverts, so `enqueue` reverts and **the curve's graduating buy reverts**, and the curve cannot
  graduate.

`initMintLimit` becomes impossible after the first mint. Without it the default is 20 a day. `proposeMintLimit` can still lower it
at once, so this is not critical.

**Nothing can front-run** either call: only the custodian or the module can mint before wiring, and both calls are the Safe's
through `WrapClaim`.

**Recommendation.** Make the unwired or wrong module a **failure** in `golive-check.sh` once step 7 is done (a flag). In
`Wrapper.wireFloorModule`, check `IFloorModuleLike(module).wrapped() == address(this)`.

### B-06 (Low): `itemArrived` is not bound to a specific arrival

`:465` requires `balanceOf(module) ≥ gradUnits + floatUnits + ONE`, an **absolute** balance. Any stray or donated wX (including a
previous failed conversion's unit that was not reclaimed) satisfies it. A retrying or buggy canister that re-sends `itemArrived`
after the first one landed would be paid `cost` a second time with no new frog. The custodian is trusted, and the canister tracks
`itemsArrived()` (`floorsink.rs`), so this is Low.

**Recommendation.** Have `Wrapper.wrapDeposit(to == floorModule)` bump a `pendingArrivals` counter on the module (or wrapper) that
`itemArrived` consumes.

### B-07 (Low): graduation griefing through the ETH-backed room

Public mints count only convertible units against `publicCap` (10). Anyone can hold ≈ 13.3 ETH-backed units: 10 convertible plus
3.3 reserve. This costs ≈ 8.4 ETH of capital and is fully recoverable at ~0.3% or more in spreads through burns. It cuts
graduation room under `maxEthBacked = 40` by a third. At stage 0, conversions stop at 3 frogs in custody, so the room also fills
naturally. Later graduations then sit `Queued` and refund after the curve's 3-day window (`FrogCurve.sol:347`). This is a DoS of
the Phase-1 "ON" graduation lane at modest cost.

**Recommendation.** Reserve room for graduations: public units may not push `E` past `maxEthBacked − graduationReserve`. Or count
public reserve units against `publicCap`.

### B-08 (Low): the canister can be paid in wX instead of ETH

The canister wraps a bought frog to the module (`wrapDeposit`), then calls `itemArrived` in a separate transaction. If burners
take `C = E − R` below one unit in between, which is legal (5 units/h), `itemArrived` reverts `NotEnoughBacked` (`:467`). The
canister can only `reclaimStray` the wX (`:548-554`). Its BTC float is then converted into wX rather than ETH. The ledger stays
correct, but the canister's float drifts, and a small, repeated griefing lever exists. I did not verify the canister's handling
of this revert.

**Recommendation.** Have `snapshotCap` (called before buying) also **reserve** one convertible unit for the pending arrival, so
burns skip it for `SNAPSHOT_TTL`.

### B-09 to B-13 (Informational)

- **B-09.** The module ignores `registry.isPaused`. The owner has no circuit breaker for mint, fill or the keeper's floor. See
  B-02.
- **B-10.** `itemArrived`'s natspec (`:457-458`) says "from the reserve beyond it", but the code draws on `revenue`
  (`:474-476`). floor-module.md §5's `postFloor(uint256)` and `floor()` signatures are stale. `FloorModule.sol:38-39` is wrong
  (B-02).
- **B-11.** `volume7d` weights the treasury's distribution. A 30 bps round trip inflates it, which matters only once there are
  two or more collections.
- **B-12.** `snapshotCap()` is permissionless and keeps `max(itemCap, snapCap)` for 2 h, even after the floor goes stale. It only
  widens the trusted custodian's allowance.
- **B-13.** The OFF paths were confirmed unreachable on chain (§2). Fees in `AutoBuyerVault` from permissionless Pons launches
  have no exit until a v2 policy exists (documented).
  - `redeemToRelist` is callable. Its ticket refunds after 7 days.
  - `AssetVault`, `Distributor` and `WrapClaim` are inert.
  - `ListingRegistry.setPaused` is instant (by design) and does not stop redemptions.

## 5. Slither triage

I ran Slither 0.11.6 on this checkout: `slither . --filter-paths "lib|test|script"`, output in
`scratchpad/rook-audit/B-slither.{json,txt}`. It returned 457 results project-wide, the same as reviewer C's same-commit run (`slither-all.json`). Filtered to my files, excluding NftWrapper:

| Detector | Count | Verdict |
| --- | ---: | --- |
| weak-prng (High) | 1 | **False positive.** `day % DAYS` in `_volume` is a ring-buffer index, not randomness. The real randomness is drand via `beacon.prove`, with a negligible modulo bias over ≤ `held` items |
| uninitialized-state (High) | 2 | **False positive.** `_charges` in `AutoBuyerVault` and `AssetVault` is a mapping written by `push` |
| divide-before-multiply (Medium) | 1 | **Benign.** `_revenue` rental accumulator rounding, capped by `renterPool`. Rentals are OFF |
| incorrect-equality (Medium) | 21 | **False positive.** Zero and status checks on internal accounting. None compares a manipulable balance for equality |
| reentrancy-no-eth (Medium) | 3 | **Not exploitable.** `rentIn` → `_w.admitRental` (own wrapper, `nonReentrant`, OFF). `Wrapper.resolve` and `Distributor.resolve` → `beacon.prove` (an immutable drand verifier; state is re-read correctly and ordering is enforced) |
| uninitialized-local (Medium) | 15 | **False positive.** Zero-initialised accumulators |
| missing-zero-check (Low) | 1 | **Intended.** `treasury_ == 0` means "no treasury" |
| reentrancy-benign / reentrancy-events (Low) | 6 | **Benign.** The treasury is trusted and the beacon immutable. `fundReserve` is `nonReentrant` |
| timestamp (Low) | many | **Reviewed.** Every wait uses both clocks (`Wait.elapsed`, `_slowElapsed`/`_fastElapsed`). Correct for Nitro's ±1 h sequencer timestamp |
| shadowing, naming, cyclomatic, low-level-calls | — | Style. `_send` checks success |

**No Slither result in scope is a real vulnerability.** None of the four Medium findings above is detected by Slither: they are
economic or design issues.

## 6. Test-coverage gaps

1. **The attack suites run the wrong configuration.** `FloorBase` runs `reserveBps = 0` and `maxItemsPerDay = 40`, while the
   deployed defaults (`StackChecks.defaultParams`) are 2,500 and 3.
   - The compromised-keeper tests (`FloorAttacks.t.sol:207`) never have a **fee-funded reserve surplus**, which is the Phase 1
     reality with the treasury at 90% reserves. This is why B-02 was missed.
   - Add a suite on the exact deployed params, with `fundReserve` and treasury `distribute` in the loop.
2. **No liveness invariant.** `FloorModule.invariant.t.sol` checks solvency, band, caps and ledger, but never "a holder can burn
   while fresh". B-01 would fall out of a handler that adds a tiny `mint` action and asserts burnability.
3. **The keeper-can-record-delivery test asserts the power exists** (`Wrapper.t.sol:215`). No test asks whether a
   keeper-recorded delivery is correct, or checks what happens to the frog afterwards (B-03).
4. **No test of permanent staleness** (B-04): what exits exist after the keeper stops.
5. **No tests of `itemArrived`/`wrapDeposit` interleavings** with burns in between (B-08), or of a second `itemArrived` against a
   stray unit (B-06).
6. **No end-to-end test of the setup order.** Nothing covers the owner steps skipped or reordered (wiring after the keeper's
   first post, or a graduation before wiring) (B-05).

## 7. What I did NOT cover

- **Canister Rust** beyond two greps: `evmtx.rs` chain id, and whether the keeper service calls `recordDelivery`. The canister's
  handling of `itemArrived` reverts, its idempotency, and its reading of ticket state are assumed from the docs.
- **The keeper service's floor computation** (two-provider agreement) and its gas and key handling.
- **`FrogCurve`, `CurveFactory`, `LiquidityLock` and `EtchRouter`**, beyond their calls into the module; they belong to other
  areas. The router's reserve-tier-only split (`toModule = min(left, room)`) sends the rest of a sale to the pool even when the
  convertible tier would take it. Worth a look by the router reviewer.
- **`ProtocolTreasury`**, beyond how it funds the module.
- **`AssetVault` line by line** (v1; unreachable in Phase 1).
- **Rentals in depth.** `rentIn`, `settleRental` and debt were checked for books consistency only. They are OFF in Phase 1.
- **The NFT wrapper, home-chain contracts and `SweepVault`.**
- **Deep fuzzing and invariant runs** (`FOUNDRY_PROFILE=deep`) and the full `forge test` suite. Only my own PoC file was run: 6/6
  pass, the fuzz at 512 runs.
