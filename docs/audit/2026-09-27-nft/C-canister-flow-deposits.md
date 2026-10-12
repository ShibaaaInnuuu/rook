# Rook security review 2026-09-27 (nft-side), area C: the canister's Ethereum flow, holder deposits and economics

Reviewer C. Written 2026-09-27. Nothing was committed, pushed, deployed or signed, and no key, wallet or identity was touched.
In the main worktree the only changes are this report and `poc/C/` (`audit_nft_c.rs` and its captured output
`audit_nft_c.output.txt`). The PoCs were written and run in a separate checkout (`/Users/jlohayon/Documents/Rook-audit-nft-C`,
detached at e90471c), which was removed afterwards. I made no network calls: every provider fact used below is quoted from
`docs/v7/phase1-nft.md` §4.5/§8, `docs/audit/2026-09-26/fixes-D-canister.md` §0, or reviewer B's measurements.

## Scope and commit

- **Checkout:** e90471c ("D5: holder deposits on at launch …"), parent b48b3ba, GitHub `main` 47a9b94.
- **Read line by line:**
  - `src/ethflow.rs`, all of it (1,635 lines): the tick order, the buy steps, arrival, finality, deliveries, the cancel,
    `superseded`, `mined_eth`, the home line, deposits and returns, the endpoints' pure parts;
  - `src/ethnft.rs` §4.4 / §2.4: `judge_transfer`, `deposit_data`, `judge_deposit`, `inbound_transfers`, `judge_claim`,
    `judge_return`, the wrapper ABI, `receipt_of` / `receipt_projection`, `buy_cap`, `cost_of`;
  - `src/ethnet.rs` as it serves the flow: `eth_tx_view`, `eth_receipt_logs`, `eth_our_receipt`, `EthNet::fees`,
    `eth_plan_transfer`, `eth_finalized`, `eth_receipt`, `eth_deposit_tx`, the tick hook and the five endpoints;
  - `canister.rs`: the tick's wiring (`:1264-1358`), the pause, `outflow_check`, `with_caps`, `pre_upgrade` / `post_upgrade`
    (`:355-440`); `custodycap.rs` (`check_acquire`, `reserve`, `settle`, `admit`, `release`); `outflow.rs` (`Flow::Return`);
    `abi.rs` (`word_at`, `bytes_at`: panics and overflow on hostile calldata);
  - `src/nft/EthNftWrapper.sol` `_admit`, `redeem*`, `recordDelivery`, `markUndeliverable`, `refund` (for what the flow relies on);
  - `ops/src/ethnft.ts` `inboundTransferTxs` and the deposit hand-in loop; `monitor/src/ethnft.mjs` (`unclaimedAlerts`, the
    deposit and delivery alerts, the custody check).
- **Not re-read:** the floor evidence judge, the Relay judge, Seaport and the buy judge's internals (areas A/B). Where a finding
  here was also reported by reviewer B against b48b3ba, the table says so and I state whether e90471c changed it.
- **Tests at e90471c:** `cargo test` **544 passed, 0 failed** (matches the brief). With my PoC file: **552 passed, 0 failed**
  (8 new tests, each asserting the weakness, so each turns red once fixed).

## Methodology

1. **Design first:** `docs/v7/phase1-nft.md` in full (§2 flows, §2.4 deposits as built, §4.4–4.6, §7, §9 D1–D9, §11);
   `docs/v4/nft-wrapper.md` §4 (the original deposit spec); the parts of `docs/v6/relay-conversion.md` the flow reuses (arrival by
   balance, refunds, `GAS_KEEP`). The 2026-09-26 area-D report, its fixes (`fixes-D-canister.md`), `review2/R2-canister.md`,
   `review2/fixes-R3-canister.md`, `review3/fixes-R4-canister.md`, `review4/part-canister.md` and `R4-check.md`, to know what was
   fixed and must not regress (H-2, H-3, H-4, M-1, M-2, L-1…L-7, N-7, N-8, N-10, N-12, R3 M-1(c)).
2. **Invariants** written down (below), then line-by-line reading of the scope, then **PoCs against the repository's own fake
   world** (`tests/ethnft.rs`, copied and extended, not modified): a second collection on the same address and line (D1), a
   receipt read that fails N times, a transfer plan that fails once with the exact text `EthNet::fees` produces, H3 waits by key,
   and a broadcast log.
3. **Internet Computer practice** (DFINITY's inter-canister call, DoS and upgrade guidance, as applied in D-canister-timelock.md):
   state at every await (what a failed read after a state change leaves behind); traps after awaits; replicated HTTPS outcalls
   and their transient failures (a read error must be a wait, never a decision); user-triggerable outcalls per tick; the pause as
   a kill switch; upgrade hooks (Candid decoding of the saved tuple).
4. **Rust:** `[profile.release]` has no `overflow-checks`; I looked for unchecked arithmetic on attacker-controlled values
   (`abi::bytes_at` offsets, `data = start + 32`; `u32` casts in `deposit_data`), `expect`/indexing after awaits, and unbounded
   collections cloned per save.
5. **Ethereum:** EIP-1559 replacement (a same-nonce replacement needs ≥ +10 % on both fee fields), mempool eviction of
   under-priced transactions and anyone's ability to re-broadcast a signed transaction, ERC-721 `Transfer` semantics (a token
   cannot arrive at an EOA twice without leaving in between), `safeTransferFrom` to an EOA (no hook; `data` visible only in
   `tx.input`), finality (the lower `finalized` of the providers).

## Launch configuration assumed

The operator's decisions of 2026-09-27 (§9): **D1** Milady, Azuki and MAYC on one canister, one Ethereum address, one Ethereum
nonce line, one home-chain line; **D2** drand-random; **D3** ≈ 0.01 ETH gas seed, pages under 0.005; **D4** stranded ETH waits;
**D5 holder deposits ON**; **D6** OpenSea key in the runner; **D7** default floor; **D8** guardian auto-pause (registry-wide);
**D9** Blur not bought. Stage 0: custody 3 items / 10 ETH per collection and 6 / 10 ETH in all; H3 3 items and 3 mints per
collection a day, 6 global, 10 ETH a day, trip factor 3, `Flow::Return` never trips. Ethereum fees: `max_fee = 4 × pinned base +
priority` (`chains.rs:151-163`, `looper::FEE_MULTIPLE = 4`), cap 50 gwei; measured base fee median 0.068 gwei, maximum 1.535 gwei
over 1,024 blocks (§8).

## Invariants the code must keep (checked)

