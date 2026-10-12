# B2: the Solana primitives of the canister-only build

| | |
|---|---|
| Scope | `autobuyer/canister-rs/src/solix2/` (`mod.rs`, `spl.rs`, `mpl.rs`, `dbc.rs`, `damm.rs`, `dlmm.rs`, `decode.rs`, `guard.rs`, `route.rs`); `solkey.rs` (the key map, `signer_paths`, `sign_with_keys`); `solhome.rs` (the four lines, `LineId::of`, `ready` / `turn` / `claim` / `release`, `sign_keys`, `tick_lines`, the incident path, `books_sink_view`) and `soltimer.rs`; `solnft.rs`, `solnftbuy.rs`, `solnftfloor.rs`, `solnftsend.rs`, `solnftm2.rs` |
| Commit | `e4d4d34` on `golive-2026-10-08` (2026-10-11) |
| Reference | `docs/v9/canister-only-solana.md` §1.1, §2–3, §5, §13.1 (W3); `docs/v9/solana-nfts-canister.md`; `docs/v9/spikes/s4-wx-pools.md` §5.2; the S5 notes at the top of `solnftm2.rs`; A4 (`a4-canister-sol-primitives.md`, baseline `f2c24e0`) for what was already audited |
| Method | The project's: threat model, then a line-by-line read of every file in scope against it; what A4 covered (`solkey`, `soltx`, `solsend`, `solrpc`, `solread`, `solix`) re-read ONLY where `git diff f2c24e0..e4d4d34` touched it (`solkey` +336: the key map and the multi-signer rule; `solsend` +265: `prepare_keys`, `payer_of`, the nonce burn, `open_squatted_nonce_ixs`, `statuses_at`; `soltx` +13: `allocate_with_seed`; `solrpc` +76: `statuses_at`, `accounts_slice`, `token_accounts`; `solread` +401: rook_core / rook_backing decoders for the keeper, programs no longer deployed under D-l, not in this scope). A PoC test for every finding that has one, PASSING by exhibiting the bug (`tests/audit_b2_primitives.rs`, pure; `solnft-svm/tests/audit_b2_judge.rs`, the marketplace programs' mainnet bytes in LiteSVM). No `src` file was changed. |
| Result | 0 Critical, 0 High, 1 Medium, 3 Low, 9 Informational |

## 1. Threat model

**Assets**

