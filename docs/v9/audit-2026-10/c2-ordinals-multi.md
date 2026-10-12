# C2: the Ordinals multi-collection change

| | |
|---|---|
| Scope | `autobuyer/canister-rs`: `src/frogsol/multi.rs` (new); the change's parts of `src/frognet.rs` (`tick_collections`, `ItemIo::for_coll`, `ensure_migrated`, `live_cap_keys`, `with_coll` / `with_frog` / `view`, `frog_turn` / `keep_frog_turn`, the endpoints that gained `coll`, `frog_collections`, `frog_wake`, `frog_sleep`), `src/custodynet.rs` (`CustodianIo::for_coll`, `ticket_key`, `reserved_elsewhere`, `tick_collections`, `probe_frog`), `src/frogsol.rs` (`owns`, the `w`-taking ticket reads, the minimum purchase), `src/frogsol/books.rs` (`coll_for`, `books:<id>` outcomes), `src/redeem.rs` (`Custody.colls`, the outflow key), `src/floortrigger.rs` / `src/floornet.rs` (`min_wei`, `set_purchase_limit`'s minimum), `src/convflow.rs` (`used_elsewhere`, `check_limits_min`), `src/frogflow.rs` (`FrogState.colls` / `dorm`, `FStep::frog_id`, `cost_checked`), `src/canister.rs` (`caps_of`, the `post_upgrade` migration); and what they call into where the change made a single-collection assumption multi-collection (`frogflow::sweep_stale`, `solhome::SolLine::outcome`, `convflow::holds_relay`) |
| Commit | `79b4e60` on `golive-2026-10-08` (the change landed in `86ded3e`, merged in `5a5812d`) |
| Reference | `docs/v9/ordinals-collections.md` §4 ("Built (2026-10-12)" and its seven parts), §7; `docs/v9/HANDOFF-2026-10-10.md` §8 ("Ordinals multi-collection"); B2 (`b2-primitives.md`) for the line (B2-10) |
| Method | The project's: threat model, then a line-by-line read of the change (`git show 86ded3e`) and of every function it now calls once per collection, against the checklist below; a PoC test for every High (and one Medium), PASSING by exhibiting the bug (`tests/audit_c2_ordinals.rs`, on `tests/frogs_sol.rs`'s books-home world and `books_two`). No `src` file was changed. |
| Result | 0 Critical, 1 High, 3 Medium, 4 Low, 6 Informational; a section on the operator's default-live decision (§4) |
| Fix round | 2026-10-12: C2-01 to C2-08 fixed, C2-10, C2-12, C2-13 fixed, C2-14 fixed in part, C2-09 and C2-11 recorded; the default-live switch made with §4's must-hold list (each finding's **Fixed** / **Recorded** line; the PoCs inverted to `fixed_` regressions) |

## 1. Threat model

**Assets**

