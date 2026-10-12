# Review round 6, 2026-09-28: the Ethereum NFT side after fix round 5 (branch `nft-side`)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `e04fa16`.

**Read:**
- `../README.md`, `../review2/` .. `../review5/` (README, the part reports, `poc/`);
- `../fixes-round1.md` .. `../fixes-round5.md`, `../fixes-round5-poc/`;
- `docs/v7/phase1-nft.md` (with the §5.1 runbook), `docs/v7/phase1-nft-pooled.md`;
- the code of `git diff 8ef82d6..HEAD` (fix round 5: `e811911`, `d2d8c66`, `4fed345`, `536860c`, `e04fa16`). Earlier code was read
  where a finding goes back further.

**Method:** as round 5. There are six scoped parts, plus a provider probe (P) and the test bar (T). Every round-5 fix was attacked,
and the fixer's claims were checked against the code and against PoCs, not taken as given. There is one PoC per new finding where
feasible, and a recommended fix for each.

**No code was modified. The only files are these review documents and their PoCs.** Nothing was pushed, deployed or signed, and no
key was used.

- **Chains written to:** local anvils only: the dry run's fork of 4663 and the Seaport fork proof's fork of Ethereum.
- **Network use (read-only):** the fork upstreams, JSON-RPC probes of public Ethereum and 4663 providers (`eth_blockNumber`,
  `eth_getLogs`, `eth_getBlockByNumber`, `eth_getTransactionReceipt`, `eth_getTransactionByHash`, `eth_getTransactionCount`,
  `eth_call`), and the reproducible build's crate fetch.
- **PoCs:** every PoC ran in a private copy under `/tmp/rook-r6-<part>/`, one folder per part. Each copy was checked with
  `diff -rq` against the worktree and removed afterwards.

| Part | Report | Scope | PoCs |
| --- | --- | --- | --- |
| A | `R6-launch-plan.md` | Option (c) as committed: `nft-pilot.json` `stages`, golive-check `--stage` / `--proposal` / A5-I-1, `staged.mjs`, `argdiff` EXPECTED paging, backlog-first delivery (A5-L-2), `held_fits` (A5-L-1), docs and site | `poc/A/` (node 4, the golive CLI) |
| B | `R6-canister-scan.md` | B5-M-1, B5-M-2, B5-L-1, B5-L-2, B4-L-2: `sale_scan`, `scan_read`, `scan_error`, `done_big`, the byte bounds, the cycles | `poc/B/` (cargo 4, 5 new switches; provider probes) |
| C | `R6-canister-wayback.md` | C5-M-1, C5-L-1, C5-L-3: `late_hold`, `relay_busy`, `eth_late_release`, `judge_payout`, `payout_pass` | `poc/C/` (cargo 5; read-only depository `eth_call`) |
| D | `R6-services.md` | Monitor and runner delta: `eth-home-unseen`, the hold texts, D5-L-1/2, the IDLs against the round-5 state, the Trust page | `poc/D/` (node 6) |
| E | `R6-canister-upgrade.md` | Upgrade safety of every new saved field and fixture, the `.did`, the rewritten H3 ask-site test (E5-I-2), new selectors and endpoints | `poc/E/` (cargo, `.did` check, a 1dc2a61 saved state) |
| F | `R6-canister-home.md` | F5-M-1 (+E5-L-1): `home_nonce_tx`, `home_effect`, adoption, `eth_home_clear`; F5-L-1, F5-I-1 | `poc/F/` (cargo 5; 4663 depth probe) |
| P | this file, "Providers" | The canister's Ethereum provider pair: depth, receipts, limits, rate limits | `poc/P/` |
| T | this file, "Test bar" | The full bar, the dry run and the reproducible build | `poc/T/test-bar.txt` |

Severity scale and threat model: as `../README.md`. A "passing" PoC shows the weakness is present; it turns red once fixed.

## Verdict

**Not ready to launch. A round-6 fix round is needed, mostly configuration.** Round 6 finds **1 Medium** and no Critical or High,
so the bar (0 C / 0 H / 0 M) is not met.

