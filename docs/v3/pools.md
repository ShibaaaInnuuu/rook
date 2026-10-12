# Pools: what a graduation seeds, who owns it, and why nobody can take it out

Written 2026-09-23 on branch `v4-pools` (from `40735bf`), **before its code**. It replaces the "Liquidity" section of
docs/v3/curve.md and the stage-2 spec of docs/v3/sweep.md. Nothing here is deployed or audited. Contracts (to be built on this
branch): `src/pools/LiquidityLock.sol` (the seeder and the lock, one per collection), `src/curve/ItemQueue.sol` (the
collection's batched buy queue), `src/pools/EtchRouter.sol` (a public router), and the graduation changes in
`src/curve/{FrogCurve,CurveFactory}.sol`. The curve is generic over the wrapped item: wFROG today, an ERC-721 wrapper (wAPE)
built elsewhere tomorrow; all it needs is an ERC-20 whose `1e18` is one item and whose `custodian()` names who may report a
purchase.

## 1. Uniswap v4 on Robinhood Chain (4663), measured

Measured on 2026-09-23 with read-only `eth_call` / `eth_getCode` against both `https://rpc.mainnet.chain.robinhood.com` and
`https://robinhood.drpc.org` (both answer `eth_chainId` = 4663; head ≈ 70,250,043). Nothing was sent.

| contract | address | has code | identifies itself by | found via |
|---|---|---:|---|---|
| PoolManager | `0x8366a39CC670B4001A1121B8F6A443A643e40951` | 24,009 B | `owner()` = `0x2BAD…46CD`; `protocolFeeController()` = `0x6d00…314c`, whose `POOL_MANAGER()` points back here; `StateView.getSlot0` of a live pool reads through it | Pons V2 factory `0x7eD5…EC7e`.`poolManager()`; Uniswap's v4 deployments page ("Robinhood Chain: 4663"); already in `tools/snapshot/exclusions.json` |
| PositionManager | `0x58daec3116aae6D93017bAAea7749052E8a04fA7` | 23,877 B | `name()` = "Uniswap v4 Positions NFT", `symbol()` = "UNI-V4-POSM", `poolManager()` = the PoolManager, `permit2()` = Permit2, `WETH9()` = `0x0Bd7…AD73`, `nextTokenId()` = 3,146,765 | Pons factory `.positionManager()`; Uniswap deployments page |
| StateView | `0xf3334192d15450cdd385c8b70e03f9a6bd9e673b` | 3,531 B | `poolManager()` = the PoolManager; `getSlot0(poolId)` of position 3,146,764's pool returned tick 376,022, lpFee 200,000, liquidity 7.7e16 | Uniswap deployments page |
| Permit2 | `0x000000000022D473030F116dDEE9F6B43aC78BA3` | 9,152 B | `DOMAIN_SEPARATOR()` = `0x4486…fad`, equal to `keccak256(EIP712Domain("Permit2", 4663, self))` recomputed locally | Pons factory `.permit2()`; PositionManager `.permit2()`; deployments page |
| UniversalRouter | `0x8876789976decbfcbbbe364623c63652db8c0904` | 24,546 B | `poolManager()` = the PoolManager | deployments page ("Universal Router") |
| UniversalRouter 2.1.2 | `0x204FAca1764B154221e35c0d20aBb3c525710498` | 24,380 B | `poolManager()` = the PoolManager | deployments page; Uniswap's Trading API lists it for 4663 |
| V4Quoter | `0x8dc178efb8111bb0973dd9d722ebeff267c98f94` | 6,118 B | `poolManager()` = the PoolManager | deployments page |
| PositionDescriptor | `0x9639443158e8c5efa35bd45287bf2effd3d8dc06` | 752 B (proxy) | `wrappedNative()` = WETH9, `nativeCurrencyLabel()` = "ETH"; PositionManager `.tokenDescriptor()` | deployments page |

Sources: https://developers.uniswap.org/llms.mdx/docs/protocols/v4/deployments (the "Robinhood Chain: 4663" table lists
exactly these), and the Pons V2 factory's own getters, which is the contract our earlier fork tests already graduate through.
The PoolManager's runtime code is identical on both RPCs (codehash `0xbd38…5626`); `test/fixtures/v4-pool-manager-4663.hex`
is that code, which the unit tests etch at the same address so they run against the real PoolManager without a network.

