# YOYO, the launch fee that decays, and the creator's fee tier

Written 2026-09-23 on branch `floor-module`, **before their code**, as three amendments to docs/v3/sweep.md (the policies) and
docs/v3/curve.md (the fees). Nothing here is deployed or audited. Contracts touched: `src/SweepVault.sol` (the third policy),
`src/interfaces/IWrappedCollection.sol` and both wrappers (`redeemToRelist`), `src/curve/FrogCurve.sol` (the decaying fee, the
tier), `src/curve/CurveFactory.sol` (the tier at launch). The NFT vaults (`src/nft/*`) get the policy as a spec here, not code.

## 1. YOYO: swept items go back on the market at a markup, and what they fetch buys the coin back and burns it

Modelled on PunkStrategy (a verified contract on Ethereum): every asset the strategy buys is relisted at once at a markup, and a
sale's proceeds buy the strategy's token from its pool and burn it. Here it is a **third per-coin settlement policy** beside
RAFFLE and BURN (docs/v3/sweep.md §"The policy, fixed at launch"): `SweepVault.Policy.YOYO`, one vault per collection for it,
chosen by the creator at launch like the other two and fixed for the coin's life.

### 1.1 The mechanism, on chain

A YOYO vault sweeps exactly as the others do (`sweep`, `sweepViaDesk`: fees in ETH buy wX at or under the keeper's cap, charged
to the coin that paid). What differs is settlement, per whole unit a coin holds:

1. **`relist(coin)`** (anyone; the cap must be fresh): the vault burns one whole wX through the wrapper's new
   **`redeemToRelist()`** — a redemption like any other, the random draw included, whose delivery destination is **the custody
   address itself** (a frog stays where it is; an NFT stays with the canister) — and records the listing:
   `ask = cap × (1 + markupBps)`, the cap being the keeper's fresh all-in floor at that moment. The unit leaves the coin's books
   (`units[coin] −= 1e18`); the ticket is charged to the coin (`retiredBy`, as BURN does), so a refunded ticket (nobody delivered
   in seven days) comes back to the coin through `reclaim`.
2. The custodian resolves the ticket as any other and **"delivers" by listing**: `recordDelivery(id, hash)` with the listing's
   hash (§1.3). The item is out of the wrapped set (`held − 1`) and on the market at the ask.
3. **`recordSale(ticketId)`** (the custodian, payable): the sale's proceeds in ETH, at least `ask × (1 − SALE_FEE_ALLOWANCE)`
   (5%: the marketplace's fee and the ETH conversion), into `proceeds[coin]`. The listing is `Sold`.
4. **`buyback(coin, spend, swapData, minOut)`** (the keeper; measured, not trusted, exactly as `sweep`): spends up to
   `proceeds[coin]` at the vault's venue (the router: `buy(coin, minOut, vault, deadline)`), checks the coins received against
   `minOut`, and sends every coin received to `0x…dEaD`. The ETH never has another exit: there is no withdraw, no transfer, no
   other recipient, and the venue is fixed at construction. Why the keeper and not anyone: a buyback has no cap to bound it
   the way a sweep's `spent ≤ cap × bought` does, so its `minOut` must be chosen by someone who reads the pool; the ETH is not
   the keeper's either way.
5. **Unsold** (the custodian delists): the item comes back through the deposit path to the vault — `wrapDeposit(…, to = vault)`
   / `mintDeposit(…, to = vault)` — and **`reclaimListing(ticketId)`** (the custodian) credits the unit back to its coin once the
   vault sees a whole unit nobody is charged with (its `stray()`). A Bitcoin re-deposit needs a fresh outpoint (one mint per
   outpoint): the canister moves the inscription to itself first.

The markup: `markupBps`, **1,000 by default (1.1×)**, set by the registry's owner (the timelock) within `[MIN_MARKUP_BPS,
MAX_MARKUP_BPS] = [1,000, 2,000]` (1.1×–1.2×), constants in the bytecode. It applies to listings made after the change.

### 1.2 Invariants (`test/YoyoVault.t.sol`, `test/Yoyo.invariant.t.sol`)

- **No exit but the burn**: `address(vault).balance ≥ totalCredit + atDesk + totalProceeds` at every step; `totalProceeds` falls
  only by `buyback`, whose every coin received goes to `DEAD` (the vault's coin balance is zero after it, and `DEAD`'s grew by
  what was measured). The vault's ABI has no function naming a recipient (`test_theYoyoVaultExposesExactlyWhatItMeansTo`).
- **A listing is charged to one coin, once**: `relist` → `Listed` → `Sold` or `Returned` (or refunded → `reclaim`), never
  twice; `listedOpen` counts `Listed` tickets; `units[coin] + listedOpen(coin) + reserved + retired == what the coin ever bought`.
- **The ask is at least the floor plus the minimum markup**: `ask ≥ cap × 1.10` at listing; a sale is refused under
  `ask × 0.95`; `markupBps` never leaves its bounds.
- RAFFLE and BURN are untouched: `relist`, `recordSale`, `buyback`, `reclaimListing` revert `WrongPolicy` on them.

### 1.3 For the canister agent (spec, not built here)

- **Bitcoin inscriptions**: a ticket whose `destination == custodyAddress` is a **relist**. Resolve it as any other (the drawn
  frog is the one to list); then create a marketplace listing for exactly that inscription at `SweepVault.listing(ticketId).ask`
  wei-equivalent (the keeper's BTC/ETH rate at listing time; the spec leaves the marketplace to the operator — a PSBT listing on
  Magic Eden's or a similar venue's API, signed by the custody key, is the model) and call `recordDelivery(ticketId,
  keccak256(listing id))`. On a sale: the sats arrive at custody; convert to ETH on 4663 through the existing float and call
  `SweepVault.recordSale{value: proceeds}(ticketId)` with at least `ask × 0.95`. On a delist (the operator's policy — e.g. after
  30 days, or when the floor has moved 20% away): cancel the listing, move the inscription to a fresh outpoint at custody,
  `wrapDeposit(…, to = vault)`, `reclaimListing(ticketId)`. `held` already excludes the item while listed: a relist ticket's
  delivery counts like any delivery.
- **EVM NFTs**: `redeemToRelist()` on `NftWrapper` is a ticket to `custodian` (the canister's EVM address, refused to anyone else
  by `redeem`). Resolve, then sign a Seaport listing (docs/v4/eth-nft-canister.md §6's rules for what the canister signs, with
  the vault's `ask` as the consideration and the canister as offerer) and `recordDelivery(ticketId, orderHash)`. On a fill the
  ETH lands with the canister on that chain; bridge to 4663 and `recordSale`. On a delist: cancel the order,
  `mintDeposit(…, to = vault)`, `reclaimListing`. The NFT vaults (`NftVaultBase`) would gain the same four entries; `INft.Policy`
  a third member. Not built on this branch.

### 1.4 What this does NOT protect against

- **The sale price is the custodian's word**, bounded below by the ask less the allowance — the same trust as every cost it
  reports (`recordItem`, `itemArrived`). A custodian under-reporting keeps the difference; over-reporting burns more coin.
- **A listing that never sells** ties up the item until the operator delists; the coin's holders wait. There is no deadline on
  chain (a stale listing is not a loss).
- **The buyback moves the pool**: a large buyback is a large buy; the keeper's `minOut` bounds the damage, not a price oracle.
- **The keeper never calling `buyback`**: the ETH waits in `proceeds[coin]`; it cannot go anywhere else.

## 2. The launch fee that decays: an anti-sniper measure on the curve

Pons's data: 22% of graduations happen within one minute of launch — bots sweep new curves. PunkStrategy opens at 95–99% and
decays to 10%. Here `FrogCurve` gains two terms, in `Config`, fixed per coin:

```
launchFeeBps    the extra fee on a BUY at t = 0 (default 5,000 = 50%; at most 9,000; 0 disables it)
launchFeeDecay  how long it takes to reach zero (default 10 minutes; at least 1 minute)

