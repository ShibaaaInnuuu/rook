# Review round 2, 2026-09-27: the Ethereum NFT side after fix round 1 (branch `nft-side`)

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `27c0962`.
Read: the round-1 reports (`../README.md`, `../A-…`–`../D-…`, `../poc/`), `../fixes-round1.md`, `docs/v7/phase1-nft.md`,
`docs/v7/phase1-nft-pooled.md`, and the code of both deltas never reviewed before: the `nft-pool` delta `09a680a..8c107e9`
(per-collection graduation targets, pooled graduations, several collections on one Ethereum address) and the round-1 delta
`8c107e9..27c0962`. The pre-fix tree (`Rook-nft-pool` worktree at `8c107e9`) was read for before/after. Method as the 2026-09-26
re-reviews (`docs/audit/2026-09-26/review2`, `review4`): four scoped parts, every round-1 fix re-attacked and its PoC re-run
unmodified, a line-by-line reading of the new code, one PoC per new finding where feasible, a recommended fix per finding.

**Nothing in the code was modified. Nothing was committed but these review files; nothing was pushed, deployed or signed; no key
was used.** Chains written to: local anvils only (the dry run's fork of 4663, the Seaport fork proof's fork of Ethereum). PoCs were
run inside the repository's harnesses, then moved here so the suites stay green.

