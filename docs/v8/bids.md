# v8: buy with bids, not only asks (a hybrid) — design

Status: **design, before any code**, 2026-09-27, branch `ideas-v8` (from `nft-side` `a37f54b`). Nothing is built, deployed or
signed. The only network calls were read-only: OpenSea's keyless `GET /api/v2/offers/collection/{slug}` (the collection-offer
book) and `collections/{slug}/stats`, Ethereum JSON-RPC (`xrpc.cl/eth`, `mainnet.gateway.tenderly.co`,
`ethereum-rpc.publicnode.com`: `eth_getLogs`, receipts, transactions, `eth_call`), and public documentation.

**Legend.** **[M]** measured on 2026-09-27 (the offer book at 22:24 UTC; the fills over Ethereum blocks 26,021,390–26,071,790,
the seven days to 22:33 UTC). **[R]** read from this repository at `a37f54b`. **[A]** assumption or estimate. **[D]** from
public documentation, cited.

**The operator's asks (2026-09-27).** "Buy with bids, not asks": the canister signs Seaport collection offers (WETH) below the
floor with its threshold key, and sellers who accept fill them. Then: "what if bids take forever to get accepted" — so the design
is a **hybrid, never bids-only**: when a buy's ETH arrives, the canister bids below the floor for a bounded window, optionally
stepping toward the floor; if nothing fills by the deadline, the offers are dead and the existing ask path (a listing within the
floor × 1.05) buys as today.

---

## 0. Summary

- **What changes:** one new branch inside an `Arrived` Ethereum buy (`docs/v7/phase1-nft.md` §2.1): `Arrived → Bidding{step} →
  (BidFilled → Final → Minting → Settling) | (BidsDead → Arrived-for-asks)`. Everything before arrival (the advance, Relay, the
  gate) and after a purchase (finality, mint, `settleAdvance`, the leftover sweep) is reused unchanged.
- **The one new trust fact:** the canister's key produces a **bearer instrument**. A signed Seaport order can be filled by anyone
  who holds the signature until it expires, is cancelled on chain, or cannot be paid. "Nothing signed while paused" no longer
  means "nothing can leave": an order signed before the pause is still live. So every rule here is about bounding a signature's
  life, price and funding, and about proving an order dead before the same ETH is used again.
- **The design choices, in one line each:**
  1. **One live order per bidding buy, quantity 1, FULL (never partial), short-lived (1 h), re-signed each hour** at a price
     re-derived from the canister's own fresh floor reading. Expiry is the normal death; nothing needs a transaction to die.
  2. **OpenSea's SignedZone** (every filled collection offer measured used it) gives an instant gasless cancel as a fast path;
     **`incrementCounter()`** is the hard kill. The canister never lists, so bumping its counter kills only its own bids.
  3. **The ask fallback starts only when every order of the buy is proven dead at Ethereum finality** (the lower finalized
     block's timestamp is past the last order's `endTime`, or a counter bump is final) and `getOrderStatus` shows it unfilled.
     The WETH is unwrapped only then. The ETH can therefore never pay twice.
  4. **A signed order counts against everything as if it filled**: H2 custody room (already reserved at the advance), J7, and H3
     value under a **stable per-buy key** (the C3-H-1 lesson: a re-sign is not a new ask).
  5. **Per collection, a switch** (`bids.enabled`), off by default, timelocked like every `eth_nft` parameter. Recommended on at
     launch for **MAYC and Milady** (thin listings, the most bid flow), then **Pudgy and BAYC**, and **off for Azuki** (14
     listings within +5 %; the ask path already buys near the floor).
- **The window (recommended):** **12 h**, three 4-hour steps at **floor − 4 %, − 3 %, − 2 %** (each step a run of 1-hour orders),
  then the ask path. §5 derives it from the measured fill rates.
- **Build and review:** ≈ 2–3 weeks to build (canister ≈ 60 %), then **its own security review round** of the signing path,
  after the round-3/4 fixes land (§11).

---

## 1. The market, measured [M]

### 1.1 The OpenSea collection-offer book now (22:24 UTC)

`GET https://api.opensea.io/api/v2/offers/collection/{slug}` answers **without a key** and returns the book best first (100
orders; the next page needs the key, which is not needed for the top). Every order was WETH, zone
`0x000056f7…d100` (OpenSea's SignedZone), `orderType` 2 (FULL_RESTRICTED), conduit key `0x0000007b…250f0000` (OpenSea's conduit
`0x1E0049783F008A0085193E00003D00cd54003c71`, read from `ConduitController.getConduit` at block 26,071,811), and OpenSea's 1 %
fee as a consideration item out of the offer.

| Collection | Floor (OS stats) | Top collection offer | vs floor | Offers within −5 % | within −10 % (units, offerers) | Listing depth within +5 % (the operator, 15:50 UTC) |
|---|---|---|---|---|---|---|
| Milady | 0.9290 | 0.866 | **−6.8 %** | 0 | 4 (5, 4) | 3 |
| Azuki | 0.6739 | 0.631 | **−6.4 %** | 0 | 4 (4, 4) | 14 |
| MAYC | 0.8558 | 0.806 | **−5.8 %** | 0 | 6 (6, 6) | 2 |
| Pudgy Penguins | 3.2862 | 3.12 | **−5.1 %** | 0 | 3 (3, 3) | 7 |
| BAYC | 6.2999 | 6.09 | **−3.3 %** | 2 | 2 (2, 2) | 5 |