| # | Invariant | Status |
| --- | --- | --- |
| E1 | The Ethereum key signs only: a judged Seaport fill (value = total ≤ cap) to Seaport; `transferFrom(us, ticket destination, ticket token)`; `transferFrom(us, a stray's own Transfer sender, that token)`; a 0-value self-send at a delivery's nonce. | **Holds.** The only signers are `sign_eth` (callers `ethflow.rs:965`, `:1028`, `:1054`) and `cancel` (`:1109`). Calldata is built by the canister (`ethnft.rs:678-680`, `nftnet::check_any`). |
| E2 | A unit is minted only for an NFT we own past Ethereum finality, once per `(token, tx)`, on the collection's own wrapper. | **Holds.** Buys: `Bought → Final` needs `block ≤ finalized` and `ownerOf = us` (`:720-734`). Deposits: finality, the receipt re-read in its final block, `ownerOf = us`, `itemOf = 0`, room (`:878-916`); the wrapper re-checks `itemOf` and `sourceUsed` (`EthNftWrapper.sol:287-302`). The claim binds collection ↔ wrapper (`judge_deposit` needs `tx.to == collection` and data naming that collection's wrapper). |
| E3 | A deposit mints only to the recipient named in the owner's own signed `safeTransferFrom` data, for the home chain and wrapper it names. | **Holds** (`ethnft.rs:715-760`; `abi::bytes_at` bounds-checked, no panic on hostile offsets). |
| E4 | A stray goes back only to its own `Transfer`'s `from`, and never a token the wrapper holds, an open buy or claim uses, or one that left us after that transfer came in. | **Holds** (`judge_return`, `token_busy`, `itemOf`, `last_out`). Two open records for one token need the token to leave between two arrivals, which only our own transactions do. Record bookkeeping is keyed without the collection (C-I-1). |
| E5 | Save, then send; never two signatures for one record. | **Holds** for `sign_eth` (`:979-986`), `cancel` (`:1112-1118`) and `send_home` (`:1236-1238`). This closes the Ethereum analogue of L-1. |
| E6 | A read the canister could not make is a wait, never a decision. | **Violated:** C-M-1 (a transport error in the simulation marks a delivery / holds a stray for good), C-M-3 (`superseded` drops the line before its read). Claims keep it (6 h retry, then unrecorded). |
| E7 | A redeemer never holds both the NFT and a refund. | **Violated** by C-M-3 (orphaned delivery) and C-L-1 (a pause past `deliverBy + 30 d`). The wrapper then keeps a phantom item. |
| E8 | One collection cannot stop another's deliveries, buys, deposits or floor (D1). | **Violated:** C-H-1 (every floor), C-M-2 (every delivery), C-L-2 (deliveries, deposit mints), C-L-3 (buys). |
| E9 | The pause stops every signing path, and what already left is still accounted for (R3 M-1(c)). | **First half holds** (the eth tick does not run while paused, `canister.rs:1346`). **Second half violated** (C-L-1). The pause is read once per tick (C-I-5). |
| E10 | What a stranger can make the canister do per tick is bounded (H-2/H-3). | **Holds:** 4 claim reads a tick (`CLAIMS_PER_TICK`), one return a tick, `Flow::Return` never trips, one deposit mint asks at a time. A pending `Seen` deposit costs no read before finality. |
| E11 | An upgrade keeps the state or refuses (M-2). | **Violated for the thirteenth element:** an `opt EthState` of another shape decodes as `null` (C-L-4). |
| E12 | Returns never eat the gas kept for deliveries (D3, D5). | **Holds** (`need = arrived + gas + 0.005 ETH`, `:957-964`). Deliveries check their own gas (`ethnet.rs:241-245`). |

## Findings

