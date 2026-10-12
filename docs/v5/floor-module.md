# The floor module: wX at the floor, instantly, backed by an item or its ETH

Written 2026-09-23 on branch `floor-module` (from `9022920`), **before its code**. Nothing here is deployed or audited. It
replaces the per-coin item wait of docs/v3/pools.md §5 (a graduation's budget sitting in `ItemQueue` until the canister has
bought every item, with a deadline and a refund) and gives the public an instant mint and an instant burn, both at the
collection's floor. Contracts to be built here: `src/floor/FloorModule.sol` (one per collection), `src/interfaces/IFloorWrapped.sol`,
`src/interfaces/IItemQueue.sol`, additive edits to `src/Wrapper.sol` and `src/nft/NftWrapper.sol`, a type change in
`src/curve/{FrogCurve,CurveFactory,CollectionDirectory}.sol`, and best-execution routing in `src/pools/EtchRouter.sol`.

## 1. In one paragraph

Anyone pays ETH at `floor × (1 + spread)` and receives wX (any amount, 18 decimals) in the same transaction; the ETH stays in
the module as that wX's backing. Anyone returns wX and receives `floor × (1 − spread)` in ETH in the same transaction, never
more than the ETH that backs each unit. Mints and burns net against each other; only when the **net** ETH-backed units reach one
item plus a buffer is the canister asked to buy an item, and when it has wrapped one into the module the module burns one whole
ETH-backed unit against it and pays the canister's cost from that unit's ETH. So every wX is always backed either by an item in
custody or by the ETH it was minted with. The floor is the keeper's posted price, moved at most ±5% an hour on-chain, with a spread
that is tight in calm markets (0.15% a side) and widens with recent movement, staleness and backing stress; per-hour mint and burn
caps, a cap on outstanding ETH-backed units, and a burn price that can never exceed a unit's own backing carry the defence. A
graduating curve converts its whole item budget through the module in one call. Holders may also lend a floor item to the
module for 30 days and get the same item back (phase 2, §9).

## 2. What it replaces, and what stays

