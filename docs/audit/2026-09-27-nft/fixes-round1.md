# Fix round 1: the 2026-09-27 review of the Ethereum NFT side

Written 2026-09-27 on branch `nft-side` (at `8c107e9`: the reviewer's D5 and launch configuration, the audit `09a680a`, and the
`nft-pool` work fast-forwarded on top). It answers `README.md`, `A-contracts.md`, `B-canister-judges.md`,
`C-canister-flow-deposits.md` and `D-services-deploy.md` finding by finding: the fix, where it is, and the test that is the
reviewer's PoC turned around (it fails on the code before this round and passes now). §9 ("As built") has the commits, the test
counts and the reproducible-build hashes. Nothing was deployed, pushed or signed with a real key; the only chains written to were
local anvils. The network use was read-only: an anvil fork of Ethereum for the Seaport fork proof, and the dry run's fork of 4663.

**The rules every fix keeps** (as `2026-09-26/fixes-D-canister.md` §1):

- Nothing is weakened to buy availability: H1 (pause), H2 (custody caps), H3 (the outflow limiter), "save, then send", and every
  judge (`relayeth`, `seaport`, `ethnft`) are unchanged in what they refuse.
- Stable state stays decodable: every new field is `opt` (`Option` with `#[serde(default)]`), every new enum case is appended. The
  C-L-4 probe (below) now REFUSES an upgrade whose saved optional element does not fit, instead of restoring it as nothing.