| ID | Severity | Title | Where (file:line @e90471c) | Reachable in the launch config? | PoC |
| --- | --- | --- | --- | --- | --- |
| C-H-1 | **High** | A purchase whose mint waits on H3's mint limit stops **every** collection's floor post until the limit's window slides (≤ 24 h). Deposits, now ON, use up that limit on demand; every module goes stale (mints and burns refused) | `ethflow.rs:1281-1285` `home_act` (`outflow_check(…Mint…)?` at `:1285`), floor step `:1412-1428`; deposit mints `:1381-1402` | **Yes** (D5 ON, D1). B-H-2's buy path, unchanged at e90471c | `audit_nft_c_a_purchase_mint_waiting_on_the_limit_deposits_used_up_stops_every_collections_floor_post` **passes** |
| C-M-1 | Medium | A transport or consensus failure of the transfer's `eth_call` simulation is "refused (simulation)": a good delivery is marked undeliverable at once, and a stray is `Held` for good | `ethnet.rs:109-113` `EthNet::fees`; `ethflow.rs:1015-1021` `eth_act`; `:949-953` `return_act` | **Yes** (any outcall failure at that read) | `audit_nft_c_a_transient_read_error_at_the_simulation_marks_a_good_delivery_undeliverable_and_holds_a_stray_for_good` **passes** |
| C-M-2 | Medium | The one Ethereum line has no bump, cancel or timeout for a fill or a return: an unmined one holds the line, and every collection's waiting deliveries are neither sent nor marked, so their holders wait for the 30-day window | `ethflow.rs:484` (`eth_act` only when the line is free), `:1071-1096` `resolve_eth` (cancel only for `Deliver`, `:1086-1093`), `:1002-1006` (the mark lives in `eth_act`) | **Yes** (base fee above 4 × the signing base for the duration) | `audit_nft_c_an_unmined_fill_holds_the_one_ethereum_line_and_another_collections_delivery_is_never_sent_or_marked` **passes** |
| C-M-3 | Medium | `superseded` clears the line before its own reads: one failed receipt read as a transfer beats its cancel orphans the delivery (`Cancelling` for ever). It is never recorded; the redeemer refunds after 30 days and keeps the NFT; the item stays as a phantom, its custody slot is never released, `outbound` never learns, and the line keeps signing | `ethflow.rs:1124-1159` `superseded` (`st.line = None` `:1125`; `?` `:1131`, `:1149`; `Err` `:1143`, `:1155`, `:1157`) | **Yes** (a read failure at that tick) | `audit_nft_c_a_read_error_as_a_transfer_beats_its_cancel_orphans_the_delivery_and_the_redeemer_keeps_nft_and_refund` **passes** |
| C-L-1 | Low | While paused nothing on the Ethereum side runs: no record, no cancel. A pause past `deliverBy + 30 d` lets a redeemer refund a delivery that already moved; on resume the canister's first act re-broadcasts the refunded ticket's delivery | `canister.rs:1346`; `ethflow.rs:451-454`, `:1072`, `:1256-1258` | Needs a pause of ≥ ~30 days | `audit_nft_c_a_pause_past_the_late_window_ends_with_nft_and_refund_and_the_resume_rebroadcasts_the_refunded_delivery` **passes** |
| C-L-2 | Low | Head-of-line across collections: `eth_act` returns on the first waiting delivery's H3 wait or read error; the first `Mint` deposit blocks every other collection's deposit mints | `ethflow.rs:998-999`, `:1027` `eth_act`; `:1385-1411` `home_act` | Yes (≤ 24 h per limit window; indefinitely for a paused listing) | `audit_nft_c_one_collections_limit_blocks_every_other_collections_deliveries_and_deposit_mints` **passes** |
| C-L-3 | Low | A new buy is refused for every collection while the first collection in the round robin that wants an item has no custody room (three deposits do it); an `Arrived` or watch-less `Stuck` buy blocks all buying with no exit | `ethflow.rs:644-653` `buy_pass` (`?` at `:653`, early return `:647`); `:302-304` `open_buy`; `:1567-1580` `give_back_now` | Yes (D1, D5) | `audit_nft_c_a_full_collection_whose_module_wants_an_item_stops_every_other_collection_from_buying` **passes** |
| C-L-4 | Low | M-2 residual: the saved tuple's `opt EthState` decodes an incompatible shape as `null` (Candid's opt rule), so `post_upgrade` restores an **empty** Ethereum side (signed line, buys, deliveries, cursors, used orders) instead of trapping. b48b3ba → e90471c is such a change | `canister.rs:393`, `:425`; `ethflow.rs:273-293` | On an upgrade with a changed `EthState` | `audit_nft_c_an_eth_state_of_another_shape_decodes_as_nothing_rather_than_refusing_the_upgrade` **passes** |
| C-L-5 | Low | No operator exit for a `Held` stray or an unreadable claim: the sender's NFT stays at the canister until a timelocked upgrade. Claims over the outcall caps (input > 64 kB, receipt > 200 kB) are never judged, and the monitor's page lapses after 7,200 blocks | `ethflow.rs:817-830`, `:171-173`; `ethnet.rs:289`, `:294`; `monitor/src/ethnft.mjs:21`, `:91` | Yes | Reasoned (C-M-1 (b) shows a `Held` that never moves) |
| C-L-6 | Low | Deliveries are signed from `ticket(i)` and `item(i)` read at the 4663 fast pin, with no 4663 finality (H9 / fixes-D H-1 dropped for this side) | `ethflow.rs:519-556` `scan_tickets`; `:998` | Needs a 4663 reorg past the fast pin | Reasoned (B-L-2) |
| C-I-1 | Info | Stray bookkeeping is keyed `(tx, token)` without the collection: the same id of two collections in one transaction updates the wrong record, leaves the other `Returning` for ever, and writes `outbound` under the wrong collection | `ethflow.rs:218`, `:335-337`, `:1167-1172`, `:1217-1227` | Yes (self-harm only) | — |
| C-I-2 | Info | H2 counts deposits at value 0 (`check_acquire(…, 0)`, `admit(…, 0)`): the per-collection and global 10 ETH value caps do not see deposited NFTs | `ethflow.rs:911`, `:1394` | Not binding at stage 0 for D1's collections | — |
| C-I-3 | Info | R3's preconditions for switching deposits on are not met: returns are paid from the gas pot with no per-sender budget (N-8), and there is no return-anomaly tripwire (N-10). One stranger with one NFT holds a collection's 3 returns a day | `ethflow.rs:946-948`; `outflow.rs:206-208` | Yes (bounded) | — |
| C-I-4 | Info | A stray's receipt is not re-read after finality (a `Seen` deposit's is), and `itemOf` is read at the home fast pin in the same tick a mint was seen mined. I found no reachable path to two open records of one token, so this matters only with a deep Ethereum reorg | `ethflow.rs:925-945` | Deep reorg only | — |
| C-I-5 | Info | The pause is read once per tick (`:451`): a guardian pause that lands during the tick's awaits still lets one Ethereum and one home transaction out; a freeze tripped inside `eth_act` still lets `home_act`'s records, marks and posts out | `ethflow.rs:451-504` | Yes (bounded to one of each) | — |
| C-I-6 | Info | The cancel is signed with no balance check and is never re-priced; above the fee cap it is not signed at all and the delivery waits (the refund window then needs the 30 days) | `ethflow.rs:1099-1120` | Fee spikes above 12.5 gwei base | — |
| C-I-7 | Info | Unbounded state, cloned into the global at every `io.save` (≈ 8 a tick) and serialised whole in `pre_upgrade`: `deliveries`, `deposits` (including a `Refused` record for any hash the runner hands in, ≤ 4 a tick), `outbound`, `used_orders` | `ethflow.rs:273-293`; `ethnet.rs:143-145` | Grows with use | — |
| C-I-8 | Info | `eth_nft_state()` publishes the line's signed bytes (intended, for the monitor), so anyone can re-broadcast a pending delivery or fill at any time. That widens C-L-1 and the cancel races | `ethnet.rs:383-391` | Yes | — |
| C-I-9 | Info | Docs: `phase1-nft.md` cites "§11.5" three times for D5's judgment calls, but there is no §11.5 at e90471c. §2.1 still says one purchase per collection (the code is canister-wide, as §11.2 says). §7 says "disagreement is nothing believed", which C-M-1 contradicts | `docs/v7/phase1-nft.md:18,575,585-588`, `:176`, `:539` | — | — |

Counts: **0 Critical, 1 High, 3 Medium, 6 Low, 9 Info.**

No path I found lets an outsider, the runner or the operator take an NFT to a destination of their choosing, mint a unit without
an NFT the canister owns past finality, mint on another collection's wrapper, or make the canister pay more than the buy cap.
The claim judge and the stray-return rules hold. The findings are availability and "what a failed read decides" issues. Two of
them (C-M-3, C-L-1) end in the one outcome the design rules out: a redeemer with both the NFT and a refund.

---

### C-H-1 (High): a purchase's mint waiting on H3 stops every collection's floor, and deposits make it happen on demand

**Description.** `home_act` does at most one home-chain act a tick, in a fixed order: records and marks; then the open buy's
step; then one deposit mint; then the floor post. The buy's `Final` step asks the H3 mint limit with `?`:

```rust
BPhase::Final { arrived, token, total, tx } => {
    ...
    io.outflow_check(&key, &col, Flow::Mint, 1)?;          // ethflow.rs:1285
```

A `Wait` verdict returns `Err` from `home_act`, so the floor step (`:1412-1428`) is not reached, on every tick, until the
purchase's mint may go.

e90471c fixed the deposit half of B-H-2: a deposit that cannot be admitted now becomes `Return` (`:1394-1397`), and a deposit's
own wait is kept aside (`deposit_wait`) so the floor still posts. The buy half is unchanged, and **deposits now feed it**:

- Deposit mints and buy mints share H3's `Flow::Mint` budget under the same collection key (`key_of(c, chain)`, `:1393` and
  `:1284`): 3 a day per collection, 6 a day in all.
- Deposit mints are held back only while a purchase is between its fill and its mint (`buy_minting`, `:1383`). While the
  purchase is `Bridging` or `Arrived`, deposits of its collection mint first and use the day's room.

**Exploit.** Anyone holding 3 NFTs of a collection (≈ 2 ETH of Azuki; the depositor gets 3 units back, so the only cost is
gas):
1. Watches for `Advanced` on that collection's module (a buy started), or simply keeps depositing when a graduation is due.
2. Deposits the 3 NFTs while the purchase bridges or waits for a listing. They go final in ≈ 15–30 minutes and mint: 3 of 3 mints
   of the day.
3. The purchase fills, goes final, and its mint waits for up to 24 h.

For that time no module's floor is posted: not Milady's, not Azuki's, not MAYC's. Each module goes stale after its `FLOOR_TTL`
(1 h), and a stale module refuses public mints, conversions and **burns** (A-I-6). The global limit (6 a day across D1's three
collections) gives the same result with deposits spread over two collections. Buys alone can also reach it: 3 collections × 3
buys a day is more than 6.

**Impact.** The attacker can deny every collection's floor, and so its mints and burns, for up to a day, repeatedly. It costs
gas; the units are kept (and can be redeemed or burned later). Holders cannot exit through burns while the modules are stale.
Redemption still works.

**PoC.** `poc/C/audit_nft_c.rs::audit_nft_c_a_purchase_mint_waiting_on_the_limit_deposits_used_up_stops_every_collections_floor_post`
**passes.** A Milady buy reaches `Final` while H3 says "3 of 3 Mint". For two hours both collections get fresh evidence every
20 minutes with a post due. Result: 0 posts (`eth: the home-chain act waits: waits (outflow limit) …` each tick), and both
readings still `due`. Posts resume once the limit frees.

**Recommended fix.**
- Make every step of `home_act` before the floor non-fatal, as `deposit_wait` already is: collect its error, fall through to the
  floor, and return the error afterwards. The same applies to `still_resolved(…)?` (`:1256`, `:1266`), `module_view(…)?`
  (`:1307`) and the `Returning` balance check (`:1299-1301`).
- Better still, give the floor post its own slot: at most one post every N ticks ahead of the queue whenever a reading is due and
  older than, say, 30 minutes.
- Reserve the purchase's mint room at the advance (H3 already sees the value there): count one `Flow::Mint` for `mint:eth:{id}` at
  the advance, so deposits cannot take it. Or refuse a deposit mint while any buy of that collection is committed (not only between
  fill and mint).

### C-M-1 (Medium): a transient failure of the transfer's simulation is taken as "this can never succeed"

**Description.** `EthNet::fees` runs the exact transaction as an agreed `eth_call` at the pin, and maps **every** failure to the
same text:

```rust
nn::agreed(&self.chain, "eth_call", serde_json::json!([tx.clone(), crate::pin::at(&p)]), 16_000).await
    .map_err(|e| format!("refused (simulation): it reverts at the pin: {e}"))?;      // ethnet.rs:112-113
```

`nn::agreed` fails on a real revert, but also on any outcall failure of either provider (timeout, HTTP 429, a body the replicas
do not agree on) and on the providers disagreeing (`nftnet.rs:52-73`). The flow then treats that text as final:

- **Deliveries** (`ethflow.rs:1015-1021`): `DPhase::Mark { "never sent: …" }` at once, whatever the deadline. `home_act` sends
  `markUndeliverable` in the same tick, and the ticket becomes refundable at `deliverBy + 72 h`.
- **Stray returns** (`:949-953`): `XPhase::Held`. Nothing ever moves a `Held` record again, and no endpoint does (C-L-5).

The design's own rule is the opposite: "a transient read never refuses a claim" (§2.4, and the 6 h retry for claims), and
"disagreement is nothing believed" (§7).

**Impact.**
- **No attacker needed.** Every delivery makes this one read, once. Reviewer B measured `1rpc.io/eth` answering with rate-limit,
  "discontinued" and HTML bodies (B-M-6). Any such answer at that moment turns a deliverable redemption into a refund 10 days
  later: the holder gets a unit back, not the NFT. For a stray, a mis-sent NFT is stranded until an upgrade.
- **A single provider** (either of the two) that errors on `eth_call` marks every delivery undeliverable. Redemption becomes
  refund-only for as long as it does.

**PoC.** `audit_nft_c_a_transient_read_error_at_the_simulation_marks_a_good_delivery_undeliverable_and_holds_a_stray_for_good`
**passes.** One plan fails with exactly the text `fees` produces for an `IC0406 SysTransient` timeout:
- the delivery is `Marked` and `markUndeliverable(0)` is sent in the same tick; five healthy hours later it is still `Marked`
  and no transfer was ever signed;
- the stray is `Held { "the return cannot succeed: refused (simulation): … SysTransient …" }` and stays so.

(B-M-2 reported the delivery half at b48b3ba. e90471c adds the return path with the same classification.)

**Recommended fix.**
- In `fees`, tell a revert from a failure. A revert is a JSON-RPC `error` with code 3 / `execution reverted` from **every**
  provider, agreed. Anything else is a transport error ("waits").
- Mark only after N consecutive agreed reverts on different ticks. Never inside the first days of the window unless the revert
  is agreed and repeated.
- For returns, keep the stray `Return` and retry; hold only on an agreed, repeated revert.

### C-M-2 (Medium): an unmined fill or return holds the one Ethereum line, and no delivery of any collection is sent or marked

