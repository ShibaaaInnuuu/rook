# Round 4, part C: H3's advance key (C3-H-1), the way back's exits (C3-M-1), the advance bound and the gas-pot sweep

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1eb5788`
(delta `55d8d32..1eb5788`: the operator's round `a37f54b` and fix round 3 `fb54018`). Scope: `ethflow.rs` `home_act`
(`NeedAdvance`: the H3 key `advance:eth:{collection}:{n}`), `advance_bound`, `advance_covers`, `buy_pass`, `step_buy` (`Bridging`,
`Stuck`, `Arrived`, `BackFunded`, `BackBridging`), `arrive`, `back_now`, `eth_act` step 3, `mined_eth` (`Buy`, `BackDeposit`),
`mined_home` (`Advance`, `GiveBack`), `sweep_pass`, `give_back_now`, `EthState.leftover`, the clocks `BACK_QUOTE_SECS`,
`BACK_RETRY_SECS`, `BACK_LATE_SECS`, `RETURN_COOLDOWN_SECS`, `ARRIVED_HOLD_SECS` and `BACK_ARRIVAL_SLACK_BPS`; `outflow.rs`
(`check`, `record`, the tripwire); `relayeth.rs` (the diff is two comments; the deadline band, rule 11); the 4663 side of the tick
where the way back and the forward leg read it (`ethnet.rs` `home_sign`, `canister.rs` `evm_balance` / `evm_nonce` / `evm_seen`,
`pin.rs` `choose_pin`). `src/floor/FloorModule.sol` (unchanged) was read for `advance`, `returnAdvance` and `fundReserve`.
Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review4/poc/C/`; nothing committed, pushed, deployed or signed.**
Every PoC ran in a private copy of `autobuyer/canister-rs` (scratch dir, made after the scratchpad clean-up and checked identical to
the worktree's `src/` and `tests/` with `diff -rq`). The harness is the current `tests/audit_nft_r3.rs` copied whole (its round-3
switches included: `real_outflow`, `pin_back`, `fee_of`), with one addition, `home_back` (`poc/C/r4_c.harness.diff`): the 4663 pin one
bucket lower, i.e. the balance and nonce there are those before our last home transaction, and a transaction signed at a nonce
already used is refused by the providers ("nonce too low"). The 17 round-3 tests of the copied harness still pass with it (22 of 22).

Severity scale: the README's.

## Verdict

**C3-H-1 is closed.** The advance's H3 key is stable while an advance waits, so an honest wait is one ask, and I found no path that
reuses a key for a second real release within the window (C4-I-1 is the one corner, and it nets to one count per release).

**C3-M-1 is closed as a stall.** Every `Back*` phase now has a bounded exit, and none of them spends ETH that is not the buy's. The
bound is long, though. It is the Relay order's own deadline, which Relay's measured quotes set 7 days out and the judge accepts up to
8, plus 25 h (C4-L-1). The exit also opens a Medium (C4-M-1).

The advance bound, the per-collection leftover and the sweep hold. A leftover is booked once, to its own collection, and debited and
re-credited symmetrically. A sweep never starts while any ETH at our Ethereum address is someone's. The ETH-moving paths that ask H3
nothing (the way back, `eth_give_back`, the sweep) send only to Relay's pinned depository for our own address, or to the buy's own
module. They move value H3 already counted when the module released it.

This part finds one High and one Medium:

- **C4-H-1 (High): the 4663 half of round 1's B-M-4 was never fixed.**
  - Every 4663 read of the flow runs at the tick's 4663 pin: the balance behind the forward refund, the way back's arrival and
    `Returning`, and above all the nonce `home_sign` signs at. That pin moves back one bucket (1,024 blocks, about 100 s) whenever one
    provider's head falls back across a bucket boundary; `choose_pin` accepts heads one bucket apart.
  - A home act signed at such a pin reuses the nonce of our last home transaction. The providers refuse it, and `resolve_home` then
    waits for ever ("nonce … is used but … not visible yet: waiting rather than guessing").
  - That wait is the canister's one 4663 line, shared through `evm_pending()` with the vault loop and the custodian. Nothing is
    posted, recorded, marked, minted or noted sent again, for any flow, until a timelocked upgrade. The modules go stale within 1 h,
    and every holder's mints and burns revert.
  - On the Relay leg, the same lower pin first reads our own Relay deposit as Relay's refund, which drops the watch and orphans the
    real arrival on Ethereum.
  - Round 1 named this ("The 4663 line has the same shape"); fix round 1 fixed only the Ethereum line (`last_mined` / `nonce_floor`);
    rounds 2 and 3 marked B-M-4 closed; E3-M-1's fix is Ethereum-only.
