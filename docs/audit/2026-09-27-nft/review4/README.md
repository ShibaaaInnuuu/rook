# Review round 4, 2026-09-27: the Ethereum NFT side after fix round 3 (branch `nft-side`)

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1eb5788`.

**Read:**
- `../README.md`, `../review2/`, `../review3/` (README, the five part reports, `poc/`);
- `../fixes-round1.md`..`../fixes-round3.md`, `../fixes-round3-poc/`, `../bayc10-decisions.md`;
- `docs/v7/phase1-nft.md`, `docs/v7/phase1-nft-pooled.md`;
- the code of `git diff 55d8d32..HEAD`: the operator's round `55d8d32..a37f54b`, not reviewed before, and fix round 3
  `a37f54b..1eb5788`. Earlier code was read where a finding goes back further.

**Method:** as round 3. Five scoped parts. Every round-3 fix was attacked, and its fixer's re-run checked. The new code was read
line by line. One PoC per new finding where feasible, and a recommended fix for each.

**No code was modified. The only files are these review documents and their PoCs.** Nothing was pushed, deployed or signed, and no
key was used.

- **Chains written to:** local anvils only: the dry run's fork of 4663 and the Seaport fork proof's fork of Ethereum.
- **Network use (read-only):** the fork upstreams and the reproducible build's crate fetch.
- **PoCs:** every one ran in a private copy of its harness, never in the worktree, so the suites here stay green.
- **Scratch copies:** a scratch-disk cleanup during the review deleted the first private copies. Every PoC whose output is kept
  here was re-run in a fresh copy made after it (each checked identical to the tree with `diff -rq` apart from the PoC file), and so
  was the whole test bar.

| Part | Report | Scope | PoCs |
| --- | --- | --- | --- |
| A | `R4-contracts-deploy.md` | `EthNftWrapper.noteSent` / `sentTx` / the `DeliverySentNotRecorded` guard; `EthNftWrapperSent.t.sol`, the invariant; `golive-check.sh` `--install-arg` (the keyless rule), `--launch`, `--monitor-config`; the dry run; the launch configuration (five collections, BAYC 10 ETH conv 1,000 reserve 0, J7, H3, D1–D9); `Pooled.t.sol`, `StackChecks.t.sol` | `poc/A/` (forge, canister ×2, the install-argument harness) |
| B | `R4-canister-floor.md` | B3-M-1: `sale_scan`, `sale_order`, `SaleScan` (`pend`, `rescan`, `judged`), the transform, the 500-block chunks; B3-L-1 | `poc/B/` (cargo 2) |
| C | `R4-canister-wayback.md` | C3-H-1 (the H3 key), the advance bound, the leftover and the sweep, C3-M-1 (the way back's exits, `eth_give_back`), the late-payout residual, C3-L-1; the 4663 line | `poc/C/` (cargo 5) |
| D | `R4-services.md` | F3-1 and its siblings (a canister-encoded state through the monitor's IDL and `plain()`), the monitor half of D3-H-1, redaction, `ops`, the IDLs, the Trust page, D3-I-2, the one-tick false SEV1 | `poc/D/` (canister encoder, node 5; review3's D PoCs re-run) |
| E | `R4-canister-flow-upgrade.md` | the canister half of D3-H-1 (`delivery_pass`, `note_sent`, `Step::Sent`), E3-M-1 (`eth_floor`), J7 / H3 / deposits at the floor, upgrade safety of all new state, E3-L-2 | `poc/E/` (cargo 5, upgrade 2, states saved by the 4949da0 and a37f54b builds, `.did` check) |

Severity scale and threat model: as `../README.md`. A "passing" PoC shows the weakness is present; it turns red once fixed.

## Verdict

**Not ready to launch. A round-4 fix round is needed.** Round 4 finds **3 High** and **1 Medium**, so the bar (0 C / 0 H / 0 M) is
not met. There is no Critical: no path was found that makes the canister sign what its judges did not build, or that drains custody
with nobody able to stop it.

**Every round-3 High and Medium is closed for the route it named** (table below). What is left open:

- **Two Highs are old.** They were missed or wrongly closed by earlier rounds:
  - **C4-H-1** is the 4663 half of round 1's B-M-4, which rounds 1–3 marked closed.
  - **E4-H-1** is fix round 1's deposit loop, with the same shape as C3-H-1.
- **B4-H-1** is round 2's retry loop, made four times as expensive by the operator's 500-block chunks.
- **C4-M-1** is the fixer's own flagged residual. It is worse than "a module can be credited another's ETH".

The contract change (`noteSent`) is sound, and upgrade safety holds.

## Counts of OPEN findings (distinct)

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| A contracts / go-live / launch (new) | 0 | 0 | 0 | 3 | 6 |
| B canister sale sample (new) | 0 | 1 | 0 | 2 | 3 |
| C H3 / way back / 4663 line (new) | 0 | 1 | 1 | 1 | 6 |
| D services (new) | 0 | 0 | 0 | 2 | 5 |
| E delivery record / pin floor / J7 / upgrade (new) | 0 | 1 | 0 | 1 | 4 |
| De-duplication: E4-L-1 = A4-L-1; E4-M-1 = C4-H-1 (E counts it 0 already); E4-I-1's second half = A4-L-2 | | | | −1 | |
| Earlier rounds, carried open (below) | 0 | 0 | 0 | 10 | (as their reports) |
| **Open** | **0** | **3** | **1** | **18** | **24** |

**The 10 carried Lows:**

- **Re-rated here:**
  - **A3-L-3:** partly; swapped D8 references pass.
  - **B3-L-1:** unchanged.
  - **C3-L-1:** partly; the gate still assumes 50 bps where the item cap binds.
  - **E3-L-2:** not fixed; gas drift deadlocks an arrival until the pot is topped up.
- **Unchanged from earlier rounds, not re-attacked beyond confirming the code:**
  - **A-L-2, B-L-5, C-L-5, D-L-3, D-L-6.**
  - **B-L-6**, the shared 4663 balance of the frog and Ethereum sides. It is narrowed by C3-L-3's fix, and C4-M-1 names its
    `eth_give_back` share.

The carried Infos are listed in each part (D3-I-2, D3-I-5, D3-I-6, C3-I-2, A3-I-*, B3-I-1, …) and are not recounted.

## Every open High and Medium

1. **C4-H-1 (High): the 4663 pin can move back, the home line then signs at a used nonce and waits for ever.**
   - **Code:**
     - `pin.rs:54-61` `choose_pin` accepts heads one bucket (1,024 blocks, about 100 s) apart, and `evmread::fast_pin` has no floor
       across ticks;
     - `ethnet.rs:217-218` `home_sign` takes the nonce at the tick's pin (`canister.rs:1197-1201`);
     - `ethflow.rs:2730-2747` `resolve_home` answers "nonce … is used but … not visible yet: waiting rather than guessing" for ever.
   - **Effect:**
     - The canister's one 4663 line is shared with the vault loop and the custodian through `evm_pending()`. It stays wedged until a
       timelocked upgrade.
     - No floor post, record, mark, mint or `noteSent` goes out. Every module goes stale within 1 h, so mints and burns revert for
       every holder.
     - A delivery whose `noteSent` cannot land is open to D3-H-1's late refund again after 30 days.
     - On the Relay leg, the same lower pin reads our own deposit as Relay's refund (`ethflow.rs:1328-1343`). The watch is dropped,
       and the real fill lands on Ethereum earmarked to nobody (E4-M-1, the same root: the operator's give-back then books almost the
       whole advance as a loss).
   - **Trigger:** no attacker. One provider's head falls back across a bucket boundary between two ticks, which a load-balanced RPC
     does.
   - **History:** round 1 named it (`../B-canister-judges.md:358`, "The 4663 line has the same shape"). Fix round 1 built
     `last_mined` / `nonce_floor` for the Ethereum line only (`ethflow.rs:589-594`, `:1836`). Rounds 2 and 3 marked B-M-4 closed,
     and E3-M-1's fix is Ethereum-only.
   - **PoCs:** `poc/C/r4_c.rs` `c4_h1` and `c4_h1b` (an ordinary floor post wedges the line), and `poc/E/audit4_nft_e.tests.rs` (the
     Relay leg).
   - **Fix:** a 4663 `(nonce, block)` floor kept in stable state, as `last_mined` is on Ethereum. Sign at `max(pin nonce, last + 1)`,
     refuse the flow's 4663 reads below the floor's block, and give `resolve_home` an exit. `looper.rs` and `redeem.rs` sign the same
     way on the same line.
2. **E4-H-1 (High): every deposit waiting to mint is its own H3 ask, so nine deposits of one collection in a day pause the canister.**
   - **Code:**
     - `ethflow.rs:2684-2717`: the comment says "one asked at a time", but the loop asks every `XPhase::Mint` deposit under its own key
       `mint:deposit:{tx}` (`:2702`) and goes on to the next after a wait;
     - `outflow.rs:197-199` counts each new key's ask toward the tripwire;
     - `canister.rs:1607-1613` pauses the canister on the trip.
   - **Trigger:** the NFT launch install's 3 mints a day per collection trip at 9 asked. With deposits on (D5) and J7's caps (16 / 22 /
     17 items), that is ordinary launch-week demand, with no attacker. A depositor can also do it on purpose with about 6 ETH of
     Azukis and lose nothing: the units are minted to them once resumed.
   - **Effect:** C3-H-1's. Paused: nothing posts floors, every module goes stale, and mints, burns and deliveries stop until the
     controller resumes. After the handover that means the timelock and k guardians.
   - **History:** fix round 1 (`ed76e9d`, C-L-2). The audit README (`../README.md:118`) says the opposite.
   - **PoC:** `poc/E/audit4_nft_e.tests.rs`, with the real limiter and the launch limits: `Tripwire{Mint, demanded 9, limit 3}`,
     paused after 40 minutes. The control with 8 deposits does not freeze.
   - **Fix:** ask at most one deposit per collection per pass (stop that collection's candidates after its first wait), or key the
     ask by collection and count, as C3-H-1's fix did for advances.
3. **B4-H-1 (High): one oversized log range stalls the sale scan for seven days and burns about 1.3 T cycles a pass.**
   - **Code:**
     - `ethflow.rs:950-1010`: on any failed chunk the chunk halves to 25 blocks, and the same start is asked again with the next unit
       of the pass's budget;
     - the budget is `13 × 2,000 / logs_max` = **52** at the operator's 500 blocks (`:945`, a37f54b; 13 before);
     - `ethnet.rs:370-386`: each failed read pays the 64 kB, 400 kB and 2 MB classes;
     - there is no failure memory, and the only exit is the window moving past the range (seven days).
   - **Trigger:** anyone holding one NFT of a collection. One transaction with about 3,300 self-transfers makes 25 blocks' `Transfer`
     logs exceed 2 MB. It costs about 0.002–0.05 ETH of gas at the measured base fees, and the NFT is not spent. The IC checks the
     size before the transform, so self-transfers count even though the transform drops them.
   - **Effect:**
     - Either no newer sale is read for a week (the floor is held, deposits stop, a first post waits), or the window's older half is
       never read.
     - The burn is about 97 T cycles a day per griefed collection on a 13-node subnet. At `EVIDENCE_CYCLES_FLOOR` (8 T) no
       collection's floor is posted, so every module goes stale and mints and burns revert for everyone. The operator can pay about
       $130–$340 a day per griefed collection, or upgrade.
     - The attacker renews it weekly.
   - **Origin:** the shape is round 2's (B-M-7). Nobody reported it.
   - **PoC:** `poc/B/b4_h1.rs.snippet`:
     - (a) walking back: 3 of 5 real sales read, 52 failed reads every pass;
     - (b) walking forward: 0 of 40 newer final sales read in a day, 51–52 failed reads every pass;
     - a control: 5 of 5 read.
   - **Fix:** at most two failed log reads a pass; at the minimum chunk, bisect to one block and mark a single block still over 2 MB
     as read-as-unreadable (the logs' B2-L-2); a cycles budget and page for the evidence pass.
4. **C4-M-1 (Medium): a way back let go after `BACK_LATE_SECS` can never be credited to its module.**
   - **Code:** `ethflow.rs:1437-1449`, `:2992-3021`, and the balance-rise reads at `:1329-1343` and `:1404-1411`.
   - **Why it is reachable:** Relay v2 refunds an unfilled order only after its deadline, and the canister watches only 25 h past it.
     A slow refund lands exactly in the gap.
   - **Effect:**
     - The operator's one exit is `returnAdvance(0)`, which books the whole advance as a loss (BAYC up to 7.353 ETH), taken from
       revenue, surplus, then the ETH-backed units. Meanwhile the ETH sits at our 4663 address and nothing can return it.
     - A payout that lands during another collection's watch is read as that order's refund. That collection bridges twice, and its
       real arrival on Ethereum belongs to nobody.
     - The same raw-balance rule decides `eth_give_back` for a watched forward `Stuck` (`:3012-3019`): the balance less the gas keep,
       capped at the advance. It can pay a module from other ETH at the address, including the 4663 gas pot.
   - **What holds:** nothing is spent twice, and no module is paid more than its own advance.
   - **Monitor:** no page for an unattributed balance.
   - **PoCs:** `poc/C/r4_c.rs` `c4_m1`, `c4_m1b`.
   - **Fix:** keep the order's watch data on the unwatched `Stuck`, and give the operator an exit that credits a payout attributed to
     that order. Best, read Relay fills by order id rather than by balance delta (E3-I-4). Measure a watched `Stuck`'s give-back against
     its watch, at a floored pin.

## Round-3 Highs and Mediums: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **D3-H-1** a delivery moved on before finality is never recorded; the late window refunds it | **Closed** | **Contract (A):** `noteSent` is custodian-only, `Resolved`-only and non-zero. It changes no stage or ledger number, and `markUndeliverable` still refunds after the grace. It gives the custodian no new power (`recordDelivery` is stronger). The wrapper is not a proxy. The selector `0x6d65f720` and topic match the canister. The fixer's 9 tests and the invariant are real and hold at BAYC's parameters. **Canister (E):** recorded from our own final receipt's `Transfer`. `noteSent` and `recordDelivery` name the version that mined, including after a re-price or cancel race. A failed `noteSent` is retried or marked. It is the only new act while paused. **Monitor (D):** exempt only for a delivery of that very item, read on its wrapper. Residuals: A4-L-2 (a noted delivery a reorg drops for good has no exit but an upgrade); D4-L-1 (the one-tick false SEV1, re-rated Low); E4-I-1; A4-I-1 (the indexer and site still offer a refund that now reverts); D4-I-1. **C4-H-1 can keep `noteSent` from landing at all**, so D3-H-1 returns while the 4663 line is wedged. |
| **C3-H-1** a waiting advance re-asks H3 under a new key each tick | **Closed** | `advance:eth:{col}:{n}`. `n` (the collection's committed, non-sweep buys) cannot change while an ask waits. A reverted advance retries under the same key. The worst honest demand is about 25 + 14 ETH, against 75 to trip. C4-I-1: a key left recorded by a reverted-then-dropped advance nets to one count per release. C4-I-5: the fixer's 44 ETH leaves out the Bitcoin side, which shares the global limit. **The same shape remains for deposit mints: E4-H-1.** |
| **B3-M-1** a flood of transfers starves the sale sample and the first post | **Closed for its route** | A rank-1 real sale waits only for older moves of distinct tokens, bounded by the NFTs a flooder holds (16 per 20 min of delay). Truncation needs 512 distinct tokens ahead. `judged` cannot overflow from reads at the runner's cadence. A prefix collision needs about 2^64 work. Sales are never evicted by non-sales. Residuals: B4-L-1 (dropped ranges wait for a queue a cheap flood keeps full), B4-L-2. **A cheaper route to the same effect is B4-H-1.** |
| **C3-M-1** the way back has no exit | **Closed as a stall** | Every `Back*` phase has a bounded exit, and none spends ETH that is not the buy's. Residuals: **C4-M-1** (Medium, above) and C4-L-1: the bound is the order's own deadline, 7 days in Relay's measured quotes, and every collection's new buys wait that long. |
| **E3-M-1** arrival reads at a pin with no floor | **Closed for Ethereum; not for 4663** | `eth_floor` floors every Ethereum balance read of the flow and cannot wedge. The 4663 sibling is **C4-H-1** (reproduced as E4-M-1). |

**Also verified:**

| Item | Verdict |
| --- | --- |
| **F3-1** (the monitor's blob encoding) | Fixed, with no live sibling. A canister-encoded state through the monitor's IDL and `plain()` is read correctly by every consumer; `ops` does not use `plain()`. |
| **The keyless-provider go-live rule** | Holds for the named keyed providers, host case included, and prints no key. Its gaps are A4-L-3: short, split, percent-encoded or subdomain keys; a subdomain key printed in the `ok` line; the canister's pair not checked for two hosts. |
| **The argument diff's redaction** | Holds (D4-I-2: two unreachable gaps). |
| **Upgrade safety** | Holds. States saved by the 4949da0 and a37f54b builds themselves, with every phase and nested option, and e04f49b's, restore into HEAD with nothing lost. Every new field is `opt` with a default, and the only new case (`Step::Sent`) is appended. The `.did` matches every case. The probe on a 4 MiB state takes 438 ms. E4-I-4: the re-encoded fixture holds none of round 3's new cases. |

## The operator's round (a37f54b), not reviewed before

| Change | Verdict |
| --- | --- |
| **J7:** per-collection caps, the global cap the sum | Correct as decided. But the global 24 deliveries a day cannot move J7's 61 items within the timelock's 72 h less a day. 48 can and 13 cannot, and `exit_fits` is checked per collection only (**A4-L-1** = E4-L-1). E4-I-3: `first_global` lets the global start at the sum of any number of 15 ETH collections. |
| **Deposits counted at the floor** | Correct. E4-I-2 / A4-I-6: a deposit admitted before its module's first post counts 0 wei (bounded by the item count). A4-I-5: a buy is booked at its advance, so BAYC's two slots hold only until the floor is up about 4 %. |
| **12 / 24 deliveries a day** | Consistent per collection. Globally: A4-L-1. |
| **The advance is the buy's bound**, grossed up for Relay's judged 3 % | Holds. Where the item cap binds (Milady, Azuki, MAYC, Pudgy) the gate still assumes 50 bps (C3-L-1, carried Low). A4-I-3: `Pooled.t.sol` models the advance as min(item cap, 8 ETH). |
| **Per-collection leftover and the gas-pot sweep** (`fundReserve`) | Hold. A leftover is booked once, to its own collection. A sweep never starts while ETH at our address is someone's. Gas spending can leave the booked leftover above the real one (folded into C4-M-1). C4-I-6: `fundReserve` repays a bootstrap loan first. |
| **500-block scans** | Canister: **B4-H-1** (×4 the per-pass cost of the stall), B4-I-3. Monitor: **D4-L-2** (the incremental deposit scan never re-reads a block its provider answered short; the old full re-read healed it). |

## The launch configuration (item 3)

| Setting | Verdict |
| --- | --- |
| BAYC at 10 ETH, `convBps` 1,000, `reserveBps` 0 | Safe: within `StackChecks`, and the books and invariants hold (part A, 200,000 random calls). |
| H3 at 25 ETH a day | Fine for advances (C3-H-1 closed). |
| Deposits on (D5) with J7's item counts | **Make E4-H-1 an ordinary launch-week event.** Do not launch deposits on until it is fixed. |
| 61 items against 24 deliveries a day | Breaks the stated "every item can leave within the delay" (A4-L-1; `phase1Trust.ts:20` and `launch-gates.md:322` state it). Either lower J7's counts, or raise the global delivery limit so that 61 fit 48 h. |
| The canister's provider pair | Must be two keyless hosts. Nothing checks it by host (A4-L-3). |
| The D8 references | Still not held against a source (A3-L-3). |
| The launch monitor template | Now Dwellir + `xrpc.cl/eth`, which fixes D3-L-3. Its deposit scan has D4-L-2. |

`phase1-nft.md` still has stale stage-0 text:
- §5's "canister H3 3 items/day, 10 ETH/day" and "NFTs in custody (≤ 3 per collection)";
- D1's "3 items and 10 ETH per collection … 6 items and 10 ETH in all";
- D4 (A4-I-4).

## The items the fixer left open, re-rated

| Item | Re-rating |
| --- | --- |
| **C3-L-1** (the gate's 50 bps where the item cap binds) | **Low**, partly fixed. Bounded by the 48 h way back and the 24 h cooldown to about two Relay fees per collection per day. |
| **E3-L-2** (gas drift deadlocks an arrival with the way back) | **Low**, not fixed. Nothing in the delta makes it worse; a gas-pot top-up clears it. |
| **A3-L-3's D8 reference swap** | **Low**, partly fixed. Swapped references between collections with close floors (Milady / MAYC) still pass. So do placeholders not written as `<…>`. |
| **D3-I-2** (a foreign transaction at a `CancelFill` nonce pages no SEV1) | **Info**, unchanged. It needs the canister's key. |
| **The one-tick false SEV1** (a mined delivery still `Sent`) | **Low (D4-L-1).** It is narrower than D2-M-1 and clears at the next tick, which comes when the runner calls, up to 600 s. It can mute a real page for the same token for at most 6 h. |
| **The late-Relay-payout residual** | **Medium (C4-M-1).** |
| **The custodian-vanishes trade-off of `noteSent`** | Acceptable, as the fixer describes. Only a custodian gone after `noteSent` and before the record leaves that ticket open. Its ledger stays consistent (`held` and `openTickets` both keep it). The ticket that matters is A4-L-2's (a reorg after the note), which is the canister's to handle. |

## Recommended order for round-4 fixes

1. **C4-H-1:** a 4663 nonce and block floor for the home line (and `looper.rs` / `redeem.rs`), an exit for `resolve_home`, and the
   flow's 4663 reads floored.
2. **E4-H-1:** one deposit ask per collection per pass, or a stable key.
3. **B4-H-1:** bound failed log reads per pass, bisect to one block, and mark it unreadable; a cycles budget for the evidence pass.
4. **C4-M-1:** an attributed exit for a late payout (keep the watch data, or read Relay by order id); measure a watched `Stuck`'s
   give-back against its watch.
5. Then the Lows:
   - A4-L-1 (a global `exit_fits` and golive reading the canister's limits, A4-I-2);
   - A4-L-2 (note at finality, or a canister path from "mined, then gone" to a mark);
   - D4-L-2, A4-L-3, C4-L-1, B4-L-1.

## Test bar (re-run by this review at `1eb5788`, PoC files out of the tree)

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites) |
| Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **621 passed**, 0 failed |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed |
| ops | **56 passed**; `tsc --noEmit` clean |
| monitor | **190 passed** |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | **72 passed**; `tsc --noEmit` clean (run after its `node_modules` was reinstalled with npm, and once more on its own) |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 checks ok on the v3–v6 stack; 536 on the five-collection NFT stack; 652 on the launch go/no-go with `--canister --canister-evm --launch --monitor-config --install-arg`; every refusal `ok`) |

Every number in `fixes-round3.md` §11 reproduced. These suites are green *with* B4-H-1, C4-H-1, E4-H-1 and C4-M-1 present.

**Reproducible build:** `docker build --no-cache --platform linux/amd64` of `autobuyer/Dockerfile.build`, on a `git archive` export
of `1eb5788`'s `autobuyer/`. It gave **identical** hashes to `fixes-round3.md` §11:

```
cbe337d84c1120e32eaade5f01ee496940de129cf7bfb5f616a13670a03fb360  buyer.wasm           (6,753,176 bytes)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

**Removed afterwards:** the export directory, the build image, the extracted wasm, the dry run's deployment files (it removes its
own) and the anvils. Docker's shared build cache was left alone, and nothing was installed.
