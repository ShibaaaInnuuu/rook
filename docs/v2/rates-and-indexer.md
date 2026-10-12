# The buyer's reference rate and runes indexer (design, before its code)

Written 2026-09-23, before any code, on branch `rates-indexer` from `a578796`.

The buyer loop (`docs/v2/buyer-loop.md`) stops at two places for want of data it must not take from the party it is checking:

- **The bridge step** refuses "for lack of a reference rate": `bridge::check` needs `floor_sats`, the least BTC the ETH is worth
  by *our* rate, less a tolerance — never the bridge's own number.
- **Settle** cannot run on its own: "delivered rune units have no replicated source". After the purchase's Bitcoin transaction
  confirms, the buyer must establish from Bitcoin data how many units of which rune landed on the winner's script — never from
  the venue that sold them.

Both are filled here through the `Io` trait's existing hooks, `reference_floor_sats` and `delivered_units`. Nothing in the loop
changes; `CanisterIo`'s two stub bodies are replaced.

## Rules both sources follow

1. **Keyless only.** An API key in a canister is public: the install argument is readable and the timelock publishes every
   upgrade's argument in full for 72 hours. A source that needs a key is out, whatever its quality.
2. **IPv6.** IC HTTPS outcalls leave over IPv6 only. A host without an AAAA record is unreachable from a canister however well
   it answers a workstation, so every source below was checked with `dig AAAA`.
3. **Replicated, and reduced before consensus.** Every read is `is_replicated: true`; a transform reduces each reply to the one
   fact read, so a body that differs only in presentation (a timestamp, a sequence number, an icon URL) cannot break agreement.
4. **Several independent sources, and they must agree** — or the loop waits. A disagreement is never resolved by picking one.
5. **Read-only.** Nothing here sends, signs or spends.

## 1. The reference rate: ETH → BTC

### Sources tested (2026-09-23, from a workstation; `dig AAAA` for the IC)

| Source | Endpoint | Keyless | IPv6 | Result |
|---|---|---|---|---|
| Coinbase Exchange | `api.exchange.coinbase.com/products/ETH-BTC/ticker` | yes | yes | **chosen** — `price` (last trade) |
| OKX | `www.okx.com/api/v5/market/ticker?instId=ETH-BTC` | yes | yes | **chosen** — `data[0].last` |
| KuCoin | `api.kucoin.com/api/v1/market/orderbook/level1?symbol=ETH-BTC` | yes | yes | **chosen** — `data.price` |
| Crypto.com | `api.crypto.com/exchange/v1/public/get-tickers?instrument_name=ETH_BTC` | yes | yes | **chosen** — `result.data[0].a` (last trade) |
| HTX (Huobi) | `api.huobi.pro/market/detail/merged?symbol=ethbtc` | yes | yes | **chosen** — `tick.close` |
| Binance | `api.binance.com/api/v3/ticker/price?symbol=ETHBTC` | — | no | rejected: HTTP 451 ("restricted location"), and no AAAA |
| Bybit | `api.bybit.com/v5/market/tickers?…` | — | no | rejected: HTTP 403 (CloudFront country block), no AAAA |
| Kraken | `api.kraken.com/0/public/Ticker?pair=ETHXBT` | yes | **no** | rejected: unreachable from the IC |
| Bitstamp | `www.bitstamp.net/api/v2/ticker/ethbtc/` | yes | **no** | rejected: unreachable from the IC |
| Gemini | `api.gemini.com/v1/pubticker/ethbtc` | yes | **no** | rejected: unreachable from the IC |
| Bitfinex | `api-pub.bitfinex.com/v2/ticker/tETHBTC` | yes | **no** | rejected: unreachable from the IC |
| Gate.io | `api.gateio.ws/api/v4/spot/tickers?currency_pair=ETH_BTC` | yes | **no** | rejected: unreachable from the IC |
| MEXC | `api.mexc.com/api/v3/ticker/price?symbol=ETHBTC` | yes | **no** | rejected: unreachable from the IC |

All five chosen sources price ETH **in BTC** directly (the ETH-BTC book), so no USD cross-rate — and no stablecoin assumption —
enters the number.