**Description.**
- The tick runs `eth_act` only when the line is free (`:484`).
- `resolve_eth` re-broadcasts the line's bytes each tick. Only for `EthKind::Deliver` does it ever act on a transaction that does
  not mine: the cancel a day before the deadline (`:1086-1093`). A `Buy` (a Seaport fill) or a `Return` is re-sent as is for
  ever. There is no fee bump, cancel, timeout or operator endpoint (§11.2: "Not built: an automatic replacement of an under-priced
  fill").
- The fee is `4 × pinned base + priority`. The base fee can rise 12.5 % a block, and the measured day ran from 0.068 to 1.535 gwei.
  So a transaction signed at a quiet moment is outbid by any sustained rise past 4× that base.

While the fill is pending:
- no delivery of any collection is signed (D1: one address, one nonce line);
- no waiting delivery is **marked** either, even past its deadline, because the "never sent, mark at once" path is inside
  `eth_act` (`:1002-1006`).

The ticket is then refundable only by the late window (`deliverBy + 30 days`), not the 72 h grace.

**Impact.** Redemption, the path that cannot be switched off, stalls for every collection for as long as the fee regime stays
above the fill's price. Holders' units are locked up to 30 days after `deliverBy` instead of 72 h. No value is lost. The operator
has no lever short of a timelocked upgrade. The monitor pages (`Buying` > 1 h).

**PoC.** `audit_nft_c_an_unmined_fill_holds_the_one_ethereum_line_and_another_collections_delivery_is_never_sent_or_marked`
**passes.** A Milady fill is signed and never mines. An Azuki ticket resolves. Hourly ticks for 241 h, past `deliverBy + 72 h`:
the Azuki delivery is still `Waiting`, nothing was marked, the line still holds the fill, and exactly one transaction was ever
signed.

(B-L-3 reported this at b48b3ba as Low. I rate it Medium: its effect is on redemption across all collections, and the launch
configuration has three collections on the one line.)

**Recommended fix.**
- Replace a pending line transaction when the pinned base exceeds its `max_fee` for more than, say, an hour:
  - a **fill**: its own cancel (a 0-value self-send at the same nonce). If the fill mines instead, it is a buy as now;
  - a **return**: a re-signed copy at higher fees (same calldata);
  - a **delivery**: a re-signed copy long before the last day, not only a cancel at the end.

  Keep every signed version in the record, so `superseded` can tell which one mined.
- Run the "never sent → mark" check for `Waiting` deliveries every tick, even while the line is busy: it signs nothing on
  Ethereum.

### C-M-3 (Medium): `superseded` drops the line before it reads, and one failed read orphans a delivery for good

**Description.** When our line transaction has no receipt but the nonce moved on, `resolve_eth` calls `superseded`
(`:1080-1083`). It starts with `st.line = None` (`:1125`), and only then reads which version mined:

- `Cancel` (our cancel is on the line; the delivery raced it): `io.eth_receipt(of).await?` (`:1131`). An error returns through
  `?`, and an `Ok(None)` returns `Err("… waiting rather than guessing")` (`:1143`). Either way the line is already gone, and the
  delivery stays `Cancelling`.
- `Deliver` without a cancel: `Err("… not ours: look before anything else is signed …")` (`:1157`), with the delivery left
  `Sent`.
- `Deliver` with a cancel: `?` at `:1149`, and `Err` at `:1155`.

Nothing looks at a `Cancelling` or `Sent` delivery again:
- `delivery_pass` only turns a `Waiting` one `Gone` (`:768-775`);
- `resolve_eth` only ever looks at the line.

"Look before anything else is signed" is not enforced: step 6 runs `eth_act` in the same tick, and it signs the next delivery,
buy or return.

**Exploit / impact.** No attacker is needed, only a delivery racing its cancel (pending a day before `deliverBy`: C-M-2's fee
conditions) and one failed receipt read at that tick (a 429 is enough). If the transfer won:

1. The redeemer holds the NFT, and the ticket is never recorded or marked.
2. At `deliverBy + 30 days` anyone may `refund` it: the redeemer gets a unit back **and** keeps the NFT. The wrapper puts the
   item back in its pending set, so there is a **phantom item**: a unit backed by nothing. Later tickets that draw it are marked
   undeliverable and refunded, again and again.
3. The custody book keeps `1:0x<collection>:<token>` held for ever (`release` runs only on a recorded delivery), so one of the
   collection's 3 stage-0 slots is lost.
4. `outbound` never learns that the token left, so a later stray of the same token is judged without that fact.

If the cancel won instead, the ticket is never marked, and the holder waits 30 days rather than 72 h.

The monitor's custody check (every held item owned by the custodian on both providers) pages SEV1 once the transfer mines.
There is no recovery short of an upgrade.

**PoC.** `audit_nft_c_a_read_error_as_a_transfer_beats_its_cancel_orphans_the_delivery_and_the_redeemer_keeps_nft_and_refund`
**passes.**
1. The transfer mines instead of the cancel, and the next receipt read fails once (HTTP 429). The line is empty and the delivery
   is `Cancelling`.
2. Within 24 hourly ticks a second delivery is signed ("look before anything else is signed" is not kept).
3. At `deliverBy + 30 d` the delivery is still `Cancelling`, the ticket is `Resolved` and unmarked, and it is refunded. The owner
   of #77 is ALICE, the book still holds #77, and `last_out(#77)` is `None`.

(B-M-3 at b48b3ba; unchanged at e90471c. The `outbound` and custody-slot consequences are new with D5.)

**Recommended fix.**
- Clear the line only after the read-back succeeded. On any read error, or neither receipt, keep the line and return
  "waits".
- For "not ours", keep the line and set a durable `halted` flag that `eth_act` and `home_act` honour until the operator
  acknowledges it.
- Add a pass that re-reads every `Sent`/`Cancelling` delivery's known hashes whenever it is not on the line.

### C-L-1 (Low): a pause longer than the late window ends with NFT and refund, and the resume re-broadcasts the refunded delivery

**Description.** `tick` calls `ethnet::tick` only when not paused (`canister.rs:1346`), so while paused:
- a delivery that moved (`Moved`) is not recorded;
- a pending one is not cancelled.

That is unlike the Bitcoin side, whose paused pass still records confirmed deliveries and conflicts out stale ones (R3
M-1(c)). `EthNftWrapper.refund` needs no mark after `deliverBy + LATE_DELIVERY_WINDOW` (30 days). Two consequences:
- **(a)** A delivery mined before or during a pause of ≥ `deliverBy + 30 d − now`: the redeemer refunds and keeps the NFT. On
  resume the record finds the ticket `Refunded` → `Gone` (`:1256-1258`), without releasing the custody slot. The wrapper keeps
  a phantom item.
- **(b)** A delivery still pending when the pause began: on resume, `resolve_eth` **re-broadcasts the saved bytes first**
  (`:1072`), before it looks at the deadline or the ticket, and only then signs a cancel. If the re-broadcast mines first, the same
  LostRace follows. The bytes are public anyway (`eth_nft_state().line.raw`, C-I-8), so a redeemer can re-broadcast them whenever
  fees allow.

**PoC.** `audit_nft_c_a_pause_past_the_late_window_ends_with_nft_and_refund_and_the_resume_rebroadcasts_the_refunded_delivery`
**passes.**
- (a) ALICE holds #77, the ticket is refunded, the delivery is `Gone`, and the slot is still booked.
- (b) The first broadcast after the resume is the refunded ticket's delivery; it mines, and the delivery ends `Gone` with ALICE
  holding #77.

(B-L-4 reported (a) at b48b3ba; (b) is the resume order.)

**Recommended fix.**
- Give the Ethereum side a paused pass, as R3 did for Bitcoin. While paused:
  - read the line;
  - record `Moved`/`Record` deliveries (reads, and one home transaction that only records);
  - cancel a pending delivery inside its last day (a self-send moves nothing).
- In `resolve_eth`, read the ticket before re-broadcasting a `Deliver`. Never re-send one whose ticket is not `Resolved` or is
  inside the stop window: go straight to the cancel.

### C-L-2 (Low): head-of-line across collections in `eth_act` and in deposit mints

**Description.**
- In `eth_act`, `read_ticket(…)?`, `eth_read(ownerOf)?` (`:998-999`) and `io.outflow_check(…, Flow::Item, 1)?` (`:1027`) return
  from the whole function. So the first waiting delivery whose collection has used its 3 deliveries of the day (or whose token read
  fails) stops:
  - every later delivery of every collection;
  - the buy's fill;
  - the stray returns.

  The vector is in resolution order, so this lasts at most until that collection's limit window slides (≤ 24 h at stage 0,
  where open tickets are bounded by 3 held per collection).
