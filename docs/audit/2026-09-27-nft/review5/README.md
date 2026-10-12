# Review round 5, 2026-09-28: the Ethereum NFT side after fix round 4 (branch `nft-side`)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1dc2a61`.

**Read:**
- `../README.md`, `../review2/` .. `../review4/` (README, the part reports, `poc/`);
- `../fixes-round1.md` .. `../fixes-round4.md`, `../fixes-round4-poc/`, `../bayc10-decisions.md`;
- `docs/v7/phase1-nft.md`, `docs/v7/phase1-nft-pooled.md`;
- the code of `git diff 2735dac..HEAD` (fix round 4: `57032dd`, `8018070`, `f47f27c`, `548f214`, `1dc2a61`). Earlier code was
  read where a finding goes back further.

**Method:** as round 4. Six scoped parts. Every round-4 fix was attacked, and the fixer's claims were checked against the code and
against PoCs, not taken as given. The new code was read line by line. One PoC per new finding where feasible, and a recommended
fix for each. The operator's decision on A4-L-1, option (c) with the scale-up plan (install at 48 items, queue the outflow raise at
launch, raise the caps to 61 about two weeks later), arrived during the review and is reviewed in part A.

**No code was modified. The only files are these review documents and their PoCs.** Nothing was pushed, deployed or signed, and no
key was used.

- **Chains written to:** local anvils only: the dry run's fork of 4663 and the Seaport fork proof's fork of Ethereum.
- **Network use (read-only):** the fork upstreams and the reproducible build's crate fetch.
- **PoCs:** every one ran in a private copy of its harness under `/tmp/rook-r5/<part>/`, never in the worktree. The copies are
  removed.
- **A cleanup mishap:** one part's cleanup ran `rm -rf /tmp/rook-r5` while the other parts were still working. It deleted their
  private copies and the first test-bar run's logs. Every part re-copied its harness, checked it with `diff -rq` against the
  worktree (identical but for the PoC file), and re-ran every PoC. The saved outputs are from those re-runs. The test bar and the
  dry run were re-run afterwards with their logs kept elsewhere; the numbers below are from those re-runs.

| Part | Report | Scope | PoCs |
| --- | --- | --- | --- |
| A | `R5-contracts-deploy.md` | A4-L-1: `validate_custody`, `exit_fits`, golive-check's `EXIT_PY` / `launchexit` / `canexit`; the options (a)–(d); **the operator's option (c) and its queue plan** (timelock queue rules); A4-L-3, A3-L-3, A4-I-2; the launch configuration; contracts unchanged | `poc/A/` (canister grid, timelock queue, golive pieces) |
| B | `R5-canister-floor.md` | B4-H-1: `scan_read`, `SaleScan.bad` / `skipped`, the marketplace-log fallback, `ScanPass` budgets, the cycles floor; B4-L-1, B4-L-2, B4-I-3, B3-L-1 | `poc/B/` (cargo 6) |
| C | `R5-canister-wayback.md` | C4-M-1: `LateOrder`, the write-off, `eth_credit_payout` / `judge_payout`, `eth_give_back`; C4-L-1, C3-L-1, E3-L-2, B-L-6 | `poc/C/` (cargo 5) |
| D | `R5-services.md` | D4-L-1, D4-L-2, D4-I-*; the C4-M-1 pages; the IDLs against the new state; the Trust page; D-L-3, D-L-6, D3-I-2 | `poc/D/` (canister encoder, node 12, ops 1) |
| E | `R5-canister-flow-upgrade.md` | E4-H-1: `mint_key`, one deposit per collection a pass; the 17 H3 ask sites and their source-reading test; the A4-L-1 check at install and upgrade; A4-L-2; E4-I-2; upgrade safety of all new state and fixtures | `poc/E/` (cargo, upgrade) |
| F | `R5-canister-pin.md` | C4-H-1: `pin::PinFloor`, `PIN_FLOORS` at upgrade, `EthNet::pin`, the nonce floors, `home_floor`, `resolve_home`'s 1 h drop; every chain read surveyed for a floor | `poc/F/` (cargo 8) |

