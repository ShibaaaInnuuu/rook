# Ordinals APIs: two hosted providers, agreeing, or nothing

Rook runs no ord of its own. Every ordinals fact a frog path needs — where an inscription sits, what an output carries, what
an address holds, how deep a transaction is, what a collection has listed — comes from hosted providers, and is believed only
when **two providers agree on it at the same height**. This document records what was measured on 2026-09-23, which provider
answers which read, the rule, what replaced ordinals.com where, and what it costs.

## 1. What was measured (2026-09-23, from a residential IPv4+IPv6 connection, `curl`, one Bitcoin Frog: #3487, `3a5b…a703i0`)

Every request carried `User-Agent: rook-ordapi-survey/1`. HTTP codes are as answered; latency is one request unless noted.

| Provider | Endpoint | HTTP | Latency | IPv6 (`curl -6`) | Notes |
|---|---|---|---|---|---|
| **ordinals.com** (ord 0.2x) | `GET /r/inscription/<id>` | **200** | 0.20–0.52 s (5 samples) | **200** | recursive endpoint: `satpoint`, `output`, `value`, `address`, `content_type`, `height` |
| | `GET /r/utxo/<outpoint>` | **200** | 0.19–0.20 s | **200** | `inscriptions`, `runes`, `sat_ranges`, `value`; spent/unknown → 404 |
| | `GET /r/blockheight` | **200** | 0.20 s | 200 | plain number |
| | `GET /inscription/<id>` (Accept: json), `GET /output/<outpoint>` | **406** `JSON API disabled` | — | — | the JSON API the keeper's `OrdHttp` used |
| | 12-request burst on `/r/blockheight` | 12 × 200 | — | — | no rate-limit headers; Cloudflare in front; answers with no User-Agent (200) |
| **Ordinals Wallet** `turbo.ordinalswallet.com` | `GET /inscription/<id>` | **200** | **0.06–0.08 s** | **200** | `satpoint`, `value`, `address`, `content_type`, `genesis_height`, meta; unknown → 404 `{"error":true}` |
| | `GET /blockheight` | **200** | 0.23 s | — | plain number; `cache-control: max-age=5` |
| | `GET /wallet/<address>` | **200** | 0.55–1.15 s | — | 59 KB for this wallet: every inscription with meta; fine for a check, not for a gate |
| | `GET /collection/bitcoin-frogs/escrows` | **200** then **503** | 0.26–0.47 s | — | the book (46 KB, 200 at 17:2x; 503 at 17:5x): intermittent |
| | `GET /collection/bitcoin-frogs` | 200 | 0.07 s | — | collection card |
| | `/outpoint/…`, `/utxo/…`, `/output/…`, `/tx/…`, `/status` | **404** | — | — | **no per-output view** |
| | 12-request burst | 12 × 200 | — | — | no rate-limit headers; answers with no User-Agent |
| **UniSat** `open-api.unisat.io` | `GET /v1/indexer/inscription/info/<id>` (no key) | **403** `code -2004` | 0.3–1.1 s | **403** (reachable) | `"exceeded the request limit for unauthenticated requests. Please provide Authorization header with format Bearer {token}"`; 403 with no User-Agent too |
| | `GET /v1/indexer/blockchain/info` (no key, 3 spaced calls) | 403 403 403 | — | — | no unauthenticated allowance in practice |
| | keyed reads | **not measured** | — | — | no `UNISAT_API_KEY` was in this machine's environment; the shapes below come from the service's `swagger.yaml` (815 KB, fetched) and are marked UNVERIFIED in code |
| **Hiro** `api.hiro.so` | `GET /ordinals/v1/inscriptions/<id>`, `?output=`, `?address=`, `/transfers`, `GET /runes/v1/…` | **410 Gone** | 0.09–0.18 s | 410 (reachable) | `"This API has been deprecated and is no longer available."` — retired 2026-03-09, migration guide points to Xverse. Still sends `x-ratelimit-limit-minute: 50`, `…-second: 20` |
| **Magic Eden** `api-mainnet.magiceden.dev/v2/ord/btc` | `stat?collectionSymbol=bitcoin-frogs` | **200** | 0.12 s | — | `floorPrice: 480000`, `totalListed: 1010`, `owners: 5143` |
| | `tokens?tokenIds=`, `tokens?collectionSymbol=…&listed=true`, `collections/…`, `activities?…kind=list` | **503** `no healthy upstream` | 0.1–0.3 s | 503 | listings unreachable all afternoon; the stat endpoint is another upstream |
| **Ordiscan** `api.ordiscan.com` | `GET /v1/inscription/<id>` | **402** | 0.24 s | **402** (reachable) | keyed, or x402 micropayment |
| **Xverse / Secret Key Labs** `api.secretkeylabs.io` | `GET /v1/ordinals/inscriptions/<id>` | **404** route not found | 0.27 s | 404 (reachable) | keyed (`x-api-key`); the inscription route has another path (docs are JS-only; not resolved) |
| **Maestro** | `xbt-mainnet.gomaestro-api.org`, `mainnet.gomaestro-api.org` | **no DNS** | — | — | docs.gomaestro.org/bitcoin → 404 |
| **OrdinalsBot** | `api.ordinalsbot.com`, `docs.ordinalsbot.com` | **no DNS** | — | — | `ordinalsbot.com` resolves (IPv4 only) |
| **BestInSlot** `api.bestinslot.xyz` | `/v3/inscription/single_info_id` | 301 | — | — | moved; keyed |
| **esplora** `mempool.space/api` | `GET /tx/<txid>/outspend/<vout>` | 200 | 0.12–0.24 s | **200** | |
| **esplora** `blockstream.info/api` | same | 200 | 0.37 s | **200** (0.78 s) | |