- **C4-M-1 (Medium): the fixer's flagged residual is worse than "a module can be credited another's ETH".**
  - A way back let go after `BACK_LATE_SECS` can never be credited to its module. Relay v2 refunds an unfilled order only after its
    deadline, so a slow refund lands exactly here.
  - The operator's only exit books the whole advance as a loss (`returnAdvance` of 0), even when the payout later sits at our own
    address.
  - If the payout lands while another collection's order is `Bridging`, it is read as that order's refund. That collection bridges
    twice, and its real arrival on Ethereum belongs to nobody.

## Counts (this part: new, plus round-3 items not actually fixed)

| Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- |
| 0 | 1 | 1 | 2 | 6 |

The Lows are C4-L-1 and the carried C3-L-1. C3-I-2 (no cancel for a `BackDeposit`) is carried unchanged and is not counted again
here.

## Round-3 items in scope

| ID | Sev (R3) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **C3-H-1**: a waiting advance re-asked under a new key each tick trips the freeze | High | **Closed** | See "C3-H-1" below. The key's `n` (`ethflow.rs:2588`) counts the collection's committed (past `NeedAdvance`), non-sweep buys, and buys are never removed from `st.buys` except a `NeedAdvance` one (`:2560-2595`) and a sweep that failed to start (`:2938`). So `n` cannot move while an ask waits. It moves back only when an advance reverts (`mined_home` `:2757`), and then the retry reuses the key it was recorded under, as intended. The fixer's tests use the real limiter; my copy re-ran them (pass). Worst honest demand: the day's released value (≤ 25 ETH, the limit on `done`) plus one waiting ask per collection, at most about 7.35 + 3.6 + 3 × 1 ≈ 14 ETH, against the 75 ETH trip. The Bitcoin side's Value flows share the global limit under their own keys (C4-I-5). |
| **C3-M-1**: the way back has no exit | Med | **Closed as a stall** (residuals **C4-M-1**, **C4-L-1**) | `BackFunded` holds new buys for 2 h (`buy_pass` `:1253-1255`) and then waits `Arrived` again, earmarked, with nothing signed (`step_buy` `:1386-1394`), retrying 24 h later. `BackBridging` past deadline + grace is watched 24 h more, then `Stuck`, unwatched and off the leg (`:1437-1449`). `eth_give_back` covers `BackFunded` → `Arrived` (`:2975`), late `BackBridging` → `Stuck` (`:2983-2991`), and an unwatched `Stuck` → `Returning { 0 }` (`:2992-3000`). A reverted `BackDeposit` re-asks for what is left (`mined_eth` `:2327-2331`). No path returns ETH twice. Every exit spends nothing of the home balance. A double `returnAdvance` reverts at the module (`NoAdvance`). A `Returning` that reverts is re-sent, but `returnAdvance` cannot revert while our advance is open, and a sweep's `fundReserve` always has a value above 0. |
| C3-L-1: the gate assumes 0.5 %, the judge allows 3 % | Low | **Partly fixed; stays Low** | `advance_bound` (`:1204-1215`) now grosses up for `relay_max_fee_bps`, but it is capped at `item_cap`. For the four conv-300 collections the item cap binds (`bayc10-decisions.md` §3), so `advance_covers` (`:1183-1194`, `ADVANCE_FEE_EST_BPS` 50) still decides alone. A quote filled at its minimum (2.3 % under in the measured quote) can arrive under the floor + 1 % + reserve. The 48 h way back and the 24 h `RETURN_COOLDOWN_SECS` now bound the cost to about two Relay fees per collection per day. Relay pays the expected amount in practice (0.36 %). |
| C3-L-2, C3-L-3, E3-L-1, E3-L-3, A3-L-1 (canister half) | Low | **Closed** (as reported) | `give_back_now` refuses an `Arrived` buy under 6 h from its first arrival (`:2963-2967`). The arrival slack is 1 % of the minimum, never more than the order sent (`:1405-1406`). `arrived_at` and `back_at` survive a reverted fill (`arrive` `:1492-1497`, `buy_holds_mint` `:604`). An unwatched `Stuck` is closed by the operator. The cooldown reads `ended_at` of `Returned` non-sweep buys (`buy_pass` `:1264-1269`); it is set by `mined_home` `GiveBack` (`:2791`), including a `Returned { 0 }` loss. The round-3 tests pass in my copy. |
| C3-I-2: no cancel for a `BackDeposit` | Info | **Unchanged** | It is repriced up to the chain's cap and never cancelled. While unmined it holds both the Ethereum line (deliveries wait, and those inside their last day are marked) and the Relay leg. |

