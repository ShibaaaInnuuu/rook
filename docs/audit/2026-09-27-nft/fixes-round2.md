# Fix round 2: the 2026-09-27 review of the Ethereum NFT side, round 2

Written 2026-09-27 on branch `nft-side` (at `79455e1`: fix round 1, `27c0962`, and the round-2 re-review's reports and PoCs). It
answers `review2/README.md`, `review2/R2-contracts.md`, `R2-canister-floor.md`, `R2-canister-flow.md` and `R2-services.md` finding by
finding: the fix, where it is, and the test that is the reviewer's PoC turned around (it fails on the code before this round and
passes now). §11 ("As built") has the commits, the test counts and the reproducible-build hashes. Nothing was deployed, pushed or
signed with a real key; the only chains written to were local anvils. The network use was read-only: an anvil fork of Ethereum for
the Seaport fork proof, the dry run's fork of 4663, and the reproducible build's crate fetch.

**The rules every fix keeps** (as round 1's):

- Nothing is weakened to buy availability: H1 (pause), H2 (custody caps), H3 (the outflow limiter), "save, then send", and every
  judge (`relayeth`, `seaport`, `ethnft`) are unchanged in what they refuse. The way back (§2) is judged by the same `relayeth::judge`
  as the forward leg, with `FROM_ETHEREUM`.
- Stable state stays decodable: every new field is `opt` (`Option` with `#[serde(default)]`), every new enum case is appended
  (`BPhase::BackFunded`, `BackSending`, `BackBridging`; `EthKind::BackDeposit`; `FloorPost.held`; `EthState.sale_scan`). The upgrade
  probe now refuses an upgrade in which ANY optional part of the saved state, at any depth, would restore as nothing (C2-L-3).
- Every outcall a stranger can cause stays bounded per tick (the canister's own sale scan: 13 log chunks and 16 receipts a pass).
- **Not changed** (operator-pending or operator-decided): BAYC's graduation target stays 9 ETH (J1); the stage-0 custody caps are
  unchanged (J7); D1–D9 and D5 as decided. Two decisions are touched only where this round's brief requires it, and each is listed in
  §9 for the operator: D4's reverse Relay leg (which D4 left to "a reviewed upgrade") is wired with D4's wait and 24 h page kept, and
  D7's numbers are kept while what they are measured against changes (the counted sales' lower quarter; no bound stops a post).
- **The Floor Module is unchanged.** An on-chain floor gate on `advance` was built and dropped: a frog's advance is one
  inscription's price, which may be under the floor, so the module cannot refuse an advance under `accepted` without breaking the
  Bitcoin side (30 forge tests and the module's invariant suites failed with it). The gate is the canister's (§2).

## 0. Summary

| ID | Sev | Status | Fix | Where | Test (the PoC, turned around) |
| --- | --- | --- | --- | --- | --- |
| **A-M-1** | Med | **Fixed** | The canister reads each collection's sales ITSELF (its own `eth_getLogs` of the collection's `Transfer` over the final blocks of the 7-day window, each receipt once): no runner chooses which sales count. A sale counts once per token and once per WALLET, never a token that went back to an earlier holder. A deposit mints only while `accepted ≤ min(L, S25) × 1.05` with `min_sales` counted. The burn side is the post: it rises only as far as `S25 × 1.10` and never above a live listing | `ethnft.rs` `counted_sales`, `round_trips`, `floor_from`; `ethflow.rs` `sale_scan`, `reading_of`, `deposit_floor_ok`; `ethnet.rs` `eth_transfer_txs` | `audit_nft_r2.rs` `r2_a_m1_wash_sales_no_longer_open_the_deposit_band_…` (ring, disjoint pairs), `r2_a_m1_the_burn_side_…` |
| **B2-M-1 = D2-M-2** | Med | **Fixed** | Sales can HOLD the floor back, never stop a post: confirmed → `L`; too few counted sales or `L` over `S25 × 1.10` → `min(L, max(accepted, S25 × 1.10))` at confidence 5,000; `L` under `S25 × 0.85` → `L`, flagged. Only too few listings (or an unconfirmed FIRST post) is no post. The sample is the whole window, so no volume of washes crowds real sales out. The runner hands in listings only | `ethnft.rs` `floor_from`; `ethflow.rs` `floor_pass`; `ops/src/ethnft.ts` `ethNftOnce` | `r2_b2_m1_ping_pong_sales_no_longer_stop_the_floor_post`, `r2_b2_m1_in_band_washes_can_hold_the_floor_but_never_stop_a_post`; `ops/test/audit_nft_d_r2.test.ts` "D2-M-2" |
| **A2-M-1** | Med | **Fixed (canister)** | No buy starts and no advance is signed unless `item_cap − GAS_KEEP − item_cap × 0.5 %` ≥ `accepted × 1.01 + 0.003 ETH` (`advance_covers`, read again right before the advance). An `Arrived` buy that has not filled for 48 h (or at once, `eth_give_back`) goes BACK: a Relay order Ethereum → 4663 judged by `relayeth::judge` with `FROM_ETHEREUM`, signed on the Ethereum line, watched on 4663 (arrival → `returnAdvance` to its module) and on Ethereum (refund → `Arrived` again). `Pooled.t.sol`'s floor-rise test is real | `ethflow.rs` `advance_covers`, `buy_pass`, `step_buy`, `back_now`, `eth_act` step 3, `mined_eth`; `ops/src/ethnft.ts` `preJudgeEth` (both legs) | `r2_a2_m1_a_module_whose_item_cap_does_not_cover_its_floor_never_advances`, `r2_a2_m1_an_arrived_buy_that_cannot_fill_goes_back_…`, `r2_a2_m1_a_way_back_refunded_on_ethereum_…`; `test/ethnft/Pooled.t.sol` `test_aFloorRiseWhilePooled_…`, `test_A2_M1_…` |
| **C2-M-1** | Med | **Fixed** | No Relay quote is judged (no arrival watch starts) in a tick whose line outcome was not read at this pin (a receipt, the nonce or `ownerOf` unread; `unsettled`). An arrival earmarks at most the order's value and at most what the balance holds beside every other earmark (else it waits); the watch ends at the arrival | `ethflow.rs` `tick`, `resolve_eth`, `home_act` (Funded), `arrive` | `r2_c2_m1_a_fill_mined_but_unread_is_never_read_as_another_collections_arrival`, `r2_c2_m1_an_arrival_is_never_earmarked_over_the_balance` |
| **C2-M-2** | Med | **Fixed** | An `Arrived` buy holds its collection's deposit mints for 6 h at most; `Stuck`, `Back*` and `Returning` hold nothing. And it goes back after 48 h (A2-M-1), the operator's `eth_give_back` at once | `ethflow.rs` `buy_holds_mint`, `ARRIVED_HOLD_SECS`, `give_back_now` | `r2_c2_m2_an_arrived_buy_that_cannot_fill_holds_deposits_six_hours_at_most_then_goes_back` |
| **D2-M-1** | Med | **Fixed** | (a) A buy's earmark comes off once the chain shows its line transaction's nonce used (its fill, cancel or way back mined, the canister not yet ticked): what it may have spent (value or total, gas at the line's max fee) is taken off, never more. (b) An item not owned by the custodian is judged after the canister's records are read again: a delivery of ours in `Sent`/`Moved`/`Record`/`Recording`/`Recorded` to that very owner is not a loss; any other owner is SEV1 as before | `monitor/src/ethnft.mjs` `earmarkOf`, `checkEthNft` (suspects) | `monitor/test/audit_nft_d_r2.test.mjs` "D2-M-1a", "D2-M-1b" (each with its control: a real loss still pages) |

**Round 1's Medium A-M-1 and the five new Mediums are closed: 0 Critical, 0 High, 0 Medium open by this round's own reading.**
Round 3 re-reviews the delta (§11).

## 1. The floor sample (A-M-1, B2-M-1 = D2-M-2, B2-L-1, B2-L-2, D2-L-1, D2-L-2, B2-I-1)

### 1.1 What the canister reads, and why it closes the runner's choice

- **Its own sales.** For each collection with evidence handed in, the canister reads the collection's `Transfer` logs itself
  (`EthIo::eth_transfer_txs`: `eth_getLogs` on every provider, projected by the `eth_transfer_logs` transform to
  `[txHash, block]` for every ERC-721 move between two wallets, one per transaction, sorted: providers agree on what moved, not on
  how they wrote it). Only FINAL blocks (`SALE_FINAL_DEPTH_BLOCKS` = 128 under the pin), over the 7-day window, newest first: 13
  chunks of up to 2,000 blocks a pass (a failed chunk is halved, down to 25 blocks, and grown back after), the three size classes
  64 kB / 400 kB / 2 MB. Then up to 16 receipts a pass, newest first (`sales_in`, unchanged). Every judged receipt is kept by
  (collection, hash) — a sale or not — so each transaction is read once (`sale_cache`, 2,048 per collection, pruned to the window);
  `SaleScan` keeps the blocks read and the transactions still to judge (512 at most).
- **The runner hands in listings only.** `eth_floor_evidence` still takes up to 12 sale hashes (the interface is unchanged) and
  ignores them. `ops/src/ethnft.ts` no longer reads sales (`recentSaleTxs`, `isSale` and the per-transaction verdict cache are gone:
  D2-L-2 is moot). What the runner can still do is withhold listings (§1.4).
- **B2-L-2:** a receipt the IC refuses at the largest class is final: it is kept as no sale and never paid for again.
- **Cost** (13-node pricing, 2 providers): a log chunk at the 64 kB class ≈ 1.4 B cycles, a receipt at 32 kB ≈ 0.8 B. Steady state
  per pass ≈ one chunk and the few new transactions: ≈ 3–5 B, ≈ 0.2–0.4 T a day per collection at the runner's 20 minutes. The
  window's backfill is paid once (≈ 37 B of logs; ≈ 0.8 B per transaction that moved a token, most for MAYC). A stranger can make
  the queue longer only with real transfers (gas each); that delays the sample, and cannot stop a post (§1.2).

### 1.2 What counts, and what a count can do

`counted_sales` (newest first, over the window, our own purchases left out):

- no self-trade, no zero seller, nothing further than ±30 % from `L` (unchanged);
- **no token that went back to a wallet that held it earlier in the window** (A → B → A, or round a ring): all its sales are washes;
- **one per token and one per WALLET**: a wallet that sold or bought in a counted sale is a party to no other. A chain of wallets
  counts every other link; a Blur bidder buying from ten sellers is one voice; a wash vote needs its own token and two wallets that
  trade nothing else in the window.

`floor_from(listings, sales, params, accepted)` (B2-M-1, B2-L-1):

| Case | Post |
| --- | --- |
| `min_sales` counted and `S25 × (1 − down_cap) ≤ L ≤ S25 × (1 + up_cap)` | `L`, confidence by `|L − S|` as before (confirmed) |
| fewer than `min_sales` counted, or `L > S25 × (1 + up_cap)` | `min(L, max(accepted, S25 × (1 + up_cap)))` (the latter only with a quorum), confidence 5,000, `held` says why |
| `L < S25 × (1 − down_cap)` | `L`, confidence 5,000, flagged in `held` |
| accepted = 0 (the first post) and not confirmed | no post (nothing to hold yet) |
| fewer than `min_listings` verified listings | no post (as before); the last reading is dropped (B2-I-1) |

So the post never leaves the listings' range; it follows the listings DOWN within the module's ±5 %/h and ±20 %/day (a price
anyone can fill; burns price at the posted target), and it rises only as far as the counted sales' lower quarter + 10 % supports.
Washes can hold a rise back; they can no longer freeze the module (liveness depends on three live listings, not on the absence of
wash trades). The band is over `S25`, not `S` (B2-L-1): lifting it takes wash votes outnumbering three quarters of the window's
real distinct-wallet sales, each vote its own token and two fresh wallets.

### 1.3 A-M-1: the conservative choice, and why

The review offered two ways: the canister choosing its own sample (closes the washer's median and the runner's choice), or a module
rule capping fresh burns at the trailing-24 h low of `accepted`. **Built: the first, with a stricter band, and the post itself as
the burn side's bound.**

- **Mint side.** `deposit_floor_ok`: a fresh reading with `min_sales` counted sales, and the module's accepted floor at most 5 % over
  the LOWER of the listing floor `L` and the counted sales' `S25`. Against the reviewer's attack (7 in-band washes along a chain, or
  7 disjoint wash pairs, at 1.20 with the real sales at ≈ 0.91): the chain counts 4 votes; either way the lower quarter stays with the
  real sales and the deposit waits (test, both shapes). The runner no longer chooses the sales at all.
- **Burn side.** A burn pays the posted floor (at most its target, at most the backing). The floor can only be ABOVE the market if
  it rose there, and it rises only while (a) there is a live listing at that price anyone can fill and (b) the canister's own counted
  sales' lower quarter supports it within 10 %. A depositor who minted in band and burns after a rise is paid a floor that both the
  book and the market's own sales show: the NFT's value, not a premium over it.
- **Why not the module rule.** It changes every holder's exit (a burn in a rising market pays the 24–48 h low, up to 20–36 % under
  the floor), on the frogs' module too (reviewed over four rounds), to close a gap that, with the canister's own sample, only remains
  for an attacker who both sweeps the book and outnumbers three quarters of the week's real sellers and buyers with disjoint wash
  wallets and tokens. That residual is stated on the Trust page (D2-L-1) and in §10.

### 1.4 What the runner and an outsider can still do

- The runner withholds listings: with the cheap ones withheld the floor can rise at most to `S25 × 1.10`; with fewer than three it
  stops posts (A-I-6, as before; the D8 no-good-post pause is armed in the launch monitor configuration: §6).
- An outsider's washes can hold a rise back (the pool then waits: the monitor's floor-gap and pool-idle pages); many real transfers
  lengthen the scan's queue. Neither stops a post or opens the deposit band.

