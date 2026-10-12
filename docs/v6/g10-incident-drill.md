# G10: the incident drill, as the operator runs it

Written 2026-09-25 on branch `p1-monitor`. This is the runbook for `docs/v5/launch-gates.md` G10's one end-to-end drill and G2's
"Day +1 to +7" drill, under the solo ceremony of `docs/v6/phase1-scope.md` §2: **p1 and p2 on device A; g1, g2, g3 on device B;
k = 2**. At every step it names the alert the K6 monitor (`docs/v6/k6-watcher-canary.md`) must post, so the drill tests the alarms
as well as the keys. Nothing in it is done by the developer: every call below is the operator's, on the operator's devices.

It is run **after the handover** (go-live checklist step 12), on the real canisters, because the point is to prove the real
identities, the real timelock and the real alert path. Every action in it is harmless by construction: the proposal hands the
buyer to the controller it already has, and it is vetoed; the pause stops only the tick, and custody holds nothing yet.

## 0. Before starting

| # | Check | Pass |
| --- | --- | --- |
| 0.1 | The monitor is running on its own host (not the keeper's), and `GET /status` shows `"ok": true` with every checker's `lastOk` under 2 minutes old | yes / no |
| 0.2 | G9: `node monitor/src/main.mjs --test-alert all` (`k6-watcher-canary.md` §10) has put **16** messages on the operator's phone, one of each kind, each starting `🧪 TEST`; and the reconciler's own `--test-alert` its **3** (`services-v1.md` §4) | 16 / 16, 3 / 3 |
| 0.3 | The monitor's config has an `expect_inbound` entry for custody (`{ "address": "custody", "min_sats": 1000, "note": "G10 drill test deposit" }`), redeployed | yes / no |
| 0.4 | An incident log is open (a plain file or the ceremony log): every step below gets a UTC time, who acted, and the time its alert arrived | yes / no |
| 0.5 | Device A has `p1`; device B has `g1`, `g2`, `g3`. Both run `dfx identity whoami` and read out the principal, and it matches `deployments/roles.json` | yes / no |
| 0.6 | The monitor's config names the stack (`evm.stack`), and `/status` lists `chain` and (if the site and services are up) `liveness` among its checkers; if O11 chose a guardian, the start line says `guardian … ARMED` | yes / no |

Write `TL=<timelock id>` and `BUYER=<buyer id>` on both devices. Every call below uses `--network ic`.

## 1. The canary sees custody (the test deposit)

**Device A (or any wallet the operator controls):** send about 2,000 sats of plain bitcoin (no inscription) to the custody
address. This is the only transaction in the drill, and it is the operator's, not the canister's.

**Alert expected**, within one poll of the explorers showing it (mempool first):

> ℹ️ INFO · custody-inbound — ✅ EXPECTED INBOUND seen at custody (bc1p…): G10 drill test deposit

**Fail** if nothing arrives within 10 minutes of the transaction appearing on mempool.space, or if it arrives as
`unexplained INBOUND` (then the `expect_inbound` entry was not deployed: fix it, it is still a pass for the canary's eyes).

## 2. A harmless proposal (device A)

```
dfx canister --network ic --identity p1 call $TL propose_controllers "(vec { principal \"$TL\" })"
```

It hands the buyer to the timelock, which already controls it. Note the proposal id it prints (`id = N`).

**Alert expected**, within one poll (60 s):

> 🚨 SEV1 · proposal — PROPOSAL N (controllers) by <p1> … — CRITICAL
> NEW CONTROLLERS for the buyer: <TL> (now, certified: <TL>) …

It is SEV1 **by design**: every controllers proposal is. The drill measures the time from this message to step 3.

## 3. Pause, from device B (one guardian)

```
dfx canister --network ic --identity g1 call $TL pause_target
dfx canister --network ic --identity anonymous call $BUYER paused --query        # (true)
```

**Alert expected:**

> ⚠️ SEV2 · governance — the guardians PAUSED the buyer at <time>

**Pass:** the pause lands **within 15 minutes** of step 2's alert arriving (G10: SEV1 → pause within 15 minutes). Write both times
in the log.

## 4. Veto the proposal with two guardian identities (device B)

```
dfx canister --network ic --identity g1 call $TL veto "(N : nat64)"             # (false): 1 of 2
dfx canister --network ic --identity g2 call $TL veto "(N : nat64)"             # (true): vetoed
dfx canister --network ic --identity anonymous call $TL status --query          # pending = vec {}
```

**Alerts expected**, in order:

> ℹ️ INFO · governance — guardian <g1> voted to VETO proposal N (1 of 2)
> ℹ️ INFO · governance — 🛑 … proposal N VETOED at <time> by <g1>, <g2>

After this, step 2's SEV1 stops repeating (it repeats every 6 hours only while the proposal is pending).

## 5. Resume with two guardian identities (device B)

```
dfx canister --network ic --identity g1 call $TL resume_target                  # (false): 1 of 2
dfx canister --network ic --identity g2 call $TL resume_target                  # (true): resumed
dfx canister --network ic --identity anonymous call $BUYER paused --query        # (false)
```

**Alerts expected:**

> ℹ️ INFO · governance — guardian <g1> voted to RESUME the buyer (1 of 2)
> ℹ️ INFO · governance — the guardians RESUMED the buyer at <time>

A drill meets G10's resume conditions trivially (the cause is the drill, and nothing needed fixing), but the resume still takes
two identities: that is what this step proves.

## 5b. The keeper's pause (4663), before any value is in the stack

`fixes-B-C-keeper-draws.md` §0: one pause stops the keeper everywhere. This proves the switch, the monitor's alarm and the Safe's
unpause, on the real stack, while it holds nothing (before step 13 opens the site). It changes nothing a holder could see.

1. **The Safe (A+B)** sends `ListingRegistry.pauseKeeper()` (or, if O11 chose a guardian, the monitor's guardian may be the one: it
   pauses only on a trigger, so for the drill the Safe does it).

   **Alerts expected**, within two polls: `🚨 SEV1 · keeper — THE KEEPER IS PAUSED, by <Safe>` (the event), then the state's
   `THE KEEPER IS PAUSED (for …)` (persistent). The keeper service's `/status` shows it refusing to send (`keeperPaused`), and after
   10 minutes the monitor's `liveness` pages the keeper's `/healthz` 503 — expected here.
2. **The Safe (A+B)** sends `ListingRegistry.unpauseKeeper()`.

   **Alert expected:** `⚠️ SEV2 · keeper — the keeper was UNPAUSED by <Safe>`; the keeper posts again within its next pass, and
   `module.fresh()` returns true within 30 minutes.

**Pass:** both alerts arrived, and the floor is fresh again. Record the pause → alert time.

## 6. What must NOT have happened

Read the phone and `GET /status` once more:

- no `custody-outbound`, `evm-mint` or `controllers` alert during the drill (nothing left custody, nothing was minted, and nothing
  executed, so the certified controllers and module hash did not move);
- no `provider-disagreement` lasting past the drill;
- `/status` still `"ok": true`.

Any of these present is a finding: stop, and treat it as the real incident it would be.

## 7. Pass, and the record

| Step | Action | Alert | Pass |
| --- | --- | --- | --- |
| 1 | test deposit to custody | custody-inbound ✅ | alert ≤ 10 min after mempool |
| 2 | p1 proposes (A) | proposal SEV1 | alert ≤ 2 min |
| 3 | g1 pauses (B) | governance SEV2 PAUSED | pause ≤ 15 min after step 2's alert |
| 4 | g1 + g2 veto (B) | VetoVote, then VETOED | both alerts; `pending` empty |
| 5 | g1 + g2 resume (B) | ResumeVote, then RESUMED | both alerts; `paused()` false |
| 5b | the Safe pauses, then unpauses, the keeper (4663) | keeper SEV1 PAUSED, then SEV2 UNPAUSED | both alerts; `fresh()` true again |
| 6 | — | none of the forbidden kinds | clean |

The log (times, identities, alert arrival times, the step-2 → step-3 interval) is published with the ceremony log. Then set
`phase1-scope.md` §3.2's G10 row to **done** with the date, and remove the `expect_inbound` entry (a second deposit would
otherwise be announced as expected).

## 8. If a step fails

- **No alert** at a step where one is expected: the drill stops. Check `/status` (`lastError` per checker). A monitor that did
  not see a pause will not see a theft; G9 is not met until the step passes on a re-run.
- **A call is refused** (`not a guardian`, `not a proposer`): the identity on that device is not the one in `roles.json`. That
  is a ceremony failure (G2), not a drill failure: stop and resolve it before any value enters custody.
- **The pause took longer than 15 minutes**: record why (device B asleep, password forgotten, dfx not installed). Fix it and
  repeat steps 2 to 5; the monthly repeat (G2 solo rule 3) uses the same runbook.
