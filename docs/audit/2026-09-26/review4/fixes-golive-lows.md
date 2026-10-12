# Go-live Lows: the answers to the round-4 targeted check (R4-check)

Branch `golive-lows` (from `main`, head `f34d153`, the R4 check). Written before the code, as the design; the "As built" section
at the end is filled in when the code lands. Nothing here is deployed, and no key, wallet or live chain is touched. Network use is
read-only: the public 4663 RPC (`cast code`, a forge fork for the Safe regression, the anvil fork of the dry run and the keeper fork
test) and the public drand relays.

Inputs: `review4/R4-check.md`, `review4/part-canister.md`, `review4/part-services.md` and their PoCs in `review4/poc/`. Each PoC
that showed a gap is turned into a regression test that now fails the attack (a test that asserted "the gap is there" now asserts
the opposite, or is replaced by one that drives the fixed code with the same world).

## 0. Summary

| ID | Sev | Fix | Where |
| --- | --- | --- | --- |
| **A4-L1** | Low | The pause floor is capped at the fill band's low edge, `accepted × (1 − fillBandBps)`, and at the least pre-spread price any mint paid in the trailing 12–24 h before the last post's freshness ran out. A mint under an earlier post of the same floor, or before a rise of `accepted`, then a pause, then a burn never profits | `FloorModule._pauseFloor`, `mint` |
| **R4-C1** | Low | "Our conflict is in both explorers' mempools" suppresses the ×1.5 re-pricing only for the first 24 h after it was first seen there; after that the conflict escalates as in round 3 | `conflict.rs` (`Conflict.held_since`, `step`, `reprice`) |
| **L-S1** | Low | The owner Safe must have been created by a canonical SafeProxyFactory (`ProxyCreation` naming it, from a factory whose code is canonical) in the **same transaction** as its one `SafeSetup`. The SafeSetup's owners and threshold are compared with `getOwners()` / `getThreshold()` and with `--safe-owners` | `script/golive-check.sh` `safecheck` |
| **L-S2** | Low | The alert latch is never shorter than `(KEEPER_ALERT_PASSES + 1) × interval + 10 min + 2 min`, for the keeper and the resolver alike | `keeper/service/status.ts`, `config.ts` |
| **L-S3** | Low | In the dense-block fallback a null or malformed header, an empty or inconsistent receipts answer, or no log of the filter's addresses in a block the provider just refused for size is an error, never an empty block. Any receipts failure (a 5xx or a timeout included) falls through to the per-address split, then the per-transaction receipts; an address that no way can read is `stuck` at that block (loud, retried every poll), and the rest of the group goes on | `monitor/src/holders.mjs` `denseBlock` |
| **L-S4** | Low | Reconciliation runs after the draw deadline and the guardian, with its own bounded budget; every state read's timeout is the budget left, with one try, and the budget is checked before each read | `monitor/src/holders.mjs` `reconcile`, `abi.mjs` `read`, `evm.mjs` |
| **A4-I1** | Info | No clean on-chain check exists (below); documented: the monitor's recompute is the guard, and a regression shows it catches the PoC's root | docs, `monitor/test` |
| **A4-I3** | Info | The indexer reports `keeperPaused` (the registry's `keeperPaused()`, or no keeper named); Books never says "fresh, so the module trades" while the keeper is paused (or the module is stale) | `indexer/src/wrapped/floorModule.ts`, `web-rook/src/ui/Books.tsx` |

## 1. A4-L1: the pause floor across two posts

### 1.1 The gap

Round 4 set `F_p = min(burnTarget, max(a(1 − fb), min(target, a)))`. That is a lower bound on what a mint paid under the **last**
post only. A mint made while the target sat below `a` and the fills sat at the band's low edge cost `a(1 − fb)(1 + sMint)`; one
minute later the keeper posts a target back at `a` (accepted unchanged), `F_p` becomes `a`, and a pause pays `a(1 − base)`. The
PoC makes +1.1 % (reserve 0) and +1.4 % (reserve 25 %).

The same shape exists when `accepted` itself rises between the mint and the pause: the paused price follows the new floor while
the mint paid the old one, and the pause adds the difference between the fresh burn spread and the base spread to what the market
move gave.

### 1.2 The fix

    F_p = min( burnTarget,  accepted × (1 − fillBandBps),  lowMint )

- `accepted × (1 − fillBandBps)` is the band's low edge. `price()` never sits below it and `mintPrice() ≥ price() × (1 + sMint)`, so
  no mint under **any** post of the same accepted floor paid less than it, spread aside. This is the reviewer's one-line fix; it
  costs an honest paused burner at most `fillBandBps` (3 %) against round 4, and only during a pause.
- `lowMint` is the least pre-spread price (`max(price(), _upTarget())`) any public mint paid, kept per 12-hour bucket of
  `block.timestamp` (two buckets, the current and the one before, each `price << 64 | bucket`). While the keeper is down it is read
  at the buckets of `postedAt + FLOOR_TTL`, the last moment a mint could have happened (a mint needs `fresh()`), so every mint of the
  last 12 h (up to 24 h) before the pause is covered, whatever posts came between it and the pause, and the value is frozen for the
  whole pause (no mint, no post). No mint in those buckets: no cap from it.
- `F_p` stays positive (every term is, once a floor was posted) and still only falls during a pause (`accepted`, `target`,
  `postedAt` and the buckets are frozen; `burnTarget`'s day anchor only lowers it).
- What is left, by design: a unit minted more than the lookback before the pause, before a real rise of the floor, gets the
  floor's rise less the base spread; a fresh burn after the rise gets the same rise less the burn spread. That is the market, not
  the pause, and it needs the market to rise.

The old claims "`F_p` is at least the next hour's legal floor" (N-1a) and "`pauseFloor() == accepted` with an ordinary keeper" become
"`F_p ≥ min(lo, a(1 − fb), lowMint) > 0`" and "`pauseFloor() == a(1 − fb)`, or less after a cheaper mint"; the tests that asserted
the old values are updated to the new ones.

### 1.3 Tests

- `test/audit4/Audit4Floor.t.sol` (both parameter sets): the PoC's target swing between the mint and the pause now **loses**; the
  control stays a loss; a rise of `accepted` between the mint and the pause loses; the pause floor is frozen through a long pause.
- `testFuzz_audit3_aPauseNeverPaysAMintBurnRoundTrip` gets an optional keeper post (any target from half to twice the floor, any
  confidence) between the mint and the pause.
- The invariant handler's `roundTrip` gets the same optional post between the mint and the pause.

## 2. R4-C1: the "held in a mempool" rule is bounded in time

`Conflict` gains `held_since: Option<u64>` (serde default, candid `opt nat64`: an older record decodes with `None`): set the first
time the explorers show any version of our conflict in their mempools, never reset. `reprice` treats the conflict as `held` (today's
rate, no ×1.5) only while `now − held_since < HELD_TRUST_SECS` (24 h). After that a conflict at today's rate that no block took is
itself the evidence that the miners do not have it, and the ×1.5 escalation every `CONFLICT_REPRICE_SECS` towards
`MAX_CONFLICT_FEE_SATS` resumes.

24 h, not 12: a conflict at today's rate normally confirms in hours; a day of honest fee spikes should not cost the 100k-sat
ceiling, and 24 h is 1/30 of the refund race. `review3.rs`'s R3-C7 test (12.7 h held) keeps passing.

Regression: `tests/audit4_partition.rs` (the reviewer's world): with the partition, our conflict now out-pays the 20k-sat pin and
confirms before the refund, no LostRace; the control is unchanged. Plus a test that the first 24 h still only follow the market.

## 3. L-S1: the owner Safe's creation is read, not only its state

`safecheck` (under `--phase1`) now also:

1. takes the `transactionHash` of the Safe's one `SafeSetup` event and reads that transaction's receipt;
2. requires exactly one `ProxyCreation(address,address)` log in it (`0x4f51faf6…e235`) that names this Safe (topic 1 in 1.4.1, the
   first data word in 1.3.0), emitted by a canonical SafeProxyFactory (1.4.1 `0x4e1DCf7A…ec67`, 1.3.0 `0xa6B71E26…6AB2`, 1.3.0
   eip155 `0xC2283458…10BC`) whose code on this chain has the canonical hash (measured on 4663, 2026-09-26), and whose `singleton`
   is a canonical singleton. The factory creates the proxy with the canonical creation code (which writes only slot 0) and calls
   the initializer in the same call, so nothing can run on the proxy between its creation and `setup()`;
3. decodes the SafeSetup's owners and threshold and FAILs unless they equal `getOwners()` / `getThreshold()` on chain (a later
   owner change is a FAIL too: the go/no-go read expects the Safe as the ceremony made it), and, with `--safe-owners`, the
   ceremony's two devices; the threshold must be 2.

Anything unreadable (no receipt, no transaction hash) is a FAIL, as every other read.

Regressions: `scripts/audit2_golive_check.test.sh` gains the cases (the fake `cast` answers `receipt`): the planted Safe's shape
(SafeSetup in a transaction with no ProxyCreation), a ProxyCreation naming another proxy, from a non-canonical factory, from a
canonical address with other code, in another transaction, on a non-canonical singleton, SafeSetup owners or threshold unlike the
chain's, a threshold of 1; the correct Safe passes. `test/audit4/Audit4SafeHidden.t.sol` (opt-in `AUDIT4_FORK=1`, a read-only fork
of 4663): the planted Safe has no canonical `ProxyCreation` naming it, so the creation read FAILs it (and a decoy factory call in the
same transaction does not help), while a Safe made by the canonical 1.4.1 factory with `setup()` as its initializer passes and has
no hidden module.

## 4. L-S2: the alert latch scales with the interval

`alertLatchFloorMs(intervalS, passes) = ((passes + 1) × intervalS + 600 + 120) × 1000`: 503 starts at the `passes`-th pass in force
(`t0 + (passes − 1) × I`) and lasts until the first pass at or after `t0 + latch`, so it lasts at least `2I + 12 min`, longer than the
monitor's 10-minute `DOWN_MS` plus a poll. `config.ts` uses `max(KEEPER_ALERT_LATCH_S, floor)`; the resolver's config is the same
function (its interval is `RESOLVER_INTERVAL_S`). Regression: `keeper/test/audit4_services.test.ts` pages the one-pass alert at 60,
300, 600, 900 and 1,800 s with the latch the config computes.

## 5. L-S3: the dense-block fallback reads or says it could not

`denseBlock(p, filter, x)` is called only after a one-block `eth_getLogs` for `filter` was refused for its size, so the block has
logs of the filter's addresses. Therefore:

- `eth_getBlockReceipts` is accepted only if it is a non-empty array of receipts all for block `x` (`blockNumber`) with one
  `blockHash`, and it holds at least one log of the filter. Otherwise (an error of any kind, 5xx and timeouts included, or a
  malformed answer) the next way is tried.
- The per-address split: a query for a part that fails for any reason other than its size marks that part's addresses `stuck` at
  `x` (never the group); an answer that is not an array is a failure.
- The per-transaction receipts: a null header, one without a `transactions` array, one for another block, or an empty
  `transactions` list is a failure (the addresses stuck, never "an empty block"); so is a receipt that is missing or for another
  block, and so is a set of receipts with no log of the address.
- Only the poll's time budget still ends the read before the block for everyone (the next poll goes on).
- Why the stuck address could not be read is kept in its `stuck.error`, and the monitor pages it (SEV1 if a deciding draw needs it).

Regressions (`monitor/test/audit4_monitor.test.mjs`): R3's dense world (a header with no `transactions`) and a null header now
leave coin A stuck at the dense block with its balances unreadable, never wrong; a 504 on the receipts now lets the group and coin
B pass the block while only coin A stays behind; receipts that are `[]` or for another block are refused.

## 6. L-S4: reconciliation cannot delay the deadline pause

- `read()` takes `{ timeoutMs, tries }` through to `rpc()`.
- `reconcile({ deadline })` checks the deadline before every read, and gives each read `timeoutMs = min(30 s, deadline − now)`
  (at least 250 ms) and one try. A coin whose reads ran out of time is not marked reconciled (it is tried again next poll).
- `checkEvm` runs it after the deadline block, the no-good-post rule and the guardian, with its own budget
  (`draws.reconcile_budget_s`, default 15 s), and saves the indexes after it.

Regression: the reviewer's hanging reads now stop at the budget (a provider that never answers costs the budget, not 16 × 90 s),
and a `checkEvm` run whose reconciliation provider hangs sends the guardian's pause first.

## 7. A4-I1: sentinel plus one junk sibling

A clean on-chain check does not exist. The contract does not know the holder list, so any structural rule (a proof depth, a
first-ticket leaf `leaf(h, 0, e)` with `h ≠ 0` proven against the root) is satisfied by a keeper that builds a junk tree of that
shape (a junk holder address, or a leaf nobody can use). The keeper can publish any root in any case; a false root is caught only by
recomputing the holders' tree, which is the monitor's job: it reproduces the honest root on its providers, judges the draw `bad`,
and the guardian pauses the keeper and cancels the draw before the pin. `openDraw`'s empty-proof refusal (A3-I2) stays as the cheap
check of the one tree shape that needs no holder list. Documented in `docs/v5` next to A3-I2 and in the monitor's rule; regression
in `monitor/test/audit4_monitor.test.mjs`: the PoC's root (sentinel `leaf(0, 7, at)` plus one junk sibling) against the honest
indexes is `bad` with the pause and cancel triggers.

## 8. A4-I3: Books while the keeper is paused

The indexer reads the registry's `keeperPaused()` and `keeper()` at the report's block and reports `keeperPaused` (true when paused
or no keeper is named; null when unreadable). The site derives "keeper down" from it (falling back to `pauseFloorWei > 0`, which
the module returns only while the keeper is down and it has not escaped, then to the round-4 derivation), and only a module that is
fresh with its keeper up is described as trading. Regression: a web-rook test of the pure derivation, with the reviewer's case (fresh
floor, keeper paused within the hour).