## C3-H-1 in detail (what was tried)

- **Can `n` change while an ask waits?**
  - The waiting buy is dropped in `home_act` (step 7) and reopened by `buy_pass` (step 4) of the next tick, and it is the only open
    buy of its collection (`open_buy_for`). Nothing between the two changes the collection's committed count. Sweeps are excluded
    (`!x.is_sweep()`), and a sweep's open buy blocks the collection's buys anyway (`open_buy_for` sees it).
  - An upgrade keeps `st.buys`, and `stranded_by` refuses one that drops a collection with an open buy.
  - `eth_give_back` never touches a `NeedAdvance` buy.
  - Pruning: none exists. State grows by one `EthBuy` per real advance and per sweep, a few hundred bytes each (Info-sized; not
    counted).
- **Can two real releases share a key, so that one goes out uncounted (an H3 bypass)?** Only through a revert that is then dropped
  (C4-I-1). That needs an advance that reverts, a module that no longer wants the item before the retry, and a new buy of the same
  collection within 24 h. The later release then rides the reverted one's `done` entry: one count for one real release, at the
  reverted one's amount. No double spend, and no systematic bypass.
- **Can anything honest still trip the freeze?**
  - Per-collection trips do not apply to Value (`per = None`).
  - The global demand is `done` (≤ 25 ETH by the limit) plus the pending asks, which the key now bounds at one per collection.
  - An ask that H3 allows but custody refuses stays one ask (same key next tick).
  - An ask whose `send_home` fails keeps its buy at `NeedAdvance`, with the same key.
- **The paths that ask H3 nothing.**
  - `BackDeposit` goes to `re::FROM_ETHEREUM.depository` with our own address as recipient and refund (`relayeth::judge`).
  - `Returning` goes to the buy's own `b.module` (`returnAdvance` / `fundReserve`).
  - The sweep's value is bounded by `min(leftover_of(c), balance − target)`.
  - None of them reaches a third party other than Relay, the counterparty the forward leg already uses. The value is value the
    module's advance released, which H3 counted then. **Safe.**

## The advance bound, the leftover, the sweep (the operator's round)

- **`advance_bound`** (`:1204-1215`): `floor = min(own reading, accepted)` if the reading is fresh, else `accepted`.
  `buy = min(floor × (1 + premium), max_purchase_wei)`, then `gross = ⌈(buy + 0.003) / (1 − fee)⌉ + 0.0002`, and the result is
  `min(gross, item_cap)`.
  - It is re-checked in `home_act` with `advance_covers(amount, floor)` (`:2577`).
  - Custody is checked and reserved at the same amount (`buy_pass` `:1287-1292`, `home_act` `:2592-2593`), and H3 is asked for it.
  - The arithmetic matches `bayc10-decisions.md` §3 (7.353 ETH for BAYC at 6.79).
  - A fill later than the reading at a higher floor is capped by `wei.min(available)` (`eth_act` `:1955`). If it cannot fill, the
    buy goes back at 48 h. No bypass.