DNS: `open-api.unisat.io`, `api.hiro.so`, `turbo.ordinalswallet.com`, `ordinals.com`, `api-mainnet.magiceden.dev`,
`api.ordiscan.com`, `mempool.space`, `blockstream.info` all publish AAAA records, and the ones that answer at all answered over
IPv6 from here. Maestro's and OrdinalsBot's API hosts do not resolve.

### What this changes about the plan

- **Hiro is gone.** It was to be the second provider; it answers 410 to everything. Its successor (Xverse's API) is keyed and
  its route layout could not be read from JS-only docs, so it is not implemented; the third slot below is open for it.
- **ordinals.com is half on.** Its *recursive* endpoints (`/r/…`), which the canister always used, answer keyless and over
  IPv6; the *JSON API* (`/output`, `/inscription`), which the keeper's `OrdHttp` used, is disabled. So the canister's frog
  paths were not failing on ordinals.com today; the keeper's floor was (`contents: null` → L-unverified). Either way, one
  provider's word was all there was, and that is what §3 replaces.
- **The keyless pair is ord + Ordinals Wallet.** Both locate an inscription; only ord (and UniSat) have a per-output view.

## 2. The providers, behind one interface

Keeper: `keeper/src/ordapi.ts` (`OrdProvider`); canister: `autobuyer/canister-rs/src/ordapi.rs` (`Provider`, `Kind`) with the
outcalls in `ordnet.rs`. The interface is ord-shaped — `Location { output, offset, value }` and `Contents { inscriptions,
runes, value }` are the types `custody.rs` always had — so a self-hosted ord is one more `Kind::Ord` provider with another
base URL, not a new dialect.

| Read | UniSat (key) | ord recursive (keyless: ordinals.com; or your own) | Ordinals Wallet (keyless) | esplora |
|---|---|---|---|---|
| `height()` | `/v1/indexer/blockchain/info` → `data.blocks` | `/r/blockheight` | `/blockheight` | `/blocks/tip/height` |
| `inscription(id)` → satpoint, value, address, content type, genesis height | `/v1/indexer/inscription/info/<id>` → `utxo.{txid,vout,satoshi}`, `offset`, `address`, `contentType`, `height` | `/r/inscription/<id>` → `satpoint`, `value`, `address`, `content_type`, `height` | `/inscription/<id>` → `satpoint`, `value`, `address`, `content_type`, `genesis_height` | — |
| `output(outpoint)` → inscriptions, runes, value; `null` = spent/unknown | `/v1/indexer/utxo/<txid>/<vout>` (`inscriptions[].inscriptionId`, `satoshi`, `isSpent`) + `/v1/indexer/runes/utxo/<txid>/<vout>/balance` (`spacedRune`) | `/r/utxo/<outpoint>` (`inscriptions`, `runes{}`, `value`; 404 = spent) | **none** | — |
| `addressInscriptions(addr)` | `/v1/indexer/address/<addr>/inscription-data?cursor&size` | — | `/wallet/<addr>` → `inscriptions[].id` | — |
| `txConfirmations(txid)` | `/v1/indexer/tx/<txid>` → `confirmations` | — | — | `/tx/<txid>/status` + tip |
| `listings(collection)` | `/v3/market/collection/auction/list` is a query-style **POST** (`filter.collectionId`, `sort`, `start`, `limit ≤ 100`); the keeper's `Http` gate is GET-only, so **not wired** (`Unsupported`) | — | `/collection/<slug>/escrows` (intermittent 503) | — |

Rune names are compared with spacers removed and upper-cased (`DOG•GO•TO•THE•MOON` = `DOGGOTOTHEMOON`): providers print
them differently, the letters are the name. The canister's `Contents.runes` now carries the spacer-less form.

**UniSat's shapes are from `https://open-api.unisat.io/swagger.yaml`** (`InscriptionData`, `UTXO`, `InscriptionItem`,
`RunesBalance`, `TxHistoryItem`) and have not been exercised with a key: the parsers refuse anything unexpected, and a refusal
is "one down", which is unverified. An envelope with `code ≠ 0` (a quota or key error is `code -2004` under HTTP 403) is an
error, never "not found"; only HTTP 200, `code 0`, `data null` reads as unknown.