- **The bid side is thinner than the ask side near the floor.** Nobody bids within 5 % of the floor except two BAYC offers. A
  canister offer at the floor − 3 % would today be **the best OpenSea collection offer in all five collections** (tied at BAYC).
- **The book is bots.** Filled offers had median lifetimes of 0.7–18 h; the top of the book is re-posted by a handful of
  addresses (the same `0xeb79f3…` sits in four of the five books). Many top orders live 10–60 minutes.
- **Blur's bid pool is not visible** (Blur's API sits behind a Cloudflare challenge, `docs/v7/phase1-nft.md` §1.1). Its fills are
  on chain and are counted below.

### 1.2 Seven days of fills, by venue and side

Every `Transfer` of each collection over 50,400 blocks, each transaction's receipt parsed for Seaport 1.5/1.6 `OrderFulfilled`
and Blur v2 `Execution721*Packed` (the canister's own parsers' rules, `ethnft.rs` `sales_in`); a listing matched to a bid in one
transaction counted once, as a bid fill. For each Seaport bid fill the order's own parameters were decoded from the fill's
calldata (`matchAdvancedOrders` / `fulfillAdvancedOrder`), giving its kind (collection, trait, item), `startTime` and `endTime`.

| Collection | Sales / day | Bid fills (share) | Seaport **collection-offer** fills / day | Blur bid fills / day | Collection offers: time from `startTime` to fill, median (p75, p90) | Their price vs today's floor, median | vs the ±12 h listing-fill median |
|---|---|---|---|---|---|---|---|
| Milady | 13.6 | 51 (54 %) | **2.1** (15) | 3.4 | 0.69 h (6.3, 32.8) | −0.9 % | −7.4 % |
| Azuki | 8.7 | 29 (48 %) | **1.1** (8) | 0.9 | 0.10 h (15.7, 490) | −3.8 % | −5.4 % |
| MAYC | 26.1 | 98 (54 %) | **3.7** (26) | 5.0 | 0.11 h (0.47, 3.1) | −4.1 % | −3.4 % |
| Pudgy | 11.7 | 51 (62 %) | **1.1** (8) | 3.9 | 2.44 h (18.3, 126) | −0.9 % | −3.9 % |
| BAYC | 10.6 | 40 (54 %) | **0.6** (4) | 2.9 | 5.1 h (10.5, —) | +0.2 % | −6.5 % |

How to read it:

- **About half of all sales are sellers hitting a bid.** The demand to sell instantly is real in every collection.
- **The canister competes in OpenSea's book only** (Blur bids are Blur's own order format funded from its Blur Pool deposit,
  outside Seaport; this design does not sign them). A seller who hits a bid takes the best of the books their tool shows; our share of
  the flow is at most OpenSea's collection-offer fills, plus whatever Blur flow a better OpenSea price wins.
- **"Time to fill" is conditional on filling** (survivorship): these are the offers that won. They are fast because they sat at
  the top; a price below the top waits for the top to be taken or to expire.
- **The two price columns bound the discount at the time.** Today's floor overstates a week-old discount where the floor has
  moved; the median of listing fills within ±12 h understates the floor (listing fills include buys slightly over it). The truth
  sits between: **collection offers filled about 1–5 % under the floor of their day**, Azuki and MAYC deepest.
- **Floors jump.** BAYC's OpenSea floor read 6.35 at 12:55 UTC (`phase1-nft.md` §1.1), 6.79 at 16:05 (the operator) and 6.30 at
  22:24 (§1.1): a thin top of the book moves the "floor" ±7 % within hours. A bid priced off a stale reading is the main risk (§6).

### 1.3 What a bid would have saved, and what it costs in waiting [A, from §1.1–1.2]

The ask path pays about the floor (the rank-2 listing, within × 1.05; with Milady's and MAYC's 2–3 listings within +5 % it is
often above the floor). Assume the ask pays **floor + 1 %** and a bid at the top of OpenSea's book fills at OpenSea's
collection-offer rate λ (a Poisson arrival; an estimate, not a measurement of our own offer):

| Collection | λ / day | P(fill) in 6 h | in 12 h | in 24 h | Saving if filled at −3 % (floor + 1 % → floor − 3 %) | Expected saving, 12 h window | Expected extra wait, 12 h window |
|---|---|---|---|---|---|---|---|
| Milady | 2.1 | 41 % | 65 % | 88 % | 0.037 ETH | 0.024 ETH | 7.4 h |
| Azuki | 1.1 | 24 % | 42 % | 67 % | 0.027 ETH | 0.011 ETH | 9.2 h |
| MAYC | 3.7 | 60 % | 84 % | 98 % | 0.034 ETH | 0.029 ETH | 5.4 h |
| Pudgy | 1.1 | 24 % | 42 % | 67 % | 0.131 ETH | 0.055 ETH | 9.2 h |
| BAYC | 0.57 | 13 % | 25 % | 43 % | 0.252 ETH | 0.063 ETH | 10.5 h |