**A measured cost nobody mentioned: the v4 protocol fee is ON on 4663.** The controller's `getFee(key)` returns, for a hookless
pool, `0x3E83E8` at a 1% LP fee (0.10% each direction), `0x1F41F4` at 0.30% (0.05%) and `0x07D07D` at 0.05% (0.0125%), and
the live pool above carries `0x3E83E8`. v4 takes the protocol fee first and the LP fee on the rest, so a trade in a 1% pool
pays ≈1.099%, of which 1% reaches LPs. `triggerFeeUpdate(key)` is permissionless, so this applies to our pools whether or not
anybody calls it at initialization. Our splits are of what the LPs earn.

## 2. Decisions in one table

| question | decision | why, in one line |
|---|---|---|
| hooks | **none, on either pool** | aggregators route hookless v4 pools by default; hooked pools with custom accounting are skipped unless allowlisted (section 7) |
| COIN/wFROG fee | the pool's own **1% LP fee**; the lock's full-range position collects it | the only hookless way to charge a trade fee; split afterwards |
| wFROG/ETH pool | **one per collection**, shared by all its coins; fee 0.30%, tick spacing 60 | every graduation deepens the same book; a low tier is what routers pick |
| layout | **layered** protocol-owned positions: 50% in ±10%, 30% in ±30%, 20% full range (parameters) | deep at the floor, never empty |
| ETH the bands cannot pair | a **bid** band just under the ±10% band, ETH-only | "the unused buffer becomes a bid under wFROG" (curve.md) |
| recentering | **no liquidity is ever moved**; the *center for new liquidity* follows the keeper's floor cap, at most one band-width a day | moving liquidity needs a decrease path and a one-sided re-add; not moving keeps "no decrease code at all" |
| add-time price guard | a graduation's shared-pool add happens only while the pool price is within ±10% of the fresh keeper cap; otherwise it waits, per coin, and anyone may retry | the only manipulable moment is the add; nothing is ever lost by waiting |
| withdraw | **no function decreases liquidity**; the only `modifyLiquidity` calls are adds and zero-delta fee pokes | "permanently locked" is then a property of the bytecode, asserted by an ABI test |
| shared-pool fees | **seeder shares**: each coin's units in each position = the liquidity it added; fees pro rata to its sweep vault, forever | early coins earn from every later coin's routing; no token, no transfer |
| coin-pool fees | quote side (wFROG or ETH) split vault 50 / creator 30 / platform 20; coin side split creator / platform, the vault's share burned | the vault holds only wFROG/ETH; converting coins on-chain without an oracle is sandwichable |
| targets | **in ETH** (5 ETH normal), independent of the floor; the item budget goes into the collection's **buy queue** | a 5 ETH Ape and a 0.63 ETH frog share one code path |
| buy queue | per collection, **FIFO by graduation order**; each item bought is split pro rata to the ETH that funded it; a coin's remainder rolls into the next item, credited to it | blue chips cost more than one coin raises; nothing is socialized and no coin can be starved by later ones |
| bootstrap | 8 ETH (hard max 10) while *this collection's* shared pool has received < 2 ETH; and while it has no shared pool at all, at least one item + the pool side | no protocol money; the first one or two coins pay for the depth, and are paid back by seeder shares |
| expensive collections | the queue is the default; **SWEEP_ONLY** is an optional creator choice (COIN/ETH pool, fees sweep items later) | a lone coin on an expensive collection has a path that needs no item at all |
| routing | `EtchRouter`: ETH → wFROG → COIN and back in one transaction; no owner, no custody | anyone can trade without our site; aggregators can do the same through the Universal Router |

## 3. The lock: one contract that owns every position and has no way out

`LiquidityLock` owns its positions **directly in the PoolManager** (owner = the lock, per-position salt), not through the
PositionManager. There is no NFT, so there is nothing to transfer, approve or lose, and no Permit2. PositionManager, Permit2
and the Universal Router are recorded above for routers and the page, not used by the lock.

What it can call on the PoolManager: `initialize`, `unlock`, and inside its callback `modifyLiquidity` with a **positive**
delta (an add) or a **zero** delta (a fee poke), `swap` only to align a pool's price before an add (3a, 3c), `sync`/`settle`/`take`.
There is no function that passes a negative delta, and the only addresses that ever receive tokens from it are the PoolManager
and the curves of the coins it seeded (their fees). The test suite reads the compiled ABI to pin its external surface.

