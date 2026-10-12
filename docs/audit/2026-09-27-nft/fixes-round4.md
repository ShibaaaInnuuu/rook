# Fix round 4: the 2026-09-27 review of the Ethereum NFT side, round 4

Written 2026-09-28 on branch `nft-side` (at `2735dac`: the round-4 re-review's reports and PoCs on top of fix round 3). It answers
`review4/README.md` and the five part reports (`R4-contracts-deploy.md`, `R4-canister-floor.md`, `R4-canister-wayback.md`,
`R4-services.md`, `R4-canister-flow-upgrade.md`) finding by finding: the fix, where it is, and the test that is the reviewer's PoC
turned around (it fails on the code before this round and passes now). `fixes-round4-poc/` has the reviewers' PoCs re-run on this
code. §11 ("As built") has the commits, the test counts and the reproducible-build hashes. Nothing was deployed, pushed or signed with
a real key; the only chains written to were local anvils (the dry run's fork of 4663, the Seaport fork proof's fork of Ethereum,
both read-only upstream), and the reproducible build fetched its crates.

**The rules every fix keeps** (as rounds 1–3):

- Nothing is weakened to buy availability: H1 (pause), H2 (custody caps), H3 (the outflow limiter and its tripwire), "save, then
  send", and every judge (`relayeth`, `seaport`, `ethnft`) refuse what they refused. What changed in H3 is the key a waiting mint is
  asked under (E4-H-1), as round 3 did for advances.
- Stable state stays decodable: every new field is `opt` (`Option` + `#[serde(default)]`); no enum case was added. States saved by
  the `4949da0`, `a37f54b` and `1eb5788` builds (the last written by that build, every round-3 field present) restore with nothing
  lost (`tests/audit_nft_r4.rs` `r4_states_saved_by_the_old_builds_…`).
- **Not changed** (operator decisions): BAYC 10 ETH / `convBps` 1,000 / `reserveBps` 0 (J1); J7's 15 ETH a collection and its item
  counts, the global the sum; H3's 25 ETH a day and the launch outflow (12 / 24 / 3 / 6 / trip 3); D1–D9, D5 deposits on; D4's
  automatic return after 48 h; Dwellir first for the monitor and the runner; keyless canister providers. **J7's item counts and the
  launch outflow do not fit the exit rule together (A4-L-1)**: this round makes the canister and golive-check refuse the pair and
  prints the options (§6); it does not change either number. That is the operator's call.
- No contract changed (`src/` is untouched).

## 0. Summary

| ID | Sev | Status | Fix | Where | Test (the PoC, turned around) |
| --- | --- | --- | --- | --- | --- |
| **C4-H-1** | High | **Fixed** | Neither chain is read at a pin below one already read (`pin::PinFloor`, canister-wide `PIN_FLOORS`, saved at upgrade): `canister::pin` refuses a 4663 pin under the highest read by any part of the canister (vault loop, custodian, frog side, NFT side, endpoints), `EthNet::pin` the same for Ethereum, and a nonce read below the highest read is an error on both. The NFT side also keeps its own home floor (`EthState.home_floor`: the next nonce past its last mined home transaction, the highest home pin): a tick at a lower pin reads and signs nothing on 4663; `send_home` refuses a nonce under the floor. `resolve_home` has an exit: a transaction whose nonce is used on chain and that is seen nowhere for `HOME_NONCE_TAKEN_SECS` (1 h) is dropped and its step taken back as a revert (done again at the next nonce), never waited on for ever (a state an earlier build may have wedged). The Relay leg's 4663 reads sit behind the same floor, and a refund gives back the rise over the watch's start, never the raw balance | `pin.rs`, `canister.rs` `pin`, `evm_nonce`, `pre/post_upgrade`; `ethnet.rs` `EthNet::pin`, `eth_nonce`, `home_pin`; `ethflow.rs` `home_floor_ok`, `send_home`, `resolve_home`, `step_buy` | `audit_nft_r4.rs` `r4_c4_h1_…` ×3, `r4_e4_m1_…`; `pin.rs` `c4_h1_…` |
| **E4-H-1** | High | **Fixed** | Every mint, a purchase's or a deposit's, is asked under its collection's NEXT-mint key (`mint:eth:{col}:{n}`, `n` = the collection's mints that went out: buys `Minting` or past, deposits `Minting`/`Minted`), so a waiting mint is one ask however many deposits wait behind it and however long; and a collection whose first candidate waits is not asked again that pass (C-L-2 kept: other collections are). A collection's demand in the window is what went out plus one: 3 + 1 against the 9 that trip. Every other H3 ask site in the canister was reviewed: each is keyed per logical outflow (a source-reading test holds the list) | `ethflow.rs` `mint_key`, `home_act` steps 2 and 3 | `r4_e4_h1_nine_deposits_…`, `r4_e4_h1_a_weeks_deposits_…` (J7's 16 + 22 items deposited at once, redemptions out, 8 days: never tripped, demand ≤ 4 a collection), `r4_every_h3_ask_is_keyed_per_logical_outflow` |
| **B4-H-1** | High | **Fixed** | A range the IC refuses for size is remembered (`SaleScan.bad`) and never asked whole again: read by halves, nearest the walk first, down to one block. A single block still over 2 MB is read through the marketplaces' own sale logs (`eth_sale_moves`: Seaport 1.5 / 1.6 `OrderFulfilled`, Blur v2 `Execution721*`, filtered by address and topic, projected to this collection's sales by `ethnft::sale_logs_body`); a block can hold no 2 MB of fills. If that fails too, the block is recorded (`SaleScan.skipped`), passed, and retried 6 h × tries later, three times at most, never every pass. Each walk stops after 2 failed reads a pass (`SALE_SCAN_FAILS_PER_WALK`) and a pass reserves at most 12 MB of responses per collection (`SALE_SCAN_BYTES_PER_PASS`, counted as `EthNet` reserves them: every class, every provider). Under `EVIDENCE_CYCLES_FLOOR` only the scan stops: listings are still verified and the floor posted from them and the sales already read. A pin that moved back no longer resets the scan (B4-I-3) | `ethflow.rs` `sale_scan`, `scan_read`, `ScanPass`, `floor_pass`, `reading_of`; `ethnft.rs` `sale_logs_body`; `ethnet.rs` `eth_sale_moves`, `logs_classes`, `eth_sale_logs` | `r4_b4_h1_a_…` (5 of 5 read, 6 failed reads in all, none after the third pass), `r4_b4_h1_b_…` (≥ 38 of 40 newer sales read in a day), `r4_b4_h1_c_…` (a sale inside the oversized block counted), `r4_b4_h1_d_…` (posted under the cycles floor), `r4_b4_i3_…`, `r4_b4_h1_the_marketplace_sale_logs_…` |
| **C4-M-1** | Med | **Fixed** | A Relay order past its deadline and grace stays WATCHED (it holds the Relay leg) until its payout is read by its own watch, or the operator writes it off; a watched `Stuck` also reads a late 4663 refund as its own. The write-off (`eth_give_back`) is refused until the canister has watched it `BACK_LATE_SECS` (24 h) past the deadline and grace; it keeps the watch (`EthBuy.late: LateOrder`). A payout that comes after all is handed in by transaction (`eth_credit_payout(buy, chain_id, tx, wei)`) and judged against that order alone (`judge_payout`: mined and successful, never credited before, it names the order id in its input or is sent by the order's own solver (`orderData.solver`, bound by the id), it pays our address, at most what the order sent and at least half), with the ETH checked to be at our address beside every other claim; it is credited to its own buy (`returnAdvance`, or an arrival on Ethereum) or, once the buy was closed as a loss, to its collection (`fundReserve` through a sweep record, or its leftover). `eth_give_back` on a watched forward `Stuck` returns only the rise over the watch's start, never `balance − keep`. A payout is never read by another order's watch, because no other order is watched while it could land | `ethflow.rs` `step_buy`, `give_back_now`, `put_payout`, `judge_payout`, `payout_pass`, `LateOrder`, `PayoutTx`; `relayeth.rs` `EthDeposit.solver`; `ethnet.rs` `eth_credit_payout`, `payout_tx`; `canister.rs` `home_payout_tx`; `evmsend.rs` `payout_view` | `r4_c4_m1_…` (the late payout returns to its module), `r4_c4_m1b_…` (Azuki bridges once, its own ETH), `r4_c4_m1c_…` (credited after a write-off, a stranger's and an over-value transfer refused, never twice), `r4_c4_m1d_…` (the give-back pays none of 2 ETH that are not the order's), `r4_c4_m1_the_payout_judge` |

**All three Highs and the Medium are closed by this round's own reading.** Also: seven Lows fixed, two partly (§7), and several Infos
(§8). **E4-H-1 is closed, so the reviewer's condition "don't launch deposits on until E4-H-1 is fixed" is met by this code** (subject
to round 5). A4-L-1 now blocks the launch configuration as it stands: the canister refuses the operator's J7 / outflow pair (§6).

