# v8 — An ordinals wave beside the five Ethereum collections (decision prep)

Written 2026-09-28 on branch `ordinals-wave` (from `nft-side` `1dc2a61`). **Docs and read-only measurement only**: no contract,
canister or service code changed; nothing deployed, pushed or signed; no key used (the operator's UniSat key was not used). Every
network call was a keyless GET or a keyless Relay `/quote` POST (read-only), or a public page read in a browser; the raw answers
were kept under `cache/` (git-ignored), not committed.

**Legend.** **[M]** measured 2026-09-28 (times below, UTC). **[R]** read from this repository at `1dc2a61`. **[A]** assumption.

## 0. Summary

- **The canister buys for exactly one Bitcoin collection.** [R] `Init.wrapper` is one address (`canister.rs:60`); a Bitcoin floor
  sink names no wrapper (`floornet.rs:32`, `FloorItem::Bitcoin`); the membership root is read from that one wrapper's listing
  (`frogflow.rs:317` `root`); `DeployStack` deploys one inscriptions collection per run (`ETCH_WRAPPER_LISTING`). The frog code is
  otherwise collection-agnostic (a collection is a Merkle root of inscription ids). So **one** ordinals collection needs no new
  canister code; **two or three** need a multi-wrapper change (canister + `DeployStack` + services) or a second buyer canister.
- **The ordinals market is thin and fragmented** [M]: 3–91 sales a week per candidate; Magic Eden's ordinals marketplace answers 404
  and its API 503; the keyless books (Ordinals Wallet, UniSat) hold a handful of listings near the floor; and **the venue carrying
  most sales fills with seller `SIGHASH_ALL` and an `OP_RETURN` output**, which the canister's buy judge refuses by design (rule 4,
  and it needs a seller-presigned `0x83` input). Only 0–51 % of last week's fills per collection (13–21 % for Puppets, Frogs and NodeMonkes) were `0x83` listing fills.
- **Recommended collections: Bitcoin Puppets and Bitcoin Frogs; NodeMonkes third** (§1.4). Quantum Cats, Pizza Ninjas, OMB and
  Runestone are not recommended now (Runestone is below the canister's 0.05 ETH per-purchase minimum and its Relay quote fails the
  3 % judge).
- **Relay's minimum-out is 2.75–3.07 % under the reference rate** [M], so the relay-float default `max_fee_bps = 300` passes a
  0.15–1.15 ETH quote by only 0.13–0.25 points and refuses a 0.05 ETH one. Recommended **400** (§2).
- **A combined stack cannot install today under the operator's A4-L-1 choice (c)**: 48 Ethereum items + the Bitcoin wrapper's 3 =
  51 > 48 that leave in two days at 24 a day. And **B-L-6** (the two sides read Relay refunds from one shared 4663 balance) is an
  open Low that must be fixed before both sides run (§3).
