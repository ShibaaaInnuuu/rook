# Rook fix round 4: targeted security check (R4-check)

Reviewer: independent AI review, 2026-09-26. Checkout `/Users/jlohayon/Documents/Rook-rev4`, detached at `fixes-2026-09-26`
(head `cf6ecda`); diff `4a215f4..fixes-2026-09-26` (72 files, +3,771 / −290). `lib/` copied from `~/Documents/Rook/lib`, copied
`.git` renamed `.git.bak-copied`.

**Nothing existing was modified. Nothing was committed, pushed, deployed or signed. No key was used.** Network use was read-only:
the public 4663 RPC (a local anvil fork for the dry run and one forge fork for the Safe PoC) and the public drand relays.
Additions are untracked PoC files only:
- `test/audit4/Audit4Floor.t.sol` (2 tests × 2 param sets), `test/audit4/Audit4Draws.t.sol` (1), `test/audit4/Audit4SafeHidden.t.sol`
  (1, opt-in `AUDIT4_FORK=1`, otherwise returns at once);
- `autobuyer/canister-rs/tests/audit4_scanner.rs` (2), `autobuyer/canister-rs/tests/audit4_partition.rs` (2);
- `monitor/test/audit4_monitor.test.mjs` (11), `indexer/test/audit4_outbound.test.ts` (6), `keeper/test/audit4_services.test.ts` (3).

The round-3 PoCs, re-run unmodified against the new code, are kept out of the tree so the suites stay green:
`scratchpad/rook-audit/R4/orig-r3-contract-pocs/` and `R4/orig-r3-canister-pocs/` (with their run logs). Part reports:
`R4/part-canister.md`, `R4/part-services.md`. Logs: `R4/*.txt|log`.

## Verdict

**Ready to merge to main.** Every round-3 Medium (A3-M1, M-A, M-B, M-E) is fixed. The round-3 PoCs re-run unmodified now fail
on their attack assertions. Round 4 adds **no new Critical, High or Medium**.

Round 4 does add new Lows. None blocks a merge by round 3's standard, where only Mediums blocked. Four of them should be fixed
before go-live, because each contradicts a claim the fix docs make:
1. **A4-L1** (contracts): the pause floor is not a lower bound on a mint's cost across two posts of the same floor. A pause can
   still pay a mint → pause → burn round trip about 1 % (PoC). This is a one-line fix.
2. **L-S1** (golive-check): a Safe whose creation code plants a hidden module passes every "real Safe" read (fork PoC: the module
   drains the Safe).
3. **L-S3** (monitor): the dense-block fallback can silently skip a block's logs, or stall the whole group on a retryable
   receipts error.
4. **L-S4** (monitor): reconciliation ignores the poll deadline and runs before the deadline pause, so a hanging provider can delay
   the pause.

