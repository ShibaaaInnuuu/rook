# Rook security review 2026-09-27 (nft-side), area D: off-chain services and deploy / go-live tooling

Reviewer D, 2026-09-27. Checkout `/Users/jlohayon/Documents/Rook-audit-nft-D`, detached at **57160b0** ("Launch configuration
(the operator's decisions of 2026-09-27)…", branch `nft-side`; parents e90471c "D5: holder deposits on at launch" and b48b3ba),
diffed against GitHub `main` 47a9b94. The worktree was removed afterwards.

**What was touched.**
- In the main worktree, only this report and `poc/D/`. No source or doc was modified.
- Nothing was committed, pushed, deployed or signed. No key, wallet, identity, Railway service or `.env` was touched.
- **Network use, all read-only:**
  - Ethereum JSON-RPC (`eth_blockNumber`, `eth_getLogs`, `eth_getTransactionReceipt`) on `mainnet.gateway.tenderly.co`, to measure
    what the honest runner hands in (D-H-1);
  - OpenSea's keyless `collections/milady/stats` (3 calls) and CoinGecko's keyless `nfts/milady-maker` (8 calls), to measure the
    monitor's references (D-M-1).

**PoCs added (in my checkout, then copied to `poc/D/`).** Each asserts the weakness, so a pass means the weakness is there:
- `ops/test/audit_nft_d.test.ts` (6);
- `monitor/test/audit_nft_d.test.mjs` (10);
- `ops/scripts/audit_nft_d_window.ts` and `audit_nft_d_measure.ts` (live, read-only), with the recorded output
  `audit_nft_d_window.2026-09-27T1652Z.out`.

## 1. Method

**References, as in `2026-09-26/E-services-workflow.md`:**
- OWASP API Security Top 10 (2023), used for API7 (SSRF) and API10 (unsafe consumption of third-party APIs: OpenSea, Relay,
  CoinGecko and the Ethereum RPCs);
- OWASP ASVS 5.0 V13, for secrets (the OpenSea key);
- Chainlink data feeds, "monitor and check timestamps / heartbeat / reasonable limits", for the floor and the D8 pause;
- Trail of Bits, Building Secure Contracts, incident response: monitoring the system's own invariants, and off-chain components
  as incident sources.

**Passes:**
1. **The previous audit's service findings and their fixes:**
   - `2026-09-26/README.md`, `E-services-workflow.md` (the model for this report);
   - `C-treasury-roles-deploy.md` (C-07, C-08, C-09);
   - `fixes-W2-monitor.md`, `fixes-W2-services.md`, `fixes-W2-site.md`;
   - `review2/R2-services.md`, `review2/fixes-R3-services.md`, `review3/fixes-R4-services.md`;
   - `review4/part-services.md`, `review4/fixes-golive-lows.md`.
2. **Design:**
   - `docs/v7/phase1-nft.md` in full (§6 services, §9 decisions D1–D9, §11.5 as built);
   - `docs/deploy/services-v1.md` (the runner's and monitor's variables);
   - `docs/deploy/canister-mainnet.md` (the install argument).
3. **Code, line by line:**
   - the runner: `ops/src/{ethnft,relay,runner,ic}.ts`, plus the `reconcile.ts` / `backing.ts` diff;
   - the monitor: `monitor/src/{ethnft,candid,argdiff,config,guardian,records,custodyhealth,main,evm,ic}.mjs`, plus the parts of
     `evm.mjs` that now judge the NFT stack's events;
   - the keeper: `keeper/service/{keeper,resolver,stack,abis}.ts`;
   - the indexer: `indexer/src/wrapped/*`;
   - the site: `web-rook/src/rook/{phase1Trust,ledger}.ts`;
   - deploy and go-live: `script/{DeployStack.s.sol,StackChecks.sol,deploy-stack.sh,golive-check.sh}`,
     `scripts/golive-dryrun.sh` (`rehearse_nft_stack`).
4. **The canister, only to confirm what it bounds** (reviewer B covers the judges in depth; `B-canister-judges.md` was read for
   overlap):
   - `ethflow.rs`: `put_*`, `floor_pass`, `deposit_pass`, `home_act`'s post step, `eth_act`'s mark rule;
   - `ethnft.rs`: `floor_from`, `sales_in`, `should_post`, `judge_transfer`.
5. **PoCs** against each package's own fakes, and a **live measurement** of the honest runner's evidence on Ethereum mainnet.

**Baseline, in my checkout before the PoCs.** Every suite passes: ops 45, monitor 153, keeper 150, indexer 135, web-rook 70
(`npm ci` then `npm test` in each; `tools/snapshot` and `staging` installed for the keeper).

**After the PoCs:**
- ops **51/51** (`tsc --noEmit` clean);
- monitor **163/163**;
- keeper, indexer and web-rook unchanged.

## 2. Scope

**In scope:**
- the runner's Ethereum step and the Relay quote step;
- the reconciler parts this branch touched;
- the monitor's Ethereum-NFT checker, and the parts of the 4663 canary that now judge the NFT stack;
- the keeper's NFT changes;
- the indexer's EthNftWrapper support;
- the site's Trust page NFT section;
- the NFT-only `DeployStack`;
- `golive-check.sh`'s NFT checks;
- the dry run's `rehearse_nft_stack`.

**Out of scope:**
- contract correctness (reviewer A);
- the canister's judges (reviewer B), cited only where a service relies on them.

## 3. What the runner holds in trust, and what the canister bounds

The runner holds the operator's hot IC identity (every `eth_*` update is `only_operator`) and `OPENSEA_API_KEY`. It is the
canister's only ticker. The central question is whether a malicious, compromised or buggy runner can make the canister do anything
the canister's own checks do not bound.

| Hand-in | What the runner chooses | What the canister bounds (read at 57160b0) | Verdict |
| --- | --- | --- | --- |
| `eth_relay_quote(buy, json)` | which quote, and when | `relayeth::judge` rules 1–12: order hash, our recipient and refunds, pinned depository, min-out ≥ 97 %, deadlines. One conversion at a time | **Bounded.** A fabricated order that passes is griefing only (B-I-3) |
| `eth_listing(buy, order)` | which valid listing | `buy_cap` = min(floor × (1 + premium), arrived − 0.003, max_purchase); fees and recipients; the simulation. The canister signs its own re-encoding | **Bounded by the floor.** The runner can pick its own listing at the cap (D-I-1). Since the floor is steerable (B-H-1), the cap is ≈ 1.22 × market. **Liveness:** the honest runner re-hands a refused cheapest listing for ever (D-M-4) |
| `eth_floor_evidence(c, listings[≤8], sales[≤12])` | which listings, and **which sale transactions** (any receipt within 7 days) | listing signature, owner, status and approval at the pin; sales parsed from receipts. **Nothing about who traded or which sales** | **Not bounded** except by the module (±5 %/h, ±20 %/day) and D8. B-H-1 / B-M-1. The honest runner's choice (the latest 12 *transfer* txs) posts nothing for hours a day (D-H-1) |
| `eth_deposit(tx)` | which transactions, and when | the canister reads the tx and receipt itself (both providers), mints only past finality, and returns strays | **Bounded.** The runner can only withhold, and the watchdog's window is 24 h (D-L-3) |
| `tick()` timing | whether and when to tick | nothing: the tick is operator-only and has no timer | Withholding stops everything; the monitor's liveness pages it (E-08 fix). A stalled provider withholds ticks too (D-L-1) |
| `eth_give_back(buy)` | when to return a `Funded`/`Stuck` advance | the module's `returnAdvance` | Bounded (ETH goes back to the module) |

**Invariants the services must keep:**

| # | Invariant | Status |
| --- | --- | --- |
| S1 | The OpenSea key is only sent to `api.opensea.io` in a header, never logged, never sent to the canister | **Holds.** Defense-in-depth gaps: D-I-4 |
| S2 | Every runner hand-in is re-judged by the canister on its own reads | **Holds**, except floor evidence (B-H-1) |
| S3 | The honest runner keeps each collection's floor fresh | **Violated** (D-H-1) |
| S4 | One collection's failure or slowness does not stop the others (runner, monitor, pause) | **Violated:** D-L-1 (a stall stops all; one throw aborts later collections), D-L-5 (monitor), D-H-2 and D-M-1 (one collection pauses all) |
| S5 | The monitor pages only on evidence, and a page's key does not flood | **Violated:** D-H-2 (honest moves read as a stolen key), D-M-2 (false SEV1), D-M-1 and D-L-4 (a new key every poll) |
| S6 | Every custodian-gated 4663 event and every Ethereum outbound transaction of the canister is matched to a record (E-canary model) | **Violated** (D-M-2, D-M-3) |
| S7 | Every inbound NFT is either judged or paged until it is | **Violated after 24 h** (D-L-3) |
| S8 | The D8 guardian pause fires on manipulation, not on markets, and can be recovered from | **Violated** (D-H-2, D-M-1) |
| S9 | The go/no-go read refuses a canister that does not serve exactly the stack's collections, with the stack's wrapper and module per collection, D5, D7 and 300 bps | **Holds** (`ethcheck`; `--canister` is required under `--phase1`) |
| S10 | The go/no-go read binds the stack's custodian (the poster and wrapper custodian) to the canister | **Violated** (D-L-6) |
| S11 | Every Trust-page statement is true of the code | **Violated** (D-L-8) |
| S12 | The keeper never posts an NFT module's floor, and the resolver reads EthNftWrapper's tickets | **Holds** (`keeper.ts:140-147`; `NFT_WRAPPER` matches `EthNftWrapper.Ticket` field for field; the resolve selector is the same) |

## 4. Findings

| ID | Severity | Title | Where (file:line @57160b0) | Reachable in the launch config? | PoC |
| --- | --- | --- | --- | --- | --- |
| D-H-1 | **High** | The honest runner's "sales" are the latest 12 **transfer** transactions of one day. Measured on mainnet, that leaves every launch collection with no floor post for hours a day (Milady 45 of 73 twenty-minute points, runs up to 11.7 h). Each module goes stale: mints and burns are refused, the canister's buy bound errors, and the reserves page turns red | `ops/src/ethnft.ts:196-205,286`; `ethnft.rs:528-530` (`min_sales`) | Yes, with no attacker | live measurement + `ops` PoC, **pass** |
| D-H-2 | **High** | The canister posts its raw reading, with no `floorBounds()` clamp. The monitor's keeper-era rule calls every clamped post a stolen key (SEV1), and two clamped posts in the same direction within 6 h make the D8-armed guardian pause the **whole registry**. An honest ≥ ~5 %/h move does it, and so does a cheap push (two cheap listings, or B-H-1) | `ethflow.rs:1424-1425`; `monitor/src/evm.mjs:264-280,772-790`; `config.mjs:127-130` | Yes (D8 arms the guardian) | `monitor` PoC, **pass** |
| D-M-1 | Medium | The D8 floor pause is registry-wide and fires on any real > 20 % move, which the module's own band guarantees it lags. It re-fires within a poll of every unpause (so it is sticky), pages a new SEV1 key every poll, and in practice rests on one keyless reference (CoinGecko 429s). With both references down the floor is unwatched, silently | `monitor/src/ethnft.mjs:99-110,166-168,183-186` | Yes | `monitor` PoC ×3, **pass**; live 429 measured |
| D-M-2 | Medium | The 4663 canary is blind to EthNftWrapper's custodian-gated `Wrapped`/`Deposited`/`Delivered` (other signatures). It judges its `Undeliverable` against the Bitcoin redemptions, so every honest eth `markUndeliverable` (which anyone causes with `redeemToRelist`) is a false SEV1 | `monitor/src/evm.mjs:20-30,80-106`; `records.mjs` (no eth delivery index) | Yes | `monitor` PoC ×2, **pass** |
| D-M-3 | Medium | The design's Ethereum outbound canary was not built. Custody is read only for items the wrapper holds, and the gas pot only once one is held. An NFT that is bought but not yet minted, held, a deposit or a stray, and the ETH of an arrived buy, can leave the canister's address unpaged | `monitor/src/ethnft.mjs:148-163` (vs `phase1-nft.md` §6) | Yes | `monitor` PoC, **pass** |
| D-M-4 | Medium | `eth_listing` always answers `Ok` (held for the tick), so the runner stops at the first listing. A cheapest listing the canister refuses is handed in again every pass, and cheaper-to-fill ones are never tried. The buy sits `Arrived` (all buys are blocked, B-M-5) until the 24 h page | `ops/src/ethnft.ts:259-274` (`break` at `:270`); `ethflow.rs:1596-1608` | Yes | `ops` PoC, **pass** |
| D-L-1 | Low | No timeout or size cap on any of the runner's outbound calls: one stalled OpenSea, Relay or Ethereum-RPC answer stops every tick. One thrown `eth_deposit` aborts the later collections' claims | `ops/src/ethnft.ts:173-194,248,304`; `runner.ts:151-153` | Yes (provider behaviour) | `ops` PoC ×2, **pass** |
| D-L-2 | Low | The Ethereum step starts half-configured (no `ETH_LOGS_RPC`, OpenSea key or slugs) and says so only in log lines. Then no sale is ever handed in and no deposit is ever discovered | `ops/src/runner.ts:52-59`; `ethnft.ts:282-283,294` | Yes (a config slip) | `ops` PoC, **pass** |
| D-L-3 | Low | Discovery and its watchdog share one 7,200-block window. An inbound NFT not judged within a day falls out of both: the page stops, and nothing mints or returns it | `ops/src/ethnft.ts:26,208-221`; `monitor/src/ethnft.mjs:20-21,171-177` | Yes (a runner outage over a day) | `monitor` PoC, **pass** |
| D-L-4 | Low | The monitor compares a provider **error** as if it were an owner: one flaky provider pages a new SEV2 key per poll per token, and two alike-failing providers give a false SEV1 "left custody" | `monitor/src/ethnft.mjs:155-159` | Yes (B-M-6 measured such answers) | `monitor` PoC, **pass** |
| D-L-5 | Low | One collection's failed read throws the whole ethnft checker: later collections' custody, floor and D8 pause are skipped for that poll | `monitor/src/ethnft.mjs:148-168` | Yes | `monitor` PoC, **pass** |
| D-L-6 | Low | Go/no-go gaps: nothing binds the stack's custodian (poster, minter and NFT holder) to the canister's key; `eth_nft.chain_id`, `max_purchase_wei` and `max_premium_bps` are not read; and the canister never checks `EthNftWrapper.custodian()` (`en::CUSTODIAN` is unused) | `script/golive-check.sh:454-487,595-606`; `ethnft.rs:604` | Deploy mistake | by inspection |
| D-L-7 | Low | `checkNftSet` does not check slugs, and the stack file is keyed by slug. A repeated slug silently drops a deployed, treasury-registered collection from the file, and so from golive-check, the keeper, the monitor and `forCanister` | `script/StackChecks.sol:84-92`; `DeployStack.s.sol:237,374,409` | Deploy mistake | by inspection |
| D-L-8 | Low | Trust-page statements that are not true of the code: that the runner only withholds and that this is bounded at +10 % over recent sales; "plus 5 %" (unchecked config); strays "go back" (some are held) | `web-rook/src/rook/phase1Trust.ts:136,143-144` | Yes | by inspection (B-H-1's PoC shows the first) |
| D-I-1 | Info | The runner does not pre-check the bound (the design says it does), and a malicious runner can hand in its own listing at the cap | `ops/src/ethnft.ts:259-274` | — | — |
| D-I-2 | Info | Monitor coverage and wording: several buy/deposit phases are unwatched; NFT-module pages say "the keeper"; `eth_nft.guardian_pause=false` does not stop the pause | `monitor/src/ethnft.mjs:32-77,183`; `evm.mjs:266-275` | — | — |
| D-I-3 | Info | The reconciler's E-05 "escrow per unit ≥ 50 % of the floor" counts the unit in transit (no escrow behind it). At launch numbers it is RED during the third advance once the floor is up 20 % | `ops/src/backing.ts:86-88` | Conditional | `ops` PoC, **pass** |
| D-I-4 | Info | The OpenSea key is not in the logger's scrub list, and `fetch` follows redirects with the custom `x-api-key` header. `argdiff` rates floor-rule and premium loosening only WARN | `ops/src/log.ts:19`; `ethnft.ts:176`; `monitor/src/argdiff.mjs:96-117` | — | — |
| D-I-5 | Info | Unbounded growth: the runner's `claimed` map and response bodies; the monitor's persistent keys per deposit | `ops/src/ethnft.ts:170,297`; `monitor/src/alerts.mjs:43` | — | — |

**Counts:** 0 Critical, 2 High, 4 Medium, 8 Low, 5 Info.

**Overlap with reviewer B** (cross-referenced, not double-counted):
- B-H-1 (anyone steers the floor through the sales) and B-M-1 (a crowded or oversized receipt voids the pass) are the canister
  half of D-H-1's runner-side choice;
- B-M-5 (one `Arrived` buy blocks every collection) is what D-M-4 feeds;
- B-M-6 (1rpc's varying answers) is the provider behaviour behind D-L-4.

### D-H-1 (High): the honest runner leaves every launch collection without a floor post for hours a day

**Description.**
- `recentTransferTxs` (`ops/src/ethnft.ts:196-205`) makes one `eth_getLogs` for every `Transfer` of the collection over the last
  7,200 blocks and keeps the newest 12 distinct transaction hashes. `ethNftOnce` hands those in as the "sales" (`:286`).
- The canister keeps only receipts with a Seaport 1.5/1.6 `OrderFulfilled` or a Blur v2 `Execution721*Packed` of one token of the
  collection, and needs `min_sales` = 3 of them (D7; `ethnft.rs:528-530`). Otherwise there is no reading and no post (`ethflow.rs:
  589-590`).
- Wallet moves, mints, delegations, bulk moves and the canister's own fills are transfers but not sales. So whether a post
  happens depends on how many of the last 12 moves were sales, not on the market.
- The canister accepts sales from **7 days** (`SALE_WINDOW_BLOCKS` 50,400). The runner never looks past one day and never
  filters for sales.

**Measured (read-only, 2026-09-27 16:52 UTC, head 26,070,099).**

`ops/scripts/audit_nft_d_window.ts`:
- **Method:**
  - reads two days of each launch collection's `Transfer` logs and every receipt;
  - marks each transaction that carries a sale of that collection. This is an **upper bound**: bundles are counted, and dust
    is not removed;
  - then, at every 100-block (≈ 20 min, the runner's cadence) point of the last day, applies the runner's own rule: the newest
    12 transfer transactions within 7,200 blocks.
- **Results:**

  | Collection | Transfer txs / 2 days | Sale txs | Points (of 73) with < 3 sales, i.e. no post | Longest run with no post |
  | --- | --- | --- | --- | --- |
  | Milady | 54 | 11 | **45** | **700 min** |
  | Azuki | 65 | 18 | 18 | 360 min |
  | MAYC | 217 | 52 | 26 | 520 min |

  (A first run 15 minutes earlier gave 47 / 18 / 27 points and 720 / 360 / 540 minutes.)
- **Consistency check:** the design's own sample (§1.1: 14 sales in Milady's 60 latest transfer transactions, 23 %) predicts
  under 3 sales in 12 about half the time.

**Impact.**

`FLOOR_TTL` is 1 h. After an hour with no post:
- the module is not `fresh()`: `mint` reverts `Stale`, and `burn` reverts `Stale` (`FloorModule.sol:503,535`) until the 7-day
  escape or a pause;
- `_tryFill` declines graduations;
- the canister's reading is over `READING_TTL_SECS`, so `buy_cap` errors and an `Arrived` buy waits (and blocks all collections,
  B-M-5);
- the reconciler turns RED after 1 h + TTL (`backing.ts:97-98`), which is the go/no-go "reserves page green";
- the monitor pages `fresh()==false` and `no good post` SEV2, persistently, for every collection, every day;
- if the operator sets `guardian.pause_on_no_good_post_h`, the guardian pauses the registry (`evm.mjs:755-766`).

No attacker is needed. With one (B-M-1: 12 plain transfers crowd out every sale, for gas only), any collection can be held stale
at will.

**PoC.**
- `ops/scripts/audit_nft_d_window.ts`: the live measurement above. **Passes**, and the output is recorded.
- `ops/test/audit_nft_d.test.ts` "D-H-1": three real sales two hours back and 12 plain transfers after them. The runner hands in
  the 12 transfers (0 sales), from one un-chunked 7,200-block query, and never the 7-day window the canister accepts. **Passes.**

**Recommended fix.**
1. Hand in **sales**, not transfers:
   - query `OrderFulfilled` (Seaport 1.5/1.6) and Blur v2 `Execution721*` logs whose data names the collection, or read receipts and
     keep only sales;
   - over up to the canister's 7 days, newest first, until 12 sales;
   - paging the range as `inboundTransferTxs` does.
2. Say in the runner's `/healthz` when a collection has had no accepted evidence for 40 minutes.
3. Keep B's canister-side fixes (B-H-1, B-M-1): the runner's choice must not be what bounds the floor.

### D-H-2 (High): an honest floor move pauses the whole registry through the D8-armed guardian

**Description.**
- The canister posts `postFloor(r.post.price, …)` with its raw reading (`ethflow.rs:1424-1425`). Nothing clamps it into
  `floorBounds()`; the module clamps it (`FloorModule.sol:393-395`).
- It posts whenever the module's post is 30 minutes old, or the reading is 1 % from `target` (`ethnft.rs:545-551`). So a reading
  that stays out of band is posted again every 30 minutes.
- The 4663 canary's FloorPosted rule, written for the keeper EOA (`evm.mjs:264-280`), says:
  - `target` differs from `accepted` by more than 50 bps: SEV1 "THE KEEPER POSTED A … FLOOR OUTSIDE THE MODULE'S BOUNDS … a stolen
    key walking the floor";
  - two such posts in the same direction within 6 h: a `runaway` trigger with `action: "pause"`.
- The guardian then calls `pauseKeeper()` on the **registry** (`evm.mjs:772-790`). This is not gated by `pause_on_floor_walk`.
- D8 makes the guardian mandatory and armed for an `eth_nft` monitor (`config.mjs:127-130`).
- `fixes-W2-monitor.md` "Left open" item 2 set out this precondition: "The runaway rule reads the keeper's intent from the clamp …
  If the service ever posts raw targets, every out-of-band move would page SEV1, and two would pause." The canister is that
  service.

**When it fires.**

The band after 30 minutes is ±2.5 %. A reading that moves ≥ ~5 % within an hour produces two clamped posts. Three ways:
- **The thin books D6 measured (Milady: 2 listings within +2 %, 3 within +5 %).** One sale of the cheapest listings, or the
  canister's own buy (reviewer B, Q2), moves the rank-2 listing floor by 5–10 %.
- **An ordinary market hour.**
- **Anyone, cheaply:**
  - two listings 6 % under the floor, from two wallets (rank 2 of distinct offerers). The cost is the discount if they are sniped;
  - or B-H-1's self-trades, for gas only.

**Impact.** Every collection's poster is zeroed (PosterSource), and the keeper stops. Mints stop, and the modules go stale.
Burns go on at the pause floor. The Safe (2 of 2) must unpause.

Each honest move also pages SEV1 "stolen key" for a key that cannot post these modules at all: the keeper EOA is refused by
PosterSource.

**PoC.** `monitor/test/audit_nft_d.test.mjs` "D-H-2":
- the NFT-only stack, accepted 0.93 ETH, and a reading of 1.00 ETH;
- two canister posts 30 minutes apart, accepted 0.953 then 0.977;
- result: SEV1 "THE KEEPER POSTED … a stolen key", then `triggers = [{ action: "pause", why: "a runaway milady floor: 2 posts
  outside the module's bounds, all up" }]`. **Passes.**

**Recommended fix.**
- The canister should post `clamp(P, floorBounds())` (read at the 4663 pin), as the keeper does (`fixes-W2-services.md` §3). That
  restores the rule's premise.
- Or the monitor should skip the clamp and runaway rules for modules whose registry is a `PosterSource`, and judge those posts
  against the canister's `readings` (the design's "FloorPosted by the poster matched to a canister reading", §6, which was not
  built).
- In either case, a pause on one NFT module's floor should not be the registry-wide keeper pause (see D-M-1).

### D-M-1 (Medium): the D8 floor pause is registry-wide, fires on real moves, is sticky, floods SEV1, and rests on one keyless reference

**Description** (`monitor/src/ethnft.mjs`).

1. **It fires on real moves.** `floorAlerts` pauses when `accepted` is more than 20 % from every available reference (`:99-110`).
   The module can move at most 5 %/h and 20 %/day (`floorBounds`). So a real fall of more than 20 % in a day always opens a gap of
   more than 20 %: the pause fires on a crash, not only on manipulation.
2. **It is sticky.** After the Safe unpauses, `accepted` can close at most ≈ 5 % an hour (and not before the canister's next
   post). The next 60-second poll sees a gap still over 20 % and pauses again (`:183-186`). Recovery needs the market to return, or
   the monitor to be reconfigured, and `eth_nft.guardian_pause = false` is documented as "only for a rehearsal".
3. **It floods.** The pause alert's key is `eth-floor-pause:${module}:${nowS}` (`:185`), which is new every poll. That is a SEV1
   message every minute for as long as the gap lasts, plus `pauseKeeper()` re-called every poll.
4. **In practice, one keyless reference.**
   - CoinGecko's keyless API answered **429 from the 6th call** in a few seconds (measured).
   - The monitor makes one CoinGecko call per collection per poll (3 a minute).
   - A failed reference is skipped silently (`:166-167`, "a missing reference is not an alert"), so the "both references" rule
     becomes OpenSea's `floor_price` alone.
   - With both references down, `floorAlerts` returns nothing at all (`:101`). For example, a module at 5× the market pages
     nothing.
5. **It is not verified.** `opensea_slug` and `coingecko_id` are free text. A swapped slug (Azuki's 0.66 against Milady's 0.93)
   pauses at start.
6. **It is registry-wide.** One collection's gap pauses all three (D1).

**PoC.** `monitor/test/audit_nft_d.test.mjs`, three "D-M-1" tests:
- five polls after a 21.5 % drop give five `pauseKeeper()` calls and five distinct SEV1 keys;
- after an unpause with `accepted` 5 % closer, it pauses again at once;
- CoinGecko 429 plus OpenSea 30 % off pauses; both down gives 0 alerts.

**All pass.**

**Recommended fix.**
- Pause only on a gap that the module's own band cannot explain: for example, `accepted` moving *away* from the references
  across two readings, or the canister's reading (not `accepted`) more than 20 % off.
- Make the pause alert persistent with a stable key, and do not re-pause while the Safe's unpause is recent.
- Page when a reference is missing for more than 30 minutes, and when both are.
- Use CoinGecko with a key or a lower cadence.
- Check each slug against the collection's contract (`collections/{slug}` returns it).
- Consider a per-collection stop (a PosterSource-level pause) instead of the registry's.

### D-M-2 (Medium): the 4663 canary cannot read EthNftWrapper's custodian-gated events, and misreads one

**Description.**
- The EVM canary watches every wrapper in the stack file (`stack.mjs:34,55`), which includes each EthNftWrapper. But it matches
  topics from `SIGS` (`evm.mjs:20-30`), which are the Bitcoin `Wrapper`'s signatures.
- EthNftWrapper emits different ones (`EthNftWrapper.sol:126-133`):
  - `Wrapped(uint256,uint256,bytes32,address)`;
  - `Deposited(uint256,uint256,bytes32,address)`;
  - `Delivered(uint256,uint256,address,bytes32)`.
- None of these reaches a rule. So an unbacked `mint` / `mintDeposit` and a false `recordDelivery` (a made-up hash, so the ticket
  is "delivered" while the NFT stays with us) are not matched against any canister record. After a false record, the token also
  drops out of the custody check (it is no longer `itemOf`), so nothing watches it.
- `Undeliverable(uint256)` has the same signature in both wrappers. It is judged against `index.attests`, which is built from the
  **Bitcoin** redemptions (`evm.mjs:95-106`); `records.mjs` indexes no eth delivery. So every honest eth `markUndeliverable` is
  SEV1 "AN ATTESTATION NO REFUSAL BACKS … a LostRace", with the pause advice.
- The canister marks a relist ticket at once (`ethflow.rs:1002`), and `redeemToRelist()` is open to any holder
  (`EthNftWrapper.sol:320`). So anyone can raise that SEV1 at will, for one unit locked until the refund (deliverBy + 72 h).
  B-M-2's false marks raise it too.

**PoC.** `monitor/test/audit_nft_d.test.mjs`, two "D-M-2" tests:
- a marked eth delivery plus its `Undeliverable` gives SEV1;
- eth-shaped `Wrapped`, `Deposited` and `Delivered` with no record give zero alerts, and none of their topics is in `SIGS`.

**Both pass.**

**Recommended fix.**
- Add EthNftWrapper's signatures to the canary.
- Index `eth_nft_state().buys` (mint: token, purchase tx, module), `deposits` (token, tx, recipient) and `deliveries` (ticket,
  wrapper, tx, marked) in `records.mjs`.
- Match `Wrapped`, `Deposited`, `Delivered` and `Undeliverable` from an NFT wrapper against those, keyed by wrapper and ticket.

### D-M-3 (Medium): the Ethereum outbound canary was not built; custody is watched only for wrapped items

**Description.**
- `phase1-nft.md` §6 promises an "Ethereum outbound canary: every transaction from our Ethereum address must be a saved line
  entry". `checkEthNft` (`ethnft.mjs:148-163`) instead does two things:
  - it reads `ownerOf` only for items `EthNftWrapper.item(i)` still holds;
  - it reads the address's balance only to page "gas pot low", and only when `held > 0`.
- **Unwatched:**
  - an NFT bought and final but not yet minted (`Bought`/`Final`/`Minting`: up to the day's mint limit, and indefinitely under
    B-H-2);
  - a deposit `Seen`/`Mint`, a `Held` stray, or a stray waiting to be returned;
  - the ETH of an `Arrived` buy (up to `max_purchase_wei`, waiting up to days under D4);
  - any transaction from the canister's address that is not a line entry. A bug, a malicious upgrade, or B-I-1's Seaport listing
    signed on the eth chain and then filled would not be matched to anything.
- The Bitcoin side's canary pages SEV1 on any unexplained custody outbound (the E-canary model).

**PoC.** `monitor/test/audit_nft_d.test.mjs` "D-M-3":
- a `Final` buy whose token is owned by a stranger;
- an `Arrived` buy with the address's balance at 0;
- the result is no alert of any kind. **Passes.**

**Recommended fix.**
- Poll the custodian's nonce on both providers. Every nonce must be a saved `EthTx` (`line`) or a known past one (keep a history of
  hashes).
- Read `ownerOf` for every token in `buys` (from `Buying` on), in `deposits` (`Seen` to `Minted`, `Held`, `Return*`) and in
  wrapped items.
- Compare the balance with the arrived buys' `wei`.

### D-M-4 (Medium): the runner re-hands a refused cheapest listing for ever

**Description.**
- For an `Arrived` buy the runner fetches OpenSea's 5 best listings and, for each, the fulfillment data. It calls `eth_listing` and
  `break`s at the first `Ok` (`ops/src/ethnft.ts:259-274`).
- `put_listing` answers `Ok("listing held for buy …; judged at the next tick")` for any well-formed order (`ethflow.rs:1596-1608`).
  The judgment happens later, in the tick, and reaches the runner only as a tick line or the buy's `note`, which the runner never
  reads.
- So the runner always hands in exactly OpenSea's cheapest listing, and nothing else. If the canister refuses it, the next pass
  hands in the same one:
  - a listing that pays an optional creator fee (Milady has no ERC-2981 receiver, so the buy judge refuses the recipient; B-L-1
    notes the same for evidence);
  - a restricted zone that is not allow-listed;
  - a listing that is stale on OpenSea (approval revoked, the item moved);
  - a price above the cap while a cheaper-to-fill one is under it.
- The buy waits `Arrived` (all collections' buys wait: B-M-5) until that listing disappears, or the 24 h page. Any seller at the
  floor can cause this, knowingly or not.

**PoC.** `ops/test/audit_nft_d.test.ts` "D-M-4": three listings; the canister's `put_listing` behaviour; the buy's note says the
listing was refused. Over three passes the runner hands in token 1, token 1, token 1. **Passes.**

**Recommended fix.**
- Read the buy's `note` (or keep a per-buy set of refused order hashes) and move to the next listing.
- Pre-check what the runner can see (price against `readings` × premium, consideration recipients against the chain's fee list,
  zone), as §6 already says it does.

### D-L-1 (Low): no timeout on the runner's outbound calls; one throw aborts later collections

- Every `fetch` in `ethnft.ts` (`os`, `ethCall`, the Relay quote at `:248`) has no `AbortSignal`, no deadline and no size cap
  (`res.text()`, `r.json()`). Undici's defaults allow 300 s for headers and 300 s *between* body chunks, so a trickling body never
  ends.
- The steps run sequentially inside `Runner.cycle()`, and `main()` awaits it before the next tick (`runner.ts:151-153`).
- So one stalled provider (OpenSea, Relay, or the public `ETH_LOGS_RPC`) stops **every tick**:
  - every collection is affected;
  - so are deliveries, deposits and the frog side;
  - only `/healthz`, 30 minutes later, notices.
- Separately, `d.buyer.eth_deposit(tx)` is outside any `try` (`:304`). A rejected update (for example, the canister is mid-tick)
  rejects the whole `ethNftOnce`, and the later collections' claims wait for the next cycle.
- The indexer's R4 fix (M-E, `guardedFetch`) is the model.

**PoC.** `ops/test/audit_nft_d.test.ts`, two "D-L-1" tests:
- a never-answering fetch: the cycle has not returned after 300 ms, `tick` was called once, and `healthy()` turns false at 3
  intervals;
- a throwing `eth_deposit` for Milady: Azuki's claim is never handed in.

**Both pass.**

**Fix:**
- give every outbound call a deadline (10–20 s) and a byte cap, with `redirect: "manual"`;
- run each collection's work under `try`;
- tick on a timer that does not wait for the Ethereum step.

### D-L-2 (Low): the Ethereum step starts half-configured

- `parseRunnerConfig` (`runner.ts:52-59`) defaults `RUNNER_ETH_NFT` to on. It requires none of `ETH_LOGS_RPC`, `ETH_NFT_SLUGS` or
  `OPENSEA_API_KEY`.
- Without `ETH_LOGS_RPC`:
  - `recentTransferTxs` returns `[]`, so evidence is handed in with 0 sales and the canister never posts;
  - `cfg.deposits && d.ethRpc` is false, so with D5 on, no deposit is ever discovered (`:294`).
- A collection the canister serves but `ETH_NFT_SLUGS` does not name gets one `eth-no-slug` log line per pass (`:282-283`).
- A missing OpenSea key is a warning per call.
- None of this shows in `/healthz` or `/status`, which report only ticks. It surfaces hours later as stale floors and unclaimed
  pages.
- `services-v1.md:210` documents the consequence; the code should refuse it.

**PoC:** `ops/test/audit_nft_d.test.ts` "D-L-2" (the config is accepted; the evidence has 0 sales; 0 deposits are handed in;
nothing is handed in for a slug-less collection). **Passes.**

**Fix:**
- with the eth step on, refuse to start without `ETH_LOGS_RPC`, `OPENSEA_API_KEY`, and a slug for every collection of
  `eth_nft_config()` (checked at the first pass);
- report the eth step's last success per collection in `/healthz`.

### D-L-3 (Low): an inbound NFT not judged within a day is forgotten by both discovery and its watchdog

- The runner looks back `DEPOSIT_LOOKBACK_BLOCKS` = 7,200 (`ops/src/ethnft.ts:26,213`). The monitor's `unclaimedAlerts` looks back
  the same `DEPOSIT_SCAN_BLOCKS` = 7,200 (`monitor/src/ethnft.mjs:21,175-176`).
- Suppose the runner is down for more than a day, or `ETH_LOGS_RPC` is unset or refusing (D-L2). Then an NFT sent in that window:
  - is paged every 6 h while it is inside the window;
  - then silently stops being paged;
  - is never handed in again.
- It sits at the canister's address. It is not minted, not returned, and not listed anywhere automated. The holder's recourse is a
  manual `eth_deposit` by the operator, who was told only while the page ran.
- (Persistent alerts have no "resolved" message, so the stop looks like nothing happened.)

**PoC:** `monitor/test/audit_nft_d.test.mjs` "D-L-3" (700 blocks old: paged; 7,300 blocks old and still unjudged: silent).
**Passes.**

**Fix:**
- give the monitor a saved cursor from the stack's deploy block (not a sliding window);
- page an unjudged inbound transfer until the canister's `deposits` names it;
- have the runner scan from a saved cursor too.

### D-L-4 (Low): provider errors are compared as owners (a flood, and a false SEV1)

**Description.** In the custody loop (`ethnft.mjs:155-159`) a failed `ownerOf` becomes the string `error: …`. That string then
takes part in the comparison:
- **One provider erroring** (B-M-6 measured 1rpc's rate-limit and "discontinued" answers) is judged a *disagreement*. The alert
  key contains the block, so each poll pages a new SEV2 per held token: up to 9 tokens × 60 an hour.
- **Two providers failing with the same text** (for example `header not found` at a block one of them has not reached) agree
  with each other. That is judged "owned by error: …, not the custodian": a persistent SEV1 "An NFT left custody … pause".

**PoC.** `monitor/test/audit_nft_d.test.mjs` "D-L-4": three polls give three distinct SEV2 keys, and identical errors give SEV1
`eth-not-held`. **Passes.**

**Fix:**
- a provider error is "unread": retry, and page once (a persistent key without the block) after N failed polls;
- compare only real addresses.

### D-L-5 (Low): one collection's failed read throws the whole ethnft checker

`checkEthNft` loops over the collections with bare `await`s (`ethnft.mjs:148-168`). One busy 4663 read on the first collection's
wrapper rejects the checker. Then the later collections' custody check, floor comparison and **D8 pause** do not run that poll.
The generic `checker-down` pages after 30 minutes. An attacker who can make one provider refuse one call delays the pause.

**PoC:** `monitor/test/audit_nft_d.test.mjs` "D-L-5" (Milady's `itemCount` answers -32005; Azuki is 57 % off; the checker
rejects; 0 pauses). **Passes.**

**Fix:** isolate each collection in its own `try`, emit a per-collection read-failure alert, and always run the floor
comparison and the pause.

### D-L-6 (Low): go/no-go gaps for the NFT stack

The go/no-go read does catch several mis-installs (§5). It does not catch these:

1. **The stack's custodian is never tied to the canister.**
   - `CUST` comes from the stack file. It is compared with each wrapper's `custodian()`, each PosterSource's `poster()` and the
     stack's `floorPoster` (`golive-check.sh:595-606`), which are all set from the same `ETCH_WRAPPER_CUSTODIAN`.
   - Nothing compares it with the canister's EVM address, even with `--canister`: `evm_address()` is operator-only
     (`canister.rs:603-604`) and is not read.
   - The canister itself never checks `EthNftWrapper.custodian()`: `ethnft::CUSTODIAN` (`ethnft.rs:604`) is defined and unused. It
     checks only `module.keeper()` before posting.
   - So a stack deployed with the wrong custodian passes the go/no-go read. For example, a staging canister's address, or a
     `test_key_1` derivation. That address mints on every EthNftWrapper and posts every NFT module's floor.
   - The same gap exists for the Bitcoin stack. On the NFT side the address also holds the floor role.
2. **Fields not read:**
   - `eth_nft.chain_id`: the canister could serve another EVM chain, with the same collection address;
   - each collection's `max_purchase_wei` and `max_premium_bps`: the Trust page states "plus 5 %", and `ethcheck` does not hold
     that.

**Fix:**
- add `--canister-evm <address>`, which the operator reads with its identity, and FAIL unless it equals `CUST`;
- or expose the address in a public query;
- have the canister refuse a collection whose wrapper's `custodian()` is not itself (at install and every tick);
- have `ethcheck` compare the chain id, and hold max purchase ≤ the stage's per-item cap and premium = the launch decision.

### D-L-7 (Low): a repeated slug drops a deployed collection from the stack file

- `StackChecks.checkNftSet(cols, names.length, symbols.length, slugs.length)` (`StackChecks.sol:84-92`) checks the list lengths
  and duplicate collections. It never reads the slugs.
- `_collectionJson` serialises each collection under `"col-" + slug`, and into `"cols"` under `slug` (`DeployStack.s.sol:374,409`).
  So `ETCH_NFT_SLUGS=milady,milady,mayc` writes two collections.
- The overwritten one is still deployed:
  - its wrapper, PosterSource, module, factory and vaults exist;
  - the owner steps register it in the treasury and directory (`_readBackNftOwnerSteps` checks index `i+1`, and passes).
- But it is missing from the stack file, and so from:
  - golive-check;
  - the keeper;
  - the monitor's watched addresses;
  - `forCanister`, and therefore the canister's `eth_nft` list.
- Its `CurveFactory` is permissionless, and treasury `distribute` still funds its module.

**Fix:** refuse duplicate (and empty, and `^[a-z0-9-]+$`-failing) slugs in `checkNftSet`, including the Bitcoin collection's slug
when both are deployed.

### D-L-8 (Low): Trust-page statements that are not true of the code

`web-rook/src/rook/phase1Trust.ts`:

- **`:143`, "the operator's runner … could withhold cheap ones, which lifts the posted floor at most 10 % over recent sales".**
  - The runner also chooses the sale transactions, from 7 days of any receipts. So does anyone who makes the latest transfers
    (B-H-1).
  - The lower clamp `S25 × 0.85` lifts the post above every listing.
  - The only real bounds are the module's ±5 %/h and ±20 %/day, and D8's 20 %.
  - B-H-1's PoC posts 1.275 ETH against listings at 0.92–0.95.
- **`:136`, "at most its own floor reading plus 5 %".** The premium is the install argument's `max_premium_bps`. The canister
  allows up to 1,000, and golive-check does not hold it (D-L-6).
- **`:144`, "An NFT of the collection sent any other way goes back to the address it came from".** Strays are **held**, not
  returned, in these cases (§2.4, `judge_return`):
  - the simulation refuses;
  - an open buy or claim uses the token;
  - more than 4 came in one transaction;
  - it left us after the transfer came in.

  "The chain 4663" does not say it is a 4-byte big-endian integer, and a holder who encodes it otherwise gets the NFT back.

**Fix:** state that the floor is bounded by the module's hourly and daily limits and by the monitor's 20 % pause, not by sales;
name the premium bound the install holds; list the held cases and the exact 48-byte layout.

### Informational

- **D-I-1: the runner's choice of listing.**
  - The design (§6) says the runner "pre-checks the bound". It does not: it hands in OpenSea's cheapest.
  - A malicious runner (or its operator) can hand in *its own* listing at the cap. The canister pays up to `floor × (1 +
    premium)`, and with B-H-1 up to about 1.22 × the market.
  - This is the documented residual ("it can pick among valid listings under the bound").
- **D-I-2: monitor coverage and wording.**
  - `ethNftAlerts` does not watch these buy phases: `Depositing`, `Final`/`Minting`/`Minted` (B-H-2 can hold a `Final` mint),
    `Settling`, `Returning`, `NeedAdvance`/`Advancing`. Nor does it watch deposit `Minting`.
  - The gas-pot page needs `held > 0`, but at launch returns of strays need the pot with nothing held.
  - The chain checker's and canary's texts for NFT modules say "the keeper" (a fresh()==false page points at the keeper, not the
    runner or the canister).
  - `checkEthNft` pauses even when `eth_nft.guardian_pause === false` (`:183` reads only `guardian.autoPause`).
- **D-I-3: the transit unit and the E-05 rule.** `backingStatus`'s "escrow per ETH-backed unit ≥ 50 % of the floor"
  (`backing.ts:86-88`) divides by `ethBacked`, which includes the transit unit. That unit's backing already left with the advance
  (`FloorModule.advance`, `backing -= released`).
  - At launch numbers (R 1.1 units, 0.3 convertible left, 1 in transit), it is RED once the floor is up 20 % since graduation.
  - Leaving the transit unit out, it is green.
  - PoC: `ops/test/audit_nft_d.test.ts` "D-I-3", **passes**. The indexer's `ethBacked > 0 && escrow == 0` rule is similar.
- **D-I-4: key handling (defense in depth) and the argument diff.**
  - `OPENSEA_API_KEY` is sent only as a header to the fixed `api.opensea.io` host (**holds**), but it is not in `envSecrets`
    (`ops/src/log.ts:19`).
  - `fetch` follows redirects and forwards the custom `x-api-key` header cross-origin. Use `redirect: "manual"`.
  - `argdiff.mjs` rates an `eth_nft.floor` change (for example `listing_rank` 1, `min_sales` 1, `up_cap` 2,000) as WARN "CHANGES",
    and a `max_premium_bps` raise as WARN. Both loosen the floor or the buy cap and could be CRITICAL LOOSENS.
  - The per-collection wrapper/module comparison is case-sensitive (a checksum-case change reads CRITICAL; louder, safe).
- **D-I-5: unbounded growth.**
  - the runner's `claimed` map (in memory);
  - unbounded response bodies;
  - the monitor's persistent keys, which are never forgotten (`alerts.mjs:43`), one per deposit and per stray.

## 5. What held up (checked, no finding)

- **The OpenSea key:**
  - read at call time, only in `os()`;
  - never in a URL, error text or log field;
  - never handed to the canister (the existing test asserts this).
- **The Relay pre-judge** (`preJudgeEth`) mirrors the canister's rules. `RELAY_API_URL` and `ETH_LOGS_RPC` must be https.
  Hostile JSON (a missing field, a non-numeric value, `BigInt` of garbage) is caught per buy.
- **Deposit discovery:**
  - ERC-721 only (4 topics; an ERC-20 `Transfer` to us is skipped);
  - 2,400-block chunks;
  - newest-head based (the canister waits for finality);
  - our own fills skipped;
  - re-hand-in after 7 h, beyond the canister's 6 h drop;
  - a restart only re-hands what the canister has not judged ("already claimed" is recognised);
  - reorged-out transactions are dropped by the canister unrecorded, and handed in again if re-mined.
- **The keeper:**
  - it never posts an NFT module (`plan: skip`, and `forKeeper.floorModules` excludes them);
  - its identity check requires `module.keeper()` = the stack's `floorPoster` and refuses its own key there;
  - the pause check still comes first;
  - the resolver's `NFT_WRAPPER` ticket struct equals `EthNftWrapper.Ticket`;
  - `resolve(uint256,bytes)` is the same selector;
  - refunded pinned tickets are skipped by the contract (`_skipRefunded`).
- **The indexer:**
  - `NFT_WRAPPER_EVENTS` match EthNftWrapper;
  - unresolved refunds use `itemId = max`;
  - `refundableAt` gates both wrappers;
  - custody is `ownerOf` on two providers;
  - `keeperPaused` is read through the PosterSource, which answers `keeperPaused()`/`keeper()` (A4-I3 **holds**).
- **golive-check catches:**
  - a canister whose wrapper or module is **swapped between collections** (per-collection `same`);
  - a canister serving an extra collection or missing one (exact set);
  - deposits off, a non-D7 floor, and Relay ≠ 300 bps;
  - a PosterSource not over the registry, a module whose `registry()` is not its PosterSource, and a poster ≠ custodian;
  - a collection address with code on 4663;
  - a pending mint-limit raise;
  - Pudgy/BAYC under `--phase1`.

  `--canister` is required under `--phase1`. The dry run shows each refusal.
- **The watcher's argument diff:** `eth_nft` on, a collection added, a changed wrapper/module, deposits on and a changed chain id
  are CRITICAL.

## 6. The previous audit's fixed service findings, re-checked on this branch

| Finding (2026-09-26) | Where it lives | Touched here? | Status at 57160b0 |
| --- | --- | --- | --- |
| E-01 wash sales (keeper floor) | `keeper/src/floor.ts` | No (NFT modules skip the feed) | Holds for Bitcoin. **The NFT analog is open** (B-H-1; D-H-1) |
| E-02 confidence-0 repair; FloorPosted watched | keeper `planPost`; `evm.mjs` | The keeper skips NFT; the canary rule is reused | Holds (the keeper key cannot post NFT modules). The reused rule misfires on canister posts (D-H-2) |
| E-03 keyless resolver | `resolver.ts` | Yes (NFT ticket ABI) | **Holds** |
| E-04 bounded passes, RPC failover | keeper | NFT floor skipped | Holds |
| E-05 reserves (balance, 50 % rule, stale) | `ops/src/backing.ts`, `invariant.ts` | Yes (T added) | Holds. The 50 % rule counts the transit unit (D-I-3). The stale rule now reds daily under D-H-1 |
| E-06 canary never frozen by a dispute | `evm.mjs` | Events added only | Holds. New blind spots (D-M-2) |
| E-07 per-argument send guard | `web-rook/src/rook/phase.ts` | No | Holds |
| E-08 liveness, fail-closed Telegram, reconciler pages | `main.mjs`, runner `/healthz` | New checker | Holds. The runner's eth step is invisible to `/healthz` (D-L-2); a stall is seen only as missed ticks (D-L-1) |
| E-09, E-10, E-11, E-14 | keeper | No | Holds |
| E-12 / R3 M-E outbound guard | `indexer/src/outbound.ts` | No | Holds. **Not applied to the runner's new outbound calls** (D-L-1, D-I-4) |
| E-13 two-provider location | `ops/src/location.ts` | No | Holds. The NFT custody read is two-provider (indexer, reconciler, monitor) |
| E-15 runbooks | `services-v1.md` | Yes (eth variables) | Holds |
| E-16 images non-root | Dockerfiles | No | Holds (keeper still root, as R2 left it) |
| W2 §1.4 guardian; "Left open" 2 (runaway needs planned posts) | `evm.mjs`, `config.mjs` | D8 arms it for NFT | **Regressed for NFT modules:** the premise is broken by raw canister posts (D-H-2) |
| R2-M3 / R3 L-C floor walk | `evm.mjs` `floorWalk` | No | Holds (it judges canister posts too; it pauses only with `pause_on_floor_walk`) |
| R2-L4 stale keyed on `lastGoodAt` | `backing.ts:97` | No | Holds |
| R2-S8 / L-S2 keeper alerts paged | keeper `status.ts` | No | Holds |
| R4 M-A, M-B, L-F, L-S3, L-S4 (holder index) | `holders.mjs` | No | Holds |
| L-S1 Safe creation read | `golive-check.sh` `safecheck` | No (hunks at 428-487, 531-548, 585-620, 684-716 only) | Holds |
| C-07 treasury split, C-08 golive gaps | `golive-check.sh` | Extended for NFT | C-07 holds. C-08: **new NFT gaps** (D-L-6) |
| C-09 Trust page claims | `phase1Trust.ts` | New NFT section | **New inaccurate claims** (D-L-8) |
| A4-I3 `keeperPaused` shown | indexer `floorModule.ts` | Yes | Holds (through the PosterSource) |

## 7. PoCs (`poc/D/`, run from each package in a checkout at 57160b0 after `npm ci`)

| File | Tests | Command | Result |
| --- | --- | --- | --- |
| `ops/test/audit_nft_d.test.ts` | D-H-1, D-L-1 ×2, D-L-2, D-I-3, D-M-4 | `cd ops && node --experimental-sqlite --no-warnings --import tsx --test test/audit_nft_d.test.ts` | 6/6 pass (the weakness is shown) |
| `monitor/test/audit_nft_d.test.mjs` | D-H-2, D-M-1 ×3, D-M-2 ×2, D-M-3, D-L-3, D-L-4, D-L-5 | `cd monitor && node --test test/audit_nft_d.test.mjs` | 10/10 pass |
| `ops/scripts/audit_nft_d_window.ts` | D-H-1 live (read-only) | `cd ops && npx tsx scripts/audit_nft_d_window.ts [rpc]` | output in `audit_nft_d_window.2026-09-27T1652Z.out` |
| `ops/scripts/audit_nft_d_measure.ts` | D-H-1 live, one moment | `cd ops && npx tsx scripts/audit_nft_d_measure.ts [rpc]` | at 16:31 UTC: Milady 3, Azuki 7, MAYC 4 sale txs in the 12 handed in |

**Suites with the PoCs:** ops 51/51, monitor 163/163. ops `tsc --noEmit` is clean.

**Measured, read-only:**
- CoinGecko keyless `nfts/milady-maker`: 200 ×5, then **429 ×3**, within a few seconds;
- OpenSea keyless `collections/milady/stats`: 200 ×3, `floor_price` 0.92999799.

## 8. Test gaps

- **ops:**
  - no test that the evidence carries sales (D-H-1);
  - no hung or slow provider (D-L-1);
  - no half-configured eth step (D-L-2);
  - no refused listing across passes (D-M-4);
  - no reconciliation with a transit unit (D-I-3).
- **monitor:**
  - no NFT-stack FloorPosted from the canister (D-H-2);
  - no repeated polls with the D8 gap, and no missing references (D-M-1);
  - no EthNftWrapper events on the canary (D-M-2);
  - no custody of non-wrapped tokens or ETH (D-M-3);
  - no provider errors in the custody loop (D-L-4);
  - no per-collection isolation (D-L-5).
- **golive:**
  - no canister whose `chain_id`, `max_premium_bps` or `max_purchase_wei` differs;
  - no custodian that is not the canister (D-L-6);
  - no duplicate slug (D-L-7).
- **End to end:** no run in which the real runner feeds a real canister's floor on a mainnet fork. The dry run posts the floor as
  the canister by hand (`cast send … postFloor`).

## 9. Not covered

- The canister's judges beyond confirming their bounds (reviewer B); the contracts (reviewer A).
- Live OpenSea keyed endpoints (no key); Relay live quotes; a replica or mainnet run of the runner against a canister.
- `web/` (the legacy site), and `staging/` beyond its installation for the keeper.
- The Railway, DNS and operator-device configuration.
