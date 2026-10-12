# Fourth pass: the platform's own security guidance, for the platform that had none applied

2026-09-22. Scope: the buyer canister and the contracts, against sources the third pass did not use. Method builds on
`audit-method-v1.md`.

**What was missing.** The third pass pointed its platform category at Arbitrum and found the pass's most severe result
there. It pointed nothing at the **Internet Computer**, which is the platform the other half of this system runs on, and
which has its own published security guidance and its own set of footguns. That asymmetry is the whole reason for this pass.

## New sources

| Source | What it contributes |
|---|---|
| [DFINITY: inter-canister calls](https://github.com/dfinity/portal/blob/master/docs/building-apps/security/inter-canister-calls.mdx) | Traps roll back only to the callback's snapshot; interleaving messages have no reliable ordering; per-caller locking with `Drop` guards; reject-code handling and idempotency; untrusted-canister patterns |
| [DFINITY: general recommendations](https://github.com/dfinity/portal/blob/master/docs/building-apps/security/misc.mdx) | Verify exported endpoints in CI (`ic-wasm check-endpoints`); reproducible builds; `raw_rand` for randomness; time is **not strictly monotonic**; no floating point in financial arithmetic |

## Summary

**1 high, 2 informational; two new tests that make the two criticals from the third pass un-reintroducible.**

| # | Sev | Where | One line | Status |
|---|---|---|---|---|
| **C1** | high | `tick()` | Two ticks could run through each other and decide twice from one reading | **fixed** |
| **C2** | info | the canister's exports | Nothing checked that a new endpoint carries the caller guard | **test added** |
| **C3** | info | the vaults' ABI | Nothing checked that "there is no withdraw" is true of the compiled artifact | **test added** |

---

## C1 — two ticks, one reading

**Severity: high.** The state machine's central promise, defeated by the platform's concurrency model rather than by any
line of the state machine.

**Where.** `canister.rs::tick`.

**The defect.** `tick` walks the open purchases and, for each, does `observe(id).await` and then decides what to act on from
what it read. DFINITY's guidance is explicit that **"messages from interleaving calls do not have a reliable execution
ordering"**, and names the consequence: time-of-check-to-time-of-use, and double spending. Nothing in this canister held a
lock. A second `tick` entering during the first one's await reads the same pre-act state, reaches the same decision, and acts
on it too.

For a state machine whose first sentence is *"nothing that spends is attempted until the intent to spend is durably
recorded"*, two deciders on one record is precisely the shape the recording exists to prevent. The interruption table in
`state-machine.md` answers "the canister dies here"; it never asks "the canister is here **twice**".

It is not exploitable today only because the acting half is not wired (B4 of `audit-btc-v1.md`: the canister cannot broadcast
an EVM transaction at all). That is a reprieve from an unfinished feature, not a property.

**The fix.** A `TickGuard`, refusing a second entry, released through `Drop`. The `Drop` matters as much as the flag: DFINITY
notes that a trap rolls back to the snapshot before **the callback**, not before the whole call, so a boolean set and cleared
by hand leaks on a trap and wedges the canister permanently on one failed outcall. `Drop` runs through `ic0.call_on_cleanup`.
`a_second_tick_is_refused_while_one_is_running_and_the_flag_is_released_on_drop` asserts both halves.

---

## C2 — nothing checked that an endpoint was meant to be there

**Informational, and it is the lesson of the third pass's first critical.** DFINITY recommends verifying in CI that a
canister's exported endpoints are the legitimate ones, precisely because an endpoint nobody meant to publish is
indistinguishable from the outside from one somebody did. B1 was exactly that: two methods that signed any 32 bytes for any
principal, guarded on which network they signed for and on nothing else.

`only_controller()` fixed the instance. `every_update_method_takes_the_controller_guard` fixes the class: it reads the
canister's own source, finds every update endpoint, and fails on any that does not take the guard — so the next one added
without a thought about who may call it fails a test rather than a vault.

## C3 — nothing checked that "there is no withdraw" was true

**Informational.** "`AssetVault` has no withdraw function, by design" is the sentence every trust document in this repository
rests on, and until now it was verified by people remembering it. `test/security/Surface.t.sol` reads the compiler's own
`methodIdentifiers` — the actual external surface of the compiled artifact — and asserts it.

Two assertions. The v2 vault's surface must be exactly the 30 functions it means to expose, so a function made public by
accident fails. And neither vault may expose anything beginning `withdraw`, `sweep`, `rescue`, `drain`, `transferTo`,
`emergency`, `setBuyer` or `setOwner`.

It found the one real edge on first run: v1's `withdrawBond`. That is allowed, explicitly and with its reason in the code —
it returns the *purchaser's own posted bond* to the address that posted it, never below the share of what is still in their
hands and never while one of their purchases is overdue. That money was never a coin's. v2 has no bond and so needs no
exception, which is the more interesting half of the result.

## Checked against the new sources and found sound

- **No floating point anywhere in the money path.** DFINITY warns against floats in financial arithmetic. The canister is
  `u64`/`u128` throughout. The JavaScript drivers use `Number` only for block heights, a recovery id, and sat fees — and
  every sat that will ever exist (2.1 × 10¹⁵) fits in a double (9.0 × 10¹⁵) with room, while every wei amount is a `BigInt`.
- **No randomness is used**, so the `raw_rand` recommendation does not bite. The one place this system needs randomness is
  the Distributor's draw, which is on Arbitrum and uses drand with an on-chain pairing check.
- **Time is used only for deadlines measured in days**, and the guidance's concern — that consecutive calls may return the
  same value — does not affect a comparison against a three-day deadline.
- **Reject-code handling**: the canister's outcalls are replicated HTTPS outcalls with a transform, not inter-canister calls,
  so the `SYS_UNKNOWN` distinction does not arise yet. It will the moment the buyer talks to the Bitcoin canister, and the
  idempotency pattern the guidance asks for is already the design's own answer (`docs/v2/state-machine.md`).

## Still open, unchanged

`audit-btc-v1.md` **B3** (the sighash is computed off-canister, so `psbt.rs::verify` is not on the path that signs — still
the largest piece of work before this holds money), **B4** (no EVM broadcast at all), **B5**, **B6**. `audit-v2.md` **S3**
(the buyer signs its own receipt), **S6** (no monitoring, no security contact, no incident-response plan), **S7**. And the
two DFINITY recommendations this codebase has no answer to: **reproducible builds**, without which a published module hash
cannot be checked against published source, and the **timelocked controller** that `autonomous-buyer.md` already names as the
right answer and that nothing implements. Until both exist, the honest sentence remains *the operator can change what the
buyer does, with a delay of zero*.
