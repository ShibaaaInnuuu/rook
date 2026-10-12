# The floor feed and the canister's netting trigger (Floor Module, v5)

Written 2026-09-23 **before its code**, on branch `floor-feed` (from `9022920`). The Solidity of the Floor Module is being written by
another agent on branch `floor-module`; this branch does not touch Solidity. Everything this side needs from that contract is
stated once, in §5, as an **assumed interface**; the contracts agent reconciles names and arguments against it.

Nothing here is deployed. No transaction was sent to any public chain; the only network use was read-only, keyless GETs and
`eth_getLogs` / `eth_getTransactionReceipt` against public endpoints, recorded into `keeper/fixtures/floor/`.

## 0. The module, as this side understands it

Per collection (Bitcoin Frogs, Bored Apes, …) the Floor Module lets anyone:

- **mint** wX with ETH, instantly, at `floor × (1 + spread)`; the ETH goes to the module's escrow;
- **burn** wX for ETH from the escrow, instantly, at `floor × (1 − spread)`.

The keeper **posts** `floor` (TTL 1 h; the chain refuses a move larger than `maxMoveBps` per hour; the spread widens as the price
ages or its confidence falls). The canister **buys a real item** only when the NET ETH-backed demand — units minted for ETH, less
units burned for ETH, less items already bought — reaches one whole item plus a buffer. It then wraps the item to the module and
calls the module's "item arrived / claim cost" function, which pays the canister the item's cost out of the escrow.