**Every round-5 Medium is closed** (table below). The code paths attacked this round did not reopen any of them. The one open Medium,
**B6-M-1**, is a liveness failure of the price feed. The canister's provider pair as committed triggers it with no attacker: the pair
is the dry run's `xrpc.cl/eth` + `ethereum-rpc.publicnode.com`, and publicnode is too shallow. It is a **launch blocker** until the
pair is replaced (see "Providers") and the forward walk can re-anchor.

No path was found that makes the canister sign what its judges did not build. None drains custody with nobody able to stop it,
and none double-pays on a provider's word. Option (c)'s staged sequence cannot be bypassed or reordered on the canister.

## Counts of OPEN findings (distinct)

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| A launch plan / option (c) (new) | 0 | 0 | 0 | 1 | 4 |
| B sale scan (new) | 0 | 0 | **1** | 2 | 3 |
| C way back / late hold / payout judge (new) | 0 | 0 | 0 | 3 | 3 |
| D services (new) | 0 | 0 | 0 | 3 | 4 |
| E upgrade / H3 guard (new) | 0 | 0 | 0 | 0 | 1 |
| F home line (new) | 0 | 0 | 0 | 2 | 3 |
| Earlier rounds, carried open (below) | 0 | 0 | 0 | 13 | (as their reports) |
| **Open** | **0** | **0** | **1** | **24** | **18** |

**Merged duplicates:**
- D6-I-3 is A6-L-1 (counted as A6-L-1).
- E6-I-2 is F6-I-1 (counted as F6-I-1).
- F6-I-2 is D6-L-1 (counted as D6-L-1, Low).
- Part A's note on `adopt_advance` is F6-I-4.

**The 13 carried Lows:**
- A5-L-3: unchanged.
- E5-L-2: stays Low. The launch steps keep their margin (22 + 3 < 36; 61 + 6 < 93). Only the stage-0 defaults with deposits on, or a
  trip factor of 2, hit it.
- B5-L-2: partly fixed. It is now also permanent: a one-sided `Ok` puts the block in `done_big`.
- C4-L-1: stays Low, but is now open-ended. From deadline + 25 h until the operator credits or releases the order, no collection
  starts a Relay buy, way back or sweep. Holders can still exit.
- C3-L-1, E3-L-2, B3-L-1: unchanged.
- B-L-6: partly fixed. The frog side's conversions are outside the hold (C6-I-3); this is unreachable in the NFT-only launch stack.
- A-L-2, B-L-5, C-L-5, D-L-3, D-L-6: unchanged. No contract changed (`git diff 8ef82d6..HEAD -- src/ test/` is empty).

## The open Medium

