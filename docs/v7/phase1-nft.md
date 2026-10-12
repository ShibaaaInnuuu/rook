# v7 Phase 1: Ethereum NFTs, bought and held by the canister (design)

Status: **design, written before its code**, 2026-09-27, branch `nft-side` (from `main` with `relay-float` merged). Nothing is
deployed, no key was used, nothing was sent to any chain. The only calls made were read-only: OpenSea's keyless
`collections/{slug}` and `collections/{slug}/stats`, CoinGecko's keyless `nfts/{id}`, Relay's `/chains` and `/quote`, and
Ethereum and Robinhood Chain JSON-RPC (`eth.drpc.org`, `1rpc.io/eth`, `mainnet.gateway.tenderly.co` for 7-day log ranges,
`rpc.mainnet.chain.robinhood.com`).

**Legend.** **[M]** measured on 2026-09-27 at Ethereum block ≈ 26,068,928 (12:55 UTC). **[R]** read from this repository.
**[A]** assumption or estimate.

**The operator's decision.** Phase 1 backs Robinhood Chain (4663) coins with **Ethereum ERC-721s** (Milady, Pudgy Penguins,
BAYC, Azuki and the like, configured per collection), using Rook's frog model, not NFTX tokens (NFTX was measured unusable:
`docs/v7/phase1-nftx.md` on `nftx-phase1`). Bitcoin frogs move to Phase 2; their code stays and stays tested.

**Decided 2026-09-27** (§9): every recommendation D1–D9 was accepted, with two changes by the operator — **D1: several
collections at launch** (Milady, Azuki and Mutant Ape Yacht Club at once; Pudgy Penguins and BAYC to follow through per-collection
graduation targets and pooled graduations on branch `nft-pool`), and **D5: holder deposits ON at launch** (§2.4, §11.5).

**Dormant collections (2026-09-29).** The allowlist grew to ten collections, each DORMANT (no scan, no cycles) until a bonded wake
request, a deposit or a queued graduation wakes it: `docs/v7/dormant-collections.md` (design and as built) and its review
`docs/audit/2026-09-27-nft/dormant-review.md`. Cycles now scale with ACTIVE collections, not listed ones, and how many may be awake
follows the canister's own cycles balance under a ceiling of 10 (§11 there; runbook §6).

---

## 0. Summary

**The model, in one line per step:**

1. A curve graduates; its item budget becomes ETH-backed wX at the collection's `FloorModule` (unchanged, [R] `FrogCurve.sol:285-292`).
2. The module's net demand reaches one item plus the buffer. The canister asks the module to **advance** one item's budget
   (`FloorModule.advance`, from `relay-float`, unchanged).
3. The canister moves that ETH **4663 → Ethereum mainnet, to its own Ethereum address, through Relay**. It judges the quote by
   the order hash, exactly as `relay-float` does, with the output chain `ethereum` and the recipient our own address.
4. The runner hands in a **Seaport listing** of the collection. The canister judges it (collection allow-listed, price at most
   its own floor reading plus a premium and at most what arrived, delivered to us, fees bounded, still valid at the pin,
   simulated), then signs and sends `fulfillBasicOrder` / `fulfillAdvancedOrder` **on Ethereum** from its threshold-ECDSA address.
5. The NFT stays **at the canister's Ethereum address** (no L2→L1 messaging, no operator key). Once the purchase is past
   Ethereum finality and `ownerOf` says us on both providers, the canister mints one unit to the module on the new
   `EthNftWrapper` (4663) and reports `settleAdvance(cost)`. The unit is now item-backed.
6. **Redemption:** a holder burns 1 wX for a **drand-random** ticket (the audited `Wrapper` ticket machine, re-keyed to token
   ids). The canister transfers that NFT on Ethereum to the destination fixed at the burn, and records the delivery once it is
   final. A delivery that can no longer confirm is `markUndeliverable`d; the refund is gated exactly as the Bitcoin wrapper's.
7. **Floor:** the canister is the module's floor poster. It posts from **verified live Seaport listings** (signature, owner,
   order status, approval and time checked at an Ethereum pin on two providers), **bounded by verified recent on-chain sales**
   (Seaport and Blur fills it parses from receipts). The keeper EOA has no floor role for these modules. The on-chain move
   limits (±5 %/h, ±20 %/day) are unchanged.
8. **Holder deposits** (a holder sends an allow-listed NFT to the canister's address with a claim; the canister verifies it
   past finality and mints a unit): **on at launch** (D5, decided 2026-09-27). The runner discovers every NFT sent to the canister;
   one without a valid claim goes back to its sender (§2.4).

**What is new, and what is reused:**

| Piece | Status |
| --- | --- |
| `FloorModule` (advance / settle / return, mint/burn, graduation lane, pause, bounds) | **unchanged** |
| `FrogCurve`, `CurveFactory`, `LiquidityLock`, `EtchRouter`, `ProtocolTreasury`, `ListingRegistry` | **unchanged** |
| `src/nft/EthNftWrapper.sol` | **new**: `Wrapper`'s audited ticket machine (rounds 1–4) keyed by ERC-721 token id; no rentals |
| `src/floor/PosterSource.sol` | **new** (≈ 60 lines): the module's `IKeeperSource` for an NFT collection — `keeper()` is the canister, zero while the registry pauses the keeper |
| canister `relayeth.rs` | **new**: the Relay judge of `relay-float` (order-hash recompute, pinned depository and allocator, rules 1–12) for ETH 4663 → ETH Ethereum |
| canister `ethnft.rs`, `ethflow.rs`, `ethnet.rs` | **new**: floor evidence, buy judge (on top of the existing `seaport.rs`), transfer and deposit judges, the flow, the real IO |
| canister `seaport.rs`, `chains.rs`, `nftnet.rs` pinned reads, `custodycap`, `outflow`, `floorsink`, `itemsink` | **reused** |
| `NftWrapper`, `NftRegistry`, `NftVaultBase`, `NftRemoteVault`, `NftSweepVault`, `src/homechain/*` | **not used** (stay OFF; tests kept) |
| keeper | floor posting **skipped** for NFT modules; caps and draws unchanged; the keyless resolver learns the NFT ticket shape |
| runner (`ops/`), monitor, reconciler, indexer, site, golive-check, deploy | extended (§6) |

---

## 1. The collections: live numbers and a shortlist

### 1.1 What was measured [M]

