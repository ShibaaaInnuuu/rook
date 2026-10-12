# Round 5, part E: the next-mint key (E4-H-1), the H3 ask sites, the whole-custody exit rule (A4-L-1), noteSent at finality (A4-L-2), E4-I-2, upgrade safety

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1dc2a61`. The delta
read is `git diff 2735dac..HEAD -- autobuyer/canister-rs/` (fix round 4, `57032dd`), with the older code where a finding goes back
further. Scope:

- E4-H-1: `mint_key`, `home_act` steps 2 and 3 (`waited`), every way the count `n` can move.
- The 17 H3 ask sites and the source-reading test `r4_every_h3_ask_is_keyed_per_logical_outflow`.
- A4-L-1: `validate_custody`, `exit_room`, the default install's clipped global, `ASSUMED_DELAY_SECS`, install and upgrade.
- A4-L-2: `noteSent` at finality (`delivery_pass`, `home_act` step 1, `home_act_paused`), marks while paused.
- E4-I-2: `deposit_floor_ok`'s `accepted == 0` refusal.
- Upgrade safety of every new saved field, the `.did`, the fixtures (`eth-state-round2.hex` re-encoded, `eth-state-round4.hex`,
  `state/saved-*-rich.hex`), the probe's cost.

Toolchain 1.98.1. `src/nft/EthNftWrapper.sol` was read for `_admit`, `resolve`, `noteSent` and `refund` as the canister uses them.

**Nothing in the worktree was modified except this report and `review5/poc/E/`; nothing committed, pushed, deployed or signed.** Every
PoC ran in a private copy (`/tmp/rook-r5/E/`, an APFS clone of `autobuyer/`, checked identical to the tree with `diff -rq` of
`canister-rs/src` and `tests`). The copy was deleted mid-review by another reviewer's `rm -rf /tmp/rook-r5`; it was re-cloned, re-checked
identical, and every PoC output kept here was produced after that. The old build is a `git archive` export of `2735dac` (its canister
tree is identical to `1eb5788`'s: `git diff 1eb5788 2735dac -- autobuyer/canister-rs` is empty). The copy was removed at the end.

Severity scale: the README's.

## Verdict

- **E4-H-1: closed.** A collection's waiting mint, a purchase's or a deposit's, is one ask under `mint:eth:{col}:{n}`, and `n` moves only
  when a mint goes out or a mint is read as not having happened. No path lets two real mints share a key in a way that releases more
  than it counts (a reverted mint released nothing). Review4's PoC now fails on its attack assertion. The variant with two purchases of
  the same collection among nine deposits mints all eleven, at most three a day, with at most four asks in any window.
  - Two residuals, both new here: **E5-L-1** (Low) and **E5-I-1** (Info).
  - E5-L-1: round 4's `waited` makes a deposit whose mint can never go out block every later deposit of its collection.
  - E5-I-1: a reverted mint's record keeps the time it was sent, so a collection can mint one more than its limit in 24 h, once
    per revert. This also held before round 4.
- **The H3 ask-site test is complete today but not meaningful as a guard.**
  - My own grep of `src/` finds exactly the 17 sites it lists, and I judge each one to be keyed per logical outflow.
  - The test counts the string `outflow_check(&` in 7 named files. It also checks that each key's text appears somewhere in the
    file.
  - It still passes when `mint_key`'s body goes back to the E4-H-1 shape, when the listed key survives only in a comment, when a site
    is added to a file it does not read, and when a site is added without the `&` (**E5-I-2**).
- **A4-L-1: the arithmetic is right. It runs at install and at every upgrade. `ASSUMED_DELAY_SECS` equals the timelock's floor.** A
  refused upgrade traps `post_upgrade` atomically, so it is recoverable.
  - The rule is only half of what "holders can all leave" needs. It checks the rate limits and not the tripwire.
  - At stage 0's defaults, a collection's whole custody (6) redeemed together with that day's 3 new units asks exactly the 9 that
    freeze custody. With `trip_factor` 2, which `outflow::validate` allows, the bank run alone trips it (**E5-L-2**, Low).
  - The launch numbers, and every option (a)–(d) of `fixes-round4.md` §6, keep a margin.
- **A4-L-2: holds.** A delivery is noted only from its own receipt at finality. A reorg-dropped transfer is never noted. A mark while
  paused signs nothing on Ethereum and moves no custody.
- **E4-I-2: closed.**
- **Upgrade safety: holds.**
  - Every new saved field is `opt`, inside a record that is new or already `opt`. No enum case was added.
  - The `.did` matches the types and is `service_compatible` with `2735dac`'s.
  - States written by `4949da0`, `a37f54b` and `1eb5788` restore with nothing lost.
  - `saved-1eb5788-rich.hex` is genuine: the `2735dac` (= `1eb5788`) build, running the fixer's dumper, writes it byte for byte.
  - The round-2 fixture was re-encoded only because the test compares bytes, and the type table grew. All four historical encodings
    decode into round 4's types with nothing lost, and round 3's and round 4's encodings are the same state.
  - `eth-state-round4.hex` leaves out `Inbox.payouts` (**E5-I-3**).
  - The probe on the largest round-4 state (5.4 MB) takes 0.61 s native.

## Counts

| | Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- | --- |
| New (E5-*) | 0 | 0 | 0 | **2** | **3** |
| Round-4 items in scope not closed | 0 | 0 | 0 | 0 | 0 |

## Round-4 items in scope

| ID | Sev (R4) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **E4-H-1** | High | **Closed** (residuals E5-L-1, E5-I-1) | See **E4-H-1** below the table. |
| A4-L-1 (the install check) | Low | **Fixed as a rate-limit rule** (its missing tripwire half: E5-L-2) | See **A4-L-1** below the table. |
| A4-L-2 | Low | **Closed** | See **A4-L-2** below the table. |
| E4-I-2 | Info | **Closed** | `deposit_floor_ok` (`ethflow.rs:2827-2830`) refuses while the module's `ACCEPTED` read is 0. The admission's `mark` is the same read (`module_floor` `:1446-1450`). `accepted` never returns to 0 after a post. `r4_e4_i2_…` passes. |
| E4-I-4 | Info | **Fixed, one field short** (E5-I-3) | `eth-state-round4.hex` holds `late`, `solver`, `used_since`, `bad`, `skipped`, `home_floor`, `pin_floors` and `credited`, but not `Inbox.payouts`. |
| Upgrade safety | — | **Holds** | See **Upgrade safety** below the table. |

**E4-H-1.** Where `n` can move while a mint waits:

- `mint_key` (`ethflow.rs:2846-2851`) counts non-sweep buys in `Minting | Minted | Settling | Settled`, and deposits in
  `Minting | Minted`.
- Every transition out of those phases was traced (`mined_home` `:3216-3240`, `:3276-3289`; all `.phase =` assignments in
  `ethflow.rs`, `ethnet.rs`, `canister.rs`):
  - `Minted → Settling → Settled` stays counted, and `Settled` is final.
  - A deposit's `Minted` is final.
  - Nothing prunes `st.buys` or `st.deposits`. Only `NeedAdvance` buys and unsent sweeps are removed.
  - `eth_give_back` and `eth_credit_payout` touch only `Funded`, `Arrived`, `Back*`, `Stuck` and `Returning` buys. A written-off
    buy (`late`) is `Stuck` without a watch and cannot reach `Minting`.
- `n` goes **down** only when a mint is read as not having happened: a revert, or C4-H-1's drop (`resolve_home` `:3150-3160`).
- `n` goes **up** only when a mint goes out, which needs `Allow` under the current key.
- A buy and a deposit of the same collection never race:
  - `buy_holds_mint` (`:694-700`) keeps deposits from asking while a purchase is between its advance and its mint.
  - When both ask in one pass, they ask the same key.
- A deposit reverting is covered above. A burn or redemption does not touch these phases. A sweep is excluded on both sides.
- An upgrade leaves the count unchanged. The old keys were `mint:eth:{buy id}` and `mint:deposit:{tx}`, a different shape, so no
  new key collides with a record carried over.
- **Window rolls:** `n` only grows over the canister's life, so a key is never reused after its record left the window.
- **One candidate per collection a pass:** verified. A candidate that ends `Ok(None)` (sent back for custody room, or held) lets the
  next candidate of the same collection ask, but under the same key.
- **Two mints under one key:**
  - After a revert, the next mint (the same deposit, or another candidate) goes out under the reverted one's key. `record` is
    idempotent, so it is not counted again.
  - Released and counted still match: the reverted mint released nothing, and the one that re-uses its key is counted once.
  - Only the time is off (E5-I-1).
  - After a drop whose mint actually mined, the re-send's estimate reverts (`AlreadyMinted` / `AlreadyHeld`), so nothing is released
    twice. That is E5-L-1's wedge instead.
- **PoCs:**
  - Review4's `e4_h1_…` re-run on this code fails on its attack assertion: "minted 3 deposits; mint asks 4; frozen None; paused
    false" (`review4_e_rerun.output.txt`).
  - `holds_e4_h1_nine_deposits_and_two_purchases_…`: two purchases in `Final` and nine deposits of Milady, at the launch limits with
    the real limiter, for four days.
  - Result: 11 minted, keys `…:0` to `…:10`, at most 4 mint asks in a window, at most 3 real mints in any 24 h, never frozen.

**A4-L-1.** `validate_custody` (`canister.rs:302-326`):

- **The checks:** each collection's `exit_fits(max_items, per_collection_items)`, and then `exit_fits(min(Σ max_items,
  global_max_items), global_items)`.
- **The arithmetic:**
  - `exit_fits` is `ceil(items / per_day) ≤ delay_days − 1`. That is 2 days at 72 h.
  - It is conservative against H3's sliding window: acts leave the window at exactly 24 h, so three windows fit in 48 h.
  - The per-collection and global checks together are also sufficient: two identical days with per-collection and global capacities
    admit any split in which each `c_i ≤ 2 p_i` and `Σ c_i ≤ 2 G`.
  - The saturating casts cannot overflow.
- **Rounding:** `div_ceil` everywhere, including the message's suggested rate (61 → 31).
- **The global cap where it binds:** `min(sum, global_max_items)` is the right total. Option (b) (a global 48 with J7's per-collection
  caps) passes, as the fixer's unit test shows.
- **The default install:**
  - `caps_of` clips the default global to `max(exit_room, largest)`: 12 at stage 0.
  - `largest` fits the global rule whenever it fits the per-collection one, because `per ≤ global`.
  - Lowering the global is always accepted by `validate_change`. A running default install whose book already holds more than 12
    only stops admitting until items leave.
- **It runs at upgrade:**
  - `post_upgrade` calls `validate(&arg)` (`canister.rs:500`), which is `validate_custody` on the raw argument.
  - Then `check_cap_change(saved, kept)` (`:502`, `:556-565`) runs `validate_custody(kept)`.
- **Can it brick an upgrade?**
  - A canister installed by an earlier build with the operator's J7 / 24-a-day pair would trap `post_upgrade` on this build unless the
    same upgrade names numbers that fit.
  - The trap is atomic: the old code and state stay, and a new proposal with option (a) or (b) passes, since lowering is always
    allowed.
  - The only cost is another pass through the timelock's 72 h. Nothing is deployed, so there is nothing to fix. The operator should
    know that the upgrade carrying round 4 must also carry the A4-L-1 numbers.
- **`ASSUMED_DELAY_SECS`** (72 h) equals `timelock-rs/src/rules.rs` `MIN_DELAY_SECS` (72 h in production; 10 s only in the
  rehearsal build). A real delay above the floor only helps.
- The missing half is **E5-L-2**.

**A4-L-2.**

- **Noted at finality only.** `noteSent` is now signed only from `DPhase::Record`, in `home_act` step 1 (`:2875-2891`) and in
  `home_act_paused` (`:2779-2795`). `delivery_pass` (`:1845-1882`) sets `Record` only when all of these hold:
  - the `Moved` block is under the finalized one;
  - our delivery's own receipt is read at the pin;
  - it has the same block and succeeded;
  - it carries the collection's `Transfer(us → destination, token)`.
- **A reorg-dropped transfer:**
  - Before the ticket leaves `Resolved`, it errors "never noted" every tick and is never noted.
  - Once the wrapper's late window refunded it, it becomes `Gone`.
  - `r4_a4_l2_a_delivery_a_reorg_drops_…` passes.
- **Can noting at finality reopen D3-H-1's 30-day late refund?**
  - Only if our own receipt cannot be read at finality for about 30 days after it was first seen mined. Round 3 had the same need at
    the first sighting, a read of the same kind.
  - The added exposure is the minutes between the sighting at the pin and finality, so it is not a new finding.
  - A collection whose successful transfer carries no standard indexed `Transfer` would never be noted. None of the five configured
    collections is one.
- **Marks while paused:**
  - `markUndeliverable` signs nothing on Ethereum and releases no item. The NFT stays in custody and goes back to the free set on
    refund. It only lets the ticket's owner be refunded after the grace.
  - The pause's other exits are unchanged: `send_home` admits `Record`, `Sent` and `Mark` only. While paused, marks come only from
    `MarkAfter` at finality (`delivery_pass`) or from marks already due; `mark_pass` does not run.
  - A mark is exactly as trusting of the chain reads as a record, which always went out while paused. It is therefore not a new
    bypass of the pause.

## Findings

### E5-L-1 (Low): a deposit whose mint went out but was read as not minted wedges every later deposit of its collection

- **Where:**
  - `autobuyer/canister-rs/src/ethflow.rs:3081-3086` and `:3112-3116` (`home_act` step 3, round 4's `waited`): once a collection's
    first `Mint` candidate returns any `Err`, the rest of that collection is skipped this pass. The first candidate is always the same
    deposit (the lowest index).
  - `ethflow.rs:3276-3288` (`mined_home`, `Step::MintDeposit`): `minted = ok || …itemOf/item read….is_ok_and(..)`. A read error
    counts as "not minted", and the deposit goes back to `XPhase::Mint`.
  - `ethflow.rs:3150-3160` (`resolve_home`, C4-H-1's drop): a home transaction whose nonce is used and which is seen nowhere for
    `HOME_NONCE_TAKEN_SECS` is taken back with `mined_home(.., false)`.
- **The trigger:**
  - A `mintDeposit` mines, but no provider shows it for an hour. This is C4-H-1's case: an index lag, or a provider failing over.
  - At the drop, the wrapper's `itemOf` read fails once, a transient error.
  - Both are faults, not attacks. Neither is refused by anything.
- **What follows:**
  - The deposit is back at `Mint`. Its unit exists on 4663 and its token is on the wrapper.
  - Every pass it asks H3 (`Allow`, under the key already recorded), then `send_home`. `home_sign`'s estimate reverts
    (`AlreadyHeld` / `AlreadyMinted`: `EthNftWrapper._admit`), the collection goes into `waited`, and no later deposit of that
    collection is tried.
  - Nothing re-reads a `Mint` deposit's `itemOf`: `deposit_pass` does so only at finality. There is no operator exit either:
    `eth_rearm_stray` takes only `Held`.
- **Effect:**
  - Every later deposit of the collection waits at `Mint` with its NFT in custody, admitted to the book, and no unit minted, until an
    upgrade.
  - New deposits keep arriving and are admitted until the collection's custody cap, then sent back.
  - Other collections are unaffected: C-L-2 holds across collections. Purchases, burns, redemptions and the floor are unaffected.
- **History:** before round 4 the loop went on to the next candidate, so only the misread deposit was stuck. `waited` turns one stuck
  deposit into a stuck collection.
- **The purchase sibling is older:** the same misread of a buy's `Step::Mint` (`:3220`) puts it back to `Final`. `buy_holds_mint`
  then holds that collection's deposits for good.
- **Why Low:** it needs two faults in the same tick, nothing is lost, and an upgrade recovers it. It escalates to Medium if the
  monitor does not page a deposit stuck at `Mint` (part D's scope), because depositors keep sending NFTs into the wedged path.
- **PoC:** `r5_e.tests.rs` `e5_l1_a_mint_misread_as_dropped_wedges_every_later_deposit_of_its_collection`.
  - The harness switches are `hidden` (the mint mined, seen nowhere), `item_of_err` (one failed `itemOf` read) and the estimate's
    refusal of a held token, as `EthNet::home_sign` does.
  - After 3 days, A is `Mint` although its token is on the wrapper, B is `Mint` (never minted), and Azuki's deposit is `Minted`.
    Every pass says "waits: would not succeed now: execution reverted: AlreadyHeld()".
  - Control: without the misread, both Milady deposits mint.
- **Fix** (any one; the first is the smallest):
  - Put a collection into `waited` only for errors that are the collection's own: H3's wait, the floor, `buy_holds_mint`, the
    module read. A failed `send_home` should move on to the next candidate.
  - Or, in `mined_home`, treat an `itemOf` read error as unknown: keep `Minting` and read again next tick, never "reverted".
  - Or re-read `itemOf` / `item` for a `Mint` deposit whose estimate reverts, and adopt it as `Minted` when the wrapper holds it from
    this very transaction.
  - Apply the same read-error rule to `Step::Mint` for purchases.

### E5-L-2 (Low): the exit rule does not keep a bank run under the tripwire; at stage 0 the full custody plus the day's mints freezes it

- **Where:**
  - `autobuyer/canister-rs/src/canister.rs:302-326` (`validate_custody`) holds custody to the rate limits only.
  - `outflow.rs:211-224`: the Item tripwire trips when a collection's distinct delivery asks in 24 h reach
    `trip_factor × per_collection_items`, or the global ones `trip_factor × global_items`. Deliveries are keyed one per ticket
    (`ethflow.rs:2211`, asked at `:2230`), so a collection's demand in a window is the items it held plus the items minted in that window.
  - `custodycap.rs:40` (`STAGE0_EVM_ITEMS` 6) and `outflow.rs:108-111` (stage 0: 3 items and 3 mints a collection a day, trip ×3).
  - The fixer's claim in the source test and in `fixes-round4.md` §2, "deliveries … custody bounds them (24 < 36; 61 + 6 < 72)",
    uses the launch numbers only.
- **The gap:**
  - The exit rule lets a collection hold `2 × per_day` items.
  - Its demand in a bank run is then `2 × per_day + per_collection_mints`, which reaches the tripwire when
    `per_collection_mints ≥ (trip_factor − 2) × per_day`.
  - The default install (`outflow_limits` and `custody_caps` null) does exactly that: 6 items + 3 mints = 9 = 3 × 3.
  - With `trip_factor` 2, which `outflow::validate` accepts, even the bank run alone trips it. So does the global, with the default's
    clipped 12 + 6 mints = 18 = 6 × 3.
- **The trigger** (default install, deposits on, a collection holding its 6 items):
  - Its holders redeem all 6. Three go out and three wait.
  - The collection's 3 mints of the day (deposits or purchases) land in the room freed.
  - Their units are redeemed within the same 24 h. The 9th delivery ask freezes custody, and the canister is paused.
- **Who can trigger it:** an honest bank run, which is the exact case the exit rule exists for. Or one party with 9 units of the
  collection and 3 of its NFTs, who gets NFTs back for the units: roughly 8 ETH of Milady capital, mostly recovered.
- **Effect:** as E4-H-1's. Floor posts, mints, burns and deliveries stop for every holder until k guardians resume. The resume
  acknowledges the asks, so the same run does not re-trip it. Holders cannot leave during the freeze, which defeats A4-L-1's promise.
- **Why Low:** the launch file names its limits:
  - Azuki's 22 + 3 < 36 per collection.
  - 48 + 6 < 72 in all under option (a) or (b).
  - Option (c) raises the trip with `global_items`.
  - So no configuration the operator plans hits it. It becomes Medium on any install that runs the stage-0 defaults (or trip ×2)
    with deposits on.
- **PoC:** `r5_e.tests.rs` `e5_l2_at_stage_0_a_full_collection_redeemed_with_its_days_mints_trips_the_item_tripwire`, using the real
  limiter and the default caps.
  - `exit_fits` passes (6 at 3 a day; 12 at 6 a day).
  - 6 delivery asks and 3 mints, then the new units' redemptions: `[Wait, Wait, Frozen(Tripwire { flow: Item, demanded: 9, limit: 3 })]`.
  - With trip ×2 at the launch rates, a collection at its exit-rule maximum (24 at 12 a day), all redeemed: the 24th ask is
    `Frozen(… demanded: 24, limit: 12)`.
- **Fix:**
  - Add the tripwire half to `validate_custody`, per collection and in all:
    `max_items + per_collection_mints < trip_factor × per_collection_items` and
    `total + global_mints < trip_factor × global_items`, or require `trip_factor ≥ 3` and `mints ≤ items`.
  - Or raise stage 0's trip factor, or lower `STAGE0_EVM_ITEMS` / the default global so that the default install passes.
  - Correct the delivery line of the H3-site list to state the bound as "custody + the window's mints".

### E5-I-1 (Info): a reverted mint's record keeps the time it was sent; the next mint under its key is counted at that earlier time

- **Where:**
  - `outflow.rs:244-251` (`record`, idempotent per key: the first `at` stays).
  - `ethflow.rs:2846-2851` (`mint_key` takes `n` back after a revert).
  - `mined_home` `:3231`, `:3288`.
- **Effect:**
  - A mint recorded at `t` reverts, and the next mint of the collection goes out under the same key at `t' > t`. It is counted as at
    `t`, so the window frees its slot at `t + 24 h` while the real mint's 24 h run to `t' + 24 h`.
  - One more real mint than `per_collection_mints` fits in a 24 h stretch, once per revert, for `t' − t`.
  - This is not a bypass of the count: the reverted mint released nothing. It is the same semantics as before round 4 for a deposit's
    own key. The wrapper's own mint limit (H11) still bounds mints.
  - Reverts are ours (a listing pause, `MintRateLimited`), not a stranger's: `_admit` has no recipient callback.
- **PoC:** `e5_i1_a_reverted_mint_s_record_keeps_its_time_…`: real mints at t0 + [0, 83,400, 84,000, 85,800] s. That is four in
  under 24 h at a limit of 3, with the reverted key recorded at t0 − 600.
- **Fix (optional):** on a revert or a drop, move the key's `done` entry to the re-send's time (`record` with an explicit refresh), or
  key the record by `(key, attempt)`.

### E5-I-2 (Info): the H3 ask-site test is a string count; it passes on the regressions it is meant to catch

- **Where:** `autobuyer/canister-rs/tests/audit_nft_r4.rs:1011-1057` (`r4_every_h3_ask_is_keyed_per_logical_outflow`). It counts
  `outflow_check(&` in seven named files, and checks that each listed key's text occurs somewhere in its file.
- **What it does not catch** (`e5_i2_…`, the test's body re-run verbatim on mutated sources; each passes):
  - (a) `mint_key`'s body keyed per backlog again: the call sites are unchanged, so it passes. This is E4-H-1 itself.
  - (b) The call site asks under `mint:deposit:{tx}` while the listed line survives as a comment.
  - (c) A new `crate::canister::outflow_check(&…)` in `looper.rs`, `delivery.rs` or any new file: only 11 files are read.
  - (d) A new site written `outflow_check(key, …)` without `&`.
- **Complete today:** my own grep of `src/` (every `outflow_check` / `outflow_record` and every `OUTFLOW.with`) finds exactly the 17
  listed sites. They are 5 in `ethflow.rs`, 1 in `convflow.rs`, 5 in `frogflow.rs`, and 1 each in `handinflow.rs`, `nftnet.rs` and
  `redeem.rs`, plus 3 in `saleflow.rs`. The only other callers are the trait adapters (`ethnet.rs`, `frognet.rs`, `custodynet.rs`)
  and `canister::outflow_check` itself.
- **Fix:**
  - Scan every file under `src/` (`std::fs::read_dir` at test time).
  - Match `outflow_check(` with any argument.
  - Tie each listed key to the line before its call.
  - Pin `mint_key`'s format string, and pin the advance's `n` expression.

### E5-I-3 (Info): the round-4 fixture leaves out `Inbox.payouts`

`tests/fixtures/eth-state-round4.hex` (`round4_state()`, `audit_nft_r4.rs:1476-1499`) holds every round-4 field except
`inbox.payouts`, which is `None`. So the services' IDLs are never tested on a `PayoutClaim` (the `.did`'s `payouts : opt vec
PayoutClaim`). The `.did` itself decodes one: `didcheck5` "round 4's fixture state with a payout claim in the inbox: ok". Fix: put one
`PayoutClaim` in `round4_state()` and re-run the monitor's and ops' decodes.

## What holds (checked, no finding)

**Every H3 ask site** was judged for its key (one per logical outflow?) and for C3-H-1 / E4-H-1's shape (can a waiting ask re-ask
under a new key?):

