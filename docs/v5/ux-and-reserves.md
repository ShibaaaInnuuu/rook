# One-click trading and proof of reserves

Written 2026-09-23 on branch `oneclick-ux` (from `9022920`), **before its code**. Read side only: `web/` (the site) and
`indexer/` (the API). No contract, canister or keeper file is touched. Nothing here is deployed; the v3/v4 stack
(`FrogCurve`, `EtchRouter`, `LiquidityLock`, `Wrapper`, `NftWrapper`) has no public deployment, so every figure the new pages
show today comes from a **fixture set** built from docs/v3/curve.md's worked examples, and every such page says so on screen.

## 1. What a visitor sees

### The coin page: "Buy ALIEN"

A coin launched on the v3 curve (`CurveFactory.launch`) gets a one-click ticket. The words **wX, wFROG, escrow, mint, burn,
queue and pool do not appear on it** unless the visitor opens *Details*.

```
 ┌ Buy ALIEN ─────────── Sell ┐
 │ You pay        [ 0.1  ] ETH │
 │ You get      ≈ 5,512,300 ALIEN
 │ Total cost       1.21 %    │   ← ONE number: every fee, every spread, and the price impact, against the pre-trade mid price
 │ At least     5,457,177 ALIEN (1% slippage)
 │ [ Buy ALIEN ]              │
 │ ▸ Details                  │   ← the route, each hop, each fee in bps and in ETH, the spread, the impact, the contract called
 │ Slippage 0.5 / 1 / 3 %  Deadline 10 / 20 / 60 min
 └────────────────────────────┘
```

- **The phase decides the venue, not the visitor.** `FrogCurve.state()`: `Trading` → the button calls `FrogCurve.buy(minOut)`
  / `sell(amount, minEth)`. `Graduating` → trading is closed; the page says "buying the items — trading opens when it
  graduates" (the queue's progress in items is not shown yet: see §7). `Graduated` → the button calls `EtchRouter.buy(coin, minOut, to, deadline)`
  / `sell(…)`, and the page says **"Graduated — now trading on Uniswap"**. `Refunding` → "this coin did not graduate; holders
  can refund" (a refund button against `FrogCurve.refund`).
- **The deadline** is enforced on-chain only by the router. The curve's `buy`/`sell` take no deadline; the page says so in
  *Details* rather than pretending the setting applies.
- **The last curve buy** is filled to the raise and the change returned in the same transaction (`FrogCurve.buy`): the quote
  says "this buy closes the curve: 0.37 ETH is used, the rest comes back".

### The one number

`totalCost = 1 − amountOut / (amountIn × Π midᵢ)` where `midᵢ` is each hop's **pre-trade** mid price (fee-free) in output per
input. It therefore contains, with nothing left out:

| hop | fee (bps of the hop's input) | source |
|---|---|---|
| curve buy/sell | `feeBps` (100) | `FrogCurve.feeBps()` |
| shared wX/ETH pool | LP fee (30) **and** Uniswap's protocol fee (5), applied protocol-first: `1 − (1−pf)(1−lp)` | `StateView.getSlot0` |
| coin pool COIN/wX | LP fee (100) and protocol fee (10) | same |
| Floor Module mint / burn | its spread (floor ± spread) | the module (§4), when deployed |
| price impact | the remainder: `totalCost − (1 − Π(1 − feeᵢ − spreadᵢ))` | computed; shown as a "price improvement" when negative (e.g. a Floor Module mint below the pool's price) |

*Details* lists each fee in bps and in ETH, the spread, the impact, and the contract and function the button calls. It is the
only place wX is named.

**Route choice** (`web/src/lib/quote.ts`, pure): after graduation, a buy is ETH → wX → COIN and a sell COIN → wX → ETH. The
ETH ↔ wX hop has two candidates: the collection's shared Uniswap pool and, once it exists, the Floor Module (mint at
floor + spread / burn at floor − spread). The quote takes the candidate that gives the visitor more, ties to the pool. Until
the Floor Module lands **and EtchRouter routes through it**, only the pool is *executable*; the module's quote is shown in
*Details* as "not routable yet" and never chosen (`ROUTER_USES_FLOOR_MODULE = false` in the adapter).

**Where the numbers come from.** The curve hop is the contract's own integer arithmetic re-implemented exactly (`buy`,
`sell`, rounding against the trader). A live pool hop is Uniswap's `V4Quoter.quoteExactInputSingle` (measured address,
docs/v3/pools.md §1) for the amount and `StateView.getSlot0` for the mid and the fees. In fixture mode a pool hop is a
constant-product estimate over the fixture's virtual reserves, labelled as such.

