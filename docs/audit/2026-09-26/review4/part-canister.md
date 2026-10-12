# R4 check, part: buyer canister and timelock

Checkout `/Users/jlohayon/Documents/Rook-rev4` at `fixes-2026-09-26` (`cf6ecda`). Scope: `autobuyer/canister-rs`, `autobuyer/timelock-rs`
in `git diff 4a215f4..fixes-2026-09-26` (13 files, +1,020 / −63). Toolchain 1.98.1 (`~/.cargo/bin`). Nothing existing modified.

Untracked additions:
- `autobuyer/canister-rs/tests/audit4_scanner.rs` (2 tests)
- `autobuyer/canister-rs/tests/audit4_partition.rs` (2 tests; its harness is copied from `tests/review3.rs`)

The R3 PoCs were copied as `tests/audit4_r3{scanner,pin}.rs` and re-run unmodified. They were then moved out of the tree to
`scratchpad/rook-audit/R4/orig-r3-canister-pocs/` with their log `rerun.log`, so `cargo test` stays green. The `target/` dirs I
created (1.2 GB and 400 MB) are deleted. There were none before.

## 1. Round-3 canister items

| ID | Verdict | Evidence |
| --- | --- | --- |
| **R3-C1** (Low): the scanner crawls one block a page through `hot` | **Fixed** | R3's PoC re-run unmodified **fails** on its attack assertion (`the honest ticket's events wait hundreds of ticks: 3`). Measured: 500-event block, honest ticket 4,000 on → read in 3 ticks (31 `eth_getLogs`), span back to 8,000, `hot` None. Was 502 / 1,000 / 505 ticks. Sweep (`audit4_scanner.rs`), all read, nothing skipped, every tick ≤ 12 rounds: dense block at offset 0/1/4,000/next page → 3 ticks; offset 3,999 or 7,999 → 4; 20 consecutive dense blocks → 11; 20 alternate → 14; 10 spread 700 apart → 30. |
| **R3-C2** (Low): one provider makes the scanner skip a ticket | **Fixed** | R3's PoC re-run **fails**. The lying provider's text size error is now `Failed`: the scan stalls with the provider named, and nothing goes to `unreadable`. `too_big` is `starts_with(SIZE_REJECT)`. That prefix is added only in `canister.rs::outcall_error`, for a `CallRejected` whose reject message *starts with* the IC's wording. Provider text arrives as `no result: …` / `unreadable: …` (`rpc_once_with`), so no provider text can carry the prefix. A padding provider (a real IC reject from one provider) gives `Split`: in paging it reshapes like `TooBig`; at a one-ticket query it stalls. It never skips. `unreadable` needs **every** provider `TooBig`. |
| R3-C3 (Info): paused pass skips `assert_chain` | **Fixed** | `canister.rs:1234-1239`, source test present. Side effect: see R4-C5. |
| R3-C4 (Info): quarantine counted per choice | **Fixed** | `FeePool::failed(…, now)` counts once per 3,600 s, and `read_ok` clears it (`review3.rs` test). A flaky entry takes ≥ 2 h to quarantine, and a failing candidate is still skipped per choice (review2 test), so a slower quarantine costs a few reads, never a delivery. |
| R3-C5 (a) (Info): orphaned `running` blocks `start_target` | **Fixed** | `clear_orphans` runs from a guardian's `may_start` after 24 h, or at once for an entry saved before `since_ns`. **Can a guardian clear a live execute?** Only past 24 h, and a real `stop_canister` / `install_code` await is minutes. If one were cleared and later resumed, `still_wanted` returns "not running": the buyer is started, nothing is installed, and `Failed` is recorded. A pre-R4 state with a running entry can exist only if that execute already trapped (an upgrade of the timelock waits for its callbacks), so "orphan at once" is correct. |
| R3-C5 (b) (Info): veto during install recorded but not honoured | **Fixed** | `commit` is set in the same `with` as the last `still_wanted`, with no await before `install_code`. For `Controllers` it is set at take, since the call is issued in the same message. **Can `committed` stick after a failed execute?** No. Every non-trap path reaches `finish(id)` (`canister.rs` after the match), which removes the entry. A trap leaves an orphan, which the rule above clears. Veto and withdraw of a committed entry are refused and not recorded. Roles changes skip committed entries. |
| R3-C6 (Info) | Open, as listed | Not changed (same-nonce re-pricing). |
| R3-C7 (Info): conflicts escalate to 100k needlessly | **Fixed as designed, with a new weakness** | `review3.rs` shows no re-price while in a mempool at today's rate. See **R4-C1**: `held` is the explorers' view, and a mempool partition turns it against us. |
| N-2 (accepted, bounded) | Unchanged | R3's `audit3_pin.rs` re-run unmodified **passes**: 4,500 replacements refused, 0 attestations, refund at deliverBy + 30 d, then LostRace. M-1(c) passes too. |

## 2. New findings

### R4-C1 (Low): R3-C7's `held` rule stops escalation under a mempool partition, so a cheap pin wins the 30-day race

- **Where:** `autobuyer/canister-rs/src/conflict.rs:260-268` (`held = seen == Mempool`) and `:363-378` (`reprice`: `held` → only today's
  rate, no ×1.5, not re-signed while `vsize × rate ≤ fee`).
- **Mechanism:**
  - `seen` means **all the explorers** have our conflict (`btcnet::btc_tx_seen`). The miners are what matter.
  - Suppose the redeemer holds a pinned delivery package at the miners, with a high absolute fee and a low feerate (e.g. by private
    submission). Suppose also that the explorers have our conflict, because it replaced the bare delivery there.
  - Then `held` stays true. Our conflict sits at today's rate (≈ 2.2k sats at 10 sat/vB) and never out-pays the pin's absolute fee
    at the miners.
  - Round 3 escalated ×1.5 every 2 h towards 100k sats, and so beat any pin under that within hours.
  - The gated wrapper refunds at deliverBy + 30 d, and the delivery confirms: LostRace.
  - R3's N-2 bound was "a pin held 30 days needs ≥ ~75k sats of child fee at risk". Under a partition the pin needs only a little
    more than our conflict's fee (a few thousand sats).
- **PoC:** `tests/audit4_partition.rs::audit4_r3c7_a_partition_keeps_the_conflict_at_todays_rate_and_a_cheap_pin_wins_after_thirty_days`,
  **PASS (weakness present)**:
  - max conflict fee over 40 days: **2,231 sats**, against a 20,000-sat pin at the miners;
  - refund at deliverBy + 2,592,000 s;
  - phase `LostRace`.
- **Control:** `audit4_r3c7_control_an_unseen_conflict_escalates_and_beats_the_same_pin`, the same world with the conflict not seen
  (round 3's treatment). The conflict escalates to 25,407 sats, out-pays the pin and is refused / conflicted out, and there is no
  LostRace.
- **Why only Low:**
  - The partition is hard: every miner that matters must hold the pin and refuse ours, while both explorers hold ours and not the
    pin, for 30 days, across Core's 14-day expiry.
  - Our conflict propagates publicly from the explorers, so any miner without the pin mines it at market rate.
  - The loss is one item paid twice, at most, per redemption.
- **Fix:**
  - Let `held` suppress the ×1.5 escalation only for a bounded time, e.g. the first 12–24 h, or the first N re-price periods,
    since the conflict was first seen.
  - A conflict at today's rate that stays unmined that long is itself evidence that the miners do not have it. After that, escalate
    as round 3 did.

### R4-C2 (Info): one provider can stall the scanner through `Split`, the same power it always had, and the stall is cheap

- **Where:** `ticketlog.rs:394-399`.
- A provider padding **every** answer stalls the scan at the first wanted block. This is the same as a provider returning any error
  before round 3.
- The stall is loud: `failures`, `last_error` names the provider, and the scanner alarm pages.
- Nothing is skipped or set aside, and the scan recovers the tick the provider answers.
- Cost: the `dense` state keeps its one-ticket shrink, so the steady state is **1** `eth_getLogs` round a tick at a 400 kB cap, not
  a fresh bisection each tick.
- **PoC:** `audit4_scanner.rs::audit4_a_provider_padding_every_answer_stalls_cheaply_and_skips_nothing`, **PASS**. Rounds a tick:
  `[12, 5, 1, 1, …]`.
- **Liveness:** a stall past a delivery's margin refunds that delivery (loud, no backing lost). That was already true for any
  provider error. The real fix is still a third provider.

### R4-C3 (Info): size detection now depends on the IC's exact reject wording

- **Where:** `evmread::ic_size_reject`, `evmread.rs:230-237`.
- The IC's reject message must *start with* "Http body exceeds size limit" or "Header size exceeds specified response size limit".
- If a replica release rewords or prefixes it, no answer is ever `TooBig`. A dense page then becomes `Failed` every tick: a
  permanent stall, the R2 N-1 symptom. It is loud and skips nothing, so it fails closed.
- **Fix:** confirm the wording on a replica and on mainnet with a real oversized answer (already an open go-live gate: "the replica
  run of the scanner"). Consider matching the reject **code** (`SysFatal` / `SysTransient`) plus a contains-check on the IC's
  message only. That is safe, because the message comes from the IC, not the provider.

### R4-C4 (Info): the paused pass's Bitcoin conflict work now needs the EVM providers to agree on the chain id

- **Where:** `canister.rs:1234-1239` (R3-C3).
- One failing EVM provider (every provider must agree in `rpc`) now also skips the paused pass's conflict-outs and re-sends.
- The unpaused tick already skips everything on `assert_chain` failure, so this adds no new capability. It does couple M-1(c)'s
  Bitcoin protection to EVM availability.
- Acceptable. Consider caching the last successful `assert_chain` result for a few ticks, for the paused pass only.

### Note (pre-existing, not introduced by R4)

- **Where:** `conflict.rs:265`: `BtcView::Unsure(e) => return Err(e)`. It runs before the re-price and the re-send.
- If the explorers keep disagreeing on whether they hold our conflict, the step errors every tick. The conflict is then never
  re-priced or re-sent.
- An attacker could cause this with a partition between the two explorers' nodes.
- Worth treating a presence disagreement as `NotYet` for the escalation decision.

### Checked, no finding

- **Scanner growth.** Doubling inside `hot`, and clearing `hot` and the cap on `dense_read` and after a by-ticket block:
  - progress is guaranteed (each success advances the cursor; each failure halves down to a by-ticket read);
  - no tick exceeds 12 rounds in any sweep case;
  - clearing `hot` only costs one 400 kB retry of the next big page.
  - An isolated dense block costs about 3 ticks. Making one costs 500 `Redeemed` events, i.e. 500 burned units (the destination is
    validated `isTaproot` on chain), so an attacker cannot make them cheaply.
- **Unreadable retry.**
  - It is one round a tick (counted in `tries`) at 48 kB, and one entry a tick, rotating.
  - Entries are added only when **every** provider says `TooBig` at 2 MB for one ticket's events. The wrapper cannot emit that,
    so an attacker cannot grow the list.
  - An entry is dropped only when the tickets were walked and its ticket is no longer in `wanted`, which means the redemption no
    longer needs events.
  - `u.remove(0)` comes before `ingest(..)?`, so an ingest error would lose the entry. But ingest fails only on malformed logs that
    every provider agrees on, and the wrapper only emits validated Taproot text. Not reachable.
- **`SIZE_REJECT` spoofing.** Not possible from a provider (see R3-C2 above). `LogsError::TooBig.to_string()` carries the prefix,
  but only `evmread::logs` callers see that string, and nothing re-parses it through `too_big`.
- **Timelock.**
  - `may_start` mutating state on its `Err` path is intended: the orphans are cleared anyway.
  - `untake` after a clear errors harmlessly.
  - The epoch is left moved on an orphan clear (documented, conservative).
  - The decode of a round-3 state is tested.

## 3. Suites

| Run | Result |
| --- | --- |
| canister `cargo test` (as shipped plus 4 audit4 tests) | **493 passed, 0 failed** (489 shipped: lib 324, e2e 2, frogs 44, handins 15, hardening 9, loop 20, ordapi 4, policy_loop 3, providers 6, redeem 29, review2 14, review3 9, sales 8, wrapper_abi 2; plus audit4_scanner 2 and audit4_partition 2) |
| R3 PoCs re-run unmodified (`audit3_scanner.rs`) | 2 of 2 **fail** on their attack assertions (R3-C1, R3-C2 closed) |
| R3 PoCs re-run unmodified (`audit3_pin.rs`) | 2 of 2 pass (the N-2 bound and M-1(c) unchanged, as intended) |
| timelock `cargo test` | **15 passed** |
| timelock `cargo test --features rehearsal` | **15 passed** |

Not run: clippy (component not installed), the reproducible Docker build, a replica or mainnet run of the scanner (which R4-C3
needs).
