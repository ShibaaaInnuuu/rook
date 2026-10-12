# Confirmation review of the go-live prep branch (`golive-prep`, 47c6cf5..0d1a132, 2026-09-28)

Scope: the 11 commits on `main` 47c6cf5 read adversarially. Inputs were `docs/v7/golive-runbook.md` and `fixes-round8.md`. The one
canister change is `173470e`; the rest is the monitor, the ops images, the Railway templates, the rehearsal kits, the depth probe and
its workflow. Read-only: no code was changed, and nothing was deployed, signed or sent.

**Verdict: 0 Critical, 0 High, 0 Medium, 0 Low, 8 Info. Ready to merge into `main`.** `buyer.wasm` `d38b808d…effc6` reproduced.

## 1. `173470e`: Ethereum block reads reserve 128 kB

- **Coverage.** Every `eth_getBlockByNumber` in `src/` was listed. The two Ethereum ones are `nftnet::pin_on` and `ethnet::eth_finalized`,
  and both now pass `block_view_bytes(chain_id)`. The others read the HOME chain, 4663, through `cfg().rpc_urls`, and their caps are
  unchanged: `ethnet::home_finalized_time` (16 kB), `canister::IcRpc::{finalized, block}` (16 / 64 kB), `canister` `our_txs_view`
  (2 MB) and `custodynet::block_clock` (`eth_getBlockByHash`, 64 kB). The new unit test covers `nftnet.rs` and `ethnet.rs`, which are
  the only files with Ethereum block reads. `git diff 47c6cf5 173470e -- autobuyer/canister-rs/src` changes only these two call sites
  and adds one constant, one function and one test. No other outcall's cap or behaviour changed.
- **Consensus.** The transforms (`nft_block_at` → `chains::block_body_ts`, `finalized_bucket`) are unchanged. They still project the
  block to `{number, hash, baseFeePerGas, timestamp}` and strip the headers. The cap applies only to the raw body, so replicas still
  agree on identical transformed bytes.
- **Cost.** One provider-read on 13 nodes costs `(3e6+60k·13)·13 + 400·13·~110 + 800·13·128,000` ≈ **1.38 B** (the runbook says 1.33 B,
  which counts the response part only). The increase over 16 kB is ≈ 1.17 B a read. That is ≈ 3.65 B a read on 34 nodes. `EthNet`
  caches the pin once per tick, so the pin costs 2 reads a tick, ≈ 0.4 T a day. `eth_finalized` is not cached (see I-1). A 1–2 T/day
  estimate means about 2–5 finality reads a tick, which is plausible for the normal load.
- **A block over 128 kB.** The read fails as `Err` and the tick's Ethereum part retries on the next tick. Nothing wedges:
  - `pin_on` takes a new pin on every tick from the heads (bucket 8, which is ≈ 96 s against a 600 s tick).
  - The `finalized` tag moves every epoch (≈ 6.4 min).
  - `PIN_FLOORS.admit_pin` runs only after a successful read.
  - No state is written on the failure path.
- **Upgrade safety.** No stored type or stable-memory layout changed. The `EthState`/`SaleScan` encodings are the same, and the
  monitor's added `down` field already existed in the canister's type.

## 2. The monitor

- **`eth-scan-window-unread`** (B8-L-1) was checked against the code:
  - The window start is `finalHead − 50,400`, so the healthy gap is ≤ 0: the canister's pin trails the monitor's head.
  - `firstSeen` is keyed on the `down` value. The 12 h timer restarts when `down` moves, and the page clears once the gap is under 600.
  - A failed Ethereum head read throws, and `pollOnce` then keeps the previous state, so a transient error does not reset the timer.
  - `up = 0` or no head pages nothing. The test decodes the canister's own fixture bytes.
- **B8-I-1.** Addresses are compared in lowercase. `validate` requires at least one collection, so an empty set cannot mute every scan.
- **`burnCheck`:**
  - Only falls in the balance count, so a top-up is never counted as burn. A top-up inside a 10 min sample hides that interval's burn,
    which is negligible.
  - It needs 6 h of samples before any page, and keeps at most `window_h·6` samples a label.
  - SEV1 is persistent. Every other level pages once per crossing and posts an INFO when it clears.
  - `now` arrives in ms from `pollOnce`. `/status.cycles` reads `state.watch.burn_report`.
- **`validate(cycles)`** bounds every field and refuses unknown keys. `launch_config.test.mjs` loads the launch file through the real
  loader.

## 3. Ops images, the log scrub, the Railway templates

- **Images.** Both images were rebuilt and started. The runner stops with `CANISTER_ID is not set` and the reconciler with its
  Telegram refusal. Neither shows `ERR_MODULE_NOT_FOUND` now: ESM static imports all load before those checks, so this is a real
  load test.
- **Guard test.** It walks value imports transitively. Its gaps (`export … from`, `import("…")`, bare `import "…"`) match nothing in
  either image's files today (I-6).
- **`ETH_LOGS_RPC`.** Both the whole URL and its 16+ character path segment are scrubbed (I-7).
- **`check-env.mjs`.** All six templates pass. Every secret is marked SECRET and has an empty value. `inline-monitor-config.mjs`
  writes `${DWELLIR_API_KEY}` unexpanded.

## 4. The rehearsal kits

