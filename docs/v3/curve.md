# Our own launch curve: the raise buys items

Written 2026-09-23 on branch `wrap-sweep`, after the design pivot that replaced the Pons-approval rollout in docs/v3/sweep.md;
**revised the same day on branch `v4-pools`** for the buy queue, ETH targets and the pools (docs/v3/pools.md has the pools,
the v4 addresses measured on 4663 and the routing). Nothing here is deployed or audited. Contracts:
`src/curve/{CurveFactory,FrogCurve,CurveToken,ItemQueue,MintDesk,CollectionDirectory}.sol`, `src/pools/{LiquidityLock,
EtchRouter}.sol`, with `SweepVault` and `Wrapper.wrapDeposit` from docs/v3/sweep.md. Chain-agnostic: no contract names a
chain; the parent chain's block time is deployment config (`lib/Wait.sol`). Generic over the wrapped item: wFROG (Bitcoin
inscriptions, `Wrapper`) or an ERC-721 on another chain (`NftWrapper`, e.g. wAPE), through `IWrappedCollection`.

## The shape

```
            buy / sell (1% fee)                         the collection's buy queue, FIFO:            coin pool COIN/wX (1% LP fee)
 Trading ─────────────────────────▶ Graduating ───────▶ the canister buys one item at a time, ──────▶ + the collection's shared
    │        the raise is reached      │  item budget     each unit split pro rata to the ETH           wX/ETH pool, both locked
    │                                  │  queued          that paid for it                              forever: Graduated
    │                                  │
    │                                  └── the window passes (both clocks) unfilled ───────────▶ Refunding
    │                                                                                            coin in → pro-rata ETH + units out
    └── fees: 0.5% sweep vault (raffle or burn) · 0.3% creator · 0.2% platform
```

A creator launches ALIEN on a collection with two choices, fixed forever: the sweep policy (RAFFLE or BURN), and whether the
coin buys items at all (SWEEP_ONLY does not). The **target is the factory's**, in ETH, fixed in the curve at launch and public.

## Worked example: frogs, 5 ETH (the test suite runs these numbers)

**Assumptions, marked as such:** a 0.02 BTC frog floor at ETH/BTC 0.0316 = 0.633 ETH; with ≈2% conversion and Bitcoin fees the
keeper's all-in cap is 0.646 ETH; the canister's purchases in the example cost 0.64 ETH each.

**Terms.** Supply 1,000,000,000 ALIEN; 200,000,000 reserved for the coin's own pool; 800,000,000 sold on the curve. Raise 5 ETH
(after trade fees).

| part | rule | ETH | goes to |
|---|---|---:|---|
| platform graduation fee | 5% of the raise, **capped at 0.25 ETH** | 0.25 | the platform, on graduation only; never on a failure |
| shared-pool ETH side | 12.6% of the raise | 0.63 | the collection's shared wFROG/ETH pool, paired with units at what they cost |
| item budget | the rest | 4.12 | the collection's buy queue, behind every coin that closed before it |

**The curve, solved so its last price is the coin pool's first.** The coin's own pool gets the units that cost
`itemBudget − poolEth` = 3.49 ETH, so it opens at `P = 3.49 / 200,000,000 = 1.745e-8 ETH` per ALIEN (FDV 17.45 ETH), valued at
what its frogs cost, whatever they cost. The curve is constant-product over virtual reserves `(V, T)` with
`(V0 + E)(T0 − S) = V0·T0` and end price `(V0 + E)/(T0 − S) = P`:

```
B = E·S / (P·S − E)  = 5 × 8e8 / (13.96 − 5)     = 4.464e8 ALIEN   (token reserve at graduation)
A = P·B              = 1.745e-8 × 4.464e8        = 7.790 ETH       (ETH reserve at graduation)
V0 = A − E = 2.790 ETH,  T0 = B + S = 1.246e9 ALIEN
start price V0/T0 = 2.24e-9 ETH (FDV 2.24 ETH)  →  end price A/B = 1.745e-8 ETH (FDV 17.45 ETH)
```

A target is refused unless `P·S > E` (the end price above the average price) and the item budget is more than twice the
pool side. `test_theCurveEndsAtThePoolsOpeningPrice` checks the last curve price against the opening price to 1e-9; a fuzz test
does it for every raise from 0.1 to 500 ETH, SWEEP_ONLY or not. Every rounding on the curve is in the curve's favour.

**Graduation, in ETH.** The queue spends 4.12 ETH at 0.64 a frog: six frogs (3.84), then 0.28 is left, which is under the coin's
**tail** (10% of its 4.12), so the coin is filled — it does not wait for another coin to fund a seventh frog it would need only
a sliver of. Its six units come back to the curve with the 0.28:

- coin pool: 6 × 3.49 / 3.84 = **5.453 wFROG** + 200M ALIEN, full range, 1% LP fee;
- shared pool: **0.547 wFROG + 0.91 ETH** (0.63 pool side + the 0.28 tail); the ETH the bands cannot pair becomes the bid;
- platform 0.25; canister 3.84. 5.00 in, 5.00 out.

## Worked example: frogs, 8 ETH (an early coin)

While the collection's shared pool has received less than 2 ETH of pool side, a coin's target is **8 ETH** (hard maximum 10 in
the constructor). Fee 0.25 (5% would be 0.40: capped), pool side 1.008, item budget 6.742: **ten frogs** at 0.64 (6.40), tail 0.342
(under its 0.674). Coin pool 10 × 5.734 / 6.40 = **8.959 wFROG**; shared pool **1.041 wFROG + 1.35 ETH**. After it the collection
has 1.35 ETH of pool side, still under 2: the second coin is early too; after it, 2.70: the third is normal. In practice: the first
coin, and probably the second, at 8 ETH, then 5 ETH.