**B6-M-1 (Medium, conditional on the canister's provider pair; launch blocker as committed): a scan gap past a provider's log depth
freezes the forward walk for about 5 days.**

- **Code:**
  - `ethflow.rs:1186-1206`: the forward walk always starts at `sc.up + 1`;
  - `:1145`: the only way out is the window reset, 50,400 blocks after `sc.up`;
  - `scan_error` `:1471-1485`;
  - the transforms `ethnet.rs:49-58` → `pin.rs:214-219`, which keep the error `code` and drop the message.
- **Trigger:**
  - `ethereum-rpc.publicnode.com` serves `eth_getLogs` only about 10,000 blocks back (≈ 33 h). It is the second provider of the
    pair the dry run installs (`scripts/golive-dryrun.sh:266`). A gap with no scan passes longer than that needs no attacker:
    the runner down, cycles under the 8 T evidence floor, or a pause.
  - After such a gap, every forward read is refused (`-32602`, seen by the canister as `Other`, never `Deep`), and the walk never
    catches up.
- **Effect:**
  - No new sale is counted for that collection for about 5.3 days, and rises are held.
  - A collection with no first post gets none.
  - Nothing is signed from this, and nothing false is counted.
- **Same root:** publicnode answers `result: null` for receipts 20–30 k blocks back, which feeds B6-L-1's head-of-line block.
- **PoC:** `poc/B/r6_b.rs.snippet` `r6_b_m1_a_…`. After a 12,000-block gap, the first newer sale is read at pass 385–395 of 504,
  where the control reads it at pass 1.
- **Fix:**
  1. **Configuration:** a pair that serves logs and non-null receipts over at least 60,000 blocks ("Providers" below). golive-check
     should probe or at least print it.
  2. **Code:** re-anchor a forward walk refused for depth or stuck at one start, record the skipped span, and page the scan lag.
  3. **Classification:** keep a class of the provider's message in the transform, so that `Deep` and the provider size refusals
     classify as `fixes-round5.md` §3 claims.

## Round-5 Mediums: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **F5-M-1** (the home line's 1 h drop redoes a mined step) | **Closed** | A value-bearing step is never redone on absence of evidence. Every deciding read (nonce at past blocks, block bodies, effect read-backs) must agree on all providers, so a wrong or malicious provider causes only a wait, never a double pay or a wrong adoption. Effects cannot flip while pending (only we undo `advance`, `itemOf`, `sentTx`, `Delivered`). Adoption cannot credit the wrong buy (one open buy per collection, one advance per module, `sourceUsed`). **`eth_home_clear`** is operator-only, refused while paused or mid-tick, bound to the exact pending nonce after an hour unseen with the nonce used, applied only to a Relay deposit or `fundReserve`, and a chain answer wins over it. A wrong "not mined" pays one deposit or sweep twice (the fixer's call 1); a wrong "mined" self-corrects through the watch's refund read. Residuals: **F6-L-1** (4663's first provider keeps state only ~6,200 blocks, about 10 min at 10 blocks/s, so identification practically never succeeds at the runner's 600 s cadence and every hidden deposit or `fundReserve` waits for the operator), **F6-L-2** (a provider disagreeing on the receipt is `Unsure` for ever: no identification, no page, no `eth_home_clear`; the 4663 line halts until a 72 h provider change), D6-L-1, D6-L-2, F6-I-1, F6-I-3, F6-I-4 |
| **C5-M-1** (the write-off frees the Relay leg) | **Closed** | `late_hold` / `relay_busy` (`ethflow.rs:655-662`) gate every path that opens a Relay watch: new buys (`:1781-1789`), every `BackFunded` deposit, sweeps included (`:2539`), and new sweeps (`:3846`). Burns, redemptions, deliveries, deposits and posts are not blocked. `eth_late_release` is operator-only, refused while paused or mid-tick, one-shot, and moves nothing. **C5-L-3 closed** (reads first; `arrive` before any record). **C5-L-1 partly:** the depository path still credits any non-`depositNative` call naming the order id at the operator's stated amount, and a stranger can make one for gas (read-only `eth_call`: `allocator()` + 32 trailing bytes succeeds on both chains) (**C6-L-1**). Residuals: C6-L-1, C6-L-2, C6-L-3, C6-I-1..3; C5-I-1 (measure a real Relay fill and refund) stays a launch precondition |
| **B5-M-1** (oversized blocks outpace the bisection) | **Closed as reported** | Only failed reads count against a walk. An oversized range is deferred and passed, and the first read anchors even when too big. No new attacker starvation of the forward walk or the first post was found. `done_big` closes B4-L-2's edge. The residual is provider-driven: **B6-M-1** |
| **B5-M-2** (a provider's refusal of a dense range takes the old path) | **Closed in effect, not by the stated mechanism** | The transforms drop the message, so only `-32005` and the IC's size reject are `TooBig`, and `Deep` never fires. Every other provider refusal is deferred only by the "6 in a row at one start" catch-all, about an hour a range. The fixer's shape test feeds messages the canister never sees (B6-I-1). **The transient case is live (B6-L-2):** xrpc.cl's HTTP 429 rate limit is `-32005`, is read as `TooBig`, and at 50 % busy loses 20 of 432 honest sales a week for good. `SALE_BAD_MAX` is not a bound (529 ranges seen) |

**Round-5 Lows and Infos:**

| Item | Verdict |
| --- | --- |
| **B5-L-1** (the byte bound) | **Closed.** Every log, marketplace and receipt read is checked against its worst case before it starts, and the day has its own bound. **≈ 10.5 T cycles a collection a day (≈ 52 T, ≈ $70 a day for five) is acceptable** as a ceiling an attacker pays 0.07–3.6 ETH a day per collection to impose (honest: under 0.4 T a day). Top-up policy should assume 52 T a day under attack, and the monitor should page the burn rate |
| **E5-I-2** (the H3 ask-site test) | **Partly.** The rewritten test catches round 5's mutations but passes five others (**E6-I-1**): a per-deposit `col`, a key mutated after its `let`, a constant key on the `outflow_record` side, an aliased call, a file in a `src/` sub-directory. None is present today |
| **A5-L-1** (held custody at upgrade) | **Closed.** `held_fits` (`canister.rs:331-350`, from `post_upgrade` `:527-535`) traps atomically. It never blocks a code-only, caps-lowering or collection-removal upgrade; it refuses only an item-limit lowering below what is held (call 9) |
| **A5-L-2** (backlog-first delivery) | **Closed** for the reported orders (step 3 leaves 0 for the margin day). A6-I-3: the test resolves every ticket at once; after a long pause the urgent path is deadline-first again |
| **A5-I-1** | **Closed** (`golive-check.sh:828-841`: every `custody_outflow()` field held to the step's) |
| **E5-L-1** | **Closed** (residual F6-I-3) |
| **F5-L-1** | **Closed for a direct chain move** (residual F6-I-1: a two-step move keeps the old floors; fails closed) |
| **D5-L-1**, **D5-L-2**, **E5-I-3** | **Closed** |
| **C5-L-1** | **Partly** (C6-L-1) |
| **B5-L-2** | **Partly**, now permanent per block (`done_big`) |
| **D5-I-3** | **Partly** (a written-off order's payout landing is still not paged) |
| **A5-L-3**, **E5-L-2** | Unchanged Lows |

## New Lows

| ID | Where | What | Fix |
| --- | --- | --- | --- |
| **A6-L-1** | `monitor/src/staged.mjs:51-61`, `:78-80`; `argdiff.mjs:291` | A step 3 plus other cap loosenings pages EXPECTED (SEV2), and `golive-check --proposal --stage 3 --baseline` passes. The other loosenings tried were a ×4 raise of a Bitcoin wrapper's cap, a binding global item cap, a ×4 global value cap, and a new collection key. Only the launch collections' caps are compared, and every LOOSENS line is downgraded. The canister's ×4 and exit rules still bound it. | Require non-launch cap keys unchanged and the global caps equal to the step's sums; downgrade only the step's own lines; require ×4 in `stagedStep` |
| **B6-L-1** | `ethflow.rs:1357-1397` | A receipt one provider answers `null` and the other serves is a disagreement, re-queued with no limit. It stays first and takes every pass's 16 receipt reads: with 16 such moves, 0 of 144 newer sales were judged in two days. | Bound the retries, then set it aside behind newer moves; a deep pair (B6-M-1) |
| **B6-L-2** | `scan_error`; `ethnet.rs:445` | xrpc.cl's rate limit (`-32005`, HTTP 429) is read as "too big": ranges are deferred and evicted, and a block whose Seaport read was busy while Blur answered goes to `done_big`. At 50 % busy (measured 9/20 sequential, 7–10/13 simultaneous) 20 of 432 honest sales are lost a week. The marketplace range read also carries every collection's Seaport fills before the collection filter, so beyond about 500 blocks only Blur's half fits. | Classify rate limits as `Other`/retry; `done_big` only when both marketplaces answered; bound `bad` for real; a pair without anonymous rate limits |
| **C6-L-1** | `ethflow.rs:4050-4051`; `relayeth.rs:224-225` | The payout judge still credits any successful non-`depositNative` depository call (or a plain transfer of at least half) whose input names the order id, at the operator's stated amount. The request id is the runner's, unbound. A stranger can make one for gas. On the operator's hand-in, it ends the hold, and the real payout is then read by the next watch (C5-M-1's shape, operator-mediated). | Require Relay's execution event naming the order and our balance rise at that block; bind or drop the request id; with C5-I-1's measurement |
| **C6-L-2** | `ethflow.rs:978`, `:4125-4130`, `:3718-3726` | A credit landing while the loss-closing `returnAdvance(0)` is unresolved records `Returned{paid: 0}` (4663), or `Arrived` for a closed advance (Ethereum). The credit can never be handed in again, and the ETH at our address belongs to nobody. | Keep the claim until the buy's pending transaction resolves; resolve with the value the transaction carried |
| **C6-L-3** | `ethflow.rs:1770`, `:4130`, `:3743` | A written-off forward order's custody reservation is released. A later credited fill mints an item that is not in the custody book (`settle`: "not in flight"), so H2 and `held_fits` are one item short per such buy. | Keep the reservation while the late order is open, or re-reserve before `arrive` |
| **D6-L-1** | `monitor/src/ethnft.mjs:113-117`; `ethflow.rs:4183`, `:3565` | The unseen-home page asks for `eth_home_clear` for every step, and `home_clear` accepts it for any step ("taken as MINED at the next tick"), but the canister applies it only to a Relay deposit or `fundReserve`. For other steps it silently does nothing. It is also refused while paused, when the paused pass's records wait behind that same pending transaction (F6-I-2). | Refuse steps it never reads; decode `step` in the monitor and word the page per step |
| **D6-L-2** | `ethflow.rs` `used_since` resets | Pending and Unsure reset the unseen clock, so a home line held by a stale pool entry or flapping providers never pages directly, whatever its age (with F6-L-2). | Page on the pending transaction's age since signing |
| **D6-L-3** | `ethflow.rs:655-662`; `ethnft.mjs:102-108` | Once the written-off buy is closed as a loss, the hold still stops every collection's Relay orders. The only page is a non-persistent SEV3; the pool-idle SEV2 comes a day later. | A persistent SEV2 that escalates while `late_hold` is set |
| **F6-L-1** | `canister.rs:1059-1062`; `ethflow.rs:2976`, `:3496`; `ops/src/runner.ts:192-195` | 4663 makes 10 blocks a second, and `rpc.mainnet.chain.robinhood.com` keeps state only about 6,200 blocks (≈ 10 min). `robinhood.drpc.org` answers no state read at a block number. At the runner's 600 s cadence the nonce bisection's lower bound is past the provider's depth by the first resolving tick. Every hidden Relay deposit or `fundReserve` therefore waits for the operator, and the whole 4663 line waits, frog side included. | Identify from block bodies scanning forward from `HomeTx.pin`, or bisect within the reachable depth; an archive 4663 provider (`rpc.ordofi.network` measured as one) |
| **F6-L-2** | `canister.rs:992-997`; `ethflow.rs:3481`, `:3589-3600`; `monitor/src/ethnft.mjs:114` | One provider missing or differing on the receipt is `Unsure`. It resets the clock and never runs identification, so there is no page and no `eth_home_clear`, and nothing else is signed on 4663 until a timelocked provider change (72 h). The same was true in round 4. | Run identification and the effect read-back on `Unsure` when the nonce is used at the pin |

Infos: A6-I-1..4, B6-I-1..3, C6-I-1..3, D6-I-1, D6-I-2, D6-I-4, D6-I-5, E6-I-1, F6-I-1, F6-I-3, F6-I-4 (see the part reports).
Among them:
- **D6-I-4:** the Trust page's "the week's real sellers" is false on the committed pair (publicnode ≈ 1.4 days).
- **D6-I-1:** the scan lag is not paged.

## The launch configuration and option (c)

| Setting | Verdict |
| --- | --- |
| `nft-pilot.json` step 1 (48: 13 / 17 / 13 / 3 / 2 at 12 / 24 a day) | Safe and enforced. The canister's `init`, golive-check and the dry run's go/no-go read on the **committed** file all pass it (662 checks). One stale comment: `_outflow` still says 61 items |
| Step 2 (`global_items` 31 alone), step 3 (J7's 61 alone) | **Cannot be bypassed or reordered on the canister.** A caps-first proposal is refused by the canister, golive-check and the monitor (SEV1). Both at once is accepted by the canister but pages SEV1 and fails `--proposal`. A step 3 queued while step 2 is pending pages SEV1 and goes stale. A stale or wrong baseline gives SEV1 (`baselineMismatch`). An edited `stage` makes golive-check fail closed, and the monitor ignores it. Any other Init field changed pages SEV1. The looseness is A6-L-1 (step 3 plus other cap loosenings is EXPECTED), with A6-I-2 (`--proposal` with `--canister` and no `--baseline` passes other Init changes with a note) and A6-I-1 (the timing rules are runbook-only) |
| Backlog-first delivery (A5-L-2) | Closed for the reported orders |
| Upgrade refusing outflow below held custody (A5-L-1) | Closed; cannot be bypassed by the argument, and blocks no needed upgrade beyond call 9 |
| **The canister's Ethereum provider pair** | **Launch blocker (B6-M-1, B6-L-1, B6-L-2).** The only committed pair is the dry run's `xrpc.cl/eth` + `ethereum-rpc.publicnode.com`. Replace it (below) |
| C5-I-1 (a real Relay fill and refund each way) | Still an operator precondition before launch (runbook §5.1) |
| 4663 providers for the canister | F6-L-1: add an archive-depth 4663 provider or change the identification |

## Providers (part P): the canister's Ethereum pair

Probed read-only on 2026-09-28, 11:00–11:10 UTC (`poc/P/`: `probe.sh`, `rc.sh`, `rng.sh`, `depth.output.txt`). Each provider got
500-block Milady `Transfer` `eth_getLogs` at depths from 1,000 to 130,000, receipts and transactions by hash at depth, range and
size limits, and a 13-parallel burst (an IC outcall is made by every replica of a 13-node subnet).

| Provider (keyless) | Logs depth | Receipts at 30–60 k | Limits | 13-burst | Verdict |
| --- | --- | --- | --- | --- | --- |
| `ethereum-rpc.publicnode.com` | **~10,000** (`-32602 Archive requests require a personal token` beyond) | **`null`** from 20–30 k | none on size (74 MB answered) | 13/13 | **Too shallow: the committed pair's weak half** |
| `xrpc.cl/eth` | 130,000+ (with retries) | ok (fixer's probe) | anonymous rate limit: `x-ratelimit-limit: 10`, HTTP 429 `-32005 busy` | **3/13** (4/20 single calls 8 s apart busy) | **Unsuitable for the canister:** the busy answers are misread as too big (B6-L-2), and mixed replies fail outcall consensus. Keep it for the monitor only |
| `rpc.mevblocker.io` (`/fast` and base) | 130,000+ | ok at 30 k, 52 k, 60 k | 9,224 logs / 5.86 MB for 100 dense blocks, no cap seen; 10,000-block Milady ranges ok | 13/13 | **Recommended** |
| `gateway.tenderly.co/public/mainnet` | 130,000+ | ok at 30 k, 52 k, 60 k | caps at 20,000 results (`-32602 invalid params`, seen by the canister as `Other`); 10,000-block Milady ranges ok | 13/13 | **Recommended** (IPv4 only: no AAAA) |
| `eth.drpc.org` (free) | 10-block ranges only (`Can't route` / `ranges over 10000 blocks…`) | ok | — | 13/13 | No (the scans need ranges) |
| `eth.api.onfinality.io/public` | 130,000+ | ok | — | **0/13** (`-32029`) | No |
| `1rpc.io/eth`, `eth-pokt.nodies.app`, `eth.blockrazor.xyz`, `eth-mainnet.public.blastapi.io`, `ethereum-mainnet.gateway.tatum.io` | 50 / 50 / 25 / 10 / 100-block caps | | | | No |
| `api.zan.top`, `eth.meowrpc.com`, `virginia.rpc.blxrbdn.com` | no keyless `eth_getLogs` | | | | No |

**Recommendation:** install the canister's `evm_chains[1]` with **`https://rpc.mevblocker.io` + `https://gateway.tenderly.co/public/mainnet`**.

- **Why these two:**
  - Both are keyless.
  - They are independent operators: CoW's MEV Blocker, behind Cloudflare, and Tenderly's own GCP-hosted gateway.
  - Both serve logs and non-null receipts well past the 7-day window (50,400 blocks), with room to spare.
  - Both took a 13-way burst cleanly.
- **Before the install argument is built, verify:**
  1. **From an actual IC outcall** (a rehearsal canister), that Tenderly's IPv4-only host is reachable and both reach consensus
     through the canister's transforms.
  2. **How `evmsend` classifies each one's `eth_sendRawTransaction` reply.** MEV Blocker routes sends to its private orderflow; a
     read-only probe cannot test it.
  3. **That the golive-check host rule still counts them as two hosts.**
- **Tenderly's 20,000-result cap** reaches the canister only as a code, so it is deferred through the "6 in a row" catch-all until
  B6-M-1's classification fix.
- **Keep `xrpc.cl/eth` as the monitor's second provider only.**
- **If either recommended provider changes its policy,** golive-check should refuse a pair that fails a 60,000-block logs +
  receipt probe (B6-M-1 fix 1).

## Recommended order for round-6 fixes

1. **B6-M-1 (blocker):**
   - replace the canister's Ethereum pair (above), and have golive-check probe or print it;
   - re-anchor a forward walk refused for depth, and page the scan lag;
   - keep a message class in the transform (B6-I-1).
   - With it: B6-L-1 (bounded receipt retries) and B6-L-2 (rate limits not "too big", `done_big` only on two answers, a real
     `bad` bound).
2. **The 4663 home line's exits:**
   - F6-L-2 (identify on `Unsure`), F6-L-1 (forward block-body identification, or an archive 4663 provider);
   - D6-L-1 (`home_clear` refuses steps it never reads, pages per step), D6-L-2 (page on pending age).
3. **The payout judge:**
   - C6-L-1, with the C5-I-1 measurement (bind a credit to Relay's execution event);
   - C6-L-2, C6-L-3, D6-L-3.
4. **A6-L-1** (staged-step recognition exact on every cap, downgrade only the step's lines).
5. **Infos:**
   - E6-I-1 (read record sites, walk `src/` recursively, or type the key);
   - F6-I-1 (save the floors' chain), F6-I-3, F6-I-4;
   - A6-I-2 (require `--baseline` with `--proposal`);
   - the `_outflow` comment, and the Trust page's "week" (D6-I-4).

## Test bar (re-run by this review at `e04fa16`, PoC files out of the tree)

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites) |
| Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **683 passed**, 0 failed |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed |
| ops | 58 passed; typecheck clean |
| monitor | **203 passed** |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | 72 passed; typecheck clean |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 on the v3–v6 stack; 536 on the five-collection NFT stack; **662 on the go/no-go read of the committed `nft-pilot.json`**; 51 refusals `ok`; `--stage 2` and `--proposal` steps 2 and 3 pass, both-at-once and caps-first refused) |

Every number in `fixes-round5.md` §11 reproduced. These suites are green *with* B6-M-1 present; the dry run does not probe
provider depth.

**Reproducible build:** `docker build --no-cache --platform linux/amd64` of `autobuyer/Dockerfile.build`, on a `git archive` export
of `e04fa16`'s `autobuyer/`. It gave hashes **identical** to `fixes-round5.md` §11:

```
3218bff1274c9655aefec2ceac3685b1e97aab4e8e4560957e1b49b065b8d99e  buyer.wasm   (7,022,016 bytes; 1,334,124 gzip -9 -n, < 2,000,000)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

**Removed afterwards:**
- the export, the build image and the extracted wasm;
- every private copy under `/tmp/rook-r6-*`;
- the dry run's deployment files (removed by the dry run itself);
- the anvils.

Docker's shared build cache was left alone, and nothing was installed.