Severity scale and threat model: as `../README.md`. A "passing" PoC shows the weakness is present; it turns red once fixed.

## Verdict

**Not ready to launch. A round-5 fix round is needed.** Round 5 finds **4 Mediums** and no Critical or High, so the bar (0 C / 0 H /
0 M) is not met.

**Every round-4 High is closed** (table below). **The round-4 Medium, C4-M-1, is only partly closed.** Two of the new Mediums come
straight out of round-4 fixes: F5-M-1 from C4-H-1's exit, and C5-M-1 from C4-M-1's write-off. B5-M-1 and B5-M-2 are B4-H-1's stall,
still reachable at a higher price.

No path was found that makes the canister sign what its judges did not build. None drains custody with nobody able to stop it,
and none freezes the canister with no attacker.

## Counts of OPEN findings (distinct)

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| A exit rule / option (c) / go-live / launch (new) | 0 | 0 | 0 | 3 | 5 |
| B canister sale sample (new) | 0 | 0 | 2 | 2 | 3 |
| C way back / payout credit (new) | 0 | 0 | 1 | 2 | 3 |
| D services (new) | 0 | 0 | 0 | 2 | 3 |
| E H3 keys / exit check / upgrade (new) | 0 | 0 | 0 | 2 | 3 |
| F pin floors / home line (new) | 0 | 0 | 1 | 1 | 5 |
| Earlier rounds, carried open (below) | 0 | 0 | 0 | 11 | (as their reports) |
| **Open** | **0** | **0** | **4** | **23** | **22** |

**The 11 carried Lows:**