## 2. A2-M-1 and C2-M-2: no advance under the floor, and a way back

- **The gate** (`ethflow::advance_covers`): `item_cap − GAS_KEEP_WEI − item_cap × ADVANCE_FEE_EST_BPS (50)` ≥
  `accepted × (1 + ADVANCE_MARGIN_BPS (100)) + ETH_GAS_RESERVE_WEI`. Checked before a buy starts (so a short pool never takes the
  Relay leg) and again right before the advance (a buy whose floor rose meanwhile is dropped, nothing paid). `accepted` is read with
  the module view (`floor()`'s first word). The judged quote is still bounded by `relay_max_fee_bps` (300); the 50 bps is Relay's
  measured 0.36 % with room. Pooled BAYC: +1 % between graduations converts, +3 % waits on 4663 (`Pooled.t.sol` scan).
- **The way back.** New phases: `BackFunded { wei, since }` (the runner is asked for a quote Ethereum → 4663 for what arrived less
  the 0.003 ETH gas reserve), `BackSending` (the Relay deposit on the Ethereum line, `EthKind::BackDeposit`), `BackBridging` (mined;
  the home balance watched for the arrival, the Ethereum balance for a refund). The watch is taken BEFORE the deposit is signed (4663
  as it was; Ethereum with the value still there). Arrival (4663 up by half the order's minimum, only this leg moves it) →
  `Returning { min(rise, value) }` → `returnAdvance` to the buy's own module; a refund on Ethereum → `Arrived` again (it tries again
  after another 48 h). The deposit takes the one Relay leg; while a buy waits to go back no new buy starts (it is next on the leg).
  Judged exactly like the forward leg (`relayeth::judge`, `FROM_ETHEREUM`: the pinned depository and allocator on Ethereum, our own
  address everywhere, native ETH, the fee bound, rule 12); `eth_plan_call` simulates it and checks the balance covers value and gas;
  what it spends is strictly this buy's (never another buy's earmark). The runner's `preJudgeEth` takes either leg.