(Expected extra wait = E[min(T, W)] = (1 − e^(−λW)) / λ, before the ask fallback's own time; add ≈ 15–30 min of Ethereum
finality either way.) **The saving is small in ETH but not in percent: ≈ 2–3 % of every item, and it lands in the collection's
own module** (§7.4). The cost is hours of delay before the unit is NFT-backed, which costs holders nothing in backing (§7.3).

---

## 2. Goal and why

**Goal:** buy each item cheaper than the ask path, without ever buying worse than it, without paying twice, and without making
the canister's key a liability it cannot bound.

**Why:**
- **Thin asks.** Milady had 3 listings within +5 % and MAYC 2 (D6). The ask path then buys the second-cheapest of very few, often
  above the floor, and waits when there are none (`Arrived` → 48 h → the way back). Bids add the other half of the market.
- **The leftover goes home.** What a bid saves stays at our Ethereum address as the buy's leftover and is swept back to **that
  collection's** module as surplus (`fundReserve`, A2-L-1), which lifts its future conversions' headroom (`phase1-nft-pooled.md`
  §1.1). Holders of that collection are the beneficiaries.
- **It reverses adverse selection only a little.** A collection offer takes whatever token a seller chooses: the least wanted
  token at the floor. That is also what a floor listing is. So the vault's quality is unchanged; only the price improves.

---

## 3. The mechanism

### 3.1 The state machine inside one buy

```
Arrived{wei}  (ETH at our Ethereum address, earmarked to this buy; §7.1)
  │  bids.enabled for this collection, the window fits (§5.3), no other buy is bidding (phase A), line free
  ▼
BidWrap        sign WETH.deposit{value = bid_max}()                          (Ethereum line)
BidApprove     sign WETH.approve(OpenSea conduit, bid_max)  (exact; §3.4)    (Ethereum line)
  ▼
Bidding{step k, order o}   every tick:
   • the price of the next order = min(our fresh floor × (1 − step_k), cap)           (§3.3)
   • sign order o_{n+1} with startTime = endTime(o_n), endTime = start + 1 h           (EIP-712, no transaction)
   • the runner posts it to OpenSea (POST /api/v2/offers)                               (§3.6)
   • stale guard: our fresh floor < price × (1 + guard) → off-chain cancel + incrementCounter   (§6)
   • fill watch: getOrderStatus(o) at the pin; Transfer(*, us, *) of the collection      (§3.5)
  │ filled                                            │ window over (last endTime passed)
  ▼                                                   ▼
BidFilled{tx, token, paid}                         BidsEnding: wait until the lower FINALIZED block's timestamp ≥ last endTime
  → Final → Minting → Settling (unchanged)           then getOrderStatus(every order) at finalized: none filled
                                                     → BidUnwrap: sign WETH.approve(conduit, 0); WETH.withdraw(bid_max)
                                                     → Arrived-for-asks (the ask path of phase1-nft §4.3, unchanged)
                                                     a fill found here → BidFilled (the ask path never started)
```

- **One live order at a time per buy**; consecutive orders never overlap (`endTime` of one = `startTime` of the next; Seaport
  accepts `startTime ≤ block.timestamp < endTime`). With one buy's WETH funding one order, a second fill cannot be paid.
- **Phase A: one bidding buy canister-wide.** WETH and the conduit allowance are per address, not per collection. With one
  bidder at a time, "the WETH at our address belongs to the one bidding buy" is exact. Phase B (§10) lifts it with a liability
  ledger once phase A has run.

### 3.2 What the canister signs (the order it builds itself)

The canister builds every field; the runner hands in nothing it signs. `OrderComponents` for Seaport 1.6 (`0x0000…68F1…B395`,
domain name "Seaport", version "1.6", chain 1; `information()` read on chain: version `1.6`, domain separator `0xfce3…ba64`):

| Field | Value | Why |
|---|---|---|
| `offerer` | our Ethereum address | |
| `zone` | OpenSea SignedZone `0x000056f7000000ece9003ca63978907a00ffd100`, pinned by code hash | the off-chain cancel (§4.2); measured on every order in the five books and on every filled collection offer of the week |
| `offer` | one item: `ERC20` (1), WETH `0xC02a…6Cc2`, amount = the price, start = end | no Dutch auction, no other token |
| `consideration[0]` | `ERC721_WITH_CRITERIA` (4), the collection, `identifierOrCriteria` **0** (any token), amount 1, recipient **us** | a collection offer, one token, to us |
| `consideration[1]` | `ERC20` WETH to OpenSea's fee recipient `0x0000a26b…a719`, **1 %** of the price | the required fee (`collections/{slug}` fee table); creator fees are optional for all five and are not paid |
| `orderType` | 2, FULL_RESTRICTED | never partial: at most one fill |
| `startTime`, `endTime` | per §3.1; `endTime − startTime` = `order_life_secs` (1 h) | short life bounds a stale fill |
| `zoneHash` | 0 | as measured |
| `salt` | from the canister's randomness | unique per order |
| `conduitKey` | OpenSea's `0x0000007b…250f0000` | as measured; the WETH allowance goes to its conduit |
| `counter` | `getCounter(us)` at the pin | §4.3 |