R4-C1 (canister, a mempool partition defeats R3-C7's re-price rule) and L-S2 (the keeper alert latch at long intervals) are
recommended too.

## Counts (new in round 4)

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| Contracts | 0 | 0 | 0 | 1 | 3 |
| Canister / timelock | 0 | 0 | 0 | 1 | 3 |
| Services (monitor, keeper, indexer, golive-check, site) | 0 | 0 | 0 | 4 | 7 |
| **Total** | **0** | **0** | **0** | **6** | **13** |

## 1. Round-3 items

| ID | Sev (R3) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **A3-M1** paused burns at the average backing | Medium | **Fixed** (residual **A4-L1**) | R3's `test_audit3_mintLowThenBurnAtBackingDuringAPause`, unmodified, now **fails**: `exitAtBacking()` is false while paused, and the burn pays `min(β, F_p)(1 − base)`. The +0.206 ETH is gone. `_tierPrices` checks escaped → keeper down → fresh → `Stale`; `F_p ≤ accepted` and `F_p > 0` (a 1-wei post cannot zero it); `F_p` only falls during a pause. The claim holds for mints under the **last** post, but not across two posts of the same accepted floor (A4-L1). A burn still never lowers the remaining holders' β (each tier releases exactly its β), so there is no holder loss beyond the documented dilution at mint. |
| **M-A** span ratchets to 1 block | Medium | **Fixed** | R3 PoC re-run: after 13 `header not found` errors the ceiling stays 10,000, and 600 new blocks cost 1 call (R3: 600). Only `rangeError` narrows; the ceiling regrows and expires after 10 min. |
| **M-B** one dense block stalls the group | Medium | **Fixed for R3's case** (residual **L-S3**) | The group and the unrelated coin pass the dense block. But in R3's own fake the dense coin's logs are **skipped**, not read (L-S3). |
| **M-E** `HTTP 101` hangs the indexer fetch | Medium | **Fixed** | R3 PoC re-run: rejects in 26 ms. Headers never sent, a 1 B/s body and endless `102` each reject at the 3 s deadline; 0 sockets left open; the process lives. |
| **A3-L1** refund race after an unpause | Low | **Fixed** | R3's `test_audit3_unpauseRace_refundBeatsFill`, unmodified, now **fails** (`the hold ends at the unpause`: `holdsRefunds()` stays true). The grace is stamped only by the owner's real unpause, runs 6 h on both clocks and is bounded by `_stuck()`. The guardian can pause but not unpause, so it cannot stretch holds past the escape hatch. Keeper zero without a pause is only reachable before the first roles are set (`proposeRoles` refuses 0). |
| **R3-C1** scanner crawls `hot` 1 block a page | Low | **Fixed** | R3 PoC fails: a 500-event block is read in 3 ticks (was 502/1,000/505). One dense block anywhere in a page takes 3–4 ticks, 20 in a row take 11 and 10 spread out take 30, each within the 12-outcall bound. |
| **R3-C2** one provider skips a ticket | Low | **Fixed** | R3 PoC fails: a provider's own text is `Failed`, not a size error (`too_big` = `starts_with(SIZE_REJECT)`, added only on the IC's own reject). `Split` stalls loudly at a one-ticket query and never sets anything aside. `unreadable` is retried every tick. |
| **L-C** floor-walk evasion | Low | **Fixed** (residual I-S1) | R3's interleaved walk pages the edge run and triggers `pause_on_floor_walk`; the −0.44 %/h walk pages SEV1 on the 72 h rule. |
| **L-F** index never self-repairs | Low | **Fixed** | The lossy index is rebuilt on the dissent and reproduces the honest root; reconciliation and `resetAt`/`brokenAt` expiry work. Outsiders cannot trigger rebuilds (every trigger needs a dissent or a state mismatch), and rebuilds are rate-limited per coin (at most once an hour). |
| **L-D** resolver caches a mismatch for good | Low | **Fixed** | `{entry, retryAt, tries}`, 10 min → 6 h backoff, cached logs dropped before the retry; the audit2 test now really mismatches. |
| **L-G** one-pass keeper alerts never paged | Low | **Fixed at the default interval only** (L-S2) | Paged at `KEEPER_INTERVAL_S` 60; not at 300; never 503 at ≥ 600. |
| **L-H** golive-check "real Safe" | Low | **Partly fixed** (L-S1) | `--safe-owners` is now required under `--phase1`, and the fallback handler and the SafeSetup initializer are read. Creation-code storage planting bypasses all of it. |
| A3-I2 sentinel-only root with tickets | Info | **Fixed as specified** (residual A4-I1) | R3 PoC fails with `BadProof`. |
| R3-C3/C4/C5/C7, I-1 | Info | **Fixed** (R3-C7 residual **R4-C1**) | See the part reports. R3-C6, I-2 and I-3 remain open, as documented. |

## 2. New findings

### Low

#### A4-L1 (contracts): the pause floor bounds a mint under the last post, not under an earlier post of the same floor; a pause still pays ~fillBand

- **Where:** `src/floor/FloorModule.sol:517-521` (`_pauseFloor`), against `mintPrice()` `:1090-1093`.
- **Mechanism:**
  - A mint pays `max(price(), _upTarget()) × (1 + sMint)`.
  - While the keeper's target is below `accepted` (a held-back fall, or one noisy post) and the fills TWAP is at the band's low
    edge, that is `a(1 − fb)(1 + sMint)`.
  - If the keeper's **next** post puts the target back at or above `a` (accepted moves < 0.1 %), `F_p = min(burnTarget, a) = a`.
  - A pause then pays `min(β, a)(1 − base)`.
  - A fresh burn at the same state pays `price() × (1 − sBurn) = a(1 − fb)(1 − sBurn)`, a loss.
  - So the pause, not the market, is worth up to `fb` (3 %) less the spreads to a pre-positioned outsider. The gain comes from the
    existing holders' β, diluted by the cheap mint.
  - The fix doc's §1.4 ("the average backing no longer enters"; "a mint → pause → burn round trip never profits") does not hold
    across posts.
  - Every round-4 test and the invariant's `roundTrip` action assume **no post between the mint and the pause**, which is why they
    miss this.