So two things must be hard to manipulate: the **posted floor** (a wrong floor lets someone mint cheap or burn dear against the
escrow) and the **trigger** (the canister must not buy on noise, on a stale price, or above what one collection's items cost).

## 1. The robust floor

### 1.1 Inputs, per collection

| input | Bitcoin inscriptions (frogs) | EVM NFTs (Ethereum, Base) |
|---|---|---|
| listings | marketplace books: Ordinals Wallet (keyless), Magic Eden (keyless; see §3) | OpenSea v2 best listings (`OPENSEA_API_KEY`), Seaport orders |
| per-listing facts | seller address, price (sats), listed-at, the outpoint, and **ord's contents of that outpoint** | seller, price (wei), listed-at, expiry, offer items, currency, zone, and whether the order is still fillable |
| listing events | list / delist per wallet with timestamps (a source's activity feed, or the keeper's own diff of successive snapshots) | the same |
| sales | txid, time, seller, buyer, price, items in the transaction | txid, time, seller, buyer, price, NFTs moved in the transaction |
| conversion | sats → wei at a reference ETH/BTC rate (the module is priced in ETH) | — |

Everything is normalised to one `Listing` / `Sale` shape in `keeper/src/floor.ts`; the statistic below never knows which chain
it is on.

### 1.2 Exclusion rules (each has an id, a test and a fixture)

A listing or sale excluded by any rule is reported with the rule's id, so a journal line says why every number was or was not
used.

**Listings**

| id | rule | why |
|---|---|---|
| `L-unverified` | a Bitcoin listing whose outpoint's contents ord has not answered for | fail closed: we cannot tell what is being sold |
| `L-extras` | the outpoint carries anything but exactly the one inscription (another inscription, any rune); an EVM order that offers more than one item, or any non-ERC-721 item | a bundle's price is not one item's price, and the canister refuses to buy these anyway (`frogbuy::judge`, `seaport::verify`) |
| `L-currency` | an EVM listing not priced in the chain's native ETH, or a criteria / private order | not fillable by the canister |
| `L-unfillable` | the source or the chain says it is not fillable now (Bitcoin: the outpoint is spent; EVM: cancelled, filled, the owner is not the offerer) | a price nobody can pay is not a floor |
| `L-stale` | the fillability check is older than `staleS` (15 min), or the source snapshot itself is older than that; an EVM listing expiring within `expiryMarginS` (30 min) | old facts are how a floor lags a delisting |
| `L-unseasoned` | listed less than `seasoningS` (30 min) ago | a flash listing must survive half an hour of snipers before it moves the floor |
| `L-burst` | the seller (or its cluster, §1.3) made ≥ 3 delists, or ≥ 6 list+delist events, in the last hour; it stays excluded for `burstCooldownS` (6 h) after its last event | list-delist cycling is how a floor is walked without a fill |
| `L-seller-dup` | a second listing from the same seller cluster (only its cheapest counts) | one wallet (or one person's wallets) is one vote |

**Sales**

| id | rule | why |
|---|---|---|
| `S-window` | older than `saleWindowS` (7 days) | |
| `S-self` | buyer = seller | a transfer dressed as a sale |
| `S-wash` | buyer and seller in the same cluster (§1.3) | wash trading |
| `S-bundle` | the transaction moved more than one item of the collection | the per-item price is unknowable |
| `S-outlier` | price outside `[L/4, 4·L]` (L the listing floor) | a 1-sat "sale" or a fat-finger |
| `S-repeat` | a second sale by the same buyer or the same seller in the window (the latest counts) | one active wallet is one vote |

### 1.3 Wallet clusters

Union–find over addresses. Two addresses are joined when, among the sales in the window plus 30 days before it:

1. they traded with each other **in both directions** (A sold to B and B sold to A, any tokens of the collection);
2. one sold to the other at under 25% of the listing floor (value moved without being paid for);
3. a source names them as linked (an explicit hint, e.g. a common funder; optional).

Clusters only ever remove data (a wash sale, a duplicate listing); a false link costs a data point, never adds one.

### 1.4 The statistic

With `K = 5`:

- **Listing floor `L`** = the median of the lowest `K` listings that survive every listing rule, one per seller cluster. Fewer
  than 3 survivors: **no floor**; nothing is posted and the posted price ages out (the module widens, then stops).
- **Sale floor `S`** = the median of the last 20 sales that survive every sale rule. Fewer than 3: no `S`.
- **Floor** = `L` when there is no `S`; otherwise `min(L, (7·L + 3·S) / 10)`.

The blend lets recent sales pull the floor **down** by at most 30% of the gap (a collection that is listed at 0.565 BTC and
sells at 0.37 is not really a 0.565 collection), but never **up**: anything above `L` can be bought at `L`, so a sale above the
listing floor says nothing the book does not. That also makes an upward wash-sale attack useless, whatever the clustering misses.

Arithmetic is integer (sats or wei), medians of an even count take the lower middle, and the result is rounded down.

### 1.5 Confidence and depth

`confidenceBps` (0–10,000) is the product of four factors, each in bps:

| factor | value |
|---|---|
| count | `min(n, K) / K`, n = listings in the lowest-K after exclusions |
| spread of the lowest K | `max(0.25, 1 − (max − min) / (2 · L))` |
| sales | 1 with ≥ 5 valid sales, 0.8 with 3–4, 0.6 with fewer |
| agreement | `max(0.5, 1 − |L − S| / L)`, 1 when there is no `S` |

The keeper **does not post** below `minConfidenceBps` (default 3,000): the price then ages, and the module widens its spread and,
past the TTL, stops, which is the safe failure.

**Depth** is two figures per collection: `depthItems` = how many valid listings sit at or under `floor × 1.10`, and `upCost10`
= what it costs (gross, in the collection's unit) to buy listings from the bottom until the median of the lowest K rises 10%
(§4). The keeper posts `depthItems`; the module may use it to bound how far net minting runs ahead of what can be bought
(§4.4).

### 1.6 Parameters

| name | default | where |
|---|---|---|
| `K` | 5 | `floor.ts` `FLOOR_PARAMS` |
| `minListings` | 3 | |
| `seasoningS` | 1,800 | |
| `staleS` | 900 | |
| `expiryMarginS` | 1,800 | |
| `burstWindowS` / `burstDelists` / `burstEvents` / `burstCooldownS` | 3,600 / 3 / 6 / 21,600 | |
| `saleWindowS` / `maxSales` / `minSales` | 604,800 / 20 / 3 | |
| `blendListingTenths` | 7 | |
| `minConfidenceBps` | 3,000 | `floorpost.ts` |
| `refreshBeforeExpiryS` | 900 | re-post an unchanged floor 15 min before its TTL ends |
| `repostMovedBps` | 100 | re-post when the target moved more than 1% |

## 2. Where the keeper gets the facts

- **Ordinals Wallet** (`turbo.ordinalswallet.com/collection/<slug>/escrows` and `/sales`): keyless GETs, read-only, one request
  per `everyMs`. Listings carry seller, price, outpoint and creation time; sales carry buyer, seller, price and
  `inscriptions_in_tx` (which is what `S-bundle` reads).
- **Magic Eden** (`/v2/ord/btc/stat`): keyless; answered 2026-09-23 (`floorPrice 480,000`, `totalListed 1,010`). Its
  `/v2/ord/btc/tokens` and `/activities` answered **503 "no healthy upstream"** on every try that day, so Magic Eden's ladder is not
  in the measurement below; the source is coded against the documented shape and marked unverified, as `MagicEdenSource` already is.
- **ord** for `L-unverified` / `L-extras`: `ordinals.com` answered **"JSON API disabled"** on 2026-09-23 for `/output/<outpoint>`
  with `Accept: application/json`. So the keeper's ord reader is an interface with a configurable base URL (a self-hosted ord);
  without one, every Bitcoin listing is `L-unverified` and no Bitcoin floor is posted. (The canister reads the same host in
  `custodynet.rs`; that is the canister's own open problem, noted in §8.)
- **OpenSea v2** for EVM listings and sales: `GET /api/v2/listings/collection/<slug>/best` and `GET
  /api/v2/events/collection/<slug>?event_type=sale`, header `x-api-key` read from `process.env.OPENSEA_API_KEY` **at the moment of
  the call**, never at load, never logged (`log.ts` scrubs the value from every line), never put in an error. Tests use a fake key
  and a mocked `fetch`, and assert the key appears in no log line and no error.
- **Seaport on chain**, keyless: `OrderFulfilled` in the receipts of a collection's `Transfer`s. Used for the measurement below
  (`xrpc.cl/eth`; `eth.drpc.org`'s free plan refused every range, `publicnode` wants a token for logs). Not used by the keeper's
  live path, whose read-only RPC allow-list has no `eth_getTransactionReceipt`.
- **The rate** for sats → wei: an interface (`RateSource`); a fixture in tests. Measured for §4: Coinbase's keyless
  exchange-rates endpoint, 0.031858 BTC per ETH, i.e. **3,185,800 sats per ETH**.

## 3. Measurements (2026-09-23, read-only)

Recorded in `keeper/fixtures/floor/`:

- `ow-bitcoin-frogs-listings-2026-09-23.json` — 79 Bitcoin Frog listings on Ordinals Wallet (oldest 2023-04-25, newest
  2026-09-22). Ord contents not verified (ordinals.com's JSON API is disabled).
- `ow-bitcoin-frogs-sales-2026-09-23.json` — the 85 most recent frog sales (2026-08-18 → 2026-09-23).
- `eth-bayc-seaport-2026-09-23.json` — every BAYC `Transfer` over 50,400 Ethereum blocks (~7 days, head 26,039,783), each priced
  from its transaction's Seaport `OrderFulfilled` where there was one: **199 transfers, 50 Seaport-priced**, 25 of them listing
  fills. The other 149 are Blur fills, wallet moves or mints, which this source cannot price.

### 3.1 Bitcoin Frogs

The ladder (lowest listing per seller, seasoned): **433,069 · 565,262 · 565,262 · 606,372 · 693,730** · 1,027,749 · 1,233,299 ·
1,541,623 · … sats. `L` = **565,262 sats**. The book is thin: the sixth seller asks 1.8× the fifth.

Sales, last 7 days: 10. Excluded: 1 `S-bundle` (5 inscriptions in one transaction), 1 `S-repeat`. In the 30 days recorded the
rules also catch what one would hope: two `S-self` sales (`bc1pwwj0…` to itself at 324,000; `bc1puszf…` to itself at 317,000), a
pair `34hRVyQe…` → `bc1pj3a2…` that "sold" one frog at **8,284 sats** (`S-outlier`, and a cluster link by rule 2) and another at
862,000, and a 2-inscription sale. Valid: 8, `S` = **371,165 sats** (334,000 … 420,000).

**Floor** = `min(565,262, (7 × 565,262 + 3 × 371,165) / 10)` = **507,033 sats = 0.1592 ETH** at 3,185,800 sats/ETH. (Magic Eden's
stat floor the same hour: 480,000.) `depthItems` (valid listings ≤ floor × 1.10 = 557,736): **1**.

The 0.63 ETH frog in docs/v3/curve.md was an assumption at a 0.02 BTC floor; the measured floor is ~0.005 BTC.

### 3.2 Bored Apes

No keyless source serves BAYC **listings** (OpenSea and Blur need keys; Magic Eden's EVM API answered "Not Found"). So the ladder
below is a **proxy, labelled as such**: the prices at which BAYC listings were actually *filled* on Seaport in the week, sorted —
6.0291 · 6.0687 · 6.0984 · 6.2964 · 6.43 · 6.47 · 6.6033 · 6.649 · 6.66 · 6.66 · 6.6825 · 6.70 · 6.7171 · 6.8805 · 6.9201 · 7.15 ·
7.2 · 7.8 · 8.0 · … ETH. `L` (proxy) = **6.0984 ETH**. A keyed read replaces it in the first run with a key; the code path is the
same.

Sales: 50 priced; excluded 8 `S-bundle` (multi-NFT transactions: one sweep moved 8641 three times at the same 7.02), 4 `S-wash`
(both-direction pairs: `0xec78…` and `0xf762…` trade tokens 1183, 3922, 4795 back and forth), 1 `S-outlier` (a **0 ETH** "sale" of
5762), 18 `S-repeat` (bid bots). Valid: 19, `S` = **6.66 ETH**, above `L`, so **floor = L = 6.0984 ETH**.

## 4. Manipulation: what it costs to move the posted floor X%

Two levers exist: the **book** (listings) and **sales**. Sales can only pull the floor down (§1.4), and only through sales the
rules keep — real sales to unrelated wallets at a discount, which cost the seller the discount.

### 4.1 Up: buy the bottom of the book

To raise the median of the lowest five by X%, buy listings from the bottom until it is X% higher. The attacker keeps the items,
so the **capital** is large and the **loss** is fees, slippage and the carry — unless the market lists into the gap first.

| collection | +5% | +10% | +25% |
|---|---|---|---|
| Frogs (OW book, measured) | 1 item, 433,069 sats = **0.136 ETH** | 2 items, 998,331 sats = **0.313 ETH** | 3 items, 1,563,593 sats = **0.491 ETH** (it jumps to +82%) |
| BAYC (proxy ladder) | 2 items, **12.1 ETH** | 10 items, **64.0 ETH** | 15 items, **97.9 ETH** |

The posted floor moves 70% of `L`'s move while `S < L` (frogs), and 100% of it when `S ≥ L` (BAYC).

### 4.2 Down: list cheap

To lower the median of the lowest five to `q = L(1 − X)`, the attacker needs `3 − c` seasoned listings at or under `q`, from
**distinct, unlinked wallets** (`L-seller-dup`), none of which may have cycled listings in the last hour (`L-burst`), each alive
for 30 minutes before it counts (`L-unseasoned`), and each **fillable** — so each can be sniped at `q`, a loss of `X·L` per item.
`c` is the number of honest listings already under `q`.

| collection | −5% | −10% | −25% |
|---|---|---|---|
| Frogs | 2 listings; if sniped, 56,526 sats = **0.018 ETH** | 2; 113,052 sats = **0.035 ETH** | 3; 423,947 sats = **0.133 ETH** |
| BAYC (proxy) | 3; **0.91 ETH** | 3; **1.83 ETH** | 3; **4.57 ETH** |

(Frogs: the posted floor moves 70% of `L`'s move, so −10% posted needs −12.8% on `L`: still 2 listings, **0.045 ETH**.)

### 4.3 Time: the chain's max move per hour, and the spread

A wrong floor pays only if it beats the round trip: minting and burning each cost `spread`, so a deviation of X pays about
`X − 2·spread` per unit. The module lets the posted price move at most `m` per hour, and the keeper never posts a jump the chain
would refuse (it steps, §5.2). So an attacker must hold the book distorted for at least `ln(1 + 2·spread) / ln(1 + m)` hours
before breaking even — **4.8 h** at `spread` 5% and `m` 2% — and the keeper re-reads the book every pass during those hours: a
cheap listing that is not sniped in 5 h is still exposed, and a bought-out bottom that stays empty for 5 h is an invitation to
every other seller.

### 4.4 What the numbers say

- **BAYC is hard to move**: 10% costs 64 ETH of capital up, or a near-certain 1.8 ETH loss down (bid bots fill underpriced apes in
  seconds, which is what the `S-repeat` count shows).
- **Frogs on one venue are cheap to move**: 0.31 ETH of recoverable capital moves the posted floor ~+18% (two items), and 0.035 ETH
  of at-risk discount moves it −10% on a book where a cheap listing may sit for hours. That is the thin 79-listing Ordinals Wallet
  book alone; Magic Eden lists ~1,010 frogs and must be merged before a frog floor is posted for real (§8).
- Hence three recommendations **to the module** (the contracts agent's call, listed here so they are not lost):
  1. `maxMoveBps` per hour well under the spread (2% vs 5%), so a manipulation must last hours;
  2. widen the spread as `confidenceBps` falls (a thin book posts a low confidence, §1.5);
  3. bound **net unbacked units** by the posted `depthItems` (mints beyond what the book can fill wait), so a wrong floor cannot be
     farmed at size.

## 5. Posting: the assumed interface and the keeper's rules

### 5.1 The interface both sides assume

```solidity
interface IFloorModule {                                             // ASSUMED (keeper/src/floorpost.ts, canister floorsink.rs)
    // identity
    function wrapper() external view returns (address);              // the IWrappedCollection whose custodian is the canister
    function payee() external view returns (address);                // where itemArrived pays: the canister's EVM address
    function keeper() external view returns (address);               // the only address postFloor accepts

    // the feed
    function floor() external view returns (uint256 priceWei, uint64 postedAt, uint64 ttl);
    function maxMoveBps() external view returns (uint16);            // per hour, relative to the posted price
    function floorBounds() external view returns (uint256 lo, uint256 hi);   // what postFloor accepts now (optional: see 5.2)
    function postFloor(uint256 priceWei, uint16 confidenceBps, uint32 depthItems) external;  // keeper only; reverts outside floorBounds

    // the netting
    function netUnits() external view returns (uint256);             // units (1e18 = one item) minted for ETH, less burned for ETH,
                                                                     // less items arrived × 1e18; 0 when negative
    function escrow() external view returns (uint256);               // wei held for items and burns
    function buffer() external view returns (uint256);               // units (1e18 scale) beyond one item before a buy is due
    function maxItemCost() external view returns (uint256);          // the most itemArrived accepts now, wei
    function itemsArrived() external view returns (uint256);
    function itemArrived(uint256 costWei) external;                  // custodian (payee) only, after the unit was minted to the module:
                                                                     // wrapper.balanceOf(this) ≥ (itemsArrived + 1) × 1e18,
                                                                     // costWei ≤ maxItemCost(), costWei ≤ escrow(); pays costWei to
                                                                     // payee; itemsArrived += 1
}
```

The selectors are the keccak of these signatures, checked in `floorsink.rs`'s tests and `keeper/test/floor-post.test.ts`; a
renamed function changes one constant on each side.

### 5.2 The keeper's posting rules (`keeper/src/floorpost.ts`, pure)

Each pass, per collection: compute the target (§1), read `floor()`, `maxMoveBps()` and, if the module has it, `floorBounds()`, then:

1. **No target** (fewer than 3 valid listings) or `confidenceBps < minConfidenceBps`: **skip**; say why.
2. **Step, never jump.** The allowed band is the intersection of the chain's `floorBounds()` (when it answers) and the keeper's own
   conservative band `posted × (1 ± maxMoveBps × min(elapsed, 3600) / 3600 / 10,000)`, rounded inward. The post is the target
   clamped into that band. The conservative band is at least as strict as either reading of "max move per hour" (per post pro
   rata, or per rolling hour), so a post the keeper makes is one the chain accepts under both. Nothing posted yet: the target
   itself (the module's first post has no reference).
3. **Keep** when the clamped value is within `repostMovedBps` of the posted one and the posted one has more than
   `refreshBeforeExpiryS` of its TTL left; otherwise **post** (a refresh re-posts even an unchanged price, so the TTL never lapses
   while the book is healthy).
4. The action carries the calldata `postFloor(price, confidence, depthItems)`; `src/` never sends it (`test/cannot-send.test.ts`
   still passes over the whole directory). The staging operator (`keeper/staging/floorfeed.ts`) sends it through
   `stagingSender` — the staging fork only, a node on this machine, the chain id asked of the node before every send.

## 6. The canister: a Floor Module is an item sink

### 6.1 One trigger for every sink

`itemsink.rs` already decides, from a pinned read adapted to a `SinkView`, whether a purchase may start, what one item may cost,
and whether a held item may be minted to the sink; `buyqueue.rs` and `curvebuy.rs` are adapters. The Floor Module becomes a third
adapter, **`floorsink.rs`** — the one file that names the assumed interface on the canister side, as `buyqueue.rs` did:

- reads at one pin: `netUnits`, `escrow`, `buffer`, `floor`, `maxItemCost`, `itemsArrived`;
- `want` = `(netUnits − buffer) / 1e18` when `netUnits ≥ 1e18 + buffer`, else 0 — **one item + buffer of NET ETH-backed demand**;
- `budget_left` = `escrow`; `item_cap` = `min(maxItemCost, the collection's purchase limit)`;
- `open` once a floor has ever been posted; **`price_expiry`** (new, optional field of `SinkView`) = `postedAt + ttl`;
- no deadline, no refund or settlement trigger (a module has neither).

`itemsink::may_start` gains one additive rule: when `price_expiry` is set, a purchase starts only if it is more than
`CAP_MARGIN_SECS` (15 min, the policy's cap margin) in the future on the pinned clock. Queues and curves have no `price_expiry`, so
their behaviour is unchanged. Minting and the claim do **not** require a fresh price: the item was bought against a fresh one, and
the module's own `itemArrived` checks bound the cost.

The rest is the existing flow, unchanged: at most one purchase started per tick, inventory before a new purchase, the frog bought
(`frogbuy::judge`), six confirmations and ord, `wrapDeposit(…, to = module)` re-checked at a fresh pin, then the report — for a
Floor sink `itemArrived(cost)` instead of `recordItem(cost)`. A 4663 reorganisation that undoes mints shows as `netUnits` falling
at the next pin: a wanted item whose sink no longer wants it fails with nothing spent; a held one becomes inventory. A Bitcoin
reorganisation sends a held frog back to waiting for six (existing).

### 6.2 EVM NFTs through the same trigger

A Floor sink is configured with its item kind: **Bitcoin** (frogs, the flow above) or **EVM** `(chain id, collection)`. For an EVM
sink the same view and the same `may_start` / `per_item_cap` decide; what they start is an **NFT want** (`floortrigger.rs`) instead
of a frog record:

1. **Wanted** `{ sink, chain, collection, cap }` — shown by `floor_wants()`. The relayer picks a floor listing and calls
   `floor_nft_sign(want, token_id, order_hex)` (operator): the canister records the target with the want's cap and signs through the
   existing `nft_sign` (every Seaport rule of docs/v4/eth-nft-canister.md §6 and §12 applies; the per-chain line allows one in
   flight). `nft_send` / `nft_status` are the existing endpoints.
2. **Signed** → **Arrived** when the chain's line records `Arrived` for that token (`floor_nft_arrived(want)`, operator; it reads
   the line, it does not take the operator's word).
3. **Arrived** → the unit is minted to the module by the NFT wrapper's custodian mint (docs/v4/nft-wrapper.md §5.2 — **spec only,
   not on this branch**, see §8).
4. When `wrapper.balanceOf(module) ≥ (itemsArrived + 1) × 1e18` at a pin, the canister sends `itemArrived(cost)` through the same
   one-transaction-at-a-time 4663 line as the frog reports → **Claimed**.

### 6.3 Per-collection purchase limits

`policy::MAX_PURCHASE_WEI` (0.5 ETH, global) refuses a 0.63 ETH frog and any ape. Additively:

- `set_purchase_limit(sink, max_wei)` — **controller only**, which after the handover means the timelock, exactly as the policy's
  constants (code) and `set_item_config` are; the limit is `≥ MIN_PURCHASE_WEI` and `≤ PURCHASE_LIMIT_CEILING_WEI` (25 ETH, code).
- `frogbuy::check_limits_for(cost, limit)`: with a limit, `[MIN_PURCHASE_WEI, limit]`; without, exactly `check_limits` (the
  global). `check_limits` itself is unchanged and still used where there is no sink.
- A Floor sink's `item_cap` is `min(maxItemCost, limit)`, or `min(maxItemCost, MAX_PURCHASE_WEI)` when no limit is set — so an
  unconfigured sink behaves as today's policy does.
- Queues, curves and the desk with no limit set: **unchanged**. A queue may be given a limit the same way (that is how the 0.63 ETH
  frog of docs/v3/curve.md gets bought).

### 6.4 Stable memory

The upgrade tuple gains an **eighth** element, `Option<floortrigger::FloorState>` (the floor sinks' item kinds, the per-collection
limits, the NFT wants), after the seven existing ones; nothing is reordered. A state saved by the seven-element wasm restores with
`None` (tested). The Floor sinks themselves are ordinary `SinkRec`s in `FrogState.sinks` (a new `SinkKind::Floor` variant, and a
new `Option` field on `SinkView`), which Candid decodes from older state.

## 7. Tests

- **keeper** (`npm test`): one fixture per exclusion rule (`L-unverified`, `L-extras`, `L-currency`, `L-unfillable`, `L-stale`,
  `L-unseasoned`, `L-burst`, `L-seller-dup`, `S-self`, `S-wash`, `S-bundle`, `S-outlier`, `S-repeat`, `S-window`); the manipulation
  scenarios (a fake cheap listing, a delist burst, a wash-sale pump, a stale listing, a bought-out bottom); the recorded frog and
  BAYC measurements reproduce §3 and §4's numbers; stepping never exceeds the chain's band; the TTL refresh; OpenSea with a fake
  key and a mocked `fetch` never logs the key; the staging poster refuses a wrong chain id.
- **canister** (`cargo test`): the selectors are the signatures' keccak; the view (below / at / above one item + buffer); a fresh
  vs stale price; the per-collection limit (a 0.63 ETH frog bought with a limit, refused without; a queue without a limit behaves
  as before); a buy failure and retry (a bad offer, a vanished listing, then a good one); a 4663 reorg that drops `netUnits`; a
  Bitcoin reorg after the frog is held; the claim; the EVM want; the eighth tuple element restores from seven. All existing tests
  green.

## 8. Open problems

1. **The module's real interface.** Everything in §5.1 is assumed; the contracts agent reconciles. Swapping names is a change to
   `floorsink.rs` and `floorpost.ts` alone.
2. **Magic Eden's frog book** was unavailable (503) on the day; until it is merged, a frog floor rests on a 79-listing venue whose
   manipulation costs (§4) are low.
3. **ord's JSON API on ordinals.com is disabled.** The keeper fails closed without an ord it can reach (no Bitcoin floor). The
   canister's `custodynet.rs` reads the same host, which is a live problem for every frog path, not just this one.
4. **EVM listing depth was not measured** without a key; §3.2 uses fills as a proxy.
5. **EVM items stop at "Arrived" on this branch**: minting the NFT wrapper's unit (docs/v4/nft-wrapper.md §5.2, finalized pins)
   is spec only; the claim is wired and waits for the unit to show at the module.
6. **The rate** (sats → wei) is one more input to manipulate; the keeper uses a single keyless reference for now.

## 9. Results (2026-09-23, recorded after the code)

**Built.** Keeper: `keeper/src/floor.ts` (the statistic, every rule, clusters, burst detection, `costToRaise` / `costToLower`),
`keeper/src/floorsources.ts` (Ordinals Wallet, an ord reader, OpenSea listings and sales, a keeper-side book diff for listing
events, the rate), `keeper/src/floorpost.ts` (the assumed ABI, the band, skip / keep / post), `keeper/staging/floorfeed.ts` (the
staging poster through `stagingSender`), and one function added to `keeper/src/opensea.ts` (`openseaGet`: the key stays read in that
one file, which `test/opensea.test.ts` enforces). Canister: `floorsink.rs` (the assumed interface), `floortrigger.rs` (item kinds,
limits, NFT wants), `floornet.rs` (endpoints); additive edits to `itemsink.rs`, `frogflow.rs`, `frogbuy.rs`, `frognet.rs`,
`canister.rs`, `buyqueue.rs`, `curvebuy.rs`, `lib.rs`, `etch_autobuyer.did` and `tests/frogs.rs`.

**What the code changed in this document.**

1. **Medians of an even count take the lower middle** (§1.4). The frog sales median is therefore **360,000 sats**, not the
   371,165 of the hand calculation, and the frog floor is `min(565,262, (7 × 565,262 + 3 × 360,000) / 10)` = **503,683 sats =
   0.1581 ETH** at 3,185,800 sats/ETH. Confidence **4,900 bps** (spread of the lowest five 0.77, sales/listing agreement 0.64);
   `depthItems` **1**; `upCost10` **998,331 sats** (two frogs). `test/floor.test.ts` pins every one of these.
2. **BAYC**: `S` = **6.6825 ETH** (lower middle), floor = `L` = **6.0984 ETH**, confidence **8,746**, `depthItems` **12**, `upCost10`
   **63.96 ETH** (ten apes). The 0 ETH "sale" of 5762 is caught by cluster rule 2 (value moved at under a quarter of the floor) and
   reported as `S-wash`, not `S-outlier`: **8 `S-bundle`, 5 `S-wash`, 20 `S-repeat`**.
3. **Stepping** (§5.2): "the conservative band satisfies either reading" is **not true** without the chain's help. Pro rata per
   post, six posts in an hour compound to `(1 + m/6)^6 − 1 = 2.017%` against the price an hour ago, over a 2% rolling limit. So the
   keeper's own band implements exactly one reading — **pro rata to the time since the last post, capped at an hour** — and a
   module that enforces a rolling hour (or anything else) must expose `floorBounds()`, which the keeper intersects and never
   leaves. `test/floor-post.test.ts` walks a 30% move under both chains: 13.2 h at 2%/h, every post inside the rule.
4. **The canister's release rule applies to floor sinks as to queues**: a wanted frog whose module stops wanting it (net demand
   fell back under one item + buffer after a 4663 reorganisation, or the price went stale before an offer came) fails with nothing
   spent; a held frog whose module cannot pay now (escrow drained) becomes inventory and goes back to a module when one can take
   it. Inventory is used only when no purchase for that sink is already wanted (the existing rule), so a module that wants one item
   and already has a wanted frog does not also take an inventory frog.

**Tests.**

- keeper `npm test`: **55 pass** (35 before + 20 new: 14 in `floor.test.ts`, 6 in `floor-post.test.ts`); `tsc --noEmit` clean.
  Staging imports need `viem` from the repository root, so the run linked `keeper/node_modules` (and the root `node_modules`) to
  the main checkout's install and removed the links after.
- canister `cargo test`: **295 pass** (277 before): 231 unit (12 new: selectors, the view below/at/above one item + buffer, a stale
  price, the cap as limit or global maximum, limits bounded and per collection, the item kind set once, the EVM want and its
  release and claim, the eighth tuple element restoring from a seven-element state whose `SinkView` had no `price_expiry`, the
  guard on every new update method), and 28 in `tests/frogs.rs` (6 new, against the fake world: the full frog flow to
  `itemArrived`; a stale floor; the 0.64 ETH frog bought with a limit and refused without, and a queue unchanged until given one;
  a bad offer, a vanished listing, then a good one; a 4663 reorganisation that undoes mints; an EVM want claimed once the unit is
  at the module). `cargo build --target wasm32-unknown-unknown --release` clean.
- Nothing was deployed, to IC mainnet or anywhere; no transaction was sent to any chain. No local replica run was needed: every
  path above is exercised against the fake world `tests/frogs.rs` already uses.

**The interface, as assumed on both sides** — for the contracts agent to reconcile (§5.1):

| function | who calls | assumed semantics |
|---|---|---|
| `floor() → (uint256 priceWei, uint64 postedAt, uint64 ttl)` | keeper, canister | the posted price; stale after `postedAt + ttl` |
| `maxMoveBps() → uint16` | keeper | per hour, pro rata to the time since the last post (or expose `floorBounds`) |
| `floorBounds() → (uint256 lo, uint256 hi)` | keeper (optional) | what `postFloor` accepts now |
| `keeper() → address` | keeper (staging) | the only poster |
| `postFloor(uint256 priceWei, uint16 confidenceBps, uint32 depthItems)` | keeper | reverts outside the bounds |
| `wrapper() → address`, `payee() → address` | canister | identity: the wrapper whose custodian is the canister; the canister's EVM address |
| `netUnits() → uint256` | canister | 1e18 = one item; minted for ETH − burned for ETH − arrived; ≥ 0 |
| `escrow() → uint256`, `buffer() → uint256`, `maxItemCost() → uint256`, `itemsArrived() → uint256` | canister | §5.1 |
| `itemArrived(uint256 costWei)` | canister (custodian) | after the unit is at the module; pays `costWei` from the escrow to `payee` |

Selectors: `floor()` 0x40695363, `netUnits()` 0x273c589e, `escrow()` 0xe2fdcc17, `buffer()` 0xedaafe20, `maxItemCost()` 0x9b04dfbf,
`itemsArrived()` 0xff40b1af, `itemArrived(uint256)` 0xd996759a, `postFloor(uint256,uint16,uint32)` per viem in the keeper's test.
