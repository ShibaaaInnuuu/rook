# Phase 8: the staging simulators

Plan, written before the code. Staging (docs/staging.md, tier 1) proves the product *works*; it does not show what the product
*looks like* with people using it. A visitor arriving at a freshly booted staging chain sees empty grids, and an empty grid tells
you nothing about whether a page is any good. The simulators fill it: coins launching, curves being traded, vaults filling,
purchases moving through every state at once, draws at every stage.

Everything below runs **only** on our staging fork (`staging/chain-id.json`). Every wallet is made at boot and thrown away, every
price and receipt is a fixture, and no key exists that is worth anything anywhere.

## What runs, and what turns it off

One process, `staging/sim/sim.ts`, started by the same container as the node, so it can use the node's own admin methods over
`127.0.0.1` (the public proxy blocks them). Every part has a switch; all default **on** in staging and nowhere else.

| Switch | Default | What it turns off |
|---|---|---|
| `STAGING_SIM` | `1` | everything below |
| `STAGING_SIM_TRADE` | `1` | launches and curve/pool trading |
| `STAGING_SIM_HARVEST` | `1` | the permissionless `harvest()` caller |
| `STAGING_SIM_SPREAD` | `1` | the state-spreader (purchases and draws held in every state at once) |
| `STAGING_SIM_COINS` | `15` | the cap on live simulated coins |
| `STAGING_SIM_WALLETS` | `30` | how many throwaway wallets |
| `STAGING_SIM_EVERY_MS` | `4000` | the base gap between trades |

## 1. Wallets

About thirty, generated in-process at each boot with `viem`'s `generatePrivateKey`, funded with `anvil_setBalance`. Their keys
are never logged, never written to the volume, and never leave the process; a restart makes new ones. They are **not** the node's
well-known development accounts: docs/phase2.md found sweeper delegations sitting on those addresses on this chain, and a
delegated address silently loses what you send it.

The simulator asks the node for its chain id before it does anything and refuses any answer but ours.

## 2. Launches and trading

Coins are launched through our own `Launcher` against whichever listings the registry holds, with creator taxes spread across
0–10% so the Launch page's fee preview has something to be checked against. Names say what they are — *Basalt (sim)*, `BSLTSIM` —
and each carries a generated SVG logo as a `data:` URI inside Pons's 512-byte cap.

Trading is on Pons's **real** curve, and coins are given moods so the grids do not all look alike:

- **steady** — the default: small buys, the odd sell.
- **momentum** — a burst of a minute or two: bigger buys, closer together.
- **fading** — sells outweigh buys; the coin drifts down.
- **runner** — buys on a schedule until the curve sells out and Pons **graduates** the coin into its v4 pool. After graduation the
  simulator keeps trading it in the pool, because post-graduation fee flow is the thing the Coin and Analytics pages most need to
  show and the one thing a single smoke run never reaches.

One coin is bought in its first seconds by a separate wallet, so Pons's decaying anti-snipe tax appears in the fees.

A new coin every 20–30 minutes, up to `STAGING_SIM_COINS`.

## 3. Harvests

`harvest()` is permissionless, so a wallet that has nothing to do with a coin calls it on a schedule. Vaults therefore fill
repeatedly rather than once, and the 80/20 split shows up again and again on the Receipts page instead of as a single row.

## 4. Every state at once

A page that can show six states is only tested when all six are on the screen. The spreader (`staging/sim/spread.ts`, pure and
unit-tested) looks at what the chain currently holds and asks for the missing states, letting the operator do the ordinary work
and stepping in only for the states the operator would never leave sitting:

| Wanted on screen | How it is held there |
|---|---|
| a declaration inside its public wait | the spreader declines to shorten one declaration's wait, so a countdown is always running |
| released, awaiting its receipt | one released purchase is left unsettled |
| settled, with a Bitcoin txid | the operator's ordinary path, several times over |
| returned unspent | the purchaser returns part of one release |
| written off | a receipt has **three days** to arrive and staging is reset long before that, so one release's stored deadline is rewritten into the past — as the operator already rewrites the public waits — and it is then written off. Without that the state could not exist at all |
| a bond top-up | the purchaser tops the bond up as outstanding grows |
| a draw inside its challenge window | one draw's window is left un-shortened |
| pinned, awaiting a drand round | pinning happens ~60 s before the round exists, so this state is naturally occupied |
| resolved, awaiting the winner's address | one winner is left to "decide" |
| delivered | the operator's ordinary path |
| overdue delivery | one delivery deadline is allowed to pass |
| a rune distribution part-claimed | some holders claim, some do not |

Creators choose across every mode — weighted draw, one-ticket lottery, named recipient, hold, and pro-rata on the rune listing —
so `#/draws` and `#/claims` are never empty and never show one shape only.

Winners' Bitcoin addresses are BIP-350 test vectors: valid taproot addresses **nobody holds a key to**, and the pages say so.

## 5. How full it looks

Measured, not guessed: a cold start on a laptop against a fork of Robinhood Chain, run for twelve minutes at
`STAGING_SIM_EVERY_MS=1200` and `STAGING_SIM_HARVEST_MS=15000`, which is about three times the default pace.

| After | Coins | Trades | Harvests | Choices made |
|---|---|---|---|---|
| 5 min | 5, one already **graduated** | 126 | 18 | 2 |
| 12 min | 5 | 292 | 35 | 7 |

Over those twelve minutes it also produced one purchase returned unspent, one written off, three winners' addresses posted, and
one failure out of some four hundred steps.

At the shipped defaults (a 4 s base gap, a harvest every 45 s) divide the trades and harvests by roughly three: about 40 trades
and 4 harvests in the first five minutes, 90 and 16 by twelve. **The coins do not scale with that**: the first five launch within
the first minute or two whatever the pace, and one every 20–30 minutes after that, so a visitor sees about 5 coins in the first
few minutes, 7 after an hour, and 9–10 after two. The cap of 15 is reached in about four hours.

The first graduation happened inside five minutes, because a *runner* buys until its curve sells out. That is the number worth
watching: post-graduation fee flow is what the Coin and Analytics pages most need, and a single smoke run never reaches it.

## 6. Operations and limits

- One summary line a minute: launches, trades, buys, sells, failures, coins by mood and phase, graduations, purchases by status,
  draws by stage.
- It exits only if the node stops answering or 50 steps fail in a row; the container restarts it, as it restarts the operator.
- It survives the node's restart: state lives on the chain, so a new set of wallets simply carries on with the coins that exist.
- **Spending.** Wallets are topped up with `anvil_setBalance` rather than the faucet, so a visitor's faucet allowance is never
  consumed by the simulator. Trade sizes are a share of what a Pons curve needs to graduate, which keeps a runner's total inside
  a few staging ETH, and staging ETH is made from nothing.
- **The fork's cache.** Every trade is on contracts the fork has already pulled in, so the simulator adds no new upstream reads
  after the first coin. The upstream shim's fallback counter (`/healthz`) is the thing to watch if that stops being true.

## What is not simulated, and why

Nothing on Bitcoin. There is no inscription, no rune, no transaction and no wallet: every receipt, delivery and payout the
simulator produces is the same `staging-fixture` hash the operator already writes, and the pages label it. A real purchase can
still be recorded by hand with `npm run attest`, which is the only way anything true about Bitcoin reaches staging.