- **PoC:** `test/audit4/Audit4Floor.t.sol::test_audit4_targetSwingThenPausePaysTheFillBand`, **PASS (attack works)**. The setup:
  - production spread and band parameters (`StackChecks` defaults: base 15, fb 300, maxMove 500);
  - one honest cheap conversion, so the fills sit at `a(1 − fb)`;
  - a post with target `0.9a`, then a mint of 2 units, then one minute later a post with target `a`, then a guardian pause.

  | Params | Paid | Fresh quote | Paused burn | Profit |
  | --- | ---: | ---: | ---: | ---: |
  | reserve 0 | 1.24467 | 1.22397 | 1.25826 | **+0.0136 ETH (+1.09 %)** |
  | reserve 25 % (Phase 1) | 1.24013 | 1.22397 | 1.25784 | **+0.0177 ETH (+1.43 %)** |

  Control `test_audit4_control_samePost` (the pause while the low target still stands): a loss, as the fix doc claims.
- **Size and preconditions:**
  - A pause must follow the swing before the next hour's posts move `accepted`. Pauses are now evidence-based at the monitor, so
    an outsider cannot cheaply force one (see L-S3 for the caveat).
  - The fills must sit below the band, from honest cheap conversions or custodian sales.
  - The keeper's target must swing (market noise).
  - Bounded by `mintPerHour`/`publicCap` per pause. This is about a tenth of R3's A3-M1.
- **Fix (one line, either):**
  - `F_p = min(_burnTarget(), max(a(1 − fb), min(target, a)), a(1 − fb) if any fill is in the window)`. Simplest:
    `F_p = min(_burnTarget(), Math.mulDiv(a, BPS − fillBandBps, BPS), max(...))`, i.e. never above the band's low edge. This costs
    honest paused burners at most 3 % more.
  - Or record the lowest pre-spread mint base price over the trailing `FLOOR_TTL + FILL_WINDOW` and cap `F_p` by it.

  Add a post between the mint and the pause to the fuzz and the invariant handler.

#### R4-C1 (canister): R3-C7's "held in a mempool" rule is defeated by a mempool partition, so a cheap pin wins the 30-day race

- **Where:** `autobuyer/canister-rs/src/conflict.rs:260-268`, `:363-378`.
- **Mechanism:** `held` means both **explorers** show our conflict. With the miners holding the redeemer's pinned delivery, the
  conflict stays at today's rate (2,231 sats) and never out-pays the pin. The result is a LostRace at deliverBy + 30 d, with the
  item paid twice.
- **PoC:** `audit4_partition.rs`, **PASS**: a 20k-sat pin wins. Control, the conflict not seen: it escalates to 25,407 sats and wins.
- **Why Low:** the partition must hold for 30 days, across the 14-day mempool expiry.
- **Fix:** suppress the ×1.5 escalation only for the first 12–24 h after the conflict is first seen.

#### L-S1 (services): golive-check's real-Safe check is defeated by storage planted in the Safe's creation code

- **Where:** `script/golive-check.sh:125-165`.
- **PoC:** `test/audit4/Audit4SafeHidden.t.sol` (`AUDIT4_FORK=1`, read-only 4663 fork), **PASS (attack works)**. Every read passes:
  - the proxy code hash;
  - the singleton;
  - an empty module list;
  - no guard;
  - the canonical 1.4.1 fallback handler;
  - one SafeSetup with a zero initializer;
  - owners equal to `--safe-owners`.

  Yet a hidden module drains the Safe with no signature.
- **Fix:** require a `ProxyCreation` event from a canonical SafeProxyFactory, with the SafeSetup in the same transaction.
- **Doc mismatch:** the fix doc says the SafeSetup owners and threshold are compared with the chain's; the script never compares
  them.

#### L-S2 (services): the keeper alert latch (L-G) fails at `KEEPER_INTERVAL_S` ≥ 300

- **Where:** `keeper/service/status.ts:36-47`; `config.ts:194` (a fixed 1,200 s).
- **PoC:** `keeper/test/audit4_services.test.ts`, **PASS** (the gap is present at 300, 600, 900 and 1,800 s).
- **Fix:** derive the latch from the interval: at least `(passes + 1) × interval + DOWN_MS`.

