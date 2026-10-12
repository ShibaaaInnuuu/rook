# If a bug is found after launch

Written in Phase 3. There are no upgrades: no proxy, no owner-settable implementation, nothing to migrate. This is what can and
cannot be done.

## What can be stopped at once

- **A listing**: `registry.setPaused(id, true)` stops new declarations and stops `release` of declarations already made. Money
  that has not left stays.
- **A declaration**: `vault.cancel(id)` before it is released.
- **A snapshot**: `cancelDraw` / `cancelDistribution` inside its 15-minute window.
- **New launches on a listing**: the Launcher refuses a paused listing.

## What cannot be stopped

- `harvest()`: permissionless and ownerless by design. Fees keep arriving in the vault of a paused listing. They are safe there
  and unusable until it is unpaused.
- A draw that is already pinned. It resolves to whatever drand publishes.
- A released purchase. The bond is what stands behind it.

## Winding a vault down

An orderly end: pause launches (there is no separate switch — pause the listing once nothing is mid-purchase), let the keeper
spend what is attributed through ordinary declared purchases until less than one item's worth is left, settle everything, let the
purchaser withdraw the bond. What remains — under one floor price, plus every coin's `waiting` dust, plus anything force-fed —
**stays in the vault for ever.**

## Why there is no sunset function

One was considered, as the plan asked: owner plus a long public delay, sending what remains somewhere. It was not built, because
every version of it is a withdraw function under another name. "No vault has a withdraw function" is the one promise the whole
design rests on; a sunset path with a 30-day delay is a 30-day-delayed rug for whoever holds the owner key, and the delay protects
only people who are watching. The cases it would help are small (dust) or are exactly the cases where the owner should not be
trusted with an exit (a bug that the owner says requires moving everything).

What stands in its place: the pause, small per-listing exposure (each listing has its own vault), the bond, and an audit before
any vault holds more than the team can afford to see stranded. If a bug makes a vault's ETH unspendable, that ETH is lost to the
bug rather than exposed to a key. **This is the first question in the audit brief**, because it is a judgement, and an auditor or
the owner may reasonably judge it the other way.