- **C2-M-2.** `buy_holds_mint(collection, now)`: an `Arrived` buy holds the collection's deposit mints for `ARRIVED_HOLD_SECS` (6 h);
  `Stuck`, `BackFunded`, `BackSending`, `BackBridging` and `Returning` hold nothing (if such a buy ever mints, its mint waits for the
  day's room like any other). The operator's exit: `eth_give_back` on an `Arrived` buy starts its way back at once.
- **`Pooled.t.sol:232-249`** (vacuous): it now makes the canister's call, `advance(maxItemCost())`, against the canister's gate in
  Solidity (`_canisterAdvances`, the same numbers), and states what holds: the MODULE would pay the short blend out (it has no floor
  gate: §0's note); the canister refuses it. A second test scans the rise (+1 % converts; +3 %, +5 %, +10 % wait) and checks an
  unpooled collection's full cap passes. The canister's refusal itself is driven in `audit_nft_r2.rs`.
- **Docs:** `phase1-nft-pooled.md` §3.4 corrected (the claim was false as built), J1's headroom restated, A2-L-1's sentence
  corrected; `phase1-nft.md` §2.1, §4.5, §4.6, D4.

## 3. Arrival accounting (C2-M-1, C2-L-1, L-2)

- **`unsettled`** (a tick-local flag): set when a version's receipt, the nonce or (in `mined_eth`) the bought token's `ownerOf`
  could not be read at this pin, or when `superseded` could not tell what took the nonce. The line then still holds a transaction
  that may be mined here uncounted in `eth_out`; the `Funded` step leaves its quote in the inbox and waits a tick ("C2-M-1"). A line
  signed this tick is after the pin and cannot be in the balance read at it.
- **`arrive`** (Bridging, Stuck, and a way back's refund): earmarks `min(got, value, balance − Σ other earmarks)`; if that is less
  than `min(got, value)` nothing arrives (a wait: the accounting disagrees with the chain). So the earmarks never exceed the balance.
  The settle cost uses what was really earmarked.
- **C2-L-1:** the arrival clears the Relay watch (`last`), so a post-arrival `Stuck` is off the Relay leg and never re-arrives spent
  ETH; the L-2 sweep's `Stuck`-without-watch branch is live again and releases its `eth:{id}` reservation (test).
- "A later 4663 refund can't strand a buy in Arrived": the phantom arrival that made it possible is gone; and every `Arrived` buy now
  has a way back (§2).

## 4. Services (D2-M-1, D2-L-3, D2-L-4, D2-I-*)

- **D2-M-1** as §0. The allowance per mined line transaction is its value (the fill's total for a cancel's rival) and its gas at
  the line's max fee over at least 300,000 gas (`LINE_GAS_ALLOW`). The monitor's Candid types now read `value`, `max_fee`, `gas` of
  the line and know the way back's phases (an unknown variant is a decode error; `tests/fixtures/eth-state-round2.hex`, held to what
  the canister encodes, is decoded by the monitor's and the runner's IDL in their tests).
- **D2-L-4:** `pools`, `refMissing` and `ownerUnread` are carried from the last poll and replaced only by a collection read in full.
- **D2-L-3:** `ethNftOnce` checks what the Ethereum step lacks (`ethStepProblems`) against what the canister serves on every pass;
  `/healthz` is 503 while one is open (a restart that met a blip, a collection added by an upgrade). The start-time refusal stays.
  `ETH_LOGS_RPC` is now needed only for deposits discovery.
- New monitor pages: a way back waiting for its quote (2 h, SEV2), a way back unpaid (30 min, SEV2); the `Arrived` page says the
  ETH goes back at 48 h.
- The monitor refuses an `eth_nft` configuration with D8 armed and no `evm.guardian.pause_on_no_good_post_h`.

## 5. Upgrade safety (C2-L-2, C2-L-3)

- **C2-L-3:** `canister::lost_opts(saved, restored)`: the stable bytes and the restored state (encoded again) are read as untyped
  Candid values (`candid` feature `value`) and walked together; an `opt` saved present that restores as absent, at any depth, is
  named, and `post_upgrade` traps. A field or case the new type dropped is not compared. The reviewer's case (an `nft-side` `EthTx`
  inside `line`) is found; a state of this build's own shape, nested options and all, loses nothing (test). The element-level probe
  (`lost_elements`) is kept as a function (round 1's test).
- **C2-L-2:** `ethflow::stranded_by(state, new config)`: an open buy whose collection, wrapper and module the new configuration does
  not name together, an unfinished delivery of a wrapper it does not name, an open or held claim of a collection it does not name, or
  a line with no configuration: `post_upgrade` refuses. If a collection is gone anyway, its `NeedAdvance` buy is dropped and its
  `Funded` buy returns the advance to its own module (the Relay leg is freed).

## 6. The launch configuration (D2-L-5, A2-I-2, the review's launch notes)

- **`script/golive-check.sh --launch deployments/launch/nft-pilot.json`**: each Ethereum collection's factory targets
  (`normalRaise`, `earlyRaise`, `earlyUntilEth`, `maxRaise`) and module `convBps` / `reserveBps` against the LAUNCH file (the frog
  defaults where it names none), the file naming exactly the stack's collections, and with `--canister` each collection's
  `max_purchase_wei` against the file's `maxPurchaseWei` (new: 2 / 2 / 2 / 4 / 8 ETH). REQUIRED under `--phase1` for an NFT stack.
  A shifted comma list (BAYC's 9 ETH on Pudgy) fails (dry run).
- **`--monitor-config <file>`**: the launch monitor configuration watches each collection with its wrapper, module and custodian as
  deployed, names both D8 references, arms the guardian (`auto_pause`, D8 not off) and `pause_on_no_good_post_h`, with no
  placeholder left and two Ethereum providers. REQUIRED under `--phase1` for an NFT stack. **`deployments/launch/monitor-nft.json`**
  is the template: the five collections with OpenSea slugs (`milady`, `azuki`, `mutant-ape-yacht-club`, `pudgypenguins`,
  `boredapeyachtclub`) and CoinGecko ids (`milady-maker`, `azuki`, `mutant-ape-yacht-club`, `pudgy-penguins`,
  `bored-ape-yacht-club`) — to be checked against each contract on the sites before launch — `pause_on_no_good_post_h` 6, the
  Ethereum pair `eth.drpc.org` + `xrpc.cl/eth` (B-M-6's recommendation); the monitor's test fills it in and validates it.
- **`--canister-evm`** under `--phase1`: a FAIL when absent (a WARN only under `GOLIVE_REHEARSAL`).
- **`DeployStack`** (A2-I-2): basis points bounded before the `uint16` cast; a one-collection variable (`ETCH_NFT_NORMAL_RAISE` …) is
  refused when several collections are deployed.
- **J7 and BAYC's 9 ETH are untouched** (operator-pending): golive-check still passes BAYC at stage 0.

## 7. The Lows (18 open at `79455e1`)

| ID | Status | What |
| --- | --- | --- |
| B2-L-1 | Fixed | §1.2 (the band over `S25`, the canister's own wallet-diverse sample): `r2_b2_l1_…` |
| B2-L-2 | Fixed | §1.1: `r2_b2_l2_an_oversized_final_receipt_is_read_once_and_kept_as_no_sale` |
| C2-L-1 | Fixed | §3: `r2_c2_l1_…` |
| C2-L-2 | Fixed | §5: `r2_c2_l2_…` |
| C2-L-3 | Fixed | §5: `r2_c2_l3_the_upgrade_probe_refuses_an_older_eth_tx_inside_the_line` |
| D2-L-1 | Fixed | The Trust page states the canister's own sample, the band, the way back and the residual (`web-rook/src/rook/phase1Trust.ts`) |
| D2-L-2 | Fixed (moot) | The runner keeps no sale verdict; the canister's cache is per (collection, hash) |
| D2-L-3 | Fixed | §4: `ops/test/audit_nft_d_r2.test.ts` "D2-L-3" |
| D2-L-4 | Fixed | §4: `monitor/test/audit_nft_d_r2.test.mjs` "D2-L-4" |
| D2-L-5 | Fixed | §6 (dry run: a shifted launch list, a wrong purchase limit, `--phase1` without `--canister-evm`, without the launch file) |
| A2-L-1 | Partly | The doc is corrected (`phase1-nft-pooled.md` §1: the leftover goes to the gas pot, ≈ 0.41–0.75 ETH per ape at `convBps` 1,100). Not built: advancing only the buy bound, or returning the leftover. The committed config (300) is unaffected |
| A2-L-2 | Not changed | J7 (stage-0 caps against BAYC and Pudgy): operator-pending, by instruction |
| A-L-2 | Not fixed | The deploy key keeps the NFT wrapper's mint-limit powers; golive-check holds `pendingMintLimit() == 0` for NFT wrappers |
| B-L-5 | Partly (improved) | C2-L-2's guard covers dropped collections; `validate` still does not require an explicit `custody_caps` to list every eth collection |
| B-L-6 | Not fixed | Unreachable in the NFT-only stack (as round 1) |
| C-L-5 | Partly | As round 1 |
| D-L-3 | Partly | As round 1 |
| D-L-6 | Partly (improved) | `--canister-evm` is a FAIL under `--phase1`; the canister still does not read `EthNftWrapper.custodian()` itself |

**Lows: 18 — 10 fixed, 5 partly (A2-L-1, B-L-5, C-L-5, D-L-3, D-L-6), 3 not (A2-L-2 = J7, A-L-2, B-L-6).**

## 8. The Infos

Fixed on the way: B2-I-1 (a judged "no post" drops the reading: no stale post or deposit on it), B2-I-2 (the comment), B2-I-3 (the
deep-crash "no post" is gone: a crash is followed at the module's pace; `phase1-nft.md` §4.2 says what a stale module does),
A2-I-1 (the Trust page: the transit may last up to 48 h before the way back), A2-I-2 (§6). Unchanged: C2-I-1 (no gas reserve in
`available`; self-correcting), C2-I-2 (`eth_line_clear` trusts the operator), C2-I-3 (reorgs deeper than the pin), A2-I-3
(`PosterUnpaused` on an unpaused poster), D2-I-1 (D8 is a guard against a broken reading, not the book), D2-I-2 (the ack), D2-I-3
(the runner's pre-check vs the canister's bound; unpruned maps), D2-I-4 (the 4 MB cap after buffering), D2-I-5 (the site's pool
figure), D2-I-6 (keeper-era walk rules).

## 9. New judgment calls for the operator

1. **Sales hold the floor back, never stop it** (§1.2): with fewer than three counted sales the floor cannot rise (it holds, and
   follows the listings down); in a rising market with thin distinct-wallet sales (a few Blur bidders), conversions can lag and the
   floor-gap page fires. The alternative (posting the listings upward unconfirmed, paced by the module) reopens the swept-book and
   withheld-listing lifts.
2. **One vote per wallet, none for a round-tripped token** (§1.2): a busy bidder counts once.
3. **The deposit band is the lower of `L` and `S25`, +5 %** (§1.3), not the median.
4. **D7's numbers, measured anew**: +10 % over `S25` (was the median); −15 % under it flags, no longer refuses.
5. **The advance gate's margins** (§2): Relay's fee estimated at 0.5 %, the floor + 1 %, 0.003 ETH: a pooled BAYC waits from about a
   2 % rise between its graduations (J1's 2.6 % was before Relay).
6. **D4's reverse leg, wired** (§2): the wait and the 24 h page are D4's; after **48 h** (`ARRIVED_BACK_SECS`) the ETH goes back
   to the module through Relay automatically, and `eth_give_back` does it at once. The alternative is operator-only (the automatic
   step off).
7. **An arrived buy holds its collection's deposits 6 h at most** (`ARRIVED_HOLD_SECS`).
8. **`pause_on_no_good_post_h` = 6** in the launch template (2 to 160; the monitor now refuses D8 without it).
9. **The first post of a collection needs a confirmed reading** (three counted sales and the listings in band).
10. **The monitor's allowance for a mined line transaction** before the canister has read it: its value and 300,000 gas at its max
    fee.

## 10. Left open (for round 3)

A wash trader with more separate wallets and tokens than three quarters of the week's real distinct-wallet sales can lift `S25`
(and with a swept book, the floor and the deposit band); the module-side burn cap (not built, §1.3). The runner can still stop posts
by handing in fewer than three listings (A-I-6). A-L-2; B-L-5's caps coverage; B-L-6; C-L-5's re-open; D-L-3's runner cursor;
D-L-6's in-canister `custodian()` read; B-M-6's go-live read of the canister's provider pair (the monitor template names it; the
canister's `evm_chains` is not read by golive-check); A2-L-1's leftover under `convBps` 1,100; J7; every Info listed as unchanged in §8.

## 11. As built (2026-09-27)

**Commits** (branch `nft-side`, on `79455e1`):

| Commit | What |
| --- | --- |
| `f9166f8` | Canister: the own sale sample, the floor rule, the deposit band, the advance gate and the way back, the arrival accounting, the upgrade guards; `tests/audit_nft_r2.rs` (B's and C's PoCs as regressions); the fakes' module floor made one their item cap covers |
| `194e5f1` | Canister: a listing floor far under the counted sales is posted, flagged (D7's −15 %) |
| `828e7bf` | Services: the runner (listings only, the way back's quote, per-pass problems), the monitor (D2-M-1, D2-L-4, the way back's pages), the Trust page; D's PoCs as regressions |
| `24fba50` | Services: the runner's IDL knows the new phases; the round-2 state fixture decoded by the monitor and the runner |
| `c332813` | Contracts tests: `Pooled.t.sol`'s floor-rise test made real, the gate's scan |
| `4ebf761` | Go-live: golive-check `--launch` / `--monitor-config`, `--canister-evm` a FAIL at go-live, `max_purchase_wei`; `deployments/launch/nft-pilot.json` purchase limits, `deployments/launch/monitor-nft.json`; `DeployStack` A2-I-2; the monitor refuses D8 without the no-good-post pause; the dry run shows each refusal |
| (this document's commit) | This document; `docs/v7/phase1-nft.md` (§2.1, §2.3, §4.2, §4.5, §4.6, D4, D7, §11.8), `phase1-nft-pooled.md` (§1, §3.4, J1); `fixes-round2-poc/` (the reviewers' PoCs re-run on this code) |

**The reviewers' PoCs on this code** (`fixes-round2-poc/`): canister B 0 of 5 pass (each fails on its attack assertion; two
compile adaptations: the fake's two new `EthIo` methods, and `floor_from`'s new `accepted` argument), canister C 1 of 5 pass (the
C-L-4-style one tests Candid itself, not the probe, as the reviewer noted for round 1's; the others fail on their attack assertions,
with the fake's module floor set to one its item cap covers so the controls hold); monitor 0 of 3 pass; ops: the file no longer loads
(`recentSaleTxs` is gone); forge (`poc/A`): the A2-M-1 PoC still passes on chain by design (the module is unchanged: the canister
refuses the advance, `audit_nft_r2.rs`).

**Differences from the text above, found while building:** an on-chain floor gate on `FloorModule.advance` was built first and
dropped (the rules above: a frog's advance is one inscription's price; 30 forge tests and the module's invariant suites failed); the fakes'
module floor (`tests/ethnft.rs`, `tests/audit_nft.rs`) is now 0.47 ETH before the tests post, one their fixed item cap (the Relay
fixture's 0.5 ETH) covers under the advance gate; the sale scan's chunks a pass went from 6 to 13 so the first pass covers about
3.6 days (the fixtures' real sales included); round 1's `c_l_2_…`, `b_m_1_…` and `b_h_1_…` regressions now put their sales at final
blocks and expect a post (the canister reads the chain, the self-trades are no move, the real sales count).

**Results:**

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,101 passed**, 0 failed (125 suites; +`Pooled.t.sol` `test_A2_M1_…`; the floor-rise test rewritten) |
| The Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **592 passed**, 0 failed (was 574: +16 `tests/audit_nft_r2.rs`, +2 unit tests in `ethnft.rs`); `tests/ethnft.rs` 26, `tests/audit_nft.rs` 21 |
| The `.did` | parses (`candid_parser` 0.1.4, 105 types) and decodes `tests/fixtures/eth-state-round2.hex` as its `EthState` |
| Timelock `cargo test` (and `--features rehearsal`) | 15 passed (15); unchanged |
| ops | **55 passed** (51 + 4 `audit_nft_d_r2`); `tsc --noEmit` clean |
| monitor | **169 passed** (164 + 5 `audit_nft_d_r2`) |
| keeper | 150 passed |
| indexer | 135 passed |
| web-rook | 72 passed; `tsc --noEmit` clean |
| typechecks (ops, keeper, indexer, web-rook, staging, tools/snapshot) | clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (536 golive checks ok on the five-collection NFT stack; with `--canister --launch --monitor-config` every collection's targets, conversion, reserve and purchase limit are the launch file's and the monitor watches all five with both references; refused: BAYC's 9 ETH shifted onto Pudgy, BAYC's purchase limit at 2 ETH, a monitor configuration without the no-good-post pause, one without a CoinGecko id, `--phase1` without `--canister-evm`, `--phase1` without the launch file and the monitor configuration; round 1's refusals unchanged) |

**Reproducible build** (`Dockerfile.build`, linux/amd64, on a `git archive` export of `4ebf761`; `autobuyer/` is unchanged since
`194e5f1`), run twice, both with `docker build --no-cache`:

```
8efa4bdd43b4544496828cf693383b6bd8ce0c38ed6084caf38ed4181c6e6938  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

`buyer.wasm` is new (6,603,914 bytes; 1,253,129 gzipped with `gzip -9 -n`, under the 2,000,000 proposal limit; +145 kB over round 1's, the `candid` `value` feature included); it replaces round 1's `915a8780…1379` for any install of this branch. The timelock is unchanged.
Nothing was installed. The export directory and the two build images were removed afterwards.

**The delta round 3 must cover:** `git diff 79455e1..HEAD` (code files):
`autobuyer/canister-rs/{Cargo.toml, etch_autobuyer.did, src/{canister,ethflow,ethnet,ethnft}.rs, tests/{audit_nft,audit_nft_r2,ethnft}.rs,
tests/fixtures/eth-state-round2.hex}`; `monitor/src/{candid,config,ethnft}.mjs`, `monitor/test/audit_nft_d_r2.test.mjs`;
`ops/src/{ethnft,ic,runner}.ts`, `ops/test/{audit_nft_d,audit_nft_d_r2,ethnft}.test.ts`; `web-rook/src/rook/phase1Trust.ts`,
`web-rook/test/phase1.test.ts`; `test/ethnft/Pooled.t.sol`; `script/{DeployStack.s.sol,golive-check.sh}`, `scripts/golive-dryrun.sh`;
`deployments/launch/{nft-pilot.json,monitor-nft.json}`. `src/` (the contracts) is unchanged.
