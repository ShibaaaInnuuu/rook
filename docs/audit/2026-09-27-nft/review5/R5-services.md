# Round 5 re-review, area D: services, the monitor's round-4 pages, the IDLs and the Trust page (R5-services)

Reviewer: independent AI re-review (part D), 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at
`1dc2a61`. The delta was `git diff 2735dac..HEAD -- monitor/ ops/ web-rook/ autobuyer/canister-rs/etch_autobuyer.did`, which covers
the monitor and runner commit `8018070` and the site commit `548f214`.

**Read:**
- `../README.md`, `../review4/README.md` and `../review4/R4-services.md`, with its PoCs;
- `../fixes-round4.md` §0, §4–§10 and `../fixes-round4-poc/D.audit4_nft_d.output.txt`. The fixer's claims were checked against the
  code and not taken on trust.

**Code read at HEAD:**
- `monitor/src/{ethnft,candid,alerts,config}.mjs`, `ops/src/ic.ts` and `ops/test/audit_nft_r4.test.ts`;
- `web-rook/src/rook/phase1Trust.ts` and `web-rook/src/content/launch-gates.md`;
- the canister where a service depends on it:
  - `ethflow.rs`: `step_buy`, `give_back_now`, `LateOrder`, `payout_pass` / `judge_payout`, `resolve_eth` / `reprice` / `cancel` /
    `retag` / `superseded`, `mined_eth`'s fill `Stuck`, `note_sent` and the Record step;
  - `canister.rs` `validate_custody`, and `outflow.rs` `exit_fits`;
  - `src/nft/EthNftWrapper.sol`: `Stage`, `noteSent`, `recordDelivery`, `refund`.

**Nothing in the worktree was modified except this report and `review5/poc/D/`.** Nothing was committed, pushed, deployed or
signed. Every PoC ran in a private copy under `/tmp/rook-r5/D/`:
- `monitor/`, `ops/` and `scripts/`, cloned with `cp -Rc`;
- `autobuyer/canister-rs`, with its `target/` cloned for speed.

That copy was removed at the end. No network was used.

`deployments/stack-9466301.json.first` shows as untracked in the worktree. It is not this part's.

Severity scale: `../README.md`. A PoC named "D5-…" passes while the weakness is present. One named "holds: …" passes while the fix
holds.

## Verdict for area D

**The round-4 service fixes hold for the routes review 4 named.**

**D4-L-1 is fixed, and it fails closed.** A `Sent` / `Cancelling` delivery whose NFT is no longer at the destination is exempt
only in this case:
- its own receipt has status 1 on **every** provider;
- that receipt holds a log whose `address` is this collection, with 4 topics equal to `Transfer`, the custodian, the destination
  and the token.

Every other outcome pages the SEV1:
- another contract, token, sender or recipient;
- a revert;
- mined on one provider only;
- either provider erroring.

A `Cancelling` delivery is exempt only through `of`, the delivery it cancels, and never through the cancel. The exemption cannot
mute a real loss: that log proves our own transfer moved this token out of custody to the ticket's destination. The ticket check
(same item, same destination, still `Resolved`) still follows it.

**D4-L-2 is fixed for its route.** The last 64 blocks are read again every poll, in one request, and a deposit read twice is one
`unclaimed` entry and one page. Two residuals (**D5-I-2**):
- a poll whose head has not passed the cursor re-reads the whole 7,200-block window (15 requests);
- a logs node more than 64 blocks behind still loses a deposit.

**The C4-M-1 pages fire where the canister's state machine says they should.** This was checked on bytes written by the 1dc2a61
canister's own encoder:
- `eth-back-late` for a way back still watched past its deadline;
- the "still WATCHED" `Stuck` page for a watched forward order;
- `WRITTEN OFF` after the write-off;
- SEV3 `eth-late-order` for every written-off order not yet credited, including one already closed as a loss and a written-off sweep;
- no SEV3 once credited.

**The IDLs decode every round-4 state and are subtype-safe.** `monitor/test/d4-state.hex` is genuinely canister-encoded:
- it is byte-identical to review 4's fixture;
- the 1dc2a61 build's own `EthState` decodes it;
- the monitor's and the runner's IDLs decode it, the canister's `eth-state-round4.hex`, and this review's `d5-state.hex`.

