# Fix round 3, area D: the buyer canister and the timelock, after the second review

Written 2026-09-26 on branch `r3-canister` (from `fixes-2026-09-26` at `2bb3ee7`), **before the code**. It answers
`review2/R2-canister.md` finding by finding: the design of each fix, its constants, and what it needs from the other sides.
Nothing here is deployed, installed or signed. §9 ("As built") is added after the code, with the test counts and the
reproducible-build hashes.

The rules of `fixes-D-canister.md` §1 still hold: nothing is weakened to buy availability (H1 pause, H2 caps, H3 limiter, H4
outbound verifier, H8 conflict-out, "save, then send"); stable state stays decodable (new fields are `opt`, new enum cases are
appended, the golden fixture `saved-e04f49b.hex` must still decode); every outcall a stranger can cause is bounded per tick.

## 1. N-1 (Medium): the ticket-log scanner never stalls on a dense block

**The failure (R2):** every `eth_getLogs` asks with `max_response_bytes = 48,000` (headers included). About 50 `Redeemed` logs
fill it. A failing page was halved down to 125 blocks and no further, so ≥ ~50 wrapper events inside any 125-block window
(one transaction, in the limit) stopped the cursor for ever.

**Design.** A page that fails *because its answer is too big* is retried inside the same tick, in this order, until it fits:

1. **A larger response cap, same span.** The cap steps `48,000 → 400,000 → 2,000,000` bytes (`ticketlog::LOG_CAPS`; 2,000,000
   is the IC's limit for an HTTPS outcall's response). The cap is paid for whether it is used or not, so it steps back down one
   level after each page that succeeds.
2. **At the largest cap, a smaller span:** divided by 8 each try (`8,000 → 1,000 → 125 → 15 → 1`), down to **one block**
   (`MIN_SPAN_BLOCKS` = 1, was 125). After a success the span doubles again, as before.
3. **A single block that does not fit 2 MB even alone** (over ≈ 2,200 wrapper events in one block) is read **by ticket**, not by
   block: `topics = [[Redeemed, Resolved], [ticketId, …]]` for that block only, in chunks of 64 tickets at the 400,000-byte cap
   (≤ 128 logs, ≈ 115 kB). The tickets asked for are the ones the canister can still need: every redemption that needs events and
   lacks them, plus every ticket from the discovery walk's cursor up to `ticketCount` at the tick's fast pin (not yet resolved,
   so its `Redeemed` event may lie in that block). A ticket whose `Redeemed` event is at or below finality was created before the
   fast pin, so `ticketCount` covers it; nothing the canister will ever deliver can be missed. The progress through the chunks is
   saved (`TicketLogs.dense`), so a block that needs more chunks than one tick allows is finished over the next ticks. When the
   last chunk is read, the cursor moves past the block. A one-ticket query that is still too big at 2 MB is something the wrapper
   cannot emit; after 3 tries that ticket is recorded in `TicketLogs.unreadable` (loud: its delivery waits and refunds) and the
   pass moves on. **So the cursor always advances.**
4. A failure that is **not** about size (providers disagree, a provider is down, finality refused) is what it was: the span is
   halved (down to 125) and the tick's scan ends with the error, loud after the first try and every sixth.

**Per-tick bound.** At most **12 outcall rounds** a tick (`MAX_TRIES_PER_TICK`; one round = one query to each provider) and at
most **8 pages** (`PAGES_PER_TICK`, was 4: N-11). The response cap is the only thing a stranger's events change.
Cycles: an HTTPS outcall costs `(3,000,000 + 60,000·n)·n + 400·n·request_bytes + 800·n·max_response_bytes` cycles on an `n`-node
subnet. On 13 nodes that is ≈ 0.55 B cycles at 48 kB, ≈ 4.2 B at 400 kB and ≈ 20.9 B at 2 MB, per provider. A normal tick
reads one page at 48 kB (≈ 1.1 B for two providers). The worst tick (every round at 2 MB) is 12 × 2 × 20.9 B ≈ **0.5 T cycles**;
on a 34-node subnet ≈ 1.3 T. That worst case needs a real burst of redemptions (each one burns a whole wX), is paid once per
dense block, and the 8 T cycles floor and the low-cycles alert stand.

