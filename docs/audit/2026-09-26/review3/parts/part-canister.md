# R3 check, part D: the buyer canister and the timelock

Checkout `/Users/jlohayon/Documents/Rook-rev3` (detached at `fixes-2026-09-26`); diff `2bb3ee7..fixes-2026-09-26 -- autobuyer`
(23 files, +2,031 / −181). Read: `R2-canister.md`, `fixes-R3-canister.md`, `integration-R3.md` §1, the full diff, and the current
`ticketlog.rs`, `redeem.rs` (custodian pass, paused pass, attest), `conflict.rs`, `custody.rs`, `timelock-rs/src/{rules,canister}.rs`.
Added (untracked, nothing else modified): `autobuyer/canister-rs/tests/audit3_scanner.rs` (2 tests), `tests/audit3_pin.rs` (2 tests).
No commit, push, deploy, key or chain write.

## 1. R2 canister High/Medium, and M-1 residuals

| R2 ID | Verdict | Why (PoC against the new code) |
| --- | --- | --- |
| **N-1** scanner stalls on a >48 kB page | **Fixed** (residual R3-C1, Low) | R2's 60-event burst is read in the first tick (the page is retried at 400 kB); a 2.7 MB block is read by ticket; the cursor always advances (`MIN_SPAN_BLOCKS` 1, dense/by-ticket, `unreadable` after 3). Re-run in `audit3_scanner.rs` (control case: 60 events, 1 tick). **But** after a bisected dense region the scan crawls one block a page to the end of `hot` (R3-C1). |
| **N-2** redeemer pin → LostRace | **Fixed as bounded, not closed** | Canister side: TRUC (`nVersion` 3) on every delivery and bump (`custody.rs:492`); conflict re-priced every 2 h up to 100,000 sats; attestation only after the conflict (or a third spend of the fee input) is 6 deep. R2's own mempool (NO replacement ever accepted) re-run in `audit3_pin.rs`: 4,500 replacements refused, **0 attestations** (the canister never claims undeliverable while the delivery can confirm), the gated wrapper refuses the refund at deliverBy + 72 h, and only at **deliverBy + 30 d** can the redeemer refund → then LostRace. So N-2 now needs a pin held 30 days (under TRUC: a ≥ ~82 sat/vB market for 30 days and ≥ 75k sats of child fee at risk; Core's 14-day mempool expiry also works against it). Acceptable residual, as documented. |
| M-1 (b) conflict never re-priced | **Fixed** | `conflict::reprice` (`conflict.rs:345-386`): ×1.5 / +incremental / today's rate, ≤ `MAX_CONFLICT_FEE_SATS`, H4 `ConflictOut` at 4× (`outbound.rs:282-285`), only custody outputs, `earlier` tracked. |
| M-1 (c) nothing done while paused | **Fixed** | `redeem::paused_tick`: no sign/bump/re-send; the rule is re-read, the conflict-out runs and re-prices (Via.paused = false), confirmed deliveries are recorded. `audit3_pin.rs` M-1(c): pause from day 5 to day 20 across the margin → conflicted out and confirmed while paused, frog never left, `markUndeliverable` first sent only after the resume. |
| N-12 limiter asked before facts | **Fixed** | `owed`: `facts_for` before `outflow_check` (`redeem.rs:748-755`); review2 regression passes. |
| N-3 / N-4 / N-5 / N-6 / N-7 (Low) | Fixed as designed | upper median (`custody.rs:230-239`); per-candidate `continue` + quarantine + listing fallback; `MAX_BUY_FEE_SATS` 24,500; timelock `running` state; N-7 dedup. |

`markUndeliverable` attestation (checked for false attestations): `Due{None}` is set only by `rule()` (called only for `Owed`, before
and after signing, nothing saved or sent) and the I-6 refusal; every refusal of saved bytes goes through `ruling` → `conflict_out`.
`Due{Some(t)}` only on `ConflictConfirmed` / `SpentElsewhere` (the fee input, shared by every version incl. bumps, spent in a block).
Not sent while paused (paused pass skips finished records); re-reads the ticket (`Resolved` only) and `may_record` first. A stranger
cannot cause an attestation for a ticket that can still be delivered. Sound.

## 2. New findings

