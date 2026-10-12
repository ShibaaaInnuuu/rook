# Keeper dry-run, version 1

Written 2026-09-21, before its code. The keeper is the operator key that posts caps and minimums, declares purchases, opens draws
and records deliveries. This package is **not** that keeper. It is the part of it that thinks, with the part that acts left out:
it reads our contracts and a price source, works out what a keeper following the published policy **would** post, and writes that
to a journal. It exists so the policy can be watched running against real prices before any key exists.

## It cannot send — by construction, not by configuration

- No signer is constructed anywhere in the package. There is no private key variable, no wallet client, no account import.
- Its one JSON-RPC function refuses every method that is not on a short read-only list (`eth_call`, `eth_getLogs`,
  `eth_blockNumber`, `eth_chainId`, `eth_getBlockByNumber`). `eth_sendRawTransaction` is not on it, so even a future bug that built a
  transaction could not broadcast one through this code.
- A test reads every source file in the package and fails on a wallet or signing import, a key-shaped variable, or a send method.
- It never calls a venue's buy endpoint. It has no code for one.

## What it reads

- **Our contracts**, at an RPC URL the operator gives it (a local anvil or a fork; the default is `127.0.0.1:8545`): each listing,
  its vault's spendable balance (`attributed − reserved`), posted cap and expiry, open purchases, whether it is blocked by an
  overdue receipt or paused.
- **A price source**, behind one interface: an inscription collection's floor in sats, a rune's price per whole unit in sats, each
  with a reference string that says where the number came from (it becomes the declaration's `capRef`). Two implementations:
  `fixture` (a JSON file; the default, and what the tests use) and `magiceden` (keyless public read endpoints, one request per
  five seconds at most, only when selected by name — docs/phase1.md F3 for why, and that the reading of their terms is ours).
  The Magic Eden response shapes were written from general knowledge of that API and are **unverified against the live service**;
  a response the code does not understand is journalled as such and produces no plan.
- **An ETH→BTC quote** from LI.FI's public quote endpoint, keyless, at most one request per three seconds: how much ETH on 4663
  must be sent for a given number of sats to arrive. A fixture quoter is the default in tests.

## The policy it applies (every number is a parameter, journalled with each decision)

1. A listing that is paused, blocked by an overdue receipt, or already has three open purchases gets no plan, and the journal says
   which.
2. **Cap**: the floor (or unit price), converted to ETH through the quote — so the cap already includes the bridge's cost — plus a
   margin (default 5%). A cap is only re-posted when the posted one has under 15 minutes left or differs from the new one by more
   than 2%.
3. **Declaration**: only when the spendable balance covers at least one item under the cap **and** is at least 0.05 ETH — below
   that the bridge's fixed costs exceed about 1% (Phase 0 §3.1). Quantity is what the balance covers, up to a maximum (default 3
   inscriptions; for a rune, the units the balance buys). `maxEth` is cap × quantity, never more than the balance.
4. **The purchase it would attempt**: venue, asset, the sats it expects to pay, the ETH it would bridge, and the bridge's quoted
   cost and duration. Recorded, never attempted.

## The journal

One JSON object per line in `keeper/journal/` (not committed): what was read, the inputs' references, each decision with the rule
that produced it. Deterministic given the same inputs and clock.

## What it does not show

That a purchase would fill at the floor, that the bridge would deliver what it quoted, or that a venue would let us buy at all.
Those are the purchaser's risks and the reason the contracts bound what the purchaser is handed.