**Trait offers are refused.** OpenSea's build endpoint returns `identifierOrCriteria = 0` for trait offers and checks the trait
"server-side at fulfillment time" [D: Build Criteria Offer]. On chain such an order accepts **any** token; only OpenSea's zone
stands between it and a floor token. The canister signs collection offers only.

### 3.3 The price, the steps and the cap

- **Price of an order** = `min(F × (1 − s_k), cap)` where
  - `F` = the lower of the canister's own last floor reading (at most 30 min old, not the 1 h of the ask judge) and the module's
    accepted floor at the 4663 pin (the same `floor` as `judge_buy` rule 2);
  - `s_k` = the current step's discount (recommended 400, 300, 200 bps; each ≥ `min_discount_bps`, 100);
  - `cap` = `min(arrived − ETH_GAS_RESERVE − bid_gas_reserve, max_purchase_wei)`; `bid_gas_reserve` covers the wrap, approve,
    unwrap and one counter bump at the chain's fee cap (4 txs × ≤ 60k gas × 50 gwei ≤ 0.012 ETH; ≈ 0.00002 ETH at measured fees).
- **A bid can never cost more than the ask path would have been allowed to**: `F × (1 − s_k) < F × (1 + max_premium_bps)`.
- **Re-pricing is re-signing.** Each hour's order takes the fresh `F`, so the bid follows the floor down within the hour and up
  never faster than the floor rule lets `F` rise (±5 %/h, ±20 %/day at the module).
- **No sniping of our own bid:** the price is public for an hour. A seller learns nothing that the book does not already show.

### 3.4 WETH: wrap, approve, unwrap

- `WETH.deposit()` with value `bid_max` = the cap. **Only the bidding buy's own arrived ETH** (its earmark, §7.1).
- `WETH.approve(conduit, bid_max)`: **exact**, never `max`. ERC-20 `approve` overwrites, so the allowance always equals what the
  one live order may pull. After the bids end: `approve(conduit, 0)`, then `withdraw(bid_max)`.
- Four new Ethereum-line transactions, each the canister's own calldata to a pinned contract (WETH `0xC02a…6Cc2`, code hash
  pinned like Seaport's): `deposit`, `approve(conduit, x)` with the conduit fixed in the configuration, `withdraw`, and Seaport's
  `incrementCounter()`. The line's rules are unchanged (one in flight, bytes saved before the send, re-priced at the same nonce).
- **The arrival accounting must count them** (§7.1): a wrap is ETH that left the native balance; an unwrap is ETH that came back.

### 3.5 Fill detection, and minting at finality

- **The canister checks its own live order** every tick: `getOrderStatus(orderHash)` at the pin (`totalFilled = 1` → filled).
  It then needs the token id and the fill transaction. It finds them itself: the collection's `Transfer(*, us, *)` logs over the
  blocks since the order's `startTime` (the canister already reads a collection's `Transfer` logs for its own sale sample, in
  ≤ 500-block chunks). The runner's inbound-transfer discovery (`eth_deposit`) hands the same transaction in as a fast path; the
  canister trusts only its own reading of the receipt.
- **The fill judge** (the receipt at the pin, both providers): status 1; a Seaport 1.6 `OrderFulfilled` with **our order hash**,
  offerer us, recipient us; exactly one `Transfer(seller, us, id)` of the collection; our WETH out = the order's price (the fee
  inside it). Anything else in the transaction is ignored.
- **The deposit judge must learn about it.** Today every `Transfer(x, us, id)` of a configured collection without valid claim
  data is a **stray** and goes back to `x` (`judge_claim`, `phase1-nft.md` §2.4). A bid fill is exactly such a transfer: without
  a change, the canister would pay WETH and then return the NFT to the seller. `judge_claim` must classify "an `OrderFulfilled`
  of one of our orders in the same receipt" as `BidFill` (handled by the buy, never a deposit, never a stray), before anything else.
  This is the most important interaction in the design, and its first test.
- **Minting is unchanged**: once the fill's block is ≤ the lower finalized block and `ownerOf = us` on both providers,
  `EthNftWrapper.mint(tokenId, fillTx, module)`, then `settleAdvance(cost)` with `cost = ⌈advanced × paid / arrived⌉`. The
  reorg rule is unchanged (before finality a vanished receipt waits).

### 3.6 OpenSea's offer API

- **Posting:** `POST https://api.opensea.io/api/v2/offers` ("Create a criteria offer": `criteria` = the collection slug,
  `protocol_address` = Seaport 1.6, `protocol_data` = parameters + signature), API key in `x-api-key` [D]. The runner builds the
  request from the canister's `eth_bid_order(buy)` query (the parameters and the signature), never from its own fields. The
  runner may use `POST /api/v2/offers/build` only as a **check** that OpenSea's expected consideration and zone match ours; the
  canister never takes its output.
- **Cancelling:** `POST /api/v2/orders/chain/ethereum/protocol/{seaport}/{order_hash}/cancel`, for orders "protected by the
  SignedZone", with the API key of the offerer's account or an `offererSignature` (EIP-712 over Seaport's name, version, address
  and chain). The response gives `last_signature_issued_valid_until`: "Cancellation is only assured if a fulfillment signature
  was not vended prior to cancellation" [D: Cancel an order]. The canister signs the cancel message (its own typed struct, a
  second EIP-712 type, §4.1); the runner posts it.
