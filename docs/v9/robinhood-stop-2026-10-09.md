# Where the Robinhood Chain launch stopped — 2026-10-09

**The operator's decision, 2026-10-09:** there is no volume on Robinhood Chain; Rook launches on **Solana** instead. This page records
exactly where the Robinhood Chain (chain 4663) launch stood when work on it stopped, so it can be resumed from here if ever wanted,
and so the Solana migration (`docs/v9/solana-migration.md`) starts from known ground. Nothing was ever deployed to 4663, the IC
mainnet (beyond the two throwaway rehearsal canisters, deleted) or Bitcoin: **there is nothing live to wind down.**

The code as it stood: branch `golive-2026-10-08`, `main` fast-forwarded to **`5c8d1ea`**, tagged `robinhood-stop-2026-10-09`.

## 1. What was finished (all on `main` at `5c8d1ea`)

| Area | State | Where |
|---|---|---|
| Contracts (Solidity, 65 files) | Complete for the phase-1 launch; forge suites green; the deep fuzz fixed (`FloorReserve` harness, `FrogCurve` tolerance); `FLOOR_TTL` 3 h; wake bond 0.002 ETH | `src/`, `test/`, `.github/workflows/ci.yml` |
| Reproducible build of the canister | Done for `07d37c0`: `buyer.wasm` 9f0e65ab…957f; **stale since `3228c5f`** (the review fixes changed the canister; not rebuilt) | `docs/v7/golive-runbook.md` §0 |
| IC-outcall rehearsal (paths A and B) | **PASS on IC mainnet**, subnet 4utr6-…-mqe; the default identity has ≈5.46 T cycles left | runbook §1, `ops/rehearsal/ic-outcall/` |
| Relay round trip (C5-I-1) | **PASS live**, four transfers creditable; `/requests/v3` filters pinned | runbook §2, `ops/rehearsal/relay/` |
| Cycles from fees (the fueler) | Built and tested, not deployed, not reviewed | `ops/src/fueler.ts`, `ops/src/fuel/`, `docs/v7/cycles-from-fees.md` §4.2.1 |
| ETH-from-any-chain → ckETH → ICP page | Built, used once (Base → 7.29 T cycles) | `ops/rehearsal/cketh/` |
| Dormant collections, first-coin wake, 3 h floor | Done | `docs/v7/dormant-collections.md`, `web-rook/src/rook/dormancy.ts`, `Launch.tsx` |
| Ordinals Wallet protected listings | Judge, state machine, runner step, review round done (P-01..P-09 fixed) | `docs/v8/ow-protected.md`, `autobuyer/canister-rs/src/owbuy.rs`, `ops/src/owprotected.ts` |
| UniSat buy step | Built (off by default); **whether `create_bid` carries the seller's signature is unverified** | `ops/src/unisatbuy.ts` |
| Bitcoin conversion review | C-01..C-08 fixed with regression tests | `docs/audit/2026-10-09-conv-review-fixes.md` |
| Bitcoin Frogs at launch | Launch file, caps (Azuki 17→14; step-3 global 62), golive-check, monitor canary, runbook §9; mixed dry run ALL PASS (phase `mixed`) | `deployments/launch/nft-pilot.json`, runbook §9 |
| Test bar at the stop | canister `cargo test` 838; ops 114; monitor 258; golive harness 119; deploy.test 37 | — |

## 2. What was open (the launch blockers as they stood)

1. **G1 outside audit decision** — never made (`docs/v5/launch-gates.md`).
2. **New `buyer.wasm`** — the review fixes (`3228c5f`) changed the canister; the reproducible build and the Ethereum go/no-go were to
   be repeated. The operator waived the second person (2026-10-09, `launch-gates.md`): a clean CI-runner build instead.
3. **The key ceremony** — not started; no `deployments/4663.json`, no `canister-ic.json`.
4. **The second 4663 RPC** — `rpc.ordofi.network` dead for historical reads ("network is busy" at every depth); the operator's Dwellir
   Robinhood archive endpoint (pons-swarm's `RPC_URL_FALLBACK`, measured: archive at 8 M blocks back, `eth_getLogs` ≤ **500** blocks)
   was being wired in (canister log span 8,000 → ≤ 500, key-in-URL redaction) when the stop came. **That work was reverted, not
   committed.**
5. **The full dry run** on an archive RPC — only the `phase1` and `mixed` phases passed; the `nft` phase outlived the public RPC's
   ~15 min state window.
6. **One funded Frog purchase** — declined by the operator; the only test of UniSat's `create_bid` shape and Ordinals Wallet's submit
   path. A no-money substitute (UniSat testnet + a regtest simulation) was started (see §3).
7. **Depth-probe re-pin** around 2026-11-14 (runbook §3).
8. **Operator setters before the handover** — `set_ow_creators` (empty refuses royalty-paying protected sales), `set_unisat_key`
   after every upgrade, the fee-pool seed, the float padding seed (runbook §9.3).
9. **Secrets hygiene** — the Relay API key appeared in a transcript: regenerate. `UNISAT_API_KEY` exists in Railway (the old `UNISAT`
   variable too; delete it).

## 3. In flight at the stop (uncommitted or still running)

- **Identity MD** (contract `0x0000ec93127baa929e58e97dd0095a2bfb38ec1d`, OpenSea slug `identitymd`): being added as the eleventh
  Ethereum collection with **OpenSea alone** as its D8 price reference (the operator's decision; every other collection keeps two).
  Uncommitted edits across the launch file, monitor (`priceRefs`), runner template, golive-check, docs. **Carries over to Solana**: the
  Ethereum-side buying is unchanged by the home-chain move.
- **The simulated frog purchase** (`ops/rehearsal/sim/`, `tests/fixtures/unisat/`): UniSat testnet `create_bid` probe + a regtest
  Bitcoin-half simulation (float → listing → judge → sign → broadcast → custody). Re-scoped at the stop to the Bitcoin half only.
  **Carries over.**
- **Dwellir swap** — reverted (above).

## 4. What carries over to Solana unchanged

The Bitcoin side (PSBT judges classic and protected, taptree, custody, conflicts, the ord/UniSat/Ordinals Wallet readers, the
runner's Bitcoin steps, the members file and root), the Ethereum-mainnet NFT side (Seaport judge, OpenSea/Blur sale scan, the
collections' allowlist and caps rules), the monitor's Bitcoin canary, the fueler's cycles maths, and every review finding and fix
on those paths. What does not: everything that touches chain 4663 — the Solidity contracts and their tests, the canister's EVM
modules (`evmread`, `evmtx`, `evmsend`, `ethflow`, `ticketlog`, the 4663 halves of `convflow`/`frogflow`/`saleflow`/`floor*`), the
keeper/reconciler/indexer/resolver/monitor 4663 readers, the web app's wallet and chain config, `golive-check` and the dry run.
`docs/v9/solana-migration.md` is the plan.

## 5. To resume Robinhood Chain from here

`git checkout robinhood-stop-2026-10-09`; read this page, then `docs/v7/golive-runbook.md` (§0 build, §9 Bitcoin, §10 rollback) and
`docs/v5/launch-gates.md`. Rebuild `buyer.wasm`, re-run the Ethereum go/no-go, wire a working archive RPC (Dwellir: ≤ 500-block log
pages, key in the path), run the full dry run, then the ceremony.
