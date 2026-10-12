# B3: the pool desk against DLMM, the raffle snapshot, the operator and guardian surface, the launch pins

| | |
|---|---|
| Scope | `autobuyer/canister-rs/src`: `pooldesk/{plan,run,build,step,state,api}.rs` against `solix2/dlmm.rs` (`price_q64`, `bin_for_price`, `quote_swap`, `position_amounts`, `claimable_fees`) and the landed fixtures `tests/fixtures/solix2/dlmm.json`; `raffle_books.rs`, `raffle.rs` (`build_with`, `winner`), `books/raffle.rs`, the raffle round of `cranks.rs`; the surface: `cranks.rs` (treasury setters, `pause_*`, `resume`, `cranks_set_off`), `pooldesk/api.rs`, `coins/api.rs` (`coins_pin`, `coins_exclude_from_supply`, `coins_set_off`), `desk/api.rs` operator methods, the guard test `every_operator_method_that_acts_refuses_while_paused` (`canister.rs:2630`) and its `EXEMPT` list; `examples/launch_pins.rs` with `docs/v9/ceremony-canister-only.md` |
| Commit | `e4d4d34` (branch `golive-2026-10-08`, the B0 fixes included) |
| Reference | `docs/v9/canister-only-solana.md` §5.3, §5.4, §9.3, §10.4, §17; `docs/v9/spikes/s4-wx-pools.md` §2.4, §5; `docs/v9/raffle-design.md`; `docs/v9/audit-2026-10/b0-review-2026-10-11.md` (F1–F12 known, not re-reported) |
| Method | Threat model; line-by-line review of every decision in `plan.rs` and `run::decide` against DLMM's arithmetic as `dlmm.rs` reproduces it and the fixtures show it (bin 586 ≈ 18.61 SOL, 0.5 SOL a bid bin, 0.0296 wX an ask bin); the raffle round followed from publish to claim with the holder's levers in mind; every operator and guardian method read for what it can start, stop or move; the launch pins read against `coins::open::register` and `books::value_params`. Independent verification of the B0 fixes F3, F4, F5, F8. A passing PoC that exhibits each High and Medium in `tests/audit_b3_desk_raffle.rs` (new file; no `src` file changed) |
| Result | No Critical. **1 High** (the operator lifts a guardian's collection pause at once), **4 Medium** (all in the pool desk's decisions), 6 Low, 1 Informational. F3, F4, F5, F8 verified: all four hold (F3 with a gap the fix did not cover, B3-03). |

## Threat model

**Assets.** The desk's capital at `K_desk[i]`: native SOL (the float for mint legs), wSOL (the deposit side, the fees, the arb
proceeds), wX inventory, and its one DLMM position; all of it treasury money (§5.2). The module's SOL at `M[i]` and the wX supply the
desk's mints and burns move through the books. The treasury's income path (§5.3: pool fees and arb profit above target depth). The
raffle pot and prize (one whole wX at a `K_tre` token account), and the fairness of the draw. The guardian's power to stop things and
the 24 h the design gives holders before a stop is lifted (§10.4). `K_main`'s float (the rent the openings and bin arrays sink).

**Actors and trust.**

| Actor | Can do | Trust |
|---|---|---|
| Anyone on Solana | trade the desk's DLMM pair (move its active bin), add their own liquidity to it, create its bin arrays, sandwich the desk's swaps, hold or move a collection's coins, burn their own coins | untrusted |
| Coin holder | hold, split, transfer coins (to any address, the canister's included); burn their own coins at any time | untrusted |
| Operator (the named principal, or a controller) | every `only_operator` method: the pins, the switches, the treasury setters (24 h delayed), `desk_pause_collection`, `resume` | trusted, bounded by code; a stolen key is the case §10.4's delays exist for |
| Guardians (through the timelock, a controller) | `pause_collection`, `pause_desk`, `pause_all_mints`, `set_paused(true)`; k of them may resume the global pause | trusted, stop-only |
| Meteora | `set_pair_status` on any permissionless pair | trusted (S4 §6, accepted) |
| drand relays | serve beacons; two must agree and the BLS signature verifies | untrusted individually |
| The canister's own code | reads chain state through two agreeing providers, decides, signs | the subject |

Assumptions: the DLMM program behaves as `lb_clmm` 0.12.0's source and the S4 spike showed (`go_to_a_bin` refuses when liquidity lies
between the active bin and the target; `initialize_bin_array` of an existing array fails; a swap fails on `min_amount_out`); the
books' mint and burn rules (A1, B0) are correct; the fake chain of the tests does not run DLMM, so the PoCs are over the desk's
decisions, not DLMM's effects.

## Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| B3-01 | The operator lifts a guardian's `pause_collection` (and an incident halt) at once through `desk_pause_collection(i, false)`: the 24 h resume delay is bypassable | High | `poc_b3_01_the_operator_lifts_a_guardian_collection_pause_at_once` (passes: exhibits it) |
| B3-02 | The up-arb's `min_out` has no floor at its mint cost: a sandwich inside the 0.3 % allowance lands the arb at a loss | Medium | `poc_b3_02_the_up_arbs_min_out_is_under_its_mint_cost` |
| B3-03 | No wSOL → native path: pool fees, up-arb proceeds and withdrawn liquidity are stranded as wSOL; nothing reaches the treasury and the mint legs starve | Medium | `poc_b3_03_the_desks_wsol_never_reaches_the_treasury_nor_a_mint_leg` |
| B3-04 | After a past-the-band withdrawal the desk only retries `go_to_a_bin`; a stranger's liquidity in between wedges it (no arb, no deposit) | Medium | `poc_b3_04_a_withdrawn_desk_only_tries_the_move_and_never_arbs_a_stranger_pool` |
| B3-05 | A floor two or three bin arrays from the position drops the position's arrays from the read: the position reads as empty and step B re-creates an existing array, ahead of everything, for ever | Medium | `poc_b3_05_a_far_floor_drops_the_positions_arrays_and_recreates_an_existing_one` |
| B3-06 | A landed raffle claim is booked at settle whatever link is open; a Collection-scoped link failing afterwards reverts it to a phantom prize | Low | none (needs a failing redemption link; reasoning below) |
| B3-07 | Anyone blocks a collection's raffle snapshot by burning dust of a coin at the (public) snapshot time; the snapshot hour is predictable to the minute | Low | existing `no_backed_coin_is_a_soft_refusal_and_a_straddled_read_is_hard` shows the refusal |
| B3-08 | `can_open` ignores the native float and `ADD_MIN_LAMPORTS`: the pair opens (≈ 0.18 SOL of rent sunk by `K_main`) while the desk cannot deposit | Low | none (pure arithmetic below) |
| B3-09 | The inventory rule burns all wX, not the excess; a burn above the hourly bucket loops every 30 min | Low | none |
| B3-10 | A pin whose curve DBC refuses wedges the unlock order after the mint leg paid: no unpin, the index is taken | Low | none |
| B3-11 | `coins_exclude_from_supply` moves redemption value between holders at once, with no delay | Low | none |
| B3-12 | Smaller deviations from §5.4 and the ceremony: the size cap is of the side taken, not the scarcer; the step budget is shared; `sides` counts strangers' liquidity; the up-arb draws on the public mint bucket; a pin's custody is not checked against the canister's own address; `K_nft`, `K_cfg` and refunded intents are not raffle-excluded | Informational | none |

### B3-01: The operator lifts a guardian's collection pause at once

**Fixed (2026-10-11, the same day, after the report):** `cranks::collection_pause(coll, paused, now)` is the one path: a pause sets the flag and clears any scheduled resume; a lift schedules `Resume::Collection` `RESUME_DELAY_SECS` ahead (as `resume` does) and never writes the flag. `desk_pause_collection` and the guardian's `pause_collection` both call it. Regression: `fixed_b3_01_the_operator_cannot_lift_a_collection_pause_at_once`.

- **Severity:** High (a stated canister-enforced control, "unpausing takes a 24 h delay", does not hold; the actor is a compromised or
  rogue operator key, which is exactly what the guardian model guards against; the money is whatever the pause protected, typically
  public mints at a wrong floor).
- **Location:** `src/desk/api.rs:351-357` (`desk_pause_collection`), `src/cranks.rs:1009-1014` (`pause_collection`), `src/cranks.rs:351-377`
  (`apply`: the delayed resume), `src/cranks.rs:1036-1045` (`resume`), `src/chainlink/ledger.rs:385` (a halt pauses the same flag).
- **Description.** §10.4 gives the guardians three stop-only methods, "effective at once", and says "Unpausing takes a 24 h
  canister-enforced delay." `pause_collection(i)` sets `books.collections[i].paused = true` and clears any scheduled resume;
  `resume(Collection, i)` schedules `set_paused(i, false)` for `now + RESUME_DELAY_SECS`, applied by the cranks. But the W4 operator
  method `desk_pause_collection(i, paused)` writes the caller's value into the same flag at once, with only `only_operator` and
  `not_paused` in front of it. An incident halt (`Ledger::halt`) pauses the same flag, and the runbook's way to lift it after
  `desk_clear_halt` is this immediate method, so it cannot simply be removed: the guardian's pause and the incident's pause are
  indistinguishable in the books.
- **Exploit scenario.** The floor feed misreads and posts low; bots mint cheap. A guardian pauses the collection through the timelock.
  The operator key (stolen, or pressed) calls `desk_pause_collection(i, false)`: the mint is open again in the same minute, no resume
  is scheduled, nothing waits 24 h, and the guardians' only recourse is to pause again (the operator lifts again) or the global
  `set_paused(true)`, which stops every collection and every line.