- **Keys:** the runner holds the OpenSea key (D6). The key is never needed by the canister and never enters an install argument.
- **Rate limits** are not published ("The API is rate-limited", [D: API overview]). One post an hour per bidding buy and one
  book read a pass are far below any plausible limit; the runner backs off on 429.
- **Reading our own order back:** the keyless offers endpoint (§1.1) shows whether OpenSea accepted and lists our order; the
  monitor pages if our live order is not in the book 10 minutes after posting (a withholding runner costs time, never money).

---

## 4. Trust and security model

### 4.1 What the canister must judge before it signs anything

A new judge, `ethbid::judge_order`, run on the canister's **own** struct (the runner hands in no order bytes and no digest):

1. The buy is `Bidding`, this collection has `bids.enabled`, and the canister is not paused.
2. Every field as §3.2, from configuration and on-chain reads: Seaport and zone and conduit addresses and code hashes; WETH;
   the fee recipient and bps (≤ `max_fee_bps`); quantity 1; FULL_RESTRICTED; `identifierOrCriteria = 0`; recipient us.
3. Price ≤ `min(F × (1 − s_k), cap)` and ≥ `F × (1 − max_discount_bps)` (no dust bids that would only fill on a crash).
4. `startTime ≥` the previous order's `endTime`; `endTime − startTime ≤ order_life_secs` (≤ 2 h, recommended 1 h);
   `endTime ≤ window_end` (§5.3).
