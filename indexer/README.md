# Rook indexer

Two processes in one package, both read-only (no key, nothing sent):

- **Solana** (`src/sol/`, `npm run sol`): follows Rook's Meteora DBC configs and the graduated DAMM v2 pools, the desks' wX/SOL
  pairs and the canister's own transactions; serves `/v1/rook/*`. The contract, setup, costs and the dry run: `docs/v9/indexer-solana.md`.
- **Robinhood** (`src/main.ts`, `npm start`): the previous chain's indexer, kept unchanged and not deployed for Solana.

`npm test` runs `test/*.test.ts` (node:test, `--experimental-sqlite`), `npm run typecheck` runs `tsc`. Node ≥ 22.5.

## W13: wX-quoted prices and desk events (canister-only design §3, §4.4 rule 4, §5, §10)

Since W13 a collection's coins are quoted in its **wX** (the DBC config's quote mint is the wX; after graduation the DAMM v2 pool
is wX/coin), and the canister labels every transaction that changes a supply or a backing balance with an SPL Memo. The indexer:

### Prices in wX, SOL and USD

- A config's quote mint is read from its account; a quote other than wSOL is a wX, and its decimals are read from the mint
  (`configs.quote_mint`, `quote_decimals`). Trades and candles are stored in that quote (`trades.sol` keeps its column name and
  holds the quote's base units; `price` is quote per whole coin).
- **wX/SOL rate** (`src/sol/wx.ts`, `wx_sol` table, one sample a minute per wX):
  1. the pool desk's DLMM pair for the collection, `[preset 262, min(wX, wSOL), max]` (`pairAddress`, the same derivation as the
     canister's `solix2::dlmm::Lb::desk`): its `active_id` and `bin_step` give the market's price, `(1 + bin_step / 10⁴)^active_id`
     = SOL per wX (9 decimals on both sides); source `dlmm`;
  2. when the pair account does not exist (no desk yet, or the desk withdrew it): the canister's accepted floor,
     `books_view(coll).accepted` (lamports per whole wX); source `books`. Needs `ROOK_CANISTER_ID`.
  A pair that is disabled or not wX/wSOL, and a collection with neither source, has no rate: nothing is estimated.
- **The API** keeps every SOL field meaning SOL. For a wX-quoted coin it goes through the rate nearest the trade's time (within
  two hours, as SOL/USD does), and is null when none is near; the exact figures in the coin's own quote sit next to it:

  | where | in the quote (exact) | in SOL (through the rate) | in USD |
  |---|---|---|---|
  | coin row (`/coins`, `/board`, `/coins/:mint`) | `quote {kind, mint, decimals, symbol}`, `wxSol`, `priceWx`, `marketCapWx`, `volume24hWx`, `volume24hQuoteRaw` | `price`, `marketCapSol`, `volume24hSol`, `volume24hLamports` (rounded from the per-trade conversion when wX-quoted) | `priceUsd`, `marketCapUsd`, `volume24hUsd` |
  | trade (`/trades`, `/wallets`) | `quoteRaw`, `quoteAmount`, `wx`, `priceWx`, `wxSol`, `feeRaw` | `solLamports`, `sol`, `price`, `feeLamports` (null without a rate) | `priceUsd` |
  | chart (`/chart`) | `wx: Candle[] \| null` | `sol: Candle[] \| null` | `usd: Candle[] \| null` |

  The chart answers all three series at once; `quote` says the coin's own quote (`"wX"` or `"SOL"`), `selected` which series
  `candles` carries (default: the coin's own quote, as before; `?quote=wx|sol|usd` picks another). A series with no rate near any
  trade is null. `change24h` is a ratio of quote prices. `/collections` items carry `coll` (the canister's collection index),
  `quote`, `wxSol {sol, at, src}`, `module` (`M[i]`) and `pair`; `/v1/health` lists each wX's latest rate and source.
- A SOL-quoted config (the pre-W13 shape) is the same code with a rate of one: its answers are unchanged.

### Desk events from memos

- `ROOK_DESK_KEYS` names the canister's Solana keys (`K_main`, the fee payer of every canister transaction, first). Each opens a
  `desk` signature stream next to the config streams. Only a signature whose RPC memo carries `rook:v1:…` is fetched
  (`getTransaction`), so a key's nonce advances and reads cost one `getSignaturesForAddress` page each.
- Each Rook memo `rook:v1:<coll>:<kind>:<entry id>[:i<intent>]` (`src/sol/memo.ts`; the canister's `chainlink/memo.rs` writes it;
  kinds: mint, burn, refund, redeem, remint, prize, sweep, coin, close, other) becomes a `desk_events` row with the label (coll,
  kind, link id, intent) and **what the transaction's balances say**: every lamport delta by account, every wX delta by token
  account, the fee payer's delta, the module's delta (when `M[i]` is known), and the net wX supply change (`wx_delta`: a MintTo adds
  with no counterpart, a Burn removes with none, a transfer sums to zero). **A memo is a label, never proof that money moved**:
  anyone can write one; the deltas are the record, the memo only says which link the canister meant. A memo with nothing behind
  it is an event with empty deltas; a transfer without a memo is no event here (the reconciler's supply check is for those).
- `ROOK_CANISTER_ID` (queries only, through `@icp-sdk/core` as `ops/` and `monitor/` do) maps a memo's collection index to its
  wX, config, module and symbol (`coins_collections`), refreshed every 10 min. Without it, events are still recorded (the wX is
  taken from the one wX the transaction moved when there is exactly one; `collection`, `module` are null).
- Endpoints:
  - `GET /v1/rook/desk/events?coll=<index|slug|->&kind=<kind>&since=<unix s>&limit=&before=<next>`: newest first, cursor-paged
    (`next` = `slot:rowid`). `coll=-` selects memos naming no collection (an intent's refund). Item: `{ signature, slot, at, coll,
    collection, kind, linkId, intent, payer, memo, feeLamports, wxMint, wxDelta, wxDeltaUnits, payerLamports, module, moduleLamports,
    lamports: [{address, delta}], tokens: [{owner, mint, delta}] }` (exact amounts as decimal strings).
  - `GET /v1/rook/desk/totals?coll=`: per collection `{ coll, collection, wxMint, wxSymbol, module, events, byKind, wxMinted, wxBurned,
    wxNet, wxNetUnits, payerOut, payerIn, moduleOut, moduleIn, firstAt, lastAt, lastSlot }`: sums of what the balances said under
    each label, for the reconciler and the proof-of-reserves page to compare with the canister's `ledger_log`; not books.

### Environment (new)

| var | default | |
|---|---|---|
| `ROOK_DESK_KEYS` | unset | the canister's Solana keys, comma-separated (`K_main` first); none: no desk events |
| `ROOK_CANISTER_ID` | unset | the canister to query; none: no books fallback for the rate, no collection index mapping |
| `IC_HOST` | `https://icp-api.io` | the IC boundary node for those queries |

The rest (`SOLANA_RPC_URL`, `ROOK_CONFIGS`, …): `docs/v9/indexer-solana.md` §4. Schema migration 2 adds the columns and tables
above to an existing database on start.

### Tests

`test/sol_wx.test.ts`: the pair derivation and bin price against the canister's landed DLMM pair
(`autobuyer/canister-rs/tests/fixtures/solix2/dlmm.json`), the floor against its `books_view` bytes (`tests/fixtures/views/books-view.hex`),
the follower storing both sources, and a wX-quoted coin through the API in all three quotes. `test/sol_desk.test.ts`: the memo
grammar, a memo-carrying transaction (`test/fixtures/sol/desk-mint-rook-memo.json`, synthetic, in the RPC's jsonParsed shape)
decoded into label and deltas, the desk stream fetching only memo-carrying signatures, and the two endpoints.

### Not done

wX/SOL history from before the service's first sample (the pair's own swap history is not read): trades backfilled from before it
have wX figures but no SOL or USD ones. The desk's own DLMM swaps are not listed as trades.