## Worked example: a 5 ETH-floor collection (Bored Apes), the same queue

Assumption: an ape at 5.1 ETH all-in. The same curve, the same splits, the same queue; only the item price differs.

- **Opening**: the collection has no shared liquidity, so the first coin's target is at least one item plus the pool side and
  the fee: `(5.1 × 1.05 + 0.25) / (1 − 12.6%) = 6.41 ETH`; the early target (8) is higher, so 8. Its 6.742 buys ape #1 (5.1) by
  itself; 1.642 remains, more than its tail, so it waits, still its own.
- **Coin 2** (8 ETH, early) queues 6.742. Ape #2 (5.1) is funded **FIFO**: coin 1's 1.642 first — coin 1 is now filled with
  1 + 1.642/5.1 = **1.322 wAPE** and graduates — then 3.458 of coin 2's, which gets 0.678 wAPE and carries **3.284 ETH**, still
  credited to it, until coin 3's ETH funds ape #3 or its window ends (then it refunds its ETH and its 0.678 wAPE pro rata).
- **Two normal coins at 5 ETH** (the illustration asked for): 4.12 + 4.12 = 8.24 queued; one ape at 5.1 takes coin A's 4.12 and
  0.98 of coin B's. Coin A gets 4.12/5.1 = **0.808 wAPE** and is filled; coin B gets **0.192 wAPE** and carries **3.14 ETH**. (Not
  0.5 and 0.5: the split is pro rata to the ETH that paid, and FIFO decides whose ETH pays first, so an earlier coin is finished
  sooner and no coin is starved by later ones. `test_theApeExample` runs exactly this.)
- Its pools: coin A's pool gets 0.808 × 3.49 / 4.12 = 0.684 wAPE with its 200M coins; the shared wAPE/ETH pool gets 0.124 wAPE +
  0.63 ETH. **In ETH, the same depth per graduation as frogs**: the pool side is 12.6% of the raise, not one item's value.

## Graduation state machine (`FrogCurve`)

- **Trading.** `buy(minOut)` / `sell(amount, minEth)`. The buy that reaches the raise is filled to it and the change returned
  in the same transaction; the curve closes, the item budget is queued (`ItemQueue.enqueue`, with the curve's deadline on both
  clocks), and the window starts.
- **Graduating.** The canister buys items for the **collection's queue**, not for a curve (docs/v3/pools.md §5). `graduate()`
  (anyone) once the queue says this coin is **filled**: the queue releases its units and tail to the curve; the coin pool gets the
  units that cost `itemBudget − poolEth`, the shared pool the rest with `poolEth + tail`; the curve **checks the lock took all of
  it** or reverts; the platform's capped fee is credited. SWEEP_ONLY: `graduate()` at once, COIN/ETH with the raise less the fee.
