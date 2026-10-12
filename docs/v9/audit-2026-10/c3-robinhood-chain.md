# C3: the Robinhood Chain (4663) NFT path

| | |
|---|---|
| Scope | `autobuyer/canister-rs/src/ethnet.rs` (`SIDES`, `SIDE_FLOORS`, `with_side` / `with_buy` / `side_of_buy`, `EthNet::new_on`, `relay_busy_except` / `any_relay_busy`, `other_side_step`, `tick`, every endpoint, `eth_nft_state_of`, `eth_line_clear_on`, `sides_blob` / `sides_of` / `restore_sides`); `ethnft.rs` (`EthCollection.chain_id` / `token_bound`, `chain_of`, `chain_of_addr`, `chains`, `on`, `validate`, `validate_home`, the floor and the buy bound as a side uses them); `chains.rs`; `relayeth.rs` (`relay_name`, `to_solana`, `ROBINHOOD_TO_SOLANA`, `judge_with`); `relaysol.rs` (S1–S12 for a 4663 destination); `relayquote.rs`; `ethflow.rs` / `ethflow/home_books.rs` as a side runs them (`owns_step`, `side_buy_base`, `new_side`, `stranded_by`, `home_sol::{send, resolve, fund}`, the way back, the custody keys); `salenet.rs` and `nftnet.rs` (the one-writer guards); `frognet.rs` (the Relay leg); `canister.rs` (`post_upgrade`'s side checks, `caps_of`); ops: `booksreserves.ts` (`rpcsOf`, `locator`), `ethnft.ts` (`openseaChainOf`, `chainOfCollection`, `sideStates`, `preJudgeEth`), `relaysol.ts` (`solLegOf`, `preJudgeSolAsk`), `ethRecords.ts`, `runner.ts` (`sideStatesOf`, `OPENSEA_CHAIN_4663`), `reconcile.ts`; the monitor's reads (`monitor/src/ic.mjs`); `deployments/launch/robinhood-collections.json`; `scripts/measure-robinhood-nfts.mjs` |
| Commit | `79b4e60` on `golive-2026-10-08` (2026-10-12) |
| Reference | `docs/v9/robinhood-everything.md` §3 ("Built"), §3.3; `docs/v9/HANDOFF-2026-10-10.md` §8 ("Robinhood Chain NFTs", "Robinhood Chain measured"); `docs/v4/eth-nft-canister.md` §2–3 (4663's 0.104 s blocks); the Round B reports for format |
| Method | The project's (README): threat model, then a line-by-line read of every file in scope against it and the checklist of the brief (cross-chain confusion, the Relay legs per destination, a side's outcomes, nonce and gas pot per chain, buy ids, the upgrade, custody keys, delivery and location per chain, the token-bound collection, OpenSea's chain name, the dormancy across chains). A PoC for every High, PASSING by exhibiting the bug: `autobuyer/canister-rs/tests/audit_c3_robinhood.rs` (pure, the `rh_*` collections of `tests/ethnft_sol.rs`) and `ops/test/audit_c3_robinhood.test.ts` (the runner's pass with Relay's real Solana quotes rebound to 4663). No `src` file was changed. |
| Result | 0 Critical, 3 High, 2 Medium, 4 Low, 6 Informational |

## 1. Threat model

**Assets**

1. **The module SOL advanced** for a 4663 buy (`M[i] → K_main`, then Relay SOL → 4663) and the SOL that comes back (refunds on Solana, the way back 4663 → SOL): the shared Solana balance every Relay leg's watch reads.
2. **ETH on each chain** at the canister's one secp256k1 address: on Ethereum and on 4663, each side's arrived buys (earmarks), its gas pot and leftovers.
3. **The custody NFTs on 4663** (CASHCAT-like ERC-721s and StonkBrokers, an ERC-6551 collection), their custody cap entries (`4663:0x…`), their deliveries.
4. **The books' module floors** of the 4663 listings (what the canister posts decides what it pays and what wX is worth).
5. **The one Relay leg** canister-wide (every side's buys, ways back and sweeps, and the frog side's Bitcoin conversions).

**Actors**

| Actor | Capability | In scope |
|---|---|---|
| A seller on 4663 | Lists on OpenSea (Seaport 1.6 on 4663), cancels, front-runs, drains a token-bound account before the fill, wash-trades between own wallets | yes |
| One malicious provider (either chain) | Any answer; the agreement of all is the trust root | yes |
| Relay | Fills, refunds (on either chain of an order), pays late, rotates its allocator | yes (as A5 / B1) |
| A griefer | Sends ETH or NFTs to the canister's address on any chain; lists junk | yes |
| A hostile runner hand-in | Any quote, listing or floor evidence handed to `eth_relay_quote` / `eth_listing` / `eth_floor_evidence` for any buy id | yes |
| The operator | The install / upgrade argument (`evm_chains`, `eth_nft`), the exits (`eth_give_back`, `eth_credit_payout`, `eth_late_release`, `eth_line_clear_on`), the launch files, the reconciler's env | trusted, foot-guns checked |

## 2. Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| C3-01 | `token_bound` is a label: a StonkBrokers NFT whose account its seller drained is bought at the floor of full accounts | **High** | `poc_c3_01_a_drained_token_bound_nft_is_bought_at_the_full_floor_nothing_reads_the_account` |
| C3-02 | The floor's sale window, final depth, re-anchor lag and scan are counted in Ethereum blocks on a 0.104 s chain: 50,400 blocks is 87 minutes on 4663, three wash sales are the whole sample | **High** | `poc_c3_02_the_sale_window_is_ethereum_blocks_on_4663_and_three_wash_sales_are_the_whole_sample` |
| C3-03 | The runner's pre-judge has no 4663 Relay leg: no SOL → 4663 or 4663 → SOL quote is ever handed in, and a `Funded` 4663 buy holds the one Relay leg (every chain's buys and the Bitcoin conversions) until the operator gives it back | **High** (ops) | `ops/test/audit_c3_robinhood.test.ts` (2); `poc_c3_03_a_4663_buy_funded_without_a_quote_holds_the_relay_leg_for_ever` |
| C3-04 | `eth_credit_payout`: on the books home `Init.chain_id` is 4663, so a 4663 side's late payout on 4663 is filed as a HOME payout, which the Solana home never reads: it cannot be credited | Medium | none |
| C3-05 | The monitor reads `eth_nft_state()` only: the 4663 side (its line, a halt, a stuck or quote-less buy holding the Relay leg, a late hold, its sale scan) is never paged | Medium | none |
| C3-06 | An upgrade that moves `eth_nft.chain_id` onto a chain that already has a side orphans that side: its open buy is never ticked and holds the Relay leg | Low | none |
| C3-07 | `home_books::coll_of` matches the books' `EthNft` listing by address alone; its `chain_id` (what the reconciler locates the item on) is never checked against the collection's chain | Low | none |
| C3-08 | The launch file's StonkBrokers custody bound (3 items, 5.52 ETH) is not what the canister enforces: stage 0's EVM default is 6 items and 15 ETH a collection | Low | none |
| C3-09 | The reconciler's 4663 `ownerOf` takes one provider's word when the other fails, and never checks `eth_chainId` | Low | none |
| C3-10 | The dormancy is refused across chains: adding one 4663 collection keeps every Ethereum collection awake (cycles), and the ETH-only endpoints | Informational | none |
| C3-11 | The 4663 sale scan and evidence use Ethereum's Seaport 1.5, Blur v2 and ConduitController addresses | Informational | none |
| C3-12 | Ethereum-sized gas constants on 4663; Relay's 4663 depository pinned by address and allocator only | Informational | none |
| C3-13 | The docs key the caps `evm:4663:0x…`; the code keys them `4663:0x…` | Informational | none |
| C3-14 | `side_buy_base` masks the chain id to 24 bits | Informational | none |
| C3-15 | A token-bound NFT's transferability may depend on its account (an ERC-6551 lock): unverified against StonkBrokers' contract | Informational | none |

---

### C3-01: `token_bound` is a label (High)

**Files:**
- `src/ethnft.rs:52-65` (`EthCollection.token_bound`, `is_token_bound`: the doc comment says "priced and bought as the NFT only … never a valuation of what the account holds (its seller can drain it before the sale)")
- `src/ethnft.rs:963-1016` (`floor_from`: the floor is the listings and the counted sales, no other input), `:1040-1053` (`buy_cap`: that floor plus the premium within `max_purchase_wei`)
- `src/nftnet.rs:218-226` (`check_any`: any token of the collection under the cap)
- `deployments/launch/robinhood-collections.json` (StonkBrokers, `maxPurchaseWei` 1.84 ETH = 1.25 × its 1.47 ETH floor, `tokenBound: true`)

**Description.** `is_token_bound` has no caller, and nothing in `ethflow`, `ethnet`, `nftnet`, `seaport`, `home_books`, `relaysol` or `custodycap` reads the flag or any ERC-6551 account. A token-bound collection is priced and bought exactly like any other. But its floor is not "the NFT only": the listings and the sales the floor is cut from are prices for NFTs whose accounts hold tokenized stocks (the reason StonkBrokers trades at 1.47 ETH; `robinhood-everything.md` §3.3: "each item's value is mostly the tokenized stocks in its ERC-6551 account … the items are not interchangeable at a floor"). The canister takes any token of the collection at or under that floor plus the premium, and never looks at what the account holds, at the judge or at the fill.

**Scenario.** A StonkBrokers holder moves the stocks out of the account (the owner controls it), then lists the empty NFT at the floor of full accounts (or just under the second seller's price, so the runner, which hands in the cheapest listing under the bound, picks it). The canister buys it at ≈ 1.47–1.54 ETH from the module's SOL. The module now holds an item worth the bare NFT; the next holder who redeems it receives an empty account. No collusion is needed and it repeats at every buy of the collection: a contents-blind buyer is the natural exit for every drained account (adverse selection), so after the first few buys the custody holds only drained items. The risks page discloses that a redeemed account may be empty; it does not disclose that the module pays the full-account price for it. Bounded per item by `max_purchase_wei` (1.84 ETH) and per collection by the custody cap (C3-08: 6 items, 15 ETH at stage 0).

**PoC.** `tests/audit_c3_robinhood.rs::poc_c3_01_…`: three sellers list full accounts at 1.47 / 1.50 / 1.55 ETH, four counted sales of full accounts at 1.45–1.50 confirm them, the attacker's drained #77 is listed at 1.50: `floor_from` posts 1.50 ETH at full confidence (the drained listing does not move it), `buy_cap` with StonkBrokers' 500 bps and 1.84 ETH bounds is above 1.50, so the drained item is bought; and a scan of the flow's sources shows `token_bound`, `is_token_bound(` and "6551" are read nowhere.

**Fix.** Either do not list a token-bound collection (§3.3's earlier view), or make the flag do something: for a token-bound collection read the account at the pin (the ERC-6551 registry's `account(...)` for the token, then its holdings of a pinned list of tokenized-stock ERC-20s, every provider agreeing) and (a) refuse a listing whose account holds less than the account of the median counted sale held, or (b) price the NFT and the account separately (floor of the bare NFT from sales of drained accounts, plus the holdings at a pinned oracle), re-checked in the simulation's block (the seller can still drain between the judge and the fill: the fill must be atomic with the check, e.g. a Seaport consideration / zone that asserts the account's balances). The PoC then fails (`is_token_bound` gets a caller, or `floor_from` / `buy_cap` take the account's value).

**Fixed (2026-10-12):** the rule is the report's "price the NFT apart from the account". The canister prices a token-bound collection as its bare NFT, at a share the argument states.
- `EthCollection.nft_value_bps` (an `opt nat16` in (0, 10,000)) is the share of a full account's market price that is the NFT alone. It is required with `token_bound` and refused without it (`ethnft::validate`).
- `ethflow::reading_of` takes every verified listing and every counted sale of the collection at that share (`ethnft::nft_only`, `Col.nft_bps`) before `floor_from_on` cuts the floor.
- So the floor posted, the books module's value and the buy bound (`buy_cap`: the reading and the accepted floor, both at the share) are the NFT's. A drained account listed at the floor of full accounts is far over the bound.
- A full account is bought only at a bare NFT's price, so what an account holds is never paid for. No rule reads the account, and a seller can drain it before the fill.
- The listings and sales are scaled together, so the floor's ratio rules (the band, the quartile, the up cap) are unchanged.
- The launch file names no share. The operator sets it from sales of drained accounts before StonkBrokers is pinned; until then the install refuses the collection.

Regression: `tests/audit_c3_robinhood.rs::fixed_c3_01_a_drained_token_bound_nft_is_over_the_bound_the_floor_is_the_nft_alone`. At a 2,000 bps share, full accounts at 1.47–1.55 ETH post 0.30 ETH; the bound is 0.315 ETH; the drained account at 1.50 ETH is refused. Also `tests/ethnft_sol.rs::rh_stonkbrokers_…` (now with its share).

---

### C3-02: the floor's block counts are Ethereum's on 4663 (High)

**Files:**
- `src/ethnft.rs:111` (`SALE_WINDOW_BLOCKS = 50_400`, "7 days of 12 s blocks"), `:139` (`SALE_FINAL_DEPTH_BLOCKS = 128`), `:143-146` (the chunk sizes and `SALE_SCAN_CHUNKS_PER_PASS = 13`), `:191` (`SALE_REANCHOR_LAG_BLOCKS = 7_200`), `:203-209` (`SALE_LAG_*`)
- `src/ethflow.rs:1467-1469` (`sale_scan`: `top = pin − SALE_FINAL_DEPTH_BLOCKS`, `bottom = top − SALE_WINDOW_BLOCKS`), `:1532` (the re-anchor), `:2280-2283` (`reading_of`: the window the floor counts)
- `src/ethnft.rs:963-1016` (`floor_from`: with `accepted == 0` the first post needs a quorum, and is otherwise unanchored)
- `src/ethnft.rs:83` (`logs_max_blocks`: one value for every chain's side)

**Description.** Every side runs the same floor with the same constants. On Robinhood Chain (0.104 s blocks, `docs/v4/eth-nft-canister.md` §2–3) the sale window of 50,400 blocks is 5,241 s, 87 minutes, where it was designed as 7 days; "final" is 13 s of blocks; a 12.5-minute lag re-anchors the walk; and 13 chunks of 500 blocks a pass at the runner's 72 passes a day read 468,000 blocks a day where 4663 makes ≈ 830,000, so the forward walk is always behind and re-anchors every pass (and pages its lag for ever). The sale confirmation that audit 2026-09-27-nft built (B-H-1, A-M-1, B2-M-1: a floor rises only when `min_sales` counted sales over a WEEK confirm it, the post bounded by their 25th percentile + 10 %) is therefore a confirmation by whatever sold in the last hour and a half.

**Scenario.** (1) Organic: Chain Mancers and Rekt Tradooors passed the measurement with ≥ 30 sales in 30 days (≈ 1 a day). Three counted sales in one 87-minute window rarely happen, so a 4663 listing's FIRST post (the module starts at `accepted = 0`, `books/floor.rs:107`) never comes ("no first post until the listings are confirmed"), and once posted the floor can only be held or fall. (2) Attack: a seller with six wallets makes three wash sales of three tokens at the price they want (distinct tokens and wallets, no round trip: exactly what `counted_sales` counts) and lists three tokens from three sybil offerers at that price. Inside one 4663 window those three sales are the whole sample: the post is the attacker's listing floor at confidence 10,000; on a fresh listing the module takes it whole. The canister then buys from the attacker at that floor plus the premium, within `max_purchase_wei` (1.5 × the measured floor for Chain Mancers and Rekt Tradooors, 1.25 × for StonkBrokers). Cost: three marketplace fees a window (gas on 4663 is negligible). On Ethereum the same three sales sit among a week of organic sales and the post is held at their 25th percentile + 10 %.

**PoC.** `tests/audit_c3_robinhood.rs::poc_c3_02_…`: the window is ≥ 7 days at 12.1 s and < 90 minutes at 0.104 s; the final depth, the re-anchor lag and the daily scan budget against 4663's blocks; `floor_from` with three sybil listings and three wash sales at 0.40 ETH (CASHCAT's market ≈ 0.26) and `accepted = 0` posts 0.40 at confidence 10,000; the same sales among ten organic ones (Ethereum's week) are held under 0.34; `buy_cap` then lets 0.40 + 5 % through under a 0.42 ETH bound.

**Fix.** Make every block count of the floor per chain: derive the window, the final depth, the re-anchor lag and the lag thresholds from the chain's block time (an `EvmChain` field, checked at the pin against the pinned block's timestamp), or cut the window by TIME (the sale's block timestamp ≥ pin timestamp − 7 days) instead of by count; give each chain its own `logs_max_blocks` and a scan budget that covers its daily blocks (or read 4663's sales through the marketplace's `OrderFulfilled` logs only, which are sparse). Anchor the first post of a listing to the measured floor (the launch file's `floorSol`) within a band. The PoC then fails at the window assertion.

**Fixed (2026-10-12):** every block count of the floor is per chain.
- `chains::block_ms(chain)` gives the chain's block time: Ethereum 12,000 ms, Robinhood Chain (and its testnet) 104 ms, Base 2,000 ms. `ethnft::validate` refuses an NFT collection on a chain without one.
- `ethnft::sale_blocks(chain)` derives the floor's block counts from spans in seconds:
  - the sale window: 7 days;
  - the final depth: 1,536 s;
  - the re-anchor lag: a day;
  - the lag pages: 8 h and a day;
  - the scan's chunks a pass: enough to cover 3 of the runner's 20-minute cadences of the chain's blocks, at least Ethereum's 13.
- For Ethereum these are exactly the old constants. On 4663 the window is 5,815,384 blocks, the final depth 14,769, the re-anchor lag 830,769, and 18 chunks a pass.
- `ethflow::sale_scan`, `reading_of` and the waking estimate use the side's own counts.
- A FIRST post (the module's accepted floor 0, nothing to hold it to) now also needs its `min_sales` counted sales spread over at least a day of the chain's blocks (`FIRST_POST_SPAN_SECS`, `floor_from_on`), on every chain. The counted sales are already distinct tokens and wallets, with no self-trade or round trip. A burst of wash sales between fresh wallets inside an hour is no first price; a held post after the first is unchanged.
- Not changed: `dormancy.rs` still cuts its backfill with Ethereum's window. The dormancy is refused with collections on several chains, so it only ever runs on the config's own chain. A single-chain 4663 config with a dormancy would need it per chain; recorded.

Regression: `tests/audit_c3_robinhood.rs::fixed_c3_02_the_sale_window_is_7_days_of_each_chains_blocks_and_three_wash_sales_are_no_first_post`. It checks the window, final depth, re-anchor lag, lag page and daily scan budget on both chains. The three wash sales in an hour are refused as a first post on 4663 and on Ethereum; the same sales spread over a day post; a held post is unchanged. Also `ethnft::tests` (first posts now from sales a day apart), and `tests/ethnft_sol.rs::rh_a_floor_post_on_4663_…`, whose synthetic sales sit under 4663's final depth.

---

### C3-03: the runner never hands in a 4663 Relay quote (High, ops)

**Files:**
- `ops/src/relaysol.ts:187-191` (`solLegOf`: `sol-btc`, `sol-eth` (destination 1), `eth-sol` (origin 1); anything else `null`), `:194-205` (`preJudgeSolAsk`: refuses a `null` leg; `destChain: "ethereum"` hard-coded), `:133-175` (`judgeEthToSol`: origin `"ethereum"` hard-coded)
- `ops/src/ethnft.ts:98-100` (`preJudgeEth`: a Solana leg goes to `sol.judge` = `preJudgeSolAsk`), `:366-375` (a refusal is logged `eth-relay-rejected` and nothing is handed in)
- `src/ethflow.rs:2516-2525` (`ask_for`: the 4663 side asks `RELAY_SOLANA → 4663`), `:3271` (its way back `4663 → RELAY_SOLANA`)
- `src/ethflow.rs:305-311` (`in_relay`: `NeedAdvance`, `Advancing`, `Funded`, `BackFunded` … hold the leg), `:2407` (no new buy while any side holds it), `src/ethnet.rs:37-45` (`relay_busy_except`, `any_relay_busy`), `src/frognet.rs:224` (the frog side waits on it)

**Description.** The canister's 4663 side asks Relay for SOL → 4663 (destination `"robinhood"`) and, on the way back, 4663 → SOL (`ROBINHOOD_TO_SOLANA`). The runner fetches each quote, then pre-judges it with `preJudgeSolAsk`, whose `solLegOf` knows Ethereum only: both 4663 legs are refused ("no Solana Relay leg from chain 792703809 to 4663") before the quote is even read, so `eth_relay_quote` is never called. The `rh_*` tests drive the canister directly (`ef::put_quote`), and the ops test of the 4663 side (`ethnft.test.ts`) covers the listing hand-in only, so nothing exercised the quote hand-in.

**Scenario.** The first 4663 buy advances its SOL (`M[i] → K_main`) and waits `Funded` with its ask. No quote ever comes. No timeout moves a `Funded` buy without a quote, and `Funded` holds the one Relay leg canister-wide: no Ethereum collection opens a buy (`relay_busy_except`), no Ethereum way back is signed, and Bitcoin Frogs' conversions wait (`any_relay_busy`). Everything stops until the operator notices (C3-05: the monitor does not read the 4663 side) and calls `eth_give_back`; a day after (`RETURN_COOLDOWN_SECS`) the round robin opens the next 4663 buy and it starts again. A 4663 buy that did arrive (none can) would likewise sit `BackFunded` on its way back. No funds are lost; the launch's NFT buying and Bitcoin conversions are.

**PoC.** `ops/test/audit_c3_robinhood.test.ts`: `preJudgeSolAsk` refuses Relay's real SOL → ETH quote rebound to `"robinhood"` (the canister's `sol_to_rh`) and the real ETH → SOL quote for a 4663 origin, while the same asks on chain 1 pass; one `ethNftOnce` pass with a `Funded` 4663 side buy asks Relay, logs `eth-relay-rejected … no Solana Relay leg`, and hands nothing in. `tests/audit_c3_robinhood.rs::poc_c3_03_…`: a `Funded` 4663 buy holds `relay_busy`, its phase does not age.

**Fix.** In `relaysol.ts`, add the two legs (`RELAY_SOLANA → 4663` judged by `judgeSolOrigin` with `destChain: "robinhood"`; `4663 → RELAY_SOLANA` by `judgeEthToSol` with origin `"robinhood"` and its deposit's `chainId` 4663), from the same table as the canister's `relay_name` / `to_solana`, and a test that hands a 4663 quote in end to end. In the canister, consider moving a `Funded` buy with no quote judged for N hours back to its module (as `BACK_QUOTE_SECS` does for the way back), so one chain's dead leg cannot hold every chain's. The ops PoC then fails.

**Fixed (2026-10-12):** the 4663 quotes are handed in, and the canister now times out a quote-less buy.
- Ops: `relaysol.ts` `solLegOf` names `sol-rh` (`RELAY_SOLANA → 4663`) and `rh-sol` (`4663 → RELAY_SOLANA`), from one table (`NFT_CHAINS`: 1 `"ethereum"`, 4663 `"robinhood"`) as the canister's `relay_name` / `to_solana`.
  - `sol-rh` is judged by `judgeSolOrigin` with `destChain: "robinhood"`.
  - `rh-sol` is judged by `judgeEthToSol` with `origin: 4663`: the deposit's `chainId`, the order's payment and its origin refund on `"robinhood"`, the same depository address.
  - The runner's existing wiring (`solJudge: preJudgeSolAsk`, `sideStates`) hands the 4663 quote in for the side's buy id.
- Canister: a `Funded` buy that no quote reached for `ethflow::FUNDED_QUOTE_SECS` (2 h, as `BACK_QUOTE_SECS`) goes back to its module (`Returning`, as C2-L-2's dropped collection). Its collection then cools down for `RETURN_COOLDOWN_SECS`, so one chain's dead leg never holds the one Relay leg for every chain and the Bitcoin conversions.

Regressions:
- `ops/test/audit_c3_robinhood.test.ts`, both inverted to `fixed C3-03`. Both 4663 legs are judged, and each chain's quote is refused on the other's leg. One runner pass hands the 4663 quote in for buy `4663 << 40`.
- `tests/audit_c3_robinhood.rs::fixed_c3_03_a_4663_buy_funded_without_a_quote_is_bounded_by_funded_quote_secs`.
- `tests/ethnft_sol.rs::fixed_c3_03_a_funded_buy_no_quote_reaches_goes_back_to_its_module_and_frees_the_relay_leg`: on the books home it is `Funded` at 2 h − 1 s, `Returning` at 2 h, then `Returned`, the leg free and the books' transit cleared.

---

### C3-04: a 4663 side's late payout is filed as a home payout (Medium)

**Files:** `src/ethnet.rs:790-809` (`eth_credit_payout`: `home = cfg().chain_id`; `put_payout(…, chain_id == home, …)` is decided before the side's chain), `src/ethnet.rs:640-646` (`payout_tx(home = true)` on the Solana home: "not read by this build"), `scripts/golive-dryrun.sh:122,141` (the books home's `Init.chain_id` is 4663).

**Description.** On the books home the install's `chain_id` is still 4663 (the vault's chain). For a buy of the 4663 side, `side` is 4663 too, so a payout handed in with `chain_id = 4663` matches `chain_id == home` and is saved as a HOME payout; the judge then asks `payout_tx(home = true)`, which the Solana home refuses. A written-off SOL → 4663 order that Relay pays late on 4663 (or a way back refunded late on its origin 4663) can therefore never be credited to its buy; the operator's only exit is `eth_late_release`, which leaves the ETH at our 4663 address unattributed (it is no collection's leftover, so no sweep returns it). Bounded by the order's value (≤ `max_purchase_wei`), on a rare path (a written-off order).

**Fix.** Decide the chain before the home: `let on_side = chain_id == side; let home_pay = !on_side && chain_id == home && cfg().home.is_none();`, and on the Solana home refuse `chain_id == home` unless it is the side's chain.

**Fixed (2026-10-12):** `ethnet::payout_is_home(chain_id, home, side, solana_home)` decides the side's chain first:
- a payout on the buy's side's chain is the side's (`put_payout(home = false)`: read on that chain, credited to its buy);
- on the Solana home, a payout on the install's chain that is not the side's is refused;
- the EVM home is unchanged.

Regression: `ethnet::tests::fixed_c3_04_a_4663_sides_late_payout_on_4663_is_the_sides_not_the_homes`.

---

### C3-05: the monitor never reads the 4663 side (Medium)

**Files:** `monitor/src/ic.mjs:35,58` (`eth_nft_state()` only), `monitor/src/ethnft.mjs` (every Ethereum-side alert), `ops/src/runner.ts:256-263` (the runner reads `eth_nft_state_of`; the monitor does not).

**Description.** Every page the monitor raises for the Ethereum NFT side (a halted line, a stuck buy, a late hold, the sale scan's lag, the gas pot, deliveries near their deadline) is computed from `eth_nft_state()`, the config's own side. The 4663 side's state is served only by `eth_nft_state_of(4663)`, which the monitor never calls: a halted 4663 line (a delivery that cannot be signed until `eth_line_clear_on`), a 4663 delivery approaching `deliverBy`, a 4663 order written off and holding the Relay leg, or C3-03's quote-less `Funded` buy are all silent.

**Fix.** Read `eth_nft_state_of(chain)` for each chain `eth_nft_config()` names beside its own (as `runner.ts sideStatesOf` does) and run the same alerts on each, keyed by chain.

**Fixed (2026-10-12):** the monitor now reads and pages every other chain's side.
- The monitor's IDL and `ic.mjs` read `eth_nft_config()` and `eth_nft_state_of(chain)`.
- `ethnft.mjs` `sideAlerts` (run by `checkEthNft` after the config's own side) runs `ethNftAlerts` on the state of every chain the collections name besides the config's own (`sideChainsOf`).
- Its pages are keyed `…:b@chain4663:…` and cover:
  - a stuck or quote-less `Funded` buy, a way back;
  - a late hold;
  - deliveries near their deadline;
  - the sale scan's lag, in that chain's own blocks (`scanBlocksOf`).
- Also paged: a halted line on that chain (SEV1, naming `eth_line_clear_on(chain)`), and a side or config the monitor cannot read (SEV2).
- The sale scan's page thresholds of the config's own side are unchanged.

Regression: `monitor/test/audit_c3_monitor.test.mjs` (3 tests).

---

### C3-06: an upgrade that moves the config's chain onto a side orphans the side (Low)

**Files:** `src/canister.rs:748-762` (the stranded check: each saved side against `arg.eth_nft.on(chain)` whenever the new config names the chain, as its own chain or not), `src/ethnet.rs:53-65` (`with_side`: the config's own chain is always `ETH`, never `SIDES`), `:37-50`.

**Description.** With `eth_nft.chain_id = 1` and a 4663 side holding an open buy, an upgrade to `chain_id = 4663` (Ethereum's collections now a side) passes: the 4663 side's open buy is "not stranded" (its collection is still on 4663) and `ETH` was idle. After it, `with_side(Some(4663))` is `ETH`, so the saved `SIDES[4663]` is never ticked, never shown (`eth_nft_state_of(4663)` returns `ETH`), and its buy holds the Relay leg for good (`relay_busy_except(None)` still iterates it); `side_of_buy` finds the buy there and `with_buy` sends the operator's exits to `ETH`, which does not have it. Operator-only, recovered by another upgrade.

**Fix.** In `post_upgrade`, refuse a saved side whose chain is the new config's own chain unless it holds no unfinished buy, delivery, line or late hold (or merge it into `ETH` when `ETH` is empty).

**Fixed (2026-10-12):** `post_upgrade` passes the saved sides through `ethnet::sides_kept(sides, new_own_chain)`.
- A side on the chain the upgrade makes the config's own is refused (the upgrade traps) while it holds a line, a home transaction, an unfinished buy or delivery, or the Relay leg (a late hold included).
- An idle side there is dropped.

Regression: `ethnet::tests::fixed_c3_06_an_upgrade_onto_a_sides_chain_is_refused_while_it_holds_work`.

---

### C3-07: the books' listing is matched by address alone (Low)

**Files:** `src/ethflow/home_books.rs:23-27` (`coll_of`: `ListingKind::EthNft { collection, .. } if collection == col.collection`), `src/books/mod.rs:243-256` (`add_collection` checks no uniqueness of an `EthNft` address across chains), `ops/src/booksreserves.ts:132-147` (the reconciler reads `ownerOf` on the listing's `chain_id`).

**Description.** The 4663 side posts, mints, settles and delivers into the first books listing whose `EthNft.collection` is the address, whatever its `chain_id`. If the books listing names another chain than `eth_nft` does (a pin's typo), or a twin collection at the same address is listed for the other chain (same-deployer, same-nonce or CREATE2 collections exist on both chains), the canister buys on one chain while the books, the site and the reconciler locate the item on the other ("unverified", or "elsewhere", or, for a twin token id, a wrong "custody").

**Fix.** Match `ListingKind::EthNft { chain_id, collection, .. }` on both, with the side's chain (`cfg.chain_id`), and refuse at `desk_add_collection` / the pin an `EthNft` listing whose `(chain, address)` disagrees with `eth_nft`.

**Fixed (2026-10-12):** the canister now matches a books listing by chain and address.
- `Col.chain_id` carries each collection's chain (`EthNftConfig::chain_of`).
- `home_books::coll_of` (`books_id_of`) matches `ListingKind::EthNft { chain_id, collection }` on both. `landed`'s `col_by_buy` takes the collection's chain from the side's config.
- So the canister never posts, mints, settles or delivers into a listing of the same address on another chain. A mismatched listing is "not listed in the books on chain N", and the step waits.
- Not changed: the refusal at `desk_add_collection` / the pin. The desk is outside this fix's scope, and a mismatched listing now fails closed at the canister's own use of it.

Regression: `ethflow::home_books::tests::fixed_c3_07_the_books_listing_is_matched_by_chain_and_address`.

---

### C3-08: StonkBrokers' custody bound is the stage-0 default, not the launch file's (Low)

**Files:** `deployments/launch/robinhood-collections.json` (`custodyItems: 3`, `custodyWei: 5.52 ETH`), `src/custodycap.rs:33,40,214-221` (stage 0 for an EVM key: 6 items, 15 ETH a collection, 15 ETH an item), `src/canister.rs:398-415` (`caps_of`: the argument's `custody_caps`, else stage 0 with each collection keyed by its own chain), `src/ethnft.rs:341-344` (`max_purchase_wei` ≤ 25 ETH).

**Description.** Nothing on the books home reads the launch file's custody numbers (the golive check that compared them is the EVM stack's), so unless `Init.custody_caps` names `4663:0x539c…`, StonkBrokers may hold 6 items up to 15 ETH where the file says 3 and 5.52 ETH; each buy is bounded by its own `max_purchase_wei` (1.84 ETH), so 6 × 1.84 ≈ 11 ETH, twice the stated exposure. The high cap is bounded (25 ETH ceiling, 15 ETH an item), not at the file's figure.

**Fix.** Name the three 4663 collections' caps in `Init.custody_caps` from the launch file (and check them in the ceremony's read-back), or derive stage 0's EVM item count from `max_purchase_wei`.

**Fixed (2026-10-12):** with no caps named, `canister::caps_of` starts each collection on a chain other than the config's own at the launch file's bound, not at J7's Ethereum stage 0.
- The bound is `FIRST_MAX_ITEMS` (3) items, each at most its own `max_purchase_wei`, so at most 3 × `max_purchase_wei` in all. This is exactly the file's `custodyItems` / `custodyWei` for the three 4663 collections (StonkBrokers 3 items, 5.52 ETH).
- An Ethereum collection keeps J7's 6 items and 15 ETH.
- An install that names its caps is held to them, as before.

Regression: `canister::tests::fixed_c3_08_a_4663_collection_starts_at_the_launch_files_custody_bound`, which reads `deployments/launch/robinhood-collections.json` itself.

---

### C3-09: the reconciler's 4663 ownerOf on one provider (Low)

**Files:** `ops/src/booksreserves.ts:139-147` (`Promise.allSettled`, then "all answering must agree": one answering is enough), `ops/src/reconcile.ts:147,164` (the default pair includes a third-party keyless RPC, `rpc.ordofi.network`).

**Description.** When one 4663 provider fails (rate limit, outage), the other's `ownerOf` alone decides "custody" or "elsewhere": a single malicious or broken provider can show a moved item as held. The RPCs' `eth_chainId` is never read either (an Ethereum URL in `RECONCILE_RH_RPC_URLS` reads the wrong chain; it fails safe to "unverified" only because the 4663 addresses have no code on Ethereum). The same pattern exists for Ethereum's `ownerOf`.

**Fix.** Require every configured provider to answer (any failure: "unverified"), and check `eth_chainId` once per run.

**Fixed (2026-10-12):** `booksreserves.ts` `locator` now checks each provider's chain and, off Ethereum, needs every provider.
- Each provider's `eth_chainId` is read once per locator (one reconciler run) and must be the listing's chain. An Ethereum URL in 4663's list is unverified.
- On a chain other than Ethereum it needs two providers at least, every one answering; one failing is "unverified", never the other's word alone.
- Ethereum keeps "all answering must agree", now with the chain check too.

Regression: `ops/test/books_reconcile.test.ts` ("2026-10-12: a Robinhood Chain NFT's owner…", extended): one 4663 provider configured, one failing, or one serving chain 1 are all unverified.

---

### C3-10: the dormancy across chains (Informational)

`ethnft::validate` refuses a dormancy with collections on several chains (`src/ethnft.rs:323-325`). Adding any 4663 collection therefore keeps every one of the 11 Ethereum collections ACTIVE as well: every collection's sale scan runs (up to `SALE_SCAN_BYTES_PER_DAY` each, ≈ 10.5 T cycles a day at worst), `eth_nft_capacity` is `None`, and the runner's `/capacity` is empty. The ETH-only endpoints (`eth_wake`, `eth_fill`, `eth_deposit`, `eth_home_clear`, `eth_nft_collections`, `eth_nft_capacity`) only reach the config's own side; with the dormancy off and deposits refused on the books home that is consistent today, but a dormancy per side would need them per chain.

**Recorded, not changed:** the dormancy stays refused across chains (fail-safe: every collection stays awake, at the cycles cost described). A per-side dormancy, with its endpoints and `dormancy.rs`'s window (still Ethereum's blocks, C3-02) per chain, is a feature for when a 4663-only or mixed dormancy is wanted.

### C3-11: Ethereum addresses on 4663 (Informational)

`eth_sale_moves_whole` (`src/ethnet.rs:594-599`) reads `OrderFulfilled` from Seaport 1.6 and 1.5 and Blur v2's `Execution721*` at their Ethereum addresses on every chain; on 4663 only Seaport 1.6 is deployed (OpenSea's measurement), so the Blur read is a wasted log read per chunk (Blur's deployer could put any code there; its logs would only add sales, which still need the collection's own `Transfer`). `eth_listing` (`:533`) reads Seaport's `ConduitController` at its Ethereum address (deterministic; an absent controller refuses a conduit order).

**Fixed (2026-10-12):** `ethnft::sale_venues(chain)` names the venues the sale scan reads. On Ethereum: Seaport 1.6 and 1.5, and Blur v2. On any other chain: Seaport 1.6 alone, at its canonical address. `ethnet::eth_sale_moves_whole` asks only those, so there is no Blur or Seaport 1.5 read on 4663. The `ConduitController` read is recorded, not changed: its address is deterministic across chains, and an absent one fails closed (a conduit order is refused).

### C3-12: Ethereum-sized gas constants, Relay's 4663 depository (Informational)

`ETH_GAS_RESERVE_WEI` (0.003 ETH a purchase), `GAS_KEEP_WEI` and the one `gas_pot_target_wei` are sized for Ethereum and applied to 4663, whose gas is ~1,000× cheaper: the 4663 side keeps more ETH idle than it needs. Relay's depository on 4663 is pinned by address and by its `allocator()` read on 4663 (`ROBINHOOD_TO_SOLANA`), not by code hash, as on Ethereum.

**Recorded, not changed:** the gas constants stay Ethereum-sized on 4663. `ETH_GAS_RESERVE_WEI` is read by the buy bound, the advance gate (`ethflow::advance_covers` / `advance_bound` and `solhome`'s), the way back and the runner's pre-check alike, so a per-chain value is a change across the flow and ops. The direction is conservative: about 0.003 ETH a buy kept idle on 4663, never a shortfall. The depository's pinning is as on Ethereum's Solana leg; noted for the next Relay review.

### C3-13: the caps' key in the docs (Informational)

`robinhood-everything.md` §3 and `HANDOFF-2026-10-10.md` say the caps are keyed `evm:4663:0x…`; `custodycap::evm_key` writes `4663:0x…`. Anything written against the documented key (a launch file, a check) would not match.

**Fixed (2026-10-12):** the docs now name the key the code writes, `4663:0x…`: `robinhood-everything.md` §3 and `canister.rs` `caps_of`'s comment. `HANDOFF-2026-10-10.md` names no such key. `fixed_c3_08_…` asserts the `4663:0x…` keys.

### C3-14: `side_buy_base` masks the chain (Informational)

`side_buy_base(chain) = (chain & 0xFF_FFFF) << 40` (`src/ethflow.rs:5354`): a chain id that is 0 modulo 2^24 would count its buys from 0, the config's own side's range, and two chains equal modulo 2^24 would share one. Unreachable while `relay_name` admits 1 and 4663 only; worth an install check if more chains come.

**Fixed (2026-10-12):** `ethnft::validate` refuses a side chain whose low 24 bits are zero, or equal another side's. The ids of today's sides (4663) are unchanged. Regression: `tests/audit_c3_robinhood.rs::fixed_c3_02_c3_14_an_nft_chain_without_a_known_block_time_is_refused_and_side_ids_are_unchanged`. Today `relay_name` refuses such a side chain first; the check guards the next chain added.

### C3-15: a token-bound NFT's transferability (Informational)

A delivery is the ERC-721 `transferFrom` alone (correct: the account goes with the NFT, and custody never calls the account). Some ERC-6551 implementations let the owner LOCK the account until a time, and some NFT contracts refuse transfers while their account is locked. If StonkBrokers' contract does, a seller can lock before selling and the canister holds an item it cannot deliver until the lock expires. Not verified (no StonkBrokers source offline): read its `transferFrom` / `_beforeTokenTransfer` before listing.

**Recorded, not changed:** this still needs StonkBrokers' verified source, which is not available offline. Before StonkBrokers is pinned, the operator reads its `transferFrom` / `_beforeTokenTransfer` and its account implementation's lock, alongside setting its `nft_value_bps` (C3-01). A locked item would sit in custody until the lock ends, bounded by the stage-0 cap of 3 items (C3-08).

## 3. Checked and found correct

**Cross-chain confusion**
- A Seaport fill: `nftnet::check` reads `eth_chainId` at every pin (`pin_on`, refused if not the side's chain), the code hash of Seaport 1.6 against THIS chain's `seaport_codehash`, `information()`'s version and domain against `domain_separator(chain_id, seaport)`, and the offerer's EIP-712 signature over that domain: an Ethereum order is refused on 4663 (`rh_an_order_signed_for_another_chains_seaport_is_refused_on_4663`), as is another collection's. Restricted orders pin each zone by its code hash on that chain.
- Every transaction a side signs is `Tx1559 { chain_id: io.eth_chain() }` (the side's chain): fills, deliveries, returns, the way back's deposit; the same key's nonces are separate per chain, and each side keeps its own line, `last_mined`, nonce floor and pin floor (`SIDE_FLOORS`): a 4663 pin is no floor for Ethereum's.
- Relay: an order id is the EIP-712 hash of the order, which names its origin and destination chains; rule 2 binds the deposit's `chainId`, rule 7 the payment's chain, rule 9 / S9 the output's: a quote for one side cannot be judged valid on the other (the real mainnet quotes are refused on the 4663 side by S8/S9 and rule 2). S8 / rule 8 accept refunds only on the order's two chains to our address, so a refund lands on the side that watches it (the Solana balance for the forward leg, the side's chain for the way back). `used_orders` per side is enough for that reason.
- Floor evidence is routed to the side of the collection's chain (`chain_of_addr`), judged at that chain's pin and Seaport.

**The shared Solana line and the Relay leg**
- Every side is `LineOwner::Eth`; `resolve` partitions the outcomes by `other_side_step` (`owns_step`: buy ids per side, collections and wrappers per chain), so a side never takes another's outcome; the taken-back rule only fires for a side's own step.
- The one leg: a side's new buy, its way back and its sweep wait on `relay_busy_except(self.side)` (every other side, the config's own included) and the frog side's conversions on `any_relay_busy`; a side saves its state (`io.save`) before the next side ticks, so two sides never open legs in one tick. A written-off order's `late_hold` holds the leg on every side.

**Ids, keys, custody, delivery, the upgrade**
- Buy ids: the config's own side from 0, each other side from `chain << 40`; `with_buy` routes every operator exit to the buy's side. Custody keys and held ids carry the side's chain (`key_of(c, cfg.chain_id)`, `held_id(chain, …)`); the install's stage 0 keys each collection by its own chain.
- Delivery on 4663: `eth_plan_transfer` re-reads `ownerOf` at the side's pin and checks the side's own gas pot (the 4663 balance), the transfer signed for 4663 (`rh_a_delivery_on_4663_…`).
- One writer per 4663 nonce line: `nft_sign` and `sale_nft_listing` refuse every chain `eth_nft` names; the vault loop opens nothing on the Solana home.
- The upgrade: `EthState.sides` is optional (an old single-chain state decodes with none); a side that does not decode refuses the upgrade; each side's stranded work is checked against its chain's collections (C3-06 aside); `EthCollection.chain_id` / `token_bound` are `opt` with serde defaults.
- The install: a collection's chain must be in `evm_chains` and named by Relay; one address once across chains; several chains only on the books home.

**Ops**
- `sideStatesOf` reads `eth_nft_state_of` for each other chain; a listing is fetched on the collection's OpenSea chain (`openseaChainOf(chainOfCollection(...))`, `robinhood`, confirmed by the measurement) and handed in by the side's buy id; `RecordsCache` reads each collection from its chain's side (the EVM-stack home only). The reconciler reads `ownerOf` on the listing's chain with `RECONCILE_RH_RPC_URLS` (C3-09 aside).
- `measure-robinhood-nfts.mjs`: GETs only, the criteria as documented, QUOTRONS excluded (ERC-404), the token-bound cap 1.25 × floor within 25 ETH.

## 4. Test results

- `cargo test --locked --no-fail-fast` in `autobuyer/canister-rs` at `79b4e60` plus `tests/audit_c3_robinhood.rs`: every test binary green (59 `test result: ok`, 1,207 tests), `audit_c3_robinhood`: 3 passed.
- `cd ops && npm ci && npm test` plus `test/audit_c3_robinhood.test.ts`: 175 passed, 0 failed (`audit_c3_robinhood`: 2 passed). The ops suite imports `indexer/src` and `keeper/src`, so their `npm ci` must have run in the checkout too (a fresh worktree without them fails 14 files on `viem`, unrelated to this change).

## 5. Fix round (2026-10-12)

Each finding's outcome is under it (**Fixed** or **Recorded, not changed**). The PoCs are inverted to `fixed_c3_0x_…` (their comments kept, "FIXED (…)" added). Candid: `EthCollection.nft_value_bps : opt nat16` is new (`etch_autobuyer.did` regenerated; `tests/fixtures/init-dormant.hex` and `init-solana.hex` regenerated from the tests' `left:`; the monitor's and the runner's IDLs carry it, and the monitor's `argdiff` pages a raise). On `golive-2026-10-08` from `1aaca49`, after `npm ci` in `indexer/`, `keeper/`, `ops/` and `monitor/`:

- `cargo test --locked --no-fail-fast` in `autobuyer/canister-rs`: 62 binaries, 1,224 passed, 0 failed (`audit_c3_robinhood`: 4 passed). `cargo check --target wasm32-unknown-unknown` and `cargo clippy --all-targets` are clean.
- `cd ops && npm test`: 177 passed, 0 failed (`audit_c3_robinhood`: 2 passed). `npm run typecheck` shows no error in `ops/` (only `tools/snapshot`'s missing `viem`, outside it).
- `cd monitor && npm test`: 283 passed, 0 failed (`audit_c3_monitor`: 3 passed).
