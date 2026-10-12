# Integration of fix round 3: contracts, canister and services on one tree

Branch `r3-integrate` (from `fixes-2026-09-26` at `9d52859`, which merges `r3-contracts`, `r3-canister` and `r3-services`, each
built in parallel against the others' designs). Written 2026-09-26. Nothing is deployed; no key, wallet or Railway service was
touched; network use was read-only (the public 4663 RPCs, the public drand relays, local anvil forks).

Inputs: `fixes-R3-contracts.md` (§8 and "Still open"), `fixes-R3-canister.md` (§9, "Still open"), `fixes-R3-services.md` ("Left
open").

## 0. Summary

| # | What was out of sync | Fixed by |
| --- | --- | --- |
| 1 | The canister's `markUndeliverable` call was coded against a working tree of the Wrapper | Verified against the committed contract: selector `0xacf756cc`, one `uint256`, custodian-only, `Resolved`-only, sent only for a refused redemption. Held on both sides to one `cast calldata` fixture (§1) |
| 2 | The keeper announced a coin whenever someone held a whole coin *now*; with the seed carry (N-2) an empty snapshot moment would be carried by every later draw of the coin for good | The keeper announces only when every moment the snapshot time can land on has a ticket; with a carried seed it reads the carried snapshot time on chain and refuses an empty one (§2) |
| 3 | The monitor's holder index folds logs older than 36 h; a carried draw can open days after its origin's window | Holds: a coin with a draw before its pin, or a carried seed, is never folded past its origin's window; a coin already folded past it is rebuilt from its launch (§2) |
| 4 | New views, events and errors of `r3-contracts` were missing from the keeper, monitor, indexer and site | Added everywhere, checked against `out/*.json` (§3) |
| 5 | The monitor could not decode the R3 timelock's history (`Started` is a new variant tag: an unknown tag fails the whole decode) and did not know the canister's new fields or alert lines | Candid types, alert rules, a canary match for `Undeliverable`, and `fixtures/canister.json` regenerated from the canister's own types (§4) |
| 6 | `golive-check.sh` assumed payee = custodian, 1e18 a unit and an unpaused listing, and looked for `itemUnits`/`listingPaused` at the top level (DeployStack writes them per collection) | Reads the stack file's `payee`, per-collection `itemUnits` and `listingPaused`; the fee pool must hold one usable entry ≥ 100,000 sats (§5) |
| 7 | The dry run's stand-in dfx answered `custody_caps()` for every method, so the new fee-pool read failed | The stand-in answers `custody_fee_pool()`; a new case refuses a pool without a 100,000-sat entry (§7) |
| 8 | Refund dates, the Trust page and the runbook still said a resolved ticket refunds at deliverBy + 72 h and that burns wait 7 days in a keeper incident | Site, indexer, docs and `phase1-scope` §3.4 updated; `gen-delivery-fixture.mjs` says version 3 (§6) |

## 1. Canister ↔ Wrapper: `markUndeliverable`

Checked against the committed `src/Wrapper.sol` and Foundry's artifact:

- **Selector and arguments.** `markUndeliverable(uint256)` = `0xacf756cc` (`cast sig`, and `methodIdentifiers` in
  `out/Wrapper.sol/Wrapper.json`); one `uint256`, nonpayable. `redeem::mark_undeliverable(id)` encodes exactly that (4 + 32 bytes,
  the id big-endian).
- **Who and when.** The contract: `msg.sender == custodian` (else `NotCustodian`) and the ticket `Resolved` (else `WrongStage`).
  The canister: attests only a redemption it **refused** (`Attest::Due`), after reading the ticket fresh (`Resolved`, else it
  records `Done` "not needed"), only if `may_record()` (it is the wrapper's custodian; it estimates gas from its own address), one
  EVM transaction of ours at a time, saved then sent, not while paused; a refusal with nothing ever broadcast at once, a
  conflicted-out delivery once the conflict is `ATTEST_DEPTH` = 6 deep. A delivery that confirms first is recorded as before.
  This matches the contract's intent (`fixes-R3-contracts.md` §4, option 3).
- **Tests added.** `test/fixtures/canister/mark-undeliverable-7.hex` is `cast calldata "markUndeliverable(uint256)" 7`.
  `test/audit2/R3CanisterCall.t.sol` (2 tests) holds it to `abi.encodeCall(Wrapper.markUndeliverable, (7))` and runs the canister's
  bytes against a real Wrapper: refused on a Pinned ticket (`WrongStage`), from anyone but the custodian (`NotCustodian`), accepted
  from the custodian on a Resolved ticket (`Undeliverable` emitted). `autobuyer/canister-rs/tests/wrapper_abi.rs` (2 tests) holds
  `redeem::mark_undeliverable(7)` to the same fixture and, when `out/` is built, every Wrapper selector the canister uses
  (`markUndeliverable`, `recordDelivery`, `ticket`, `ticketCount`, `custodian`, `registry`, `listingId`), the `ticket(uint256)`
  struct's field order that `decode_ticket` reads, and the custodian-only / Resolved-only guard in the source.

Nothing had to change in the canister: its calldata and conditions already matched. The canister's build inputs are unchanged since
`c2fa7ae` (only `tests/` and `examples/` gained files), so the reproducible-build hashes of `fixes-R3-canister.md` §9 stand:

```
36ae6e24876d3b697f47c68bf3fcdd7e054cd94f4791fac60707f261c575ef59  buyer.wasm
92248ca7ecb160c093665a8c19b7894cdf7f4a13f74746f3794f4a6e972c48f1  timelock.wasm
742bf4748d805fe47c8ef081076054ce783f90e7774243fa554cd4561b9758b1  timelock-rehearsal.wasm
```

## 2. `snapshotTimeOf` and the seed carry

`SweepVault.snapshotTimeOf(id, r)` hashes the draw's seed **origin** and counts back from the origin's `announcedAt`
(`SeedCarried`). Every service now reads the time on chain and none computes it from `announcedAt`:

| Service | Reads | Change |
| --- | --- | --- |
| keeper | `snapshotTimeOf(id, randomness)` before building the snapshot (already); `seedOrigin(id)` for its plan; `carriedSeed(coin)` before announcing | see below |
| monitor | `draw(id).snapshotAt` (fixed by `openDraw`) and `SnapshotFixed` (already); `seedOrigin`, `carriedSeed` for pages and holds | pages and holds |
| tools/snapshot | `draw(id).snapshotAt` (already); `seedOrigin` to say a draw carries another's seed; a Voided/Cancelled draw with no snapshot says its seed passed to the coin's next draw | message |
| indexer | `SnapshotFixed` only (already); `SeedCarried` (held until its `DrawAnnounced`, same transaction) → `seedOrigin`; `DrawCancelled` and the void of an Announced draw → `seedPassedOn` | fields |
| site | the draw page says "reuses draw N's seed round and snapshot time" / "the coin's next draw reuses this draw's seed" | text |

**Found in integration (keeper): an empty snapshot is carried for good.** `openDraw` refuses zero tickets. The keeper announced a
coin whenever somebody held a whole coin *at the head*, but the snapshot time lands anywhere in the 24 h before the announcement: a
young coin (whole coins only bought in the last hours) could draw an empty moment. Before R3 the void re-rolled the time; with the
carry, every later draw of that coin inherits the same empty time, is voided, and carries it again, and only a pin (which needs an
opened draw) clears it: the coin's prize is never drawn and every day costs an announcement and a void. Fixed in the keeper:

- **New seed:** `firstEmptyBlock` (pure, `keeper/service/draws.ts`) replays the coin's transfers from the last block at or before
  `now − SNAPSHOT_WINDOW` and returns the first block where no non-excluded holder has a whole coin; the coin is not announced until
  the whole window has tickets. In the stack's first day the window reaches before the stack's first block: nothing is announced
  (a plan, not a thrown error).
- **Carried seed:** the keeper reads `carriedSeed(coin)`, proves the origin's (public) seed round through the relays, reads
  `snapshotTimeOf(origin, randomness)` on chain, counts the tickets there, and refuses to announce an empty one with the alert
  `draw-carried-empty` (the owner must act: see §8).
- The ticket count before announcing no longer builds a tree (`snapshot()` throws on a coin whose holders all hold less than one
  whole coin, which aborted the collection's whole draw pass).

**Found in integration (monitor): the index forgot a carried window.** The holder index folds logs older than 36 h
(`SNAPSHOT_WINDOW` + `OPEN_TTL` + 6 h). A carried draw opens up to a day or more after its origin (the void at 6 h, the next slot),
and its snapshot is in the origin's window, so the index could no longer replay it: the draw would stay unverified and the
deadline would pause the keeper. Now each poll computes, from the vaults' state, a **hold** per coin (the L2 block at or before the
origin's `announcedAt − 25 h`) for every coin with a draw before its pin and every coin whose `carriedSeed` is set; compaction never
folds a held coin past it, a coin already folded past it is rebuilt from its launch, and the holds are kept in the state.

## 3. ABIs

| Package | Added |
| --- | --- |
| keeper (`service/abis.ts`) | `SweepVault.seedOrigin`, `carriedSeed`, event `SeedCarried`; errors `NotUndeliverable`, `SameKey` (named so a simulation meeting one says which) |
| monitor (`src/abi.mjs`, `evm.mjs`) | `FloorModule.exitAtBacking`; `SweepVault.seedOrigin`, `carriedSeed`, events `DrawVoided`, `SeedCarried`; the Wrapper's `Undeliverable` topic |
| indexer (`src/wrapped/abis.ts`, `floorModule.ts`) | `Undeliverable`, `undeliverable`, `LATE_DELIVERY_WINDOW`, `SeedCarried`, `carriedSeed`, `seedOrigin`, `holdsRefunds` (queue and module), `exitAtBacking` |
| web-rook (`src/rook/abis.ts`) | Wrapper `undeliverable`, `LATE_DELIVERY_WINDOW`, `Undeliverable`, `NotUndeliverable`/`TooEarly`/`WrongStage`; RAFFLE vault `carriedSeed`, `seedOrigin`, `SeedCarried`; FrogCurve `STUCK_GRACE`, `QueueFull`, `TooEarly`; `FLOOR_MODULE_R3` (`exitAtBacking`, `holdsRefunds`), `ITEM_QUEUE_R3` (`holdsRefunds`, `pure` as compiled), `LISTING_REGISTRY_ERRORS` (`SameKey`) |

The site's ABI-vs-forge test now also compares events (with indexed flags) and errors, and has an R3 case; the indexer has its own
check of every new entry against `out/*.json`. The changed-semantics entries (`burnPrice`/`burnCapacity`/`quoteBurn` non-zero while
paused, `hasRoomFor`/`canClose` false while paused, `refundsOpenAt`/`openRefunds` held while `holdsRefunds()`, `itemCap`/`maxItemCost`
0 while paused) keep their signatures; the site already quoted burns through `burnCapacity`/`quoteBurn`, and nothing blocked a burn
on a stale floor.

## 4. Monitor

- **Draw pages.** `SeedCarried` → SEV2 "draw N CARRIES draw M's seed round and snapshot time" (from the log, and from the vault's
  state through `seedOrigin`, one key); `DrawVoided` of a draw the monitor knew as Announced, or a Voided draw with no pin, no
  resolve and no snapshot seen in the state within two days → SEV2 "VOIDED while still Announced" (the keeper declined to open; the
  void carries the seed, so it is only a delay). The "announced and not open" page says the void carries the seed.
- **exitAtBacking.** The keeper-paused page reads each module's `exitAtBacking()` and says burns continue at the backing per unit
  less the base spread from the moment of the pause (and that closing buys wait and draws cannot be pinned).
- **The canary: `Undeliverable`.** `markUndeliverable` is custodian-only and lets a resolved ticket refund at deliverBy + 72 h. An
  `Undeliverable` log is matched against the canister's record: a **refused** redemption whose attestation is `Sent` or `Done`
  passes; anything else (none, `Due`, a redemption not refused) is SEV1 "AN ATTESTATION NO REFUSAL BACKS" (the canister's EVM key
  speaking without it; a delivery that then confirms is a LostRace).
- **The canister's new fields** (all `opt`, so an older canister still decodes): `Redemption.attest` (variant `Due`/`Sent`/`Done`),
  `ConflictTx.{fee, signed_at, earlier}` (the canary matches every conflict version's txid; the one that confirms may be an
  earlier one), `TicketLogs.{cap, hot, dense, dense_blocks, unreadable}`, `custody_fee_pool()` (`FeePool.quarantined`,
  `FeeEntry.fails`); the timelock's `Status.running` (a `vec`, read as `opt vec` so a pre-R3 timelock decodes) and
  `Event::Started` (without it the R3 timelock's history would not decode at all).
- **Alerts.** Log lines: `PIN WATCH` (SEV2), `FEE ESTIMATES DISAGREE` (SEV2), `FEE POOL: … quarantined` (SEV2), `UNREADABLE ticket
  events` (SEV1), `read by ticket` (INFO), `markUndeliverable mined (reverted…)` and `markUndeliverable would not succeed` (SEV2),
  `TICKET LOGS NOT READ` (SEV2, as before). Fields: an unreadable ticket (SEV1), a block read by ticket (INFO), a quarantined fee
  entry (SEV2), no fee-pool entry ≥ 100,000 sats (SEV2), an attestation `Due`/`Sent` for more than a day (SEV2). Timelock: `Started`
  (SEV2; SEV1 if it failed), a proposal `running` (SEV2), and `NotStarted`/`NotStopped` reworded for N-6 (a guardian can
  `start_target`; a failed stop may run again after 10 minutes).
- **Fixtures.** `examples/watch_fixtures.rs` gained a re-priced conflict, three attested refusals, a dense `TicketLogs` with an
  unreadable ticket, and a fee pool with a quarantined entry; `monitor/fixtures/canister.json` was regenerated with
  `cargo run --example watch_fixtures` (every earlier reply changed too: they now carry the R3 fields as the canister encodes them).

## 5. golive-check

- `payee`: the module's and the desk's `payee()` are compared with the stack file's `payee` (may differ from the custodian); missing
  under `--phase1` is a FAIL.
- `itemUnits` (per collection): the wrapper's `ONE_ITEM()` must equal it, and the rate and ETH-cap checks use it; missing under
  `--phase1` is a FAIL.
- `listingPaused` (per collection): `registry.isPaused(listingId)` must equal it; missing under `--phase1` is a FAIL. The earlier
  code looked for both keys at the top level, where `DeployStack` never writes them.
- Fee pool: with `--canister`, `custody_fee_pool()` is read with dfx as the anonymous identity; FAIL unless one usable, not
  quarantined entry holds ≥ 100,000 sats; a quarantined entry is a WARN; an unreadable pool FAILs.
- `scripts/audit2_golive_check.test.sh`: 13 new cases (39 in all). `scripts/golive-dryrun.sh`: its stand-in dfx answers
  `custody_fee_pool()` (150,000 sats), and a pool of 99,999 sats is refused.

## 6. Site, Trust page, docs, fixture script

- **Refund dates.** Indexer `refundableAt`: a Resolved ticket refunds at deliverBy + 72 h once `Undeliverable`, else deliverBy +
  30 days (`LATE_DELIVERY_WINDOW`); pinned tickets unchanged (7 days after the pin). The site's ticket page shows the date with
  "custody's word", and says an honest non-delivery is attested and refunds about 72 h after the deadline, and a late delivery is
  still recorded (no refund races it).
- **Trust page** (`phase1Trust.ts`): "burns never stop" is gone; burns pay the backing per unit less the base spread from the moment
  the keeper is paused (and after a week stale); the one time a burn waits is an ordinary outage nobody has paused; the graduation
  paragraph states the refused close while paused, the accepted B-07 front-run, the refund hold (at most four more days) and the
  escape hatch 7 days after the close.
- **`docs/v5/launch-gates.md`** "Keeper key stolen": holders burn at the backing per unit from the moment the keeper is paused
  (`exitAtBacking()`), and a week stale without a pause reaches the same escape; the custody-paused paragraph gives the new refund
  times. `web-rook/src/content` re-synced (`launch-gates.md`, and `floor-module.md`, which had drifted after r3-contracts).
- **`docs/v6/phase1-scope.md` §3.4:** step 1 ships the gated Wrapper and this canister build together; step 4 records the hashes
  above; step 8 names the new stack-file checks; step 11 funds the fee pool with ≥ 100,000 sats and re-runs golive-check, confirms
  both explorers accept version-3 transactions and one signet delivery before value enters custody, and starts the monitor's
  holder index at stack deploy. §3.2 and §3.3 say ≥ 100,000 sats.
- **`autobuyer/gen-delivery-fixture.mjs`** builds `version: 3`; run, it prints exactly the txid and both digests `custody.rs`
  already asserts (`3ab1dcf5…`, `272db9c8…`, `ada45320…`, change 2,076), so nothing in the canister changed.

## 7. Runs (the merged tree)

| Run | Result |
| --- | --- |
| `forge test` | **1023 passed, 0 failed** (115 suites; 1021 + `R3CanisterCall` 2; the fork suites return early without `--fork-url`) |
| canister `cargo test` (toolchain 1.98.1) | **478 passed, 0 failed** (476 + `wrapper_abi.rs` 2) |
| timelock `cargo test` / `--features rehearsal` | **12 / 12 passed** |
| keeper `npm test` / typecheck | **130 pass** (125 + `integration_r3.test.ts` 5) / clean |
| tools/snapshot `npm test` / `tsc` | **18 pass** / clean |
| monitor `npm test` | **102 pass** (93 + `integration_r3.test.mjs` 9) |
| ops `npm test` / typecheck | **31 pass** / clean |
| indexer `npm test` / typecheck | **120 pass** (117 + `r3.test.ts` 3) / clean |
| web-rook `npm test` / typecheck / `npm run build` / `VITE_ROOK_PHASE=1 npm run build` | **62 pass** / clean / clean / clean (the copies test holds `src/content` to `docs/`) |
| staging `npm test` / typecheck | **29 pass** / clean |
| `script/deploy.test.sh` | **37 ok**, exit 0 |
| `scripts/audit2_golive_check.test.sh` | **39 ok**, exit 0 (26 + 13 R3 cases) |
| `scripts/timelock-watch.test.mjs` | **4 pass, 1 skipped** (the live-replica case: no local rehearsal timelock), 0 fail; `Started`, `NotStarted`, `NotStopped` wording now asserted |
| `STACK_PORT=8697 scripts/golive-dryrun.sh stack` (real drand; a local anvil fork of 4663 from the public RPC under our staging chain id) | first run: **2 FAIL**: the dry run's stand-in dfx answered `custody_caps()` for every method, so the new `custody_fee_pool()` read could not parse. Fixed (the stand-in answers `custody_fee_pool()` from its own file; a new case refuses a pool with no 100,000-sat entry). Second run: **DRY RUN: ALL PASS**, 44 ok; golive-check 146 checks ok on the stack file with `payee`, `itemUnits`, `listingPaused`; launch → graduation → module mint → sweep → draw announced → opened with the seed round's real evmnet signature → pinned → resolved with the pinned round's real signature → one whole wX claimed |
| `KEEPER_FORK_PORT=8741 scripts/keeper-fork-test.sh` (run after the dry run, not beside it) | **KEEPER FORK TEST: ALL PASS**, 43 ok (real drand; the keeper's new announce guard passes on the fork, whose holders are a day old; step 9 skipped: no dfx) |
| Canister reproducible build | not re-run: `git diff c2fa7ae HEAD` is empty for `autobuyer/canister-rs/src`, `timelock-rs/src`, both `Cargo.lock`, `autobuyer/scripts`, `Dockerfile.build` and `rust-toolchain.toml`; the hashes of §1 stand |

## 8. Still open

- ~~**A carried empty snapshot has no way out on chain (contracts).**~~ **Closed by `fixes-R3-carry.md`** (branch `r3-carry`): an
  empty snapshot is opened as the sentinel alone (`totalTickets` 0) and `requestDraw` closes it with no winner (`Empty`), which
  consumes the carry; a false empty is recomputed by the monitor and cancelled, and the cancel carries the same snapshot time. The
  keeper now announces a carried empty seed (and opens it empty) instead of paging `draw-carried-empty`.
- From `fixes-R3-contracts.md`: N-4, N-5; the accepted B-07 front-run; a compromised keeper walking the floor before the pause;
  `NftWrapper`/`HomeWrapper` (OFF) lack the late-delivery gate; a carried window can be days old if the keeper keeps declining (now
  held by the monitor's index, §2).
- From `fixes-R3-canister.md`: the replica / mainnet-fork run of the scanner against the real providers; TRUC relay on both
  explorers and one signet delivery (now go-live step 11); N-8, N-10 (deposits OFF); `MAX_FINALITY_LAG`; L-6.
- From `fixes-R3-services.md`: an independent floor reference in the monitor; drpc is the only second log provider (100-block
  spans); the keeper's `outbound.ts` DNS pinning; the Info items listed there.
- Not run here: `cargo clippy`/`cargo audit` (not installed), the monitor's Docker image, a fork run driving the keeper and the monitor
  together, `dfx` (the keeper fork test's step 9 skips without it).