- **Refunding.** `openRefunds()` (anyone, window passed on both clocks, not filled; or the queue's `refundExpired()`): the queue
  returns this coin's unspent ETH and the units already assigned to it; `refund(amount)` pays `amount / S` of the ETH and of the
  units. Units already bought are handed out, not sold back (a Bitcoin sale at a price the canister picks, another conversion loss,
  hours of latency; a unit redeems one-for-one for a random item on the existing drand path).
- **What the platform gets on a failure: its trade fees only.**

## Public mint on demand (`MintDesk`)

Anyone escrows ETH for one frog (`request()`, the amount is their maximum: the page quotes floor + ≈2% conversion + fees +
a buffer). The canister buys a floor frog **with its own float**, confirms it into custody, `wrapDeposit`s it to the desk,
and calls `fulfill(id, cost)`: in one step it is paid `cost ≤ amount`, the requester's wFROG and change become collectable.
A request unfilled when either clock passes `ttl` is collected back in full, and cannot be filled after that.

Fronting from a float is deliberate: the requester's ETH never leaves before its wFROG exists, so a failed purchase costs the
requester nothing but time. The canister's float turns over once per fill (it bridges the ETH back to BTC).

**Supply is elastic, and arbitrage holds wFROG near the floor:** above floor + costs, mint (or deposit) and sell; below the
floor, buy and redeem (the existing drand path). The two sweep routes and every coin's fees feed the same settlement.

### The deposit route is now the arbitrage route

A holder who owns frogs sends one to custody with an `OP_RETURN` naming their EVM address; the canister attests it after six
confirmations; `Wrapper.wrapDeposit` mints one wFROG (docs/v3/sweep.md, 1a, for the full canister checks). Its attack surface:

- **Front-running a deposit's claim:** none. The recipient is in the Bitcoin transaction the frog's owner signed
  (`SIGHASH_ALL`); there is no EVM claim to race.
- **A frog carrying runes or a second inscription:** refused by the canister (two sources: ord + an explorer) and returned;
  the contract cannot see sats. Minting it would make one wFROG worth more than one frog.
- **Double-mint on a reorg:** an outpoint mints once, ever (contract). A reorg that drops a minted deposit leaves one
  unbacked wFROG; bounded by the six-confirmation depth.
- **Double-mint of a frog already held:** the wrapper refuses an inscription it already holds (contract); the canister
  refuses a deposit that spends a custody UTXO.
- **A lying custodian:** mints against nothing. The same key already holds every frog; deposits change what a stolen key can
  do quietly, so the canister rate-limits deposits and the monitor reconciles `held` against the custody UTXO set daily.

## Fees: one settlement, two currencies

1% of every trade, parametrized (`feeBps`, `vaultShareBps`, `creatorShareBps`; the platform gets the rest): 0.5% sweep vault ·
0.3% creator · 0.2% platform.

- **On the curve** fees are ETH. `flushVaultFees()` deposits the vault's share with `SweepVault.deposit(coin)` (the factory names
  the curve as the coin's router). The vault sweeps it into items through the pool (`sweep`) or the canister (`sweepViaDesk`).
- **The coin's own pool** has no hook: its 1% is the pool's LP fee, earned by the lock's full-range position (and by anyone else
  who adds liquidity). `LiquidityLock.collectCoinFees(coin)` (anyone) hands it to the curve, which splits it the same way: the
  wX side 50 / 30 / 20 (the vault's share by `depositWrapped`), the coin side 30% creator, 20% platform, and the vault's 50%
  **burned** (the vault holds only wX and ETH; swapping coins for wX without an oracle is a trade anyone can sandwich; burning
  returns the value to every holder pro rata). Uniswap's protocol fee on 4663 (0.1% on a 1% pool) is charged on top.
- **The shared pool's fees are seeder shares**: each coin earns, forever, the fees its own liquidity units earn in every shared
  position, paid by `claimShared(coin)` (anyone) to the curve and from there **all of it** to the coin's sweep vault.

All of it lands as `credit[coin]` / `units[coin]` in the coin's policy vault. The raffle and the burn never see the route.

## Liquidity: protocol-owned, never withdrawable

Built (docs/v3/pools.md). The lock owns its positions directly in the PoolManager: no NFT, no owner, and no code path with a
negative liquidity delta. Each coin: one full-range COIN/wX position. Each collection: one shared wX/ETH pool (0.30%, hookless)
with **layers** around a center — 50% of each contribution offered to ±10%, 30% to ±30%, the rest full range, and the ETH the
bands cannot pair as a bid 10–25% under the floor. The center follows the keeper's cap one bounded step (≤ 960 ticks) a day;
positions already added never move. An add waits unless the pool price is within 10% of the fresh cap.

## Risks, plainly

- **About 2% is lost converting ETH to items** (Relay ETH→BTC plus marketplace and Bitcoin fees). A 5 ETH raise buys ≈3.80 ETH
  of frogs at floor.
- **Holders are exposed to the item price.** ALIEN trades against wFROG; if the floor halves, ALIEN in ETH halves at an
  unchanged ALIEN/wFROG price. That is the product, not a side effect.
- **Sweeping raises the floor.** Every graduation and every fee sweep buys items. In a thin collection the canister pays up the
  book; items above the keeper's cap are not bought, and a coin that cannot be filled in its window refunds.
- **The arbitrage band, honestly.** Minting costs the floor + ≈2% conversion + the marketplace fee (0–2.5%) + Bitcoin fees and
  takes about an hour (purchase, six confirmations, mint); redeeming burns one unit for a **random** item delivered within hours.
  So wX should trade within roughly **−3% … +5% of the floor** in calm markets, with **hours of lag** after a big move of the floor
  or of ETH/BTC, during which only the pool's depth absorbs flow. Nothing enforces the band; arbitrageurs with inventory on both
  chains do.
- **Depth.** The old per-coin seed (1 wFROG + 0.63 ETH, full range) moved wFROG/ETH by **34%** on a 0.1 ETH buy. The shared,
  layered pool (layout as built; pool price at the keeper's cap; 0.30% fee; price impact = the move of the marginal price;
  schedule: two early coins at 8 ETH, then 5 ETH coins):

  | frogs (0.646 ETH all-in) | shared pool holds | buy wFROG with 0.1 / 1 / 5 ETH | sell wFROG for 0.1 / 1 / 5 ETH |
  |---|---|---|---|
  | after 1 graduation | 1.12 wFROG + 1.29 ETH | +2.1% / exhausts the bands (+1,177%) / exhausts | −2.1% / −21% / exhausts |
  | after 5 graduations | 4.02 wFROG + 5.22 ETH | +0.6% / +5.9% / exhausts (+3,838%) | −0.6% / −5.7% / −82% |
  | after 20 graduations | 12.9 wFROG + 18.4 ETH | **+0.2% / +1.8% / +9.2%** | −0.2% / −1.8% / −8.8% |

  | 5 ETH-floor collection (5.1 all-in) | shared pool holds | buy wAPE with 0.1 / 1 / 5 ETH | sell wAPE for 0.1 / 1 / 5 ETH |
  |---|---|---|---|
  | after 1 graduation | 0.198 wAPE + 1.01 ETH | +1.5% / +160% / exhausts | −1.5% / −99.8% / exhausts |
  | after 5 graduations | 0.766 wAPE + 3.91 ETH | +0.4% / +3.9% / +828% | −0.4% / −3.8% / exhausts |
  | after 20 graduations | 2.62 wAPE + 13.4 ETH | +0.1% / +1.1% / +5.7% | −0.1% / −1.1% / −5.6% |

  **Expensive items do not make the shared pool deeper per graduation in this design**: its pool side is 12.6% of the raise in
  ETH whatever the item costs, so a frog coin and an ape coin add the same ETH depth (the ape table is slightly deeper because
  its contribution is balanced and none of it goes to the bid). The "1 wAPE + 5 ETH, a 0.1 ETH buy moves ~4%" intuition is the
  old one-whole-item-per-graduation seed: full range, 1 item + its value in ETH moves `(1 + 0.1/5)² − 1 = 4.0%` for apes against
  34% for frogs. What does differ is the coin's own pool on the route: after 5 graduations a 0.1 ETH buy of ALIEN routed
  ETH → wFROG → ALIEN moves wFROG 0.6% and ALIEN/wFROG 5.7% (5.4 wFROG full range in the coin's pool), **ALIEN +6.3% in ETH**; a
  1 ETH buy, +72%. The coin pool, not the shared one, is the binding depth for a single coin; the shared pool is what lets every
  coin route through wFROG at all (docs/v3/pools.md §7).
- **Uniswap's protocol fee** is on for these pools (fork-tested: 0.1% on the 1% coin pool, 0.05% on the 0.3% shared pool).

## Canister: graduation buys, the mint desk and holder deposits (2026-09-23, before the code)

Branch `canister-curve` (from `40735bf`). What the canister does for the three new duties, decided here before the code.
Everything below is new modules; the shared files (`canister.rs`, `lib.rs`, `custodynet.rs`, `redeem.rs`, the `.did`) get
wiring only, listed under "Shared-file edits".

### Modules

| module | pure? | what it decides |
|---|---|---|
| `frogbuy.rs` | yes | whether a marketplace PSBT for one inscription may be signed (sat flow, caps, fee ceiling, policy limits); sats ↔ wei at the recorded rate; confirmation depth; the collection's Merkle proof; the frog's record |
| `itemsink.rs` | yes | for any contract that takes items and reimburses per item: start a purchase, mint, report, drive its refund or settlement trigger, or wait; the per-item cap; both clocks. Knows nothing about what an item is |
| `buyqueue.rs` | yes | the adapter for a collection's **buy queue** (the design since 2026-09-23; its interface is ASSUMED, below) |
| `curvebuy.rs` | yes | the adapter for a graduating `FrogCurve`, as merged at `40735bf` |
| `mintdesk.rs` | yes | per request: fill from inventory, buy, mint, fulfill, or leave; never above the request's `amount` |
| `deposits.rs` | yes | the `EWD1` `OP_RETURN`, the six checks of sweep.md 1a, the rate limit, and the return of a refused frog |
| `frogflow.rs` | no network | the sequencer over a `FrogIo` trait: one tick, one EVM transaction of ours at a time; tested against a fake world (`tests/frogs.rs`) |
| `frognet.rs` | — | the real `FrogIo` (pinned reads, ord, explorers, the keys) and the canister endpoints |

### The buy queue (the coordinator's change, 2026-09-23), and the interface the canister assumes

Graduation no longer means "buy N frogs for this curve". Coins graduate at an ordinary ETH target and their item budget is
credited, first in first out, to their **collection's buy queue**; the canister buys **one** item whenever the queue holds at
least the item cap plus the buffer, confirms it into custody, mints it to the queue, and reports it; the contract splits the
unit pro rata to the credits that paid for it. So the canister's duty is per collection, not per coin, and the rules are
written for any **item sink** (`itemsink.rs`): a contract that says how many items it can pay for now, how much it holds,
its per-item maximum, a deadline, and whether its refund or settlement trigger is due. A queue and a curve are two adapters
onto the same rules; an EVM NFT collection bought through Seaport (docs/v4/eth-nft-canister.md) can be a third, with its own
acquisition in place of the Bitcoin purchase.

**The queue contract was not on this branch.** The canister is written against this interface, all of it in `buyqueue.rs`,
so the real one replaces one file:

```solidity
interface IBuyQueue {                                        // ASSUMED by the canister
    function wrapper() external view returns (address);      // the Wrapper whose custodian is the canister
    function payee() external view returns (address);        // where recordItem pays: the canister's EVM address
    function queued() external view returns (uint256);       // wei credited and not yet spent or refunded
    function itemCap() external view returns (uint256);      // the most one item may cost now, wei; 0 = none posted
    function buffer() external view returns (uint256);       // held beyond the cap before an item is bought, wei
    function itemsRecorded() external view returns (uint256);
    function headDeadline() external view returns (uint64 at, uint64 atBlock); // the oldest credit's; (0, 0) when empty
    function recordItem(uint256 cost) external;              // custodian only, after wrapDeposit(…, to = queue):
                                                             // balance ≥ (itemsRecorded + 1) units, cost ≤ itemCap,
                                                             // cost ≤ queued; pays cost to payee; splits the unit FIFO
    function refundExpired() external;                       // anyone, once the head deadline is past on both clocks
}
```

How the canister reads it: open once a cap is posted; it wants `⌊queued / (itemCap + buffer)⌋` items, one at a time; the
per-item cap is `itemCap`; the **oldest credit's deadline is the deadline** (a credit refunded while an item is being bought
takes the money the item was bought against), so purchases stop `PURCHASE_LATENCY + MINT_MARGIN + CAP_MARGIN` before it and
mints stop `MINT_MARGIN` before it; once it is past on both clocks the canister calls `refundExpired()` (anyone may; the
canister drives it once per recorded count). Queues are configured by a controller (`set_item_config`), never discovered.

### The queue as built (branch `v4-pools`), against that interface

`src/curve/ItemQueue.sol` implements every function above with the same signatures (the selectors in `buyqueue.rs` match). The
differences, each on purpose:

| | assumed | built | why |
|---|---|---|---|
| `queued()` | wei credited, not spent or refunded | wei of coins still **queued**; a filled coin's tail is held for its curve, not counted | a tail cannot pay for anyone's item; the canister's `⌊queued / (itemCap + buffer)⌋` stays right |
| `itemCap()` | the most one item may cost; 0 = none posted | the freshest of the collection's two vault caps (the keeper's, 1 h TTL on both clocks); **0 when stale** | the same cap the vaults' sweeps obey |
| `buffer()` | held beyond the cap, wei | `bufferBps` (5%) × `itemCap()` | |
| `recordItem` balance check | `balance ≥ (itemsRecorded + 1) × 1e18` | `balance ≥ unitsHeld + 1e18` | units leave on release and on refunds; the assumed check would refuse every record after the first coin graduates |
| `recordItem` cost bound | `cost ≤ itemCap` | `cost ≤ max(itemCap, a snapshot taken within 2 h)` | gap (1) below |
| `refundExpired()` | anyone, head deadline past | the same; it calls the head coin's `openRefunds()`, which cancels it here | the refund pool lives in the curve |
| a coin's completion | budget fully converted | remaining ≤ **max(0.001 ETH, 10% of its deposit)** (its tail, released to its shared-pool ETH side) | a lone coin would otherwise wait for another coin to fund a sliver of an item |

