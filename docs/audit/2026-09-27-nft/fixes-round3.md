# Fix round 3: the 2026-09-27 review of the Ethereum NFT side, round 3

Written 2026-09-27 on branch `nft-side` (at `a37f54b`: the round-3 re-review's reports and PoCs, `55d8d32`, and the operator's
decisions of 2026-09-27 fast-forwarded on top, `bayc10-decisions.md`). It answers `review3/README.md` and the five part reports
(`R3-contracts-deploy.md`, `R3-canister-floor.md`, `R3-canister-wayback.md`, `R3-services.md`, `R3-canister-flow-upgrade.md`)
finding by finding: the fix, where it is, and the test that is the reviewer's PoC turned around (it fails on the code before this
round and passes now). `fixes-round3-poc/` has the reviewers' PoCs re-run on this code. §11 ("As built") has the commits, the test
counts and the reproducible-build hashes. Nothing was deployed, pushed or signed with a real key; the only chains written to were
local anvils. The network use was read-only: an anvil fork of Ethereum for the Seaport fork proof, the dry run's fork of 4663, and
the reproducible build's crate fetch.

**The rules every fix keeps** (as rounds 1 and 2):

- Nothing is weakened to buy availability: H1 (pause), H2 (custody caps), H3 (the outflow limiter), "save, then send", and every
  judge (`relayeth`, `seaport`, `ethnft`) are unchanged in what they refuse. H3's limits and tripwire are unchanged; what changed is
  the key one waiting advance is asked under (C3-H-1).
- Stable state stays decodable: every new field is `opt` (`Option` with `#[serde(default)]`), and the one new enum case is appended
  (`Step::Sent`). No `BPhase`, `DPhase`, `EthKind` or `XPhase` case was added, so the services' IDLs decode every state this build
  can write. `tests/fixtures/eth-state-round2.hex` is re-written from the canister's own encoder (the new `opt` fields); the monitor
  and the runner decode it with their IDLs.
- Every outcall a stranger can cause stays bounded per tick (the sale scan: the same 13 log chunks and 16 receipts a pass).
- **Not changed** (operator decisions): BAYC 10 ETH / `convBps` 1,000 / `reserveBps` 0 (J1); J7's 15 ETH a collection with the
  global cap the collections' sum; H3's 25 ETH a day; D1–D9, D5; D4's automatic return after 48 h; Dwellir first for the monitor and
  the runner. Where a fix touches the ground a decision stands on, it is listed for the operator in §9 (D4's 48 h are now counted
  from the first arrival; the way back gets bounded exits around them).
- **One contract changes** (`EthNftWrapper`, D3-H-1: `noteSent` and the late-window guard, §1). No other file under `src/` changed.
  Nothing is deployed, so the stack deploys the new wrapper; a canister of this build calls `noteSent` and needs it.

## 0. Summary

