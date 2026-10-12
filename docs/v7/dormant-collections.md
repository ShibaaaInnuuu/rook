# v7: dormant, wake-on-demand collections (design)

Status: **design, written before its code**, 2026-09-29, branch `dormant-collections` (from `origin/main` at `9efed1b`). It
amends `docs/v7/phase1-nft.md` and `docs/v7/phase1-nft-pooled.md`. Nothing is deployed, nothing was signed or sent, no cycles or
ETH were spent. The only network calls were read-only: OpenSea's keyless `collections/{slug}` and `collections/{slug}/stats`,
CoinGecko's keyless `nfts/ethereum/contract/{address}`, and Ethereum JSON-RPC (`rpc.mevblocker.io`: `eth_getCode`,
`eth_getStorageAt`, `supportsInterface`).

**Legend.** **[M]** measured on 2026-09-29 (Ethereum block ≈ 26,081,410). **[R]** read from this repository. **[A]** assumption or
estimate.

**The operator's ask (2026-09-29).** "Everything available": a user can launch a coin on any hyped blue-chip, and whatever they
pick gets launched. The budget is tight (≤ $100 of cycles at launch, in all), so a collection nobody uses must not burn cycles.
Today every configured collection is watched all the time: its sale scan (a sales-confirmed floor over a 7-day backfill), its
floor evidence, its module and wrapper reads, and a floor post about every 40 minutes. That is roughly 1.5–2 T cycles a day per
collection (§7), whether anyone uses the collection or not.

---

## 0. Summary

1. **An allowlist of ten blue chips** (§1): Milady, Azuki, MAYC, Pudgy Penguins and BAYC (the pilot), plus Doodles, Moonbirds,
   CloneX, Lil Pudgys and Checks (VV Originals); and since 2026-10-09 (the operator) **Identity MD**, dormant like the five, whose only
   D8 price reference is OpenSea (CoinGecko does not list it: `nft-pilot.json` `priceRefs`, §1). Five candidates are left out, each for a reason the buyer cannot work around:
   CryptoPunks (both wrappers), DeGods, Captainz and Meebits. Every allowlisted collection is deployed on 4663 as today (its own
   wrapper, poster source, module, factory and vaults). It is listed in the canister's install argument, so adding one or removing
   one is still a timelocked upgrade.
2. **Every collection has a life cycle in the canister** (§2): `DORMANT → WAKING → ACTIVE → DORMANT`.
   - **DORMANT** costs nothing: no evidence pass, no sale scan, no module or wrapper read, no post, no buy. A handed-in
     evidence is dropped without an outcall.
   - **WAKING** runs the sale scan with the 7-day backfill, faster than usual. There is no post until the window is read and the
     listings are confirmed by the counted sales. Buys and deposit mints are refused. The canister publishes "warming up (~N
     min)" (§4).
   - **ACTIVE** starts only when that first confirmed post has mined.
   - Back to **DORMANT** after `idle_days` with no position, nothing open on 4663 or in the canister, or six hours after ACTIVE when
     none ever appeared (§5). A bare launch is no position: it costs gas alone (dormant-review DR-H-1).
3. **A wake costs cycles, so a wake is rate-limited and bonded** (§3):
   - **Canister caps**: at most `max_active` collections ACTIVE or WAKING, `max_waking` waking at once, `wakes_per_day` wakes a
     day in all, and a per-collection cooldown after a wake that failed or a return to dormancy.
   - **A bond on 4663**: a new `WakeDesk` contract takes a public, permissionless wake request with a bond (0.003 ETH at launch).
     The canister reads the requests itself. The bond goes back to the requester once a coin they launched on that collection's
     factory after asking has reached its raise. Otherwise, after the window, anyone can close the request: the bond goes to the
     protocol treasury if the canister posted the collection's floor since the request (the wake was served), and back to the
     requester if it did not (dormant-review DR-H-3). The desk allows one open request per collection.
4. **Adaptive slow cadence for ACTIVE collections** (§4):
   - The evidence pass (the listing reads and the post) runs every 30 minutes. That is as slow as the module's one-hour floor TTL
     allows, since a stale module refuses burns.
   - The sale scan runs every hour. It backs off to 4 hours after passes that count no new sale.
   - The module read that starts a buy runs every 30 minutes.
   - Near a pending buy, everything runs at every hand-in: an open buy, a deposit waiting for its mint, or a module that wants an
     item.
5. **A fresh read before every buy** (§4.3): the fill is signed only against a reading at most 15 minutes old. That reading
   comes with a sale scan that is current to within 300 final blocks, and the counted sales confirm it. It must also agree with
   the module's accepted floor within 5 % and with the floor the advance was sized on within 10 %. Otherwise the fill waits for
   the next pass, and its listing is kept. No buy starts, and no queued graduation is filled, until the module's accepted floor is
   within 5 % of our reading (after a long dormancy the first posts are clamped by the module's bounds: dormant-review DR-M-1).
6. **Nothing already invariant changes** (§6): the pin floors, the response caps, the nonce lines, H3, the staged caps and the
   exit rule all stay as they are. The install's custody caps name a **binding** global (48 items at step 1), so ten listed
   collections still let every item out within the timelock's 72 h less a day.
7. **Cycles scale with ACTIVE collections, not listed ones** (§7). Idle with nothing active costs about 0.1–0.3 T a day. Each
   ACTIVE collection costs about 1.5–2.2 T a day, and each wake about 0.5–1.5 T once. On a 13-node subnet a $100 budget
   (≈ 74 T) lasts about 250 days idle, about 34–45 days with one collection active, 17–23 with two and 11–16 with three.
8. **The limits auto-scale with the canister's own cycles balance** (§11, 2026-09-29, amending §3.2 and §7). `max_active` and
   `wakes_per_day` are now only hard ceilings (10 and 12 at launch, timelocked). Under them, a collection may be awake only while the
   balance pays 7 days of every awake collection's burn plus a reserve, and a wake starts only if the balance after its one-time cost
   still does with one more awake. The launch starts at 34 T (≈ US$46), enough for one collection; each top-up raises the limit.

The dormancy is **opt-in by configuration**: `eth_nft.dormancy = null` keeps today's behaviour exactly (every collection ACTIVE
all the time). That is how every earlier test runs unchanged. The launch configuration turns it on.

---

## 1. The allowlist [M, R]

**The rule** (from `phase1-nft.md` §1.3, restated for "everything available"): the collection must be one the buyer can buy,
hold, deliver and price.

- It is an **ERC-721 on Ethereum**. The buyer fills Seaport 1.6 orders for one ERC-721 paid in ETH, and delivers by
  `transferFrom` (`seaport.rs`, `ethnft.rs`) [R].
- It has **no proxy and no admin who can change its code**: the EIP-1967 implementation, admin and beacon slots are all empty.
  An upgradeable collection's admin could move or freeze what the canister holds (`phase1-nft.md` §7, "Collection admin
  powers").
- **Its Seaport listings pass the buy judge.** No required fee may go to an address that is neither a marketplace fee recipient
  of the chain nor the collection's ERC-2981 receiver (`seaport.rs` rule 6).
- **Its market is on Seaport or Blur**, the two venues whose fills the sale scan parses. At least one real sale a day is needed
  in practice for `min_sales` counted sales in the window.