**The four gaps the canister found, closed:**

1. **A cap drop between the mint and `recordItem`**: `snapshotCap()` (anyone; the canister calls it before starting a purchase)
   stores the fresh cap; for 2 hours on both clocks `recordItem` may pay up to it even if the keeper lowered the cap since. A
   unit minted outside that window can still be recorded at the new cap, or reclaimed (below). The contract cannot see when a
   unit was minted, so the binding is to the snapshot, not the mint.
2. **Discovery**: `CollectionDirectory.queues()` lists every collection's queue in one read (`at(i)` gives the wrapper, factory,
   queue and lock); append-only, `add` by the admin, and it checks the four contracts name each other.
3. **`MintDesk.reclaimStray()`** (custodian): takes `balance − assigned`, i.e. a unit of ours minted for a request that expired
   before `fulfill`. `ItemQueue.reclaimStray()` does the same for `balance − unitsHeld`.
4. **A refund landing between the canister's mint and its report**: the queue, not the curve, receives every minted unit, and a
   coin's refund takes only its own remaining ETH and the units already assigned to it; the new unit stays unassigned in the queue,
   recordable for the next coins or reclaimable. `recordFrog` no longer exists: `curvebuy.rs` ("the curve as merged") drives a
   contract that is gone.

What the canister should change: call `snapshotCap()` before each purchase; read queues from the directory instead of
`set_item_config`; drop `curvebuy.rs`; use `reclaimStray()` for strays at the desk and the queue.