5. `counter = getCounter(us)` at the pin; our WETH balance ≥ price + fee inside it; allowance = `bid_max`.
6. The EIP-712 digest is computed by the canister from this struct and Seaport's domain; **the canister never signs a hash it
   did not compute.** (The same key signs Ethereum transactions; a caller-supplied 32-byte hash could be a transaction's.)
7. **No bulk signatures** (Seaport's merkle-tree bulk orders): one order, one signature.
8. The live-order ledger is written **before** the signature leaves the canister (like the line's saved bytes): order hash,
   price, `endTime`, counter.

A second typed struct, OpenSea's off-chain cancel message, is signed only for an order hash in the live ledger.

### 4.2 What can go wrong

| Risk | What happens | What bounds it |
|---|---|---|
| **A stale bid fills after a floor crash** | The floor drops 7 % in an hour (BAYC today, §1.2); a bot buys a floor NFT at the new floor and sells it into our bid | The price is re-derived from a ≤ 30-min reading each hour; a 1-hour order life; the stale guard (§6) cancels off chain at once and bumps the counter on chain. Worst case: one item overpaid by the crash beyond the bid's discount, within the ask path's own bound |
| **OpenSea keeps vending fulfilment signatures after an off-chain cancel** | The order stays fillable until `last_signature_issued_valid_until` or longer if OpenSea misbehaves | The off-chain cancel is the fast path only; the hard kill is `incrementCounter()` on chain (one transaction, ≈ 30–45k gas); and the 1-hour life |
| **The canister is paused (guardian) with a live order** | The order can still fill: a pause stops signing, not a signature already given | Order life ≤ 1 h; **the pause path signs the counter bump and the off-chain cancel** (the only signing allowed while paused, §9 B6); the fill is still a correct buy at a bounded price and is minted after the resume |
| **Paying twice** (a bid fill during the ask fallback) | The same ETH buys two NFTs, or one purchase fails midway | The ask path starts only after every order is dead at finality (§3.1) and the WETH is unwrapped; a late fill cannot be paid (no WETH, no allowance). §8 has the full race analysis |
| **A bid-filled NFT treated as a stray** | The canister returns the NFT to its seller after paying | `judge_claim` classifies our own order's fill first (§3.5); a test holds it |
| **The runner** | Withholds posting, posts late, withholds the fill hand-in | It can only delay: the canister finds its own fill (§3.5); the monitor pages an unposted or unlisted order. It cannot change a signed field |
| **A malicious "order" to sign** | The runner hands in a crafted order or digest | There is no endpoint that takes one (§4.1 rule 6) |
| **A collection token with an exploit** (a transfer hook, a stolen-flagged token) | The seller picks the token | Allow-listed collections only (plain ERC-721s, no hooks on `transferFrom` to an EOA); OpenSea's zone may refuse items OpenSea flags (not relied on); anything else is a floor token, which the ask path could have bought |
| **WETH or the conduit** | A contract changes | Both addresses and code hashes are pinned in the configuration; WETH is immutable; the conduit is OpenSea's, and the allowance is exact and zeroed after |
| **Signature malleability / replay on another chain** | | Seaport's domain binds chain 1 and the contract; the canister signs only chain 1 |
| **Cycles** | Hourly `sign_with_ecdsa` | 26.15 B cycles per signature with `key_1` [D: ICP cycle costs] ≈ 24 a day per bidding buy ≈ 0.63 T cycles a day at most |

### 4.3 Cancellation: `incrementCounter` vs per-order `cancel`

- **Expiry is the normal death.** A 1-hour order dies by itself; no transaction, no gas.
- **`incrementCounter()` is the kill switch.** "Cancels all orders from a given offerer … in bulk by incrementing a counter",
  jumping it by a large quasi-random amount [D: Seaport documentation]. The canister is the only offerer at its address and never
  lists, so a bump kills exactly its own bids and nothing else. One transaction, no arguments, ≈ 30–45k gas.
- **Per-order `cancel(OrderComponents[])`** is not used: it needs the order's components on the line and gains nothing over a
  counter bump while there is one live order.
- **When:** the stale guard (§6), the guardian's pause, an upgrade proposal that changes `bids` or Seaport, and the end of a
  window that had a fill race (§8). After a bump the next order reads the new counter at the pin.

### 4.4 New attack surface, stated plainly

1. A new kind of signature (EIP-712 typed data) from the custody key, with two struct types (the order, OpenSea's cancel).
2. Four new transaction targets on the Ethereum line (WETH `deposit`, `approve`, `withdraw`; Seaport `incrementCounter`).
3. A second balance at our address (WETH) and a standing allowance to OpenSea's conduit while bidding.
4. A dependency on OpenSea's zone for liveness (it may refuse to vend fulfilment signatures: then our bid never fills and the
   ask path runs; no loss) and on OpenSea's API for posting.
5. A new inbound path for NFTs (a fill), which the deposit judge must tell apart from deposits and strays.

---

## 5. Sizing, the window and the steps (the operator's question)

### 5.1 How far below the floor

The measured facts: OpenSea's best collection offer sits 3.3–6.8 % under the floor now; filled collection offers paid about
1–5 % under the floor of their day; nobody bids within 5 % except at BAYC. So **−2 % to −4 % puts the canister at the top of
OpenSea's book** while still under almost every recent collection-offer fill. Deeper than −5 % the rate falls off (Pudgy: no
collection offer filled deeper than −4.8 % vs today's floor in seven days).

### 5.2 How many open offers

**One per bidding buy, quantity 1.** A buy is one item (the module advances one at a time). Several offers or a quantity 2 would
buy a second item with ETH the module never advanced.

### 5.3 The window, and why 12 h

Constraints [R]: the way back fires at **48 h `Arrived`** (D4, `ARRIVED_BACK_SECS`), and E3-L-1 restarts the `Arrived` clocks on
re-entry; the ask path needs time of its own before that (thin collections can wait a day for a listing within the bound); an
arrived buy holds its collection's deposit mints up to 6 h (C2-M-2, §7.3). So the window must end **≥ 24 h before the way back**:
`window ≤ 24 h`.

From §1.3: at 12 h a top-of-book bid fills with ≈ 65 % (Milady), 84 % (MAYC), 42 % (Azuki, Pudgy), 25 % (BAYC). Going to 24 h
adds 19–25 points for Milady, Azuki and Pudgy and 18 for BAYC, for 12 more hours of ETH-backed units and a later ask fallback.
**Recommendation: `window_secs` = 12 h by default, per collection within [2 h, 24 h]; BAYC 24 h** (the saving per ape, ≈ 0.25 ETH,
pays for the wait; its fill rate is the lowest).

### 5.4 The steps

**Three steps over the window: −4 % for the first third, −3 % for the second, −2 % for the last** (`steps_bps = [400, 300, 200]`).
Why: the first hours try for the deepest price while nobody else is near; each step up is still under the recent fills; the last
step at −2 % is ≈ 3 % under what the ask path would pay. Each step is a run of 1-hour orders re-priced from the fresh floor. No
step reaches the floor: at the floor the ask path is as good and has no stale-bid risk.

### 5.5 Per collection (recommendation)

| Collection | Listings within +5 % | Collection-offer fills / day | Bids | Window | Why |
|---|---|---|---|---|---|
| MAYC | 2 | 3.7 | **on** | 12 h | the thinnest asks and the most bid flow |
| Milady | 3 | 2.1 | **on** | 12 h | thin asks, steady bid flow |
| Pudgy | 7 | 1.1 | on (phase A+) | 12 h | 0.13 ETH per item at −3 %; moderate flow |
| BAYC | 5 | 0.6 | on (phase A+) | 24 h | the largest saving per item; slowest flow; pooled graduations mean few buys |
| Azuki | 14 | 1.1 | **off** | — | deep asks; the ask path already buys at the floor; low collection-offer flow |

The switch is `eth_nft.collections[i].bids` (§9); off by default; a change is a timelocked upgrade argument like every other.

---

## 6. Stale offers after a floor crash

- **The stale guard, every tick while `Bidding`:** if the canister's fresh floor reading (or the module's accepted floor) is
  below `price × (1 + guard_bps)` (recommended 100 bps: our bid would be within 1 % of the new floor), or any reading is older
  than 30 min, or the monitor's D8 condition holds for the collection's `PosterSource`: (1) sign OpenSea's off-chain cancel for
  the live order; (2) put `incrementCounter()` first on the line (after the one transaction already in flight, which the line
  re-prices); (3) sign no new order this window. The buy goes `BidsEnding` and proves the order dead as §3.1.
- **Why not only on-chain cancels:** a mispriced bid on a liquid collection fills in minutes (MAYC's median collection-offer fill
  was 0.11 h from its start); an on-chain cancel waits for the line and a block. The price rule and the 1-hour life are the real
  bound; the cancels shorten it.
- **What a crash can cost at most:** one item, overpaid by the part of the crash beyond the bid's discount, within the hour the
  order lives or until the counter bump mines. The ask path carries the same exposure (a listing bought just before a crash), and
  the item is still a real NFT of the collection backing a unit.

---

## 7. Interaction with the existing invariants

### 7.1 Every unit 1:1 backed; earmarks; the arrival accounting

- Nothing changes on 4663: the unit stays in transit (backed by the advance) until `settleAdvance`, exactly as for an ask.
- **Earmark = native ETH + WETH of the bidding buy.** The buy's spend bound becomes `paid ≤ arrived − reserves` in either form.
- **The balance-based arrival watch must count wraps and unwraps** (C2-M-1, E3-M-1 were phantoms in exactly this accounting): a
  wrap is our own mined spend (`eth_out` += value); an unwrap is our own mined receipt of ETH (`eth_out` −= value). **Rule: no
  unwrap while the canister-wide Relay leg is `Bridging`** (another buy could read the unwrapped ETH as its arrival); the unwrap
  waits for the leg. Every flow read at a pin at or above the last mined block (E3-M-1's fix) is a precondition.

### 7.2 Per-collection isolation

- A bid spends only its own buy's arrived ETH (phase A: one bidding buy; phase B: a per-order liability ledger, §10).
- The fill mints on its own wrapper and settles its own module; the leftover is booked to its own collection.
- Another collection's buys are not blocked by a bidding buy past arrival, except that phase A allows one bidder at a time and
  the line carries the bid's four transactions (seconds each at measured fees).

### 7.3 Caps, the timelock, holders' units while bidding

- **H2 / J7 custody room:** reserved at the advance (unchanged). A bid adds no item beyond the one reserved.
- **H3 value:** the ask at signing time, under the buy's **stable key** `buy:eth:{buy_id}` (never per order: C3-H-1's tripwire
  counts each new key as a new ask; 12 re-signs a window would trip it).
- **The timelock:** `bids` parameters are part of `eth_nft` (published with the argument, 72 h). The Seaport, zone, conduit and
  WETH addresses and code hashes are configuration, so a change of any is a published proposal.
- **Holders' units while bidding** stay ETH-backed at the module (in transit). That is full backing; it delays NFT backing (and
  so redemption availability for that item) by up to the window. The site shows "bidding until HH:MM, then buying from listings".
- **Deposits:** an arrived buy holds its collection's deposit mints up to 6 h (C2-M-2). Bidding is not "between a fill and its
  mint", so the hold should start at a fill, not at arrival, for bidding buys (a review point: as built, a 12 h window could hold
  deposits for its first 6 h).