### 2.1 The key

`UNISAT_API_KEY` is read by `keeper/src/ordapi.ts` and nowhere else in the keeper, at the moment of a call, into
`Authorization: Bearer` of `open-api.unisat.io` and no other host (the `Http` gate refuses another host's URL). `log.ts`
reads it only to cut it out of every line, as it does `OPENSEA_API_KEY`; `test/send-guards.test.ts` and
`test/ordapi.test.ts` keep both facts true. No key: no request (`missing UNISAT_API_KEY`). `HIRO_API_KEY` is not read by
anything: there is no Hiro.

In the canister the key is the one secret `ordnet.rs` holds: `set_unisat_key(text)` (operator only), kept on the heap —
an upgrade clears it, set it again — never in the install argument (which a timelocked proposal publishes), never returned
by a query (`ord_providers` lists names and bases), never in the log (the note says how many characters), sent in exactly
one header to exactly one host. **An outcall's headers are visible to every replica of the subnet**: use a read-only key on a
plan whose loss costs a quota, not money. `ordnet::tests` reads the source to keep the key in one place.

Every request from either side carries a `User-Agent` (`etch-keeper-ord/1 (read-only)`, `etch-autobuyer/1 (read-only)`).
Cloudflare fronts every provider here; none 403'd a missing User-Agent today except UniSat (which 403s everything keyless),
and the header costs nothing.

### 2.2 Rate limits and caching

- UniSat free tier: 5 calls/s, 2,000 calls/day (documented). The keeper spaces UniSat at 250 ms; ord and Ordinals Wallet at
  250–500 ms. A verified read costs **three calls per provider** (height, fact, height) — so one location is 9 UniSat-day
  units with three providers; at 2,000/day that is ~220 verified locations, enough for a tick every few minutes, not for a
  crawl. Contents through UniSat is four calls (utxo, runes, two heights).
- A **429** parks that provider for `Retry-After` (capped at 60 s) or, without one, 5 s doubling per consecutive 429 to 60 s;
  a parked provider is "rate-limited", not asked, and the read is unverified until it returns (`keeper/src/backoff.ts`'s
  schedule). Hiro's headers (`x-ratelimit-*`) are the only rate-limit headers any provider sent.
- **Cache** (keeper): `listings(collection)` is cached 45 s; it is one provider's word and moves no money by itself (the
  outpoint check that gates a purchase is the verified `output` read). Nothing that gates custody, minting, delivery or
  judging is cached: the height is read around every such fact.