### Keys: the float is its own key

Graduation buys and desk fills are **fronted from a BTC float** and reimbursed in ETH afterwards (`recordFrog`, `fulfill`). The
float cannot be the vault buyer's address (`b"btc"`): `state.rs` reads "funded" from that address's balance (`Pulled` with
`btc_sats > 0` is `Funded`), so a standing float there would be spent by the next vault purchase as if it had been bridged.
The float is therefore a third derivation path, **`b"float"`**, beside `b"btc"` (vault purchases) and `b"custody"` (frogs):

- a frog buy signs **only** inputs paying the float's script, with the float key; it never signs a custody input (a PSBT that
  lists one is refused) and never a vault-buyer input (they do not carry the float's script);
- the frog lands on the **custody** script and is only ever moved by custody's own paths (a redemption delivery, or the
  return of a refused deposit);
- `float_address()` prints the address. **Refilling the float is not automated**: reimbursements land as ETH at the
  canister's EVM address (the curve's and the desk's `payee`), and turning them back into BTC is Relay (`relayquote.rs`),
  which is still not configured as a bridge (accepting Relay's residual trust is the owner's decision). Until it is, the
  operator tops the float up; one frog's worth is enough (spec: "Float needed: one frog").

### Buying one frog (`frogbuy.rs`)