### 7.4 Advances, pooling and the leftover sweep

- The advance bound (`advance_bound`: floor + 5 % and fees) is unchanged: a bid spends less, never more.
- The leftover (`arrived − paid − gas`) is booked to the collection and swept back to its module as surplus (A2-L-1) once
  nothing at our address is anyone's. **"Anyone's" must include WETH and live orders**: no sweep while a bid is live or WETH is
  held.
- Pooling is unaffected: bids start only after arrival.

---

## 8. The race: a bid fills while the fallback ask is being bought

The operator's question. The rule that makes it impossible, and what remains:

1. **The ask path never starts while any order could fill.** An order can fill only in a block with `timestamp < endTime` and
   the current counter. The canister waits until the **lower finalized** block's timestamp is ≥ the last order's `endTime` (or a
   counter bump is final): after that no block that can still be produced or reorganised can include a fill (post-merge slots have
   fixed timestamps, so a reorg of a non-final block replaces it with a block of the same slot time; final blocks never change).
   This costs ≈ 15–30 min after the window [M: finality 72–168 blocks behind the head, `phase1-nft.md` §4.5].
2. **Then it reads `getOrderStatus` of every order of the window at the finalized block.** One filled → `BidFilled` (with the
   fill transaction found as §3.5); the ask path never starts. None filled → `approve(conduit, 0)`, `withdraw`, and the ask path.
3. **If both somehow happened** (a bug in 1–2): the ask's `fulfillBasicOrder` is paid in native ETH, which exists only after the
   unwrap; a bid fill needs WETH and allowance, which are gone after the unwrap. So at most one of the two can be paid from this
   buy's funds, and the other reverts. The residual is **another buy's WETH** in phase B, which is why phase B needs the liability
   ledger (every order unexpired and uncancelled counts, and WETH is never wrapped for a new bid while an unaccounted order lives).
4. **A fill in the last minutes of the window** is found by step 2 and handled as a fill; the fallback is skipped.
5. **"Reserve for both"** (fund the bid and the ask at once) is rejected: the module advanced one item's ETH; there is nothing to
   reserve a second purchase with.

---

## 9. Configuration (sketch) and open decisions for the operator

