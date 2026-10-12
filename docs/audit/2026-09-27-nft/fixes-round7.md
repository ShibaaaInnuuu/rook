# Fix round 7: the 2026-09-27 review of the Ethereum NFT side, round 7

Written 2026-09-28 on branch `nft-side` (at `effc270`: the round-7 re-review's reports and PoCs on top of fix round 6). It answers
`review7/README.md` and its six part reports (`R7-launch-plan.md`, `R7-canister-scan.md`, `R7-canister-wayback.md`, `R7-services.md`,
`R7-canister-upgrade.md`, `R7-canister-home.md`) finding by finding. For each it gives the fix, where it is, and the test that turns
the reviewer's PoC around: the test fails on the code before this round and passes now. `fixes-round7-poc/` holds the reviewers' PoCs
re-run on this code and this round's read-only probes. §9 ("As built") has the commits, the test counts and the reproducible-build
hashes.

Nothing was deployed, pushed or signed with a real key. The only chains written to were local anvils: the dry run's fork of 4663 and
the Seaport fork proof's fork of Ethereum, both with read-only upstreams. Network use was read-only JSON-RPC:
- `eth_blockNumber`, `eth_getLogs` and `eth_getTransactionReceipt` on the canister's Ethereum pair, to pin `--probe-depth`'s range (§2);
- the probe itself, run on the pair;
- the reproducible build's crate fetch.

No `eth_sendRawTransaction` was made.

**The rules every fix keeps** (as rounds 1–6):

- **Nothing is weakened to buy availability.** These are unchanged:
  - H1 (pause), H2 (custody caps), H3 (the outflow limiter and its tripwire), and "save, then send";
  - every judge refuses what it refused. The payout judge now credits a real depository payout (C7-L-1), but only through the
    depository's own records of the executed request, at an amount the operator names from Relay's records, within the old bounds.
- **No false sale is counted.** A provider's refusal never makes a sale count. It can only make the scan read again, slower.
- **Stable state stays decodable.**
  - Every new field is `opt` (`Option` + `#[serde(default)]`), and no enum case was added to a saved type.
  - The state the round-6 build wrote restores with nothing lost. `tests/audit_nft_r7.rs` `r7_the_state_the_round_6_build_wrote_…`
    runs on `a78a542`'s own `eth-state-round6.hex` bytes, kept as `tests/fixtures/state/eth-state-a78a542.hex`.
  - The round-2, -4, -5 and -6 fixtures were re-encoded only because their tests compare bytes and the type table grew.
  - The `.did` was checked against the Rust encoder on the round-2, -4, -5, -6 and -7 fixtures, with no difference either way. It is
    `service_compatible` with `a78a542`'s (`fixes-round7-poc/didcheck.output.txt`, review 7's `didcheck7` method).
- **Not changed** (operator decisions):
  - BAYC 10 ETH / `convBps` 1,000 / `reserveBps` 0;
  - option (c)'s staging;
  - H3's 25 ETH a day and trip ×3;
  - D1–D9, with D5 deposits on;
  - the canister's Ethereum pair (MEV Blocker + Tenderly);
  - keyless canister providers.
- No contract changed (`src/` and `test/` are untouched).

## 0. Summary