extra(t) = launchFeeBps × (launchFeeDecay − min(t, launchFeeDecay)) / launchFeeDecay        t on the SLOWER clock since launch
fee(t)   = gross × (feeBps + extra(t)) / 10,000
```

- Linear decay, from the coin's launch (both clocks are stamped in the constructor). `t` is the smaller of the two clocks'
  elapsed time (`lib/Wait.sol`'s rule): a sequencer stamping blocks ahead cannot shorten the window.
- **The extra goes to the coin's sweep vault, entirely** — not to the creator, not to the platform, not to the reserve or the
  liquidity slices, and not to us. It is `feesForVault` like the vault's ordinary share; `flushVaultFees` moves it. A sniper
  therefore pays the holders (a raffle, a burn, or a relist — whichever the coin's policy is), and the coordinator's line holds:
  we take nothing from it.
- Sells are not affected (the measure is against buying early, not against leaving).
- The last buy that reaches the raise is grossed up at the total rate of that moment, as today.
- The curve's `feeBps` (the LP fee of the coin's pool, `coinKey`) is unchanged: the decay lives on the curve only.

Tests (`test/LaunchFee.t.sol`): a buy at `t = 0` pays `feeBps + launchFeeBps`; at half the decay half the extra; at and after
`launchFeeDecay` the base fee alone; the extra reaches the vault only; the sequencer's clock alone does not shorten the window;
the raise is still reached exactly with the fee grossed up.

## 3. The creator's fee tier, fixed at launch and public

A creator chooses the coin's trade fee at launch, between `minFeeBps` and `maxFeeBps` (factory `Targets`, **100 and 800**: 1% to
8%), **default 1%** (the four-argument `launch` keeps the template's fee; the five-argument `launch(name, symbol, policy,
sweepOnly, feeBps)` takes the tier). The fee is the curve's `feeBps`, **immutable**; there is no setter, on the curve or the
factory (`test_theFeeTierIsImmutable`: the ABI has none; the launch event and `CurveFactory.feeOf(coin)` publish it).

The split scales with it: the shares are in bps *of the fee*, so a 5% coin sends 2.5% of every trade to sweeps at the worked
50 / 30 / 20 split (2% at the 40 / 30 / 5 / 5 / 20 split of docs/v5/floor-module.md §13.2), 1.5% to the creator, 1% to the
platform. The coin pool's LP fee follows the tier (`feeBps × 100` pips: 80,000 at 8%, within Uniswap v4's maximum).

**ETH volume needed per item**, at the vault's share of the fee, `item / (fee × vaultShare)`:

| | fee 1% | 3% | 5% | 8% |
|---|---:|---:|---:|---:|
| frogs, 0.158 ETH an item, vault share 50% | 31.6 ETH | 10.5 ETH | 6.3 ETH | 4.0 ETH |
| frogs, vault share 40% (§13.2 split) | 39.5 ETH | 13.2 ETH | 7.9 ETH | 4.9 ETH |
| apes, 6.85 ETH an item, vault share 50% | 1,370 ETH | 457 ETH | 274 ETH | 171 ETH |
| apes, vault share 40% | 1,713 ETH | 571 ETH | 343 ETH | 214 ETH |

(The keeper's cap adds its conversion allowance on top; a sweep pays at most the cap.) A creator choosing 8% promises a frog
every 4 ETH of volume and a pool that charges 8% a side: the tier is a public trade-off, shown on the coin's page, not ours to
tune.