| Site | Key | One per logical outflow? A waiting ask re-keyed? |
| --- | --- | --- |
| `ethflow.rs:2107` stray return | `return:eth:{tx}:{token}` | Yes. `Return` never trips. |
| `ethflow.rs:2211`/`:2230` delivery | `deliver:{wrapper}:{ticket}` | Yes. A ticket leaves `Resolved` only at `deliverBy + REFUND_GRACE` (≥ 7 d), so no item yields two tickets in a window. A ticket to us or to nobody is marked before the ask. Bounded as E5-L-2 says. |
| `ethflow.rs:2920`, `:3098` mints | `mint:eth:{col}:{n}` | Yes (E4-H-1 above). |
| `ethflow.rs:2980`/`:2983` advance | `advance:eth:{col}:{n}` (`committed()` count) | Yes. `n` moves only on an advance out or a revert (C3-H-1). |
| `convflow.rs:212` conversion | `buy:frog:{frog}` | Yes. A dropped conversion re-opens under the same frog. |
| `frogflow.rs:1038` frog buy value | `buy:frog:{id}` | Yes. |
| `frogflow.rs:1648` Bitcoin return | `return:{outpoint}` | Yes. `Return` never trips. |
| `frogflow.rs:1792` floor claim | `claim:{want}` | Yes. Admitted to custody before it asks. |
| `frogflow.rs:1909` Bitcoin deposit mint | `mint:{outpoint}` (the old E4-H-1 shape) | Bounded. Only `Ready` deposits ask, and `Ready` means admitted to the wrapper's 3-item custody. In a window, ≤ 3 minted + ≤ 3 waiting = 6 < 9. |
| `frogflow.rs:1981` bought frog mint | `mint:frog:{id}` | Yes. In custody. |
| `handinflow.rs:109` hand-in reward | `claim:handin:{board}:{id}` | Yes. Only for a hand-in we bought through. |
| `nftnet.rs:315` operator fill | `fill:{order_hash}` | Yes. Asked at signing. |
| `redeem.rs:754` Bitcoin delivery | `ticket:{n}` | Yes. |
| `saleflow.rs:269`, `:455`, `:520` | `listing:{ticket}`, `mint:sale:{id}`, `sale:{id}` | Yes. |

