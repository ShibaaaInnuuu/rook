# Rook second review (R2), area D: the buyer canister and the timelock, after the fixes

Written 2026-09-26. Reviewer: second, independent AI review. Checkout `/Users/jlohayon/Documents/Rook-rev2-canister`, detached at
`46a4f1e` (branch `fixes-2026-09-26`); diff reviewed `e04f49b..46a4f1e -- autobuyer` (31 files, +2,993 / −454) plus the current
source of every file the fixes touched. Read first: `docs/audit/2026-09-26/D-canister-timelock.md`, `fixes-D-canister.md`,
`fixes-W2-services.md` (§8, custodian-only `may_record`).

Nothing was committed, pushed, installed or signed; no key or identity was used. The only change to the checkout is one new
test file, `autobuyer/canister-rs/tests/audit2_canister.rs` (7 tests, all named `audit2_*`). Network use: read-only
`eth_blockNumber` / `eth_getLogs` against `rpc.mainnet.chain.robinhood.com` and `rpc.ordofi.network` (to size a real log and the
response headers). The reproducible builds were run on `git archive` exports in the scratchpad, not in the checkout.

## Summary

| | Count |
| --- | --- |
| Original D findings (4 H, 2 M, 7 L) | **Fixed 9**, **Partially 3** (M-1, L-5, L-7), **Not fixed 1** (L-6, accepted and documented) |
| Original Infos (7) | Fixed 5 (I-1, I-4, I-5, I-6; I-2 documented), Partially 1 (I-3), n/a 1 (I-7) |
| New findings | **0 Critical, 0 High, 2 Medium, 5 Low, 5 Info** |

The fixes are careful and mostly right. H-1's event-based finality works against fakes with the measured limits. The deposit
switch, backoff and caps bound H-2/H-3. The fee pool removes H-4's dependency on listing. The decode trap (M-2) and the
stop/install/start sequence (L-2) are correct.

The two Mediums are:

- **N-1:** the new ticket-log scanner stalls forever on any page whose response is over the 48,000-byte outcall cap.
- **N-2:** M-1's re-pricing and conflict-out can be beaten by the redeemer. The redeemer owns the delivery's output 0 and can
  pin the delivery, then refund and confirm it.

Neither is reachable in Phase 1 at stage-0 caps without a fee spike (N-2) or a burst of about 50 or more redemptions (N-1).
Both should be fixed before stage 2.

## 1. The original findings