| Part | Report | Scope | PoCs |
| --- | --- | --- | --- |
| A | `R2-contracts.md` | `PosterSource` pause/unpause, `EthNftWrapper`, the pooled-units backing invariant, `Pooled.t.sol`, `DeployStack` / `StackChecks` / golive-check per-collection lists | `poc/A/` (forge, 8; round 1's re-run) |
| B | `R2-canister-floor.md` | `ethnft.rs` floor rule (rank-2 listing, sales veto, 15 % crash rule), `post_reading` clamp, `deposit_floor_ok`, `pin.rs` normalizing transforms, cycle cuts | `poc/B/` (cargo, 5; round 1's re-run) |
| C | `R2-canister-flow.md` | `ethflow.rs` concurrency: one buy per collection, one Relay leg, earmarks, deliveries first, re-pricing / versions / cancel / superseded / halted / `eth_line_clear`, reorgs, `buy_holds_mint`, upgrade safety and the C-L-4 probe | `poc/C/` (cargo, 5; round 1's re-run) |
| D | `R2-services.md` | runner (sales hand-in, listings), monitor (D8 per collection, latch/ack, outbound canary, pool page), guardian, site, golive-check, dry run, the launch configuration | `poc/D/` (node, 5; round 1's re-run) |

Severity scale: as `../README.md` (Critical / High / Medium / Low / Info). "Passing" PoC = the weakness is present; each turns red
once fixed.

## Verdict

**Not ready to launch: needs a round-2 fix round.** No Critical and no High: no path was found that takes an NFT out of custody,
mints a unit that nothing backs, or makes the canister sign a transaction its judges did not build. Every round-1 **High** is
closed. Of the round-1 **Mediums**, all are closed except **A-M-1** (the canister-side deposit band is set by washable sales and
gates only the mint). Round 2 finds **five new Mediums** in code the round-1 fixes built or that `nft-pool` added, so the bar
(0 C / 0 H / 0 M) is not met.

## Counts of OPEN findings (distinct)

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| A contracts / pooled / deploy (new) | 0 | 0 | 1 | 2 | 3 |
| B canister floor / judges (new) | 0 | 0 | 1 | 2 | 3 |
| C canister flow / upgrade (new) | 0 | 0 | 2 | 3 | 3 |
| D services / deploy / launch (new) | 0 | 0 | 2 | 5 | 6 |
| De-duplication (D2-M-2 = B2-M-1) | | | −1 | | |
| Round 1 not (fully) fixed | 0 | 0 | 1 (A-M-1) | 6 | (Infos listed as unchanged in `fixes-round1.md` §6) |
| **Open** | **0** | **0** | **6** | **18** | **15 new** |

The six Lows carried from round 1: A-L-2 (not fixed; golive-check also does not read an NFT wrapper's `pendingMintLimit`), B-L-5
(partly; C2-L-2 is a new path), B-L-6 (not fixed, unreachable in the NFT-only stack), C-L-5 (partly), D-L-3 (partly), D-L-6
(partly). C-L-4 and L-2 are counted through their residuals C2-L-3 and C2-L-1.

## Every open Medium

1. **A-M-1 (round 1, not closed)**: deposit → burn at a floor above the market. The canister gate compares `accepted` with the
   median of the ≤ 12 sales the runner hands in; seven gas-only in-band wash sales along a wallet chain (or a malicious runner's
   choice of real sales) set that median, and the gate binds the mint, not the later burn —
   `autobuyer/canister-rs/src/ethflow.rs:1771-1784` `deposit_floor_ok`; `src/ethnft.rs:524-558`; `src/floor/FloorModule.sol:525-539`.
   PoC `poc/B/audit2_nft_b.rs` `audit2_a_m1_…`.
2. **B2-M-1 = D2-M-2 (new)**: the sales veto is the attack. Ten ping-pong sales of one NFT between two wallets (far from `L`, dropped
   before the dedup) crowd the 12-sale sample below `min_sales`, or seven in-band washes veto `L`: no post, the collection's module
   goes stale (mints, fills, burns revert `Stale`; buys and deposit mints wait) until the D8 no-good-post poster pause or the 7-day
   escape. Gas only, per collection — `src/ethnft.rs:524-535, 597-613`; `ops/src/ethnft.ts:255-282, 376`; `ethflow.rs:745`.
   PoCs `poc/B/audit2_nft_b.rs` `audit2_b2_m1_…` ×2, `poc/D/ops/test/audit2_nft_d.test.ts` "D2-M-2".
3. **A2-M-1 (new)**: a pooled collection (BAYC at 9 ETH, Pudgy: the launch config's two pooling collections) whose floor rises
   more than ≈ 5 % between its graduations advances the blended pool below the floor: the ETH leaves 4663, sits `Arrived` on
   Ethereum with no give-back, and the module's conversions stop (`AdvanceOpen`). Contradicts `phase1-nft-pooled.md` §3.4; the
   test claiming it (`test/ethnft/Pooled.t.sol:232-249`) is vacuous — `src/floor/FloorModule.sol:1329-1333`;
   `ethflow.rs:1866-1890, 880-894, 2132-2137`. PoC `poc/A/AuditNftA2.t.sol` `AuditNftA2_M1_…`.
4. **C2-M-1 (new)**: one collection's mined fill read as another collection's Relay arrival (the arrival is a relative balance
   delta; a fill mined at the pin whose receipt or `ownerOf` read failed that tick is not yet in `eth_out`): the other buy goes
   `Arrived` on ETH it never received, its earmark exceeds the balance (starving every other buy and every stray return), the
   settle cost is wrong, and a later 4663 refund leaves it `Arrived` for good — `ethflow.rs:1913-1923, 916-921, 1440, 1642-1648`.
   PoC `poc/C/audit2_nft_c.rs` `c2_a_fill_mined_at_the_pin_…`.
5. **C2-M-2 (new)**: an `Arrived` buy that cannot fill (the launch collections have ≈ 2.3–2.6 % floor-rise headroom) holds every
   deposit of its collection unminted, with no bound and no depositor or operator exit (A-L-1's fix, `buy_holds_mint`) —
   `ethflow.rs:468-471, 1962-1964, 2132-2138`. PoC `poc/C/audit2_nft_c.rs` `c2_an_arrived_buy_that_cannot_fill_…`.
6. **D2-M-1 (new)**: the outbound canary and the custody check page SEV1 "pause, then look" on every honest buy (fill mined, buy
   still `Buying` until the next tick: `eth-arrived-gone`) and every honest redemption (`itemOf` cleared only by
   `recordDelivery` after Ethereum finality: `eth-not-held`) — `monitor/src/ethnft.mjs:358-361, 247-260`. PoC
   `poc/D/monitor/test/audit2_nft_d.test.mjs` "D2-M-1a/b".

## Round-1 Highs and Mediums: verdicts

| ID | Sev | Verdict | Part |
| --- | --- | --- | --- |
| B-H-1 floor steered by self-trades | High | **Closed** (residuals B2-M-1, B2-L-1) | B |
| B-H-2 / C-H-1 one refusal stops every post | High | **Closed** | B, C |
| D-H-1 runner hands in transfers | High | **Closed** (residuals D2-M-2 = B2-M-1, D2-L-2) | D |
| D-H-2 unclamped post / registry-wide runaway pause | High | **Closed** (canister clamp: bounds only widen, checked incl. the day-anchor reset; monitor pauses the poster) | B, D |
| A-M-1 deposit → burn at a high floor | Med | **Not closed** | A, B |
| A-M-2 `redeemToRelist` | Med | **Closed** | A |
| B-M-1 one bad receipt voids the pass | Med | **Closed** (residual B2-L-2) | B |
| B-M-2 / C-M-1 transport failure = refusal | Med | **Closed** | C |
| B-M-3 / C-M-3 `superseded` clears before reads | Med | **Closed** (C2-I-2) | C |
| B-M-4 nonce at a backwards pin | Med | **Closed** | C |
| B-M-5 / C-L-3 one collection blocks all | Med | **Closed for the reported paths**; new cross-collection paths C2-M-1, C2-L-1, C2-L-2 | C |
| B-M-6 provider shapes | Med | **Closed** (normalization cannot make different answers agree) | B |
| B-M-7 cycles | Med | **Closed for honest load** (B2-L-2) | B |
| B-L-3 / C-M-2 unmined line tx | Med | **Closed** (regression asserts a weak disjunction) | C |
| D-M-1 D8 registry-wide / sticky | Med | **Closed** (contract and monitor halves) | A, D |
| D-M-2 canary vs Bitcoin redemptions | Med | **Closed** | D |
| D-M-3 no outbound canary | Med | **Closed** as built, but it raises D2-M-1 | D |
| D-M-4 refused listing re-handed | Med | **Closed** | D |

**Carried-over 2026-09-26 fixes:** H9 (ticket facts final) **closed**; R3's paused pass **closed**; L-2 **partly** (its `Stuck`
branch is dead code at HEAD, C2-L-1); L-5 **closed** (the fill cancel); W2 "Left open 2" (= D-H-2) **closed**; C-L-4 **partly**:
the probe checks only the 13 top-level elements, so an older `EthTx` inside `line` restores as `line = None` and the upgrade
proceeds (C2-L-3). The `nft-pool` `#[serde(default)]` non-`opt` `EthTx` fields are confirmed (latent: `nft-side`-era state loses the
line; `nft-pool` → HEAD decodes, only `opt` fields and one appended variant were added).

## The Lows and Infos (details in the part reports)

New Lows: **B2-L-1** washes disarm the upward veto (a swept thin book is posted, +23 % in the PoC); **B2-L-2** oversized sale
receipts never cached (paid drain, and under 8 T cycles the evidence pass stops for every collection); **C2-L-1** a post-arrival
`Stuck` keeps its Relay watch and re-arrives spent ETH; **C2-L-2** an upgrade dropping a collection whose buy holds the Relay leg
stops every collection's buys; **C2-L-3** the C-L-4 probe misses nested `opt`s; **A2-L-1** J1's alternative (`convBps` 1,100) leaks
≥ 0.41 ETH per ape to the gas pot, not module revenue as documented; **A2-L-2** stage-0 custody caps against five collections (J7);
**D2-L-1** the Trust page's A-M-1 sentence; **D2-L-2** runner sale verdicts cached per tx, not per collection; **D2-L-3** the
runner's start refusal runs once and is skipped when the config is unread; **D2-L-4** one failed read resets a collection's monitor
timers (the 24 h pool page never fires with one blip a day); **D2-L-5** nothing compares the deployed targets with the launch file,
and `--canister-evm` is only a WARN under `--phase1`.

Infos: B2-I-1..3, C2-I-1..3, A2-I-1..3, D2-I-1..6.

## The launch configuration (five collections, BAYC at 9 ETH, deposits on, D1–D9)

What makes launch unsafe or not as described, beyond the Mediums above:

- **J7 is still open and must be decided before launch** (A2-L-2). At stage 0 the global cap is 6 items / 10 ETH across all five
  collections, shared with deposits. One ape (≈ 6.8 ETH) fits only while the other four together hold ≤ ≈ 3 ETH; while an ape is
  held, Pudgy never fits; a depositor with a few cheap NFTs can fill the room for free and stop BAYC's and Pudgy's buys (or, at
  six items, every collection's). No loss (pools stay ETH-backed and burnable), but the pooled collections will rarely buy.
- **BAYC and Pudgy are exactly the collections A2-M-1 hits** (pooled; ≈ 2.6 % headroom at 9 ETH with the defaults), and C2-M-2
  then freezes their deposits for as long as the buy waits.
- **`pause_on_no_good_post_h` must be armed for every Ethereum collection** in the launch monitor config: under B2-M-1 it is the only
  thing that reopens burns before the 7-day escape. No launch monitor config is in the repository, so this and each collection's
  `opensea_slug` / `coingecko_id` could not be checked.
- golive-check does not hold the launch values: targets are compared with the stack file the same deploy wrote, not with
  `deployments/launch/nft-pilot.json` (a shifted comma list puts 9 ETH on Pudgy and passes; a `uint16` cast wraps before the bounds
  check; a single-collection variable applies to all five); `max_purchase_wei` is not read; `--canister-evm` is a WARN (D2-L-5,
  A2-I-2).
- The Ethereum provider pair for `evm_chains[1]` should be the fixer's recommended `eth.drpc.org` + `xrpc.cl/eth` (B-M-6); there is
  no go-live check of it.

## Recommended order for round 2 fixes

1. The floor sample (B2-M-1 / D2-M-2, A-M-1, B2-L-1): the canister choosing its own time-based, wallet-diverse sale sample (its own
   `eth_getLogs`) closes all three and the runner residual; at minimum the runner hands in only sales that count, and A-M-1 is
   closed on the module (cap fresh burns at the trailing-24 h low of `accepted`).
2. Pooled advance gate (A2-M-1): no advance unless `item_cap` after Relay's fee and gas covers the fresh rank-2 listing (better, plus
   the premium); an operator exit for a long `Arrived` buy; fix the doc and the vacuous test.
3. Arrival accounting (C2-M-1, C2-L-1): do not take `eth_before` while a line transaction's outcome is unread; clear the watch on
   arrival; cap `got` at the quote.
4. Deposits vs a waiting buy (C2-M-2): reserve the buy's mint room instead of blocking deposits, or bound the hold.
5. Monitor false SEV1s (D2-M-1), then the Lows; J7 decided by the operator.

## Test bar (re-run by this review)

Every number of `fixes-round1.md` §9 reproduced at `27c0962`, with the PoC files out of the tree:

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,100 passed**, 0 failed (125 suites) |
| Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **574 passed**, 0 failed |
| Timelock `cargo test` (and `--features rehearsal`) | 15 passed (15) |
| ops | **51 passed**; `tsc --noEmit` clean |
| monitor | **164 passed** |
| keeper | 150 passed |
| indexer | 135 passed |
| web-rook | 72 passed; `tsc --noEmit` clean |
| typechecks (ops, keeper, indexer, web-rook, staging, tools/snapshot) | clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (536 golive checks ok on the five-collection NFT stack; BAYC "pools two graduations per ape"; one poster paused while the next collection posts; the paused-poster refusal) |

**Reproducible build:** `docker build --no-cache` of `autobuyer/Dockerfile.build` on a `git archive` export of `27c0962` (no
`autobuyer/` change since `29c3287`) gave **identical** hashes:

```
915a8780eda64455532921da53def5048ec4ceb344eda17a350f6f2de3c11379  buyer.wasm           (6,458,868 bytes)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

The export directory, the build image and its build cache were removed afterwards; no anvil is left running.

Note: these suites are green *with* the round-2 weaknesses present. The fixer's regressions do not cover the new findings (and one,
`Pooled.t.sol` `test_aFloorRiseWhilePooled_…`, is vacuous: A2-M-1), so a green bar is not evidence that they are absent.