## 1. C4-H-1: the 4663 pin never goes back, and the home line never waits for ever

- **The pin floors.** `pin::PinFloor { pin, nonce }` per chain, in `canister::PIN_FLOORS` (heap, written into `EthState.pin_floors`
  at `pre_upgrade` and restored at `post_upgrade`; `eth_nft_state` shows them). Every 4663 read of the canister takes its pin from
  `canister::pin()`, which now refuses a pin under the highest read: a provider whose head fell back across a bucket makes the tick
  wait instead of reading our own mined transactions undone. `evm_nonce` refuses a nonce under the highest read (the second line).
  The Ethereum side's `EthNet::pin` and `eth_nonce` do the same (E3-M-1's `eth_floor` stays: it also names our last mined block).
- **The NFT side's home floor** (`EthState.home_floor = (next nonce, highest home pin)`), for a world that does or does not say its
  pin: `home_floor_ok` at the tick's start refuses a home pin under the highest this side read, or (when the world cannot say its pin)
  a nonce at the pin under the next nonce; `send_home` refuses to sign under it; `resolve_home` raises it when our transaction is seen
  mined.
- **The exit.** A pending home transaction whose nonce the chain shows used, and which is seen nowhere at the pin, is waited on
  `HOME_NONCE_TAKEN_SECS` (1 h: a provider's index catching up), then dropped: `mined_home(.., false)` takes its step back as a revert
  (an advance back to `NeedAdvance`, a deposit back to `Funded`, a record, a note or a mark back to be signed again, a post simply due
  again; `Settling` now re-signs its settle), and the floor moves past the nonce. Every re-signed step is guarded by its estimate and
  by the contracts (a second `advance` reverts `AdvanceOpen`, a second record `WrongStage`, a second deposit mint the wrapper's
  itemOf), and a re-deposit by the balance check (`bal < value`) at a floored pin.
- **The other signers on the line** (`looper.rs`, `redeem.rs`, `frogflow.rs`) take their nonce from `evm_nonce` at `canister::pin`,
  so the canister-wide floor covers them. Their own "nonce used, not visible" waits were left as they are (outside the NFT side's
  scope; with the floor, a lower pin can no longer cause them).
- **Every chain read audited for a floor.** 4663: all through `canister::pin` (loop, custodian, frog side, NFT side,
  `evm_tx_status`, `broadcast_evm`'s read-back, `pinned_read`, the payout judge). Ethereum (NFT side): all through `EthNet::pin`;
  balance reads additionally behind `eth_floor`. The operator endpoints `nft_*` of `nftnet.rs` (the operator's own line on other
  chains, `pin_on`) are not floored: they sign nothing on the NFT side's chain (they refuse it) and re-read at signing; noted in §10.

## 2. E4-H-1: a waiting mint is one ask

- `mint_key(st, col, collection)` = `mint:eth:{col}:{n}`. `n` counts that collection's mints that went out (buys `Minting`,
  `Minted`, `Settling`, `Settled`, not sweeps; deposits `Minting`, `Minted`). While a mint waits nothing moves `n`, so every tick's
  ask is the same key whichever buy or deposit asks. A reverted mint (`Minting` → `Final` / `Mint`) takes `n` back and asks again under
  the key it was recorded under (`record` is idempotent per key).
- `home_act` step 3 asks at most one deposit per collection per pass (`waited`), and the key change alone already bounds the demand.
- **Every other H3 ask site**, reviewed (the list is held by `r4_every_h3_ask_is_keyed_per_logical_outflow`, which fails on any new
  or changed site): deliveries one per ticket (custody bounds them: 24 < 36 a collection, 61 + 6 < 72 in all); stray returns one per
  claimed token (`Return` never trips); advances C3-H-1's key; the frog side's buys, conversions, claims, deposits, mints and returns
  one per frog / outpoint / want (each admitted to custody before it asks, so bounded by the Bitcoin wrapper's 3 items); hand-in
  rewards one per hand-in; the operator's fills one per order; Bitcoin deliveries one per ticket once its events are in; sales one per
  sale.

## 3. B4-H-1: an oversized log range costs its bisection once

- `scan_read` is the one read of all three walks. `SaleScan.bad` (at most 8 ranges, pruned to the window) holds what the IC refused
  for size; the walk asks the half nearest it; a success narrows the range. At one block: `eth_sale_moves`, else `SaleScan.skipped`.
- The walks count failed reads separately from the chunk budget and stop at 2 each; `ScanPass` also stops at 12 MB reserved this pass.
- The re-read of dropped ranges now gets one chunk every pass even while the queue is full (B4-L-1).
- **Cycles, re-measured** (the reviewer's price: 10,400 cycles a reserved byte on a 13-node subnet). Before: 52 failed reads a pass of
  2.464 MB × 2 providers, ≈ 1.34 T a pass, every pass for seven days per attacker transaction. Now: the harness's worst pass reserved
  12.16 MB (≈ 126 B cycles) and only the first three passes after the oversized block appeared failed at all (6 failed reads in all);
  afterwards the block costs nothing. An attacker who plants a new oversized block every 20-minute pass (0.002–0.05 ETH each, ≈ 0.15–
  3.6 ETH a day) makes us spend at most ≈ 126 B a pass per collection, ≈ 9 T a day, and the floor is still posted; under the cycles
  floor only the scan pauses. B4-I-1 (the evidence floor above the cycles page) is therefore no longer a floor outage.
- The sales of an oversized block are read from the marketplaces' logs; if even those fail the block's sales are not counted and the
  reading says so (B2-L-2's rule for receipts, now for logs).

## 4. C4-M-1: a payout is always its own order's

As §0. The flow: a way back (or a forward order) past deadline + grace stays watched; the monitor pages it (`eth-back-late`, then the
`Stuck` page "still WATCHED"); the operator writes it off with `eth_give_back` once Relay says nothing will come and not before a day
past the grace; a second `eth_give_back` closes it as a loss (`returnAdvance(0)`, as round 3); the written-off order pages its note
(SEV3 `eth-late-order`); if Relay pays after all, the operator hands in the transaction Relay's status names (`eth_credit_payout`) and
the canister judges it against the order. The judge relies on Relay's solver address (`orderData.solver`) or the order id in the
input; §9 lists that as a call.

## 5. C4-L-1: the way back's bound (re-rated by this round)

C4-M-1's fix lengthens C4-L-1: an order Relay neither pays nor refunds now holds the Relay leg (every collection's new buys) until
the operator writes it off, at the earliest a day past its deadline and grace (7–8 days out in Relay's quotes). Round 3 let it go by
itself at that point, which is exactly what made a late payout read as another order's. This trades liveness (new buys, never
holders' burns or redemptions) for attribution; the SEV1 `Stuck` / SEV2 `eth-back-late` pages tell the operator to act. It stays a
Low, and it is §9's call 2.

## 6. A4-L-1: the whole custody against the exit rule — the operator's options

**What changed.** `canister.rs validate_custody` (install and every upgrade) now also requires the whole custody (the collections'
item caps summed, or the global cap where it binds) to leave within the timelock's 72 h less a day at `global_items` a day, and says
the numbers when it does not. The default install (no caps named) caps its global item count at what its limits let out (stage 0's 6
a day: 12). `golive-check.sh` holds the same rule on the launch file (whose outflow limits are now data, `nft-pilot.json` `outflow`,
A4-I-2) and, with `--canister`, on the canister's `custody_caps()` and `custody_outflow()`, and prints the options below. The dry run
shows the operator's file refused and runs its go/no-go read on option (a).

**The numbers.** Two usable days (72 h less a day). J7's counts are 16 + 22 + 17 + 4 + 2 = **61 items**. At the launch outflow's
**24 deliveries a day in all, 48 items can leave; 13 cannot**. Each collection alone fits (22 Azukis at 12 a day). To move all 61
within the delay needs **31 deliveries a day in all** (61 / 2, rounded up). An install may start `global_items` at most at 24 (×4
stage 0's 6), so 31 is not reachable at install.

**The options** (golive-check prints them; none is applied):

- **(a) Item caps summing to at most 48** at 24 a day. In proportion: Milady 16 → 13, Azuki 22 → 17, MAYC 17 → 13, Pudgy 4 → 3,
  BAYC 2 → 2 (the dry run's go/no-go read uses this). Each still holds 15 ETH of value cap; fewer items fit it.
- **(b) A global item cap of 48** with J7's per-collection counts: custody holds at most 48 items at once across the five. This makes
  the total bind, which J7 said it should not.
- **(c) Install at (a) or (b), then raise `global_items` to ≥ 31 a day by a timelocked upgrade** (the ×4 rule allows up to 96 from 24),
  and only then the item caps back to 61.
- **(d) A longer delay**: 61 items at 24 a day need 4 days (96 h) of delay; the canister's `ASSUMED_DELAY_SECS` (72 h) would have to
  change with the timelock's floor: a code change.

The Trust page and `launch-gates.md` G7 state the global limit and say the numbers are the operator's to settle.

## 7. The Lows (18 open at `1eb5788`: 8 new, 10 carried)

| ID | Status | What |
| --- | --- | --- |
| A4-L-1 | Fixed (the check); the numbers are the operator's (§6) | The canister and golive-check hold the whole custody to the exit rule: `canister.rs` `a4_l1_…`, the dry run |
| A4-L-2 | Fixed | `noteSent` is signed at finality from the delivery's own receipt, just before the record: a transfer a reorg drops before finality is never noted, keeps the late window, and is `Gone` once refunded; a mark goes out while paused: `tests/ethnft.rs` `r4_a4_l2_…` ×2 |
| A4-L-3 | Fixed | The canister's providers counted by host per chain (the argument decoded by the monitor's IDL); keys of 12+ characters in a path part, split by separators, percent-encoded or in a `;param`, or in a subdomain, refused; only the registrable domain printed: the dry run's three new refusals, `fixes-round4-poc/A.install_check.output.txt` |
| D4-L-1 | Fixed | A `Sent` / `Cancelling` delivery whose NFT is elsewhere is exempt when its own receipt, on every Ethereum provider, succeeded and holds the Transfer to the destination: `audit_nft_d_r4` |
| D4-L-2 | Fixed | The deposit scan re-reads the last 64 blocks every poll: `audit_nft_d_r4`, `bayc10.test.mjs` |
| B4-L-1 | Fixed | One re-read chunk a pass whatever the queue: `r4_b4_l1_…` |
| A3-L-3 | Fixed | The monitor's D8 references held against the launch file's `openseaSlug` / `coingeckoId` by collection; `TODO`-style placeholders refused: the dry run's two refusals |
| B4-L-2 | Partly | An oversized block inside the hull no longer stalls it (`scan_read`); the hull itself still spans disjoint drops |
| B-L-6 | Partly | `eth_give_back` no longer pays from the raw 4663 balance (C4-M-1), which was its NFT-side share; the shared address remains |
| C4-L-1 | Re-rated, not fixed | §5 |
| C3-L-1, E3-L-2, B3-L-1 | Not fixed | Unchanged (as round 4 rated them) |
| A-L-2, B-L-5, C-L-5, D-L-3, D-L-6 | Unchanged | As `fixes-round2.md` §7 |

**Lows: 18 — 7 fixed, 2 partly, 9 not / unchanged (one of them, C4-L-1, lengthened by C4-M-1's fix).**

## 8. The Infos

Fixed on the way: B4-I-3 (a pin that moved back no longer resets the scan: `r4_b4_i3_…`), B4-I-1 (the evidence floor no longer stops
posts), E4-I-2 / A4-I-6 (no deposit mints while its module's accepted floor is 0: `r4_e4_i2_…`), E4-I-4 (`eth-state-round4.hex`,
every round-4 field present, decoded by the monitor's and the runner's IDLs), A4-I-2 (the launch outflow is data and held), A4-I-4
(D1 / D4 / §5 stale text marked overtaken in `phase1-nft.md`), D4-I-1 (the exemption needs the ticket still `Resolved`), D4-I-2
(`/status` heads redacted before they are cut), D4-I-3 (the `Stuck` and way-back page texts), C4-I-4 (the monitor comment), D4-I-5
(the Trust page: "once noted sent"). Unchanged: A4-I-1 (the indexer and site still offer a late refund that reverts for a noted
ticket; rarer now that noting waits for finality), A4-I-3, A4-I-5, B4-I-2, C4-I-1, C4-I-2, C4-I-3, C4-I-5, C4-I-6, E4-I-1 (the
`MarkAfter` half), E4-I-3, D3-I-2, and the earlier rounds' as listed there.

## 9. New judgment calls for the operator

1. **A home transaction seen nowhere for 1 h with its nonce used is dropped** and its step done again (`HOME_NONCE_TAKEN_SECS`). The
   alternative (wait for an upgrade) was the wedge. A transaction that actually mined but that both providers failed to show for an
   hour would have its step re-signed; every step is guarded by its estimate, the contracts and the balance check (§1).
2. **The Relay leg waits for a late order** until its payout is read or the operator writes it off (§5, C4-L-1): no other
   collection starts a buy meanwhile.
3. **The write-off is refused until a day past the deadline and grace**, whatever Relay's status says (Relay refunds after the
   deadline; round 3 allowed the grace alone).
4. **A late payout is credited by transaction, judged by the order id in its input or by Relay's solver address** (`orderData.solver`)
   as its sender, at most the order's value and at least half. If Relay pays from another address without the id in the input, the
   operator cannot credit it (an upgrade could).
5. **`noteSent` at finality** (A4-L-2), not at the first sighting: the late window is 30 days after `deliverBy`, so nothing is lost.
6. **The sale scan's bounds**: 2 failed reads per walk, 12 MB of responses per collection and pass, an oversized block read through
   Seaport 1.5 / 1.6 and Blur v2 logs only (another marketplace's sale inside such a block is not counted), a block unreadable both
   ways retried 3 times (6 h, 12 h, 18 h apart) and then given up with a note.
7. **Under the cycles floor the floor is still posted** from verified listings and the sales already read; only the scan pauses.
8. **The default install (`custody_caps = null`) caps its global item count at what its outflow lets out** (12 at stage 0), so an
   install that names no caps always passes the new exit rule.
9. **The launch configuration's A4-L-1 pair** (§6): which option, (a)–(d).
10. **The monitor reads a Sent delivery's receipt on every provider** before exempting it (D4-L-1), and re-reads 64 blocks of the
    deposit scan every poll (D4-L-2).

## 10. Left open (for round 5)

- C4-L-1 as re-rated (§5); C3-L-1, E3-L-2, B3-L-1; B4-L-2's hull; the shared 4663 address (B-L-6); A-L-2, B-L-5, C-L-5, D-L-3,
  D-L-6; the Infos listed as unchanged in §8.
- `nftnet.rs`'s operator endpoints (`nft_preview`, `nft_sign`, `nft_status`) read other chains at `pin_on` without a floor (they
  never sign on the NFT side's chain).
- The other home-line signers' own "nonce used, not visible" waits (`looper.rs`, `redeem.rs`, `frogflow.rs`) have no drop-after-1-h
  exit; the canister-wide pin floor keeps a lower pin from causing them.
- E4-M-1's reviewer PoC still passes in its own harness, which cannot tell a lower pin from a refund (`fixes-round4-poc/README.md`); the
  fix is at the pin itself.

## 11. As built (2026-09-28)

**Commits** (branch `nft-side`, on `2735dac`):

| Commit | What |
| --- | --- |
| `57032dd` | Canister: C4-H-1, E4-H-1, B4-H-1, C4-M-1, A4-L-1 (the install check), A4-L-2, B4-L-1, B4-I-3, E4-I-2; the `.did`; the round-2 fixture re-encoded, `eth-state-round4.hex`, the old builds' saved states; `tests/audit_nft_r4.rs` |
| `8018070` | Monitor and runner: D4-L-1, D4-L-2, D4-I-1, D4-I-2, the page texts (C4-M-1, D4-I-3); `audit_nft_d_r4`, `ops/test/audit_nft_r4.test.ts` |
| `f47f27c` | Go-live: A4-L-1 (golive-check and the launch file's outflow), A4-L-3, A3-L-3; the dry run |
| `548f214` | Site: the Trust page, `launch-gates.md` G7 |
| (this document's commit) | This document; `fixes-round4-poc/`; `docs/v7/phase1-nft.md` (§2.1, §2.2, §4.6, §5, §9 D1/D4 notes, §11.11), `phase1-nft-pooled.md` (§3.4, §6 J7) |

**Results:**

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites; unchanged: no contract changed) |
| The Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **647 passed**, 0 failed (was 621: +22 `tests/audit_nft_r4.rs`, +2 `tests/ethnft.rs` `r4_a4_l2_…`, +2 unit: `pin.rs`, `canister.rs` `a4_l1_…`) |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed (unchanged) |
| ops | **57 passed** (+1); typecheck clean |
| monitor | **195 passed** (+5 `audit_nft_d_r4`) |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | **72 passed**; typecheck clean (the Docs page's copy of `launch-gates.md` re-synced) |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 checks ok on the v3–v6 stack; 536 on the five-collection NFT stack; **655** on the launch go/no-go read; 46 refusals `ok`, new: the operator's A4-L-1 pair on the launch file and on the canister, swapped D8 references, a `TODO` placeholder, a key in a subdomain (printed nowhere), a 15-character key, one Ethereum host twice) |

**Reproducible build** (`autobuyer/Dockerfile.build`, linux/amd64, two `git archive` exports of `548f214`, each
`docker build --no-cache`, the base image re-pulled after the operator's cache prune): the same hashes both times.

```
429e7ad6f92d3fcf16e423b8b9acfd29e3c3ece587ea2a34cd947fd441e7f730  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

`buyer.wasm` is new (6,920,875 bytes; 1,310,281 gzipped with `gzip -9 -n`, under the 2,000,000 proposal limit) and replaces
`cbe337d8…b360` (`fixes-round3.md`) for any install of this branch. The timelock is unchanged. Nothing was installed; the exports, the
two build images, the extracted wasm and the private PoC copies were removed (Docker's shared build cache left alone).

**The delta round 5 must cover:** `git diff 2735dac..HEAD`:
`autobuyer/canister-rs/{etch_autobuyer.did, src/{canister,ethflow,ethnet,ethnft,evmsend,pin,relayeth}.rs,
tests/{audit_nft,audit_nft_r2,audit_nft_r3,audit_nft_r4,ethnft}.rs, tests/fixtures/{eth-state-round2.hex, eth-state-round4.hex,
state/saved-{1eb5788,4949da0,a37f54b}-rich.hex}}`; `monitor/src/{alerts,candid,ethnft}.mjs`,
`monitor/test/{audit_nft_d_r3,audit_nft_d_r4,bayc10}.test.mjs`, `monitor/test/d4-state.hex`; `ops/test/audit_nft_r4.test.ts`;
`script/golive-check.sh`, `scripts/golive-dryrun.sh`, `deployments/launch/nft-pilot.json`; `web-rook/src/rook/phase1Trust.ts`,
`web-rook/src/content/launch-gates.md`, `docs/v5/launch-gates.md`; the docs above.