#### L-S3 (services): the dense-block fallback can silently skip a block (N-S1), or stall the group on a retryable receipts error (N-S2)

- **Where:** `monitor/src/holders.mjs:95-112`.
- **N-S1:** a `null` header, or one with no `transactions`, is treated as an empty block, and so is `[]` receipts for a block the
  provider just called too big. The dense coin's logs are dropped and the cursor passes the block with nothing marked stuck.
  - In R3's own fake, the index holds 100 / 0 where the chain holds 49 / 51.
  - That can feed a false dissent. Combined with a degraded second provider it becomes a pause (I-S2).
- **N-S2:** a 5xx or timeout on `eth_getBlockReceipts` rethrows, cools the provider down, and stalls the whole group again, poll
  after poll. Every draw is then "degraded", which since round 4 means escalate, not pause.
  - This weakens §3.4's "a thief cannot cheaply make every provider degraded".
  - How the real providers answer a ~5–8 MB receipts call was not measured.
- **PoC:** `audit4_monitor` tests 6–8, **PASS** (the gaps are present).
- **Fix:**
  - Treat a malformed header or empty receipts as a throw.
  - Sanity-check receipts (block hash and number, non-empty).
  - On any receipts failure, fall through to the per-address split, so only the dense address stays behind.

#### L-S4 (services): `reconcile()` ignores the poll deadline and runs before the deadline pause

- **Where:** `monitor/src/holders.mjs:399-429`, called at `evm.mjs:668`, before the deadline block `:705-731`.
- **Mechanism:** up to 17 sequential reads a coin, each with a 30 s timeout and 3 tries. A hanging provider can hold `checkEvm`
  for tens of minutes, and a bad root can be pinned meanwhile.
- **PoC:** `audit4_monitor` test 11, **PASS**.
- **Fix:**
  - Bound each read by the remaining budget, with 1 try.
  - Check the deadline per read.
  - Better: run reconciliation after the deadline and guardian block.

### Info

- **A4-I1 (contracts):** the A3-I2 refusal covers only the empty proof. A sentinel plus one junk sibling (a 1-node proof) with
  `totalTickets = 7` still opens, pins and consumes the carry, with nobody able to claim.
  - PoC: `Audit4Draws.t.sol::test_audit4_sentinelPlusJunkNodeStillConsumesTheCarry`, **PASS**.
  - It is a bad root like any other and the monitor's recompute catches it. No new power: the keeper can forge any root.
- **A4-I2 (contracts):** during a long pause the frozen floor can sit well above a fallen market. Wrapped-item holders can sell into
  the module at `F_p` within `burnPerHour`, a free option on every pause. This is documented ("Still open") and smaller than round
  3's β.
- **A4-I3 (site):** `web-rook/src/ui/Books.tsx:23` derives `keeperDown = burnsOpen && !floorFresh && !escaped`. For up to an hour
  after a pause (the last post is still fresh) the Books page says "fresh, so the module trades", while mints are refused and burns
  pay the pause floor.
  - Fix: expose `keeperPaused` from the indexer and use it.
- **R4-C2:** one padding provider can stall the scanner through `Split`, the same power as before; the stall is loud and skips
  nothing.
- **R4-C3:** size detection depends on the IC's exact reject wording. If it changes, the scan stalls (fail-closed); a replica or
  mainnet test is needed.
- **R4-C4:** the paused pass's Bitcoin work now needs the EVM providers to agree on the chain id (R3-C3).
- **I-S1:** the slowest undetected floor walk loses −31 %/week with no page, and −49 %/week with SEV2 only.
- **I-S2:** a stale, unconfirmed dissent plus a degraded second provider pauses. This is deliberate per §3.3.
- **I-S3:** a repair from launch at drpc's ~100-block span takes about 36 h.
- **I-S4:** "query timeout" answers never narrow the span.
- **I-S5:** the committed `web/src/content/trust-model.md` is stale, so a build dirties the tree.
- **I-S6:** with the opt-in no-good-post pause, an outsider who can depress the keeper's confidence gets a guardian pause. This
  costs liveness only.
  - The pause does not loop after an unpause: the trigger key includes `lastGoodAt`, and `acted` is never pruned.
  - A monitor state reset would re-pause once.

### Checked, no finding