## 9. Runs this branch must pass

forge (all, plus `AUDIT4_FORK=1` for the Safe regression), canister and timelock `cargo test`, keeper, monitor, indexer, web-rook,
ops and tools/snapshot suites and typechecks, `scripts/audit2_golive_check.test.sh`, then `scripts/golive-dryrun.sh stack` and
`scripts/keeper-fork-test.sh` one after the other; the canister's reproducible Docker build twice (its code changes here), with the
hashes recorded below.

## As built

Code and tests: `46afad1` (on `e3b46f2`, this design). Each regression was also run against the code before the fix and fails
there: the A4-L1 tests and the extended fuzz (the invariant, 64 runs, did not reach the fill-band shape on the old code; the direct
tests and the fuzz did), `audit4_partition` (`escalated` never reached), the L-S1 shell cases (the planted shape passed with exit
0), and the monitor's L-S3/L-S4 cases (all five fail; the old reconciliation times out).

### Contracts (§1, §7)

- `FloorModule`: `_pauseFloor = min(_burnTarget(), accepted × (1 − fillBandBps), mint lows)`; `mint` records `max(price(),
  _upTarget())` in `_mintLow[bucket % 2]` (`MINT_LOW_BUCKET` 12 h, a public constant); the buckets are read at `postedAt +
  FLOOR_TTL`. `mintPrice()` is unchanged in value (`_mintBase()` factored out). Runtime 28,633 bytes (+323; 4663's limit is
  98,304).