### The collection page (`#/collection/<slug>`)

The floor (the keeper's fresh cap, `CurveFactory.freshCap()`, or "no fresh floor" when stale), items held, wX supply, the
reserve status (one line, linking to the Proof of Reserves page), recent activity (items arriving, redemptions, deliveries,
refunds, raffle wins and burns from the sweep vaults), and every coin on the collection with its phase. Two actions: **Redeem
an item** and **Rent your item**.

### Redeem an item (`#/redeem/<slug>`, `#/redeem/<slug>/<ticket>`)

Optional, separate from trading. Burn one whole wX for one **random** item: "the item is chosen by a drand round that does not
exist yet when you burn, so nobody — including us — can pick which one you get." The destination is a **Bitcoin taproot
address** (`bc1p…`, checked with the existing bech32m validator) for inscriptions (`Wrapper.redeem(string)`), or an **EVM address
on the collection's chain** for NFTs (`NftWrapper.redeem(address)`, refusing zero and the custodian). The ticket page tracks
`Pinned → Resolved (which item, deliver-by) → Delivered (tx link)`; past `deliverBy` on both clocks with no delivery it offers
`refund(id)` ("anyone may press this; one whole token goes back to the ticket's owner"), and a pinned round unproven for 24 h
offers `redraw(id)`.

### Rent your item (`#/rent/<slug>`)

A description of the rental being built, and nothing that can be pressed: no form, no deposit, no fake yield. It says the
contracts are in progress and links to this document.

### Proof of reserves (`#/reserves`, `#/reserves/<slug>`)

The custody is a honeypot, so everything is shown. Per collection:

- **the status**, one of three, with the time it was last computed and the time it was last **green**:
  - **green** — every item was found where it should be by every public source, every read is fresh, and the equation holds
    exactly;
  - **red** — an item was found somewhere else, or the equation fails in a way no unread source could fix;
  - **unverified** — any source failed, disagreed or is stale. Never green. The page lists which.
- **the equation**, computed live (§2), with each term's source;
- **every item in custody**: inscription id (or NFT contract + token id), where it sits (the Bitcoin custody address, or the
  canister's address on the NFT's chain), how and when it arrived (vault purchase, holder deposit — rental once it exists),
  the arrival transaction, and per-item verdict with the links a reader follows to check it themselves;
- **tickets in flight**: redemptions burned and not yet delivered or refunded, with their stage and deadline (overdue in red);
- **ETH-backed units and the escrow balance**, from the Floor Module, or "no Floor Module deployed for this collection" (then
  the term is zero by construction, not by assumption).

The JSON is served at `/v1/reserves` and `/v1/reserves/<slug>`; the page renders it.

## 2. The invariant

`Wrapper` and `NftWrapper` keep `totalSupply == (held − openTickets) × 1e18` on their own books. That line proves nothing about
custody: `held` is the custodian's word. The page therefore recomputes it from **outside** sources:

```
   (items found in custody − tickets in flight) × 1e18  +  ETH-backed units   =   wX totalSupply
      Bitcoin: ord + 2 esplora              wrapper.openTickets    Floor Module          wrapper on 4663
      NFT: ownerOf on every provider                                                      (same block)
```

"Items held × 1" in the brief is the left term: an item promised to a burned ticket still sits in custody until delivered, but
backs no circulating token, so it is subtracted. Decisions:

1. The indexer folds the wrapper's own events into the **ledger** of held items (arrivals minus deliveries). The ledger count
   must equal `held()` read at the same block, and the ledger's open tickets must equal `openTickets()`; either difference is
   **red** (the books disagree with themselves).
2. Each ledger item is checked at its public source and gets one verdict: `in-custody`, `elsewhere` (a source *proved* it is
   not in custody), or `unverified` (a source failed, sources disagreed, or the read is older than `maxAgeS`).
3. **Red** if any item is `elsewhere`; if the books disagree (1); or if the equation fails **even counting every unverified item
   as present** (under-backed whatever the unread sources say); or if every item is verified and the two sides differ in either
   direction (over-backed is a mismatch too: the books are wrong).
4. **Unverified** if not red and any item is unverified, any read failed, any read is stale, or the ledger could not be read in
   full.
5. **Green** only when none of the above: every item `in-custody`, all reads fresh, the equation exact.

### Sources, all public and keyless

| what | how | a reader checks it at |
|---|---|---|
| an inscription's location | ord `https://ordinals.com/r/inscription/<id>` (`address`, `satpoint`); then the satpoint's output at **both** esplora explorers the canister is configured with (`mempool.space/api`, `blockstream.info/api`): it exists, pays the custody address and is unspent | ordinals.com/inscription/<id>, mempool.space/tx/<txid>, mempool.space/address/<custody> |
| an NFT's owner | `ownerOf(tokenId)` by `eth_call` at one pinned recent block (the lowest head over providers, minus a margin) on **every** configured provider of that chain (docs/v4/eth-nft-canister.md: Ethereum `eth.drpc.org`, `xrpc.cl/eth`; Base `mainnet.base.org`, `base.drpc.org`, …); all must name the custodian | the chain's explorer, token page |
| wX supply, `held`, `openTickets`, tickets, events | the wrapper on 4663 via the indexer's read-only RPC, at one block | the Robinhood Chain explorer |
| ETH-backed units, escrow | the Floor Module (§4), same block | same |

Ord says elsewhere → `elsewhere`. Ord says custody but an explorer says the output is spent or pays another script → sources
disagree → `unverified`. Providers disagree on an owner → `unverified`; all agree on another owner → `elsewhere`. A 404 from ord
for an id the ledger holds → `unverified` (ord may lag), never green.

## 3. The API

`indexer/src/wrapped/`: `ledger.ts` (pure fold), `invariant.ts` (pure, §2), `verify.ts` (Bitcoin and EVM checkers over an
injected `fetch`), `floorModule.ts` (the adapter), `service.ts` (reads, caches 60 s, keeps `lastGreenAt`). Configured by
`WRAPPED_CONFIG=<file.json>` (collections: slug, kind, wrapper, factory, lock, floor module or null, custody, providers,
`fromBlock`). Unset: every endpoint answers `configured: false` and empty lists — nothing is invented.

| path | returns |
|---|---|
| `/v1/wrapped` | `WrappedList` — collections with supply, items held, floor, status |
| `/v1/wrapped/:slug` | `WrappedCollection` — the collection page's data: coins with phase, activity, open tickets |
| `/v1/wrapped/:slug/tickets/:id` | one redemption ticket |
| `/v1/reserves` | `ReserveReport[]` |
| `/v1/reserves/:slug` | `ReserveReport` — status, reasons, the equation's terms, every item with its verdict and links, tickets in flight |

Fixtures: `indexer/fixtures/wrapped/` is written by `npm run fixtures:wrapped` running the **real** service over an in-memory
world built from docs/v3/curve.md's examples (frogs at 5 ETH; an ape bought FIFO), with one Bitcoin explorer made to fail so
the fixture also shows an *unverified* collection. Every file carries `source.kind = "fixture"`; the site copies them to
`web/public/fixtures/wrapped/` byte for byte (web/test/api.test.ts checks) and shows them only under `?demo=sample`, with a
**FIXTURE** banner on each page.

## 4. The Floor Module adapter

The Floor Module (branch `floor-module`, docs/v5/floor-module.md when it lands) mints wX with ETH at floor + spread and burns
at floor − spread from an ETH escrow. Its interface is **assumed**, in one file per package, named `IFloorModule`
(`web/src/lib/floorModule.ts` for quotes, `indexer/src/wrapped/floorModule.ts` for reserves):

```solidity
interface IFloorModule {                                   // ASSUMED until docs/v5/floor-module.md lands
    function wrapped() external view returns (address);    // the wX it mints and burns
    function floor() external view returns (uint256);      // wei per whole wX
    function spreadBps() external view returns (uint256);  // mint at floor·(1+s), burn at floor·(1−s)
    function ethBackedUnits() external view returns (uint256); // wX units in supply backed by escrowed ETH, not by items
    function escrowBalance() external view returns (uint256);  // ETH held against them, wei
    function quoteMint(uint256 ethIn) external view returns (uint256 unitsOut);
    function quoteBurn(uint256 unitsIn) external view returns (uint256 ethOut);
}
```

Reconciling later touches those two files and the collection config's `floorModule` address; the invariant, the quote engine
and the pages consume only the adapter's typed result.

## 5. What this does not do

- It sends nothing on any public chain and holds no key. The site's buttons ask the visitor's wallet to sign.
- It cannot see an item leave custody between two checks; the page shows how old each check is.
- An NFT collection whose admin can move tokens, or a Bitcoin reorg deeper than the explorers report, is outside what a read
  can prove; the verdict is only as good as the sources, which is why every link is shown.
- Rental is described, not built.

## 6. Built (the same day, with the code)

Commits on `oneclick-ux`: this document; a one-line declaration fix so `web` typechecks and builds again on main's tree; the
indexer (`src/wrapped/`, the endpoints, `fixtures/wrapped/`, types); the quote engine and the Floor Module adapter; the pages.
Tests: indexer **83 pass** (14 new in `test/wrapped.test.ts`: the invariant's green / red / unverified cases, stale reads,
partial source failures, an incomplete log, the ledger, the Bitcoin and EVM checkers against an in-memory world, the service
over the worked examples, the endpoints, the config); web **113 pass** (9 in `test/quote.test.ts`, 6 in `test/v5.test.ts`
rendering the components with `react-dom/server`). `web` builds; both packages typecheck. Screens were checked at 1440 and 400
px wide in both themes under `?demo=sample` with no horizontal scroll.

What changed from the design above while building it:

- **An incomplete event log decides nothing.** A failed `eth_getLogs` leaves the ledger empty, and "0 items against 4 in
  supply" would have read as under-backed. With the ledger incomplete the arithmetic is skipped and the status is unverified.
- **`Wrapper` mints every Bitcoin arrival through `wrapDeposit`**, the canister's own purchases included, so "bought" and
  "deposited" are told apart by the recipient: a `Deposited` to the collection's queue or mint desk is a purchase.
- **A vault-receipt arrival (`Wrapper.wrap`) names no Bitcoin txid**; the item's arrival transaction is taken from ord's
  satpoint at check time, which is the arrival only while the inscription has not moved inside custody.
- In a fixture every explorer link is printed as text: the ids are invented and a link would lead nowhere.

## 7. Open problems

- **Nothing here has met a real chain.** The live paths — the service against 4663, ord, esplora and Ethereum providers, and
  `liveQuote.ts` against FrogCurve, `V4Quoter` and `StateView` — typecheck and run only against the in-memory world. The
  `V4Quoter.quoteExactInputSingle` parameter struct and the `protocolFee` packing (low 12 bits 0→1) are Uniswap v4-periphery's
  as published, not read off 4663's bytecode; check them on a fork before the first live quote.
- **The site cannot send in `api` mode.** `TxButton` refuses without a bundled deployment file, so a build pointed at an indexer
  alone shows quotes and reserves but trades nothing. The v3/v4 stack needs its addresses in the site's deployment file (and a
  `WRAPPED_CONFIG` for the indexer) when it deploys. **2026-09-25 (K5):** `web-rook`'s Phase-1 build bundles
  `deployments/stack-4663.json` (the same file is the indexer's `WRAPPED_CONFIG`) and its `TxButton` refuses without it, and sends
  only to that stack's contracts or to coins and curves its factory launched (docs/v6/site-design.md §7).
- **The Floor Module** is an assumed interface in two adapter files; `ROUTER_USES_FLOOR_MODULE` stays false until EtchRouter
  routes through it.
- **Redeem does not read the new ticket id** from the receipt; the visitor tracks it by number or from the open-ticket list.
- **Depth of the checks.** Bitcoin: an output counts once confirmed at all, not N deep; EVM: `ownerOf` is read 8 blocks behind
  the lowest head, not at `finalized`. Both are one constant each to tighten.
- **Cost of the first read.** 4663's blocks are 0.1 s, so a wrapper deployed a month before the indexer starts is ~260
  `eth_getLogs` chunks of 100,000 blocks on the first pass (then incremental).
- **A graduating coin** shows its phase, not how many of its items the queue has bought yet (`ItemQueue.entry(coin)` is the read).
- **Rental** is a described page; `ItemArrival.how = "rental"` exists in the types and nothing produces it.
- The fixture's pools are full-range constant-product estimates of docs/v3/curve.md's layered depth, labelled as estimates.
