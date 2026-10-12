# Round 7, part F: the 4663 home line after fix round 6 (F6-L-1, F6-L-2, D6-L-1, D6-L-2)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `a78a542`
(delta `bb713fe..HEAD`, canister commit `2d747ba`; runner `7b9e8ba`).

**Scope:**
- `canister.rs` `home_nonce_tx` (`:1066-1114`, the reach start `:1073-1088`), `nonce_at` (`:1058`), `evm_seen` (`:991-1013`), `evm_nonce`
  (`:1367-1373`); `evmread::fast_pin` (`:63-79`), `pin::choose_pin` (`pin.rs:54-61`);
- `ethflow.rs` `HOME_STATE_REACH_BLOCKS` (`:115-120`), `HomeTx.sent_at` (`:492-495`), `send_home` (`:3221`), `resolve_home`
  (`:3716-3849`), `home_effect` (`:3899-3937`), `home_clear` (`:4502-4521`), `home_clear_applies` (`:4525-4531`), the tick's order
  (`:976-979`, step 7);
- `ops/src/runner.ts` `sleepAfter` and the main loop (`:73-81`, `:208-215`), only as far as it sets the first resolving tick (the runner
  itself is part D's).

Toolchain 1.98.1. **Nothing in the worktree was modified except this report and `review7/poc/F/`.**

- **Where the PoCs ran:** a private copy (`git archive a78a542 autobuyer/canister-rs` into `/tmp/rook-r7-F`), `diff -rq` identical to
  the worktree before anything was added, removed afterwards.
- **Read-only probes:** `eth_blockNumber`, `eth_getBlockByNumber`, `eth_getTransactionCount` on `rpc.mainnet.chain.robinhood.com` and
  `rpc.ordofi.network` (`poc/F/depth_scan.output.txt`). Nothing was sent.

Severity scale and threat model: `../README.md`'s. A passing PoC shows the weakness is present.

## Verdict

- **F6-L-2 is closed.** A receipt the providers disagree on (`Unsure`) with the nonce used now runs the same path as "seen nowhere"
  (`:3735-3848`), and the clock is no longer restarted by the disagreement. Every value-deciding read still has to agree on every
  provider, so one lying or lagging provider can only make the line wait, never drop, redo or double-pay a step (details below).
- **D6-L-1 (canister side) is closed.** `home_clear_applies` accepts exactly the steps for which `home_effect` returns `Ok(None)`,
  minus `Post` (which drops itself): a Relay `Deposit` and a sweep's `GiveBack` (`fundReserve`). The monitor's `clearable` uses the same
  rule.
- **D6-L-2 (canister side) is closed.** `HomeTx` is built in exactly one place (`send_home`, `:3221`), which sets `sent_at` to the
  signing time. A resend re-broadcasts the same bytes and keeps it. The only gap is a transaction saved by a pre-round-6 build
  (`sent_at` absent), which the monitor never pages (E7-I-2, part E).
- **F6-L-1 is partly fixed (F7-L-1, Low).** The reach start is **sound**: it can never mis-identify or adopt the wrong transaction,
  and it cannot make the line wait for ever on a step with an effect. But the fix's margin is thinner than `fixes-round6.md` §4 says.
  - The reach is measured from the provider's HEAD. `HomeTx.pin` is the fast pin, 1,024-2,047 blocks under the head (more with a
    provider lagging).
  - So from `HomeTx.pin` the reach is 4,169-5,192 blocks, 417-519 s, not the "≈ 620 s" against a "≈ 300 s" next tick.
  - The fixer's test models neither the pin's lag nor the canister's new reach branch (the fake's `home_nonce_tx` ignores it), and
    the branch itself only helps a transaction that mined more than 400 s after signing.
- **No Critical, High or Medium.**

## Counts (part F, open)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 1 (F7-L-1) | 2 (F7-I-1, F7-I-2) |

Carried, unchanged: F6-I-1 (a two-step chain move keeps the old floors), F6-I-2's second half (`eth_home_clear` refused while paused),
F6-I-3, F6-I-4, F5-I-1 (the other 4663 signers).

## F6-L-2: `Unsure` with the nonce used (closed)

**What changed** (`resolve_home`, `:3716-3849`): `EvmView::Unsure` joins `NotYet`.
- **Nonce unused** (`chain <= p.nonce`): `used_since` cleared, the `Unsure` error returned. It waits, as before.
- **Nonce used:** `used_since` starts (or keeps running), then `home_nonce_tx` identifies the transaction and `home_effect` reads the
  step back, exactly as for "seen nowhere".

**Attacks tried, each against the code:**