### And the IC's own Exchange Rate Canister

The IC runs an exchange rate canister (XRC, `uf6dk-hyaaa-aaaaq-qaaaq-cai`) that queries major exchanges by HTTPS outcall and
returns "the **median** of all received rates"; a call "requires 1 billion cycles attached upfront. Unused cycles are refunded"
([ICP docs, exchange rate canister](https://docs.internetcomputer.org/concepts/chain-fusion/exchange-rate-canister/)). Its
interface ([`xrc.did`, dfinity/exchange-rate-canister](https://github.com/dfinity/exchange-rate-canister)) takes a base and quote
`Asset { symbol, class }` and an optional timestamp, and returns `ExchangeRate { timestamp, rate: nat64, metadata { decimals,
…num_received_rates…, standard_deviation } }` or an error (`Pending`, `RateLimited`, `InconsistentRatesReceived`, …).

**Choice: XRC is one more source, not the only one.** It is the natural primary on mainnet — it already solves the consensus
problem below and aggregates more exchanges than five — but it exists only on the IC mainnet, so a local replica can never
exercise it, and a buyer whose only rate is one canister has one point of failure it cannot see into. So `ETH/BTC` is asked of
XRC when it answers (mainnet), and its answer joins the five exchange readings in the same median and the same spread test. On
a local replica the call fails and XRC is simply absent. Its answer is used only if its `timestamp` is within **10 minutes** of
the canister's clock (a stale-rate refusal) and it reports at least 2 received rates for each asset.

### Agreement between replicas: the price-feed problem

Thirteen replicas read the same ticker a second or so apart; a live price can move between them, and IC consensus needs a
supermajority of **byte-identical** transformed replies. So the transform keeps only the price and **quantises it down** to three
significant figures of satoshis per ETH (at ETH ≈ 0.0319 BTC that is 3,190,000 sat, in steps of 10,000 sat — about 0.31%). Two
replicas then disagree only when the price straddles a step boundary inside that second, which makes the source fail for that
round, not the rate: the aggregate needs three sources, not all six. Timestamps, sequence numbers and volumes are dropped before
consensus; an unreadable reply becomes a fixed error string, so even failures agree.

### The aggregate

- **At least 3 sources** must answer (of the five exchanges plus XRC). Fewer: no rate; the loop waits.
- **Spread:** `(max − min) / median` must be at most **1%** (100 bps). Wider: the sources disagree about the market; no rate.
- **The rate is the median** (lower median for an even count). A single wrong or manipulated source cannot move it.
- **Staleness:** there is no cached rate. The rate is read in the same `tick` that uses it, from live tickers; XRC's own
  timestamp is checked as above. A ticker cannot be checked for staleness inside a transform (replica clocks differ, and a
  transform must be deterministic), so a stale exchange is caught the way a wrong one is: it falls outside the 1% spread or is
  outvoted by the median.
- **The floor:** `floor_sats = send_wei × sats_per_eth / 10¹⁸ × (1 − tolerance)`, rounded down, with the tolerance **3%**
  (300 bps). A bridge quote below the floor is refused by `bridge::check`. 3% covers a bridge's fee and the quantisation step;
  it is a policy constant, stated here and in `rates.rs`, and changing it is a code change behind the timelock.

## 2. The runes indexer: units delivered

### Sources tested

| Source | Endpoint | Keyless | IPv6 | Networks | Result |
|---|---|---|---|---|---|
| ordinals.com (the reference `ord` server) | `/r/utxo/<txid>:<vout>` | yes | yes | mainnet | **chosen** — per-output runes, JSON |
| ordinals.com | `/output/…`, `/rune/…`, `/status` with `Accept: application/json` | — | yes | mainnet | the JSON API is **disabled** (HTTP 406 "JSON API disabled"); only the recursive `/r/` endpoints answer |
| Ordinals Wallet | `turbo.ordinalswallet.com/wallet/<address>/rune-balance` | yes | yes | mainnet | **chosen, as corroboration** — per-address balances with `rune_id` and name |
| Hiro Runes API | `api.hiro.so/runes/v1/…` | — | yes | — | rejected: HTTP 410 "deprecated and no longer available" (Hiro deprecated its L1 APIs on 9 March and points to Xverse — [Hiro blog](https://www.hiro.so/blog/upcoming-deprecation-of-ordinals-runes-and-brc-20-apis)) |
| Xverse API | `api.secretkeylabs.io` | **no** (`x-api-key` required — [Xverse API docs](https://docs.xverse.app/api/api-reference/runes/runes-by-address/rune-balances)) | — | — | rejected: keyed |
| Ordiscan, UniSat | `api.ordiscan.com`, `open-api.unisat.io` | **no** | yes | — | rejected: keyed |
| Magic Eden | `api-mainnet.magiceden.dev/v2/ord/btc/runes/…` | — | yes | — | rejected: HTTP 503 "no healthy upstream" on every runes route tried |
| signet.ordinals.com, testnet4.ordinals.com | `/r/…` | — | **no** | test | rejected: no AAAA, and no answer at all from a workstation |

**So there is no keyless runes indexer for signet or testnet4.** The indexer runs on mainnet only; on a test network
`delivered_units` refuses with that reason, as it did before this change. The project's earlier signet runes rehearsal was
credited through UniSat's keyed API — acceptable for a rehearsal run by hand, not for a canister.

### What each source can say, and what it cannot

- **ord's `/r/utxo`** reports each output's runes **by spaced name** (`{"UNCOMMON•GOODS": {"amount": 3586, …}}`), for unspent
  outputs only: an output that has been spent answers `output … not found`. It is the reference implementation of the protocol,
  so it decides what a runestone moved, cenotaphs and all (a cenotaph's outputs show `{}`).
- **Ordinals Wallet** reports an **address's** balance per rune, with the rune's id and name
  (`{"rune_id": "1:0", "name": "UNCOMMON•GOODS", "amount": "7322"}`). It cannot say which output a unit is in.

No keyless source reports runes per output a second time. So the two are used for what each can establish, and neither is
believed about the one thing only the other can check:

### How `delivered_units(txid, quote)` is established

1. **The transaction's outputs** come from the configured esplora explorers (`btc_api_urls`), replicated, agreeing on
   `(vout, scriptpubkey, address)` and the confirmation height. The outputs paying `quote.recipient_script` are the delivery.
2. **The rune's name comes from Bitcoin, not from an indexer.** A rune id `B:T` is transaction `T` of block `B`; the etching's
   runestone carries the rune number (tag 4) and its spacers (tag 3). The canister fetches that transaction through the same
   explorers and decodes the name itself, with the protocol's rules: bijective base-26 letters, `•` after the letters the
   spacers bitfield marks, the reserved name when an etching names none, and `UNCOMMON•GOODS` for `1:0` (the genesis rune, which
   has no etching transaction). This closes the obvious hole in the two-source design: an indexer that lies about which name an
   id has could otherwise make the buyer count a worthless rune as the one it paid for.
3. **ord must have indexed the block**: its `/r/blockheight` must be at least the transaction's confirmation height. Otherwise:
   wait.
4. **Units** = the sum, over the delivery outputs, of ord's amount for that name. An output ord reports `not found` (spent
   before it was read) is **an error, not zero** — the loop waits, and if it cannot read the delivery before its deadline the
   state machine's own `Stuck(Delivery)` path applies.
5. **Corroboration:** Ordinals Wallet's balance for the recipient's address must list the same **id** under the same **name**,
   with an amount **at least** the units ord counted (the address may hold more from before). If ord counted units and Ordinals
   Wallet does not show the rune, or shows fewer, or names it differently: no answer, the loop waits.
6. **Zero is an answer** only when it is consistent: ord shows no units of that name in any delivery output, and every delivery
   output was found. A cenotaph reads as zero, correctly.

### Residual trust, stated

- ord's per-output figure has no second keyless source. A wrong `ord` would be caught only when it claims *more* than the
  address holds per Ordinals Wallet; it could still under-count (which fails safe: nothing settles) or, together with a lying
  Ordinals Wallet, over-count. Running an `ord` node the operator controls, beside `ordinals.com`, is the fix, the same as for
  the second EVM RPC (`docs/deploy/canister-mainnet.md`).
- Both indexers sit behind Cloudflare; whether they answer outcalls from IC nodes is proven only by the first mainnet read.

## What is tested

- **Pure modules, against real captured replies** (`tests/fixtures/rates-indexer/`, captured 2026-09-23): each exchange's ticker;
  ord's `/r/utxo` with runes, empty and spent; Ordinals Wallet's balance with and without the rune; an esplora transaction; two
  real etchings (`840000:3` → `DOG•GO•TO•THE•MOON`, `840000:28` → `RSIC•GENESIS•RUNE`, as `ordinals.com/rune/<id>` names them).
- **The decisions**: median, spread, source-count and staleness refusals; the floor's arithmetic; every branch of
  `delivered_units`.
- **Live, on a local replica, on a new canister of this branch**: the reference rate from the five real exchanges, replicated;
  and the indexer on real mainnet outputs (read-only).

## Hooks

- `canister.rs`: `CanisterIo::reference_floor_sats` and `CanisterIo::delivered_units` call this module instead of refusing.
- `runestone.rs`: `varint`, `Payload` and `payload_of` become `pub(crate)` so the etching name is read by the same audited
  parser that reads edicts — not by a second copy of it.

## Results (2026-09-23): built as designed; live on a local replica

Code: `src/rates.rs` and `src/runeindex.rs` (pure), `src/oracle.rs` (outcalls, transforms, the XRC call, endpoints);
`CanisterIo::reference_floor_sats` and `CanisterIo::delivered_units` now call `oracle::reference_floor` and `oracle::delivered`.
`cargo test`: 126 unit (13 new, on the captured replies), the loop's 14 and the 2 end-to-end tests pass; the wasm builds clean.

**Live, on a new local canister of this branch** (`autobuyer_oracle`, `ialvw-7h777-77777-aaaiq-cai`, module `0294341e…`,
installed for signet), every read replicated through the local replica's consensus:

- **`reference_rate`**: Coinbase 3,180,000, OKX 3,190,000, KuCoin 3,190,000, Crypto.com 3,190,000, HTX 3,170,000 sat/ETH;
  **median 3,190,000**, spread 62 bps. Coinbase read directly at the same moment: 0.031915 BTC/ETH (3,191,500). XRC: "Canister
  uf6dk-hyaaa-aaaaq-qaaaq-cai not found" — absent on a local replica, as designed, and left out of the median.
- **`probe_rune_name`** (mainnet explorers, both agreeing, name decoded from the etching by this canister): `840000:3` →
  `DOG•GO•TO•THE•MOON`, `840000:28` → `RSIC•GENESIS•RUNE`, `1:0` → `UNCOMMON•GOODS` — as `ordinals.com/rune/<id>` prints them.
- **`probe_delivered`** on real mainnet outputs from block 968,217:
  `ec2eb840…:0` → **3,586** UNCOMMON•GOODS (address holds 7,322 per Ordinals Wallet: this output and `99cad292…:0`);
  `12c21f10…:0` → **2,720** (address holds 82,387); `2fa93915…:1` → **25** (address holds 17,100); the same `ec2eb840…:0`
  asked for `840000:3` → **0** DOG•GO•TO•THE•MOON (it holds none); `847dbadb…:0`, already spent → **"spent before ord could be
  read: no answer"**, not zero.
- The anonymous principal is refused on every endpoint.

**Not shown live:** XRC (mainnet only); `delivered_units` through the loop end to end (the canister is installed for signet, where
it refuses by design — there is no keyless signet runes indexer); whether these hosts answer outcalls from IC nodes rather than a
workstation (all sit behind Cloudflare; the first mainnet read proves it).

**Open:**
- ord amounts beyond `u64` are refused rather than read (serde_json without `arbitrary_precision` would read them through a
  float). A rune whose raw amounts exceed 1.8×10¹⁹ units cannot settle until that is lifted.
- Only one per-output source: see "Residual trust" above. An `ord` node the operator runs is the fix.
- The tolerance (3%) and spread cap (1%) are policy constants chosen here, not fitted.