**N-12, in the same fix: tickets behind a failing scan do not count toward the tripwire.** `redeem::owed` asks the H3 limiter
only **after** the ticket's final events are in (`facts_for`), not before. A ticket whose events have not been read is a wait
that says what the scanner is doing, and asks nothing of the limiter. The tripwire still sees every ticket that could actually
be signed.

**Tests:** a provider fake with a response-size cap (`tests/fakerpc`: `max_bytes`, the per-log size of a Nitro node's minified
JSON, ≈ 1 kB of headers; topic1 filtering). R2's PoC becomes a regression that passes only because the burst is read (the cursor
passes it and the honest ticket's events arrive in the first tick). New: a block of 3,000 events (past 2 MB) read by ticket over
several ticks with the cursor always advancing; a non-size failure still halves and stops; the limiter is not asked while a
ticket lacks its events.

## 2. N-2 (Medium) and M-1's remainders: pinning, re-pricing, and the pause

### 2.1 What can and cannot be done on the Bitcoin side

The delivery's output 0 is the redeemer's own taproot key, by design: the Wrapper records the destination, and a delivery that
paid anything else would not be a delivery. So the redeemer **always** holds a spendable unconfirmed output once the delivery is
in a mempool. "Don't give the redeemer a spendable output until confirmation" is not available:

- an intermediate hop (custody → custody-held output → redeemer) only moves the problem to the second transaction, whose output
  0 is again the redeemer's;
- a relative timelock (CSV) or any condition on output 0 would need a script the redeemer's wallet does not have: the output is
  their key, not ours.

What *is* available is to limit what the redeemer can hang on output 0:

