# Review round 8, 2026-09-28: confirmation of fix round 7 (branch `nft-side`)

Reviewer: independent AI re-review. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `577c992`. Scope:
`git diff effc270..HEAD` (`48d62fa`, `533c8a9`, `428c32a`, `3300c96`, `577c992`), against `../review7/` and `../fixes-round7.md`.

No code was modified. Nothing was pushed, deployed or signed. PoCs ran in private copies under `/tmp/rook-r8/` (removed afterwards).
Network use was read-only JSON-RPC only: `eth_blockNumber`, `eth_getLogs`, `eth_getTransactionReceipt`, `eth_getBlockByNumber` and
`eth_getTransactionByHash` on Ethereum (MEV Blocker, Tenderly, publicnode) and 4663 (`rpc.mainnet.chain.robinhood.com`); the
fork upstreams; the build's crate fetch.

## Verdict

**Still 0 C / 0 H / 0 M.** All six round-7 Lows are closed. There is one new Low (B8-L-1), a paging regression caused by the
B7-L-1 fix, and there are three Infos. None of them blocks launch.

| ID | Verdict | Basis |
| --- | --- | --- |
| **B7-L-3** | **Closed** | `SaleScan.current` moves only when the forward walk reaches `top` (a re-anchor never moves it). `behind` is `top − current`. The monitor reads its own head first (the lower of two) and pages `head − 128 − current` (or `behind` if larger): SEV2 at 2,400 blocks, SEV1 at 7,200, persistent. A stopped scan pages too (PoC `r8_stale.mjs`: SEV2 at 12 h, SEV1 at 2 and 30 days). A healthy scan lags about 140 + 100 blocks (Ethereum bucket 8), far below 2,400 |
| **D7-L-1** | **Closed** | `depth_probe` requires exactly 10 logs over 26,012,410..26,012,909, plus the receipt of `0x17133e8b…c093` (not null, block 26,012,500, status 1, a Milady `Transfer`). An empty or short list fails. **Re-probed today, read-only:** both providers returned 10 logs at 63,807 blocks deep, first tx `0x17133e8b…`, receipt `(26,012,500, 0x1, 1 Transfer)` (`poc/pinned_range_probe.py`) |
| **B7-L-1** | **Closed** as reported | `ask_scan` / `pair_deep`: the answer is `deep` only when every provider refuses as too deep. One provider's "too deep" beside a served answer is `Other`. A hole is never given up: it is kept, `hole_deep` is recorded, it is retried 3× at once then every 6 h, and it is paged (`eth-scan-deep`, `eth-scan-hole-day`). The mixed error's wording matches no `error_class` words (checked). **Regression: B8-L-1** |
| **B7-L-2** | **Closed** | `rbusy` backs off 600 s × 2^(n−1), capped at 6 h. Busy receipts share the 4 retry reads with `rfail` and are pruned to the queued moves. `busy` counts passes in a row. Both pages are SEV2. Divergence and "beyond current head" are `Busy` |
| **F7-L-1** | **Closed** | `HomeTx.pin + BUCKET` is the lowest rounded head at signing (`choose_pin` = min − b; the pin is cached per tick and read before the broadcast). Each start is verified by the nonce read before use (a used or refused start moves on), so a wrong start cannot mis-identify. Arithmetic: (6,198 − 1,023)/10 = 517 s, and 415 s with the other provider one bucket ahead. The runner: a 120 s period from the cycle's start, digits only, 60–300 and ≤ tick; normal cadence once `used_since` is 15 min old |
| **C7-L-1** | **Closed in code, pending C5-I-1** (whether real refunds take this path) | **The struct hash was recomputed independently** (python + `cast keccak`, no repo code). For Ethereum `0x86eee61f…d84d`, the input byte-equals the test constant and `keccak(CallRequest typehash, keccak(call hashes), nonce, expiration)` = the log's id `0x84334077…6659`. The same holds on a **real 4663 execute** `0x89635e98…6a38` (id `0x14ba700e…8157`, selector `0x2d9fb478`). Spoofs refused (`poc/r8_canister…diff`, `r8_c_b`): a crafted request against another request's record; a record paying another address with our order id in its data; a zero-value record. An amount collision (two equal records) is credited once at that amount, never the sum. The operator-only hand-in and the 4663 balance guard (balance − open buys' ETH ≥ paid) bound a wrong naming to ETH that really arrived and is free |

## New findings

| ID | Sev | Where | What | Fix |
| --- | --- | --- | --- | --- |
| **B8-L-1** | Low | `nftnet.rs` `pair_deep`; the `ethflow.rs` walk back; `monitor/src/ethnft.mjs` | The B7-L-1 fix stops ONE provider's "too deep" from steering the scan. But it also stops a genuinely shallow provider from paging. Say one provider of the pair loses log retention after go-live, beyond the probe's single moment. Its refusals are now `Other`: the walk back retries at `down` every pass and never records `lost` or `hole_deep`. Nothing the monitor reads is set. In round 7 this was `lost`, paged SEV2 `eth-scan-lost`. PoC `r8_b_one_genuinely_shallow_provider_is_no_longer_paged` (one provider 20,000 blocks deep, 144 passes): 30 of 71 deep sales counted, `lost`/`hole`/`hole_deep`/`bad` all None, `down` stuck at 26,049,151 (stuck count 1). The control with both providers shallow: 29 of 71, `lost` set (paged). Sales are only uncounted, never false (a shorter floor sample). This is the silent case D7-I-4 already names for `down` | Record persistent mixed-deep answers (first at, count) and page them. Or have the monitor page `down > bottom` for more than a day |
| B8-I-1 | Info | monitor `eth-scan-behind` | Every `sale_scan` entry is paged whatever the config. A collection removed by a later upgrade, or a long pause (the tick returns before `floor_pass`), keeps a frozen `current`: a persistent SEV1 that cannot clear. The pause case is true, but the page does not name the pause | Filter to configured collections; say "paused" in the text |
| B8-I-2 | Info | `ethflow.rs` rule 4 (C7-I-1) | `credited` is per transaction hash, so a request paying us two refunds (two written-off orders batched) can credit only one. The other's ETH stays unbooked (bound: one order's value, as C7-L-1). PoC `r8_c_a` | Key credits by (tx, record index), or note it for C5-I-1 |
| B8-I-3 | Info | `ethflow.rs` `hole_due` | While `hole_deep` is backed off (only after 3 answers, when `eth-scan-deep` already pages), the whole hole walk is skipped. A new re-anchor's gap merged into that hole then waits up to 6 h. It is only a delay | Read the hole's newer part above the refused span |