**Upgrade safety** (`r5_e_upgrade.rs`, `didcheck5`, `regen_1eb5788.output.txt`):

- **New saved fields:**
  - `EthState.home_floor`, `pin_floors`, `credited`; `Inbox.payouts`; `EthBuy.late`; `Watch.solver`; `HomeTx.used_since`;
    `SaleScan.bad`, `skipped`. Each is `Option` + `#[serde(default)]`, and Candid reads a missing `opt` field as `null`.
  - `LateOrder`, `PayoutClaim` and `PinFloor(s)` are new records reached only through an `opt`.
  - `PayoutTx` and `EthDeposit.solver` are not saved: they have no `Serialize`.
  - No enum case was added.
- **The `.did`:**
  - It mirrors every new field (`solver : opt blob`, `late : opt LateOrder`, `used_since : opt nat64`, `payouts : opt vec
    PayoutClaim`, `home_floor`, `pin_floors`, `credited`, `bad`, `skipped`).
  - `candid_parser::service_compatible(new, 2735dac)` is `Ok(())`. The new method `eth_credit_payout` and the added `opt` fields are
    subtypes.
  - The round-4 fixture, the round-2 fixture in two of its historical encodings, and the 1eb5788 state's Ethereum side all decode with
    the `.did` with no `opt` lost.