The lock is **per collection** (one wrapped token). A second collection gets its own lock, its own shared pool, its own seeder
shares and its own bootstrap state.

Wiring: the lock is deployed first; the collection's `CurveFactory` is deployed naming it; the deployer calls `lock.wire(factory)`
once. `seed` then accepts only `factory.routerOf(coin) == msg.sender`, i.e. a coin's own curve. Without that check anyone could
initialize a coin's pool before its graduation at a price of their choosing.

### 3a. The coin's own pool (COIN/wFROG, or COIN/ETH for SWEEP_ONLY)

Hookless, LP fee = the curve's `feeBps` (1%), tick spacing 200, **one full-range position per coin** holding the reserved supply
and the coin's `W − w_s` wrapped units (section 5). Full range because it must never go out of range for a memecoin that can move 100×.

**Somebody may initialize this key before graduation** (it is hookless; anyone can). The lock handles it without trusting
anyone: if the pool exists at the wrong price, the lock swaps *toward* the seed price with a price limit at the seed price,
spending at most what it is seeding. Every unit it buys on the way is bought at a price better than the seed price, so an
attacker who pre-initialized and added liquidity only sells to us below our own valuation; one who initialized without
liquidity moves nothing (the swap crosses empty ticks for free). Then it adds at whatever price results. Coin dust left over
stays in the lock, i.e. is burned; wFROG left over joins the coin's shared-pool contribution.

### 3b. The shared pool (wFROG/ETH), layered

One pool per collection: `currency0` = native ETH, `currency1` = wFROG, fee 0.30%, tick spacing 60, no hook. (docs/v3/sweep.md
had chosen 1% to blunt sandwiches of the vault's own buys; those buys are capped by the keeper's price anyway, and a 0.30% pool
is what a router takes as a hop. Parameter.)

Positions are *layers* around a center tick `c`, each a separate v4 position:

| layer | range (ticks, spacing 60) | ≈ price range | share of each graduation's pair |
|---|---|---|---:|
| near | `c ± 960` | floor ±10% | 50% |
| wide | `c ± 2640` | floor ±30% | 30% |
| full | min … max usable tick | everything | 20% |
| bid | `c + 960 … c + 2880` | wFROG 10%–25% under the floor, ETH only | whatever ETH is left |

(Tick up = more wFROG per ETH = wFROG cheaper, because ETH is `currency0`. So an ETH-only position sits *above* the current
tick, and is a bid for wFROG as it falls.)

A graduation's shared contribution — `w_s` wrapped units and `poolEth + dust` ETH (section 5) — is added layer by layer: each layer gets its
weight of what is left, adds the most liquidity those amounts allow at the current price, and passes the rest on. The full-range
layer takes what the bands could not pair. Whatever ETH remains goes into the bid, whatever cannot be placed (dust, or wFROG
when the price is outside every band) waits as the coin's `pending` and is added by the next `deploy(coin)`.

**"Every graduation adds to the same position"**: while the center is unchanged, every graduation adds liquidity to the same four
v4 positions (same ticks, same salt). A center move (below) opens a new set for *new* liquidity; old positions stay where they
are, still locked, still earning.

### 3c. The only manipulable moment is the add, so it is guarded

Adding liquidity at a price somebody pushed is the one way to lose money here: they move the price, we add at it, they move it
back through our fresh liquidity. So `deploy(coin)` adds only when:

1. the keeper's floor cap for this collection is fresh on both clocks (the cap the sweep vaults already use: posted by the
   keeper, one-hour TTL, `lib/Wait.sol`), and
2. the pool's current tick is within `maxDeviation` (1,000 ticks ≈ 10%) of the cap's tick.

Otherwise the contribution waits in `pending[coin]`, and anyone may call `deploy(coin)` later. Graduation never reverts because of
the pool, so nobody can block a graduation by moving the price; they can only delay the shared-pool add, at the cost of holding
the price off the floor against arbitrage.