## Upgrade safety

The new fields `SaleScan.current`, `hole_deep`, `rbusy` and `busy` are all `opt` with `#[serde(default)]`, and no enum case was added.
`current` of an old state is set to `up.min(top)` on its first pass. `state/eth-state-a78a542.hex` is byte for byte `a78a542`'s
`eth-state-round6.hex` (sha1 `093771c1…`), and `r7_the_state_the_round_6_build_wrote_…` decodes it. `eth-state-round7.hex` is decoded by the
canister and monitor tests. The `.did`, the monitor and the ops IDLs carry the same fields. `pending.used_since` was already in the
`.did`. Holds.

## Test bar (at `577c992`; no tracked file changed)

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | 1,115 passed, 0 failed (126 suites) |
| Seaport fork proof (anvil fork via `eth.drpc.org` at 26,055,690) | 2/2 passed |
| Canister `cargo test` (cargo 1.98.1) | **713 passed**, 0 failed |
| Timelock, and `--features rehearsal` | 15 and 15 passed |
| ops / monitor / keeper / indexer / web-rook / staging / tools/snapshot | 63 / 219 / 150 / 135 / 72 / 29 / 19 passed; every typecheck clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS**: 146 / 536 / **664** (the go/no-go read of the committed `nft-pilot.json`); 52 refusals `ok` |

**Reproducible build:** a `git archive` of HEAD's `autobuyer/`, built with `docker build --no-cache --platform linux/amd64 -f Dockerfile.build`.
The hashes are identical to `fixes-round7.md` §9:

```
b7532c4dbdeb3704bc3c8932a7dc15c86461b856e3d6f12acb2942de357628ec  buyer.wasm  (7,106,636 bytes; gzip -9 -n 1,351,080)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

**Removed afterwards:** the image, the export and the wasm; `/tmp/rook-r8`; the anvils. Docker's shared build cache was left alone.

## Operator pre-launch tasks

As in `../review7/README.md`: the IC-outcall rehearsal of the pair, C5-I-1, the monitor config, the runner env with
`RUNNER_PENDING_EVERY_S` 120, and steps 2 and 3 with `--phase1`. Add one task until B8-L-1 is fixed: watch the scan note's `down`
against the window's bottom by hand, or re-run `--probe-depth` periodically.