**Deliveries (and returns) are TRUC transactions (`nVersion = 3`, BIP 431).** Under TRUC policy (standard since Bitcoin Core
28.0, October 2024; that the explorers' nodes run it is a go-live check, §8): an unconfirmed TRUC transaction may have **one**
unconfirmed child, the child must itself be TRUC and at most **1,000 vB**, and a non-TRUC transaction cannot spend its outputs
while it is unconfirmed. R2's pin (a ≈ 100 kvB child at 1 sat/vB, ≈ 100k sats of absolute fee at a feerate no miner wants) is
therefore not relayable. To make a replacement pay much more, the redeemer's child must carry its fee in ≤ 1,000 vB, i.e. at a
high feerate, and a high-feerate child makes the package attractive to miners (CPFP), which **confirms the delivery**, before the
refund, which is the outcome the canister wants. The signing path supports it unchanged: BIP 341's digest commits to `nVersion`
whatever its value (`sighash.rs`, already tested with a non-default version), and the builders just write 3. The cross-check
against `@scure/btc-signer` is regenerated for version 3 (`gen-delivery-fixture.mjs`).

**Bounded, not closed.** A TRUC child of 1,000 vB can still carry an absolute fee `F`. A replacement (the conflict-out) must pay
at least `parent fee + F + its own size`. The pin then holds only while the market stays above the package's feerate,
`(parent fee + F) / (parent vsize + 1,000)`, for the whole margin plus grace (≈ 96 h), and it costs the redeemer `F` in fees when
the package does confirm. With the conflict ceiling below (100,000 sats), beating the conflict needs `parent fee + F` > ≈ 99,900
sats, so the market must stay above ≈ 82 sat/vB (99,900 / 1,212 vB) for four days, and `F` ≥ ≈ 75,000 sats (a delivery bumped to
its 24,500 cap) up to ≈ 98,000 (an unbumped one). Without TRUC the same pin needed ≈ 1 sat/vB and 25,000 sats of child fee.
That is still possible in a sustained spike like December 2023, so the canister side reduces N-2 to "a very expensive pin
during an extreme spike"; **the Wrapper-side re-burn (§2.5) is what makes it unprofitable.**

**Constraints to check before go-live (recorded in §8):** both configured explorers must accept a TRUC transaction on `POST /tx`
(Core ≥ 28 behind them), and a signet rehearsal must show one delivery built by this code relayed and mined. If an explorer
refuses version 3, the delivery is not re-signed as version 2 by this code: it stays `Signed`, is re-sent, and at the margin is
conflicted out (the conflict is version 2, spends only confirmed custody outputs, and is relayable everywhere). That is loud and
safe; turning TRUC off would need a new build.

### 2.2 The conflict-out is re-priced (M-1 (b))

- A `ConflictTx` gains `signed_at` and `earlier` (every earlier version's txid; all pay custody, so any of them confirming is
  "the conflict confirmed").
- Every **2 h** (`CONFLICT_REPRICE_SECS`) that our conflict is not confirmed (in a mempool or not seen), it is built, checked by
  H4, saved and sent again at `max(1.5 × old, old + its vsize × 1 sat/vB + 1, today's rate × vsize)`, the leaked fee plus its
  size as before, and at most `MAX_CONFLICT_FEE_SATS`. If the fee input alone cannot pay that and keep dust, one clean custody
  UTXO is added (as today). A confirmation of any version (outspend of the fee input by a txid in `earlier` or the current one)
  settles it as `ConflictConfirmed`.
- **`MAX_CONFLICT_FEE_SATS` = 100,000** (four times the 25,000-sat delivery ceiling). A conflict pays nobody but custody (H4's
  `ConflictOut` purpose), so its fee is the only thing at risk; it exists to keep a frog from leaving against a refunded ticket,
  and 100,000 sats is well under one item. H4's verifier takes the ceiling for `ConflictOut` as `4 × max_fee_sats`
  (`outbound::CONFLICT_FEE_FACTOR`); deliveries and returns keep 25,000. Reaching the top needs a fee-pool entry that large; the
  runbook says so, and a re-price the inputs cannot fund says so in the log.
- **A pin is watched.** Each time a delivery that sits in a mempool is looked at again (every 6 h), the canister reads who spends
  its output 0. An unconfirmed spender is logged as `PIN WATCH: … output 0 has an unconfirmed child …` (a watcher alert), and a
  bump that the mempool did not take (the new version not seen while the old one is) says so too.

### 2.3 While paused (M-1 (c))

The pause (a guardian, the operator, or the H3 tripwire) means **nothing new leaves custody**. It no longer means that custody
stops protecting what already left. While paused the custodian pass runs in a restricted mode (`redeem::tick` with
`io.paused()`):

| Allowed while paused | Why it is safe |
| --- | --- |
| Reads: a delivery confirmed, seen, evicted; an earlier version confirmed | Reads only |
| The rule on a fresh read for a delivery in flight, every 6 h, and **the conflict-out** when it refuses (inside the margin, refunded) | A conflict pays only custody (H4 `ConflictOut`), with a bounded fee: it can only stop an item leaving |
| Stepping and **re-pricing a conflict-out** | As above |
| `recordDelivery` for a delivery Bitcoin has **confirmed** | It moves nothing; it records a fact Bitcoin shows, and it is what stops a refund of a delivered ticket (a LostRace) |

| Not while paused | Why |
| --- | --- |
| Signing a new delivery (`Owed`) or a bump | Either pushes an item out; the pause may be a forgery alarm |
| Re-sending saved delivery bytes | The same: a pause stops items leaving, including by re-broadcast |
| The discovery walk and the log scan | Not needed by any allowed step; resumed with the buyer |
| The frog pass (buys, returns, deposits, floors, sales) | Unchanged: nothing there runs while paused |

So a pause of any length that spans a delivery's margin ends in the conflict-out (or the delivery confirming and being
recorded), not in the old LostRace. `canister::tick` runs this restricted custodian pass when paused (only when a wrapper is
configured); everything else still does nothing.

### 2.4 Bumps

Bumps are TRUC like the first send, capped at `MAX_BUMP_FEE_SATS` (24,500) as before, and never while paused.

### 2.5 The Wrapper-side re-burn (parallel, `r3-contracts`) and what the canister must do with it