| ID | Sev | Status | Fix | Where | Test (the PoC, turned around) |
| --- | --- | --- | --- | --- | --- |
| **D3-H-1** | High | **Fixed** | A delivery is judged at finality by its OWN receipt (success, final, the collection's `Transfer(us → destination, token)` in it), never by who owns the token now; recorded whatever the redeemer did with the NFT. Contract-side: as soon as the transfer is seen mined the canister calls the wrapper's new `noteSent(id, tx)`; `refund()`'s 30-day late window no longer refunds a ticket noted sent (only `markUndeliverable` does). The monitor accepts any owner once our delivery of that very item mined | `ethflow.rs` `delivery_pass`, `note_sent`, `home_act`, `home_act_paused`, `mined_home`; `ethnft.rs` `delivered_in`, `NOTE_SENT`; `src/nft/EthNftWrapper.sol` `noteSent`, `sentTx`, `refund`; `monitor/src/ethnft.mjs`, `evm.mjs` | `tests/ethnft.rs` `r3_d3_h1_…` ×3; `test/ethnft/EthNftWrapperSent.t.sol` `test_D3H1_…` ×4 (+5), `EthNft.invariant.t.sol` `invariant_aNotedDeliveryIsNeverRefundedWithoutTheMark`; `monitor/test/audit_nft_d_r3.test.mjs` "D3-M-1" |
| **C3-H-1** | High | **Fixed** | An advance's H3 ask is keyed by its collection and the count of that collection's advances that went out before (`advance:eth:{col}:{n}`), not by the buy's id: a buy that waits on the day's value is still dropped (it never holds the Relay leg) and reopened, and the reopened buy asks under the SAME key, so the wait is one ask in the tripwire's demand however long it lasts. The limiter counts in `done` only what went out (unchanged). With five collections waiting at once the demand is at most the day's released value plus one advance each, far under 3 × 25 ETH | `ethflow.rs` `home_act` (`NeedAdvance`) | `tests/audit_nft_r3.rs` `r3_c3_h1_…` ×2 (the canister's real limiter) |
| **B3-M-1** | Med | **Fixed** | The scan's log read names each move's token and sender (the transform); receipts are read in `sale_order`: one per token and per sender at a time, oldest first, so a flood (one token ping-ponged, many tokens from one wallet, a chain of fresh wallets, even a fresh NFT per transfer) is read after every real sale queued before it. A queue over 512 drops the last in that order and reads their blocks again once it has room (`rescan`). A judged transaction without a sale is kept apart (`judged`, 8,192 per collection) and never takes a sale's place in the cache. The first post needs the same three counted sales as before, which a flood can no longer hide | `ethflow.rs` `sale_scan`, `sale_order`, `SaleScan`; `ethnft.rs` `transfer_logs_body`, `transfer_moves_of`, `Move`; `ethnet.rs` `eth_transfer_moves` | `audit_nft_r3.rs` `r3_b3_m1_…` ×5 |
| **C3-M-1** | Med | **Fixed** | Bounded exits for the way back and the sweep, none spending ETH that is not the buy's: `BackFunded` holds other collections' new buys for its 2 h quote window at most, then waits `Arrived` again (earmarked, nothing signed; it may take a listing) and retries a day later; `BackBridging` past its deadline + grace is watched 24 h more (it must hold the Relay leg while watched), then `Stuck` without a watch, off the leg; `eth_give_back` stops a `BackFunded`, unwatches a late `BackBridging`, and closes an unwatched `Stuck` as a loss (`returnAdvance` of 0: the module books it; a sweep ends with nothing). A reverted way-back deposit asks again for what is left; dust ends the buy | `ethflow.rs` `step_buy`, `buy_pass`, `back_now`, `give_back_now`, `mined_eth` | `audit_nft_r3.rs` `r3_c3_m1_…` ×3, `r3_c3_i3_…` |
| **E3-M-1** | Med | **Fixed** | Every balance read of the flow (a watch's start in `Funded` and at the way back's deposit, the arrival in `Bridging` and `Stuck`, the refund on Ethereum in `BackBridging`, the sweep) is made only at a pin at or above our last mined transaction's block and the watch's own pin (`Watch.pin`); lower, it waits | `ethflow.rs` `eth_floor`, `step_buy`, `home_act`, `eth_act`, `sweep_pass` | `audit_nft_r3.rs` `r3_e3_m1_…` ×2 |

**Both Highs and all three Mediums are closed: 0 Critical, 0 High, 0 Medium open by this round's own reading.** Also closed: the
flag in `bayc10-decisions.md` §5 (§6 below), eleven of the Lows (§7) and a monitor bug found while fixing (F3-1, §4). Round 4
re-reviews the delta (§11).

## 1. D3-H-1: a delivery is recorded from its own receipt, and a delivery noted sent is never refunded

**The canister.** At finality (`Moved`, block ≤ the lower finalized), `delivery_pass` reads our delivery transaction's receipt at the
pin (`eth_sale_receipt`, the log-bearing projection) and records when it succeeded in the same block and holds the collection's
`Transfer` of the token from our address to the ticket's destination (`ethnft::delivered_in`). Where the token is now is not asked:
a redeemer who sold it into a bid, moved it to another wallet or a sweeping smart wallet was delivered to. A receipt in another block
(a reorg before finality) waits for that block's finality; a receipt now showing a revert goes to `MarkAfter` (the transfer did not
happen); a receipt without the `Transfer` is an error that waits for the operator (it cannot happen for our own `transferFrom`).

**The contract** (`src/nft/EthNftWrapper.sol`, the brief's "contract-side guard if needed"). The canister alone cannot make "refund
never fires for a delivered ticket" hold: any failure of the record step, for 30 days, re-opens D3-H-1. So:

- `noteSent(id, txHash)`: custodian only, a `Resolved` ticket, a non-zero hash; stores `sentTx[id]`, emits
  `DeliverySent(ticketId, tokenId, txHash)`; changes no ledger number and no stage.
- `refund(id)` on a `Resolved` ticket: the 72 h grace first, as before; then without `markUndeliverable`, a ticket noted sent
  reverts `DeliverySentNotRecorded()`, and one never noted sent refunds at `deliverBy + LATE_DELIVERY_WINDOW` as before (the exit for
  a custodian that is gone before it sent). With `markUndeliverable` (the canister's word that no version can confirm, e.g. a
  transfer a reorg dropped for good) it still refunds at `deliverBy + REFUND_GRACE`.
- The canister calls `noteSent` as soon as it sees the transfer mined (`Moved`), before finality, as a home-chain act next to the
  records and marks (`Step::Sent`, `Delivery.sent`), also while paused (it records what already left). The late window therefore
  closes minutes after the NFT moved; the record follows at finality.
- Why not "refund only after `markUndeliverable`": that removes the late window for a custodian that is gone before it sent, the
  exit rounds 1–4 of the Bitcoin `Wrapper` built deliberately and the incident runbook relies on (`docs/v5/launch-gates.md`). What the
  guard gives up instead: a custodian gone AFTER `noteSent` and before the record leaves that ticket open for good; its NFT was sent.

**Every other place that reads current ownership** was checked (`ownerOf` in `ethflow.rs`, `ethnet.rs`, `nftnet.rs`): each one is a
present-state precondition on a token only our key can move (ours before a delivery or a return is signed, ours after a fill mined
and at its finality before the mint, ours at a deposit's finality before its mint, the offerer's for a listing). None judges a past
event that someone else can change afterwards. The monitor's custody check did (below).

**The monitor** (paired, or every honest case above becomes a false SEV1): a wrapper-held item owned by someone else is exempt only
if the canister has a delivery of that collection and token whose ticket, read on the wrapper, resolved to THAT item and its
destination, and either the transfer mined (`Moved`, `Record`, `Recording`, `Recorded`: any owner) or it is being mined (`Sent`,
`Cancelling`: the owner is the destination; D3-I-7). A buy's, a deposit's or a stray's token is never exempt (D3-L-1); a ticket read
that fails pages. `eth-deliver-deadline` for a mined delivery says so, SEV1 while `noteSent` has not landed, SEV2 once it has. The
canary judges `DeliverySent` against the canister's records (a mined delivery of that ticket naming that transaction), SEV1 otherwise.

## 2. C3-H-1: a waiting advance is one ask

- `home_act` asks H3 for an advance under `advance:eth:{chain:collection}:{n}`, `n` the number of that collection's buys (sweeps
  apart) past `NeedAdvance`. A buy whose advance waits (the day's value, or the custody room) is dropped as before, so it never holds
  the Relay leg, and the round robin tries the next collection; its successor asks under the same key. An advance that reverted
  returns its buy to `NeedAdvance` and asks under its own key again (it released nothing, and `record` is idempotent per key).
- The tripwire's demand is therefore the day's released value plus at most one waiting ask per collection. Stage 0, the five
  collections: at most 25 + 8.3 + 4.2 + 3 × 2.1 ≈ 44 ETH against the 75 ETH that trips. Honest waiting cannot trip it.
- The way back, `eth_give_back` and the sweep ask H3 nothing (their value goes to our own address and back to its own module); what
  changed for them is that a returned advance no longer re-asks at once: a collection waits a day after its advance came back
  (A3-L-1, §7), and asks once when it does.
- Test: the real `outflow::Outflow` in the fake (as part C's PoC). Milady's advance goes out; Azuki's waits 200 ticks: two asks in
  all, no freeze; a day on, Azuki's advance goes out under the same key. And the PoC's own path (three Relay refunds, returned, the
  module still wanting): no freeze, the collection waits out its day.

## 3. B3-M-1: a flood of transfers no longer starves the sale sample

- **The transform** (`ethnft::transfer_logs_body`) keeps, per transaction, its block, and the least `(sender, token)` of its moves
  (so the entry does not depend on the provider's log order): `[txHash, block, tokenId, from]`. `EthIo::eth_transfer_moves` reads it
  (`eth_transfer_txs` stays, derived from it).
- **The order** (`ethflow::sale_order`): pending transactions sorted oldest first, each ranked one more than the older pending moves
  of its token or from its sender; read by rank, then age. A real sale of a token and seller that moved once in the queue has rank 1
  and is read before any flood transaction of a repeated token or sender; a flood of distinct NFTs from distinct wallets is read
  before it only for its transactions OLDER than that sale. So a sale waits at most for what was queued before it (bounded by the
  flooder's NFTs), never for what comes after. Before: newest first, truncated, never re-read.
- **The queue** (`SaleScan.pend`, 512): the last in the order are dropped and their blocks recorded (`rescan`); they are read again,
  a chunk at a time within the pass's budget, once the queue is under half full. A flood that keeps it full keeps being read in order.
- **The cache**: a judged transaction without a sale goes to `SaleScan.judged` (8-byte hash prefix and block, 8,192 per collection,
  pruned to the window) and is never read again; `sale_cache` sorts sales first, so judged non-sales never push a sale out.
- **The first post** still needs three counted sales (the operator's round-2 call #9); a flood can no longer displace them.
- Cost: unchanged per pass (13 log chunks, 16 receipts); each flood transaction costs one receipt read, once.
- Tests: the PoC's flood in four shapes (the PoC's ping-pong, one wallet, a chain of fresh wallets, a fresh NFT per transfer): every
  real sale read in the first pass, the floor confirmed, the deposit minted; the first post under a flood; 2,100 judged non-sales
  never evict a sale; a real sale dropped from a full queue read again from its blocks (38 passes); the order itself.

## 4. C3-M-1: the way back's exits (and the sweep's), and a monitor bug found on the way

- **`BackFunded`**: holds other collections' new buys only during its quote window (`BACK_QUOTE_SECS`, 2 h from `since`); with no
  deposit signed by then it returns to `Arrived` (its ETH still its own earmark; its quote dropped; nothing was signed, so nothing can
  be spent twice) and its next way back is due `BACK_RETRY_SECS` (24 h) later. It may take a listing meanwhile. A sweep keeps its own
  2 h abandon (A2-L-1, unchanged).
- **`BackBridging`**: past its order's deadline + the 1 h grace it stays watched `BACK_LATE_SECS` (24 h) more, holding the Relay leg
  (a watch must: arrivals and refunds are balance rises only one leg may cause). Then `Stuck { why }` without a watch: off the leg,
  its collection's buys wait, the others go on. A Relay payout later than that is not read as this order's (§10).
- **The operator's path** (`eth_give_back`, still `only_operator`, not while paused): `BackFunded` → `Arrived` at once;
  `BackBridging` past deadline + grace → unwatched `Stuck` at once; an unwatched `Stuck` (this one, or E3-L-3's post-arrival `Stuck`)
  → `Returning { wei: 0 }` → `returnAdvance` of 0: the module ends its transit and books the loss (revenue, then surplus, then the
  ETH-backed units), `Returned { 0 }`; for a sweep, `Returned { 0 }` and the leftover is written off. Every exit spends nothing:
  no ETH of the home balance is returned for a way back (it may be another buy's).
- **Wedges** (C3-I-3): `eth_give_back` refuses an `Arrived` buy holding no more than the way back's gas reserve; a way back whose ETH
  no longer covers that reserve keeps it as leftover and returns the advance as a loss; a reverted way-back deposit asks again for
  what is left (its ask is re-computed).
- **F3-1 (found while fixing; the monitor)**: `ic.mjs` hands `eth_nft_state()` through `plain()`, which turns every blob into hex text,
  and `ethnft.mjs` / `records.mjs` read that text as UTF-8 bytes (`Buffer.from(text)`). Against a live canister no blob compared equal:
  every honest redemption would page SEV1, bought and deposited tokens were never checked, the canister's floor reading was never
  found (so D8 could not pause), and every `Wrapped` / `Delivered` / `Undeliverable` of an NFT wrapper would page a false SEV1. The
  monitor's tests used byte-array states. Both helpers now take hex text too; a test runs the checks on `plain()`-shaped state and
  fails on the old code. By the README's scale this is a Medium of the services (the out-of-band detector blind in production),
  found and fixed here; round 4 should re-check it.

## 5. E3-M-1: the pin floor for every balance read

- `ethflow::eth_floor(st, watch)`: the pin must be at or above `last_mined`'s block (every spend `eth_out` counts is in the balance
  there) and the watch's own `pin`; otherwise the read waits a tick, with the reason in the buy's note.
- Applied to the `Funded` watch start (the quote stays in the inbox), the way back's watch (taken in `eth_act`, whose B-M-4 guard
  already refuses a lower pin; now it records its pin), `Bridging`, a watched `Stuck`, `BackBridging`'s Ethereum refund, and the sweep.
- Tests: part E's `pin_back` fake. (a) the pin one bucket under a counted fill: Azuki stays `Bridging` (no phantom), the earmarks
  never exceed the balance, and Relay's real fill later arrives as Azuki's. (b) a quote handed in at the lower pin waits; at the pin
  the watch starts at the real balance and the real arrival is seen.

## 6. The canister's providers stay keyless (bayc10-decisions.md §5, the flag)

- **golive-check** `--install-arg <hex | file>` (else the monitor file's `install_arg_hex` when it is real Candid): every `http(s)://`
  URL in the argument (a Candid text: the LEB128 length before it, exactly that many printable bytes) is judged. Keyed, a FAIL that
  prints only `scheme://host/…`: userinfo, a query, a fragment, `${`, a run of 16+ key characters in a path segment, or a keyed
  provider's host (Dwellir, Infura, Alchemy, QuickNode, Blast, Chainstack, GetBlock; drpc with a dkey). Under `--phase1` on an NFT
  stack no install argument is a FAIL. The dry run builds a real `Init` (the monitor's `candid.mjs` types) with the keyless pair
  (`xrpc.cl/eth`, `ethereum-rpc.publicnode.com`) and shows the refusal of a fake Dwellir key, which it checks is printed nowhere.
- **The monitor's argument diff** compares provider URLs in full and prints them as `scheme://host[:port]/…` (`config.mjs`
  `redactUrl`); a change hidden in a path or key reads "reconfigured (a path or key changed)". The same redaction for the other alert
  texts that printed a configured URL (liveness, the Bitcoin canary's explorer base, config refusals).

## 7. The Lows (21 open at `a37f54b`: 14 new, 7 carried)

| ID | Status | What |
| --- | --- | --- |
| A3-L-1 | Fixed | A collection whose advance came back waits `RETURN_COOLDOWN_SECS` (24 h) before the next one; the cost of a round trip is stated (`phase1-nft-pooled.md` §1.1, the Trust page): `r3_a3_l1_c3_l2_…` |
| A3-L-2 | Fixed | golive-check `--launch` holds each wrapper's `name()`, `symbol()` and the stack's slug against the launch file by collection address (dry run: labels rotated by one are refused) |
| A3-L-3 | Partly | `--monitor-config`: a `<` placeholder anywhere is refused, Ethereum providers are counted by host, `evm.chain_id` must be 4663, `evm.guardian.address` must be the registry's `guardian()` (dry run: each refused). Not done: holding the D8 references against the launch file, `evm.stack_file` |
| A3-L-4 | Fixed (the operator's round) | Deposits count at the module's floor (`bayc10-decisions.md` §4) |
| B3-L-1 | Not fixed | The sale scan still needs byte-identical `eth_getLogs` from both providers (a reconciliation is a design change; §10) |
| C3-L-1 | Partly (the operator's round) | The advance is grossed up for the judged 3 % (`advance_bound`); the gate still assumes 0.5 % where the item cap binds |
| C3-L-2 | Fixed | `eth_give_back` on an `Arrived` buy only once it has waited `ARRIVED_HOLD_SECS` (6 h) from its first arrival, and a returned collection waits a day: the operator key can no longer churn a module |
| C3-L-3 | Fixed | The way back's arrival is a 4663 rise of the order's minimum less 1 % (`BACK_ARRIVAL_SLACK_BPS`), never more than the order sent: `r3_c3_l3_…` |
| D3-L-1 | Fixed | §1 (the monitor exempts only a delivery of the item held now): `audit_nft_d_r3` "D3-L-1" |
| D3-L-2 | Fixed | The canary judges the earmarks against a balance read after the records; its key names the buys: `audit_nft_d_r3` "D3-L-2" |
| D3-L-3 | Fixed (the operator's round) | The template's pair is Dwellir + `xrpc.cl/eth`, the deposit scan in 500-block ranges (`bayc10-decisions.md` §5) |
| E3-L-1 | Fixed | The 6 h hold and the 48 h are counted from the first arrival (`EthBuy.arrived_at`, `back_at`); a reverted fill restarts neither: `r3_e3_l1_…` |
| E3-L-2 | Not fixed | Gas drift into an earmark can still deadlock an arrival with a way back until the pot is topped up (the monitor's gas-pot page); making deliveries respect earmarks trades a holder's delivery for the accounting (§10) |
| E3-L-3 | Fixed | The operator closes an unwatched `Stuck` buy as a loss (§4), which also unpins its collection for upgrades: `r3_e3_l3_…` |
| A2-L-2 | Decided | J7 (the operator, `bayc10-decisions.md` §4) |
| A2-L-1 | Fixed (the operator's round) | The advance is the buy's bound and the leftover is swept to its own module |
| A-L-2, B-L-5, C-L-5, D-L-3, D-L-6 | Unchanged | As `fixes-round2.md` §7 |

**Lows: 21 — 8 fixed here (A3-L-1, A3-L-2, C3-L-2, C3-L-3, D3-L-1, D3-L-2, E3-L-1, E3-L-3), 4 closed by the operator's round
(A3-L-4, D3-L-3, A2-L-1, and A2-L-2 decided), 2 partly (A3-L-3 here, C3-L-1 by the operator's round), 2 not (B3-L-1, E3-L-2),
5 unchanged from earlier rounds (A-L-2, B-L-5, C-L-5, D-L-3, D-L-6).**

## 8. The Infos

Fixed on the way: C3-I-3 (§4), C3-I-4 (`relayeth.rs`'s comment), D3-I-1 (a `since` of 0 pages from first seen), D3-I-3 (the Trust
page: "once a collection has a floor", its first post waits for three counted sales, "Relay's estimated fee", "48 hours after it
first arrived"), D3-I-4 (a `BackSending` page after 1 h), D3-I-7 (§1), E3-I-2 (the `.did` checked with the reviewers' `didcheck`
against every `BPhase`, `DPhase`, `XPhase`, `EthKind` and `Step` case, the new `opt` fields present: all ok; the check is kept in
`fixes-round3-poc/`, not as a committed test, to keep the canister's dependencies unchanged), B3-I-2 (bounded as before, now one
receipt per flood transaction ever). Unchanged: A3-I-1..3, B3-I-1, C3-I-1, C3-I-2, C3-I-5, C3-I-6, D3-I-2, D3-I-5, D3-I-6, E3-I-1,
E3-I-3, E3-I-5.

## 9. New judgment calls for the operator

1. **The contract change (D3-H-1)**: `noteSent` switches the 30-day late refund off for a ticket whose delivery mined. A custodian
   gone after `noteSent` and before the record leaves that ticket open for good (its NFT was sent). The alternative, refunds only
   after `markUndeliverable`, would remove the exit for a custodian gone before it sent.
2. **The way back's clocks** (C3-M-1): a 2 h quote window during which other collections wait (`BACK_QUOTE_SECS`), a retry a day
   later (`BACK_RETRY_SECS`), and a way back watched a day past its order's deadline before it is let go (`BACK_LATE_SECS`). D4's
   48 h are unchanged, now counted from the first arrival (E3-L-1).
3. **Closing a lost way back**: after the watch ends, the operator's `eth_give_back` books the advance as a loss (`returnAdvance` of
   0) once Relay says the order is dead. A Relay payout later than the watch lands at our 4663 address unattributed (§10).
4. **A day between a returned advance and the next** for the same collection (`RETURN_COOLDOWN_SECS`, A3-L-1).
5. **`eth_give_back` on an arrived buy only after 6 h** (C3-L-2; the monitor's `Arrived` page fires at 24 h, so the page's advice
   still works).
6. **The way back's arrival at the order's minimum less 1 %** (C3-L-3), not half.
7. **The sale scan's order** (one move per token and per sender first, oldest first) and its bounds: 512 queued, 8,192 judged
   non-sales per collection (8-byte hash prefixes), a dropped range read again once the queue is under 256.
8. **H3's key for an advance** is per collection and per count of its advances (C3-H-1): a collection's waiting advance is one ask.
9. **golive-check refuses what looks keyed** (§6): a 16-character run in a path, or a keyed provider's host. The canister's pair must
   be keyless (the template's `xrpc.cl/eth` + `ethereum-rpc.publicnode.com` pass); a keyless provider that trips the heuristic needs
   the check changed, not overridden.
10. **The deadline page for a mined delivery** is SEV1 while its `noteSent` has not landed, SEV2 once it has.

## 10. Left open (for round 4)

- B3-L-1 (strict `eth_getLogs` consensus), E3-L-2 (gas drift, the arrival cap and the way back), C3-L-1's gate estimate where the
  item cap binds, A3-L-3's D8 references against the launch file, and the carried Lows (A-L-2, B-L-5, C-L-5, D-L-3, D-L-6).
- The residual of C3-M-1's bound: a way back is let go a day after its order's deadline; a Relay payout after that is not read as its
  (a later forward buy's refund or arrival watch could read it as its own; nothing is spent twice, but a module can be credited
  another's ETH). The robust fix is still the reviewers': read Relay's fill by its order id, not a balance delta (E3-I-4).
- The residual of the late window (§1, §9.1).
- The monitor's residual of D3-H-1 (found by the PoC re-run): a delivery the canister still has as `Sent` (our transfer mined, the
  canister not yet ticked) whose NFT the redeemer already moved on pages SEV1 `eth-not-held` until the canister's next tick moves it
  to `Moved`; the exemption for `Sent` needs the owner to be the destination. A false page for at most one tick interval.
- Every Info listed as unchanged in §8.

## 11. As built (2026-09-27)

**Commits** (branch `nft-side`, on `a37f54b`):

| Commit | What |
| --- | --- |
| `dd29f8f` | Contracts: `EthNftWrapper.noteSent`, `sentTx`, `DeliverySent`, `refund`'s late-window guard (D3-H-1); `test/ethnft/EthNftWrapperSent.t.sol`, the invariant suite's `noteSent` and late-refund actions |
| `3f90f03` | Monitor: D3-H-1's exemption by item, D3-L-1, D3-L-2, D3-I-1, D3-I-4, D3-I-7, `DeliverySent`, URL redaction, F3-1; `monitor/test/audit_nft_d_r3.test.mjs` |
| `0e14558` | Go-live: `--install-arg` and the keyed-URL refusal, A3-L-2, A3-L-3 (partly); the dry run's real `Init` and refusals |
| `f8be4b0` | Site: the Trust page |
| `fb54018` | Canister: D3-H-1, C3-H-1, B3-M-1, C3-M-1, E3-M-1 and the Lows of §7; the `.did`; `tests/audit_nft_r3.rs`; the fixture re-encoded |
| (this document's commit) | This document; `fixes-round3-poc/` (the reviewers' PoCs re-run on this code); `docs/v7/phase1-nft.md` (§2.1, §2.2, §2.3, §3.1, §4.5, §4.6, §11.10), `phase1-nft-pooled.md` (§1.1, §3.4) |

**The reviewers' PoCs on this code** (`fixes-round3-poc/README.md`): every PoC of the five Highs and Mediums fails on its attack
assertion (C3-H-1 at the freeze; C3-M-1 both; E3-M-1 both; B3-M-1's attack and first post; D3-H-1's canister half); C3-L-2, C3-L-3,
E3-L-1, E3-L-3, A3-L-2, D3-L-1, D3-L-2, D3-I-1 and A3-L-1's canister PoC (the cooldown) too. Still passing, as §7, §8 and §10 say:
C3-L-1 (partly), E3-L-2 (not fixed; its strict run first fails at a setup step), A3-L-3's swapped D8 references (partly), D3-I-2
(unchanged), and the forge A3-L-1 PoCs (the Floor Module's accounting is unchanged on chain; the fix is the canister's cooldown). The
monitor's "D3-M-1" PoCs page because their fake world has no `ticket(uint256)` (the new check reads it, and a failed read pages by
design); with it answered, only the one-tick `Sent` window of §10 remains.

**Results:**

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites; +9 `EthNftWrapperSent.t.sol`, +1 invariant, +1 handler test) |
| The Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **621 passed**, 0 failed (was 600: +17 `tests/audit_nft_r3.rs`, +3 `tests/ethnft.rs` `r3_d3_h1_…`, +1 `tests/wrapper_abi.rs`); `ethnft::tests` extended |
| The `.did` | parses (106 types) and matches every `BPhase`, `DPhase`, `XPhase`, `EthKind` and `Step` case, the new `opt` fields present (the reviewers' `didcheck`, `fixes-round3-poc/didcheck.*`) |
| Timelock `cargo test` (and `--features rehearsal`) | 15 passed (15); unchanged |
| ops | **56 passed**; `tsc --noEmit` clean (unchanged; decodes the re-encoded fixture) |
| monitor | **190 passed** (176 + 14 `audit_nft_d_r3`) |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | 72 passed; `tsc --noEmit` clean |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 checks ok on the v3–v6 stack; 536 on the five-collection NFT stack; **652** on the launch go/no-go with `--canister --canister-evm --launch --monitor-config --install-arg`; 40 refusals `ok`, new: a keyed provider in the install argument (the fake key printed nowhere), `--phase1` with no install argument, a guardian that is not the registry's, a placeholder / one provider twice / `evm.chain_id` 1, the launch file's labels rotated by one) |

**Reproducible build** (`autobuyer/Dockerfile.build`, linux/amd64, two `git archive` exports of `fb54018`, each
`docker build --no-cache`): the same hashes both times.

```
cbe337d84c1120e32eaade5f01ee496940de129cf7bfb5f616a13670a03fb360  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

`buyer.wasm` is new (6,753,176 bytes; 1,280,985 gzipped with `gzip -9 -n`, under the 2,000,000 proposal limit) and replaces
`0e54b2cd…2830` (`bayc10-decisions.md`) for any install of this branch. The timelock is unchanged. Nothing was installed; the
exports and the two build images were removed (Docker's shared build cache left alone).

**The delta round 4 must cover:** `git diff a37f54b..HEAD` (with `55d8d32..a37f54b`, the operator's round, not yet reviewed):
`src/nft/EthNftWrapper.sol`, `test/ethnft/{EthNftWrapperSent.t.sol, EthNft.invariant.t.sol}`;
`autobuyer/canister-rs/{etch_autobuyer.did, src/{ethflow,ethnet,ethnft,relayeth}.rs, tests/{audit_nft,audit_nft_r2,audit_nft_r3,ethnft,wrapper_abi}.rs,
tests/fixtures/eth-state-round2.hex}`; `monitor/src/{argdiff,canary,candid,config,ethnft,evm,liveness,records}.mjs`,
`monitor/test/{argdiff,audit_nft_d_r2,audit_nft_d_r3}.test.mjs`; `script/golive-check.sh`, `scripts/golive-dryrun.sh`;
`web-rook/src/rook/phase1Trust.ts`, `web-rook/test/phase1.test.ts`; the docs above.
