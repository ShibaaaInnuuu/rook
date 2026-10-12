# Round 4, part A: the wrapper's `noteSent` guard (D3-H-1's contract half), go-live checks and the launch configuration

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1eb5788`
(the operator's round `55d8d32..a37f54b` and fix round 3 `a37f54b..1eb5788`). Scope:

- `src/nft/EthNftWrapper.sol`: `noteSent`, `sentTx`, `DeliverySent`, `refund`'s `DeliverySentNotRecorded` guard. The canister's
  calls to it are read only as far as the contract depends on them: when `noteSent` is signed (`ethflow.rs` `note_sent`,
  `home_act`, `home_act_paused`, `mined_home` `Step::Sent`), the `NOTE_SENT` encoding, `tests/wrapper_abi.rs`.
- The tests `test/ethnft/EthNftWrapperSent.t.sol`, `EthNft.invariant.t.sol`, `Pooled.t.sol`, `test/StackChecks.t.sol`.
- `script/golive-check.sh`: `--install-arg` (the keyless-provider rule), `--launch` (A3-L-2, J7), `--monitor-config` (A3-L-3).
  `scripts/golive-dryrun.sh`'s new cases.
- The launch configuration: `deployments/launch/nft-pilot.json`, `monitor-nft.json`, `docs/v7/phase1-nft.md` §9 (D1–D9), J7's caps
  against H3's limits and the timelock's exit rule (`canister.rs` `validate_custody`, `outflow::exit_fits`, `custodycap.rs`).
- The indexer and site where they read the wrapper's refund rule.

**Nothing in the worktree was modified except this report and `review4/poc/A/`; nothing committed, pushed, deployed or signed.**
Every PoC ran in a private copy under the session's scratch directory: the forge project (`src test script lib`, `foundry.toml`,
remappings), `autobuyer/` without its `target/`, and `script/golive-check.sh`'s install checker extracted unchanged. No anvil was
started. A disk cleanup removed the first scratch copies mid-review. Every PoC below was then re-run in a fresh `git archive`
copy of `1eb5788`, and the saved outputs are from that re-run.

Severity scale: the README's.

## Verdict

**The contract half of D3-H-1 is closed.** `noteSent` is custodian-only, `Resolved`-only, refuses a zero hash, and changes no stage
and no ledger number. `refund` checks the late window only when there is no `markUndeliverable`, so a mark still refunds a noted
ticket at `deliverBy + REFUND_GRACE`. The guard cannot be reached by anyone but the custodian. It gives the custodian no power it did
not have: `recordDelivery` already closes a ticket on the custodian's word alone, and more strongly. The wrapper is deployed
directly (`DeployStack._create`, immutables, no proxy), so the new mapping has no layout question. The selector and event topic are
the canister's (`0x6d65f720`, `0x40ecca63…1654`, checked with `cast`). The fixer's nine unit tests and the new invariant are real.
They hold at BAYC's launch parameters too: convBps 1,000, reserve 0, deep profile, 1,000 runs × 200 calls.

The guard's cost is new. `noteSent` is signed once the transfer is **mined**, before finality, and nothing on 4663 can tell a
delivery that later dropped out of Ethereum from one that landed. The canister, for its part, has no path from "mined, then gone"
to a mark. Such a ticket is now stuck until an upgrade, where before round 3 the 30-day late window refunded it (A4-L-2). This is
narrow: it needs a pre-finality reorg that drops our transfer for good.

The go-live changes do what they claim for the cases they name:

- a Dwellir key, host case, drpc's dkey, userinfo, a query or a fragment are refused, and the fake key is printed nowhere;
- names, symbols and slugs are held by collection address;
- a placeholder anywhere, one provider twice, `chain_id` 1 and a wrong guardian are refused.

The keyless rule is a heuristic with gaps: a key under 16 characters, split by `.` or `/`, percent-encoded, or in a subdomain. The
last one is also printed in the `ok` line. The rule also never checks that the canister's two Ethereum providers are two hosts
(A4-L-3). A3-L-3's swapped D8 references still pass (Low, partly closed as the fixer says).

**The launch configuration is not unsafe for value, but it breaks one stated invariant.** J7 makes the custody total 61 items.
H3's global delivery limit is 24 a day, the most an install may start at. So the whole custody can no longer leave within the
timelock's 72 h less a day, the rule `exit_fits` exists to keep. 48 items can leave and 13 cannot. The install check tests each
collection alone, so the canister accepts it (A4-L-1).

## Counts (this part: new, plus earlier items in scope not fully fixed)

| Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- |
| 0 | 0 | 0 | 3 new (A4-L-1..3) + 1 carried (A3-L-3, partly) | 6 |

## Round-3 items in scope

| ID | Sev (R3) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **D3-H-1**, the contract half | High | **Closed.** New residual A4-L-2 (Low); A4-I-1 | Read `noteSent` and `refund` (`EthNftWrapper.sol:408-418`, `:432-458`) line by line. Every caller, stage and ordering was tried: stranger, redeemer, Pinned, Delivered, Refunded, a nonexistent id, re-note, note then mark, note then record, a paused listing. Re-ran `EthNftWrapperSent.t.sol` (9/9) and the invariant suite. The new `invariant_aNotedDeliveryIsNeverRefundedWithoutTheMark` also ran at convBps 1,000 / reserve 0 in the deep profile (`poc/A/AuditNftA4.t.sol`). The handler calls `noteSent` and `lateRefund` at random, and its guarded flag is computed before the call, so it is a real check. The canister's canister-side regression (`tests/ethnft.rs` `r3_d3_h1_*`) was read. `wrapper_abi.rs` holds the selector to Foundry's artifact and the `refund` body to the guard's text |
| **A3-L-2**, labels not held | Low | **Closed** | `launchcheck` (`golive-check.sh:572-586`) reads `name()` / `symbol()` from each wrapper and compares them case-sensitively with the launch file, by collection address; the slug compares too. The dry run's rotated-labels refusal fails as it should, and 15 `(A3-L-2)` ok lines are required |
| **A3-L-3**, `--monitor-config` gaps | Low | **Partly closed (Low, carried)** | Now refused: a placeholder anywhere, providers counted by host, `evm.chain_id` ≠ 4663, a guardian ≠ `registry.guardian()` (`:614-655`). Still passes: D8 references swapped between collections with close floors (Milady ↔ MAYC, 0.93 / 0.86). The fixer's own re-run shows this (`fixes-round3-poc/golive_harness.filled.output.txt:23`). Placeholders not written as `<…>` (e.g. `TODO`) pass. Fix as round 3: put `opensea_slug` / `coingecko_id` in `nft-pilot.json` and hold them there |
| **Keyless provider rule** (bayc10-decisions §5 flag) | — | **Holds for the named cases; gaps are A4-L-3** | `poc/A/install_check.sh`: real buyer `Init`s built with the monitor's Candid (as the dry run's `installhex`), then run through the checker extracted unchanged (`:659-720`). 12 cases |
| A3-L-4, deposits at 0 wei | Low | **Closed** (the operator's round), residual A4-I-6 | `ethflow.rs:1753-1757`, `:2703`: `check_acquire` / `admit` at the module's accepted floor |
| A3-I-1, `Pooled.t.sol` gate copy | Info | **Unchanged**; see A4-I-3 | The J1 tests still judge a Solidity copy of `advance_covers`, and model the advance as `min(maxItemCost, 8 ETH)`, not `advance_bound` |
| A3-I-3, golive does not read the canister's provider pair / limits | Info | **Partly**: the global caps are read now (J7 `tot` check). The provider pair's hosts and `custody_outflow()` are not (A4-L-3, A4-I-2) | |

## Findings

### A4-L-1 (Low): J7's custody total does not fit the timelock's exit rule; the install check tests each collection alone

- **Code:**
  - `autobuyer/canister-rs/src/canister.rs:280-290` (`validate_custody`: `exit_fits(c.max_items, l.per_collection_items, 72 h)` per
    collection, no global test);
  - `outflow.rs:15-21` (the rule: holders "must be able to take every item out before it lands"), `:150-156`, `:202` (a delivery
    counts against `global_items`);
  - `deployments/launch/nft-pilot.json` `_custody` (61 items in all; `per_collection_items` 12, `global_items` 24, "the most an
    install may start at", ×4 of stage 0's 6).
- **Impact:**
  - Every collection passes alone: Azuki's 22 items at 12 a day take 2 days, and 2 are usable.
  - The whole custody does not: 61 items at 24 a day need 3 days, and only 2 are usable. With every collection full, a holder run
    against a pending upgrade gets 48 items out under the code's own rule, and **13 cannot**. A rolling 24 h window allows a third
    batch at the 48 h mark, but only by spending the day of margin the rule deliberately keeps (redeem → drand → delivery →
    Ethereum finality).
  - The canister accepts this install, and `golive-check` does not look. Nor can the limit be raised at install, since 31 a day
    exceeds the ×4 rule.
  - The Trust page overstates it: `web-rook/src/rook/phase1Trust.ts:20` says "at most 12 items a day per collection (so every item
    can leave within the timelock's delay)" and omits the 24 in all. So does `docs/v5/launch-gates.md:322`, "so 22 Azukis can leave
    within the delay": true for one collection.
- **Who can trigger:** only a malicious or compromised upgrade proposal that k guardians do not veto (the timelock's stated limit)
  turns the shortfall into a loss. The loss is bounded by the items that cannot leave, about 13 items, at most ≈ 15 ETH of the
  75 ETH total. Hence Low. With stage 0's old global (6 items / 6 a day) the rule held; J7 broke it.
- **PoC:** `poc/A/a4_exit_fits_global.rs` (canister crate, public `custodycap` / `outflow`). It builds the launch caps and limits,
  checks they validate and are within ×4 of stage 0, and shows each collection passes `exit_fits` while the global does not:
  "61 items held at most, 24 a day, 2 usable days: 48 can leave, 13 cannot". It passes (the weakness is present).
- **Fix:** either
  - add the global test to `validate_custody` (`exit_fits(global_max_items, global_items, delay)`) and to golive-check; or
  - cap the launch's items so the sum is at most 48 (e.g. Azuki and MAYC lower); or
  - lengthen the delay to 4 days at launch.
  
  Also correct the Trust page and G7's wording to name the global limit.

### A4-L-2 (Low): a noted delivery that a reorg drops for good has no exit but an upgrade

- **Code:**
  - `ethflow.rs:2370-2378` and `:2506` (`noteSent` is signed once the transfer is **mined**, `DPhase::Moved`, before finality);
  - `:1540-1548` (at finality a receipt not seen at the pin is `Err("… read again next tick")`, for ever: no path to
    `MarkAfter` / `Mark`);
  - `src/nft/EthNftWrapper.sol:447-451` (noted and not marked: `DeliverySentNotRecorded`, whatever the time);
  - `ethflow.rs:2358` (`send_home` refuses `Step::Mark` while paused; only `Record` and `Sent` go out).
- **Impact:** a pre-finality reorg that removes our mined transfer, which is then never re-included, leaves the ticket `Resolved`,
  noted and unmarked. The redeemer has burned the unit and holds no NFT. The NFT is back at our address, and it is still an item on
  the wrapper, promised to this ticket. Before round 3 the 30-day late window refunded this. Now only an upgrade that marks it does.
  The same holds while paused for a noted ticket whose transfer reads as reverted at finality (`MarkAfter` → `Mark` waits for the
  unpause). The late window used to cover it regardless of the pause. The fixer's §9.1 trade-off covers "a custodian gone after
  `noteSent`". This is the custodian present and the delivery gone.
- **Likelihood:** low. Post-merge reorgs are short, a dropped transaction is normally re-included (the canister's
  `r.block != block` path then handles it), and our line would wedge visibly (the next nonce waits on the freed one).
- **Why the early note is not needed:** a delivery is never signed within a day of `deliverBy` (`ethnft.rs:951`), and the late window
  opens 30 days after it, so noting at finality loses nothing.
- **PoC:**
  - `poc/A/a4_noted_reorg_append_to_tests_ethnft.rs`, appended to `tests/ethnft.rs`. The delivery mines and is noted. Its receipt
    and log then vanish (the reorg), and the token is ours again. After 30 days of 10-minute ticks it is still `Moved` ("… is not
    seen at the pin now: read again next tick"), never marked, and the ticket is `Resolved`. The test passes: the weakness is
    present.
  - Contract half: `poc/A/AuditNftA4.t.sol` `test_A4_L2_aNotedTicketHasNoExitButTheCustodians`. A year on, `refund` reverts; the
    mark opens it.
- **Fix:** note at finality from the receipt, i.e. the same judgment as the record (`delivered_in`), not at the first sighting.
  Or, for a `Moved` delivery whose receipt stays absent past finality for N blocks while our nonce is back below it, re-broadcast
  the saved raw transaction or cancel at its nonce, and let a final cancel reach `Mark`. Also allow `Mark` in the paused pass: it
  signs nothing on Ethereum and only lets a holder be refunded.

### A4-L-3 (Low): the keyless rule's gaps, and the canister's Ethereum pair is never checked for two hosts

- **Code:** `script/golive-check.sh:659-720` (`INSTALL_PY`: `https?://` only; a key is a path segment of 16+ `[A-Za-z0-9_-]`, or a
  listed host), `:716-719` (the `ok` line prints every host). `canister.rs:312-317` (the canister asks for two distinct URL *strings*).