| ID | Sev | Title | Where | PoC | Result |
| --- | --- | --- | --- | --- | --- |
| R3-C1 | Low | After a range too big even at 400 kB is bisected, successes inside `hot` keep the span, so once the dense block is read the scanner walks the rest of `hot` **one block a page** (8 a tick) | `ticketlog.rs:251-254` (and `:235-239`: `hot`/`span` not reset after the dense block) | `audit3_scanner.rs::audit3_a_dense_block_at_the_start_of_a_page_…` | PASS: 500-event block at a page's start delays a ticket 4,000 blocks later by **502 ticks**, 7,990 blocks later by **1,000 ticks (≈ 6.9 days at 600 s)**; a 2,400-event block read by ticket: 505 ticks. Past every delivery's margin → all refund (R2 N-1's outcome) |
| R3-C2 | Low | ONE provider can make the scanner give up a chosen ticket's events for good: a size-looking error (`too_big` substring-matches the whole error string, including a provider's JSON-RPC message, `evmread.rs:174-177`; or a padded body) on one block sends it down the dense path, the one-ticket query fails 3×, the ticket goes to `unreadable` and the cursor passes the block, although the other provider served the events. Pre-R3 a provider could only stall everyone, loudly; this bypasses the "providers must agree" rule for a targeted ticket | `ticketlog.rs:262-271, 323-330`; `evmread.rs:174-177`; `canister.rs:604` | `audit3_scanner.rs::audit3_one_provider_can_make_the_scanner_skip_…` | PASS: cursor past the block in 3 ticks, ticket 5 `unreadable`, never read (also a spurious `(block, 6)` entry). Impact: that ticket refunds (loud SEV1 `UNREADABLE`), no backing lost |
| R3-C3 | Info | Paused pass skips `assert_chain` and still signs/sends `recordDelivery` from explorer reads; a pause raised as a forgery alarm (explorers or RPC suspected) keeps acting on them | `canister.rs:1218-1232` | reasoned | — |
| R3-C4 | Info | Quarantine counts are per choice, not per tick: 3 fee choices in one tick (3 Owed tickets) with a transient per-output explorer failure quarantine a good entry; entries never auto-restore (operator re-adds); a flaky/429-ing explorer slowly erodes the pool toward the listing fallback (H-4 surface) | `redeem.rs:880-892`, `custody.rs:369-388` | reasoned | — |
| R3-C5 | Info | Timelock: (a) if an `execute` traps after its first await (e.g. cycles), its `running` entry is never removed: that proposal can never run and `start_target` is refused for ever (`may_start` refuses while anything is running); no method clears it. (b) A veto/withdraw landing during `install_code` returns "vetoed"/records `Vetoed`/`Cancelled`, yet the install completes and `Executed` is recorded too | `timelock-rs/src/rules.rs:397-399, 459-468`; `canister.rs:223-251` | reasoned | — |
| R3-C6 | Info | `markUndeliverable` is signed once at `4 × base fee` with no re-pricing (as `recordDelivery`); while `Attest::Sent` it counts as `evm_pending` and blocks every other redemption's `recordDelivery`. A stuck one held ≥ 30 days would leave confirmed deliveries unrecorded and refundable (LostRace). Pre-existing pattern, now on one more tx | `redeem.rs:233, 596-616` | reasoned | — |
| R3-C7 | Info | With the gated wrapper there is no refund race until the attestation, yet conflicts still escalate ×1.5 every 2 h to 100,000 sats whenever not confirmed within 2 h; overpayment in busy markets (bounded, custody-only) | `conflict.rs:258, 292-294` | reasoned | — |

Also checked, no finding: cursor advance and loop bounds (`MAX_TRIES_PER_TICK` 12 rounds, 8 pages; by-ticket progress saved,
`rest` empty ⇒ block done with no read); `wanted` = redemptions lacking events + `cursor..ticketCount@fast pin` covers every ticket
whose events can lie at or below finality (the walk stops at the first `Pinned`); TRUC: frog and fee inputs are required confirmed,
so no v3 spend of an unconfirmed parent; conflicts are v2 spending only confirmed custody outputs (replacing a v3 tx across versions
is allowed); bump and conflict fee math and caps (≤ 24,500 / ≤ 100,000 enforced in `build` and H4); upper-median rule; timelock
`untake` (only if still wanted, epoch restored, `STOP_RETRY_NS` 10 min), veto/withdraw/roles-prune of a running proposal,
`start_target` guardian-only and refused while running.

Reachability of R3-C1: bisection needs > ~440 wrapper events in one 8,000-block page with most in one block. At stage-3 caps
(192 items per collection) Redeemed + Resolved in one window is ≈ 232 × (900 + 700) B ≈ 370 kB, just under the 400 kB retry, so
it is at the edge of reachable (larger per-log size from another node type, or higher caps, cross it). Fix: after a success inside
`hot` double the span too (capped at `h − to`), and clear `hot` (and reset `span` growth) once the dense block is read.

Fix R3-C2: treat a size error as such only when the IC reports it (match the IC reject code/format, not provider JSON text), and
require the size error from every provider (or from the provider whose answer really is large) before shrinking below the span the
other provider served; retry `unreadable` tickets later instead of dropping them for good (re-read the block by ticket when the
redemption reaches its margin).

## 3. Suites

| Suite | Result |
| --- | --- |
| `autobuyer/canister-rs` `cargo test` (as shipped) | **478 passed, 0 failed** (322 unit + 156 integration) |
| same, with the 4 `audit3_*` tests | **482 passed, 0 failed** |
| `autobuyer/timelock-rs` `cargo test` | **12 passed, 0 failed** |
| `autobuyer/timelock-rs` `cargo test --features rehearsal` | **12 passed, 0 failed** |

Logs: `$S/canister-test.txt`, `$S/timelock-test.txt`, `$S/timelock-reh.txt`. Not run: reproducible Docker build, clippy/audit,
replica/fork run of the scanner against real providers, signet TRUC relay (all listed as open by the fix docs).