- **OpenSea** (`api/v2/collections/{slug}/stats`, keyless): the floor, sales and volume over 1, 7 and 30 days, owners.
  `api/v2/collections/{slug}`: the fee table (OpenSea's own 1 % fee to `0x0000a26b…a719` is **required** on every collection
  below; creator royalties are **optional** except Meebits' 2.5 %).
- **CoinGecko** (`api/v3/nfts/{id}`, keyless): the floor's change over 24 h, 7 d and 30 d.
- **On chain** (Ethereum): the collection's code (size, EIP-1967 proxy slot, ERC-2981, a Limit Break transfer validator,
  OpenSea's operator-filter registry in the code). Then every `Transfer` log of the collection over the last **50,400 blocks
  (7 days)**, and the receipts of the 60 most recent transfer transactions. Each receipt was parsed for a Seaport 1.5/1.6
  `OrderFulfilled` or a Blur v2 `Execution721*Packed` that sold **exactly one token of the collection for ETH, WETH or Blur
  Pool ETH**. "Sale median" is over the sales found in those 60 transactions; "bid" fills are accepted offers.
- **Not measurable without a key: listing depth near the floor.** OpenSea's `listings/collection/{slug}/best`, `events` and
  `offers` all answer "Missing an API Key"; Blur's API is behind a Cloudflare challenge; Magic Eden's `rtp` routes are gone. The
  7-day sale count and the spread of sale prices around the floor stand in for it here. The runner holds an OpenSea key in
  production (§4.1), and **measuring the depth with it is a go-live check (§9, D6).**

| Collection | Floor (OS) | 7 d sales / vol | 30 d sales | Floor Δ 7 d / 30 d (CG) | On-chain sales in sample | Sale median (vs floor) | Venues seen | Contract notes |
|---|---|---|---|---|---|---|---|---|
| **Milady** `0x5af0…25a5` | **0.930** | 113 / 94.9 | 467 | +3.3 % / **−10.9 %** | 14 (8 listing, 6 bid) | 0.9105 (−2.1 %) | Seaport 1.6, Blur v2 | plain ERC-721, no proxy, no 2981, no validator, no filter |
| **Pudgy Penguins** `0xbd35…2cf8` | **3.282** | 137 / 432.7 | 448 | −3.1 % / −25.4 % | 36 (14 / 22) | 3.255 (−0.8 %) | Seaport 1.6, Blur v2 | plain ERC-721, none of the four |
| BAYC `0xbc4c…f13d` | 6.350 | 176 / 563.5 | 353 | −5.9 % / −19.6 % | 9 (7 / 2) | 6.400 (+0.8 %) | Seaport 1.6, Blur v2 | plain; one 0-price fill in the sample (a `matchAdvancedOrders` side), ignored by the rules of §4.2 |
| **Azuki** `0xed5a…c544` | **0.660** | 92 / 53.8 | 324 | −2.9 % / −25.0 % | 20 (13 / 7) | 0.650 (−1.5 %) | Seaport 1.6 (**11 listing fills**), Blur v2 | ERC721A, none of the four |
| **Mutant Ape YC** `0x60e4…a7c6` | 0.865 | **416** / 191.9 | 877 | −6.5 % / −25.3 % | 17 (10 / 7) | 0.8595 (−0.6 %) | Seaport 1.6, Blur v2 | plain, none of the four |
| Lil Pudgys `0x524c…a048` | 0.317 | 303 / 78.0 | 1,003 | n/a | 22 (12 / 10) | 0.306 (−3.5 %) | Seaport 1.6, Blur v2 | plain, none of the four |
| Remilio `0xd3d9…ebab` | 0.149 | 91 / 13.5 | 309 | n/a | 49 (9 / **40**) | 0.144 (−3.5 %) | mostly Blur bids | plain |
| Doodles `0x8a90…992e` | 0.272 | 104 / 26.2 | 529 | **−17.4 %** / −27.5 % | 22 | 0.274 | Seaport, Blur | plain |
| Moonbirds `0x2358…a68b` | 0.424 (CG) | n/a | n/a | −12.5 % / **−41.6 %** | 20 | 0.405 | Seaport, Blur | ERC-2981 |
| Meebits `0x7bd2…6bc7` | 0.316 | 155 / 24.3 | 300 | −11.4 % / −26.4 % | 14 | 0.312 | mostly Blur | **2.5 % creator fee required** |

(Floors in ETH. ETH ≈ $2,700 [M, `phase1-nftx.md`].)

**What the numbers say:**

- **Recent on-chain sales sit 0–4 % under the marketplace floor** for every collection. Bid fills pull the median below the
  floor, as expected. So an on-chain sale median is a sound **bound** on a listing-based floor (§4.2), not a replacement for it.
- **Every candidate is a plain ERC-721** with no proxy, no Limit Break transfer validator and no operator filter in its code.
  So an owner's own `transferFrom` (a delivery) cannot be blocked by a marketplace rule. A simulation at the pin is still
  required before every transfer (§4.4).
- **About half of Ethereum listing fills of these collections are basic orders paid in ETH**, which the canister's
  `seaport.rs` already fills. The rest are `matchAdvancedOrders` (WETH or collection offers) or aggregators. For example,
  Azuki showed 11 Seaport listing fills, 7 of them `fulfillBasicOrder` in ETH.
- **Blur's listings are not Seaport orders.** The canister buys only Seaport (OpenSea and every Seaport marketplace). Blur's
  sales still count as **evidence** for the floor.

### 1.2 Fit with the stage-0 caps [R]

- A normal graduation's item budget is `raise − min(5 %, 0.25 ETH) − 12.6 %`: **≈ 4.12 ETH** at the 5 ETH raise, **≈ 6.6 ETH**
  at the 8 ETH early raise (`DeployStack._template`, `_targets`).
- Stage 0 holds **3 items and 10 ETH per collection** in custody, and **10 ETH per item** (`custodycap::stage0`).
- One graduation buys `⌊budget / (floor × 1.03)⌋` items. The rest stays ETH-backed wX, burnable at the module.

| Collection | Items one 5 ETH graduation funds | Stage-0 room (3 items, 10 ETH) |
|---|---|---|
| Milady (0.93) | 4 (3 fit) | 3 items ≈ 2.9 ETH |
| Azuki (0.66) | 6 (3 fit) | 3 items ≈ 2.0 ETH |
| MAYC (0.865) | 4 (3 fit) | 3 items ≈ 2.7 ETH |
| Pudgy (3.28) | 1 | **2 items** (3 × 3.38 > 10 ETH) |
| BAYC (6.35) | **0** (two graduations for one item) | **1 item** |

### 1.3 The shortlist (recommendation)

**Rule:** ≥ 50 sales in 7 days; sale median within 5 % of the floor; a 30-day floor change no worse than −30 %; a plain ERC-721
(no proxy, no transfer validator); at least one item per graduation at stage 0; no required creator fee above 1 %.

1. **Milady — the Phase-1 pilot** (named by the operator). It has the most stable floor of the set (−10.9 % over 30 days vs
   −19 % to −42 % for the others), 113 sales a week, a sale median 2 % under the floor, and 3 items per collection fit at stage 0.
2. **Azuki** — second. 92 sales a week, the most Seaport basic-order fills in the sample, cheap enough that stage 0 is 3 items.
3. **Mutant Ape Yacht Club** — third. The deepest by sales (416 a week).
4. **Pudgy Penguins** — the flagship, but only 1 item per graduation and 2 at stage 0. Add it **after stage 1** (12 items,
   40 ETH per collection), or accept that its units stay mostly ETH-backed at stage 0.
5. **BAYC** — **not at stage 0**: a graduation's budget does not buy one ape.

Not recommended now: Doodles and Moonbirds (falling floors), Remilio (mostly Blur bids, few Seaport listings), Meebits (a
required 2.5 % creator fee), Lil Pudgys (fine, but 22k supply and thin per-item value; a later candidate).

The collections are **per-collection configuration** everywhere (install argument, stack file, runner, monitor), so adding one
is a deploy plus a timelocked canister upgrade, not code.

---

## 2. The flows

### 2.1 Buying one NFT (the frog model, on Ethereum)

```
4663                                             canister (ICP)                                   Ethereum
──────────────────────────────────────────────  ──────────────────────────────────────────────  ─────────────────────────────
FrogCurve.graduate → FloorModule.enqueue
  (ETH-backed wX; netUnits ≥ 1 + buffer)  ───▶  tick: floorsink view says "want 1"
                                                 EthBuy{NeedAdvance}   H2 room reserved, H3 value
FloorModule.advance(amount ≤ maxItemCost) ◀───  (one 4663 tx, canister-wide)
  unit in transit; ETH → canister's 4663 addr
                                                 EthBuy{Funded}: ask = amount − GAS_KEEP
                                        runner: POST relay.link/quote (4663 → 1, to us)  → eth_relay_quote
                                                 relayeth::judge (12 rules, order hash)
Relay depository.depositNative(us, orderId) ◀── EthBuy{Depositing → Bridging}
                                                 watch: Ethereum balance ≥ before + minOut  ───▶  (Relay solver pays us)
                                                        or 4663 balance back (refund), or deadline → Stuck
                                                 EthBuy{Arrived{wei}}
                                        runner: OpenSea best listings + fulfillment data  → eth_listing
                                                 ethnft::judge_buy + nftnet::check at an Ethereum pin
                                                 sign fulfillBasic/AdvancedOrder (Ethereum line)  ───▶  Seaport: NFT → us
                                                 read back: receipt at pin; ownerOf = us; block ≤ finalized
EthNftWrapper.mint(tokenId, buyTx, module) ◀─── EthBuy{Minting}  (H3 mint)
FloorModule.settleAdvance(cost)            ◀─── EthBuy{Settling}; cost = ⌈advanced × paid / arrived⌉
  transit unit → item-backed                     EthBuy{Settled}; the ETH left over stays at our Ethereum address as the gas pot
```

- **One purchase in flight per collection, and one Relay conversion at a time canister-wide** (as built since `nft-pool`; the module refuses a second
  `advance` while one is open, `AdvanceOpen`).
- If Relay refunds, the canister asks again (3 attempts), then returns the ETH (`returnAdvance`), exactly as `relay-float`.
- If the ETH arrives and no listing within the bound appears (the floor ran away), the buy waits `Arrived` and the monitor pages
  after 24 h. There is **no automatic Ethereum → 4663 return in this change** (§9, D4): the operator's options are to wait, or a
  reviewed upgrade that adds the reverse Relay leg (the judge is written for it: `relayeth` takes the origin as a parameter).
  *(Fix round 2 of the 2026-09-27 review, A2-M-1 / C2-M-2, is that change: after 48 h `Arrived` (or at once, `eth_give_back`),
  what arrived less the gas reserve goes back through a Relay order Ethereum → 4663 judged by `relayeth::judge` with
  `FROM_ETHEREUM` (`BackFunded` → `BackSending`, the deposit on the Ethereum line → `BackBridging`, watched on 4663 and for a refund
  on Ethereum), then `returnAdvance` to its module. And no advance is made unless it reaches Ethereum covering the module's floor
  + 1 % after Relay's estimated fee (`advance_covers`), so a pooled module whose floor rose waits on 4663 instead.)*
  *(Fix round 3, C3-M-1 / E3-L-1 / A3-L-1: the 48 h are counted from the FIRST arrival (a fill that reverts does not restart them,
  nor the 6 h deposit hold). A way back with no deposit signed within its 2 h quote window (`BACK_QUOTE_SECS`) stops holding other
  collections' buys: it waits `Arrived` again (it may take a listing) and tries again a day later (`BACK_RETRY_SECS`). A way back
  Relay neither pays nor refunds is watched until a day after its order's deadline and the grace (`BACK_LATE_SECS`), then `Stuck`
  without a watch, off the Relay leg; the operator closes it (`eth_give_back`: `returnAdvance` of 0, the loss booked by the module)
  once Relay says the order is dead. A collection whose advance came back waits a day (`RETURN_COOLDOWN_SECS`) before the next.)*
  *(Fix round 4, C4-M-1: a Relay order past its deadline stays WATCHED (it holds the Relay leg) until its payout is read or the
  operator writes it off: a payout later than the watch was read as another order's refund or arrival. The operator may write it
  off (`eth_give_back`) only once the canister has watched it a day past its deadline and the grace; its watch is kept
  (`EthBuy.late`), and a payout that comes after all is credited to that order and collection from its own transaction
  (`eth_credit_payout`, judged by `judge_payout`: it names the order by id or comes from the order's own solver, pays our address,
  at most what the order sent and at least half; `returnAdvance` while the buy is open, `fundReserve` or the collection's leftover
  once it was closed). What a refund or arrival gives back is the rise over the watch's start, never the raw balance.)*

### 2.2 Redemption

```
holder: EthNftWrapper.redeem(ethAddress)      burns 1 wX; ticket pinned to drand round now + 20; destination fixed
anyone (keyless resolver, site): resolve(id, drandSig)   draws a token id among those free before the burn; deliverBy = +7 d
canister tick: reads ticketCount / ticket(i) at the 4663 pin (no logs)
   Resolved, not delivered, now < deliverBy − 1 day
   ethnft::judge_transfer: ownerOf(token) = us at the Ethereum pin; transferFrom(us, dest, token) simulates; fees capped
   sign on the Ethereum line (H3: one item out)  ──▶  Ethereum: transferFrom
   at finality: our delivery's own receipt, block ≤ finalized, its Transfer(us → dest, token)  (not who owns the token now)
   then EthNftWrapper.noteSent(id, txHash)  ◀── canister (fix round 3; round 4, A4-L-2: at finality, not at the first sighting)
EthNftWrapper.recordDelivery(id, txHash)  ◀── canister (one 4663 tx)
```

- **Random, not queue-ordered.** A FIFO queue lets anyone see which token is next and redeem only when a better one is at the
  head. With holder deposits (§2.4) the items are not all floor-bought, so that selection is real. drand is already in the Phase-1
  stack (the Bitcoin `Wrapper` uses it), and its ticket machine carries four review rounds of fixes, so reusing it adds no new
  dependency. The cost is one `resolve` transaction per ticket (keyless: the keeper's resolver, or the site's user) and about
  one minute.
- **No LostRace.** A resolved ticket is refundable only (a) after `deliverBy + 72 h` **and** the custodian's `markUndeliverable`,
  or (b) after `deliverBy + 30 days`. The canister marks undeliverable only when **no version of the delivery can ever
  confirm**:
  - nothing was ever signed (the simulation fails: a blocked destination, a collection that paused transfers);
  - or the transfer was mined and reverted, final;
  - or its nonce was consumed by our own **cancel** (a 0-value self-send at the same nonce, fees bumped), and that is final.

  The canister stops signing deliveries a day before `deliverBy` and cancels one still pending then. So a redeemer can never
  hold both the NFT and a refund. *(Fix round 3, D3-H-1: the delivery is recorded from its own receipt at finality, whoever owns
  the token by then: a redeemer who sold it into a bid or moved it on before finality was delivered to. And as soon as the
  transfer is seen mined the canister calls `noteSent(id, tx)`: from then on the wrapper's 30-day late window no longer refunds
  that ticket, only `markUndeliverable` does, so no failure of the record can pay a delivered ticket twice.)* *(Fix round 4,
  A4-L-2: noted at finality, from the same receipt as the record, just before it: a transfer a reorganisation drops before
  finality is never noted, so its ticket keeps the late window, and the canister calls it gone once the wrapper refunded it. A
  mark goes out while paused too.)*
- `redeemToBurn` delivers to `0x…dEaD` on Ethereum (the BURN policy's exit). `redeemToRelist` (YOYO) stays OFF: the canister
  never lists an NFT.

### 2.3 The floor

```
runner (every 20 min): OpenSea best listings (key, runner-side) + fulfillment/protocol data  ──▶ eth_floor_evidence(collection,
   listings[≤ 8], [])   (fix round 2: sale hashes are no longer handed in, and any handed in are not used)
canister, at one Ethereum pin, both providers:
   its OWN sale sample (fix round 2): the collection's Transfer logs over the final blocks of the last 7 days (eth_getLogs, a few
     chunks a pass, newest first), then each transaction's receipt once (16 a pass), every verdict kept (fix round 3, B3-M-1:
     read one per token and per sender at a time, oldest first, so a flood of plain transfers cannot starve a real sale; a
     queue over 512 drops the last in that order and reads their blocks again later; a verdict without a sale never takes a
     sale's place in the cache)
   each listing: ETH for one token of the collection; not expired; getCounter → order hash; getOrderStatus not cancelled or
     filled; ownerOf(token) = offerer; conduit approved (isApprovedForAll / getApproved); EOA or bulk signature recovers to the
     offerer (ERC-1271 listings are left out)
   each sale: receipt at or below the pin, within 7 days of blocks; one token of the collection for ETH/WETH/Blur Pool ETH in a
     Seaport 1.5/1.6 OrderFulfilled or a Blur v2 Execution721*Packed; one sale per (tx, token)
   ethnft::floor_from(listings, sales, params, accepted)  → (price, confidence, depth)  — "no post" only with too few listings
     (or no confirmed first post)
FloorModule.postFloor(price, confidence, depth) ◀── canister, as the module's keeper (PosterSource), one 4663 tx
   the module clamps to ±5 %/h and ±20 %/day as today
```

### 2.4 Holder deposits (on at launch: `eth_nft.deposits = true`, D5, 2026-09-27)

```
holder, on Ethereum: collection.safeTransferFrom(holder, canister, tokenId, "EWN1" ‖ uint32 BE 4663 ‖ wrapper ‖ recipient)   (48 bytes)
runner (discovery): every Transfer(*, canister, *) log of a configured collection in the last day  → eth_deposit(txHash)
canister (the claim judge, ethnft::judge_claim), tx and receipt at the pin (both providers):
   a DEPOSIT: to = the collection, input = safeTransferFrom(from, us, id, data) with from = the tx's sender, data = "EWN1" ‖ this
     home chain ‖ this collection's wrapper ‖ a recipient that is not zero, the wrapper or us; exactly one Transfer(from, us, id)
     log from the collection; status 1
   STRAYS: any other Transfer(x, us, id) of a configured collection in it (no data, garbled data, another chain or wrapper, a
     plain transferFrom, moved by an approved operator or a contract): each goes back to its own x
   NOTHING of a configured collection (or our own transaction): refused, nothing is touched
deposit, past finality (block ≤ the lower finalized): the receipt read again (a reorg moves it), ownerOf = us, itemOf = 0, H2 room
   (none → it goes back) → EthNftWrapper.mintDeposit(tokenId, txHash, recipient)   (one asked at a time; H3 and H11 limits)
stray, past finality: ownerOf = us, itemOf = 0, not in any open buy or claim, and not moved out by us since it came in
   → transferFrom(us, x, id) on the Ethereum line, after deliveries and the buy, within the day's return budget (Flow::Return,
   which never trips the freeze), only while our balance less an arrived buy's ETH keeps 0.005 ETH above the return's gas
```

- **The data names the deployment** (docs/v4/nft-wrapper.md §4): the holder's own signature binds the home chain and the wrapper
  as well as the recipient, so a claim cannot mint on a wrapper the holder did not name. The earlier 36-byte form (a recipient
  alone) is refused, and such an NFT is returned.
- **Discovery is the runner's; judgment is the canister's.** The runner can only withhold. The monitor reads the same Transfer
  logs on its own provider and pages (SEV2) an inbound transfer the canister has not judged two hours on; the operator can hand
  it in by hand (`eth_deposit`). A claim that cannot be read yet (not mined, above the pin, a provider error) is tried again for
  six hours and then dropped **unrecorded**, so a transient failure never refuses a deposit for good.
- **Strays go back** to the `from` of their own `Transfer` log (the owner, even when an operator moved it), by `transferFrom`
  (no hook runs), once final. A stray is **held** for the operator (never minted, never returned automatically) when the return
  cannot be judged or signed: the wrapper holds that token, an open buy or claim uses it, it left us after that transfer came in
  (an old transaction handed in again), the simulation refuses, or more than four came in one transaction.
- **An NFT of a collection the canister does not serve** is never touched: the canister calls configured collections only (no
  call to an unknown contract from the custody key). It stays at the address; the reconciler lists it. Sending one is the
  sender's loss, stated on the site.
- **No custody room** (stage 0: 3 items per collection, 6 in all) sends a valid deposit back rather than keeping it unminted.

## 3. Contracts (4663)

### 3.1 `EthNftWrapper` (new)

`Wrapper.sol`'s redemption machine (after rounds 1–4), keyed by an ERC-721 token id on one EVM chain instead of an inscription:

- **Identity** (immutables): `registry`, `listingId` (its pause flag gates mints), `beacon`, `chainId` (≠ this chain), `collection`,
  `custodian` (the canister's EVM address: the minter, the delivery recorder, and — the same 20 bytes — the holder of every NFT
  on `chainId`), `parentBlockSeconds`, `deployer`.
- **Mints** (custodian only, listing not paused, H11's `_countMint`): `mint(tokenId, purchaseTx, to)` and
  `mintDeposit(tokenId, depositTx, to)`. A token id is held at most once (`itemOf`), and a `(tokenId, tx)` source mints at most
  once, ever (`sourceUsed`).
- **Redeem:** `redeem(address destination)` (not zero, not the custodian), `redeemToBurn()` (`DEAD`), `redeemToRelist()`
  (kept for `IWrappedCollection`; its delivery is "stays with the custodian", which the canister never performs, so it
  refunds by the late window). A ticket needs an unpromised item: `held ≥ openTickets + 1`.
- **Resolve** strictly in ticket order, drawing among items that were free when the ticket was burned (pending stamps).
  **No `redraw`** (audit C-02: a redeemer who has seen the pinned round could discard it). A ticket still pinned after
  `PIN_TIMEOUT` (7 days, both clocks) may be refunded, and resolution skips it (E-03).
- **Deliver:** `recordDelivery(id, txHash)`, custodian only, while `Resolved` (also past `deliverBy`, until a refund).
  `markUndeliverable(id)`, custodian only, while `Resolved`. `noteSent(id, txHash)` (fix round 3, D3-H-1), custodian only, while
  `Resolved`: the delivery mined (`sentTx`, `DeliverySent`). `refund(id)`: from `Resolved`, past `deliverBy + REFUND_GRACE` (72 h)
  with `undeliverable`, or past `deliverBy + LATE_DELIVERY_WINDOW` (30 days) without it and without `noteSent`
  (`DeliverySentNotRecorded`); the item goes back to the pending set and one unit to the ticket's owner.
- **The floor module:** `wireFloorModule(module)` once, by the deployer, before anything was minted, and only a module whose
  `wrapped()` is this wrapper (B-05). `moduleMint` / `moduleBurn` unchanged. **Rentals are not supported:** `admitRental` and
  `returnRental` revert (`NoRentals`); `rentalState` is `None`, `openReturns` and `rentedHeld` are 0. Rentals are OFF in Phase 1
  and the module's `rentIn` then reverts.
- **The ledger:** `totalSupply == (held − openTickets) × ONE_ITEM + ethBacked` (the invariant suite asserts it).

Why a new contract and not `NftWrapper`: `NftWrapper` predates the review rounds. It still has `redraw` (C-02), refunds a
resolved ticket at `deliverBy` with no grace and no attestation (D M-1 / review2 N-2: a late delivery plus a refund), and no
B-05 check. Porting those four fixes into it would touch every one of its tests. `EthNftWrapper` is `Wrapper`'s code, which
already has them, re-keyed. `NftWrapper` stays OFF and untouched.

### 3.2 `PosterSource` (new)

```solidity
contract PosterSource is IKeeperSource {
    IKeeperSource public immutable registry;   // the ListingRegistry: its pause, its resume stamps, its owner and guardian
    address public immutable poster;           // the canister's EVM address (code-less, fixed)
    bool public paused;                        // this collection's own floor switch (fix round 1, D-M-1 / A-L-3)
    function pause()   // the registry's owner or guardian, at once;   unpause(): the owner only (stamped: resumedAt, resumedAtBlock)
    function keeper() → paused || registry.keeperPaused() ? address(0) : poster
    function keeperPaused → paused || registry.keeperPaused();  keeperResumedAt(Block) → the later of its own and the registry's
    function owner / guardian → the registry's
}
```

- **Per-collection switch (fix round 1 of the 2026-09-27 review, D-M-1 / A-L-3):** the monitor's D8 pause (and a runaway or
  walked floor of an NFT module) pauses that collection's `PosterSource` only; every other collection goes on. The registry's
  pause stays the canister-wide switch. golive-check requires `paused() == false` at go-live.

- The NFT collection's `FloorModule` is constructed with `registry_ = PosterSource`. Its floor is then posted **only by the
  canister**. The keeper EOA keeps the registry's roles (caps and draws, which are OFF for sweeps in Phase 1).
- **The incident switch still works:** the Safe or the guardian pausing the keeper at the registry zeroes the poster at once.
  Every inflow priced by the poster stops (`_keeperDown`), and burns go on at the pause floor (A3-M1 / A4-L1, unchanged).
- The poster address is immutable. The canister's EVM address does not change across upgrades (same key, same derivation path). A new
  canister would need a new module, which a new collection deployment is anyway.

### 3.3 `FloorModule`: unchanged

`advance`, `settleAdvance` and `returnAdvance` (relay-float) are used exactly. `payee` = the canister's EVM address. The
parameters are the stage-0 defaults (`StackChecks.defaultParams`), with per-collection caps from §5.

### 3.4 Deploy (`DeployStack`)

- `ETCH_STACK_KIND=nft` deploys **the NFT collection alone** (no Bitcoin `WrapClaim` / `Wrapper` / OfferBoard). The treasury and
  directory are made as today.
- Per NFT collection: `EthNftWrapper` (deployer = the deploy key, wired and `initMintLimit`ed in the same run, before any mint),
  `PosterSource(registry, ETCH_NFT_POSTER)`, then the module (`registry_ = PosterSource`), lock, factory, router and vaults as today.
- The stack file's collection gets `kind: "nft"`, `floorPoster`, `posterSource` and `custody { kind: "evm", chainId, collection, custodian }`.
- `StackChecks` refuses: no poster, a poster with code, a poster that is not the custodian, `chainId` = this chain, no collection.

---

## 4. The canister

### 4.1 Configuration (install argument, operational: taken from every upgrade's argument, so a change is a published, timelocked act)

```candid
eth_nft : opt record {
  chain_id : nat64;                    // 1; must be one of evm_chains (pins, providers, Seaport code hash, fee caps come from there)
  collections : vec record {
    collection : text;                 // ERC-721 on chain_id
    wrapper : text;                    // its EthNftWrapper on 4663 (custodian() must be us)
    module : text;                     // its FloorModule (wrapper() = wrapper, payee() = us, keeper() = us)
    max_purchase_wei : nat;            // ≤ custody per-item cap, ≤ 25 ETH
    max_premium_bps : nat16;           // a buy's total ≤ our floor × (1 + this); ≤ 1,000
  };
  relay_max_fee_bps : nat16;           // Relay's min-out ≥ amount × (1 − this); [50, 500]; default 300
  deposits : bool;                     // holder deposits (and stray returns); TRUE at launch (D5, 2026-09-27)
  floor : opt record { min_listings : nat8; listing_rank : nat8; min_sales : nat8; up_cap_bps : nat16; down_cap_bps : nat16 };
}
```

- `null` = the Ethereum NFT side is inert (a Bitcoin-only install is unchanged).
- Validation: `chain_id` must be listed in `evm_chains` and not be the home chain; collections distinct; bounds as above.
  **The Ethereum chain's line belongs to this flow alone:** `nft_sign` refuses `chain_id` once `eth_nft` names it (two writers
  on one nonce line would break "one transaction in flight").
- Custody caps (H2): the default (`custody_caps = null`) now lists each `eth_nft` collection at stage 0 beside the Bitcoin
  wrapper. The key is `evm:1:0x<collection>`.
- **The canister's Ethereum providers (`evm_chains[1].rpc_urls`), review round 6 (B6-M-1):** `https://rpc.mevblocker.io` and
  `https://gateway.tenderly.co/public/mainnet`: keyless (the argument is public), independent operators, and each measured
  (2026-09-28, read-only) to serve `eth_getLogs` and non-null receipts 130,000+ blocks deep, past the 7-day sale window (50,400
  blocks), and 13-way bursts. Each provider of the pair must serve the whole window: the sale scan's two-provider reads stop at the
  shallower one (`ethereum-rpc.publicnode.com`'s keyless logs stop ~10,000 blocks back, which froze the scan for days after any gap
  in review round 6). `golive-check --phase1` refuses a chain-1 provider not on its measured list unless `--probe-depth` measures it
  now; `xrpc.cl/eth` (an anonymous rate limit of 10) stays the monitor's second provider only.

### 4.2 The floor (`ethnft::floor_from`)

**Why not an on-chain price, and why not a marketplace API alone:**

- Listings live off chain (OpenSea, Blur). There is no on-chain order book or pool for these collections (NFTX was measured
  empty).
- Sales are on chain, but they are last-traded prices. They include bid fills below the floor and trait sales above it, and a
  wash sale costs only gas on Blur.
- An API answer (OpenSea's `floor_price`) is one company's word, and the canister cannot check it.

**So the canister posts what it can verify:** real, currently fillable Seaport listings. It bounds them with real, recent
on-chain sales, which the canister parses from receipts itself. Neither input can be forged. The runner can only **withhold**,
and withholding is bounded by the sales from above and by the monitor from outside.

**Default parameters** (`floor` = `null`): `min_listings` 3, `listing_rank` 2, `min_sales` 3, `up_cap_bps` 1,000,
`down_cap_bps` 1,500.

1. **Listings.** Each passes the checks of §2.3. Keep the cheapest per offerer. With at least `min_listings` distinct offerers,
   `L` = the `listing_rank`-th cheapest total (what a buyer pays, fees included). One listing cannot set the floor alone.
2. **Sales.** Each passes the checks of §2.3, and the collection's own `Transfer` of the token is in the receipt (who sold it,
   who bought it). Counted: no self-trade (seller = buyer), one sale per token, per seller and per buyer (the newest), and only
   within ±30 % of `L` (`SALE_BAND_BPS`). With at least `min_sales` counted, `S` = the median and `S25` = the 25th percentile.
   *(Fix round 2, A-M-1 / B2-M-1 / D2-M-2: the sales are the canister's OWN read of the collection over the window (§2.3), never
   a hand-in; one per WALLET (a wallet that sold or bought in a counted sale is a party to no other), and none of a token that
   went back to an earlier holder in the window; our own purchases are left out.)*
   *(Fix round 1 of the 2026-09-27 review, B-H-1: the text below was "a burst of cheap listings cannot pull it more than 15 % under";
   as built the sales CLAMPED the post, which let wash sales set it. They now only veto.)*
3. **The post is `L`, always** (a verified live listing: the post never leaves the listings' range). The sales only **veto** it:
   `L > S × (1 + up_cap)` (cheap listings withheld) or `L < S25 × (1 − down_cap)` (listings the market has not traded at) is **no
   post**. Wash sales can therefore stop a post (at the cost of as many wallets as the real sellers they outvote), never steer one.
   Before it is sent the post is clamped into the module's `floorBounds()` read at the pin (D-H-2), as the keeper does.
   *(Fix round 2, B2-M-1 / B2-L-1: a veto no longer stops the post. Confirmed (`min_sales` counted and `S25 × (1 − down_cap) ≤ L ≤
   S25 × (1 + up_cap)`): the post is `L`. Too few counted sales, or `L` over that band: the post is **held**,
   `min(L, max(accepted, S25 × (1 + up_cap)))` at confidence 5,000 (it follows the listings down and rises only as far as the
   counted sales support). `L` under the band: posted as it is at 5,000, flagged. Only the first post needs the confirmed case.
   The band is now over `S25`, not `S`.)*
4. **Confidence:** 10,000 if `|L − S| ≤ 5 %` of `S`; 7,000 within 15 %; 5,000 beyond. The module's minimum is 3,000.
   `depthItems` = the distinct offerers with a verified listing at most `L × 1.05`.
5. **No post** (the module goes stale, by design) if either input is short. A stale module refuses mints, and burns go on
   (the module's own rules, unchanged). *(Fix round 2: only too few listings is "no post" now; a stale module refuses burns too
   (`Stale`) until the poster pause or the 7-day escape, so the launch monitor configuration arms `pause_on_no_good_post_h`.)*
6. **Cadence:** post when the module's post is older than 30 minutes, or `P` differs from the accepted floor by more than 1 %.
   At most one post per tick; a post whose module was last posted 40 minutes ago goes before every other home-chain act
   (fix round 1, B-L-9).

**What this measures today [M]:** the median of recent sales sat 0.6–3.5 % under the OpenSea floor for every collection in
§1.1. So `P = L` (unclamped, confidence 10,000 or 7,000) is the normal case.

### 4.3 The buy judge (`ethnft::judge_buy`, then the existing `nftnet` checks)

The runner hands in `eth_listing(buy_id, order_hex)`. The canister refuses unless **all** of these hold:

1. **The buy is `Arrived`**, for this collection. The order is for **this collection** and one token (`seaport::verify`).
2. **The price bound:** `total ≤ min(floor × (1 + max_premium_bps), arrived − ETH_GAS_RESERVE, max_purchase_wei)`.
   - `floor` = the lower of the canister's own last reading (at most 1 h old) and the module's accepted floor at the 4663 pin.
   - `ETH_GAS_RESERVE` = 0.003 ETH: this purchase's gas, plus a floor for the gas pot.
3. **Seaport** is the canonical 1.6, with its code hash and `information()` domain on chain 1.
4. **Delivered to us:** a basic order delivers to `msg.sender`; for an advanced order the canister writes the recipient itself.
5. **No money to strangers:** additional recipients are the chain's fee recipients or the collection's ERC-2981 receiver, at most
   `max_fee_bps` of the total, no tips. The measured case is OpenSea's 1 %.
6. **Still fillable at the pin:** not expired (the chain's clock), not cancelled or filled, `ownerOf` = the offerer, the offerer's
   signature recovers, and the exact transaction **simulates** from us at the pin.
7. **No extra approvals or transfers:** the canister signs only its own re-encoding of the order (`fulfillBasicOrder` or
   `fulfillAdvancedOrder` to Seaport, value = total). It never signs an `approve`, a `setApprovalForAll` or anything the runner
   wrote.
8. **H2 and H3:** the value against the day's limit; the item's room reserved at the advance and settled at arrival.
9. **One transaction in flight on the Ethereum line**; its bytes are saved before the send.

A SignedZone (restricted) order passes only through the allow-listed zone, pinned by its code hash (the existing §12 rules).

### 4.4 The transfer judge (`ethnft::judge_transfer`)

Before signing a delivery, the canister checks all of these:

- The ticket is read at the 4663 pin: `Resolved`, not recorded, and its `itemId` → `tokenId` from `item(itemId)`.
- The destination comes from the ticket, never from the operator.
- `now < deliverBy − 1 day` (the canister's clock and the 4663 pin's timestamp).
- `ownerOf(tokenId)` = us at the Ethereum pin.
- The calldata is `transferFrom(us, destination, tokenId)`: no hook runs in our transaction, and the recipient's code is the
  redeemer's own choice. It simulates at the pin. Fees are within the chain's caps.
- The Ethereum line is free.
- H3: one item out (`Flow::Item`).

### 4.5 Arrival, finality and reorgs

- **Ethereum reads** use `nftnet`'s pin (bucket 8: 1–2 buckets behind the head, both providers agree on the block hash).
- **Anything that mints or records on 4663** also needs the transaction's block to be **≤ the finalized block**: the lower of
  both providers' `finalized`, rounded down to the bucket. Measured [M]: finalized sat 72–168 blocks (15–34 min) behind the
  head, and the two providers differed by 95 blocks. The lower one is used.
  - A reorg before finality re-reads (the receipt disappears, and the line waits).
  - After finality there is no reorg to handle.
- **Relay arrival** is a rise in our Ethereum balance (at the pin) of at least the order's `minimumAmount` since the deposit
  mined. This is balance-based, like the refund detection in `relay-float`. A stranger's gift can be mistaken for an arrival:
  the result is a buy paid by the gift, with no module ETH lost (`relay-float` §5.1). *(Fix round 2, C2-M-1 / C2-L-1: the watch
  starts only when every transaction of ours mined at the pin is counted (no quote is judged while the line's outcome is unread
  that tick); what is earmarked is at most the order's value and what the balance holds beside the other buys' earmarks; the watch
  ends at the arrival.)* *(Fix round 3, E3-M-1: every balance read of the flow (a watch's start, an arrival, a refund on Ethereum,
  the sweep) is made only at a pin at or above our last mined transaction's block and the watch's own pin; a pin that moved back
  waits. The way back's arrival on 4663 is the order's minimum less 1 %, no longer half of it (C3-L-3).)*

### 4.6 State and endpoints

- `EthState` (stable, the **thirteenth** element of the upgrade tuple, optional so older states decode) holds:
  - `buys`
  - `deliveries`
  - `deposits`
  - per-collection floor readings and ticket cursors
  - the Ethereum line (one transaction in flight, its raw bytes saved before any send)
  - the 4663 pending transaction (part of the canister-wide "one EVM transaction of ours at a time")
  - every Relay order id ever accepted (never pruned)
- Operator endpoints:
  - `eth_relay_quote(buy, json)`
  - `eth_listing(buy, order_hex)`
  - `eth_floor_evidence(collection, listings, sales)`
  - `eth_deposit(tx_hash)` (refused while `deposits = false`): a claim, handed in by the runner's discovery or the operator
  - `eth_give_back(buy)` (`Funded` → `returnAdvance`; since fix round 2, `Arrived` → its way back through Relay, since fix round 3
    only once it has waited 6 h; since fix round 3 also `BackFunded` → `Arrived` again, and an unwatched `Stuck` → closed as a loss,
    `returnAdvance` of 0; since fix round 4 (C4-M-1) a `BackBridging` or a watched `Stuck` is written off only once it has been
    watched a day past its deadline and the grace, and a watched `Stuck` gives back only its order's measured refund)
  - `eth_credit_payout(buy, chain_id, tx_hash, wei)` (fix round 4, C4-M-1): a late payout of a written-off order, credited to that
    order and collection once its transaction is judged against the order
- Queries: `eth_nft_state()` and `eth_nft_config()`.
- Every update is guarded by `only_operator`, and every signing path by the guardian's pause (H1). The tick runs the flow
  after the frog flow and shares its "one 4663 transaction at a time" rule. `nftnet::nft_sign` refuses the Ethereum chain.

---

## 5. Limits and caps (stage 0)

| Bound | Where | Value |
| --- | --- | --- |
| One advance in flight per module; one conversion canister-wide | contract `AdvanceOpen`; canister | 1 |
| Per purchase | `maxItemCost()` (floor + 3 %); `max_purchase_wei`; custody per-item cap | e.g. Milady 1.0 ETH |
| A buy's price | our floor × (1 + 5 %), and what arrived less the 0.003 ETH gas reserve | — |
| Items per day | module H11 `maxItemsPerDay` 3 (at the advance); wrapper H11 mints 3/day; canister H3 3 mints a day per collection and 6 in all (a collection's waiting mint is one ask: fix round 4, E4-H-1), 25 ETH a day | 3 |
| In custody | H2 (J7, the operator, 2026-09-27): 15 ETH per Ethereum collection, one item up to 15 ETH. Items (the operator's option (c), 2026-09-28, `fixes-round5.md` §6): **13 / 17 / 13 / 3 / 2 at launch (48)**, J7's 16 / 22 / 17 / 4 / 2 (61, what 15 ETH fits at the floor) after step 3 (**since 2026-10-09, with Bitcoin Frogs at launch: Azuki 14 at steps 1–2, so 45 + Frogs' 3 = 48; step 3's global 62**, `golive-runbook.md` §9); in all the collections' sum, so no total binds; a deposit counts at the module's floor | stage 0 |
| Deliveries | H3: 12 a day per collection and 24 in all at the launch install, 31 in all after step 2 (a timelocked upgrade changing only `global_items`, proposed once the install has executed); tripwire at 3× freezes custody. Every item must leave within the timelock's 72 h less a day, each collection alone AND the whole custody (A4-L-1), on the caps and on what custody holds at an upgrade (A5-L-1); deliveries go backlog first, a delivery within two days of its deadline before all (A5-L-2) | stage 0 |
| Value out a day | H3: 25 ETH (the operator, 2026-09-27; 10 ETH before) | stage 0 |
| Relay fee | min-out ≥ amount × (1 − 3 %) | measured 0.36–2.3 % (§8) |
| Ethereum fees | `evm_chains[1].max_fee_wei` 50 gwei, gas cap 400k; a pin needing more waits | measured base 0.068 gwei median, 1.5 gwei max [M] |
| The floor | ±5 %/h, ±20 %/day (contract); sales band +10 % / −15 % (canister) | — |
| Public mints | `maxPublicEthBacked` 10 items, 5 an hour (module params) | unchanged |

**The new exposure at any moment:**

- one advance (≤ floor + 3 %) in Relay's hands for seconds [M: `timeEstimate` 4 s];
- the NFTs in custody (J7: up to 15 ETH of them per collection, counted at the floor);
- the gas pot on Ethereum (the purchases' leftovers, ≈ 1–3 % of each price).

### 5.1 Runbook: the staged launch (option (c)), the round-5 and round-6 pages

The operator's option (c) (2026-09-28, `fixes-round5.md` §6), step by step (`deployments/launch/nft-pilot.json` `stages`):

1. **Install (step 1).** The launch install names the 48-item split at 12 / 24 a day. The go/no-go read is
   `script/golive-check.sh … --phase1 --launch deployments/launch/nft-pilot.json --canister <buyer> …` (the file's `"stage": 1`).
2. **Queue the outflow raise (step 2) only after step 1 has EXECUTED and the handover is done** (a raise queued beside a timelocked
   install goes stale when the install executes). Build its argument from step 2's numbers (a whole `Init`: every field as running,
   `outflow_limits.global_items` 31) and check it before proposing: `script/golive-check.sh --phase1 --proposal <arg hex> --launch
   deployments/launch/nft-pilot.json --stage 2 --canister <buyer> --baseline <the running install's argument>` (review round 7,
   A7-I-1: `--phase1` requires the baseline, so every other `Init` field is compared; without it an extra change passes with a note). The monitor's argument diff pages it as **EXPECTED (SEV2)**, not SEV1, because it is exactly step 2 from the baseline
   (`launch_file` in its configuration); any other diff is judged as ever. Announce it (H6).
3. **No other upgrade of the buyer may execute during its 72 h.** Any executed target action makes the raise stale (the timelock's
   `target_epoch`): if a security upgrade must go first, execute it and **queue the raise again**. A withdrawn and re-queued raise
   waits the full 72 h again.
4. After it runs, set the launch file's `"stage": 2` and read the canister again (`--stage 2`): its whole `custody_outflow()` must be
   step 2's (A5-I-1).
5. **The caps back to J7 (step 3), after two clean weeks at step 2:** a separate proposal whose argument changes only the item caps
   (J7's 61, the global their sum) and carries `global_items` 31; `golive-check --phase1 --proposal … --stage 3 --baseline <step 2's
   running argument>`. A caps-first proposal (61 at
   24 a day) is refused by the canister at execute time and by golive-check before.

**Go-live prep (2026-09-28): every check below is prepared as a runbook with its scripts in `docs/v7/golive-runbook.md`** (the IC
rehearsal kit `ops/rehearsal/ic-outcall/`, the C5-I-1 kit `ops/rehearsal/relay/`, the depth-probe baseline and its daily workflow, the
filled monitor configuration, the Railway variables, the cycles plan). Review 8's B8-L-1 is fixed on the monitor's side
(`eth-scan-window-unread`), so no periodic manual probe is needed.

**Before the install argument is built (review round 6, B6-M-1), the operator's rehearsal checks:**

- **The canister's Ethereum pair from a real IC outcall.** On a rehearsal canister (a local replica does not count: the question is
  the IC's own network), read through the canister's transforms, on both `https://rpc.mevblocker.io` and
  `https://gateway.tenderly.co/public/mainnet`: a pinned `eth_getBlockByNumber`; Milady's `Transfer` `eth_getLogs` over the FIXED
  range 26,012,410..26,012,909, which holds exactly **10** logs (measured on both providers 2026-09-28, 63,500 blocks deep then:
  `docs/audit/2026-09-27-nft/fixes-round7-poc/P/`); and the receipt of `0x17133e8bbc4d8f1fbdf25e1698229bbe9f732d085069b82dd03cd75cdd55c093`
  (block 26,012,500, one Milady `Transfer`). Both must reach consensus (every replica the same answer), agree with each other, and
  return the 10 logs: **an empty or short list fails the check** (review round 7, D7-L-1: a provider that answers `[]` past its log
  retention looks deep to an empty range). **Tenderly's host publishes no IPv6 (AAAA) record**: confirm the IC's outcalls reach it (they have reached IPv4-only hosts
  through the IC's NAT64 since 2024, but this host has not been shown to). If either fails, pick another keyless provider that
  passes `script/golive-check.sh --phase1 --probe-depth` and this check, and name it in `golive-check`'s measured list.
- **`script/golive-check.sh … --phase1 --probe-depth --install-arg <the argument>`** at go-live: every chain-1 provider serves the
  7-day window now (a failure FAILs even a listed provider). Round 7 (D7-L-1): the probe reads the same fixed range and receipt and
  requires the 10 logs; it says when the range is more than 400,000 blocks deep (pin a newer one then, measured the same way). A listed
  host counts only as `https://` on the default port (A7-I-3).
- **Sends:** the canister broadcasts each Ethereum transaction to both providers and believes only the read-back (a send's reply is a
  report: `evmsend.rs`). Measured 2026-09-28 (a garbage payload and an already-mined stranger's transaction, nothing new sent): MEV
  Blocker answers `-32000 "… already known"` for a mined transaction and routes new ones to its private builders (a transaction of
  ours is visible to others only once mined, or through Tenderly's broadcast); Tenderly answers `-32602 "invalid params"` and
  `3 "nonce too low"`. Nothing in the canister acts on either.
- **The runner's cadence** (review round 6, F6-L-1; round 7, F7-L-1 / D7-I-3): 4663's first provider keeps state only 6,198–6,234
  blocks under its own head, **≈ 10 minutes** at 9.95 blocks a second. The canister's search for a mined transaction every index
  misses starts at the lowest provider's rounded head at signing (0–1,023 blocks under the head then), so it is inside that reach for
  **≈ 520 s after signing at worst** (≈ 415 s if the other provider is a whole bucket behind; `ethflow::home_search_reach_secs`).
  While a 4663 transaction of the Ethereum NFT side is out, the runner's next tick comes `RUNNER_PENDING_EVERY_S` after the START of
  the cycle that signed it (a period, not a sleep after the cycle): **default 120 s, keep it at 120 (the runner refuses more than
  300)**. The rehearsal measures the runner's cycle length: a cycle over ≈ 250 s leaves no margin. A transaction seen nowhere for
  15 min is past the reach and the runner goes back to `RUNNER_TICK_EVERY_S` (the operator's path decides it). No other keyless 4663 provider keeps archive state besides
  `rpc.ordofi.network` (probed 2026-09-28: `robinhood.drpc.org` and `robinhood-rpc.publicnode.com` serve no historical state; the
  others tried need a key or do not serve 4663).

The round-5 and round-6 pages and exits:

- **`eth-home-unseen` (SEV1)**: our 4663 transaction is seen by no provider for an hour although its nonce is used (review round 6,
  F6-L-2: or the providers disagree on its receipt). The canister never does it again on that absence alone; it identifies what used
  the nonce or reads the step's effect back by itself. **Only a Relay deposit or a sweep's `fundReserve` leave nothing to read
  back**, and only for those does the page name `eth_home_clear` (the canister refuses it for any other step, D6-L-1): look the
  transaction up on the 4663 explorer (our address's transactions, by nonce) and say `eth_home_clear(<nonce>, true)` if it mined,
  `false` if another transaction of ours took the nonce. A wrong "not mined" pays that deposit or sweep twice.
- **`eth-home-out` (SEV2 at 30 min, SEV1 at 2 h, D6-L-2)**: a 4663 transaction signed that long ago and not resolved, whatever the
  providers show (a stale pool entry, providers that disagree). Nothing else is signed on 4663 meanwhile. Look at the transaction
  and at the canister's 4663 providers.
- **A written-off Relay order** holds the Relay leg for new orders until a payout of it is credited (`eth_credit_payout`) or the
  operator releases it (`eth_late_release(<buy>)`). Review round 6 (D6-L-3): `eth-late-hold` pages it SEV2, persistent, and SEV1
  after 72 h. **Release it only once Relay's own records show its ETH will never reach our addresses** (refunded elsewhere, or
  definitively lost), never to free the leg while a payout may still come: a payout that lands after the release is read by the next
  order's watch whatever is credited later (C6-I-2).
- **Crediting a late payout (review round 6, C6-L-1; round 7, C7-L-1):** a plain transfer to our address that names the order is
  credited at its value. A payout out of Relay's depository is its allocator-signed `execute(CallRequest, signature)`: the canister
  decodes the request from the transaction's input, recomputes its EIP-712 struct hash, and credits only the depository's own
  `RelayCallExecuted` records **with that id** paying our address. The id is never an order id, and a real `execute` carries only its
  calls, a random nonce, the expiry and the signature, so **nothing on chain names the order: the operator names it**, by handing in
  the transaction Relay's own records (`/requests/v3`; v2 is throttled and retires 2026-11-24) give for that order **and stating its
  amount** (`eth_credit_payout(<buy>, <chain>, <tx>, <wei>)`), which must be exactly one of that request's records paying us (with the
  other rules: after the deposit, at least half and at most the order's value). A call that merely names the order is refused. **This
  rule is pending C5-I-1's measurement below.** A claim handed in while that buy has a 4663 transaction out waits for it (C6-L-2); if
  the note says it waits and then that it is `stale` (600 s), hand it in again once that 4663 transaction has resolved (C7-I-2).
- **The sale scan (B6-M-1 / D6-I-1; round 7, B7-L-1 / B7-L-2 / B7-L-3):** `eth-scan-behind` (the monitor's own Ethereum final head
  less where the scan was last current: SEV2 at 2,400 blocks, 8 h; SEV1 at 7,200, a day; it grows while passes do not run and a
  re-anchor cannot hide it), `eth-scan-lost` (SEV2: every provider refuses the window's start as too deep), `eth-scan-deep` (SEV2:
  every provider refuses part of a re-anchor's hole as too deep; it is read again every 6 h, never given up on one answer, and one
  provider's "too deep" is never taken at all), `eth-scan-hole-day` (SEV2: a hole unread for a day), `eth-scan-busy-receipts` and
  `eth-scan-busy` (SEV2: a provider answers receipts, or the scan, as a rate limit for hours: a quota from the IC's addresses; busy
  receipts back off exponentially to 6 h and never hold newer sales), `eth-scan-hole` and `eth-scan-receipts` (SEV3, being read),
  and (review round 8, B8-L-1) `eth-scan-window-unread` (SEV2: the walk back to the window's start unmoved for 12 h while 600 blocks or
  more above it: typically ONE provider of the pair no longer serving logs that deep, which the canister never takes as "too deep" on
  one provider's word; run `python3 script/depth_probe.py` and replace the failing provider by an upgrade).
- **C5-I-1 measured live, 2026-10-09: all four PASS** (the operator's wallet `0x58d6…23ad`, the page `ops/rehearsal/relay/page.html`; each
  quote judged by `preJudgeEth` at the canister's 3 % before signing, each payout through the canister's `judge_payout` by `verify.mjs`):
  
  | Leg | Deposit | Payout | Paid back | Verdict |
  | --- | --- | --- | --- | --- |
  | 4663 → Ethereum, 0.002 ETH | `0xa520a177…4c96` (4663) | `0x6cc33d9f…5763` (Ethereum), solver `0x331d…2466` | 0.001985973 ETH | CREDITED, creditable as built |
  | Ethereum → 4663, 0.002 ETH | `0xb049dd5b…0918` (Ethereum) | `0x482e6f7a…8916` (4663), solver `0xf42b…acec` | 0.001990041 ETH | CREDITED, creditable as built |
  | Forced refund on 4663, 0.002 ETH | `0xdf33131d…d4f3` (4663) | `0x9c857c25…0473` (4663), `0xa5a5…a142` | 0.001999338 ETH | CREDITED, creditable as built |
  | Forced refund on Ethereum, 0.002 ETH | `0xaaed9cb3…c5cb` (Ethereum) | `0x5cc05357…2178` (Ethereum), `0xa5a5…a142` | 0.001996662 ETH | CREDITED, creditable as built |
  
  Every payout, fill or refund, is a plain ETH transfer to the depositor whose input is exactly the 32-byte order id: the shape the canister
  credits with no stated amount (`eth_credit_payout(<buy>, <chain>, <tx>, 0)`). No depository `execute` appeared. Measured on the way:
  Ethereum → 4663 at 0.001 ETH fails the 3 % rule (Relay's minimum 3.95 % under; 2.49 % at 0.002 ETH), so the canister cannot move a back leg
  that small. Relay's `/requests/v3` filters by `depositTxHash`, `fillTxHash`, `refundTxHash` or `user`, and silently ignores `hash` (v2's name,
  answering anyone's latest 20): `verify.mjs` now uses the v3 names, and each of the four found exactly its own request. **v2's retirement on
  2026-11-24 no longer blocks anything here, given a key.**
- **C5-I-1 answered read-only, 2026-09-28** (`docs/audit/2026-09-27-nft/relay-evidence-2026-09-28.md`): 35 real payouts run
  through the canister's own `judge_payout`. Fills and refunds in both directions are a plain ETH transfer from a Relay solver whose
  input is exactly the 32-byte order id; refunds pay the depositor on the order's origin chain. 19 of 20 refunds and 9 of 9 fills were
  credited with no amount stated (the one refusal: a refund under the at-least-half rule after gas). No depository `execute` paid ETH
  to a third party, no refund was a claimable withdrawal, and none was batched. **The canister's Ethereum and 4663 addresses must never
  carry code** (never sign an EIP-7702 authorization): a payee with code is paid through Relay's router, which the judge refuses. A live
  transfer is optional; it would only confirm the same shape for our own address. The original task follows.
- **Before launch (C5-I-1): measure one real Relay fill and one real refund in each direction** (4663 → Ethereum and back, a few
  thousand wei each is enough) and record their transactions: that the fill's calldata carries the request id or order id, where a
  refund pays from (the depository's `execute`, emitting `RelayCallExecuted`, or the solver), and at which chain. The payout judge
  (`ethflow::judge_payout`) accepts a transfer to us naming the order, or the depository's `execute` whose own execution records (id
  = the request's struct hash) pay us, named by the operator's stated amount (review round 7, C7-L-1). Record which it is. **If Relay
  refunds an unfilled deposit as a withdrawal the depositor must claim itself** (an `execute` sent by the payee, as the one non-solver
  payment sampled on Ethereum was), that refund can never reach the canister (it signs no such call, and the judge refuses our own
  transactions): say so here and treat such an order as lost to Relay's records until a reviewed signer path exists. If a real payout
  matches none of these, the judge must follow it before any order can be credited late.

---

## 6. Services, site and checks

- **Runner (`ops/src/ethnft.ts`)**. Reads `eth_nft_state()`. For each buy it:
  - `Funded` → quotes Relay for exactly the ask, pre-judges it (the TypeScript mirror of rules 1–9 and 11) and hands it in;
  - `Arrived` → fetches OpenSea's cheapest listings for the collection (`OPENSEA_API_KEY`, read at call time and never logged),
    fulfillment data for our address, pre-checks the bound, hands in the cheapest;
  - every 20 minutes → the floor evidence (≤ 8 listings, ≤ 12 recent sale tx hashes).

  Then it ticks. `RUNNER_ETH_NFT` (default on when `eth_nft_state` answers).
- **Monitor (`monitor/src/ethnft.mjs`)**:
  - **Custody:** every `EthNftWrapper.item` held, `ownerOf` = the custodian on **both** Ethereum providers; any other owner is SEV1.
  - **The floor:** the module's accepted floor against OpenSea's keyless `floor_price` and CoinGecko's. More than 10 % apart is
    SEV2; the canister's own fresh reading more than 20 % from BOTH (agreeing) references is SEV1, and the guardian **pauses that
    collection's PosterSource** (fix round 1, D-M-1; latched until acknowledged), which zeroes that poster only (§3.2).
  - **Buy states:** `Bridging` > 30 min SEV2, `Stuck` SEV1, `Funded` > 2 h SEV2, `Arrived` > 24 h SEV2.
  - **Deliveries:** a `Resolved` ticket undelivered 1 h after it resolved is SEV2; within a day of `deliverBy` it is SEV1.
  - **4663 events:** `Advanced` / `AdvanceSettled` / `AdvanceReturned` matched to an `EthBuy`; `FloorPosted` by the poster matched
    to a canister reading.
  - **Ethereum outbound canary:** every transaction from our Ethereum address must be a saved line entry.
- **Reconciler (`ops/src/reconcile.ts`)**. Adds the NFT collection: `held` vs `ownerOf` on both providers, the gas pot, and
  the transit unit. It is red on any mismatch, and red after 3 h stale.
- **Indexer / reserves page (`indexer/`, `web-rook/`)**. The NFT collection's ledger `E = C + R + T`; items held with token ids
  and "owned by the custodian on Ethereum (checked by two providers at block N)"; the Trust page's NFT section (§7).
- **Keeper**:
  - it skips floor posts for `kind: "nft"` collections;
  - its identity check expects `module.keeper()` = the stack's `floorPoster` there;
  - the keyless resolver reads the NFT ticket layout.
- **golive-check** (`--phase1` with an NFT stack). It checks:
  - the wrapper's custodian and chain and collection;
  - the module's keeper = poster = custodian;
  - `PosterSource.registry` = the stack's registry;
  - `wireFloorModule` done and `maxMintsPerDay` = the stage's;
  - with `--canister`, `eth_nft_config()` naming the same collection, wrapper and module, and the custody cap at stage 0.
- **Dry run** (`scripts/golive-dryrun.sh stack`): unchanged for the Bitcoin stack. The NFT stack is covered by forge tests (the
  full graduation → advance → mint → settle → redeem → deliver path against `EthNftWrapper`) and the Ethereum fork test (§8).

---

## 7. Trust and threat model

**New trust, in one sentence.** The canister's threshold key now holds NFTs on Ethereum at rest. Relay holds an advance for
seconds. The floor is the canister's reading of verified listings and sales, fenced by the contract.

| Threat | What an attacker can do | What stops it |
| --- | --- | --- |
| **Marketplace order manipulation** (a listing crafted to pass as cheap) | Hand in an order with a wrong token, extra recipients, a tip, a hidden fee, a far future `startTime`, a contract-wallet signature, a restricted zone | The judge recomputes everything from the order: the token and collection, total ≤ bound, recipients (fee list or 2981 receiver only), no tips, times vs the chain's clock, order hash and status, owner, signature, and **the exact transaction simulated from us at the pin**. The canister signs its own calldata only |
| **Fake floor evidence** | Hand in listings that do not exist | A listing counts only if its signature, status, owner and approval check out at the pin. A sale counts only if its receipt says so |
| **Wash floors (upward)** | Wash-trade high on Blur (gas only); withhold cheap listings | The post is always a verified listing; sales only veto it, and self-trades, repeats of one wallet and far prices are not counted (fix round 1, B-H-1). Withholding cheap listings lifts it at most `up_cap` (10 %) over the median of the sales handed in, then ±5 %/h and ±20 %/day. The monitor compares the canister's reading against OpenSea and CoinGecko and pauses that collection's poster. **Burns cannot pay more than a unit's backing**, and a deposit is minted only while the module's floor is at most 5 % over the verified sales' median (A-M-1): the review found a high floor WAS paid out through deposit then burn (2026-09-27, A-M-1), now bounded by that band |
| **Wash floors (downward)** | List own NFTs cheaply to cheapen public mints | Such a listing is real, and buyable by anyone, the canister included. The floor is the `listing_rank`-th cheapest across distinct offerers, bounded below at `S25 × 0.85` and by ±5 %/h and ±20 %/day. Public mints are capped at 10 items (and 5 an hour). The residual is the documented keeper-bound loss (`FloorModule` @dev), now with the canister as the poster |
| **Royalty rules / operator filters** | A collection blocks marketplace conduits or owner transfers | Measured: none of the shortlist has a transfer validator or an operator filter (§1.1). OpenSea's operator filter never blocks an owner's own `transferFrom`. Every buy and every transfer is **simulated** at the pin before signing, so a blocked path is refused, never paid for. A delivery that cannot simulate is marked undeliverable (a refund, not a loss) |
| **NFT transfer hooks** | A malicious collection or recipient re-enters | Deliveries use `transferFrom` (no `onERC721Received` call). Buys go through Seaport to our EOA, which has no code, so no hook runs for us. Only allow-listed collections are ever touched. Reentrancy on 4663 is unchanged (the wrapper and module are `nonReentrant` or checks-effects) |
| **Reorgs** | Undo a purchase, a delivery or a deposit | Nothing mints or records on 4663 until the Ethereum block is ≤ the lower `finalized` of both providers. Before that, a vanished receipt just waits. 4663 transactions: the existing saved-bytes resend |
| **Relay** | Keep an advance (fail to fill and to refund); fill late | One advance at a time (≤ floor + 3 %), three a day. The judge recomputes the order hash: our recipient and refunds, native ETH on chain 1, min-out ≥ 97 %, pinned depository and allocator (the same `0x4cd0…bc31` and `0x63C1…1b56` on both chains [M]). Stuck → SEV1; recovery as `relay-float` §5.3 |
| **Canister Ethereum signing** | A compromised canister (upgrade through the 72 h timelock), a malicious operator | The operator cannot name a recipient, a price over the bound, a calldata, or a collection. It can pick among valid listings under the bound, delay, or withhold. A malicious **upgrade** can move every NFT: the timelock's 72 h, the published argument, the guardians' veto, and custody caps of 3 items and 10 ETH per collection bound it — the same trust as Bitcoin custody |
| **Provider disagreement or lies** | One Ethereum RPC lies | Every read is replicated on 13 nodes and must agree on both providers, through a transform that re-emits each answer in one canonical shape (fix round 1, B-M-6: measured, `1rpc.io/eth` answered in varying shapes; `xrpc.cl/eth` and `eth.drpc.org` answered identically 13/13 and are the recommended pair). Disagreement is "nothing believed", and a failed or disagreeing read is a wait, never a decision: a transfer is "cannot succeed" only on an agreed revert from every provider for an hour (B-M-2). The pinned block is agreed by hash |
| **Gas spikes on Ethereum** | Deliveries wait | Deliveries have 7 days plus grace. The fee cap is 50 gwei (the measured max over 1,024 blocks is 1.5 gwei). Pending at `deliverBy − 1 day` → cancel → `markUndeliverable` → refund after 72 h. The redeemer gets their unit back, never both |
| **The gas pot runs dry** | Deliveries wait | Each purchase leaves `arrived − price` (≥ the 0.003 ETH reserve) at our address. The operator seeds 0.01 ETH once (§9, D3). The monitor pages under 0.005 ETH |
| **Stray NFTs / dust sent to our address** | Pollute custody, trigger "unexplained" alerts, drain the gas pot with returns | Never minted without a valid claim. A configured collection's stray goes back to its sender once final, one at a time, within a daily budget that never trips the freeze, and never under the gas kept for deliveries; anything else is held and listed. Another collection's NFT is never touched. Custody counts only what the wrapper holds |
| **Holder deposits** (on at launch) | Fake a deposit; replay a deposit transaction; front-run a claim; deposit then race a redemption; mint twice | The claim is the holder's own signed transaction (no claim to front-run; the recipient, chain and wrapper are in its data); read at the pin on both providers and again past finality; `(tokenId, tx)` mints once and a held token never mints (wrapper); a new item is drawable only by tickets burned after it; a claim of our own transaction is refused |
| **Collection admin powers** | An upgradeable or admin-movable collection takes the NFT back | The shortlist has no proxy (EIP-1967 slot empty [M]). Adding a collection needs the same checks (golive-check reads them) |

**What does not change:** the pool LP is never withdrawable; redemption is never pausable (only its custody side, through the
canister's pause); the module has no owner; stage-0 caps bound custody, not the 4663 contracts (`phase1-scope.md` §0).

---

## 8. Relay, measured [M]

- `/chains`: Ethereum's `protocol.v2.depository` is `0x4cd00e387622c35bddb9b4c962c136462338bc31`, the same address as on 4663.
  Its `allocator()` on Ethereum is `0x63C1d3E9C646184529C5694630a01C00dF171b56`, the same as on 4663. Its owner is `0xF61A…775A`,
  again the same. It has no EIP-1967 implementation. Its code hash on Ethereum is `0x27fe…799e`, and on 4663 `0xc8b9…4b07`
  (per-chain immutables).
- **Quote 0.5 ETH, 4663 → 1** (fixture `tests/fixtures/relay-quote-4663-1-2026-09-27.json`):
  - one `deposit` step, `depositNative(user, orderId)` to the depository, `chainId` 4663;
  - `orderData.output.chainId = "ethereum"`, one payment to our address in native ETH, no calls, no fees;
  - refunds on `robinhood` and `ethereum` to our address;
  - expected 0.49844 ETH (−0.31 %), minimum 0.48848 (−2.30 %), deadline = quote + 7 days.
- **Quote 0.2 ETH, 1 → 4663** (the reverse, for D4): the same depository on Ethereum, one deposit step, expected −0.06 %.
- **Ethereum gas** over 1,024 blocks: base fee median 0.068 gwei, p90 0.083, max 1.535; tip median 0.036 gwei.
  A Seaport basic fill used **183,795 gas** (Milady #9180, block 26,055,691), which is ≈ 0.00002 ETH at today's fees.

---

## 9. Decisions for the operator (decided 2026-09-27)

The operator accepted every recommendation below on **2026-09-27**, with two changes (D1 and D5). The launch configuration is
this section; `golive-check.sh` holds D1, D5 and D7 (with `--canister`), and the monitor refuses an `eth_nft` configuration
without the guardian of D8.

- **D1. The collections — DECIDED, changed by the operator: several at once.** The pilot launches with **Milady, Azuki and
  Mutant Ape Yacht Club** together (`0x5af0…25a5`, `0xed5a…c544`, `0x60e4…a7c6`), one `EthNftWrapper`, `PosterSource` and
  `FloorModule` each, in one NFT-only stack (`ETCH_NFT_COLLECTIONS`, §11.5). **Pudgy Penguins and BAYC** follow through
  per-collection graduation targets and pooled graduations (branch `nft-pool`, a later delta); until then golive-check's
  `--phase1` read refuses them in the stack. (The recommendation had been Milady alone first.) Nothing in the code is
  Milady-specific: a collection is configuration (§1.3); the stage-0 custody caps are 3 items and 10 ETH per collection and **6
  items and 10 ETH in all**, which three collections share. *(Overtaken: Pudgy Penguins and BAYC launch too (the pooled delta,
  J1), and J7 sets the custody caps: 15 ETH per Ethereum collection, the global the sum; `bayc10-decisions.md` §4.)* *(BAYC's parameters, J1 of `phase1-nft-pooled.md`, decided 2026-09-27: 10 ETH, `convBps` 1,000,
  `reserveBps` 0, one graduation per ape with +8.26 % of floor-rise headroom from 6.79; §1.1 there has every collection's
  margin.)*
- **D2. Redemption order — DECIDED: drand-random.**
- **D3. Gas pot on Ethereum — DECIDED:** seed ≈ 0.01 ETH once at the canister's Ethereum address; purchases' leftovers top it
  up. The monitor pages under 0.005 ETH. With deposits on, stray returns never spend below 0.005 ETH over their own gas (§2.4).
  **Amended by the operator, 2026-09-27 (A2-L-1):** the leftovers top the pot up only to its target, 0.05 ETH
  (`eth_nft.gas_pot_target_wei`, within [0.01, 1] ETH). Each purchase's leftover is booked to its collection; above the target,
  and only while no buy, delivery, return or line transaction holds ETH at the address, it goes back through the way back (Relay
  Ethereum → 4663, `relayeth::judge`) to that collection's own module (`fundReserve`: its surplus, which tops up its conversions and
  never pays a burn). And a buy's advance is only what the buy may cost (the floor + 5 % and the fees), never the whole item cap
  (`docs/v7/phase1-nft-pooled.md` §1.1).
- **D4. Stranded ETH on Ethereum — DECIDED: wait, page at 24 h.** The reverse Relay leg stays unwired (a reviewed upgrade).
  *(Overtaken: the reverse leg is wired (fix round 2) and the return is automatic after 48 h (the operator's yes, below).)*
  *(Fix round 2 of the 2026-09-27 review wires it, as its brief requires (A2-M-1, C2-M-2): the wait and the 24 h page are
  unchanged; after 48 h, or at once by `eth_give_back`, the ETH goes back through Relay to its module. The 48 h bound is the
  operator's to confirm: `fixes-round2.md` §7.)*
  **Operator's yes, 2026-09-27: automatic.** ETH stuck on Ethereum goes back automatically after 48 h `Arrived`, through the
  reverse Relay leg built in round 2 (`ARRIVED_BACK_SECS`, then `returnAdvance` to its module). `eth_give_back` still does it at
  once, and the 24 h page is kept. This confirms the 48 h bound `fixes-round2.md` §7 left open, with no code change.
- **D5. Holder deposits — DECIDED, changed by the operator: ON at launch** (`eth_nft.deposits = true`). Because they are live,
  launch adds (§2.4, §11.5): automatic discovery by the runner with a monitor watchdog; returns of strays of a configured
  collection to their sender; a claim format naming the chain and the wrapper; no permanent refusal on a transient read; no
  custody room → returned; a deposit waiting on a limit never holds up the floor. **No deposit fee** (a contract change; the
  depositor pays only Ethereum gas). The operator's judgment calls taken conservatively are listed in §11.5.
- **D6. The OpenSea key — DECIDED:** the runner holds one. **The Ethereum provider — DECIDED by the operator, 2026-09-27:** Dwellir
  (the operator's paid plan, `https://api-ethereum-mainnet.n.dwellir.com/<key>`), replacing `eth.drpc.org` (its free tier refuses
  the scans, review round 3 D3-L-3), for the monitor (first of its pair, `xrpc.cl/eth` second) and the runner (`ETH_LOGS_RPC`).
  Its plan serves `eth_getLogs` over at most 500 blocks a request, so every scan asks at most that (`logs_max_blocks`,
  `ETH_LOGS_MAX_BLOCKS`). The key lives in the services' environment only (`${DWELLIR_API_KEY}` in the monitor's configuration,
  scrubbed from its alerts and status); the canister keeps a keyless pair, since its install argument is published with every
  timelock proposal (review round 6: MEV Blocker + Tenderly's public gateway, §4.1). Depth measured by the operator on 2026-09-27 (~15:50 UTC, OpenSea
  best listings, ETH/WETH): Milady floor 0.930, within +2 % 2 listings (2 sellers), +5 % 3 (3), +10 % 9 (6); Azuki 0.670: 10 (4),
  14 (4), 14 (4); MAYC 0.8735: 2 (2), 2 (2), 5 (4); BAYC 6.79: +5 % 5 (3); Pudgy 3.353: +5 % 7 (4). The ≥ 5 listings within 5 % rule
  is met by Azuki only (and BAYC, Pudgy): the review (`docs/audit/2026-09-27-nft/`) reports what thin depth means for buys.
- **D7. Floor parameters — DECIDED: the defaults** (rank 2 of ≥ 3 listings, ≥ 3 sales, +10 % / −15 %): `eth_nft.floor = null`.
  *(Fix round 2: the numbers are unchanged; the +10 % is over the counted sales' 25th percentile (was the median), and neither
  bound stops a post any more: over it the post is held, under it the post is flagged, §4.2.)*
- **D8. The guardian's floor pause — DECIDED: yes** (as built after fix round 1 of the 2026-09-27 review, D-M-1: the guardian pauses
  that collection's own `PosterSource`, not the registry; it fires on the canister's own fresh reading, not the lagging accepted
  floor; both references must be read and agree within 10 %; one pause per collection until the operator acknowledges it). The
  original text: the monitor pauses the keeper (zeroing the poster) when the posted floor is
  > 20 % from both references; an `eth_nft` monitor configuration without an armed guardian is refused (`guardian_pause = false`
  only for a rehearsal). The pause is registry-wide: it stops every collection's poster at once.
  **Per-collection exception (the operator, 2026-10-09):** OpenSea ALONE is acceptable as **Identity MD**'s reference (CoinGecko does
  not list it). The launch file says so on that collection only (`"priceRefs": ["opensea"]` with a `_priceRefs` note, no
  `coingeckoId`), its monitor entry names `price_refs: ["opensea"]` and no `coingecko_id`, and the pause then fires at more than 20 %
  from OpenSea (same side); a missing OpenSea answer pages ("the floor is not watched") and never pauses by itself. Every other
  collection still needs both references agreeing within 10 %. The monitor's loader refuses `price_refs` in any other shape or where
  the launch file does not decide it; `golive-check --launch` refuses a missing `coingeckoId` without `priceRefs`, and
  `--monitor-config` accepts OpenSea alone only where the launch file says so (`script/d8refs.py`).
- **D9. Blur — DECIDED: accepted** (Blur listings are not bought; Blur sales still count as evidence).

## 10. What needs a security review

1. `EthNftWrapper` (a new contract, though its logic is `Wrapper`'s) and `PosterSource`.
2. The canister's new code: `relayeth` (the chain-1 judge), `ethnft` (floor evidence parsing, the buy bound, the transfer and
   deposit judges), `ethflow` (the state machine, especially `markUndeliverable`'s conditions and the cancel), and `ethnet` (the
   finality reads).
3. The canister as floor poster: the replacement of the keeper's word by the canister's reading, and the parameters of §4.2.
4. `DeployStack`'s NFT-only mode and golive-check's NFT checks.
5. Holder deposits, now on at launch (D5): the claim judge, the returns of strays, the runner's discovery, the monitor's watchdog.
6. Several collections at once (D1): what they share (the canister's Ethereum address and lines, the global caps, the pause).

---

## 11. As built (2026-09-27)

What the code does, where it differs from the text above, and what was run. Nothing was deployed, pushed or signed with a real
key; the only chains written to were local anvils.

### 11.1 Contracts

- `src/nft/EthNftWrapper.sol` (11,575 bytes of runtime) is §3.1 exactly. It is `Wrapper.sol`'s ticket machine, keyed by token id:
  - no `redraw`;
  - `PIN_TIMEOUT` refunds a pinned ticket (itemId = `type(uint256).max` in the event);
  - `markUndeliverable`, `REFUND_GRACE` (72 h) and `LATE_DELIVERY_WINDOW` (30 days), on both clocks;
  - B-05 in `wireFloorModule`;
  - the listing's pause gates `mint` and `mintDeposit`;
  - H11;
  - no rentals (`NoRentals`).
- `src/floor/PosterSource.sol` (1,375 bytes) is §3.2 exactly. `FloorModule` is unchanged.
- **Tests:**
  - `test/ethnft/` holds the wrapper's unit tests (every refusal, both clocks, the fuzz of deliveries and refunds), the poster, and
    the 4663 side of a whole buy: advance → mint to the module → `settleAdvance` → redeem → resolve → record. It also covers
    return, the pause and one advance at a time.
  - An invariant suite (the books, `C + R + T = E`, `held − delivered`, an item for every open ticket) runs at a 25 % reserve.
  - `test/StackChecks.t.sol` covers the NFT-only plan.
- **The fork proof:** `test/fork/EthNftSeaport.fork.t.sol` runs on an anvil fork of Ethereum at block 26,055,690. It takes the
  canister's own calldata for the real Milady #9180 listing. `ethnft::tests` writes that calldata and checks it byte for byte in
  `test/fixtures/ethnft/milady-9180-canister-fill.json`.
  - The calldata, sent from the canister's address with exactly 0.999899999 ETH, **puts #9180 at the canister's address**.
  - The canister's `transferFrom` then delivers it.
  - One wei less, another token id, and an underpayment each revert. **2/2 passed.**
  - Run it with: `anvil --fork-url <archive rpc> --fork-block-number 26055690 --port 8642`, then
    `forge test --match-path test/fork/EthNftSeaport.fork.t.sol --fork-url http://127.0.0.1:8642`.

### 11.2 Canister

**Modules:**

- `relayeth.rs`: the `relay-float` judge with the destination as a parameter (`TO_ETHEREUM`; `FROM_ETHEREUM` is written and
  tested, not wired). Tested on the real 0.5 ETH quote of 2026-09-27 and the real reverse quote, with every rule refusing its lie,
  including consistent re-hashed hostile orders.
- `ethnft.rs`: the configuration and its validation; listing terms and judge; sales from real receipts (Seaport 1.5/1.6
  `OrderFulfilled`, listing and bid sides, and Blur v2 `Execution721*Packed`, whose packing was confirmed on real receipts); the
  floor; the posting cadence; the buy cap; the cost share; the wrapper's ABI; the transfer and deposit judges.
- `ethflow.rs`: the flow, with `EthIo`.
- `ethnet.rs`: the real IO, three transforms, and the endpoints.

**Hooks:**

- `Init.eth_nft`, its validation, and `restore_init`.
- The thirteenth saved element. An upgrade that drops or moves the chain while the Ethereum line holds a transaction is refused.
- The default custody caps now list each `eth_nft` collection at stage 0.
- The tick runs the side after the frog side, sharing "one home-chain transaction at a time" both ways.
- `nft_sign` refuses the `eth_nft` chain.
- The `.did` is updated.

**Where the code differs from, or adds to, the text:**

- A delivery whose transfer **reverted**, or whose **cancel** mined, is marked undeliverable only once that block is final
  (`MarkAfter`). A delivery that never had a transaction signed (the simulation refuses, a relist ticket, or inside the deadline's
  last day) is marked at once.
- One Ethereum buy is open at a time **canister-wide**, not per collection: arrival is read from our one Ethereum balance, so two
  bridging buys could not be told apart.
- The arrival watch adds back what our own mined Ethereum transactions spent since the deposit (`eth_out`), so a delivery's gas
  cannot hide an arrival.
- A buy whose nonce was taken by something that is not ours goes `Stuck` and nothing more is signed on Ethereum until someone
  looks.
- **Not built:**
  - an automatic replacement of an under-priced fill (the monitor pages a fill unmined for an hour);
  - the reverse Relay leg (D4);
  - discovery of deposits (claims come from the runner or the operator, `eth_deposit`). **Built since (§11.5, D5).**
- **Tests:**
  - `tests/ethnft.rs` runs 14 end-to-end flows against a fake world, with the real Relay quote and the real sale receipts:
    - the whole buy through settle;
    - over-cap, no-reading and stale listings refused;
    - a tampered quote and a rotated allocator refused;
    - refunds retried, then returned;
    - stuck, then a late arrival;
    - delivered, final and recorded;
    - cancel then mark once final;
    - the transfer winning the race with its cancel;
    - an impossible transfer never sent and marked;
    - a relist ticket never delivered;
    - the floor posted only as the module's keeper and only when due;
    - deposits off, then on;
    - one home transaction at a time, and nothing signed while paused;
    - a buy dropped when the module no longer wants it.
  - Guard tests: every `ethnet` update is operator-only, refuses while paused, and refuses mid-tick.

### 11.3 Services, deploy and site

- **Runner** (`ops/src/ethnft.ts`): Relay quotes pre-judged by rules 1–11 (`relay.ts`'s encoder now accepts `ethereum`); OpenSea
  listings to fulfillment calldata (basic, or advanced with the zone's data); protocol data to evidence calldata; recent Transfer
  transactions as sales. New variables: `RUNNER_ETH_NFT`, `ETH_NFT_SLUGS`, `OPENSEA_API_KEY` (read at call time) and
  `ETH_LOGS_RPC` (`docs/deploy/services-v1.md`).
- **Monitor** (`monitor/src/ethnft.mjs`):
  - buy and delivery states;
  - custody by `ownerOf` on both Ethereum providers at one block;
  - the gas pot;
  - the floor against OpenSea and CoinGecko, with the guardian's pause at 20 %;
  - the watcher decodes and words `eth_nft` in upgrade arguments;
  - an Ethereum buy backs its module's `Advanced`/`AdvanceSettled` events, so there is no false SEV1.
- **Keeper:** never posts an nft module's floor. Its identity check expects the module's keeper to be the stack's `floorPoster`.
  The keyless resolver reads `EthNftWrapper`'s ticket.
- **Indexer and reconciler:** `EthNftWrapper`'s `Undeliverable` and gated refunds. Custody reconciliation was already `ownerOf`
  on two providers (`eth.drpc.org`, `xrpc.cl/eth`: a pair independent of the canister's).
- **Site:** the Trust page's "Ethereum NFTs" section.
- **Deploy** (`DeployStack`, `ETCH_STACK_KIND=nft`): the `EthNftWrapper` and `PosterSource`, the stack file's `floorPoster` /
  `posterSource`, and `forKeeper` without nft modules.
- **golive-check:** the poster source, the module's keeper = the custodian, the wrapper's registry and listing, the collection
  another chain's, the canister's cap under `1:0x<collection>`, `eth_nft_config()` with `--canister`, and no fee-pool requirement
  without a Bitcoin collection.

### 11.4 Results (2026-09-27, this branch)

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,088 passed**, 0 failed (124 suites) |
| The Ethereum fork proof | 2/2 passed |
| Canister `cargo test` | **536 passed**, 0 failed (349 unit + 187 integration, 14 of them `tests/ethnft.rs`) |
| Timelock | 15 passed |
| ops | 44 |
| monitor | 150 |
| keeper | 150 |
| indexer | 135 |
| web-rook | 70 |

All of the service suites pass.

**`scripts/golive-dryrun.sh stack`: DRY RUN: ALL PASS.** It includes the new NFT-only phase:
- the stack deployed and read back (148 golive checks ok);
- the canister's post landing;
- the keeper EOA refused;
- the registry's pause zeroing the poster and refusing the canister.

**Reproducible build**, twice, the second with `--no-cache`, identical:
- `buyer.wasm` `0f9ee000192bcec849628919e33b321bf9849f61ab2257934fba1292c7aa29fe` (6,182,319 bytes; 1,176,204 gzipped). This
  replaces `relay-float`'s hash for any install of this branch.
- `timelock.wasm` `b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41` (unchanged).
- `timelock-rehearsal.wasm` `2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4` (unchanged).

### 11.5 The launch decisions as built (2026-09-27): D5 holder deposits on, D1 several collections

**D5 (commit "D5: holder deposits on at launch").** Holder deposits are live launch scope, so the two known gaps were closed
before go-live rather than documented:

- **(a) Discovery: built, in the runner, watched by the monitor.** `ops/src/ethnft.ts` `inboundTransferTxs` reads every
  ERC-721 `Transfer(*, canister, *)` of each configured collection in the last 7,200 blocks (2,400-block chunks on
  `ETH_LOGS_RPC`), skips what the canister has judged and our own fills, and hands each in with `eth_deposit` (again after 7 h if
  the canister dropped it unread). The canister trusts nothing in it. The monitor (`unclaimedAlerts`) reads the same logs on its
  own Ethereum provider and pages (SEV2) any inbound transfer not judged two hours on. Discovery was **not** moved into the
  canister (no new outcall surface, no `eth_getLogs` consensus to build): the runner can only withhold, and withholding pages.
- **(b) Strays: a return path, for configured collections.** `ethnft::judge_claim` classifies every transfer to us in a claimed
  transaction: a valid deposit (the 48-byte data naming chain 4663, this collection's wrapper and a recipient) is minted past
  finality; anything else of a configured collection (no data, garbled data, another chain or wrapper, a plain `transferFrom`,
  an approved operator's or a contract's move) is **returned to its own `Transfer`'s `from`** once final, by `transferFrom` on the
  Ethereum line (`EthKind::Return`), after deliveries and the buy, one at a time, within the day's `Flow::Return` budget (3 per
  collection, 6 in all; it never trips the freeze), and only while our balance less an arrived buy's ETH stays 0.005 ETH above
  the return's gas. `ethnft::judge_return` refuses a token the wrapper holds (`itemOf ≠ 0`), one an open buy or claim uses, and
  one that left us (a delivery or a return, `EthState.outbound`) after the claimed transfer came in. Such a stray is `Held` (SEV2,
  the operator looks). **Another collection's NFT is never touched.**
- **Also fixed on the way:** a claim that cannot be read yet (not mined, above the pin, a provider error) is tried again for 6 h
  and then dropped unrecorded (it was refused for good); a deposit waiting on H2/H3/H11 no longer returns an error that stopped
  the floor post (one deposit mint asked at a time, so a backlog cannot inflate the tripwire's demand); no deposit is minted while
  a purchase is between its fill and its mint; after finality the receipt is read again (a reorg before finality moves it) and
  `itemOf` is checked; no custody room returns the deposit; a claim of our own transaction is refused; the site's deposit text.

**The operator's judgment calls, taken conservatively (for the operator to confirm or change):**

1. The claim data **names the chain and the wrapper** (docs/v4/nft-wrapper.md §4's 48-byte form), not the recipient alone; the
   old 36-byte form is returned, not minted.
2. **No custody room → the NFT goes back** (rather than waiting unminted for room).
3. **An NFT of an unconfigured collection is never touched** (no call to an unknown contract from the custody key): it stays,
   listed, the sender's loss. A stray that cannot be judged or simulated is **held**, never guessed.
4. Returns spend the gas pot only above **0.005 ETH** plus their own gas, and only after deliveries and the buy; at most 3 a day
   per collection (6 in all).
5. **Discovery lives in the runner**, with the monitor as the watchdog; `eth_deposit` stays operator-only (a public claim
   endpoint would let anyone spend the canister's cycles).
6. **No deposit fee** (a contract change). A deposit mints only one at a time and never ahead of a purchase's mint.
7. Returns go to the `Transfer`'s `from` (the owner), not the transaction's sender.

**D1 (commit "Launch configuration").** `DeployStack` deploys several Ethereum collections in one NFT-only stack
(`ETCH_NFT_COLLECTIONS`, `ETCH_NFT_NAMES`, `ETCH_NFT_SYMBOLS`, `ETCH_NFT_SLUGS`; `StackChecks.checkNftSet` refuses a collection
named twice, a zero address or lists of different lengths). `golive-check.sh` holds the launch decisions: each NFT collection is
in the operator's launch list (Milady, Azuki, MAYC; Pudgy Penguins and BAYC are refused under `--phase1` until `nft-pool`), and
with `--canister` the canister's `eth_nft_config()` has deposits on (D5), the D7 floor rule, Relay's 300 bps, and exactly the
stack's collections. The monitor refuses an `eth_nft` configuration without an armed guardian (D8). The dry run deploys the three
launch collections, reads them back, checks the canister's configuration through a stand-in `dfx` (and that deposits off, a
rank-1 floor and an extra collection are each refused), posts each module's floor as the canister, and shows the registry's
pause zeroing every poster at once.

### 11.6 Results after D5 and D1 (2026-09-27, this branch)

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,089 passed**, 0 failed (124 suites) |
| The Ethereum fork proof (anvil fork of block 26,055,690) | 2/2 passed |
| Canister `cargo test` | **544 passed**, 0 failed (351 unit + 193 integration, 20 of them `tests/ethnft.rs`) |
| Timelock | 15 passed |
| ops | 45 |
| monitor | 153 |
| keeper | 150 |
| indexer | 135 |
| web-rook | 70 |

**`scripts/golive-dryrun.sh stack`: DRY RUN: ALL PASS**, with the three-collection NFT stack (325 golive checks ok on it) and
the `--canister` refusals of §11.5.

**Reproducible build**, twice, the second with `--no-cache`, identical:
- `buyer.wasm` `d96899775d9bfd8189a30496e374e6eb53a9927b1e152e217e45dff39b7a821a` (6,246,208 bytes; 1,200,874 gzipped). This
  replaces §11.4's hash for any install of this branch.
- `timelock.wasm` `b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41` (unchanged).
- `timelock-rehearsal.wasm` `2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4` (unchanged).

The security review of this branch with deposits on is `docs/audit/2026-09-27-nft/`.

### 11.7 Fix round 1 of the 2026-09-27 review (2026-09-27, this branch)

`docs/audit/2026-09-27-nft/fixes-round1.md` maps every finding to its fix and test. In short: the floor is the verified rank-2
listing and sales only veto it (B-H-1), the runner hands in sales (D-H-1), the canister posts inside `floorBounds()` and the
monitor pauses one collection's `PosterSource`, on the canister's own reading and two agreeing references, latched (D-H-2,
D-M-1); no refusal holds up a floor post (B-H-2 / C-H-1); failed or disagreeing reads are waits (B-M-1…B-M-6); the evidence
pass costs about a tenth of what it did (B-M-7); `redeemToRelist` is refused (A-M-2); deposits mint only while the floor is
within 5 % of the verified sales (A-M-1); the canary and the Ethereum outbound canary read the NFT side (D-M-2, D-M-3); the
earlier review's H9, R3 paused pass, L-2, L-5, W2 precondition and M-2 residual are carried over. Results and the new
`buyer.wasm` hash are in that document's §9; it replaces §11.6's and `phase1-nft-pooled.md` §7's.

### 11.8 Fix round 2 of the 2026-09-27 review (2026-09-27, this branch)

`docs/audit/2026-09-27-nft/fixes-round2.md` maps every round-2 finding to its fix and test. In short: the canister reads each
collection's sales itself (its own `eth_getLogs`, final blocks, each receipt once) and counts one sale per token and per wallet,
none of a round-tripped token; sales can hold the floor back but never stop a post, and the band is the counted sales' lower quarter
+ 10 % (A-M-1, B2-M-1 = D2-M-2, B2-L-1, B2-L-2); a deposit mints only within 5 % of the lower of the listing floor and that quarter;
no advance is made that cannot pay the floor on Ethereum, and an arrived buy that cannot fill goes back through Relay to its module
after 48 h, holding its collection's deposits 6 h at most (A2-M-1, C2-M-2); no arrival watch starts while the line's outcome is
unread, and earmarks never exceed the balance (C2-M-1, C2-L-1); the monitor no longer pages honest fills and redemptions (D2-M-1);
upgrades that strand a collection's work or lose a nested option are refused (C2-L-2, C2-L-3); golive-check holds the launch file
and the launch monitor configuration (D2-L-5). Results and the new `buyer.wasm` hash are in that document's §11; they replace §11.7's.

### 11.9 The operator's decisions of 2026-09-27, as built (branch `nft-bayc10`)

`docs/audit/2026-09-27-nft/bayc10-decisions.md` maps each decision to its change, tests and results, and lists the diff for the
round-3 review. In short: BAYC graduates at 10 ETH with `convBps` 1,000 and `reserveBps` 0 (J1: one graduation per ape, +8.26 %
floor-rise headroom, `phase1-nft-pooled.md` §1.1); D4's automatic return after 48 h is confirmed (§9); J7's custody caps (15 ETH
per Ethereum collection, no binding total, deposits at the floor) and H3's 25 ETH a day are stage 0; a buy's advance is its bound
and each collection's leftover is swept back to its own module (A2-L-1, D3 amended); the Ethereum provider is Dwellir for the
services, with every `eth_getLogs` at most 500 blocks (D6). The canister changed: its new `buyer.wasm` hash is in that document.

### 11.10 Fix round 3 of the 2026-09-27 review (2026-09-27, this branch)

`docs/audit/2026-09-27-nft/fixes-round3.md` maps every round-3 finding to its fix and test. In short: a delivery is recorded from its
own receipt at finality, whoever owns the token afterwards, and the wrapper's late window no longer refunds a delivery the canister
noted sent (`noteSent`, a contract change) (D3-H-1); an advance waiting on H3 keeps one key, so waiting never trips the freeze
(C3-H-1); the sale scan reads one move per token and per sender first, oldest first, reads again what a full queue dropped, and never
evicts a sale for a non-sale (B3-M-1); the way back has bounded exits and an operator path, and one collection's stuck return no
longer stops the others (C3-M-1); every balance read of the flow has the pin floor (E3-M-1). The canister's install argument must name
keyless providers (golive-check refuses a keyed URL; the monitor's argument diff prints no URL path). Results and the new
`buyer.wasm` hash are in that document; they replace §11.9's.

### 11.13 Fix round 6 of the 2026-09-27 review (2026-09-28, this branch)

`docs/audit/2026-09-27-nft/fixes-round6.md` maps every round-6 finding to its fix and test. In short: the canister's Ethereum pair
is `rpc.mevblocker.io` + `gateway.tenderly.co/public/mainnet` (both measured deep: §4.1), and golive-check refuses a provider not
measured deep (or failing `--probe-depth`). Whatever the pair, the sale scan no longer freezes after a gap: the transforms keep the
class of a provider's refusal (a rate limit, a history not kept, a result cap), a forward walk a day behind or refused as too deep
re-anchors at the newest blocks and reads what it passed over newest first, what the providers cannot serve is recorded and paged,
a rate limit is retried and never taken as too big, a block is remembered as read only when both marketplaces answered and a skipped
block is never given up, and an unreadable receipt no longer holds the newer sales (B6-M-1, B6-L-1, B6-L-2, B5-L-2). The 4663 line:
a receipt the providers disagree on is decided as "seen nowhere" is (F6-L-2), `eth_home_clear` applies only to the steps it can
(D6-L-1), a home transaction out too long pages (D6-L-2), and the runner ticks sooner while one is out (F6-L-1). The payout judge
credits a depository call only on Relay's own execution record (C6-L-1); a credit waits for the buy's pending 4663 transaction
(C6-L-2); a written-off forward order keeps its custody room (C6-L-3); the late hold pages persistently (D6-L-3). Results and the new
`buyer.wasm` hash are in that document; they replace §11.12's.

### 11.12 Fix round 5 of the 2026-09-27 review (2026-09-28, this branch)

`docs/audit/2026-09-27-nft/fixes-round5.md` maps every round-5 finding to its fix and test. In short: a 4663 transaction of ours
whose nonce is used and that no provider shows is never done again on that absence alone: the canister identifies what used the
nonce (our nonce count at past blocks and that block's transactions) or reads the step's own effect back (the module's advance,
the ticket's stage, `sentTx`, `undeliverable`, `itemOf`); a Relay deposit or a sweep's `fundReserve`, which leave nothing to read,
wait and page until the chain shows them or the operator says (`eth_home_clear`); a provider that shows it restarts the clock
(F5-M-1). A written-off Relay order keeps the Relay leg until a payout of it is credited or the operator releases it
(`eth_late_release`), and a payout is credited only by a transaction that names the order, is not ours and mined after the order
(C5-M-1, C5-L-1). The sale scan never lets a range too big to read (the IC's size reject, a provider's own limit error, or an
error repeated at one start) hold a walk: the range's sales are read through the marketplaces' logs in one read and the rest later,
and the pass's and the day's response bytes are real bounds (B5-M-1, B5-M-2, B5-L-1). The operator's option (c) is committed:
`deployments/launch/nft-pilot.json` `stages` (48 items at 12 / 24; `global_items` 31; J7's 61), held by golive-check (`--stage`,
`--proposal`) and by the monitor, which pages the exact staged step as expected. Measured on the canister's keyless pair
(2026-09-28): `ethereum-rpc.publicnode.com` serves `eth_getLogs` only about 10,000 blocks deep (about a day and a half of the 7-day
sale window) and answers dense ranges in full (74 MB for 1,000 blocks); `xrpc.cl/eth` answers `-32005` under load. Results and the
new `buyer.wasm` hash are in that document; they replace §11.11's.

### 11.11 Fix round 4 of the 2026-09-27 review (2026-09-27, this branch)

`docs/audit/2026-09-27-nft/fixes-round4.md` maps every round-4 finding to its fix and test. In short: no read of either chain goes
back to a pin below one already read, and nothing is signed on 4663 below the last mined home nonce; a home transaction whose nonce
another of ours took is dropped and its step done again, never waited on for ever (C4-H-1); a collection's waiting mint (a
purchase's or a deposit's) is one H3 ask under the collection's next-mint key, so honest deposits never trip the tripwire (E4-H-1);
the sale scan never asks an oversized range whole twice (read by halves to one block, then through the marketplaces' own sale
logs, else skipped with a record), spends a bounded number of failed reads and response bytes a pass, and low cycles stop only the
scan, never the post (B4-H-1); a Relay order past its deadline stays watched until its payout is read or the operator writes it
off, and a payout after the write-off is credited to its own order by its transaction (C4-M-1). The install check holds the whole
custody to the timelock's exit rule (A4-L-1: J7's 61 items do not fit 24 deliveries a day; the operator's call). Results and the
new `buyer.wasm` hash are in that document; they replace §11.10's.