**Contracts:**
- The four burn states (escaped → keeper down → fresh → `Stale`).
- `burnsOpen()` is closed only in the documented short outage (A3-I1).
- `F_p` is monotone non-increasing during a pause, and positive.
- The pause floor is never above `accepted` or `β`.
- No mint, float, sale, fill, rental or snapshot happens while the keeper is down.
- A burn never lowers the remaining holders' β.
- A short pause and unpause (fresh) is no lever.
- After a long pause nothing is priced until the first post, which moves at most one hour's `maxMove`.
- Repeated pauses price from the same frozen state.
- Pause and unpause authority: the owner or guardian pauses, only the owner unpauses, and the grace is stamped only on a real
  transition.
- The `holdsRefunds` grace is on both clocks, and `_stuck()` still bounds it.
- `openDraw` refuses an empty sentinel proof with tickets; a legitimate list always has ≥ 1 sibling.
- New interface members compile across all implementers.

**Canister/timelock:**
- Orphan clearing only after 24 h (or pre-`since_ns` state), and a cleared upgrade that resumes installs nothing.
- `committed` cannot stick.
- The `unreadable` retry is within the tick bound, and an attacker cannot grow the list cheaply (500 burned units a dense block).
- Quarantine spacing: alternating good and bad reads clear the count, the same as before, and a bad candidate is still skipped per
  choice.

**Services:**
- The `evidenceOf` / `deadlineCall` table (a thief's bad root is still `bad` when both providers read the coin).
- The per-address split and address batching.
- Stuck-coin consistency; a stuck factory still holds the group.
- `rangeError` vs `retryable` ordering.
- The `guardedFetch` race leaves no socket leak, and the `inflight` timeout.
- SafeSetup decoding for 1.3.0 and 1.4.1, the SafeToL2Setup code-hash acceptance, and no false fail on a factory-made Safe.

## 3. Suite results (this machine, `fixes-2026-09-26`)

| Suite | Result |
| --- | --- |
| `forge test` (as shipped) | **1053 passed, 0 failed** (120 suites; the fork suites return early without `--fork-url`) |
| `forge test --match-path 'test/audit4/*'` | 6 pass: A4-L1 ×2 param sets (attack works) plus 2 controls, A4-I1, and the Safe PoC (skips without `AUDIT4_FORK`) |
| R3 contract PoCs, unmodified | 3 attack tests **fail** (A3-M1: assertion; A3-L1: `the hold ends at the unpause`; A3-I2: `BadProof`), so the attacks are closed; the A3-I1 and N-2 re-runs pass as documented |
| `Audit4SafeHidden` with `AUDIT4_FORK=1` (read-only 4663 fork) | 1 pass: the L-S1 attack works |
| canister `cargo test` | **489 pass**, 0 fail; 493 with the 4 audit4 tests |
| timelock `cargo test` / `--features rehearsal` | **15 / 15 pass** |
| R3 canister PoCs, unmodified | `audit3_scanner` fails its attack assertions (fixed); `audit3_pin` passes (the accepted bounded N-2) |
| monitor `npm test` | **122 pass** (133 with the 11 audit4) |
| keeper `npm test` / typecheck | **134 pass** / clean (137 with the 3 audit4) |
| indexer `npm test` / typecheck | **132 pass** / clean (138 with the 6 audit4) |
| web-rook `npm test` / typecheck / build | 66 pass / clean / ok |
| web `npm test` / typecheck / build | 118 pass / clean / ok (the build rewrites the stale `trust-model.md`, I-S5; restored) |
| ops, staging, tools/snapshot | 31, 29, 19 pass; typechecks clean |
| `scripts/timelock-watch.test.mjs` | 4 pass, 1 skipped (no local replica) |
| `scripts/audit2_golive_check.test.sh` | **49 ok**, exit 0 |
| `STACK_PORT=8897 scripts/golive-dryrun.sh stack` (public read-only fork, real drand) | **DRY RUN: ALL PASS**, 44 ok, exit 0: launch → graduation → module mint → sweep → draw announced → opened with a real seed signature → pinned → resolved → claimed; the golive-check `--phase1` read passes and its refusals behave. Log `R4/dryrun-stack.log` |
| `script/deploy.test.sh` | **37 ok**, exit 0 |

Not run:
- `keeper-fork-test.sh`;
- a fork run driving the keeper and the monitor together;
- a replica or mainnet run of the scanner (needed for R4-C3);
- the canister's reproducible Docker build;
- clippy and `cargo audit`.

Cleanup: the `cargo target/` dirs, `out/`, `cache/`, and every `node_modules` and `dist` installed for this check have been
deleted. `lib/` copies are left in place for `forge`.
