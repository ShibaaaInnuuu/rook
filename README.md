# Rook

*(formerly Etchwrap / etched; the repository was renamed to `rook` on 2026-09-23 — GitHub redirects the old URL)*

Rook is a launchpad on Robinhood Chain (chain id 4663) where memecoins are paired with wrapped collectibles: Bitcoin
inscriptions (wFROG) held by an IC canister, and EVM NFTs held by keyless home-chain vaults. Coins launch on Rook's own curve
with a decaying anti-sniper fee and a creator-chosen fee tier; graduation mints the coin's wX through the Floor Module (mint and
burn at the keeper's floor ± spread, an ETH reserve, items bought for net demand); fees sweep items for a raffle, a burn, or a
relist-and-buyback (YOYO); the protocol funds itself through a treasury with fixed allocations. Design docs: `docs/v5/` and
`docs/v6/`; launch gates: `docs/v5/launch-gates.md`. The text below describes the original Etch/etched system (v1–v2), which
the v3+ stack builds on.

---

# etched

https://etched.meme

A memecoin launchpad on Robinhood Chain (chain id 4663), over Pons V2, in which a coin's creator fees buy Bitcoin assets —
inscriptions from one ordinals collection, or units of one rune — and hand them to the coin's holders: inscriptions by a draw
settled by a drand round, rune units pro rata by Merkle claim. Each coin gets an ownerless fee router that Pons pays as the
coin's creator; the router sends 80% to the listing's vault and 20% to the treasury, for ever. A vault has no withdraw function:
ETH leaves only through a purchase declared in public under a fresh price cap, released an hour later, and receipted by a
different key — or the vault stops. The assets are on Bitcoin, so the purchase, the receipt and the holder snapshot are people's
word; `docs/trust-model.md` says exactly which, and what bounds them.

**Status: unaudited, undeployed, Phase 3.** Phase 3 was our own security pass — not an audit — and it found two high-severity
defects in our own code (a vault that could be locked for cents; a purchase that could keep its change), now fixed with regression
tests: `docs/phase3.md`, `docs/security/`. Nothing here has been deployed to a public chain, no key has been used, no transaction
sent to one. Phase 2 added a deploy script that refuses by default, a web app that says so when there is nothing to read, and a
keeper that runs the whole cycle against a fork on your own machine. Phase 6 added a hosted staging fork (its own chain id, 9466301, play
money, no key anywhere: `docs/staging.md`), draft legal pages, and the *preparation* for a small real test that only the owner
can run and nobody has. Not affiliated with Robinhood Markets, Pons, Nosh, UniSat or
Magic Eden.

## Read first

- `docs/design.md` — what it is, what contracts enforce, what is somebody's word
- `docs/staging.md` — how to try all of it with play money, what is simulated there, and the small real test (prepared, not run)
- `docs/phase0.md` — what was verified on the live chain and on a fork, and what is unverified
- `docs/phase1.md` — what reaches the router after graduation (measured), the venues and their terms, the bond
- `docs/phase3.md`, `docs/security/` — the internal security pass: findings and fixes, threat model, slither triage, gas bounds, recovery, a brief for an auditor
- `docs/phase2.md` — the deploy script's refusals, the local end-to-end run, the web app, and how the web goes on Railway
- `docs/snapshot-v1.md`, `docs/keeper-dryrun-v1.md` — the rules of the two off-chain tools
- `docs/trust-model.md` — every actor × what they can and cannot do

## Run the tests

```
forge test                                                          # unit, fuzz and invariant tests
forge test --match-path "test/fork/*" --fork-url robinhood -vv      # Pons V2 integration on a fork of chain 4663 (public RPC)
```

```
cd tools/snapshot && npm install && npm test     # the snapshot tool; `npm run fixtures` rewrites what test/SnapshotTool.t.sol reads
cd keeper && npm install && npm test             # the keeper dry-run: plans, journals, and cannot send
cd indexer && npm install && npm test            # the indexer and its read-only API, replayed from a recorded local run (no node needed)
```

```
bash script/deploy.test.sh                       # the deploy wrapper's refusals, against a bare local node; deploys nothing
cd web && npm install && npm test && npm run build
```

## See it run, locally

```
cd keeper && npm install && (cd ../tools/snapshot && npm install) && npm run e2e:local
cd ../web && npm install && npm run build && npm start        # then open http://localhost:8080/?demo
```

`e2e:local` starts a fork of Robinhood Chain on your machine (chain id 31337) if none is running, deploys through the guarded
script from the node's unlocked accounts — no key exists in this repository — and ends with a winner's claim and a recorded
delivery, using a real drand round. Prices, the Bitcoin receipt and the delivery are fixtures and are labelled as such.
`keeper/local/` can send only to a node that itself reports chain id 31337; `keeper/src/` still cannot send at all.

### The data layer, served locally

```
npm run demo        # at the repo root: the fork, the end-to-end run above, then the indexer + API left serving it on http://127.0.0.1:8787
```

`indexer/` follows our contracts' events into SQLite and answers read-only JSON (`/v1/health`, `/v1/stats`, `/v1/coins`,
`/v1/listings`, `/v1/acquisitions`, `/v1/draws`; shapes in `indexer/API.md`, types in `indexer/types.ts`). It has no signer and
calls nothing but the chain RPC. With no `deployments/<chainid>.json` for the node's chain it idles and says so. Static copies of
every answer are in `indexer/fixtures/demo/` (that local run) and `indexer/fixtures/synthetic/` (invented events, every state).
Formulas, including "volume until the next buy": `docs/phase4.md`.

Without `--fork-url` the fork tests return early. The drand verifier is tested against real beacons of drand's `evmnet`
(`test/fixtures/drand-evmnet.json`).

## Contracts

`ListingRegistry` · `Launcher` · `FeeRouter` (EIP-1167 clone per coin) · `AssetVault` (one per listing, both kinds) ·
`Distributor` · `rand/DrandEvmnet` + `rand/BN254HashToCurve` · `lib/BtcAddress`. There is no deployment script.

Off chain, both read-only: `tools/snapshot/` (recompute a holder snapshot's root from public logs) and `keeper/` (what a keeper
following the policy *would* post; it has no signer and its one RPC function refuses anything but reads).
