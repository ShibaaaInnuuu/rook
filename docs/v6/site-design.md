# The Rook site: design (v6)

Written 2026-09-23 on branch `rook-site`, **before its code**, for a new package `web-rook/` beside `web/`. `web/` is not touched;
what `web-rook/` takes from it is data and nothing else: `web/src/lib/`, `web/src/api/`, `web/src/deployments/`, `web/src/config.ts`,
the wagmi setup (`web/src/Web3.tsx`), the fixture sets under `web/public/fixtures/`, and the indexer's API contracts
(`web/src/api/types.ts`), all copied byte for byte. Every visible thing — layout, components, styles, copy — is written here from
scratch. Nothing in this document is deployed; where nothing is deployed the site says so on screen and never invents a number.

## 1. What Rook is, to the two people who use it

Rook is a launchpad for memecoins backed by blue-chip collectibles — Bitcoin frogs, punks, apes — on Robinhood Chain (4663). Every
coin's trade fees buy the collection's floor into custody; the items are then raffled to holders, burnt against the supply, or
relisted at a markup with the proceeds buying the coin back (YOYO). Two audiences arrive at once and the site serves both without
making either learn the other's words:

- **the trader** wants one button and one number: buy this coin, and this is what the trade costs, all in. Never "wX", never
  "escrow", never "mint" on the main path; those live under *Details*;
- **the holder** of a collectible wants the floor swept, an item raffled, or their own item rented into the hoard, and wants to
  see — with links they can follow — that every item said to be in custody is in custody.

The name is the bird that hoards shiny things, and the site speaks its language: **the hoard**. A collection's page is a
*nest*; the home hero is the hoard, live; the reserves page is a ledger, because a hoard that cannot be counted is a rumour.

Tone: dark, glossy, precise. Not a purple-gradient AI landing page; not cream and serif; no Inter, no Space Grotesk; no emoji;
no three-equal-feature-cards; no centred hero over a mesh. Copy is written from the visitor's side, in plain words, with the
protocol's own words (wX, β, the module, the router) held back until *Details* is opened.

## 2. Tokens

Dark first. Defined on `:root`; the light theme redefines the same names under `:root[data-theme="light"]` and under
`@media (prefers-color-scheme: light)` for a root with no `data-theme`. A remembered choice is applied before first paint by a
tiny external script (`public/theme.js`, as in `web/`, because the CSP allows scripts from `'self'` only).