On the **first** add, the pool may not exist (the lock initializes it at the cap's price) or may have been initialized by
someone else. If the pool has **no in-range liquidity**, the lock swaps one wei with a price limit at the cap's price, which
crosses empty ticks for free, and re-checks. It never trades real amounts against other people's liquidity in the shared pool.

### 3d. Recentering: the center moves, the liquidity does not

Inscription floors and ETH/BTC can move 20–30% in a day. Moving a concentrated position to follow them means removing it (a
decrease path, the one thing this contract must not have) and re-adding it, and a band that went out of range is 100% one
token, so it cannot be re-added around the new price without a swap. Both are worse than the alternative:

- **old positions never move.** A band the price left behind is a standing bid (if wFROG rose) or ask (if it fell) at the old
  levels, still locked, and back in play if the price returns. The full-range 20% is always in range.
- **the center for new liquidity follows the keeper's cap.** `moveCenter()` is permissionless: with a fresh cap whose tick is
  more than half a near-band (480 ticks) from the center, the center moves toward it by at most `maxShift` (960 ticks ≈ 10%),
  at most once per day on both clocks. The bound is on how fast *new* liquidity can be steered, which is the only thing a bad
  cap could steer. A cap cannot move old liquidity at all, and new liquidity is added only at a pool price within 10% of it
  (3c), so a wrong center costs depth in the wrong place, never tokens at a wrong price.

The "max X% of the position per day" bound the brief asked for is therefore 0%: no liquidity moves.

## 4. Fees

### 4a. Shared pool: seeder shares

Each shared position keeps, per coin, `units` = the liquidity that coin's contributions added to it. Because no liquidity is ever
removed and every add is attributed to exactly one coin, **`sum(units) == position liquidity` exactly**, forever.

Fees are collected by a zero-delta `modifyLiquidity` (a poke) and credited per position with a Q128 accumulator, the way v4
itself credits LPs: `acc += fees · 2¹²⁸ / liquidity`, a coin's due is `units · (acc − its checkpoint) / 2¹²⁸`. Every add pokes
the position **first**, so fees earned before a coin arrived are credited before its units exist: a later coin cannot dilute an
earlier coin's accrued fees. `claimShared(coin)` (anyone) pays a coin's due, ETH and wFROG, to its curve, which deposits all
of it into the coin's sweep vault (`SweepVault.deposit` / `depositWrapped`, where the curve is the coin's router). 100% to the
vault: these are the coin's holders' returns on the liquidity their raise paid for. Rounding leaves at most a few wei per
position per claim in the lock, credited to nobody.

### 4b. The coin's own pool

`collectCoinFees(coin)` (anyone) pokes the coin's full-range position and hands everything to the coin's curve:

- the quote side (wFROG, or ETH for SWEEP_ONLY) is split exactly like the curve's trade fees: 50% to the sweep vault, 30% to
  the creator, 20% to the platform (`vaultShareBps`, `creatorShareBps`, the rest);
- the coin side: 30% to the creator, 20% to the platform, and the vault's 50% is **burned** (sent to `0x…dEaD`). The vault can
  hold only wFROG and ETH, and turning coins into wFROG on-chain without an oracle is a swap anyone can sandwich. Burning returns
  that value to every holder pro rata, which is what the vault's raffle and burn are for.

Third parties may add liquidity to either pool and earn their share of its LP fee; that is depth for free, and the fees our
positions earn are split as above.

## 5. Targets, the buy queue, and the bootstrap (all per collection)

**The raise is ETH, and it is split at the close** (bps of the raise E, parameters):

| part | default | goes to |
|---|---:|---|
| platform graduation fee | 5%, **capped at 0.25 ETH** | the platform, on graduation only. A bigger target never means a bigger platform cut |
| shared-pool ETH side | 12.6% | paired with the wrapped units the coin's budget bought, in the collection's shared pool |
| item budget | the rest (≈82.4%) | the collection's buy queue, credited to this coin |

**The queue.** `ItemQueue` (one per collection) holds every graduating coin's item budget in graduation order. Whenever it holds
at least the keeper's cap plus a 5% buffer, the canister buys one item (never above the cap, or a snapshot of it taken within two
hours), wraps it to the queue and calls `recordItem(cost)`. The queue pays `cost` to the canister's payee and **funds it FIFO**: from the
oldest unfinished coin's remaining ETH first, then the next. The item's `1e18` units are split **pro rata to the ETH each coin
put into it**, the last contributor taking the rounding, so the units assigned are exactly `items × 1e18`. A coin whose remaining
ETH falls to at most its **tail** — the larger of 0.001 ETH and 10% of what it deposited — is **filled**; its tail goes to its
shared-pool ETH side (so it stays the coin's, earning seeder fees), never to anybody else. (Built this way rather than with an
absolute dust only: at 0.64 a frog, a 4.12 ETH budget leaves 0.28 after six frogs, and a lone coin would otherwise wait for
another coin to fund a seventh it needs a sliver of.) A remainder above the tail and below one item waits for the next coin's ETH
and is never moved to another coin.