- **Re-rated here:**
  - **C4-L-1:** stays Low, longer. The Relay leg is held until the operator writes the order off: at least deadline + 25 h (about
    8–9 days in Relay's quotes), with no automatic release. The write-off is what triggers C5-M-1.
  - **C3-L-1:** unchanged.
  - **E3-L-2:** unchanged.
  - **B-L-6:** partly fixed. `eth_give_back` no longer pays the raw balance, but the automatic 4663 reads still take the frog side's
    inflows as a refund, and the payout judge counts the frog side's ETH as free (C5-L-1).
  - **B4-L-2:** partly fixed. The hull still merges disjoint ranges, and a block already resolved gets its bisection paid again.
  - **B3-L-1:** stays Low. A provider disagreement on the fallback read now loses that block's sales for good after 3 retries.
- **Unchanged, not re-attacked beyond confirming the code:** A-L-2 (the wrapper's `deployer` is the deploy key), B-L-5, C-L-5,
  D-L-3 (the runner's 7,200-block window), D-L-6 (the canister never reads `custodian()`).

**Closed this round:** A3-L-3 (the D8 swap is refused; the placeholder residue is A5-I-3), and every round-4 Low the fixer marked
fixed (A4-L-1's check, A4-L-2, A4-L-3 with the residue A5-L-3, D4-L-1, D4-L-2, B4-L-1).

## Every open Medium

1. **F5-M-1 (Medium): the home line's 1 h drop redoes a step whose transaction actually mined.**
   - **Code:** `ethflow.rs:3138-3162` (`resolve_home`, the drop), `mined_home` `:3171-3205`, `home_act` `:2938-2946`, `:2948-2997`,
     `:3030`.
   - **Why it is reachable:** the pin floor now keeps a lower pin from reusing a nonce. So the drop fires only in one case: our own
     transaction mined, and no provider shows its receipt for an hour. The trigger is provider failure, with no attacker.
   - **Why "seen nowhere for 1 h" is weak:** it is really two samples an hour apart. `Unsure` and `Pending` reads in between do not
     restart the clock.
   - **Effect:** the step is taken back as a revert and signed again. `fixes-round4.md` §1 and §9 call 1 say the contracts guard
     every redo. They do not guard the value-bearing ones:
     - a Relay deposit is paid again from other ETH at the shared address, and the first order's fill belongs to nobody;
     - `fundReserve` is paid again;
     - an `advance` is orphaned: the module's advance stays open, and the buy sits in `NeedAdvance`, holding the Relay leg for
       every collection with no operator exit;
     - a `returnAdvance` leaves the buy `Returning`, holding the leg with no exit;
     - a dropped `Record` goes `Gone` without releasing the item from the custody book.
   - **Related:** the same drop is E5-L-1's trigger for a deposit mint.
   - **PoC:** `poc/F/r5_f.tests.rs` (8 tests on the fixer's `audit_nft_r4.rs` harness with three switches).
   - **Fix:**
     - before redoing, identify positively what used the nonce, or read each step's own state back (the module's advance, the
       Relay order, the ticket's stage);
     - reset the clock on any other view;
     - give `NeedAdvance`-with-an-open-advance and `Returning` an operator exit.
2. **C5-M-1 (Medium): the write-off frees the Relay leg, so a late payout is still read as the next order's.**
   - **Code:**
     - the write-off: `ethflow.rs:3412-3415` (way back) and `:3456-3462` (forward), where `b.last.take()` makes `in_relay` false;
     - `in_relay` at `:230-237`, and `buy_pass` starting the next buy at `:1550`;
     - the balance-rise reads at `:1621-1624`, `:1628`, `:1667` and `:1713`;
     - the refused credit at `:3557-3566`.
   - **Why:** the fixer's "no other order is watched while it could land" holds only until the write-off. The code's own comment says
     "the next order's watch starts only now". `eth_credit_payout` exists exactly for a payout that comes after the write-off.
   - **Effect:** C4-M-1's second half, unchanged in effect. In the PoC, Azuki reads Milady's late payout as its refund and bridges
     twice. The operator's `eth_credit_payout` for Milady is then refused, Milady's module gets nothing, and 0.51 ETH at our Ethereum
     address belongs to nobody.
   - **PoC:** `poc/C/r5_c.rs`.
   - **Fix:** while a written-off order is uncredited, keep the Relay leg held for new orders and sweeps. Better, attribute every
     leg by transaction, not by balance rise.
3. **B5-M-1 (Medium): a steady rate of oversized blocks outpaces the bisection.**
   - **Code:** `ethflow.rs:1302-1384` (`scan_read`), the walks at `:1093-1162`, `ScanPass::can` at `:1281-1283`; `ethnft.rs:124`.
   - **Why:** isolating one oversized block costs about 9 failed reads. Each walk may fail only twice a pass. So a steady supply of
     oversized blocks outpaces the scan.
   - **Effect:**
     - (a) A burst of 126 oversized blocks, one block apart, costs about 0.13–6 ETH once. No newer sale is read for 504 passes, a
       week, and the last ~22 h of posts are held.
     - (b) One oversized block a pass costs about 0.07–3.6 ETH a day. The forward walk is 30,406 blocks behind after a week.
     - (c) At launch or after a window reset, one every 50 blocks means the first read never lands: no sales, no first post.
   - **What holds:** the floor is still posted from what was read (B4-I-1's fix), and no fake sale is produced.
   - **PoC:** `poc/B/r5_b.rs.snippet`, with a control.
   - **Fix:**
     - count a size refusal toward the block's isolation, not the walk's failure budget; or bisect the walk's own range with a
       separate budget;
     - let the forward walk skip past an unresolved range and come back to it (keep it in `bad`);
     - page when the forward walk lags the head by more than a set margin.
4. **B5-M-2 (Medium, conditional on the provider): a provider's own refusal of a dense range still takes the old path.**
   - **Code:** `ethflow.rs:1349`, `:1381`, `:1107-1111`; `canister.rs:749-751`.
   - **Why:** an error returned by the provider itself (a result-count cap, a "range too large") is not treated as a size refusal. It
     goes down the old halve-and-retry path with no memory.
   - **Effect:** two dense blocks (about 60 M gas, 0.004–0.09 ETH) stall the forward walk for the whole window.
   - **Condition:** this depends on the canister's keyless provider pair actually capping results, which was not verified against
     the real providers. If neither caps, it does not arise. It stays Medium until that is measured, since the canister's pair is a
     configuration choice, and a provider's result policy is not under the canister's control.
   - **PoC:** `poc/B/` (`dense_blocks` switch).
   - **Fix:** treat a provider's range or size error as a size refusal (the same memory and bisection), and give it the same bound.

## Round-4 Highs and Mediums: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **C4-H-1** the 4663 pin moves back; the home line signs at a used nonce and waits for ever | **Closed** | No 4663 read anywhere in the canister is taken at a pin below one already read, and nothing is signed at a nonce below one already read. The floor is raised only after every provider agreed on the pin. A far-future head cannot raise it (`choose_pin` refuses heads more than a bucket apart and pins under the lowest). A lagging provider only causes refused ticks, never a permanent refusal. The parts sharing the floor stall each other for at most a tick. The fixer's claim that only `nftnet`'s `nft_*` operator endpoints read unfloored holds for pinned reads; the other unfloored reads are finality reads and the lookup of a transaction by hash, and none decides a spend. E4-M-1 is closed on the canister (its PoC's harness raises the balance without moving the pin or nonce, which a real lower pin never shows). Residuals: **F5-M-1** (the new exit), F5-L-1 (one Ethereum floor per canister, not per chain), F5-I-1 (the loop, `redeem.rs` and the frog side still wait with no exit on the same trigger), F5-I-2..5 |
| **E4-H-1** every waiting deposit mint is its own H3 ask | **Closed** | `mint:eth:{col}:{n}`. `n` goes up only when a mint goes out and down only when one is read as reverted or dropped. Buys and deposits never race for the key, and a key is never reused after its window rolls. One deposit candidate per collection per pass. Review 4's nine-deposit PoC fails its attack assertion. A variant with two purchases and nine deposits of one collection over four days, real limiter, launch limits, never froze (at most 4 asks in any window). **The source-reading test of the 17 ask sites is complete today** (an independent grep of `src/` finds exactly those 17, each keyed per logical outflow), **but weak as a guard** (E5-I-2: it counts the string `outflow_check(&` in seven named files, and passes when `mint_key` regresses to E4-H-1's shape, when a key survives only in a comment, or when a site is added elsewhere). Residuals: E5-L-1, E5-I-1 |
| **B4-H-1** one oversized log range stalls the scan for seven days at 1.3 T cycles a pass | **Closed as reported** (one oversized block: 7–9 failed reads, once; 60 of 60 newer sales read by pass 4) | The marketplace fallback cannot produce fake sales: every transaction it finds is judged by its receipt and by `counted_sales`' rules, like the normal path. Posting when cycles are low is safe, since it uses the canister's own judged sales in the current window. **The cycles figure is not a bound.** A failed read counts as 4.928 MB (about 51 B cycles; about 26 B actually charged). The 12 MB budget is checked only before each read, and a read can do the transfer read and the fallback. Receipts (up to 7.4 MB) are outside it. The worst case per collection per pass is about 30 MB counted, about 0.32 T cycles, and about 69 T a day for three collections at 72 passes. The attacker actually drove 15.0–15.4 MB a pass, about 11.5 T a day per collection. One constructed pass reserved 19.71 MB, above the fixer's own test bound of 16.93 MB (B5-L-1). Residuals: **B5-M-1, B5-M-2**, B5-L-1, B5-L-2 (the fallback is filtered by marketplace, not collection: a padded block hides its real sales), B5-I-1..3 |
| **C4-M-1** a late Relay payout is nobody's, or read as another order's | **Partly closed** | The watch stays open past the deadline. The write-off is refused until a day past deadline + grace. A payout landing before the write-off goes back to its own module. Every `eth_give_back` path now pays only ETH measured as the buy's own. There is one credit per written-off order and one per transaction hash. Upgrade-safe. **Not closed:** after the write-off, the next order's watch reads the late payout (**C5-M-1**). The judge does not bind a transaction to the order (C5-L-1): `orderData.solver` is Relay's one shared solver, and any call with the order id in its input passes at the operator's stated amount, our own `depositNative` included. A credit is recorded before a fallible read (C5-L-3). Relay's real payouts likely do not match the judge's rules (C5-I-1: in the fixture `requestId` differs from `orderId`, and refunds are internal transactions): measure one real fill and one real refund each way before launch |

**Round-4 Lows and Infos the fixer marked fixed:**

| Item | Verdict |
| --- | --- |
| **A4-L-1** (the whole custody against the exit rule) | **The check is fixed.** The canister and golive-check agree on all 392 grid rows that reach the rule. It runs at install and at every upgrade. `ASSUMED_DELAY_SECS` equals the timelock's production floor. A refused upgrade traps atomically. Residuals: A5-L-1 (checked on named caps, not on held custody), A5-L-2 (deadline-first delivery leaves items for the margin day in every option), E5-L-2 (the rule ignores the tripwire; the stage-0 defaults, not the launch numbers) |
| **A4-L-2** (`noteSent` at finality) | **Fixed.** It adds only the minutes to finality to the 30-day exposure. A mark while paused signs nothing on Ethereum and moves no custody |
| **A4-L-3** (keyless providers by host) | **Fixed for the round-4 cases.** Residuals: A5-L-3 (a key split 11/11/10, an 11-character key, `wss://`, a key subdomain printed in the host-count FAIL line, a trailing-dot host counted twice) |
| **D4-L-1** (a Sent delivery's exemption) | **Fixed; fails closed.** A delivery is exempt only if its receipt has status 1 on every provider and holds this collection's `Transfer` from the custodian to the destination. Every bad shape tried pages. Residual: D5-I-1 (only the newest version of a re-priced delivery is read) |
| **D4-L-2** (deposit scan re-read) | **Fixed.** A deposit read twice is one entry and one page. Residual: D5-I-2 |
| **D4-I-1** | **Fixed, but it causes D5-L-1**: a false SEV1 when an honest record lands mid-poll |
| **B4-L-1**, **B4-I-3**, **E4-I-2** | **Fixed** |
| **B4-I-1** | **Partly:** B5-I-3 (between 8 T and 5 T cycles, the scan pause is a note, never a page) |
| **D4-I-2**, **D4-I-3** | **Partly** (`R5-services.md`) |
| **E4-I-4** | **Fixed,** but the round-4 fixture leaves out `Inbox.payouts` (E5-I-3) |

## Upgrade safety (all new saved state)

**Holds.**

- **The fields:** every new saved field is `opt` with a `#[serde(default)]`:
  - `pin_floors`, `home_floor`, `HomeTx.used_since`;
  - `SaleScan.bad` / `skipped`;
  - `EthBuy.late` (`LateOrder`);
  - `PayoutClaim` / `Inbox.payouts`;
  - `EthDeposit.solver`.
- **The interface:** the `.did` matches the types and is service-compatible with `2735dac`'s. No variant case was added.
- **Old states:** states written by the `4949da0`, `a37f54b` and `1eb5788` builds restore with nothing lost. The floors start at 0.
  `pre_upgrade` gained no trap.
- **The fixtures are genuine:**
  - `saved-1eb5788-rich.hex` was regenerated byte for byte from a `git archive` of `2735dac`;
  - the round-2 fixture was re-encoded only because its test compares bytes, and all four historical encodings decode into round
    4's types with nothing lost;
  - `monitor/test/d4-state.hex` is byte-identical to review 4's canister-encoded fixture.
- **The services:** the monitor's and the runner's IDLs decode the round-4 state and a new round-5 state written by the `1dc2a61`
  encoder, and are subtype-safe against an older or a newer canister.
- **Probe:** the largest round-4 state, 5.44 MB, decodes in 0.61 s (native release).
- **Residuals:**
  - F5-I-2: the first tick after an upgrade from a pre-round-4 build has no floor. That gives the three other 4663 signers a
    one-tick window to reuse a nonce, and they have no exit.
  - F5-L-1: the Ethereum floor survives an upgrade that moves `eth_nft` to another chain.
  - Part E's note: a canister installed by an earlier build with J7 / 24-a-day numbers can take this build only in an upgrade that
    also changes those numbers.

## The launch configuration and the operator's option (c)

| Setting | Verdict |
| --- | --- |
| BAYC at 10 ETH, `convBps` 1,000, `reserveBps` 0 | Unchanged; contracts untouched (`git diff 2735dac..HEAD -- src/ test/` is empty). OK |
| H3 at 25 ETH a day; timelock 72 h | Consistent. `ASSUMED_DELAY_SECS` equals the production floor. The launch file's outflow is never compared with the canister's `custody_outflow()` (A5-I-1) |
| Deposits on (D5) | **E4-H-1's condition is met.** Deposits are bounded by the smaller caps under (c). E5-L-1 (a deposit mint dropped by F5-M-1's path wedges its collection's later deposits) goes with F5-M-1's fix |
| D1–D9 | D8 held per collection (A3-L-3 closed). D6: the A5-L-3 residuals. B5-M-2 depends on D6's providers |
| **The committed `nft-pilot.json`** | **Still J7's 61.** The real go/no-go read FAILs on it: the intended fail-closed. The dry run passes only on the file it generates. Commit the 48 split before launch |
| Docs and site | The Trust page (`phase1Trust.ts:20`), `launch-gates.md` G7 row 0 (`:322`), `phase1-nft.md` §5 and `phase1-nft-pooled.md` §6 J7 still say "the operator chooses". G7 row 1 (`:323`) sits below J7's stage 0, and `ops/src/stage.ts` disagrees with it (A5-I-4). The Trust page overclaims the way back ("never read as another's": C5-M-1) and the oversized-block route (Seaport 1.5 / 1.6 and Blur v2 only) (D5-I-3) |

**The A4-L-1 options** (part A, `R5-contracts-deploy.md`):

- **(a) 48 items at 24 a day (13 / 17 / 13 / 3 / 2):** safe as a launch state. The canister and golive-check both pass it. It loses
  13 items of capacity. Under the worst redemption order, 5 Azukis leave on the margin day (A5-L-2).
- **(b) a global cap of 48 over J7's counts:** the canister accepts it, but golive-check's own J7 read refuses it
  (`golive-check.sh:410-417`), and it changes J7: a binding total lets one collection crowd out the others. Not recommended.
- **(c) install at (a), raise `global_items` to 31 or more, then caps back to 61:** recommended, as the operator chose. The canister
  enforces the order:
  - a caps-first upgrade is refused;
  - outflow first, then caps, is accepted;
  - lowering the outflow under 61 is refused;
  - 24 → 96 is one step, and 97 is refused.

  Every intermediate configuration fits the rule. Day-1 exposure of a compromised signing path rises from about 43 to 49 ETH of
  items.
- **(d) a longer delay:** needs code changes in the buyer, golive-check (hard-coded `days = 2`) and the timelock floor, plus a
  timelock migration, for one item of margin. Not recommended.

**The operator's (c) plan, checked:**

1. **The install at 48 is safe and enforced.**
   - The canister panics in `init` on 61 at 24.
   - golive-check refuses 61 on the launch file and on the canister's pair, and passes 48 on both. The dry run shows both.
   - Not enforced: that the launch file's outflow is the canister's (A5-I-1).
2. **The order (outflow first, caps second) is enforced by the canister at execute time,** on the argument as executed.
   - **But every upgrade argument is a whole `Init`.** One upgrade that raises both at once is accepted (61 at 31 fits). An outflow
     raise built from the committed `nft-pilot.json` (still 61) would do exactly that and skip the two-week gate.
   - The guardians' only signal is the monitor's `argdiff`. It pages the planned raise as CRITICAL / SEV1 "LOOSENS" and shows caps
     changes beside it.
3. **Queuing the raise at launch is allowed and safe.**
   - Its own argument is checked when it executes.
   - It must be proposed after the launch install executes: a raise queued beside a timelocked install goes stale when the install
     executes.
   - The timelock cannot reorder the two steps:
     - one proposer has one pending proposal;
     - any executed target action makes older proposals stale, a refused one included;
     - no path skips the 72 h (pause / start / resume install nothing);
     - a withdrawn and re-queued proposal waits the full delay again;
     - any other upgrade in the 72 h forces the raise to be re-queued.
4. **Conditions:**
   - **(i)** Fix A5-L-2 (backlog-first delivery) before the caps go back to 61. Otherwise step 1 must also raise
     `per_collection_items` to at least 22, or `global_items` to at least 42. At 61 / 31 with deadline-first delivery, 9 Azukis are
     left for the margin day in the worst order.
   - **(ii)** Make the raise an outflow-only diff.
5. **Runbook and golive-check changes needed** (part A §5):
   - commit the 48 split and update the Trust page, G7 and the two design docs;
   - a golive-check `--proposal` mode that decodes the pending argument, requires an outflow-only diff (step 1) or caps-only diff
     (step 2), and runs the exit rule on it;
   - a staged launch file, and hold the canister's whole `custody_outflow()` against it;
   - `argdiff` annotates a proposal that fails the exit rule, and pages a lowering that held custody would not fit (A5-L-1);
   - the runbook: queue after the install and the handover; no other target proposal in the 72 h; re-queue after any security
     upgrade; the caps raise as a separate proposal carrying `global_items` of at least 31.

## Recommended order for round-5 fixes

1. **F5-M-1:** positive identification (or a state read-back) before a dropped home step is redone; reset the clock on any view;
   operator exits for `NeedAdvance` with an open advance and for `Returning`. E5-L-1 goes with it.
2. **C5-M-1:** keep the Relay leg held while a written-off order is uncredited (or attribute legs by transaction). With it, C5-L-1:
   bind a credited transaction to the order (after its deposit, not from us, the solver alone not enough) and count only ETH that
   is no other flow's. Also C5-L-3, and measure real Relay payouts (C5-I-1).
3. **B5-M-1 / B5-M-2:** a size refusal's isolation outside the walk's failure budget, a forward walk that can pass an unresolved
   range, provider range errors treated as size refusals, a lag page; with them B5-L-1 (a real byte bound, receipts included) and
   B5-L-2 (the fallback filtered by collection).
4. **Launch configuration:** commit (c)'s 48 split and the docs; A5-L-2 before the caps raise; golive-check `--proposal`; A5-I-1.
5. Then the Lows: A5-L-1, A5-L-3, D5-L-1, D5-L-2, E5-L-2, F5-L-1, and E5-I-2 (make the H3 site test parse keys, not strings).

## Test bar (re-run by this review at `1dc2a61`, PoC files out of the tree)

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites) |
| Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **647 passed**, 0 failed |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed |
| ops | **57 passed**; `tsc --noEmit` clean |
| monitor | **195 passed** |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | **72 passed**; typecheck clean |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 checks ok on the v3–v6 stack; 536 on the five-collection NFT stack; 655 on the launch go/no-go read, on the file the dry run generates with option (a)'s split; 46 refusals `ok`, the operator's J7 / 24 pair among them) |

Every number in `fixes-round4.md` §11 reproduced. These suites are green *with* F5-M-1, C5-M-1, B5-M-1 and B5-M-2 present.

**Reproducible build:** `docker build --no-cache --platform linux/amd64` of `autobuyer/Dockerfile.build`, on a `git archive` export
of `1dc2a61`'s `autobuyer/`. It gave hashes **identical** to `fixes-round4.md` §11:

```
429e7ad6f92d3fcf16e423b8b9acfd29e3c3ece587ea2a34cd947fd441e7f730  buyer.wasm           (6,920,875 bytes)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

**Removed afterwards:**
- the export directory, the build image and the extracted wasm;
- the private PoC copies under `/tmp/rook-r5/`;
- the dry run's deployment files (it removes its own; a stray `deployments/stack-9466301.json.btc` left by the interrupted first
  run was removed);
- the anvils.

Docker's shared build cache was left alone, and nothing was installed.