- **PoC.** `poc_b3_01_…`: what the two methods do to the books (`set_paused(0, true)` closes `quote_mint`; `set_paused(0, false)`
  reopens it at once; `cranks` has no resume scheduled), and the source of both methods (the operator's writes the caller's value and
  never touches `resumes`; the guardian's clears `resumes`). Passes on `e4d4d34`.
- **Suggested fix.** Separate the two pauses (`paused_by_guardian` and `paused_by_incident`, the mint closed while either holds), or
  keep one flag and make `desk_pause_collection(i, false)` schedule `Resume::Collection` instead of clearing the flag. Either way the
  only transition from paused to open is `cranks::apply` after `RESUME_DELAY_SECS`. Extend `set_paused_lets_the_operator_stop_but_only_a_controller_resume`'s
  idea: a test that every writer of `set_paused(_, false)` and `set_keeper_paused(false)` is `cranks::apply`. (`pause_desk` and
  `pause_all_mints` have no immediate lift: `pool_set_off` toggles the whole-desk switch, not `DeskBook.off`.)

### B3-02: The up-arb's `min_out` is under its mint cost

**Fixed (2026-10-11, the same day, after the report):** `plan::up_min_out` sets the up-arb's `min_out` to the quote less 0.3 % but never under `lamports + ARB_MIN_PROFIT` (the quote clears that bound whenever the profit floor is met, so no shrinking is needed); `run::decide` keeps the same floor after scaling `min_out` to the units actually minted. No signed up-arb can land under its mint cost. The down-arb is unchanged. Regression: `fixed_b3_02_the_up_arbs_min_out_is_never_under_its_mint_cost`.

- **Severity:** Medium (a bounded loss of desk money per arb, up to 0.3 % of the proceeds less the expected profit; 12 arbs an hour
  per collection; the trigger is an organic buying wave, which the attacker need not pay for).
- **Location:** `src/pooldesk/plan.rs:207` (`min_out: q.amount_out - q.amount_out * SLIPPAGE_BPS / 10_000`), `:205` (the profit floor),
  `:24-28` (`ARB_MIN_PROFIT` 0.0012 SOL, `SLIPPAGE_BPS` 30); `src/pooldesk/run.rs:326` (`min_out` scaled to the minted units).
- **Description.** §5.4: "minimum_amount_out = the quoted amount less 0.3 %. If a sandwich or race moves the pool, the swap fails and
  costs only the fee." That is true only while 0.3 % of the proceeds is under the expected profit. The arb is sized for the most
  profit within the bounds, and the profit floor is a flat 0.0012 SOL, so any arb with more than 0.4 SOL of proceeds and a thin
  margin has `min_out < lamports`: the mint leg pays `lamports` into `M[i]`, the swap may return `min_out`, and the transaction
  still lands. A sandwich (sell wX first, buy back after) inside the allowance takes the difference. The down-arb has the same
  shape (`:225`), bounded by the burn quote instead.
- **PoC.** `poc_b3_02_…`: the landed pair and bins (bids ×20, ≈ 10 SOL in the active bin), the mint price 0.1 % under the threshold.
  `plan::arb` plans 0.54 wX minted for 9.990 SOL, quoted 10.000 SOL out (profit 0.0100 SOL), `min_out` 9.970 SOL: a fill at
  `min_out` loses 0.0200 SOL. Passes.
- **Suggested fix.** `min_out = max(quote × (1 − 0.3 %), lamports + ARB_MIN_PROFIT)`; if that exceeds the quote, shrink the size (the
  optimiser can take the clamp into `profit_of`). The same clamp for the down-arb against `burn_of(min_out)`.

### B3-03: No wSOL → native path

**Fixed (2026-10-11, the same day, after the report):** a new step, `PoolStep::Unwrap { coll, lamports }` (`build::unwrap_tx`: SPL `CloseAccount` of the desk's wSOL account to `K_desk[i]`, signed by the desk; all of the wSOL and the account's rent arrive native). `decide` plans it (once the wSOL is at least `CLAIM_MIN_LAMPORTS`) before a sweep that the native above the float cannot cover, before an up-arb while the native float is spent, and before a deposit's mint leg that is short of native. The builders that pay into the wSOL account (`claim_tx`, `withdraw_tx`, the in-place re-centre) now re-create it first with `create_ata_idempotent` (the add, the up-arb and the move re-centre already did). The did, the monitor's `PoolStep` IDL and the fake chain's `CloseAccount` (a native account closes with its balance, as SPL does) are updated. Regression: `fixed_b3_03_the_desks_wsol_is_unwrapped_and_reaches_the_treasury`.

- **Severity:** Medium (treasury money stranded in the desk; the treasury's stated income from pools never arrives; the desk's
  ask side cannot be funded once the native float is spent; no loss to holders).
- **Location:** `src/pooldesk/build.rs:127-134` (`claim_tx` pays `desk_y()`, the wSOL account; `sweep_tx` is a native transfer),
  `:67-75` and `src/pooldesk/run.rs:434-453` (wrapping, the only direction), `src/pooldesk/run.rs:384` (the sweep bounded by native
  above the float), `:410` and `plan.rs:195` (mint legs from native only), `src/pooldesk/step.rs:6-35` (no unwrap step).
- **Description.** Everything the pool pays the desk is wSOL: fees (`claim_fee2`, Y = wSOL by the preset), the up-arb's proceeds, a
  withdrawal's SOL side. Everything the desk pays the module is native: the mint leg of an up-arb or a top-up. Review F3 made the
  sweep native (right: the treasury pays native) but left no way for wSOL to become native. So: (1) at target depth
  `excess_for_treasury` counts the wSOL as the treasury's, the sweep takes `min(excess, native − float − rent)` = 0, and `wants_fees`
  is false, so the wSOL sits for ever; (2) in a rising market the up-arbs and top-ups spend the native float on mint legs and receive
  wSOL, until `by_native` is 0 and the ask side cannot be minted though the desk is rich in wSOL. The only native inflows are the
  coins' fee cash (its burned half, `desk_lamports`) and the down-arb's burns.
- **PoC.** `poc_b3_03_…`: a position at target (the fixture's amounts ×20), native = float + rent + 1,000 lamports, wSOL 30 SOL.
  `excess_for_treasury` ≥ 30 SOL; `decide` returns nothing; `wants_fees` is false; the builders know `sync_native` and no
  `close_account`. Passes.
- **Suggested fix.** An `Unwrap { lamports }` step: `close_account` of the desk's wSOL account to the desk (all of it; the next add
  re-wraps what it needs), planned before the sweep and before a mint leg short of native. Or let the sweep pay wSOL into a treasury
  wSOL account and have the treasury crank unwrap it.

### B3-04: A withdrawn desk only retries the move

**Fixed (2026-10-11, the same day, after the report):** on an empty position `decide` plans `Move` only while the step is ready and has been refused fewer than `run::MOVE_TRIES` (3) times. Otherwise it skips the re-centre and goes on to the arb (F), the burn (G) and, once the pool is within the re-centre trigger of the floor, the deposit around the active bin (H, `plan_add`, which moves the position's range). A landed withdrawal clears the move's count. Regression: `fixed_b3_04_a_withdrawn_desk_arbs_a_stranger_pool_and_gives_up_the_move`.

- **Severity:** Medium (liveness: any Solana user with 0.07 SOL of bin-array rent and dust liquidity wedges a collection's desk after
  its next past-the-band re-centre; the desk's capital sits idle, its fee share keeps arriving and parks; no loss).
- **Location:** `src/pooldesk/run.rs:279-290` (D: an empty position moves or deposits, and returns), `:292-296` (E: `Past` withdraws
  before any arb), `src/solix2/dlmm.rs:255-260` (`go_to_a_bin`: "refused when liquidity lies between").
- **Description.** S4 R2 is "the arb buys the pool to its top bin, then remove, `go_to_a_bin`, add". The code withdraws as soon as
  `recenter` says `Past`, arb or not, and then, the position being empty, only ever plans `Move` while `|active − fb| > 1`. A
  stranger's position between the pool's price and the floor's bin (a permissionless pair: anyone adds) makes the move fail at
  simulation; the step waits 30 minutes and is planned again, and nothing else is tried: `decide` returns from D before the arb
  (F) and the deposit (H). The arb that would trade through that liquidity and bring the price to the floor is exactly what the
  design wants here, and it is profitable.
- **PoC.** `poc_b3_04_…`: the floor 28 bins over the pool's price, the position's shares zeroed, the fixture's asks as the stranger's
  liquidity. `decide` plans `Move { target: 614 }`; with that step waiting (as `sign` files a simulation refusal) `decide` plans
  nothing; after the wait, `Move` again; meanwhile `plan::arb` finds a down-arb. Passes.
- **Suggested fix.** On an empty position let F run (the arb needs no position), and let D's deposit go around the active bin the
  arb reaches instead of insisting on `fb`; keep `Move` for the case of no liquidity in between (the first attempt), and after a
  refusal fall through.

### B3-05: A far floor drops the position's arrays and re-creates an existing one

**Fixed (2026-10-11, the same day, after the report):** `run::read` always asks for the position's own bin arrays and the floor band's (at most two each); the floor's ±2 bands fill what is left of `ARRAYS_READ`. Step B creates only those position and band arrays the read did not return, never the stretch between them. A position the read cannot value (`position_amounts` errs) makes `decide` return an error, so nothing is signed and the next pass reads again; it is no longer taken as empty. Regression: `fixed_b3_05_a_far_floor_reads_the_positions_arrays_and_creates_only_the_bands`.

- **Severity:** Medium (liveness: the desk is wedged with no operator path out but an upgrade; a floor that doubled or halved since
  the last re-centre is plausible after a `pause_desk`, a wedge of B3-04, or weeks off; the dropped read also mis-states the
  position's value in `pool_desks`).
- **Location:** `src/pooldesk/run.rs:131-150` (`arrays_wanted`: at most `ARRAYS_READ` = 4, the nearest to the floor's array),
  `:276` (`position_amounts(..).unwrap_or((0, 0))`: an unreadable position is "empty"), `:262-268` (B: every array from the position
  to the floor's band not in `r.arrays`, two a pass), `:195-202` (`record` keeps the last good `pos_x`/`pos_y` silently).
- **Description.** The read wants the position's arrays and the floor band's (±2h = 77 bins, up to three arrays), truncated to the
  four nearest the floor's. A position over two arrays with a floor whose band covers three (B3's PoC: [587, 633] and bin 805)
  loses its lower array. Then `position_amounts` errs, `decide` takes the position as empty (D: `Move`, which DLMM refuses on a
  live position), but before that B wants arrays 8..=11 less those read, signs `initialize_bin_array(8)`, which exists, fails at
  simulation, waits 30 minutes and is planned again ahead of every other step. With a one-array position the same happens at three
  arrays' distance.
- **PoC.** `poc_b3_05_…`: the fake chain with the pair, array 8 and the position `position_rebalance_past` planted, the floor at bin
  805. `run::read` returns no array 8; `position_amounts` errs; `decide` plans `OpenArrays [8, 9]`; the tick signs it. Passes.
- **Suggested fix.** Always read the position's own arrays (they are at most two) and fill the rest with the floor's; treat an
  unreadable position as not empty (`Err` → skip the pass with a wait); in B create only the arrays the next add will touch (the
  band's), never the stretch in between.

### B3-06: A landed raffle claim is booked under an open Collection-scoped link

**Fixed (2026-10-11, by the B1-06 fix):** the landed claim's `claim_raffle` goes through `desk::book_outcome`, which queues the write while any link of the collection is open and applies it once the link is decided (after an undo's restore), so no snapshot restore wipes it. Regression: `fixed_b1_06_…`.

- **Severity:** Low (needs an NFT redemption or ticket-refund link on the same collection, open when the claim lands and failing
  afterwards; the loss is one wX of bookkeeping and a month of a wedged raffle, no money).
- **Location:** `src/cranks.rs:844-857` (the claim booked at settle, F4), `src/chainlink/ledger.rs:97-114` (`Scope::Collection`
  snapshots the whole `Collection`, raffle included), `src/cranks.rs:565-567` (F1's `collection_busy` guards the round, not the
  settle).
- **Description.** F1 made every non-link writer wait on an open link, but the claim's `claim_raffle` runs when the transfer lands,
  whatever is open. A `Scope::Collection` link (a redemption, a refund) open at that pass and later `Failed`/`Undone` restores the
  collection's snapshot: phase `Drawn`, `prize = ONE`, while the wX left the treasury's account and `claims` no longer holds the
  winner. The next claim attempt is none (no `claims` entry), the round expires after 30 days with a phantom prize, the next round
  is committed against it, and its winner's transfer fails on chain for lack of wX (hourly retries). Module-scoped links (mints,
  burns, the prize) do not snapshot the raffle and are safe.
- **Suggested fix.** Book the claim under the F1 rule too: if `collection_busy`, keep the landed claim in a "paid, to book" list and
  book it at the first free pass (the chain fact cannot be undone, the books must follow it).

### B3-07: Anyone blocks a raffle snapshot; the snapshot hour is public

**Fixed (2026-10-11, the same day, after the report):** when the holders' sum is not the supply read before them, that mint's supply is read again after the holders and taken when it is their sum (a burn between the reads; nothing mints); a sum that still differs is the hard refusal it was (an incomplete listing). The snapshot hour stays public (recorded, not changed).

- **Severity:** Low (a denial of the weekly draw at dust cost; no money).
- **Location:** `src/raffle.rs:221-225` (the supply-sum check, a hard refusal), `src/raffle_books.rs:131-146` (the reads: supplies,
  then holders, over ≤ 3,000 slots), `src/cranks.rs:553` (retried in `RAFFLE_EVERY` = 1 h), `src/cranks.rs:949-955`
  (`treasury_status.next` and `raffle_status.committed_at` are public).
- **Description.** The check `Σ accounts == supply` is right for a mint, but any holder may `Burn` their own coins at any time, and
  the attempt's time is readable (`next["raffle"]`, `committed_at + 7 d`). A dust burn landing between the supply read and the
  `getProgramAccounts` makes the snapshot hard-refused, and the next attempt is an hour later, equally public. A DBC coin's mint
  authority is revoked at launch, so `Σ accounts > supply` can only mean a burn in between (the burner lost weight): it is safe to
  accept. The public timing also lets a holder buy coins minutes before the snapshot and sell after (the raffle weighs a point in
  time, as `raffle-design.md` accepts); a jitter on the hour would blunt the cheap version.
- **Suggested fix.** Read the supply again after the holders and accept `Σ accounts ≥ supply_after` when the mint has no mint
  authority; retry a hard refusal after minutes, not an hour; add a random offset to the snapshot's hour.

### B3-08: `can_open` ignores the float and the least deposit

**Fixed (2026-10-11, the same day, after the report):** `can_open` asks for the opening's rent, the native float and the key's rent before the two 0.5 SOL sides (the wX side counted at the floor as what it saves on the mint leg), so the first deposit is the designed size (`nothing_is_signed_before_the_desk_can_fund_an_opening`, `tests/pooldesk.rs`).

- **Severity:** Low (≈ 0.18 SOL of `K_main`'s rent sunk early; the pool exists empty until more fees arrive; no loss).
- **Location:** `src/pooldesk/plan.rs:235-239`, `src/pooldesk/run.rs:278` (`budget` = native + wSOL − float − rent), `:286`
  (`ADD_MIN_LAMPORTS` 0.2 SOL), `src/pooldesk/build.rs:43-57` (`K_main` is every funder).
- **Description.** `can_open` asks for 0.5 SOL a side plus the opening rent (1.236 SOL in all, 0.735 SOL of it SOL, the rest wX at the
  floor); the deposit asks for `native + wSOL ≥ 1.0009 + 0.2` SOL and ignores wX. A desk with 0.74 SOL and 2 SOL of fee wX opens its
  pair and arrays (0.0346 + 0.0714 × 1–2, sunk, paid by `K_main` although §5.2 books the rent as desk money) and then reports "too
  little to deposit" until 0.46 SOL more arrives; with no wX it opens at 1.24 SOL and deposits 0.12 SOL a side, a quarter of
  `OPEN_MIN_SIDE_LAMPORTS`. The arb cannot run on the empty position (B3-04's shape).
- **Suggested fix.** `can_open`: `sol ≥ OPEN_RENT + NATIVE_FLOAT + KEY_RENT + 2 × OPEN_MIN_SIDE` (the wX side counted only as what
  it saves on the mint leg), so the first deposit is the designed 0.5 SOL a side.

### B3-09: The inventory rule burns everything; a burn over the bucket loops

**Fixed (2026-10-11, the same day, after the report):** G burns the excess over the inventory share (`value × INVENTORY_BURN_BPS` in wX at the floor; all of it once the target is met), bounded by the module's hourly `burn_room`, and only from `MIN_BURN_UNITS`.

- **Severity:** Low.
- **Location:** `src/pooldesk/run.rs:346-366` (G: `units = r.wx`), `src/books/floor.rs:500-504` (the hourly burn bucket).
- **Description.** §5.4: "desk wX above 60 % of the position's value → burn the excess." G burns all of `r.wx` once the rule or the
  target trips, and the next top-up mints wX back (both spreads paid to the module, a transaction each). A burn of all `r.wx` above
  the hourly bucket's room (fee wX accumulate without bound; the public shares the bucket) is refused by the books, waits 30 min
  and is retried unchanged.
- **Suggested fix.** Burn `min(excess, burn_room)`.

### B3-10: A pin whose curve DBC refuses wedges the unlock order

**Fixed (2026-10-11, the same day, after the report):** `register` validates the curve as DBC does (strictly rising sqrt prices above the start, positive liquidity, the whole curve raising at least `t_wx × 10⁹`), and a config step refused by simulation `CONFIG_REFUSALS` (3) times refuses its collection so the unlock order moves on. Regression: `fixed_b3_10_a_curve_dbc_would_refuse_is_refused_at_the_pin` (`tests/coins_flow.rs`).

- **Severity:** Low (operator error with a tested generator; recoverable only by an upgrade).
- **Location:** `src/coins/open.rs:91-115` (`register` checks shape, not the curve against DBC's rules or `t_wx`), `:131-133`
  (`next_pinned` is the lowest index neither `Open` nor `Refused`), `:171-178` (a config step waits and is retried), no unpin method.
- **Description.** `register` accepts any non-empty curve of ≤ `MAX_CURVE_POINTS` with non-zero `t_wx` and start price. The mint
  step lands (≈ 0.02 SOL of `K_main`'s float), then `create_config` is refused at simulation for ever (a non-monotone curve, a
  threshold the curve cannot reach, a start price at the first point). The collection stays `Opening`, not `Refused`, so every later
  index waits behind it; the index and symbol are taken. `launch_pins` runs `register` and Meteora's own `designCurve`, so the
  shipped pins are sound; a hand-edited `.args` is not caught.
- **Suggested fix.** Validate the curve in `register` as DBC does (strictly increasing `sqrt_price` above the start, positive
  liquidity, the quote the curve raises ≥ `t_wx × 10⁹`), and give a config step that is refused N times a `Refused` stage so the
  order moves on; or a `coins_unpin(index)` for a collection that never opened.

### B3-11: `coins_exclude_from_supply` moves redemption value at once

**Fixed (2026-10-11, the same day, after the report):** `coins_exclude_from_supply` queues the change (`pending_exclusions`), applied by the coins' pass after `TREASURY_DELAY`; a new ask replaces the pending one; `coins_exclusions` shows both lists. Regression: `fixed_b3_11_a_supply_exclusion_waits_its_day` (`tests/coins_flow.rs`).

- **Severity:** Low (operator trust, but it is a fund lever without the delay the treasury setters have).
- **Location:** `src/coins/api.rs:147-159`, `src/coins/redeem.rs:72-88` (`supply − excluded` in the payout's denominator).
- **Description.** Excluding an account raises every other holder's payout `V·a/S` at once, and the excluded holder's coins, if
  redeemed later, find a vault already drained at the higher rate. Intended for pool vaults and lockers, it can equally exclude a
  large wallet. Every other operator knob on money waits 24 h.
- **Suggested fix.** Apply exclusions after `TREASURY_DELAY`, as `set_shares` does, and publish the pending list in `coins_log`.

### B3-12: Smaller deviations (Informational)

- **Size cap** (`plan.rs:196`, `:215`): 25 % of the side the arb takes (Y for an up-arb, X for a down-arb), where §5.4 says the
  pool's scarcer side; and `sides` (`:113-116`) sums every bin read, strangers' liquidity included, so the cap is of the pool, not the
  desk's position. Both only loosen a bound the profit search already respects.
- **The step budget** (`state.rs:92`, `run.rs:74-77`): `STEPS_PER_DAY` = 288 for the whole desk, which is one collection's 12 arbs an
  hour; `tick` signs one step a pass in index order (`run.rs:97-101`), so a busy low index starves a high one. Per-collection budgets.
- **The public mint bucket** (`run.rs:313`, `books/floor.rs:462-465`): the up-arb's mint leg draws on the collection's hourly
  `mint_per_hour`, shared with holders (A1-03's bucket). During a buying wave the direct mint closes while the desk refills the
  pool at the mint price: self-limiting, but the site should say why a mint is refused.
- **Pins and custody** (`examples/launch_pins.rs:87-94`, `ceremony-canister-only.md` §2): `--evm-custody` and `--btc-custody` are
  taken as given; the canister checks neither against its own `evm_address`/`custody_address`. A wrong EVM custody only breaks the
  destination check (`ledger.rs:336`); a wrong taproot prefix breaks every redemption destination of the frog collection
  (`ledger.rs:339-343`). `open::pass` could compare them to the derived addresses before listing.
- **Raffle exclusions** (`raffle_books.rs:64-83`): `KeyPath::PositionNft(n)` and `KeyPath::Config(i)` are on the curve and not
  excluded; an intent that was `Refunded`/`Expired` is not either. Coins sent there count as a holder the canister never claims
  for: dilution, no steering.
- **`value_params` from a stale floor** (`launch_pins.rs:83-86`): the caps are fixed at listing from `graduation-tiers.json`'s floor;
  a floor 10× off gives caps 10× off for good. The runbook's review of `pins.json` is the only check.

## Verification of the B0 fixes

| Fix | Verdict | What was checked at `e4d4d34` |
|---|---|---|
| F3 (pool sweeps pay native) | **Holds**, with a gap (B3-03) | `build::sweep_tx` is `transfer(K_desk → K_tre)` (`build.rs:132-134`); `decide` bounds it by `native − KEY_RENT − NATIVE_FLOAT` (`run.rs:384`); `apply_landed` books it with `books.receive` when it lands (`run.rs:571-575`), so the treasury's chain balance is never under its books. The desk's wSOL, which F3 says "stays its working capital", has no way back to native, so the sweep only ever reaches the coins' burn half and the down-arbs' burns. |
| F4 (a landed claim always clears) | **Holds** | `settle` removes the `claims` entry before booking (`cranks.rs:846`), books `claim_raffle` once, and files a refusal as the wait `claim_paid:<coll>:<round>` with the operator note (`:851-856`); a mismatched round is `NothingPending` and the same wait. A failed or expired transfer keeps the entry and is retried (`:865-883`). The line's `line_free` check (`:332`) prevents a second signing while one is out. |
| F5 (no expiry under an in-flight claim) | **Holds** | `claim_in_flight` reads the crank line's `pending` step (`cranks.rs:557-563`) and the round skips the collection (`:576-578`); `settle` runs before `raffles` in `tick` (`:321-327`), so a landed claim is booked before any expiry could see it. B3-06 is the one booking F5/F4 do not cover (a later undo of a Collection-scoped link). |
| F8 (a books refusal in `decide` is the step's wait) | **Holds** | ArbUp (`run.rs:333`), Burn (`:363`) and the top-up's mint (`:427`) file `s.wait(step.key(), e)` and fall through; D's `plan_add` returns `Ok(None)` and the tick moves to the next collection (`:102`); a wait is keyed by kind and collection (`step.rs:67-69`), so a refused arb does not block the claim or the sweep. |

## Checked and found correct

**DLMM arithmetic.** `price_q64` is DLMM's square-and-multiply on the inverted base (bin 586 = 18.61 SOL; `bin_for_price` inverts it,
nearest by difference); `half_width(50, 1,000)` = 19 and `trigger` = 14 are the S4 numbers; `quote_swap` walks the bins as
`swap_exact_in` (max-in with the fee excluded and rounded up, the partial fill's fee on the amount in, the per-bin volatility
accumulator from the references at `now`, capped; `complete` false past a missing bin or the pair's limits) and `arb` refuses an
incomplete quote; `position_amounts` and `claimable_fees` are per-bin and rounded down as the chain pays (B2 checked them against the
landed fixtures). `spot_amounts` funds every bin the same (`x0 = y0`), bounds the ask side by what is held plus the mint's room with
the 1 % margin, and the bid side by `y/(h+1)`; the rebalance's `x0` is the Y-value per ask bin as the SDK defines it.

**The desk's decisions.** `recenter` returns `No` within 14 bins of the centre, `Past` at or beyond an edge, `Inside` only once the
pool is within one bin of the floor and `WaitArb` otherwise (the arb first, as §5.4); the re-centre is one `rebalance_liquidity`
with the old range removed explicitly (S4: `None` would mean the active bin). The arb bounds: `ARBS_PER_HOUR` counted on landed arbs,
`inventory ≤ 10 % of backing at the floor` less what is held, the up-arb by the mint's room and the native above rent, the down-arb
by the wSOL and the burn quote (a burn over the bucket makes the quote `None`, so the search never picks it); the profit search is a
grid and a ternary refinement over a concave profit. The up-arb is one transaction (mint and swap, so no unsold leg), booked as a
`Mint` link whose `min_units` is 0.1 % under the plan and whose `min_out` follows the minted units; the down-arb's burn is a `Burn`
link paid from `M[i]` by `pre_sign`. Every landed step is re-read before the next (`record`), the native float is kept by every add
(`wrap ≤ budget/2`), and the desk's wX and SOL are the desk's (`K_desk[i]` signs for them, `K_main` only pays). The hourly mint
bucket is restored by a link's undo. `wants_fees` is false at target so `q_pool` stops. `under_target` and `excess_for_treasury` are
as §5.3 (both sides at target, the float kept). The pass skips a desk that is `off`, waits while the pool line is busy, and signs
at most one step. The one-pass read is one `getMultipleAccounts` with a bounded size.

**The raffle.** `build_with`: weights `⌊balance × wx(c) / circ(c)⌋` in u128, floored per coin and per wallet, zero weights dropped,
leaves in owner order with prefix sums; `circ` is the supply less every excluded account, so sending coins to an excluded key only
lowers the sender's weight and scales everyone else's alike; splitting a balance loses only the floors; the supply-sum check refuses
a read that straddles a mint; `MAX_EXCLUDED`, `MAX_LEAVES` and a u64 total are enforced. `exclusion` names the incinerator, the
system program and every off-curve owner (the DBC and DAMM v2 vaults by name); `canister_paths` adds `K_main`, `K_relay`, `K_fc`,
`K_tre`, every listed `K_coll[i]`/`K_desk[i]`, every known `K_coin[c]` and every live intent. `take` reads between two finalized
clocks ≤ 3,000 slots apart and commits the bucket's end. `winner` finds the leaf by `partition_point` over the prefix sums and
proves it with the sorted-pair tree; the books and the builder hash alike (`leaf_hash`, `ticket`). The round: the snapshot is
published before anything is committed; the commit needs a later finalized slot, a snapshot newer than the last and under two days
old; the drand round is pinned 20 rounds after the commit's, drawn only once two relays agree and the BLS signature verifies; the
ticket is keyed by the books' key and the round; the claim is a `TransferChecked` of ONE signed by `K_tre` to the winner's
associated account; an unclaimed prize stays for the next round; a committed round not drawn in a week is cancelled; a collection
with an open link is skipped (F1). A holder cannot steer: the holder set is fixed before the randomness exists.

**The surface.** `only_operator` is the named principal or a controller; `resume` needs `not_paused` so a globally paused canister
cannot schedule a lift, and asking again restarts the clock; `pause_desk` and `pause_all_mints` have no immediate lift; the
treasury setters are 24 h delayed (`set_shares` must sum to 10,000, the reserve cap and raffle bps bounded), only the first `set_ops`
is immediate (the ceremony); no method transfers SOL or wX by hand (`desk_sweep_unbooked` books, `desk_reconcile` reads,
`desk_add_collection` registers under `validate_params`). The global `set_paused(true)` idles the whole Solana timer
(`soltimer.rs:99-101`), so the exempt switches `pool_set_off`, `coins_set_off` and `cranks_set_off` cannot make anything sign while
paused, and the three guardian pauses only stop; the other exemptions are reads, the pause itself, `abandon` (ETH back to the vault)
and the ordinals providers' configuration. `coins_pin` refuses a taken index or symbol and bad shapes; `register` is what
`launch_pins` runs on every pin it writes, and `check_config`/`check_wx_mint` read the chain side back before a collection is
listed, so a config that landed is the pin's. `launch_pins`: `T_wX = max(3, ⌈tier / floor⌉)` with the tier in lamports, the curve is
Meteora's own `designCurve` for that threshold, `value_params` as §4.3, the custody by listing kind, one `.args` per index in the
unlock order.