1. **The inscriptions in custody**: one taproot custody address for every Inscriptions listing; each item is one collection's (its books ledger, its tickets).
2. **The float and custody's fee UTXOs**: the float pays every collection's purchases; custody's plain outputs pay every collection's deliveries.
3. **Each collection's books**: its module (floor, backing, advances), its ledger (items, tickets), its custody cap, its purchase limit and minimum.
4. **The canister-wide limits**: the custody caps (H2) and the global item cap the exit rule sized (A4-L-1: what custody holds can leave within the timelock's delay), the outbound limiter (H3), the one Relay leg (B-L-6, rule S12), the one Solana money line.
5. **Cycles** (every collection's world reads Solana and Bitcoin).

**Actors**

| Actor | Capability | In scope |
|---|---|---|
| A depositor | Sends any output to the custody address: a non-member, another collection's member, several inscriptions, runes, dust | yes |
| The runner (operator principal) | `frog_floor_reading`, `frog_proof`, `frog_offer(_protected)`, `relay_quote`, `relay_give_back`, `probe_frog`, `frog_wake` / `frog_sleep`, for any `coll`, any frog id, any proof | yes (a hostile or buggy hand-in) |
| A seller on Ordinals Wallet / UniSat | Lists, reprices, spends the listed output between the offer and the landing | yes |
| Relay | Fills, refunds, partially refunds, late-refunds an order | yes |
| One malicious provider (Solana, Bitcoin, ord) | Any answer; disagreement | yes (agreement of two is the trust root) |
| The controller | `set_floor_sink`, `set_purchase_limit`, `set_ow_creators`, the install's caps | trusted; foot-guns checked |

**Checklist** (the brief's): an inscription credited to the wrong collection or twice (overlapping roots, a proof handed in for one used by another); one collection's floor, purchase limit, cap or Relay order (S12) affecting another's books; the shared Relay leg and frog turn on the one line (starvation, an outcome taken by the wrong owner); dormancy (a dormant collection buying, posting, blocking exits; `frog_sleep` with things open); the exit rule and the caps' re-sizing; the upgrade migration (the single state into the right entry, nothing lost or duplicated, a delivery in flight counted twice); the outflow key `ticket:<coll>:<n>`; the minimum purchase's bounds; deposits of a dormant collection's members; the custodian for dormant listings.

**What held** (read and found correct): frog ids stay unique (`next_frog` at the top, `take` / `put` take the larger); a book outcome names its collection (`books:<id>`) and `owns` gives every SOL-moving step (`Advance`, `Convert`, `GiveBack`, `LateCredit`, each naming a frog) to the collection holding the frog, so no landing is taken by another collection; `frog_offer(_protected)` / `relay_give_back` find the frog in the named collection only; a buy's proof is judged against `coll(io)`'s root and its mint goes to that collection's ledger (`books::send`), so a member of B bought through A's offer is refused; overlapping roots route as ambiguous and are refused; S12 reads every other collection's orders (`used_elsewhere`) and B-L-6 every collection's conversions (`held_elsewhere`, `ethnet`'s `relay_busy`); the custodian runs every listing, dormant ones included, and `frog_sleep` does not stop it; a sleeping collection's cap leaving the policy never blocks its exits (`custodycap::release` reads no policy); each collection's ticket reads, item reads and records go to its own books (`coll_for(Some(w))`); fee inputs are reserved across collections (`reserved_elsewhere`, `custodynet::reserved`); the minimum purchase is bounded [0.15 SOL, 1 SOL] and at most the maximum, applied at the advance and at `cost_checked`; `post_upgrade` migrates after the desk is restored, into the lowest-id Inscriptions listing (`BTreeMap`), the one the single-collection `coll()` served.

## 2. Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| C2-01 | One collection's pass releases every other collection's in-flight custody-cap reservation (`sweep_stale`); the arrival is then never booked, so the caps and the exit rule stop bounding custody | **High** | `poc_c2_01_one_collections_pass_releases_anothers_purchase_in_flight_and_its_arrival_is_never_booked` |
| C2-02 | `frog_sleep` accepts a collection whose written-off Relay order is still watched: the dormant collection is never ticked and holds the one Relay leg for every collection and the Ethereum / Robinhood sides | Medium | `poc_c2_02_a_collection_sleeps_while_its_written_off_order_holds_the_relay_leg_for_every_other` |
| C2-03 | Each collection pushes its book outcomes onto the money line raw, past `OUTCOMES_KEPT`; with many live collections a landed advance's outcome can be dropped before its collection reads it, and is then taken back as failed (B2-10, made reachable) | Medium | none |
| C2-04 | Every collection's world re-reads the Solana clock (and rates) on its own: with N listings a tick makes about N times the outcalls of before | Medium | none |
| C2-05 | One custody output whose contents cannot be read stops custody's whole classification every tick (no set-aside, unlike `deposits_pass`'s H-3) | Low | none |
| C2-06 | Classification is decided once, against the live roots and the proofs already handed in: a dormant collection's member, or a member whose proof comes later, is refused for good | Low | none |
| C2-07 | The one Relay leg and the frog turn go to the lowest collection id that wants them, and the Ethereum side yields to any collection's `NeedAdvance`: with many collections, high ids and the Ethereum / Robinhood side can wait indefinitely | Low | none |
| C2-08 | The caps follow the live set at run time: `frog_wake` adds a stage-0 cap and +10 ETH of global value without the controller's ×4 path, and `post_upgrade` compares caps computed before the frog state and the books are restored | Low | none |
| C2-09 | The outflow key changed from `ticket:<n>` to `ticket:<coll>:<n>`: an ask already counted, or acknowledged by a resume, under the old key counts again | Informational | none |
| C2-10 | The migration wakes the lowest-id listing when the single state holds only configured floor sinks, and copies every configured sink into it | Informational | none |
| C2-11 | `frog_sleep` is no brake for work in flight (refused while anything is open); the only stop for one collection is the global pause | Informational | none |
| C2-12 | Default-live (§4) removes the human wake check: a collection's first floor is accepted from nothing (`post_floor_raw`, `prev == 0`), gated only by the runner's CoinGecko band | Informational | none |
| C2-13 | A partial Relay refund re-funds the conversion with what came back (`Funded { wei: back }`), which may be under the 0.15 SOL deposit floor: it then leaves only by its give-back | Informational | none |
| C2-14 | An overlapping root is refused as ambiguous (safe), but nothing in the canister refuses a second listing whose root overlaps an existing one; the books' `held_markers` are per collection | Informational | none |

---

### C2-01: one collection's pass releases every other collection's in-flight custody-cap reservation (High)

**Files:**
- `src/frogflow.rs:680` (step 4a of `tick_all`) and `:1320-1331` (`sweep_stale`: every in-flight `frog:<id>` entry of the ONE book whose frog is not in `st` is settled `None`)
- `src/frognet.rs:565` (`tick_collections`: `tick_all` once per live collection, each with its own entry as `st`)
- `src/frogflow.rs:1486` (`count_and_reserve`, at signing) and `src/convflow.rs:396-399` (the reservation at the advance), both `frog:<id>`
- `src/frogflow.rs:970` (`settle(frog:<id>, Some(inscription))` at the arrival; its error is only a note)

**Description.** `sweep_stale` was written for one state that held every frog: an in-flight reservation `frog:<id>` whose frog is missing from `st` was a purchase lost to an upgrade (audit D, L-2). Since the change, `st` is one collection's entry and the custody book is still one book; frog ids are unique canister-wide, so for collection A every reservation of collection B is "missing" and is released, at every pass of A. B's purchase (or B's Relay conversion, reserved at the advance) then has no room reserved; when B's inscription arrives, `settle` finds nothing in flight and returns an error the flow only notes, so the item is held in custody and never booked. From then on the book undercounts custody by one item per purchase of every collection but the last to tick: `check_acquire` admits purchases over each collection's cap and over the global item cap that A4-L-1 sized so that everything custody holds can leave within the timelock's delay. No actor is needed: two live collections and one purchase are enough, every time.

**Scenario.** NodeMonkes (listing 12, cap 1 item) signs a float purchase of frog 0: `frog:0` is in flight. Bitcoin Frogs (listing 7) runs its pass the same tick: "custody book: frog:0 was reserved for a purchase that is gone: released". Frog 0 confirms: "held (custody book: frog:0 is not in flight)"; the NodeMonke is minted into listing 12's books and is in custody, the book holds nothing of NodeMonkes, and `check_acquire` admits a second NodeMonke.

**PoC.** `tests/audit_c2_ordinals.rs::poc_c2_01_one_collections_pass_releases_anothers_purchase_in_flight_and_its_arrival_is_never_booked` (the books-home world with `books_two`; the real NodeMonke and proof): the release line, `frog:0` gone from the book, the "not in flight" note at the arrival, 0 NodeMonkes in the book while one is held, and `check_acquire` passing at a cap of one.

**Fix.** `sweep_stale` releases only the reservations of the collection whose state it holds: the reservation's collection key (`Entry.collection`) must be `collection_key(io)`, or the frog must be missing from every collection's entry (`multi::states`), not from `st`. The PoC's release line, the missing entry and the passing `check_acquire` must all fail. Also make a failed `settle` at the arrival loud (an item custody holds that the book does not count is an H2 breach, not a note).

**Fixed (2026-10-12):** `frogflow::sweep_stale` releases only the in-flight reservations whose collection key is the pass's own
(`collection_key(io)`), so one collection's pass never touches another's purchase or advance in flight; at the arrival
`custodycap::hold_arrived` turns the reservation into the held item, and with none in flight books the item held all the same (uncapped:
it is in custody already) with a loud `CUSTODY BOOK, audit C2-01` line, so the book never undercounts custody. Regression:
`tests/audit_c2_ordinals.rs::fixed_c2_01_one_collections_pass_releases_anothers_purchase_in_flight_and_its_arrival_is_never_booked` (the
PoC inverted: no release line, `frog:0` still in flight, the NodeMonke held under its collection, `check_acquire` refusing a second) and
`fixed_c2_01_an_arrival_with_no_reservation_in_flight_is_booked_all_the_same`.

---

### C2-02: `frog_sleep` with a written-off order still watched (Medium)

**Files:** `src/frogsol/multi.rs:112-123` (`open_work`: `!c.finished()`), `:148` (`sleep`); `src/relayconv.rs:354-364` (`finished()` is true for `Returned` whatever `late`; `late_open()`); `src/convflow.rs:288-292` (`holds_relay` counts `late_open()`); `src/frogsol/multi.rs:102` (`held_elsewhere`); `src/frognet.rs:565` (a dormant listing is not ticked).

**Description.** A conversion that gave back while `Stuck` is `Returned` (finished) and keeps its written-off order watched (`late`, conversion review C-03) until the refund is credited, the fill seen, or the operator's `relay_late_release`. That watch holds the one Relay leg. `open_work` reads `finished()` only, so `frog_sleep` accepts the collection; once dormant it is never ticked, so the watch is never read: a late refund stays at our address uncredited to the module, a late fill is never recorded, and every other collection's advance and the Ethereum / Robinhood sides' Relay legs wait on it (B-L-6) for as long as it sleeps. Recovery exists (`frog_wake`, or `relay_late_release`, which finds the frog by id), so this is availability and an operator trap, not a loss.

**PoC.** `poc_c2_02_a_collection_sleeps_while_its_written_off_order_holds_the_relay_leg_for_every_other` (pure, `frogsol::multi`): the sleep is accepted with `holds_relay`; after it the listing is dormant and `held_elsewhere(…, 12)` is still true.

**Fix.** `open_work` also reports a conversion with `late_open()` (and, more generally, anything `convflow::holds_relay` or `watching` sees) and a book outcome of the collection still on the line (see C2-03); or the tick keeps stepping a dormant collection's conversions (as the custodian keeps its redemptions).

**Fixed (2026-10-12):** `multi::open_work` reports a conversion whose written-off order is still watched (`late_open`) and anything
`convflow::holds_relay` or `watching` sees, so `frog_sleep` is refused while the collection holds the leg; `frog_sleep` also refuses a
collection with an outcome on the Solana line it has not read (`frogsol::owns` over the line's outcomes). Regression:
`fixed_c2_02_a_collection_sleeps_while_its_written_off_order_holds_the_relay_leg_for_every_other` (the PoC inverted: the sleep refused,
the collection still live) and `frogsol::multi::tests::c2_02_no_sleep_while_a_conversion_holds_the_relay_leg` (released by
`relay_late_release`, it sleeps).

---

### C2-03: book outcomes pushed raw past `OUTCOMES_KEPT` (Medium)

**Files:** `src/frogsol/books.rs:191` (`l.outcomes.push(…)`, not `SolLine::outcome`); `src/solhome.rs:351-357` (`outcome`: keeps the last 16, dropping the oldest, whatever owner); `src/frogsol.rs` `resolve` ("is no longer on the line and left no outcome: taken back", then `books::landed(…, false)` and `mined_any(…, false)`); `src/frogsol/books.rs:200-204` (`Advance` not ok → `ReturnAdvance` booked).

**Description.** Each collection's book step (`Post`, `Settle`, `Arrived`, `Mint`) leaves one outcome `books:<id>` on the money line, read at that collection's next pass; with N live collections up to N such outcomes are outstanding at once (the runner hands in a floor reading for every live collection in one cycle), plus the Ethereum side's raw `books` outcomes, plus any a slept collection left (never read). Raw pushes do not trim; the next landing's `outcome()` trims to 16 from the front. A SOL step that landed (an advance) whose collection reads it only next tick (it was resolved by a later pass of this tick) can be the oldest and is dropped. Its collection then finds its `sol_step` neither pending nor answered and takes it back as not done: the books record the advance returned to the module (`ReturnAdvance`) while the lamports are at our address, and the conversion goes back to wanting an advance. With one collection this needed 16 other outcomes in one window; with 15 live collections it needs one busy runner cycle. This is B2-10 made reachable by the change.

**Fix.** Never drop a SOL step's outcome for a book one: book outcomes go through `outcome()` and the trim drops book outcomes (final at once, re-readable from the books) before any landing; or book steps report to their flow without the line. A slept collection's outcomes are dropped at the sleep.

**Fixed (2026-10-12):** the line's outcomes are capped and evicted by kind (`solhome::is_book_outcome`): a signed step's outcome
(`outcome()`) counts only against the other signed steps' (`OUTCOMES_KEPT`, 16), and a book update's (`SolLine::book_outcome`, used by
`frogsol::books::send`) only against book outcomes (`BOOK_OUTCOMES_KEPT`, 64), oldest first; the Ethereum side's raw `books` outcomes
count as book outcomes too. A landed advance is never trimmed to make room for book outcomes. A collection with an unread outcome
cannot sleep (C2-02), so none is left behind by a sleep. Regression: `solhome::tests::c2_03_a_landed_advance_is_never_trimmed_for_book_outcomes`
(31 book outcomes and 15 signed steps after a landed advance: the advance is still read; each kind trimmed only by its own).

---

### C2-04: every collection's world re-reads the clock and rates (Medium)

**Files:** `src/frognet.rs:565` (`ItemIo::for_coll` per live collection, plus one for the classification), `src/custodynet.rs:529` (`CustodianIo::for_coll` per listing, live or dormant); each builds a fresh `SolNet::of_install()` (`src/solhome.rs:1284`), whose `clock()` / `rate()` / `btc_rate()` caches live in that instance (`:1287-1352`); `src/frogsol/books.rs:254` (`ticket_count` reads `sol_clock` for every listing every pass) and `:105` (`read_sink` reads it again).

**Description.** Before the change one world per tick read the finalized clock (`getSlot` + `getBlockTime`, each agreed by two providers: 4 outcalls) and the oracles once. Now each listing's custodian reads the clock (even with no ticket), and each live collection's frog side reads it again with its sinks, so a quiet tick costs about 8 outcalls per live listing and 4 per dormant one: with 15 Ordinals listings live about 120 HTTPS outcalls a tick where there were 8, at the idle cadence (60 s) and more at the busy one (10 s). Cycles scale with it, and so does the providers' rate limit; a refused read stalls that collection's pass (and, for the custodian, its exits) for the tick.

**Fix.** One `SolNet` (clock, rates) per canister tick shared by every collection's world; `ticket_count` reads the books' count first and the clock only when there is a new ticket; a listing with nothing open skips its sink read when its module's posted floor has not changed.

**Fixed (2026-10-12):** one Solana world per canister tick (`frognet::tick_sol` / `keep_tick_sol`): the custodian's pass makes it and
hands it, listing to listing (`CustodianIo::for_coll_with`), to the frog side's pass of the same tick (`ItemIo::for_coll_with`, the
classification's world included), so the finalized clock is read once and each rate at most once a tick for every listing. Count (in
the comment on `tick_sol`): a quiet tick with 15 listings makes one clock read, 4 HTTPS outcalls (getSlot + getBlockTime, two
providers each), plus each oracle rate only when a purchase is valued or a floor posted: 4 to 8 Solana outcalls, where it was about 120;
the per-listing reads that remain (ticket counts, sink views) are the books', in memory. The ticket count and the sink read keep reading
the (now shared, cached) clock. The cycles budget with 15 listings is still to be re-measured on the rehearsal canister (§4, must-hold
4). Regression: none in the test worlds (they implement `SolIo` without `SolNet`); the sharing is the two `tick_collections`' code and
the count is the comment's.

---

### C2-05: one unreadable output stops the classification (Low)

**Files:** `src/frogsol/multi.rs:336` (`io.contents(&u.outpoint).await?`), `src/frognet.rs` (`tick_collections`: "custody: waits: not classified").

**Description.** `classify` returns the first contents error for the whole batch, before any route is applied; the same output is first in line next tick. A depositor who can make ord's providers disagree on an output (or one that times out) stops every later output from being classified, for good. The classification only feeds the reconciliation (`unmarked`, per collection), so nothing is mis-credited; the reconciliation goes blind and the outcalls repeat each tick. `deposits_pass` had the H-3 set-aside (`read_failures` → `unread`) for exactly this; the new path reads `unread` but never adds to it.

**Fix.** Count contents failures per output as `deposits_pass` does and set the output aside after `MAX_READ_FAILURES`; skip, never abort the batch.

**Fixed (2026-10-12):** `multi::classify` skips an output whose contents cannot be read and returns it with the routes; `multi::set_aside`
counts the failures per output (`read_failures`) and sets it aside (`unread`) after `deposits::MAX_READ_FAILURES`, as `deposits_pass`
does (H-3); `apply` drops spent outputs from both lists. The rest of the batch is classified the same tick. Regression:
`frogsol::multi::tests::c2_05_an_unreadable_output_is_set_aside_and_never_the_batchs_end`.

### C2-06: classification is decided once (Low)

**Files:** `src/frogsol/multi.rs:265-285` (`route`: live roots only; proofs already handed in only), `:296-300` (`seen` forever).

**Description.** A member of a dormant collection, or a member whose `frog_proof` comes after the output is seen, routes `NotMember` and is `seen`; waking the collection or handing in the proof never re-routes it. With holder deposits off on the Solana home nothing asks the runner for a proof of an arriving output, so in practice almost every member lands in the custody-wide `unmarked`, and `unmarked_custody(Some(coll))` is empty. Reconciliation only; no credit.

**Fix.** Keep `NotMember` for "no root at all"; re-route the custody-wide `unmarked` on `frog_wake` and on `frog_proof`.

**Fixed (2026-10-12):** `multi::reopen` takes every output refused custody-wide out of `seen` (it stays in `unmarked` until a route
makes it a collection's), at `frog_proof`, `frog_wake` and `frog_sleep`; the next ticks route it again (bounded by
`MAX_NEW_OUTPUTS_PER_TICK`), and `apply` moves a member to its collection's `unmarked` and refuses a non-member again without a second
line. Under default-live the dormant half is gone (every root is live but a slept one's). Regression:
`frogsol::multi::tests::c2_06_a_member_refused_before_its_proof_or_its_wake_is_routed_again`.

### C2-07: allocation of the Relay leg and the frog turn by collection id (Low)

**Files:** `src/frognet.rs:565` (collections in id order), `:221` (`eth_relay_busy` + `held_elsewhere`), `src/frogsol/multi.rs:106` / `src/ethnet.rs` (`wants_relay` across every collection), `src/convflow.rs:319`.

**Description.** When the leg frees, the first collection in id order with `NeedAdvance` takes it; the Ethereum side starts no buy while any collection waits (B-L-6's fairness, written for one Bitcoin side). Under steady demand on low ids, high-id collections and the Ethereum / Robinhood purchases are not served. No loss.

**Fix.** Round-robin the leg among collections (a cursor in `FrogState`), and let the Ethereum side yield to the Bitcoin side once per leg, not to every collection in turn.

**Fixed (2026-10-12):** the live collections take their passes in a rotation (`FrogState.turn`, `multi::in_turn`: after the last that
took the leg or a step on the line), so the first wanting the leg or the frog turn is the next in a round. When an Inscriptions
collection takes the leg the Ethereum side's turn is due (`FrogState.eth_turn`): `multi::wants_relay` no longer holds the Ethereum
side's new buys back, and once the leg is free the Bitcoin side yields it for `ETH_TURN_SECS` (120 s; `multi::eth_turn_now`, the
collections' `eth_relay_busy`), the turn lapsing if the Ethereum side takes nothing and ending when it does. Regression:
`frogsol::multi::tests::c2_07_the_leg_and_the_frog_turn_rotate_and_the_ethereum_side_gets_its_turn`.

### C2-08: caps follow the live set at run time (Low)

**Files:** `src/canister.rs:398-428` (`caps_of` → `frognet::live_cap_keys`, read at every `with_caps`), `:719-726` (`check_cap_change` and `held_fits` run before `FROGS` and the desk are restored, `:742` / `:815`).

**Description.** H2's rule is that caps rise only by the controller's upgrade, ×4 at most, a new collection at stage 0. With default caps, the operator's `frog_wake` adds a collection at stage 0 (3 items, 10 ETH) and raises the global value cap by its 10 ETH at once (the global items stay clamped by the exit rule); `frog_sleep` removes them. At an upgrade, the before/after comparison and the held-items check see no Inscriptions collection at all (their caps are computed while `live_cap_keys` reads empty state). The exit rule still holds for the global item count; the H2 statement "caps change only through the timelock" no longer does for Inscriptions.

**Fix.** Name the Inscriptions caps from the books (every listing, live or not, as default-live will) rather than the life cycle; run `check_cap_change` / `held_fits` after the frog state and the desk are restored.

**Fixed (2026-10-12):** `caps_of` names every Inscriptions listing in the books at stage 0 (`frognet::listing_cap_keys`, live or asleep,
a repeated root excepted), so `frog_wake` / `frog_sleep` never change the caps; the caps follow the controller's pins only. In
`post_upgrade`, `check_cap_change` and `held_fits` run after the books (`desk::after_upgrade`) and the frog state are restored, so both
sides of the comparison name the same listings. Regression: `frogsol::multi::tests::the_default_caps_name_every_listing_and_the_exit_rule_sizes_the_global`
(a slept listing keeps its cap, a wake changes nothing) and the ordering check in
`canister::tests` (`held_fits` / `check_cap_change` after `desk::state::after_upgrade()` and the frog state's restore).

### C2-09 – C2-14 (Informational)

- **C2-09 (`ticket:<coll>:<n>`).** `owed` asks the limiter under the key before signing and records it after (`src/redeem.rs:903-966`). A redemption that asked (and waited) under `ticket:<n>` asks again under `ticket:<coll>:<n>` after the upgrade: its demand counts twice toward the tripwire for 24 h, and a key acknowledged by a resume (`acked`) is fresh again. A delivery already signed is recorded once (no re-record). Before this change the books home ran no custodian (`Init.wrapper` unused, `cfg().wrapper` gated the tick), so no such state exists to migrate; a later key change should map old keys.
  **Recorded, not changed:** no custodian state ran under the old key on the books home, so there is
  nothing to map; a later key change maps old keys.
- **C2-10 (migration).** `has_collection_work` counts `sinks` (`src/frogsol/multi.rs:171-173`): floor sinks configured before the first migration (a ceremony that pins and then calls `set_floor_sink` between two ticks) wake the lowest-id listing without `frog_wake`, and the entry receives every configured sink (the others are refused there as another collection's module). Moot under default-live.
  **Fixed (2026-10-12):** `has_collection_work` no longer counts the configured floor sinks (moot under
  default-live, where nothing waits for a wake). Regression: `the_single_state_of_before_restores_and_moves_into_its_listing`.
- **C2-11 (brakes).** `frog_sleep` is refused while anything is open and stops nothing in flight; there is no per-collection pause. Under default-live, where sleep is the operator's only per-collection brake, a collection misbehaving mid-purchase can only be stopped by the global pause.
  **Recorded, not changed:** a per-collection pause of work in flight would stop the very passes that finish it
  (and C2-02 refuses a sleep that would strand the leg); the global pause stays the stop for work in flight, `frog_sleep` the brake on new
  work.
- **C2-12 (first floor).** `post_floor_raw` clamps a reading into the band of the previous accepted floor only when there is one; the first reading of each listing is accepted as given. With dormancy the operator's wake was a human check before the first reading; default-live leaves the runner's 20 % CoinGecko band as the only gate on 14 first floors.
  **Fixed (2026-10-12), with the default-live switch:** a listing's first reading (no accepted floor) is refused,
  nothing posted, unless it is within ±50 % (`FIRST_FLOOR_BAND_BPS`) of the floor its launch pin was sized from (`graduation-tiers.json`
  `floor_sol`, keyed by members root in `frogsol::launch`, held to the launch files by a test); a listing no launch file names posts no
  first floor. Later readings are clamped by the module as before. Regression: `fixed_c2_12_a_first_floor_outside_the_band_of_its_launch_floor_is_refused`
  (NodeMonkes: 4.86 SOL refused, 17.46 SOL posted) and `frogsol::launch::tests`.
- **C2-13 (minimum and refunds).** After a partial refund the conversion is re-funded with what came back (`src/convflow.rs:164-172`); if that is under `RELAY_SOL_BTC_MIN_LAMPORTS` every next deposit is refused at the judge, and the conversion leaves only by its give-back (its deadline or `relay_give_back`). Correct (no order under what Relay fills is sent); a liveness note for the runbook.
  **Fixed (2026-10-12):** a partial refund under `RELAY_SOL_BTC_MIN_LAMPORTS` (on the Solana home)
  goes straight back to its module (`Returning`) instead of being re-funded for a deposit the judge would always refuse. Regression:
  none (a refund of half a deposit under 0.15 SOL needs a deposit under 0.3 SOL, which no test world's quote makes; the threshold is one
  comparison in `convflow::watch`).
- **C2-14 (overlapping roots).** `route` refuses an inscription proving against two live roots, and the launch-params test keeps the launch file's members disjoint; nothing in the canister (`coins_pin`, `frog_wake`) refuses a listing whose root overlaps another's, and `held_markers` are per collection, so the same inscription could be one item in each of two ledgers only if bought twice. No path today.
  **Fixed (2026-10-12), in part:** a listing whose members root repeats a lower id's is never live and
  gets no cap (`multi::repeats_root`); two different roots sharing members cannot be told from the roots, and stay the launch-params
  test's (the launch table's roots are distinct, `frogsol::launch::tests`). Regression: `c2_14_a_repeated_members_root_is_never_live`.

## 3. Not findings (checked)

- **A credit to the wrong collection.** Items enter a ledger only by `Call::Mint` after a buy judged against `coll(io)`'s root; classification credits nothing (it feeds the reconciliation). A proof handed in for collection A that verifies against B's root proves B membership: routing it to B is right.
- **An outcome taken by the wrong collection.** Every SOL-moving frog step names its frog; book steps name their collection; `None` (the old `"books"` signature) is anyone's, which only the migration's leftovers can carry.
- **A dormant collection buying or posting.** It is not ticked; `frog_floor_reading` refuses it; its endpoints store hand-ins that are never acted on.
- **Exits of a dormant collection.** The custodian runs every listing; `release` reads no cap.
- **The minimum's bounds.** `[0.15 SOL, 1 SOL]`, at most the maximum, Solana home only, `None` restoring 1 SOL; the Relay deposit floor is 0.15 SOL less the keep, so a minimum-cost purchase's deposit is exactly the floor.

## 4. Operator decision (2026-10-12): every Inscriptions listing LIVE at install

The fix round will make every Inscriptions listing start live, pin all 14 Ordinals at the ceremony with the others, and let a collection mint only once the runner hands in a floor reading that passed its 20 % CoinGecko band; `frog_wake` / `frog_sleep` stay as brakes. Assessment:

**What default-live exposes.**
- **Deposits, proofs, buys with no floor yet.** Safe by construction: `books_sink_view` reports `open = accepted > 0`, so a module never posted wants nothing and no frog is created; `mint` / `burn` refuse a stale module. Deposits of members are only classified (holder deposits are off on Solana); with every root live, a member routes to its collection when its proof was handed in first (C2-06's dormant half disappears; the proof-later half remains). `frog_proof` / `frog_offer` for a collection with no floor are stored and judged as before.
- **The first floor.** Each listing's first reading is accepted without a previous floor to clamp it (C2-12): the runner's band check is the only gate for 14 first floors at once. The canister should hold a first reading to a confidence and depth minimum (`min_confidence_bps`, `depth_items`), or the ceremony should post each first floor by hand.
- **Caps.** `caps_of` names every live listing at stage 0 (3 items, 10 ETH): 15 Ordinals listings plus the Ethereum collections. The global item cap is the exit rule's 12 (stage 0's 6 a day × 2 days), so the WHOLE canister holds at most 12 items across every collection, first come; the global value cap is the sum (about 150 ETH for the Ordinals alone), so the item count binds. That is the A4-L-1 rule working, but it makes C2-01 decisive: with the book undercounting, neither bound holds.
- **The custodian, cycles and the line.** 15 live listings make C2-04 about 120 outcalls a quiet tick; C2-03's window opens every runner cycle (15 readings → 15 raw outcomes); C2-07's allocation by id decides which collections and whether the Ethereum side get the Relay leg under load.
- **Classification and reconciliation.** Every root live: one ord read per new custody output (bounded by `MAX_NEW_OUTPUTS_PER_TICK`), C2-05's stall risk unchanged.
- **The brakes.** `frog_sleep` is refused with anything open (C2-11) and, as built, can strand the Relay leg (C2-02); `frog_wake` re-adds caps at run time (C2-08).

**Findings it changes.**
- C2-01: from "two collections woken" to certain from the first purchase after install. **Must be fixed before the switch.**
- C2-03, C2-04, C2-07: likelihood up (15 live from day one).
- C2-06: the dormant-member half goes away.
- C2-08: the wake-time cap addition goes away at install (all named from the start); `frog_sleep` still removes a cap at run time (the safe direction) and a later `frog_wake` re-adds it outside the ×4 path.
- C2-10: moot (nothing to wake). The life-cycle default flips ("no life = live"); `is_live`, `live_cap_keys` and the classification's roots must all read the new default, and `frog_sleep` must create a dormant life.
- C2-12: new exposure (the human gate is gone).

**What must hold before the switch.**
1. C2-01 fixed, with its PoC inverted.
2. C2-03: a landed SOL outcome is never trimmed for book outcomes.
3. C2-02: `frog_sleep` refuses a collection holding the Relay leg (or the tick steps dormant conversions), since sleep becomes the brake.
4. C2-04: the per-tick reads shared across collection worlds, and the cycles budget re-measured with 15 listings (dry run on the rehearsal canister).
5. Every listing's purchase limit and minimum (`set_purchase_limit`), OW creators (`set_ow_creators`) and floor sink (`set_floor_sink`) set at install, before the runner's first reading: dormancy was the step that waited for them (§5's wake check); default-live does not.
6. The first-floor guard (C2-12) or a hand-posted first floor per listing.

**The switch, done (2026-10-12).** Every Inscriptions listing is live from install: a listing with no life is live (`multi::is_live`), and
`live`, the caps (`listing_cap_keys`, every listing) and the classification's roots read it; `frog_sleep` gives a listing a DORMANT life
(the operator's brake), `frog_wake` lifts it; the ceremony calls neither (`docs/v9/ceremony-canister-only.md` §5, no `frog_wake(4)`).
The must-hold list: 1. C2-01 fixed, its PoC inverted. 2. C2-03 fixed. 3. C2-02 fixed (`frog_sleep` refused while the leg is held or an
outcome is unread). 4. C2-04 fixed (one clock read a tick for every listing); the cycles budget with 15 listings is still to be measured
on the rehearsal canister's dry run. 5. Before a listing's first reading is posted the canister sets, where nothing is set, its floor
sink (its module), its purchase limit and minimum (`nft-pilot.json`), its OW creators (none pinned: a royalty-carrying protected sale is
refused) (`multi::launch_defaults`, `frogsol::launch`). 6. The first-floor guard (C2-12, ±50 % of the launch floor). Regression:
`frogsol::multi::tests::default_live_a_fresh_install_with_15_listings` (15 listings from the launch files: all live, none can mint or buy
without a floor, a first floor outside the band refused, 45 items named and the global held to the exit rule's 12),
`default_live_a_listing_is_live_with_no_life_and_sleeps_only_as_the_operators_brake`,
`default_live_the_launch_defaults_are_set_where_nothing_is`.