| Collection | Contract | Floor (OS) | 7 d / 30 d sales | Proxy slots | 2981 | Required fees | Verdict |
|---|---|---|---|---|---|---|---|
| Milady | `0x5af0…25a5` | 0.920 | 99 / 463 | empty | no | OS 1 % | **allow** (pilot) |
| Azuki | `0xed5a…c544` | 0.644 | 98 / 322 | empty | no | OS 1 % | **allow** (pilot) |
| Mutant Ape YC | `0x60e4…a7c6` | 0.839 | 371 / 878 | empty | no | OS 1 % | **allow** (pilot) |
| Pudgy Penguins | `0xbd35…2cf8` | 3.280 | 123 / 449 | empty | no | OS 1 % | **allow** (pilot) |
| BAYC | `0xbc4c…f13d` | 6.150 | 140 / 342 | empty | no | OS 1 % | **allow** (pilot, J1) |
| Doodles | `0x8a90…992e` | 0.252 | 131 / 537 | empty | no | OS 1 % | **allow** |
| Moonbirds | `0x2358…a68b` | 0.409 | 52 / 289 | empty | yes | OS 1 % | **allow** (nested birds cannot move: such a listing fails the simulation and is never bought) |
| CloneX | `0x49cf…a28b` | 0.135 | 212 / 470 | empty | no | OS 1 % | **allow** |
| Lil Pudgys | `0x524c…a048` | 0.315 | 277 / 998 | empty | no | OS 1 % | **allow** |
| Checks (VV Originals) | `0x0367…48b1` | 0.159 | 48 / 117 | empty | no | OS 1 % | **allow** (the thinnest: ~7 sales a day, wide trait spread; its wake may time out, §2.3) |
| Identity MD (added 2026-10-09) | `0x0000…ec1d` | ~1.78 | ~190 / — | empty | no | OS 1 % | **allow** (plain Solady ERC-721, no burn / pause / transfer hooks, owner the CREATE2 deployer, which cannot call; passes the buy judge's four tests. Not on CoinGecko: OpenSea is its ONE D8 reference, the operator's decision of 2026-10-09. maxPurchaseWei 2.5 ETH; custody 15 ETH / **8 items at every step**, J7's min(24, 15 / 1.78)) |
| CryptoPunks, wrapped (WPUNKS) | `0xb7f7…13f6` | 40 | **0 / 0** | empty | no | none | **excluded** |
| CryptoPunks 721 | `0x0000…5c2f` | 0 | **0 / 1** | empty | no | OS 1 % | **excluded** |
| DeGods (Ethereum) | `0x8821…d280` | 0.190 | 48 / 81 | **EIP-1967 implementation set** | yes | OS 1 % | **excluded** |
| Captainz (Memeland) | `0x7692…44cc` | 0.030 | 97 / 324 | **EIP-1967 implementation and admin set** | no | OS 1 % | **excluded** |
| Meebits | `0x7bd2…6bc7` | 0.313 | 146 / 282 | empty | **no** | OS 1 % + **2.5 % creator, required**, to `0x98c4…bdc7` | **excluded** |

**Why the five are left out:**

- **CryptoPunks** are not ERC-721. They trade on their own market contract (`CryptoPunksMarket`), which neither the buyer (Seaport
  only) nor the sale scan (Seaport and Blur fills) reads. Both ERC-721 wrappers exist and would be mechanically buyable and
  deliverable. But their Seaport/Blur market is empty: the legacy wrapper had 0 sales in 30 days and "CryptoPunks 721" had 1.
  So no floor could ever be confirmed by counted sales, and a wake could only time out. Supporting punks needs a reader of the
  punks market's own `PunkBought` events and a buyer of `buyPunk` then `wrap`: a new judge, out of scope here.
- **DeGods** (the Ethereum contract) is an EIP-1967 proxy (implementation `0xd200…0a1e`). Its admin can replace the code under
  what the canister holds. The same goes for **Captainz** (implementation and admin both set). The allowlist admits no
  upgradeable collection.
- **Meebits** requires a 2.5 % creator fee to `0x98c4…bdc7` on every OpenSea listing and has no ERC-2981 `royaltyInfo`. That
  address is neither a marketplace fee recipient of the chain nor a royalty receiver, so the buy judge refuses every Meebits
  listing ("unknown recipient"). Adding the address to the chain's `fee_recipients` would allow it for every collection. That
  is a policy change for the operator, not something this design does quietly.

**Per-collection parameters** (`deployments/launch/nft-pilot.json`, the new entries follow J7's rule):

- `custodyWei` is 15 ETH.
- `custodyItems` is `min(24, ⌊15 / floor⌋)`. The cheap collections are capped at 24, what 12 deliveries a day let out in the two
  usable days.
- `maxPurchaseWei` is about the floor × 1.1, rounded up, at least 0.5 ETH.
- The graduation targets are the defaults (5 ETH).
- The J7 step-3 counts follow the same rule.

The **global** item cap is what binds (§6).

## 2. The life cycle (canister, `dormancy.rs`)

```
              wake request (desk bond | operator | a deposit | a launch or graduation seen)      caps: max_active, max_waking,
DORMANT ─────────────────────────────────────────────────────────────▶ (QUEUED) ──room──▶ WAKING     wakes_per_day, cooldown
   ▲                                                                                      │
   │ idle_days with no position (§5)                         backfill read + confirmed    │  timeout (12 h): back to DORMANT,
   │ or none at all within 6 h of ACTIVE                     reading → the first post     │  cooldown, paged
   └───────────────────────────────────────────── ACTIVE ◀──── mined ──────────────────────┘
```

### 2.1 State

`EthState.dorm` (optional, so older states decode) holds, per collection:

- the phase (`Dormant`, `Waking { since, backfilled_at }`, `Active { since }`);
- what woke it, and a queued request if there is one;
- `cooldown_until`, and whether it was ever active;
- the module's factory, and when a position was last seen (the idle clock);
- the cadence clocks (the last pass, scan and module view, the scan's back-off, `hot_until`);
- the published estimate, and a note.

Canister-wide it holds the desk cursor and the start times of the last day's wakes.

**Initialization.** On the first tick with `dormancy` set, a collection is **ACTIVE** if the canister already has anything of it
(a reading, a buy, a delivery, a deposit, a scan, a ticket cursor) or its module's `accepted()` is non-zero at the 4663 pin (it
was posted, so it may have holders). Otherwise it is **DORMANT**. A fresh launch install has every collection DORMANT. A
collection added by a later upgrade starts DORMANT. A removed one is refused by the existing `stranded_by` rule while it holds
work.

### 2.2 DORMANT

The tick skips it entirely:

- no `scan_tickets` (it holds nothing: §5 requires `held = 0`);
- no `floor_pass` (an evidence hand-in is dropped with no outcall; `eth_floor_evidence` refuses one);
- no module view in `buy_pass`, and no post.

A claim of it is still judged: a stray goes back as today, and a valid deposit is recorded and waits for its mint (which needs
ACTIVE) while it requests a wake.

**The watchdog.** Every 6 h, and only for a collection that was **ever active**, the canister makes one small 4663 read: the module's
`gradEth()`. A graduation queued there requests an owed wake. A never-posted module cannot take a launch at all: `CurveFactory.launch`
needs `hasRoomFor`, which needs a non-zero `gradPrice()`.

*As reviewed (DR-H-1):* the design read the factory's `coinCount()` too and woke on a new coin. A coin costs gas alone, so that let
anyone spend the day's wakes. A coin still trading needs no fresh floor, and its graduation is the owed wake.

### 2.3 WAKING

- **The scan starts afresh.** The collection's `SaleScan` and cached sales are dropped at the wake (they were dropped at
  dormancy too): the backfill reads the whole 7-day window at the new pin.
- **Faster than usual:** every evidence hand-in is a pass (the runner hands in a waking collection every cycle), and the scan
  judges up to `WAKE_RECEIPTS_PER_PASS` = 64 receipts a pass instead of 16. The pass's and the day's **byte** budgets
  (`SALE_RECEIPT_BYTES_PER_PASS`, `SALE_SCAN_BYTES_PER_DAY`) are unchanged, so the worst case per collection is unchanged
  (≈ 10.5 T a day).
- **No post during the backfill.** The reading is computed each pass, but `due` stays false until the backfill is complete and
  the reading is confirmed:
  - complete means the window's logs were read down to its start, with no hole, no lost span, no raw range, no skipped block, no
    pending move and no re-scan left;
  - confirmed means `min_sales` counted sales, and the listing floor inside the counted sales' band (`held == None`).

  Then the first post goes out through the normal home line.
- **ACTIVE when that post mines** (`mined_home`, `Step::Post`, success).
- **Timeout:** WAKING longer than `wake_timeout_secs` (12 h by default) goes back to DORMANT with the cooldown. Its scan is
  dropped, and the monitor pages SEV2. A wash-sale campaign that holds the confirmation back therefore costs us one wake, never
  an open-ended scan (review §2).
- **Refused while WAKING:** a new buy (`buy_pass` starts buys for ACTIVE collections only), a fill (`eth_act` checks the
  phase), and a deposit mint (`home_act` checks the phase). Deliveries, returns, records and marks are never gated: a WAKING
  collection holds nothing to deliver.

### 2.4 ACTIVE

Everything as today, at the slow cadence of §4.

## 3. Wake requests, rate limits and the bond

### 3.1 Sources

| Source | How the canister learns it | Cooldown applies | Notes |
|---|---|---|---|
| **Desk** (the public path) | `WakeDesk.requestCount()` each tick; up to 4 new `requestAt(i)` a tick | yes | a bonded request; the UI's "Launch on a dormant collection" |
| **Operator** | `eth_wake(collection)` (operator-only update) | yes | the runner's or the operator's own call |
| **Deposit** | a valid deposit of a DORMANT collection judged in `deposit_pass` | **no** | an NFT really sent to us: costly for the sender, and its mint is owed |
| **Graduation** | the watchdog reads `gradEth() > 0` | **no** | a whole raise waiting on our floor: owed |

Every source is subject to `max_active`, `max_waking` and `wakes_per_day`. A request waits in a queue (one entry per collection),
**owed first** (deposit, graduation), then first come first served.

**As reviewed (DR-H-1, DR-H-2).**
- Owed wakes have one slot above `max_active` (`OWED_RESERVE`): the slots bought by desk and operator wakes never keep an owed
  wake from starting. The cycles budget is therefore planned at `max_active + 1` for a day at a time.
- An owed wake that times out backs off: the next owed wake of that collection waits `cooldown × 2^(fails − 1)`, at most 7 days.
  After `OWED_FAILS_MAX` = 3 timed-out owed wakes, a deposit that asked for them goes back to its sender through the existing
  return path. One NFT sent to the canister can never hold the waking slot for good. A wake that becomes ACTIVE clears the count.

**As reviewed independently (DI-H-1).** An owed wake is asked whatever is queued already: a deposit of a DORMANT collection whose
desk or operator wake is queued upgrades that wake to owed (keeping its place), and the watchdog reads a DORMANT once-active module
whose queued wake is not owed. The owed reserve therefore holds with a desk wake queued, on a balance that keeps no desk wake.

### 3.2 The caps (`eth_nft.dormancy`, in the install argument: timelocked)

| Field | Default | Bounds | Meaning |
|---|---|---|---|
| `max_active` | 10 (was 3) | 1..=16, ≤ the collections | ACTIVE + WAKING at most: **the hard ceiling** (§11: under it the cycles balance decides) |
| `max_waking` | 1 | 1..=`max_active` | backfills at once |
| `wakes_per_day` | 12 (was 4) | 1..=24 | wakes started in any 24 h, all sources: **the hard ceiling** (§11: each wake also needs the balance) |
| `wake_timeout_secs` | 43,200 | 3,600..=172,800 | WAKING longer than this fails |
| `cooldown_secs` | 86,400 | 3,600..=2,592,000 | after a failed wake or a return to DORMANT |
| `idle_days` | 3 | 1..=60 | ACTIVE with no position this long (§5) goes DORMANT |
| `wake_desk` | none | an address | the 4663 `WakeDesk`; none: no public path |
| `scaling` | the §11.2 numbers | §11.2 | the limits follow the cycles balance; none: the ceilings are fixed caps (as before §11) |

**The most one attacker can make the canister spend on wakes** is `wakes_per_day` × one wake plus six hours of ACTIVE
(§5, `NO_POSITION_SECS`): 4 × (≈ 1.5 T + ≈ 0.5 T) ≈ 8 T a day. That needs four bonds a day. Each is forfeited unless the attacker's
coin reaches its raise (≥ 5 ETH bought on its curve; dormant-review DR-H-1), or the wake is never served (no post: the bond comes
back, DR-H-3, and the wake cost at most a backfill). The cooldown stops the same collection from being cycled.

*Amended by §11:* the ceiling is now 12 wakes a day, but each wake also needs the balance after its cost to keep every awake
collection's 7-day runway and the reserve, and the 24 h cooldown lets each of the ten collections be woken (not owed) about once a day.
So a griefer can spend at most the balance's headroom above what the awake collections need (≈ 2.1 T a wake, at a 0.003 ETH bond each),
never their runways or the reserve (dormant-review, Scaling).

### 3.3 `WakeDesk` (4663, new, ≈ 150 lines)

- **Immutables:** `directory` (the `CollectionDirectory`: which wrappers exist, admin-listed as today), `treasury` (the
  `ProtocolTreasury`), `bond` (within [0.0005, 1] ETH; 0.003 at launch), `window` (within [1 day, 30 days]; 7 days at launch), and
  `parentBlockSeconds` (the `Wait` rule: both clocks).
- `request(wrapper) payable`: `msg.value == bond`, the wrapper is in the directory, no **open** request for it exists (one
  younger than the window and not closed), and **its module is not fresh** (`fresh()` false: the collection is asleep; independent
  review DI-M-2). It records the requester, both clocks, and the factory's `coinCount()` then. Events: `Requested`.
- `reclaim(id, coinIndex)`, by anyone: all of these hold, and then the bond goes to the requester and the request is closed.
  - The request is open.
  - The factory's `coins(coinIndex)` exists, at an index ≥ the recorded `coinCount`.
  - Its `creatorOf` is the requester.
  - **Its curve reached its raise** (`Graduating` or `Graduated`, dormant-review DR-H-1): a bare launch costs gas alone, and would
    make every wake free.
  - **It is not a floor-buys-only coin** (`sweepOnly()` false): such a coin needs no floor, so it is not what a wake is for.
  - **The coin has not reclaimed a bond before** (`spent(coin)`, dormant-review DR-L-5): one launch pays back one bond.

  Allowed until the window plus a day's grace. The window is 7 days at launch, time for a real coin to fill.
- `forfeit(id)`, by anyone, after the window plus the grace, closes the request (dormant-review DR-H-3, independent review DI-M-1):
  - if the wake was served **inside the window** (`served(id)`: a post recorded by `markServed`, or the module's latest post in
    `[at, at + window]`): the bond goes to the treasury (`Forfeited`);
  - otherwise it was not served in time (queued behind the caps, timed out, or served only after the window, when no reclaim was
    possible any more): the bond goes back to the requester (`Refunded`). An address that does not take it within the 50,000-gas
    stipend is credited (`Owed`, DI-L-2) and withdraws with `withdraw(to)`; it never goes to the treasury for that. A module that
    cannot say (`postedAt()` reverts) counts as served.
- `markServed(id)`, by anyone (the canister sends it, `Step::Served`, after the post that served the request): records the module's
  latest post while it lies in `[at, at + window]`, on the timestamp alone (independent review 2, D2-L-1: a post later than
  `at + window` is refused even while the block clock's half of the window is still running, the same bound `served` reads). The
  module keeps only its latest post, so this is what keeps a served wake served after later posts (DI-M-1).
- `reclaim`'s payment, too, is credited instead of reverting when the requester's address does not take it (DI-L-2).
- **Views:** `requestCount()`, `requestAt(id)` (wrapper, requester, at, bond, closed), `openOf(wrapper)`, `served(id)`,
  `servedAt(id)`, `owed(address)`.
- **No owner, no pause, no upgrade**: it only holds bonds. Checks-effects-interactions, and `nonReentrant`.

The canister trusts the desk only for "someone asked". It never moves funds on it and never reads a price from it; its one call
there is `markServed`, which the desk checks against the module itself. The desk's address is in the install argument, and
golive-check holds it to the stack file's.

**How the canister holds a request (independent review DI-M-1, DI-M-2).** It reads `window()` once. Each request read is kept on its
collection's life with its window's end (`Life.desk`). DORMANT: its wake is queued only while a wake started now could still post
inside the window (`now + wake_timeout_secs + 6 h ≤ at + window`, `DESK_START_MARGIN_SECS`); past that it is dropped from the queue and
never started late. Its bond then goes back unless the collection is posted inside the window by another wake (a deposit's, a
graduation's, the operator's): the desk counts any post inside the window as serving the request, and the canister's log, the life's
note and the site say exactly that (independent review 2, D2-L-3). A queued desk wake about to start is read again on the desk, and
dropped if the request was closed there. A new request of a collection whose queued desk wake belongs to an earlier request (closed,
not read again yet) takes that place at its own read time, never the earlier request's (D2-I-2). WAKING: the wake under way serves it. ACTIVE (a request the desk took while the module was stale): the next
post serves it, and if the collection sleeps first its wake is queued (once: a wake that timed out is not asked again for it). The
first post of ours at or after `at` inside the window marks it served, and the canister records that on the desk (`markServed`).
It gives the mark up only on the chain's word (D2-L-2): the desk already holding it, the request closed (`requestAt`), or the module's
latest post past the window. An estimate that failed while the chain shows the mark can still be taken (a provider that timed out or
disagreed reads the same as a revert) is tried again at the next tick.

## 4. Cadence

### 4.1 Constants (`dormancy.rs`)

| Constant | Value | What |
|---|---|---|
| `PASS_EVERY_SECS` | 1,800 | ACTIVE, not hot: an evidence pass at most this often. With the runner's 20-minute hand-ins, that is a pass (and a post) about every 40 minutes, inside the module's 1 h `FLOOR_TTL` |
| `SCAN_EVERY_SECS` | 3,600 | ACTIVE, not hot: the sale scan inside a pass at most hourly |
| `SCAN_BACKOFF_MAX_SECS` | 14,400 | doubled after each scan pass that counted no new sale, up to 4 h; reset by a new sale |
| `VIEW_EVERY_SECS` | 1,800 | ACTIVE, not hot: the module read that decides a new buy |
| `HOT_SECS` | 3,600 | hot after a module view wanted an item, or a fill waited for a fresh read |
| `SLEEP_CHECK_SECS` | 21,600 | the re-dormancy reads, the ACTIVE launch count, and the DORMANT watchdog |
| `WAKE_RECEIPTS_PER_PASS` | 64 | WAKING: receipts judged a pass (the byte budgets bound it) |
| `DESK_READS_PER_TICK` | 4 | new desk requests read a tick |
| `NO_POSITION_SECS` | 21,600 | ACTIVE after a wake with no position at all this long: DORMANT (six hours: a once-active module takes launches while it sleeps, so a wake stays up only for what needs a fresh floor) |
| `OWED_RESERVE`, `OWED_FAILS_MAX`, `OWED_BACKOFF_MAX_SECS` | 1, 3, 604,800 | owed wakes: one slot above `max_active`; back-off after a timeout; a deposit returned after 3 (DR-H-1, DR-H-2) |
| `FLOOR_TTL_SECS`, `TTL_MARGIN_SECS` | 3,600, 1,500 | an ACTIVE pass is due 25 min before the module's post goes stale, whatever the cadence (DR-M-3) |
| `COINS_PER_FILL_TICK`, `FILL_RETRY_SECS` | 8, 1,800 | the fill hunt: the newest coins and the operator's `eth_fill` hand-ins, a try every 30 min |

**As reviewed (DR-M-3).** Evidence handed in before a pass is due is kept (until its 10-minute TTL or a newer hand-in), not
dropped, and a pass is due whenever the module's last post is within 25 minutes of its one-hour TTL. One or two late or failed
hand-ins no longer let a module with holders go stale.

**Hot** (fast cadence: every hand-in is a pass with a scan, and the view is read every tick) means an open buy of the collection,
a deposit of it waiting for its mint, or `hot_until` in the future.

**Why posts stay at about 40 minutes.** A stale module refuses burns (`burnsOpen`), refuses a graduation's fill (and nothing else
calls `fill`, so a stale fill would drift into refunds), and refuses an advance. The post is therefore kept at the TTL's pace
while ACTIVE. Each post is a threshold signature (26.15 B cycles), so posting is the largest per-collection cost (§7). **That is
exactly why DORMANT exists**: an unused collection makes no post at all.

### 4.2 The API and the UI

- `eth_nft_collections() -> vec CollectionLife` (query): for every configured collection, its state (`always-on`, `dormant`,
  `queued`, `waking` or `active`), since when, the queue position, what woke it, the cooldown, `eta_secs` while WAKING, and
  `status`. `status` is the human text: "dormant: a launch wakes it", "queued (#2)", "warming up (~40 min)", "active".
- **The ETA:**

  ```
  passes = max(log passes, receipt passes) + 1
  eta    = passes × 600 s + 600 s
  ```

  - Log passes: the blocks left to read over the pass's log reads × the chunk.
  - Receipt passes: the moves left over 64. The moves left are the pending moves, plus the unread blocks × the moves per block
    seen so far (700 moves assumed before the first read).
  - The last 600 s is the post's own tick.

  It is recomputed at every pass.
- **The runner** serves `GET /collections`: `eth_nft_collections()`, cached for 60 s, public and read-only.
- **The site** reads it (`VITE_COLLECTIONS_URL`) and shows the status on the Launch page. A dormant or queued collection offers
  "Wake it (bond 0.003 ETH, back when you launch)", which calls `WakeDesk.request(wrapper)`.
- **As reviewed (DR-M-4)**, the page fails closed. `GET /collections` is an uncertified relay, so it only says how to wait. With a
  desk in the stack file, a coin that buys items launches only while the page's own chain read of `FloorModule.fresh()` is true.
  The wake button reads `WakeDesk.openOf(wrapper)` and is not offered while a request is open, and the bond's reclaim and refund
  terms are said beside it.
- **As reviewed independently (DI-M-2, DI-L-1).** The wake is not offered while the module is fresh (the desk refuses it then). An
  open request on a collection the canister still calls dormant is worded from its age (`requestAt`) and the desk's `window()`:
  under an hour, "the canister reads it at its next tick"; older, the canister could not serve it inside its window, its bond goes back
  unless another wake posts the collection inside the window (D2-L-3: the desk's rule, worded as it is), and a new request can be made
  when the window ends. With no lives served, a module read not fresh offers the wake from the chain alone (gated on `openOf`), but only
  once its `postedAt()` is at least two hours old (D2-I-1: an ACTIVE collection's brief staleness, a post late past the TTL, is not
  taken for dormancy), and the page says that if the collection is in fact awake its next post serves the request and the bond pays
  for it. The Docker build takes `VITE_COLLECTIONS_URL` as an `ARG`.

### 4.3 The fresh read before a buy (`eth_act`, only with `dormancy` set)

Before `eth_plan_buy` for an arrived buy, all of these must hold:

1. The collection is ACTIVE.
2. Our reading is at most `BUY_FRESH_SECS` = 900 s old, and that pass **ran the sale scan**, which ended at most
   `BUY_SCAN_LAG_BLOCKS` = 300 final blocks behind (the reading's new `scanned` and `scan_behind`).
3. The reading is **confirmed** (`held == None` and `sales ≥ min_sales`).
4. `|reading − module accepted| ≤ BUY_MISMATCH_BPS` = 500 of the accepted floor, read at the 4663 pin in the same act.
5. `|reading − floor at the advance| ≤ BUY_DRIFT_BPS` = 1,000 (the life's `buy_floor`, set when the advance was sent). A buy with
   no recorded floor is refused, never passed (dormant-review DR-L-4).

**As reviewed (DR-M-1).** A new buy is not started, and `Step::Fill` is not sent, until the module's accepted floor is within
`BUY_MISMATCH_BPS` of our reading. After a long dormancy the module clamps each post to its bounds, so the first posts walk the
accepted floor towards the market. A graduation filled on the way would convert at a price that has not caught up.

After these, the existing bound applies unchanged:
`total ≤ min(reading, accepted) × (1 + premium)`, what arrived less the gas reserve, and `max_purchase_wei`.

**If any check fails, nothing is signed.** The listing is put back when (2) failed alone, the collection goes hot, and the next
hand-in runs a scanned pass. On a mismatch (4 or 5) the listing is dropped and the buy stays `Arrived`: the 48 h way back still
applies. The runner hands in the collection's evidence with every listing it hands in.

## 5. Back to DORMANT

Every `SLEEP_CHECK_SECS`, an ACTIVE collection's positions are read. It goes DORMANT once it has had none for its idle period.

**A position** is any of these:

- **On 4663**, read at the pin every check:
  - the wrapper's `totalSupply()`, `held()` or `rentedHeld()` above 0 (a unit, an item, a rental);
  - the module's `gradEth()`, `gradUnits()`, `transitUnits()` or `floatUnits()` above 0 (a queued or unreleased graduation, an
    advance, a float).
- **In the canister:**
  - a buy of it unfinished (a sweep included), or a written-off Relay order of it still holding the leg;
  - a delivery of its wrapper not Recorded, Marked or Gone;
  - a deposit of it waiting for its mint (Seen, Mint or Minting);
  - a home transaction of it pending, or an Ethereum line transaction of it;
  - a leftover of at least `SWEEP_MIN_WEI`.

**The idle period** is `idle_days` since a position was last seen, or `NO_POSITION_SECS` (six hours) after ACTIVE when none was ever seen
since the wake. The positions are read every check, so the clock restarts at every check that sees one.

**A bare launch is no position** (dormant-review DR-H-1). `CurveFactory.launch` costs gas alone. So "no launch for N days" would let
anyone keep a collection awake forever, and a desk request plus a launch would make every wake free. A coin still trading needs no
fresh floor: its curve trades without the module. Its graduation, when it comes, raises `gradEth`, which the watchdog reads within
6 h as an owed wake. The curve's window (3 days, `FrogCurve`) is far longer than a watchdog period plus a wake.

**Each collection's fill waits on its own** (independent review DI-M-3). The fill hunt waits for a collection whose module is not fresh,
or whose accepted floor has not caught up with our reading (DR-M-1), by trying it again after `FILL_RETRY_SECS` and going on to the next
collection with a due hunt. Only a signature the home line refuses (the nonce, the pause) ends the hunt for the tick.

When it sleeps, the reading, the scan and the collection's cached sales are dropped, and the cooldown starts. The module keeps its
last `accepted`, which goes stale, and nobody holds anything whose burn it would refuse.

**One case remains**, and it is the one new wait a holder can see: a graduation that reaches a stale module. `_tryFill` leaves it
`Queued`, and nothing in the stack calls `FloorModule.fill(coin)`. So the canister does:

- **`Step::Fill { collection, coin }`**, a new home act. When an ACTIVE collection's module reads `gradEth() > 0` while its floor is
  fresh, the canister tries candidate coins and sends the permissionless `fill(coin)` to the first whose curve is `Graduating` and
  whose graduation `isFilled` is false. Its effect is read back by `isFilled(coin)`, like every other step (F5-M-1).
  - The candidates are the operator's hand-ins (`eth_fill(collection, coin)`, the coin of the module's `GradQueued` event) and the
    factory's **8 newest** coins, tried at most every 30 minutes.
  - A hand-in is dropped only once the chain shows it filled or its curve no longer `Graduating` (independent review 2, D2-L-2). A
    `fill` whose estimate failed is not taken as a revert: the hand-in is kept and tried again at the next hunt.
  - A flood of coins launched for gas alone can never make the hunt read them all (dormant-review DR-M-2). An older coin is the
    hand-in's, and the monitor pages a graduation left queued for an hour (`eth-grad-queued`).
  - When the curve's window passes unfilled, the curve refunds pro rata: nothing is lost, the launch just did not happen.

## 6. Invariants kept

| Invariant | How it holds |
|---|---|
| **Pin floors, monotonic chain reads** (C4-H-1, E3-M-1) | Every new read goes through `EthIo::home_read`, at this tick's 4663 pin, after `home_floor_ok`. The dormancy pass runs after that and after the pending transaction and the line are resolved. Nothing new reads Ethereum except through the scan, which is unchanged. |
| **`max_response_bytes` set before the transform** (`173470e`) | No new outcall kind and no new transform. The desk and module reads are `eth_call`s through the existing capped reader. The waking scan uses the same size classes and the same per-pass and per-day byte budgets. |
| **Nonces**, one transaction of ours in flight per line | Waking and the dormancy's bookkeeping sign nothing; reading the desk signs nothing. The first post is an ordinary `Step::Post`. The new home acts, `Step::Fill` (§5) and `Step::Served` (`WakeDesk.markServed`, §3.3), go through `send_home` and `resolve_home` like every step: one home transaction at a time, the nonce floor, the saved bytes, and its effect read back (`isFilled`, `servedAt`) when no provider shows it. |
| **H3**: 25 ETH a day, items and mints | Untouched. Dormancy moves no funds and adds no outflow kind. |
| **Staged caps** (steps 1–3) and **the exit rule** vs the 72 h timelock | The allowlist names ten collections. At step 1 the install's `custody_caps` global is **48 items / 75 ETH**, not the sum, so it binds, and `validate_custody` already takes `min(sum, global)`. Only ACTIVE collections can hold items (§5 requires none held before dormancy, and nothing is minted unless ACTIVE), so the custody in force is at most `max_active` collections' worth, within the global. golive-check holds the global to the stage's total and the exit rule on `min(sum, global)`. |
| **Governance through the timelock** | The allowlist (`eth_nft.collections`), the caps (`eth_nft.dormancy`) and the desk's address are in the install argument. Only the runtime phase is canister state. `eth_wake` is an operator hand-in bounded by the same caps, like `eth_deposit`. |
| **Monitor pauses** | The no-good-post pause (`pause_on_no_good_post_h`) and the floor guard skip DORMANT and WAKING collections. A paused `PosterSource` needs the Safe to unpause, and would wedge every future wake (review §2, D-H-1). |

## 7. Cycles (13-node subnet; ×≈ 2.7 for outcalls on 34 nodes; signatures are charged at the key's subnet either way)

Prices: `docs/v7/golive-runbook.md` §6. One provider read reserving 8 kB costs ≈ 0.13 B cycles, 64 kB ≈ 0.72 B and 128 kB
≈ 1.33 B. A `key_1` signature costs 26.15 B.

| Part | Basis | T a day |
|---|---|---:|
| **Idle base** (0 ACTIVE) | the 4663 pin and chain check a tick (≈ 1.7 B), plus the desk read (0.26 B); 48–144 ticks (the runner ticks every 15 min while all are dormant, every 10 min otherwise); the watchdog (negligible) | **≈ 0.1–0.3** |
| **Per ACTIVE collection:** posts | ≈ 36 a day × (26.15 B signature + ≈ 8 B of home reads) | ≈ 1.2 |
| evidence passes | ≈ 36 a day × (≈ 5.5 B listings + ≈ 2.7 B Ethereum pin and finality) | ≈ 0.3 |
| sale scan | each new move's receipt once (unchanged), fewer log reads | ≈ 0.1–0.4 |
| module views, tickets, the re-dormancy reads | 48 views × 11 reads, 144 ticket counts, 4 checks | ≈ 0.2 |
| **per ACTIVE, in all** | | **≈ 1.5–2.2** |
| **One wake**, once | ≈ 101 log reads at 64 kB on two providers (0.15 T), ≈ 300–1,500 receipts at 32 kB on two (0.2–1.1 T), ≈ 10–25 passes of listings (0.1–0.2 T) | **≈ 0.5–1.5** |

| Scenario | T a day | $100 (≈ 74 T) lasts |
|---|---:|---:|
| Idle, 0 ACTIVE | 0.1–0.3 | ≈ 250+ days |
| 1 ACTIVE | 1.6–2.5 | ≈ 30–45 days |
| 2 ACTIVE | 3.1–4.7 | ≈ 16–24 days |
| 3 ACTIVE (`max_active`) | 4.6–6.9 | ≈ 11–16 days |
| Worst case, bounded | `max_active` × 10.5 (the scan's day budget) + the base | ≈ 32 T a day at 3 |

**The earlier estimate** (runbook §6: 6–10 T a day for five always-on collections) counted about 12 signatures a day. Posts
alone are about 36 a day per collection. So always-on, the ten allowlisted collections would cost about 15–22 T a day, which
dormancy avoids.

**Starting balance and top-ups** (replacing the runbook's 250 T, which the $100 budget does not allow). *Superseded by §11.5: the
launch starts at 34 T, sized for one collection, and the limit follows the balance.*

- **60 T in the buyer at the handover** (≈ US$81). The rest of the $100 is a top-up reserve in the operator's cycles ledger
  (≈ 14 T), and 2 T goes to the timelock as `canister-mainnet.md` says.
- The freezing threshold stays at 90 days, which reserves only the idle burn.
- **The top-up page:** the monitor's burn page is unchanged, with `burn_warn_t_per_day` at 10 (about 1.5 times three ACTIVE
  collections) and `min_balance_t` at 10.
- Before `max_active` is raised, the budget must be raised with it. golive-check prints the estimate from `max_active`, and
  fails when the launch file's `cycles.start_t` does not cover `cycles.min_days` at `max_active`.

## 8. Changes

- **Contracts:** `src/floor/WakeDesk.sol` (new) and its tests (`test/wake/WakeDesk.t.sol`). `DeployStack` (NFT mode) deploys the
  desk when `ETCH_WAKE_BOND` is set and writes `wakeDesk` to the stack file.
- **Canister:**
  - `src/dormancy.rs` (new, pure: types, rules, cadence, the ETA);
  - `ethnft.rs`: `EthNftConfig.dormancy` and its validation;
  - `ethflow.rs`: the dormancy pass, the gates, the fresh read, the waking scan's receipt count, the ACTIVE transition on a
    mined post, and the dormancy hooks;
  - `ethnet.rs`: `eth_wake`, `eth_nft_collections`, and `eth_floor_evidence` refusing a dormant collection;
  - the `.did`;
  - tests: `tests/dormant.rs` (the fake world) and unit tests in `dormancy.rs`.
- **Services:**
  - runner: dormant collections get no evidence; waking and hot ones get it every cycle; the evidence goes with a listing;
    `GET /collections`; the idle tick; `/healthz` ignores dormant collections.
  - monitor: the no-good-post pause and the floor, reference and pool checks skip non-ACTIVE collections; a wake over 6 h pages
    SEV2, and a timed-out wake pages SEV2; `candid.mjs` and `argdiff.mjs` word `dormancy`.
  - site: the status and the wake button.
- **Tooling:** `deployments/launch/nft-pilot.json` (ten collections, `dormancy`, `cycles`, the binding global),
  `monitor-nft.json` (ten collections), and `golive-check.sh` (the allowlist, the binding global, `dormancy` and `wake_desk`
  with `--canister`, the desk's read-back, the cycles estimate). The dry run covers the ten.
- **Docs:** this file, and `docs/v7/golive-runbook.md` §6 (the dormant cycles model). The review is
  `docs/audit/2026-09-27-nft/dormant-review.md`.

## 9. Tests (the bar)

- **Forge:** every earlier test, plus `WakeDesk` (bond, listing, one open request, reclaim by a launch, forfeit after the window
  and the grace, both clocks, reentrancy, no owner).
- **Cargo:** every earlier test (with `dormancy = null`), plus the life cycle end to end in the fake world:
  - dormant costs no read;
  - desk, operator, deposit and graduation wakes;
  - every cap;
  - no post before the backfill, then ACTIVE on the mined post;
  - the timeout;
  - the cadence and the back-off;
  - hot near a buy;
  - the fresh read refusing a stale, unconfirmed or mismatched reading;
  - re-dormancy blocked by each open position, then taken;
  - initialization after an upgrade;
  - the ETA.

## 10. As built (2026-09-29, branch `dormant-collections`)

- **Commits:** `9e8b84f` (this design), `ab5a8e7` (`WakeDesk`), `64ac434` (canister), `43a3ae9` (runner, monitor), `3ac84af` (site),
  `6bb54d7` (launch tooling), then the review's fixes: `ef292b0` (canister, monitor), `134a06c` (site), `1e3ef63` (`WakeDesk`),
  `984222c` (launch file, dry run). The review is `docs/audit/2026-09-27-nft/dormant-review.md` (3 High, 4 Medium, 5 Low; all
  fixed but DR-L-2, which DR-H-1's reclaim rule and the watchdog cover).
- **The allowlist** is the ten of §1; CryptoPunks, DeGods, Captainz and Meebits are left out for the reasons there.
- **Tests:** forge 1,157 (14 of them `WakeDesk`), cargo 748; monitor 247;
  ops 73; web-rook 93. The dry run (`DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack`) deploys the ten collections and the
  desk on a local fork and holds golive-check to them.
- **What changed from the design above, by the review:** a bare launch is no position and the idle clock runs from the last
  position (six hours after a wake with none); owed wakes have a reserve slot, back off, and return a deposit after three
  timeouts; the fill hunt reads the newest coins and the operator's `eth_fill`; no fill or buy until the module's accepted floor
  has caught up with our reading; a pass is due 25 minutes before the module's post goes stale; the desk's reclaim needs a
  graduated, item-buying, unspent coin, its window is 7 days, and an unserved request is refunded; the Launch page fails closed on
  the chain's own `fresh()`.
- **Cycles** (§7, runbook §6): idle 0.1–0.3 T a day; one ACTIVE collection 1.6–2.5 T a day, two 3.1–4.7, three 4.6–6.9; a wake
  0.5–1.5 T once. $100 (≈ 74 T) lasts ≈ 250 days idle, 30–45 days at one active, 16–24 at two, 11–16 at three. Start with 60 T;
  the monitor pages under 10 T.
- **Not done here:** nothing is deployed; the wasm is built locally only (not the reproducible build, and not installed).
- **Amended by §11 (auto-scaling, the same day):** `max_active` 10 and `wakes_per_day` 12 are ceilings; the limits follow the cycles
  balance; the launch starts at 34 T, not 60 T.
- **Amended by the independent review, round 1** (`docs/audit/2026-09-27-nft/dormant-review-independent-1.md`; the fixes in
  `dormant-fixes-round1.md`: `4ccb4d6` desk, `4a4a23c` canister and monitor, `6ebf385` site):
  - a deposit or a graduation asks for its owed wake whatever is queued (DI-H-1, §3.1);
  - `WakeDesk.request` refuses while the module is fresh; a bond is charged only for a post inside the window, recorded by
    `markServed`; a refund or reclaim the requester's address does not take is credited for `withdraw` (DI-M-1, DI-M-2, DI-L-2, §3.3);
  - the canister holds each desk request with its window, starts its wake only while it can still post inside it, reads it again
    before the wake, keeps one read while WAKING or ACTIVE, and records the post that served it (DI-M-1, DI-M-2, §3.3);
  - one collection's fill wait no longer stops another's (DI-M-3, §5);
  - the site words an open request from the canister's life and the request's age, offers the wake from the chain when no lives are
    served, and its Docker build takes `VITE_COLLECTIONS_URL` (DI-M-2, DI-L-1, §4.2); `web-rook/` uses npm only (DI-I-1).
- **Amended by the independent review, round 2** (`docs/audit/2026-09-27-nft/dormant-review-independent-2.md`; the fixes in
  `dormant-fixes-round2.md`: `bea711a` desk, `4145059` canister, `3a652b3` site):
  - `markServed` refuses a post later than `at + window` on the timestamp alone, whatever the block clock (D2-L-1, §3.3);
  - the canister gives up a `markServed` or an operator's fill hand-in only when the chain shows it cannot be taken, never on a failed
    estimate alone (D2-L-2, §3.3, §5);
  - a desk request dropped from the queue is worded as the desk decides it: refunded unless another wake posts the collection inside
    its window (the canister's log and note, the site) (D2-L-3, §3.3, §4.2);
  - with no lives served, the page offers the wake only once the module's last post is two hours old, and says what the bond may pay
    for (D2-I-1, §4.2); a new desk request queues at its own read time (D2-I-2, §3.3).

## 11. Auto-scaling from the cycles balance (2026-09-29, approved change)

**The ask.** The operator caps the total launch spend at $100 and does not want the platform limited once volume arrives. `max_active`
(3) and `wakes_per_day` (4) existed only to protect the cycles budget, so both now **follow the canister's own cycles balance**, under
governance ceilings. Nothing else in §1–§10 changes: the life cycle, the desk and its bond, the owed reserve and back-off (DR-H-1,
DR-H-2), the refund of unserved wakes (DR-H-3), the cadence and the fresh read.

### 11.1 The formula (`dormancy.rs`: `effective_max_active`, `wake_fits`, `owed_wake_fits`, `wake_headroom`)

With the scaling's figures (§11.2) in cycles:

```
fixed  = reserve + runway_days × idle                     (held whatever is awake)
slot   = runway_days × active + exit                       (one awake collection: its runway and its exit reserve)
need(n) = fixed + n × slot                                 (the balance that keeps n awake for the runway)

available            = balance − waking × wake             (a backfill under way is not paid yet: counted as unpaid)
effective_max_active = min(max_active, ⌊(available − fixed) / slot⌋), 0 when available < fixed
a wake (not owed)    starts only if available − wake ≥ need(awake + 1)
an owed wake         starts only if awake < effective_max_active + OWED_RESERVE (1)
                     and available − wake ≥ reserve + (awake + 1) × exit             (the runways, never the reserve)
wake headroom        = min(max_active − awake, ⌊(available − need(awake)) / (slot + wake)⌋)
```

- `effective_max_active` is at least 1 exactly when the balance covers one collection's 7 days, its exit reserve and the reserve.
  It is 0 below that: no wake that is not owed starts at all.
- `awake` is ACTIVE + WAKING. The ceilings still apply as before: `max_active` (+ `OWED_RESERVE` for owed wakes), `max_waking`,
  `wakes_per_day`, the cooldown and the owed back-off.
- **Wakes per day scale the same way.** Every wake is gated by the balance after its own cost, so the number of wakes a day is
  what the balance pays for, up to `wakes_per_day`. Several wakes in one tick are each checked against the balance less the ones
  already started.
- **Scaling down never strands anything.** A collection with a position (§5: a unit, an item, a rental, a graduation, an advance or
  a float on 4663, or anything open in the canister) is never put to sleep. It stays awake above the limit and counts against it, so
  while the limit is exceeded no new wake starts. An awake collection that holds nothing at its check (every 6 h) is put to sleep at
  once when more are awake than the limit ("scaled down"), instead of waiting out its idle period. Owed wakes, which carry value
  already at the canister, have one slot above `effective_max_active` (as they had one above `max_active`) and may use the runways but
  never the reserve (review S-1: not one above the ceiling, or a deposit of each collection would wake all ten on a balance that pays
  for one). Nothing is decided on a balance that could not be read: no wake starts and nothing is put to sleep (review S-3).
- All arithmetic is in `u128` cycles from `u64` B-cycle figures; every step saturates or divides by a `slot` the bounds keep
  positive, so nothing overflows or underflows (tested at `u128::MAX` and at the largest configuration the bounds allow).

### 11.2 The figures (`eth_nft.dormancy.scaling`, in the install argument: timelocked)

| Field | Launch | Bounds | Basis |
|---|---:|---|---|
| `active_b_per_day` | 2,500 (2.5 T) | 500..=100,000 | the high end of one ACTIVE collection, §7 (1.5–2.2 T) with room |
| `idle_b_per_day` | 300 (0.3 T) | 0..=10,000 | the idle base's high end, §7 |
| `wake_b` | 1,500 (1.5 T) | 100..=50,000 | one wake's high end, §7 |
| `runway_days` | 7 | 1..=60 | the days each awake collection must be paid for; the monitor pages under 7 days of burn |
| `reserve_b` | 8,000 (8 T) | ≥ 8,000 (the scan floor)..=1,000,000 | under `EVIDENCE_CYCLES_FLOOR` (8 T) the canister stops its own sale scan: no wake can finish its backfill and no buy passes the fresh read (§4.3). What is below it is the wind-down's: posts, deliveries, returns. So the reserve can never be set under it |
| `exit_b_per_awake` | 3,000 (3 T) | 0..=100,000 | one awake collection's exits, returns and owed work once the runway is spent: about two days of posts and passes without the scan (≈ 1.7 T a day), so holders can still burn and leave within the timelock's 72 h less a day, and its deliveries (the binding global of 48 items at ≈ 0.07 T each is ≈ 3.4 T in all); with the 8 T reserve this covers the wind-down of up to 11 awake collections |

`max_active` 10 and `wakes_per_day` 12 are the hard ceilings. `scaling = null` keeps the fixed caps (the behaviour before §11).

At these figures `fixed` = 10.1 T and `slot` = 20.5 T:

| Awake | Liquid balance that keeps them (`need`) | To start the wake that makes it so (`need + wake`) |
|---:|---:|---:|
| 1 | 30.6 T | 32.1 T |
| 2 | 51.1 T | 52.6 T |
| 3 | 71.6 T | 73.1 T |
| 4 | 92.1 T | 93.6 T |
| 10 | 215.1 T | — |

### 11.3 Cycles accounting and gaming

- **The balance** is `ic0.canister_liquid_cycle_balance128` (`ic_cdk::api::canister_liquid_cycle_balance`), read once a tick. It is
  what the canister can spend: the freezing threshold's reserve is already excluded, so the scaling never counts cycles that keep the
  canister from freezing. (The scan floor itself keeps using `canister_cycle_balance`, unchanged.)
- **Nobody can take cycles out.** No method of the canister pays cycles to a caller, and it accepts none from calls; the only cycles it
  attaches are its own fees (outcalls, signatures, and the exchange-rate canister's in `oracle.rs`, a fixed amount per call of its own).
  The IC lets no one withdraw a canister's cycles. The balance falls only by what the canister spends and rises only by a top-up, which
  anyone may make.
- **A top-up then a drain.** A top-up legitimately raises the limit: it pays for the runway of whatever it lets wake. After it, the
  balance can only fall by the canister's own spending, which is bounded as before (the scan's byte budgets, the cadence, the caps).
  A collection woken on a top-up that later holds positions stays awake on the reserve and pages (§11.4); one that holds nothing
  sleeps at its check. A "top-up" cannot be faked: the balance is the IC's own counter.
- **Backfills under way** are counted as unpaid (`available`), so a wake started this tick, or a few ticks ago, cannot be counted twice
  as headroom before its backfill has spent its cost.
- **The estimate, not the measurement.** The runway is priced at the configured burn (the high end of §7), not a burn the canister
  measures, because a measured rate would read one-time wake costs as a daily burn and could be pushed up by a burst. The bounded worst
  case (the scan's day budget, ≈ 10.5 T a day per awake collection) would shorten the 7 days; the monitor's burn-rate page (§11.4) is
  the guard for that, as before.
- **No balance, no wake.** With `scaling` set, a tick that cannot read the balance (never on the IC; a fake world) starts no wake.

### 11.4 What is published and paged

- **`Dorm.cap`** (in `eth_nft_state()`) and the query **`eth_nft_capacity()`** (the balance read now): the collections, the ceiling,
  `effective_max_active`, the awake and the pinned counts (awake with a position), the wake headroom, the day's wakes left, the balance
  at which one more wake starts, and the text "N of M collections can be awake right now (K awake)".
- **The runner** serves `GET /capacity` (the canister's query as of its last pass) beside `GET /collections`.
- **The site's Launch page** shows "N of M collections can be awake right now" beside the floor watch, and, for a collection that is not
  awake, what a wake asked now does (starts, or waits in the queue for room or a top-up).
- **The monitor** shows `eth_capacity` (effective_max_active, the wake headroom, the rest) on `/status`, and pages
  **`eth-capacity-short`** (SEV2, persistent) when `effective_max_active` is below the number of collections with positions: they stay
  awake, no new wake starts, and the page says the balance at which the next wake starts. The burn pages are unchanged; the balance page
  is now 12 T (the 8 T reserve plus one collection's 3 T exit reserve).
- **golive-check** holds the canister's `dormancy` (its scaling included) to the launch file's, and FAILs unless the launch file's start
  (`cycles.startT` less `cycles.freezeT`, the freezing threshold's share) gives `effective_max_active ≥ 1` and lets the first wake start.

### 11.5 The launch balance

**34 T** in the buyer at the handover (≈ US$46 at 1.35 $/T): the first wake needs 32.1 T liquid (1.5 T + 7 × (2.5 + 0.3) T + 8 T + 3 T),
and 1 T is left for the freezing threshold's share (90 days of the idle memory burn, under 1 T for up to about 1 GiB). The rest of the
$100 (≈ 40 T, less 2 T for the timelock) stays in the operator's cycles ledger: each 20.5 T more keeps one more collection awake (plus
1.5 T for its wake).

**Why not 18–20 T.** The request estimated the start at one collection's 7 days (7 × 2.5 = 17.5 T) plus a small reserve. But the
canister stops its own sale scan under 8 T (`EVIDENCE_CYCLES_FLOOR`, round 4): with 20 T, one awake collection reaches that floor in
about 4 days, after which no wake can finish and no buy passes its fresh read. The reserve therefore has to start at 8 T, and one
collection's exits need their own share on top. Lowering the scan floor would be a separate, reviewed change.

**What to expect at launch.** 34 T keeps one collection awake with 7 days of runway only just: after the first wake (≈ 32.5 T left),
`effective_max_active` falls to 0 within about a day of one collection's burn. From then, until a top-up, no new wake starts; the awake
collection stays up while it holds positions, and `eth-capacity-short` pages when it does. That is the intended shape: the platform is
limited by what has been funded, and grows as soon as it is funded.

### 11.6 Tests

- `dormancy.rs` (unit): the formula at the launch figures; a zero and a low balance (no wake, the reserve never spent); a wake at exactly
  its threshold and one cycle below; positions pinning collections above the limit (a desk wake waits, an owed wake uses the runway, not
  the reserve); a top-up raising the limit, with the backfills under way counted; no balance fails closed and fixed caps need none; no
  overflow or underflow at `u128::MAX` and the largest configuration; the bounds, the reserve never under the scan floor.
- `tests/dormant.rs` (fake world): a desk wake held on a low balance until a top-up, at its threshold to the cycle, with the capacity
  published each tick; scaling down puts an idle collection to sleep at its check and never one with a position; positions pin every
  collection awake on a balance that keeps none.
- monitor, runner, site and golive-check: the capacity decoded and paged, `GET /capacity`, the Launch page's line, and the dry run's
  refusals (a canister with fixed caps, one with a lower burn estimate, a ceiling that is not the launch file's, a 20 T start).