| token | dark | light | what it is for |
|---|---|---|---|
| `--ground` | `#0B0D10` | `#F5F6F8` | the page: cool feather black / paper |
| `--surface` | `#14181D` | `#FFFFFF` | panels |
| `--raised` | `#1B2128` | `#EEF0F3` | a panel on a panel; inputs; the ticket |
| `--line` | `#262D36` | `#D9DDE3` | every rule and border; nothing else draws lines |
| `--ink` | `#ECEFF3` | `#0B0D10` | text |
| `--muted` | `#9AA3AE` | `#5B6570` | secondary text, labels |
| `--accent` | `#2FD3C6` | `#0E7C73` | the corvid sheen: **one live element per screen** (the primary button, the current phase, the equation's result). Never a fill for decoration |
| `--sheen-2` | `#6E7BFF` | `#6E7BFF` | only inside the rook mark's iridescence. Never a page gradient |
| `--good` / `--warn` / `--bad` | `#35C979` / `#F2B33D` / `#F0554D` | `#1E8A52` / `#A87308` / `#C6362E` | verdicts: green / unverified / red |

Everything else (focus rings, hover, the banner) is derived with `color-mix()` from these nine, so a theme is nine values.

## 3. Type

- **Display**: *Bricolage Grotesque* (variable; the width axis is used wide for the wordmark and H1s, with tight tracking, −0.02em
  and tighter as it gets bigger). Self-hosted through `@fontsource-variable/bricolage-grotesque`, because the CSP (`font-src 'self'`)
  is `web/`'s and does not change.
- **Body**: *Hanken Grotesk* (variable; already in the repository's `web/`, self-hosted).
- **Numbers**: *IBM Plex Mono*, `font-variant-numeric: tabular-nums`, on every figure that lines up with another: the hoard
  panel, the ticket, the ledger, the countdowns.

A real scale, on a 1.2 ratio at phone width and 1.25 from 720px: `--t-1` 0.8rem, `--t0` 1rem, `--t1` 1.2rem, `--t2` 1.44rem,
`--t3` 1.73rem, `--t4` 2.07rem, `--t5` 2.49rem, `--t6` 3rem; the H1 is `clamp(2.2rem, 6vw, 4rem)`. Headings `text-wrap: balance`;
body measure `65ch`; line height 1.5 for body, 1.05 for display.

## 4. Layout and motion

- **Asymmetric and left-aligned.** A 12-column grid from 880px (`--gutter` 16px at 400px, 24px from 720, 40px from 1200; the
  page is at most 1280px wide). Nothing is centred except a figure inside a panel.
- **Home** is a two-column hero: the thesis on the left in one line — **"Memecoins that buy the floor."** — with one sentence
  under it and two entries (*Trade a coin* / *Sweep your collection*), and on the right the **hoard panel**: items in custody,
  the floors, the last raffle / burn / relist, coins live, all from the same reads the collection pages make. Under it, three
  rows that are not cards: *How a coin works* (a numbered sequence, drawn as one line), *The nests* (each collection as a row),
  and *Count the hoard* (the reserves status line).
- **Coin** is built around the ticket: the Buy/Sell ticket takes the first column at 1440 and the whole width at 400, with the
  coin's phase, the fee tier, the launch-fee countdown and the policy beside it. Below: policy activity, then *Details*.
- **Collection** is the nest: floor, items held, reserve, the coins on it, the activity, with *Redeem* and *Rent* as two entries.
- **Reserves** is a ledger: a status line, the equation as a column of rows that sum, then every item as a row with its verdict
  and links.
- **Motion**: one orchestrated load sequence (the wordmark, then the hero's two columns, then the rows, 60 ms apart, 320 ms each,
  `cubic-bezier(.2,.7,.2,1)`), small hover states (a 1px lift and the line brightening), and the mark's sheen on hover. Under
  `prefers-reduced-motion: reduce` every transition is 0 ms and nothing moves.
- **Mobile first** at 400px: 16px gutters, no horizontal scroll (tables scroll inside their own box; long ids break with
  `overflow-wrap: anywhere`).

## 5. The rook mark: four birds, one worn

The user chose the bird over the chess piece and asked for a better-drawn one (2026-09-23), so four are built, in one
language — a 32-unit grid, clean paths (lines and one-arc curves, no hand noise), fills and 2.4-unit strokes in `currentColor`,
one accent-coloured shape, the eye as a hole in the ink that takes the accent on hover. The geometry is one file
(`web-rook/src/rook/marks.ts`); the components (`RookPerched`, `RookHead`, `RookGlyph`, `RookFlight`), the standalone files
(`public/brand/bird-<id>.svg`) and the favicons (`favicon-<id>.svg`, written by `scripts/write-marks.mjs`) all come from it.
`#/brand` shows the four at 16, 24, 32 and 96 px on the dark and the light ground with the wordmark. The choice is one line
(`web-rook/src/rook/brand.ts`, `BRAND_MARK`) plus copying the favicon. The chess piece stays in the code, not on the page.

1. **Perched** — the whole rook in profile on a perch: heavy body, long tail, legs; the pale bare face patch at the base of
   the beak, which is what tells a rook from a crow, is the one accent shape.
2. **Head** — a circle, one decisive wedge of a beak, a neck. Bold and geometric.
3. **Glyph** — the least that still reads as a rook: the open beak as a chevron, the crown as one arc, the eye. Built for 16 px.
4. **Flight** — a lens of a body, beak forward, tail fanned, wings as two strokes.

On hover (only on hover, and not under reduced motion) every fill and stroke takes a linear gradient from `--accent` to
`--sheen-2` — the iridescence of a rook's feathers — for 320 ms. Nowhere else on the site is `--sheen-2` used.

**My pick: the head (2).** It is unmistakably a corvid at every size from the browser tab to the hero — the heavy beak does the
work — and as a solid shape it holds its own beside the wide, heavy wordmark, where a stroked mark thins out. The perched bird is
the most *rook* of the four (the face patch is the species' own signature) but at 16–24 px its legs and tail become texture; it
is the right one for a social card or a hero illustration, not for the lockup. The glyph is the sharpest at 16 px and the least
specific; the flight mark reads as any seabird at small sizes.

## 6. Pages

Every page reads through the copied `lib/` and `api/`. Where nothing is deployed, a page shows the fixture set under a banner
that says **FIXTURE — not deployed** and offers nothing that could be mistaken for a live number.

| page | route | what the visitor sees | data |
|---|---|---|---|
| Home | `#/` | **the board** (amended 2026-09-23: a launchpad board, not a brochure): one strip with the thesis and the hoard's count, the live tape, the king of the hoard, then every coin under New / Trending / Graduating / Graduated / All / Watching with sorts (trending, market cap, 24h volume, age, holders, progress, floor swept) and instant search; per row a logo or identicon, name and symbol, nest, market cap, 24h change, volume, holders, progress to graduation, policy, fee tier, age, a star | `/v1/rook/board`, `/v1/rook/activity`, `useWrappedList` |
| Coin | `#/coin/<address>` | chart (candles or line, price or market cap, 1m / 5m / 15m / 1h / 1d) beside the ticket; market cap, price, 24h, volume, holders, trades; progress and "graduates at X ETH", or "graduated — trading on Uniswap"; Share (copies the rook.fun link) and a star; tabs Trades (time, side, price, size, ETH, USD, trader, tx), Holders (rank, address, share, bar, launcher label), Info (nest, policy, fee tier, launch fee, contracts, links), Activity (raffles, burns, arrivals) | `/v1/rook/coins/:a/chart|trades|holders`; the ticket as below |
| Wallet | `#/wallet/<address>` | any address: what it launched, holds (with share and value), traded lately, and won in raffles; the connected wallet when no address is given | `/v1/rook/wallets/:address` |
| Launch | `#/launch` | a four-step wizard: **collection** (a nest) → **policy** (raffle / burn / YOYO, one sentence each) → **fee tier** (1–8%, a slider, with the "ETH of trading per item" preview from the collection's floor at the vault's 40% share) → **target and summary**. The decaying anti-sniper fee is explained in one line: *"For the first 10 minutes a buy pays an extra fee that starts at 50% and falls to nothing; all of it goes to the coin's holders."* | the collections; `CurveFactory.launch(name, symbol, policy, sweepOnly, feeBps)` when deployed |
| Coin | `#/coin/<address>` | the ticket (You pay / You get / **Total cost**, one number / At least), the phase (curve / graduating / graduated / refunding), the fee tier, the launch-fee countdown under the ticket (from the coin's own terms, `WrappedCoin.launchFee`, run on the clock), the module's own spreads, a **YOYO** section for a YOYO coin (open listings, proceeds waiting, bought back and burnt, each listing's stage), the policy's activity (raffles, burns, arrivals, relists, sales, buybacks, returns), and *Details* (each hop, each fee in bps and ETH — the tier and, while it lasts, the launch fee as its own line — the spread, the impact, the contract called). The one accent on the screen is the ticket's cluster: the total cost, the Buy button, the countdown | `quoteCoin` over `marketVenues` (fixture: the module through `stateFrom` — its fills price, each side's spread, its room) or `liveVenues` (chain: `price()`, `spreads()`, `mintCapacity()`, `burnCapacity()`, and the amounts from the module's own `quoteMint` / `quoteBurn`); `FrogCurve.buy/sell`, `EtchRouter.buy/sell`, `FrogCurve.refund` |
| Collection | `#/collection/<slug>` | the nest: floor, items held, supply, reserve line, the coins with their phase, activity, and the two entries Redeem / Rent | `useWrappedCollection` |
| Reserves | `#/reserves`, `#/reserves/<slug>` | the ledger: status, `items + ETH-backed units (to be converted + reserve + float) = supply`, red / green / unverified with reasons; then the Floor Module's books (its fills price against the keeper's floor, both spreads, the convertible pot, the standing reserve, the float inventory, the revenue pot, the room right now, the loan, the 7-day volume, the last fills), the protocol treasury (held, the four pots and shares, operations, totals, the registered nests by weight) and the protocol-owned liquidity in the lock; every item with links; tickets in flight | `useReserves`, `useReserve` (`ReserveReport.floorModule`, `.treasury`, `.protocolLiquidity`) |
| Redeem | `#/redeem/<slug>`, `#/redeem/<slug>/<ticket>` | one whole token for one random item, drawn by a drand round that does not exist yet; the ticket tracker (pinned → resolved → delivered); Resolve (anyone, in ticket order, once the round is published) and Refund (anyone: 7 days after the pin, or 72 h after the delivery deadline). No redraw: the pinned round is final (docs/audit/2026-09-26/fixes-W2-site.md) | `useTicket`, `Wrapper.redeem` / `resolve` / `refund`, `NftWrapper.redeem` |
| Bots | `#/bots`, `#/bots/<slug>` | how anyone earns: hand a fillable listing in with a bond, get the bond back plus a reward when the purchase is six blocks deep; per Bitcoin nest, the board's terms, its totals (reward pool, paid today, open, rewards paid, bought, slashed), the open hand-ins cheapest first and the settled ones newest first | `/v1/handins/:slug` (the indexer's `HandinReport`, live or fixture); a nest whose report names no board shows the doc's suggested parameters, labelled |
| Docs | `#/docs`, `#/docs/<name>` | the repository's design documents, rendered | markdown copied in at build (`scripts/sync-content.mjs`) |
| Trust & Legal | `#/trust`, `#/terms`, `#/risk`, `#/privacy` | the trust model word for word; terms, risk, privacy | `docs/trust-model.md`; `config.ts` |

### 6.1 Endpoints added for the board (`indexer/src/rook/`, wired with three lines in `api.ts` and `main.ts`)

| path | returns |
|---|---|
| `/v1/rook/board` | `Board`: every coin of both kinds — `curve` (a Rook coin on a FrogCurve, read from its own `Bought`/`Sold`/`Graduated`, `Transfer` and PoolManager `Swap` logs through `CurveReader`) and `pons` (a coin launched through the Launcher, from the follower's database) — with price, 24h change, market cap, 24h volume, trades, buys, holders, new holders, progress, policy, fee tier |
| `/v1/rook/coins/:address/chart?res=1m\|5m\|15m\|1h\|1d` | `RookChart`: candles at that resolution over 240 buckets (a month for `1d`), the graduation, the supply and the dollar rate |
| `/v1/rook/coins/:address/trades`, `/holders` | one tape, one holder list, in one shape for both kinds (`before=block:logIndex`, `offset=`) |
| `/v1/rook/activity?limit=` | the tape across the board: buys, sells, launches, graduations, raffles, burns, arrivals, newest first |
| `/v1/rook/wallets/:address` | `WalletProfile`: launches, holdings with share and value, the last 50 trades, raffle wins |

The types are `indexer/src/rook/types.ts`, byte-copied to `web-rook/src/rook/apiTypes.ts`; the fixture set `indexer/fixtures/rook/`
(written by `npm run fixtures:rook`: the pons coins composed from the demo recording, the Rook coins from the worked world with a
few invented trades walked along the worked curve, read by the real reader) is byte-copied to `web-rook/public/fixtures/rook/`.
Under `?demo=sample` a pons coin's chart, tape and holders come from the demo recording's own endpoints, reshaped, moved onto the
same clock; the chart's footer prints the bucket that was actually recorded when it is not the one asked for.

### 6.2 What "trending" means

Never age alone. The score is computed in the browser from three fields the board carries (`web-rook/src/rook/boardlogic.ts`):

```
score = ln(1 + volume24h in ETH) + 0.5 · ln(1 + newHolders24h) + 0.25 · ln(1 + buys24h)
```

`volume24hWei` is the pair asset traded in the day; `newHolders24h` is the addresses whose first inbound transfer was in the day
and that still hold (Rook coins: from the reader's transfers; pons coins: from the `events` table's `Transfer`s, first-in per
address kept per coin); `buys24h` is the day's buys (buys per hour × 24). Logs, so one whale's volume cannot drown a coin that
many people are buying. A term the source does not know counts zero; a coin with no term known at all scores null and sorts last,
never as a quiet coin. The demo recording keeps only a chart's largest trades, so its pons rows carry null buys and null new
holders and are ranked on volume alone, which the row's other columns make visible. The king of the hoard is the top score among
coins that can still be traded.

### 6.3 What the launch wizard cannot put on chain (specified, not built)

`CurveFactory.launch(name, symbol, policy, sweepOnly, feeBps)` carries nothing else. The wizard shows an image and X /
Telegram / website fields and says, in the same panel, that they are **stored off-chain and not uploaded yet**: the site never
reports an upload it did not make. Where they will go: a coin-metadata registry in the indexer (`/v1/rook/coins/:address/meta`,
`PUT` with a body `{ image, x, telegram, website, at }` signed by the coin's creator key — EIP-712 over the coin address and the
body's hash, verified against `CurveFactory.creatorOf(coin)` — the image as a data URL of at most 512 kB, served back on the
board row's `logoUrl` and the coin page). Until that exists the fields are a preview and the launch transaction is unchanged.

### 6.4 The small things every launchpad user expects

- **Search**: the masthead box searches the board's rows by name, symbol, nest or address as you type; a full address opens the
  wallet page. The board has its own search under its tabs.
- **Watchlist**: a star per row and on the coin page; the **Watching** tab. Stored in this browser only (`localStorage`,
  `rook-watch`), every read and write wrapped, never required and never sent anywhere.
- **Share**: copies `rook.fun/#/coin/<address>` and says so; the OG meta on `index.html` describes the site, and `og:image` /
  `twitter:image` point at **`/og.png`**, a 1200 × 630 card — the worn mark and the wordmark in ink on the dark ground — that
  `vite.config.ts` draws at build (`scripts/og-image.mjs`: the mark from `marks.ts`, the wordmark as Bricolage Grotesque's
  own outlines for "Rook" at the masthead's instance kept as path data, a small scanline rasteriser, node's zlib for the PNG;
  no renderer, no font engine, no native dependency) and serves at `/og.png` in dev. `test/og.test.ts` decodes it back.
- **First visit**: a four-step "how it works" panel on the board, dismissed once and remembered locally (`rook-intro-seen`).
- **Phones**: a bottom nav — Board / Launch / Nests / Reserves / Wallet — fixed, with the safe-area inset; the board's nest column
  hides under 720 px so the numbers stay in view, and tables scroll inside their own box.

### The one number

`totalCost = 1 − amountOut / (amountIn × Π mid_i)`, from `lib/quote.ts` unchanged (docs/v5/ux-and-reserves.md §1). The ticket
prints it once, in the accent, with the fee tier's share of it and the spread and the impact under *Details*. In fixture mode the
pool hops are labelled estimates.

### The ledger's equation

`(items in custody − tickets in flight) × 1 + to be converted + reserve + float = supply`, where the three ETH-backed terms are the
Floor Module's `ethBackedUnits` split into what the canister will convert into items (`C − float`), the standing reserve's units
(`R`, docs/v5/floor-module.md §13.1) and the module's own float inventory. The indexer's `ReserveReport` carries the module's
books as built (`reserveUnits`, `convertibleUnits`, `floatUnits` beside `ethBackedUnits`), so the ledger always shows the three
rows when a module was read, one zero row when the nest has none, and one row naming the error when it could not be read; the
ledger agrees with the report only if the rows sum to its `backingUnits` *and* the ETH rows to its `ethBackedUnits`. A pure
function `ledgerOf(report)` produces the rows and the verdict; it is unit-tested against the fixture's green (frogs, with a module)
and unverified (apes, without) reports and against hand-built red, re-split and unreadable-module cases.

### The copies, and the test that keeps them copies

Everything under `web-rook/src/lib/`, `src/api/{adapt,source,types}.ts`, `src/Web3.tsx` and `src/deployments/README.md` is
`web/`'s file byte for byte (`api/types.ts` is `indexer/types.ts`; `src/rook/apiTypes.ts` is `indexer/src/rook/types.ts`), and every
set under `public/fixtures/` is `indexer/fixtures/` file for file (regenerated with `npm run fixtures:wrapped` and `fixtures:rook`
in `indexer/`, then copied). `test/copies.test.ts` compares each pair whenever the repository copy is beside the package, so a
change in `web/` or the indexer that is not re-copied fails the suite instead of drifting. The site's own code lives only under
`src/rook`, `src/ui`, `src/pages` and `src/styles`; a `src/lib` file that is not `web/`'s fails the same test.

## 7. What is live, what is a fixture

- The v3/v4 stack has no public deployment (docs/v5/ux-and-reserves.md). With no `VITE_ETCH_API` and no bundled deployment,
  every page says **nothing is deployed** and links to the fixture (`?demo=sample`), where every page carries the FIXTURE banner
  with the set's own note.
- With an indexer that reports `configured: true`, the wrapped pages are live; the ticket can send only with a bundled deployment
  file (`TxButton` refuses otherwise, as in `web/`).
- The Floor Module's books, a coin's launch-fee terms and the OfferBoard's state all arrive through the indexer's reports
  (`ReserveReport`, `WrappedCoin.launchFee`, `HandinReport`) — live or fixture, the same fields — so no page reads a contract of
  its own for them and no build-time list of addresses is needed. The launch wizard alone previews the template's defaults, labelled.
- **Phase-1 mode (2026-09-25, docs/v6/phase1-scope.md §8.3, K5).** `VITE_ROOK_PHASE=1` at build (any other non-empty value fails
  the build). The nav, footer and reserves page drop Bots; `#/bots` and `#/rent` say why they are off; the launch wizard offers
  Raffle and Burn, not YOYO, and launches through the factory in the bundled `src/deployments/stack-4663.json` (refusing if the
  indexer names another); pons coins are left off the board and the tape, and a pons coin's page sends nobody to trade it; a YOYO
  coin's page says its policy is off. There is no mint-desk UI, and a test keeps it so. `TxButton` refuses without a valid bundled
  4663 stack file at stage 0, and before the wallet is asked checks the target on chain: one of the stack's wrappers, factories,
  routers or floor modules, or a coin or curve one of its factories launched (`CurveFactory.routerOf`) — never the desk, the offer
  board or the YOYO vault. The Trust page opens with the Phase-1 risk statement (§0 of that document) and the stack's addresses, a
  one-line banner links to it, and `/.well-known/security.txt` is written once `SECURITY_CONTACT` (site.config.ts) is set.
  `test/phase1.test.ts` is the spec; screenshots `docs/v6/screens/phase1-*.jpg`.
- **Wave 2 (2026-09-26, docs/audit/2026-09-26/fixes-W2-site.md).** The Phase-1 check reads what a transaction says, not only where
  it goes (E-07): the router, the wrapper and the raffle vault a page sends to or approves come from the stack file; each kind of
  target allows only the calls the site makes; every address argument has a rule (the approval's spender is the stack's router, a
  trade pays the connected account, a coin is one the factory launched) or the request is refused. The coin page of a raffle coin
  shows its draws and the keyless steps (pin, resolve, claim, void); the ticket uses the curve's deadline overloads, says when the
  closing buy waits for room (A-03) and offers Graduate / Open refunds at the escape hatch (A-01); the wizard says when launches are
  paused. The Trust page states the roles and powers after the fixes (the Safe takes the whole treasury; the keeper's reach and
  worst case; the guardian; the 7-day rotation; the stale fallback). `test/audit_e07.test.ts`, `test/keyless.test.ts` and
  `test/abis.test.ts` (every ABI entry against forge's `out/`) are its spec.

## 8. Build and serving

Same stack as `web/` (Vite + React + TypeScript + wagmi/viem), same scripts (`dev`, `sync`, `typecheck`, `test`, `build`,
`start`), the same `server.mjs` with the same Content-Security-Policy — `connect-src 'self' https: http://127.0.0.1:* http://localhost:*` —
and the same Dockerfile shape, so `web-rook/` deploys where `web/` deploys by changing the build path. Plain CSS with tokens; no
Tailwind; fonts self-hosted.

## 9. Quality bar

`tsc` clean; `vite build` clean; unit tests for the quote's total-cost composition (ported from `web/test/quote.test.ts`, the Floor
Module through its own spreads and room), the ledger's equation, the fee-tier preview and the launch-fee decay, the reports the
pages draw (YOYO, launch fee on the clock, the offer board), the copies' byte-identity and the social card; screenshots at 400 and
1440 in dark and light (`scripts/screenshots.mjs`, every page including `#/bots/<slug>` and a YOYO coin) with no horizontal
scroll and no console errors. Every column of numbers is set in the mono face with tabular figures (`.num`, `.fig__value`,
`.kv dd`).