- **Leftover booking** is credited in three places, each once per event:
  - a mined fill: `arrived − total − fee` (`mined_eth` `:2296-2297`);
  - a mined `BackDeposit`: `wei − value − fee` (`:2335-2336`);
  - dust in `back_now` (`:1515`).

  Each is credited to `b.collection`. A sweep debits its collection when it starts (`:2934`) and re-credits it when it is abandoned
  (`:2894-2904`), refunded (`:1425-1431`) or fails to start (`:2937-2940`), and never becomes an arrival.

  I found no double credit. Abandonment matches `BackFunded` only; a `BackSending` sweep is on the line, not abandoned. Nor did I find
  a credit to another collection.
- **The sweep's start condition** (`:2906-2912`) refuses while any unfinished buy is a sweep, holds the Relay leg, has an earmark or
  is `Arrived`, while any delivery is unfinished, while a stray is being returned, while the line is busy, and while the line is
  halted. The amount is the booked leftover, never more than the balance over `gas_pot_target` (0.05 ETH, bounded [0.01, 1]).
  - Booked leftover can exceed the real leftover, because gas for deliveries, returns and cancels comes out of the same pot. The
    sweep then takes whatever is over the target, including ETH that is nobody's (C4-M-1(b)'s orphan), and credits it to the
    collection with the most leftover.
  - That is a misattribution into a module's surplus, never out of custody (folded into C4-M-1).
- `fundReserve` pays a treasury bootstrap loan first (`FloorModule.sol:1040-1045`, `_repay`). The canister comment says there is
  none in Phase 1. If one exists for a module, its swept leftover repays it instead of joining its surplus (C4-I-6).

## Findings

### C4-H-1 (High): a 4663 pin one bucket back signs a home transaction at a used nonce, and the canister's one 4663 line waits for ever; on the Relay leg it first reads our own deposit as refunded

**Where.**