The robust fix is on the EVM side: a delivery recorded after a refund takes the refunded unit back, so a LostRace no longer leaves
an unbacked unit. The canister today ends a confirmed delivery whose ticket reads `Refunded` as `LostRace` and does nothing on
chain. When the Wrapper gains the reconciliation, **the canister must call it**: `record` on a `Refunded` ticket goes to the
reconciliation call (the same one-EVM-transaction-at-a-time rule, the same save-then-send, allowed while paused like
`recordDelivery`), and `LostRace` becomes the outcome only when the reconciliation is refused. The exact function and its
arguments come from `r3-contracts`' design; §9 records what was built against it, or what is left for when it lands.

## 3. N-3 / L-7: the fee rate is not set by one low explorer

`custody::combine_rates` becomes the **upper median** of the estimates (`sorted[n / 2]`), then the 1–100 sat/vB clamp:

- two explorers: the **higher** one (one explorer reporting low cannot lower the rate; one reporting high can at most raise it to
  the 100 sat/vB clamp, which is bounded overpayment, ≤ 21,200 sats a delivery);
- three or more: the median, which one explorer in either direction cannot move outside the honest ones;
- one: that one.

**Strong disagreement** (the highest more than 4× the lowest and more than 20 sat/vB apart) is not a wait (a wait would give a
lying-high explorer a liveness lever over deliveries and conflict-outs): the upper median is used and the canister logs
`FEE ESTIMATES DISAGREE: …` for the watcher.

## 4. N-4: an unreadable fee-pool entry never blocks fee choice

- `choose_fee` (deliveries), `conflict::pick_extra` and `frogflow::give_back` **skip** a candidate whose read fails (`continue`,
  not `?`), and try the next.
- A pool entry that fails to read in a choice where another read succeeded (so the explorers are up and the entry is the
  problem) has a failure counted (`FeeEntry.fails`); at **3** it is **quarantined**: moved out of the candidates into
  `FeePool.quarantined` (kept, visible in `custody_fee_pool()`), with a `FEE POOL: … quarantined` line for the watcher. A
  successful read resets the count. An explorer outage fails every read, so it quarantines nothing.
- The operator can remove an entry (`remove_custody_fee_input`, operator only, not mid-tick); re-adding a quarantined one with
  `add_custody_fee_input` restores it.
- **Fallback:** when the pool has unreserved entries but none of them is usable, the address's listing is tried (as it is when
  the pool is empty). A pool whose only entries are claimed by deliveries in flight still waits for their change (as built in
  round 1).

## 5. N-5: a buy near the ceiling can be abandoned

- `frogbuy::fee_bound` is capped at `MAX_DELIVERY_FEE_SATS − 500` (24,500), as a bump already is, so the float conflict (fee +
  its own ≤ 154 vB) always fits under the delivery ceiling.
- The abandon conflict uses the conflict ceiling (`MAX_CONFLICT_FEE_SATS`, §2.2), so a buy already signed at up to 25,000 sats
  (saved by an earlier build) can still be conflicted out. `Abandoning → Stranded` then happens only for what cannot be (the
  verifier refusing our own conflict); its `frog:{id}` reservation is kept then, on purpose: the stale buy's bytes are still
  valid and could still bring the frog into custody.

## 6. N-6: the timelock's `execute`

- **Visible while running.** `take_ready` moves a target proposal to `State.running` (not out of the state). `veto` and
  `withdraw` find it there: a withdrawal, or a veto reaching the threshold, marks it cancelled; a `Roles` change that removes its
  proposer marks it cancelled too. `status()` lists it. A second `execute` of it is refused ("running").
- **Checked after the stop.** Once `stop_canister` returns, `execute` asks the state whether the proposal is still wanted. If it
  was cancelled meanwhile, nothing is installed, the buyer is started, the epoch is restored and the proposal is dropped with its
  `Vetoed`/`Cancelled` event.
- **A failed stop.** The proposal goes back to pending only if it is still wanted (not vetoed, not withdrawn, its proposer still a
  proposer), keeps its veto votes, and may not run again for **10 minutes** (`STOP_RETRY_SECS`), so a stuck stop is not retried in
  a loop by anyone. So it is never runnable twice at once, and never runnable after a veto.
