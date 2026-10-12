# Review round 3, 2026-09-27: the Ethereum NFT side after fix round 2 (branch `nft-side`)

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `4949da0`.
Read: `../README.md`, `../A-…`–`../D-…`, `../fixes-round1.md`, `../review2/` (README, the four part reports, `poc/`),
`../fixes-round2.md`, `../fixes-round2-poc/`, `docs/v7/phase1-nft.md`, `docs/v7/phase1-nft-pooled.md`, and the code of the
round-2 delta `git diff 79455e1..HEAD`, with the earlier code where a finding goes back further. Method as round 2 and the
2026-09-26 re-reviews: scoped parts, every round-2 fix attacked and its PoC re-run, the fixer's re-run outputs checked for
honesty, a line-by-line reading of the new code, one PoC per new finding where feasible, a recommended fix per finding.

**No code was modified. The only commits are these review files; nothing was pushed, deployed or signed, and no key was
used.** The only chains written to were local anvils: the dry run's fork of 4663 and the Seaport fork proof's fork of Ethereum.
Network use was read-only: the fork upstreams, the reproducible build's crate fetch, and part D's probes of the template's
providers, OpenSea and CoinGecko. Every PoC ran in a private copy of its harness, never in the worktree, so the suites here stay
green.

| Part | Report | Scope | PoCs |
| --- | --- | --- | --- |
| A | `R3-contracts-deploy.md` | `src/` unchanged (verified), `returnAdvance` under the way back, `Pooled.t.sol`, `DeployStack`, `golive-check.sh --launch / --monitor-config`, the dry run's refusals, the launch configuration (five collections, J7) | `poc/A/` (forge 4, canister 1, golive harness; round 1's and round 2's re-run) |
| B | `R3-canister-floor.md` | the canister's own sale sample (`sale_scan`, `eth_transfer_txs`), `counted_sales` / `round_trips` / `floor_from`, the deposit band, the burn-side bound (A-M-1, B2-M-1 = D2-M-2, B2-L-*) | `poc/B/` (cargo 3; round 2's re-run) |
| C | `R3-canister-wayback.md` | the advance gate (A2-M-1), the new signing path Ethereum → 4663 (`BackFunded` / `BackSending` / `BackBridging`, `relayeth::judge` `FROM_ETHEREUM`, the 48 h return, `eth_give_back`), C2-M-2 | `poc/C/` (cargo 6; round 2's re-run) |
| D | `R3-services.md` | monitor (D2-M-1 both ways, new pages, D2-L-4), runner (listings only, the way-back quote), the IDL against the `.did`, the Trust page, the launch monitor template | `poc/D/` (node 6, cargo 1; round 2's re-run) |
| E | `R3-canister-flow-upgrade.md` | arrival accounting (C2-M-1 `unsettled`, `arrive`'s cap), the deposit hold, concurrency, upgrade safety (`lost_opts`, `stranded_by`, the round-2 fixture) | `poc/E/` (cargo 8, a `.did` check; round 2's re-run) |

Severity scale and threat model: as `../README.md`. A "passing" PoC shows the weakness is present, and it turns red once fixed.

## Verdict

**Not ready to launch. A round-3 fix round is needed.** Round 3 finds **2 High** and **3 Medium**, so the bar
(0 C / 0 H / 0 M) is not met. There is no Critical: no path was found that makes the canister sign a transaction its judges did
not build, or that drains custody with nobody able to stop it.

Of round 2's six Mediums, four are closed as reported (A-M-1, A2-M-1, C2-M-2, D2-M-1). Two are only partly closed.
**B2-M-1 = D2-M-2** has a new freeze route, B3-M-1. **C2-M-1** has a new phantom-arrival route, E3-M-1. The two Highs:

- **D3-H-1** has existed since the delivery code was written and was missed by rounds 1 and 2.
- **C3-H-1** comes from fix round 1's drop-and-reopen of a waiting buy. Round 2's way back makes it much easier to reach.

The new signing path, the way back through Relay, holds on what it signs:

- it goes only to our own address, in native ETH, through the pinned depository and allocator, within the fee bound;
- it has no replay;
- a fill and the way back never spend the same ETH;
- a double `returnAdvance` reverts at the module.

What it lacks is an exit (C3-M-1).

## Counts of OPEN findings (distinct)

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| A contracts / deploy / launch (new) | 0 | 0 | 0 | 4 | 3 |
| B canister floor / sale sample (new) | 0 | 0 | 1 | 1 | 2 |
| C advance gate / way back (new) | 0 | 1 | 1 | 3 | 6 |
| D services / monitor / template (new) | 0 | 1 | 0 | 3 | 7 |
| E arrival accounting / upgrade (new) | 0 | 0 | 1 | 3 | 5 |
| De-duplication (E3-I-4 folded into C3-L-3) | | | | | −1 |
| Earlier rounds, not (fully) fixed | 0 | 0 | 0 (counted through B3-M-1 and E3-M-1) | 7 | — |
| **Open** | **0** | **2** | **3** | **21** | **22** |

The 7 Lows carried from earlier rounds:

- A2-L-2 (J7, operator-pending);
- A-L-2 (partly; the deploy key can still cut the wrapper's mint limit at once);
- A2-L-1 (partly; the doc was fixed, the behaviour was not);
- B-L-5, C-L-5, D-L-3, D-L-6: partly, as `fixes-round2.md` §7. They were not re-attacked here beyond confirming the code is unchanged.

B-L-6 is open and wider; it is counted as C3-L-3. B-L-3 / C-M-2's cancel does not cover `BackDeposit`; that is an Info of C.

## Every open High and Medium

1. **D3-H-1 (High).** A redemption whose NFT the destination moves on before Ethereum finality is never recorded. Thirty days
   later `refund()` mints the unit back, so the redeemer keeps the NFT and gets a unit backed by nothing.
   - **Code:** `autobuyer/canister-rs/src/ethflow.rs:1233-1242` (`delivery_pass`, `Moved`: at finality it records only if
     `ownerOf == destination`, otherwise it errs and stays `Moved` for ever). `src/nft/EthNftWrapper.sol:408-433` (`refund`: anyone,
     past `deliverBy + LATE_DELIVERY_WINDOW` (30 d), puts the item back in the set and mints `ONE_ITEM`).
   - **Who can trigger it:** anyone who redeems and transfers within about 15 minutes. It also happens with no attacker: a sale
     into a bid, or a sweeping smart wallet.
   - **Who can stop it:** there is no pause on `refund` and no operator path to record the delivery. Only a timelocked canister
     upgrade inside the 30 days stops it. The monitor pages SEV1.
   - **PoC:** `poc/D/canister/`. The wrapper half is the repository's own `test_withoutTheAttestationTheLateWindowIsThirtyDays_…`.
     I verified the code path.
   - **Fix:** judge the delivery by its final receipt (the `Transfer` custodian → destination in our mined transaction), not by
     the current owner. The monitor must accept any owner once our delivery is mined, or every honest case becomes a false SEV1.
2. **C3-H-1 (High).** An advance that waits on H3's daily value limit is dropped (`ethflow.rs:2222-2230`) and reopened by
   `buy_pass` (`:1039-1072`) under a new id each tick. Its key `buy:eth:{id}` is therefore new each time, and the limiter counts
   each new key as a new ask (`outflow.rs:194-195`, before any verdict). Demand grows by one advance per tick until the
   tripwire (3× the limit) freezes and pauses the whole canister (`canister.rs:1600-1614`).
   - **Effect:** once paused, nothing posts floors, every module goes stale within its 1 h TTL, and mints, burns and deliveries
     stop for all holders until the controller (after handover, the timelock and k guardians) resumes.
   - **Trigger at stage 0 (H3 10 ETH/day, trips at 30 ETH asked), no attacker needed:** any day whose advances exceed 10 ETH. For
     example, a BAYC ape (about 7 ETH) returned by the way back or by `eth_give_back` and wanted again the same day. Or BAYC plus
     Pudgy once J7 raises the custody caps but not H3. `buy_pass` pre-checks the custody cap but not H3.
   - **PoC:** `poc/C/r3_c.rs`, with the canister's real limiter: `Tripwire{flow: Value, demanded 3.0012, limit 0.9}`, paused.
   - **Fix:** a stable per-collection key for the advance's ask; or keep the waiting buy (off the Relay leg) instead of dropping
     it; or check H3 in `buy_pass` before a buy is opened.
3. **B3-M-1 (Medium).** A flood of plain ERC-721 transfers (gas only, not sales) starves real sales out of the canister's own
   sample.
   - **Code:** `ethflow.rs:879-889`.
   - **How the queue loses sales:** the queue of transactions to judge is built newest first and truncated to
     `SALE_TODO_MAX` (512), with 16 receipts read a pass. Truncated entries are dropped, and their blocks are never scanned again.
   - **How the cache loses sales:** the cache keeps the newest 2,048 judged transactions per collection (`:915-921`), so judged
     flood entries also push out cached real sales.
   - **Effect:** the floor is held and never confirms or rises, and deposits (D5 on) never mint (`deposit_floor_ok`,
     `:2101-2123`). Before a collection's first post (`accepted == 0`) nothing is posted at all (`ethnft.rs:719-720`): the
     module is stale from launch, and its mints, burns and graduation fills revert.
   - **Cost:** holding it needs more than 16 fresh transfers per 20-minute pass (plus enough to outnumber the real sales in the
     cache's span). Order of 0.05–0.5 ETH a day at Ethereum gas, per collection.
   - **Why Medium:** no value is lost and it heals when the flood stops. It refutes `fixes-round2.md` §1.2 / §1.4 ("no volume of
     washes crowds real sales out", "neither stops a post").
   - **PoC:** `poc/B/b3_nft_flood.rs.snippet` (control, attack, first post).
   - **Fix:** never drop a pending hash, since they are 32 bytes each; or record which blocks are fully judged; read the oldest
     unread first when the budget is spent; and page, rather than wait silently, when a first post has no sample.
4. **C3-M-1 (Medium).** The way back has no exit, and while it waits no collection buys.
   - **Code:**
     - `buy_pass` opens no buy while any buy is `BackFunded` (`ethflow.rs:1036-1038`);
     - `BackBridging` holds the Relay leg (`:160-167`) and past its deadline only waits (`:1166-1169`);
     - `eth_give_back` refuses every `Back*` phase (`:2492-2503`).
   - **Triggers:** Relay cannot quote an item-sized amount back to 4663 (only 0.2 ETH was measured), the runner withholds the
     quote, or Relay neither fills nor refunds.
   - **Effect:** conversions stop for every collection until an upgrade. Burns and redemption are unaffected. The forward leg has
     an operator exit here; the way back does not.
   - **PoCs:** `poc/C/r3_c.rs`, two of them: 7 days in `BackFunded`, and 3 days in `BackBridging` past the deadline.
   - **Fix:** a bound on `BackFunded` and on `BackBridging` past the deadline, after which the buy returns to `Arrived` and off the
     leg; an operator exit for `Back*`; and a monitor page for each (D3-I-4).
5. **E3-M-1 (Medium).** The arrival reads use a pin with no floor. Only signing has B-M-4's guard (`ethflow.rs:1532-1538`). The
   pin is the lowest provider head less one 8-block bucket (`pin.rs:54-61`), so it moves back when a head falls back.
   - **Code:** the Funded watch start `:2272`; the reads in Bridging `:1093-1096`, in Stuck `:1124-1133`, and at the way back's
     refund `:1161-1164`.
   - **Effect at a lower pin:**
     - A fill already counted in `eth_out` looks unspent. Another collection's `Bridging` buy then reads it as its own arrival,
       which is C2-M-1's phantom again: earmarks above the balance (0.495 earmarked, 0.013 held in the PoC), and a 4663 refund
       goes unseen.
     - Or a watch started at the lower pin never sees the real arrival. The buy goes `Stuck` holding the one Relay leg, and
       `eth_give_back` returns gas money instead of the ETH that arrived.
   - **The fixer's test:** `r2_c2_m1_an_arrival_is_never_earmarked_over_the_balance` passes only because another buy's earmark
     happens to fill the room.
   - **PoC:** `poc/E/audit3_nft_e.rs`.
   - **Fix:** every Ethereum read of the flow at `pin ≥ max(last_mined block, the watch's own pin)`, or wait.

## Round-2 Mediums: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **A-M-1** deposit → burn at a floor above the market | **Closed** | The canister reads its own sales (final blocks, `eth_getLogs`), one count per token and per wallet, and a round-tripped token counts nothing. The band is `accepted ≤ min(L, S25) × 1.05`, and the post rises at most to `S25 × 1.10` and never above a live listing. Round 2's PoC fails on its attack assertion (median 0.91, not 1.20). Chains, rings, disjoint pairs and clusters were tried: a lift needs wash votes outnumbering ¾ of the week's real distinct-wallet sales, each vote its own token and two fresh wallets. That residual is stated in `fixes-round2.md` §10. The module-side burn cap was not built. Residual: the band can be kept shut (B3-M-1). |
| **B2-M-1 = D2-M-2** sales veto stops the post | **Partly closed** | After a first post, sales hold the floor back and no longer stop it: a held post keeps the module fresh, and round 2's PoCs fail. At the first post, and for the floor's rise and deposits, the freeze is back by a cheaper route (**B3-M-1**). The runner hands in listings only; its sale hashes are ignored (B3-I-1). A seller with three wallets can list at rank 2 only by listing real, fillable NFTs, so `L` stays a price anyone can fill. |
| **A2-M-1** pooled advance under the floor | **Closed** | `advance_covers` is checked before a buy starts and again right before the advance. Its constants are exact (GAS_KEEP 0.0002, 50 bps, 100 bps, 0.003 ETH), and the fixer's canister tests are real. `Pooled.t.sol` tests a Solidity copy of the gate, and its last branch never runs (A3-I-1). New issues in the way back: C3-M-1; C3-H-1's trigger; C3-L-1 (the gate assumes 0.5 %, the judge allows 3 %); A3-L-1 / C3-L-2 (repeated round trips charged to holders, no cooldown); C3-L-3. |
| **C2-M-1** a fill read as another buy's arrival | **Partly closed** | `unsettled` works as claimed: it is reset every tick, covers all collections, and covers the `BackDeposit` line transaction. `arrive` caps the earmark at the balance. A pin that moves back reopens the phantom (**E3-M-1**). |
| **C2-M-2** an arrived buy holds deposits unbounded | **Closed** | The hold is 6 h, then the way back at 48 h. The fixer's re-run of round 2's PoC fails at a harness `unwrap`, not its assertion. With that call made tolerant, the deposit is `Minted` and the PoC fails on the attack assertion (parts C and E). Residual E3-L-1: every re-entry to `Arrived` restarts both clocks, and a seller who lists and cancels ahead of the fill can repeat it. |
| **D2-M-1** monitor pages honest flows | **Closed** | The allowance covers one buy, and only until the canister's next tick. The custody suspects are judged after a re-read of the records. The fixer's controls are real, and a real loss still pages. Residuals: D3-L-1 (a stale `Recorded` masks a later loss of the same token to the same address), D3-L-2 (a balance / records read race gives a false SEV1 that mutes a real one for 6 h), D3-I-2, D3-I-7. |

Round-2 Lows: B2-L-1, B2-L-2, C2-L-2, D2-L-1..5 are closed. C2-L-1 is closed (residual E3-L-3) and C2-L-3 is closed (E3-I-1,
E3-I-3). Upgrade safety of the new state holds:

- every new field is `opt` + `serde(default)`, and every new case is appended;
- the round-2 fixture is the canister encoder's own output, compared byte for byte;
- `lost_opts` raises no false refusal on the `e04f49b` fixture or on a rich 79455e1-built state;
- the probe cost on the largest state the code allows is far under the upgrade instruction limit.

## The Lows and Infos (details in the part reports)

New Lows:

- **A3-L-1:** each way back is charged to revenue, then surplus, then backing, with no cooldown. It is bounded only by the gate.
- **A3-L-2:** `--launch` does not check wrappers' names, symbols or slugs.
- **A3-L-3:** `--monitor-config` passes swapped D8 references, a guardian that is not the registry's, and placeholders outside two
  blocks.
- **A3-L-4:** deposits enter custody at 0 wei. About 29 ETH can be deposited with BAYC and Pudgy, and the value caps never see it
  (C-I-2 sharpened).
- **B3-L-1:** the sale scan needs byte-identical `eth_getLogs` from both providers. One lagging or capping provider stalls the
  sample.
- **C3-L-1:** the fee estimate against the judge's bound.
- **C3-L-2:** the runner holds the operator identity and can `eth_give_back` at will.
- **C3-L-3:** any 4663 inflow during `BackBridging` is taken as the arrival (B-L-6 wider).
- **D3-L-1, D3-L-2:** the monitor residuals above.
- **D3-L-3:** the template's first Ethereum provider (`eth.drpc.org`) refuses the monitor's 7,200-block deposit scan. Measured:
  it answered 100 blocks and refused 500. D5's unclaimed-deposit watch is dead at launch. The same cap would halve every
  2,000-block chunk of the canister's sale scan to about 62 blocks, taking days of passes before a first post.
- **E3-L-1:** the `Arrived` clocks restart on every re-entry.
- **E3-L-2:** C2-I-1's gas drift plus `arrive`'s hard wait deadlocks the Relay leg. Topping up the Ethereum address unblocks it.
- **E3-L-3:** a post-arrival `Stuck` buy has no exit, and `stranded_by` then pins its collection.

Infos:

- A3-I-1..3;
- B3-I-1..2;
- C3-I-1..6: indefinite pooled wait, no `BackDeposit` cancel, two more `BackFunded` wedges, a stale `relayeth.rs` comment, the
  fixer's C2-M-2 re-run, what holds;
- D3-I-1..7: including D3-I-3, three Trust-page overstatements (first post; the 0.5 %; the 48 h clock that resets), and D3-I-6,
  `MONITOR_COINGECKO_KEY` unmentioned;
- E3-I-1..3, E3-I-5.

## The launch configuration (five collections, BAYC at 9 ETH, deposits on, stage-0 caps unchanged)

What makes launch unsafe as configured, beyond the Highs and Mediums above:

- **C3-H-1 is reachable at stage 0 with the launch numbers.** H3 is 10 ETH a day. One BAYC advance is about 7 ETH, so any
  second large advance within 24 h trips the canister-wide pause within a few ticks. The monitor's own `Arrived` page suggests
  `eth_give_back`, which is one such trigger.
- **D3-H-1 applies to every collection from the first redemption.**
- **J7 is still open** (A2-L-2, operator-pending). Corrected by part A: deposits count 0 wei, so a depositor blocks buys by filling
  the 6-item global count, not the value cap. With BAYC and Pudgy deposited, custody can hold about 29 ETH against a "10 ETH"
  stage (A3-L-4).
- **The advance gate at today's floors (part A):** BAYC's 8 ETH purchase limit and 9 ETH target pass at a 6.3 ETH floor with
  1.4 % to spare, and BAYC keeps buying up to a floor of about 7.88. Pudgy's 4 ETH limit works up to about 3.94. After a 10 % rise
  a pooled BAYC needs about 6 more graduations before it converts (A3-I-2).
- **The launch monitor template:** its slugs and CoinGecko ids match the five contracts (checked live), and
  `pause_on_no_good_post_h` is 6. But its first Ethereum provider breaks the deposit scan (D3-L-3). Put `xrpc.cl/eth` first, or
  use a provider that serves 7,200-block `eth_getLogs`, and check the canister's `evm_chains[1]` pair the same way (B3-L-1).
- `golive-check` still does not read the canister's `max_item_wei`, global custody caps or provider pair (A3-I-3, B-M-6's go-live
  read).

## Recommended order for round-3 fixes

1. **D3-H-1:** judge the delivery by its final receipt, with the matching monitor change.
2. **C3-H-1:** a stable key for a waiting advance, or check H3 before a buy opens.
3. **E3-M-1:** a pin floor for every flow read, not only signing.
4. **B3-M-1:** never drop pending sale hashes, read the oldest unread first, and page a first post with no sample. Then B3-L-1
   (reconcile `eth_getLogs` across providers) and the provider pair (D3-L-3).
5. **C3-M-1:** bounds and an operator exit for `BackFunded` / `BackBridging`, then E3-L-2 / E3-L-3 (the other no-exit states) and
   E3-L-1.
6. The Lows. J7 stays with the operator; decide it together with H3's value limit (C3-H-1).

## Test bar (re-run by this review at `4949da0`, PoC files out of the tree)

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,101 passed**, 0 failed (125 suites) |
| Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **592 passed**, 0 failed |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed |
| ops | **55 passed**; `tsc --noEmit` clean |
| monitor | **169 passed** |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | 72 passed; `tsc --noEmit` clean |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 checks on the v3–v6 stack; 536 golive checks ok on the five-collection NFT stack; `--canister --launch --monitor-config` passes, and the refusals are all `ok`: BAYC's 9 ETH shifted onto Pudgy, BAYC's purchase limit at 2 ETH, no no-good-post pause, a missing CoinGecko id, `--phase1` without `--canister-evm`, `--phase1` without the launch file and monitor config) |

Every number in `fixes-round2.md` §11 reproduced. These suites are green *with* the round-3 weaknesses present. No regression
covers D3-H-1, C3-H-1, B3-M-1, C3-M-1 or E3-M-1.

**Reproducible build:** `docker build --no-cache --platform linux/amd64` of `autobuyer/Dockerfile.build` on a `git archive` export
of `4949da0` gave **identical** hashes to `fixes-round2.md` §11:

```
8efa4bdd43b4544496828cf693383b6bd8ce0c38ed6084caf38ed4181c6e6938  buyer.wasm           (6,603,914 bytes)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

Removed afterwards: the export directory, the build image, the extracted wasm, the dry run's deployment files (it removes its
own) and the anvils. Docker's shared build cache was left alone. Nothing was installed.
