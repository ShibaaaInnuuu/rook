# Brief for an auditor

Etch is **unaudited**. This is what we would hand an auditor on day one. It was written by the people who wrote the contracts,
after their own security pass (docs/phase3.md), which is not an audit and found two high-severity defects in its own code — take
that as the prior for what is left.

## Scope

Commit: the head of `main` at the time of engagement (this brief was written at the commit that adds it). Solidity 0.8.28, via-IR,
optimizer 200, EVM `cancun`, OpenZeppelin 5. Target chain: Robinhood Chain (4663), an Arbitrum Nitro L2. In scope, 1,565 lines:

| File | Lines | What it is |
|---|---|---|
| `src/AssetVault.sol` | ~450 | One per listing. Holds fees, releases them only through a declared, capped, delayed, bonded purchase. **The money.** |
| `src/Distributor.sol` | ~390 | Who gets what was bought: draws by drand round with Merkle ticket ranges; pro-rata Merkle claims for runes; delivery records. |
| `src/ListingRegistry.sol` | ~250 | Listings, roles, every slow change. Deploys each vault. |
| `src/FeeRouter.sol` | ~115 | One clone per coin; Pons's creator-fee recipient; splits 80/20. Ownerless. |
| `src/Launcher.sol` | ~90 | Launches on Pons V2 with the clone as recipient. Ownerless. |
| `src/rand/DrandEvmnet.sol`, `BN254HashToCurve.sol` | ~200 | On-chain verification of drand evmnet (BLS on BN254, RFC 9380 SVDW hash-to-curve). |
| `src/lib/BtcAddress.sol` | ~55 | bech32m check for `bc1p…`. A typo guard, not a safety property. |

Out of scope: Pons V2 (their contracts; our assumptions about them are in docs/phase0.md and docs/phase1.md, verified on a fork),
the keeper, the snapshot tool, the web app. **Off-chain trust is the design, not a bug**: docs/trust-model.md.

## Architecture in two pages

Read `docs/design.md` §1–4 and `docs/trust-model.md`. In one paragraph: a coin launched through the Launcher names a FeeRouter
clone as its Pons creator-fee recipient, permanently. Anyone calls `harvest()`; 80% goes to the listing's AssetVault attributed to
the coin, 20% to the treasury. The keeper posts a price cap (1 h TTL) and declares a purchase within cap × quantity; after one
public hour anyone may release its ETH to the purchaser **named at declaration**, if that purchaser's own bond covers the
registry's share of everything in their hands. A different key, the attestor, receipts what arrived on Bitcoin; a purchase that
delivers less than it declared cannot be closed until the difference is back; an unreceipted purchase is written off against the
bond, by the owner after the deadline or by anyone a week later. Each acquisition belongs to the coin whose fees paid; its creator
chooses hold / never / weighted draw / per-wallet draw / named / pro rata. Draws: keeper's Merkle root, 15 public minutes, anyone
pins a drand round 60 s ahead, anyone proves it, anyone proves the winner, the winner posts a Bitcoin address.

## Invariants we believe hold (and test)

1. ETH leaves a vault only as a matured, capped, bonded release to the purchaser named at declaration, or as a refund of a
   purchaser's own unneeded bond. There is no other transfer out. (`invariant_vaultOutflowIsReleasesAndBondRefunds`)
2. `attributed + waiting + bond + forced == balance`; `reserved ≤ attributed`; per-purchaser bonds and outstanding sum to the
   totals; a coin is in the queue at most once. (`invariant_books`)
3. For every purchase, the sum of its charges equals `maxEth`. (`SystemHandler.release`)
4. Every wei that entered through any fee path is in a router, escrow, the vault, the treasury, or a purchaser's hands.
   (`invariant_conservation`)
5. 80/20 to the wei, rounding in the treasury's favour by at most 1 wei a harvest, whatever the treasury does. (`invariant_split`)
6. A share is claimed once, only by its true (holder, units) pair, and claims never exceed what was bought.
   (`invariant_aClaimPaysOnce`)
7. Every loop is bounded by `MAX_PAYERS` (64) or `MAX_OPEN` (3). (`docs/security/gas.md`)

## Known issues and accepted risks

docs/phase3.md lists all sixteen findings with their fixes; the accepted ones are I1–I6 there. The ones we would most like a
second opinion on:

1. **No sunset path** (docs/security/recovery.md). ETH stranded by a bug is lost rather than exposed to the owner key. Is that
   the right side of the trade?
2. **`MIN_ENTRY` (0.01 ETH) and `MAX_PAYERS` (64).** Is there a cheaper griefing of the queue we did not see — e.g. 64 coins parked
   at the head with 0.01 ETH each to force small declarations? It costs 0.64 ETH of real fees that are then spent; we think that is
   acceptable. Is a release that spans more than 64 payers ever *needed*?
3. **The settlement rule** `(maxEth − returned) × quantity ≤ maxEth × received`. Rounding, and whether a purchaser and attestor
   can game `quantity` (declared by the keeper) against it.
4. **`writeOff` by anyone after 7 days**, and the slash's pro-rata credit back to coins. Rounding; re-entrancy is guarded.
5. **The draw on an L2**: `PIN_AHEAD = 20` rounds against a sequencer-set clock; `voidDraw` as a second draw; the 24 h `redraw`
   choice (I2).
6. **`DrandEvmnet`**: the hash-to-curve against the RFC and drand's own implementation (we test 22 real beacons and a fuzz that
   the map lands on the curve; we have not compared intermediate values against a reference implementation). The constants
   (public key, DST, genesis, period). What happens if evmnet is retired.
7. **Pons as a dependency**: the recipient can be redirected by Pons's owner (3-day timelock); post-graduation sweeps belong to
   Pons's operator. We treat both as facts of the platform. Is anything in the router unsafe if Pons's contracts are upgraded?
8. **Creators who never choose** (I3): should there be a default?

## Tests

`forge test --no-match-path "test/fork/*"`: 127 tests — unit, fuzz (512 runs), invariants (64 × 64). `FOUNDRY_PROFILE=deep` raises
that to 10,000 fuzz runs and 1,000 × 200 invariant calls (45 s). `forge test --match-path "test/fork/*"`: 8 tests against the
public 4663 RPC and the live Pons factory. `test/security/` is this phase: regressions per finding, what held, 20 real drand
beacons, the system campaigns, gas. `cd keeper && npm run e2e:local`: the whole flow on a local fork, 15 steps, a real drand round.

## How to run everything

```
forge build --sizes
forge test --no-match-path "test/fork/*"
FOUNDRY_PROFILE=deep forge test --no-match-path "test/fork/*"
forge test --match-path "test/fork/*"
bash script/deploy.test.sh
(cd tools/snapshot && npm ci && npm test)
(cd keeper && npm ci && npm test)
slither . --filter-paths "lib/|test/|script/" --exclude-informational --exclude-optimization
```

## What we most want attacked

The vault's state machine and its accounting under adversarial ordering (rotation mid-purchase, write-off racing a return, a cap
expiring mid-flight), anything that lets ETH leave without a bond behind it, and anything that lets a draw be steered.
