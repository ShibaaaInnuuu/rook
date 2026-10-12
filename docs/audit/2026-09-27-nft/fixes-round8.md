# Fix round 8 of the 2026-09-27 NFT review (branch `golive-prep`, off `main` at `47c6cf5`)

Scope: review round 8's one Low (B8-L-1) and one Info (B8-I-1), found while preparing the operator's go-live checklist
(`docs/v7/golive-runbook.md`), and one NEW canister finding from the IC-outcall rehearsal (G8-H-1 below, the only canister source
change: `173470e`). **New `buyer.wasm`: `d38b808d77d807713e3468981d3a16e4d5ae150dcd18607286a156237bdeffc6`** (7,106,677 bytes; a
`--no-cache` reproducible build of `173470e`; timelock hashes unchanged). **It needs a short confirmation review.**

## G8-H-1 (found by the rehearsal; fixed): the buyer's Ethereum pin read could not succeed

`nftnet::pin_on` read `eth_getBlockByNumber(pin, false)` on Ethereum with `max_response_bytes` 16,000, as did `ethnet`'s
`eth_finalized`. The IC holds that cap to the provider's RAW body, and today's Ethereum blocks with hashes only are 11–51 kB (both
providers of the pair, read-only: 19,309..32,635 bytes over eight blocks at the pin spacing; the finalized block 24,279; 22 of 24 blocks
over 16,000 in the rehearsal's sample). Reproduced with the review-8 `buyer.wasm` on a local replica making real outcalls:
`nft_chain_status(1)` failed 3 of 3 (`Http body exceeds size limit of 16000 bytes`). Impact as built: almost no tick takes an Ethereum
pin, so no Ethereum read runs (sale scan, buys, custody checks, deliveries, arrivals): a liveness failure of the whole NFT side from
the first tick, nothing unsafe. No test saw it (fake worlds; the dry run makes no IC outcalls). **Fix:** `nftnet::block_view_bytes`:
128,000 bytes for chain 1, 16,000 for any other (a 4663 block is ≈ 4.3 kB, unchanged cost); both Ethereum block reads use it; a unit
test holds every `eth_getBlockByNumber` in `nftnet.rs` and `ethnet.rs` to it. Cost ≈ 1.33 B a provider-read on 13 nodes (≈ 1–2 T a
day). After it, the rehearsal passes locally (probe: every check; the rebuilt `buyer.wasm`: the pin read on every call that reached
the providers). The mainnet run remains the operator's (`ops/rehearsal/ic-outcall/`).

| ID | Status | What changed |
| --- | --- | --- |
| **B8-L-1** | **Fixed (monitor side)**, as review 8's second option | The monitor pages `eth-scan-window-unread` (SEV2, persistent) when the scan's walk back (`SaleScan.down`, now in the monitor's IDL) has not moved for 12 h while it is still 600 blocks (2 h) or more above the window's start (the monitor's own final head − 50,400). Plus a daily read-only depth probe of the pair from GitHub (`.github/workflows/depth-probe.yml`), which fails loudly and opens an issue |
| **B8-I-1** | **Fixed** | `checkEthNft` passes the configured collections; a `sale_scan` entry of a collection the configuration no longer names pages nothing. (The pause wording is not added: the tick's pause is visible in the watcher's own pages) |
| B8-I-2, B8-I-3 | Open (Info) | Unchanged: both need canister changes; neither blocks launch |

## B8-L-1: why the monitor, and why 12 h rather than a day

The review offered two fixes: record persistent mixed-deep answers in the canister, or have the monitor page `down > bottom` for
over a day. The monitor fix was taken because it needs no canister change (no new wasm, no confirmation review of the canister
before launch), and because it watches the symptom, not one cause: any failure that holds the walk back (a mixed "too deep", a
persistent `Other`, D7-I-4's unread `down`) now pages.

A stuck `down` does not stay stuck forever: the window's start moves up 7,200 blocks a day and passes it, after which every block in
the window has been read (the forward walk is current) and the sample is whole again. So the gap shrinks while it waits. With a
day's timer, a provider keeping 43,000 blocks (a 7,400-block gap) would have cleared before the page fired. At 12 h and a 600-block
gap: any provider keeping under ≈ 46,000 blocks pages at the install's first backfill or after an outage longer than the window.
The canister's healthy lag behind the monitor's head (a pin a bucket back, a pass every 20 minutes) is a few hundred blocks, under
the 600. A walk back that moves never pages (the first hours after install).

**Regression tests, the review's PoC turned around:**

- `autobuyer/canister-rs/tests/audit_nft_r7.rs`: the review's `mixed_depth` world (applied from `review8/poc/`), and
  `r8_b8_l1_one_shallow_provider_leaves_the_walk_back_stuck_above_the_window_start_and_that_is_what_the_monitor_pages`: one provider
  20,000 blocks deep, 216 passes. `lost` / `hole` / `hole_deep` stay unset (B7-L-1 holds), `down` is unmoved over the last 72 passes
  and 9,151 blocks above the window's start; the bytes it ends in are `tests/fixtures/eth-state-round8-b8l1.hex`. Controls: an honest
  pair's `down` is the window's start; both providers shallow sets `lost`.
- `monitor/test/audit_nft_d_r8.test.mjs` decodes those bytes with the monitor's IDL: round 7's pages stay silent on them (the
  finding), and `eth-scan-window-unread` fires SEV2 at exactly 12 h, repeats while the gap lasts and clears once it is under 600
  blocks; a moving walk back never pages; a step restarts the 12 h; no head or `up = 0` pages nothing; the monitor's window and final
  depth are read from the canister's source (`SALE_WINDOW_BLOCKS`, `SALE_FINAL_DEPTH_BLOCKS`). B8-I-1: an unconfigured collection's
  frozen scan pages nothing; a configured one (any case) still does.
- `script/test_depth_probe.py` (offline fakes): an honest pair passes and agrees; an empty (D7-L-1) or short answer, a null receipt, a
  deep refusal, a head too low and two providers with different logs fail; the CLI retries a transient error, golive-check's single
  read does not; a URL is printed as scheme and host only.

## Also found and fixed while preparing go-live

- **`ops/Dockerfile.runner` could not start**: it did not copy `relay.ts` / `ethnft.ts`, which `runner.ts` imports
  (`ERR_MODULE_NOT_FOUND`). **`ops/Dockerfile.reconciler`** missed `ethRecords.ts`, `indexer/src/outbound.ts`,
  `indexer/src/rook/catalogSources.ts` and `monitor/src/config.mjs`. Both images now build and start to their configuration refusal;
  `ops/test/guards.test.ts` holds every image's COPY lines and `.dockerignore` to what its entry point loads, transitively.
- `ops/src/log.ts`: the runner's keyed `ETH_LOGS_RPC` (Dwellir: the key is the path) is scrubbed from every log line.
- `script/golive-check.sh`: the depth probe moved, unchanged, into `script/depth_probe.py`, which golive-check imports; the same
  module is the daily workflow's and the operator's CLI (`python3 script/depth_probe.py`), so the three cannot drift.
- The monitor: a burn-rate page for the buyer's cycles (`watcher.mjs` `burnCheck`, `cycles` in the configuration; runbook §6).

## Test bar (this branch)

See `docs/v7/golive-runbook.md` §8.