- **Impact (`poc/A/install_check.output.txt`, real `Init`s):**
  - These pass as "all keyless":
    - a 15-character key;
    - a 32-hex key split by dots, or into 15/15/2 path segments;
    - a percent-encoded key;
    - a `;key=` parameter;
    - a key as a subdomain of an unlisted host. This one is also **printed in full** in the `ok` line (`f3a9…6b7c.eth.rpc.example.net`).
  - A `wss://` URL is not read at all. It is unusable as an outcall, so this is harmless.
  - `https://xrpc.cl/eth` and `https://xrpc.cl/eth/` pass as two providers ("names 6 provider URLs, all keyless"). The canister
    accepts them as distinct, and nothing in go-live counts the canister's Ethereum pair by host. The monitor's pair is counted
    (A3-L-3's fix), the canister's is not.

  The keyed providers the operator actually uses (Dwellir, Infura, Alchemy, QuickNode, Ankr, drpc) are all caught. A missed key
  costs a paid plan's quota, not funds. A one-host pair makes the canister's two-provider agreement (receipts, finality, the sale
  scan, the delivery record now judged from a receipt) a single provider's word. Both need an operator mistake.
- **Fix:**
  - For every URL in `evm_chains[*].rpc_urls` of the install argument, require two distinct hostnames per chain, and the
    Ethereum pair to be the monitor file's `eth_nft.eth_rpc` hosts minus the keyed one.
  - Print only the registrable domain in the `ok` line.
  - Treat any path or subdomain label with 12+ hex or base62 characters as keyed, or better, allow-list the canister's providers
    (`xrpc.cl`, `ethereum-rpc.publicnode.com`) and refuse anything else under `--phase1`.

### A3-L-3 (Low, carried, partly closed)

As the table: swapped D8 references pass. The fixer's own filled harness shows it. Unchanged recommendation.

### Infos

- **A4-I-1: the indexer and site still promise the late refund of a noted ticket.**
  - Code: `indexer/src/wrapped/ledger.ts:121-127` (`refundableAt`: resolved, not marked → `deliverBy + 30 days`, for both wrappers;
    `void kind`); `web-rook/src/rook/keyless.ts:39-58` ("refundable by anyone now"). The indexer does not index `DeliverySent`
    (`indexer/src/wrapped/abis.ts`).
  - Effect: for a noted, unrecorded ticket (the D3-H-1 residual, A4-L-2), the site offers a Refund that reverts with an error its
    ABI does not name.
  - Fix: index `DeliverySent` and show "delivery mined; refundable only if custody marks it undeliverable".
- **A4-I-2: go-live does not hold H3's launch limits.**
  - `golive-check.sh` never reads `custody_outflow()`. The launch's 12 / 24 / 3 / 6 / 25 ETH / 3 exist only as prose in
    `nft-pilot.json` `_custody`.
  - The dry run's install argument has `outflow_limits: []`, `custody_caps: []`, `eth_nft: []`
    (`golive-dryrun.sh:110-127`). So the real launch `Init` is never built and never run through the canister's `validate_custody`.
    That is how A4-L-1 went unseen.
  - Fix: put the limits in the launch file and hold them with `--canister`.
- **A4-I-3: `Pooled.t.sol`'s J1 tests and the launch note model the pre-A2-L-1 advance.** `test/ethnft/Pooled.t.sol:155-168`
  advances `min(maxItemCost, 8 ETH)` ("floorsink::view"), and `nft-pilot.json` `_canister` says the same. The canister now advances
  `advance_bound` (7.353 ETH at 6.79). The edges are unaffected, because `advance_covers` still judges the item cap, and only the
  A2-L-1 tests use the bound. The gate is still a Solidity copy (A3-I-1).
- **A4-I-4: stale launch text.** `docs/v7/phase1-nft.md:628-629` (D1: "3 items and 10 ETH per collection and 6 items and 10 ETH in
  all"; "until then golive-check's `--phase1` read refuses" Pudgy and BAYC), `:641` (D4: "the reverse Relay leg stays unwired"),
  both overtaken by the amendments below them.
- **A4-I-5: J7's BAYC and Pudgy item counts are nominal.** A buy is booked in custody at its advance, and it stays at that value when
  settled (`ethflow.rs:1287-1289`, `custodycap::settle`). Two BAYCs at `advance_bound` are 14.71 ETH of the 15 ETH cap. The second
  BAYC waits for custody room once the floor at its start is above ≈ 7.07 (+4.1 %). Pudgy's fourth is similar. This is liveness
  only: the ETH stays on 4663, and C3-H-1's stable key keeps the wait at one ask.
- **A4-I-6: a deposit minted before its module's first post counts at 0 wei.** `module_floor(..).0` is `accepted`, which is 0
  until the first post, and `deposit_floor_ok` passes when accepted is 0 (`ethflow.rs:2466-2470`). It is reachable only if the
  first post fails (e.g. the poster paused by D8 before it) while our reading is fresh with three counted sales. The item count
  still bounds it, and J7's counts already mirror 15 ETH at the floor.

## The launch configuration (item 3)

| Decision | Flag |
| --- | --- |
| Five collections, BAYC 10 ETH / convBps 1,000 / reserveBps 0 (J1) | **OK.** `StackChecks` bounds (conv 300–1,600, reserve ≤ 2,500) hold. Reserve 0 is accepted by `FloorModule` (`reserveBps > 5_000` is the only refusal, and `BPS − reserveBps` is never 0). The books and the D3-H-1 invariant hold at 1,000 / 0 over 200,000 random calls (`poc/A/AuditNftA4.output.txt`). The cost is the operator's: a graduation's unit is priced at 1.10 × the floor, a burn pays about the floor, and there is no standing ETH pot for exits |
| Deposits on (D5), counted at the floor | OK; A4-I-6 |
| J7 15 ETH a collection, global = the sum (61 items, 75 ETH) | **A4-L-1**: the sum no longer fits the exit rule at 24 a day. A4-I-5 |
| H3 25 ETH a day, trip 3 | Consistent with J7: 3 BAYC advances a day; demand trips at 75 ETH asked. Part C re-checks C3-H-1's worst case |
| 12 / 24 deliveries a day | Per collection OK (Azuki 22 in 2 days). **Global not** (A4-L-1) |
| Timelock 72 h | The code floor; with J7's total it is too short for `exit_fits` globally (A4-L-1) |
| D6 Dwellir for the monitor and the runner; canister keyless | The template (`monitor-nft.json`) keeps the key in `${DWELLIR_API_KEY}`, counted as 2 hosts with `xrpc.cl`. The canister's pair must still be checked by host (A4-L-3) |
| D7 defaults, D8 on with `pause_on_no_good_post_h` 6, D9 | Unchanged by this delta; D8's references still unheld (A3-L-3) |
| D1 / D4 text | Stale (A4-I-4) |

## What holds (checked, not findings)

- **Who can note.** Only the custodian, only for a `Resolved` ticket. A zero hash is refused. A re-note overwrites, which is
  harmless: only non-zero matters to `refund`. A note after `Delivered` / `Refunded` / `Pinned`, or on a nonexistent id, reverts.
- **No stronger power.** `recordDelivery` already closes a ticket on the custodian's word (`:390-403`), and `noteSent` is strictly
  weaker (`test_A4_whatHolds_noteSentIsWeakerThanRecordDelivery`).
- **The listing pause.** It stops mints only; `noteSent`, `markUndeliverable` and `refund` ignore it, which is right
  (`test_A4_whatHolds_noteSentAndMarkIgnoreTheListingPause`).
- **The mark still refunds.** `markUndeliverable` after `noteSent` refunds at `deliverBy + 72 h` on both clocks. No ordering of
  note, mark, record and refund breaks the ledger (invariants).
- **The late-window race.** The canister never signs a delivery within a day of `deliverBy` and cancels a pending one then, so a
  delivery mined near the late window, racing a refund, is not a new path. The pre-existing "kept unmined for 30 days" is in the
  contract's `@dev`.
- **Upgradeability.** The wrapper is not a proxy (constructor, immutables, `_create`). The deploy writes the new code, and there is
  no state to migrate.
- **The ABI.** `noteSent(uint256,bytes32)` = `0x6d65f720` and `DeliverySent(uint256,uint256,bytes32)` =
  `0x40ecca63…1654` (`cast sig` / `cast keccak`). `en::note_sent_call(7, [9;32])` is the selector, the word 7 and the hash
  (`wrapper_abi.rs`). `decode_ticket`'s struct is unchanged.
- **J7 in golive-check.** The per-collection caps are held against the launch file, the one-item cap against 15 ETH, and "the total
  never binds" (`:410-417`, `:460-482`). The dry run's two refusals (a binding total, BAYC at 3 items) fire.
- **Key printing.** golive-check prints no path, query or userinfo of any URL it refuses. The dry run checks the fake Dwellir key is
  absent from the output. `monglobal` prints counts, not URLs. The one leak is a subdomain key in an `ok` line (A4-L-3).

## PoCs (`poc/A/`)

| File | Finding | Result |
| --- | --- | --- |
| `a4_exit_fits_global.rs` (+ `.output.txt`) | A4-L-1 | **Passes** (the weakness): 48 of 61 items can leave within the delay |
| `a4_noted_reorg_append_to_tests_ethnft.rs` (`a4_noted_reorg.output.txt`) | A4-L-2 | **Passes** (the weakness): `Moved` for 30 days, never marked |
| `AuditNftA4.t.sol` (+ `.output.txt`) | A4-L-2 (contract half); what holds; the invariant suite at J1's parameters | 3 unit tests pass; the invariant suite passes at convBps 1,000 / reserve 0 in the deep profile (1,000 runs × 200 calls) |
| `install_check.sh`, `mkinit.mjs` (+ `install_check.output.txt`) | A4-L-3 | Cases 1–5, 8 and 10 pass as "all keyless"; case 4 prints the key; Dwellir, drpc.live and Ankr are refused |
