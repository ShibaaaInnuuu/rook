# Fixes for area D: the buyer canister and the timelock

Written 2026-09-25 (US Eastern) on branch `fix-canister` (from `fixes-2026-09-26`), before the code. It answers
`D-canister-timelock.md` finding by finding: the design of each fix, the constants it uses, and what it needs from the EVM contracts
or the off-chain services. Nothing here is deployed, installed or signed. The provider numbers below were re-measured read-only
for this document.

## 0. Measured provider limits (2026-09-26, 03:02–03:20 UTC)

All read-only: `eth_blockNumber`, `eth_getBlockByNumber` (`latest`, `safe`, `finalized`, by number), `eth_getBalance` at a
number and at a hash, and `eth_getLogs` against the two C8 RPCs; one `GET …/address/<a>/utxo` per explorer.

| Limit | Robinhood `rpc.mainnet.chain.robinhood.com` | ordofi `rpc.ordofi.network` |
| --- | --- | --- |
| State served behind the head (bisected, `eth_getBalance` at `head − n`) | **6,201 answers, 6,250 fails** (≈ 10.4 min) | archive: 200,000 answers |
| `eth_getLogs` block span, inclusive (`toBlock − fromBlock + 1`) | 8,000,001 answers (no limit found) | **10,000 answers; 10,001 → `-32005` "the network is busy"** |
| `head − finalized` (45 samples, 20 s apart) | **8,096 – 12,023** | 8,096 – 12,023 |
| How `finalized` moves | steps of **3,663 – 3,939 blocks every ≈ 6.3 min** | the same steps |
| Two providers' `finalized` at the same moment | equal, except for **≈ 20 s after a step**, when they differ by one step (3,663 / 3,939 seen) | |
| Heads at the same moment | within 10, and once ordofi **≈ 340 behind** | |
| Block rate | 9.9 blocks/s (5,816 blocks in 587 s): **≈ 5,940 blocks per 10-minute tick** | |

So on 2026-09-26 no block is both finalized and inside Robinhood's state window: the finalized block is 8.1k–12k behind the
head, and Robinhood keeps 6.2k. And one `eth_getLogs` over 8,000,000 blocks is refused by ordofi. This confirms H-1 as written.

Esplora UTXO listing (`/address/1BitcoinEaterAddressDontSendf59kuE/utxo`, 5,830 UTXOs): mempool.space **HTTP 400 "Too many
unspent transaction outputs (>500)"**; blockstream.info HTTP 200, **1,464,318 bytes** (≈ 251 B per UTXO, past the canister's
400 kB cap at ≈ 1,600). This confirms H-4.

## 1. The rules every fix keeps

- **Nothing is weakened to buy availability.** Every signing path keeps H1 (pause), H2 (caps), H3 (limiter), H4 (outbound
  verifier), H8 (conflict-out) and "save, then send".
