# Rook security review 2026-09-27 (nft-side), area B: the canister's judges, network reads and floor poster

Reviewer B. Written 2026-09-27. Nothing was committed, pushed, deployed or signed, and no key, wallet or identity was touched. In
the main worktree the only changes are this report and `poc/B/audit_nft_b.rs`. The PoCs were written and run in a separate
checkout (`/Users/jlohayon/Documents/Rook-audit-nft-B`, detached at b48b3ba), which was removed afterwards.

The network calls I made were public and read-only:
- `eth_blockNumber`, `eth_getBlockByNumber("finalized")`, `eth_getTransactionReceipt`, `eth_call` (`ownerOf`, `supportsInterface`,
  `royaltyInfo`) and `eth_getTransactionCount`;
- against `eth.drpc.org`, `1rpc.io/eth` (the canister's documented pair, phase1-nft §7) and `xrpc.cl/eth`.

## Scope and commit

- **Checkout:** b48b3ba (branch `nft-side`), diffed against `main` at 47a9b94. That diff includes the `relay-float` merge.
- **Read line by line:**
  - `relayeth.rs`, plus the parts of `relayconv.rs` it reuses (the encoders, `order_id`, `refunded`, `settle_call`, `GAS_KEEP_WEI`,
    `in_band`, `uint_of`);
  - `ethnft.rs` (configuration, listing terms, `judge_listing`, sale parsing, `floor_from`, `should_post`, `buy_cap`, `cost_of`,
    the wrapper ABI, `judge_transfer`; `judge_deposit` only as far as it interacts with my areas);
  - `ethnet.rs` (all of it);
  - `ethflow.rs` (everything except the deposit judge's internals);
  - `nftnet.rs` (`ask`/`one`/`agreed`/`pin_on`/`call_at`/`quantity_at`/`check_any`/`check`, and the `nft_sign` guard);
  - in `seaport.rs`, the new call sites: `verify_terms`, `verify`, `verify_advanced`, `offerer_signed`, `decode_advanced`,
    `encode_advanced`;
  - `chains.rs` (`EvmChain`, `validate`, `fees_and_gas`);
  - `canister.rs` (the diff: `Init.eth_nft`, `validate`, `restore_init`, `caps_of`, the thirteenth saved element, the
    `post_upgrade` guard, the tick wiring, `rpc_once_with`, the transforms);
  - `salenet.rs` `sale_nft_listing`, because it is another EVM signer.
- **Read for their interaction:**
  - `FloorModule.sol`: `postFloor`, `floorBounds`, `mint`, `burn`, `_tierPrices`, `advance`, `settleAdvance`, `returnAdvance`,
    `itemCap`, `maxItemCost`, `fresh`, `escaped`, `_tryFill`;
  - `EthNftWrapper.sol`: `redeem`, `resolve`, `recordDelivery`, `markUndeliverable`, `refund`;
  - `ops/src/ethnft.ts` (what the honest runner hands in);
  - `custodycap.rs`, `outflow.rs` `Limits`.
- **Line numbers** are at b48b3ba. `ethflow.rs`, `ethnft.rs` and `ethnet.rs` are being edited in parallel for D5, so every
  citation also names the function.

## Methodology

1. **Design first.**
   - `docs/v7/phase1-nft.md` §0, §2.1–2.3, §4.1–4.6, §5, §7–§11;
   - `docs/v6/relay-conversion.md` §3.1 (rules 1–12), §5 and §7;
   - the 2026-09-26 audit: `D-canister-timelock.md` (the model), `fixes-D-canister.md`, `R2-canister.md`,
     `fixes-R3-canister.md`, `fixes-R4-canister.md`, `review4/part-canister.md`, `R4-check.md`;
   - the operator's launch decisions D1–D9, as updated on 2026-09-27: the pilot is **multi-collection** (Milady, Azuki and MAYC
     at once) and D5 deposits are **ON**.
2. **Invariants written down** (below), then a line-by-line reading.
3. **PoCs against the repo's own fake world.** `tests/ethnft.rs`'s `Fake` was copied and generalised to two collections with
   their own modules and wrappers. Fault injection was added for:
   - a receipt read that errors once;
   - a sale receipt that errors;
   - a nonce read at a lower pin;
   - a transport error at the transfer simulation;
   - an H3 flow refusal.

   Each PoC asserts the weakness and should turn red once it is fixed.
4. **Live read-only measurement** of the documented provider pair, because the fakes answer every read identically and cannot
   show consensus failures.
5. **IC security best practices:**
   - outcall cycles at the reservation pricing that `canister-mainnet.md` uses:
     `(3,000,000 + 60,000·n)·n + 400·n·request_bytes + 800·n·max_response_bytes`;
   - DoS from untrusted input;
   - **transform determinism**: every replica must see byte-identical transformed bodies, so a supermajority, 2f+1 = 9 of 13
     replicas, must agree;
   - state committed at every await, and "save, then send";
   - upgrade hooks.
6. **Rust:** overflow in release builds (no `overflow-checks`), panics in consensus code (`clamp`, slicing, `expect`), and
   unbounded collections.

## Invariants the code must keep (checked)

| # | Invariant | Status |
| --- | --- | --- |
| E1 | The canister's Ethereum key signs only: our own re-encoding of a judged Seaport fill (value = total ≤ cap), `transferFrom(us, ticket destination, token)` for a resolved ticket, or a 0-value self-send cancel at a delivery's nonce | **Holds** for `ethflow` (`sign_eth`, `cancel`) and `nft_sign` (refused on the eth chain). `sale_nft_listing` still signs Seaport *listings* on that chain; it is inert without an approval (B-I-1) |
| E2 | A Relay deposit only ever pays our own address, in native ETH, on Ethereum (or refunds us), within `relay_max_fee_bps`, never twice for one order | **Holds** (`relayeth::judge` rules 1–12 re-checked; the order hash binds the order we inspect). Fabricated orders pass (B-I-3), bounded by the same rules |
| E3 | A buy costs at most `min(floor × (1 + premium), arrived − 0.003, max_purchase)` and delivers the NFT to us | **Holds** as coded (`buy_cap`, `eth_act`, `nftnet::check`). **But the floor that bounds it can be steered by anyone** (B-H-1) |
| E4 | One unit is minted per NFT that is ours past Ethereum finality | **Holds** (finality, then `ownerOf` = us at a later pin; the wrapper and cap book de-duplicate). Deep-reorg edge: B-I-2 |
| E5 | No LostRace: a redeemer never holds both the NFT and a refund | **Violated** by B-M-3 (an orphaned `Cancelling` delivery that did move reaches the 30-day late window unrecorded) |
| E6 | `markUndeliverable` only when no version of the delivery can ever confirm | **Holds** in intent (nothing marked while a transfer can still mine). **But a transient read failure is taken as "can never succeed"** (B-M-2): a holder who should get the NFT gets a refund |
| E7 | The floor posted is the canister's reading of verified, fillable listings, bounded by real sales; "neither input can be forged, the runner can only withhold" (§4.2) | **Violated**: the sale band *drives* the post (B-H-1); one receipt voids the pass (B-M-1); listings are not required to be fillable by anyone (B-L-1) |
| E8 | One collection cannot starve another (multi-collection launch) | **Violated** (B-H-2, B-M-5, B-L-8, B-L-9) |
| E9 | Nonces: never sign below a nonce already used; one transaction in flight per line | **Violated** when the pin moves backwards (B-M-4) |
| E10 | Replicated outcalls reach consensus with the configured providers | **Violated** in measurement: `strip_volatile` passes bodies through, and `1rpc.io/eth` answers in varying shapes (B-M-6) |
| E11 | Cycles: untrusted parties cannot make the canister spend faster than the operator sees | Holds for strangers (every Ethereum read is operator-triggered). **The operator's default cadence alone is heavy** (B-M-7) |
| E12 | Pause (H1) stops every signing path; endpoints are operator-only, not paused and not mid-tick; upgrades keep state | **Holds**: `canister::tick` skips the eth side while paused; the 5 endpoints have `only_operator` / `not_paused` / `not_ticking`; the thirteenth element is `opt`; `post_upgrade` still traps on a decode error. Consistency gaps: B-L-4, B-L-5 |

## Findings

| ID | Severity | Title | Where (b48b3ba) | Reachable in launch config? | PoC |
| --- | --- | --- | --- | --- | --- |
| B-H-1 | **High** | Anyone can steer the posted floor: self-trades as the "latest transfer txs" drive `P` through the sale band, above or below every real listing, bounded only by the module (±5 %/h, ±20 %/day) and D8 | `ethnft.rs:507-542` `floor_from` (clamp `:533-536`); `ethnft.rs:403-429` `seaport_sale`, `:448-477` `sales_in`/`sale_prices`; runner `ops/src/ethnft.ts:179-193` `recentTransferTxs` | Yes (D7 defaults) | `audit_nft_b_h1_*` (2 tests) **pass** |
| B-H-2 | **High** | Cross-collection: any refusal in `home_act`'s buy or deposit steps returns before the floor posts, so every collection's module goes stale (mints, burns and graduations refused). A deposit into a capped collection does it for as long as the cap is full; the day's global mint limit does it for up to 24 h | `ethflow.rs:1038-1206` `home_act` (`?` at `:1074`, `:1107-1108`, `:1179-1182`; posts at `:1189-1204`) | Yes (D5 ON; multi-collection) | `audit_nft_b_h2_*` (2 tests) **pass** |
| B-M-1 | Medium | Anyone can freeze a collection's floor: one unreadable or oversized sale receipt voids the whole evidence pass; 12 plain transfers crowd every sale out | `ethflow.rs:510-557` `floor_pass` (`?` at `:534`); `ethnet.rs:286-291` `eth_sale_receipt` (200 kB cap) | Yes | `audit_nft_b_m1_*` **passes** |
| B-M-2 | Medium | Any transport or consensus failure of the transfer's simulation is classified "refused (simulation)", and the delivery is marked undeliverable at once, days before its deadline | `ethnet.rs:109-121` `EthNet::fees` (`map_err` `:112-113`); `ethflow.rs:837-847` `eth_act` | Yes (and likely: B-M-6) | `audit_nft_b_m2_*` **passes** |
| B-M-3 | Medium | `superseded` drops the line before its read-back; one read error or a lagging index orphans the delivery in `Cancelling`/`Sent` forever. If the transfer did move, it is never recorded, so the redeemer refunds after 30 days while keeping the NFT (LostRace) | `ethflow.rs:937-977` `superseded` (`st.line = None` `:938`; `?`/`Err` at `:944`, `:952`, `:958`, `:964`, `:966`) | Yes (needs a read failure at that tick) | `audit_nft_b_m3_*` **passes** |
| B-M-4 | Medium | The nonce is read at the tick's pin, which can move backwards. A transaction signed at a regressed pin reuses a mined nonce: a delivery is stranded in `Sent` (never retried or marked); a fill leaves the buy `Stuck` with no exit, halting all buys and leaking its custody reservation | `ethnet.rs:109-121` `fees`, `nftnet.rs:253`; `ethflow.rs:937-977` `superseded`; `nftnet.rs:82-94`/`pin.rs:54-61` `choose_pin` | Yes (a provider head straddling a bucket between two ticks) | `audit_nft_b_m4_*` (2 tests) **pass** |
| B-M-5 | Medium | Cross-collection: a collection at its custody cap that still wants blocks every other collection's buys (`buy_pass` returns on its refusal and never advances the round robin). One `Arrived`/`Stuck` buy blocks all collections and has no exit | `ethflow.rs:577-612` `buy_pass` (`:594-598`, `:603`); `ethflow.rs:267-269` `open_buy`; `ethflow.rs:1330-1343` `give_back_now` | Yes: Azuki wants 4 with a cap of 3 at a 5 ETH raise (§Q1) | `audit_nft_b_m5_*` (2 tests) **pass** |
| B-M-6 | Medium | `strip_volatile` passes provider bodies through unnormalised. Measured, `1rpc.io/eth` answers one `eth_call` in two field orders plus rate-limit, "discontinued" and HTML bodies, so replicated reads miss the 9-of-13 quorum | `canister.rs:693-695` `strip_volatile`; `nftnet.rs:71-73` `agreed` and every `call_at`/`quantity_at`/`eth_estimateGas` | Yes (the documented pair) | Live measurement (below) |
| B-M-7 | Medium | Cycles: one evidence pass reserves ≈ 72 B cycles per collection (12 × 200 kB receipts, 96 × 16 kB calls). At the runner's 20-minute cadence that is ≈ 5.2 T/day per collection on 13 nodes; 3 collections ≈ 15.6 T/day (≈ 42 T on 34 nodes) | `ethnet.rs:286-291`, `:260-285`; `ethflow.rs:510-557`; `ops/src/ethnft.ts:239-253` | Yes | Computed; receipt measured at 4,161 B vs 200,000 reserved |
| B-L-1 | Low | Evidence listings need not be fillable by anyone: zone and order type are not checked, so a restricted order behind any zone (or a contract order) counts, and creator-fee listings the buy judge refuses still count | `ethnft.rs:200-245` `terms`, `:272-292` `judge_listing`; `ethnet.rs:260-285` `eth_listing` | Malicious runner; OpenSea-indexed only if OpenSea accepts the zone | Reasoned |
| B-L-2 | Low | Deliveries are signed from `ticket(i)` at the 4663 fast pin with no 4663 finality: a departure from H9 / fixes-D H-1 ("a reorganisation can no longer un-redeem the ticket under a signed delivery") | `ethflow.rs:469-506` `scan_tickets`, `:812-855` `eth_act` | Needs a 4663 reorg past the fast pin | Reasoned |
| B-L-3 | Low | An unmined fill blocks the one Ethereum line: no delivery of any collection is signed or marked (eth_act does not run), and there is no cancel or replacement for a buy | `ethflow.rs:434-441` `tick`; `:884-910` `resolve_eth` (cancel only for `Deliver`) | Yes (a base-fee spike past `max_fee`) | Reasoned |
| B-L-4 | Low | While paused the whole eth side is idle: a delivery that already moved is not recorded and a pending one cannot be cancelled. That differs from R3's M-1(c) rule for Bitcoin; a pause longer than deliverBy + 30 days allows a LostRace | `canister.rs:1345-1355` `tick` | Only for very long pauses | Reasoned |
| B-L-5 | Low | Upgrade and config hazards: dropping `eth_nft` with a home transaction pending blocks every other flow's home transactions for ever; removing a collection with an open buy wedges all buys; an explicit `custody_caps` without an eth collection is not refused at validation | `canister.rs:260-270` `caps_of`, `:297-309` `validate`, `:425-432`; `ethflow.rs:866`, `:1071` | On a later upgrade | Reasoned |
| B-L-6 | Low | Relay refunds are detected from the shared 4663 balance: a frog-side advance (relay-float) landing there reads as an eth-side refund and is re-bridged, and vice versa | `ethflow.rs:637-651` `step_buy`; `relayconv.rs:219-221` `refunded`; `convflow.rs:135` | No (NFT-only stack); yes if both sides run | Reasoned |
| B-L-7 | Low | Rule 8 allows refunds on Ethereum, with refund `minimumAmount` unchecked (the real quote says `"0"`). A refund on Ethereum below `min_out` is neither an arrival nor a refund: `Stuck`, no operator exit, all buys halted (B-M-5) | `relayeth.rs:158-178` `judge`; `ethflow.rs:625-669` `step_buy` | Depends on Relay's choice | Reasoned |
| B-L-8 | Low | Cross-collection: `eth_act` returns on the first delivery's H3 refusal or read error, so one collection's day limit blocks other collections' deliveries and the buy fill | `ethflow.rs:820-821`, `:849` `eth_act` | Needs > 3 deliveries per collection or > 6 in all per day | Reasoned |
| B-L-9 | Low | Cross-collection: posting capacity is one home act per tick, shared with buys, records and marks. 3 collections need about 6 posts/h against about 9 ticks/h, so bursts let a module lapse past `FLOOR_TTL` (1 h) | `ethflow.rs:1189-1204`; `ops/src/runner.ts:43` (600 s) | Yes (tight margin) | Reasoned |
| B-I-1 | Info | `sale_nft_listing` signs Seaport listings of the canister's NFTs on the eth_nft chain (any collection, any ask). Inert without a `setApprovalForAll`, but it contradicts "the canister never lists an NFT" | `salenet.rs:77-111` | — | — |
| B-I-2 | Info | `eth_finalized`: the minimum over providers, with no bound against the head (one provider's stale value is a liveness DoS). `Bought{block}`/`Moved{block}` are not re-checked after finality (a reorg of more than 8 blocks) | `ethnet.rs:193-201`; `ethflow.rs:670-684`, `:698-709` | — | — |
| B-I-3 | Info | Relay: a runner-fabricated order that passes rules 1–12 (solver, salt, version unpinned) is only griefing (funds wait on Relay's manual refund). Deadlines are `as u64`-truncated from u128; refund `minimumAmount` is unchecked; `requestId` is dropped | `relayeth.rs:171`, `:208`, `:216` | — | — |
| B-I-4 | Info | Unbounded vectors: `buys`, `deliveries`, refused `deposits`, `used_orders`. `eth_nft_state()` returns all of them | `ethflow.rs:242-258` | — | — |
| B-I-5 | Info | `depth` counts every verified listing, including one offerer's several and duplicate hand-ins | `ethnft.rs:539` | — | — |
| B-I-6 | Info | The advance's leftover (`arrived − paid`, about 2.5 % of each price, more under an inflated floor) accumulates on Ethereum with no return path (D4). Module revenue/surplus, not recoverable | `ethflow.rs:1080-1084`; FloorModule `advance`/`settleAdvance` | — | — |

**Counts: 0 Critical, 2 High, 7 Medium, 9 Low, 6 Info.**

---

### B-H-1 (High): anyone can steer the posted floor through the sale evidence

**Description.** `floor_from` computes `P = clamp(L, S25 × (1 − down_cap), S_med × (1 + up_cap))` (`ethnft.rs:533-536`).

The design reads this as "listings set the floor; sales only bound it". But whenever `L` lies outside the band, it is the
*sales* that set `P`:
- with sales at `X`, `P ∈ [0.85·X, 1.1·X]` whatever the listings say;
- the confidence is 5,000 when clamped (`:538`), above the module's `minConfidenceBps` of 3,000, so the module accepts the post
  and marks it good (`lastGoodAt`).

The sales are whatever receipts are handed in:
- `sales_in` accepts any Seaport 1.5/1.6 `OrderFulfilled` or Blur v2 execution of one token of the collection
  (`:403-470`);
- it does not check that buyer and seller differ, or anything else about who traded.

The honest runner hands in "the last `n` distinct transactions that moved a token of the collection" over 7,200 blocks
(`ops/src/ethnft.ts:179-193`, called with `n = 12` at `:248`). So whoever made the latest 12 transfers chooses `S`.

A self-trade on Seaport costs only gas:
- the offerer and the fulfiller are the same address, so the ETH goes from the attacker to the attacker;
- the attacker needs one NFT (reusable) and `X` of liquidity;
- about 150–185k gas, ≈ 2·10⁻⁵ ETH at today's ≈ 0.1 gwei.

Blur v2 wash trades cost the same.

**The design's own claims do not hold:**
- "wash sales cannot lift it above real listings": the lower clamp lifts `P` to `0.85 × S25`, above every listing;
- "the runner can only withhold": it chooses the sales, and so does anyone who controls the latest 12 transfers.

**Impact** (per collection, stage 0):

1. **How far it moves.** The module moves the accepted floor at most 5 %/h (2.5 % per 30-minute post) and 20 % from the day's
   anchor. D8's monitor pauses the keeper at more than 20 % off both OpenSea and CoinGecko, and wash sales do not move those
   references. So an attacker holds `accepted` at up to about ±19 % indefinitely; it takes about 4–5 hours to get there.
2. **Down (cheap mints).** Public mints are capped at `maxPublicEthBacked` 10 units and 5 an hour. At −19 % that is up to
   10 × 0.19 × floor: ≈ 1.8 ETH on Milady (0.93), ≈ 1.3 on Azuki, ≈ 1.7 on MAYC. This is the module's documented
   "compromised keeper" bound, now open to anyone. It is realised through redemptions (≤ the 3 items held per collection)
   or pool sales.
3. **Down (buys stall).** At the same time every advance ends `Arrived` with no fillable listing: the cap is
   ≈ 1.0236 × 0.81 × floor, under the whole book. The ETH is stranded (D4), and every collection's buying stops (B-M-5).
4. **Up.** Burns cannot overpay (they are capped at the backing per unit). But:
   - the advance is `1.03 × accepted`, so an honest runner's buy at the real floor leaves about 0.19 × floor per buy stranded
     on Ethereum (≈ 0.18 ETH per Milady buy, up to 3 buys a day);
   - with a malicious runner and a seller who lists at the cap, the canister pays up to ≈ 1.22 × the real floor.

**PoC** (`poc/B/audit_nft_b.rs`):
- `audit_nft_b_h1_the_sale_band_drives_the_post_away_from_every_real_listing`: with listings at 0.92/0.93/0.95, three sales at
  1.5 ETH give `P = 1.275` ETH, and sales at 0.093 ETH (not "dust") give `P = 0.1023` ETH. Both have confidence 5,000.
- `audit_nft_b_h1_self_trades_handed_in_as_the_latest_transfer_txs_set_the_modules_floor`: 12 self-trade `OrderFulfilled`
  receipts (offerer = recipient = attacker) as the runner would hand them in. The canister signs
  `postFloor(1275000000000000000, 5000)` and `postFloor(102300000000000000, 5000)`. **Passes.**

**Recommended fix** (canister-side, because the runner is not trusted):
- Sales should only *veto*, never set: post `L` if it lies within the band, otherwise **no post** (or a post at `L` with low
  confidence). Never post the band edge.
- Count a sale only if:
  - buyer ≠ seller (the `OrderFulfilled` recipient ≠ the offerer; Blur's trader ≠ `tx.from`);
  - the price is within, say, ±30 % of `L`;
  - at most one sale per token and per seller per window.
- Take the median over distinct (buyer, seller) pairs, and require ≥ 3 distinct sellers.
- Better: the canister chooses the sample itself. Read `OrderFulfilled`/`Execution721*` logs for the collection over a fixed
  window with `eth_getLogs` (topic-filtered), not hashes the runner picks.
- The runner should hand in sales from a marketplace sales feed, not raw Transfer transactions. This is not sufficient alone,
  since wash trades appear there too.

### B-H-2 (High): one refusal in `home_act` starves every collection's floor posts

**Description.** `home_act` (`ethflow.rs:1038-1206`) runs its steps in order and returns on the first `Err`:
1. records and marks;
2. the open buy;
3. deposits' mints;
4. floor posts (`:1189-1204`).

Several refusals in steps 2 and 3 use `?`:
- `outflow_check(Flow::Mint)` for a `Final` buy (`:1074`);
- `outflow_check(Flow::Value)` and `custodycap::reserve` for `NeedAdvance` (`:1107-1108`). `buy_pass` only checked
  `check_acquire(…, 0)`, so a value cap can still refuse here;
- `custodycap::admit` and `outflow_check(Flow::Mint)` for a deposit in `XPhase::Mint` (`:1179-1182`);
- `c.ok_or(…)` for a collection no longer configured (`:1071`, `:1176`).

When one of them fails, step 4 is never reached, **for any collection**. Nothing retires the blocking record:
- a deposit that cannot be admitted stays `Mint` and fails again every tick;
- a `Final` buy over the H3 limit waits for the window to roll.

`FLOOR_TTL` is 1 hour. After that each module is not `fresh()`:
- `mint` reverts `Stale`;
- `burn` reverts `Stale` (`_tierPrices`) until `STALE_ESCAPE`, **7 days**;
- `_tryFill` declines graduations.

**Who can trigger it (launch config):**
- **Anyone, with D5 deposits ON.**
  1. Deposit one NFT into a collection already at its stage-0 custody cap (3 items). Azuki reaches it from a single 5 ETH
     graduation (§Q1).
  2. The deposit verifies and goes final, then `admit` refuses `CollectionItems` every tick.
  3. Every collection's posts stop until an item of that collection is redeemed and delivered. The depositor then gets their
     unit, so this costs them nothing.
  4. They can repeat it.
- **Ordinary operation.** H3's `global_mints` is 6 a day across all collections and deposits
  (`outflow.rs:105-108`). The 7th mint of the day (3 collections × up to 3 buys a day, plus deposits) stops every
  collection's posts for the rest of the 24 h window.

**Impact.** All modules stale:
- no mints or burns for up to 7 days, so holders cannot exit through the module;
- graduations wait.

Redemption via the wrapper still works. The operator cannot clear it: the deposit switch and caps are timelocked upgrade
arguments (72 h), and no endpoint removes a deposit record.

**PoC:**
- `audit_nft_b_h2_one_deposit_into_a_capped_collection_stops_the_floor_posts_of_every_collection`: Azuki full, one Azuki
  deposit, then Milady's due post is never sent in an hour of ticks with fresh evidence. The control posts at once.
- `audit_nft_b_h2_the_days_global_mint_limit_reached_by_one_collection_stops_the_other_collections_floor_posts`: Milady's
  `Final` mint refused by H3, and Azuki's post is never sent. Clearing the limit posts it.

**Both pass.**

**Recommended fix:**
- Make each step of `home_act` independent: collect the refusals, and never let one record block another kind of act.
- Give floor posts their own slot, one per tick at least every N ticks, before optional acts.
- Refuse a deposit claim up front when the collection has no cap room (in `deposit_pass`, before `Seen`), rather than parking
  it in `Mint`.
- Check `reserve(amount)` in `buy_pass`, not `check_acquire(0)`.

### B-M-1 (Medium): anyone can freeze a collection's floor through the evidence

**Description.** `floor_pass` reads each handed-in sale receipt with `io.eth_sale_receipt(*hsh).await?` (`ethflow.rs:534`).
One failure voids the whole pass: no reading, no post. `eth_sale_receipt` reserves 200,000 bytes and requires both providers
to agree (`ethnet.rs:286-291`). So any one of these fails it:
- a receipt over 200 kB;
- a provider lagging by a block;
- a consensus failure (B-M-6).

With the honest runner's "latest 12 transfer txs" there are two cheap attacks:
1. **One padded transaction.** A transaction that moves one token and emits about 100 kB of log data (≈ 0.9M gas, ≈ 10⁻⁴ ETH)
   voids every pass while it stays among the latest 12.
2. **Crowding out.** 12 plain transfers between the attacker's wallets leave 0 sales, and `min_sales = 3` fails.

**Impact.** No post, so the module goes stale (as in B-H-2, but for one collection). The canister's own reading also goes stale,
so `buy_cap` errors: an `Arrived` buy waits and blocks all collections (B-M-5).

The runner can be changed off-chain to mitigate this (skip unreadable receipts; pick sales from a feed), hence Medium, not High.

**PoC.** `audit_nft_b_m1_one_oversized_or_unreadable_sale_receipt_voids_the_whole_evidence_pass`: five good sales and three
listings plus one failing receipt give no reading and no post; 12 plain transfers give "0 verified sales". **Passes.**

**Fix:**
- Treat a failed receipt as "not a sale" (count it, log it), never abort the pass.
- Cache receipts that are already final by hash: they never change.
- Reserve by size class (e.g. 32 kB first, a larger retry only if needed).
- Combine with B-H-1's canister-side sampling.

### B-M-2 (Medium): a transient read failure marks a delivery undeliverable

**Description.** `EthNet::fees` maps *any* error of the simulation `eth_call` to
`"refused (simulation): it reverts at the pin: {e}"` (`ethnet.rs:112-113`). That covers:
- an HTTP 429;
- a replica consensus failure;
- the providers disagreeing (`nn::one`);
- a provider down.

`eth_act` treats that string as "the collection's or the destination's rule: never sent, never will be" and moves the delivery
straight to `DPhase::Mark` (`ethflow.rs:841-844`). The same tick's `home_act` sends `markUndeliverable`.

**Impact:**
- The holder asked for an NFT that the canister holds and could deliver. After one flaky outcall they get a refund at
  `deliverBy + 72 h` instead, 10 days later, and the NFT goes back to the set.
- With the measured behaviour of `1rpc.io/eth` (B-M-6) this is **likely**, not rare.
- The deliveries' promise ("delivered, or refunded only when it can never confirm") silently degrades.

**PoC.** `audit_nft_b_m2_a_transport_error_at_the_transfer_simulation_marks_the_ticket_undeliverable_a_week_early`: a
7-days-early ticket is marked in one tick after one "SysTransient: HTTP 429". **Passes.** (The fake returns the exact text
`fees` produces; the mapping itself is code at `ethnet.rs:112-113`.)

**Fix:**
- Tell a revert apart from a transport failure. `rpc_once_with` already separates `no result: {error}` (a JSON-RPC error from a
  provider) from an outcall `Err`; require every provider to return an `execution reverted` error with agreement.
- Require the refusal on N ticks spanning at least an hour before marking.
- Keep `Mark` for the deterministic cases (relist, within a day, a final revert).

### B-M-3 (Medium): a read error while superseding orphans the delivery (LostRace)

**Description.** `superseded` begins with `st.line = None` (`ethflow.rs:938`) and then reads receipts. Every failure path after
that returns `Err` with the line already gone:
- `io.eth_receipt(of).await?` (`:944`);
- "neither … visible: waiting rather than guessing" (`:952`);
- `io.eth_receipt(tx).await?` (`:958`);
- "not visible yet" (`:964`);
- "nonce used by a transaction that is not ours" (`:966`).

After that:
- `resolve_eth` never runs again for it (there is no line);
- `delivery_pass` only reads the ticket's stage for `Waiting`/`Sent`/`Cancelling` (`:718-726`);
- `eth_act` only picks `Waiting` (`:817`).

So the delivery stays `Cancelling` or `Sent` for ever. Nothing records it or marks it.

**Impact.** In the transfer-won-the-race case, the NFT is at the destination but `recordDelivery` is never sent.
`EthNftWrapper.refund` opens at `deliverBy + LATE_DELIVERY_WINDOW` (30 days) without a mark, and it mints the redeemer one unit
back. The redeemer ends with the NFT **and** the unit: the LostRace the design rules out (§2.2), and one unit unbacked.

The trigger is one read failure (B-M-6), a provider disagreement, or a receipt index lagging the nonce, at the one tick that
sees the nonce move.

**PoC.** `audit_nft_b_m3_a_read_error_while_superseding_orphans_a_delivery_that_did_move_so_the_redeemer_keeps_nft_and_refund`:
1. cancel signed;
2. the transfer mines;
3. its receipt read fails once;
4. 40 days of ticks follow.

Result: the delivery is still `Cancelling`, the NFT is owned by Alice, the ticket is still `Resolved`, and nothing is marked.
**Passes.**

**Fix:**
- Clear the line only after the outcome is known; on any error keep it and retry next tick.
- Give `delivery_pass` a watchdog: a `Sent`/`Cancelling` delivery with no line re-reads its hashes.
- Add an operator endpoint that re-reads a delivery.

### B-M-4 (Medium): nonces read at a pin that can move backwards

**Description.** Plans read the nonce with `eth_getTransactionCount` at the tick's pin (`ethnet.rs:119` `fees`,
`nftnet.rs:253` `check`). The pin is `min(heads rounded to 8) − 8` (`pin.rs:54-61`). It moves backwards between two ticks when
the lower provider's head drops into a lower bucket. That is common with load-balanced endpoints whose back ends differ by a
few blocks, and the runner "ticks again at once" after hand-ins.

No persistent "next nonce" is kept. So:
1. a transaction mined just under the old pin clears the line;
2. the next signing at the regressed pin reuses that transaction's nonce;
3. the reused-nonce transaction can never mine;
4. `superseded` then leaves it orphaned.

**Impact:**
- **A delivery** stays in `Sent` with "not ours: look" and is never retried, cancelled or marked. The holder waits for the
  30-day window.
- **A fill** leaves the buy `Stuck` with `last = None`:
  - `give_back_now` refuses it;
  - the arrived ETH is stranded;
  - no buy of any collection starts again (`open_buy`);
  - `eth:{id}` stays `in_flight` in the cap book (there is no stale-reservation sweep, unlike L-2's fix for frogs).
- The 4663 line has the same shape: `resolve_home` waits for ever on "nonce is used but … not visible" (`:1216-1218`). That
  blocks every flow's home transaction through `evm_pending`.

**PoC:**
- `audit_nft_b_m4_a_nonce_read_at_a_lower_pin_strands_a_delivery_with_no_mark_and_no_retry`;
- `audit_nft_b_m4_the_same_for_a_fill_strands_the_arrived_eth_and_every_later_buy`.

**Both pass.**

**Fix:**
- Keep `EthState.next_nonce = max(nonce at pin, last signed + 1)` and sign at that.
- Or refuse to sign while the pin is below the block of our last mined transaction.
- Settle stale `eth:{id}` reservations of `Stuck` buys.
- Add an operator exit for `Stuck` and `Arrived` buys, bounded by H3.

### B-M-5 (Medium): cross-collection buy blocking

**Description:**
1. **The round robin halts at the first refusal.** In `buy_pass` (`ethflow.rs:577-612`), for each collection in round-robin
   order:
   - a module read error `return`s (`:594-598`);
   - `check_acquire(...)` refusals propagate with `?` (`:603`).

   `st.rr` only advances when a buy is created. So a collection whose module still wants an item but is at its cap
   (per-collection 3, global 6), or that is not listed in an explicit `custody_caps`, stops every later collection, every
   tick.
2. **One open buy, canister-wide** (`open_buy`, `:267-269`). An `Arrived` buy with no listing under its fixed `arrived − 0.003`
   cap blocks every collection. So does a `Stuck` buy without a watch. `give_back_now` refuses both (`:1330-1343`). The cap
   margin is only about +2.2–4.4 % over the floor at the advance (§Q1), so a rise of that much during the Relay leg is enough.

**Impact.**
- At a 5 ETH raise, Azuki's module wants 4 units and the per-collection cap is 3 (§Q1). After its third buy, whenever it is
  first in the round robin, Milady's and MAYC's buys never start.
- D4 ("wait, page at 24 h") was decided for Milady alone. With three collections it halts all buying.

**PoC:**
- `audit_nft_b_m5_a_collection_at_its_custody_cap_that_still_wants_blocks_every_other_collections_buys`: 20 ticks, no Azuki
  buy; the control starts it at once.
- `audit_nft_b_m5_one_arrived_buy_that_cannot_fill_blocks_every_collection_and_has_no_exit`: five hours, no Azuki buy, and
  `give_back_now` refused.

**Both pass.**

**Fix:**
- In `buy_pass`, `continue` on a refusal (log it) and advance `rr`.
- Skip collections whose `want` exceeds their remaining cap.
- Allow one open buy per collection: arrival is a balance rise, so key arrivals by Relay order and require the order's
  `min_out` to be matched per buy.
- Or add an operator exit for `Arrived` (a bounded re-bridge, the D4 reverse leg) and a cap re-evaluation at fill time.

### B-M-6 (Medium): `strip_volatile` bodies versus the documented providers

**Description.**
- `nn::agreed` (`nftnet.rs:71-73`) uses the `strip_volatile` transform (`canister.rs:693-695`). It drops the headers and
  returns the provider's body **byte for byte**.
- Every Ethereum state read goes through it: `eth_call` (listings, `ownerOf`, simulations, Seaport code/`information`),
  `eth_getBalance`, `eth_getTransactionCount`, `eth_chainId`, `eth_estimateGas`.
- IC consensus needs 2f+1 identical transformed bodies (9 of 13 replicas).

**Measured 2026-09-27** (13 sequential identical requests each, at a pin-depth block):

| Provider | `eth_call ownerOf(9180)` at head − ≈16 | `eth_getTransactionCount` |
| --- | --- | --- |
| `eth.drpc.org` | 13 × `{"id":1,"jsonrpc":"2.0","result":…}` | — |
| `1rpc.io/eth` | 8 × `{"jsonrpc":"2.0","id":1,"result":…}`, **3 × `{"jsonrpc":"2.0","result":…,"id":1}`**, 1 × Nodies rate limit, 1 × "This endpoint has been discontinued" | 9 × one shape, 1 × another order, 1 × OnFinality 429, 1 × discontinued, 1 × nginx 301 HTML |
| `xrpc.cl/eth` | — | 13 × identical |

At a block about 3,000 behind the head, `1rpc.io/eth` also answered "historical state … is not available", "Archive requests
require a personal token" and `{"result":{"code":503,…}}`. The last is a `result`, so `rpc_once_with` accepts it as data.

**Impact.**
- On IC replicas (13 separate IPs at once) the three body shapes alone keep `1rpc` from reaching 9 identical bodies on a large
  share of calls. Every such read errors.
- Liveness only, for buys, evidence and deliveries, **but** it feeds B-M-2 (false marks) and B-M-3 (orphans).
- Separately, `1rpc`'s answer that the endpoint is "discontinued" suggests the provider is being retired.

**Fix:**
- Replace `strip_volatile` for JSON-RPC reads with a normalising transform: parse, then emit `{"result": <canonical>}` or a
  canonical `{"error": {"code": c}}` with no message. The receipt and block transforms already do this.
- Re-select the pair: `xrpc.cl/eth` answered identically 13/13, and so did `drpc`.
- Add a go-live check that issues 13 back-to-back reads per method and requires identical bodies.

### B-M-7 (Medium): cycles of the evidence pass

Each hand-in costs, per collection:
- 8 listings × 6 `eth_call`s (`getCounter`, `getOrderStatus`, `ownerOf`, `getConduit`, `isApprovedForAll`, `getApproved`) × 2
  providers at 16 kB;
- 12 receipts × 2 providers at 200 kB.

At `(3M + 60k·n)·n + 800·n·max_bytes` that is ≈ 0.217 B cycles per 16 kB call and ≈ 2.13 B per 200 kB receipt on 13 nodes: **≈
72 B per pass** (≈ 193 B on 34 nodes).

The runner hands in every 20 minutes per collection (`evidenceEveryS = 1,200`), so:
- ≈ 5.2 T/day per collection (13 nodes);
- ≈ **15.6 T/day for Milady + Azuki + MAYC**;
- ≈ 42 T/day on a 34-node subnet;
- plus about 2 T/day of module reads (11 reads × 2 providers per collection per tick in `buy_pass`).

A real Milady sale receipt measured **4,161 bytes** against 200,000 reserved. The earlier audits' budget was about 20 T.

**Fix:**
- Cache final receipts by hash (they never change).
- Reserve about 16 kB, with an explicit larger retry.
- Re-verify a listing only when its (hash, pin bucket) changed.
- Lower the evidence cadence to what `should_post` needs.
- Add an in-canister cycles floor for the evidence pass (as H-3's 8 T floor for deposits).

(If the subnet bills actual response bytes, the receipt part falls by about 48×. The canister is still cycle-hungry on listings.)

### Lows

**B-L-1: evidence listings are not required to be fillable.**
- `terms`/`judge_listing` check neither `zone` nor `order_type`. The evidence path therefore accepts:
  - a restricted order behind any zone (which can refuse every fulfiller);
  - a contract order (type 4);
  - a listing whose creator-fee recipient the buy judge would refuse (none of Milady, Azuki or MAYC implements ERC-2981,
    measured).
- A malicious runner, or a seller if OpenSea indexes such orders, can pull `L` down without risking the NFT. It stays bounded
  below by `0.85 × S25`.
- **Fix:** require `order_type ∈ {0, 1}`, or `{2, 3}` through an allow-listed zone, as the buy judge does. Also apply the buy
  judge's recipient rules to evidence.

**B-L-2: no 4663 finality for tickets.**
- `scan_tickets` and `eth_act` deliver on `ticket(i)` read at the fast pin.
- The Bitcoin side requires final `Redeemed`/`Resolved` events (fixes-D H-1 point 2, the H9 goal). The eth side does not.
- A 4663 reorg past the fast pin, after the Ethereum transfer is signed, could un-burn the redeemer's wX.
- **Fix:** require `ticket.deliverBy`'s block ≤ finalized (or the `Resolved` event final) before signing.

**B-L-3: an unmined fill blocks the line.**
- A buy fill can sit unmined if the base fee rises past `max_fee` (base × `FEE_MULTIPLE` + tip). The measured base-fee maximum
  was 22× the median in 1,024 blocks.
- While it sits there, `eth_act` does not run (`tick:434`):
  - no delivery of any collection is signed;
  - a `Waiting` delivery is not even marked at `deliverBy − 1 day`, so holders wait 30 days.
- `resolve_eth` cancels only `Deliver`.
- **Fix:** a same-nonce replacement or cancel for fills after N minutes; and let the mark step run with the line busy.

**B-L-4: pause semantics.**
- `canister::tick` skips `ethnet::tick` entirely while paused (`canister.rs:1345-1355`).
- R3 §2.3 allowed recording a confirmed delivery and conflicting out while paused, for Bitcoin. On the eth side:
  - a delivery that already moved waits unrecorded;
  - a pending one cannot be cancelled.
- A pause longer than deliverBy + 30 days re-opens the LostRace.
- **Fix:** a restricted eth pass while paused (resolve the line, record final deliveries, cancel at the deadline).

**B-L-5: upgrade and config hazards.**
- `ethnet::evm_pending()` gates the other flows (`canister.rs:1314`, `1326`, `1337`). With `eth_nft = None` the eth tick never
  runs, so a pending home transaction is never resolved and blocks every other flow for ever. `post_upgrade` checks only the
  Ethereum line (`:425-432`), not `pending`.
- Dropping a collection with an open buy gives "no longer configured" every tick, blocking all buys (B-M-5).
- `validate` does not require that an explicit `custody_caps` lists every eth collection.
- **Fix:** extend the `post_upgrade` guard to `pending` and open buys; validate the cap coverage.

**B-L-6: shared 4663 balance.**
- `refunded(home, balance_after, value)` (`relayconv.rs:219-221`) is used by both `step_buy` and `convflow.rs:135`. A frog-side
  advance (≥ value/2) landing during an eth `Bridging` reads as a refund, and `back` is re-bridged with the frog side's ETH.
- Not reachable in the NFT-only stack.
- **Fix:** allow one open Relay leg canister-wide across both sides, or detect refunds by the depository's refund event.

**B-L-7: an Ethereum refund below `min_out`.**
- Rule 8 accepts `refunds[].chainId == "ethereum"`, and refund `minimumAmount` is never bounded. The real quote has two
  refunds, `"0"` each.
- A Relay refund on Ethereum below `min_out` (fees, a price move):
  - never matches the arrival test (`got ≥ min_out`);
  - never raises the 4663 balance;
  - so the buy goes `Stuck`, watched only for `got ≥ min_out`, and there is no exit.
- **Fix:** after the deadline, treat `got ≥ REFUND_SHARE` of `value` on Ethereum as an arrival; or refuse destination-chain
  refunds.

**B-L-8: `eth_act` early returns.**
- `read_ticket(…)?`, `eth_read(ownerOf)?` (`:820-821`) and `outflow_check(Flow::Item)?` (`:849`) abort the whole of `eth_act`.
- One collection's day limit (3 per collection, 6 in all) or a read failure blocks other collections' deliveries and the buy
  fill.
- **Fix:** `continue` per delivery.

**B-L-9: posting capacity.**
- There is one home act per tick (the pending transaction must mine first). Records, marks and the buy's 4 home acts come first.
- The runner ticks every 600 s plus once per hand-in, ≈ 9 ticks an hour.
- 3 collections due every 30 min need ≈ 6 posts an hour. The margin to `FLOOR_TTL` (1 h) is about 3 ticks.
- **Fix:** batch posts (one per collection in a tick through separate nonces), or post before optional acts.

### Info

- **B-I-1:** `sale_nft_listing` (`salenet.rs:77-111`, operator) signs a Seaport listing of any `(chain, collection, token, ask)`
  with the canister's EVM key. `nft_sign` was fenced off the eth_nft chain; this was not. It is inert unless an approval to
  Seaport or a conduit exists, and no path signs one today. Refuse `chain_id == eth_nft.chain_id`.
- **B-I-2:** `eth_finalized` takes the minimum of the providers' `finalized` with no `head − finalized` bound (L-7's rule for 4663
  was never ported). A stale provider stops everything final; that is liveness only. `Bought{block}` is not re-read after
  finality (needs a reorg of more than 8 blocks).
- **B-I-3:** `relayeth::judge` binds the order it inspects. But a runner can invent an order (any solver, salt or version) that
  meets rules 1–12: the deposit then waits on Relay's manual handling. Other gaps:
  - deadlines are `uint_of(..) as u64` (`:171`, `:208`), so a u128 beyond 2⁶⁴ truncates into the band;
  - refund `minimumAmount` is not checked;
  - `requestId` is dropped.

  Pin `solver` and `solverChainId` to the measured values (`0xf70d…dbef`, `base`), bound the deadlines as u32, and keep
  `requestId` for the monitor.
- **B-I-4:** `buys`, `deliveries`, `deposits` (including refused claims, ≤ 16 a tick) and `used_orders` are never pruned, and
  `eth_nft_state()` returns them all. Bound or page them.
- **B-I-5:** `depth` counts all verified listings, not distinct offerers or tokens.
- **B-I-6:** The advance's leftover stays on Ethereum as the gas pot, about 2.5 % of each price (§Q1), and about 19 % under an
  inflated floor. It cannot be recovered without D4's reverse leg.

---

## Cross-collection isolation (launch: Milady, Azuki and MAYC at once)

| Question | Answer |
| --- | --- |
| Can one collection's reading, evidence, arrival or Relay order be attributed to another? | **No.** Evidence is kept per collection (`put_evidence` retains by collection), readings by collection (`st.reading(c)`), and the buy cap uses the buy's own collection's reading and module (`eth_act:866-870`). `eth_plan_buy` refuses another collection's order (`check_any`). One buy at a time means an arrival or order can only belong to that buy. |
| Round robin | Breaks at the first refusal: **B-M-5 (1)**. |
| "One Ethereum buy at a time" | One `Arrived`/`Stuck` buy halts every collection, with no exit: **B-M-5 (2)**, B-L-7, B-M-4. |
| "One home tx at a time" | One blocked act stops every collection's posts: **B-H-2**. Capacity: B-L-9. |
| Custody caps (stage 0: per collection 3 items / 10 ETH; **global 6 items / 10 ETH**) and H3 (per collection 3, global 6 mints, 6 items, 10 ETH a day) | At a 5 ETH raise the collections want 3 + 4 + 3 = 10 items against a global 6. Whoever advances first wins, and the rest are blocked by B-M-5. With deposits ON, holders can fill the global 6 and block all buys (by design of the cap), and fill a collection's 3 to trigger B-H-2. |
| One Ethereum address | Nonces are sequential on one line: one stuck transaction blocks every collection (B-L-3). The gas pot is shared; it cannot be drained meaningfully (a transfer is about 60k gas, ≤ 50 gwei × 60k ≈ 0.003 ETH at the cap, ≈ 6·10⁻⁶ ETH today). |
| Floor posts per collection | A bad floor in one module cannot enter another's post (keyed). But one collection's refusal stops the others' posts (B-H-2), and capacity is tight (B-L-9). |
| Cycles | Linear in the number of collections: B-M-7 (≈ 15.6 T/day for three at 13 nodes). |
| Configuration shared across collections | `evm_chains[1].fee_recipients` / `max_fee_bps` are per chain. Adding one collection's creator payout address allows it for every collection. |

## Operator review questions

### Q1: how many items a stage-0 graduation actually buys at the measured depth

Assumptions:
- item budget 4.12 ETH;
- `gradPrice = 1.03 × P` (`convBps` 300);
- `reserveBps` 2,500 of the units are never converted;
- the canister buys while net units ≥ 1.05 (`bufferBps` 500);
- `P` = the posted floor = the 2nd-cheapest distinct seller;
- advance `= 1.03 × accepted`;
- Relay −0.31 % (expected; −2.3 % worst);
- `cap = min(1.05·P, arrived − 0.003, max_purchase)`.

In every case **`arrived − 0.003` binds, not the 5 % premium**.

| Collection (floor) | `P` range from the depth | Units / net | Buys wanted | Custody cap | Cap vs floor (expected Relay) | Cap at worst Relay (−2.3 %) |
| --- | --- | --- | --- | --- | --- | --- |
| Milady (0.930) | 0.930–0.9486 (2 sellers within +2 %) | 4.2–4.3 / 3.2 | **3** | 3 | +2.3 % to +4.4 % (0.952–0.971) | ≈ P + 0.3 % (0.933–0.951) |
| Azuki (0.670) | 0.670–0.683 (4 sellers within +2 %) | 5.9–6.0 / 4.4–4.5 | **4** | **3**: the 4th never; it triggers B-M-5 (1) | +2.2 % to +4.3 % | ≈ P + 0.2 % |
| MAYC (0.8735) | 0.8735–0.891 (2 sellers within +2 %) | 4.5–4.6 / 3.4 | **3** | 3 | +2.3 % to +4.4 % | ≈ P + 0.3 % |

- **Every buy can fill as long as the rank-1 and rank-2 listings survive the Relay leg.** Rank 1 ≤ rank 2 = P ≤ cap, even at
  the worst Relay fill.
- **After a buy, the book moves up.** For Milady, the 3rd buy is from the +5–10 % band (9 listings, 6 sellers). That is still
  within cap once `accepted` has followed (≤ 5 %/h).
- **What happens to the rest:**
  - The unconverted 25 % reserve and the fractional net units stay ETH-backed in the module; they are not advanced, not
    stranded.
  - What is stranded on Ethereum is each buy's `arrived − paid`: ≈ 0.025 ETH per Milady buy, ≈ 0.018 per Azuki,
    ≈ 0.023 per MAYC. That is ≈ 0.2 ETH across the 9 launch buys, kept as the gas pot with no return path (B-I-6).
- **The D4 path** (`Arrived` waits, pages at 24 h) happens only if the cheap listings vanish during the Relay leg and the next
  one is more than about 2.3–4.4 % higher. That is plausible for Milady and MAYC, which have only 2 sellers within +2 %. It then
  halts every collection's buys (B-M-5 (2)).
- **Azuki's 4th unit** does not strand ETH. It blocks the other collections instead (B-M-5 (1)).

### Q2: does the canister's own buying walk the floor up, and can a seller exploit it?

**Yes, it walks the floor up.** Buying rank 1 promotes rank 3 to rank 2, and the canister's own purchases then enter the
"latest transfer" sales, lifting `S_med` and so the band's top (`1.1 × S_med`).

**The bounds** are the module's 5 %/h and 20 % from the day anchor, and the canister's buy rate (≤ 3 per collection per day;
custody cap 3). The sale band does not bound the walk, because the canister's own buys feed it.

**A seller with 3 wallets** (distinct offerers per the dedup):
1. List 3 items just above the floor.
2. Once the canister has eaten the cheaper others, the seller holds ranks 1–3. `P` = the seller's rank 2, and the canister buys
   the seller's rank 1 at ≤ 1.0236 × `accepted`.
3. The seller relists higher each time.

**Per step:** about the cap margin (+2.3–4.4 %) plus however far the seller's rank-2 price leads `accepted` (≤ 5 %/h).

**Per day:** ≤ 20 % over the day's anchor across the 3 buys. Roughly ≤ (0.07 + 0.14 + 0.20) × floor ≈ 0.4 ETH on Milady, and
then nothing more until the custody cap frees.

**D8 does not stop it:** OpenSea's floor is the seller's own cheapest listing, and CoinGecko follows the same books.

**With deposits ON:**
- deposit→burn pays `min(price − spread, backing per unit)`;
- backing per convertible unit ≈ `gradPrice` ≈ 1.03 × the graduation floor;
- so once the floor has walked more than about 3 % up, a depositor receives ≈ 1.03 × the old floor for an NFT worth the
  old floor: ≈ +0.028 ETH per Milady;
- ≤ `burnPerHour` 5 an hour, and deposits stop at the 3-item cap;
- ≈ 0.08 ETH per collection per cap cycle.

This is small, but the same deposits trigger B-H-2.

A thin book (MAYC: 2 sellers within +5 %) makes the ratchet realistic. Azuki's is concentrated (Q3).

### Q3: Azuki's depth is concentrated in 4 sellers

- The canister needs ≥ 3 distinct offerers among the ≤ 8 listings handed in. The runner asks OpenSea for `best?limit=8` without
  paging (`ops/src/ethnft.ts:174`).
- With 10–14 cheap Azuki listings from only 4 sellers, the 8 cheapest can come from 2 sellers. Then there is no post, and the
  module goes stale by design, with the effects of B-H-2 for that collection.
- Two of the four sellers (or one seller with two wallets; `floor_from` dedups by address only) control rank 2, so they set
  `L`, within the band and the module's move limits.
- After the canister buys from the cheapest seller, the distinct count drops further.
- **Recommendation:** page the runner's listings until ≥ 5 distinct offerers (or 32 listings); use rank 2 of ≥ 5 for
  concentrated books; alert on distinct-offerer count.

### Is anything Milady-specific?

**Nothing in `src/`.** `MILADY` appears only in `ethnft.rs` unit tests and fixtures. A switch to Azuki is configuration only.
Three configuration dependencies differ per collection, all measured or read today:
1. **ERC-2981.** None of Milady, Azuki or MAYC implements ERC-2981 (`supportsInterface(0x2a55205a)` = false; `royaltyInfo`
   reverts). A listing that pays a creator fee is therefore refused by the buy judge unless the payout address is in the chain's
   `fee_recipients` (shared by all collections). Milady's fixture carried OpenSea's 1 % only.
2. **Absolute prices.** `ETH_GAS_RESERVE_WEI` (0.003 ETH) and `GAS_KEEP_WEI` are absolute: 0.45 % of Azuki's price versus 0.32 % of
   Milady's. They slightly lower Azuki's cap margin.
3. **Sales parsing.** Seaport 1.5/1.6 and Blur v2 only. Azuki's and MAYC's volume on other venues is invisible, which makes
   `min_sales` harder.

## The canister as floor poster: what each party can move

| Party | Per hour | Per day | Sustained | Value at stake (stage 0, per collection) |
| --- | --- | --- | --- | --- |
| **Outsider with one NFT and self-trades** (B-H-1) | `accepted` ±5 % | ±20 % from the anchor | ≈ ±19 % (under D8) | Down: ≤ 10 × 0.19 × floor ≈ 1.8 ETH of cheap mints (Milady), plus every buy stalled. Up: ≈ 0.19 × floor stranded per buy (3/day). Cost ≈ 2·10⁻⁵ ETH a self-trade |
| **Outsider freezing** (B-M-1, B-H-2) | — | — | while repeated | Module stale: no mints or burns for up to 7 days (all collections via B-H-2 with deposits ON) |
| **Seller with 2–3 wallets and real listings** | +2.3–4.4 % per canister buy | ≤ 20 % | until other sellers undercut | ≈ 0.4 ETH/day of overpay (Q2); real NFTs at risk to other buyers |
| **Malicious runner** | chooses sales and listings: all of the outsider's power, with no gas | same | same | plus: picks its own listing at the cap (≤ 1.0236 × `accepted`), withholds quotes, listings and evidence, and hands in fabricated Relay orders (B-I-3) |

## Malicious runner or operator: what the endpoints accept and what bounds it

| Endpoint | Accepts | Bounded by | Residual |
| --- | --- | --- | --- |
| `eth_relay_quote(buy, json)` | any JSON ≤ 64 kB for a `Funded` buy | rules 1–12, `QUOTE_TTL` 300 s, used-order replay | worst quote within 3 %; fabricated order griefing (B-I-3); choice of a long deadline (≤ 8 days) |
| `eth_listing(buy, hex)` | any Seaport calldata ≤ 32 kB for an `Arrived` buy | `buy_cap`, `nftnet::check` (Seaport code and domain, zone allow-list, recipients, tips, times, status, owner, signature, simulation, balance) | picks any in-bound listing (its own at the cap); listings that revert cost gas only |
| `eth_floor_evidence(coll, ≤ 8, ≤ 12)` | any listings and tx hashes | `judge_listing`, the parsers, `floor_from` | **sets `P`** (B-H-1), freezes it (B-M-1), un-fillable listings (B-L-1) |
| `eth_deposit(tx)` | any hash (≤ 16 queued) | `judge_deposit`, finality, caps | B-H-2 trigger; refused records never pruned (B-I-4) |
| `eth_give_back(buy)` | `Funded`, or `Stuck` with a watch | only `returnAdvance` to the module | can return a tiny amount while a stuck deposit is still at Relay, so the module books a loss (its design) |
| `sale_nft_listing` | any chain, collection, token, ask | an approval must exist | B-I-1 |
| Timing | when to hand in | TTLs of 600 s / 300 s | can wait for a favourable reading within the 1 h `READING_TTL` |

The operator cannot name a recipient, a calldata, a collection or a price over the cap. Nothing it hands in can move an NFT out
of custody.

## Re-check of the 2026-09-26 fixed canister findings

The branch did not modify: `redeem.rs`, `custody*.rs`, `conflict.rs`, `ticketlog.rs`, `evmread.rs`, `outflow.rs`,
`custodycap.rs`, `deposits.rs`, `pin.rs`, `timelock-rs`. So the Bitcoin-side fixes stand as the R4 check left them. For code this
branch did touch:

| Finding | Status on b48b3ba |
| --- | --- |
| H-1 (finality, log paging) | Bitcoin: unchanged. **Eth side does not port H9's finality for tickets** (B-L-2) |
| H-2 (deposits switch; returns never trip) | Frog side unchanged. Eth deposits have their own switch (`eth_nft.deposits`), which the launch turns ON; new interaction: B-H-2 |
| H-3 (cycles bound per tick) | Eth deposits ≤ 16 claims a tick. **New cycle sink**: B-M-7 |
| H-4 (fee pool) | Unchanged |
| M-1 / N-2 / R3 M-1(c) (no LostRace; paused pass) | Bitcoin unchanged. **Eth: LostRace reachable (B-M-3); no paused pass (B-L-4)** |
| M-2 (trap on decode) | **Holds**: `post_upgrade` still traps (`canister.rs:438`); the thirteenth element is `opt`; the golden `saved-e04f49b` fixture test passes; the new guard refuses an upgrade that drops the chain with a line in flight |
| L-1 (save before send) | **Holds** for `sign_eth`, `cancel`, `send_home` (state saved, then broadcast) |
| L-2 (`pre_upgrade` mid-tick; stale reservations) | `pre_upgrade` trap unchanged, **holds**. **No eth analog of the stale-reservation sweep** (B-M-4: `eth:{id}` of a `Stuck` buy leaks) |
| L-3 (`inspect_message`) | **Holds**: the new endpoints are operator-only, and `may_ingress` is unchanged |
| L-4 (writers refuse mid-tick) | **Holds**: all 5 eth updates have `not_ticking` (`eth_give_back` after its await); the source test now includes `ETH.with`; 14 methods checked |
| L-5 (signed buy never abandoned) | **Eth analog missing**: no replacement or cancel for a pending fill (B-L-3) |
| L-7 / N-3 (provider extremes) | Eth `finalized` unbounded (B-I-2); `eth_estimateGas` max, bounded by `gas_cap` |
| I-5 (limits ×4) | **Holds** (the eth side adds no new limits) |
| I-6 (no delivery to own addresses) | **Holds**: `judge_transfer` refuses us, and `EthNftWrapper.redeem` refuses the custodian |
| N-1 … N-12, R3-C1 … C7, R4-C1 … C4 | Code untouched; **not regressed**. R3-C3 (`assert_chain` first): the eth side runs after `assert_chain` in the unpaused tick, **holds** |
| H1 (pause) | **Holds**: the whole eth side is skipped while paused; eth endpoints refuse while paused; `nft_sign` refuses the eth chain. `sale_nft_listing` is B-I-1 |

## What I tried that held

**`relayeth::judge`:**
- a recipient, refund, currency or chain changed without and with a recomputed id;
- an allocator rotation;
- the calldata's depositor;
- a second step or item;
- the `paymentDetails` mismatch;
- replay (rule 12, recorded before the send);
- a stale hand-in;
- the reverse leg's rules refusing a forward quote.

All refused. The order hash matches Relay's (a fixture test).

**The buy:**
- a wrong collection or token;
- an offerer that is us;
- tips (`totalOriginal…` ≠ length);
- unknown recipients;
- fees over `max_fee_bps`;
- times against the chain clock;
- partial fractions and criteria resolvers;
- a zone not on the allow-list;
- a consideration item not in native ETH;
- `plan.value != plan.total`;
- over the cap.

All refused, and the canister always signs its own re-encoding with itself as the recipient (`encode_advanced`).

**Deliveries:** the destination comes from the ticket only; relist or zero destinations are never sent; `transferFrom` runs no
hook; the simulation runs before signing; a cancel is signed only at the transfer's nonce, 0-value, to itself, with bumped fees;
a mark is made only after a final revert or a final cancel.

**Overflow and panic:**
- the sale sums use `checked_add` and `small()`;
- `median` avoids overflow;
- `clamp(lo.min(hi), hi)` cannot panic;
- the `quantile` index is safe when non-empty;
- `blur_sale` works on fixed slices;
- `seaport_sale`'s `(off/32) as usize` would truncate on wasm32, but only on data Seaport itself emitted.

## Test results

| Run | Result |
| --- | --- |
| `cargo test` at b48b3ba (baseline, in the checkout) | **536 passed**, 0 failed (as §11.4 says) |
| With `tests/audit_nft_b.rs` | **547 passed**, 0 failed: 536 plus 11 PoCs, each asserting a weakness |

## Not covered

- `ethflow` deposits and returns in depth (another reviewer); I covered only their interaction with posts and caps.
- The Solidity side beyond the functions named above.
- The services, except `ops/src/ethnft.ts`'s evidence selection.
- A replica or mainnet run of HTTPS-outcall consensus: B-M-6 is inferred from 13 sequential requests from one IP, not from 13
  replicas; B-M-7 uses the reservation pricing formula.
- The keeper's D8 monitor logic and its references.
- The reproducible build.