| ID | Verdict | Evidence / why |
| --- | --- | --- |
| **H-1** (no delivery on the C8 pair) | **Fixed** (for Phase 1). See N-1 for a new stall mode at scale | The ticket state is read at the fast pin (inside Robinhood's ≈6.2k window). Finality comes from `Redeemed`/`Resolved` events at or below a bounded `finalized`, paged ≤ 8,000 blocks (`ticketlog.rs`, `evmread.rs`). `tests/providers.rs` shows the old reads refused and the new ones answered by fakes with the measured limits. `facts_of` requires the fast-pin state and the final events to agree on item and destination. The idle skip is sound: idle is set only when the walk reached `ticketCount` (no `Pinned` ticket) and no Owed/Signed/Sent redemption lacks events, and the jump is `min(idle − 20,000, fin)`. The only miss would need a >20k-block reorg while `head − fin` > 20k. **Still open:** the fix doc's own go-live gate (a replica or fork run against the real providers) has not been done. |
| **H-2** (junk deposits freeze custody) | **Fixed** | `deposits_on()` is false unless `holder_deposits = opt true` (`frognet.rs:275`, `canister.rs:238`). Plain sats are `NotAFrog` before the marker is judged. `Flow::Return` never trips (`outflow.rs` `trips` for `Return` = none), and I confirmed that 1,000 distinct return asks do not freeze (`audit2_returns_are_rate_limited_but_never_trip_whatever_their_number`). `unfreeze` acks the backlog. The limiter is asked before any outcall in `give_back`. Residuals are in N-8 (deposits ON only). |
| **H-3** (cycles drain via deposits) | **Fixed** | Per tick: one listing, ≤ 8 classifications, ≤ 4 steps (`MAX_DEPOSITS_STEPPED_PER_TICK`). Also: backoff of 10 min × 2ⁿ up to 24 h; `AwaitingProof` not re-read without a proof; `Parked` at 14 days; ≤ 32 open; unreadable transactions set aside after 3 tries; `seen`/`unmarked` pruned; an 8 T cycles floor. The regression `audit_d_a_ticks_reads_do_not_grow_with_a_strangers_deposits` passes. |
| **H-4** (dust flood breaks fee selection) | **Fixed**, conditional on the operator keeping the pool funded | Fee inputs come from `FEE_POOL` (operator top-ups plus own change and conflict outputs ≥ 5,000 sats), each re-verified on every explorer and on ord at use. The listing is used only when the pool is empty. It is still breakable by a dust flood then, but the operator can recover in minutes with `add_custody_fee_input`, not only by a 72 h upgrade. A bad pool entry blocks everything (N-4). The change of a confirmed *earlier* bump version is not pooled (`confirmed(.., None)`), which makes the pool empty sooner. |
| **M-1** (stuck delivery past `deliverBy` → LostRace) | **Partially** | Bumps every 6 h (×1.5, cap 24,500); a conflict-out when refused inside the margin, even while in a mempool; earlier versions tracked; Wrapper `REFUND_GRACE` = 72 h (`Wrapper.sol:70, 578`). The fee math checks out: a conflict over a bump at its cap is ≤ 24,669 < 25,000 (`audit2_a_delivery_bumped_to_its_cap_can_still_be_replaced_by_the_conflict_out`). **Not covered:** (a) the redeemer owns output 0 and can pin the delivery so that neither a bump nor the conflict (both ≤ 25k absolute) can replace it (N-2, PoC passes); (b) the conflict-out is never re-priced once sent (`conflict.rs:231`); (c) while the buyer is paused (guardian, operator or tripwire), nothing is bumped, conflicted or even re-sent (`conflict.rs:218`, `canister.rs` `tick`), so a pause of more than ~96 h spanning a delivery's margin is the old LostRace. |
| **M-2** (`post_upgrade` fails open) | **Fixed** | A decode error traps (`canister.rs:399-403`). The golden fixture `saved-e04f49b.hex` decodes with every record kept (`the_state_the_audited_build_saved_decodes_into_this_build_with_every_record_kept` passes). The timelock traps on `Err`, and `Ok(None)` still inits. A trap refuses the upgrade atomically, so the old code keeps running: it cannot brick. The only upgrade-path brick left is `pre_upgrade` exceeding limits, which needs unbounded state (see "Not covered"). |
| **L-1** (return sent before saved) | **Fixed** | `give_back` writes `d` into `st`, then `io.save(st)`, then `outflow_record`, then `btc_send` (`frogflow.rs` end of `give_back`). The regression `audit_d_a_return_is_saved_before_it_is_sent` passes. |
| **L-2** (upgrade of a running buyer) | **Fixed** (residual N-6) | The timelock runs `stop_canister` → `install_code` → `start_canister`, always starting it. A failed stop leads to `untake` (the epoch is restored). `pre_upgrade` traps while `TICKING`. `TickGuard` is released through cleanup on a trap (ic-cdk-executor 2.0.0 cancels tasks and drops them, so `Drop` runs). `sweep_stale` settles `frog:{id}` for Wanted, Failed or missing frogs. `accept` is called only inside the tick, so a Wanted frog holding a reservation after step 4 really is stale. |
| **L-3** (no `inspect_message`; rate table) | **Fixed** | Buyer: controllers and the operator; `submit_offer` only with a board and a non-anonymous caller. Timelock: role-based, and `execute` for anyone once ripe. I checked that the ops runner (the operator) calls `tick`, the monitor uses queries only (`monitor/src/ic.mjs`: every method is `query` in the `.did`), and the keeper and resolver never call the canister. `Rate` prunes every idle caller and is capped at 1,000. Note: a non-operator calling a *query* method through the update (`call`) endpoint is now refused. That is harmless today. |
| **L-4** (lost updates mid-tick) | **Fixed** | `not_ticking()` or `hold_tick()` is on every update that writes FROGS, FLOORS, HANDINS, SALES or CUSTODY. The source test enforces it, and a manual `grep` of every `borrow_mut` on those statics found no unguarded writer. `FEE_POOL` is outside the ticked copy. `frog_offer` writes only `OFFERS`. Liveness: `submit_offer` is refused during ticks, so the submitter has to retry. That is acceptable, and it is OFF in Phase 1. |
| **L-5** (signed buy held forever) | **Partially** | A 48 h clock leads to `Abandoning`, which spends the float's largest input of the buy back to the float with the float key (`FloatVia`: float script, `sign_float`, H4 `ConflictOut` against the float script). The seller's input is never touched. A confirmation that wins goes to `Sent`; a conflict that confirms goes to `Failed` and the reservation is settled. The float-key spend is correct. **But** a buy whose fee is within 111 sats of 25,000 cannot be conflicted out (N-5). |
| **L-6** (seller re-witness) | **Not fixed** (accepted) | Documented as left open (`fixes-D §4`). `Stranded` stays loud. |
| **L-7** (provider extremes) | **Partially** | Finality is fixed: per provider `finalized ≤ head`, `head − fin ≤ 50,000`, spread ≤ 8,192, lowest taken (`evmread::choose_finality`). The fee rule `min(max, 2·min)` swaps the old lever ("one explorer forces the 100 sat/vB clamp") for a new one: one explorer that is stale, buggy or lying low sets *every* custody rate to ≤ 2× its number. That underprices deliveries and conflict-outs and feeds N-2 (N-3). |
| I-1 | Fixed | `names_this_item` also needs the final `Resolved` event's `acquisitionId`. `outbound.rs` says what is not tied. |
| I-2 | Fixed (documented) | — |
| I-3 | **Partially** | `ensure_btc_network` gates only the custody pass (`custodynet.rs:419`). If it fails, the frog pass (buys, abandon conflicts, deposits) still reads the unchecked explorers in the same tick. Broadcasts still check every time. |
| I-4 | Fixed | 12 h TTL on the resume round (`rules.rs` `RESUME_VOTE_TTL_NS`), with a test. |
| I-5 | Fixed | `outflow::validate_change`: ×4 per proposal, applied in `check_cap_change`. |
| I-6 | Fixed | Deliveries refuse the float and buyer scripts (`redeem.rs` step 1d). Returns exclude custody, float and buyer. |
| I-7 | n/a | Lockfiles unchanged. |
| W2 §8 (`may_record` custodian-only) | Correct | `custodynet.rs` `may_record` compares only `custodian()`, which matches `Wrapper.recordDelivery` (`NotCustodian`). A non-custodian canister parks in `AwaitingRecord`, and `pending_delivery_records` stays for monitoring. |

## 2. New findings

| ID | Sev | Title | Where | PoC (all in `tests/audit2_canister.rs`) |
| --- | --- | --- | --- | --- |
| N-1 | **Medium** | A page of wrapper events over the 48 kB outcall cap stalls the ticket-log scanner for ever: every delivery waits, then refunds | `canister.rs:722`, `ticketlog.rs:21,157-166` | `audit2_a_burst_of_redemptions_in_one_block_stalls_the_ticket_log_scanner_for_ever`: **passes** |
| N-2 | **Medium** | M-1 residual: the redeemer pins the delivery through its own output 0, which defeats both the bump and the conflict-out, then refunds and confirms it (LostRace, one unit unbacked) | `redeem.rs:544-605`, `conflict.rs:217-262` (conflict never re-priced), `custody.rs` 25k ceiling, `Wrapper.refund` | `audit2_a_redeemer_who_pins_the_delivery_defeats_the_bump_and_the_conflict_and_ends_with_frog_and_refund`: **passes** |
| N-3 | Low | L-7's `min(max, 2·min)` lets one low-reporting explorer underprice every custody send | `custody.rs:213-217`, `custodynet.rs` `fee_rate` | `audit2_one_explorer_reporting_a_low_estimate_sets_every_custody_rate`: **passes** |
| N-4 | Low | One fee-pool entry the explorers cannot read blocks every delivery. It is never dropped, and the listing fallback never runs because the pool is non-empty | `redeem.rs:693`, `conflict.rs:334`, `frogflow.rs:1589`, `custody.rs:307` (no remove), `custodynet.rs:450` | `audit2_one_unreadable_fee_pool_entry_blocks_every_delivery_and_is_never_dropped`: **passes** |
| N-5 | Low | A stale buy at the judge's fee bound (24,890–25,000 sats) cannot be abandoned. It ends `Stranded` with its `frog:{id}` reservation leaked | `frogbuy.rs:59`, `conflict.rs:151`, `frogflow.rs:794, 855-870` | `audit2_a_stale_buy_at_the_judges_fee_bound_cannot_be_conflicted_out_under_the_ceiling`: **passes** |
| N-6 | Low | Timelock `execute`: a proposal is invisible to `veto`/`withdraw`/Roles pruning while the stop is awaited, and `untake` restores it anyway. If install fails *and* start fails, the buyer stays Stopped with no event and no way to start it | `timelock-rs/src/canister.rs:188-214`, `rules.rs:362` | reasoned |
| N-7 | Low (deposits ON) | `seen` is pruned to the *confirmed* listing: after a reorg or a listing flap, a deposit is classified again as a second `DepositRec` for the same outpoint | `frogflow.rs:1180, 1210` | reasoned |
| N-8 | Info (deposits ON) | Deposit-intake starvation by 32 junk open deposits, whose returns are paid by custody. "Costs the attacker an inscription" is inaccurate: it is returned. Genuine deposits still trip the **Mint** tripwire (9 a day at stage 0), so a holder of 9 frogs can freeze custody | `frogflow.rs:1195-1216`, `frogflow.rs:1824` | reasoned |
| N-9 | Info | I-3 partial: the frog pass is not gated when the checkpoint check fails | `canister.rs:1256-1275`, `custodynet.rs:419` | reasoned |
| N-10 | Info | `unfreeze` acks every un-done ask blindly, and returns never trip: an anomalous-return bug drains at the Return limit (3/day at stage 0, 32/day at stage 3) indefinitely with no freeze | `outflow.rs` `unfreeze`, `check` | `audit2_returns_are_rate_limited_…` shows the separate 3 + 3 budget |
| N-11 | Info | Scanner catch-up is net ≈ 26k blocks per tick, so a `wrapper_from_block` weeks back delays the first deliveries by days. `MAX_FINALITY_LAG` (50k ≈ 83 min) refuses honest providers during an L1 or batch-posting finality stall (loud, liveness only) | `ticketlog.rs:145`, `evmread.rs:22` | reasoned |
| N-12 | Info | Owed tickets ask the H3 limiter *before* their facts are checked (`redeem.rs:618`), so tickets that can never be sent (N-1 stall) still add to the tripwire's demand and can freeze custody | `redeem.rs:616-620` | reasoned |

### N-1 (Medium): the ticket-log scanner can be stalled for ever by one burst of events

- **The limit:**
  - `IcRpc::logs` asks with `max_response_bytes = 48,000`, which counts the headers (`canister.rs:722`). ordofi's headers are
    ≈ 984 bytes, measured live today.
  - A live Robinhood Chain log is ≈ 556 bytes with 2 topics and 32 data bytes.
  - A `Redeemed` log (3 topics, 160 data bytes) is ≈ 880–900 bytes, so a page holds about **50 `Redeemed` or ≈ 70 `Resolved`
    logs**.
- **What the scanner does:** a failing page is halved down to `MIN_SPAN_BLOCKS = 125` (12.5 s of chain) and never goes lower.
  If ≥ ~50 wrapper events fall inside any 125-block window (in the limit, one transaction), every page from the cursor fails at
  every span.
- **The effect:**
  - The cursor never passes that block, and no later ticket's events are ever read.
  - Every delivery waits (`facts_for`), reaches the margin, is refused, and every ticket refunds, until an upgrade raises the cap
    (72 h, or 7 days at stage 3).
  - Meanwhile each resolved ticket still asks the limiter (N-12), so at stage 0 nine of them in a day also freeze custody.
- **Who can trigger it:**
  - A holder with ~50 wX redeeming through a contract loop in one block. They get their tokens back by refund, so the cost is gas
    and about 10 days of capital.
  - Or a batched redemption rush.
  - Unreachable at stage 0/1 (3 or 12 items). Reachable from stage 2 (48 per collection) and easy at stage 3 (192).
- **PoC:** 60 `Redeemed` logs in one block, then an honest ticket 50 blocks later, over a week of ticks. Result: "cursor stuck at
  72816686 (burst at 72816687), span 125, 1008 failures: …Http body exceeds size limit of 48000 bytes"; the honest ticket's events
  are never read.
- **Fix:**
  - Query only what is needed: `topics = [[Redeemed, Resolved], [ticketId…]]` for the redemptions lacking events, which is
    bounded by open redemptions. Keep the cursor scan only as a hint.
  - Or, at `MIN_SPAN`, retry with a large `max_response_bytes` (≤ 2 MB), or split by topic.
  - Add a test with a response-size cap to `tests/fakerpc`.

### N-2 (Medium): pinning beats M-1's bump and conflict-out

- **The setup:**
  - Custody's delivery pays output 0 to the redeemer's own taproot key.
  - While the delivery is below the market (any fee spike above its rate; first sends are clamped at 100 sat/vB; see also N-3),
    the redeemer hangs a large low-feerate child on output 0 (≈ 100 kvB).
  - BIP125 rule 3, which cluster-mempool RBF keeps, then asks any replacement for more absolute fee than parent plus child.
  - Every bump (≤ 24,500) and the margin's conflict-out (≤ 25,000) are refused by mempools.
- **What the canister does:** it keeps re-signing bumps, then conflicts out and waits. `conflict::step` just re-sends the same
  bytes; it never re-prices.
- **The payoff:**
  1. At `deliverBy + REFUND_GRACE` the redeemer calls `refund` and gets the wX back.
  2. They CPFP their own child, and the pinned delivery confirms.
  3. `EarlierConfirmed`, then `Confirmed`, then `record`, which reads `Refunded` and records **LostRace**.
  4. The redeemer holds the frog and the token; one unit is unbacked.
- **What the attack needs:** the delivery must stay under the market until refund. The pin stops replacement, not mining. So it
  is opportunistic in sustained spikes, which have lasted days (Dec 2023, Apr 2024).
- **PoC:** a mempool that refuses every replacement of the first delivery. Result: phases `Signed, Sent, Signed, Conflicting`;
  865 replacements refused; 19 threshold signatures; ends `LostRace { txid: <original> }`.
- **Fix (either works; the first is the robust one):**
  - On the EVM side, make LostRace unprofitable: a `recordDelivery` after a refund re-burns the refunded unit from the owner or
    escrows it for the grace (the fix doc's §6 option 1b). Or let `refund` require that the canister attests the delivery's fee
    input was spent by a conflict.
  - Build deliveries as TRUC (`nVersion = 3`): the child is limited to 1,000 vB, and sibling eviction and package RBF make pins
    cheap to beat.
  - Also re-price the conflict-out every few hours up to the ceiling, and alert when a delivery's output 0 has an unconfirmed
    child.

### N-3 (Low): one explorer reporting low sets every custody fee rate

`combine_rates(&[1, 150]) == 2`; `combine_rates(&[0, 150]) == 1`. Before the fix, one explorer could only make custody *overpay*
(bounded at 25k sats). Now one stale or buggy estimator makes custody *underpay*: every delivery, return and conflict-out is
priced at ≤ 2× the low figure, and conflicts are never re-priced. That is the precondition for N-2 without any market spike.

**Fix:** take the max (the safe side) and treat a disagreement of more than 2× as a wait plus an alert. Or use a median of ≥ 3
sources.

### N-4 (Low): an unreadable fee-pool entry blocks every delivery

- **Why it blocks:**
  - `choose_fee` calls `io.output(&u.outpoint).await?` per candidate (`redeem.rs:693`). `pick_extra` (`conflict.rs:334`) and
    `give_back` (`frogflow.rs:1589`) do the same.
  - So one entry whose read errors (a mistyped vout or txid through `add_custody_fee_input`, or an explorer 404) aborts the whole
    choice before later candidates are tried.
  - The smallest entries (5,000 sats) sort first.
  - Entries are dropped only when read as *spent*. No operator method removes one. A non-empty pool disables the listing fallback.
- **PoC:** the pool holds a good 20,000-sat entry plus a bogus 5,000-sat one. A day of ticks gives "waits: … HTTP 404", with
  0 signatures and 0 sends. The control (good entry only) signs.
- **Recovery:** 32 more `add_custody_fee_input` calls (FIFO eviction), or an upgrade.
- **Fix:**
  - `continue` on per-candidate errors.
  - Drop an entry that verifies wrong, or that fails N times.
  - Add `remove_custody_fee_input`.
  - Fall back to the listing when no pool entry verifies.

### N-5 (Low): a buy at the fee bound cannot be abandoned

- **The gap:** `fee_bound` reaches 25,000 for spends ≥ ~833k sats. The float conflict needs `fee + 111` (BIP125 rule 4), and
  `conflict::build` refuses anything over 25,000. So buys paying 24,890–25,000 sats cannot be conflicted out.
- **Who can use it:** the seller chooses the fee within the bound. A seller who also keeps a conflicting spend alive gets the
  frog `Stranded` after 48 h.
- **What is left behind:** the buy bytes stay valid. `frog:{id}` stays `in_flight` for ever (`sweep_stale` skips `Stranded`),
  holding one H2 slot.
- **Fix:** cap `fee_bound` at `MAX_DELIVERY_FEE_SATS − 500`, as `MAX_BUMP_FEE_SATS` already is, and settle the reservation of a
  frog that went from `Abandoning` to `Stranded`.

### N-6 (Low): timelock `execute` edges

**Proposals invisible during the stop:**

- `take_ready` removes the proposal before `stop_canister` is awaited, which can take minutes while a tick drains. During that
  window a guardian's `veto` or the proposer's `withdraw` fails ("no such proposal").
- A `Roles` proposal executed in the same window prunes `pending` but not this proposal.
- If the stop then fails, `untake` pushes the proposal back, runnable at once by anyone, even if the veto was attempted or its
  proposer lost the role.

**A buyer that stays stopped:**

- `NotStarted` is recorded only when the install *succeeded*.
- If the install fails *and* `start_canister` fails, the buyer stays Stopped with no event.
- The timelock has no method to start it, so the only way out is a new upgrade after the full delay.

**Fix:**

- Keep taken-but-unfinished proposals in a visible state that veto and withdraw can hit.
- Re-check `by ∈ proposers` and the veto on `untake`.
- Add a guardian `start_target` (starting is safe), and record start failures unconditionally.

### N-7 (Low, deposits ON): duplicate deposit records after a reorg

- **The cause:** `st.seen.retain(|o| utxos.contains(o))` uses the *confirmed* listing (`frognet.rs:271-273`). A deposit
  classified at 1 confirmation that is reorged out loses its `seen` entry.
- **The effect:** when it confirms again, it is pushed as a second `DepositRec` for the same outpoint; nothing checks
  `st.deposits`.
- **Outcomes:**
  - A minted one is adopted twice, which the cap book dedups.
  - A refused one gets two returns signed with different fee inputs, which breaks "sign once per record" and wastes fee.
- **Fix:** dedup against `st.deposits` before pushing; prune `seen` only for outputs read as spent.

## 3. Areas the brief asked about: what I checked and the conclusion

- **Paged scan from a saved cursor:**
  - Cursor corruption: the cursor is set before paging and only ever moves forward to a page end. A lower finality simply reads
    nothing. There are no gaps: `from = cursor + 1`, and the idle jump is bounded by `fin`.
  - Reorgs: pages never pass the lowest bounded `finalized`.
  - Providers disagreeing: exact equality of projected logs is required; a disagreement fails the page loudly and halves the span.
    That recovers, as tested.
  - Bad event payloads: `ingest`'s `?` could stall on a malformed event, but `Wrapper.redeem` enforces a taproot string and
    `Resolved` ids are vault ids or `DEPOSIT_FLAG | n` (rentals too), all of which `frog_id_of_word` decodes.
  - **The real stall mode is response size (N-1).**
- **Holder-deposit switch, return budget, acked asks:**
  - OFF by default and taken from every upgrade's argument. An upgrade that omits it turns deposits off mid-flight; returns in
    flight stay reserved and are just not tracked. Info.
  - Deliveries still trip. Returns never do, by design, with a separate 3 + 3/day budget (N-10).
  - Acked asks expire with their original ask's window. After a long freeze they are simply gone, and so are the old `done`
    entries, so no false re-trip.
  - The tripwire is not bypassable for deliveries.
- **Deposit processing and parking:** bounded as claimed (H-3). Parking wakes on `frog_proof`, and `refuse_deposit` works on
  `Parked`.
- **Fee pool and `add_custody_fee_input`:**
  - Only the operator or a controller can add, and not while paused or mid-tick.
  - Entries are ≥ 5,000 sats, deduplicated, capped at 32 with FIFO eviction.
  - A bad entry cannot be *spent* (it is re-verified), but it can block (N-4).
  - Double use is prevented by `reserved` (deliveries, returns, sale moves, conflicts).
- **RBF re-pricing and conflict-out:**
  - The fee-cap math is correct.
  - A confirmed earlier version is detected through `outspend` in both `Signed`/`Sent` and `Conflicting`.
  - The 72 h grace gives the conflict 96 h (margin plus grace).
  - Gaps: pinning (N-2), no conflict re-pricing, nothing done while paused.
- **Trap on decode failure:** the upgrade is refused atomically and the old code keeps running, so it cannot brick.
- **Stop/start and `pre_upgrade` mid-tick:**
  - A stopped canister has no live tick, because `TickGuard` drops even on a callback trap.
  - A direct controller upgrade mid-tick is refused and can be retried.
  - Residual: N-6.
- **`inspect_message`:** no legitimate caller is blocked. The runner is the operator, the monitor uses queries, the keeper and
  resolver never call the canister, and the timelock's `execute` is open to anyone once ripe (the rehearsal scripts call it
  anonymously, which is still admitted).
- **Writes refused during a tick:** correct. Liveness costs only a retry, and every refusal returns `Err`, never a trap.
- **Buy abandonment after 48 h:** the float-key spend is correct (float script, float key, the seller's input untouched, largest
  float input, H4 `ConflictOut` against the float script). Residual: N-5.
- **Provider-bound rules:** finality bounds are right. The fee-rate rule is N-3.

## 4. Test results

| Suite | Result |
| --- | --- |
| `autobuyer/canister-rs` `cargo test` (toolchain 1.98.1) | **466 passed, 0 failed**: 459 existing (matches `fixes-D §8`) + 7 new `audit2_*` |
| `autobuyer/timelock-rs` `cargo test` | **10 passed, 0 failed** |
| New: `tests/audit2_canister.rs` | 7 passed. N-1, N-2, N-3, N-4 and N-5 assert the weakness (they turn red once fixed); one bump-cap check and one returns-budget check are positive |

**Reproducible build** (`autobuyer/scripts/reproducible-build.sh`, Docker linux/amd64, on a `git archive` export of `46a4f1e`
with no audit test inside), run twice, the second with `--no-cache`, **identical both times**:

```
a2c7ef89bd31ea02e29148c7938b40381831766c9336566bff8b1a006185d8f0  buyer.wasm
8089983478c4260faea4d2ffdfd293022259ec9c8e0a616a6a1d2742ed6bb8de  timelock.wasm
3186e4f929570348d3209d6da76975c861f48ac0d00454ca01a04a55c2ef020a  timelock-rehearsal.wasm
```

- Both timelock hashes equal the ones `fixes-D §8` recorded for `4e8c78f`, so the timelock is unchanged and the build reproduces
  exactly.
- The buyer's hash is new, as W2 §8 said it would be: `may_record` changed. Nobody has recorded a reference for `46a4f1e`, so
  record this one.
- The buyer is 5,076,351 bytes, **1,002,193 gzipped** (the timelock's limit is 2,000,000).
- A native macOS arm64 build gives different hashes for all three, which is expected. That is why the Docker build is the
  canonical one.

## 5. Not covered

- **No replica or mainnet-fork run** of the scanner or the delivery path against the real C8 providers (the fix doc's own go-live
  gate). My provider numbers came from one live sample. Real HTTPS-outcall consensus across 13 or 34 nodes was not run: bucket
  straddling, and the extra rate-limit risk of every replica querying ordofi.
- Cycle cost per tick after the fixes. I checked outcall counts, not cycles.
- A `pre_upgrade` size and instruction drill at stage-3 scale (G8). `deposits` (finished records), `redemptions` and `unread` are
  never pruned; attacker growth is expensive, but it is unbounded.
- `cargo clippy` and `cargo audit` (not installed; the lockfiles are unchanged since the first review).
- The Solidity side beyond `Wrapper.redeem`/`resolve`/`recordDelivery`/`refund`, the off-chain services, and the monitor's
  alerting on the new states (I only confirmed its IDL knows them).
- The v2 loop, NFT, sale and runes paths (OFF in Phase 1), beyond their L-4 guards.