- **No starvation:** a later coin's ETH is never spent while an earlier coin still has more than dust; an earlier coin can only
  be finished sooner by later ones.
- **Completion:** once filled, `graduate()` (anyone) takes the coin's units W and tail from the queue. The coin's own pool gets
  the units that cost `itemBudget − poolEth` (`W · (itemBudget − poolEth) / spent`), so it opens exactly at the curve's last
  price valued at cost; the shared pool gets the rest of the units with `poolEth + tail` ETH.
- **Deadline:** a coin not filled when its window passes (both clocks) is cancelled in the queue: its unconverted ETH and any
  units already assigned come back to its curve, and holders refund pro rata (ETH and units), as before.

**The curve's end price** is solved against value, not items: the coin's pool opens with units that cost
`itemBudget − poolEth` ETH, so `P = (itemBudget − poolEth) / R` ETH per coin at acquisition cost, whatever the item costs.

**Targets.** Normal: 5 ETH. **Early** (this collection's shared pool has received less than `earlyUntilEth` = 2 ETH of ETH side
from graduations): 8 ETH, hard maximum 10 ETH in the constructor. **Opening** (the collection has no shared liquidity at all
yet): at least `(cap · 1.05 + 0.25) / (1 − 12.6%)`, i.e. one item plus the pool side and the fee, so a lone first coin on an
expensive collection can always buy one item on its own. The target is fixed in the coin's curve at launch and public. A second
collection starts early and opening regardless of the first one's depth. Early coins pay more and hold larger seeder shares.

