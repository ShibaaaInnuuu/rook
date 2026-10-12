# A1: rook_core floor module and money core

| | |
|---|---|
| Scope | `solana/programs/rook_core/src`: `instructions/floor.rs` (init_module, post_floor, mint, burn, convert, advance, settle_advance, item_arrived, return_advance, snapshot_cap, fund_reserve, skim), `state/floor.rs`, `math.rs`, `units.rs`, `params.rs`, `clock.rs`, `events.rs`, `error.rs`, the lamport/token helpers in `seeds.rs`, and the floor-related reads of `state/registry.rs` (`keeper_down`, `live_poster`, `is_converter`) |
| Commit | `f2c24e0` (branch `golive-2026-10-08`) |
| Reference | `src/floor/FloorModule.sol` and `src/floor/PosterSource.sol` at tag `robinhood-stop-2026-10-09` (identical to the working tree), `docs/v9/rook-core-as-built.md`, `docs/v9/solana-migration.md` |
| Method | Threat model; line-by-line review of every handler and every `Module` function against the account, arithmetic, economic, time and state-machine checklist; a diff of each function's semantics against the Solidity; LiteSVM proofs and measurements in `solana/programs/rook_core/tests/audit_a1_core.rs` (new file, no existing source changed) |
| Result | No Critical or High. 1 Medium, 2 Low, 5 Informational. Q15 measured: not worse than the plan states. |

## Threat model

**Assets.** The lamports in each `ModuleVault` (`["module_vault", module]`), split by the books into `backing` (convertible
tier), `reserve` (reserve tier), `surplus` and `revenue`. The wX supply and its two backings: items (`held − open_tickets`) and
lamports (`Wrapper.eth_backed`). Invariants: `vault.lamports − rent ≥ backing + reserve + surplus + revenue`, and
`supply = (held − open_tickets)·10⁹ + eth_backed`.

**Actors and trust.**

| Actor | Can call | Trust |
|---|---|---|
| Anyone | `mint`, `burn`, `snapshot_cap`, `fund_reserve`, `skim`; SPL transfers/burns of their own wX; sending lamports or wX to any account | untrusted |
| Holder | `burn` (their wX), `redeem` (A2) | untrusted |
| Poster: the canister (EthNft) or the registry keeper (Inscriptions) | `post_floor` | trusted for the price, bounded by ±`max_move_bps`/h and ±20 %/day after the first post |
| Custodian: the canister | `advance`, `settle_advance`, `item_arrived`, `return_advance` | trusted, bounded by `max_item_cost`, one open advance, H11 |
| Converter: rook_backing's `["converter"]` PDA | `convert` (lamports from a payer signer) | trusted program; units go to a coin vault (A3) |
| Owner (Squads) | `init_module`, `set_paused`, `unpause_poster` | trusted |
| Guardian | `pause_keeper`, `pause_poster` | trusted, may only stop things |
| Malicious CPI caller | any instruction through CPI | untrusted; signer checks are the only gate |

Assumptions: `Clock::unix_timestamp` is monotonic within a fork and not attacker-controlled (stake-weighted, bounded drift);
the SPL Token program is the only token program; parameters are fixed at `init_module` and validated.

## Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| A1-01 | `pause_poster` is a silent no-op on an Inscriptions listing | Medium | `poc_a1_01_pausing_an_inscriptions_poster_stops_its_floor`: FAILS on f2c24e0 (finding confirmed) |
| A1-02 | Anyone overwrites the item-cap snapshot downward | Low | `demo_a1_02_anyone_lowers_the_snapshot_cap`: passes (behaviour shown) |
| A1-03 | The hourly mint and burn buckets are shared and griefable | Low | reproduction below (inherited design) |
| A1-04 | `price()` subtracts fill timestamps unchecked | Informational | none (needs a backwards clock) |
| A1-05 | Stray wX in `module_wx` satisfies `NoWrapped` and cannot be recovered; stray lamports and outside burns are unaccounted | Informational | `demo_a1_05_a_stray_public_unit_satisfies_no_wrapped`: passes |
| A1-06 | An escaped `skim` takes revenue and surplus while an advance is open | Informational | none (inherited) |
| A1-07 | `item_arrived` does not refuse while an advance is open | Informational | none (custodian trusted; inherited) |
| A1-08 | `convert` has no price protection beyond the caller's `min_units` | Informational | none (cross-ref A3) |
| Q15 | Floor-post front-running (accepted risk): measured | not worse than stated | `measure_a1_q15_front_running_a_floor_post`: passes |