- **Old states:** `4949da0`, `a37f54b` and `1eb5788` restore with `lost_opts` = []. The 1eb5788 state keeps its 19 buys (every phase),
  12 deliveries, 9 deposits, the pending `Step::Sent`, `Watch.pin`, `pend`, `rescan` and `judged`.
  - The `4949da0` and `a37f54b` fixtures are byte-identical to review4's, which those builds wrote.
  - **`saved-1eb5788-rich.hex` is genuine:** `fixes-round4-poc/dump_state_1eb5788.rs` on a `git archive` of `2735dac` writes the same
    32,198 hex characters.
  - The fixture covers every round-3 field. It does not hold a non-empty `Outflow` with the old mint keys, but those keys cannot
    collide with the new shape (checked above).
- **The re-encoded round-2 fixture hides nothing:** `audit_nft_r2.rs:1112` compares bytes, so any new `opt` field changes the type
  table and forces a re-encode.
  - The fixture's bytes as `24fba50`, `fbc9174`, `fb54018` (= `2735dac`) and `HEAD` wrote them all decode into round 4's `EthState`
    with nothing lost.
  - `fb54018`'s and `HEAD`'s decode to the same value. The earlier two differ in content only because `fbc9174` added a sweep to
    `round2_state()`.
- **Probe cost:** review4's largest state plus every round-4 field at its most:
  - `bad` 8 and `skipped` 6,300 per collection;
  - a `LateOrder` with a watch on each of 3,000 buys;
  - 8 payouts and 1,000 credited;
  - 5.44 MB of stable memory, probed in 0.61 s native (release), against 0.44 s for 4 MiB in round 4. That is well inside the
    upgrade's instruction limit.