| before (v3/v4) | now |
|---|---|
| a graduation's item budget waits in `ItemQueue`, FIFO, until the canister buys every item; a 3-day window, then refunds | the budget is converted to wX **at once** through the module (`enqueue` fills immediately while the floor is fresh); the curve graduates in the next call. Refunds only if the floor is never fresh for the whole window |
| the canister buys one item per `itemCap + buffer` of queued budget | the canister buys one item per `itemCap + buffer` of **net ETH-backed units** (the netting signal). Same selectors, same `buyqueue.rs` |
| `MintDesk`: escrow ETH, wait an hour for a frog | the module mints wX instantly. The desk stays (the sweep vaults' canister route uses it) |
| wX → ETH only through the Uniswap pool or a random-item redemption | the module buys wX back at the floor while it holds ETH for them; otherwise the random redemption, unchanged |
| `EtchRouter` swaps ETH ↔ wX in the shared pool only | it fills the pool up to the module's price and the module for the rest (or the other way round), per trade |

What does not change: the curve and its fee split; the coin pools and the shared pool (hookless, locked, seeder shares); the sweep
vaults (raffle and burn); the drand redemption and its fairness rules; `ItemQueue` itself (kept, still deployable; a factory is
wired to either the queue or a module, the curve cannot tell which).

## 3. The design choice: the wrappers mint for the module, the module does not issue its own token

Two designs were weighed:

1. **Module-owned claim token** (the wrappers only ever mint item-backed units, to the module; the module issues its own ERC-20
   claim, "fX", redeemable for a wX or its ETH). The wrappers stay untouched. But every consumer — the curve, both pools, the
   router, the sweep vaults, every aggregator route — is priced in wX, so either all of them move to fX (a second token for
   every collection, the pools and their locked liquidity re-seeded, two tokens to route) or fX is a side token nobody routes.
   Its attack surface is small but it does not do the job: "anyone receives **wX** instantly".
2. **The wrapper mints ETH-backed wX for one module** (built). The wrapper gains one counter, `ethBacked`, changed only by
   the module, and its ledger becomes `totalSupply == (held − openTickets − openReturns) × 1e18 + ethBacked`.

(2) is built. Its attack surface, stated exactly:

- **Who can mint without an item**: one address, `floorModule`, wired **once** by the wrapper's deployer and **only while the
  wrapper has never minted anything** (`totalSupply == 0`, `held == 0`, no tickets). No holder ever holds a wX whose minter set can
  still change. The module has no owner and no setter; every bound in §6 is in its bytecode.
- **What the module can do to the wrapper**: `moduleMint(to, amount)` (adds to `ethBacked`), `moduleBurn(amount)` (burns its
  **own** balance, never above `ethBacked`), `admitRental` and `releaseRental` (§9). It cannot mint an item-backed unit, take an
  item, choose an item, record a delivery or touch a ticket.
- **Redemption cannot outrun the items**: `redeem`/`redeemToBurn` now revert unless `held − openTickets − openReturns ≥ 1`, so a
  ticket always has an item to draw (before, `held − openTickets` was `totalSupply / 1e18` and could not be zero with supply).
  An ETH-backed unit is therefore redeemable for an item only while some item is not already promised, and for ETH at the module.
- **What a bug in the module could do**: mint wX against too little ETH (bounded by `maxEthBacked`, a constructor constant),
  never take an item. That is the price of (2), and the reason the module's own invariants are fuzzed hardest.

## 4. The books, exactly

Notation: `E` = ETH-backed units outstanding (`wrapper.ethBacked()`, 18 decimals); `B` = the ETH that backs them (`backing`);
`β = B / E` wei per whole unit (the backing per unit); `F` = the accepted floor, wei per whole item.

**Wrapper (every step, both wrappers):**

```
(W1) totalSupply == (held − openTickets − openReturns) × 1e18 + ethBacked
(W2) every held item is in exactly one place: drawable (free or pending), on one Resolved ticket, or awaiting return (§9)
(W3) redeem only if held − openTickets − openReturns ≥ 1  ⇒  resolve never meets an empty set
```

**Module:**

```
(M1) module.ethUnits == wrapper.ethBacked()
(M2) address(module).balance ≥ B + S + V + gradEth + rentalEscrow + renterPool + owedTotal   (== absent forced ETH; S, V: §13.1)
(M3) B == Σ mint payments + Σ rental escrow returned − Σ basis released (burns, conversions, rental intakes)
(M4) a burn of u units releases exactly ⌊B·u/E⌋ of backing and pays at most that; the difference is revenue
(M5) a conversion (an item recorded) releases ⌊B/C⌋ per unit from the convertible pot; the canister's cost is paid from it, and any excess from the revenue pot `V`
(M6) β never decreases, except when a mint adds units at a lower price (a weighted average of what minters paid) or a rental
     returns its intake basis (§9)
(M7) E ≤ maxEthBacked; public mints keep E ≤ maxPublicEthBacked
```

So **"every wX is backed by an item or its ETH"** is (W1) for the count and (M3)–(M6) for the value: each ETH-backed unit is
backed by `β`, which is at least the lowest price any outstanding ETH-backed unit was minted at (with its spread), less nothing:
outflows never pay a unit more than `β`. **Escrow never negative** is (M2) with (M4): the module never pays a burn more than the
backing it releases, and the backing it releases never exceeds `B`. Tests: `test/floor/FloorModule.invariant.t.sol`.

### Why burns are capped at β and not at the floor

The attack every floor venue must survive is **push the floor up, then burn**: buy or delist the cheapest listings so the posted
floor rises, then sell wX to the module above what it was minted at. Capping the per-unit payout at `β` makes that drain
impossible by construction: the module can only ever hand back, per unit, the average of what was paid in for the units still
outstanding. If the floor really rose, ETH-backed wX is worth less than an item (`β < F`); its holders get `β` at the module, or
redeem for an item (random, unchanged), or sell to the pool; the canister converts ETH-backed units to items and the reserve pays
the difference (M5). That basis risk is bounded by `maxEthBacked` and by how fast the canister converts (§10).

## 5. The floor: posting, the move limit, the lag

```solidity
function postFloor(uint256 target) external;   // keeper only; wei per whole item
function floor() external view returns (uint256 accepted, uint256 target, bool fresh);
```

- **Robust floor, off-chain (another agent builds it; interface only here).** The keeper computes, per collection, the median
  of the five lowest active listings and the recent sales (their own rules: marketplaces, fees netted, outliers dropped) in ETH,
  and posts it as `target`. It posts at least every 30 minutes, and on every move of more than 0.5%. The posting key is the
  same keeper as the sweep vaults' caps. **Amended 2026-09-26** (`docs/audit/2026-09-26/fixes-B-C-keeper-draws.md`): the module
  binds no key; it reads `registry.keeper()` on every post, so the owner's rotation (seven public days) moves it, and the
  registry's incident switch (`pauseKeeper`, the Safe or the guardian, at once) stops the keeper and every mint, fill, float,
  sale and rental intake. Burns never stop. Besides ±5%/h, the accepted floor stays within ±20% of the day's anchor.
  **Amended again (R3, `docs/audit/2026-09-26/review2/fixes-R3-contracts.md` §1)**: while the keeper is paused burns pay each tier's
  backing per unit less the base spread (no floor) at once, and conversions (`itemArrived`, `snapshotCap`) stop too; a fresh burn
  counts the keeper's `target` no lower than the accepted floor could legally reach within the next hour (nor below the day band),
  so no post can price a burn at zero.
- **Freshness**: a post is fresh for one hour on **both** clocks (`lib/Wait.sol`), exactly as the vaults' caps. Stale ⇒ no mint,
  no burn, no graduation fill, no item record (recordItem may use a 2-hour snapshot, as the queue does), no rental intake.
- **The move limit, on-chain**: the accepted floor moves toward `target` by at most `maxMoveBps × min(Δt, 1 h) / 1 h` from the
  previous accepted value (default 500 = 5% per hour), where `Δt` is the **smaller** of the two clocks' elapsed time since the
  last post (a sequencer running an hour ahead cannot buy itself a bigger step). A keeper silent for five hours still moves only
  one hour's worth; frequent posts compound to at most `e^0.05 − 1 = 5.13%` per hour.
- **The lag**: `target` is stored beside the accepted floor. Mints price at `max(accepted, target)`, burns at `min(accepted,
  target)`: whichever way the real floor moved faster than the limit, the lagging side is priced against the trader, never
  against the escrow. A keeper that posts nonsense in the direction that would help an attacker moves the price only at the limit.
- **The first post** sets the anchor unbounded (there is nothing to bound it by). Everything waits for it.

## 6. Price defences (the highest risk; built first, tested hardest)

### 6.1 The spread function, and its bounds

```
s_mint = min(S_max, s_base + s_vol + s_stale + s_stress)
s_burn = min(S_max, s_base + s_vol + s_stale)

s_vol    = k_vol × m          m = the accepted floor's absolute movement over the trailing hour, in bps (linear decay over 1 h)
s_stale  = s_staleMax × clamp((age − 10 min) / 50 min, 0, 1)      age on the OLDER-looking clock
s_stress = s_stressMax × min(1, E / maxPublicEthBacked)          mint side only: minting more is dearer as backing piles up

P_mint = max(F_acc, F_tgt) × (1 + s_mint)                      rounded up
P_burn = min( min(F_acc, F_tgt) × (1 − s_burn),  β )           rounded down
```

| parameter | default | why |
|---|---:|---|
| `s_base` | **15 bps** a side | the trader-economics target (§11): a calm round trip within ~0.5 pp of Pons |
| `k_vol` | 1.0× | a move of `m` widens both sides by `m`, which cancels a manipulated move within its hour (§6.3) |
| `s_staleMax` | 100 bps | a post 10–60 min old costs up to +1%; after 60 min nothing trades |
| `s_stressMax` | 300 bps | at `E = maxPublicEthBacked`, minting pays ≈ the conversion cost the canister will incur (§10) |
| `S_max` | 1,000 bps | no side ever wider than 10% |

Bounds: in calm markets (no move in the last hour, a post under 10 minutes old, no ETH-backed units outstanding) both sides are
exactly `s_base`. Never below `s_base`, never above `S_max`. The spread is **not** what defends the escrow: the move limit, the caps
and the `β` cap do (§6.2–6.4), and the test suite runs every attack at the 15 bps base (and at zero volatility weight) to show it.

### 6.2 Caps

| cap | default frogs (F ≈ 0.633) | default apes (F ≈ 5) | rule |
|---|---:|---:|---|
| mints per hour (`mintPerHour`) | 5 items (≈ 3.2 ETH) | 2 items (≈ 10 ETH) | leaky bucket: at most the cap at once, refilling at the cap per hour, on the slower clock |
| burns per hour (`burnPerHour`) | 5 items | 2 items | same |
| ETH-backed outstanding, public (`maxPublicEthBacked`) | 10 items (≈ 6.3 ETH) | 4 items (≈ 20 ETH) | a public mint is refused if it would push `E` past it |
| ETH-backed outstanding, hard (`maxEthBacked`) | 40 items | 10 items | graduations too; a graduation that would exceed it waits (`fill`) |
| conversion allowance (`convBps`) | 300 | 300 | `itemCap = F_acc × 1.03`: floor + ≈2% ETH→BTC + fees, what the old queue's cap meant |
| canister buffer (`bufferBps`) | 500 | 500 | the canister buys once net ETH-backed units ≥ 1.05 items |

A leaky bucket passes at most `C + C·t/1 h` in any window of length `t`: at most 2C in any one hour. It refills on the **slower**
clock, so a sequencer jumping its timestamp an hour cannot refill it.

### 6.3 Manipulation analysis, with bounds

**Push-up, then burn** (buy or delist the cheapest listings so the keeper's median rises, then burn wX at the module).
- Escrow drain: **zero, by construction**. `P_burn ≤ β`; a burn releases `⌊B·u/E⌋` and pays at most that (M4); `β` of the units
  left does not fall (M6). The escrow pays back, per unit, at most what the outstanding units paid in.
- Gain against the pool (bought wX at the pool's fair price, burnt at the module): at most `β − (pool price)` per unit, i.e. the
  mint spread that other minters left behind, bounded by the burn cap per hour. At frogs' 5/h: `5 × 0.633 × 0.0015 = 0.0047 ETH/h`.
- Moving the floor itself: `+δ` at most 5%/h; the volatility term adds `δ` to both sides, so the burn price after the push is
  `F(1+δ)(1 − s_base − δ) ≤ F(1 − s_base)` — never better than burning before the push. `test_pushUpThenBurn_*` runs this at a
  tight spread, with the volatility weight at 1× and at 0 (the `β` cap alone must hold).
- Cost to the attacker off-chain: buying every listing below the new median (they then hold items bought above the old floor).

**Push-down, then mint** (list items cheaply, or have the keeper see cheap sales, so the floor falls; mint cheap wX; sell it in the
pool at the fair price or hold until the floor recovers).
- Within an hour: the floor falls at most `δ ≤ 5%`, and `P_mint = F(1−δ)(1 + s_base + δ + s_stress) ≥ F(1+s_base)(1 − δ²)`: at
  5%, 0.25% cheaper than before the push, on at most 5 frogs: ≤ 0.008 ETH.
- Sustained (the median held down for hours): bounded by `maxPublicEthBacked × F × drop`. The loss lands as basis risk on the
  ETH-backed units (`β < F`), not as an escrow shortfall. At frogs 10 × 0.633 × 10% = 0.63 ETH for a 10% fake drop held two hours.
- Cost to the attacker: to hold a median of the five lowest listings down they must **offer** items at the lower price for the
  whole time, and anyone — our canister first, it converts ETH-backed units at the floor — can buy them.

**Stale price**: after three hours on either clock nothing trades at the module; the router falls back to the pool (`FLOOR_TTL`: one hour
until the operator's decision of 2026-10-09). Between 10 minutes and 3 hours the spread grows to +1%. The sequencer cannot make a stale post look fresh (both clocks), nor refill a cap early.

**Compromised keeper**: it can move the accepted floor 5% per hour, and at most 20% per 24 h window, in either direction until it
is paused (the Safe or the guardian, at once). Push-up drains nothing: burns pay at most the backing per unit the units paid in
(the reserve tier at its **basis**, `reserve / R`; fee and treasury money sits in `surplus`, which no burn is paid from — the
2026-09-26 review's B-02 showed that before this split a pushed floor paid the surplus out). Push-down lets it mint up to `maxPublicEthBacked` units cheaply, and sell them: worst case
`maxPublicEthBacked × F × (1 − 0.95^h)` after `h` hours — frogs 10 × 0.633 × 23% = 1.4 ETH after 5 h; apes 4 × 5 × 23% = 4.6 ETH.
A watcher comparing posts against the marketplaces bounds `h`. It cannot touch items, and graduations price at
`max(accepted, target)` so it cannot make one mint cheap.

**Sandwich between the module and the pool**: the module's price does not move with trades (only with the keeper's floor), and
the router fills the pool only up to the module's price (§8). A sandwicher who moves the pool against a victim moves the victim's
remainder to the module: the victim's average price is at most `P_mint` (while the module has room), and the sandwicher pays the
pool fee twice for nothing. `test_sandwich_*`.

**Mint–burn churn**: a round trip costs `s_mint + s_burn` ≥ 30 bps; each unit churned adds that to the reserve.

### 6.4 The graduation lane

A graduation converts its whole item budget at `P_grad = max(F_acc, F_tgt) × (1 + convBps)` — the all-in price the old queue
paid per item — with no hourly cap (it cannot be sized by an attacker: it costs a whole raise and the units go into locked pools),
bounded by `maxEthBacked`. Its ETH joins `B`; the canister converts it to items through the same netting signal.

## 7. The canister: the same interface, a different address

The module implements every selector `autobuyer/canister-rs/src/buyqueue.rs` calls, with the same signatures:

| call | on the module |
|---|---|
| `wrapper()`, `payee()` | as the queue |
| `queued()` | `want × (itemCap + buffer)`, where `want = ⌊(E_free − buffer units) / 1e18⌋` and `E_free = E − 1e18 × (rentals leaving)` |
| `itemCap()` | `F_acc × (1 + convBps)` while fresh; 0 when stale |
| `buffer()` | `itemCap × bufferBps` |
| `itemsRecorded()` | items converted so far |
| `headDeadline()` | `(0, 0)` always: nothing here expires |
| `recordItem(cost)` | custodian only, after `wrapDeposit(…, to = module)` / `mint(…, to = module)`: a fresh unit is here beyond what the module owes; `cost ≤ max(itemCap, a snapshot within 2 h)`; burns one ETH-backed unit, releases `β`, pays `cost` from it (the reserve covers `cost − β`, or it reverts and the canister waits) |
| `refundExpired()` | reverts `NothingExpires` (the canister never drives it: `drive_refunds` is false with no deadline) |
| `snapshotCap()`, `reclaimStray()` | as the queue |

So `buyqueue.rs` drives the module unchanged: `want = queued / (itemCap + buffer)` is exactly the netting signal, and
`per_item_cap = min(itemCap, queued / need) = itemCap`. `CollectionDirectory.queues()` returns the module for a collection
whose factory is wired to it (the directory types it as `IItemQueue`). **What the canister agent should change**: nothing for
purchases; for rentals, §9.6.

## 8. Routing: the cheaper venue per trade, split when that is cheaper

`EtchRouter` gains the module (constructor argument; zero keeps pool-only routing) and two direct functions,
`buyWrapped(minOut, to, deadline)` and `sellWrapped(amountIn, minEthOut, to, deadline)`. For every ETH ↔ wX leg — the direct ones
and the first/last leg of a coin trade:

- **Buy (ETH → wX)**: swap in the shared pool with a price limit where the pool's marginal cost **including its LP and protocol
  fee** reaches `P_mint`; send the rest of the ETH to the module (up to its room: caps and `E`); anything beyond the module's room
  goes back to the pool without a limit. Pool first while it is cheaper, the module at its constant price, the pool again only past
  the module's room: that is the cheapest fill for a convex pool and a flat venue.
- **Sell (wX → ETH)**: the mirror, with `P_burn`.
- The module unusable (stale floor, no room): pool only, as before. `minOut` binds the whole trade.

The pool stays hookless and aggregators route it as before; they may also call the module directly (plain functions, no hook).

## 9. Item renting (phase 2)

### 9.1 What it is for

A holder lends one **floor** item to the module for `rentTerm` (30 days) and gets **the same item** back at the end. While lent,
it backs one unit that was ETH-backed (the canister need not buy one), and it is in the random-redemption set like any item. The
renter earns `rentShareBps` (50%) of the module's realized spread revenue, shared equally among active rentals. If a burner's
ticket draws the renter's exact item and it is delivered, the renter receives `F × (1 + premiumBps)` (10%) in ETH instead.

### 9.2 Intake (both chains, through the wrappers' deposit paths)

The canister (custodian) calls `module.rentIn(renter, returnTo, eligibilityProof, attestation)`:
- the item is on the keeper's **eligibility list** (`rentalRoot`, a Merkle root the keeper posts; rare items are not on it): leaf
  `keccak256(abi.encode(itemKey))`, where `itemKey` is the inscription's listing leaf (Bitcoin) or `keccak256(abi.encode(tokenId))`;
- there is an ETH-backed unit for it to back: `E_free ≥ 1e18`; rentals active `< maxRentals`; the floor is fresh;
- the module calls `wrapper.admitRental(attestation, returnTo)`, which runs **the same checks as the deposit path** —
  Bitcoin: `abi.encode(inscriptionId, proof, txid, vout)` as `wrapDeposit` (membership, outpoint once, not held); NFT:
  `abi.encode(tokenId, depositTx)` as `mintDeposit` (not held, source once) — stores the item as a **rental** with its return
  destination (a taproot address / an EVM address, checked there), puts it in the pending set with the usual stamp, and mints
  one unit **to the module**;
- the module burns that unit against `E` (one ETH-backed unit becomes item-backed) and moves `β` from `B` into the rental's escrow.

Net: supply unchanged, `held + 1`, `E − 1e18`, and the ETH that backed the unit waits beside the rental.

### 9.3 States (in the wrapper, which alone knows where an item is)

```
Active ──(a ticket draws it)──▶ ActiveDrawn ──(delivered)──▶ Taken          (renter paid F × 1.1)
  ▲                                  └──────(refunded after 7 d)──▶ Active   (back in the set, as any item)
  └──(term over: releaseRental)──▶ Leaving(L) ──(every ticket < L resolved: removed from the set)──▶ Ready ──(recordReturn)──▶ Returned
                                      └──(a ticket < L draws it)──▶ LeavingDrawn ──(delivered)──▶ Taken  (+1 unit minted to the module)
                                                                               └──(refunded)──────▶ Ready
```

### 9.4 Fairness of the draw (a renter cannot steer or predict)

- **Entering**: a rental item enters the pending queue stamped with the ticket count, exactly like any mint: a ticket burned
  before it arrived can never draw it. Nothing about the draw is chosen by the renter; the round is a future drand round.
- **Leaving**: `releaseRental` records `L = tickets.length`. Every ticket burned **before** the release (id < L) keeps the item in
  its set — its set was fixed when it was burned — and every ticket burned after (id ≥ L) never has it: `resolve(id)` removes every
  leaving item with `L ≤ id` from the free set (or tombstones it in pending) **before** drawing, and `processLeaving()` (anyone)
  does the same once `nextToResolve ≥ L`. Removal depends only on ticket order, never on a caller or on a known round, so nobody —
  the renter, a burner, the module — can take the item out of a draw whose randomness is already public.
- **What the renter can choose**: only *when* the release is requested after the term ends (anyone may request it; the keeper
  does at term end). Choosing to stay exposed to unresolved tickets is a bet on a future round, not a steer; the premium is paid
  from spreads, bounded by `maxRentals`.
- **Not counted as owned**: `rentedHeld` is exposed; at release the unit it backed becomes ETH-backed again (`ethBacked + 1e18`, its
  escrow back into `B`), so the netting signal asks the canister to buy a permanent replacement if one is still needed.
- **No id is ever chosen by a function, except**: returning a rented item to the address stored at its intake. The wrapper's
  exact-ABI test holds that `releaseRental` takes only an item that was admitted as a rental.

### 9.5 The books with rentals

`openReturns` counts rentals Leaving, LeavingDrawn and Ready (items promised back). (W1) holds at every step:
- release: `ethBacked + 1e18`, `openReturns + 1` (the unit becomes ETH-backed; the item stops backing supply);
- return delivered: `held − 1`, `openReturns − 1`;
- LeavingDrawn delivered: the item left through a ticket, so the promise cannot be kept: `openReturns − 1` and one unit is minted
  to the module (backed by the item that ticket did not take); the module burns it against `E` and pays the renter from its `β`
  plus the premium from the reserve. The module keeps `E_free ≥ 1e18 × (rentals leaving)`, so that burn always has an `E` to burn.
Compensation beyond the rental's escrow is paid from `reserve`; any shortfall is recorded per rental and paid by `payDebt` as
the reserve refills (before any skim).

### 9.6 For the canister agent (spec, not built here)

- Intake marker, Bitcoin: `OP_RETURN "EWR1" ‖ chainId (4, BE) ‖ module (20) ‖ renter on 4663 (20)` = 48 bytes; the return
  destination is the taproot script of the deposit's input 0 (refused otherwise). NFT: `safeTransferFrom(owner, custodian, id,
  "EWR1" ‖ chainId ‖ module ‖ renter)`, top-level from the owner; the return destination is `tx.from`. All of sweep.md 1a's /
  nft-wrapper.md §4's checks apply; then `rentIn` instead of `wrapDeposit`/`mintDeposit`. Check `rentalRoom()` first; an item
  that cannot be taken now is returned as a refused deposit.
- Returns: for every item in `Ready` (`pendingReturns()` / `rentalOf`), deliver to its stored `returnTo` and call
  `recordReturn(itemId, txHash)` (custodian only). There is no deadline and no refund for a return. That call also closes the
  module's rental (`rentalReturned`): the unit the item backed is ETH-backed from that call and its escrow is its backing again in
  the same transaction (deep-fuzz 2026-09-28; before, a separate `settleRental` did it and the unit was unbacked in between).
- Keeper: `postRentalRoot(root)`; call `endRental(id)` at term end and `settleRental(id)` after a draw's delivery.

## 10. What happens to the ETH: the revenue pot

(Amended 2026-09-23: what this section called "the reserve" is the **revenue pot** `V` (`revenue`, `revenueTarget`) since §13; "the
reserve" now means the standing reserve `S` of §13.1, which is a different thing: a share of every mint's ETH, never revenue.)

Revenue is realized when a unit leaves at less than its backing: a burn (`released − paid`), or a conversion that cost less than
`β`. `rentShareBps` of it goes to active renters, the rest to `revenue`. The revenue pot pays conversions that cost more than `β`
(the canister's ≈2–3% over the floor on units minted at `F × 1.0015`), rental premiums, and rental debts. Above `revenueTarget`
anyone may `skim()` the excess to the platform address — the protocol treasury (§13.3). **At a tight base spread the reserve is fed by churn, not by one-way flow**: a
sustained one-way mint flow raises `s_stress` toward the conversion cost; if the reserve cannot pay a conversion, it waits (the
canister holds the unit as a stray and retries or reclaims), public mints stop at `maxPublicEthBacked`, and the router uses the pool.

## 11. Trader economics: the round trip (ETH → coin → ETH)

The coin pool charges its 1% LP fee plus Uniswap's protocol fee on 4663 (0.10% on a 1% pool: 1.099% a side). The ETH ↔ wX leg is
the module at `s_base` in calm markets, or the pool (0.30% + 0.05% protocol + impact) when the module is wider or out of room.
Pons: ~1% a side (the coordinator's figure). Round-trip cost = `1 − Π(1 − cost of each leg)`, price impact excluded (a round trip
through the same pools returns most of it; measured numbers are in §13 with the code).

| | ETH→wX | wX→coin | coin→wX | wX→ETH | round trip | vs Pons 1.99% |
|---|---:|---:|---:|---:|---:|---:|
| calm, `s_base` 15 bps (default) | 0.15% | 1.099% | 1.099% | 0.15% | **2.48%** | +0.49 pp |
| calm, `s_base` 25 bps | 0.25% | 1.099% | 1.099% | 0.25% | 2.67% | +0.68 pp |
| calm, `s_base` 50 bps | 0.50% | 1.099% | 1.099% | 0.50% | 3.16% | +1.17 pp |
| stressed: a 5% move this hour (module 5.15%, router takes the pool) | 0.35% + impact | 1.099% | 1.099% | 0.35% + impact | 2.87% + impact | +0.88 pp + impact |
| stressed and no pool depth (module only) | 5.15% | 1.099% | 1.099% | 5.15% | 12.0% | |

The requested base range (0.25–0.5%) and the target (within ~0.5 pp of Pons) cannot both be met while the coin pool alone costs
2.2% a round trip: 0.5 pp leaves 0.3% for both wX legs, so the default is **15 bps**. The defences do not depend on it (§6).

**Confirmed with the code (2026-09-23, `test/floor/FloorRouter.t.sol::test_roundTripCosts_calmAndStressed`, against the real
PoolManager's 4663 runtime code, which has no protocol fee controller set, so no protocol fee):** measured round trips through
`EtchRouter.buy` then `sell`, both wX legs at the module in the calm case:

| trade | calm (module 21 bps a side: 15 base + 6 stress) | stressed (a 5% move this hour: the router takes the pool for the wX legs) |
|---|---:|---:|
| 0.1 ETH | **232 bps** | 255 bps |
| 1 ETH | 219 bps (the coin pool's impact comes back on the way out) | 235 bps |

232 bps = 2 × 100 (coin pool) + 21 + 15 (module) + a little coin-pool impact; against Pons's 1.99% that is **+0.33 pp** (1 ETH:
+0.20 pp) — inside the ~0.5 pp target, so the base spread stays **15 bps**. A 25 bps base would measure ≈ 2.52% (+0.53 pp);
50 bps ≈ 3.02% (+1.03 pp): the requested 0.25–0.5% range does not fit the ~0.5 pp target and is not used. Note what the
measurement is sensitive to: the pool's impact is recovered on a round trip, the module's spread is not, so the more of the wX
leg the module takes (the cheaper it is), the closer the round trip sits to the fee legs' 2.3%. The reserve share (§13.1)
changes no price: it decides where a mint's ETH sits, not what the minter pays.

## 12. What this does NOT protect against

- **The keeper's word**, within the move limits (§6.3): a compromised keeper costs at most `maxPublicEthBacked × F × 20%` a day
  through cheap mints until it is paused, and nothing through burns (they are capped at the paid-in basis, and priced at no less
  than the next hour's legal floor). A keeper that stops (or posts with no confidence) stops mints and fills; while it is paused,
  burns pay the lower of the backing per unit and the floor it left (`pauseFloor()`, never above what a mint under that floor
  cost: `docs/audit/2026-09-26/review3/fixes-R4-contracts.md` §1; capped at the fill band's low edge `accepted × (1 − fillBandBps)`
  and at the cheapest public mint of the 12 to 24 hours before the pause, so no mint under an earlier post or before a rise of the
  floor exits a pause above its cost either: `docs/audit/2026-09-26/review4/fixes-golive-lows.md` §1), and after a week with no good post the backing per unit with no
  floor (`exitAtBacking()`); after that week `skim` also sends the surplus to the treasury (`escaped()`), so the ETH is never
  locked.
- **Basis risk**: ETH-backed wX does not appreciate with the floor. If the floor rises faster than the canister converts, `β < F`
  for the units outstanding: bounded by `maxEthBacked` and the conversion rate.
- **The custodian**: as today, it can mint against nothing (deposits, conversions and rentals alike) and must deliver returns.
- **A canister that stops converting**: `E` stays up to `maxEthBacked`; public mints stop at `maxPublicEthBacked`; burns still work.
- **Rental premium beyond the reserve**: recorded as debt, paid as spreads come in.
- **Uniswap's protocol fee** is Uniswap's to change.

## 13. Self-funding: the protocol funds itself (amendment of 2026-09-23, before its code)

The floor module as built (§1–§12) is funded by nothing but its spreads: every conversion's 2–3% over the floor is paid from
the revenue pot, or waits. This amendment makes the protocol fund its own floor: a standing ETH reserve kept from every mint, a
slice of every trade fee flowing into that reserve and into protocol-owned liquidity, one treasury that every protocol income
stream reaches and that allocates by fixed percentages only the timelock can change, and a module price that follows the
protocol's own fills inside the keeper's band. Contracts touched: `src/floor/FloorModule.sol` (the pots, `itemSold`, the fills
price, the funding entries), `src/floor/ProtocolTreasury.sol` (new), `src/curve/FrogCurve.sol` (two more fee shares),
`src/pools/LiquidityLock.sol` (protocol-owned liquidity), `src/interfaces/{IItemQueue,ICapSource,ILiquiditySeeder}.sol`
(additive), `src/curve/ItemQueue.sol` (forwards the new fee entries). Nothing in the keeper, the canister or the web changes for
it, except that they may read two new views (§13.6).

### 13.1 The reserve share: two pots, one blended backing

Every public mint and every graduation puts `reserveBps` (**2,500** = 25%) of its ETH into the **standing reserve** `S`, and the
same share of its units into the **reserve units** `R`; the rest goes to the convertible pot `B` and the convertible units
`C = E − R`. The canister converts only `C` (`netUnits() == C`); `S` is never converted into an item: it is the ETH that stays
ETH so that burns are paid without waiting for the canister, and it takes the treasury's reserve allocation (§13.3) and the
proceeds of the protocol's item sales (§13.4). A float (§13.3) is a mint with no reserve share: all of it is convertible.

```
E = C + R                      wrapper.ethBacked(); R = reserveUnits (the module counts R; C follows the wrapper)
β_B = B / C                    what a conversion or a rental intake releases (M5)
β_S = S / R                    what the reserve holds per reserve unit (∞ when R = 0: a surplus)
β   = min(β_B if C > 0, β_S if R > 0)     the burn cap: never more per unit than the pot it draws from holds per unit
```

**Burns draw the reserve first, in tiers** (as built; the first draft of this section mixed the pots by value and would have
turned treasury top-ups into skimmable revenue): while `R > 0` a burn takes reserve units only, at most `R` per call
(`burnCapacity()` says so; the router sends the rest of a larger sell to the pool), paid from `S` at
`P_burn = min(F(1 − s), β_S)` per unit — and **`S` keeps the spread**: it pays exactly what the burner receives, so `β_S` never
falls on a burn and the reserve stands (it shrinks only by what burners are paid; it grows by every mint's share, the treasury's
allocation, sale proceeds, and the burn spread it keeps). Once `R = 0` a burn takes convertible units at
`P_burn = min(F(1 − s), β_B)`, releasing exactly `⌊B·u/C⌋` as built (M4), the difference being revenue. `beta()` is the current
tier's backing per unit; a burn of the last reserve units leaves what is left of `S` as a surplus with no units, which the next
mint's reserve units share (their `β_S` is then above the mint price). A conversion (`itemArrived`) takes one unit from `C` and
`β_B` from `B`, exactly as built; a rental intake likewise. The blended backing per ETH-backed unit is `(B + S) / E`:
**wX stays fully backed** — every unit by an item in custody, or by ETH in `B` or `S`. A mint whose reserve share would carry a
unit but no wei (or the reverse) puts all of it in `B`: a reserve unit is never left without backing. The public cap and the
stress term count convertible units only (reserve units are never converted, so they are no backlog for the canister).

Books, with the new symbols (`V` = the revenue pot of §10, `S` = the standing reserve, `L` = ETH owed as a loan repayment, which
is nothing the module holds):

```
(M2')  balance ≥ B + S + V + gradEth + rentalEscrow + renterPool + owedTotal
(M8)   R ≤ E; reserveUnits changes only by a mint (+reserveBps of its units) or a burn of reserve units (−u, u ≤ R)
(M9)   S changes only by: a mint (+reserveBps of its ETH), a reserve-tier burn (−what the burner is paid), fundReserve (+), itemSold (+proceeds). Never by a conversion, a rental, a graduation release, a float or a skim.
(M10)  β_S never falls on a burn; β_B never falls on a burn (it releases exactly β_B per unit) nor on a conversion
```

### 13.2 Fee-funded liquidity and reserves

`FrogCurve.Config` gains two shares of the trade fee beside the vault's and the creator's: **`reserveShareBps`** and
**`liquidityShareBps`** (of the fee; `vault + creator + reserve + liquidity ≤ 10,000`, the platform takes the rest). The worked
terms become 40 / 30 / 5 / 5 / 20: of a 1% fee, 0.05% of volume to the collection's reserve, 0.05% to protocol-owned liquidity,
0.2% to the platform (the treasury). The coin pool's LP fees are split the same way (`onPoolFees`, `forwardWrappedFees` pay the
new shares in ETH; the wX and coin fee paths keep their old three-way split, there being no ETH to route).

- **Reserve slice** → `queue.fundReserve{value}()`: at a floor module, into `S` — after any bootstrap loan is repaid (§13.3);
  at an `ItemQueue` collection (which has no reserve), into protocol liquidity.
- **Liquidity slice** → `queue.fundLiquidity{value}()`: the module repays a loan first, then hands the rest to the collection's
  `LiquidityLock.addProtocolLiquidity{value}()`; a SWEEP_ONLY coin (no queue) sends both slices straight to the lock.
- **Protocol-owned liquidity** in the lock is a pseudo-coin, `PROTOCOL = address(1)`, with the same layers as any graduation's
  contribution. It arrives as ETH only, so it pairs with whatever wX the protocol positions have earned in fees (kept pending for
  exactly that) and puts the rest into the ETH-only bid layer below the near band; `deploy(PROTOCOL)` (anyone) retries what the
  price guard deferred. **It is never withdrawable**: the lock has no removing path, and `PROTOCOL` has no curve to pay. Its ETH
  fees go to the treasury (`wireTreasury`, once, by the deployer; unwired, they compound as more protocol liquidity); its wX fees
  compound.

### 13.3 The treasury: every income stream in, fixed percentages out, no discretionary path

`ProtocolTreasury` (one per deployment) receives ETH from anywhere (`receive()`): the platform's 0.2% (`claimPlatformFees` on
every curve, whose `platform` address is the treasury), the capped graduation fee (same route), the modules' spread revenue
(`skim()` to `platform`), the protocol liquidity's LP fees (§13.2), and every unused buffer or tail that already reaches the
platform address. It has **no function that names a recipient**, no withdraw, no pause. It allocates by four percentages that
sum to 10,000 and that only the timelock — the registry's owner, as for every ceiling in this repository — may change:

| allocation | default | where it goes |
|---|---:|---|
| collection reserves | **40%** | `module.fundReserve{value}()` on each registered collection, weighted by its trailing 7-day mint + burn volume (`volume7d()`) |
| item floats | **30%** | `module.fundFloat{value}()`, same weighting: a mint to the module with no reserve share and no spread charged back — the units the module holds (`floatUnits`) raise `C`, so the canister buys items ahead of demand; the module's inventory is then what `itemSold` sells |
| bootstrapping | **20%** | a pot from which any registered collection that never had one may draw ONE loan of `bootstrapLoan` (timelock-set, default 2 ETH; less if the pot holds less), half to its reserve, half to a float (or all to the reserve while its floor is stale) |
| operations | **10%** | paid at once to `ops`, an address only the timelock sets |

`allocate()` (anyone) moves every wei that arrived since the last call into the three pots and pays operations; `distribute()`
(anyone) pays each pot out by weight to the collections whose weight is non-zero, keeping what a stale module cannot take now
(`fundFloat` needs a fresh floor and room under `maxEthBacked`) for the next call. A week with no volume anywhere leaves the pots
where they are. Registering a collection (`addCollection(module)`) is the timelock's; it checks the module names this treasury.

**Bootstrap loans.** `bootstrap(module)` (anyone) funds a registered module once, records `principal = what was delivered`, and
the module repays it **before any other fee route**: its `fundReserve` and `fundLiquidity` entries pay `treasury.repay{value}()`
first, up to what is outstanding, and only the remainder reaches the reserve or the lock — whoever the funder is, the
treasury's own `distribute` included (a collection still owing its loan repays it from its reserve allocation, and the repayment
is income again). Invariants: `repaid ≤ principal`
always (`repay` refuses more than is outstanding, so a loan can never be repaid twice); `outstanding == principal − repaid`;
the only other way it falls is `forgive(module)` by the timelock, which is an event nobody can miss; a module never repays a
loan it does not have (`outstanding == 0` ⇒ the entries route everything on).

Invariants (tests: `test/floor/Treasury.t.sol`, `test/floor/Treasury.invariant.t.sol`): the four shares sum to 10,000 at every
step; `reservesPot + floatsPot + bootstrapPot ≤ balance`; every wei allocated went to a registered module, to `ops`, or is
still in a pot; loans are monotone; `distribute` never reverts on a stale module.

### 13.4 The fills-based price

The module keeps the last eight **fills** — the protocol's own executed prices, each **normalised to the floor it implies**: a
purchase reported by `itemArrived(cost)` is recorded at `cost / (1 + convBps)` (a cost is all-in: the floor plus the ≈3%
conversion allowance, so a canister paying exactly the cap moves the price nowhere), a sale reported by
**`itemSold(proceeds)`** (new; payable, the custodian, `msg.value == proceeds`) at `proceeds / (1 − convBps)` (proceeds are net
of the same) — and prices itself at their **time-weighted average over the trailing 24 hours**, treating the price as a step
function that holds each fill's value until the next one, and the keeper's accepted floor before the first fill in the window
(a fill older than the window is not carried in: when no fill is in the window the keeper floor alone stands, which is a jump
of at most the band when the last fill expires):

```
price() = clamp( TWAP_24h(fills), F_acc × (1 − fillBandBps), F_acc × (1 + fillBandBps) )     fillBandBps default 300
        = F_acc                                                                                 with no fill in the window
```

`price()` replaces `F_acc` in `P_mint`, `P_burn` and `P_grad` (§6.1, §6.4); the lag rule stays (`max(price, target)` for
mints, `min` for burns). It does **not** replace `F_acc` in what bounds the canister — `itemCap()` and the minimum a sale may
report (`minSaleProceeds() = F_acc × (1 − convBps)`) — nor in a rental's compensation, and `floor()` still answers the accepted
keeper floor: the keeper steps its own posts against it, and the canister's cost bounds are its posts, not our fills (were the
cap on our fills, paying more would raise the cap would allow paying more).

**Manipulation.** Nobody but the custodian can create a fill. A purchase fill is at most `itemCap = F_acc × 1.03`; a sale fill
is at least `F_acc × 0.97`; the band clamps the result to ±3% of the accepted floor, which itself moves at most 5% an hour and
expires in one. So the worst a custodian can do to the price through fills is ±3%: pushing it up makes minting dearer (no gain to
anyone) and burning dearer by at most 3% — still capped at `β`, so the escrow pays nobody more than what backs them (§6.3 holds
unchanged); pushing it down makes minting up to 3% cheaper, the §6.3 push-down bound with `δ = 3%` on top of the keeper's move.
The time weighting means a burst of fills in one block weighs as much as the seconds it held the latest price. A canister that
reports nothing leaves the keeper floor alone, as before any fill.

**Sales.** `itemSold` is how the protocol's inventory turns back into reserve: the custodian pays `proceeds` and receives one
whole unit of the module's float inventory (`floatUnits ≥ 1e18`; graduation units and strays are never sold); the ETH goes to
`S`. It needs a fresh floor and `proceeds ≥ minSaleProceeds()`. What the custodian does with the unit (redeem it for a random
item and sell that, or sell the wX) is the custodian's side, as every purchase is. The canister agent may implement selling; nothing
here requires it.

### 13.5 The graduation lane and the sweep vaults

Unchanged: a graduation converts its budget at once (`enqueue` → `_tryFill`), its units are minted to the module at
`P_grad` and released to the curve in the same transaction as `graduate()` opens the coin's pool — the pool opens immediately
with ETH-backed units (`test_aGraduationConvertsItsBudgetAtOnce`, `FloorRouter.t.sol::_twoCoins`). The reserve share applies to
it (25% of its ETH into `S`), so the canister buys items for 75% of a graduation's units and the other 25% stay ETH-backed in the
pools until burned. The sweep vaults are as built: a whole item is raffled when the vault holds one, the burn policy burns as it
goes, no fractional prizes.

### 13.6 For the keeper, the canister and the web

Nothing they call changes: `floor()`, `netUnits()`, `escrow()`, `buffer()`, `maxItemCost()`, `itemsArrived()`,
`itemArrived(uint256)`, `postFloor(uint256,uint16,uint32)`, `floorBounds()`, `quoteMint(uint256)`, `quoteBurn(uint256)`,
`wrapped()`, `spreadBps()`, `ethBackedUnits()` keep their selectors (`test_theModulesNewSelectors`). Two semantics moved:
`netUnits()` answers the **convertible** units `C` (the canister's netting signal: what it should still buy), and
`escrowBalance()` (the web's proof-of-reserves read) answers `B + S`, every wei backing ETH-backed units, while `escrow()` (the
canister's `budget_left`) still answers `B`. Renamed: `reserve()` → `revenue()`, `reserveTarget()` → `revenueTarget()` (the old
revenue pot); `reserve()` now means the standing reserve. Additive reads: `price()` (the floor the module prices at; the web
should quote with `quoteMint`/`quoteBurn`, which use it), `reserveUnits()`, `floatUnits()`, `betaConvertible()`,
`betaReserve()` (`beta()` is the current burn tier's), `fillCount()`, `fill(uint256 i)` (newest first), `minSaleProceeds()`,
`volume7d()`, `loanOutstanding()`, `treasury()`, `seeder()`, `FILL_WINDOW()`. New custodian entry: `itemSold(uint256 proceeds)`
payable. New entries anyone may call with ETH: `fundReserve()`, `fundLiquidity()`; `fundFloat()` takes ETH from the treasury
only (a stranger funding floats could fill `maxEthBacked` and block graduations). The module's constructor takes the treasury
after `platform` (zero for none). `CurveFactory`/`ICapSource` expose `seeder()`; `FrogCurve.Config` has `reserveShareBps` and
`liquidityShareBps` and the curve `flushProtocolFees()`; `IItemQueue` has `fundReserve()` and `fundLiquidity()`;
`ILiquiditySeeder`/`LiquidityLock` have `addProtocolLiquidity()`, and the lock `PROTOCOL()`, `treasury()`, `wireTreasury(address)`.
`ProtocolTreasury`: `allocate()`, `distribute()`, `bootstrap(address)`, `repay()`, the timelock's `setShares`, `setOps`,
`setBootstrapLoan`, `addCollection`, `forgive`, and views (`test_theTreasuryExposesExactlyWhatItMeansTo` lists them all).
