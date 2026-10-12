# IC-outcall rehearsal of the canister's Ethereum pair (operator runbook)

The hard precondition of review round 7 (`docs/audit/2026-09-27-nft/review7/README.md`, "Remaining operator pre-launch tasks" 1;
`docs/v7/phase1-nft.md` §5.1): before the install argument is built, show **from a real IC outcall on the launch subnet** that the
buyer's Ethereum reads work on both providers of its pair, `https://rpc.mevblocker.io` and `https://gateway.tenderly.co/public/mainnet`:
consensus on every read, the two providers agreeing after the buyer's transforms, the pinned facts (Milady's 10 `Transfer` logs over
26,012,410..26,012,909 and the receipt of `0x17133e8b…c093`), and Tenderly's IPv4-only host reached through the IC's NAT64.

A local replica does not count for the network question (its outcalls leave from your own machine), but it runs the same code.

Nothing here holds a key, signs, or sends a transaction. It spends the operator's own cycles: a canister's creation fee and a handful
of outcalls (below). The canister is deleted at the end, always, and what is left goes back to the identity's cycles-ledger balance.

## STATUS AT PREP TIME (2026-09-28, local replica)

**First run: FAIL on the buyer's own pin read; fixed in the canister (commit `173470e`); second run: PASS.**

The finding: every Ethereum read of the buyer starts with `nftnet::pin_on`, which asked `eth_getBlockByNumber(pin, false)` with
`max_response_bytes` 16,000 (and `ethnet`'s `eth_finalized` read the finalized block with the same cap). The IC's cap is on the
provider's RAW body, before the transform, and today's Ethereum blocks are bigger even with transaction hashes only: 24
bucket-aligned blocks were 10,960..51,161 bytes (median 25,521, 22 of 24 over 16,000); eight more read on both providers 19,309..32,635;
the finalized block 24,279. With the real `buyer.wasm` then (installed in a throwaway local canister): `nft_chain_status(1)` failed 3
of 3 with `Http body exceeds size limit of 16000 bytes.` So as built the buyer could take an Ethereum pin on almost no tick, and no
Ethereum read (sale scan, buys, custody, deliveries) would have run. No review round saw it: the tests use a fake world and the dry
run makes no IC outcalls. **The fix** (`nftnet::block_view_bytes`): Ethereum block reads reserve 128,000 bytes; other chains keep
16,000 (a 4663 block is ≈ 4.3 kB). New reproducible `buyer.wasm`: `d38b808d77d807713e3468981d3a16e4d5ae150dcd18607286a156237bdeffc6`
(it needs a short confirmation review). The probe asks the pin read at `check::PIN_BLOCK_BYTES` = 128,000, held by a test to the
buyer's constant.

Re-run locally after the fix (dfx 0.32.0's PocketIC, real HTTPS outcalls from this machine):

| Read (both providers) | Local result |
| --- | --- |
| `eth_blockNumber` via `head_bucket` | PASS, both |
| `eth_getBlockByNumber(pin, false)` via `nft_block_at` at the buyer's cap (128,000) | **PASS, both** (pin 26,077,248, 163 identical transformed bytes); FAIL at 16,000 before the fix |
| `eth_getBlockByNumber(26,012,500)` via `nft_block_at` (64 kB, for its hash) | PASS, both, hash `0xda4c2b0a…b909` |
| Milady `eth_getLogs` 26,012,410..26,012,909 via `eth_transfer_logs` (64 kB) | PASS, both: the 10 logs are 7 transactions, `0x17133e8b…` among them, 1,559 identical bytes |
| receipt `0x17133e8b…` via `eth_receipt_logs` (32 kB) | PASS, both: block 26,012,500, status 1, a Milady `Transfer`; 1,061 identical bytes |
| pin (`choose_pin`, bucket 8), Tenderly reached | PASS (Tenderly: locally only, not evidence for the IC's NAT64) |
| **Path A verdict** | **`REHEARSAL: PASS`**; the canister deleted, 0.902 T of 1.5 T withdrawn |
| Path B, the new `buyer.wasm`, `nft_chain_status(1)` × 3, twice | the pin read passed on every call that reached the providers (pins 26,077,256 / 26,077,376 / 26,077,384); the other calls failed with `ConnectionReset` or no answer **from this machine** to the providers (not the cap): run it on mainnet |

## Two paths

- **Path A, `run.sh` (the probe canister, `probe/`).** A 1.1 MB canister that makes the buyer's reads the way the buyer makes them:
  replicated outcalls, the buyer's response caps, the buyer's transform NAMES and BODIES. The bodies are copied from
  `autobuyer/canister-rs/src` (`probe/src/bodies.rs`; the buyer's crate cannot be linked into another canister: every buyer
  endpoint, `canister_init` included, would be exported with it, measured) and `cargo test` holds each copy **byte for byte** to the
  buyer's own function on both providers' real answers (`probe/tests/fixtures/`, read-only RPC of 2026-09-28) and on error bodies.
  It covers everything §5.1 asks: the pin read, the pinned block, the log read, the receipt read, agreement, Tenderly.
- **Path B, `buyer-throwaway.sh` (the real buyer.wasm).** The reproducible build's `buyer.wasm` in a throwaway canister with a
  throwaway Init (`buyer-init.mjs`: the pair as `evm_chains`' only chain, nothing custodial, signet, the test key), asked
  `nft_chain_status(1)`: the buyer's own `pin_on` and two `json_rpc_norm` reads at the pin. The buyer exposes no controller endpoint
  for the sale scan's log or receipt reads (`pinned_read` is the 4663 home chain's), so Path B cannot replace Path A; run both.

## Prerequisites

- dfx 0.32 (`dfx --version`), Rust 1.98.1 with `wasm32-unknown-unknown` (`rustup target add wasm32-unknown-unknown --toolchain 1.98.1`),
  node 22, python3; `npm ci` in `monitor/` (Path B's Init is encoded by the monitor's IDL).
- Your own dfx identity with cycles on the cycles ledger: `dfx cycles balance --network ic` (top up with
  `dfx cycles convert --amount <ICP> --network ic`). About **2 T** is plenty for both paths (most of it comes back).
- The LAUNCH subnet: the subnet the buyer is (or will be) on. Once the buyer exists, `--next-to <buyer id>` puts the throwaway
  canister on its subnet; before, `--subnet <subnet id>` (the one you will create the buyer on).

## Run it

```bash
cd ops/rehearsal/ic-outcall
( cd probe && cargo +1.98.1 test )                         # the copies = the buyer's bodies; the verdict logic (6 tests)

# Path A on the launch subnet (asks you to type 'rehearse'; --yes skips it)
./run.sh --network ic --next-to <buyer canister id>          # or: --subnet <launch subnet id>

# Path B, with the reproducible build's wasm (autobuyer/Dockerfile.build; hash as fixes-round*.md §9 / the build log)
./buyer-throwaway.sh --wasm <path to buyer.wasm> --network ic --next-to <buyer canister id>

# the same locally (a project-local replica on 127.0.0.1:4955; --clean touches only this folder's replica)
dfx start --clean --background && ./run.sh --network local && ./buyer-throwaway.sh --wasm <buyer.wasm> --network local; dfx stop
```

`run.sh` prints one `PASS`/`FAIL` line per check (5 per provider, then the pair's agreement on the pinned block, the block, the logs
and the receipt, the pin, and Tenderly), the cycles the outcalls cost, each provider's transformed answers, and `REHEARSAL: PASS` or
`REHEARSAL: FAIL` (exit 0 only on PASS). The full report is saved to `$TMPDIR/rook-ic-outcall-report.json`; keep it with the ceremony
log. Another provider can be tried with `--urls https://a,https://b` (keyless only: the probe's URLs are not secret, but do not type
a key into a command line).

**Expected output on a good subnet** (the lines of the local run, with the pin read passing once the buyer is fixed):

```
PASS rpc.mevblocker.io: head (eth_blockNumber via head_bucket): replicated outcall reached consensus: rounded head 26077176
PASS rpc.mevblocker.io: the buyer's pin read (eth_getBlockByNumber(pin, false) via nft_block_at, cap … bytes as nftnet::pin_on): block … hash 0x…
PASS rpc.mevblocker.io: block 26012500 (…): consensus, and the measured hash: hash 0xda4c2b0a15d889c5b6dbcb29d21ffd6b99255cea2cdf426da26e8cbe9843b909
PASS rpc.mevblocker.io: Milady Transfer logs 26012410..26012909 (…): 7 moves, 0x17133e8b…c093 among them
PASS rpc.mevblocker.io: receipt 0x17133e8b…c093 (…): block 26012500, status 0x1, 1 Milady Transfer log(s)
… the same five for gateway.tenderly.co …
PASS the pair agrees on the pinned block / the block / the logs / the receipt
PASS the pair's heads pin (pin::choose_pin, within one bucket): pin …
PASS gateway.tenderly.co (IPv4 only) is reachable from this subnet: its outcall came back (whatever the answer): the IC reaches it
REHEARSAL: PASS
```

**On a FAIL:** a Tenderly line that says the outcall failed (connect / DNS) means the IC's NAT64 does not reach it from this subnet:
pick another keyless provider that passes `script/golive-check.sh --phase1 --probe-depth` and this rehearsal (`--urls`), and add it
to golive-check's measured list. A short log list or a null receipt is a provider that does not keep that depth (D7-L-1). A
"transformed answers DIFFER" line means the buyer would believe nothing on that read. `Http body exceeds size limit` on the pin read
is the finding above.

## Clean-up and what it costs

Both scripts trap EXIT: `dfx canister stop`, then `dfx canister delete -y`, which first withdraws the canister's remaining cycles to
your identity's cycles-ledger balance (dfx's default), and removes `canister_ids.json`. They print the balance before and after
(Path A). If a delete fails they print the exact command to run by hand.

Outcall price (docs/deploy/canister-mainnet.md, "Cycles"): `(3,000,000 + 60,000·n)·n` per request + `400·n` per request byte +
`800·n` per RESERVED response byte. Path A makes 5 reads per provider reserving 8 + 128 + 64 + 64 + 32 = 296 kB:

| | 13-node subnet | 34-node subnet (fiduciary) |
| --- | --- | --- |
| per provider (5 outcalls) | 5 × 49.1 M + 296,000 × 10,400 ≈ **3.33 B** | 5 × 171.4 M + 296,000 × 27,200 ≈ **8.91 B** |
| Path A, both providers | ≈ **6.7 B** (measured locally at the old 16 kB cap: 4.40 B) | ≈ **17.8 B** |
| Path B, per `nft_chain_status` (per provider: head 8 kB, block 128 kB, balance and nonce 8 kB each) | ≈ 2.4 B; ×3 ≈ 7.2 B | ≈ 6.5 B; ×3 ≈ 19.5 B |
| canister creation fee | 500 B | ≈ 1.31 T (scales with the subnet size) |
| install (1.1 MB probe / 7.1 MB buyer), calls, deletion | a few B | a few B |
| **net spend per path** | **≈ 0.5–0.6 T** (measured locally for Path A: 1.5 T in, 0.908 T withdrawn) | **≈ 1.35 T** |

`--cycles` sets what the canister is created with (default 1.5 T for Path A, 2 T for Path B, enough for the creation fee plus a 30-day
freezing reserve of a small canister); everything above the spend comes back on delete.

## Verified where

| What | Where verified |
| --- | --- |
| The transform copies equal the buyer's functions, byte for byte, on both providers' real answers and error bodies | `cargo test` (native) |
| The verdict: good answers pass; empty/short logs, a `deep` refusal, a null receipt, a receipt above the pin, an unreachable Tenderly, a disagreement, heads a bucket apart, the pin read over the cap all FAIL | `cargo test` (native) |
| The probe installs, makes replicated outcalls through its transforms, reaches consensus, and `run.sh` creates, reports and deletes (cycles withdrawn) | local replica (dfx 0.32.0 / PocketIC), 2026-09-28 |
| Both providers serve the pinned logs and receipt, agreeing after the transforms | local replica, real outcalls from this machine |
| **The buyer's pin read failed at its old 16,000-byte cap** (probe and the real buyer.wasm, 3 of 3); passes at 128,000 (probe, and the rebuilt buyer.wasm `d38b808d…effc6`) | local replica |
| Consensus across a real subnet's 13 (or 34) replicas, and Tenderly reached through the IC's NAT64 | **mainnet only**: the operator's run |
| `dfx canister delete`'s withdrawal to the cycles ledger on mainnet, and the exact creation fee on the launch subnet | **mainnet only** |