**New: two Lows, three Infos.**
- **D5-L-1** is a regression from D4-I-1's fix. It raises a false SEV1 when a recorded delivery's record lands during the poll.
- **D5-L-2** is a page-text regression from D4-I-3's fix. The one SEV1 for a bought NFT that is not ours after its fill now says
  the buy is "still WATCHED, holds the Relay leg", and that the operator should wait a day.

No Critical, High or Medium.

## Counts (new in round 5, area D)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 2 | 3 |

Carried, not recounted: D-L-3, D-L-6 (Low, unchanged), D3-I-2 (Info, unchanged), and D4-I-2 / D4-I-3 (Info, partly; see §1).

## 1. Round-4 items in scope

| ID | Sev (R4) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **D4-L-1** | Low | **Fixed** (residual **D5-I-1**) | `ethnft.mjs:250-263` `deliveredOn`, `:512-521`. Every forged or misattributed receipt shape was tried: another collection's `Transfer`, another token, a sender other than the custodian, a recipient other than the destination, status 0, mined on one provider only, provider A erroring, provider B erroring. Each pages (PoC "holds: D4-L-1 …"). `Cancelling`: the delivery won the race and is exempt; the cancel mined and the NFT is elsewhere, and it pages. **Which tx is read:** `Sent.tx` / `Cancelling.of`, which is the delivery's **newest** version. `reprice` and `retag` (`ethflow.rs:2474-2492`, `:2431-2446`) re-point it at every re-price, and the earlier versions live only in `line.earlier` / the cancel's `rivals`. The monitor's `EthTx` does not decode those (`candid.mjs:211`). An earlier version that mined therefore fails closed: a false SEV1, never a silent exemption (D5-I-1). **Can it mute a theft?** No. The exemption needs our own delivery's receipt to carry the custodian → destination `Transfer` of this token. After that the token is outside custody by our own act. A theft of the same token would need it to come back and be stolen again inside one tick, and as a returned token it would be a new item or a deposit, which is not exempt. |
| **D4-L-2** | Low | **Fixed** (residuals **D5-I-2**) | `ethnft.mjs:428-433`. With the cursor at `done`, the next poll reads `[done + 1 − 64, head]` in one request (69 blocks at a 60 s poll). A deposit read by two overlapping polls is one `unclaimed[tx]` (`??=`) and one `eth-unclaimed:<tx>` page (PoC "holds: D4-L-2 …"). **Cost:** one request per collection per poll in the normal case. When `done ≥ head`, the `else` branch is `head − 7,200`, and the `from > head` guard never holds, so the whole window (15 requests) is read. That happens on no new block since the last poll, or a load-balanced node one block behind. It is harmless but not what the comment says. **Depth:** a node more than 64 blocks behind that answers without an error still loses the deposit (D5-I-2). |
| **D4-I-1** | Info | **Fixed, and it opened D5-L-1** | `ethnft.mjs:527` requires `stage == 2` (`Resolved`). A refunded ticket (stage 4) no longer exempts (the fixer's test). But `recordDelivery` sets stage 3 **and** deletes `itemOf` in one transaction (`EthNftWrapper.sol:389-402`). The monitor reads `itemOf` at the start of the poll and `ticket()` at its end. A record that lands in between turns an honest, recorded delivery into a SEV1 (D5-L-1). |
| **D4-I-2** | Info | **Partly** | `/status` heads are redacted before the cut (`alerts.mjs:97`): review 4's PoC now fails at its first assertion. The second half is unchanged: `redact` matches the whole value only, and the checkers still cut texts before `format` redacts them. The new `deliveredOn` error path also cuts `err.message` at 100 characters. This is still not reachable, because no rpc error text carries the URL. Carried as Info. |
| **D4-I-3** | Info | **Partly** | Fixed: the `Stuck` texts (split by `late`) and the way-back text. Not fixed: the `Arrived` page still counts from the phase's `since`, not `arrived_at`; a sweep's `BackFunded` / `BackSending` pages still say "it could not fill"; `sent = true` still also means "nothing to note" (`ethflow.rs:2723-2727`). **The `Stuck` split is wrong for a fill-Stuck buy (D5-L-2).** One text is now stale since A4-L-2 (D5-I-3). |
| C4-M-1 pages | new | **Hold** (except D5-L-2) | See §3 and the PoC "holds: the C4-M-1 pages …" on canister bytes. |
| IDLs / E4-I-4 | new | **Hold** | See §4. |
| **D-L-3** | Low | **Unchanged (partly fixed, as round 1)** | The runner's `DEPOSIT_LOOKBACK_BLOCKS` is still 7,200 (`ops/src/ethnft.ts:28`). The monitor still remembers an unjudged transfer past its window. `ops/src` is untouched by the delta. |
| **D-L-6** | Low | **Unchanged** | golive-check's `--canister-evm` is still a FAIL under `--phase1` (`golive-check.sh:1004-1006`). The canister still never reads `EthNftWrapper.custodian()`: `ethnft.rs:918` `CUSTODIAN` is referenced nowhere. |
| **D3-I-2** | Info | **Unchanged** | `superseded` (`ethflow.rs:2537-2558`) is not in the delta. A foreign transaction at a cancel's nonce still keeps the line "read again next tick". It needs the canister's key, which is trusted. |

## 2. Findings

### Low

#### D5-L-1: a delivery whose record lands on 4663 during a poll pages a false SEV1 (introduced by D4-I-1's stage check)

- **Where:**
  - `monitor/src/ethnft.mjs:527`: the exemption needs `Number(t.stage) === 2`;
  - `:361` and `:525`: the wrapper's `itemOf` is read per item at the start of the poll, and `ticket()` only after every
    collection's reads, the floor references and the outbound canary;
  - `src/nft/EthNftWrapper.sol:389-402`: `recordDelivery` sets `Stage.Delivered` (3) and deletes `itemOf` in the same transaction.
- **Mechanism:**
  1. A redeemer moves the NFT on after our transfer mined and before its record lands. That window is about 15 min of finality,
     then up to a 10 min tick, then the `noteSent` tick, then the record tick. Moving in it is what an arbitrage reseller does.
  2. At that point the item is still wrapper-held, the owner is not the destination, and the delivery is `Record` / `Recording`
     (in `DELIVERY_MINED`).
  3. The poll reads `itemOf` → held, adds a suspect, and reads the canister's records (still `Recording`).
  4. The record mines on 4663 meanwhile.
  5. The ticket then reads stage 3, so the check fails and `eth-not-held` pages "An NFT left custody without a delivery of ours:
     pause, then look".

  Before 1dc2a61 the same state was exempt, because the stage was not read. Stage 3 with `itemId` equal to this item and
  `deliveryTx` equal to the delivery's tx means the wrapper itself recorded this very delivery.
- **Impact:** a one-poll false SEV1 on the custody-loss page. For one collection of 8 items there are more than 30 requests between
  the first item read and the ticket read. At J7's 61 items over five collections a poll takes tens of seconds against a 60 s
  interval. So a sizeable share of the early-resold deliveries D4-L-1 was meant to quiet now page once, when their record lands.
  This is the same class and weight as D4-L-1: Low.
- **Trigger:** no attacker. An honest redeemer who moves the NFT before the record, plus the record landing mid-poll.
- **PoC:** `poc/D/monitor/test/audit5_nft_d.test.mjs`:
  - "D5-L-1": the canister-encoded `Recording` delivery (ticket 7), ticket stage 3 → SEV1; control, stage 2 → exempt;
  - "D5-L-1 (the window)": the request gap.
- **Fix:** accept stage 3 when `itemId == x.item`, the destination matches and `t.deliveryTx` equals the delivery's `tx`. Or re-read
  `itemOf(token)` before paging and drop the suspect if it is no longer held. Keep refusing stage 4, which is D4-I-1.

#### D5-L-2: the SEV1 `Stuck` page calls a fill-Stuck buy (a bought NFT that is not ours) "still WATCHED" and tells the operator to wait a day

- **Where:**
  - `monitor/src/ethnft.mjs:66-73`: the text is chosen by `late` alone;
  - `monitor/src/candid.mjs:192`: the monitor's `EthBuy` does not decode `last`, so it cannot tell a watched `Stuck` from an
    unwatched one;
  - `ethflow.rs:2604-2606` and `:1770-1773`: a fill that mined with `ownerOf` not ours, or a bought token not ours at finality, is
    `Stuck` with `last = None` (the watch ended at `arrive`, `:1796`) and `late = None`;
  - `give_back_now` (`:3419-3429`) closes such a buy as a loss on the **first** call.
- **Mechanism:** for these buys the page now reads: "Its order is still WATCHED (it holds the Relay leg for every collection): a
  late fill on Ethereum or refund on 4663 is read as its own. Once Relay confirms nothing more will come (and not before its
  deadline + 1 h + 24 h), eth_give_back(id) writes it off; a second eth_give_back(id) closes it". Every clause is false for this
  buy:
  - nothing is watched;
  - it holds no Relay leg;
  - there is no Relay order to wait on;
  - one call closes it.

  The page does not say that an NFT we paid for is at someone else's address. That is the only page for this loss: the custody
  check reads buys only in `Bought` / `Final` / `Minting`, so a `Stuck` buy's token is not owner-checked. The round-3 text said
  "no longer watched … closes an unwatched stuck buy as a loss", which was right for this case. A written-off sweep's `Stuck`
  page also says "returnAdvance of 0, the module books the loss". A sweep is instead closed with nothing (`Returned`, `:3421-3425`).
- **Impact:** operational. On the one SEV1 that reports a purchased NFT lost after its fill, the operator is told a Relay order is
  pending and to wait at least a day. No value moves because of the text. The `why` does name the owner.
- **PoC:** `audit5_nft_d.test.mjs` "D5-L-2", on the canister's own bytes (`d5-state.hex`, buy 2 and buy 6); control: buy 1 (a
  watched forward order) is right.