1. **`K_main`'s SOL** (the fee payer of every line) and **every derived key's holdings**: `K_coll[i]` (a collection's backing, the wX mint authority), `K_fc` (every DBC config's fee claimer and leftover receiver, the partner LP positions), `K_coin[c]`, `K_desk[i]` (the pool desk's inventory and its DLMM position), `K_tre`, `K_int[n]`.
2. **The wX mints** (§2): mint authority `K_coll[i]`, no freeze authority, 9 decimals; and **the metadata's update authority**.
3. **The DBC configs** (§3.1): what the canister trusts as Rook's (`check_config`): where the curve's fees, the migration fee, the surplus and the leftover go, and the threshold bound to the curve.
4. **The four send lines** (§13.1 W3): one transaction in flight each, their durable-nonce accounts, their queues, their incident state, and the timer that drives them.
5. **The Solana NFTs in custody** (`solana-nfts-canister.md`): membership (one-of-one, verified, nobody but the owner can move it), custody (held by `K_main`'s wallet with no delegate and no lock), what a purchase may cost (`limit`), what a delivery may do (one item, to the redeemer).
6. **Cycles** (every outcall the timer and the lines make).

**Actors**

| Actor | Capability | In scope |
|---|---|---|
| One malicious RPC provider | Any answer to any request; keeps every signed transaction | yes (agreement of ≥ 2 is the trust root, as A4) |
| Both providers colluding | Forge any agreed read | **no** |
| A hostile account owner | Any account data at any address the canister reads, **owned by them**: a Token/DBC/DAMM/DLMM/Tensor/mmm/M2 account's bytes are written only by that program | yes |
| A seller / pool owner on a marketplace | Lists, edits the price, cancels, delegates, freezes what the programs let them, between the canister's read at `finalized` and the landing | yes |
| Anyone on Solana | Transfers to any address, creates a DLMM pair or a DAMM pool at a derived address first, submits any public bytes, front-runs | yes |
| The marketplace APIs (the co-signed path the judge is written for) | Any transaction bytes handed to the judge | yes |
| A Meteora / Metaplex / marketplace program upgrade | Changes a program's checks or fees | noted (doc §8), not modelled |
| The operator | `sol_timer`, `sol_clear_incident` / `sol_clear_line_incident`, `sol_address_of`, the launch file's pins | trusted, foot-guns checked |
| A flow of the canister (Eth, Frog, Custody, Crank, Coin, Treasury, Desk, Pool) | Calls `ready` / `turn` / `sign_keys` / `release` with its own `owner` and `step` | checked for a step holding another's turn |

## 2. Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| B2-01 | The NFT judges pass any Compute Budget instruction unread: a `SetComputeUnitPrice` makes `K_main` pay a priority fee `limit` never covered | **Medium** | `poc_b2_01_every_nft_judge_passes_a_compute_unit_price_the_limit_does_not_cover` (pure); `poc_b2_01_the_judge_passes_a_compute_unit_price_and_the_canister_pays_ten_sol_over_its_limit` (LiteSVM: 10 SOL over the limit on Tensor's mainnet bytes) |
| B2-02 | `refuse_forbidden`'s "rename only" rule reads the last three bytes: an `UpdateMetadataAccountV2` that moves the update authority to a key ending in `0x00` passes | Low | `poc_b2_02_refuse_forbidden_passes_a_metadata_update_that_moves_the_update_authority` |
| B2-03 | A stale place in a line's queue keeps the timer at the busy cadence for ever (the 1 h pruning lives in `claim`, which `tick_lines` does not use) | Low | `poc_b2_03_a_stale_queue_entry_keeps_the_timer_at_the_busy_cadence_for_ever` |
| B2-04 | `check_config` leaves `migrated_pool_base_fee_mode`, `enable_first_swap_with_min_fee` and the migrated pool's market-cap fee scheduler unpinned | Low | `poc_b2_04_check_config_accepts_a_migrated_pool_fee_mode_rook_never_pinned` |
| B2-05 | `refuse_forbidden` is wired on the coins' and the pool desk's paths only; the money line and the mint/burn desk's driver never call it | Informational | none |
| B2-06 | A co-signed venue (mmm) cannot be signed: no primitive adds `K_main`'s signature to a partially signed transaction, so `best_ask`'s `cosigned` is dead today | Informational | none |
| B2-07 | `tick_lines` ends the pass on any log text containing "queue" and "full" | Informational | none |
| B2-08 | `sign_keys` does not check the turn; the `ready(owner)` / `sign_keys(step)` pairing is the caller's (every caller consistent today) | Informational | none |
| B2-09 | A pNFT's rule set is not allowlisted: any `ProgrammableConfig` rule set passes membership, and a later revision of it can veto the delivery | Informational | none |
| B2-10 | `OUTCOMES_KEPT`: past 16 outcomes on one line the oldest of any owner is dropped once no crank outcome is left | Informational | none |
| B2-11 | The timer dies on a trap BEFORE the first await of a run (none exists today); it lives only until the next upgrade's `arm_after_install` otherwise | Informational | none |
| B2-12 | `dlmm::position_amounts` and `claimable_fees` sum in unchecked `u128`; `PositionV2::decode` refuses a position wider than 70 bins (unreachable: the desk's band is 39) | Informational | none |
| B2-13 | `solread`'s +401 lines since A4 are rook_core / rook_backing decoders for a keeper whose programs are no longer deployed (D-l) | Informational | none |

---

### B2-01: The NFT judges pass any Compute Budget instruction unread (Medium)

**Fixed (2026-10-11, the same day, after the report):** `soltx::compute_budget_ok`: the three judges read every Compute Budget instruction and accept `SetComputeUnitLimit` (tag 2, 5 bytes) only; a unit price, a heap frame or a loaded-accounts bound is refused by name. Regression: `fixed_b2_01_every_nft_judge_refuses_a_compute_unit_price_and_passes_the_unit_limit (pure), fixed_b2_01_the_judge_refuses_a_compute_unit_price_and_the_canonical_purchase_lands_within_its_limit (LiteSVM)`.

**Files:**
- `src/solnftbuy.rs:612-615` (`judge`, J1: `if ix.program == COMPUTE_BUDGET_PROGRAM { continue }`)
- `src/solnftm2.rs:174-177` (`judge`, the same)
- `src/solnftsend.rs:449-451` (`judge_send`: `filter(|i| i.program != cb)` on both sides)
- `src/solnftbuy.rs:676-679` (J3: `all_in = spend + rent ≤ limit`, no fee term)
- `src/soltx.rs:223` (`compute_unit_price`: the builder exists, nothing in the canister sets it; A4-16)

**Description.** The three judges are written for "whoever built" the transaction (`solnftbuy.rs` header: "the canister, or a marketplace API whose transaction comes back co-signed"). Each skips every instruction of the Compute Budget program without reading it. `SetComputeUnitPrice(p)` is one: the fee payer pays `cu_limit × p / 10⁶` lamports of priority fee on top of the transaction. With `BUY_CU` = 400,000 and `p` = 25 × 10⁹ micro-lamports a unit that is 10 SOL; with `u64::MAX` it is whatever `K_main` holds (the transaction then fails for want of funds, but anything under the balance lands). J3's `limit` bounds the venue instruction's spend and the rent only.

**Scenario.** A marketplace API (the mmm path, the only reason the judge accepts foreign bytes) returns a co-signed transaction that carries `SetComputeUnitLimit(400_000)` and `SetComputeUnitPrice(25_000_000_000)`: the judge passes it, the purchase lands, `K_main` pays 10 SOL more than the limit it enforced. Today nothing feeds the judges a transaction the canister did not build (no caller outside the modules: `solnft*` is not yet wired into a flow, doc §9), so the finding is latent: the gate is wrong, not yet reachable. It is rated Medium for that reason (impact: the fee payer's whole balance; likelihood: the co-signed path, which the design intends).

**PoC.**
- `tests/audit_b2_primitives.rs::poc_b2_01_every_nft_judge_passes_a_compute_unit_price_the_limit_does_not_cover`: `solnftbuy::judge` (Tensor, legacy FFF), `judge_send` and `solnftm2::judge` (Mad Lads) all pass their canonical instructions prefixed with `compute_unit_price(u64::MAX)`, while a limit one lamport under the ask is still refused.
- `solnft-svm/tests/audit_b2_judge.rs::poc_b2_01_the_judge_passes_a_compute_unit_price_and_the_canister_pays_ten_sol_over_its_limit`: on Tensor Marketplace's mainnet bytes, the judged purchase (limit 10.202 SOL) lands and the payer is down 20.202 SOL: `10000000000 lamports over the limit (the priority fee 10000000000)`.

**Fix.** In all three judges, read every Compute Budget instruction: accept `SetComputeUnitLimit` (tag 2) only, or accept `SetComputeUnitPrice` (tag 3) with the fee `limit_units × price / 10⁶` added to J3's `all_in` (and a cap). Refuse `SetLoadedAccountsDataSizeLimit` and `RequestHeapFrame` too (no reason to carry them). The fix must make the pure PoC's three `expect`s fail.

---

### B2-02: `refuse_forbidden`'s "rename only" rule reads the tail (Low)

**Fixed (2026-10-11, the same day, after the report):** `mpl::is_rename_only` walks the Borsh argument (tag, `Some`, the three strings by their lengths) and requires exactly eight zero bytes after them (seller fee 0, no creators, collection or uses, and the three trailing options `None`); the guard uses it instead of the tail. Regression: `fixed_b2_02_refuse_forbidden_refuses_a_metadata_update_that_moves_the_update_authority`.

**Files:**
- `src/solix2/guard.rs:29-33` (`Some(&mpl::UPDATE_METADATA_ACCOUNT_V2) if i.data.len() >= 4 && i.data[i.data.len() - 3..] == [0, 0, 0] => {}`)
- `src/solix2/mpl.rs:21-27` (`update_metadata_v2`, the only builder)

**Description.** `UpdateMetadataAccountV2`'s Borsh is `Option<DataV2>`, `Option<Pubkey>` (update authority), `Option<bool>` (primary sale), `Option<bool>` (is_mutable). The guard means "the last three options are `None`" and checks that the last three BYTES are zero. With `update_authority = Some(k)` and the two bools `None`, the tail is `[k[31], 0, 0]`: for one key in 256 (`k[31] == 0`) the update passes as a rename and moves the wX metadata's update authority. (`primary_sale_happened` or `is_mutable` as `Some` cannot pass: their tag byte 1 lands in the tail.)

**Scenario.** The canister builds its own Metaplex instructions and never names an update authority, so the guard is defence in depth: it protects against a builder bug or a future caller, not an outside party. An attacker holding such a key is not in the picture. Low.

**PoC.** `poc_b2_02_refuse_forbidden_passes_a_metadata_update_that_moves_the_update_authority`: the crafted update with a key ending in `0x00` passes; the same with `0x01` is refused (the rule depends on the key, not the argument); the canister's own rename and `SetAuthority` behave as documented.

**Fix.** Parse the argument: after the `Option<DataV2>` (tag 0, or tag 1 then three Borsh strings, a `u16`, three option tags that must be 0 for wX's `DataV2`), the next byte must be 0, then 0, then 0, and nothing may follow. The fix must make the PoC's first `expect` fail.

---

### B2-03: A stale queue entry keeps the timer at the busy cadence (Low)

**Fixed (2026-10-11, the same day, after the report):** `tick_lines` prunes every line's queue by `TURN_SECS` (saving the line when it dropped something) before counting a non-empty queue as work. Regression: `fixed_b2_03_a_stale_queue_entry_is_dropped_by_the_timers_view_after_an_hour`.

**Files:**
- `src/solhome.rs:1055-1058` (`tick_lines`: `busy = … || !l.queue.is_empty()`)
- `src/solhome.rs:380` (`claim`: `queue.retain(… now − at ≤ TURN_SECS)`, the only pruning)
- `src/soltimer.rs:43-48` (`next_delay`: busy → 10 s, idle → 60 s), `:113-124` (`pass` runs the desk, the coins, the pool desk and the cranks on every run)

**Description.** A flow that asks for a busy line is queued once and keeps its place until it claims again (then served or pruned after `TURN_SECS` = 1 h) or releases. `tick_lines` counts a non-empty queue as work for the timer, with no age check. An owner that asked once and then has nothing to do, and whose code path does not release (every flow's path releases today; the money line's flows rely on `release_untaken_turn`, which only acts on a FREE line's HEAD entry), leaves the timer at 10 s instead of 60 s for as long as the entry stays: six times the desk / coins / pool / cranks passes, each with their reads. Cycles only; Low.

**PoC.** `poc_b2_03_a_stale_queue_entry_keeps_the_timer_at_the_busy_cadence_for_ever`: a money line with `queue = [(Eth, 0)]` and nothing out reports `busy` on every pass for 30 days (`next_delay` = `BUSY_SECS`), while a `claim` by anyone at the same `now` drops the entry at once.

**Fix.** In `tick_lines`, count only entries younger than `TURN_SECS` (or prune the queue there as `claim` does, and save it). The fix must make the PoC's `assert!(busy)` fail from the second day on.

---

### B2-04: `check_config` leaves three migrated-pool fields unpinned (Low)

**Fixed (2026-10-11, the same day, after the report):** `check_config` pins `migrated_pool_base_fee_mode == 0` and `enable_first_swap_with_min_fee == 0` with the migrated pool's other fee fields. The market-cap fee scheduler is not decoded (left as the report notes; the two decoded fields were the gap the PoC used). Regression: `fixed_b2_04_check_config_refuses_a_migrated_pool_fee_mode_rook_never_pinned`.
**Changed (2026-10-11, operator decision):** `enable_first_swap_with_min_fee` is a curve setting (the creator's launch buy at the minimum fee), not a migrated-pool one; Rook's config now turns it ON and `check_config` pins it to 1 in its own check (`dbc::pins::FIRST_SWAP_WITH_MIN_FEE`). The regression test refuses both a migrated-pool fee mode of 1 and the flag at 0.

**Files:**
- `src/solix2/guard.rs:99-137` (`check_config`)
- `src/solix2/decode.rs:218-219` (`migrated_pool_base_fee_mode` at 364, `enable_first_swap_with_min_fee` at 365: decoded, never checked); the market-cap fee scheduler (`migrated_pool_market_cap_fee_scheduler`, `dbc.rs:97`) is not decoded
- `src/solix2/dbc.rs:196-198` (`rook_wx_config` pins them to 0 / default / `false`)

**Description.** The pin covers the quote, the receivers, the fee schedule, the LP split, the supply, the threshold bound to the curve and the migrated pool's fee bps. The migrated DAMM v2 pool's base fee MODE (0 fee scheduler, 1 rate limiter), the first-swap-at-minimum-fee flag and the market-cap scheduler are not compared with what `rook_wx_config` creates. A config created by the ceremony (the launch file pins its address: `coins/open.rs:144`, `pin.config`) with one of these set is accepted as Rook's; its coins graduate into pools whose fee behaves differently from the design's 1 % (a rate limiter raises the fee with trade size; a scheduler decays it). No fee is routed elsewhere (the receivers and the LP split are pinned), so the harm is to traders and the arb loop's assumptions, not to custody. Low.

**PoC.** `poc_b2_04_check_config_accepts_a_migrated_pool_fee_mode_rook_never_pinned`: the recorded Rook config (`flow.json`) with bytes 364 and 365 set to 1 decodes to `(1, 1)` and passes `check_config` unchanged.

**Fix.** Pin `migrated_pool_base_fee_mode == 0`, `enable_first_swap_with_min_fee == 0`, and decode and pin the scheduler bytes (`number_of_period`, `sqrt_price_step_bps`, `scheduler_expiration_duration`, `reduction_factor` all 0) to the values `rook_wx_config` writes. The fix must make the PoC's final `expect` fail.

---

### B2-05: `refuse_forbidden` is on two of the four signing paths (Informational)

`guard.rs:17` says "Called on every message before any signature". It is called by `coins/link.rs:25,54` (`sign_plain`, `sign_linked`: the coins on the crank line, the pool desk on the pool line, the treasury's steps). `chainlink/driver.rs:109-114` (the mint/burn desk's line) and the money line's flows go to `solhome::sign_keys` → `solsend::prepare_keys` without it. Those paths build no `SetAuthority`, `Approve` or Metaplex update today. Moving the call into `solsend::prepare_keys` (or `sign_intent`, before the simulation) makes the header true for every signature.

### B2-06: a co-signed venue cannot be signed today (Informational)

`best_ask(.., cosigned)` admits mmm when "an API that co-signs is configured for that venue", and `mmm_fulfill_sell` names the pool's cosigner as a signer. `signer_paths` (`solkey.rs:259-266`) refuses any signer that is not a held key, and `sign_intent` compiles from instructions and assembles every signature itself: a transaction already carrying Magic Eden's notary signature has no way in. So `cosigned` is dead until a "add `K_main`'s signature to these bytes" primitive exists, which would need its own judge (B2-01 first). Nothing is wrong today; the design should say which.

### B2-07: the pass ends on a text heuristic (Informational)

`tick_lines` (`solhome.rs:1042`) stops the pass when a line's log text satisfies `is_queue_full` (`starts_with(QUEUE_FULL) || (contains("queue") && contains("full"))`). A provider error that happens to carry both words ("sol line: waits: …") ends the pass for the lines after it, for one tick (they go first next time). Harmless; a typed signal from `sol_tick` would be exact.

### B2-08: `sign_keys` trusts the caller's turn (Informational)

`sign_keys` (`solhome.rs:811`) checks `busy()` and the line of `step.owner()`, not that the caller claimed the turn, nor that `ready`'s `owner` was the step's owner. Every caller pairs them today (`coins/link.rs`: the same `owner` and a step of that owner; `chainlink/driver.rs:109-114`: `Desk` / `Op { owner: Desk }`; `solcrank.rs:1450`; `cranks.rs:801-805`). A `debug_assert!(LineId::of(owner) == LineId::of(step.owner()))` in `sign_plain` / `sign_linked`, or `sign_keys` taking the owner it was readied for, pins it.

### B2-09: any pNFT rule set passes membership (Informational)

`is_member` (`solnft.rs:659`) takes `md.programmable.flatten()` as the item's rule set, whatever it is; `FOUNDATION_RULE_SET` and `COMPATIBILITY_RULE_SET` are named but not an allowlist. A collection whose rule set (or its next revision; the rule set's authority can publish one at any time) denies the wallet-to-wallet transfer makes the delivery fail; custody is kept, the redeemer waits. Doc §8 records the risk for the two known sets; the launch file's 12 collections are checked there. An allowlist (or a pinned `rule_set_revision` in the token record, which `TransferV1` honours) would make the check the canister's.

### B2-10: outcomes past the bound drop the oldest of anyone (Informational)

`SolLine::outcome` (`solhome.rs:362-376`): past `OUTCOMES_KEPT` = 16, a crank's outcome goes first (its fate is re-read from the chain); when none is left, `drain(..extra)` drops the oldest outcome of ANY owner. On the crank line (Crank, Coin, Treasury) and the pool line, an owner that does not `take` for 16 landings loses its earliest outcome; the chain links reconcile from the chain (B1), the `Op` steps do not. A per-owner bound would be exact.

### B2-11: the timer's two edges (Informational)

`run` (`soltimer.rs:79-97`) arms a fallback before its first await, so a trap after an await leaves a timer armed and the `TickGuard` drops through the cleanup. A trap BEFORE the first await (in `arm`, `pass` up to `assert_home().await`) would roll back the fallback and leave the IC's global timer, zeroed before the handler runs, unarmed until the next `sol_timer(true)` or upgrade. No such trap exists today (`cfg()` is set after install; `hold_tick` returns `Err`); `a_run_arms_its_fallback_first_and_never_overlaps_a_tick` pins the order by source scan. Note it beside the test so a future first line of `pass` is written with it in mind.

### B2-12: DLMM arithmetic and the 70-bin cap (Informational)

`position_amounts` and `claimable_fees` (`dlmm.rs:660-719`) sum per-bin terms in `u128` with `+=`: on data the program wrote (`share ≤ liquidity_supply`) each term is ≤ a `u64`, so 70 terms cannot overflow; on inconsistent data the release build wraps. `checked_add` costs nothing. `PositionV2::decode` (`dlmm.rs:595`) reads at most 70 bins of a fixed 8,120-byte account and refuses any other length, so a position grown past 70 bins (`increase_position_length2`) is unreadable; the desk's band is `2 × 19 + 1` = 39 bins and its re-centre adds `[−h, h]` (`pooldesk/run.rs:271,301,444`), so it never grows past 39. If the band ever widens past 70, the desk cannot read its position.

### B2-13: `solread`'s growth since A4 (Informational)

The +401 lines in `solread.rs` since `f2c24e0` (`Treasury`, `DrandRound`, `Partner`, `DbcPool`, `DammPool` / `DammPosition` decoders for the registry keeper) decode rook_core and rook_backing accounts. Under D-l those programs are not deployed, `SolHomeCfg::books()` is the launch's mode, and `solix2::decode` carries the DBC / DAMM decoders the canister uses. They were not re-audited here; B3 (the keeper's cranks) or a removal should own them.

## 3. Checked and found correct

**`solix2` builders (`spl`, `mpl`, `dbc`, `damm`, `dlmm`, `route`)**
- Every instruction is laid out as the program unpacks it, and `tests/solix2_builders.rs` / `tests/solix2_dlmm.rs` compare each builder byte for byte with an instruction that LANDED in LiteSVM on the mainnet bytes (accounts, flags, data); the DLMM discriminators are checked against `sha256("global:<name>")`.
- Optional accounts take the program id (Anchor's convention) in DBC, DAMM v2, DLMM and Tensor; `RemainingAccountsInfo { slices: [] }` is four zero bytes.
- The PDAs: `["pool", config, max, min]` and `["token_vault", mint, pool]` (DBC), `["cpool", max, min]` / `["pool", config, max, min]` / `["position", nft]` / `["position_nft_account", nft]` (DAMM v2), `[preset, min, max]` / `[pair, mint]` / `["oracle", pair]` / `["bin_array", pair, i64]` / `["position", pair, base, lower, width]` (DLMM), `["metadata", program, mint]` (Metaplex); the ordering is `Pubkey`'s `Ord` where DBC / DAMM sort descending and DLMM ascending, each as its program does. `find_program_address` and the on-curve test were A4's.
- `route::two_hop_buy` is a user's transaction (the site's wallet signs it), never the canister's: its `wx_min` / `coin_min` are the user's slippage; the coin hop spends exactly `wx_min`. `open_pool` keeps §5.1's anti-squat order (mint, wrap, create in one transaction); `desk_atas` and `open_pool_memo` are the size fit the spikes measured; `range_around` refuses an empty or overflowing range and keeps inside `MIN/MAX_SQRT_PRICE`.
- `memo` names its signers as signer metas, so the program checks them.

**Decoders (`decode.rs`, `dlmm.rs`)**
- Every decoder checks the OWNER, the EXACT length and (Anchor) the 8-byte discriminator before any read; a short or foreign account is an `Err`, never a panic (A4-05 is closed for these). The DBC config's curve ends exactly at byte 1,048 (`408 + 20 × 32`), so `MAX_CURVE_POINTS` cannot read past the slice; `u128_at` is only reached after the length check.
- SPL `Mint` (COption at 0 / 46, decimals 44, `is_initialized` 45 must be 0 or 1) and `Account` (delegate 72, state 108 ≤ 2, `is_native` 109, close authority 129) match `spl_token::state::Pack`.
- The DBC `PoolConfig` and `VirtualPool`, the DAMM v2 `Pool` and `Position`, the DLMM `LbPair`, `PositionV2` and `BinArray` offsets are checked against accounts the programs WROTE in the fixtures (`flow.json`, `dlmm.json`); the decimals of wX (9) and the coin (6) are pinned where they matter (`check_wx_mint`, `pins::COIN_DECIMALS`, `price_of_floor`).
- The DLMM price arithmetic is the program's (Q64.64, the inverted base for a positive id, `bin_for_price` inverting `price_q64`, `half_width` 19 at step 50 for ±10 %), with unit tests; `quote_swap` walks bins in the swap's order and ends on a missing bin or the pair's bounds (`complete = false`), never looping past the arrays given; `div_wide` refuses an overflowing quotient.
- A hostile account at a derived address must be written by the owning program: a DLMM pair, position or bin array is the program's own state, so "lying fields" reduce to the program's rules (a stranger can create the pair first, which S4 §5.1 and the pool desk own).

**Guards (`guard.rs`)**
- `check_wx_mint`: initialized, 9 decimals, mint authority `K_coll[i]`, freeze authority NONE; `coins/open.rs:169-173` also refuses a mint with supply before the collection is listed.
- `check_config`: version 0, quote = wX, `fee_claimer` = `leftover_receiver` = `K_fc`, activation by timestamp, fees in quote, DAMM v2 migration, SPL token, 6 decimals, customizable migration fee option, LP permanently locked 50/50 with no liquidity share, no vesting, no creation fee, no creator trading fee, the pinned migration fee (50 %) and migrated pool fee (1 %), the fixed 1B supply, immutable token update authority, Rook's fee scheduler, no dynamic fee; the threshold = `T_wX × 10⁹`, the start price and the curve equal to the pin, and `quote_raised` over the decoded curve equal to the threshold within 0.1 % with the migration price at the curve's first point. There is no field through which a config the check accepts routes the curve's fees, the migration fee, the surplus or the leftover away from `K_fc` (B2-04 is about the migrated pool's fee shape only). `quote_raised` is DBC's `get_delta_amount_quote` rounded down.
- `refuse_forbidden` refuses `SetAuthority`, `Approve`, `ApproveChecked` on Token and Token-2022 and any Metaplex instruction but the create and the rename (B2-02 aside).

**Keys and the multi-signer rule (`solkey.rs`)**
- The derivation encoding is pinned (`tests/solkey_paths.rs`: bytes, text round trip, offline re-derivation as a child of `K_main`); the eleven paths are pairwise distinct at their second element (`coll` / `claimer` / `coin` / `desk` / `treasury` / `intent` / `nft` / `config`), the integers fixed-width, so no two roles share an address.
- `signer_paths`: a message with no signer is refused; EVERY signer must be a held key the caller named (`held_keys`: `K_main`, the Relay key, and `extra`); the Relay key never signs with another; the fee payer (signer 0) is `K_main` or the Relay key alone (so `K_coll` / `K_int` / `K_desk` never pay a fee); the signed size ≤ 1,232 bytes; all before the simulation and before any signature. `sign_paths_checked` asks every signature at once, keeps none unless all come back, and `assemble` re-checks the size. The single-key `signable` stays for `K_main`-only callers; the nonce burn signs with `K_main` alone.
- `sign_intent` (A4's, changed): `payer_of` chooses the Relay key only when every signer of the instructions is it, and forces a recent blockhash for it; `held` / `paths` are computed once and the same `paths` sign. A fallback re-sign uses the saved `keys`.
- `tests/sol_lines.rs` verifies each multi-key signature with real Ed25519 in the compiled order and the four refusals.

**The four lines and the timer (`solhome.rs`, `soltimer.rs`)**
- `LineId::of` partitions the owners (Eth / Frog / Custody → money; Crank / Coin / Treasury → crank; Desk; Pool); `line_seed` gives each line its own permanent nonce address (`"<tag>:<seed>"` cut to 32 bytes; the prefixes differ, so no two lines share a nonce account); `open_line_on` adopts an existing nonce of ours, opens a squatted System account in place (A4-02's fix: `AllocateWithSeed`, which only `me` can sign for), and refuses anything else for the operator.
- `claim`: the line must be free and the owner at the head (or the queue empty); a turn is taken once and removed; an owner is queued once; a turn older than 1 h is dropped; `release` gives a place back. `Crank`, `Coin`, `Treasury` and `Pool` release right after any refusal (`solcrank.rs:1454`, `coins/link.rs:28,34,89,100`), so they never hold a place (as §13.1 says); Eth / Frog / Custody release on an empty pass (`ethflow.rs:1254,1279`, `frogflow.rs:692`, `redeem.rs:1292`, `convflow.rs:432` with the quote kept). A step of one owner cannot take another's outcome (`take` partitions by owner) nor another's turn (`claim` by owner).
- `sign_keys`: refuses a busy line, an `Op` step of a non-desk owner, a Relay deposit with extra keys or not of the Relay key alone; the pending record and the Relay leg are saved with the signed bytes BEFORE the first send; the intent counter advances once per prepared transaction.
- `tick_line_inner`: at most one transaction per line; a landing is matched to the pending step by intent; the opening's landing sets the nonce account from `me` and the seed; `Abandoned` reports `ok: None`; `Idle` with a pending step (after an operator's clear) takes the step back as "abandoned"; the Relay legs are A5-03's. Lines are independent: one line's incident, error or busy transaction never holds another (`tests/sol_lines.rs`).
- The incident path: `clear_incident_on` holds the tick's flag, re-reads every signature of the saved intent (expired ones included) agreed, refuses if any is known, refuses if the line changed meanwhile, then moves the record to `done` as abandoned; the next tick's `Idle` reports the step unlanded and its flow re-signs under a fresh nonce read. A landed transaction cannot be re-signed through it: any known signature refuses the clear, and the old bytes sit on a nonce that moved.
- `pass_order` / `next_cursor`: round robin from a saved cursor; a full signing queue stops the pass and the lines after it go first next time.
- The timer: one-shot, re-armed by its own run (`arm` replaces and clears the previous id, so at most one is armed; `sol_timer(true)` is refused mid-tick); the fallback is armed before the first await; `hold_tick` serialises it with the runner's tick and with the operator's methods (`Held` → 10 s); an incident, a pause, `timer_off` and no home all idle at 60 s; the cluster is re-checked at most hourly; `init` and `post_upgrade` both call `arm_after_install` (`canister.rs:357,803`). `TimerSeen` in `sol_lines` is what the monitor pages on.
- `sol_lines` / `sol_line` expose `public_view` (no signed bytes in flight; the raffle book removed); `Done.history` holds signatures only.
- `books_sink_view`: lamports throughout, the cap `min(max_item_cost, limit)`, `want` in 1e18 scale as `floorsink::wanted` counts, `price_expiry` = `posted_at + FLOOR_TTL`; the same shape as `sink_view`.

**Solana NFTs (`solnft*.rs`)**
- Membership (Token Metadata): classic SPL mint (Token-2022 and Core under a TM listing refused), initialized, supply 1, decimals 0; `MetadataV1` of this mint, standard `None` / `NonFungible` / `ProgrammableNonFungible` only, not itself a collection, `collection == Some { verified: true, key }`; the edition is the mint's PDA (also by the metadata's `edition_nonce`) and a master edition. With a master edition the mint's freeze authority is either none or the edition PDA, so only Token Metadata can freeze, and only through a delegate the custody check refuses.
- Membership (Core): `AssetV1` (a hashed asset refused), `update_authority == Collection(key)`, a `CollectionV1`; no permanent delegate on either, no foreign owner-managed delegate, no lifecycle-veto adapter, a royalties allow-list must contain the System Program.
- Custody: the holder's ATA is SPL Token's, of this mint, owned by the holder, amount 1, no delegate, no close authority; a legacy account not frozen; a pNFT's token record `Unlocked` with no delegate and no locked transfer; a Core asset owned by the holder and not frozen. Every decoder is bounded (`Reader`); the Core plugin registry walk is capped (64 plugins, 64 adapters, 16 creators).
- Asks: Tensor — the `ListState` is this asset's PDA, version 1, SOL, unexpired, not private, and the list state's ATA (or, Core, the asset's owner) holds the item; mmm — the `SellState` is this asset's PDA of its pool, the pool is that owner's PDA, SOL, unexpired, with sell-side assets, and the pool's ATA holds the item; M2 — the trade state names this mint, the pinned auction house, size 1, SOL, unexpired, and the program-as-signer's or the seller's (with M2 as delegate) token account holds it. `total` is an upper bound of what the buyer pays; one ask per asset (the cheaper) in `verified_asks`; `floor_from` takes one ask per seller, the `rank`-th cheapest of ≥ `min_sellers`, refuses a reference gap over `max_gap_bps`, and names the cheapest keyless ask. A listing that expires or is edited between the read and the landing is refused by the program (a fee, no loss).
- The judges (J1–J4) rebuild the canonical instruction and compare every account and flag; the price arguments are bounded (`max_amount ≤ price + royalty` for Tensor, `max_payment_amount ≤ total + taker` for mmm, every argument byte-equal for M2), no trailing data, `all_in ≤ limit` with rent. **Verified on Tensor Marketplace's mainnet bytes** (`solnft-svm/tests/audit_b2_judge.rs::checked_b2_tensor_max_amount_binds_a_listing_the_seller_raises_after_the_judge`): the program binds the listing amount PLUS the royalty to `max_amount` (`buy.rs:267 PriceMismatch`), so a seller who edits the listing up by one lamport after the canister's read cannot raise what it pays; the doc's "a front-run cannot raise it" holds. `best_ask` never chooses M2 (notary-gated to LIST; `solnftm2` is the separate, judged buyer-only path S5 proved) and only keyless asks unless `cosigned` (B2-06). The 15 % premium is the caller's `limit`; the seller being the canister itself cannot arise (the canister never lists, and a Tensor listing is escrowed by its own PDA).
- Delivery: the destination must be a real on-curve wallet other than the canister; legacy = idempotent ATA + `TransferChecked(1, 0)` + close of the emptied source; pNFT = `TransferV1` with both token records and the item's rule set; Core = `TransferV1` with the collection; `judge_send` accepts exactly that (B2-01 aside).
- M2 (S5): the three instructions (`deposit` of exactly price + 2 % + royalty, `buy_v2`, the execute for the standard) with the notary as an account, not a signer; the escrow, trade states and destination are the canister's; the seller trade state is re-derived from the listing's fields and must equal the candidate; `tests/m2_buy.rs` lands all 22 recorded listings and the judge refuses a limit one lamport short.

## 4. Test results

- `cargo test --locked --no-fail-fast` in `autobuyer/canister-rs` at `e4d4d34` plus `tests/audit_b2_primitives.rs`: every test binary green (55 `test result: ok`), `audit_b2_primitives`: 4 passed.
- `cargo test --locked` in `autobuyer/solnft-svm` plus `tests/audit_b2_judge.rs`: `audit_b2_judge` 2 passed, `buy_send` 7, `m2_buy` 1; `m2_explore` 5 passed and 2 failed (`one_failure`, `grid_legacy`: they `unwrap` the `M2_FIXTURE` environment variable, a spike's exploration harness, failing identically without this change).