```candid
// eth_nft.collections[i] gains:
bids : opt record {
  enabled : bool;               // the per-collection switch; default false
  window_secs : nat32;          // [7_200, 86_400]; recommended 43_200 (BAYC 86_400)
  steps_bps : vec nat16;        // strictly decreasing discounts, each in [min_discount_bps, max_discount_bps]; recommended [400, 300, 200]
  order_life_secs : nat32;      // [900, 7_200]; recommended 3_600
  guard_bps : nat16;            // the stale guard; recommended 100
};
// eth_nft gains: seaport_zone, zone_code_hash, conduit_key, conduit, weth, weth_code_hash, fee_recipient, fee_bps (OpenSea's 1 %),
// min_discount_bps (100), max_discount_bps (1_000).
```

**Open decisions (the operator):**

- **B1.** Which collections bid at launch (recommended: MAYC and Milady; Pudgy and BAYC after a week; Azuki off).
- **B2.** The window (12 h; BAYC 24 h) and the steps (−4 / −3 / −2 %).
- **B3.** The order life (1 h) and the stale guard (100 bps).
- **B4.** OpenSea's SignedZone (recommended: yes; gives the gasless cancel and OpenSea's flagged-item checks, costs a liveness
  dependency on OpenSea) vs open orders (anyone holding the signature fills on chain; no off-chain cancel).
- **B5.** Phase A's one bidding buy canister-wide (recommended) before phase B's concurrent bids.
- **B6.** Signing while paused: allow exactly two acts while the guardian's pause is on — OpenSea's off-chain cancel and
  `incrementCounter()` (recommended yes: they can only stop value leaving). This changes the pause's meaning and must be written
  into the guardians' standing instructions (G2).
- **B7.** Deposits' hold for a bidding buy: from the fill (recommended), not from arrival.
- **B8.** Whether to pay optional creator royalties on bids (recommended: no, as for asks; all five make them optional).

---

## 10. Phasing

1. **After the round-3/4 fixes land** (the pin floor E3-M-1, the stable H3 key C3-H-1, the way back's exits C3-M-1): these are
   preconditions, not follow-ups.
2. **Phase A (this design):** one bidding buy canister-wide; MAYC and Milady; the monitor watching live orders, their price
   against the references, and fills.
3. **The mainnet rehearsal** (`docs/v8/mainnet-rehearsal.md`) runs the ask path first; a second rehearsal item runs the bid path
   with a 2 h window on the operator's small stake before bids are switched on for holders.
4. **Phase B:** concurrent bids across collections, with a per-order liability ledger (every signed order, its funding and its
   death proof); lifting it is its own review.
5. **Measure, then tune:** after 30 bid windows, compare realised fill rates and discounts to §1.3 and re-set B2.

---

## 11. Build and review estimate [A]

| Piece | Work |
|---|---|
| Canister | `ethbid.rs`: the order builder, EIP-712 digest (Seaport's struct hashes are in `seaport.rs`), the judge, the cancel struct (≈ 500 lines); `ethflow.rs`: the `Bidding` phases, WETH steps on the line, the finality death proof, the stale guard, the `judge_claim` fill classification, the `eth_out` accounting for wraps (≈ 600); state and upgrade (`opt` fields, `stranded_by` refuses dropping a live order); tests against the fake world: every refusal, the race of §8, a fill found without the runner, the stray misclassification, the pause cancel, the H3 stable key (≈ 900) |
| Runner | post the order (`POST /api/v2/offers`), the off-chain cancel, the fill hand-in, back-off (≈ 300 lines TS + tests) |
| Monitor | live orders vs the floor references (a live bid above both references is SEV1), unposted orders, WETH and allowance outside a bid, a fill not minted in 2 h |
| Site | "bidding" on the collection page and the reserves page (open bids, §`proof-of-reserves.md`) |
| Fork proof | an anvil fork: the canister's order and signature filled by a real seller's `matchAdvancedOrders` through OpenSea's conduit (the zone replaced by an open order for the fork), then the canister's `transferFrom` |

**Estimate: 2–3 weeks to build, then a dedicated security review round of the signing path (≈ 1 week) and its fix round.** The
review scope: the EIP-712 builder and judge, the death proof, the arrival accounting with WETH, the claim classifier, the pause
exception (B6), and the upgrade safety of live orders.

## Sources

- OpenSea API: [Create a criteria offer](https://docs.opensea.io/reference/post_criteria_offer_v2),
  [Build Criteria Offer](https://docs.opensea.io/reference/build_offer_v2), [Cancel an order](https://docs.opensea.io/reference/cancel_order),
  [Get Collection Offers](https://docs.opensea.io/reference/get_collection_offers_v2), [API overview](https://docs.opensea.io/reference/api-overview),
  [Seaport interface](https://docs.opensea.io/docs/seaport-interface).
- Seaport: [SeaportDocumentation.md](https://github.com/ProjectOpenSea/seaport/blob/main/docs/SeaportDocumentation.md) (counters, `incrementCounter`, bulk orders).
- ICP: [cycle costs](https://docs.internetcomputer.org/references/cycle-costs/) and [threshold ECDSA](https://docs.internetcomputer.org/building-apps/network-features/signatures/t-ecdsa).
- On chain [M]: `ConduitController.getConduit(0x0000007b…250f0000)` = `0x1E0049783F008A0085193E00003D00cd54003c71`;
  Seaport `information()` = (`1.6`, `0xfce34bc6…ba64`, `0x00000000F9490004C11Cef243f5400493c00Ad63`); the SignedZone holds 6,967 bytes of code.