- `test/audit4/Audit4Floor.t.sol` (4 tests × 2 parameter sets): the PoC's swing now loses; the control; a +5 % rise of `accepted`
  between the mint and the pause loses; the pause floor is the band edge with no recent mint, the cheapest recent mint otherwise,
  and never rises in a 5-day pause.
- `testFuzz_audit3_aPauseNeverPaysAMintBurnRoundTrip` takes an optional post (target ½–2× the floor, any confidence) between the
  mint and the pause; `LivenessHandler.roundTrip` too (`postsBetween`), with a non-vacuity test.
- Tests that asserted `pauseFloor() == accepted` (R2Floor N-1b, Audit3Floor A3-M1) now assert the band's low edge.
- `SweepVault`: NatSpec only (A4-I1); `docs/snapshot-v1.md` says the same.

### Canister (§2)

- `conflict.rs`: `HELD_TRUST_SECS` = 24 h; `Conflict.held_since` (serde default; `.did` `held_since : opt nat64`) set at the first
  `Mempool` sighting of our conflict and never reset; `held` = seen in a mempool **and** within 24 h of `held_since`.
- `tests/audit4_partition.rs`: the reviewer's partition now escalates 26 h after the first sighting (24 h + one re-price period),
  reaches 25,407 sats, out-pays the 20k pin and confirms 36 h after the conflict was first sent; no LostRace. The first day held is
  still not re-priced (R3-C7 kept; `review3.rs`'s R3-C7 test passes unchanged). The control is unchanged.
- A unit test: a `Conflict` saved before the field decodes (candid and serde) with `None`.
- `examples/watch_fixtures.rs` compiles with the field; `monitor/fixtures/canister.json` is not regenerated (as in round 4).

### Services (§3–§6, §8)

- `script/golive-check.sh`: the creation read and the SafeSetup owner/threshold comparison (§3). `scripts/audit2_golive_check.test.sh`:
  **67 ok** (49 + 18 L-S1 cases). `test/audit4/Audit4SafeHidden.t.sol`: with `AUDIT4_FORK=1`, **2 pass** on a read-only 4663 fork —
  the planted Safe still passes every round-4 read, and the creation read FAILs it (also with a decoy canonical `ProxyCreation` in
  the same transaction); a Safe made by the canonical 1.4.1 factory with `setup()` as initializer passes and hides no module.
- `keeper/service/status.ts` `alertLatchFloorMs`, used by `config.ts` for both roles; `docs/deploy/services-v1.md` updated.
  `keeper/test/audit4_services.test.ts`: paged at 15, 60, 300, 600, 900 and 1,800 s, keeper and resolver; a latch set under the
  floor is raised.
- `monitor/src/holders.mjs` `denseBlock` (§5), `reconcile` (§6); `abi.mjs` `read` passes `timeoutMs`/`tries`; `evm.mjs` runs
  reconciliation after the guardian (`draws.reconcile_budget_s`, default 15). `monitor/test/audit4_monitor.test.mjs` (13: R3
  re-runs, L-S3 ×4, L-S4 ×2, two Info items kept as documented behaviour), `holders.test.mjs` +2 (A4-I1 through a whole poll: `bad`,
  pause, cancel; L-S4 through a whole poll: the guardian acts before any reconciliation read and the poll ends in its budget).
- Indexer: `keeperPaused` in the module read (`I_KEEPER_SOURCE` gains `keeperPaused()`/`keeper()`, held to `out/` in `r3.test.ts`);
  `indexer/test/audit4.test.ts`. The type is copied byte for byte to `web/` and `web-rook/`.
- web-rook: `src/rook/moduleState.ts` (`moduleStateOf`, `keeperDownOf`), used by `Books.tsx`; `test/audit4.test.ts`. The pause text
  names the new cap. The synced docs (`floor-module.md`, `launch-gates.md`) are re-copied.

### Runs (this machine, after `46afad1`)

| Suite | Result |
| --- | --- |
| `forge test` | **1064 passed, 0 failed** (123 suites; was 1053: +8 Audit4Floor, +2 Audit4SafeHidden returning early offline, +1 invariant non-vacuity) |
| `AUDIT4_FORK=1 forge test --match-contract Audit4SafeHidden` | **2 passed** (read-only 4663 fork) |
| canister `cargo test` | **493 passed**, 0 failed (489 + 1 unit + 3 `audit4_partition`) |
| timelock `cargo test` / `--features rehearsal` | **15 / 15** |
| keeper `npm test` / typecheck | **137 pass** / clean |
| monitor `npm test` | **137 pass** (122 + 13 + 2) |
| indexer `npm test` / typecheck | **133 pass** / clean |
| web-rook `npm test` / typecheck / build | **68 pass** / clean / ok |
| web `npm test` / typecheck | 118 pass / clean (the shared `types.ts`; no build, which rewrites `trust-model.md`, I-S5) |
| ops, staging, tools/snapshot | 31, 29, 19 pass; typechecks clean |
| `scripts/audit2_golive_check.test.sh` | **67 ok**, exit 0 |
| `script/deploy.test.sh` | 37 ok, exit 0 |
| `scripts/timelock-watch.test.mjs` | 4 pass, 1 skipped (no local replica) |
| `STACK_PORT=8913 scripts/golive-dryrun.sh stack` (public read-only fork, real drand) | **DRY RUN: ALL PASS**, 44 ok, exit 0 |
| `KEEPER_FORK_PORT=8921 scripts/keeper-fork-test.sh` | **KEEPER FORK TEST: ALL PASS**, 43 ok, exit 0 (step 9 skipped: no `dfx` on this machine) |

**Reproducible build** (`Dockerfile.build`, linux/amd64, the script's steps on a `git archive` export of `46afad1`), run twice, the
second with `docker build --no-cache`, **identical both times**:

```
fc7edc5111f6588fbcf22b64172a22bc293b02d70b4dff1dc19153f4d469bde5  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

The buyer is new (round 4 recorded `dead93f…`: `conflict.rs` changed); both timelocks are unchanged (`b23a190…`, `2145099…`). The
buyer's wasm is 5,203,480 bytes, 1,018,435 gzipped (`gzip -9 -n`; the proposal limit is 2,000,000). Nothing was installed.

### Left open

- A4-I2, R4-C2..C4, I-S1..I-S6 and the pre-existing `BtcView::Unsure` note (part-canister.md) are unchanged, as the check listed
  them. R4-C3 still needs a replica or mainnet run of the scanner.
- A4-L1: a unit minted more than 12–24 h before a pause, before a real rise of the floor, still exits the pause at the risen floor
  less the base spread (§1.2): the market's move, which a fresh burn after the rise also pays, less its burn spread.
- L-S3: how the real providers answer a multi-MB `eth_getBlockReceipts` was not measured; a coin whose dense block no way can read
  stays stuck (paged, SEV1 if a deciding draw needs it) until a provider can.
- L-S1: a Safe created any other way (a non-canonical factory, `setup()` in a later transaction) now FAILs the go/no-go read; the
  ceremony must create it with the canonical factory and `setup()` as the initializer (docs/v6/phase1-scope.md step 8).
- Not run: clippy, `cargo audit`, the keeper fork test's step 9 (needs `dfx`), a replica run.