- Every outcall a stranger can cause stays bounded per tick; the evidence pass is cut (B-M-7).
- **Not changed** (operator-pending or operator-decided): BAYC's graduation target stays 9 ETH (J1); the stage-0 custody caps are
  unchanged (J7); D1–D9 and D5 as decided. D8's decision (the guardian pauses at > 20 % from both references) stands; what changed
  is how it is built (per collection, on the canister's own reading, two agreeing references, latched), as the review asked.

## 0. Summary

| ID | Sev | Status | Fix | Where | Test (the PoC, turned around) |
| --- | --- | --- | --- | --- | --- |
| **B-H-1** | High | **Fixed** | Sales only veto: the post is the rank-2 verified listing, always; `L` over `S × 1.1` or under `S25 × 0.85` is NO post. A sale counts only with the collection's own `Transfer` in the receipt, never a self-trade, one per token, seller and buyer, within ±30 % of `L` | `ethnft.rs` `sales_in`, `counted_sales`, `floor_from` | `audit_nft.rs` `b_h_1_…`; `ethnft::tests::b_h_1_…` |
| **D-H-1** | High | **Fixed** | The runner hands in SALES (Seaport 1.5/1.6 `OrderFulfilled` or Blur v2 `Execution721*` of the collection, with a Transfer from one address to another), newest first, over the canister's 7-day window in 2,400-block chunks; each verdict remembered | `ops/src/ethnft.ts` `recentSaleTxs`, `isSale` | `ops/test/audit_nft_d.test.ts` "D-H-1" |
| **D-H-2** | High | **Fixed** | The canister clamps its post into the module's `floorBounds()` (read at the 4663 pin) before sending; the monitor's clamp/runaway rule for NFT modules names the canister and pauses that collection's `PosterSource`, never the registry (the W2 "Left open 2" premise holds again) | `ethflow.rs` `post_reading`; `monitor/src/evm.mjs` `judgeStackLogs`, `floorWalk` | `audit_nft.rs` `d_h_2_…`; `monitor/test/audit_nft_d.test.mjs` "D-H-2" |
| **B-H-2 / C-H-1** | High | **Fixed** | `home_act`'s steps are independent (every wait collected, none returns before the floor); an urgent floor post (module 40 min old) goes first; a buy that cannot take its value or room is dropped before its advance (it never holds the Relay leg); no deposit of a collection mints while its buy holds the mint room (advance → own mint) | `ethflow.rs` `home_act`, `due_readings`, `EthState::buy_holds_mint`, `buy_pass` | `audit_nft.rs` `b_h_2_c_h_1_…`, `c_h_1_a_l_1_…` |
| **A-M-1** | Med | **Fixed (canister side)** | A deposit is minted only while the module's accepted floor is ≤ 5 % over the median of the sales the canister verified (fresh reading); otherwise it WAITS (never refused, never returned). No deposit fee (D5), no module change | `ethflow.rs` `deposit_floor_ok`; `ethnft.rs` `DEPOSIT_FLOOR_BAND_BPS` | `audit_nft.rs` `a_m_1_…` |
| **A-M-2** | Med | **Fixed** | `EthNftWrapper.redeemToRelist()` reverts `NotSupported` | `src/nft/EthNftWrapper.sol` | `test/ethnft/EthNftWrapper.t.sol` `test_A_M2_…` |
| **B-M-1** | Med | **Fixed** | A receipt that cannot be read is not a sale and never voids the pass (counted and said) | `ethflow.rs` `reading_of` | `audit_nft.rs` `b_m_1_…` |
| **B-M-2 / C-M-1** | Med | **Fixed** | The simulation tells a revert from a failure: "refused" only when EVERY provider answers a JSON-RPC error saying it reverted (normalized); anything else is "unread", a wait. A delivery is marked (a stray held) only on agreed reverts standing `SIM_REFUSAL_SECS` (1 h) over several ticks | `ethnet.rs` `fees`; `pin.rs` `json_rpc_norm_body`, `said_reverted`; `ethflow.rs` `eth_act`, `return_act` | `audit_nft.rs` `b_m_2_c_m_1_…` |
| **B-M-3 / C-M-3** | Med | **Fixed** | `superseded` clears the line only once the outcome is read; a failed read keeps it (nothing else is signed meanwhile). A nonce taken by a transaction that is not ours HALTS the line (`EthState.halted`, SEV1) until the operator's `eth_line_clear` | `ethflow.rs` `superseded`, `line_clear`; `ethnet.rs` `eth_line_clear` | `audit_nft.rs` `b_m_3_c_m_3_…`, `b_m_3_a_nonce_taken_…` |
| **B-M-4** | Med | **Fixed** | `EthState.last_mined = (nonce, block)`: nothing is signed below `nonce + 1`, whatever the pin says, and nothing while the pin is below `block`; the stale `eth:{id}` reservation sweep (L-2) | `ethflow.rs` `sign_eth`, `eth_act`, `note_mined`, `buy_pass` | `audit_nft.rs` `b_m_4_…` |
| **B-M-5 / C-L-3** | Med | **Fixed** (nft-pool + this round) | nft-pool: one buy per collection, `continue` past a refused or unreadable collection. This round: the H2 check at the item's value before a buy starts, a NeedAdvance buy that cannot reserve is dropped (frees the Relay leg), a Relay refund on Ethereum under `min_out` becomes an arrival (B-L-7) | `ethflow.rs` `buy_pass`, `home_act`, `step_buy` | `audit_nft.rs` `b_m_5_c_l_3_…`, `b_l_7_…` |
| **B-M-6** | Med | **Fixed** (transform); provider pair recommended | Every Ethereum JSON-RPC state read goes through `json_rpc_norm`: `{"result": …}` with keys sorted and strings lower-cased, a JSON-RPC error as `{"error":{"code","reverted"}}`, every unusable body the same error | `pin.rs`, `canister.rs` `json_rpc_norm`, `nftnet.rs` `agreed` | `pin::tests::b_m_6_…` |
| **B-M-7** | Med | **Fixed** | Final receipts cached by hash (never read twice); receipts reserved 32 kB first (200 kB only on the IC's size reject); listings verified only until 5 distinct offerers; small reads at 8 kB, and `getApproved` skipped when approved-for-all; an 8 T cycles floor for the pass | `ethflow.rs` `reading_of`; `ethnet.rs` `eth_sale_receipt`, `eth_listing`, `call_small`, `cycles_ok` | `audit_nft.rs` `b_m_7_…` |
| **B-L-3 / C-M-2** | Med | **Fixed** (nft-pool re-pricing + this round) | A fill unmined `FILL_CANCEL_SECS` (6 h) after it was first signed is cancelled at its nonce (`EthKind::CancelFill`; if the fill mines it is the buy); the "never sent → mark" check runs every tick whatever holds the line (`mark_pass`) | `ethflow.rs` `resolve_eth`, `cancel`, `superseded`, `mark_pass` | `audit_nft.rs` `c_m_2_…` |
| **D-M-1** | Med | **Fixed** | Per collection (`PosterSource.pause()`: the registry's owner or guardian; `unpause()`: the owner); the monitor pauses on the canister's own FRESH reading (not the lagging accepted floor), > 20 % from BOTH references on one side, the references read and agreeing within 10 %; latched until `eth_nft.floor_pause_ack.<module>`; stable keys; a missing reference pages after 30 min, both at once; references read at most every 5 min (`MONITOR_COINGECKO_KEY` optional) | `src/floor/PosterSource.sol`; `monitor/src/ethnft.mjs` `floorAlerts`, `checkEthNft`; `guardian.mjs` `pausePoster`; `config.mjs` | `test/ethnft/EthNftFlow.t.sol` `test_D_M_1_…`; `monitor/test/audit_nft_d.test.mjs` "D-M-1" ×3 |
| **D-M-2** | Med | **Fixed** | The canary reads EthNftWrapper's `Wrapped` / `Deposited` / `Delivered` and judges them, and its `Undeliverable`, against the canister's Ethereum records (`eth_nft_state`: buys, deposits, deliveries), never the Bitcoin redemptions | `monitor/src/evm.mjs` `judgeLogs`, `records.mjs`, `stack.mjs` | `monitor/test/audit_nft_d.test.mjs` "D-M-2" ×2 |
| **D-M-3** | Med | **Fixed** | The Ethereum outbound canary: the custodian's nonce (both providers) against the canister's `last_mined` and line; `ownerOf` of every token bought-not-minted, deposited or strayed; the balance against arrived buys' ETH; a halted line pages SEV1 | `monitor/src/ethnft.mjs` `checkEthNft` | `monitor/test/audit_nft_d.test.mjs` "D-M-3" |
| **D-M-4** | Med | **Fixed** | The runner remembers each listing it handed in per buy (2 h) and hands in the next cheapest; it pre-checks the bound (what arrived less the reserve, our reading plus the premium) | `ops/src/ethnft.ts` `ethNftOnce` | `ops/test/audit_nft_d.test.ts` "D-M-4" |

**Carried over from the 2026-09-26 fixes (the review's "not carried to the Ethereum side"):**

| Earlier fix | Now on the Ethereum side | Test |
| --- | --- | --- |
| H9 / fixes-D H-1 (ticket facts final) = B-L-2 / C-L-6 | A delivery is signed only when its ticket resolved strictly before the home chain's finalized block's time (`deliverBy − 7 d` < the lower providers' finalized timestamp, rounded down): a 4663 reorg can no longer un-redeem a ticket under a signed delivery | `audit_nft.rs` `b_l_2_c_l_6_…` |
| R3 M-1(c) (the paused pass) = B-L-4 / C-L-1 | While paused, the Ethereum line is read, a Moved delivery that is final is recorded, a delivery in its last day is cancelled (a self-send); nothing new is signed. A delivery's saved bytes are re-sent only while its ticket is Resolved and outside its last day | `audit_nft.rs` `c_l_1_…` |
| L-2 (stale reservations) | `eth:{id}` reservations of a buy gone, finished or stuck with nothing on the way are settled every tick | (in `buy_pass`; exercised by the flow tests) |
| L-5 (a signed buy never abandoned) | The fill cancel (C-M-2) | `audit_nft.rs` `c_m_2_…` |
| fixes-W2-monitor "Left open 2" (the runaway rule's premise) = D-H-2 | The canister posts inside the bounds; the rule pauses the collection's poster | `monitor/test/audit_nft_d.test.mjs` "D-H-2" |
| M-2's residual = C-L-4 | `post_upgrade` decodes the saved tuple a second time with every optional element as `opt reserved`: an element saved present that restores as nothing traps the upgrade | `audit_nft.rs` `c_l_4_…` |

## 1. The floor (B-H-1, D-H-1, D-H-2, B-M-1, D-M-1)

### 1.1 B-H-1: nobody steers the post

The reviewer's first recommendation, as built (`ethnft::floor_from`):

- **The post is `L`, the `listing_rank`-th cheapest verified listing (one per offerer), always.** It never leaves the range of
  listings anyone can fill (B-L-1: open orders, or restricted ones through an allow-listed zone; `ListingTerms::fillable`).
- **Sales only veto.** `L > S × (1 + up_cap)` (cheap listings withheld) or `L < S25 × (1 − down_cap)` (listings the market has
  not traded at) is no post. The band's edge is never posted.
- **What counts as a sale** (`sales_in`, `counted_sales`): a Seaport 1.5/1.6 `OrderFulfilled` or Blur v2 execution of one token,
  AND the collection's own `Transfer` of that token in the same receipt (the seller is its first `from`, the buyer its last `to`);
  never a self-trade; one sale per token, per seller and per buyer (newest first); within ±30 % of `L` (`SALE_BAND_BPS`). A far
  sale is no vote, so it can only take the count under `min_sales` (no post). A wash ring needs as many wallets as the real
  sellers it outvotes, and then it can only stop a post.
- Confidence 10,000 / 7,000 / 5,000 by `|L − S|`; `depth` counts distinct offerers (B-I-5).

**Judgment call (for the operator):** a real crash deeper than 15 % under the recent sales' lower quarter is now "no post" until
the market trades there (it was a post at the band's edge, above every listing). The module goes stale meanwhile (mints and
fresh burns wait; the D8 pause or the stale escape end it). The alternative the reviewer offered (post `L` at low confidence) makes
the module just as stale, so it adds nothing.

### 1.2 D-H-1: the honest runner hands in sales

`ops/src/ethnft.ts` `recentSaleTxs`: the collection's Transfer logs read backwards in 2,400-block chunks over 50,400 blocks (the
canister's `SALE_WINDOW_BLOCKS`), each transaction's receipt read once (`saleVerdicts`) and kept only if `isSale` (the canister's
rule, as a pre-filter), newest first, at most 12, at most 60 new receipts a pass. The canister judges every one again.

### 1.3 D-H-2: clamped posts, and the monitor's rule for the canister

- `ethflow::post_reading` reads `floorBounds()` (selector `0xd64dcf23`) at the 4663 pin and posts `clamp(L, lo, hi)`. The bounds
  only widen between the read and the transaction (they grow with time), so the module accepts what was posted: `target ==
  accepted`, as with the keeper's plan (fixes-W2-services §3).
- `monitor/src/evm.mjs`: for a module whose collection has a `posterSource` (`stack.mjs` now carries `kind`, `posterSource` and the
  NFT collection), a clamped post is worded as the canister's key used by something else, and the runaway, the walked floor and
  the no-good-post triggers are `pause-poster` on that collection's PosterSource (`guardian.pausePoster`), never `pauseKeeper()`.

### 1.4 B-M-1

A sale receipt that errors (over the cap, a lagging provider, a disagreement) is counted as unread and not a sale; the pass goes
on (`reading_of`), and the reading's line says how many were unread.

### 1.5 D-M-1: the D8 pause, per collection

- **Contract:** `PosterSource.pause()` (the registry's owner or guardian, at once), `unpause()` (the owner only, stamped on both
  clocks); `keeper()` is zero and `keeperPaused()` true while either switch is set; `keeperResumedAt(Block)` is the later of the
  two, so the module's RESUME_GRACE runs from the last resume. Constructor and immutables unchanged: DeployStack is unchanged.
- **Monitor** (`floorAlerts`, `checkEthNft`): pause only on the canister's own reading (fresh ≤ 1 h), > 20 % from both references
  on the same side, both references read and within 10 % of each other; otherwise SEV1 "not paused, look" or nothing. A pause is
  latched per module (`next.latch`) until `eth_nft.floor_pause_ack.<module>` ≥ the latch time. Keys are stable (no poll time).
  A reference missing for 30 min is SEV2; both missing is SEV2 "unwatched" at once. References are read at most every 5 minutes
  per collection (CoinGecko's keyless 429s); `MONITOR_COINGECKO_KEY` (env, optional) is sent as `x-cg-demo-api-key`. With D8 armed
  the config must name both references per collection. `eth_nft.guardian_pause = false` now stops the pause (D-I-2).
- **Not built:** checking each OpenSea slug against the collection's contract (a slug mix-up is still possible; the two-reference
  agreement makes a single wrong slug unable to pause).

## 2. Cross-collection stalls (B-H-2 / C-H-1, B-M-5 / C-L-3, B-L-3 / C-M-2, D-M-4)

**What `nft-pool` had already fixed:** one buy per collection with one Relay leg canister-wide (an `Arrived` buy no longer blocks
another collection), `buy_pass` continuing past an unreadable or capped collection, `home_act`'s buy steps collected instead of
returned, per-arrived-buy listing errors, and re-pricing of every line transaction every 30 minutes up to the chain's cap.

**This round:**

- `home_act` (B-H-2 / C-H-1): records, marks, deposits' mints and every floor post collect their waits too; a due post whose module
  was last posted ≥ 40 min ago goes first (B-L-9); `still_resolved` and the keeper read no longer return early.
- The mint room (C-H-1 / A-L-1): no deposit of a collection mints while a buy of that collection is between its advance and its own
  mint (`buy_holds_mint`), so deposits cannot use up the H3 and H11 room a purchase needs. **Judgment call:** a buy that waits long
  (D4's `Arrived`) delays that collection's deposit mints as long; the monitor pages a deposit waiting 26 h.
- B-M-5's residual: the one Relay leg could be held by a `NeedAdvance` buy that H3's value limit or H2's value cap refuses. The cap
  is now checked at the item's value before a buy starts, and a `NeedAdvance` buy that cannot reserve is dropped (nothing was paid)
  so the round robin moves on.
- B-L-7: a watched `Stuck` buy whose Ethereum balance rose by at least half the deposit (Relay refunding on Ethereum, under
  `min_out`) becomes `Arrived` with that ETH: it no longer holds the Relay leg for every collection.
- C-M-2: `mark_pass` marks never-signed deliveries (relist, empty destination, inside the last day) every tick, whatever holds the
  line; a fill unmined 6 h after its first signature is cancelled (0-value self-send, fees bumped over its newest version, every
  version its rival; if the fill mines, it is the buy; if the cancel mines, the buy is `Arrived` again less the cancel's gas).
- C-L-2 / B-L-8: in `eth_act` each delivery's wait (its ticket read, `ownerOf`, H3's per-collection limit, the simulation) is its own;
  deposit mints take the first that may mint.
- D-M-4: the runner (§1.2's module) remembers the listings it handed in per buy for 2 h and pre-checks the bound.

## 3. Failed reads are never outcomes (B-M-2 / C-M-1, B-M-3 / C-M-3, B-M-4, B-M-6)

- **B-M-6:** `json_rpc_norm` (§0). Measured bodies from the review are unit tests: two field orders and case agree; a rate-limit page,
  HTML, "discontinued" and `{"result":{"code":503…}}` are all the one unusable error; a revert is `reverted: true` whatever the
  provider's words. Used by `nftnet::agreed` (every `eth_call`, `eth_getBalance`, `eth_getTransactionCount`, `eth_chainId`,
  `eth_getCode`) and `eth_estimateGas`. **Recommended provider pair** for `evm_chains[1]` (the operator's install argument):
  `eth.drpc.org` and `xrpc.cl/eth` (identical 13/13 in B's measurement), not `1rpc.io/eth`. **Not built:** a go-live check that
  fires 13 back-to-back reads per method.
- **B-M-2 / C-M-1:** `EthNet::fees` asks every provider through `json_rpc_norm`; only all-revert is `refused (simulation)`. The flow
  marks a delivery (holds a stray) only after an agreed revert has stood `SIM_REFUSAL_SECS` = 1 h (`refused_since`, cleared by any
  simulation that passes). The deterministic marks (a relist ticket, inside the last day, a final revert, a final cancel) are
  unchanged.
- **B-M-3 / C-M-3:** `superseded` for a `Cancel` reads the delivery's versions, for a `CancelFill` the fill's; only an outcome read
  clears the line. For a `Deliver`, `Buy` or `Return` whose nonce moved with none of its versions mined, the line is kept and
  `EthState.halted` stops every Ethereum signature (SEV1 in the monitor) until the operator's `eth_line_clear()` (only while halted;
  the record goes back to before its signature, and nothing is signed below the taken nonce again). The review's "watchdog" is not
  needed: no path leaves a `Sent`/`Cancelling` delivery off the line any more.
- **B-M-4:** `last_mined` and the pin check (§0). The monitor's outbound canary (D-M-3) uses the same numbers.

## 4. The rest (A-M-1, A-M-2, D-M-2, D-M-3, B-M-7)

- **A-M-2:** `redeemToRelist()` is `pure` and reverts `NotSupported`; the `RedeemedToRelist` event is gone from `EthNftWrapper`
  (the Bitcoin `Wrapper`, `NftWrapper` and `HomeWrapper` are unchanged).
- **A-M-1 — the conservative choice, and why.** The review offered a deposit fee (excluded: D5 decided no deposit fee), a module
  rule (cap fresh burns by the trailing 24 h low of `accepted`: a `FloorModule` change reaching the Bitcoin frogs' module too, after
  four review rounds of its burn pricing), a wrapper lock on deposit-minted units (does not cover pool-bought units), or minting
  deposits only below the posted floor. Built: **a deposit is minted only while the module's accepted floor is at most 5 %
  (`DEPOSIT_FLOOR_BAND_BPS`) over the median of the sales the canister itself verified at its last reading (fresh ≤ 1 h)**; otherwise
  the deposit waits. What a depositor can then take by deposit-then-burn is at most about 5 % of the sales' median less the burn
  spread (not the review's +10 % to +19 %), per deposit, within the 3-a-day limits, and only when the market's own sales are that
  close. The residual: a malicious runner can still choose which real sales to hand in (it could hand in the week's highest); the
  canister choosing its own sample (eth_getLogs in the canister) is not built. The honest gap (a rank-1 listing under the rank-2
  floor, A-M-1d) is within the band: at an honest floor the module buys the NFT through a deposit+burn for less than its own Relay
  path costs (`gradPrice` + fees), so this is not treated as a loss. Deposits still mint without any fee; they wait only while the
  floor is above the market's sales.
- **D-M-2, D-M-3:** §0.
- **B-M-7 (the numbers):** at the reservation pricing the review used, a pass was ≈ 72 B cycles per collection (96 reads at 16 kB,
  24 receipts at 200 kB). Now a steady pass (5 listings verified, 5 small reads each at 8 kB on 2 providers, and only receipts not
  seen before, at 32 kB) is ≈ 4.5 B for the listings plus ≈ 0.4 B per NEW receipt: ≈ 5–6 B per pass, ≈ 0.4 T a day per collection
  at the runner's 20-minute cadence, ≈ 1.2 T for three (was ≈ 15.6 T). Below 8 T of cycles the pass is skipped (the rest of the
  tick is not).

## 5. The Lows (21 distinct at `57160b0`)

| ID | Status | What |
| --- | --- | --- |
| A-L-1 | Fixed (canister) | `buy_holds_mint` (§2) |
| A-L-2 | **Not fixed** | The deploy EOA keeps the NFT wrapper's mint-limit powers. Moving it behind a Safe-owned passthrough (as `WrapClaim`) is a deploy change with its own review; golive-check does not read `deployer()` yet. Left for round 2 |
| A-L-3 | Fixed | The per-collection `PosterSource` switch (§1.5) |
| B-L-1 | Fixed | `ListingTerms::fillable` (§1.1) |
| B-L-2 / C-L-6 | Fixed | H9 (carried over, §0) |
| B-L-4 / C-L-1 | Fixed | The paused pass (carried over, §0) |
| B-L-5 | Partly | `post_upgrade` refuses dropping or moving the chain with a line, a pending home transaction or an open buy. Not built: `validate` requiring an explicit `custody_caps` to list every eth collection |
| B-L-6 | Not fixed | The shared 4663 balance between a frog-side and an eth-side Relay leg: not reachable in the NFT-only stack (no frog side); fix when both sides run |
| B-L-7 | Fixed | §2 |
| B-L-8 / C-L-2 | Fixed | §2 |
| B-L-9 | Fixed | The urgent post (§2) |
| C-L-4 | Fixed | The decode probe (carried over, §0). Found on the way: `nft-pool`'s `EthTx` fields (`to`, `data`, `gas`, `earlier`, `rivals`) are `#[serde(default)]` but not `opt`, and Candid does NOT fill them: an `EthTx` saved by an `nft-side` build does not decode, and inside the `opt` element it restores as nothing. Nothing is installed from `nft-side`, so this is latent; with the probe such an upgrade now traps. §11/§7 of the design docs said older state loads; corrected |
| C-L-5 | Partly | `eth_rearm_stray(tx, token)` (operator) puts a `Held` stray back on its return path, where every check runs again. Not built: re-opening a claim over the outcall caps |
| D-L-1 | Fixed | Every runner call has a 15 s deadline, a 4 MB cap and no redirect (`guarded`); one thrown `eth_deposit` never aborts the others; each step of a cycle is caught |
| D-L-2 | Fixed | The runner refuses to start when the canister serves collections and `ETH_LOGS_RPC`, `OPENSEA_API_KEY` or a slug is missing (`ethStepProblems`); `/healthz` is 503 when a served collection has had no evidence accepted for 40 min (`staleEvidence`) |
| D-L-3 | Partly | The monitor remembers an unjudged inbound transfer past its 7,200-block window and pages it until the canister judges it. The runner's discovery window is unchanged (the operator hands the paged one in with `eth_deposit`) |
| D-L-4 | Fixed | An `ownerOf` error is "unread", never an owner: one stable SEV2 after 3 polls; two alike-failing providers are no custody loss |
| D-L-5 | Fixed | Each collection in its own `try`; its failure pages and the others (and their pauses) run |
| D-L-6 | Partly | golive-check holds `max_premium_bps` = 500 and `chain_id` = 1, and `--canister-evm <evm_address()>` ties the stack's custodian to the canister's key (a WARN under `--phase1` without it). Not built: the canister reading `EthNftWrapper.custodian()` itself |
| D-L-7 | Fixed | `StackChecks.checkNftSlugs`: non-empty, `[a-z0-9-]+`, each once |
| D-L-8 | Fixed | The Trust page: sales only veto, the runner's real reach, the premium the install holds, the per-collection guardian pause, the held cases and the exact 48-byte layout |

**Lows: 21 distinct — 15 fixed, 4 partly (B-L-5, C-L-5, D-L-3, D-L-6), 2 not fixed (A-L-2, B-L-6).**

## 6. The Infos

Fixed on the way: B-I-1 (`sale_nft_listing` refuses the `eth_nft` chain), B-I-5 (depth by distinct offerers), A-I-3 (a claim naming a
wrapper or Floor Module of ours is a stray, returned), C-I-5 (the pause is read again before every Ethereum and home-chain
signature; while paused only a record goes out), C-I-9 (§2.1 now says one purchase per collection; §7's "disagreement is nothing
believed" is true again), D-I-2 (the gas-pot page no longer needs an item held; `guardian_pause = false` stops the pause), D-I-3
(the reconciler's 50 % rule leaves the transit unit out), D-I-4 (the OpenSea key is scrubbed from every log line; no redirect with
the key). Unchanged: A-I-1, A-I-2, A-I-4, A-I-5, A-I-6 (a runner outage still stales the module; with D8's per-collection pause and
`pause_on_no_good_post_h` it is now that collection's poster that pauses), B-I-2, B-I-3, B-I-4, B-I-6, C-I-1, C-I-2, C-I-3 (R3's
per-sender return budget and return-anomaly tripwire: still not built), C-I-4, C-I-6, C-I-7, C-I-8, D-I-1, D-I-5.

## 7. New judgment calls for the operator

1. **No post in a deep crash** (§1.1): a listing floor more than 15 % under the recent sales' lower quarter is not posted until the
   market trades there; the module stays stale meanwhile (D8 / the stale escape end it).
2. **Deposits wait while the floor is more than 5 % over the verified sales** (A-M-1, §4): no fee, no refusal; the band is
   `DEPOSIT_FLOOR_BAND_BPS` = 500 (a code constant; changing it is an upgrade).
3. **Deposits of a collection wait while its buy holds the mint room** (§2).
4. **A fill is cancelled after 6 h unmined** (`FILL_CANCEL_SECS`), after re-pricing every 30 min (J6).
5. **A nonce taken by something else halts the Ethereum line** until `eth_line_clear` (§3); the monitor pages SEV1.
6. **D8 as built** (§1.5): per collection, on the canister's reading, two agreeing references, latched until
   `eth_nft.floor_pause_ack` — the monitor config needs `opensea_slug` and `coingecko_id` per collection.
7. **Deliveries wait for 4663 finality** of their ticket's resolution (≈ 15–20 min more per redemption).
8. **The Ethereum provider pair**: `eth.drpc.org` + `xrpc.cl/eth` recommended over `1rpc.io/eth` (B-M-6).

## 8. Left open (for round 2)

A-L-2; B-L-5's cap coverage; B-L-6; C-L-5's re-open of an unreadable claim; D-L-3's runner cursor; D-L-6's in-canister
`custodian()` read; B-M-6's 13-read go-live check; the canister choosing its own sale sample (B-H-1's "better", which would also
close A-M-1's runner residual); every Info listed as unchanged in §6.

## 9. As built (2026-09-27)

**Commits** (branch `nft-side`, on `8c107e9`):

| Commit | What |
| --- | --- |
| `527799f` | Contracts: `PosterSource`'s own switch, `redeemToRelist` refused, `checkNftSlugs`; forge regressions |
| `ed76e9d` | Canister: the floor, the reads, the flow, the carried-over fixes; `tests/audit_nft.rs` (B's and C's PoCs as regressions) |
| `dce6cb8` | Services: runner, monitor, reconciler, Trust page; D's PoCs as regressions (`ops/test/audit_nft_d.test.ts`, `monitor/test/audit_nft_d.test.mjs`) |
| `323f2b2` | Go-live: golive-check and the dry run |
| `29c3287` | Canister: the gas estimate through the normalizing transform too (B-M-6) |
| (this document's commit) | This document; `docs/v7/phase1-nft.md` (§3.2, §4.2, §6, §7, §9 D8, §11.7) and the `phase1-nft-pooled.md` §7 correction |

**Differences from the text above, found while building:** none of substance. The C-L-4 probe exposed `nft-pool`'s non-`opt`
`EthTx` fields (§5, C-L-4 row). The dry run's NFT phase now also shows the premium and custodian refusals and one collection's
poster paused while the next collection posts.

**Results:**

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,100 passed**, 0 failed (125 suites; `nft-pool` had 1,096: +`test_A_M2_…`, `test_D_M_1_…`, `test_A_L3_…`, `test_D_L_7_…`) |
| The Seaport fork proof (anvil fork of Ethereum at 26,055,690, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **574 passed**, 0 failed (was 550: +21 `tests/audit_nft.rs`, +3 unit tests); `tests/ethnft.rs` 26, updated where the rules changed (an agreed revert must stand an hour; a deposit mints against a fresh reading) |
| Timelock | 15 passed (and 15 with `--features rehearsal`); unchanged |
| ops | **51 passed** (45 + 6 `audit_nft_d`); `tsc --noEmit` clean |
| monitor | **164 passed** (154 + 10 `audit_nft_d`) |
| keeper | 150 passed |
| indexer | 135 passed |
| web-rook | 72 passed; `tsc --noEmit` clean |
| typechecks (ops, keeper, indexer, web-rook, staging, tools/snapshot) | clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (536 golive checks ok on the five-collection NFT stack; the new refusals: a 10 % premium, a custodian that is not the canister's key, a paused poster; one poster paused while the next collection posts; the registry's pause still zeroes every poster) |

**Reproducible build** (`Dockerfile.build`, linux/amd64, on a `git archive` export of `29c3287`), run twice, the second with
`docker build --no-cache`, **identical both times**:

```
915a8780eda64455532921da53def5048ec4ceb344eda17a350f6f2de3c11379  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

`buyer.wasm` is new (6,458,868 bytes; 1,225,992 gzipped with `gzip -9 -n`, under the 2,000,000 proposal limit); it replaces
`nft-pool`'s `acaba6a2…a5e` for any install of this branch. The timelock is unchanged. Nothing was installed. The build's
export directory and its Docker image were removed afterwards.

**The delta round 2 must cover** (with the `nft-pool` delta, `09a680a..8c107e9`): `git diff 8c107e9..HEAD`, 39 code files —
`src/floor/PosterSource.sol`, `src/nft/EthNftWrapper.sol`, `script/{StackChecks.sol,DeployStack.s.sol,golive-check.sh}`,
`scripts/golive-dryrun.sh`, `test/{StackChecks.t.sol,ethnft/EthNftFlow.t.sol,ethnft/EthNftWrapper.t.sol,ethnft/Pooled.t.sol}`;
`autobuyer/canister-rs/{etch_autobuyer.did, src/{canister,ethflow,ethnet,ethnft,nftnet,pin,relayconv,salenet}.rs,
tests/{audit_nft,ethnft}.rs}`; `monitor/src/{candid,config,ethnft,evm,guardian,records,stack}.mjs`,
`monitor/test/{audit_nft_d.test.mjs,ethnft.test.mjs}`; `ops/src/{backing,ethnft,log,runner}.ts`,
`ops/test/{audit_nft_d,ethnft,guards}.test.ts`; `web-rook/src/rook/phase1Trust.ts`, `web-rook/test/phase1.test.ts`.
