# v6 — the collections catalogue: every collection is an open nest

Written 2026-09-23 on branch `collections-catalog` (from `f101866`), with its code. New files only: `indexer/src/rook/catalog*.ts`,
`indexer/src/rook/catalog-seed.json`, `indexer/src/rook/write-catalog-fixtures.ts`, `indexer/fixtures/catalog/`, the site's
`src/rook/catalog*.ts` and its Nests pages; `indexer/src/api.ts` and `main.ts` each gained the lines that wire it. No contract,
canister, keeper or deploy script is touched. Nothing is deployed; every network use recorded here was a keyless, read-only GET
(and one keyless read of a contract on Robinhood Chain), on 2026-09-23, and is written down with its HTTP status.

## 0. What it is

The site's Nests page used to show the collections in the deployment's wrapped configuration — two fixtures. This document makes it
a **catalogue**: a directory of every relevant collection on Bitcoin, Robinhood Chain, Ethereum and Base, whether or not Rook lists
it, each with its floor (and where that floor came from and how old it is), its items and holders, its volumes, its picture, its
links, a sample of its items, and its **Rook status** against the listing rule of §3. Every entry is an open nest: a page of its
own with the coins on Rook if it is live, and what listing it would take if it is not.

Served by the indexer at `GET /v1/rook/catalog` and `/v1/rook/catalog/<slug>` (types in `indexer/src/rook/catalogTypes.ts`, a
byte copy at `web-rook/src/rook/catalogTypes.ts`), with pictures at `/v1/rook/catalog/<slug>/image` and
`/v1/rook/catalog/<slug>/items/<i>/image`.

## 1. What is curated and what is fetched

