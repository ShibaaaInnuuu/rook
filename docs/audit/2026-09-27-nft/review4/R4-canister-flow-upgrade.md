# Round 4, part E: the delivery record (D3-H-1, canister half), the pin floor (E3-M-1), J7 / H3 / deposits at the floor, upgrade safety

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1eb5788`. The delta
read is `git diff 55d8d32..HEAD` in `autobuyer/canister-rs` (the operator's round `fbc9174` and fix round 3 `fb54018`), with the earlier
code where a finding goes back further. Scope:

- D3-H-1's canister half: `delivery_pass` at finality (`eth_sale_receipt`, `ethnft::delivered_in`), `note_sent`, `Step::Sent`,
  `Delivery.sent`, `home_act` / `home_act_paused`, `mined_home`, `NOTE_SENT`.
- E3-M-1: `eth_floor` and every balance, `ownerOf` and nonce read of the flow.
- The operator's round: J7 (`custodycap.rs`: `FIRST_EVM_*`, `first_global`, `stage0`), deposits counted at the module's floor,
  `outflow.rs` (`STAGE0_VALUE_WEI`, `exit_fits`) and the launch install's 12 / 24 deliveries a day, the `canister.rs` diff.
- Upgrade safety of every new field and case since `55d8d32`, the re-encoded round-2 fixture, `lost_opts`, the `.did`.
- E3-L-2 and E3-L-3.

Toolchain 1.98.1. `src/nft/EthNftWrapper.sol` was read for `noteSent` / `refund` as the canister uses them; the contract itself is
part A's.

**Nothing in the worktree was modified except this report and `review4/poc/E/`; nothing committed, pushed, deployed or signed.** Every
PoC ran in a private copy (a `git archive` export of `HEAD`'s `autobuyer/` in a scratch directory, checked identical to the tree with
`diff -rq` of `canister-rs/src` and `tests`). The old builds are `git archive` exports of `4949da0` (`55d8d32`'s canister tree is
identical) and `a37f54b`. PoCs (`review4/poc/E/`):

- `audit4_nft_e.tests.rs` + `audit4_nft_e.harness.diff`: the current `tests/audit_nft_r3.rs` fake world, up to its first test and
  copied whole, with one switch added, `home_pin_back` (the home pin one bucket lower: the 4663 balance before a spend of ours). The
  real H3 limiter (`real_outflow`, already in that harness) is used for E4-H-1. There are 5 tests: 3 **pass** (the weakness is shown)
  and 2 controls. The output is `audit4_nft_e.output.txt`.
- `audit4_nft_e_upgrade.rs` (2 tests, pass: what holds) and its output. It uses `fixtures/`: saved states written **by the old builds
  themselves** (`dump_state_old.rs`, and `dump_state_old.a37f54b.sed` for a37f54b's fields), and each build's `eth_nft` config.
- `didcheck.r4.diff` + `didcheck.output.txt`: review3's `poc/E/didcheck` with the fixer's two diffs (`fixes-round3-poc/didcheck.*`),
  plus a non-empty `SaleScan.pend` (the fixer's diagnostic had an empty one, so the `.did`'s `Move` record was never decoded).

Severity scale: the README's.

## Verdict

Counts (this part: new findings, plus round-3 items in scope not closed):

| | Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- | --- |
| New (E4-*) | 0 | **1** | 0 | 1 | 4 |
| Round-3 items in scope not closed | 0 | 0 | 0 | 1 (E3-L-2) | 0 |
| De-duplicated (counted in another part, not here) | | (E4-M-1 = C4-H-1) | | (E4-I-1's second half = A4-L-2) | |

- **D3-H-1, canister half: closed.** A delivery is recorded from our own final receipt, whoever holds the NFT now. `noteSent` names
  the mined version's hash, is sent once, cannot wedge the home line, and is the only new act allowed while paused. The one residual
  needs a reorg deeper than the pin (E4-I-1).
- **E3-M-1: closed for Ethereum**, as reported: every Ethereum balance read of the flow is floored. It is **not closed for the home
  chain.** The forward leg's refund is read from the 4663 balance at a pin with no floor, and one tick at a home pin that moved back
  reads our own Relay deposit as Relay's refund. That is part C's **C4-H-1** (the 4663 pin has no floor), which states this very
  path; it is counted there once. **E4-M-1** below is kept as an independent reproduction, adding what C4-H-1 does not show: what
  happens to Relay's real fill afterwards (stranded, unattributed, the advance booked as a loss).
- **New High, E4-H-1.** "Deposits' mints: one asked at a time" (`home_act` step 3) is not what the code does. Every deposit in `Mint`
  is asked under its own key, and nine deposits of one collection within a day trip the mint tripwire, which freezes custody and
  pauses the whole canister.
  - It dates from fix round 1 (`ed76e9d`, C-L-2) and was missed by rounds 1–3; the README and part C of round 1 state the opposite.
  - D5 (deposits on) and J7's caps (16 Miladys, 22 Azukis) make it an honest launch-week event.
- **J7 and deposits at the floor work as the operator decided.** The one gap is the exit rule: `exit_fits` is checked per collection
  only, and J7's 61 items at 24 a day cannot all leave within the timelock's delay (E4-L-1).
- **Upgrade safety holds.** States written by the `4949da0` and `a37f54b` builds restore into `HEAD` with nothing lost and no false
  refusal. Every new field is `opt` + `serde(default)`, the only new case (`Step::Sent`) is appended, and the `.did` matches.
- **E3-L-2 is unchanged (Low). E3-L-3 is closed.**

## Round-3 items in scope

| ID | Sev (R3) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **D3-H-1**, the canister half | High | **Closed** (residuals E4-I-1; A4-L-2) | See **D3-H-1** below the table. |
| **E3-M-1** | Med | **Closed for Ethereum; the home-chain sibling is C4-H-1 (reproduced here as E4-M-1)** | See **E3-M-1** below the table. |
| E3-L-1 | Low | Closed | `buy_holds_mint` and the 48 h use `arrived_at` / `back_at`, set once at the first arrival (`arrive` `:1481-1501`). A reverted fill (`mined_eth` `Buy`) and a cancelled one set `Arrived { since: now }` but move neither. |
| E3-L-2 | Low | **Not fixed** (still Low) | See **E3-L-2** below the table. |
| E3-L-3 | Low | Closed | `give_back_now` `:2992-3001`: an unwatched `Stuck` buy goes to `Returning { 0 }`, then `returnAdvance` of 0, then `Returned`, which is finished, so `stranded_by` no longer pins its collection. It spends nothing. |
| Upgrade safety (E3's "what holds") | — | **Holds** | See **Upgrade safety** below the table. |

**D3-H-1.** The canister half was checked on these paths:

- **At finality:** `delivery_pass` `:1540-1564` reads our delivery's receipt at the pin. A different block waits for that block. A
  revert goes to `MarkAfter`. A success must carry the collection's `Transfer(us → destination, token)`
  (`ethnft::delivered_in` `:386-390`: topics[1..2] zero-padded addresses, topics[3] the token, four topics, the log's address the
  collection).
- **Which hash:**
  - `Moved.tx` is the version that mined: `resolve_eth` retags an earlier re-priced version (`:2061-2073`).
  - The cancel race records the rival that won (`superseded` `:2226-2244`).
  - `noteSent` and `recordDelivery` both name that same hash.
- **No wrong ticket:**
  - A reverted delivery never gets `Moved`, so it is never noted.
  - `note_sent` reads the ticket first and sets `sent` without a call once the ticket has left `Resolved` (`:2370-2379`).
  - A `noteSent` that reverts because the ticket was recorded meanwhile is marked sent (`mined_home` `Step::Sent` `:2838-2847`).
  - One that reverts while still `Resolved` is retried. The estimate refuses a standing revert, so nothing is spent.
- **No wedge:** a failed `note_sent` pushes a wait, and the loop tries the next delivery (`home_act` step 1 `:2491-2524`).
- **While paused** (`home_act_paused` `:2423-2451`): records go first, then `noteSent`. Nothing else leaves (`send_home` `:2358`
  admits `Record` and `Sent` only). Each noted delivery is our own mined transfer and is noted once, so a pause cannot be used to
  spend 4663 gas without bound.
- **Order:** `noteSent` is sent from the first tick the transfer is seen mined at the pin, then `Record` at finality. A `Record`
  reached before `noteSent` simply records. `markUndeliverable` is still reachable only from `MarkAfter` (a cancel or a revert that
  mined) or from "never sent". A ticket noted sent is therefore marked only if a reorg deeper than the pin turns our mined transfer
  into a revert (E4-I-1).
- **Tests:** the reviewers' D PoC fails on its attack assertion (`fixes-round3-poc/d3_h1_*`). The fixer's `r3_d3_h1_*` pass in the
  bar.

**E3-M-1.** Every **Ethereum** balance read of the flow now calls `eth_floor(st, watch)` (`:1169-1180`) before reading:

- the `Funded` watch start (`:2631`);
- `Bridging` (`:1322`);
- a watched `Stuck` (`:1354`);
- `BackBridging`'s refund on Ethereum (`:1414`);
- the sweep (`:2918`);
- `eth_act`'s reads, which sit under its B-M-4 guard `:1856-1861` (`pin ≥ last_mined`). The way back's watch records its pin (`:2025`).

Can waiting on the floor wedge? No. `last_mined`'s block comes only from receipts read at a pin, and `Watch.pin` is a pin, so both
are behind every head that later grows. A provider swap (an upgrade) lags at most until the new pair catches up. The reviewers' two
PoCs fail on their attack assertions. The **home** chain's balance reads have no such floor: see **E4-M-1** (= C4-H-1).

**E3-L-2.** `arrive` (`:1481-1501`) still refuses an arrival that exceeds the balance beside other buys' earmarks. The fixer's
diagnostic (`fixes-round3-poc/audit3_nft_e.e3_l2-tolerant.*`) still shows the deadlock until the gas pot is topped up (720 refusals in
10 days, Azuki `Bridging` holding the Relay leg).

Nothing in the delta makes it worse. The sweep only takes the balance above the 0.05 ETH target, and only when no buy holds or awaits
ETH (`sweep_pass` `:2911-2915`). It stays Low: the monitor's gas-pot page names the remedy, and nothing is lost.

**Upgrade safety.** `audit4_nft_e_upgrade.rs`:

- **Old states, written by the old builds themselves** (`dump_state_old.rs`), with every buy phase including the three `Back*`,
  every delivery and deposit phase, every nested `opt` present, and a round-2 `SaleScan.todo` and an empty-sales cache entry:
  - The `4949da0` and `a37f54b` states restore with `lost_opts` = [], and the new fields read as absent.
  - a37f54b's `sweep`, `leftover` and `sweep_after` are kept.
  - `e04f49b` still passes.
- **Config:** both builds' `eth_nft` configs decode, with `gas_pot_target_wei` / `logs_max_blocks` absent or kept.
- **Probe cost:** at `HEAD`'s largest state (the sale cache 2,048 × 5; the new `judged` 8,192 × 5 and `pend` 512 × 5 `Move`s;
  3,000 buys; 6,000 used orders), 4 MiB of stable memory is probed in 438 ms native (release). That is far inside the upgrade
  instruction limit.
- **The `.did`** (`didcheck.output.txt`) decodes every case and every new field, `Move` included.
- **Semantics of old states:**
  - An old `Arrived` without `arrived_at` keeps `since`.
  - An old `BackFunded` older than 2 h returns to `Arrived` with a retry.
  - An old `Moved` is noted then recorded by receipt.
  - `todo` joins `pend`, one group per hash (`sale_scan` `:931-935`).
  - Old `Watch`es have no `pin`, so only `last_mined` floors them.

## Findings

### E4-H-1 (High): every deposit waiting to mint is a separate H3 ask, so nine deposits of one collection in a day pause the canister

- **Where:**
  - `autobuyer/canister-rs/src/ethflow.rs:2684-2717` (`home_act` step 3). The comment at `:2684` says "one asked at a time (so a
    backlog never inflates the tripwire's demand)". The loop `for i in candidates` calls `io.outflow_check(&key, &col, Flow::Mint, 1)?`
    at `:2702` under each deposit's own key (`mint:deposit:{tx}`), and on a `Wait` pushes the wait (`:2717`) and asks the next.
  - `outflow.rs:197-199`: every ask is recorded in `asked` before the verdict.
  - `outflow.rs:208-224`: the tripwire trips when a collection's asks reach `per_collection_mints × trip_factor`.
  - The canister's `outflow_check` then sets `PAUSED` (`canister.rs:1607-1613`).
- **History:** at `e90471c` the step asked only the first `Mint` deposit (`position(..)`). Fix round 1 (`ed76e9d`, C-L-2: one
  collection's wait must never hold up another's) turned it into a loop over every candidate and kept the comment. Round 1's part C
  (`C-canister-flow-deposits.md:517`) and the README (`:118`) state "one deposit mint is asked at a time".
- **The limits:** the launch install is `per_collection_mints` 3, `global_mints` 6, `trip_factor` 3 (`nft-pilot.json` `_custody`),
  and stage 0 is the same. So the tripwire trips at 9 mint asks of one collection, or 18 in all (purchases' `mint:eth:{id}` count too),
  within 24 h.
- **The trigger:**
  - Nine deposits of one collection reaching `Mint` within a day. All of them do: `deposit_pass` checks each against the custody book
    with `check_acquire`, which reserves nothing, so a batch that is final together passes together.
  - Three mint; the fourth to ninth each wait and each ask. Demand is 9, the tripwire freezes, and the canister is paused.
  - With J7's caps a collection admits 16 (Milady), 17 (MAYC) or 22 (Azuki) items, so this is ordinary demand in a launch week with
    D5 on. Before J7 the asks still happened (the ask comes before the admission), but the room was 3 items.
- **Who can trigger it:** any nine depositors, with no attacker needed. An attacker needs nine NFTs of the cheapest collection
  (about 6 ETH of Azukis) and loses nothing: the NFTs are custodied and the units mint after the resume.
- **Effect:** the same as C3-H-1's.
  - A paused canister posts no floor, so every module goes stale within its 1 h TTL, and mints, burns, graduation fills and
    deliveries stop for all holders.
  - Only the controller resumes it (after handover, the timelock and k guardians). The resume acknowledges the asks, but a new batch
    trips it again.
- **Who can stop it:** no one, before the pause. Deposits cannot be switched off without an upgrade (`eth_nft.deposits`).
- **PoC:** `audit4_nft_e.tests.rs` `e4_h1_nine_deposits_of_one_collection_in_a_day_trip_the_mint_tripwire_and_pause_the_canister`
  runs the launch limits (the real limiter) and J7's Milady cap. After 4 ticks (40 min): 3 minted, 9 mint asks,
  `Tripwire { collection: "1:0x5af0…", flow: Mint, demanded: 9, limit: 3 }`, paused. The control
  (`e4_control_eight_deposits_…`) shows eight deposits: 3 minted, the rest wait, no freeze.
- **Fix** (any one; the first is the smallest):
  - Once one deposit of a collection has waited on `outflow_check` in this pass, skip the collection's other candidates.
    C-L-2 still holds, because other collections are still tried.
  - Or give the limiter a check that does not record an ask, used for a backlog.
  - Or key a deposit's ask per collection and day, not per transaction.
  - Also correct the comment and README line 118. Add a regression with `real_outflow` and ten deposits of one collection.
  - Consider the same review for every loop that asks per item: deliveries in `eth_act` step 1 ask each waiting delivery too. There,
    custody bounds the demand (at most 24 items per collection, under 12 × 3, and 61 + 6 in all, under 24 × 3), so it does not trip at
    J7's numbers, but only by a margin of 5 globally.

### E4-M-1 (= C4-H-1, counted there): a home pin that moves back reads our own Relay deposit as Relay's refund (E3-M-1 on the home chain)

**De-duplication.** The root is part C's C4-H-1 (High): the 4663 pin (`evmread::fast_pin`, `pin::choose_pin`) has no floor, the 4663
half of round 1's B-M-4. C4-H-1 already names this Relay-leg phantom refund and adds the worse effect, a home transaction signed at a
used nonce that wedges `resolve_home` for ever. This finding is **not counted separately**. It is kept for its independent PoC and
for the consequence below that C4-H-1 does not trace: Relay's real fill is stranded at our Ethereum address, and the operator's only
exit books the advance as a loss. C4-H-1's fix (a floor on every home read at the block of our last mined home transaction) closes both.

- **Where:**
  - `autobuyer/canister-rs/src/ethflow.rs:1328-1340` (`step_buy`, `Bridging`): the refund is `home_balance()` against
    `w.balance_after`, via `relayconv::refunded` (`relayconv.rs:219-221`: a rise of half the deposit). It is read at the home pin
    with no floor. `eth_floor` (`:1322`) floors only the Ethereum read above it.
  - The home pin (`evmread.rs:61-79`, `fast_pin`) is one 1,024-block bucket below the lowest provider's rounded head. So, like the
    Ethereum pin, it moves back a whole bucket when one provider's head falls back across a boundary.
  - Nothing records the home block our deposit mined in: `HomeTx` has none, and `resolve_home` drops `evm_seen`'s block.
  - `fixes-round3.md` §5 says "every balance read of the flow"; the list there is Ethereum only.
- **The path:**
  - The deposit mined at home block H and was seen at a pin ≥ H: the buy is `Bridging`.
  - Relay has not filled yet, or its fill is not yet under the Ethereum pin (≥ 8 blocks and a tick behind). A slow fill is in the
    threat model.
  - One tick at a home pin below H shows the balance before the deposit, `balance_after + value`, and `refunded` holds.
  - The buy drops its watch (`last = None`) and goes `Funded { wei: value }` for a new quote.
- **What follows:**
  - No new quote can be paid: the real balance is under the value, and each one is consumed and refused. The buy stays `Funded`,
    which is on the Relay leg, so no collection starts a buy.
  - Relay's fill arrives at our Ethereum address with no watch, and is nobody's: not an earmark, not leftover.
  - The operator's only exit (`eth_give_back` on `Funded`, `give_back_now` `:3003-3010`) returns what 4663 holds beside the gas keep.
    The module books the rest of the advance as a loss (revenue, then surplus, then its ETH-backed units): up to a whole item, about
    7.35 ETH for BAYC.
  - The ETH stays stranded at our Ethereum address. The sweep moves only booked leftover, so only an upgrade can attribute it.
- **Also checked, no phantom:**
  - The other home reads: `Funded`'s `bal < d.value` (a lower pin only waits); `Returning`; `BackBridging`'s arrival (a lower home
    pin shows less, apart from our own 4663 gas, which the 1 % slack covers).
  - The home chain's own `resolve_home`: a lower pin shows `NotYet`, and the transaction is re-broadcast (harmless).
- **Who can trigger it:** no one directly. It needs a lagging or failing-over 4663 provider (the same premise as E3-M-1, on a chain
  whose bucket is 1,024 blocks) during a Relay fill slower than the home pin's lag. It is rarer than E3-M-1's trigger, but it has a
  larger per-event loss and it stalls every collection.
- **PoC:** `audit4_nft_e.tests.rs` `e4_m1_a_home_pin_that_moves_back_reads_our_own_relay_deposit_as_a_refund`:
  - One tick at the lower home pin gives "REFUNDED: 500000000000000000 wei back; attempt 2 of 3 next", `Funded`, no watch.
  - Relay's fill (0.498 ETH) then lands and is never read.
  - A new quote is refused ("our balance 2200000000000000 is under the deposit's 500000000000000000").
  - `relay_busy`, and Azuki's module wants an item but starts no buy.
  - `eth_give_back` returns 0.002 of the 0.5002 ETH advanced. Our Ethereum balance holds the fill with nothing earmarked or booked.
  - Control: `e4_control_without_the_home_pin_moving_back_the_fill_arrives`.
- **Fix:**
  - Keep the home chain's own "last mined" (the block `evm_seen` returns for our mined home transactions, or the home pin at which
    `Step::Deposit` was seen mined, saved in the `Watch`).
  - Read the home refund, and every home balance judgement of the flow, only at a home pin at or above it; lower, wait.
  - The robust fix, as for E3-I-4: read Relay's refund by its order id, not as a balance rise.

### E4-L-1 (Low): the install's exit rule is per collection only; J7's custody cannot all leave within the delay

- **Where:** `autobuyer/canister-rs/src/canister.rs:280-292` (`validate_custody`) checks `exit_fits(c.max_items,
  l.per_collection_items, 72 h)` for each collection, never the sum against `global_items`.
- **The numbers:**
  - Before J7 the sum was bounded by the global cap (6 items, against 6 a day).
  - J7's launch caps are 16 + 22 + 17 + 4 + 2 = 61 items at `global_items` 24 a day: 3 days, against the 2 that "the timelock's delay
    less a day" allows.
  - The default install (`custody_caps = null`: 6 items each, the sum 30) at stage 0's 6 a day in all needs 5 days.
  - `custodycap.rs:38-40` says "the default install always passes `exit_fits`". That is true per collection only.
- **Effect:** the H3 design promise, "a rate limit never traps holders behind an upgrade they can see coming" (`canister.rs:278-279`),
  fails for about 13 of 61 items when custody is full. In the benign case (a bank run) every delivery still leaves within its 7-day
  deadline, so the effect is only against a malicious upgrade, which the threat model trusts. Hence Low.
- **PoC:** `e4_l1_j7_custody_cannot_all_leave_within_the_delay_at_the_global_item_limit`: every collection passes `exit_fits` at 12 a
  day, and the whole does not at 24 a day. The default does not either.
- **Fix:** also require `exit_fits(Σ max_items, global_items, delay)` at install and upgrade. With J7's caps that needs
  `global_items` ≥ 31, which is over stage 0's ×4 (24). So either lower the collections' item caps (Azuki 22 → fewer), or accept and
  document the global gap. That is the operator's call, in the launch file's `_custody` note.

### E4-I-1 (Info): the receipt at finality judges `Moved`, not `MarkAfter`

- **`MarkAfter` is not re-read** (`ethflow.rs:1566-1572`). A cancel or a reverted transfer seen mined at the pin is marked
  undeliverable once its block number is under the finalized one, without reading its receipt again. If a reorg before finality made
  the delivery mine instead of our cancel, the ticket is marked, refunded after the grace, and the redeemer keeps the NFT. That is
  D3-H-1's mirror.
- **A noted ticket can be stuck** (= part A's **A4-L-2**, counted there). `Moved` after `noteSent` whose transfer is gone at
  finality (`:1545`: "not seen at the pin now: read again next tick", for ever) can be neither recorded nor refunded: `sentTx` is
  set, and no path leads to `Mark`. The canister side of it is the missing `Moved` → `Mark` path (the fix below).
- **Why Info:** the `MarkAfter` case needs a reorg deeper than the pin (≥ 8 blocks below every provider's head) before finality, which post-merge
  Ethereum does not have outside a finality incident.
- **Fix:** read the cancel's (or the revert's) receipt again at finality, as `Moved` now does, and go back to `Moved` if a delivery
  version is the one that is final. Give the operator a `Moved` → `Mark` path once a final pin shows no version of the delivery and
  the nonce is used by our cancel.

### E4-I-2 (Info): a deposit is admitted at 0 wei while its module has never posted

- **Where:** `ethflow.rs:2454-2470` (`deposit_floor_ok`) passes when `accepted` is 0 (`accepted > top` is false), and the admission uses
  `module_floor(..).0` (`:2703`), which is also 0. So A3-L-4's value bound is off for that collection.
- **Reachability:** normally unreachable. The first post is urgent (`home_act` step 0) and lands before any deposit mint. It can
  happen when the module's keeper is not the canister (`post_reading` refuses every post) or the post keeps reverting.
- **Fix:** refuse a deposit mint while `accepted == 0`.

### E4-I-3 (Info): from nothing, a policy's global cap may start at the sum of any number of 15 ETH collections

- **Where:** `custodycap.rs:191-195` (`first_global`) and `:146-176` (`validate_change`).
- **The change:** an install, or an upgrade after one that lowered every cap to 0 (lowering is always allowed), may name N Ethereum
  collections at 15 ETH and 24 items each, with a global of 15 N ETH. Before J7 the global from nothing was at most 10 ETH, so the
  "×4 per 72 public hours" bound (`:142-145`) was within 10 ETH of the running exposure after two proposals. It is now unbounded in N.
- **Why Info:** timelocked upgrades are trusted in this threat model.
- **Fix:** bound N, or the global from nothing (for example, the launch file's 75 ETH).

### E4-I-4 (Info): the re-encoded round-2 fixture holds none of round 3's new cases

- `tests/fixtures/eth-state-round2.hex` is compared byte for byte with `round2_state()`, and every round-3 field in it is `None`.
  There is no `Step::Sent`, no `Delivery.sent`, no `SaleScan.pend`, no `Watch.pin` and no `EthBuy.arrived_at`.
- So the services' IDL decode (monitor, ops) is not tested on a state this build writes with them present. It is covered only by the
  fixer's uncommitted `didcheck` diagnostic, and by this round's run with a non-empty `pend`.
- The monitor's IDL omits `pending`, so `Step::Sent` cannot break it today. A later IDL that adds `pending` without `Sent` would fail
  on a live state.
- **Fix:** a `round3_state()` fixture with every new case present, decoded by both services' tests.

### Note (not counted; part B's scope)

`sale_scan`'s budget is `(13 × 2,000) ÷ logs_max_blocks` chunks a pass (`ethflow.rs:950`): 52 `eth_getLogs` outcalls a pass per
collection at the launch's 500. `fixes-round3.md` says "the same 13 log chunks". While a window is being read that is four times the
outcalls, and cycles, of round 2.

## What holds (checked, no finding)

- **`noteSent` cannot name a wrong ticket or hash.** The call is our own (custodian), for the ticket whose transfer we signed, with
  the mined version's hash. `NOTE_SENT` is `0x6d65f720` = `noteSent(uint256,bytes32)` (checked with `cast sig`, and by
  `tests/wrapper_abi.rs`).
- **Custody book:** a deposit is admitted at the accepted floor at its mint. A purchase is reserved at its advance (the bound, not the
  item cap) and settled into the book at its mint. J7's caps, the global equal to the sum, the ×4 rule on upgrade and the Bitcoin
  wrapper's unchanged 3 items / 10 ETH are as `bayc10-decisions.md` §4 says.
- **H3's items at J7's numbers:** deliveries (`deliver:{wrapper}:{ticket}`, one key per ticket) cannot reach a tripwire. A collection's
  demand is bounded by its custody (≤ 24 < 36), and the global by 61 + 6 mints' refills (< 72). Returns never trip.
- **No wedge from the floors:** `eth_floor` can only delay a read. The watch pin and `last_mined` never exceed a pin once seen.
- **Old states restore** as described under "Upgrade safety" above.