### A1-01: `pause_poster` is a silent no-op on an Inscriptions listing

- **Severity:** Medium (an incident control that reports success and does nothing; loss bounded by the floor's caps).
- **Location:** `src/state/registry.rs:136-141` (`Listing::keeper_down`), `src/instructions/registry.rs:293-298` (`handle_pause_poster`).
- **Description.** `keeper_down` (the floor's `_keeperDown()`) reads `poster_paused` only for `ListingKind::EthNft`; for
  `Inscriptions` it is `reg.live_keeper().is_none()`. `handle_pause_poster` accepts any listing kind, sets `poster_paused = true`
  and emits `PosterPausedSet { paused: true }`. On an Inscriptions listing nothing changes: `live_poster` still names the
  keeper, `post_floor` is accepted, and `mint`/`convert`/`advance` still price at the keeper's word. The plan (§ "Guardian",
  solana-migration.md:127) lists `pause_poster(collection)` as the guardian's per-collection switch; the EVM had no such switch
  for Inscriptions (PosterSource fronted NFT modules only), so the Solana port keeps the EVM capability but adds an instruction
  that silently does not work for one kind.
- **Impact.** During an incident on one Inscriptions collection (a wrong frog floor from the keeper service), the guardian or
  monitor pauses that collection, sees success, and the floor keeps moving ±5 %/h with public mints and conversions at it.
  The only working stop is `pause_keeper`, which stops every Inscriptions collection and drand proving.
- **Exploit scenario.** The keeper service's frog feed misreads a floor and posts high. The guardian calls `pause_poster` on
  the frog listing. Bots keep minting at the stale-high mint price (losing) or, if the misread is low, mint low and burn after
  the correction; the guardian believes the collection is stopped.
- **PoC.** `poc_a1_01_pausing_an_inscriptions_poster_stops_its_floor` lists an Inscriptions collection, has the keeper post,
  pauses its poster as the guardian, then shows `post_floor` and a public `mint` both accepted. It asserts they are refused,
  and FAILS on f2c24e0 (`post accepted = true, public mint accepted = true`). The control
  `demo_a1_01_control_the_switch_works_on_an_ethnft_listing` passes (EthNft: `NotKeeper`, `Paused`).
- **Recommendation.** Make the switch effective for both kinds:
  `ListingKind::Inscriptions { .. } => self.poster_paused || reg.live_keeper().is_none()`. `live_poster` follows, since it calls
  `keeper_down`. Alternatively refuse `pause_poster` on an Inscriptions listing (`WrongKind`) so the guardian knows to use
  `pause_keeper`. The first is strictly a protection and matches the per-collection intent of D-M-1.

### A1-02: Anyone overwrites the item-cap snapshot downward

- **Severity:** Low (inherited from `FloorModule.snapshotCap`; it costs the canister, not holders).
- **Location:** `src/state/floor.rs:574-580` (`snapshot_cap`), `src/instructions/floor.rs:350-354`.
- **Description.** `snapshot_cap` is permissionless and always replaces `snap_cap`/`snap_at` with the current `item_cap`, even
  when an unexpired snapshot is higher. The snapshot exists so a purchase priced at a fresh cap can be recorded after the
  floor falls (`item_arrived` within `SNAPSHOT_TTL`).
- **Impact.** After the canister snapshots and buys on the pay-on-arrival path, any third party re-snapshots once the floor
  has dropped; `item_arrived(cost)` at the bought cost is refused (`OverItemCap`) and the canister must record a lower cost,
  absorbing the difference. Measured: a 1 h, −5 % move lowers the cap by 500 bps. The advance path (paid up front) is not
  affected, and the canister uses it.
- **PoC.** `demo_a1_02_anyone_lowers_the_snapshot_cap` passes: `snapshot 206000000000 overwritten by anyone with 195700000000
  (500 bps lower)`; `item_arrived(206e9)` is refused, `item_arrived(195.7e9)` accepted.
- **Recommendation.** Keep the higher of the two while the earlier one has not expired
  (`if now < snap_at + SNAPSHOT_TTL && snap_cap >= cap { return Ok(snap_cap) }`), or restrict `snapshot_cap` to the custodian.

### A1-03: The hourly mint and burn buckets are shared and griefable

- **Severity:** Low (inherited design; costs the griefer money every hour).
- **Location:** `src/state/floor.rs:431-434` (mint bucket), `466-469` (burn bucket), `382-385` (`level`).
- **Description.** `mint_per_hour` and `burn_per_hour` (5 units each by default) are one leaky bucket per module, shared by
  every user. One actor who mints 5 units at the start of each hour and burns them back fills both buckets and refuses every
  other mint and burn (`OverCap`) for the rest of the hour.
- **Impact.** Exits and entries for a collection are denied as long as the griefer pays. Cost per hour ≈ 5 units × floor ×
  (mint spread + burn spread): with the defaults and a 200 SOL floor, 5 × 200 × 0.30 % ≈ 3 SOL/h at the base spreads, more
  when the stress, volatility or staleness terms apply. No funds
  are lost; a holder waits. On Solana a griefer can also land the bucket-filling transaction first in a slot through a bundle.
- **Recommendation.** Accept for launch (the EVM had the same buckets), and have the monitor alert on one key filling a bucket
  in consecutive hours. Later: a per-key share of the bucket, or a burn bucket that refills faster for small burns.

### A1-04: `price()` subtracts fill timestamps unchecked

- **Severity:** Informational.
- **Location:** `src/state/floor.rs:254-255`.
- **Description.** `weighted += prev_price * (f.at - prev_at) as u128` casts an `i64` difference to `u128`. If a fill were ever
  stamped earlier than the one before it, the difference is negative, the cast yields ~2¹²⁸, the multiplication overflows and
  the program panics (`overflow-checks = true`). Every fresh-path `mint`, `burn` and `convert` would then fail for up to 24 h
  (until the fill leaves `FILL_WINDOW`). The final term already uses `elapsed()`.
- **Impact.** None today: fills are stamped with `Clock::unix_timestamp`, which the bank keeps non-decreasing.
- **Recommendation.** Use `elapsed(f.at, prev_at)` for both terms, for parity with the final term and robustness.

### A1-05: Stray value in the module cannot be recovered and stray wX satisfies `NoWrapped`

- **Severity:** Informational.
- **Location:** `src/state/floor.rs:617`, `646` (`module_wx >= ONE`); `reclaimStray` not ported (as-built §5).
- **Description.** Anyone can mint or transfer wX into `module_wx` (a plain SPL account of the wX mint, authority the module
  PDA). Such units never leave: nothing transfers out of `module_wx`, and only `settle_advance`/`item_arrived` burn from it.
  They also satisfy the `NoWrapped` check, so a custodian call made before its item was wrapped succeeds on a stray unit.
  Likewise lamports sent directly to a `ModuleVault` are outside the books forever, and wX burned through the SPL program
  directly (not `burn`) leaves `eth_backed` higher than the units in circulation, lowering every later burn's backing per
  unit slightly. None of this lets anyone take value; it only strands it.
- **PoC.** `demo_a1_05_a_stray_public_unit_satisfies_no_wrapped` passes: a public `mint` into `module_wx`, then
  `item_arrived` succeeds with `held` unchanged (the books and the ledger still hold).
- **Recommendation.** Have the canister check that the item's mint landed (it reads `held`/the item PDA) before `settle_advance`;
  optionally port `reclaimStray` (custodian: `module_wx` balance above what an open advance expects goes back to it).

### A1-06: An escaped `skim` takes revenue and surplus while an advance is open

- **Severity:** Informational (inherited, `FloorModule.skim`).
- **Location:** `src/state/floor.rs:684-698`.
- **Description.** Once `escaped()`, `skim` sends all revenue and surplus to the treasury. If an advance is open, a later
  `return_advance` with less than its basis then has no revenue or surplus to cover the shortfall, and the convertible holders
  carry the whole loss. An advance can only start while fresh, so this needs an advance open for more than 7 days + 3 h.
- **Recommendation.** Keep `advance_basis` out of the escaped skim (`keep = if esc { 0 } else { target }` plus the open
  basis), or note it in the canister's runbook.

### A1-07: `item_arrived` does not refuse while an advance is open

- **Severity:** Informational (custodian trusted; identical to `itemArrived`).
- **Location:** `src/state/floor.rs:612-624`.
- **Description.** With an advance open and the advanced item wrapped into `module_wx`, `item_arrived(cost)` pays a second time
  for that item (from another unit's backing), leaving the advance open. Only a faulty custodian can do this.
- **Recommendation.** `require!(self.transit_units == 0, AdvanceOpen)` in `item_arrived`, a cheap guard against a canister bug.

### A1-08: `convert` has no price protection beyond `min_units`

- **Severity:** Informational (cross-reference for A3).
- **Location:** `src/state/floor.rs:509-521`; caller `rook_backing/src/instructions/back.rs:164-208`.
- **Description.** `convert` prices at `gradPrice = ⌈max(price, upTarget)·(1+conv_bps)⌉`, where `upTarget` may sit up to 20 %
  above the day anchor while `accepted` lags, and it applies none of the mint-side limits (hourly bucket, public cap, stress,
  volatility, staleness or low-confidence spreads), as `_tryFill` did not. Its only slippage bound is `min_units`, which
  rook_backing's `back` takes from whoever cranks it. A cranker who also holds wX can choose the moment of conversion (a high
  `upTarget`), so the coin vault receives fewer units and the pooled backing per unit rises for existing holders.
- **Recommendation.** In rook_backing, derive `min_units` on-chain (for example from `accepted` and `conv_bps` with a tolerance)
  rather than from the cranker. No change needed in rook_core.

### Q15 (accepted risk): floor-post front-running, measured

The plan states: a bot mints before a post and burns after, gaining at most `max_move` 5 %/h less 2 × 15 bps, at most 5 units/h
each way, at most 20 %/day. Measured with the default parameters (`measure_a1_q15_front_running_a_floor_post`):

| Scenario | Result |
|---|---|
| (a) Atomic sandwich: mint, +1.67 % post, burn at once | **loses 127 bps**: the post sets `vol_bps` to the move it made and `vol_weight_bps` = 100 % charges all of it to the burn spread |
| (b) Mint before a post lifting the target 0.8 → 1.0 floor, hold 4 h while `accepted` climbs to the day-band top (0.96), burn; pool minted at 0.8 | **loses 59 bps**: each burn is capped at the tier's backing per unit, which is the pool's own basis |
| (c) Same, pool minted earlier at 1.0 (floor fell, then rebounds) | **gains 1183 bps** on 5 units over 4 h (≈ 3 %/h), taken from the earlier holders by dilution of their backing per unit |

So the risk is not worse than stated, and narrower: an atomic front-run is unprofitable; the gain needs a multi-hour hold, is
bounded by 5 %/h and the 20 % day band, by 5 units/h and the public cap, and exists only when holders' basis is above the new
floor. Burns never lower the backing per unit of the remaining holders (each tier releases exactly its share), so the loss is
dilution at mint time.

## Checked and found sound

**Accounts (every handler).**
- `registry`: `seeds = ["registry"], bump = registry.bump` everywhere it is read; `listing`: `Account<Listing>` (owner and
  discriminator), bound by `module.listing == listing.key()`; `module`: `Account<Module>`; `module_vault`, `wrapper`, `wx_mint`,
  `module_wx`: `address =` the module's or wrapper's stored keys; `treasury`: PDA with stored bump. `init_module`: owner signer,
  `wrapper.listing == listing`, `wx_mint == wrapper.wx_mint`, all three PDAs `init` with canonical bumps (pre-funding a PDA
  does not block Anchor's `init`); one module per listing.
- Signers: poster = `live_poster` (the EthNft poster or the live keeper, none while paused); custodian = `listing.custodian`;
  converter in `Registry.converters[..count]`; burn's `owner` signs and owns `owner_wx` (`token::authority = owner`, so no
  delegate path); payers sign their system transfers.
- Token accounts: `dest`/`owner_wx` constrained to `token::mint = wx_mint`; SPL Token only (`Program<Token>`); the wX mint has
  no freeze authority. `module_wx` authority is the module PDA, so only `wx_burn_module` moves its units.
- Duplicate mutable accounts: the vault cannot sign, so no payer can be it; `burn`'s `to` equal to the vault leaves the paid
  lamports in the vault (the burner loses, the books stay covered); `to` equal to the module or a token account only credits
  lamports. No `remaining_accounts` in scope.
- Lamport moves out of program-owned accounts: `pay_out` debits only `ModuleVault`/`Treasury` (owned by rook_core), keeps the
  source at its rent-exempt minimum, and checks the credit; inflows use system transfers from signers.

**Arithmetic.** `mul_div`/`mul_div_ceil` use `u128` intermediates, refuse division by zero and results above `u64`; every
rounding is against the caller (mint units ⌊⌋ at a ⌈⌉ unit price; burn tier prices ⌊⌋, paid ⌊⌋, released ⌊⌋ with the last unit
taking the dust; `gradPrice` ⌈⌉; spreads' added terms ⌈⌉). All `BPS − x` terms are bounded by `validate_params` (identical to
the Solidity constructor). Plain `+`/`-` in `mint`, `burn` and `release` panic on overflow (`overflow-checks = true` in the
release profile), never wrap. `split`'s dust rule (`r_l == 0 || r_u == 0` → all convertible) keeps a reserve unit from being
unbacked; a crafted tiny mint skews a tier by < 1 lamport. Units at 10⁹ per item lose no meaningful precision at any realistic
floor (a 0.01 SOL floor gives 100 units per lamport).

**Economics.**
- At any instant, mint base `max(price, upTarget)` ≥ `price` ≥ burn base `min(price, burnTarget)`, and both spreads are ≥ 0, so
  a same-instant round trip never pays.
- Every burn pays at most its tier's backing per unit and releases exactly that share; the rest goes to surplus (reserve tier)
  or revenue (convertible tier). No burn is paid from surplus or revenue. Remaining holders' backing per unit never falls on a
  burn.
- Paused poster: each tier pays `min(backing, pause_floor)·(1 − base)`, the pause floor capped by the fill band's low edge and
  the 12–24 h mint lows (A3-M1, A4-L1 ported exactly). Escaped (`FLOOR_TTL + STALE_ESCAPE` without a good post): backing less
  the base spread. Stale and not paused: refused. Low confidence: not fresh. All as `_tierPrices`.
- `mint`: fresh and poster up; public cap on `net_units + units − r_units`; `max_eth_backed`; hourly bucket. `convert`: fresh,
  poster up, listing not paused, `max_eth_backed`, `min_units` (EVM `_tryFill` plus the `ListingPaused` and `min_units` checks).
- Advance: one at a time (`transit_units == 0`), `amount ≤ max_item_cost` (cap or unexpired snapshot, and per-unit backing +
  revenue + surplus), H11 ring of 32 ≥ `max_items_per_day` (read before overwrite at `m = 32`). Settle: requires an open
  advance, `cost ≤ advanced`, burns the module's unit, `eth_backed −= ONE`; no lamports. Return: basis back to backing, a
  shortfall from revenue then surplus, the rest a shared loss; double settle/return and settle without advance refused
  (`NoAdvance`).
- `live ≥ reserve_units` holds under every instruction (reserve burned first; items taken only from `net ≥ ONE`; transit leaves
  live unchanged on settle), so no division by a negative or empty tier.
- Invariants checked after every money instruction in the suite and in `measure_a1_books_ledger_and_compute_units`:
  `vault − rent ≥ books` (mint/convert/fund_reserve/return add exactly what they book; burn/advance/item_arrived/skim book −paid
  and pay paid) and `supply = (held − open)·10⁹ + eth_backed` (mint/convert +units both sides; burn −units both; settle and
  item_arrived −ONE both). `treasury::distribute` also books and pays the same `r`.
- Treasury weight: `volume_7d` counts mint and burn lamports, so wash trading raises a collection's share of reserve
  distributions; the share lands in surplus, which no burn pays from, at a cost of both spreads per round trip (EVM parity).

**Time.** One clock (`unix_timestamp`, non-decreasing, drift-bounded) replaces the EVM's two; `elapsed` floors negatives at 0.
TTL (3 h), stale grace, the 1-hour cap on a post's step, the day anchor (reset to the previous accepted floor after 24 h), the
12 h mint-low buckets, `SNAPSHOT_TTL`, H11 and the leaky buckets match the Solidity. First post: unbounded (trusted poster), sets
the day anchor; `last_good_at == 0` never escapes, but nothing can be minted before a good post either.

**State machine.** `init` once per listing; no close; no setter for parameters; paused/keeper-paused interactions identical to
`_keeperDown` (EthNft: registry pause, poster pause, or no poster; Inscriptions: see A1-01). Mints are not stopped by
`set_paused` (EVM parity: `setPaused` stops wrapper item mints and, here, conversions).

**Divergences from the Solidity reviewed and accepted:** no graduation queue (`convert` converts or refuses); converted units
go to `dest` instead of the module; rentals, floats, `itemSold`, the loan, `fundLiquidity`, `reclaimStray`, `RESUME_GRACE`
(only `holdsRefunds` used it) dropped; the custodian is the payee; the H11 ring is bounded at 32. None weakens a protection
except the missing `reclaimStray` (A1-05, informational).

**Compute.** Measured (LiteSVM, SBPF v2): `post_floor` 14 867 CU, `mint` 35 779, `mint` with 8 fills in the price walk
36 439, `burn` 35 474, `convert` 35 473, `advance` 18 440, `settle_advance` 32 093. All far below 200 000.

## Test results

Built with `cargo build-sbf --arch v2` into an isolated target directory.

| Suite | Result |
|---|---|
| `cargo test -p rook_core --lib` | 7 passed |
| `cargo test -p rook_core --test rook_core` (existing LiteSVM suite) | 70 passed |
| `cargo test -p rook_core --test audit_a1_core` | 5 passed, 1 failed: `poc_a1_01_pausing_an_inscriptions_poster_stops_its_floor` (expected while A1-01 stands) |

`audit_a1_core.rs`: `poc_a1_01_pausing_an_inscriptions_poster_stops_its_floor` (FAILS: A1-01),
`demo_a1_01_control_the_switch_works_on_an_ethnft_listing`, `demo_a1_02_anyone_lowers_the_snapshot_cap`,
`demo_a1_05_a_stray_public_unit_satisfies_no_wrapped`, `measure_a1_q15_front_running_a_floor_post`,
`measure_a1_books_ledger_and_compute_units` (all pass).

## Verification (independent, 2026-10-10)

Method: I re-read every cited location at `f2c24e0` (the working tree's `solana/` matches it), rebuilt rook_core with
`cargo build-sbf --arch v2` and ran `cargo test -p rook_core --test audit_a1_core`. The result was 5 passed and 1 failed
(`poc_a1_01_…`), with the same numbers as the report: snapshot 206e9 → 195.7e9, Q15 −127 / −59 / +1183 bps, and the same CU
figures. I also checked how the canister calls these instructions (`autobuyer/canister-rs/src/{solix,frogsol}.rs`).

| ID | Verdict | Final severity | Reason |
|---|---|---|---|
| A1-01 | CONFIRMED | Medium | `keeper_down` ignores `poster_paused` for `Inscriptions` (`state/registry.rs:136-141`), and the PoC fails as stated. The impact is worse than written: the fallback, `pause_keeper`, also sets `reg.keeper_paused`, which `keeper_down` reads for **every EthNft listing**. So the only working stop for Bitcoin Frogs halts all 12 collections and drand proving. |
| A1-02 | CONFIRMED-ADJUSTED | Informational | The behaviour is real, but no caller is hurt. The canister never calls `snapshot_cap` (no use in `canister-rs/src`; frogs go through `item_arrived` without a snapshot), and `cap_or_snapshot` takes `max(live cap, unexpired snapshot)`, so an overwrite never pushes the limit below the live cap. |
| A1-03 | CONFIRMED | Low | One module-wide leaky bucket (`state/floor.rs:431-434, 466-469`). The griefer pays both spreads every hour. Inherited from the EVM. |
| A1-04 | CONFIRMED | Informational | `(f.at - prev_at) as u128` is reached only after `f.at > start`, and `prev_at` is the previous fill. It cannot go negative while the clock is monotonic. |
| A1-05 | CONFIRMED | Informational | `module_wx >= ONE` is the only `NoWrapped` test, so a stray unit satisfies it (demo passes). Value is stranded, not taken. |
| A1-06 | CONFIRMED | Informational | `skim` with `esc` zeroes `keep` and the surplus while an advance basis is open (`state/floor.rs:684-698`). |
| A1-07 | CONFIRMED | Informational | `item_arrived` never reads `transit_units`. A faulty custodian is required. |
| A1-08 | CONFIRMED | Informational | Duplicate of A3-06 (rated Low there). `convert` applies only `priced()`, `max_eth_backed` and `min_units`. |
| Q15 | CONFIRMED | accepted risk | The measurements reproduce exactly. Note that case (c) is real dilution of earlier holders, about 3 %/h on 5 units, which is within the plan's 5 %/h statement. |

**A1-02 adjustment.** This becomes relevant only if the canister starts to snapshot before buying on the pay-on-arrival
path. If it does, adopt the "keep the higher unexpired snapshot" fix at the same time.

**A1-01 fix note.** `ListingKind::Inscriptions { .. } => self.poster_paused || reg.live_keeper().is_none()` is the right shape.
`unpause_poster` (owner) already handles both kinds, and `prove_round` reads `live_keeper` directly, so drand is not stopped by a
collection pause.

No new issue found in A1's scope.

## Fixes (2026-10-10)

Paths under `solana/programs/rook_core/`. Built `cargo build-sbf --arch v2`; `cargo test -p rook_core`: 8 lib + **6/6 A1** +
7/7 A2 + 71/71 LiteSVM (before: 7 + 5/6 + 4/6 + 70).

| ID | Status | Fix | Where | Test |
|---|---|---|---|---|
| A1-01 | **Fixed** | `keeper_down` for `Inscriptions` is `self.poster_paused \|\| reg.live_keeper().is_none()`; `live_poster` follows. `prove_round` reads `live_keeper` and is not stopped by a collection pause. | `src/state/registry.rs:141` | `poc_a1_01_pausing_an_inscriptions_poster_stops_its_floor` now **passes** (post `NotKeeper`, mint `Paused`); `demo_a1_01_control_…` unchanged |
| A1-04 | **Fixed** | every fill interval in `price()` uses `elapsed(f.at, prev_at)`, as the final term | `src/state/floor.rs:255` | the floor suite and the canister vectors (`vectors::the_canisters_vectors_are_what_this_crate_computes`) unchanged |
| A1-08 | **Fixed in rook_backing** (with A3-06) | `back` derives its bound from the module's accepted floor: `max(min_units, 90 % × fee / (accepted × (1 + conv_bps)))` | `programs/rook_backing/src/instructions/back.rs:110-123, 195` | `fixed_a3_06_back_bounds_the_price_by_the_accepted_floor` |
| A1-02 | Not changed (Informational after verification: the canister never snapshots) | — | — | — |
| A1-03 | Not changed (accepted, inherited; monitor alert) | — | — | — |
| A1-05, A1-06, A1-07 | Not changed (Informational) | — | — | — |

Not done here: the canister's own copy of the rule (`autobuyer/canister-rs/src/solread.rs` `Listing::keeper_down`) still reads
the keeper switch only for Inscriptions. It decides only whether the canister attempts a post, so a mismatch costs a refused
transaction, not money; align it when the canister is fixed.
