# Fix round 5: the 2026-09-27 review of the Ethereum NFT side, round 5

Written 2026-09-28 on branch `nft-side` (at `8ef82d6`: the round-5 re-review's reports and PoCs on top of fix round 4). It answers
`review5/README.md` and the six part reports (`R5-contracts-deploy.md`, `R5-canister-floor.md`, `R5-canister-wayback.md`,
`R5-services.md`, `R5-canister-flow-upgrade.md`, `R5-canister-pin.md`) finding by finding: the fix, where it is, and the test that is
the reviewer's PoC turned around (it fails on the code before this round and passes now). `fixes-round5-poc/` has the reviewers' PoCs
re-run on this code. §11 ("As built") has the commits, the test counts and the reproducible-build hashes. Nothing was deployed, pushed
or signed with a real key; the only chains written to were local anvils (the dry run's fork of 4663, the Seaport fork proof's fork of
Ethereum, both read-only upstream); read-only `eth_getLogs` probes measured the canister's keyless Ethereum pair (§3); the
reproducible build fetched its crates.

**The rules every fix keeps** (as rounds 1–4):

- Nothing is weakened to buy availability: H1 (pause), H2 (custody caps), H3 (the outflow limiter and its tripwire), "save, then
  send", and every judge (`relayeth`, `seaport`, `ethnft`) refuse what they refused. The payout judge refuses more (C5-L-1). The
  first floor post still needs the collection's own counted sales (§3, call 7).
- Stable state stays decodable: every new field is `opt` (`Option` + `#[serde(default)]`); no enum case was added. The state the
  round-4 build wrote restores with nothing lost (`tests/audit_nft_r5.rs` `r5_the_state_the_round_4_build_wrote_…`, on the bytes
  1dc2a61's own test held equal to its encoder: `tests/fixtures/state/eth-state-1dc2a61.hex`); the round-2 and round-4 fixtures
  were re-encoded only because their tests compare bytes and the type table grew.
- **Not changed** (operator decisions): BAYC 10 ETH / `convBps` 1,000 / `reserveBps` 0 (J1); J7's 15 ETH a collection and its end
  counts (61), with option (c)'s staging (§6); H3's 25 ETH a day and trip ×3; D1–D9, D5 deposits on; D4's automatic return after
  48 h; Dwellir first for the monitor and the runner; keyless canister providers.
- No contract changed (`src/` is untouched).

## 0. Summary

| ID | Sev | Status | Fix | Where | Test (the PoC, turned around) |
| --- | --- | --- | --- | --- | --- |
| **F5-M-1** (+ E5-L-1) | Med | **Fixed** | A home transaction whose nonce is used and that no provider shows is never done again on that absence alone. First, what used the nonce is identified without the transaction index (`EthIo::home_nonce_tx`: our nonce count at past blocks, bisected from the pin it was signed at, `HomeTx.pin`, then that block's transactions from us, every provider agreeing): ours → MINED; another of ours → dropped. Else the step's own effect is read back at the floored pin (`home_effect`: the module's `transitUnits` for an advance, a settle, a `returnAdvance`; the ticket's stage for a record; `sentTx` for a note; `undeliverable` for a mark; `itemOf` / `item` for a mint): present → mined; absent, on every read in a row for `HOME_NONCE_TAKEN_SECS` → taken back. A Relay deposit and a sweep's `fundReserve` leave nothing to read: they wait, paged SEV1 (`eth-home-unseen`), until identified or the operator looks and says (`eth_home_clear(nonce, mined)`). `Pending` and `Unsure` restart the clock. The effects are idempotent where the chain allows: an open advance no buy tracks is adopted (never paid again), a closed one ends a `Returning` buy, a token already on the wrapper ends a `Final` buy or a `Mint` deposit, a `Delivered` ticket is recorded and releases its item, `sentTx` / `undeliverable` already set are not signed again. E5-L-1: a deposit whose mint only fails (its estimate) no longer holds its collection's other deposits, and one already on the wrapper from its own transaction is adopted | `ethflow.rs` `resolve_home`, `home_effect`, `transit_open`, `wrapper_holds`, `adopt_advance`, `record_done`, `mark_done`, `note_sent`, `home_act` (`Final`, `Returning`, `NeedAdvance`, step 3), `home_clear`; `canister.rs` `home_nonce_tx`, `our_txs_view`; `evmsend.rs` `our_txs_body`; `ethnet.rs` `eth_home_clear`; `ethnft.rs` `SENT_TX`, `UNDELIVERABLE`, `ADVANCED` | `audit_nft_r5.rs` `r5_f5_m1_a_…` (never deposited again; MINED once identified), `…_a_operator_word_…`, `…_a_control_another_transaction_at_the_nonce_…`, `…_b_…` (fundReserve once), `…_c_…`, `…_c2_…`, `…_c3_…` (an orphan adopted), `…_d_…` (Unsure restarts the clock), `…_e_…`, `…_e2_…`, `…_record_…`, `r5_e5_l1_a_…`, `r5_e5_l1_b_…` |
| **C5-M-1** (+ C5-L-1, C5-L-3) | Med | **Fixed** | A written-off order keeps the Relay leg's attribution: while it is neither credited nor released, `relay_busy()` stays true (`EthState::late_hold`), so no new buy, way back or sweep starts and no other order's watch is open to read its late payout. It ends when a payout of it is credited (`eth_credit_payout`), or when the operator, from Relay's own records, releases it (`eth_late_release`, `LateOrder.released_at`). C5-L-1: the payout judge ties a transaction to THIS order: its input carries the order id or Relay's request id (`Watch.request_id`, from the quote's `steps[0].requestId`), it is not ours (our own `depositNative` names the order too), it mined after the order was deposited (`Watch.home_pin` on 4663, `Watch.pin` on Ethereum), and a call's stated amount counts only for a call to Relay's depository that is not a deposit; the solver alone (every order's) no longer names it. C5-L-3: every read comes before any write; a failed read credits nothing and the payout can be handed in again | `ethflow.rs` `EthState::late_hold`, `relay_busy`, `buy_pass`, `sweep_pass`, `eth_act` (way back), `give_back_now`, `judge_payout`, `payout_pass`, `late_release`; `relayeth.rs` `EthDeposit.request_id`; `ethnet.rs` `eth_late_release` | `r5_c5_m1_a_…` (no order starts; the late payout credited to Milady, then Azuki bridges once), `r5_c5_m1_b_…` (released: new orders start, a payout still creditable), `r5_c5_l1_a_…`, `r5_c5_l1_b_…`, `r5_c5_l1_c_…`, `r5_c5_l3_a_…` |
| **B5-M-1** | Med | **Fixed** | A range too big to read never holds a walk: its sales are read at once through the marketplaces' own sale logs of the whole range (one read per marketplace, address and topic filtered: `eth_sale_moves_range`), and the range is deferred (`SaleScan.bad`) to be bisected later through its `Transfer` logs for any other sale, newest first, on what is left of the pass (`SALE_REFINE_FAILS_PER_PASS`); a single oversized block goes through its own marketplace logs once and is remembered (`SaleScan.done_big`: never bisected or asked again, B4-L-2's edge). Only a read that FAILED counts against a walk. The first read anchors the scan even when it is too big, so a collection's first post needs only the newest blocks' sales. The reading says how far the scan is behind the final head | `ethflow.rs` `sale_scan`, `scan_read`, `block_sales`, `defer`, `remember_big`, `ScanPass`; `ethnet.rs` `eth_sale_moves_range` | `r5_b5_m1_a_…` (126 blocks one apart: a newer sale read at pass 3, all 60 in two days; was: none for a week), `r5_b5_m1_b_…` (one a pass for a week: never behind, 504 of 504; was 30,406 behind, 200), `r5_b5_m1_c_…` (no anchor, one every 50 or every 10 blocks: first post within a day; was never), `r5_b4_l2_…` (the hull never bisects it again; was 9 failed reads, 46 MB) |
| **B5-M-2** | Med | **Fixed** | Every "too big" answer takes the same path, whoever says it (`scan_error`): the IC's size reject; a provider's result, range, size or time limit (measured on the canister's keyless pair, §3: publicnode answers dense ranges whole, so the IC refuses them; xrpc.cl answers `-32005`); and any error repeated `SALE_SCAN_STUCK_FAILS` (6) times at one start (`SaleScan.stuck`). A provider that keeps no logs that deep (`Deep`: publicnode's "Archive requests require a personal token") stops the walk back for the pass without shrinking the newer blocks' chunk | `ethflow.rs` `scan_error`, `ScanErr`, `scan_read` | `r5_b5_m2_a_…` (40 of 40 newer sales in a day; was 0), `r5_b5_m2_b_…` (an error that names nothing, deferred after 6), `r5_b5_m2_every_too_big_answer_…_the_measured_shapes` |

**The four Mediums are closed by this round's own reading.** Also: the operator's option (c) is committed and checked (§6), ten
Lows fixed or partly (§7), and several Infos (§8).

## 1. F5-M-1: never redo a value-bearing step on absence of evidence

- **What the canister does now** when its pending 4663 transaction is seen nowhere (no receipt at the pin, no transaction by hash)
  while its nonce is used:
  1. **Identify** (`EthIo::home_nonce_tx`, `canister::home_nonce_tx`): `eth_getTransactionCount(us, n)` bisected between the pin the
     transaction was signed at (`HomeTx.pin`, new; an older pending one searches `HOME_NONCE_SEARCH_BLOCKS` back) and the pin, then
     `eth_getBlockByNumber(n, true)` projected to our transactions' `[hash, nonce]` (`our_txs_view`). Every step is read on every
     provider and must agree. This does not use the transaction index, which is what failed. Ours → **MINED** (its status is its
     effect where one is readable; a Relay deposit or `fundReserve` is taken as succeeded: a deposit that reverted is read by its own
     watch as refunded, the ETH being there). Another transaction of ours → ours can never mine: **dropped**, its effect still read.
  2. **Else read the effect back** (`home_effect`): advance → `transitUnits` open; settle and `returnAdvance` → closed; record → the
     ticket `Delivered`; note → `sentTx == tx`; mark → `undeliverable`; mint → `itemOf` (and, for a deposit, `item(n-1)` from this very
     transaction). Present → mined. Absent at a pin past the nonce, on every read in a row for `HOME_NONCE_TAKEN_SECS` → taken back.
     A floor post (harmless) goes after the hour too.
  3. **Else wait and page**: a Relay deposit or a sweep's `fundReserve` is never done again. The line says `UNIDENTIFIED`, the
     monitor pages `eth-home-unseen` (SEV1) after an hour, and the operator, after looking the transaction up, says
     `eth_home_clear(nonce, mined)` (only for that pending transaction, and only once it has been unseen for the hour).
  4. **The clock**: `Pending` and `Unsure` reset `used_since`; "seen nowhere" is every read in a row.
- **Idempotent effects**, so an earlier build's wrong drop is repaired and never paid twice: `home_act`'s `NeedAdvance` adopts a
  module advance that is already open (`advanced()` read, H3 recorded under its own key, custody reserved, the buy funded);
  `Returning` whose module advance is already closed is `Returned` without paying; `Final` whose token is already on the wrapper is
  minted; a `Record` whose ticket is `Delivered` is `Recorded` and releases its item (before: `Gone`, the item's room lost); a note
  or a mark already on the wrapper is not signed again.
- **E5-L-1**: in step 3 (deposits' mints), only a wait that is the collection's (H3, the floor, a purchase holding the room, a
  module read) skips its other deposits; a mint whose own estimate refuses moves on to the next deposit, and one whose token is on
  the wrapper from its own transaction is adopted as minted. `mined_home`'s `itemOf` read is now only reached for a real revert
  (the drop path decides by `home_effect`, where a read error is a wait).

## 2. C5-M-1: a written-off order keeps the Relay leg's attribution

- `EthState::late_hold()`: the first buy whose `late` order is neither credited nor released. `relay_busy()` includes it, so
  `buy_pass` starts no new buy (it says why), `eth_act` signs no way back and `sweep_pass` starts no sweep. Burns, redemptions,
  deliveries, posts and fills of already-arrived buys go on.
- The hold ends when a payout of the order is credited (the judge below), or the operator releases it (`eth_late_release(buy)`)
  once Relay's own records show where its ETH went. A payout that lands after a release can still be credited, but may be read by a
  new watch first: the release is the operator's call (§9, call 4).
- **C5-L-1, the judge** (`judge_payout`): not ours; names the order (order id or request id anywhere in the input); mined above the
  watch's pin on that chain; a plain transfer to us at its value, or a call to Relay's depository (not `depositNative`) at the
  stated amount; at most the order's value and at least half. The solver-only path is gone. C5-I-2's dead clause is removed and the
  comment says what is held.
- **C5-L-3**: `payout_pass` reads the balance(s) first, applies the credit (the one step that can refuse, `arrive`, goes first),
  and only then writes `st.credited` and `late.credited`.
- **C5-I-1**: the judge's shapes are not yet measured against a real Relay fill or refund. The runbook (`phase1-nft.md` §5.1) makes
  "measure one real fill and one real refund each way" a pre-launch operator task.

## 3. B5-M-1 / B5-M-2 / B5-L-1: the scan's progress cannot be starved, and its bytes are bounded

- **Walks** (`sale_scan`, `scan_read`): ranges already deferred, oversized blocks already read, and skipped blocks are stepped over.
  A read that is too big (`ScanErr::TooBig`, or `Other` six times in a row at one start) of one block goes through its own
  marketplace logs (`block_sales`); of a range, the range is deferred and its marketplace logs read in one read
  (`eth_sale_moves_range`), and the walk goes past it either way. Only failures count against a walk's 2 a pass.
- **Refinement**: the newest deferred range's upper half is read through its `Transfer` logs, split again when too big, down to
  single blocks (their own marketplace logs, once). It runs after the walks, on its own 2 failures a pass and what is left of the
  bytes, and never ahead of a walk. `SaleScan.bad` keeps 32 ranges (the oldest, whose sales leave the window first, go).
- **Error shapes, measured 2026-09-28** on the dry run's keyless pair (read-only probes, `eth_getLogs` of USDT's and Milady's
  `Transfer`): `ethereum-rpc.publicnode.com` answered 11.6 MB for 100 blocks and 74.6 MB for 1,000 blocks (no result cap: the IC's
  2 MB class refuses it), `-32701 broker: node error` for 5,000, `-32701 Please specify an address` without one, and `-32602 Archive
  requests require a personal token` for any range older than about 10,000 blocks; `xrpc.cl/eth` answered `-32005 XRPC anonymous
  capacity temporarily busy` and `-32000 … syncing and fallbacks busy` for wider ranges under load, and served 500-block Milady
  ranges at any depth. So: the IC's reject and `-32005` / "more than" / "too many" / "exceeds" / "limit" / "timeout" / "range" words
  are `TooBig`; "archive" / "pruned" / "missing trie node" are `Deep`; the rest `Other` (deferred only when it persists).
- **B5-L-1, a real bound**: a log read is started only when its worst case (`EthIo::logs_read_worst`: every class on every provider,
  4.928 MB on two) still fits the pass's 10 MB (`SALE_SCAN_BYTES_PER_PASS`, one oversized read and its marketplace read); the
  receipts have their own 4 MB (`SALE_RECEIPT_BYTES_PER_PASS`, counted by `EthNet` per class and provider, `EthIo::receipt_bytes`);
  and the day's scan is bounded at 72 passes' worth (`SALE_SCAN_BYTES_PER_DAY`, `SaleScan.spent`). **Re-measured worst:** 14 MB of
  reserved response a collection-pass ≈ 146 B cycles at 10,400 cycles a byte on a 13-node subnet; ≈ 10.5 T cycles a day a collection
  at most (≈ 52 T for five), whatever the runner's cadence. The harness's worst passes: 9.98 MB of logs (the reviewer's constructed
  pass: 5.18 MB, was 19.71 MB), receipts under 4 MB with 40 oversized receipts queued.
- **B5-L-2 (partly)**: `EthNet` reads Seaport's and Blur's logs separately and fails only when both fail, so a padded fill on one
  marketplace hides nothing on the other. A block padded on Seaport AND flooded is still unreadable for Seaport's sales there (as the
  reviewer's case).

## 4. The Mediums' residuals, as this round leaves them

- F5-M-1: identification needs providers that serve historical state as far back as the transaction mined; `rpc.mainnet.chain.
  robinhood.com` serves about 6,200 blocks. Where it cannot be told, state-bearing steps decide by their effect; a Relay deposit or
  a sweep's `fundReserve` waits for the operator.
- C5-M-1: the hold trades liveness for attribution (C4-L-1 re-rated: new Relay orders wait until the operator credits or releases a
  written-off order).
- B5-M-1: a deferred range's non-Seaport, non-Blur sales wait for its refinement, which a steady supply of oversized blocks can keep
  behind (on its own budget; the walks and the marketplace sales are never held).

## 5. The first floor post

The task's suggestion "listings within bounds post; sales only hold back" for the FIRST post was not adopted: the first post is the
only one without the module's own ±5 %/h, ±20 %/day bounds, and the design keeps it sales-confirmed (`phase1-nft.md` §4.2, A-M-1).
What starved it (no anchor, B5-M-1 (c)) is gone: the first read anchors even when too big, and the newest blocks' marketplace sales
count at once, so the first post comes as soon as three counted sales are read (the harness: pass 13, or 26 with an oversized block
every 10 blocks). §9 call 7.

## 6. The operator's launch plan: option (c), committed and checked

- **The launch file** (`deployments/launch/nft-pilot.json`): `custodyItems` are step 1's (Milady 13, Azuki 17, MAYC 13, Pudgy 3,
  BAYC 2: 48), `outflow` 12 / 24 / 3 / 6 / 25 ETH / ×3, `"stage": 1`, and `stages`: step 1 (install), step 2 (`global_items` 31,
  nothing else), step 3 (J7's 16 / 22 / 17 / 4 / 2: 61, nothing else), each with when it may be proposed.
- **A5-L-2, picked: backlog-first delivery** (`ethflow::delivery_order`): the collection with the most deliveries waiting goes
  first, its nearest deadline first; a delivery within two days of its deadline (`DELIVERY_URGENT_SECS`) before any backlog.
  **Why this over raising `per_collection_items` to 22 or `global_items` to 42 at step 2:** it closes the gap for every option with
  no configuration change and no added exposure (a compromised signing path's day-1 reach stays 12 a collection and 31 in all), and
  the rule's two tests are then enough: the real limiter at 10-minute ticks, the reviewer's worst order (Azuki's tickets last), leaves
  0 items for the margin day at step 1, 2 and 3 (deadline-first left 5, 3 and 9: `r5_a5_l2_…`, which reproduces the reviewer's
  counts before asserting 0). So step 2 changes `global_items` alone.
- **golive-check** (`script/golive-check.sh`):
  - `--launch` reads the file's current step (`--stage` overrides): `capcheck` holds the canister's caps to that step's counts,
    `launchexit` the exit rule on it, and `stagecheck` the plan (step 1 = the file's own numbers; each step fits; step 2 changes
    `global_items` alone, raised within ×4 to at least what step 3's items need; step 3 the item counts alone).
  - **A5-I-1**: with `--canister`, every field of `custody_outflow()` (items, mints, value, trip factor) is held to the step's.
  - **`--proposal <hex|file> --stage <2|3>`** (no chain read): decodes the pending argument with the monitor's IDL and requires its
    caps and limits to be exactly the step's, the running pair (`--canister`'s `custody_caps()` / `custody_outflow()`, or
    `--baseline`) the step before, the change only the step's own kind, with `--baseline` every other Init field unchanged, and its
    pair inside the exit rule (`monitor/src/staged.mjs` `checkStagedProposal`).
- **The monitor**: a proposal whose diff from the baseline is exactly one staged step (`stagedStep`, reading `launch_file` from its
  configuration, `deployments/launch/monitor-nft.json`) pages as **EXPECTED (SEV2)** with its loosenings marked; anything else,
  the same numbers with any other change included, stays SEV1. A proposal whose pair fails the exit rule is annotated "the running
  code refuses it, so executing it fails".
- **The runbook** (`phase1-nft.md` §5.1): queue step 2 only after step 1 has executed and the handover is done; no other target
  upgrade may execute in its 72 h (any executed action makes it stale: re-queue it after a security upgrade); set `"stage": 2` and
  read the canister again after it runs; step 3 after two clean weeks, a separate caps-only proposal carrying `global_items` 31.
- **Docs and site**: the Trust page states the installed numbers and the plan; `launch-gates.md` G7 (both copies) states (c)'s
  steps and notes that rows 1–3 are the Bitcoin wrapper's (A5-I-4); `phase1-nft.md` §5, `phase1-nft-pooled.md` §6 J7 (DECIDED).
- **The dry run** reads the committed file: the go/no-go read passes on it, J7's 61 at 24 a day is shown refused from a copy, a
  canister at step 2 is read with `--stage 2`, and `--proposal` passes step 2 and step 3 and refuses both-at-once and caps-first.

## 7. The Lows

| ID | Status | What |
| --- | --- | --- |
| C5-L-1 | Fixed | §2: the judge ties a payout to the order: `r5_c5_l1_a/b/c_…` |
| C5-L-3 | Fixed | §2: `r5_c5_l3_a_…` (nothing recorded on a failed read; handed in again, credited) |
| B5-L-1 | Fixed | §3: real bounds, receipts included, per pass and per day: `r5_b5_l1_…` ×2 |
| B5-L-2 | Partly | §3: one read per marketplace; a block padded on one marketplace and flooded still hides that marketplace's sales there |
| E5-L-1 | Fixed | §1: `r5_e5_l1_a_…`, `r5_e5_l1_b_…` |
| F5-L-1 | Fixed | `post_upgrade` resets the Ethereum pin and nonce floors and `last_mined` when `eth_nft` moves to another chain (`ethflow::chain_moved`): `r5_f5_l1_…` |
| A5-L-1 | Fixed | `post_upgrade` holds what custody HOLDS (the restored book) to the exit rule at the kept limits (`canister::held_fits`); lowering the caps alone stays allowed: `canister.rs` `a5_l1_…` |
| A5-L-2 | Fixed | §6: backlog-first delivery: `r5_a5_l2_…` |
| D5-L-1 | Fixed | The monitor exempts a ticket `Delivered` by THIS delivery (`deliveryTx` = its transaction); refunded still pages: `audit_nft_d_r5` |
| D5-L-2 | Fixed | The monitor decodes `EthBuy.last` and words three cases (watched; written off, with the hold; fill-Stuck: "NOT watched … pause, then look"); a sweep closes with nothing: `audit_nft_d_r5` |
| B4-L-2 | Fixed (the reviewer's new edge) | Resolved oversized blocks are remembered (`done_big`) and stepped over; the hull itself still merges ranges |
| E5-L-2 | Not fixed | The exit rule's tripwire half: the launch numbers keep a margin (22 + 3 < 36; 61 + 6 < 93 at step 3); only the stage-0 defaults (with deposits on) or a trip factor of 2 hit it. Left for the operator (§10) |
| A5-L-3 | Not fixed | golive-check's keyless rule residuals (a key split 11/11/10, `wss://`, a key subdomain printed on the host-count FAIL line, a trailing-dot host): operator-mistake only; the cost of a missed key is a paid plan's quota |
| C4-L-1 | Re-rated, longer | §2: new Relay orders now also wait for a written-off order until it is credited or released (§9 call 4) |
| C3-L-1, E3-L-2, B3-L-1 | Not fixed | Unchanged |
| B-L-6 | Partly | The judge no longer credits another flow's payment (it must name the order); the automatic 4663 reads still take a rise as a refund |
| A-L-2, B-L-5, C-L-5, D-L-3, D-L-6 | Unchanged | As `fixes-round2.md` §7 |

**Lows: 11 new at `8ef82d6` (A5-L-1..3, B5-L-1..2, C5-L-1, C5-L-3, D5-L-1..2, E5-L-1..2, F5-L-1) plus the carried: 9 fixed (A5-L-1,
A5-L-2, B5-L-1, C5-L-1, C5-L-3, D5-L-1, D5-L-2, E5-L-1, F5-L-1), 1 partly (B5-L-2), 2 not (A5-L-3, E5-L-2); carried: B4-L-2's edge
fixed, B-L-6 partly, C4-L-1 lengthened, the rest unchanged.**

## 8. The Infos

Fixed: **E5-I-2** (the H3 ask-site test reads every file under `src/` at test time, every `outflow_check(` call with any argument
and comments stripped, resolves each key to its `let` binding in scope, matches each against a reviewed list of logical outflows,
and pins `mint_key`'s body and the advance key's count; it catches all four of the reviewer's mutations and C3-H-1's shape:
`r5_e5_i2_…`), **E5-I-3** (`eth-state-round5.hex` holds an `Inbox.payouts` claim and every round-5 field; the monitor and the runner
decode it), **A5-I-1**, **A5-I-4** (§6), **C5-I-2** (§2), **D5-I-3** (the mined-delivery deadline page says the note waits for
finality), F5-I-5 (the dangerous drops are tested). Partly: B5-I-1 (32 deferred ranges, the oldest evicted), B5-I-2 (the walks no
longer spend the bytes on failures), D5-I-3's first half (a written-off order's payout landing is not paged; its SEV3 says the hold).
Unchanged: C5-I-1 (a runbook task, §2), C5-I-3, B5-I-3, D5-I-1, D5-I-2, E5-I-1, F5-I-1..4, A5-I-2, A5-I-3, A5-I-5, and the earlier
rounds' as listed there.

## 9. New judgment calls for the operator

1. **A Relay deposit or a sweep's `fundReserve` whose transaction mined unseen waits for the operator** when the chain cannot tell
   what used its nonce (`eth_home_clear(nonce, mined)` after looking). A wrong "not mined" pays twice (bounded by one deposit or one
   sweep); a wrong "mined" leaves the step undone (the deposit's watch then reads no arrival and the buy goes Stuck). The one 4663
   line waits meanwhile (SEV1 `eth-home-unseen`).
2. **A state-bearing step is taken back after an hour of unseen reads in a row when its effect is absent** at a pin past its nonce
   (a revert of ours reads the same way); a floor post likewise.
3. **An earlier build's wrong drop is adopted, never paid again**: an open module advance no buy tracks becomes the collection's new
   buy's (at the module's `advanced()`), a closed one ends a `Returning` buy, a token already on the wrapper is minted.
4. **A written-off Relay order holds the Relay leg** until a payout of it is credited or the operator releases it
   (`eth_late_release`) on Relay's records. New buys, ways back and sweeps wait meanwhile (C4-L-1 longer).
5. **A late payout is credited only by a transaction that names the order** (order id or Relay's request id in its input), is not
   ours, mined after the order, and pays us directly or is a call to Relay's depository at the stated amount. If Relay's real payouts
   match neither shape (C5-I-1: measure before launch), no late payout can be credited until the judge follows them. The request id
   is the quote's, not bound by the order hash; it only ever helps an operator hand-in pass, never pays more than the order sent.
6. **The scan's bounds**: 10 MB of logs and 4 MB of receipts a collection-pass, 72 passes' worth a day (≈ 10.5 T cycles a day a
   collection at worst); a range too big to read (however a provider says it, or failing 6 times at one start) is read through
   Seaport's and Blur's logs and its other marketplaces' sales later; 32 deferred ranges at most.
7. **The first post stays sales-confirmed** (three counted sales), not listings-alone (§5).
8. **A5-L-2 is closed by backlog-first delivery**, not by raising `per_collection_items` to 22 or `global_items` to 42 at step 2
   (§6); a delivery within two days of its deadline still goes first.
9. **An upgrade is refused while what custody holds would not leave in time at its limits** (A5-L-1). An emergency LOWERING of the
   outflow while custody is full is therefore refused too; lowering the caps alone is always allowed, and the pause (H1) stops every
   outflow at once.
10. **The monitor pages the exact staged step as EXPECTED (SEV2)**, not SEV1; the guardians still check it against its
    announcement.
11. **The keyless Ethereum pair's depth** (measured, §3): `ethereum-rpc.publicnode.com` serves logs about 10,000 blocks deep (about
    a day and a half), so the canister's two-provider read of the 7-day sale window stops there (the walk back stops at `Deep`; the
    floor counts what is read). Either pick a second keyless provider that serves the window (`xrpc.cl/eth` did at 130,000 blocks),
    or accept a shorter sample.

## 10. Left open (for round 6)

- E5-L-2 (the exit rule's tripwire half), A5-L-3 (golive-check keyless residuals), B5-L-2's remainder, C3-L-1, E3-L-2, B3-L-1,
  B-L-6's automatic reads, A-L-2, B-L-5, C-L-5, D-L-3, D-L-6; the Infos listed as unchanged in §8.
- F5-I-1: the other 4663 signers (`looper.rs`, `redeem.rs`, `frogflow.rs`) still wait with no exit when a mined transaction of theirs
  is shown by no provider; `home_nonce_tx` could serve them too.
- The monitor does not page a balance rise at our addresses while a written-off order is uncredited (D5-I-3's first half), nor the
  scan's lag (the reading's note says it).
- The reviewers' own PoCs re-run on this code (`fixes-round5-poc/`): every F, C, B PoC and E5-L-1 fail on their attack assertions;
  D5-L-1's PoC still passes because its fake wrapper answers `deliveryTx` 0 (the real `recordDelivery` sets it, and
  `audit_nft_d_r5` holds the exemption with it); E5-I-2's PoC re-runs round 4's string test, which is kept beside the new one.

## 11. As built (2026-09-28)

**Commits** (branch `nft-side`, on `8ef82d6`):

| Commit | What |
| --- | --- |
| `e811911` | Canister: F5-M-1, E5-L-1, C5-M-1, C5-L-1, C5-L-3, B5-M-1, B5-M-2, B5-L-1, B5-L-2 (partly), B4-L-2's edge, A5-L-1, A5-L-2, F5-L-1, E5-I-2, E5-I-3; the `.did`; the round-2 and round-4 fixtures re-encoded, `eth-state-round5.hex`, `state/eth-state-1dc2a61.hex`; `tests/audit_nft_r5.rs`; the older tests that encoded round 4's write-off freeing the leg or the solver-only judge updated (`audit_nft_r3.rs` ×2, `audit_nft_r4.rs` ×2) |
| `d2d8c66` | Monitor and runner: `eth-home-unseen`, the hold's texts, D5-L-1, D5-L-2, D5-I-3's wording, `staged.mjs` (EXPECTED staged steps, the exit rule on proposals); `audit_nft_d_r5`, `staged_r5`, `ops/test/audit_nft_r5.test.ts`; `audit_nft_d_r3` updated (a watched Stuck now needs its `last`) |
| `4fed345` | Go-live: the launch file's option (c), golive-check `--stage` / `--proposal` / A5-I-1, the monitor configuration's `launch_file`, the dry run |
| `536860c` | Site: the Trust page, `launch-gates.md` G7 (both copies) |
| (this document's commit) | This document; `fixes-round5-poc/`; `docs/v7/phase1-nft.md` (§5, §5.1 the runbook, §11.12), `phase1-nft-pooled.md` §6 J7 |

**Results:**

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites; unchanged: no contract changed) |
| The Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **683 passed**, 0 failed (was 647: +33 `tests/audit_nft_r5.rs`, +3 unit: `canister.rs` `a5_l1_…`, `evmsend.rs` and `ethnft.rs` `f5_m1_…`; `wrapper_abi` reads `sentTx` / `undeliverable` from the artifact) |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed (unchanged) |
| ops | **58 passed** (+1); typecheck clean |
| monitor | **203 passed** (+5 `audit_nft_d_r5`, +3 `staged_r5`) |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | **72 passed**; typecheck clean |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 checks ok on the v3–v6 stack; 536 on the five-collection NFT stack; **662** on the launch go/no-go read, now on the COMMITTED `nft-pilot.json`; 51 refusals `ok`, new: J7's 61 at 24 a day from a copy, a canister trip factor of 12 (A5-I-1), a step-2 canister read as step 1, a both-at-once and a caps-first `--proposal`, a `--proposal` without its step; and `--stage 2` and `--proposal` steps 2 and 3 passing) |
| The real go/no-go read on the committed launch file | Passes: the dry run's launch read runs golive-check `--canister --launch deployments/launch/nft-pilot.json --monitor-config --install-arg` on it (step 1: the canister's caps are the file's 48-item split, its six outflow limits the file's, the staged plan checks, the exit rule fits) |

**Reproducible build** (`autobuyer/Dockerfile.build`, linux/amd64, two `git archive` exports of `e811911` (no later commit of this
round touches `autobuyer/`), each `docker build --no-cache`): the same hashes both times.

```
3218bff1274c9655aefec2ceac3685b1e97aab4e8e4560957e1b49b065b8d99e  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

`buyer.wasm` is new (7,022,016 bytes; 1,334,124 gzipped with `gzip -9 -n`, under the 2,000,000 proposal limit) and replaces
`429e7ad6…f730` (`fixes-round4.md`) for any install of this branch. The timelock is unchanged. Nothing was installed; the exports, the
two build images, the extracted wasm, the private PoC copies and the anvils were removed (Docker's shared build cache left alone).

**The delta round 6 must cover:** `git diff 8ef82d6..HEAD`:
`autobuyer/canister-rs/{etch_autobuyer.did, src/{canister,ethflow,ethnet,ethnft,evmsend,relayeth}.rs,
tests/{audit_nft_r2,audit_nft_r3,audit_nft_r4,audit_nft_r5,wrapper_abi}.rs, tests/fixtures/{eth-state-round2.hex,
eth-state-round4.hex, eth-state-round5.hex, state/eth-state-1dc2a61.hex}}`;
`monitor/src/{argdiff,candid,config,ethnft,staged,watcher}.mjs`, `monitor/test/{audit_nft_d_r3,audit_nft_d_r5,staged_r5}.test.mjs`,
`monitor/test/d5-state.hex`; `ops/test/audit_nft_r5.test.ts`; `script/golive-check.sh`, `scripts/golive-dryrun.sh`,
`deployments/launch/{nft-pilot,monitor-nft}.json`; `web-rook/src/rook/phase1Trust.ts`, `web-rook/src/content/launch-gates.md`,
`docs/v5/launch-gates.md`; the docs above.