- **Sequencing: an ordinals second wave**, after the Ethereum launch and after the queued outflow raise (§3.6). The review scope
  (§4) is mainly code no review has read: the Bitcoin Relay conversion (`relayconv.rs`, `convflow.rs`, built 2026-09-27, after the
  2026-09-26 audit's commit `e04f49b`; the NFT rounds covered `relayeth`/`ethflow`, not these).

---

## 1. The collections, measured

### 1.1 Sources and times [M]

| Source | What | When (2026-09-28, UTC) | Answer |
|---|---|---|---|
| Ordinals Wallet `turbo.ordinalswallet.com/collection/<slug>`, `/escrows`, `/sales` | card (supply, OW "fair" price and its 7-day change), OW's own open listings with seller addresses and outpoints, the last ≤ 100 sales across marketplaces (price, block time, seller sighash, OW's marketplace code) | 09:27–09:30 | **200** for all slugs (`quantum-cats` 404; the slug is `quantum_cats`). `/sales` does not paginate (`offset`, `page`, `cursor`, `before` all return the same page), so a 7-day count is exact only when the page reaches back 7 days; otherwise it is marked **≥** |
| Magic Eden `api-mainnet.magiceden.dev/v2/ord/btc/…` | `stat` | 09:27 | `bitcoin-frogs` **200 but frozen**: floor 480,000, listed 1,010, owners 5,143 — byte-for-byte the 2026-09-23 answer (`ordinals-apis.md` §1), while frogs sold at 280–340k this week. Every other symbol and every other route (`tokens`, `collections`, `activities`, `popular_collections`) **503 "no healthy upstream"**; `api-mainnet.magiceden.io` is a Cloudflare challenge; `stats-mainnet.magiceden.io` answers "Only solana is supported"; **`magiceden.io/ordinals/marketplace/nodemonkes` and `magiceden.us/ordinals` redirect to 404** (browser, 09:34). Treated as **not a source** |
| UniSat marketplace (public pages `unisat.io/market/collection?collectionId=…`, rendered in a browser) | floor, listed/supply, the cheapest page of listings | 09:36–09:40 | frogs, nodemonkes, runestone, omb, pizza-ninjas answered; `bitcoin-puppets`, `quantum_cats` fell back to the rankings page (no such collection id). Its JSON API (`unisat.io/api/market-v4/…`) answers **403 "no sign"** keyless; `open-api.unisat.io` **403 `-2004`** (keyed). Sellers are not shown |
| CoinGecko `api.coingecko.com/api/v3/nfts/<id>` | floor in BTC, 7/30-day floor change | 09:31–09:34 | **200** (after one 429 back-off). Its floors disagree with the books (below) and its source is not stated; used for the 30-day change only |
| ord `ordinals.com/r/utxo/<outpoint>` | the cheapest OW listings' outputs: inscriptions, runes, value | 09:41 | **200**; every one carries exactly one inscription and no runes |
| mempool.space `/api/tx/<txid>`, `/api/v1/fees/recommended`, `/api/v1/mining/blocks/fee-rates/{1m,1y}` | sample sales' shapes; fees | 09:33–09:44 | 200 |
| Relay `POST api.relay.link/quote` (4663 → Bitcoin, the 09-26 fixture's user and recipient), `/chains` | min-out vs the reference | 09:42 | 200; 4663 and Bitcoin both enabled |
| Coinbase `api.coinbase.com/v2/exchange-rates?currency=ETH` | the rate | 09:31 | **0.031994 BTC per ETH** (3,199,400 sats/ETH), ETH $2,646 |
| Not reached keyless | OKX (`/api/v5/mktplace/nft/ordinals/*` needs `OK-ACCESS-KEY`), Best in Slot (keyed; 301/404), Ordiscan (402 x402), Satflow (403), Gamma (503), Magisat (404) | 09:35 | — |

### 1.2 The table [M]

"Book floor" is the cheapest listing on the two keyless books (Ordinals Wallet + UniSat). "Within +x %" counts listings at or under
book floor × (1 + x), with the distinct Ordinals Wallet sellers among them (UniSat does not show sellers). "`0x83` fills" are last
week's sales whose seller signed `SIGHASH_SINGLE|ANYONECANPAY` — the listing shape the canister can buy (§1.3). ETH at 0.031994.

| Collection (OW slug) | Book floor, BTC / ETH | +2 % | +5 % | +10 % | Listed (OW / UniSat); OW sellers | 7-day sales, median | `0x83` fills / 7 d | 30-day floor Δ (CoinGecko) | CoinGecko floor |
|---|---|---|---|---|---|---|---|---|---|
| **Bitcoin Frogs** (`bitcoin-frogs`) | 0.00411 / **0.128** | 1 (1) | 3 (1) | 4 (2) | 81 / 83; **60** | 29, 0.00340 | 5 | +17.3 % (OW fair: −12.0 % this week) | 0.00400 |
| **NodeMonkes** (`nodemonkes`) | 0.03274 / **1.023** | 2 (2) | 2 (2) | 4 (3 +1 UniSat) | 7 / 13; 7 | 28, 0.02965 | 6 | −1.8 % | 0.02318 |
| **Bitcoin Puppets** (`bitcoin-puppets`) | 0.01747 / **0.546** | 1 (1) | 1 (1) | 2 (2) | 15 / —; 14 | **91**, 0.01731 | 12 | −0.6 % | 0.01024 |
| Quantum Cats (`quantum_cats`) | 0.01901 / 0.594 | 1 (1) | 1 (1) | 1 (1) | 1 / —; **1** | 3, 0.00741 | 1 | **−22.0 %** | 0.01045 |
| Runestone (`runestone`) | 0.00100 / **0.031** | 2 (0 +2) | 15 (3 +12) | 25 (5 +20) | 175 / 380; 124 | **≥ 100** (page spans 3.9 d), 0.00101 | 51 of 100 | +28.6 % | 0.00139 |
| Ordinal Maxi Biz (`omb`) | 0.01850 / 0.578 | 1 (1) | 1 (1) | 1 (1) | 11 / 3; 9 | 35, 0.01750 | 14 | **+45.9 %** | 0.01750 |
| Pizza Ninjas (`pizza-ninjas`) | 0.00781 / 0.244 | 1 (1) | 1 (1) | 1 (1) | 1 / 2; **1** | **5**, 0.00420 | **0** | −17.7 % | 0.00872 |
| *also read:* Bitcoin Punks | 0.00421 / 0.132 | 1 | 1 | 1 | 89 / 91; 66 | 44, 0.00385 | 7 | +8.6 % | 0.00314 |
| Natcats (`dmtnatcats`) | 0.00399 / 0.125 | 1 | 1 | 1 | 33 / —; 23 | 8, 0.00265 | 2 | −9.8 % | 0.00284 |
| Bitcoin Wizards | 0.05550 / 1.735 | 1 | 1 | 1 | 11 / —; 10 | 2, 0.02229 | 0 | −10.0 % | 0.01350 |
| The Wizards of Ord (`wizards`) | 0.01542 / 0.482 | 1 | 1 | 1 | 3 / —; 3 | 7, 0.00580 | 2 | +35.0 % | 0.00540 |
| Taproot Wizards | no open listing on OW or UniSat | — | — | — | 0 / 0 | 6, 0.03335 | 1 | +4.5 % | 0.04700 |

Notes on the numbers:

- **The books sit above the trades.** Frogs trade at 280–340k sats while the cheapest listing on either keyless book is 411k and
  UniSat's floor is 550k; Quantum Cats traded at 0.0074 against a single 0.019 listing. The sales happen on venues whose books are not
  readable keyless (OW's marketplace code 2, below). CoinGecko's floors differ from both (Puppets 0.01024 vs 0.01747 on the books).
- **Supplies disagree by source**: OMB 9,001 (OW) / 5,243 (CoinGecko) / 2,091 (UniSat) — OMB has sub-collections, so its membership
  list is a choice; Quantum Cats 3,313 (OW) / 3,333 (CoinGecko). The membership root must name one list, checked against two sources.
- **UniSat is not where volume is**: its 30-day ordinals volume leader is Runestone at 0.091 BTC (rankings page, 09:38).
- **Bitcoin fees** [M]: 1 sat/vB now; block-median fee rate over the last year: median 1, p99 4, max 6 sat/vB (mempool.space).

### 1.3 Listing compatibility with the canister's buy path [R, M]

The canister buys only from a PSBT the operator hands in (`frog_offer`; the runner's `npm run offer --psbt | --unisat-bid`), judged
by `frogbuy::judge`: one seller input with a final key-path signature; **no `OP_RETURN` output at all** (rule 4: a runestone could move
runes onto custody's output); exactly one output to custody; the inscribed sat lands on it and only the seller's or the float's sats
overlap it; spend ≤ the arrived sats; miner fee ≤ min(3 % of spend, 24,500 sats). The seller's output must carry exactly the one
inscription and no runes (`frogflow.rs:985`). A usable listing is therefore a seller input pre-signed `SIGHASH_SINGLE|ANYONECANPAY`
(`0x83`), completed by us. Sample transactions, one per OW marketplace code [M, mempool.space]:

| OW code | Seller sighash seen | Shape | Fee output | Canister |
|---|---|---|---|---|
| **2** (most sales: 47–93 % per collection) | `0x01` ALL, `0x81` ALL\|ACP | 546-sat inscription output, **an `OP_RETURN`**, fee 2.5 % to `bc1qt40u…fl0w`, or a bare 2-in/2-out swap | 2.5 % | **Refused** (OP_RETURN; and a seller signing ALL signs after the buyer's outputs exist, which the hand-in model cannot use) |
| 8, 0, 1, 9, 10 | `0x83` | the classic padded listing: 2 × 600 padding in, seller, payment; out 1,200 padding, the inscription, seller payment, fees, change | 0–2.5 %, several paid from the seller's payout | **Compatible in shape** (the judge's own test fixture, `frogbuy.rs:480`) |
| `0x81` fills on other codes | `0x81` | an offer (bid) accepted by the seller | — | not reachable: the canister makes no bids |

- **Ordinals Wallet escrows**: the cheapest ≤ 8 listings of each candidate (ord `/r/utxo`) each hold exactly one inscription and no
  runes, postage 330–10,000 sats. Runestone listings checked were rune-free too.
- **Padding (new finding)**: the padded shape needs **two float inputs before the seller's** (input index = the seller's payout index
  under `0x83`). After a Relay conversion the float holds one UTXO, and the canister has no split path. The first buy therefore needs a
  second float UTXO: a one-time seed of ≈ 1,200 sats to the float (a decision, §5 R10). Later buys reuse the padding output and the
  change. To be confirmed on the first real venue PSBT.
- **UniSat's `create_bid` shape** is from its swagger and has never been exercised (`ordinals-apis.md` §6.1); the operator's keyed run
  is a go-live check.

### 1.4 The item budget on the BTC path [R, M]

As `phase1-nft-pooled.md` §1.1, adapted to the Bitcoin module (conv 300 = 3 %, reserve 2,500 = 25 %, the frog stack's defaults):

1. Item budget B = 4.12 ETH (5 ETH raise) or 6.742 (8 ETH early raise); convertible units c = 0.75 × B ÷ (1.03 F); a buy is due at
   c ≥ 1.05, so one graduation buys alone while **F ≤ 2.857 ETH** (normal) or **4.676** (early). Every candidate is far under it.
2. The advance = `maxItemCost()` = 1.03 F (the whole item cap: the BTC side has no A2-L-1 bound), within the collection's purchase
   limit — **the global `policy::MAX_PURCHASE_WEI` is 0.5 ETH** unless `set_purchase_limit` (controller-only, ≤ 25 ETH) names more —
   and at least **`MIN_PURCHASE_WEI` 0.05 ETH** (`frogbuy::check_limits`).
3. Relay: 0.0002 ETH kept for gas; measured cost 0.77–1.09 % expected, **min-out 2.75–3.07 % under the reference** (below); judged
   at `max_fee_bps`.
4. The buy: listing + any buyer-side venue fee + miner fee (≈ 1,000 sats at today's 1–2 sat/vB for ~500 vB; ≤ 24,500 sats and ≤ 3 %
   of spend) ≤ the arrived sats.

| Collection | F (book) ETH | Advance ETH | Items / graduation 5 / 8 ETH | Listing it can buy (vs book floor): no buyer fee / 2.5 % fee | Stage-0 custody (3 items) | Purchase limit |
|---|---|---|---|---|---|---|
| Bitcoin Frogs | 0.128 | 0.132 | 23 / 38 | **+1.8 % / −0.7 %** | 0.40 ETH | default 0.5 fits |
| Bitcoin Puppets | 0.546 | 0.563 | 5.5 / 9.0 | +2.1 % / −0.4 % | 1.69 ETH | **needs `set_purchase_limit` ≥ 0.7 ETH** |
| NodeMonkes | 1.023 | 1.054 | 2.9 / 4.8 | +2.1 % / −0.4 % | 3.16 ETH | **needs ≥ 1.3 ETH** |
| OMB | 0.578 | 0.596 | 5.2 / 8.5 | +2.1 % / −0.4 % | 1.79 ETH | needs ≥ 0.75 ETH |
| Quantum Cats | 0.594 | 0.612 | 5.0 / 8.3 | +2.1 % / −0.4 % | 1.84 ETH | needs ≥ 0.75 ETH |
| Pizza Ninjas | 0.244 | 0.252 | 12 / 20 | +2.0 % / −0.5 % | 0.75 ETH | fits |
| Runestone | 0.031 | **0.032** | 96 / 157 | +0.5 % / −1.9 % | 0.10 ETH | **under the 0.05 ETH minimum: cannot buy one** (a code constant) |

**No candidate's floor is too high for one graduation**: the smallest graduation buys ≥ 2.9 NodeMonkes. The binding limits are the
others: **the 0.5 ETH default purchase limit** (Puppets, NodeMonkes, OMB, QC need `set_purchase_limit` — controller-only, so before
handover or through a reviewed configuration path, §3.5); **the 0.05 ETH minimum and the Relay judge** (Runestone); and **the margin
per buy is about +2 % over the posted floor with no buyer-side venue fee, and negative with a 2.5 % one** — so buys land only on
listings at or just above the floor the module accepted, and on seller-paid-fee venues.

**The posted floor is the keeper's, from the OW book**: the lower median of the lowest 5, one per seller cluster, with ≥ 3 clusters
(`keeper/src/floor.ts`), sales pulling it down at most 5 %. From today's books [M]: Frogs ≈ 565k sats (0.177 ETH, **+37 %** over the
cheapest listing and +66 % over the week's median sale), Puppets 2.05M (0.639, +17 %), NodeMonkes 3.60M (1.124, +10 %), OMB 2.96M
(0.925, **+60 %**); **Quantum Cats and Pizza Ninjas have one OW seller each: no floor is ever posted**, so their module can neither
mint, burn nor graduate. Where the posted floor is well above the cheapest listing, each buy leaves large **spare sats** in the float
(relay-float §2.4 expected 1–2 %; Frogs would leave ≈ 25 %), which is decision R1 (§2).

### 1.5 Recommendation

1. **Bitcoin Puppets — first.** The most traded (91 sales in 7 days, 12 of them `0x83` listing fills), a flat floor (−0.6 % over 30
   days), 14 OW sellers so the keeper posts, and its book floor sits on its sales median (0.01747 vs 0.01731: small spare). Needs a
   purchase limit of ≈ 0.7 ETH. Weak point: depth at the floor is 1–2 listings on the keyless books.
2. **Bitcoin Frogs — second** (or first if the operator wants zero configuration beyond the rehearsed path). The rehearsed stack
   (`golive-dryrun.sh stack` is frogs), the deepest seller base (60 OW sellers), fits the default 0.5 ETH limit, 29 sales a week. Weak
   points: the books sit 20–60 % above where frogs actually trade (OW's fair price −12 % this week), so graduations price units above
   market and buys leave large spare sats; stage 0's 3 items are one graduation's 5 %.
3. **NodeMonkes — third.** Stable (−1.8 % / 30 d), 28 sales a week, but 7 OW sellers and ~1.02 ETH an item (limit ≈ 1.3 ETH; 3 items =
   3.2 ETH of custody).

**Not now:** Quantum Cats (one listing, 3 sales/week, −22 % / 30 d, no posted floor); Pizza Ninjas (5 sales/week, zero `0x83` fills, one
OW seller, −17.7 %); OMB (volatile +46 % / 30 d, a posted floor 60 % over the book, and an ambiguous membership list); Runestone (below
the 0.05 ETH minimum; its Relay quote at that size fails the 3 % judge). Bitcoin Punks (44 sales/week, 66 sellers, 0.13 ETH) is the
best alternate to Frogs.

**Two or three at once need code** (§0): today's canister holds one Bitcoin wrapper. The no-code way to run two is a second buyer
canister (its own install, timelock, cycles, monitor and ceremony) — heavier than the multi-wrapper change.

---

## 2. The relay-float deferred decisions (`docs/v6/relay-conversion.md` §9 and the open points of §2.4, §5)

| # | Decision | Options | Recommended default | Why (measured where marked) |
|---|---|---|---|---|
| R1 | **Spare sats** (arrived − spent stay in the float) | leave; sweep by a future reviewed path; let the next buy of that collection use them | **Leave, report on the reserves page, page when the float's spare exceeds one item's advance**; choose collections whose posted floor sits near the book (Puppets) | With the keeper's OW-book floor, spare is 2–25 % per buy, not 1–2 % [M]. A sweep is new outbound signing: not at stage 0 |
| R2 | **Stuck deposits** (no fill, no refund past deadline + 1 h) | Relay support; a reviewed endpoint that signs only Relay's withdrawal with recipient = `evm_address` | **Relay support** at stage 0; SEV1 page as built | Exposure is one advance (≤ ≈ 1.1 ETH for NodeMonkes). A new signing endpoint needs its own review |
| R3 | **The allocator pin** | keep (a rotation stops conversions until a 72 h upgrade); pin the depository only | **Keep**, and re-read both at go-live (phase1-scope step 11) | Unchanged since 09-26: `/chains` [M] still lists 4663 and Bitcoin enabled |
| R4 | **`max_fee_bps`** (min-out ≥ reference × (1 − this)) | 200 / 300 / 400 / 500 (bounds [50, 500]) | **400** (`frog_funding = { max_fee_bps = 400; float_first = false }`) | Relay's min-out is **−3.07 % at 0.05 ETH, −2.87 % at 0.15, −2.76 % at 0.6, −2.75 % at 1.15** vs Coinbase [M 09:42]; expected −0.77 to −1.09 %. At 300 the judge passes by ≤ 0.25 points and refuses small asks; the doc's "lower to 2 %" would refuse every quote. The canister's own rate differs from Coinbase's, so 300 can flap |
| R5 | **Gas on 4663** | fund once; per-advance keep | **Already funded for the Ethereum launch** (the same `evm_address`); keep `GAS_KEEP_WEI` 0.0002 | — |
| R6 | **Retries** (`MAX_ATTEMPTS`) | 3 then `returnAdvance`; more | **3** | A refund costs the round trip (≈ 1 %); the ETH side waits a day after a return (A3-L-1), the BTC side does not: note for review |
| R7 | **BTC arrives, no item buyable** (§5.4: floor moved above the arrived sats, or no compatible listing) | wait; a reviewed BTC → ETH way back | **Wait; SEV2 at 24 h (built) and SEV1 at 7 days (new monitor rule)**; no reverse leg at stage 0 | Books are 1–2 listings deep near the floor [M]: waiting days is plausible. Exposure ≤ one advance; the ETH side has a 48 h automatic return, the BTC side has none |
| R8 | **BTC fee spikes** | keep the bounds; raise `MAX_BUY_FEE_SATS` | **Keep**: buys wait while min(3 % of spend, 24,500 sats) is short; deliveries keep their 25,000-sat ceiling and the fee pool | Block-median fee over a year: max 6 sat/vB [M]; a Frog's 3 % ≈ 12,300 sats ≈ 25 sat/vB at ~500 vB |
| R9 | **Refund vs gift ambiguity** (§5.1) and **the shared balance with the ETH side** (B-L-6) | accept; one Relay leg canister-wide | **One Relay leg canister-wide across both sides (code)** | Accepting was fine with one side; with both it is B-L-6 (§3.3) |
| R10 | **Float padding** (new, §1.3) | seed the float once with ≈ 1,200 sats; add a canister split path | **Seed once**, recorded as a known inbound for the canary, before the first conversion | Keeps "no operator float" in substance (≈ $1); no new signing path |
| R11 | **Purchase limits** (new) | `set_purchase_limit(module, …)` before handover; carry it in the upgrade argument | Puppets **0.7 ETH**, NodeMonkes **1.3 ETH**, Frogs default | 0.5 ETH global default [R] is below three candidates' advances |
| R12 | **Refunds of a returned advance** (§2.2: shortfall from revenue, then surplus, else a shared loss) | as built | **As built** | Bounded by one advance |

---

## 3. A combined stack: what it takes

### 3.1 Deploy (`DeployStack`) [R]

- **Mixed mode exists**: the default kind (`btc`) deploys the one inscriptions collection (`ETCH_WRAPPER_*`) **and** any
  `ETCH_NFT_COLLECTIONS` in the same run. The Ethereum launch is planned as `ETCH_STACK_KIND=nft` (no Bitcoin `WrapClaim`/`Wrapper`/
  OfferBoard).
- A **second wave** is a second `deploy-stack.sh` run (kind `btc`, no NFT variables) against the same registry: a second stack file.
  Everything that reads `deployments/stack-4663.json` (site `npm run sync`, keeper, monitor, reconciler, indexer, golive-check)
  reads one file today: a merge or a list of stack files is a work item.
- Per collection: the membership Merkle root of every inscription id (from two sources; OMB's list is a choice), `list-auto.sh`
  (`listInscriptionsAuto`, btcVault = `custody_address`), the Safe's owner steps (`initMintLimit(3)`, `wireFloorModule`,
  `addCollection`, `directory.add`, `proposeCapCeiling`).
- **Not rehearsed**: a mixed or two-file stack. `golive-dryrun.sh` has a frogs stack and a five-collection NFT stack, separately.

### 3.2 golive-check [R]

It loops per collection by `kind` (`inscriptions` checks listing `btcVault` = custody, `WrapClaim` wired; `nft` checks poster/
custodian), so mixed is plausible, but: the launch file (`deployments/launch/nft-pilot.json`) carries Ethereum collections and the
outflow only; the A4-L-1 check must count the `btc:0x<wrapper>` cap; `--canister` must see `wrapper` set and a Bitcoin floor sink;
and the custody fee pool (≥ 20,000 sats) and UniSat checks apply again. Add a mixed phase to the dry run.

### 3.3 The canister: caps, outflow, the shared Relay leg [R]

- **Custody caps (H2)**: the Bitcoin wrapper keeps stage 0's **3 items / 10 ETH** beside J7's Ethereum caps.
- **The 72 h exit rule (A4-L-1)** counts every collection's items against `global_items` over two usable days. The operator's
  choice (c) installs at **48 Ethereum items at 24 a day**, with the outflow raise queued at launch. **48 + 3 = 51 > 48**: the install
  is refused. Either (i) shave 3 Ethereum items (e.g. Azuki 17 → 14), or (ii) add the Bitcoin collection **after** the queued raise has
  executed (≥ 26 a day; the ×4 rule allows up to 96). (ii) fits the second-wave sequencing with no change to the Ethereum caps.
- **Per-collection outflow**: 12 deliveries a day against 3 Bitcoin items — ordinals delivery rate is never the binding term; a
  Bitcoin delivery is 1 conf to broadcast + TRUC, well inside a day. The **shared** limits are: 6 mints a day in all (3 per collection),
  25 ETH of value a day (a BTC advance counts at the advance), the tripwire at 3×, and **one 4663 transaction canister-wide** (the frog
  tick runs before the Ethereum one).
- **One Relay leg across both sides (B-L-6, open Low)**: both sides detect a Relay refund as a rise of the same 4663 balance at
  `evm_address`; the ETH side's lock (`EthState::relay_busy`) sees only Ethereum buys and the frog side's only its own `convs`. With both
  running, a frog advance landing during an Ethereum `Bridging` reads as its refund and is re-bridged, and vice versa. **Fix before
  both sides run**: a canister-wide "one Relay leg" across `FrogState.convs` and `EthState.buys` (simplest), or per-side attribution by
  the order id. Cost of the simple fix: a BTC conversion holds the leg ≈ 4 min (Relay's `timeEstimate` 241 s [M]) plus three Bitcoin
  blocks for the arrival (≈ 30 min); an Ethereum one ≈ seconds. At 3 items a day per collection that is no bottleneck.
- **Configuration after handover**: `set_floor_sink`, `set_item_config` and `set_purchase_limit` are **controller-only**; after
  handover the timelock can only Upgrade / Controllers / Roles. Adding a Bitcoin collection post-launch needs either the S-size change
  already recommended in `phase1-scope.md` §4 (the upgrade argument carries floor sinks, item config and purchase limits — reviewed
  once, timelocked like any argument) or a `propose_controllers` round trip (two 72 h delays and a controller key back in play). The
  `wrapper` itself can go from none to set by upgrade [R: `canister.rs` test "the wrapper: set once from nothing"].
- **UniSat key**: contents (deposits, deliveries, buys' fee inputs) verify only with two per-output providers; keyless there is one
  (ord). `set_unisat_key` after install and after **every** upgrade; its shapes are unexercised (`ordinals-apis.md` §6.1) — a keyed
  `probe_frog` on a known item is a go-live check. Its header is visible to the subnet's replicas: a read-only plan.

### 3.4 Services [R]

- **Runner**: `RUNNER_RELAY` (frog conversions) and `RUNNER_ETH_NFT` both on, one tick. **Bitcoin buys are manual**: the operator
  fetches a venue PSBT (OW / UniSat) and runs `npm run offer`; there is no keyless listing feed for the canister's venues (Magic Eden
  is gone, UniSat's API is signed or keyed). Budget operator time per buy.
- **Keeper**: posts the Bitcoin module's floor from the OW book (`KEEPER_OW_SLUGS`); the canister posts the Ethereum ones. Needs ≥ 3
  OW seller clusters (Puppets 14, Frogs 60, NodeMonkes 7 [M]).
- **Monitor**: the conversion alerts and the three `Advanced`/`AdvanceSettled`/`AdvanceReturned` matches exist for both sides, but
  must attribute each event to the right side's record; the Bitcoin canary on custody, float and buyer addresses; `frog_funding`
  decoded and diffed; the R7 7-day page is new. **Reconciler**: the Bitcoin module's transit unit and custody next to the NFT ones.
- **Ops**: custody fee pool ≥ 20,000 sats + `add_custody_fee_input`; the float padding seed (R10); the TRUC signet delivery
  rehearsal and both explorers accepting version-3 transactions (phase1-scope step 11).

### 3.5 Same-day vs a second wave

| | Same-day (one mixed stack, before handover) | **Ordinals second wave (recommended)** |
|---|---|---|
| A4-L-1 | shave 3 Ethereum items now | none, once the queued raise has executed |
| Controller-only setters | set before handover (no new code) | the S change (upgrade argument carries sinks/limits), reviewed with the wave |
| B-L-6 | must be fixed and reviewed **before** the Ethereum launch | fixed and reviewed in the wave |
| Review | reopens a four-round-reviewed launch build: new buyer.wasm hash, new dry run, the Ethereum go/no-go repeated | a separate round on the BTC paths only; the Ethereum build ships as reviewed |
| Market risk | thin, fragmented books go live on day one | a week or two of watching the books and the keeper's floor on the live OW feed first |
| Cost | delays the Ethereum launch by a review round | one timelocked upgrade (72 h) plus a second stack deploy and a stack-file merge |

**Superseded 2026-10-09: the operator decided ordinals launch WITH the Ethereum collections (same-day, one mixed stack), Bitcoin
Frogs as the one Bitcoin collection, Azuki 17 → 14 at steps 1–2 for A4-L-1, step 3's global 62: `deployments/launch/nft-pilot.json`
`bitcoin` / `_bitcoin`, `docs/v7/golive-runbook.md` §9.** The original recommendation, kept for the record:

**Recommendation: launch Ethereum as reviewed; ordinals as a second wave** — Puppets (or Frogs) alone first, as soon as (1) the
queued outflow raise has executed, (2) B-L-6 and the post-handover configuration path are built, (3) the review of §4 is done and
fixed, (4) the mixed/two-file dry run passes, (5) UniSat's keyed shapes are confirmed, and (6) one real venue PSBT has been judged in
a rehearsal. A second ordinals collection follows the multi-wrapper change (its own review), not before.

---

## 4. Review scope: what no earlier review covered

The 2026-09-26 audit read `e04f49b` (2026-09-25), before the Bitcoin Relay conversion existed (`convflow.rs` is absent there;
`relayconv.rs`/`convflow.rs` landed 2026-09-27 in `76d6c80`, `1c5b1ce`). Its rounds 2–4 checked fix diffs up to `cf6ecda` and never
name them. The 2026-09-27 NFT rounds 1–4 read `relayeth`, `ethnft`, `ethflow`, `ethnet`, `EthNftWrapper`, `PosterSource`, the changed
`FloorModule` and the NFT deploy — and reported B-L-6 as "not reachable in the NFT-only stack". So:

1. **The Bitcoin Relay judge** `relayconv.rs` (848 lines): the EIP-712 order hash, Relay's Bitcoin address encoding (including its
   non-segwit native-BTC id), rules 1–12, min-out against the canister's rate, freshness and replay. [must]
2. **The conversion flow** `convflow.rs` (389 lines) and its hooks in `frogflow.rs` / `floorsink.rs` / `frognet.rs` / `canister.rs`
   (`frog_funding`): arrival detection on the float's UTXOs (three deep, every explorer, `float_before`), refund detection from the
   balance, `Stuck`, `relay_give_back` from `Funded`/`Stuck`, H2/H3 counted at the advance, the "never released once advanced" rule,
   the cost share `⌈advanced × spend / arrived⌉` and spare sats. The ETH side's later lessons (a one-day wait after a return, the
   C4-M-1 write-off rules, pin floors) should be checked for their BTC counterparts. [must]
3. **Both sides in one canister**: the B-L-6 fix; one 4663 transaction canister-wide and starvation between the frog and Ethereum
   ticks; shared H3 counters and the tripwire; `validate_custody`'s A4-L-1 sum including `btc:` keys; upgrade-state decoding with both
   `FrogState.convs` and `EthState` populated. [must]
4. **The post-handover configuration path** (upgrade argument carrying floor sinks, item config, purchase limits): its validation, and
   the monitor's argument diff for it. [must if second wave]
5. **Deploy and checks**: a second btc-kind stack against the live registry (or a mixed stack), `StackChecks`, golive-check with both
   kinds and the Bitcoin cap in A4-L-1, the stack-file merge, the dry run. [must]
6. **Services on the BTC path**: `ops/src/relay.ts` (206 lines, the runner's pre-judge) and the offer tool with real venue PSBTs; the
   monitor's side attribution of `Advanced`/`AdvanceSettled`/`AdvanceReturned`; the keeper's OW-book floor on these thin books (≥ 3
   seller clusters, single-venue sales pull, floors 17–60 % over the cheapest listing). [must]
7. **Venue compatibility**: rule 4 (no `OP_RETURN`) and padding against real OW/UniSat PSBTs; UniSat's `create_bid` and keyed
   indexer shapes. [must, mostly a rehearsal]
8. **Multi-wrapper Bitcoin** (only for a second or third ordinals collection): `Init.wrapper` → a list; `FloorItem::Bitcoin
   { wrapper }`; the membership root, ticket scanning (`wrapper_from_block`), deliveries, deposits and custody keys per wrapper;
   `DeployStack` with several inscriptions collections. New code: a full review of its own. [later]

**Size**: items 1–3 are ≈ 1.3k lines of unreviewed Rust plus the B-L-6 change; 4–6 are S–M changes and ≈ 0.5k lines of TypeScript;
the `FloorModule` advance path (456 changed lines since `e04f49b`) was already read by NFT round A and needs only its Bitcoin-caller
assumptions re-checked. **About one review round of four areas** (judge, flow, cross-side, deploy/services), the size of one
2026-09-27 round; item 8 later is another.

---

## 5. Decisions for the operator

1. **Sequencing**: ordinals as a **second wave** after the Ethereum launch and the executed outflow raise (recommended), or same-day
   (shave 3 Ethereum items, fix and review B-L-6 and the BTC Relay code before the Ethereum launch).
2. **The first ordinals collection**: **Bitcoin Puppets** (recommended; purchase limit 0.7 ETH) or **Bitcoin Frogs** (the rehearsed
   path; default limit). Second: the other one. Third: **NodeMonkes** (limit 1.3 ETH). Not now: Quantum Cats, Pizza Ninjas, OMB,
   Runestone.
3. **More than one ordinals collection**: build multi-wrapper support (recommended, its own review) or run a second buyer canister.
4. **The post-handover configuration path**: the upgrade argument carries floor sinks, item config and purchase limits (recommended,
   S) or a `propose_controllers` round trip.
5. **A4-L-1 with a Bitcoin collection**: add it after the raise executes (recommended) or shave 3 Ethereum items now.
6. **B-L-6**: one Relay leg canister-wide across both sides (recommended) or per-order attribution.
7. **R4 `max_fee_bps`: 400** (recommended; 300 is 0.13–0.25 points from refusing Relay's measured quotes).
8. **R1 spare sats**: leave and report, page above one advance (recommended).
9. **R2 stuck deposits**: Relay support at stage 0 (recommended).
10. **R3 allocator pin**: keep (recommended).
11. **R7 arrived-but-unbought BTC**: wait, SEV2 at 24 h, new SEV1 at 7 days; no reverse leg at stage 0 (recommended).
12. **R8 BTC fee bounds**: keep (recommended).
13. **R10 float padding**: seed ≈ 1,200 sats once before the first conversion (recommended).
14. **R11 purchase limits**: 0.7 ETH Puppets, 1.3 ETH NodeMonkes, default for Frogs.
15. **Runestone**: stays out unless the 0.05 ETH minimum and the Relay judge's small-ask behaviour are changed (a code change; not
    recommended).
16. **Membership lists**: which list defines each collection (OMB's especially), taken from two sources before `listInscriptionsAuto`.

## 6. Decided (the operator, 2026-09-28)

The operator accepted every recommended default above: **ordinals as a second wave** after the Ethereum launch and the executed
outflow raise; **Bitcoin Puppets first** (purchase limit 0.7 ETH), then Bitcoin Frogs, then NodeMonkes (1.3 ETH); **multi-wrapper
support** for more than one ordinals collection (its own review); the **upgrade-argument configuration path**; the Bitcoin
collection **added after the raise executes**; **B-L-6 as one Relay leg canister-wide**; R4 `max_fee_bps` **400**; R1, R2, R3, R7,
R8, R10 and R11 as recommended; Runestone out; membership lists from two sources. The Bitcoin-path review (§4, items 1–7) runs after
the Ethereum side's audit comes back clean.