- **Fix:** decode `last: opt EthWatch` (record subtyping keeps older states decodable) and choose from three texts:
  - `last` present: watched;
  - `late` present: written off, with the sweep's own close;
  - neither: "not watched; the fill mined but the NFT is not ours: pause, then look; eth_give_back closes it as a loss once
    understood".

### Info

- **D5-I-1 (D4-L-1's residual: a re-priced delivery).** `ethnft.mjs:516` reads only `Sent.tx` / `Cancelling.of`, the newest
  version. When an earlier version mined (the canister re-prices after `LINE_BUMP_SECS` = 30 min; both versions are in the
  mempool) and the redeemer resells within the tick, the delivery pages a false SEV1. This fails closed, and it needs a gas spike,
  an early-version fill and a fast resale. Fix: decode `EthTx.earlier` and `rivals` (`.did:124`) and try every version of the
  line's `Deliver { wrapper, ticket }` (for `Cancelling`, the cancel's `rivals`). PoC "D5-I-1" (canister bytes: `line.earlier = [V1]`,
  `Sent.tx = V2`, V1's receipt only).
- **D5-I-2 (D4-L-2's residuals).**
  - `ethnft.mjs:428-430`: when `done ≥ head` the scan falls to `head − 7,200`, which is 15 requests instead of none or one. The
    guard `from > head` is dead code.
  - A logs node lagging more than 64 blocks and answering short without an error still loses a deposit. The runner's discovery still
    re-reads its window, so this needs a second failure.
  - Fix: scan to `min(head, finalized)` and keep the cursor at the finalized block, or keep the overlap and handle
    `done ≥ head` as "read `[head − 64, head]`".

  PoCs "D5-I-2 (a)" and "D5-I-2 (b)".
- **D5-I-3 (C4-M-1 page and text notes).**
  - `eth-late-order` is a non-persistent SEV3. It is sent once, and again only after the forget period, and nothing pages when the
    written-off order's payout actually lands at one of our addresses. The operator must watch Relay's status to use
    `eth_credit_payout`. Hardening: have the monitor watch for a balance rise at our 4663 or Ethereum address while an uncredited
    `LateOrder` exists.
  - The mined-delivery deadline page (`ethnft.mjs:110`) still says "the canister notes a mined delivery right after it mines".
    Since A4-L-2 it notes at finality, in the `Record` step (`ethflow.rs:2880-2885`). A delivery that mines in its last day now
    shows a routine ~15-25 min SEV1 "Look now … one unit unbacked", although the late window is 30 days away.

  PoC "D5-I-3" (non-persistent).

## 3. The C4-M-1 pages against the canister's state machine

| Canister state (`ethflow.rs`) | Monitor page | Right? |
| --- | --- | --- |
| Forward `Bridging` past deadline + grace → `Stuck`, `last` kept (`:1642-1646`); `in_relay()` true | SEV1 `eth-stuck` "still WATCHED"; write-off refused until deadline + 1 h + 24 h (`give_back_now:3450-3455`) | Yes |
| `BackBridging` past deadline + grace + 24 h: stays `BackBridging`, watched (`:1745-1754`) | SEV2 `eth-back-late` (from 30 min; persistent, key per `since`) | Yes |
| Write-off (`eth_give_back`): `Stuck`, `last = None`, `late = Some{credited: None}` (`:3406-3416`, `:3439-3462`) | SEV1 `eth-stuck` "WRITTEN OFF" + SEV3 `eth-late-order` | Yes |
| Written off, then closed (`Returning{0}` → `Returned{0}`), `late` kept | INFO returned + SEV3 `eth-late-order` | Yes |
| Written-off sweep | SEV3 yes; the `Stuck` text names `returnAdvance` (D5-L-2) | Text wrong |
| Credited (`payout_pass:3578-3581`) | no SEV3 | Yes (the canister's `eth-state-round4.hex`) |
| Fill `Stuck` (`last = None`, `late = None`) | "still WATCHED" | **No (D5-L-2)** |

**Unattributed ETH that goes unpaged.** A written-off order's payout landing later is unpaged beyond the one SEV3 (D5-I-3). A payout
landing while **another** order is watched is read by that watch: its rise over the watch's start counts as that order's arrival or
refund. `payout_pass` then refuses to credit it, because the ETH is no longer free beside the earmarks. Nothing in the monitor pages
that mismatch either. That needs Relay to pay after the operator was told it would not. It is the canister's residual (part C's to
rate), and it bears on the Trust page's wording (§5).

## 4. The IDLs

**Monitor** (`candid.mjs:190-193`):
- `LateOrder { watch { order_id, value, deadline }, back, let_go_at, credited : opt (blob, nat, bool) }` is a record-subtype of the
  `.did`'s `LateOrder`. The tuple is `record {0; 1; 2}`, as in the `.did`.
- `EthBuy.late : opt LateOrder` is optional.

**Runner** (`ops/src/ic.ts:47-48`): it does not name `late` and skips it.

**New variant cases:** none. `BPhase`, `DPhase`, `XPhase` and `EthKind` in the `.did` have the tags the JS IDLs list. `EthStep` /
`HomeTx` are not decoded by either service.

**Checked on bytes:**

| Bytes | Decoded by | Result |
| --- | --- | --- |
| `monitor/test/d4-state.hex` | the 1dc2a61 canister's `EthState` (`d5_fixture.rs` second test) | Decodes: 4 buys, 4 deliveries, 2 deposits, every `late` none. Also byte-identical to review 4's `poc/D/monitor/test/d4-state.hex`. It is canister-encoded (1eb5788 build), not hand-written. |
| `eth-state-round4.hex` | monitor, runner | Decodes |
| `d5-state.hex` (this review, the 1dc2a61 encoder, byte-identical on a re-run) | monitor, runner | Decodes |

A synthetic newer canister (extra fields in `Watch` and `LateOrder`) decodes in the monitor. An older one without `late` decodes to
`[]`.

## 5. The Trust page and launch-gates.md

| Statement (`phase1Trust.ts`) | Code | Holds? |
| --- | --- | --- |
| 12 a day per collection, **24 a day in all**, 3 mints a day per collection, 25 ETH a day, tripwire ×3; a waiting mint or advance counts once | `nft-pilot.json` `outflow` (12 / 24 / 3 / 6 / 25 ETH / 3); E4-H-1's next-mint key | Yes |
| Every item must leave within the delay less a day, each collection and the whole custody; the canister refuses an install that does not fit; 48 of J7's 61 can | `canister.rs:302-325` (per collection and summed, `min` global cap), `outflow.rs:150-156` (`days = delay / day − 1`) | Yes |
| noteSent "once the transfer is final … before it records it"; a reorg-dropped transfer is never noted | `ethflow.rs:2876-2889` | Yes |
| "a block too full of transfers to read is read through the marketplaces' own sale records instead, so it cannot stall the reading" | `eth_sale_moves` reads Seaport 1.5 / 1.6 and Blur v2 only. A block unreadable both ways is skipped after 3 tries (fixes-round4 §9 call 6) | "Cannot stall": yes. "Read … instead" is complete only for those marketplaces (Info, wording) |
| The way back "stays watched past its order's deadline, so a late payment is always read as that order's own and never as another's … a day after the deadline the operator may write it off … a payment that still comes afterwards is credited back to that same collection from its own transaction" | See the notes below the table | Overclaims (Info) |

Notes on the last row:
- **"A day after".** The write-off is refused until deadline + 1 h + 24 h (`give_back_now`).
- **"Never as another's".** This holds only until the write-off. A payout after it that lands while another order is watched is read
  by that watch (§3).
- **"Credited back … from its own transaction".** This needs three things:
  - the operator hands the transaction in;
  - the payer is the order's solver, or the input names the order id (§9 call 4);
  - the ETH is still free beside the earmarks.

  Suggested wording: "…is watched until paid, or written off by the operator once Relay confirms nothing more will come; a payment
  that still comes afterwards can be credited to that collection from its own transaction".

**`launch-gates.md` G7:** it matches `validate_custody` and fixes-round4 §6. Stage 1's row is unchanged, so D4-I-4 is still open
and is part A's.

## 6. What holds (summary)

- **D4-L-1:** fails closed on every provider error or disagreement. The log check covers contract, topics count, `Transfer`, `from`
  = custodian, `to` = destination and the token. `Cancelling` uses `of`, never the cancel. It cannot exempt a theft.
- **D4-L-2:** the overlap heals a short answer or a tip reorg within 64 blocks. A re-read deposit is one entry and one page.
- **D4-I-1:** a refunded ticket exempts nothing.
- **C4-M-1 pages:** right on canister bytes, except D5-L-2.
- **IDLs:** subtype-safe both ways, and `d4-state.hex` is genuinely canister-encoded.
- **Trust page:** the exit rule and noteSent-at-finality statements match the code.
- **Review 4's part-D PoCs re-run on 1dc2a61:** D4-L-1, D4-L-2, D4-I-1 and D4-I-2 each fail on their attack assertion (F3-1 still
  holds): `poc/D/monitor/review4-pocD-rerun-on-1dc2a61.output.txt`.
- **The fixer's `audit_nft_d_r4.test.mjs`:** 5/5 in the private copy.

## 7. PoCs and outputs (`review5/poc/D/`)

| File | Findings | Result |
| --- | --- | --- |
| `canister/d5_fixture.rs` (+ `d5_fixture.output.txt`) | Writes `d5-state.hex` from the 1dc2a61 canister's encoder: watched and fill `Stuck`, written-off (credited none, closed, sweep), a late way back, a re-priced delivery (`line.earlier`), a `Recording` delivery. It also decodes `monitor/test/d4-state.hex` with the current `EthState`. | 2/2 pass; 7,260 hex chars; byte-identical on a re-run |
| `monitor/test/audit5_nft_d.test.mjs` + `d5-state.hex` (`monitor/audit5_nft_d.output.txt`) | holds: D4-L-1 ×2, C4-M-1 pages, D4-L-2, IDL subtyping; D5-L-1 ×2, D5-L-2, D5-I-1, D5-I-2 ×2, D5-I-3 | **12/12 pass**: each fix holds; each weakness present |
| `ops/test/audit5_nft_ops.test.ts` (`ops/audit5_nft_ops.output.txt`) | The runner's IDL on `d5-state.hex` and `eth-state-round4.hex` | 1/1 pass |
| `monitor/review4-pocD-rerun-on-1dc2a61.output.txt` | Review 4's `audit4_nft_d.test.mjs` on HEAD | F3-1 passes; D4-L-1, D4-L-2, D4-I-1, D4-I-2 fail on their attack assertions (fixed) |

**To run:**
1. Copy `monitor/`, `ops/`, `scripts/` and `autobuyer/canister-rs` (keeping the `autobuyer/` layout).
2. Put the test files in their `test/` directories. For the fixture, run
   `D5_OUT=<monitor>/test/d5-state.hex cargo test --test d5_fixture` (toolchain 1.98.1).
3. Run `node --test test/audit5_nft_d.test.mjs` in `monitor/`.
4. Run `node --experimental-sqlite --import tsx --test test/audit5_nft_ops.test.ts` in `ops/`.