- In `home_act`, deposit mints take the **first** `Mint` record (`:1385`). If it waits (its collection's mint limit), or its mint
  keeps failing (its listing paused: `ListingPaused` makes the estimate fail every tick), no other collection's deposit mints.
  The monitor pages at 26 h (`DEPOSIT_MINT_LATE_S`).

**PoC.** `audit_nft_c_one_collections_limit_blocks_every_other_collections_deliveries_and_deposit_mints` **passes**:
- (a) 12 hours of ticks with Milady's delivery at its limit: Azuki's delivery stays `Waiting` and nothing is signed;
- (b) a Milady deposit whose mint waits keeps an Azuki deposit at `Mint`, unminted.

(B-L-8 reported (a) at b48b3ba; (b) is new in e90471c.)

**Recommended fix.** Treat a per-delivery wait as `continue`, not `?`. Pick the first *mintable* deposit, skipping records whose
collection's limit or listing refuses. Do both round robin by collection.

### C-L-3 (Low): one full collection stops every other collection's buys; `Arrived` and watch-less `Stuck` have no exit

**Description.** `buy_pass` walks the collections round robin, but:
- `io.with_caps(check_acquire…).map_err(…)?` (`:653`) returns on the **first** collection that wants an item and has no room;
- a module that cannot be read returns too (`:647`).

`rr` is not advanced, so the same collection is first again next tick. Three holder deposits fill a collection's stage-0 cap;
while that collection's module still wants an item (after any of its graduations), Azuki and MAYC never start a buy. The global
cap (6) filled by deposits across two collections stops all three, by design of the cap.

Separately, one Ethereum buy is open canister-wide (`open_buy`, `:302-304`):
- an `Arrived` buy whose collection's floor ran away (D4: wait) blocks every collection's buys;
- so does a `Stuck` buy without a watch (`superseded` for `Buy` sets `last = None`, `:1163`), and `give_back_now` refuses it
  (`:1571-1572`).

**Impact.** Graduations of the other collections stay ETH-backed; nothing is lost. The NFT backing D1 promises is
halted by one collection's state.

**PoC.** `audit_nft_c_a_full_collection_whose_module_wants_an_item_stops_every_other_collection_from_buying` **passes**:
- 24 ticks with Milady full and wanting, and Azuki wanting with room: 24 cap refusals and 0 buys;
- when Milady stops wanting, an Azuki buy starts at once.

(B-M-5 at b48b3ba, rated Medium there. I rate it Low: it touches buying, not redemption or funds.)

**Recommended fix.**
- `continue` past a collection refused by its cap or unreadable, and advance `rr` past it.
- Give an `Arrived` buy an operator exit (the reverse Relay leg of D4, or a "release" that settles the advance as a loss after N
  days). Give a watch-less `Stuck` buy one as well.

### C-L-4 (Low): the M-2 fix does not cover the thirteenth element: an `EthState` of another shape comes back empty

**Description.**
- The upgrade tuple's thirteenth element is `Option<crate::ethflow::EthState>` (`canister.rs:393`).
- `post_upgrade` traps on a decode **error** (M-2's fix), then uses `eth.unwrap_or_default()` (`:425`).
- Candid's `opt` rule decodes an optional value whose content does not fit the expected type as **`null`**, not as an error.

So an upgrade whose `EthState` (or anything inside it: `DepositRec`, `XPhase`, `EthKind`, `Step`, …) changed shape
incompatibly restores an **empty** Ethereum side, silently. That includes:
- the signed line and the pending home transaction, which breaks save-then-send and the one-transaction rule;
- in-flight buys (an advance whose fill then mines unminted, and an unsettled module transit);
- deliveries and their phases;
- the ticket cursors, which go back to 0: every still-`Resolved` ticket becomes a new `Waiting` delivery;
- `used_orders` (Relay rule 12);
- the deposit records (every claim may be handed in again).

The same holds for the other optional elements (frogs, floors, custody, the fee pool). The "every new field optional" rule and the
e04f49b golden fixture do not catch it: e04f49b has no thirteenth element. **b48b3ba → e90471c is exactly such a change**:
- a new non-`opt` `outbound` field;
- `XPhase::Seen { block }` became `Seen`;
- `DepositRec` gained `from` and `block`.

Nothing is installed from b48b3ba, so this is latent.

**PoC.** `audit_nft_c_an_eth_state_of_another_shape_decodes_as_nothing_rather_than_refusing_the_upgrade` **passes**. The test
encodes `(Some(state without outbound, with a signed line at nonce 9, cursor 42, one used order)),` and decodes it as
`(Option<EthState>,)`: the result is `None`, with no error. Decoding the same state without the `opt` fails, as M-2 intends.

**Recommended fix.**
- In `post_upgrade`, trap when the previous build saved `Some` and it decoded as `None`. For example, save a
  `(version, bytes)` side element, or decode the element as `candid::Reserved` / raw first and check it is `null` before
  accepting `None`.
- Add golden fixtures of the thirteenth element from each released build to CI.

### C-L-5 (Low): nothing lets the operator return a held stray, or judge a claim the canister cannot read

**Description.** §2.4 says a stray is "held for the operator" when its return cannot be judged or signed. But:
- no endpoint returns, releases or re-judges a `Held` record;
- no endpoint re-judges a `Refused` one;
- a `Held` record never changes again.

Claims whose transaction input is over 64,000 bytes (`eth_tx_view`, `ethnet.rs:294`) or whose receipt is over 200,000 bytes
(`:289`) always fail to read. They are retried for 6 h and dropped unrecorded (`ethflow.rs:817-830`). The runner hands one in again
every 7 h while it is in its 7,200-block lookback, then never. `safeTransferFrom` to an EOA accepts any `data`, and a marketplace
batch can have a large receipt. The monitor's unclaimed page (`unclaimedAlerts`) reads the same 7,200-block window, so it stops
after about a day.

**Impact.** The NFT sits at the canister's address, neither minted nor returned, until a timelocked upgrade adds an exit. Every
`Held` from C-M-1 (b) falls in this case, as does any return whose simulation genuinely refuses (e.g. a sender now on a
collection's blocklist).

**Recommended fix.**
- Add an operator endpoint that re-arms a `Held` stray to `Return` (the canister re-judges it with every check), and one that
  re-opens a dropped claim with a larger read cap.
- Keep unreadable claims in a durable list (a count, never a decision) that the monitor pages until judged.

### C-L-6 (Low): deliveries rest on the 4663 fast pin

`scan_tickets` creates a delivery from `ticket(i)` and `item(i)` read at the tick's home-chain pin (`:519-556`). `eth_act`
re-reads the ticket at that pin before signing (`:998`). Neither needs the `Redeemed`/`Resolved` events to be final. On the
Bitcoin side, fixes-D H-1 made exactly this final ("a reorganisation can therefore no longer un-redeem the ticket or change its
item or its destination under a signed delivery", H9). A 4663 reorg past the fast pin (1,024–2,048 blocks, a few minutes) would
need the chain's sequencer to misbehave. Recommended: read the ticket's `Resolved` event at or below the home chain's finalized
block before signing, as `ticketlog` does for Bitcoin. (B-L-2 at b48b3ba, unchanged.)

### Info

- **C-I-1:** `EthKind::Return { tx, token }` and `deposit_mut(tx, token)` have no collection. If one transaction moved Milady #5
  and Azuki #5 to us:
  - both are strays, and the first record wins every lookup;
  - the Azuki return's receipt marks the Milady record `Returned` again and writes `outbound` under Milady;
  - the Azuki record stays `Returning` for ever. It is still `open()`, so `token_busy` holds any later Azuki #5 stray.

  Self-harm only. Fix: add `collection` to `EthKind::Return` and key by `(tx, collection, token)`.
- **C-I-2:** deposits enter H2 at value 0, so the value caps (10 ETH per collection, 10 ETH global) do not see them. Stage 0's
  item caps bind first for D1's collections (6 × < 1 ETH). They will not for Pudgy/BAYC-class collections (`nft-pool`). Use the
  collection's `max_purchase_wei` (or our floor reading) as the deposit's mark.
- **C-I-3:** `review2/fixes-R3-canister.md` §7 left two items "for when deposits are switched on":
  - a per-sender return budget (N-8);
  - a return-anomaly tripwire (N-10).

  Neither exists. With `judge_return`'s checks, a return can only send a stranger's NFT back to its own sender. The residuals
  are: the day's 3 returns per collection can be used up by one stranger (honest strays wait), and custody pays the gas.
- **C-I-4:** `return_act` uses the claim-time `block` and does not re-read the receipt past finality, as `deposit_pass` does. It
  reads `itemOf` at the home fast pin in the same tick that `resolve_home` may have seen a mint mined. To be exploitable, one token
  would need two open records at once. I could not construct that: ERC-721 moves the token out only through our own
  transactions. It needs a reorg deeper than the Ethereum pin (8–16 blocks). Re-read the receipt at finality, as for deposits.
- **C-I-5:** `io.paused()` is read once (`ethflow.rs:451`). A guardian's pause that lands during the tick's awaits (seconds of
  threshold signing) still lets the rest of that tick's Ethereum act and home act go out. A tripwire freeze inside `eth_act`
  still lets `home_act` post, record and mark. Re-check the pause before `sign_eth`, `cancel` and `send_home`.
- **C-I-6:** `cancel` checks neither the balance nor, later, the price. A cancel refused by the 50 gwei cap (a base fee over
  ≈ 12.5 gwei) leaves the delivery pending, and its refund then needs the 30-day window.
- **C-I-7:** state never pruned: `deliveries`, `deposits`, `outbound`, `used_orders`. `Refused` claim records come from any
  hash the runner hands in (≤ 4 a tick). All of it is cloned into the global at every `io.save` (about 8 a tick), and serialised
  whole in `pre_upgrade`. (With B-I-4.) Prune finished records after a retention period, and keep hashes only.
- **C-I-8:** `eth_nft_state()` blanks the inbox but keeps `line.raw` (for the monitor), so a pending delivery's signed bytes are
  public and anyone can re-broadcast them. That is fine by itself, but it removes any hope that an evicted delivery stays dead
  (C-L-1 (b), C-M-2's cancel race).
- **C-I-9:** docs.
  - `phase1-nft.md` refers to "§11.5" for D5's judgment calls (lines 18, 575, 585–588), and no §11.5 exists at e90471c.
  - §2.1 says "one purchase in flight per collection", while the code (and §11.2) have one canister-wide.
  - §7's "Disagreement is 'nothing believed'" is contradicted by C-M-1.

## Holder deposits: what I tried that held

- **Fake deposits:**
  - a Transfer log from another contract;
  - `tx.to` not the collection;
  - a plain `transferFrom`;
  - `safeTransferFrom` sent by an approved operator (`from ≠ tx.from`);
  - 36-byte (v4) data, another chain id, another wrapper (including another D1 collection's wrapper), a zero, wrapper or custodian
    recipient;
  - two Transfer logs of the collection in the transaction;
  - a reverted transaction.

  Every one is refused as a deposit. The ones that moved an NFT to us become strays that go back to their own `from`.
- **Replay and front-running:**
  - the claim is the owner's own signed transaction, so there is nothing to front-run;
  - a transaction is judged once (`st.deposits` dedupe, `put_deposit` refuses a repeat);
  - `(tokenId, tx)` mints once, and a held token never mints (wrapper);
  - an old deposit transaction handed in again after the token left and came back is either already recorded, or (if never
    recorded) cannot exist without our own outbound transaction in between, which `outbound`/`last_out` sees. The one gap is the
    orphaned delivery of C-M-3, which never reaches `outbound`.
- **Our own transactions** (fills, deliveries, returns, cancels) handed in as claims are refused by `from == us`. The runner
  also skips its own fills.
- **Reorgs:**
  - a `Seen` deposit is re-read in its final block (a moved or failed receipt waits);
  - `ownerOf = us` is checked after finality;
  - strays wait for finality.
- **Unit supply:** every mint comes after finality, `ownerOf`, `itemOf` and H2. A deposit racing a redemption is protected by
  the wrapper's pending stamps (only tickets burned after it can draw it).
- **The H3 tripwire (prior H-2):** returns never trip (`outflow.rs:206-208`), and one deposit mint asks at a time. Deliveries ask
  only after the judge and the simulation (N-12). Tickets are bounded by held items, so at most 6 delivery keys per collection in
  24 h, under the trip at 9. No deposit or return path can freeze custody.
- **Cycles (prior H-3):**
  - ≤ 4 claim reads a tick;
  - no reads for a `Seen` deposit before finality;
  - one final read-back each;
  - one return judged a tick;
  - nothing for `Held`/`Refused`.

  A stranger's NFTs are needed for anything more, and each is returned.
- **Gas pot:**
  - returns keep `arrived + gas + 0.005 ETH`;
  - deliveries check their own gas;
  - the attacker pays the gas to send;
  - at the measured fees a return costs ≈ 60k gas × ≤ 0.3 gwei.
- **Other collections:** an NFT of an unconfigured collection is never called (no stray is recorded for it).
- **Calldata parsing:** `abi::bytes_at` / `word_at` bound every offset and length with `checked_add` or a bounded `u32`.
  `data = start + 32` cannot wrap in a way that yields a slice: `word_at(start)` fails first. No panic or trap on hostile input
  was found.

## The runner's discovery and the monitor

- **The runner** (`ops/src/ethnft.ts:208-221, 297-309`) lists `Transfer(*, us, id)` logs of each configured collection over the
  last 7,200 blocks on one `ETH_LOGS_RPC`. It skips transactions already judged and its own fills, and hands each in at most once
  per 7 h.
  - **A withholding runner** (or its one RPC) delays deposits and returns. The monitor pages an unjudged inbound transfer after
    600 blocks (SEV2) on its own provider, but only while it is inside the monitor's 7,200-block window (C-L-5).
  - **A malicious runner** can hand in arbitrary hashes (each becomes a permanent `Refused` record, ≤ 4 a tick; C-I-7), withhold
    listings (an `Arrived` buy waits for ever: C-L-3) or evidence (stale floors), and choose among valid listings under the cap.
  - It **cannot** forge a deposit, pick a recipient or a return destination, mint before finality, or deliver anywhere but the
    ticket's destination.
- **The monitor** catches:
  - the phantom item of C-M-3 and C-L-1 (custody SEV1: a held item not owned by the custodian);
  - `Held` strays (SEV2, persistent), late `Mint` (26 h), late returns (2 h), deliveries near their deadline (SEV1), a fill
    unmined for an hour, a low gas pot.

  It cannot fix any of them.

## Re-check of the 2026-09-26 canister findings on the code this branch touched

| ID | Status at e90471c (Ethereum side) | Evidence |
| --- | --- | --- |
| H-2 junk deposits freeze custody | **Holds (fixed).** Returns are `Flow::Return` (never trips), one deposit mint asks at a time, deliveries ask after their facts | `outflow.rs:206-208`; `ethflow.rs:946-948`, `:1381-1402`, `:1027` |
| H-3 cycles per pending deposit | **Holds (fixed).** ≤ 4 claims a tick, a `Seen` deposit is not read before finality, one return a tick; nothing grows per tick with a stranger's input except `Refused` records (C-I-7) | `ethflow.rs:36`, `:806-866`, `:925-970` |
| H-4 dust breaks fee selection | **N/A** on Ethereum (no UTXO listing). The analogues (stray NFTs, ETH gifts) are bounded: returns by budget, gifts counted as documented arrivals | §4.5 |
| M-1 LostRace | **Design holds** (mark only once no version can confirm, after finality; the late window is 30 days). **Residuals:** C-M-3 (orphan), C-L-1 (pause) | `ethflow.rs:1099-1159`, `:760-767` |
| M-2 upgrade decode | **Holds for decode errors; residual C-L-4** (the opt rule turns an incompatible `EthState` into `None`) | `canister.rs:396-440` |
| L-1 return before save | **Holds (fixed)**: `sign_eth` saves the line before the broadcast, returns included | `ethflow.rs:979-986` |
| L-2 mid-tick upgrade | **Holds**: `pre_upgrade` traps while `TICKING` | `canister.rs:358-360` |
| L-4 lost updates mid-tick | **Holds**: all five `ethnet` updates call `not_ticking()` (source test), and `eth_give_back` checks it after its await | `ethnet.rs:330-379`, `:398-411` |
| L-5 a signed buy held for ever | **Open on Ethereum**: C-M-2 (no replacement for a fill or return) | `ethflow.rs:1071-1096` |
| L-7 provider extremes | **Open (Info)**: `eth_finalized` is the minimum with no bound (B-I-2); a single provider's failure at the simulation is final (C-M-1) | `ethnet.rs:193-201` |
| R2 N-7 duplicate deposit records | **Holds**: dedupe by transaction hash | `ethflow.rs:810`, `:1627` |
| R2 N-8 intake starvation, returns paid by custody | **Open (Info C-I-3)**: no per-sender budget; "costs an inscription" is now "costs gas" | — |
| R2 N-10 returns never trip, no anomaly tripwire | **Open (Info C-I-3)**; mitigated by `judge_return` | — |
| R2 N-12 limiter asked before facts | **Holds**: `eth_act` asks after `judge_transfer` and the plan | `ethflow.rs:1000-1027` |
| R3 M-1(c) paused pass records and conflicts | **Not carried to Ethereum**: C-L-1 | `canister.rs:1346` |
| fixes-D H-1 / H9 finality of ticket facts | **Not carried to Ethereum**: C-L-6 | `ethflow.rs:519-556` |
| B-H-2 (area B, b48b3ba) | **Deposit half fixed** in e90471c (`Return` on no room; `deposit_wait`); **buy half open and now fed by deposits**: C-H-1 | `ethflow.rs:1381-1433` |

## Suites

| Run | Result |
| --- | --- |
| `cargo test` at e90471c (baseline) | **544 passed, 0 failed** |
| `cargo test` with `tests/audit_nft_c.rs` | **552 passed, 0 failed** |
| `tests/audit_nft_c.rs` alone | 8 passed. Each asserts the weakness (C-H-1, C-M-1, C-M-2, C-M-3, C-L-1, C-L-2, C-L-3, C-L-4) and turns red once fixed. Output in `poc/C/audit_nft_c.output.txt` |

## Not covered

- The floor evidence judge, the Relay judge, Seaport and `nftnet::check_any` internals (areas A and B). I used them only as black
  boxes bounding the buy.
- `EthNftWrapper.sol` beyond the functions the flow calls (area A).
- Live provider behaviour. I made no network calls; how often an agreed `eth_call` or receipt read fails on 13 replicas is what
  sets C-M-1's and C-M-3's frequency (B-M-6 has measurements).
- The keeper, indexer, site and golive-check changes in e90471c.