## 3. The rule (the fail-safe)

A fact is **verified** when, for every provider asked:

1. its height was read before and after the fact and did not move (a block arriving mid-read makes two questions of one);
2. the fact was read (an HTTP error, a body that does not parse, an answer about another inscription, a 429: "one down");
3. at least **two** providers answered the read (`MIN_PROVIDERS`); a read only one supports is "unsupported";
4. every height is within **one block** of every other (`MAX_HEIGHT_GAP`; two apart is a stale indexer or a reorg: "stale");
5. every answer is the same answer ("disagree" otherwise). Known to one and unknown to another is a disagreement, not "the
   one that knows wins". For inscriptions, fields one provider lacks (no address, no value) are filled from another, but a
   field both give must match. All unknown is a verified `null`.

Anything else is **unverified**, and unverified is the missing answer every flow already handles: the floor's
`L-unverified` rule excludes the listing (`VerifiedOrd.contents` → `null`); `deposits::check_output` waits ("ord has not
indexed … yet"); `frogflow::custody_holds` reads again next tick; a hand-in is judged `Later`; `redeem` waits; the buyer
refuses to sign a fee input it cannot verify (`buyer_input_clean`: *refused: … could not be verified, so it is not signed*).
Nothing is retried in a tight loop: the tick repeats. The keeper logs `ord_unverified` **once an hour per (read, reason)**
(`FEED.muteS`); the canister's reason string names the provider and the heights (`unverified: ord at 968323, ordinalswallet
at 968321 are 2 blocks apart on the frog's location`).

Transaction facts (confirmations) are verified across every provider that answers them plus esplora (mempool.space,
blockstream.info): two sources at least, equal within one block; a transaction one explorer knows and another does not is a
disagreement. So the keyless pair alone verifies a confirmation count through the two explorers.

### 3.1 What replaced ordinals.com where

| Path | Before | Now |
|---|---|---|
| Canister `custodynet::CustodianIo::locate` (deliveries via `redeem`, `frogflow::custody_holds`, hand-in judging via `frogflow::accept`, deposits via `frogflow` → `deposits::check_output`) | ordinals.com `/r/inscription` alone | `ordnet::locate`: every provider (ord + Ordinals Wallet keyless; + UniSat with the key) agreeing |
| `custodynet::CustodianIo::contents` (the six deposit checks' contents, custody's fee-input choice, hand-in seller-output check) | ordinals.com `/r/utxo` alone | `ordnet::contents`: every provider with a per-output view (ord + UniSat) agreeing; **keyless, this is one provider and therefore unverified** — the operator sets the UniSat key (or adds an ord of their own) before deposits or deliveries can proceed on mainnet |
| `custodynet::buyer_input_clean` (the buyer's belt-and-braces before signing a fee input) | ordinals.com, fail closed on mainnet; skipped with a note on test networks | verified contents, fail closed wherever any provider is configured; skipped with a note only where none is |
| `probe_frog` (operator rehearsal) | ordinals.com | mainnet's providers as configured now (`ordnet::mainnet_now`) |
| Canister transforms `custody_ord_inscription` / `custody_ord_contents` | one dialect | `ordapi_height` / `ordapi_location` / `ordapi_contents` / `ordapi_runes`, dispatching on a provider-kind byte in the context |
| Keeper floor feed (`floorsources.withContents`, the `L-unverified` rule) | `OrdHttp` on ordinals.com's JSON API (406) or `null` | `VerifiedOrd` over `OrdFeed` (`mainnetFeed()`); `OrdHttp` kept for an ord of one's own |
| `oracle.rs` (rune deliveries: ord `/r/utxo` + Ordinals Wallet address balance) | unchanged | unchanged — already two sources; migrating its `/r/utxo` read onto `ordnet::contents` is listed under open problems |

### 3.2 The canister's providers and the IC's IPv6

IC replicas have IPv6 only; an outcall first tries the host over IPv6 and, if that fails (an IPv4-only server), retries
through a SOCKS proxy on the boundary nodes (docs.internetcomputer.org, "HTTPS outcalls: how it works"). All three providers
here (`ordinals.com`, `turbo.ordinalswallet.com`, `open-api.unisat.io`) publish AAAA records and answered over IPv6, so the
canister needs no proxy route. If a provider is ever IPv4-only (Maestro's and OrdinalsBot's hosts do not resolve at all today;
`ordinalsbot.com` is IPv4-only), the SOCKS fallback is automatic and operator-visible only as latency; the operator-assisted
alternative is `add_ord_provider(https://…)` pointing at an ord of one's own on an IPv6 host. Replicated outcalls need every
replica to see the same body: the transforms reduce each reply to `ok <fact>` / `err <reason>`, and `verified` runs on the
reduced facts.

Providers on the canister, by network: **mainnet** — ord (ordinals.com) + Ordinals Wallet, + UniSat once its key is set;
**any network** — up to two ords the operator adds (`add_ord_provider`, https only, no duplicate base). Adding providers can
only make a read *fail* (every provider must agree), never make a false fact pass: a stolen operator key that adds a lying
ord produces "unverified", which moves nothing. These three configuration methods are exempt from the pause gate for that
reason (`canister::tests::every_operator_method_that_acts_refuses_while_paused`).

## 4. Cost

| Provider | Tier | Price | Limits | Source |
|---|---|---|---|---|
| UniSat | Free ("Tester") | $0, "one-time-key for your short-term project" | **5 calls/s**; "up to 2,000 calls/day" (docs.unisat.io, Open API "Rate Limits") or "30,000 calls/month" (docs.unisat.io/developer-support/plans) — the two pages disagree; budget for the lower | docs.unisat.io; unisat.io/open-api/plans |
| UniSat | Specialist / Enterprise / Pay as you go / Custom | **not captured**: the plans page renders its prices in JavaScript (the fetched page showed "0 USD · 1,000 calls" placeholders for both Specialist and Enterprise) — read them at developer.unisat.io/plan before buying | Specialist described as "2,000 calls/day and 5 calls/second" in the docs' search snippet; Enterprise "dedicated infrastructure and SLA" | unisat.io/open-api/plans, docs.unisat.io |
| ord recursive (ordinals.com) | keyless | $0 | none published; 12-burst passed; **no terms, no SLA**: the JSON API was switched off without notice, the recursive API can be | measured |
| Ordinals Wallet | keyless | $0 | none published; 12-burst passed; the book endpoint 503s intermittently | measured |
| Hiro | — | — | 50 rpm keyless / 500 rpm keyed (its headers still say 50/min, 20/s) | **retired 2026-03-09 (410)** |
| Ordiscan (third-slot candidate) | keyed or x402 | not captured (docs 403 to a fetch) | — | api.ordiscan.com answered 402 |
| Xverse API (Hiro's named successor) | keyed (`x-api-key`) | not captured (JS-only docs) | — | api.secretkeylabs.io reachable |
| esplora (mempool.space, blockstream.info) | keyless | $0 | mempool.space asks heavy users to self-host | measured |

A tick that verifies one location and one output's contents with three providers is ~21 provider calls, 13 of them to
UniSat; on the free tier that is ~150 such ticks a day.

## 5. Tests

- `keeper/test/ordapi.test.ts` (12): agree (the real ord + Ordinals Wallet bodies of 2026-09-23 and a swagger-shaped UniSat
  body, User-Agent on every call, the key to one host only), disagree (outpoint, offset, extra inscription, runes, known-to-one),
  one down (503, garbage, wrong id), rate limited (Retry-After park, doubling park, unparked), stale (two apart, moved mid-read,
  one apart passes), unsupported (keyless pair cannot verify contents → `VerifiedOrd` gives `null`), tx facts vs esplora,
  listings cached 45 s, the key never in a log line or an error, the hourly log, merging, "only ordapi.ts reads UNISAT_API_KEY".
  `test/send-guards.test.ts` now allows exactly `UNISAT_API_KEY` in `ordapi.ts` and `log.ts`. Suite: 67 pass.
- `autobuyer/canister-rs/src/ordapi.rs` tests (5): the three dialects parse the same frog to the same `Location`; contents from
  ord and UniSat with rune names compared without spacers; heights; the rule (agree / disagree / one down / behind / moved /
  alone); URLs, names, bases. `src/ordnet.rs` test: every update is operator-only, the key is read in one place into one
  header, no query returns it. `tests/ordapi.rs` (4): the custody read through `custody::frog_at` and `deposits::check_output`
  when providers agree, disagree, one is down or rate-limited, and across a reorg (heights apart, height moved mid-read, the
  frog elsewhere after the reorg with the old outpoint spent on both sides). Suite: 302 lib + 112 integration pass.

## 6. Open problems

1. **UniSat is unexercised.** No key was available; every UniSat shape is from the swagger. First keyed run: hit
   `probe_frog` on a known frog and compare `ord_providers()`' three answers; expect the parsers to refuse something.
2. **Contents verify only with a key.** Keyless, the canister has one per-output provider (ord) and refuses deposits,
   deliveries and fee-input signing on mainnet as unverified. That is the rule working; it is also a launch dependency on a
   UniSat key (or a self-hosted ord, which the user has decided against).
3. **Hiro's slot is empty.** Xverse (keyed) or Ordiscan (keyed / x402) would be a third independent indexer; both need a key
   and route confirmation. Maestro and OrdinalsBot do not resolve.
4. **Listings.** Magic Eden's ord book answered 503 all afternoon; Ordinals Wallet's book intermittently; UniSat's is a POST
   the keeper's GET-only gate does not make. The floor keeps working on Ordinals Wallet's book with the verified outpoint check
   and its `L-unverified` rule; a second book is unfinished.
5. **UniSat's key on the canister is visible to the subnet's replicas** (any outcall header is). Mitigated by scope (read-only
   quota) and by never persisting it; not eliminated. A non-replicated outcall would not help (every read here must be
   replicated to be agreed).
6. **Quota arithmetic.** Three calls per provider per fact is honest but expensive on a 2,000/day tier; reading the height once
   per tick instead of around every fact would cut UniSat use ~3× at the cost of a wider "same height" window.
7. **`oracle.rs`'s rune-delivery read** still reads ord's `/r/utxo` alone (its second source is an address balance, not a second
   per-output view). Moving it onto `ordnet::contents` is a one-line change once UniSat's `runes/utxo` shape is confirmed.
8. **ordinals.com's recursive API has no terms.** It exists for inscriptions' own recursion, not for indexers; if it goes the
   way of the JSON API, the keyless pair is Ordinals Wallet alone for locations and nothing for contents. `add_ord_provider`
   is the escape hatch.