- **Nothing signs or sends.**
  - The probe holds no key. It has one controller-only update (`probe`) and the transforms.
  - `buyer-throwaway` installs with `test_key_1` on the IC (`accept_production_key_name` false). The buyer has no timers, and
    `nft_chain_status` reads only.
  - `quote.ts` POSTs to `/quote` and prints the transaction the wallet must sign, field by field. It re-derives the order id and prints
    "do NOT sign" on any mismatch.
  - `verify.mjs` and `relay_verify.rs` are read-only and offline respectively.
- **Cleanup.** `run.sh` and `buyer-throwaway.sh` both set an EXIT trap after creation that stops the canister and runs
  `dfx canister delete -y`. dfx's default withdraws the cycles to the identity's cycles ledger. INT/TERM exit through the trap.
- **Probe fidelity.** The 7 probe tests pass. They hold `head_bucket_body`, `block_body_ts`, `transfer_logs_body`,
  `json_rpc_norm_body`, `receipt_projection`, `busy_body`, `choose_pin` and `TRANSFER` byte for byte to the buyer's, on real answers
  and on 6 malformed bodies. They also hold `PIN_BLOCK_BYTES` to `ETH_BLOCK_VIEW_BYTES`. The transform wrappers (`rate_limited`, the
  status handling) were compared by reading: they match.
- **Secrets.** `RELAY_API_KEY` goes only into a header and is never printed. Only the `--urls` / `--user` public values appear in args
  and logs.

## 5. The depth probe and the workflow

- `depth_probe.py` is the old golive-check code moved unchanged, plus `facts` and the CLI's agreement check. It prints only
  `scheme://host` and scrubs URLs out of errors.
- golive-check `cd`s to the repo root before importing it. The dry run below exercises that import.
- The workflow uses no secret. It has `contents: read` at the top level and adds `issues: write` for the job only. The providers input
  goes through `env` and is never templated into the script. An issue is opened only on `schedule` when the probe step fails.

## Findings (all Info; none blocks merge)

| ID | Finding |
| --- | --- |
| G9-I-1 | `eth_finalized` is not memoized per tick. It is called for each open buy (`Bought`), delivery (`Moved`/`MarkAfter`) and payout waiting on finality, plus deposits and returns. Each call is now 2 × ≈ 1.38 B, so the burn grows ≈ 0.4 T a day for each item that waits all day. It is bounded by open items and `burnCheck` sees it. Caching it once per tick in a later upgrade would bound it. |
| G9-I-2 | 128 kB has good headroom on today's blocks (largest seen 51 kB), not on the theoretical maximum. A 45 M-gas block of 21k-gas transfers is ≈ 148 kB hashes-only, and ≈ 197 kB at 60 M gas. Such a block is a failed read retried next tick (no wedge, §1). Denying reads on purpose would take continuous full blocks at a base fee that rises 12.5% each block. Revisit if the gas limit rises a lot. |
| G9-I-3 | Neither rehearsal path exercises `eth_finalized` at 128 kB: Path A reads the pin and a fixed block, and Path B uses `nft_chain_status`. The transform is unchanged and the cap only widens, so the risk is low. |
| G9-I-4 | Stale text. The `buyer-throwaway.sh` header still says the pin read has "a 16,000-byte cap". The `depth-probe.yml` header says the monitor pages "for a day", but the timer is 12 h. The runbook's "1.33 B" leaves out the per-request base, which gives ≈ 1.38 B. |
| G9-I-5 | `buyer-throwaway.sh` does not check that `--cycles`/`--tries` are numbers (`run.sh` does) and prints no cycles-ledger balance before or after. A failed delete prints a WARNING but does not change the exit code. In both scripts, a Ctrl-C between a successful `canister create` and `CREATED=1` would skip the cleanup (a tiny window). |
| G9-I-6 | The image guard's import walker does not follow `export … from`, dynamic `import()` or bare `import "…"`. None exist in the images' files today. `check-env.mjs` lists the reconciler's sources by hand, not transitively. |
| G9-I-7 | `urlSecrets` does not scrub path segments under 16 characters or a query key's value on its own. This is fine for Dwellir's path key; recheck if another keyed provider is used. |
| G9-I-8 | Scheduled workflows run only from the default branch, so the daily probe starts after the merge. On a public repository, GitHub disables schedules after 60 days without activity. |

## Verification (run on `golive-prep` HEAD 0d1a132)

| Suite | Result |
| --- | --- |
| Canister `cargo +1.98.1 test` | 716 passed, 0 failed |
| IC probe `cargo +1.98.1 test` | 7 passed |
| monitor / ops (+ typecheck) / indexer / keeper / tools/snapshot | 238 / 68 (clean) / 170 / 150 / 19 passed |
| `script/test_depth_probe.py` | 10 passed |
| `check-env.mjs` | all six ok |
| runner and reconciler images | built; both start to their configuration refusal |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 / 536 / 664 checks; private ports 8763/8764) |
| Reproducible build: `git archive 173470e`, `docker build --no-cache --platform linux/amd64 -f Dockerfile.build` | `buyer.wasm` **`d38b808d77d807713e3468981d3a16e4d5ae150dcd18607286a156237bdeffc6`** (7,106,677 bytes); timelock `b23a1903…dc41` and timelock-rehearsal `2145099e…62f4` unchanged. `autobuyer/` is identical at 173470e and HEAD |
