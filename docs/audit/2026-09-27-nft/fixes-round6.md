# Fix round 6: the 2026-09-27 review of the Ethereum NFT side, round 6

Written 2026-09-28 on branch `nft-side` (at `bb713fe`: the round-6 re-review's reports and PoCs on top of fix round 5). It answers
`review6/README.md` and the six part reports (`R6-launch-plan.md`, `R6-canister-scan.md`, `R6-canister-wayback.md`, `R6-services.md`,
`R6-canister-upgrade.md`, `R6-canister-home.md`) finding by finding: the fix, where it is, and the test that is the reviewer's PoC
turned around (it fails on the code before this round and passes now). `fixes-round6-poc/` has the reviewers' PoCs re-run on this
code and this round's provider probes (`P/`). §11 ("As built") has the commits, the test counts and the reproducible-build hashes.

Nothing was deployed, pushed or signed with a real key. The only chains written to were local anvils (the dry run's fork of 4663, the
Seaport fork proof's fork of Ethereum, both read-only upstream). Network use was read-only JSON-RPC probes of public Ethereum and 4663
providers (§1.1). The one exception to "read-only" is two `eth_sendRawTransaction` calls per provider carrying bytes that can never be
included: a one-byte garbage payload, and the raw bytes of a stranger's transaction already mined (its nonce used). They measured the
reply shapes and could not change any state. The reproducible build fetched its crates.

**The rules every fix keeps** (as rounds 1–5):

- Nothing is weakened to buy availability: H1 (pause), H2 (custody caps), H3 (the outflow limiter and its tripwire), "save, then
  send", and every judge (`relayeth`, `seaport`, `ethnft`) refuse what they refused. The payout judge refuses more (C6-L-1). The
  custody book counts more (C6-L-3). The first floor post still needs the collection's own counted sales.
- Stable state stays decodable. Every new field is `opt` (`Option` + `#[serde(default)]`), and no enum case was added to a saved
  type. The state the round-5 build wrote restores with nothing lost: `tests/audit_nft_r6.rs` `r6_the_state_the_round_5_build_wrote_…`
  runs on the bytes e04fa16's own fixture test held equal to its encoder (`tests/fixtures/state/eth-state-e04fa16.hex`). The round-2,
  -4 and -5 fixtures were re-encoded only because their tests compare bytes and the type table grew. The `.did` was checked against
  the Rust encoder on the round-2, -4, -5 and -6 fixtures (no difference either way) and is `service_compatible` with e04fa16's.
- **Not changed** (operator decisions): BAYC 10 ETH / `convBps` 1,000 / `reserveBps` 0; option (c)'s staging (48 at 12 / 24, then
  `global_items` 31, then J7's 61); H3's 25 ETH a day and trip ×3; D1–D9, D5 deposits on; D4's automatic return after 48 h; Dwellir
  first for the monitor and the runner; keyless canister providers.
- No contract changed (`src/` is untouched).

## 0. Summary

| ID | Sev | Status | Fix | Where | Test (the PoC, turned around) |
| --- | --- | --- | --- | --- | --- |
| **B6-M-1** | Med (launch blocker) | **Fixed** | (1) **The pair:** the canister's Ethereum providers are `https://rpc.mevblocker.io` + `https://gateway.tenderly.co/public/mainnet`, re-probed read-only (§1.1). (2) **Classification:** the transforms keep a CLASS of the provider's words (`busy`, `deep`, `limit`) beside its code, so `Deep` and the size refusals classify in production. (3) **The walk:** a forward walk a day behind the final head, or refused as too deep, re-anchors at the newest final blocks. What it passes over (`SaleScan.hole`) is read newest first by a walk of its own. What the providers refuse as too deep is recorded (`SaleScan.lost`), and the lag after each pass is kept (`SaleScan.behind`). The monitor pages all three. (4) **golive-check:** under `--phase1` it refuses a chain-1 canister provider that is not on its measured list, and `--probe-depth` measures one now (logs and a receipt 60,000 blocks back), a failure refusing even a listed one. (5) **The runbook:** the operator's pre-install rehearsal check (both providers through an IC outcall and the transforms, Tenderly's IPv4-only host) | `pin.rs` `error_class`, `busy_body`, `json_rpc_norm_body`; `ethnet.rs` the log/receipt transforms, `rate_limited`; `ethflow.rs` `scan_error`, `ScanErr::Busy`, `reanchor`, `sale_scan` (the hole walk; Deep / Busy in every walk), `SaleScan.hole/lost/behind`; `ethnft.rs` `SALE_REANCHOR_LAG_BLOCKS`; `script/golive-check.sh` `DEEP`, `depth_probe`, `--probe-depth`; `scripts/golive-dryrun.sh`; `monitor/src/ethnft.mjs`; `docs/v7/phase1-nft.md` §4.1, §5.1 | `audit_nft_r6.rs` `r6_b6_m1_a_…` (12,000-block gap on a 10,000-block provider: the first newer sale at pass 1, was 385–395; a day's read; the walk at the head; the loss recorded, for words the canister knows and for words it does not), `r6_b6_m1_b_…` (the measured shapes through the real transforms); `audit_nft_r5.rs` `r5_b5_m2_every_too_big_answer_…` rewritten through the transforms (B6-I-1); the dry run refuses the round-5 pair under `--phase1`; `monitor/test/audit_nft_d_r6` (the pages) |

**No Medium remains open by this round's own reading.** Also fixed (§7): all 11 new Lows (A6-L-1, B6-L-1, B6-L-2, C6-L-1, C6-L-2,
C6-L-3, D6-L-1, D6-L-2, D6-L-3, F6-L-1, F6-L-2) and the carried B5-L-2. The E5-I-2 residual (E6-I-1) and several Infos are fixed too
(§8).

## 1. B6-M-1: the canister's Ethereum pair, and a scan that survives any pair

### 1.1 The providers, re-probed (2026-09-28, 11:32–11:37 UTC, read-only; `fixes-round6-poc/P/`)

| Probe | `rpc.mevblocker.io` | `gateway.tenderly.co/public/mainnet` |
| --- | --- | --- |
| `eth_getLogs`, 500-block Milady `Transfer` at 1,000 / 10,000 / 20,000 / 40,000 / 52,000 / 60,000 / 70,000 / 100,000 / 130,000 blocks deep | all served (0–5 logs) | all served, the same logs |
| a 10,000-block Milady range 60,000 blocks deep | 44 logs, 28 kB | 44 logs, 28 kB |
| `eth_getTransactionReceipt` (and the transaction by hash) at 1,000 / 30,000 / 52,000 / 60,000 / 70,000 blocks | all served, not null | all served, not null |
| the result cap (USDT `Transfer`, 1,000 dense blocks) | `-32005 "query returned more than 10000 results. Try with this block range […]"` | `-32602 "invalid params"`, `data: "Query returned more than 20000 results. Try with this block range […]"` |
| Seaport `OrderFulfilled` (every collection's), 250 / 500 blocks | 0.82 MB / 1.71 MB | the same |
| 13 simultaneous `eth_getLogs` (one IP), once | 13/13 | 11/13 (2 × HTTP 429 `-32005 "rate limit exceeded"`) |
| 13 simultaneous `eth_getLogs`, 20 rounds 1 s apart (one IP) | 156 × 200, 104 × 429 (Cloudflare `error code: 1015`, not JSON); ≥ 9 of 13 in 12 rounds | 248 × 200, 12 × 429; ≥ 9 of 13 in 20 rounds |
| 13 simultaneous receipts 52,000 blocks deep | 13/13, one body | 13/13, one body |
| `eth_sendRawTransaction`: garbage `0x00` | `-32000 "… typed transaction has unknown type: 0"` | `-32602 "typed transaction too short"` |
| `eth_sendRawTransaction`: an already-mined stranger's transaction | `-32000 "Transaction 0x… already known"` | `3 "nonce too low: next nonce 1, tx nonce 0"` |
| DNS | A and AAAA | **A only** (35.227.193.242) |

**What it means for the canister:**

- **Depth.** Both serve logs and non-null receipts past the 7-day window with more than twice its margin.
- **Result caps.** Each cap now classifies as `limit` → `TooBig`: the range is deferred and read through the marketplaces' logs.
  This applies to MEV Blocker's 10,000 (under `-32005`) and to Tenderly's 20,000 (its words are in `data`, which the class reads).
- **Rate limits.** A 429 (JSON `-32005 "rate limit exceeded"`, or MEV Blocker's non-JSON Cloudflare page) classifies as `busy`. It
  is retried next pass, never taken as too big.
- **Bursts.** The one-IP bursts are a pessimistic model: an IC outcall's 13 replicas call from 13 hosts. An HTTPS outcall's result
  is agreed by a supermajority of replicas returning the same transformed body, so a few 429s among them need not fail it. A mostly
  refused round fails the read, which is retried (the `busy` class, §1.2). The rehearsal check measures it from the IC itself.
- **Sends.** A send's reply is a report the canister never acts on (`evmsend.rs`; `EthNet::eth_broadcast` ignores it) and the
  read-back decides. MEV Blocker answers "already known" even for a mined transaction, which `classify` would take as `AlreadyKnown`,
  harmless. It routes new transactions to its private builders, so ours become visible to others once mined, or through Tenderly's
  public broadcast (the canister sends to both).
- **What a read-only probe cannot show** is whether an IC outcall reaches Tenderly's IPv4-only host, and whether both providers reach
  consensus through the transforms from the IC. This is the operator's rehearsal check (`phase1-nft.md` §5.1, below).
- **Rejected as the canister's pair:**
  - `ethereum-rpc.publicnode.com`: logs about 10,000 blocks deep; receipts `null` 20–30 k back.
  - `xrpc.cl/eth`: an anonymous rate limit of 10, and 7–10 of 13 of a burst busy. It stays the monitor's second provider.
  - The rest of the reviewer's table.
- **The committed places that named the old pair:**
  - the dry run's install argument (`scripts/golive-dryrun.sh`, which feeds the monitor configuration's `install_arg_hex` too);
  - `phase1-nft.md` §4.1, D6 and §11.12.

  No other file templated it. The canister's `evm_chains` default names no Ethereum provider: the install argument does.

### 1.2 The scan, robust to a shallow provider regardless

- **Classification (B6-I-1).**
  - `pin::error_class(words)` reads a JSON-RPC error's `message` and a string `data`, lower-cased, against three word lists, in this
    order:
    - `busy`: rate limit, too many requests, capacity, busy, throttle, quota, Cloudflare's 1015;
    - `deep`: archive, pruned, missing trie node, historical state, unknown state, "personal token";
    - `limit`: "more than", "too many", "too large", "too wide", "exceeded", response size, block range, timeout, results.
  - `json_rpc_norm_body` (every state read's transform, and the scan's through `transfer_logs_body` / `sale_logs_body`) keeps it as
    `"class"` beside the code, only when the words name one and the call did not revert. The same words give the same class on every
    replica, so consensus is unchanged.
  - The scan's and the receipts' transforms give an HTTP 429 without a result the one `busy` body.
  - `scan_error` reads the class first:
    - `busy` → `ScanErr::Busy` (new);
    - `deep` → `Deep`;
    - `limit` → `TooBig`;
    - a bare code is `Other`: `-32005` is both a rate limit and a result cap, so it names nothing and is deferred only when it
      persists.
  - The IC's own size reject is `TooBig` as before.
- **Re-anchor (B6-M-1).**
  - When it happens:
    - at a pass's start, if the forward walk is more than `SALE_REANCHOR_LAG_BLOCKS` (7,200, a day) behind the final head;
    - or when a forward read is refused as `Deep` more than a chunk behind.
  - What `reanchor` does: moves `sc.up` to the newest final chunk and adds the blocks passed over to `SaleScan.hole`.
  - The hole walk reads the hole newest first each pass (after the forward walk, before the walk back), with the walks' own bounds.
  - A `Deep` refusal there records the hole's remainder in `SaleScan.lost` and ends it. A `Deep` refusal of the walk back records
    what is below it.
  - A later successful read trims `lost` (a provider back to depth).
  - `SaleScan.behind` is the lag after every pass.
  - The words matter only for speed. A refusal whose words name nothing (only its code) is caught by the lag rule a day later, and
    its first newer sale is still read at pass 1 in the PoC (the new walk reads the newest blocks first).
- **The pages** (`monitor/src/ethnft.mjs`, D6-I-1):
  - `eth-scan-behind`: SEV2, a day behind;
  - `eth-scan-lost`: SEV2, the providers refused part of the window: its sales are not counted; replace the shallow provider;
  - `eth-scan-hole` and `eth-scan-receipts`: SEV3, being read.
- **golive-check.** `INSTALL_PY`'s `DEEP` list holds each measured host, its paths and the date.
  - Under `--phase1`, a chain-1 provider off the list FAILs: "is not known to serve eth_getLogs and receipts over the 7-day window:
    probe it with --probe-depth or add it to the allowlist after measuring".
  - `--probe-depth` runs, per provider: the head; a 500-block Milady `Transfer` `eth_getLogs` 60,000 blocks back; that block's first
    transaction's receipt, which must not be null.
    - A failed probe FAILs even a listed provider.
    - A passing probe admits an unlisted one.
    - Only `scheme://host` is printed.
  - The host-count and keyless rules are unchanged; the new pair is two hosts and keyless.
  - The dry run builds its install argument with the new pair and shows the round-5 pair refused under `--phase1`.
- **The runbook** (`phase1-nft.md` §5.1): before the install argument is built, on a rehearsal canister on the IC (not a local
  replica), the operator reads a pinned block, a 500-block launch-collection `eth_getLogs` 60,000 blocks back and a receipt there
  through the canister's transforms on both providers. Both must reach consensus and agree, and the IC's outcalls must reach
  Tenderly's IPv4-only host. At go-live the operator runs `golive-check … --phase1 --probe-depth`.

## 2. B6-L-2 and B5-L-2: a rate limit is retried, and a missed block is never missed for good

- **Rate limits.** `ScanErr::Busy`:
  - In every walk, it ends that walk for the pass: nothing deferred, split or shrunk, and no count toward "stuck".
  - In the refinement it stops the pass.
  - For a block it is no try.
  - For a receipt it is no failure.
- **One-sided marketplace reads.** `EthIo::eth_sale_moves_whole` (`EthNet`: the two marketplace queries, and whether both
  answered).
  - A single block is remembered as read (`done_big`) only when both answered.
  - A one-sided answer counts what it read and keeps the block `skipped` for the other.
- **Skipped blocks** (B5-L-2) are never given up while in the window. Their retries back off to `SALE_SKIP_TRIES` ×
  `SALE_SKIP_RETRY_SECS` (18 h) and are tried again until they read or leave the window. They were given up after 3 tries, with their
  sales lost for good.
- **The deferred ranges' bound.** The refinement's split goes through `defer` (was pushed past the bound: 529 seen, B6-I-2).
  - A range whose marketplace logs were not read whole (a failure, a one-sided answer, the pass's budget) is marked `SaleScan.raw` and
    is evicted last.
  - A raw range goes only when every kept range is raw and there are twice the bound.
- **Tests:**
  - `r6_b6_l2_a_…` (a week at 0 / 30 / 50 % busy with Seaport's range half over the cap above 520 blocks): 432 of 432 due sales
    counted at every rate, 0 lost for good, `bad` at most 0: a rate limit no longer defers anything (was, at 50 %: 376 counted, 20 lost,
    529);
  - `r6_b5_l2_a_…`: a block unreadable both ways is still tried after `SALE_SKIP_TRIES` and is read and counted once its marketplace
    logs answer.
- **Not done:** reading a range's marketplace logs in ≤ 250-block sub-ranges. Each sub-read reserves a whole read's worst case and
  would break B5-L-1's pass bound. A range whose Seaport half is over the cap is `raw` instead, and its sales are read through the
  refinement's `Transfer` bisection.

## 3. B6-L-1: an unreadable receipt no longer holds the newer sales

`SaleScan.rfail` keeps `(transaction, failed reads, not before)`:

- **What counts as a failure:** an `Ok(None)` (no receipt for a final transfer) or a failed read. A rate limit, a size reject and a
  lagging provider do not count.
- **The retries:**
  - `SALE_RECEIPT_FAST_TRIES` (3) at once;
  - then only when due: `SALE_RECEIPT_RETRY_SECS` (6 h) times the tries past it, at most 24 h;
  - retries take at most `SALE_RECEIPT_RETRY_READS` (4) of a pass's 16 reads, so they can never hold a newer sale.
- **Nothing is lost:** a failed receipt stays queued until it reads or leaves the window (a null is never "no sale").
- **The record** is keyed by the whole hash, not the 8-byte prefix `judged` uses, so it can never hold back another transaction.
- **The note, and the monitor's SEV3,** say how many are unreadable.
- **Test:** `r6_b6_l1_…` (16 honest moves 30,000 blocks back that one provider nulls): 144 of the 144 newer final sales in
  two days (was 0), the 16 kept, and read once served.

## 4. The home line (F6-L-1, F6-L-2, D6-L-1, D6-L-2)

- **F6-L-2: providers that disagree on a receipt.**
  - With the nonce used, `EvmView::Unsure` is decided as "seen nowhere" is: identification, then the effect read-back, then (a Relay
    deposit or a `fundReserve`) the operator after the hour.
  - The disagreement no longer restarts the clock. A drop still needs the effect absent at a pin past the nonce on every read for the
    hour, and an effect is read on every provider, so one provider's word never drops a step.
  - With the nonce unused it waits, as before.
  - A provider showing it in its POOL still restarts the clock. `r5_f5_m1_d_…` now drives that with the fake's new `pool` switch.
  - Test: `r6_f6_l2_…`: identified MINED at the first tick although the providers disagree. Where it cannot be identified, the clock
    runs and `eth_home_clear` opens after the hour; the deposit is made once.
- **F6-L-1: 4663's state depth.** Probed read-only (`fixes-round6-poc/P/h4663.output.txt`):
  - `rpc.mainnet.chain.robinhood.com` keeps state 6,000 but not 10,000 blocks back;
  - `rpc.ordofi.network` is an archive (1,000,000 back);
  - `robinhood.drpc.org` and `robinhood-rpc.publicnode.com` serve no historical state;
  - every other endpoint tried (thirdweb, ankr, tenderly, onfinality, blockpi, 1rpc, blast, subquery, nodies) needs a key, does not
    serve chain 4663, or does not resolve.

  **No deeper keyless provider exists to replace Robinhood's own**, and identification must agree on every provider. The fix is the
  cadence and the search:
  - **The runner** (`ops/src/runner.ts` `sleepAfter`, `RUNNER_PENDING_EVERY_S`, default 240 s, from 60 up to `RUNNER_TICK_EVERY_S`)
    ticks sooner while `eth_nft_state().pending` is set. Its IDL reads `pending`. The next tick then comes about 300 s after the
    signing tick, inside the ≈ 620 s of state. It was 660 s, outside.
  - **The canister** (`canister::home_nonce_tx`): when the transaction's own pin is further back than `HOME_STATE_REACH_BLOCKS`
    (4,000) under the pin, it starts the search at that reach if the nonce is still unused there, which catches a transaction mined
    late (a resend). The error names the depth in minutes.
  - **The operator path** (runbook §5.1, the page): look the nonce up on the 4663 explorer, then `eth_home_clear`.
  - Tests: `r6_f6_l1_…` (at 240 + 60 s: MINED at the first tick; at the old 600 + 60 s: UNIDENTIFIED, and `eth_home_clear` applies);
    ops `audit_nft_r6` (`sleepAfter`, the IDL).
- **D6-L-1 / F6-I-2's first half.**
  - `ethflow::home_clear_applies`: `eth_home_clear` is accepted only for a Relay deposit or a sweep's `fundReserve`, the steps the
    canister ever applies it to. For any other step it is refused, with the reason: the step is decided by its own effect on chain.
  - The monitor decodes `HomeTx.step` and words the SEV1 per step: `eth_home_clear` only where it applies; otherwise "the canister
    decides by its effect; if it keeps waiting, look at the 4663 providers".
  - Tests: `r6_d6_l1_…`; monitor `audit_nft_d_r6` D6-L-1.
- **D6-L-2.**
  - `HomeTx.sent_at`: the time it was signed, `opt`.
  - `eth-home-out` pages a home transaction out 30 min (SEV2) and 2 h (SEV1), persistent, whatever `used_since` says (a stale pool
    entry, flapping providers).
  - Tests: `r6_d6_l2_…`; monitor `audit_nft_d_r6` D6-L-2 (a week on, still paged).

## 5. The payout judge and the credit (C6-L-1, C6-L-2, C6-L-3, D6-L-3)

- **C6-L-1: a depository call needs Relay's own record.** The depository's bytecode (read-only `eth_getCode`, the same code on both
  chains; `fixes-round6-poc/P/depository-events.txt`) has three events of its own:
  - `RelayNativeDeposit`;
  - `RelayErc20Deposit`;
  - `RelayCallExecuted(bytes32 id, (address to, bytes data, uint256 value, bool allowFailure) call)`, emitted only by its
    allocator-signed `execute`.

  The judge's depository path now requires, in the payout's receipt (`EthIo::payout_logs`: on Ethereum at its pin, on 4663 through
  `canister::home_receipt_logs`, every provider agreeing), a `RelayCallExecuted` from the depository paying OUR address. It credits
  that record's amount. An operator statement that differs is refused.

  Refused now:
  - a view function called with trailing bytes (it emits nothing);
  - a deposit (another event);
  - another contract's look-alike (another address).

  Also:
  - The order is named by the input or by the record's id.
  - A request id with twelve leading zero bytes (an address or a small number) names nothing.
  - A plain transfer to us naming the order stays a payout at its value: it is real ETH (§9, call 3).

  Tests: `r6_c6_l1_a_…` (the stranger's `allocator()` + order id call refused; the hold stands; nothing paid out of the other flows'
  ETH; Relay's own execution credited at its amount), `r6_c6_l1_b_…` (the ERC-20 call through our address word refused; a forged
  record, or one paying someone else, refused); `r4_c4_m1_…` and `r5_c5_l1_b_…` updated.
- **C6-I-1.** A deposit's 4663 pin is read with `?` at both deposit sites: a failed read waits a tick instead of leaving the judge's
  "mined after the deposit" rule off.
- **C6-L-2: no credit while the buy has a 4663 transaction out.**
  - `payout_pass` keeps the claim (first in line, its hand-in time unchanged) while `st.pending` is a step of that buy: advance,
    deposit, give back, mint, settle.
  - It is judged after that transaction resolves. A claim past its TTL is refused as stale and handed in again.
  - Test `r6_c6_l2_a_…`: the claim waits while the loss-closing `returnAdvance(0)` is out; after it resolves, the payout is paid to
    the module (as a `fundReserve` sweep), never recorded `Returned { paid }` with nothing paid. The reviewer's Ethereum branch (its
    `r6_c_l2_b_…`) fails the same way (`fixes-round6-poc/C…`).
- **C6-L-3: the item is in the custody book.**
  - `buy_pass`'s reservation sweep keeps `eth:{id}` while its written-off forward order is open (neither credited nor released).
  - A credit on Ethereum of an order released first takes the room again before `arrive`, and settles it back if `arrive` refuses.
  - With no room, the ETH is the collection's leftover (swept back to its module), never an item outside the book.
  - Tests: `r6_c6_l3_a_…` (the reservation kept, the item in the book, no "not in flight"); `r6_c6_l3_b_…` (released, then credited:
    reserved again, the item in the book).
- **D6-L-3 / D6-I-2.**
  - `eth-late-hold` pages the written-off order's hold on the Relay leg: SEV2, persistent; SEV1 72 h after the write-off. The text
    says every collection's new buys, ways back and sweeps wait, and how to end it (credit, or release only on Relay's records).
  - "Held" is `!credited && !released`, the canister's own `late_hold`.
  - Test: monitor `audit_nft_d_r6` D6-L-3.

## 6. A6-L-1: a staged step is exactly the step

`monitor/src/staged.mjs` (the monitor and `golive-check --proposal`):

- **What a staged step must be:**
  - every cap key that is not a launch collection's is unchanged (none new, none removed);
  - the global item and value caps equal the step's sums (J7: no total binds);
  - nothing (cap, global cap or limit but the trip factor) rises past ×4 at once (A6-I-4);
  - step 3 moves only the launch collections' caps.
- **What `argdiff.mjs` `proposalAlert` downgrades** to WARN are only the LOOSENS lines the step itself names:
  - step 2: `global_items`;
  - step 3: the five item counts and the global item line.

  Every other LOOSENS line keeps its severity, and a "more than ×4" line is never downgraded.
- **A6-I-2:** under `--phase1`, `--proposal` requires `--baseline` (or the monitor configuration's `install_arg_hex`), so every other
  Init field is compared.
- **Tests:**
  - `monitor/test/staged_r6.test.mjs` (the reviewer's PoC turned around: step 3 with a ×4 Bitcoin raise and the global caps above
    the sums pages SEV1 and fails `--proposal`; each extra alone too; a new key; a global of 62; the exact steps still EXPECTED);
  - the reviewer's CLI case (`golive-check --proposal evil3.hex --stage 3 --baseline s2.hex`) now FAILs ("global item cap 62 is not
    the caps' sum …", "other caps move too").
- **The launch file's stale `_outflow` text** (it said 61 items) now states step 1's 48.

## 7. The Lows

| ID | Status | What |
| --- | --- | --- |
| A6-L-1 | Fixed | §6 |
| B6-L-1 | Fixed | §3 |
| B6-L-2 | Fixed | §2 (the ≤ 250-block sub-reads not done: `raw` instead) |
| C6-L-1 | Fixed | §5; with C5-I-1's measurement still a launch precondition (§9 call 3) |
| C6-L-2 | Fixed | §5 |
| C6-L-3 | Fixed | §5 |
| D6-L-1 | Fixed | §4 |
| D6-L-2 | Fixed | §4 |
| D6-L-3 | Fixed | §5 |
| F6-L-1 | Fixed (by cadence) | §4: no deeper keyless 4663 provider exists; the runner ticks inside the state reach while a home transaction is out, the search starts inside the reach when it can, and the operator path is paged and written |
| F6-L-2 | Fixed | §4 |
| B5-L-2 (carried) | Fixed | §2: a skipped block is never given up; one-sided reads are not remembered. A block whose Seaport half is persistently over the cap (a padded fill in a flooded block) is still unreadable for Seaport there, but retried for as long as it is in the window |
| E5-L-2, A5-L-3 | Not fixed | Unchanged (`fixes-round5.md` §7): the launch steps keep their margins |
| C4-L-1 | Unchanged | The hold is now paged persistently (D6-L-3) |
| C3-L-1, E3-L-2, B3-L-1 | Not fixed | Unchanged |
| B-L-6 | Partly | Unchanged (C6-I-3: the frog side's conversions outside the hold; unreachable NFT-only) |
| A-L-2, B-L-5, C-L-5, D-L-3, D-L-6 | Unchanged | As `fixes-round2.md` §7 |

**Lows: 11 new at `bb713fe`, all fixed; the carried B5-L-2 fixed; the rest carried as listed.**

## 8. The Infos

- **Fixed:**
  - **E6-I-1:** the H3 guard (`audit_nft_r5.rs` `h3_sites`, `h3_check`, `src_files`) catches the reviewer's five mutations:
    - it reads every `outflow_record(` too, and a record's key must be a reviewed key of its file;
    - it resolves `mint_key`'s scope argument to its binding;
    - it refuses a key bound `let mut`;
    - it refuses any use of `outflow_check` / `outflow_record` other than a call or a declaration (path calls, aliases);
    - it walks `src/` recursively.

    `r5_e5_i2_…` asserts each of the five.
  - **B6-I-1** (§1.2).
  - **B6-I-2** (§2).
  - **C6-I-1** (§5).
  - **D6-I-1** (§1.2).
  - **D6-I-2** (§5).
  - **D6-I-3** (the same as A6-L-1).
  - **A6-I-2**, **A6-I-4** (§6).
  - **D6-I-4:** the Trust page's "the week's real sellers" now says the canister reads the whole week (both providers must serve it,
    and a shorter sample pages), and "never read as another order's" says the release is only on Relay's records.
  - **F6-I-2's first half** (§4).
  - **C6-I-2:** the runbook states the release rule.
- **Unchanged:**
  - **F6-I-1 / E6-I-2:** a two-step chain move keeps the old chain's floors. It fails closed and needs two timelocked moves off
    Ethereum.
  - **F6-I-2's second half:** `eth_home_clear` is still refused while paused. The paused pass's records wait behind an unidentified
    deposit until the resume.
  - **F6-I-3:** a collection-wide mint refusal admits its waiting deposits to the book.
  - **F6-I-4:** `adopt_advance` with no room, reachable only from an earlier build's state.
  - **B6-I-3:** one shared `stuck` counter.
  - **C6-I-3.**
  - **A6-I-1:** the plan's timing is the runbook's.
  - **A6-I-3.**
  - **D6-I-5.**
  - **D5-I-3's first half:** a written-off order's payout landing is not paged. The hold is.
  - **C5-I-1** (a launch precondition), **C5-I-3**, and the earlier rounds' as listed there.

## 9. New judgment calls for the operator

1. **The canister's Ethereum pair is MEV Blocker + Tenderly's public gateway** (B6-M-1's recommendation, re-measured). Both are
   third parties with keyless tiers that can change. golive-check `--probe-depth` and the `eth-scan-lost` / `eth-scan-behind` pages
   are the watch. A replacement is a timelocked upgrade. MEV Blocker sends our Ethereum transactions to private builders; the canister
   also sends to Tenderly, which broadcasts publicly.
2. **A forward walk a day behind re-anchors** at the newest blocks and reads the gap newest first, instead of reading it oldest first.
   The newest sales are counted first; what the providers cannot serve is recorded as not counted (the floor's sample is short, and
   paged), never waited on.
3. **The payout judge credits a call to Relay's depository only with the depository's `RelayCallExecuted` paying us, at its amount**
   (read from the depository's bytecode; not yet seen on a real Relay payout). C5-I-1's measurement before launch must confirm that a
   real refund carries it. If it does not, no depository-paid refund can be credited until the judge follows it, and the order can
   only be released. A plain transfer to us naming the order is still credited at its value: whoever sends it pays that ETH, and it
   ends the hold.
4. **While a 4663 transaction of the Ethereum NFT side is out, the runner ticks every 240 s** (`RUNNER_PENDING_EVERY_S`), not 600 s:
   a few more ticks' cycles, for identification inside the ≈ 10 minutes of state Robinhood's provider keeps.
5. **A receipt the providers disagree on, with its nonce used, is decided as "seen nowhere" is** (F6-L-2): the effect read-back (every
   provider agreeing) decides a state-bearing step, and the operator a Relay deposit or a `fundReserve` after the hour.
6. **A credit waits while its buy has a 4663 transaction out.** A claim that goes stale meanwhile (600 s) is handed in again.
7. **A written-off forward order keeps its custody room until credited or released.** A released one credited later takes the room
   again, or its ETH is the collection's leftover when the cap is full.
8. **A skipped oversized block is retried for as long as it is in the window** (at most every 18 h), no longer given up after 3 tries.

## 10. Left open (for round 7)

- **Carried Lows and Infos:** E5-L-2, A5-L-3, C3-L-1, E3-L-2, B3-L-1, B-L-6's automatic reads, A-L-2, B-L-5, C-L-5, D-L-3, D-L-6; the
  Infos listed as unchanged in §8.
- **The operator's pre-install checks** (runbook §5.1):
  - the canister's Ethereum pair through a real IC outcall (Tenderly's IPv4-only host; consensus through the transforms);
  - `--probe-depth` at go-live;
  - C5-I-1's real Relay fill and refund each way, which must show the `RelayCallExecuted` the judge now requires.
- **F5-I-1:** the other 4663 signers (`looper.rs`, `redeem.rs`, `frogflow.rs`) still wait with no exit on a transaction every index
  misses. They benefit from the runner's shorter tick only while the Ethereum NFT side has one out.
- **The reviewers' PoCs re-run on this code** (`fixes-round6-poc/`). Every B, C, A, D and F Low/Medium PoC fails its attack
  assertion, except:
  - `r6_f6_l1_…`, which drives the canister at the old 600 s cadence by construction (the fix is the runner's);
  - `r6_b_m2_a_…`, which asserts nothing (its printout: 432 of 432 counted, 0 lost);
  - A6-I-2's JS-level PoC (golive-check's `--phase1` is where the baseline is required);
  - D6-L-2's PoC, whose state is the round-5 type with no `sent_at`.

## 11. As built (2026-09-28)

**Commits** (branch `nft-side`, on `bb713fe`):

| Commit | What |
| --- | --- |
| `2d747ba` | Canister: B6-M-1 (classification, re-anchor, hole, lost, behind), B6-L-1, B6-L-2, B5-L-2, F6-L-1 (the search's reach), F6-L-2, D6-L-1, D6-L-2 (`sent_at`), C6-L-1, C6-L-2, C6-L-3, C6-I-1, E6-I-1; the `.did`; `eth-state-round6.hex`, `state/eth-state-e04fa16.hex`, the round-2/4/5 fixtures re-encoded; `tests/audit_nft_r6.rs`; the older tests that encoded the replaced behaviour updated (`audit_nft_r4.rs` the judge, `audit_nft_r5.rs` the judge, the Unsure clock, the measured shapes, the H3 guard; struct literals in `audit_nft_r2..r5`) |
| `7b9e8ba` | Monitor and runner: D6-L-1, D6-L-2, D6-L-3, D6-I-1, D6-I-2, B6-M-1's pages; A6-L-1, A6-I-4 (`staged.mjs`, `argdiff.mjs`, `watcher.mjs`); the runner's pending cadence and IDL (F6-L-1); `audit_nft_d_r6`, `staged_r6`, ops `audit_nft_r6`; `guards` and `runner` tests updated |
| `b4c3f55` | Go-live: golive-check's measured Ethereum list and `--probe-depth` (B6-M-1), `--proposal`'s baseline under `--phase1` and the exact staged step (A6-I-2, A6-L-1); the dry run's install argument on the new pair and the old pair refused; the launch file's `_outflow` text |
| `785b384` | Site: the Trust page (D6-I-4) |
| `2841f90` | Go-live: the provider-depth lines redact a key-like subdomain as the keyless rule does (the dry run's A4-L-3 subdomain-key case caught it) |
| (this document's commit) | This document; `fixes-round6-poc/`; `docs/v7/phase1-nft.md` (§4.1, D6, §5.1 the runbook, §11.13) |

**Results** (2026-09-28, on `2841f90`'s tree):

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites; unchanged: no contract changed) |
| The Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **699 passed**, 0 failed (was 683: +16 `tests/audit_nft_r6.rs`; the E6-I-1 mutations, the measured shapes through the transforms and the judge's execution record are asserted inside existing tests) |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed (unchanged) |
| ops | **60 passed** (+2 `audit_nft_r6`); typecheck clean |
| monitor | **214 passed** (+6 `staged_r6`, +5 `audit_nft_d_r6`) |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | 72 passed; typecheck clean |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| The `.did` | `service_compatible` with e04fa16's; every field of the round-2, -4, -5 and -6 fixtures the same through the `.did` and through the Rust encoder, both ways (review 6's `didcheck6` method) |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS**: 146 checks ok on the v3–v6 stack; 536 on the five-collection NFT stack; **664** on the launch go/no-go read of the COMMITTED `nft-pilot.json`, with the install argument on the new pair (both providers on the measured list); 52 refusals `ok`, new: the round-5 pair (xrpc.cl + publicnode) refused under `--phase1` (B6-M-1). `--stage 2` and `--proposal` steps 2 and 3 pass; both-at-once and caps-first are refused |
| The real go/no-go read on the committed launch file | Passes (the dry run's launch read: golive-check `--canister --launch deployments/launch/nft-pilot.json --monitor-config --install-arg`, step 1) |

**Reproducible build** (`autobuyer/Dockerfile.build`, linux/amd64): two `git archive` exports of `2d747ba`, each built with
`docker build --no-cache`. No later commit of this round touches `autobuyer/`. Both builds gave the same hashes:

```
00d2d63530fee23d98a762c5ce81657a3f8bda65e7dd9459deb9c110a28ed3f1  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

- **`buyer.wasm` is new:** 7,076,857 bytes, 1,345,257 gzipped with `gzip -9 -n` (under the 2,000,000 proposal limit). It replaces
  `3218bff1…d99e` (`fixes-round5.md`) for any install of this branch.
- **The timelock is unchanged.**
- **Nothing was installed.**
- **Removed afterwards:**
  - the exports, the two build images and the extracted wasm;
  - the private PoC and probe copies;
  - the anvils.

  Docker's shared build cache was left alone.

**The delta round 7 must cover:** `git diff bb713fe..HEAD`:

- `autobuyer/canister-rs/`:
  - `etch_autobuyer.did`;
  - `src/{canister,ethflow,ethnet,ethnft,pin,relayconv}.rs`;
  - `tests/{audit_nft_r2,audit_nft_r3,audit_nft_r4,audit_nft_r5,audit_nft_r6}.rs`;
  - `tests/fixtures/{eth-state-round2.hex, eth-state-round4.hex, eth-state-round5.hex, eth-state-round6.hex, state/eth-state-e04fa16.hex}`.
- `monitor/`:
  - `src/{argdiff,candid,ethnft,staged,watcher}.mjs`;
  - `test/{audit_nft_d_r6,staged_r6}.test.mjs`.
- `ops/`:
  - `src/{ic,runner}.ts`;
  - `test/{audit_nft_r6,guards,runner}.test.ts`.
- `script/golive-check.sh`, `scripts/golive-dryrun.sh`, `deployments/launch/nft-pilot.json`.
- `web-rook/src/rook/phase1Trust.ts`.
- `docs/v7/phase1-nft.md`, this document, `fixes-round6-poc/`.