**SWEEP_ONLY** (a creator's choice at launch): the curve raises the normal target, graduates at once into a COIN/ETH pool
(reserved supply + the raise less the capped fee), buys nothing, and its fees accumulate in the sweep vault until they can
afford an item (the vault's desk route), which is raffled or burned. No queue, no shared pool, no seeder shares.

## 6. The arbitrage band, honestly

- **Minting** (public mint desk or a holder deposit) costs the floor plus ≈2% ETH→BTC conversion plus the marketplace fee
  (0–2.5%) plus Bitcoin network fees, and takes about an hour (purchase, six confirmations, the mint).
- **Redeeming** burns one wFROG for a **random** frog (drand), delivered by the custodian within hours; a random frog is worth at
  least the floor.

So wFROG should trade within roughly **−3% … +5% of the floor** in calm markets, with **hours of lag** after a big move of the
floor or of ETH/BTC, during which only the pool's depth absorbs flow. Nothing in the contracts enforces the band; arbitrageurs
do, and they need inventory on both chains to do it fast.

## 7. Routing by third parties

`EtchRouter` (no owner, holds nothing between transactions): `buy(coin, minOut, to, deadline)` payable and
`sell(coin, amountIn, minEthOut, to, deadline)`; for a GRADUATION_BUYS coin both legs (ETH ↔ wFROG ↔ COIN) happen in one
`unlock` with the wFROG never leaving the PoolManager; for a SWEEP_ONLY coin it is one hop. Anyone can call it; aggregators can
do the same through the Universal Router.

**Who routes 4663 today** (checked read-only 2026-09-23 against each one's public docs or unauthenticated API):

| router / aggregator | on 4663 | v4 pools | hooked v4 pools |
|---|---|---|---|
| Uniswap (interface, Trading API, UR 2.1.2) | yes (supported-chains page) | yes | only allowlisted hooks, or hooks without swap-delta permissions on non-major pairs (`uniroute-public`) |
| KyberSwap | yes (chain config + a live quote) | yes | only hooks it has a handler for (pons-v2, fables, …); unknown swap hooks are likely skipped |
| Velora / ParaSwap | yes (live price route) | yes (a hookless one seen) | not seen; ETH→PONS avoided the hooked pool |
| OpenOcean | yes (dex list) | yes | named families only |
| 1inch | yes (announced 2026-07-07; API needs a key) | unknown | unknown |
| 0x / Matcha | yes (RFQ launch; long-tail tokens need listing) | not documented | unknown |
| LI.FI, Jumper, Relay, deBridge | yes (via Kyber/OpenOcean/1inch/"nordstern") | via those | via those |
| Enso | yes (docs) | unknown | unknown |
| Odos | no (shut down) | — | — |

**What makes wFROG a hop** (Uniswap's routing source; others are best-price graph searches with no published thresholds): a
hookless wFROG/ETH pool deep enough to be among the top pools for wFROG by TVL, and each coin's deepest pool being COIN/wFROG,
so the router takes wFROG as the intermediary; Uniswap keeps only the top 10 v4 pools per pair and fee tier. Depth is the lever,
which is why the layout concentrates it at the floor and why the bootstrap exists.

## 8. What this does NOT protect against

- **The keeper's cap.** It bounds what the queue pays per item, sets the opening target, gates shared-pool adds and steers the
  center. A wrong cap lets the canister pay up to it for an item and puts new depth in the wrong place; it cannot move
  locked liquidity or make the lock add at a pool price more than 10% from itself.
- **A stale keeper.** Item purchases, opening launches and shared-pool adds wait for a fresh cap. Refunds do not.
- **The canister's reported cost per item** is its word, bounded by the fresh cap plus the buffer, as `recordFrog` was.
- **A slow queue.** On an expensive collection, a coin whose remainder is below one item waits for later coins; if none come
  before its window ends, it refunds. The opening rule makes the first coin self-sufficient; later ones are not.
- **Third-party LPs** earn their share of either pool's LP fee; our share of fees falls as they add.
- **Uniswap's protocol fee** (0.1% on a 1% pool, 0.05% on 0.3%) is charged on top and is Uniswap's to change.
- **A sandwich of the coin-pool alignment** can only sell to the lock below the seed price (3a); a sandwich of a shared-pool add
  is bounded by the 10% guard and by the depth already there.

## 9. Built (the same day, with the code)

Commits on `v4-pools`, rebased onto main `d94b80e`: this document (before the code); `lib/Wait.sol` as deployment config;
the queue, the curve and factory changes, the lock, the router, the directory; the queue and pool test suites; the fork test;
the docs. **`forge test --no-match-path "test/fork/*"`: all green** (see the final count in the hand-off); the fork suites
(`test/fork/*`, 16 tests, 2 of them new) green against a local anvil fork of 4663 on port 8621.

What changed from the design above while building it, each on purpose:

- **The canister's interface** (`autobuyer/canister-rs/src/buyqueue.rs`) is implemented with its signatures; the differences
  (what `queued` counts, the balance check, the cap snapshot, the tail) are listed in docs/v3/curve.md, "The queue as built".
- **The tail rule** (above) replaced a dust-only completion.
- **Seeder units are v4 liquidity**: no liquidity is removed and every add is attributed, so `sum(units) == liquidity` holds
  exactly, and the invariant suite checks it against the PoolManager's own position records.
- **Coin-pool alignment** spends at most half of one side toward the seed price; the shared pool is aligned only across empty
  ticks (one wei), never against someone's liquidity.
- **The collection directory** (`CollectionDirectory`) and `reclaimStray()` on the desk and the queue close gaps the canister
  found; `Wrapper` implements `IWrappedCollection` (`redeemToBurn`, `refunded`, `BURN_ADDRESS` moved from the vault), and the
  vault, desk, curve, queue and lock are typed against it — the same path runs over `NftWrapper` in `test/pools/Collections.t.sol`.

What the fork test showed on the real PoolManager: the recorded addresses answer as in §1; a curve graduates into it (the coin
pool's full-range position and the four shared layers read back from the manager's own storage); after Uniswap's permissionless
`triggerFeeUpdate` the protocol fee is **0x3E83E8 on the 1% coin pool and 0x1F41F4 on the 0.3% shared pool**; our router
round-trips (1 ETH → ALIEN → 0.98 ETH at that depth: large trades lose less than the four fee legs because the shortfall sells at
the cheap end of the moved curve; small ones lose exactly 2.58%); coin-pool and seeder fees reach the vault.