| | curated (the seed, a committed file) | fetched (every refresh) |
|---|---|---|
| the list of collections | yes — `catalog-seed.json`, §2 | no: a source's ranking is never the list, so an outage never empties it |
| name, description, links, contract / inscription-collection id, chain | yes | name, description, links and picture are re-read a few times a day (`detailTtlS`, 6 h) and overlay the seed field by field |
| the picture's URL | yes: the collection's public logo at its marketplace (OpenSea's `image_url`, Ordinals Wallet's icon inscription at the content gateway, Nosh's registry image for the chains it relays) — the brand image every aggregator shows; each URL was fetched and answered an image when the seed was written | yes, from the source's directory |
| floor | a **dated record** of what the source answered when the seed was written (`recorded`), served **flagged stale** | yes: the cheapest open listing (Ordinals Wallet's escrows), the stat endpoint's floor (Magic Eden), `stats.total.floor_price` in ETH (OpenSea) |
| items, holders, listed | the dated record | yes |
| 24 h / 7 d volume | never | OpenSea's `intervals`; on Bitcoin, the sum of Ordinals Wallet's recent sales, and **only when the list reaches back that far** — a truncated list is a lower bound, which is not a volume, so it is `null` |
| the item sample (≤ 12) | the old catalogue's inscription ids (Bitcoin); the first eight tokens' pictures read through `tokenURI` on a public RPC (EVM, §2.2), Larva Labs' punk images for CryptoPunks, the projects' own image services for Nouns (`noun.pics`) and Moonbirds (`collection-assets.proof.xyz`), whose `tokenURI` reverts on every public RPC tried | Ordinals Wallet `inscriptions` (Bitcoin, pictures only), OpenSea `collection/<slug>/nfts` (needs the key), else the contract's `tokenURI` through a public RPC and an IPFS gateway (keyless) |
| the sats-per-ETH rate | a dated record (`rate`) | Coinbase's keyless exchange-rates endpoint |
| Rook status | never | computed on every answer from this indexer's wrapped collections and §3 |

A number the source did not give is `null` and printed as —, never as zero and never estimated. A floor that is not from the last
successful refresh carries `stale: true` and its date, and the page says "not the price now" beside it.

## 2. Sources

### 2.1 The seed: two lists

**Nosh's `CollectionRegistry` on Robinhood Chain**, `0x83B35632Ef70ab85DD5B01180FdD7856b68E4B0f`, read end to end on 2026-09-23 with
`collectionCount()`, `collectionList(i)`, `getCollection(nft)` and the `ExternalCollectionAdded(key, vault, slug, chainName,
marketplace)` events (the ABI was taken from Nosh's own page bundle; `docs/v4/nft.md` §1 has the earlier read). **58 collections**
(53 when §1 of v4 read it): **12 real ERC-721s on 4663** — Hashcats, StonkBrokers, Chain Mancers, Cash Cats, Rare Friends Genesis,
RH Machines, OpenCatz, Quotrons, The Royal Mechanica, AGNTS, Rekt Tradooors, Ape Hood — and **46 external**: 30 on Ethereum, 14
on Solana, 1 on HyperEVM (Hypurr), 1 on Zcash (zkSNARKs). Nosh's slugs are OpenSea slugs, and its own `api/collections?slugs=…&stats=1`
(read once, `200`) relays OpenSea's picture, floor and owners per slug; where OpenSea's `/stats` did not answer keyless, that relayed
figure is the seed's record, marked `source: "nosh"`.

**The ordinals collections the old etched site listed**: `web/public/catalogue/catalogue.json` (docs/phase9.md; 50 collections,
fetched 2026-09-23 01:40 UTC from Ordinals Wallet's index, ranked by lifetime volume; `staging/catalogue/listings.ts` and
`staging/bootstrap/listings.ts` turn the same file into the staging listings). Each carries its floor and listed count as of that
fetch, its supply, its inscription-number range, and up to eight inscription ids with names — the item sample. Twelve of them
(Bitcoin Frogs, NodeMonkes, Bitcoin Puppets, Quantum Cats, Runestone, OMB, Taproot Wizards, Bitcoin Wizards, Bitcoin Punks,
Natcats, Pixel Pepes, The Wizards of Ord) were re-read live on 2026-09-23 17:xx UTC (§2.2) and carry that fresher record.

Plus the Ethereum and Base blue chips probed on OpenSea for §2.2 that Nosh does not list (Azuki, Pudgy Penguins, Lil Pudgys,
Doodles, CloneX, Milady, BEANZ, Autoglyphs; onchain gaias, Kemonokaki, Based Fellas, based punks, Basenames).

**114 entries**: 50 Bitcoin, 12 Robinhood Chain, 31 Ethereum, 5 Base, 16 other (Solana, HyperEVM, Zcash). `parseSeed` refuses a
duplicate slug, an EVM id that is not a lower-case contract, a Bitcoin entry that is not an inscription collection, an undated
record, a sample over twelve.

### 2.2 The live sources, and what they answered on 2026-09-23

Every request is a GET with a User-Agent (`rook-indexer-catalog/1 (read-only; collections directory)`: Cloudflare fronts 403 a
bare client), spaced per host (Ordinals Wallet 900 ms, Magic Eden 600 ms, OpenSea 300 ms, pictures 150 ms), 15 s timeout, and an
answer that is not the documented shape is `ok: false` with its status — never a number.

| source | endpoint | keyless? | answered |
|---|---|---|---|
| **Ordinals Wallet** | `turbo.ordinalswallet.com/collection/<slug>` (name, icon, supply, socials), `…/escrows` (open listings → floor, listed), `…/sales` (→ 24 h / 7 d), `…/inscriptions?limit=12` (the sample) | yes | **200** on all four for every one of the 12 slugs re-read; its `collections?offset=&limit=100` index answered **200** for 8 pages, then **503** on `?slug=`, `search/` and a 9th page — so the index is not used live; the best-known collections are not in its first 800 rows anyway (docs/phase9.md §11.3) |
| **Magic Eden** | `api-mainnet.magiceden.dev/v2/ord/btc/stat?collectionSymbol=<s>` (floor, owners, supply, listed) | yes | **200** for `bitcoin-frogs` and `bitcoin-wizards`; **503 "no healthy upstream"** for the other ten, every try (docs/v5/floor-feed.md §2 saw the same) |
| **ord** | `ordinals.com/content/<id>` (an item's picture) | yes | **200 image/webp** for a frog; `/inscription/<n>` with `Accept: application/json` answers **406 "JSON API disabled"**, so ord is a picture source only |
| **OpenSea v2** | `api.opensea.io/api/v2/collections/<slug>` (name, description, image, contracts, links, supply), `…/stats` (floor in ETH, owners, 1 d / 7 d volume), `collection/<slug>/nfts?limit=12` (the sample, with pictures) | the first two answered keyless with intermittent **401**s (`proof-moonbirds`, `veefriends` stats, `degods` stats, `basedpunks` stats); `nfts` answers **401 "Missing an API Key"** keyless; `collections?chain=base&order_by=…` (the ranking) **401** keyless | **200** for 54 slugs' `collections`, **200** for 49 `stats`; Robinhood Chain collections answer with `chain: "robinhood"` |
| **the contract itself** (`tokenuri`) | `eth_call tokenURI(id)` from the first of ids 1, 0, 2 that answers, walking up past ids that revert (a burned #1: Chain Mancers'), at most 16 asked, else `tokenByIndex`, on a public RPC (`eth.drpc.org` / `xrpc.cl/eth`, `mainnet.base.org`, `rpc.mainnet.chain.robinhood.com`); the metadata decoded from a `data:` URI, or fetched from `https`, or from `ipfs://` through `ipfs.filebase.io` then `4everland.io`; its `image` (ipfs → Filebase; a plain-text SVG `data:` URI or raw `image_data` SVG re-encoded as base64) is the item's picture | yes | RPCs **200** for every EVM collection; **`ipfs.io`, `dweb.link` and `nftstorage.link` answer 429 "switching to a service worker gateway only" to every path** and `gateway.pinata.cloud` 429, so an `https://ipfs.io/ipfs/…` URL a source names is renamed onto Filebase, which answered a burst of a dozen with 200 (a first fetch of a picture it has not served can take 20 s); item pictures were read for the seed this way for 39 EVM collections (§5) |
| **Coinbase** | `api.coinbase.com/v2/exchange-rates?currency=ETH` → BTC per ETH | yes | **200**: 0.031683 BTC, i.e. 3,168,300 sats per ETH |
| **Nosh** | `usenosh.app/api/collections?slugs=…&stats=1` | yes | **200** for all 58 slugs; used for the seed only, never live (an undocumented private API, and its figures are OpenSea's) |

`OPENSEA_API_KEY` is read from `process.env` **at the moment of each OpenSea call**, never at load, and goes into the `x-api-key`
header of `api.opensea.io` and nowhere else. It is in no log line (the refresh log carries counts and statuses), in no error (an
error is `HTTP <status>` or `no answer: <name>`), and in no answer. The tests use a fake key and a mocked `fetch` and assert all
three, and that the key is on every OpenSea request and on no other host. Unset, OpenSea is asked keyless and the catalogue says
so in its `sources` line.

### 2.3 Refresh and cache

`CATALOG_TTL_S` (default 600, clamped to 300–900). `list()` never waits on the network: it answers from the snapshot it has and
starts a refresh in the background when the last attempt is older than the TTL, one at a time. Before the first refresh, and
whenever every source fails, the snapshot is the seed's dated records with `stale: true`, `refreshedAt: null` and, per source,
`asked`/`answered`/`statuses`. A refresh that fails in part serves what did answer and flags the entry and the catalogue stale.
`refreshedAt` is set only when every read answered. Slow-moving facts (name, description, picture URL, links, the item sample) are
kept for `detailTtlS` (6 h) so a refresh asks for floors and stats and not for everything: about 50 × 2 + 12 + 60 × 1 ≈ 170 spaced
requests every ten minutes, ≈ 250 more four times a day. The HTTP answer carries `cache-control: public, max-age=60`.

### 2.4 Pictures

A collection's picture is its public logo at its marketplace — OpenSea's `image_url` (a `seadn.io` CDN URL), Ordinals Wallet's
icon, Nosh's registry image for the chains it relays — the same brand image every aggregator shows. Ordinals Wallet names its icon
as a preview on its own host (`turbo.ordinalswallet.com/inscription/preview/<id>`), which answers **404**; the parser takes the
icon's inscription (`icon_inscription`, when its content type is an image) at the content gateway instead, renames an IPFS icon
onto a working gateway, and drops a Discord attachment (those links expire and answer 404). An item's picture is the ordinals
content gateway (`ordinals.com/content/<id>`), the marketplace's CDN, or the token's own metadata `image` (an `ipfs://` one on
Filebase's gateway; an on-chain SVG as the `data:image/svg+xml;base64` URI it is, which the image route decodes and serves without
a request — Hashcats, StonkBrokers, Rare Friends, OpenCatz and Chain Mancers on Robinhood Chain are drawn that way). The catalogue
carries these URLs (`image`, `sample[i].image`); 112 of the 114 seed entries have a logo and 85 have item pictures (§5).

**Live**, the site never loads them from there: the footer's promise stands, and a visitor's browser talks to our own indexer
only. `GET /v1/rook/catalog/<slug>/image` and `…/items/<i>/image` fetch the URL the catalogue holds — the route takes a slug and
an index, never a URL, so a visitor names no host — from any public `https` host (loopback, link-local, private ranges and bare
hostnames are refused before any request), serve only an `image/*` answer (or an untyped `application/` / `binary/octet-stream`
one whose first bytes are PNG, JPEG, GIF or WebP — CyberKongz' bucket names no type) under 1.5 MB (an Azuki is 637 kB, a Moonbird
782 kB) with `content-length`, `nosniff`, a `sandbox` CSP (an SVG opened on its own runs nothing) and a day's cache, and answer **404 for anything else, and the page draws a mark** (two letters cut from the name on a ground from the
slug: `Picture` in `Collection.tsx`, whose `onError` is the mark too, so there is never a broken image). The cache is bounded
(800 pictures, 64 MB, oldest out) and remembers a miss for ten minutes.

**In the labelled fixture** (`?demo=sample`) there is no indexer to fetch through, so the page loads each picture straight from
the public URL the seed recorded (with no referrer), with the mark as the fallback; the fixture's note and the footer say so. The web server's
`img-src` is open to `https:` for these two cases; the app names no other image host.

## 3. The listing rule, with the numbers

`docs/v5/launch-gates.md` G7 stages custody: at **stage 0 (the canary)** a collection may hold **3 items, ≤ 10 ETH**, the whole
custody **6 items, 10 ETH**, and one item at most **10 ETH** ("one blue-chip"); from stage 1 the per-item maximum is **1.2 × floor**
and the collection cap 40 ETH (12 items), then 160 ETH (48), then 640 ETH (192) — ×4 a stage, ≥ 4, 8 and 12 weeks apart without an
incident. The 20% is also the keeper's cap step bound (custody-hardening H12). Nothing is deployed, so the current stage is 0.

**The rule** (`catalogRule.ts`, pure, tested): a collection is **listable** when one item at its floor, with the 20% allowance,
fits both the stage's per-item maximum and its per-collection cap —

```
1.2 × floor ≤ min(perItemMax, perCollectionCap)          stage 0: min(10, 10) ETH  ⇒  floor ≤ 8.333 ETH
                                                          stage 1: 40 ETH           ⇒  floor ≤ 33.333 ETH
                                                          stage 2: 160 ETH          ⇒  floor ≤ 133.333 ETH
                                                          stage 3: 640 ETH          ⇒  floor ≤ 533.333 ETH
```

and the status says how many of the stage's 3 slots that floor fills (`slotsNow` = min(3, ⌊cap / (1.2 × floor)⌋): BAYC at 6.847 ETH
fills one; MAYC at 0.91 fills three). In order of precedence:

1. **live** — listed on Rook: this indexer's wrapped configuration names it, matched by contract (an EVM chain) or by name and slug
   (Bitcoin, whose config carries no marketplace slug). The status carries the nest's slug and its coin count; the entry carries
   what custody holds (`heldByRook`). A nest the seed does not name is added from the indexer's own reads, `curated: false`.
2. **unsupported**, with why: **Base** — no native bridge route to 4663 (docs/v6/home-chain-vault.md §2.3, §8.3: ≈ 7 days down,
   14 up through Ethereum; a fast bridge is a different trust question and is not built); **Solana, Zcash, HyperEVM** — not a chain
   a vault here exists on (the home-chain vault is an EVM contract with a native bridge; the canister holds Bitcoin only);
   **ERC-1155** and anything else that is not one ERC-721, CryptoPunks (the vault's punk path) or an inscription collection.
   Robinhood Chain's own collections need no bridge at all (`docs/v4/nft.md` §1) and are judged on the floor alone.
3. **later**, with the stage: the floor is above the current stage's limit — the status names the first stage whose caps hold it
   (CryptoPunks at 34 ETH: stage 2; Autoglyphs at 59.69: stage 2) or says even stage 3's do not — or **no floor is known**, so the
   rule cannot be checked (`stage: null`): not listable until a source reports one.
4. **listable** otherwise.

Bitcoin floors are in sats and go through the catalogue's sats-per-ETH rate (a fresh Coinbase read, or the seed's dated one
flagged stale); no rate, no ETH floor, and the status is `later` with no stage. A listable status checked against a stale floor
says so in its sentence.

The API carries the rule it applied (`rule`: stage, per-item maximum, per-collection cap and items, allowance, the highest listable
floor, the sentence and the document) so the page prints the numbers the service used, not a paraphrase.

## 4. The site

- **Nests** (`#/collections`) is the catalogue: a row per collection with its picture (or mark), chain badge, standard, floor (in
  its unit and in ETH, dollars when the indexer has a rate), items, holders, 7 d volume and status badge; chain tabs (All / Bitcoin /
  Robinhood / Ethereum / Base / Other, with counts), search (name, slug, contract, chain, status, Rook slug), sorts (status, floor,
  7 d volume, items, holders, name), the rule panel with the stage's numbers, and the sources line with each source's
  answered/asked and HTTP statuses. Unsupported rows are dimmed, never hidden.
- **A collection** (`#/collection/<slug>`, by the catalogue slug, the nest's Rook slug, or the contract) is laid out like Nosh's:
  a header with the picture, name, chain badge, status, links and description, and the **Launch a coin** button — enabled for a
  live or listable collection, disabled with the reason otherwise; a stats strip (floor, dollars, items, holders, 24 h and 7 d
  volume, held by Rook) with the floor's source and age under it; the item grid (the sample, pictures through the indexer, marks
  where there are none); for a live nest, the coins on it, custody, activity and tickets from the wrapped reads; the **Request
  listing** panel with the status, the rule, and what listing takes in order (a timelocked custody-policy proposal at the stage's
  caps, a wrapper and a posted floor, the wrapped configuration) — it records nothing and says so: there is no form; the sources
  panel, per read.
- **The board's nest filter** is the catalogue: a live nest can be picked, the rest are listed greyed with their status.
- **The launch wizard's nest step** is the catalogue in status order: a live nest launches (its curve factory exists); a listable
  one walks the wizard, previews ETH-per-item against the catalogue's floor (said as such), and the transaction button explains
  that the launch waits for the listing; the rest are greyed with their reason.
- **Home's hoard strip** counts the catalogue's live nests and says how many collections the catalogue holds.
- `useCatalog` / `useCatalogEntry` read the hosted indexer, or the `catalog` fixture set under `?demo=sample`; `useCatalogSource`
  is null when there is neither, and the pages say nothing is deployed and offer the fixture.

## 5. The fixture

`npm run fixtures:catalog` (indexer) writes `indexer/fixtures/catalog/` from the seed with **no network** — every floor is the
seed's dated record, flagged stale — over `test/wrappedWorld.ts`, so Bitcoin Frogs and Bored Ape Yacht Club are live with the
worked world's invented coins and custody. Every entry carries the public picture URLs the seed recorded (§2.4), each fetched
on 2026-09-23 and answering an image. **Logos: 112 of 114** — every Ethereum (31), Base (5), Robinhood Chain (12) and other-chain
(16) collection, and 48 of 50 on Bitcoin; OrdiBots and Recursive Frogs have none (OrdiBots' icon is an expired Discord attachment
and its inscriptions are text; Recursive Frogs' icon inscription is HTML), so they show their mark. **Item pictures: 85** — 43
Bitcoin (the old catalogue's inscriptions at the content gateway; the other seven collections' inscriptions are text, HTML or
3D models, which no `<img>` draws), all 12 on Robinhood Chain, 28 of 31 on Ethereum and 2 of 5 on Base. Where no keyless source
gives item pictures the grid keeps marks and the page says **"Item pictures need the OpenSea key"**: Kaiju Kingz and Onchain Gaias
(their metadata hosts no longer resolve), Kemonokaki (its IPFS metadata is on no gateway), Autoglyphs (its token is ASCII text,
not a picture), Max Pain and Frens and Basenames (no sequential `tokenURI`), and the 16 Solana / HyperEVM / Zcash collections
(no keyless reader here). The old etched site's artwork files are no longer copied in: the page loads the URLs. Copied to `web-rook/public/fixtures/catalog/` and checked byte for byte by
`web-rook/test/copies.test.ts` (the set is one of the indexer's) and `test/catalog.test.ts` (the types too).

## 6. Tests and gates

Indexer (`test/catalog.test.ts`, 12 tests): the rule's numbers and every status; each source's parser against the shapes recorded
above (the bought escrow does not count, a truncated sales list is no volume, a USDC floor is no floor, an HTML inscription is not a
picture); the seed's invariants; the seed-only snapshot; a refresh over a fake network (fresh floors, samples, per-entry source rows
with statuses, the User-Agent on every request, the key on OpenSea's and on no other host and in no log or answer, details kept
across refreshes); pictures (public hosts only, `image/*` only, cache, bounds, a miss is null); the `tokenURI` source over a fake RPC (ids from 1, 0 or 2, burned ids walked past, `tokenByIndex`, `data:`, `https` and `ipfs://` metadata, a plain-text SVG re-encoded, a limiting gateway); dead gateways renamed, Ordinals Wallet's icon rules, untyped rasters sniffed; the seed's pictures all https or base64 SVG and on no dead host; every source failing; `list()` not
waiting and spacing; the uncurated live nest; the endpoints including the picture routes and the index. `test/guards.test.ts` now
names `catalogSources.ts` as the one file that talks to anything but the RPC. Site (`test/catalog.test.ts`, 5 tests): the copies,
the fixture's shape, tabs / search / sorts, the wizard's order and `launchable`, the words. Gates before each commit: indexer
`tsc` + `npm test` (99), site `tsc` + `npm test` (33) + `vite build`, and `scripts/screenshots.mjs` at 400 and 1440, dark and light,
over every page including the catalogue, a live nest, a Robinhood Chain collection, a Bitcoin one, a `later` one, an unsupported one
and a missing slug, with no console error and no horizontal scroll.

## 7. Open

- Magic Eden's Bitcoin API answered for two symbols of twelve; the Bitcoin floors are Ordinals Wallet's book, one venue.
- OpenSea's item sample needs the key; keyless, the `nfts` endpoint is 401 and the grid is the contract's own `tokenURI` through
  a public RPC and an IPFS gateway instead. The public IPFS gateways are thinning out (ipfs.io, dweb.link and nftstorage.link
  stopped serving paths): the pictures lean on Filebase's, and a collection whose metadata no gateway serves shows marks and says
  the key is needed. The `collections` ranking endpoint is 401 keyless too, which is
  why the list is curated and not ranked.
- The 4663 collections' floors are OpenSea's `stats` for `chain: "robinhood"`; Nosh's vaults hold items of three of them (63 in
  all, docs/v4/nft.md §1) — the catalogue reports what Rook holds, not what Nosh holds.
- "Request listing" records nothing by design; when a channel exists that a person reads, the panel will say who.