- **Stable state stays decodable.** Every new field is an `Option` (candid `opt`), every new enum case is added, never
  reordered, and every new top-level element joins the end of the upgrade tuple. A golden fixture saved by the baseline build
  (`e04f49b`, `tests/fixtures/state/saved-e04f49b.hex`) must decode into the new types (M-2's test).
- **Every outcall a stranger can cause is bounded per tick**, whatever the number of things the stranger creates.

## 2. The Highs

### H-1: reads the two providers can actually serve

**Design: the ticket's facts come from events that are final, not from state at the finalized block.**

1. **The ticket's state** (`ticket(id)`) is read at the **fast pin**, as every other read of the tick is: one bucket (1,024)
   below the lower provider's rounded head, so 1,024–2,048 + lag behind the head, inside Robinhood's 6.2k window with a 3× margin.
2. **Finality comes from the wrapper's own events.** The contract emits `Redeemed(ticketId, owner, round, destination)` at the
   burn and `Resolved(ticketId, acquisitionId, deliverBy)` at the draw. Logs need no historical state, so both providers serve
   them at any depth. H4's facts become:
   - `resolved`: the ticket reads `Resolved` at the fast pin **and** its `Resolved` event is at or below the finalized block;
   - `destination_script`: the ticket's destination (fast pin); `event_destination_script`: the finalized `Redeemed` event's;
   - `names_this_item`: the ticket's acquisition (fast pin) **and** the finalized `Resolved` event's `acquisitionId` both equal
     the redemption's. This is a second, independent read, which also answers I-1.
   - `finalized: true`, meaning "the events these facts rest on are final".

   After `Resolved`, a ticket can only become `Delivered` (by us or the keeper) or `Refunded` (after `deliverBy`); the rule
   (`delivery::decide`) already reads that at a fresh fast pin immediately before sending. A reorganisation can therefore no longer
   un-redeem the ticket or change its item or its destination under a signed delivery, which is exactly what H9 asked of
   the finalized read.
3. **Logs are paged from a stored cursor** (`ticketlog.rs`, saved inside `Custody`): the wrapper's `Redeemed` and `Resolved` logs
   (one `eth_getLogs` with `topic0 ∈ {Redeemed, Resolved}`), in pages of at most **8,000 blocks** (ordofi's 10,000 less 20%),
   never past the finalized block, at most **4 pages a tick** while behind. The chain makes ≈ 5,940 blocks a tick, so one page a
   tick keeps up and four catch up at ≈ 32k blocks a tick. A page that fails is retried at half the span next tick (a burst of
   events larger than the response cap), down to 125 blocks, and grows back after a success. What was found is kept per ticket
   (the destination; the acquisition id) and pruned when the redemption finishes.
4. **Idle skip.** A tick in which every ticket is walked and no `Owed` redemption lacks its events records the fast pin as
   `idle_through`. The next scan starts no earlier than `min(idle_through − 20,000, finalized)`: nothing relevant can lie below a
   pin at which every ticket was already accounted for (the 20,000-block re-scan covers a reorganisation below the fast pin).
   An idle custodian makes **no** log reads.
5. **The finalized block, bounded (L-7).** Every provider's head and finalized, each rounded to the bucket; each provider's
   `head − finalized` must be ≤ **50,000** (4× the measured 12,023) and the providers' finalized blocks within **8,192** of each
   other (2× one step); the pin is the lowest. A provider that reports finality too deep, or not at all, is refused by name.
6. **Loud, not silent.** Every scan records its outcome (`TicketLogs.failures`, `last_error`, the cursor's lag behind finality).
   A failing scan writes `custody: TICKET LOGS NOT READ (n ticks): …` to the log (not a "waits" line, so it is noted), and a
   redemption waiting on events says the scanner's error. `ticket_logs()` (query) shows it all for the watcher.
7. **The provider layer is pure and tested against fakes with the measured limits.** `evmread.rs` holds the multi-provider logic
   (the fast pin, finality, agreed `eth_call` and `eth_getLogs`) over an `Rpc` trait; `canister.rs` implements it with the
   existing transforms. `tests/fakerpc` has a Robinhood-like provider (state only 6,200 behind its head, logs unbounded) and an
   ordofi-like one (archive state, logs refused past 10,000 blocks with `-32005`), both with the measured finality lag and step,
   and one that lags a step behind the other. `tests/providers.rs` shows the old reads (state at the finalized block; one
   8M-block log query) are refused by these fakes, and the new ones answer. `tests/redeem.rs`' world now serves the ticket events
   through the same fakes, so the delivery flow cannot pass against a provider that answers everything again.
8. `REDEEMED_LOOKBACK_BLOCKS` (8,000,000) and `pin_finalized` are removed.

**Install argument:** one new optional field, `wrapper_from_block : opt nat64` (the first block the scanner reads from when it
first runs and is not idle; `null` = the finalized block less 200,000). Phase 1 may leave it `null`: no ticket can exist
before the canister names the wrapper, so the first tick is idle.

### H-2: junk deposits can no longer freeze custody

1. **Holder deposits have a switch, off by default.** New optional install field `holder_deposits : opt bool`; `null`/`false`
   means the deposits pass does not run at all: no classification, no reads, no returns. Phase 1 leaves it `null`. Turning it on
   is an upgrade argument, published for the timelock's delay like the rest.
2. **Plain sats are never returned.** A marked output that carries no inscription and no rune is `NotAFrog`, whatever its marker
   says, and is checked before the marker's validity. Only an inscription or a rune can now be refused and returned, so a junk
   return costs the attacker an inscription each.
3. **Returns have their own limiter flow.** `outflow::Flow::Return`: its own budget (the item limits, counted separately from
   deliveries), and it **never trips the freeze**. A return is verified by H4 to send one deposit back whole to its own sender, and
   its rate is still bounded (the limit and `RETURNS_PER_DAY`), so what the tripwire was for (forged tickets, a broken signing
   path) keeps its trip; demand that a stranger can manufacture does not.
4. **A resume does not re-trip on the same backlog.** `unfreeze` acknowledges the keys that were asking (`Outflow.acked`, kept
   for the window): they still wait on the limit, but no longer count toward the trip. New keys still do, so a continuing forgery
   re-freezes.
5. **The cheap checks first.** `give_back` asks the return rate limit and the limiter before any outcall.

### H-3: a tick's cost no longer grows with a stranger's deposits

With deposits off (H-2.1), nothing is read. With them on:

- at most **4** unfinished deposits are stepped a tick (`MAX_DEPOSITS_STEPPED_PER_TICK`), the ones due earliest;
- each deposit backs off exponentially after a wait: 10 min × 2ⁿ, at most 24 h (`DepositRec.next_check`, `tries`);
- a deposit waiting for a membership proof is not read again until a proof for its inscription is handed in (`awaiting`);
- one waiting **14 days** for a proof is `Parked` (finished; the operator may still refuse it, which returns it, or hand in the
  proof, which wakes it);
- at most **32** unfinished deposits are tracked; classification adds none past that;
- a custody output whose transaction cannot be read 3 times (for example padded past the 200 kB cap) is set aside in
  `unread` and not retried;
- `seen` is pruned to the outputs still unspent, so it stays bounded by the UTXO set;
- below **8 T** cycles the deposits pass is skipped (deliveries are not).

Per tick the deposits pass then makes at most 1 listing + 8 classifications + 4 steps of outcalls, whatever a stranger sends.

### H-4: fee inputs no longer depend on listing a public address

1. **A fee pool held in the canister** (`FEE_POOL`, the upgrade tuple's twelfth element): custody's own change outputs and
   conflict outputs (added when the transaction that made them confirms), and top-ups the operator registers with
   `add_custody_fee_input(outpoint)` (operator, refused while paused or ticking). Nothing is trusted from the pool: at use, each
   candidate must show, on every explorer, custody's script, confirmed, unspent and the recorded value, and ord must show it
   carrying nothing (`fee_eligible`, as before). A spent entry is dropped. At most 32 entries.
2. **Listing is only a fallback**, when the pool has no usable entry, and it ignores outputs under **5,000 sats**
   (`MIN_FEE_INPUT_SATS`), so dust is never a candidate even there.
3. No sweep is needed: a stranger's dust is never listed, chosen or spent.

The discovery of holder deposits still lists the address (deposits are off in Phase 1). Listing more than 500 UTXOs fails on
mempool.space, so a dust flood stops deposit discovery until a paged discovery exists; that is recorded as open (§6), and
it cannot touch deliveries any more.

## 3. The Mediums

### M-1: a delivery stuck in a mempool is re-priced, and conflicted out before the deadline

- `Signed`/`Sent` records gain `bump: opt Bump { first_at, last_at, fee, earlier : vec txid }`.
- A delivery not confirmed **6 h** after it was last signed is checked: the rule runs again on a fresh read.
  - **Refused** (inside the 24 h margin, refunded, anything): straight to H8's conflict-out, even while the delivery is in a
    mempool. The conflict pays more than the delivery (BIP125 rules 3 and 4), so it replaces it.
  - **Send**: an RBF bump. Same frog, same fee input, same destination, output 0 unchanged; the fee is
    `max(1.5 × old, old + vsize·1 sat/vB + 1, vsize × today's rate)`, capped at `MAX_DELIVERY_FEE_SATS` (25,000), taken from the
    change. Verified by H4 against the facts, saved, then sent. No change output, or at the cap: no bump, and the margin's
    conflict-out is the answer.
- Every earlier version's txid is kept. Who spent the fee input (`outspend`) says which version confirmed, so a confirmation of
  an earlier version is recorded with its own txid; the conflict treats an earlier version like the leaked one.
- The 100 sat/vB clamp stays for the first send; bumps go past it up to the 25,000-sat ceiling (≈ 117 sat/vB with change).
- **EVM (for the Solidity side):** `Wrapper.refund` should wait a grace period after `deliverBy` (≥ 24 h; 72 h suggested) so the
  canister's margin conflict-out has time to confirm. Without it, a conflict that does not confirm within the margin can still
  lose the race (as now, but only against a miner who mines the lower-fee, replaced delivery).

### M-2: `post_upgrade` traps on a decode failure

`stable_restore` failing traps (the IC's install is atomic, so the upgrade is refused and everything is kept). Same in the
timelock. The golden-fixture test decodes the baseline build's saved state with the new types.

## 4. The Lows

| ID | Fix |
| --- | --- |
| L-1 | `give_back` writes the `Returning` record into the state and saves it before `btc_send`; the frogs fake world asserts save-before-send for Bitcoin sends too. |
| L-2 | The timelock's `execute` stops the buyer, installs, and starts it again (always starting it, whatever happened). A stop that fails is recorded, the buyer is started, nothing is installed, and the proposal goes back to pending with its epoch. The buyer's `pre_upgrade` also traps while a tick runs. Stale `frog:{id}` reservations whose frog is not acquiring are settled every tick. |
| L-3 | `canister_inspect_message` on both canisters. Buyer: controllers and the operator; `submit_offer` only while a board is configured, never anonymous. Timelock: proposers for `propose_*` and `withdraw`, guardians for `veto`, `pause_target` and `resume_target`, and `execute` only for a pending proposal whose delay has passed. `Rate` prunes every caller's stale entries and is bounded. |
| L-4 | Every update method that writes state a tick holds a copy of (`FROGS`, `FLOORS`, `HANDINS`, `SALES`, `CUSTODY`) refuses while a tick runs (`not_ticking`), and a source test keeps it so. |
| L-5 | A frog buy `Signed` and never seen for 48 h, or in a mempool for 72 h, is abandoned: one of its float inputs is spent back to the float by H8's conflict machinery with the float key (`FPhase::Abandoning`), and then the frog fails and its reservation is settled; if the buy confirms first it continues as bought. |
| L-6 | **Left open**: telling a key-path-only seller output from one with a script tree needs a proof ordinary wallets cannot give, and moving an extra inscription out of custody needs a sat-range-aware split. Both are more than a cheap fix; the `Stranded` record stays loud. |
| L-7 | The fee rate: the highest estimate, but never more than twice the lowest (`min(max, 2·min)`), then the 1–100 clamp. Finality bounded as in H-1.5. |

## 5. The Infos

| ID | Fix |
| --- | --- |
| I-1 | `names_this_item` now also compares the finalized `Resolved` event's `acquisitionId`; `outbound.rs` says what it does and does not tie together. |
| I-2 | Documented (operator can starve contents verification by clearing the UniSat key; added ords can only refuse). No code change. |
| I-3 | The Bitcoin checkpoint is checked on every explorer before any custody or frog pass reads Bitcoin, cached for 24 h per explorer list (so at least once per wasm and per day). |
| I-4 | Resume votes expire 12 h after the round's first vote (`resume_round_ns`); a pause still clears them. |
| I-5 | `outflow_limits` follow the ×4 rule at upgrade (every limit at most 4× the running one; lowering is free; `trip_factor` may not fall below 2). |
| I-6 | A delivery to the canister's float or buyer address is refused (final); a return never goes to the buyer's address either. |
| I-7 | `paste` is transitive through `ic-cdk`/`candid`; nothing to change here. Re-audited. |

## 6. What the other sides must do

**EVM contracts (for the Solidity agents; not changed here):**
1. `Wrapper.refund`: a grace period after `deliverBy` (M-1), or a reconciliation that lets a confirmed late delivery be recorded
   after a refund (for example, `recordDelivery` on a refunded ticket re-burns the refunded unit from the owner's escrow).
2. Nothing else is required: H-1 reads only events the Wrapper already emits (`Redeemed`, `Resolved`).

**Off-chain services:**
1. **Monitor (K6):** the upgrade-argument decoder learns `holder_deposits : opt bool` and `wrapper_from_block : opt nat64` in
   `Init`, and the limiter's `Flow::Return` and `Outflow.acked`; alert on `TICKET LOGS NOT READ`, on `ticket_logs().failures > 0`
   for more than an hour, and on `Parked` deposits. The custody canary must match every txid of a delivery (the record's current
   txid and `bump.earlier`) and the float's abandon conflicts (`frog_buys`, `Abandoning`).
2. **Operator runbook:** fund custody's fee pool with plain sats (one output ≥ 20,000 sats) and call `add_custody_fee_input`
   (phase1-scope O7 said "custody: nothing"; with an empty pool deliveries fall back to listing, which a dust flood breaks).
3. **Timelock watcher:** the buyer is briefly `Stopped` during an executed upgrade.

## 7. Left open

- L-6 (above).
- Deposit discovery past 500 custody UTXOs (a paged or indexed discovery), needed before holder deposits are switched on.
- A replica or mainnet-fork run of `ticket_facts` against the real C8 providers (the fakes carry today's measured limits; the
  limits can change). Recommended as a go-live gate.

## 8. As built (2026-09-26)

Built as designed above, with these differences, each found while testing:

- **H-4:** the listing is the fallback only when the pool is **empty**. A pool whose only entry is claimed by another delivery
  in flight waits for that delivery's change instead of listing the address.
- **M-1:** a bump is capped at `MAX_BUMP_FEE_SATS` = 24,500 (the 25,000 ceiling less room for the conflict-out that may still
  have to replace it: BIP125 asks the replacement for the replaced fee plus its own size, under the same ceiling). Bytes that
  no mempool keeps (`Signed`, never seen, e.g. under a risen mempool floor) are re-priced like bytes that sit in one. Nothing is
  re-priced while the buyer is paused (a custody signature, H1).
- **H-2:** a marked output ord has not indexed yet waits; only an indexed plain-sat output is `NotAFrog` (a plain output is
  never refused, and so never returned, for want of a read).
- **H-3:** `unmarked` is pruned to unspent outputs, like `seen`.
- **L-5:** the conflict-out spends the float's **largest** input of the stale buy (read on every explorer), so it can pay the
  replacement fee alone; the seller's input is never touched.

**The audit's PoCs**, as regression tests that pass only because the attacks now fail (the originals stay in `poc/D` as the
audit's record, as `README.md` says):

| PoC (`poc/D`) | Regression test |
| --- | --- |
| H-1, live measurement | `tests/providers.rs` `audit_d_the_old_finalized_reads_are_refused_by_providers_with_the_measured_limits` (and five more there against the measured limits) |
| `audit_d_anyone_trips_the_custody_freeze_with_refused_junk_deposits_and_it_re_trips_after_resume` | `tests/frogs.rs` `audit_d_junk_deposits_can_no_longer_freeze_custody`; `outflow.rs` `returns_never_trip_and_a_resume_does_not_re_freeze_on_the_same_backlog_but_new_demand_still_does` |
| `audit_d_pending_deposits_are_re_read_in_full_every_tick_forever` | `tests/frogs.rs` `audit_d_a_ticks_reads_do_not_grow_with_a_strangers_deposits` (1, 8 and 40 waiting deposits: one listing a tick each) |
| `audit_d_a_return_is_sent_before_it_is_saved` | `tests/frogs.rs` `audit_d_a_return_is_saved_before_it_is_sent` |
| `audit_d_a_delivery_pending_past_its_deadline_is_never_bumped_or_conflicted_and_ends_a_lost_race` | `tests/redeem.rs` `audit_d_a_delivery_stuck_in_a_mempool_is_re_priced_then_conflicted_out_before_its_deadline_and_never_lost` |

**Tests.** `cargo test`: canister **459 pass, 0 fail** (baseline 432); timelock **10 pass, 0 fail** (baseline 8). New: the
provider fakes (`tests/fakerpc`, `tests/providers.rs`); the golden saved state (`tests/fixtures/state/saved-e04f49b.hex`, written
by the baseline build before any type changed, decoded by
`canister::tests::the_state_the_audited_build_saved_decodes_into_this_build_with_every_record_kept`); M-1 (re-pricing, an earlier
version confirming); H-2/H-3/L-1/L-2/L-5 in `tests/frogs.rs`; the source tests for M-2/L-2 (`post_upgrade` never re-inits,
`pre_upgrade` refuses mid-tick), L-3 (`may_ingress`) and L-4 (every writer of ticked state refuses mid-tick); the timelock's I-4
expiry and L-2/L-3 rules. `tests/redeem.rs` now takes every ticket's events through the fakes; the tests that used 700-sat fee
inputs use 5,000 at 25 sat/vB (the same no-change shape; smaller inputs are never chosen now).

**Not run:** `cargo clippy` (the clippy component is not installed for the pinned 1.98.1 toolchain nor for stable on this
machine) and `cargo audit` (not installed). Neither lockfile changed, so the audit's dependency result (0 vulnerabilities;
`paste` unmaintained, transitive) stands.

**Size:** the buyer's wasm is 5.08 MB, **991 KB gzipped** (the timelock's proposal limit is 2,000,000 bytes).

**Reproducible build** (`autobuyer/scripts/reproducible-build.sh`, the Docker linux/amd64 build, on commit `4e8c78f`):

```
04e1444464fe6a5b99c913e4e0216a99605f90f6f501b5df37c5eb40e3bcca21  buyer.wasm
8089983478c4260faea4d2ffdfd293022259ec9c8e0a616a6a1d2742ed6bb8de  timelock.wasm
3186e4f929570348d3209d6da76975c861f48ac0d00454ca01a04a55c2ef020a  timelock-rehearsal.wasm
```
