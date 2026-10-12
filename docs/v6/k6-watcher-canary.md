# K6: watcher v2 and the custody canary (H5, G9)

Written 2026-09-25 on branch `p1-monitor` (from `56da1a0`), **before the code**. It turns `docs/v5/custody-hardening.md` H5 and
`docs/v5/launch-gates.md` G9 into what Phase 1 (`docs/v6/phase1-scope.md` §3.1 K6) runs. Nothing here deploys anything, holds a
key, or sends a transaction. Every read is a public one: anonymous IC query calls and certified `read_state`, public esplora
APIs, public JSON-RPC `eth_getLogs`.

## 1. What it is

One small always-on Node service, `monitor/`, with three checkers and one alert path:

| Checker | Reads | Alerts on |
| --- | --- | --- |
| **watcher** (timelock + buyer) | the timelock's `status()` / `history()`; both canisters' `cycles()`; the buyer's `custody_caps()` / `custody_outflow()`; both canisters' **certified** controllers and module hash (`read_state`) | every timelock event (v1's words); for every proposal, the **decoded upgrade argument diffed against the running one** and the **module hash against the published reproducible builds**; controllers or module hash changed with no `Executed`; the timelock not configured as expected; cycles levels; the tripwire freeze |
| **canary** (Bitcoin) | the custody, float and buyer addresses on **two** esplora providers (mempool.space, blockstream.info); the buyer's record queries `redemptions()`, `holder_deposits()`, `frog_buys()`, `sale_state()`, `purchases()` | **any outbound spend that no canister record names**; a matched spend whose delivery destination differs from the record; the providers disagreeing; an expected inbound (the test deposit) as confirmation; any other unexplained inbound |
| **evm canary** (4663) | `eth_getLogs` on each Wrapper and the FloorModule from **two** RPCs (C8's robinhood and ordofi), agreeing; since wave 2 also the registry and the three sweep vaults (§11) | a custodian-gated `Deposited` (mint), `Delivered` (delivery record) or `ItemArrived` that no canister record names; any `Wrapped` or `RentalReturned` (paths OFF in Phase 1); a `MintLimitProposed` (H11 loosening); the two RPCs disagreeing; wave 2: the keeper's floor, caps and draws, the pause, the guardian, role rotations, and every draw root recomputed (§11) |
| **chain**, **canister**, **liveness** (wave 2, §11) | 4663 state on two RPCs; the buyer's `ticket_logs()`, `log()`, `ord_providers()`, `holder_deposits()`; each service's `/healthz` | the keeper paused, a stale floor or cap, the stale fallback, tickets and draws nobody resolves; the ticket-log scanner failing, no UniSat, a parked deposit; a service down or refusing |

Alerts go to Telegram (`ALERT_TELEGRAM_BOT_TOKEN`, `ALERT_TELEGRAM_CHAT_ID`), deduplicated, with a daily heartbeat, and
`--test-alert <kind|all>` sends one of each kind (G9: "a test alert of each kind reaches the operator's phone"). A status
endpoint answers what each checker last saw and when.

Watcher v1 (`scripts/timelock-watch.mjs`) stays as it is: it needs `dfx`, is what a person runs from a laptop, and its tests run
against the local replica. v2 **imports v1's pure functions** (`describe`, `surprises`), so the words for an event are the same in
both, and talks to the IC through `@icp-sdk/core` (the maintained successor of `@dfinity/agent`) instead of `dfx`, so that it fits
in a small container. v1's cycles check is replaced by one with G8's second level (the buyer under 2 T).

## 2. The argument diff

**What "the running argument" is.** The buyer has no `config()` query. Its configuration after an upgrade is `restore_init(saved,
arg)` (`canister.rs:186`): `operator`, `rpc_urls`, `btc_api_urls`, `evm_chains`, `custody_caps` and `outflow_limits` follow the
upgrade argument; `wrapper` is set once from nothing; everything else (`chain_id`, `vault`, `key_name`,
`accept_production_key_name`, `bridge_allowlist`, `btc_network`) is kept from the install. So the baseline is, in order:

1. the argument of the **last executed upgrade** in the timelock's own `history()` (its `Proposed` view carries `arg_hex`, and the
   `Executed` event names its id); else
2. the **install argument** recorded at the ceremony (`install_arg_hex` in the monitor's config, the bytes of the pre-handover
   reinstall, published in the ceremony log);
3. and it is **checked against the live canister**: the baseline's effective caps and limits (`null` means stage 0: `custodycap::
   stage0` for the wrapper's key, `outflow::Limits::stage0`) must equal what `custody_caps()` and `custody_outflow()` answer now.
   A mismatch is its own alert: the monitor's picture of the running configuration is wrong, and a diff against it proves
   nothing.

**Decoding.** The argument is Candid bytes of `Init`. It is decoded with the buyer's `Init` type written out in JS, and **before**
that, its type table is read directly: every record field and variant tag hash in the bytes must be one this monitor knows. Candid
decoding silently drops fields the reader does not know (record subtyping); a new field in a new module's `Init` would otherwise
vanish from the diff. An unknown field, bytes that do not decode, or trailing bytes are all **CRITICAL**.

The fixtures are **encoded by the canister's own Rust `Init`** (`autobuyer/canister-rs/examples/watch_fixtures.rs`), so the tests
check the JS decoder against the real encoder, not against itself.

**Classification**, field by field, against the baseline's *effective* values (so `null` and an explicit stage 0 are the same):

| Field | A change is |
| --- | --- |
| `custody_caps` (per collection: items, value, per-item value; global items and value) | **loosening** (CRITICAL) if any number rises or a collection is added; **tightening** (INFO) if it falls or a collection is removed. A raise beyond ×4 is also noted as "the running code refuses this: execution will fail" |
| `outflow_limits` (items, mints, value per day; the trip factor) | **loosening** (CRITICAL) if any rises; **tightening** (INFO) if it falls |
| `operator` | CRITICAL (H5): whoever it names can call every operator path |
| `rpc_urls`, `btc_api_urls` | CRITICAL (H5): the providers are what the canister believes. Fewer providers is also called loosening |
| `evm_chains` | CRITICAL (H5); Phase 1 has none |
| identity fields: `chain_id`, `vault`, `key_name`, `accept_production_key_name`, `bridge_allowlist`, `btc_network`, `wrapper` | CRITICAL. The running code ignores them on upgrade, but a new module may not; they should never differ, and a proposal that changes one needs an explanation |
| nothing | "no configuration change" (a code-only upgrade): INFO, unless the module is unknown |

**The module hash.** Every `upgrade` proposal's `wasm_hash` is compared with the published reproducible-build hashes
(`deployments/canister-ic.json`, section `builds`: `{ commit, buyer, timelock }`). Unknown means **CRITICAL** ("unknown module:
veto unless a reproducible build of an announced commit prints this hash"), as in the guardians' standing instruction 2.

**Controllers and roles proposals.** A `controllers` proposal is always CRITICAL, with the proposed list beside the certified
current one. A `roles` proposal is diffed against `status().roles`: fewer guardians, a lower `k`, a new proposer or a replaced
guardian is loosening (CRITICAL); anything else still alerts (WARN).

**The alert** is plain English, one line per change, loosening first. The loosening fixture renders as (abridged):

```
🚨 SEV1 · proposal
PROPOSAL 7 (upgrade) by <p1> on the buyer timelock <id> — CRITICAL
can run from 2026-09-28T04:52:49Z (72.0 h after it was proposed)
module ffff…ffff (1603201 bytes): UNKNOWN — not a published reproducible build. Veto unless a reproducible build of an announced commit prints this hash
argument sha256 eeee…eeee
diff against the install argument recorded at the ceremony:
  LOOSENS custody cap btc:0x…aa: items 3 → 12
  LOOSENS custody cap btc:0x…aa: value 10 ETH → 40 ETH
  LOOSENS custody cap, global: items in all 6 → 24
  LOOSENS outflow limit: items out a day, per collection 3 → 12
  LOOSENS outflow limit: tripwire factor (acts before custody freezes, × the limit) 3 → 4
  unchanged: chain_id, vault, key_name, accept_production_key_name, bridge_allowlist, btc_network, wrapper, operator, rpc_urls, btc_api_urls, evm_chains
Guardians: veto unless this exact hash and diff were announced (H6); pause first if anything here is unexplained (G2 standing instruction 1).
```

Every pending proposal is re-judged on every poll, and a SEV1 one repeats every 6 hours until it is vetoed, withdrawn or run.

## 3. Certified controllers and module hashes

Each poll reads `/canister/<id>/controllers` and `/canister/<id>/module_hash` for the buyer and the timelock through
`read_state` (the IC's certificate is verified by the agent against the IC root key: this is what `dfx canister info` shows, and
a boundary node cannot forge it). Alerts:

- the buyer's controllers are not exactly `[timelock]`, or the timelock's are not `[]` (blackholed): CRITICAL, every poll until
  fixed (deduplicated per distinct wrong value);
- the buyer's module hash is not a published buyer build, or the timelock's is not the published timelock build: CRITICAL;
- either changed since the last poll **and** no new `Executed` event appeared in the same poll: SEV1 (G10: "a controller or
  module-hash change without `Executed`"). One exception, because `execute` installs the module *before* it records `Executed`:
  if a pending proposal that is past its delay names exactly the new hash (or controller list), the change is held for one poll,
  and becomes SEV1 if `Executed` still has not appeared.

## 4. The custody canary

**Order matters.** The canister saves a transaction's bytes (and so its txid) **before** it first broadcasts them (H4, `redeem.rs`
`Signed`; `frogflow` `Signed`). So the canary reads the explorers **first** and the canister's records **second**: any outbound
transaction the explorers show was saved by the canister before it could have been seen. Reading in the other order would race.

**Outbound** means a transaction with an input whose prevout pays a watched address. It **matches** when its txid equals a txid
the canister recorded for one of its own spends:

| Record | Txids it vouches for |
| --- | --- |
| `redemptions()` | `Signed`, `Sent`, `Confirmed`, `AwaitingRecord`, `Recording`, `Done`, `LostRace` (a delivery; output 0 must also pay `destination`) |
| `holder_deposits()` | `Returning`, `Returned` (a refused deposit's return) |
| `frog_buys()` | `buy.outpoint.txid` in `Signed` … `Stranded` (a float spend buying a frog) |
| `sale_state()` | `Moving`, `Sold.fill_txid`; and any spend of a `Listed` / `Recorded` / `Delisting` outpoint (a listing someone filled). YOYO is OFF in Phase 1 |
| `purchases()` | a v2 buyer-address purchase whose phase text names the txid. OFF in Phase 1 |

A txid commits to every input and output, so a match means these are exactly the bytes the canister signed and saved. Anything
else is an **unexpected outbound transaction**: SEV1, with the transaction, the amounts, the destinations, both explorer links and
the words "PAUSE NOW" (the guardians' standing instruction 1). A delivery whose txid matches but whose output 0 does not pay the
record's `destination` is also SEV1 (it cannot happen unless the records are wrong).

**Two providers, agreeing; never a guess.** Each address is read from both explorers.

- A tx both show: judged as above.
- A tx only one shows: if it is an outbound spend no record names, that is **SEV1 provider-disagreement**: "one provider reports an
  unexplained spend, the other does not show it; not guessing which is right: check both now". Otherwise it is given 15 minutes
  (mempool propagation), then **SEV2 provider-disagreement**.
- Both show it confirmed at different block hashes: SEV2 provider-disagreement (a reorg in progress or a lying provider).
- A provider that does not answer is not a disagreement: it is a `watcher-error` after 30 minutes of silence. While only one
  answers, an unexplained outbound it reports is still SEV1 (marked "one provider only").
- G9's "persisting over 1 hour": a disagreement still open after an hour is re-sent once as SEV2 "still disagreeing".

**Inbound.** A tx paying a watched address that spends none of them. It is explained when a record names it (a frog purchase's
`buy.outpoint.txid` landing at custody; a holder deposit's outpoint). An **expected inbound** (config `expect_inbound: [{ address,
min_sats, note }]`, set for the G2/G10 drill's test deposit) is announced as ✅ confirmation, once. Anything else is INFO
"unexplained inbound" (nothing left custody; somebody sent something there).

**The first run** judges the whole history of every address (esplora pages of 25, up to a bound), so a canary started late still
checks every spend. Txids already judged are kept in the state file.

## 5. The 4663 canary

`eth_getLogs` from the two C8 RPCs over `[cursor, min(head_a, head_b) − 20]`, in chunks of 5,000 blocks, for the Wrapper(s) and
the FloorModule. The two answers must be the same set of `(tx, logIndex, topics, data)`; otherwise SEV2 disagreement and the
cursor does not move. Then:

| Event (emitter) | Who can emit it | Matched when |
| --- | --- | --- |
| `Deposited(frogId, inscriptionId, txid, vout, to)` (Wrapper; a mint) | the custodian only | a `frog_buys()` record in `Held`/`Minted`/`Settled` with `buy.outpoint = txid:vout` and the same inscription, or a `holder_deposits()` record (`Ready`/`Minted`) or sale record with that outpoint |
| `Delivered(ticketId, acquisitionId, txid)` (Wrapper) | the keeper **or** the custodian | a redemption for `ticketId` whose recorded txid is `txid`. A keeper-made record with no delivery behind it (T4) is exactly this alert |
| `ItemArrived(item, cost, released)` (FloorModule) | the custodian only | an unconsumed `frog_buys()` record targeting that module (`Sink(module)`), `Minted`/`Settled`, with `cost_wei = cost`; each record is consumed once |
| `Wrapped`, `RentalReturned` (Wrapper) | — | never in Phase 1 (paths OFF): SEV2 |
| `MintLimitProposed` (Wrapper) | the Safe | always alerts (SEV2): H11's limit is being raised |

An unmatched mint, delivery record or arrival is SEV1 ("an unbacked mint" in G10's table). Records are read **after** the logs,
for the same reason as §4: the canister saves an EVM transaction before sending it.

## 6. Alert kinds, severity, dedup, heartbeat

| Kind | Examples | Severity |
| --- | --- | --- |
| `proposal` | a proposal with its argument diff and module check | SEV1 if CRITICAL, else INFO |
| `governance` | withdrawn, veto vote, vetoed, executed, failed, paused / resumed, resume vote, roles changed | INFO (paused: SEV2) |
| `controllers` | certified controllers or module hash wrong, or changed without `Executed` | SEV1 |
| `config` | the timelock not as expected (v1's `surprises`); baseline ≠ live caps/limits; the tripwire freeze | SEV1 / SEV2 |
| `custody-outbound` | an unexpected outbound spend; a matched delivery to the wrong destination | SEV1 |
| `custody-inbound` | the expected test deposit seen (✅); an unexplained inbound | INFO |
| `provider-disagreement` | Bitcoin explorers or 4663 RPCs disagree | SEV1 (unexplained spend on one side) / SEV2 |
| `evm-mint` | a 4663 mint, delivery record or arrival no canister record names; a path that is OFF used; H11 proposed | SEV1 / SEV2 |
| `cycles` | below the canister's own threshold; buyer below 2 T (G8) | SEV2 |
| `watcher-error` | a checker has not completed for 30 minutes (IC, explorer or RPC unreachable); the 4663 cursor stalled | SEV2 |
| `heartbeat` | once a day: what is watched, last good poll per checker, alerts sent in the last 24 h | INFO |
| `keeper` (wave 2) | a floor post outside the module's bounds, low confidence, a YOYO or out-of-range cap; the keeper paused; a stale floor or cap; the stale fallback; a role rotation proposed | SEV1 / SEV2 |
| `draw` (wave 2) | a draw announced, opened, verified (INFO); a bad root (SEV1); not verified before its window ends, or not opened / resolved an hour late (SEV2); at the deadline (`draws.deadline_s` before the pin) still unverified: SEV1, and the guardian pauses **only on evidence** (a caught-up index computing another root, a dissent confirmed after a rebuild, a failure no degraded state explains), otherwise the page is escalated by hand every poll and the monitor's `/healthz` answers 503 (round 4, `docs/audit/2026-09-26/review3/fixes-R4-services.md` §3; `draws.pause_on_degraded` restores the fail-closed rule) | SEV1 / SEV2 / INFO |
| `guardian` (wave 2) | what the guardian did (SEV1, with the tx), that it failed or is off (SEV1 "act by hand" with the Safe's exact call) | SEV1 |
| `liveness` (wave 2) | a service's `/healthz` not 200 for 10 minutes; its return | SEV2 / INFO |
| `canister` (wave 2) | the ticket-log scanner failing an hour, `TICKET LOGS NOT READ`, no UniSat provider, a parked deposit | SEV2 / INFO |

**Dedup.** Every alert has a key (`custody-outbound:<txid>`, `controllers:<canister>:<value>`, `proposal:<id>`, …). A key is sent
once; a SEV1 key whose condition persists is repeated every 6 hours until it clears (silence is not safety). A failed post leaves
the key unsent, so the next poll retries. The state file (on a Railway volume, `/data`) keeps keys, judged txids, the EVM cursor
and the last certified values.

**Test alerts.** `node monitor/src/main.mjs --test-alert all` (or one kind) renders each kind from a fixture with the real
formatter, prefixed `🧪 TEST — no action needed`, and posts it. It reads no network other than Telegram and touches no state.

**Secrets.** The bot token is read from the environment, used only in the request path, scrubbed from every error, and never
logged, printed, put in the status endpoint or written to the state file. There is no other secret: every read is public.

## 7. Where it runs, and why not on the keeper's host

A Railway service of its own (`monitor/Dockerfile`, `monitor/railway.json`), in a **different Railway project and account from the
keeper and the operator runner**, with a volume for its state and one HTTP port for `/status` and `/healthz`.

Wave 2 (audit E-16): the image runs as the `node` user. Railway mounts the volume owned by root, so the entrypoint alone runs as
root, makes `/data` writable by `node` and drops to it (`ops/docker/drop-to-node.sh`).

Why not beside the keeper:

- **The keeper host holds the keys the canary watches for misuse** (the keeper key and the operator identity: G2's table). If
  that host is compromised, a canary on it can be silenced or taught to ignore exactly the spends the attacker makes (T4, T5, T6).
  G10 lists "the keeper or operator host suspected" as an incident; the canary must be what reports it, not a casualty of it.
- **Failure independence.** A keeper outage (a crash loop, a full volume, a bad deploy) must not also silence the alarm that
  would notice the consequences.
- **Least privilege.** The monitor holds one secret (the bot token), no key, and makes only public reads. Co-locating it gives an
  attacker who reaches either one the other's privileges.

The honest limit: a Railway-wide compromise or outage still takes both down. The heartbeat is the check on that: a missing
daily heartbeat is itself the alarm, and the operator treats one as SEV2.

## 8. What it does not do (and where that lives)

- **The ledger reconciliation** (`totalSupply + openTickets × 1e18 == held × 1e18`): K7, hourly, with the reserves page.
- **4663 governance beyond the wrapper**: since wave 2 the registry's pause, guardian and role rotation and the vaults' caps and
  draws are watched (§11); the other `Propose*` steps (roots, BTC vaults, treasury, bond) are still read only by
  `golive-check.sh`, whose `--phase1` fails on any pending one.
- ~~**drand**~~: since wave 2 the monitor reads the tickets and draws itself and pages one Pinned an hour past its round (§11).
- **The EVM custody address on other chains** (ERC-721 `Transfer(from = us)`): the NFT side is OFF in Phase 1.

## 9. Tests are the spec

`monitor/test/` (Node's runner, no network):

- argument diff: Rust-encoded install, neutral (identical, and `null` vs explicit stage 0), tightening, loosening (stage 0 → 1),
  operator swap, provider swap, identity change, an unknown field, garbage bytes; the rendered words;
- module hash known / unknown; controllers and roles proposals;
- certified controllers / module hash changed with and without `Executed`;
- canary on real mainnet transactions (an address with one inbound and one outbound): the spend matching a delivery record (no
  alert), the same spend with no record (SEV1), the destination differing, provider disagreement (one-sided unexplained spend →
  SEV1; one-sided matched spend → wait, then SEV2), a provider down, the expected test inbound, an unexplained inbound;
- 4663: a mint matching a frog record, one matching nothing, a keeper-made delivery record with no delivery, an arrival consumed
  once, RPC disagreement;
- alerts: dedup, the 6-hour SEV1 repeat, a failed post retried, the heartbeat once a day, `--test-alert all` posts one of each
  kind against a mock Telegram with a fake token that never appears in any output or file.

## 10. Running it (the operator)

**What it needs, all public except the token:**

- `deployments/canister-ic.json` gains a `builds` list, written at ceremony step 4 from both devices' reproducible builds:
  `"builds": [{ "commit": "<sha>", "buyer": "<buyer.wasm sha256>", "timelock": "<timelock.wasm sha256>", "timelock_rehearsal":
  "<timelock-rehearsal.wasm sha256>" }]`. Every later release appends its entry **before** its proposal is made (H6), so a
  proposal of an announced build is recognised and anything else is UNKNOWN.
- The monitor config (`monitor/config.example.json`): the two canister ids, the expected roles, `install_arg_hex` (the
  pre-handover reinstall's argument, from the ceremony log), the three Bitcoin addresses, the Wrapper and FloorModule, and the
  4663 `from_block`. It refuses to start with a placeholder, with fewer than two providers per chain, or without published builds.
- `ALERT_TELEGRAM_BOT_TOKEN` and `ALERT_TELEGRAM_CHAT_ID`: a bot made with BotFather for this purpose only, and a chat the
  operator (and, later, every guardian) is in.

**Railway** (a project of its own, not the keeper's): one service from this repo with root directory `/`, config file path
`/monitor/railway.json` (it builds `monitor/Dockerfile` with the repo root as context), a volume mounted at `/data`, and the
variables `MONITOR_CONFIG_JSON` (the config, inline), `ALERT_TELEGRAM_BOT_TOKEN`, `ALERT_TELEGRAM_CHAT_ID` (both required: without
them it refuses to start) and, only with a guardian, `MONITOR_GUARDIAN_KEY`. `/healthz` is 503 once any checker has been silent for
30 minutes or is degraded (a range disputed for an hour, a stalled cursor); Railway reads it **only when it deploys** (it restarts
a service only when the process exits), so a monitor that is up but blind is caught by the missing heartbeat, not by Railway.
`/status` is the JSON the heartbeat summarises.

**The test alerts (G9).** Either on the service itself (`railway ssh`, then `node monitor/src/main.mjs --test-alert all`), or
from the operator's laptop with the same two variables set:

```
cd monitor && npm ci
ALERT_TELEGRAM_BOT_TOKEN=… ALERT_TELEGRAM_CHAT_ID=… node src/main.mjs --test-alert all      # 16 messages, one per kind
node src/main.mjs --test-alert all --dry-run                                                  # print them instead
node src/main.mjs --test-alert custody-outbound                                               # one kind
```

Each arrives marked `🧪 TEST — no action needed.` G9 is met when all 16 have reached the operator's phone (and, once there are
guardians, every guardian's) **and** the reconciler's own three (`cd ops && node --import tsx src/reconcile.ts --test-alert`, with the
reconciler service's variables): the two services have separate variables, and each must be tested (audit E-08b, E-15).

**Acknowledging.** An explained SEV1 (an unexpected spend or mint that turned out to have a cause) repeats every 6 hours until
its txid is added to the config's `acknowledged` list. The list, with a line of explanation per entry in the incident log, is
the "every canary alert explained" record that stage 1 requires (`phase1-scope.md` §4).

## 11. Wave 2 (2026-09-26): the keeper on chain, draws, the guardian, liveness

Design and reasons: `docs/audit/2026-09-26/fixes-W2-monitor.md` §1. In short:

- **Configuration.** `evm.stack` (the stack file, inline) or `evm.stack_file` replaces the list of wrappers and the module: the
  monitor derives the registry, every wrapper, module and sweep vault from it. `draws.exclusions` is `tools/snapshot/exclusions.json`
  copied in. `liveness` lists each service's `/healthz` (keeper, standalone resolver, runner, reconciler, indexer; the keeper's `/healthz` is
  503 while it refuses or its last pass is old, and carries counts only). `evm.guardian` (optional) names the guardian's address.
- **Events** (`src/evm.mjs` `judgeStackLogs`): a `FloorPosted` whose target the module clamped (the keeper asked for more than
  `floorBounds()`) is SEV1, and two in the same direction within 6 h are a **runaway floor**; low confidence is SEV2; a cap on the
  YOYO vault is SEV1, a RAFFLE/BURN cap above 125 % of the floor SEV2; `KeeperPaused` SEV1; `GuardianSet` SEV2; `RolesProposed` SEV1.
- **Draw roots** (`src/draws.mjs`): every `DrawOpened` is recomputed from the coin's `Transfer` logs at the chain's `snapshotAt`
  with the keeper's exclusions and the v2 sentinel leaf, on **both** RPCs. Both reproducing another root is a **confirmed bad
  root** (SEV1). The tree code is held to `tools/snapshot`'s committed fixture byte for byte.
- **The guardian** (`src/guardian.mjs`, off unless configured): on a confirmed bad root it calls `cancelDraw(id)` (while the draw
  is Open or Announced) and `pauseKeeper()`; on a runaway floor, `pauseKeeper()`. It checks the chain id, its own code and the
  registry's guardian on both RPCs before it signs, simulates, caps gas and fee, and sends to both RPCs. Without it, the same
  triggers page "ACT BY HAND" with the Safe's exact `cast send`.
- **State** (`src/chain.mjs`): `keeperPaused()` and `escaped()` at once, a floor not fresh for 15 minutes, no good post for 45, a
  RAFFLE/BURN cap stale for 15 while the floor is fresh, a ticket or draw Pinned an hour past its round, a draw Announced an hour
  ago — each confirmed on the second RPC, persistent.
- **E-06**: a disputed range is kept aside and re-read; the logs both RPCs return in it are judged at once; the cursor goes on; a
  dispute repeats every 6 h (and "STILL" after an hour) and an hour-old one makes `/healthz` 503.
- **The canister** (audit D): the decoder knows `holder_deposits` and `wrapper_from_block` (switching deposits on is CRITICAL),
  `Flow::Return`, `Outflow.acked`, and the new record variants; the Bitcoin canary vouches for every version of a delivery
  (`bump.earlier`), every conflict-out and the float's abandon conflicts; the timelock's `NotStopped` (SEV2) and `NotStarted` (SEV1).
- **Fixtures** are regenerated by `autobuyer/canister-rs/examples/watch_fixtures.rs`; the audited build's encodings are kept in
  `monitor/fixtures/canister-pre-audit-d.json` and must still decode. The stack events are encoded by `cast`
  (`monitor/fixtures/gen-stack-logs.py`).