| ID | Sev | Status | Fix | Where | Test (the PoC, turned around) |
| --- | --- | --- | --- | --- | --- |
| **B7-L-3** | Low | **Fixed** | The lag is measured from where the scan was last current, not from the re-anchored forward walk. The canister keeps `SaleScan.current`: the final head of the last pass whose forward walk reached it. `behind` = the final head less that. The monitor pages its own Ethereum final head less `current`: SEV2 from 2,400 blocks (8 h), SEV1 from 7,200 (a day). So a scan that stopped running, and a re-anchored one, both page | `ethflow.rs` `SaleScan.current`, `sale_scan` (the forward walk's end, `behind`); `ethnft.rs` `SALE_LAG_*`; `monitor/src/ethnft.mjs` (`eth-scan-behind`; `checkEthNft` reads the head first) | `audit_nft_r7.rs` `r7_b7_l3_…` (three days all busy: `behind` reaches 21,598; SEV2 on 192 and SEV1 on 144 of 216 passes, was 1; back to 0 once served), `r7_b7_l3_b_…` (upgrade); `monitor/test/audit_nft_d_r7` (a stopped scan: SEV2 at 8 h, SEV1 at a day, still three days on) |
| **D7-L-1** | Low | **Fixed** | `--probe-depth` reads a FIXED range known to hold logs: Milady's `Transfer` over Ethereum blocks 26,012,410..26,012,909. That range holds exactly 10 logs on both providers, measured today at 63,500 blocks deep. The probe requires those 10 logs, and the receipt of `0x17133e8b…c093` in block 26,012,500 with its `Transfer` log and status 1. An empty or short list FAILs. The runbook's IC rehearsal reads the same range | `script/golive-check.sh` `depth_probe`, `PIN_*`; `docs/v7/phase1-nft.md` §5.1 | `fixes-round7-poc/D/probe_depth_pinned.py`: the reviewer's shallow provider (answers `[]` past 10,000 blocks) FAILs, and so do a short answer (9 logs) and a null receipt; an honest fake passes; the live pair passes (read-only) |
| **B7-L-1** | Low | **Fixed** | Two changes. (1) **Both providers:** the scan's log reads go through `nftnet::ask_scan`. When one provider refuses as too deep, the other is asked too. The read is `deep` only if every provider says so (`pair_deep`). One provider's "too deep" beside another's answer is a refusal with no class: read again, deferred only if it persists. (2) **Never given up:** a hole's too-deep refusal no longer drops the hole. It is recorded (`SaleScan.hole_deep`: first answer, count, not before), the hole is kept, and it is read again: next pass for the first 3, then every 6 h until it is read or leaves the window. It is paged (`eth-scan-deep` SEV2; `eth-scan-hole-day` SEV2 for a hole unread a day). Hole spans no longer go into `lost`, so the walk back's trim cannot erase them (B7-I-2) | `nftnet.rs` `ask_scan`, `pair_deep`; `ethnet.rs` `logs_classes`; `ethflow.rs` the hole walk, `SaleScan.hole_deep`; `ethnft.rs` `SALE_DEEP_*`; `monitor/src/ethnft.mjs` | `r7_b7_l1_a_…` (one deep answer: 119 of 119 gap sales, was 88; four answers: still all; refused for good: the hole kept and backed off, never lost), `r7_b7_l1_b_…` (`pair_deep`: one provider deep + one served → no class; both → deep; deep + busy → busy); monitor `audit_nft_d_r7` |
| **B7-L-2** | Low | **Fixed** | Busy receipts: a receipt answered as a rate limit is recorded in `SaleScan.rbusy` (count, not before). It backs off exponentially, 600 s doubling to a 6 h cap, and counts against the retries' share (4 of a pass's 16 reads). Newer sales are processed around it and never wait behind it. Passes: `SaleScan.busy` counts passes in a row that met a rate limit anywhere. Pages: a receipt busy 6 times in a row (`eth-scan-busy-receipts`) and 9 busy passes in a row, about 3 h (`eth-scan-busy`), both SEV2. The walks still read again each pass (the lag page is their watch) | `ethflow.rs` the receipts loop, `busy_again`, `ScanPass.busy`, `SaleScan.rbusy` / `busy`; `ethnft.rs` `SALE_BUSY_*`; `monitor/src/ethnft.mjs` | `r7_b7_l2_a_…` (16 old receipts busy: 144 of 144 newer sales, was 0; the 16 kept, backed off past the page, 12–13 reads each in two days instead of one a pass; read and counted once served); `r7_b7_l3_…` (the pass counter over 200 in three days) |
| **F7-L-1** | Low | **Fixed** | The search starts where the nonce is provably unused nearest the head. That is the lowest provider's rounded head at signing, `HomeTx.pin + BUCKET`, 0–1,023 blocks under the head then. Every provider's head was at or past it before the broadcast, and nothing else of ours signs while it is out. Before, the search started at the fast pin itself, 1,024–2,047 blocks under the head. The start is a pure function (`ethflow::home_search_starts`). The reach is measured from the provider's own head: about 520 s after signing at worst in sync, about 415 s with the other provider a whole bucket behind (`home_search_reach_secs`). The runner's pending cadence is now a PERIOD from the start of the cycle, default 120 s, 60 to 300, plain digits only (D7-I-3). A transaction seen nowhere for 15 min returns the runner to its normal cadence (F7-I-2). The runbook says keep 120 | `canister.rs` `home_nonce_tx`; `ethflow.rs` `home_search_starts`, `home_search_reach_secs`, `HOME_PROVIDER_STATE_BLOCKS`; `ops/src/runner.ts` `sleepAfter`, `pendingWantsHurry`, `parseRunnerConfig`; `ops/src/ic.ts`; runbook §5.1 | `r7_f7_l1_a_…`: the start arithmetic at the worst rounding; the flow on a fake whose pin is the FAST pin under a moving head and whose state depth is measured from that head (the canister's own start function), 6,198 blocks: identified at 300, 420 and 500 s (420 s was UNIDENTIFIED in the PoC), and at 700 s it goes to the operator. ops `audit_nft_r7` (the period, the bounds, the stale fallback) |
| **C7-L-1** | Low | **Fixed, pending C5-I-1** | Three parts. (1) **The event as the contract encodes it:** `call_executed` reads only the one-topic encoding, with the id in the data. The two-topic shape the round-6 tests used is refused. (2) **Bound to the executed request:** a call to the depository is credited only when its input decodes as `execute(CallRequest, signature)` (`decode_execute`, bounds-checked). Only the depository's records whose id equals the recomputed EIP-712 struct hash of that request (`call_request_hash`) count, and only those paying us. This was verified on the real transaction `0x86eee61f…d84d`: the recomputed hash equals its log's id. (3) **Named by the operator:** a real `execute` carries nothing of the order, so the operator names it from Relay's records (`/requests/v3`) by stating the amount. The amount must be exactly one of that request's records paying us. The old bounds still hold: after the deposit, and between half and all of the order's value. Several records paying us need a statement (C7-I-1). **The shape of a real Relay refund is still the operator's C5-I-1 measurement:** see §6 | `relayconv.rs` `call_executed`, `EXECUTE`, `decode_execute`, `encode_execute`, `call_request_hash`; `ethflow.rs` `judge_payout`; runbook §5.1 | `r7_c7_l1_a_…` (the real input decodes, its struct hash is its log's id, the encoder is the decoder's inverse on the real bytes, the indexed shape is no record), `r7_c7_l1_b_…` (the PoC's real-shape refund to us: credited at 4.9 ETH with the amount stated, was refused; unstated, another request's record, a forged emitter, another payee, the indexed shape, the stranger's gas-only call, before the deposit, under half: refused), `r7_c7_l1_c_…` (the flow credits it and the hold ends), `r7_c7_i1_…` (two records: one credited, never the sum); `audit_nft_r4`/`r5`/`r6`'s judge tests on the real encoding |

**Also fixed** (§7): A7-I-1 (the runbook's `--proposal` commands carry `--phase1` and the baseline), A7-I-2, A7-I-3, B7-I-1,
B7-I-2, B7-I-4, C7-I-1, C7-I-2 (runbook), D7-I-1 (= B7-L-3), D7-I-2, D7-I-3, D7-I-4, E7-I-1 (partly), E7-I-2, F7-I-1, F7-I-2.

## 1. B7-L-3: a scan-lag page that fires

**What was wrong:** `behind` was `top − up` at a pass's end, and the pass-start re-anchor keeps `up` within a day of the head whether
or not anything was read. So the ≥ 7,200 page held only by chance. A scan that stopped never rewrote `behind` at all.

**The canister:**

- `SaleScan.current` is the final head (`top`) of the last pass whose forward walk ended at it (`up >= top`).
  - A state saved before this round starts it at `up`.
  - A fresh scan starts it at its first pass's head, so a scan whose very first read fails still shows its lag.
- `behind` = `top − current` at each pass's end. The note says so from 2,400 blocks on.
- A re-anchor moves `up` but never `current`: only a read that reaches the head does.

**The monitor:**

- `checkEthNft` reads the Ethereum head (the lower of its two providers) before the canister's state.
- `ethNftAlerts` computes the lag as `head − 128 − current`, or the canister's `behind` if that is larger or there is no head.
- `eth-scan-behind` is SEV2 from 2,400 final blocks (8 h) and SEV1 from 7,200 (a day), persistent.
- A canister before this round (no `current`) is paged on its own `behind`.

**Tests:** `r7_b7_l3_…`, `r7_b7_l3_b_…`, monitor `audit_nft_d_r7` (the stopped scan). Review 7's `r7_b_l2_…` now fails its assertion:
`behind` reaches 21,598 and 144 passes page (`fixes-round7-poc/B.review7-pocs-on-round7.output.txt`).

## 2. D7-L-1: depth proved by a non-empty answer

**Pinning the range** (read-only, `fixes-round7-poc/P/pinned_range.py`, `.output.txt`, 2026-09-28):

- The 500-block ranges about 63,500 blocks under the head were counted on both providers. The first with logs on both, and the same
  count, was picked: Milady `Transfer` over 26,012,410..26,012,909, **10 logs** on each.
- The receipt of its first log's transaction, `0x17133e8b…c093` (block 26,012,500, status 1, one Milady `Transfer`), was read on both.

**`depth_probe` now does this:**

- reads the head (the pinned range must be at least 60,000 blocks under it);
- reads the pinned range and requires exactly 10 logs, so an empty or short list FAILs and names why;
- reads the pinned receipt and requires it not null, in its block, successful, and carrying its `Transfer`;
- past 400,000 blocks deep, still passes but says to pin a newer range (the window needs about 50,400).

A passing probe prints what it read. **The runbook's IC rehearsal** reads the same range and receipt through the canister's
transforms, and an empty or short list fails it.

**Test:** `fixes-round7-poc/D/probe_depth_pinned.py` runs the probe extracted from `INSTALL_PY` against local fakes and, with `--live`,
against the pair:

| Case | Result |
| --- | --- |
| the reviewer's shallow provider | FAIL (was `ok`) |
| 9 logs | FAIL |
| null receipt | FAIL |
| an honest fake | ok |
| MEV Blocker and Tenderly, live | ok |

**Not done:** comparing the two providers' counts over a range relative to the head. The fixed range makes the two answers the same
question with a known answer.

## 3. B7-L-1: one provider never gives a span up

- **Both providers.** `EthNet::logs_classes` (every `eth_getLogs` of the scan and of the marketplace reads) uses `nftnet::ask_scan`.
  - A provider's refusal whose class is `deep` does not end the read: the next provider is asked too.
  - Any other refusal ends it at once, as before.
  - `pair_deep` then decides:
    - every answer served: the answers;
    - any non-deep refusal: that refusal;
    - every provider deep: `deep`;
    - some deep, others served: a refusal with no class, whose words contain none of `error_class`'s. The walk treats it as `Other`:
      read again, the chunk halved, deferred only when it persists (`SALE_SCAN_STUCK_FAILS`).
  - A provider that says "pruned" alone can no longer steer the walk, which restores audit-R3's rule for the scan.
  - The only cost is one more outcall, and only when a provider says "too deep".
- **Retry, never give up.** The hole walk's `Deep` branch:
  - records `hole_deep = (first, n, not before)`;
  - keeps the hole and ends the walk for the pass;
  - is due again next pass for the first `SALE_DEEP_FAST_TRIES` (3), then every `SALE_DEEP_RETRY_SECS` (6 h).

  A successful read clears it, and the hole leaving the window clears it. The hole's span never goes into `lost`. `lost` is now only
  the walk back's, below the window's read part, which that walk re-reads every pass. So B7-I-2 (the walk back's trim erasing a hole's
  record) cannot happen.
- **Pages:**
  - `eth-scan-deep` (SEV2, persistent): `hole_deep` with at least 3 answers;
  - `eth-scan-hole-day` (SEV2, persistent): a hole the monitor has seen for a day. The lag page does not cover it, because the
    forward walk is current.
- **The attack the reviewer described** (stall a range near the head, then one `deep` answer) needs both providers now, and even then
  gives nothing up.
- **Tests:** `r7_b7_l1_a_…`, `r7_b7_l1_b_…`. Review 7's `r7_b_l1_a_…` fails its assertion (119 of 119, `lost` None).

## 4. B7-L-2: a bound for `busy`

- **Receipts.** In the receipts loop:
  - A busy answer adds or bumps `rbusy (hash, n, busy_again(now, n))`: 600 s × 2^(n−1), at most 6 h.
  - A receipt in back-off is kept and not read.
  - A due one is a retry, like an `rfail` retry, so retries take at most `SALE_RECEIPT_RETRY_READS` (4) of the pass's 16 reads.
  - Any other outcome clears it.
  - `rbusy` is kept only for queued moves, so it is bounded by `SALE_TODO_MAX`.

  An honest daily quota and a provider refusing on purpose look the same, and both now cost only the receipts they name.
- **Passes.** `ScanPass.busy` is set wherever a read of the pass answered busy: the walks, the refinement, a skipped block, a
  receipt. `SaleScan.busy = (since, n)` counts passes in a row that did so and clears on a pass that did not.
- **Pages** (SEV2, persistent): `eth-scan-busy-receipts` (a receipt busy `SALE_BUSY_PAGE_TRIES` = 6 times in a row, about 5 h) and
  `eth-scan-busy` (`SALE_BUSY_PAGE_PASSES` = 9 passes, about 3 h). The note says both.
- **Not done:** backing off the log walks themselves. They read again each pass, one read per walk before breaking. The lag page is
  their watch, and a rate limit that lifts is recovered from at the next pass.
- **B7-I-1:** the IC's divergence reject ("No consensus could be reached. Replicas had different responses.") is now `Busy`, not
  `TooBig`. Measured again: 432 of 432 due sales at 30 % and 50 % divergence (`r7_b7_i1_…`).
- **B7-I-4:** "beyond current head" (a lagging Tenderly backend) is `busy`, so the range is read again rather than deferred.
- **Tests:** `r7_b7_l2_a_…`, `r7_b7_i1_…`. Review 7's `r7_b_l1_b_…` fails its assertion (144 of 144).

## 5. F7-L-1: the search starts at the head bucket

- **The fact used.** `HomeTx.pin` is the fast pin, `min(rounded heads) − BUCKET`. So `HomeTx.pin + BUCKET` is the lowest provider's
  rounded head at that tick.
  - Every provider's head was at or past it before the transaction was broadcast (after the pin read), so the transaction mined
    above it.
  - Nothing else of ours signs while a home transaction is out.
  - So the nonce is unused there. A lying provider cannot raise it: it is the minimum, and the heads must agree within a bucket.
- **`home_search_starts(signed_pin, pin, bucket)`** returns, highest first:
  - `pin − HOME_STATE_REACH_BLOCKS` if it is above the next start (a transaction mined late);
  - `signed_pin + bucket`;
  - `signed_pin` itself (round 6's start, as a last resort);
  - with no `signed_pin`, `pin − HOME_NONCE_SEARCH_BLOCKS`.

  `canister::home_nonce_tx` tries each in turn. It takes the first whose nonce count is unused. A used count or a refused read moves to
  the next, and a refusal of all of them is the error, as before. The bisection and identification are unchanged.
- **The margin, measured from the provider's own head** (`home_search_reach_secs`; 6,198 blocks of state; 10 blocks a second taken,
  the conservative side of 9.95):

  | Case | Reach from signing |
  | --- | --- |
  | best rounding, in sync | 619 s |
  | worst rounding, in sync | 517 s |
  | worst rounding, the other provider a whole bucket behind | 415 s |

  Round 6 had 417–519 s at best.
- **The runner.** While a home transaction is out, the next tick is due `RUNNER_PENDING_EVERY_S` after the START of the cycle that
  signed it, at least 10 s: a slow cycle no longer adds its length.
  - The default is 120 s, bounded 60 to 300, plain digits only (`0x3c` and `6e1` refused: D7-I-3).
  - At 120 s plus a tick's outcalls (30–60 s), the first resolving read is about 180 s after signing: a margin of at least 235 s even
    with a lagging provider.
  - Once `used_since` is 15 min old, the transaction is past any reach and the runner returns to `RUNNER_TICK_EVERY_S` (F7-I-2).
  - The runner's IDL reads `pending.used_since`.
- **The runbook** (§5.1) states the reach, the 120 s, and that the rehearsal measures the cycle length.
- **Tests:** `r7_f7_l1_a_…` (the arithmetic, and the flow on a fast-pin fake through the canister's own start function), ops
  `audit_nft_r7`.
- **About review 7's `r7_f_reach_…`:** it still prints its old table on this code. Its fake's pin is the head and its `home_nonce_tx`
  ignores the start the canister chooses (F7-I-1). The new fake (`fast_pin`, `mined_head`) runs the real start function.

## 6. C7-L-1: a real depository payout

**The event.** `RelayCallExecuted(bytes32 id, (address to, bytes data, uint256 value, bool allowFailure) call)`, topic0
`0x4be10945…c5ea`, `id` NOT indexed. `call_executed` accepts only one topic, with `[id, 0x40, to, 0x80, value, allowFailure, len,
data]` in the data. The round-6 tests' `relay_exec` put the order id in a second topic, a shape the contract never emits. It is fixed
in `audit_nft_r6.rs` and replaced in r4/r5/r6/r7 by builders of the real encoding.

**The request.**
- `decode_execute` decodes `execute(((address,bytes,uint256,bool)[],uint256,uint256),bytes)`, selector `0x2d9fb478`:
  - every offset and length is checked;
  - at most 64 calls;
  - a value over 128 bits or a malformed flag is `None`.
- `call_request_hash` is the depository's EIP-712 struct hash:
  `keccak(CALL_REQUEST_TYPEHASH, keccak(concat(keccak(CALL_TYPEHASH, to, keccak(data), value, allowFailure))), nonce, expiration)`.
- On the real transaction `0x86eee61f…d84d` (Ethereum block 0x18dd4e1, review7/poc/C), the recomputed hash equals its log's id
  `0x84334077…6659`.
- `encode_execute` is the decoder's inverse, and reproduces the real input byte for byte.

**The judge** (`judge_payout`, rule 4):
- A call to the depository (not `depositNative`) must decode as `execute`.
- The records counted are the depository's `RelayCallExecuted` whose id equals that request's recomputed hash and which pay our
  address a value above 0.
- One such record with no statement is credited at its amount, but only if the input names the order.
- With a statement, it must be exactly one of those records (C7-I-1: never the sum of several).
- Rules 1, 3 and 5 are unchanged: success, not ours, after the deposit, between half and all of the order's value.

**Naming.** A real `execute` input carries the calls, the allocator's random nonce, the expiry and the signature, and nothing of the
order. So:
- the order id or request id in the input still names it (as before);
- otherwise a depository `execute` is named by the operator: the hand-in (`eth_credit_payout`, operator-only) with a stated amount
  equal to one of that request's records paying us.

**The operator's side** (runbook §5.1):
- Take the transaction and the amount from Relay's own records for the order (`/requests/v3`; v2 is throttled and retires
  2026-11-24).
- **Pending C5-I-1** (unchanged as a precondition): record whether a real refund is a solver transfer, which is creditable when its
  input names the order, or a depository `execute`, creditable this way.
- If it is a withdrawal the depositor must claim itself (an `execute` sent by the payee, as the one non-solver native payment sampled
  was), it can never reach the canister, and the runbook says so.

**Tests:** `r7_c7_l1_a_…`, `r7_c7_l1_b_…`, `r7_c7_l1_c_…`, `r7_c7_i1_…`, and the updated `r4_c4_m1_…`, `r5_c5_l1_b_…`,
`r6_c6_l1_a/b_…`. Review 7's `r7_c_l1_a_…` fails at its assertion: with the amount stated, the real-shape refund is `Ok(4.9 ETH)`.

## 7. The Lows and Infos

| ID | Status | What |
| --- | --- | --- |
| B7-L-1 | Fixed | §3 |
| B7-L-2 | Fixed | §4 (the walks not backed off: the lag page watches them) |
| B7-L-3 | Fixed | §1 |
| C7-L-1 | Fixed, pending C5-I-1 | §6: the matching rule is the safest the chain allows. Whether Relay's refunds take this path is the operator's live measurement |
| D7-L-1 | Fixed | §2 |
| F7-L-1 | Fixed | §5 |
| Carried: A5-L-3, E5-L-2, C4-L-1, C3-L-1, E3-L-2, B3-L-1, B-L-6 (partly), A-L-2, B-L-5, C-L-5, D-L-3, D-L-6 | Unchanged | As `fixes-round6.md` §7 |

**Infos:**

- **Fixed:**
  - **A7-I-1:** the runbook's step-2 and step-3 commands are `golive-check --phase1 --proposal … --baseline <running argument>`.
  - **A7-I-2:** `nft-pilot.json` `_custody` says 48 items at step 1, 61 after step 3.
  - **A7-I-3:** a listed Ethereum provider counts only as `https://` on the default port. `http://…` and `:8443` now FAIL under
    `--phase1` (`fixes-round7-poc/A.allowlist-on-round7.output.txt`).
  - **B7-I-1** and **B7-I-4:** §4.
  - **B7-I-2:** §3.
  - **C7-I-1:** §6.
  - **C7-I-2:** the runbook's credit line says to hand a claim in again if it waits and goes stale.
  - **D7-I-1:** the same as B7-L-3.
  - **D7-I-2** / **E7-I-2:** a home transaction without `sent_at` is aged from when the monitor first saw it out.
  - **D7-I-3:** §5.
  - **D7-I-4:** the Trust page's sentence now says what pages (a reading that falls behind, a part both providers refuse, hours of
    rate limits) and that each provider is checked at go-live on a past range known to hold sales. The late-payout sentence names the
    depository path.
  - **F7-I-1:** the start is a pure function under test.
  - **F7-I-2:** §5.
- **Partly: E7-I-1.** `r7_e7_i1_…` refuses `#[path` and `include!(` anywhere in `src/`, covering the guard's (D) and (E).
  `include_str!` of test fixtures stays: it is data. (A)–(C) (a decoy `let` in a closed block, a typed `let`, `key_of`'s body) are
  not guarded. They need a deliberately misleading edit or a change to a reviewed body, and none is present.
- **Unchanged:**
  - **B7-I-3** (merged holes re-read: cycles only);
  - **B7-I-6** (`skipped` has no count cap; each entry costs the attacker an oversized block);
  - **C7-I-3**, **C7-I-4** (accepted in review 7).

## 8. New judgment calls for the operator

1. **The scan's lag pages at 8 h (SEV2) and a day (SEV1)**, from the monitor's own Ethereum head. A healthy scan is at most a pass and
   a chunk behind. A runner outage or a day's scan budget spent also pages it, and should.
2. **One provider's "too deep" is no class** (read again, deferred if it persists). Only every provider's is `deep`, and even then the
   span is read again every 6 h for as long as it is in the window. A truly shallow pair therefore pages for days, never silently.
3. **Busy receipts back off to 6 h and page at about 5 h.** A rate-limited receipt's sale can be counted up to 6 h late once the limit
   lifts.
4. **A depository payout is named by the operator's stated amount** (C7-L-1): the transaction and amount come from Relay's records
   for the order. A wrong choice can only move ETH that really arrived from Relay's depository between our own orders, within their
   values. Pending C5-I-1.
5. **The runner's pending period is 120 s from the cycle's start** (was a 240 s sleep after it): more ticks while a 4663 transaction
   is out, for identification inside the provider's state.
6. **`--probe-depth` reads a range pinned today.** It gets deeper with time. Past 400,000 blocks the probe says to pin a newer one,
   measured the same way.

## 9. As built (2026-09-28)

**Commits** (branch `nft-side`, on `effc270`):

| Commit | What |
| --- | --- |
| `48d62fa` | Canister: B7-L-3, B7-L-1, B7-L-2, B7-I-1, B7-I-2, B7-I-4, F7-L-1, F7-I-1, C7-L-1, C7-I-1, E7-I-1 (partly); the `.did`; `eth-state-round7.hex`, `state/eth-state-a78a542.hex`, the round-2/4/5/6 fixtures re-encoded; `tests/audit_nft_r7.rs`; the r4/r5/r6 judge tests on the real encoding; struct literals in r2–r6 |
| `533c8a9` | Monitor and runner: the scan pages (B7-L-3, B7-L-1, B7-L-2), D7-I-2, the pending period (F7-L-1, D7-I-3, F7-I-2), the IDLs; `audit_nft_d_r7`, ops `audit_nft_r7`; the r6 tests that encoded the replaced defaults |
| `428c32a` | Go-live: `--probe-depth` on the pinned range (D7-L-1), https / default port for the list (A7-I-3), the launch file's `_custody` text (A7-I-2) |
| `3300c96` | Site: the Trust page (D7-I-4, C7-L-1) |
| (this document's commit) | This document; `fixes-round7-poc/`; `docs/v7/phase1-nft.md` §5.1 (A7-I-1, D7-L-1, F7-L-1, C7-L-1, C7-I-2, the pages) |

**Results** (2026-09-28, on `3300c96`'s tree):

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites; unchanged: no contract changed) |
| The Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **713 passed**, 0 failed (was 699: +14 `tests/audit_nft_r7.rs`; the older judge tests rewritten on the real encoding) |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed (unchanged) |
| ops | **63 passed** (+3 `audit_nft_r7`); typecheck clean |
| monitor | **219 passed** (+5 `audit_nft_d_r7`) |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | 72 passed; typecheck clean |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| The `.did` | `service_compatible` with `a78a542`'s; every field of the round-2, -4, -5, -6 and -7 fixtures the same through the `.did` and the Rust encoder (`fixes-round7-poc/didcheck.output.txt`) |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS**: 146 checks ok on the v3–v6 stack; 536 on the five-collection NFT stack; **664** on the launch go/no-go read of the COMMITTED `nft-pilot.json` (step 1, the install argument on MEV Blocker + Tenderly); 52 refusals `ok` |
| `--probe-depth` on the pair (read-only) | both pass the pinned range (10 logs) and receipt (`fixes-round7-poc/D/probe_depth_pinned.output.txt`) |

**Reproducible build** (`autobuyer/Dockerfile.build`, linux/amd64): two `git archive` exports of `3300c96` (no later commit of this
round touches `autobuyer/`; its `autobuyer/` is `48d62fa`'s), each built with `docker build --no-cache`, one image at a time. Both builds
gave the same hashes (the extracted files compared byte for byte):

```
b7532c4dbdeb3704bc3c8932a7dc15c86461b856e3d6f12acb2942de357628ec  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

- **`buyer.wasm` is new:** 7,106,636 bytes, 1,351,080 gzipped with `gzip -9 -n` (under the 2,000,000 proposal limit). It replaces
  `00d2d635…d3f1` (`fixes-round6.md`) for any install of this branch.
- **The timelock is unchanged.**
- **Nothing was installed.**
- **Removed afterwards:** the exports, the two build images and the extracted wasm; the private PoC copies under `/tmp/rook-fix7`; the
  fork anvil and the dry run's anvils. Docker's shared build cache was left alone.

**The delta a confirmation review must cover:** `git diff effc270..HEAD`:

- `autobuyer/canister-rs/`:
  - `etch_autobuyer.did`;
  - `src/{canister,ethflow,ethnet,ethnft,nftnet,pin,relayconv}.rs`;
  - `tests/{audit_nft_r2,audit_nft_r3,audit_nft_r4,audit_nft_r5,audit_nft_r6,audit_nft_r7}.rs`;
  - `tests/fixtures/{eth-state-round2.hex, eth-state-round4.hex, eth-state-round5.hex, eth-state-round6.hex, eth-state-round7.hex, state/eth-state-a78a542.hex}`.
- `monitor/`: `src/{candid,ethnft}.mjs`, `test/{audit_nft_d_r6,audit_nft_d_r7}.test.mjs`.
- `ops/`: `src/{ic,runner}.ts`, `test/{audit_nft_r6,audit_nft_r7,runner}.test.ts`.
- `script/golive-check.sh`, `deployments/launch/nft-pilot.json`.
- `web-rook/src/rook/phase1Trust.ts`.
- `docs/v7/phase1-nft.md`, this document, `fixes-round7-poc/`.