A buy is for a **target** (an item sink — a collection's queue or a graduating curve — or a desk request) or none
(inventory). **One Bitcoin purchase is in flight at a
time**, across curves and the desk: the float is one balance. The offer comes the way the vault buyer's venue does
(`relay.rs`): no marketplace is keyless and reachable from the canister, so the operator (the relayer) hands in, for a frog
record the canister opened, `frog_offer(id, inscription_id, psbt, to_sign, proof)`. The relayer's power is bounded by what the
canister checks; it can waste time, not money. Before anything is signed:

1. **The inscription is in the collection**: the Merkle proof verifies against the Wrapper's listing root read at the pin
   (`leafOf = keccak256(keccak256(id))`, OpenZeppelin's sorted-pair tree, as `ListingRegistry.isMember`). A frog is never
   bought that `wrapDeposit` would refuse.
2. **Where it is**: ord's `/r/inscription/<id>` gives the seller's outpoint and the inscribed sat's offset; ord's
   `/r/utxo/<outpoint>` must show **exactly that inscription and no runes**; every explorer must agree the output exists,
   confirmed, unspent, at ord's value.
3. **The PSBT**, judged against those facts, never against the relayer's word:
   - every input asked to be signed pays the float's script, and no other float input is left unsigned-for; no input pays the
     custody script;
   - exactly one input spends the seller's outpoint, with ord's value;
   - **sat flow** (ordinal theory, first-in-first-out): the inscribed sat's absolute position (the values of the inputs before
     the seller's, plus the offset) falls in an output paying **the custody script**, exactly one output pays custody, and
     every sat of that output comes from the seller's input or from our own (ord-clean) inputs — so the custody output carries
     exactly the one inscription;
   - no `OP_RETURN` output at all (a runestone could mint or move runes onto the frog's output);
   - every foreign input's final witness is a key-path signature or a P2WPKH pair (no script path, so no inscription
     envelope can be revealed by the purchase);
   - fee ≥ 0; what leaves the float (`spend` = our inputs − our change) ≤ the target's cap in sats;
   - **the policy limits of `policy.rs`, applied to every buy**: the ETH cost is within `[MIN_PURCHASE_WEI, MAX_PURCHASE_WEI]`
     (0.05–0.5 ETH), and the miner fee plus the conversion allowance is within `MAX_COST_BPS` (5%) of the spend. The EVM
     transactions use the loop's own fee ceiling (`FEE_MULTIPLE` × the pinned base fee) and gas margin.
4. Our inputs are ord-clean (`custodynet::buyer_input_clean`, the vault buyer's belt and braces).

**Cost, fixed when the offer is accepted**: `cost_wei = ⌈spend_sats × 1e18 / rate⌉ × (1 + CONVERSION_BPS)`, where `rate` is the
reference rate (`oracle::read_rate`, the median of five tickers and XRC) read at that moment and `CONVERSION_BPS = 200` is the
doc's ≈2% for turning the reimbursement back into BTC. A queue's bound is its `itemCap` and what it holds; a curve's is the
total (`frogSpent + cost ≤ frogBudget + buffer`); the desk's is the request's `amount`. "The rate the bridge gave" is not available when the frog is fronted from the
float, so the reference rate plus the stated allowance is used; it is recorded with the frog, and nothing re-prices it later.

Then: sign each of our inputs with the float key, **save the signed bytes, then broadcast** (as the vault buyer), and believe
only Bitcoin. **Six confirmations**: the transaction's block must be at least six deep on every explorer (the minimum tip), and
the block at its height must still be the block the explorers named (a reorg check); then ord must show the custody output
carrying exactly the inscription, no runes, and the explorers agree it is unspent at the custody script. Only then is the
frog **held**. A reorg that drops it below six sends it back to waiting (the same saved bytes are re-sent); the depth is
re-read immediately before any `wrapDeposit`.

**Listing vanished**: if our transaction is not in a block and the seller's outpoint is spent, confirmed, by a different
transaction, ours can never confirm: the record fails (nothing of ours was spent) and the target gets a new record.
**No offer within the cap** (the floor moved past the buffer): the record waits for an offer it can accept until the target no
longer needs it (the curve's window closes, the request expires), then fails having spent nothing.

**Inventory**: a held frog is not minted until a target can take it safely. A frog whose target went away (the request
expired, the curve's window closed, the curve refunds) stays unminted in custody and is offered to the next target at its
recorded cost (a desk request only if `cost ≤ amount`; a curve only if within its per-frog cap). Minting later is always
possible: `wrapDeposit`'s outpoint has never minted.

### Item sinks: a collection's queue, or a graduating curve (`itemsink.rs`, `buyqueue.rs`, `curvebuy.rs`)

- **Which sinks**: the queues a controller configured, and (for the curve as merged) the factory's `coinCount()`/`coins(i)`/
  `routerOf(coin)`, walked with a cursor. A sink whose `payee()` is not our EVM address or whose `wrapper()` is not ours (or,
  for a curve, whose `factory()` is not the configured one) is **never served**: we would be reimbursed nowhere.
- **Per tick, from one pinned read** of each sink (bounded per tick, round robin, sinks with our frogs on the way first) and
  the pinned block's two clocks, `itemsink.rs` decides:
  - a held frog for this sink is minted (`wrapDeposit(…, to = sink)`) only while **both clocks have more than `MINT_MARGIN`
    (1 hour) left** before the sink's deadline — the single-clock rule of `delivery.rs`, the other way round: the nearer
    clock binds — and only if its cost is within what the sink holds and its per-item maximum. One unreported unit per sink
    at a time;
  - a minted frog is reported at once (`recordItem(cost)` / `recordFrog(cost)`), whatever the clocks say: the unit is already
    there, and the reimbursement is ours;
  - a **new purchase is started** only while the nearer clock has more than `PURCHASE_LATENCY` (3 hours: a purchase plus six
    confirmations, generously) plus `MINT_MARGIN` plus `policy::CAP_MARGIN_SECS` left, one frog at a time across everything,
    with the cap `min(per-item maximum, (what it holds − Σ costs of ours on the way) / (items it wants − ours on the way))`.
    An inventory frog within that cap is used first;
  - the sink's **refund trigger** (`refundExpired` / `openRefunds`) is driven when its deadline is past on **both** clocks,
    and its **settlement** (`graduate`, a curve's only) when it has every item. Frogs of ours not yet minted return to
    inventory; one minted and not reported when a curve opens refunds is **stranded** (its unit is the refunders', its cost
    lost to the float), which the mint margin exists to prevent short of a sequencer or parent-chain stall.
- **The curve as merged** reads as a sink that is open while `Graduating`, wants `N − k`, holds `frogBudget + buffer −
  frogSpent`, has its window as the deadline, and refunds once both clocks pass it short of N.
- **Progress** is readable without a call: `item_sinks()` and `item_progress(sink)` (queries) return, as of the last tick's
  pinned read, items recorded (and N for a curve: "k of N"), what the sink holds, the next item's cap, the deadline on both
  clocks, the sink's own state word, and our frogs on the way.

### The public mint desk (`mintdesk.rs`)

- Requests are walked with a cursor over `requestCount()`; open ones are kept until filled, collected or expired.
- A request is taken only if its `amount` is at least `MIN_PURCHASE_WEI` and the time to expiry — **the nearer of the two
  clocks**, since the desk expires on either — exceeds `MINT_MARGIN` (inventory frog) or `PURCHASE_LATENCY + MINT_MARGIN`
  (a purchase). The purchase's cap is the request's `amount` (converted at the rate, less the allowance); **the fill is never
  above `amount`**: `cost > amount` is refused before signing and before `fulfill`.
- Immediately before `wrapDeposit(…, to = desk)` and again before `fulfill(id, cost)`: a fresh pin, the request still `Open`,
  `isExpired(id)` false and the margin still there. If it expired between the mint and `fulfill`, the wFROG sits at the desk
  unassigned; it is recorded as a **stray unit of ours** and the next request is filled from it with `fulfill` alone (the desk
  counts any unassigned unit), at the stranded frog's cost if within that request's `amount`.
- Sinks come before the desk when both want the float: a sink's deadline is shared by every coin that funded it.

### Holder deposits (`deposits.rs`)

**Which outputs are deposits.** Custody receives more than deposits: frogs bought for the vault (`wrap`), our own frog buys,
delivery change, and the operator's plain-sat top-ups. A custody output is a **deposit candidate only if its transaction
carries an `OP_RETURN` whose payload begins `"EWD1"`**. Anything else is never minted and never returned by this code;
inscription-bearing outputs with no marker are recorded as `unmarked` for the daily reconciliation (a holder who forgot the
`OP_RETURN` is made whole by a person, not by a guess).

**The six checks of sweep.md 1a**, each a refusal with its reason:

1. six confirmations, with the reorg check above, re-read immediately before the EVM call; fewer is a wait;
2. no input of the deposit transaction pays the custody script (refused, **not returned**: it is custody's own coin);
3. ord shows exactly one inscription on the output and no runes; ord's location of that inscription is this output; every
   explorer agrees the output is at the custody script, unspent, at ord's value. Runes, a second inscription or a
   reinscription: refused. No inscription and no rune at all: not a frog (plain sats stay as custody's fee float);
4. the inscription is in the collection: a proof handed in by the operator (`frog_proof`) that verifies against the listing
   root; until one does, the deposit waits. The operator may instead **refuse** a waiting deposit as not in the collection
   (`refuse_deposit`), which only ever leads to its return;
5. every input is signed `SIGHASH_ALL` or `DEFAULT`: taproot key path (64 bytes, or 65 ending `0x01`), P2WPKH and
   P2SH-P2WPKH (the signature's last byte `0x01`). An input the canister cannot read that way (legacy, script path, an annex)
   is refused: the binding to the recipient cannot be shown;
6. exactly one `OP_RETURN` in the transaction, a single push of exactly 48 bytes `"EWD1" ‖ chainId (4, BE) ‖ wrapper (20) ‖
   recipient (20)`, naming this chain and this wrapper, recipient not zero. Anything else with the `EWD1` prefix is malformed.

Then the contract's own state at the pin: `depositMinted(keccak256(abi.encode(txid, vout)))` already true is adopted as
minted (a reply we never saw); `inscriptionHeld(leafOf(id))` true is a wait. **Rate limit**: at most 50 deposits minted in any
24 hours (by the canister's clock); over it, deposits wait. Then `wrapDeposit(id, proof, txid, vout, recipient)`, once per
outpoint, one EVM transaction of ours at a time. `txid` is in display order (as an explorer shows it), the same order as
`recordDelivery`, for purchases and deposits alike.

**The return of a refused frog.** A deposit refused for a reason that is the depositor's (malformed marker, runes or extras,
not in the collection, a sighash the canister cannot verify) is sent back, whole, to the script of its transaction's
**input 0** — the depositor's own address — when that script is taproot, using custody's existing delivery builder
(`custody::build`: output 0 takes exactly the refused output's value, so every sat and everything on it goes back; the fee
from a clean custody UTXO; change to custody). Signed with the custody key, the bytes saved before the broadcast, only Bitcoin
believed. A fee input chosen for a return is excluded from redemption deliveries' fee choice and vice versa. **Returns are rate
limited** (`RETURNS_PER_DAY = 10`): each pays a fee from custody's float, so a stream of marked junk cannot drain it; over the
limit a refused frog waits, it is not lost. A refused frog
whose input 0 is not taproot (or is ours) is **held**, recorded, and left to a person: the canister does not invent a
destination. A deposit that spent a custody input is never returned.

### Frogs minted by `wrapDeposit` can be redeemed

Every frog this code mints (curve, desk, deposit) has a Wrapper id `DEPOSIT_FLAG | n` (bit 255). The custodian's ticket reader
refused any id wider than 128 bits, so the first redemption of such a frog would have stopped the custodian's walk for good.
The reader now carries bit 255 through (as bit 127 of its internal id), and the custodian finds the inscription for a
deposit id from the Wrapper's `leafOfFrog(id)` matched against the inscriptions this canister minted.

### One EVM transaction at a time, and restarts

Every EVM call here (`wrapDeposit`, `recordItem`, `recordFrog`, `fulfill`, `refundExpired`, `openRefunds`, `graduate`) is
signed, **saved**, then sent, and counts in the canister-wide one-transaction rule with the vault purchases and the
custodian's `recordDelivery`. The most urgent goes first (a report, then a mint, then a deposit, then a trigger); one whose
estimate fails now does not hold up the others. A mined
revert is resolved by reading the contract (`depositMinted`, `frogsBought`, the request's status), never by trying again
blind; an estimate that fails is a wait. All state (curves, requests, frogs, deposits, proofs, the pending transaction) is a
new seventh element of the upgrade tuple (after the NFT line's sixth), optional, so a state saved before it restores. The
factory and the desk are set once, from nothing, and queues only added, by a controller (`set_item_config`), like the
wrapper: identity, not operation.

### Shared-file edits

`lib.rs` (eight `pub mod` lines); `canister.rs` (the upgrade tuple's **seventh** element, after the NFT line's sixth; the
tick's pass after the custodian, and the one-EVM-transaction rule counting ours both ways; a restore test);
`custodynet.rs` (deposit ids in `inscription_of`; fee inputs our returns have claimed excluded from `our_utxos`);
`redeem.rs` (bit 255 of a ticket's frog id, and its test); `etch_autobuyer.did`.

### Endpoints

Controller: `set_item_config(factory, desk, queues)` (set once each / append only). Operator: `float_address()`,
`frog_offer(frog, inscription, psbt, to_sign, proof)` (the relayer's hand-in), `frog_proof(inscription, proof)`,
`refuse_deposit(outpoint, reason)` (only a deposit waiting for its proof). Queries: `item_sinks()`, `item_progress(sink)`,
`frog_buys()`, `holder_deposits()`, `desk_requests()`, `unmarked_custody()`. Everything else happens in `tick`.

### Constants (code, so changing one passes the timelock)

`CONFIRMATIONS = 6`, `PURCHASE_LATENCY = 3 h`, `MINT_MARGIN = 1 h`, `CONVERSION_BPS = 200`, `DEPOSITS_PER_DAY = 50`,
`RETURNS_PER_DAY = 10`; from
`policy.rs`: `MIN_PURCHASE_WEI`, `MAX_PURCHASE_WEI`, `MAX_COST_BPS`, `CAP_MARGIN_SECS`.

**Stated plainly: at this document's own numbers the policy limit refuses a frog.** A frog in the worked example costs
≈0.6467 ETH and `MAX_PURCHASE_WEI` is 0.5 ETH, so a 0.02 BTC floor frog is over the per-purchase maximum and is not bought.
The limit is applied as instructed; raising it (or giving frog buys their own maximum) is a code change through the timelock.

### Built: what the code settled, and what was shown (2026-09-23, with the code)

- **Commits** on `canister-curve`, rebased onto `8df69d4` (main, with the Seaport buyer of `6eb96e6`; nothing under `autobuyer/` changed after it): the design (three doc commits, each before its
  code), the pure half (`frogbuy`, `itemsink`, `buyqueue`, `curvebuy`, `mintdesk`, `deposits`), the wiring (`frogflow`,
  `frognet`, the shared-file edits above, the `.did`, `tests/frogs.rs`), and `autobuyer/scripts/items-replica-rehearsal.sh`.
- **`cargo test`: 272 passed, 0 failed** (214 lib, 2 e2e, 22 frogs, 20 loop, 3 policy_loop, 11 redeem), up from 214 at
  `6eb96e6`: 36 unit tests in the new modules and one in `canister.rs`, and 22 against the fake world. Every selector and
  every encoding the canister sends is checked against Foundry's `cast` (`wrapDeposit` byte for byte, twice; `requestOf`'s
  struct; `outpoint_key`; `leafOf`), and the assumed queue's selectors against the keccak of the signatures above.
- **The fake world** (`tests/frogs.rs`) runs, for a queue: one item bought, six deep, ord-verified, minted, reported and
  reimbursed at the cost fixed at acceptance; the floor past the buffer (the offer refused, nothing signed, the want failing
  when the oldest credit's deadline comes within a purchase's latency, the refund driven once both clocks are past); the
  listing vanished (failed, nothing spent, a new want); a reorganisation below six before the mint (nothing minted until six
  again); an eviction (the same saved bytes, never a new signature); not the custodian, a queue that pays someone else, and
  another EVM transaction out. For the curve as merged: N frogs recorded and graduation driven; the window closing on the
  timestamp alone (nothing) and then on both clocks (`openRefunds`), a held frog becoming inventory, and that inventory filling
  a desk request. For the desk: a fill at cost, a request below the cost never filled, a request expiring between mint and
  `fulfill` leaving a stray that fills the next request with `fulfill` alone. For deposits: a marked deposit minted once to
  its recipient; a duplicate outpoint adopted, never minted again; a reorganised deposit not minted; an unmarked frog neither
  minted nor returned; a malformed marker, another chain, runes, a second inscription and a non-`ALL` signature refused and
  returned whole to the taproot sender; one spending custody held, not returned; the proof and the rate limit waited for; the
  returns' own rate limit; a report that cannot go not holding up a deposit's mint.
- **Local replica** (`items-replica-rehearsal.sh`, canister `my5ab-np777-77777-aaara-cai`): the buyer's, custody's and the
  float's addresses are three different keys (`tb1pak5h…`, `tb1p8k98…`, `tb1pn59j…`); the config is set once and a change is
  refused; every new query and update decodes through the `.did`; a real upgrade (a new module hash, so `pre_upgrade` and
  `post_upgrade` ran) brought the factory, the desk and the queue back.
- **wasm**: 740,206 B gzipped, up from 628,390 at `ff7e99b`.
- **What was not shown**: anything on a chain. There is no keyless ord for signet or testnet4, so the Bitcoin half (a
  purchase's sat flow against a real listing, a real deposit's ord checks) runs only against the fake world; the queue
  contract does not exist to call; no `tick` was run on the replica.

## Open problems

- **`lib/Wait.sol`**: the parent block time is deployment config everywhere this branch may edit. ListingRegistry, `src/nft/*`,
  and the AssetVault / AutoBuyerVault the registry deploys still use the one-argument `blocksFor` at 4663's 12 s; threading the
  config through them is a change to files other agents own.
- **The canister's `MAX_PURCHASE_WEI`** (0.5 ETH) is under a 0.633 ETH frog and far under a 5 ETH ape: at these numbers every
  item is refused until the limit is raised through the timelock (or item buys get their own maximum).
- **A slow queue on an expensive collection**: a coin whose remainder is above its tail but below one item waits for later
  coins; if none come in its window, it refunds its ETH and the units already assigned. The opening rule makes the first coin
  self-sufficient; later ones are not.
- **The keeper's cap** now bounds the queue's per-item cost, sets the opening target, gates shared-pool adds and steers the
  center. It still cannot move locked liquidity.
- **Pre-graduation pools.** Anyone can open a coin's pool key before graduation; the lock aligns the price first (buying below
  its own valuation) and a page should show only the curve until then.
- **Deployment**: no deploy script builds the curve stack yet (lock, queue, factory, vaults, wiring, directory); the fork test is
  the reference sequence.