- **Start failures are always recorded.** `NotStarted` is recorded whenever `start_canister` fails: after a failed stop, after a
  failed install, and after a successful one.
- **A way to start the buyer.** New `start_target()`: any guardian (starting is safe: a paused buyer stays paused, its own flag;
  the code is whatever is installed), refused while a proposal is running (it would break that execute's stop). Recorded as a
  new `Started` event. `inspect_message` admits it for guardians.

## 7. Infos and the rest

| ID | Fix |
| --- | --- |
| **I-3 / N-9** | The Bitcoin checkpoint check gates the **frog pass** too (`frognet::tick` calls `ensure_btc_network` first; cached per explorer list, so no extra outcall in the normal case). A failed check skips both passes. |
| **N-7** (deposits ON) | A marked output that already has a `DepositRec` is never pushed again (dedup against `st.deposits`, whatever `seen` says). |
| **N-11** | `PAGES_PER_TICK` 4 → 8 (≈ 58k blocks a tick net of the chain's ≈ 6k while catching up). `MAX_FINALITY_LAG` is left at 50,000: it is loud and liveness-only, and loosening it widens what a lying provider can do; listed. |
| **N-12** | §1. |
| N-8 (Info, deposits ON) | Not changed: deposits stay OFF for Phase 1; before they are switched on, returns should be paid from a per-sender budget and the doc's "costs an inscription" wording corrected. Listed. |
| N-10 (Info) | Not changed: returns are rate-limited (3/day at stage 0) and never trip, by H-2's design. A return-anomaly tripwire (e.g. returns of items that are not the sender's own recent deposit) is left for when deposits are switched on. Listed. |

## 8. Left open, and go-live checks

- TRUC relay: confirm both configured explorers accept a version-3 transaction, and run one signet delivery through the
  rehearsal scripts (`signet-rehearsal.mjs`) before value enters custody.
- N-2 is bounded, not closed, on the Bitcoin side (§2.1); closed only with the Wrapper's re-burn and the canister's call to it.
- The fee pool should hold one entry ≥ 100,000 sats for a conflict to reach its ceiling (runbook).
- The replica / mainnet-fork run of the scanner against the real providers (round 1's go-live gate) is still not done.
- N-8, N-10, `MAX_FINALITY_LAG` (above); L-6 (round 1).

## 9. As built (2026-09-26)

Code and tests: commit `c2fa7ae` on `r3-canister`. Built as designed above, with these differences, each found while testing:

- **N-1, the order of retries.** Shrinking by eighths at the 2 MB cap paid ≈ 21 B cycles for pages that were in fact empty. As
  built, a page too big at 48 kB is retried once at 400 kB (a burst of up to ≈ 440 events, R2's case, costs one extra round);
  if that fails too, the page is **halved towards the dense range at the 48 kB cap** (`TicketLogs.hot` remembers the end of the
  range known to be too big, so successes inside it keep the span rather than doubling), down to one block; only that one block
  is asked at 400 kB and then 2 MB, and then read by ticket. In the 3,000-event test the 2 MB cap is paid at most twice. A
  failure that is not about size halves the span down to one block (not 125). R2's 60-event burst is read in the first tick;
  a 2.7 MB block is read by ticket in 8 ticks (80 `eth_getLogs` a provider, ≤ 12 a tick).
- **N-2, the wrapper side landed as a gate, not a re-burn.** `r3-contracts` (design commit `5b46cea`) chose to refund a resolved
  ticket early only once the custodian attests that no delivery can confirm (`markUndeliverable(uint256)`, custodian only,
  `Resolved` only), or after `LATE_DELIVERY_WINDOW` (30 days) without it. **The canister change that must accompany it is
  built here:** every final refusal carries `Redemption.attest` (`Due` / `Sent` / `Done`). A refusal with nothing of ours ever
  broadcast (the rule's refusal of an owed ticket, the I-6 own-address refusal) is attested at once; a conflicted-out delivery (or
  one whose fee input a third spend took) once that transaction is `ATTEST_DEPTH` = 6 deep (the frog side's depth read: every
  explorer's status, the block still at its height, the lowest tip). The attestation reads the ticket fresh (not `Resolved`:
  nothing to do), needs this canister to be the custodian, is one EVM transaction of ours at a time, saved then sent, and only the
  chain's receipt is believed. It is not sent while paused (it waits for the resume; the 30-day path remains). A delivery that
  confirms before any attestation is recorded as today (`recordDelivery` on a `Resolved` ticket), so a pinned delivery that
  confirms late is backed, not a LostRace (`review2.rs` residual test, second half). **Must go live together:** a canister without
  this path, under the gated wrapper, would leave every honest non-delivery unrefunded for 30 days; the gated wrapper without the
  TRUC and re-pricing work would still see pins that last until the attestation.
- **N-2, the cross-check fixture.** The version-3 txid and digests in `custody.rs` were produced by `@scure/btc-signer` running
  `autobuyer/gen-delivery-fixture.mjs` with `version: 3`; the script itself is outside this change's scope and still says 2 (the
  test keeps the version-2 numbers too, flipped from the same build). Whoever owns `autobuyer/` should change that one word.
- **N-4:** `remove_custody_fee_input` also refuses while paused (as `add_custody_fee_input` does; the pause-guard source test
  requires it).
- **N-6:** `Status.running` lists the proposals being executed with what cancelled them; `Event::Started` records a guardian's
  start. A veto that lands during a `Controllers` execute's single `update_settings` await is recorded but cannot stop it (there
  is no stop to check after); upgrades are the case N-6 was about.

**For the monitor (r3-services), new fields and lines to learn** (all `opt`, so an older decoder still reads the replies):
`TicketLogs.{cap, hot, dense, dense_blocks, unreadable}`, `FeePool.quarantined`, `FeeEntry.fails`, `ConflictTx.{signed_at,
earlier}` (the canary must match every conflict version's txid too), `Redemption.attest`; the timelock's `Status.running` and
`Event::Started`. Alert on log lines `PIN WATCH`, `FEE ESTIMATES DISAGREE`, `FEE POOL: … quarantined`, `UNREADABLE ticket events`,
`read by ticket`, and on `attest` staying `Due`/`Sent` for more than a day. `examples/watch_fixtures.rs` compiles with the new
fields; `monitor/fixtures/canister.json` was not regenerated (outside this change).

**The review's PoCs, as regression tests** (the originals stay in `review2/poc/audit2_canister.rs` as the review's record):

| R2 PoC | Regression (`autobuyer/canister-rs/tests/`) |
| --- | --- |
| N-1 `audit2_a_burst_of_redemptions_in_one_block_stalls_the_ticket_log_scanner_for_ever` | `review2.rs` `audit2_a_burst_of_redemptions_in_one_block_no_longer_stalls_the_ticket_log_scanner`; also `…a_block_past_the_largest_cap_is_read_by_ticket_and_the_cursor_always_advances`, `…even_one_ticket_too_big_to_read_alone_does_not_stop_the_cursor`, `…a_failure_that_is_not_about_size_still_halves_and_stops` |
| N-12 (reasoned) | `review2.rs` `audit2_a_ticket_waiting_behind_a_failing_scan_never_counts_toward_the_tripwire` |
| N-2 `audit2_a_redeemer_who_pins_the_delivery_defeats_the_bump_and_the_conflict_and_ends_with_frog_and_refund` | `review2.rs` `audit2_a_redeemer_who_pins_a_truc_delivery_in_a_spike_is_outbid_by_the_re_priced_conflict_and_gets_no_frog` (60 sat/vB for ten days: 108 replacements refused, the conflict confirms at 83,061 sats, `markUndeliverable` sent); the bound: `audit2_residual_a_pin_in_a_spike_past_the_conflict_ceiling_costs_the_redeemer_its_child_fee` (100 sat/vB: LostRace against the old wrapper with ≥ 75k sats of child fee paid; recorded, backed, against the gated one) |
| M-1 (c) (reasoned) | `redeem.rs` `the_conflict_runs_while_paused_never_touches_the_limiter_and_keeps_the_item_counted` (replaces "the conflict waits for the pause"), `a_delivery_confirmed_while_paused_is_still_recorded_and_nothing_new_leaves` |
| N-2 wrapper side | `redeem.rs` `a_refused_redemption_is_attested_undeliverable_to_the_wrapper_and_only_by_the_custodian` |
| N-3 `audit2_one_explorer_reporting_a_low_estimate_sets_every_custody_rate` | `review2.rs` `audit2_one_explorer_reporting_a_low_estimate_no_longer_sets_the_custody_rate`; `custody.rs` `the_fee_rate_is_the_upper_median_so_one_low_explorer_cannot_set_it` |
| N-4 `audit2_one_unreadable_fee_pool_entry_blocks_every_delivery_and_is_never_dropped` | `review2.rs` `audit2_one_unreadable_fee_pool_entry_is_skipped_counted_and_the_delivery_signs`, `audit2_a_pool_with_no_usable_entry_falls_back_to_the_listing` |
| N-5 `audit2_a_stale_buy_at_the_judges_fee_bound_cannot_be_conflicted_out_under_the_ceiling` | `review2.rs` `audit2_a_stale_buy_at_the_judges_fee_bound_can_be_conflicted_out` |
| (positive) bump cap, returns budget | kept in `review2.rs` |
| N-6 (reasoned) | `timelock-rs` `rules.rs` `a_proposal_being_executed_can_still_be_vetoed_withdrawn_or_pruned_and_a_failed_stop_never_revives_it`, `a_state_saved_before_running_existed_decodes_with_nothing_running`, and the updated `ingress_is_admitted_by_role_…` |
| N-7 (reasoned) | `frogs.rs` `audit2_a_deposit_that_leaves_the_listing_and_comes_back_keeps_one_record` (fails without the fix: two records) |
| I-3 / N-9 (reasoned) | `review2.rs` `audit2_the_bitcoin_checkpoint_gates_the_frog_pass_as_well_as_the_custodian` (source test) |

**Tests.** `cargo test` (toolchain 1.98.1): canister **476 pass, 0 fail** (baseline 459; +14 `review2.rs`, +2 `redeem.rs`,
+1 `frogs.rs`; four existing tests changed with the behaviour: the paused conflict, the fee-rate rule, the 24,500 fee bound, the
paging test's start). Timelock **12 pass, 0 fail** (baseline 10), also with `--features rehearsal`. The golden saved state
(`saved-e04f49b.hex`) still decodes with every record kept. Not run: `cargo clippy`, `cargo audit` (not installed); the lockfiles
did not change.

**Size:** the buyer's wasm is 5,194,102 bytes, **1,024,700 gzipped** (the timelock's proposal limit is 2,000,000 bytes).

**Reproducible build** (`autobuyer/scripts/reproducible-build.sh`, Docker linux/amd64, on a `git archive` export of `c2fa7ae`),
run twice, the second with `docker build --no-cache`, **identical both times**:

```
36ae6e24876d3b697f47c68bf3fcdd7e054cd94f4791fac60707f261c575ef59  buyer.wasm
92248ca7ecb160c093665a8c19b7894cdf7f4a13f74746f3794f4a6e972c48f1  timelock.wasm
742bf4748d805fe47c8ef081076054ce783f90e7774243fa554cd4561b9758b1  timelock-rehearsal.wasm
```

All three are new: the buyer changed throughout, and the timelock's `execute`, `start_target` and state changed (round 1 and R2
recorded `8089983…` / `3186e4f…` for the timelock, `a2c7ef8…` for the buyer at `46a4f1e`). Nothing was installed.

**Still open** (beyond §8): the replica / mainnet-fork run of the scanner; confirming TRUC relay on both explorers and a signet
delivery; the monitor learning the new fields and alert lines; `gen-delivery-fixture.mjs` saying `version: 3`; N-8 and N-10
(deposits OFF); `MAX_FINALITY_LAG`; L-6; the gated wrapper and this canister must ship together.