| Attack | What the code does | Result |
| --- | --- | --- |
| One provider fakes a success receipt for a transaction that did not mine | `evm_seen` needs every provider's receipt equal (`canister.rs:992-997`): `Unsure`. The nonce read (`evm_nonce`, `rpc`: all agree, `:804-816`) decides whether anything happens | Waits or runs identification. Never `Mined` on one provider's word |
| One provider hides a mined receipt (the F6-L-2 case) | `Unsure` + nonce used → identification (nonce counts and the block's transactions, every provider agreeing) → `MINED` | Resolved at the first tick in reach (`r6_f6_l2_…` reproduces). Out of reach: the effect decides, or the operator for a Deposit / `fundReserve` after the hour |
| One provider lies about the nonce count at the pin | `rpc` refuses a disagreement: an error, the tick waits | Wait only |
| One provider lies in the identification (a nonce count at a past block, a block's transactions) | Each step of `home_nonce_tx` is `rpc` / `rpc_agreed` with a disagreement check (`:1069-1112`) | An error: the effect decides |
| One provider lies about the effect (e.g. `transit_open`, `itemOf`) to force a DROP | `home_effect`'s reads go through the same agreed reads. A DROP (`:3808-3814`) needs the effect ABSENT on every provider, at a pin an hour after the nonce was first seen used | An honest provider shows the effect, so there is no drop. Both providers lying is outside the threat model (trusted pair on 4663) |
| Wrong "another transaction took the nonce" (`Ok(Some(h))`, `h != p.hash`): a Deposit taken back as not done, paid twice | Needs every provider to agree on a block body holding a different transaction of ours at that nonce | Not reachable with one honest provider |
| Flapping pool entry (one provider shows it pending, one not) | That is `Unsure`, not `Pending`: the clock runs. With the nonce used, a pool entry is stale by definition: the nonce is used, so it cannot mine again | Correct. `Pending` on every provider still restarts the clock (`:3842-3847`; `r5_f5_m1_d_…` uses the fake's `pool` switch) |
| The effect is read before the transaction is under the pin | `evm_nonce` reads the count AT the pin (`:1370`), so "nonce used" already means mined at or under the pin. `home_effect` reads at a later or equal pin (`PIN_FLOORS` never go back) | The effect is visible if it happened |

**Residual:** none new. The operator's `eth_home_clear` is still the only unverified decision, and only for the two effect-less
steps (fixer's call 5; round 5's call 1).

## D6-L-1: `eth_home_clear` restricted (closed, canister side)

- `home_clear` (`:4502`) refuses with a reason unless `home_clear_applies` (`:4525-4531`):
  - `Step::Deposit` → true;
  - `Step::GiveBack { buy }` → the buy is a sweep;
  - everything else → false.
- That is exactly the set that `resolve_home` consumes the operator's word for: `home_effect` → `Ok(None)` (`:3904-3907`, `:3935`), less
  `Post`, which drops itself at `:3815-3821`.
- The check is at hand-in time; the word is consumed only in the `Ok(None)` branch (`:3822-3830`), so no hand-in can be accepted and
  then ignored.
- `r6_d6_l1_…` passes. A round-5 build's inbox entry for another step (possible only on a rehearsal canister) is never consumed: it is
  harmless, because nothing else signs at that nonce.

## D6-L-2: `HomeTx.sent_at` (closed, canister side)

- There is one construction of `HomeTx` in `src/`: `send_home` (`:3221`), `sent_at: Some(io.now())` in seconds (`EthNet::now`,
  `ethnet.rs:220-222`), the unit the monitor compares against (`monitor/src/ethnft.mjs:142-143`).
- `resolve_home` re-broadcasts `p.raw` and never rebuilds the record, so a resend keeps the first signing time. That is the right
  reading for "out this long".
- There is no re-sign path: fees are not bumped on 4663.
- A `HomeTx` restored from a pre-round-6 state has `sent_at: None`, and the monitor skips it (`sentAt > 0`). See part E, E7-I-2.

## F7-L-1 (Low): F6-L-1's reach margin is 417-519 s from the transaction's pin, not ~620 s; the fix's test models neither the lag nor the code

**Code:**
- `canister.rs:1072-1088`: the search starts at `from` = `HomeTx.pin`, or at `near` = pin − 4,000 only if our nonce is still unused
  there;
- `evmread.rs:61-79` / `pin.rs:54-61`: the fast pin is the lowest provider's head, rounded down to 1,024, minus 1,024;
- `ethflow.rs:3221`: `HomeTx.pin` is that pin at signing;
- the fake `tests/audit_nft_r6.rs:591-598`: its pin is the head, and it checks the depth only from `from`.

**What was measured** (read-only, `poc/F/depth_scan.output.txt`):
- 4663 makes 9.95 blocks a second.
- `rpc.mainnet.chain.robinhood.com` serves `eth_getTransactionCount` 6,198-6,234 blocks under its own head. It is a steady sliding
  window: `-32000 "historical state … not available"` beyond it.

**The arithmetic the fix skipped:**
- `HomeTx.pin` is L = 1,024-2,047 blocks under the head at signing. It can be up to 1,024 more when the providers' heads differ; the
  pin allows one bucket of spread.
- The first read of `home_nonce_tx` that can fail is `nonce_at(from)`. It succeeds only while 10·Δt + L ≤ ~6,216, that is
  **Δt ≤ 417-519 s** after signing.
- `fixes-round6.md` §4 compares "≈ 300 s after the signing tick" to "≈ 620 s of state". That is a head-relative reach against a
  pin-relative delay.

**What Δt is in practice:**
- The home transaction is signed at the END of a canister tick (step 7).
- The runner then runs its Relay and Ethereum steps (`convertOnce`, `ethNftOnce`: OpenSea listings, Dwellir logs, hand-ins).
  - If any hand-in was taken, it ticks again at once (good: Δt small).
  - Otherwise it sleeps `RUNNER_PENDING_EVERY_S` (240 s).
- The next tick reaches the lo read after about 8-10 sequential outcalls: the floor check, the receipt, the transaction by hash, the
  pin, the nonce at the pin, the reach probe.
- So Δt ≈ 240 s + the runner's steps + ~30-60 s: about 280-330 s on a quiet cycle, and **more whenever OpenSea, Relay or Dwellir are
  slow without a hand-in being taken**.
- The margin is therefore 90-240 s, not the ~320 s the fix states. `RUNNER_PENDING_EVERY_S` ≤ 300, which the runbook allows
  (`phase1-nft.md` §5.1), leaves almost none at L = 2,047.

**The reach branch (`near`)** starts the search 4,000 blocks under the pin only if the nonce is UNUSED there, i.e. the transaction
mined more than ~400 s after signing (a resend). The common case, a transaction mined seconds after it was sent, gets no help from it.
Beyond the reach, both reads fail, and a Deposit or `fundReserve` waits for the operator, as before the fix. The branch is sound: an
unused count at `near` plus a used count at the pin brackets the block, and the block's transactions (every provider agreeing)
identify it. Nothing in the harness executes it: `canister.rs` is not under the fake.

**PoC:** `poc/F/r7_f_reach.append.rs` runs the fixer's own `hidden_deposit_live` with the fake's depth set to the reach seen from the
pin (`r7_f_reach.output.txt`):

| Reach from the pin | First resolving tick after signing | Result |
| --- | --- | --- |
| 4,169 (L = 2,047) | 300 s | MINED (identified) |
| 4,169 | 420 s | **UNIDENTIFIED**: waits for `eth_home_clear` |
| 5,192 (L = 1,024) | 420 s | MINED |
| 5,192 | 540 s | **UNIDENTIFIED** |

**Effect:**
- With a slow runner cycle, a hidden Relay deposit or `fundReserve` again waits for the operator (SEV1 after an hour, then the
  explorer and `eth_home_clear`). Meanwhile the one 4663 line holds every collection's posts, records, mints and advances.
- Nothing is paid twice, and nothing is mis-identified. It is F6-L-1's own impact at a lower, cadence-dependent frequency, hence Low.

**Fix (any one):**
1. Record at signing the block the search can start from nearest the head: e.g. the agreed head bucket rather than the fast pin, or
   the pin plus the fast pin's own lag. Our nonce is unused at the pin and nothing else of ours signs meanwhile (`others_pending`).
2. Read `nonce_at(from)` FIRST in `resolve_home` when the nonce is used (before `home_seen`'s reads), and have the runner re-tick
   immediately after a tick that signed a home transaction (not after the Relay/OpenSea steps).
3. Default `RUNNER_PENDING_EVERY_S` to 120 s, and let the runbook bound be "≤ 180 s".
4. Test the real `home_nonce_tx` arithmetic (e.g. factor the start choice into a pure function over (pin, HomeTx.pin, reach,
   lag)), and give the fake's pin the fast pin's lag.

## Infos

- **F7-I-1:** `r6_f6_l1_…` "proves" the cadence fix in a fake whose pin is the head and whose `home_nonce_tx` ignores the `near`
  branch (`tests/audit_nft_r6.rs:591-598`). The canister's actual reach code (`canister.rs:1076-1083`) is exercised by no test. It
  is correct by reading (see F7-L-1), but a regression there would be silent.
- **F7-I-2:** a pending home transaction that can no longer be identified keeps the runner at the 240 s cadence for as long as it
  waits for the operator (hours to days): about 2.5 × the tick cycles for no gain once the pin is out of reach. `sleepAfter` could
  fall back to `RUNNER_TICK_EVERY_S` once `used_since` is older than, say, 15 minutes. That is part D's code; it is noted here for
  the cost.

## Round-6 items: verdicts (part F)

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **F6-L-1** (canister side) | **Partly** (F7-L-1) | The search's reach start is sound and never misidentifies. The cadence alone carries the fix, with a 90-240 s margin, not ~320 s. There is no deeper keyless 4663 provider: Robinhood's reach is 6,216 blocks, measured |
| **F6-L-2** | **Closed** | See the table above: one provider can only make the line wait |
| **D6-L-1** (canister) | **Closed** | `home_clear_applies` equals `home_effect`'s `Ok(None)` set minus `Post` |
| **D6-L-2** (canister) | **Closed** | One construction, `sent_at` at signing, kept across resends. A restored pre-round-6 `HomeTx` is not paged (E7-I-2) |
| C6-I-1 (home pin read with `?` at both deposit sites, `:2803`, `:3601`) | **Closed** | A failed read now waits a tick |