- `ethnet.rs:217-227` (`home_sign`: `let nonce = self.home.evm_nonce().await?`, at the tick's pin).
- `canister.rs:1193-1202` (`evm_balance`, `evm_nonce`: at `self.pin()`, cached per tick, no floor across ticks).
- `pin.rs:54-61` (`choose_pin`: heads up to one bucket apart are accepted; the pin is one bucket under the lowest).
- `pin.rs:13` (`BUCKET` = 1,024 blocks on 4663, about 103 s).
- `ethflow.rs:2730-2747` (`resolve_home`: `NotYet` with the nonce used is `Err(… waiting rather than guessing)` for ever, `:2739`).
- `ethflow.rs:1329-1343` (`Bridging`: the forward refund is `refunded(home_balance, balance_after, value)` at that pin).
- `ethflow.rs:1404-1411` (`BackBridging`: the arrival on 4663 at that pin).
- `canister.rs:1360`, `:1408`, `:1420`, `:1431` (every other 4663 sender waits on `ethnet::evm_pending()`).

There is no persistent home nonce floor and no last-mined home block, the analogues of `EthState.last_mined` / `nonce_floor` on
Ethereum (B-M-4) and of `eth_floor` (E3-M-1).

**How.**

1. Tick t resolves our home transaction at nonce n as `Mined`: its block B is at or under the pin P, and above the previous tick's
   pin (a transaction is first seen mined in the bucket just under the pin).
2. At tick t+1 one 4663 provider answers a head in the previous bucket: a load-balanced endpoint whose back ends differ by seconds,
   near a bucket boundary. The heads are one bucket apart, which `choose_pin` accepts, so the pin is P − 1,024, under B.
3. At that pin:
   - `evm_nonce` returns n, so the next home act (a floor post, a record, `noteSent`, a mint, an advance, a deposit, a
     `returnAdvance`) is signed at nonce n. The providers refuse it ("nonce too low"). `send_home` saved it as `pending` before
     sending. From the next tick on, `resolve_home` sees `NotYet` and the chain's nonce past n, and returns
     `Err("… waiting rather than guessing")` on every tick for good. `home_act` never runs again (`st.pending` is `Some`). The vault
     loop, the custodian and the frog side wait on `ethnet::evm_pending()`, so the whole canister's 4663 side stops, including while
     paused (the paused pass also resolves the line first).
   - If the last home transaction was our Relay deposit (`Step::Deposit`), `home_balance` at the lower pin is the balance before it,
     which is `balance_after + value`. `refunded()` is then true, and `Bridging` reads "REFUNDED". It drops its watch (`x.last = None`)
     and asks for a new quote while Relay fills the first order on Ethereum. That arrival is now nobody's. The next deposit is signed
     at the stale nonce, as above.
   - `BackBridging`'s `balance_after`, if taken at a lower pin (higher: before a `Returning` or gas spend), can hide the real arrival
     and send the way back to `Stuck`, which is C4-M-1.

**Impact.**

- There is no exit short of a timelocked upgrade: no endpoint clears `EthState.pending`, and `eth_line_clear` is the Ethereum line's.
- Within 1 h every Floor Module is stale (no `postFloor`), so mints, burns and graduation fills revert for every holder of every
  collection.
- Recorded deliveries stop, and so do `noteSent`, the Bitcoin side's custodian and the vault loop.
- A delivery whose `noteSent` never lands is again refundable after the 30-day late window (D3-H-1's guard needs the note), so the
  wedge also re-opens D3-H-1 for every delivery in flight.

By the scale this stops the floor for everyone, with no attacker, on a lagging provider, which is in the threat model. It is also
quiet: the monitor sees stale modules and a stuck buy, not the cause.

**Likelihood.** It needs a one-bucket regression within the tick or two after a home transaction mines, with nothing pending at that
moment. That is rare per event. But home acts happen every few minutes over a launch lifetime (five collections' posts, records,
`noteSent`, mints), and load-balanced public 4663 endpoints are what the canister uses. The Ethereum twin was rated Medium in round 1
and fixed for exactly this reason. The 4663 consequence is canister-wide and permanent, hence High.

**PoC** (`poc/C/r4_c.rs`, both pass):

- `c4_h1_a_4663_pin_one_bucket_back_reads_the_deposit_as_refunded_and_wedges_the_home_line_for_good`: after Milady's deposit mines,
  one tick at a pin one bucket back reads "REFUNDED". The next quote is signed at nonce 1 while the chain is at 2. Three days later
  it is still the one home transaction out, 216 ticks have said "waiting rather than guessing", no 4663 call has been made, the buy
  holds the Relay leg (`Depositing`), and the first order's 0.498 ETH arrival on Ethereum is earmarked to nobody.
- `c4_h1b_any_home_act_signed_at_a_lower_pin_after_a_mined_one_wedges_the_line_a_floor_post`: no Relay at all. A floor post mines,
  the next post is signed at the used nonce, and no floor is posted again in 200 ticks (posts 1 → 1).

**Fix.**

- Keep `EthState.home_mined = (nonce, block)` (or one canister-wide, since the 4663 line is shared).
- Sign at `max(pin nonce, last signed + 1)`.
- Refuse to sign, and refuse the 4663 balance reads of the flow (the forward refund, `BackBridging`'s arrival and `balance_after`,
  `Returning`'s check, `Funded`'s deposit check), while the 4663 pin is under the last mined home block. This is B-M-4 and E3-M-1 for
  4663.
- Give `resolve_home` a way out for "nonce used, ours not visible". Once a later pin shows the nonce taken by one of our own recorded
  transactions, or after an operator look like `eth_line_clear`, re-sign the step at the next nonce instead of waiting for ever.
- The vault loop (`looper.rs:321-325`, `:342`) and the custodian (`redeem.rs:588`, `:610-611`, `:923`) sign the same way. They are
  outside this review, but they share the line, and the same fix belongs there.

### C4-M-1 (Medium): a way back let go after `BACK_LATE_SECS` can never be credited: a late Relay payout is nobody's, the operator can only book the advance as a loss, and a payout landing during another order's watch is read as that order's refund

**Where.**

- `ethflow.rs:1437-1449` (`BackBridging` past deadline + grace + `BACK_LATE_SECS` → `Stuck`, `last = None`).
- `:2983-2991` (the operator's early unwatch).
- `:2992-3000` (an unwatched `Stuck` → `Returning { wei: 0 }` → `returnAdvance` of 0).
- `:3021` (a `Returned` buy has no exit).
- `:1329-1343` (another order's `Bridging` reads any 4663 rise of half its value as its refund).
- `:1404-1411` (another way back reads a rise of 99 % of its minimum as its arrival).
- `:1316-1326`, `:1351-1366` (an Ethereum-side late refund read by a forward watch or a watched `Stuck`).
- `relayeth.rs:171-173`, `:207-210` (rule 11: deadlines 1 to 8 days out; Relay's measured quotes are quote + 7 days).

**What happens.**

- Relay v2 refunds an unfilled order after its deadline. The canister watches for 25 h past it, then lets the order go, and nothing
  ties a later payout to the order: arrivals and refunds are balance rises.
- The operator's instructions are to close it "once Relay says the order is dead". The only close is `returnAdvance(0)`, so the
  module books the whole basis as a loss, covered by revenue, then surplus, then the ETH-backed units.
- **(a) The payout lands while no watch is open.** It sits at our 4663 address (a 4663 refund of an Ethereum-origin order goes to
  the origin, Ethereum; an arrival goes to 4663). Either way no endpoint can credit it to the module that lost it (`Returned` has no
  exit). Only a timelocked upgrade can.
- **(b) The payout lands on 4663 while another collection's forward order is `Bridging`.** It is that order's "refund": the order
  is re-quoted and bridges the other module's ETH, while its real fill on Ethereum is unwatched. The same happens for an Ethereum-side
  refund landing in a forward watch, read as an early arrival, and for another way back's watch on 4663.
- **What holds:** nothing is spent twice, earmarks never exceed the balance (`arrive`'s cap), and no module is ever returned more
  than its own advance (`min(value)` / `min(advanced)`). The harm is the accounting: a real loss booked to holders of the lost
  module's ETH-backed units (BAYC: up to 7.353 ETH) while the ETH sits in custody unattributed. It is invisible to the monitor. The
  canary checks earmarks ≤ balance; a surplus pages nothing; the `Stuck` page is the only signal.

**Why Medium.**

- It is a bounded loss (one advance), conditional on Relay not filling and not refunding within deadline + 25 h. That is the
  threat model's "Relay failing to fill or refund", and exactly C3-M-1's trigger.
- Given that trigger, the likely outcome of Relay's refund process is a late payout, so the residual is not a corner.
- The operator's choice is between pinning the collection for good (an unwatched `Stuck` holds its buys) and booking a loss for
  ETH we hold.
- The forward leg's operator exit has the same shape. A watched forward `Stuck` returns `home − keep` (`:3012-3019`) and a late
  Ethereum arrival is then orphaned. That part is pre-existing.
- **What that forward exit returns (checked at part D's request).** `give_back_now` on a watched `Stuck` returns
  `min(home balance − GAS_KEEP, b.advanced)`. That is the whole 4663 balance less one keep, not a measured refund of this order.
  - **Another NFT buy's ETH: never.** A watched `Stuck` holds the Relay leg (`in_relay`: `last.is_some()`), so no other buy can be
    `Funded`, `Depositing`, `Bridging`, `Returning`, `BackSending` or `BackBridging` at the same time. `buy_pass` opens no buy
    (`:1253`), `eth_act` step 3 signs no way back (`:1984`), and `sweep_pass` starts no sweep (`:2909`).
  - **Anything else at the shared 4663 address: yes**, up to the advance:
    - C4-M-1's late, unattributed way-back payout of another module (misattributed as this module's return);
    - the frog side's ETH when both sides run (B-L-6, carried Low);
    - a stranger's donation;
    - the accumulated `GAS_KEEP`s that pay every later 4663 transaction.
  - **Consequences:**
    - If this order's own Relay refund has not landed yet, the module is paid from that other ETH. The refund then arrives later,
      unwatched and nobody's.
    - With nothing else there, the module gets `home − keep` and books the rest as a loss. Returning the gas keeps also drains the
      home gas pot.
    - Nothing is spent twice, and no module gets more than its own advance.
  - **Rating:** folded into C4-M-1 (the same root: a balance delta or balance, not the order, decides whose ETH it is). The frog-side
    share stays B-L-6. It is an operator-invoked path.
  - **Fix:** C4-M-1's attributed exit. For a watched forward `Stuck`, return only a refund measured against the watch
    (`refunded(home, balance_after, value)` at a floored pin, as `Bridging` does), never the raw balance.

**PoC** (`poc/C/r4_c.rs`, both pass):

- `c4_m1_a_late_way_back_payout_is_nobodys_after_the_operator_closes_it_as_a_loss`: the deadline is 604,800 s after the quote. After
  the close (`returnAdvance` of 0), Relay's 0.4855 ETH lands at our 4663 address. The module got 0 back, and `eth_give_back` is
  refused ("buy 0 is Returned { wei: 0 }").
- `c4_m1b_…_read_as_another_collections_refund_and_orphans_its_real_arrival`: Azuki's forward order reads Milady's late payout as
  "REFUNDED" and bridges twice (2 Relay deposits). Milady returns 0. 0.512 ETH over the gas pot on Ethereum is nobody's: the booked
  leftover is 0.003.

**Fix.**

- Keep the order's watch data on the unwatched `Stuck`: order id, value, `min_out` and the balances. Don't clear it.
- Give the operator an attributed exit: `eth_give_back(buy, "arrived")`, accepted only when a Relay fill or refund naming the order
  id is read on chain, or at least when the 4663 or Ethereum balance above everything that is someone's covers the order's minimum.
  Use it to credit the late payout to the buy (`Returning { wei }` or `Arrived`).
- Robust (E3-I-4, the reviewers' earlier advice): read Relay's fill by its order id, not a balance delta, for every leg.
- Add a monitor page for balance at either address that no record accounts for.

### C4-L-1 (Low): the way back's bound is its order's own deadline (7 days in Relay's quotes); every collection's conversions wait that long

`buy_pass` opens no buy while any buy holds the Relay leg (`:1253`), and `BackBridging` holds it until deadline + 1 h + 24 h
(`:1437-1449`). `eth_give_back` refuses to unwatch it before deadline + 1 h (`:2984-2986`), whatever Relay's status says. Relay's
measured quotes have the deadline 7 days out, and the judge accepts 1 to 8 days (rule 11).

So one way back that Relay neither fills nor refunds holds every collection's new buys for about 7 to 8 days. Burns and redemptions
are unaffected, and the pools wait ETH-backed on 4663. The forward leg has the same deadline-bound hold (pre-existing).

**PoC:** `c4_l1_…`: `eth_give_back` refused for 169 h, and Azuki waited all that time.

**Fix:** let the operator unwatch before the deadline once the runner's Relay status (by order id) says failed or refunded,
together with C4-M-1's attribution. Or ask Relay for a shorter deadline, keeping rule 11's floor at 1 day.

### C3-L-1 (Low, carried): see the table above

### Infos

- **C4-I-1:** a reverted advance whose buy is then dropped leaves its key `advance:eth:{c}:{n}` in `done`. The next buy of that
  collection within 24 h has the same `n` and goes out under that entry, uncounted. The count still nets to one per real release, at
  the reverted amount. It needs `advance` to revert (the item cap fell between read and mine), then a drop, then a new buy.
  Hardening: record under the key only once the advance mines, or key by `n` plus the attempt.
- **C4-I-2:** the way back's `Returning { wei }` is the full 4663 rise (`:1406`, `min(value)`), with no `GAS_KEEP`. The
  `returnAdvance` then needs the existing home gas beside it. `bal < wei` is checked but gas is not (`:2549`). An empty home gas pot
  makes the estimate fail, a wait until it is topped up. The forward `Returning` keeps `GAS_KEEP` (`:1331`, `:1340`).
- **C4-I-3:** a reverted `BackDeposit` returns to `BackFunded { since: now }` (`mined_eth` `:2328`), which restarts the 2 h window
  during which no new buy starts. Repeated on-chain reverts after a passing simulation would hold new buys 2 h per attempt, bounded
  by the buy's ETH (gas each time).
- **C4-I-4:** `monitor/src/ethnft.mjs:60` says "`eth_give_back` stores `BackFunded { since: 0 }`". Since round 3, `give_back_now`
  stores `since: now` (`:2971`), and only a sweep is created at 0, and `back_now` sets it at once. Harmless (the page handles 0); the
  comment is stale.
- **C4-I-5:** the fixer's worst-case demand (about 44 ETH against 75) counts the NFT side only. The Bitcoin side's Value flows
  (`convflow.rs:212`, `frogflow.rs:1038`, `handinflow.rs:109`, `nftnet.rs:315`) share `global_value_wei` and the tripwire under their
  own keys. They were not re-audited here, and their key discipline decides the real headroom.
- **C4-I-6:** a sweep's `fundReserve` repays any treasury bootstrap loan of that module first (`FloorModule._repay`), so "its surplus"
  holds only while no loan is outstanding. The canister comment says none in Phase 1.

## What holds (re-attacked, no finding)

- A fill and the way back never spend the same ETH: the earmark is carried through `BackFunded` and `BackSending`; `eth_act` step 3
  checks `wei.min(balance − others' earmarks)`; `arrive` caps the earmark at the balance.
- No double `returnAdvance`: one per buy, the module refuses a second (`NoAdvance`), and `GiveBack` is `Returning`-gated.
- `BackFunded` → `Arrived` after 2 h drops its quote from the inbox and signs nothing. It does not re-enter the Relay leg until it
  is re-armed 24 h later.
- The operator's `eth_give_back` refuses a sweep except when it is unwatched and `Stuck`. It refuses `BackSending`, `Returning` and
  `Returned`.
- The E3-M-1 floor (`eth_floor`) is applied to every Ethereum balance read of the way back and the sweep. Its 4663 analogue is
  missing (C4-H-1).
- H3 is untouched for items, mints and returns. The tripwire's `acked` logic is unchanged.

## PoCs

`poc/C/r4_c.rs` holds the tests. `poc/C/r4_c.harness.diff` is the `home_back` switch against `tests/audit_nft_r3.rs`.
`poc/C/r4_c.output.txt` is the run, with the per-tick `  | ` lines removed. Run: copy `tests/audit_nft_r3.rs` to `tests/r4_c.rs`,
apply the diff, append `r4_c.rs`, then `cargo test --test r4_c -- --nocapture`.

| Test | Finding | Result |
| --- | --- | --- |
| `c4_h1_a_4663_pin_one_bucket_back_reads_the_deposit_as_refunded_and_wedges_the_home_line_for_good` | C4-H-1 | **Passes** (weakness present) |
| `c4_h1b_any_home_act_signed_at_a_lower_pin_after_a_mined_one_wedges_the_line_a_floor_post` | C4-H-1 | **Passes** |
| `c4_m1_a_late_way_back_payout_is_nobodys_after_the_operator_closes_it_as_a_loss` | C4-M-1 | **Passes** |
| `c4_m1b_a_late_way_back_payout_is_read_as_another_collections_refund_and_orphans_its_real_arrival` | C4-M-1 | **Passes** |
| `c4_l1_a_way_back_relay_does_not_fill_holds_every_collections_buys_for_the_orders_whole_deadline` | C4-L-1 | **Passes** |
| the 17 round-3 tests of the copied harness (`r3_*`) | control | **Pass** (the `home_back` switch is off by default) |