- **Semantics of an old state:**
  - `pin_floors` absent starts at nothing, and the first tick sets it.
  - An old pending `HomeTx` has no `used_since` and starts the 1 h clock when first seen used.
  - The next-mint key counts the old state's minted buys and deposits. That is 6 for the fixture.

**Also checked:**

- `home_act_paused` signs only `Sent`, `Record` and `Mark`.
- `send_home` refuses everything else while paused (`:2700`).
- A `Mark` reached while paused can only have come from `MarkAfter` at finality or from before the pause.

## PoCs (`review5/poc/E/`)

- `build_harness.py`: builds `tests/r5_e.rs` in a private copy from the CURRENT `tests/audit_nft_r4.rs`.
  - It takes lines 1–750 (the fake world) and the E4-H-1 helpers.
  - It adds five switches: `mint_revert`, `mint_log`, `hidden`, `item_of_err`, and the estimate's refusal of a held token.
  - It appends `r5_e.tests.rs`. Nothing else in the fake changes.
- `r5_e.tests.rs` (5 tests, all **pass**; output in `r5_e.output.txt`, the per-tick `  | ` lines removed):
  - `holds_e4_h1_nine_deposits_and_two_purchases_…` (what holds);
  - `e5_i1_…`;
  - `e5_i2_…` (the H3-site test's body verbatim, on mutated sources);
  - `e5_l1_…` with its control;
  - `e5_l2_…`.
- `r5_e_upgrade.rs` (3 tests, **pass**; `r5_e_upgrade.output.txt`, release): the four encodings of the round-2 fixture, the 1eb5788
  state, and the probe cost. It needs `tests/fixtures/r5/eth-state-round2.{24fba50,fbc9174,fb54018}.hex` from `git show`.
- `didcheck5.main.rs` + `didcheck5.output.txt`: `service_compatible` against `2735dac`'s `.did`, and the fixtures and states decoded
  with the `.did`.
- `regen_1eb5788.output.txt`: the fixer's dumper on a `git archive` of `2735dac` writes `saved-1eb5788-rich.hex` byte for byte.
- `review4_e_rerun.output.txt`: review4's `audit4_nft_e.tests.rs` re-run by its own recipe on `1dc2a61`.
  - `e4_h1_…` **fails on its attack assertion** (mint asks 4, not frozen).
  - The controls and `e4_l1_…` pass, as they assert `exit_fits` on the launch numbers.
  - `e4_m1_…` passes for the harness reason `fixes-round4-poc/README.md` gives. C4-H-1 is part C's.
