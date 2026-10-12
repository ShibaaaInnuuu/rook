# Round 2 re-review, area D: services, deploy and the launch configuration (R2-services)

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `27c0962`
(round-1 fixes on `8c107e9`, which carries the `nft-pool` delta `09a680a..8c107e9`). Read: `D-services-deploy.md`, `fixes-round1.md`,
`docs/v7/phase1-nft-pooled.md`, and the code at HEAD: `ops/src/{ethnft,runner,backing,log}.ts`, `monitor/src/{ethnft,evm,guardian,
config,candid,records,stack,ic,main,alerts}.mjs`, `web-rook/src/rook/{phase1Trust,pool}.ts`, `indexer/src/wrapped/{service,floorModule}.ts`,
`script/{DeployStack.s.sol,StackChecks.sol,golive-check.sh}`, `scripts/golive-dryrun.sh` (NFT phase), `deployments/launch/nft-pilot.json`;
the canister (`ethflow.rs`, `ethnft.rs`) only where a service relies on it.

**Nothing in the code was modified. Nothing was committed, pushed, deployed or signed. No network call was made.** forge, cargo,
docker and the dry run were left to the lead. PoCs (node tests against each package's own fakes) were run in the tree, then moved
to `poc/D/` with their logs: `poc/D/monitor/test/audit2_nft_d.test.mjs` (3), `poc/D/ops/test/audit2_nft_d.test.ts` (2),
`poc/D/logs/`. Each asserts the weakness: a pass means it is there.

**Round-1 PoCs re-run unmodified** (`poc/D/logs/r1poc_*.out`): monitor 9 of 10 now fail on their attack assertions; the tenth
(D-M-2 "Undeliverable judged against the Bitcoin redemptions") still passes, but only because it calls `judgeLogs` without the
`nft_wrappers` that `checkEvm` now derives from the stack file (`evm.mjs:862-864`, from `kind: "nft"`, which `DeployStack` writes,
`DeployStack.s.sol:403`): wired as production wires it, the mark is matched to the canister's Ethereum delivery (the fixer's
regression). The ops PoC file no longer loads (`recentTransferTxs` was removed): the D-H-1 attack it encoded (plain transfers crowd
out every sale) was re-built against `recentSaleTxs` and is closed; its successor is D2-M-2 below.

## Verdict for area D

Every round-1 service High and Medium is closed as written. Round 2 adds **two new Mediums** in area D, both about what the fixes
built around: the outbound canary and the custody check page a false SEV1 "pause, then look" on **every honest buy and every honest
redemption** (D2-M-1), and the sales-only hand-in can still be crowded out, now with cheap wash sales instead of plain transfers
(D2-M-2, the residual of D-H-1 / B-M-1). Neither loses custody or mints anything; D2-M-1 is the round-1 D-M-2 pattern (false SEV1s
train the operator to ignore the one page that matters), D2-M-2 is an outsider stalling one collection's floor for tens to hundreds of
dollars a day.

## Counts (new in round 2, area D)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 2 | 5 | 6 |

Plus the round-1 Lows left partly or not fixed in this area (carried, not recounted): D-L-3 (partial), D-L-6 (partial).

## 1. Round-1 items in area D

| ID | Sev (R1) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **D-H-1** honest runner hands in transfers | High | **Closed** (residual **D2-M-2**, **D2-L-2**) | `recentSaleTxs` (`ops/src/ethnft.ts:255-281`) reads Transfer logs newest-first in 2,400-block chunks over 50,400 blocks, keeps only receipts `isSale` accepts (`:233-246`: Seaport 1.5/1.6 `OrderFulfilled` or Blur v2 naming the collection, plus a Transfer `from != to`), ≤ 60 new receipts a pass, verdicts cached. Plain transfers, mints, self-trades no longer take slots (fixer's regression). Cold start fits: Milady's ~12 sales need ~55 receipts (2 days), under the 60 budget. But the runner hands in the 12 newest *sale transactions* without the canister's counting rule (one per token/seller/buyer, ±30 % of `L`), so 12 cheap wash sales still void the pass (D2-M-2); and the verdict cache is keyed by tx, not (collection, tx) (D2-L-2). |
| **D-H-2** (monitor half) clamped posts read as a stolen key; registry pause | High | **Closed** | `judgeStackLogs` (`evm.mjs:299-320`) names the canister and emits `pause-poster` on the collection's `posterSource` (from `stack.mjs:39`); `floorWalk` and `no-good-post` do the same (`evm.mjs:213-215, 816-817`); `checkEvm` dispatches `guardian.pausePoster` (`:836-838`), never `pauseKeeper` for an NFT module. The premise (the canister posts inside `floorBounds()`, `ethflow.rs:1728-1736`) holds as long as the bounds are read after the canister's previous post has landed (the canister's single home line; left to the canister part). Round-1 PoC fails. |
| **D-M-1** D8 registry-wide, fires on real moves, sticky, floods, one keyless reference | Med | **Closed** (notes **D2-I-1**, **D2-I-2**) | `floorAlerts` (`monitor/src/ethnft.mjs:136-171`): pause only on the canister's reading fresh ≤ 1 h, > 20 % from both references on one side, the two read and within 10 %; latched per module until `eth_nft.floor_pause_ack.<module> ≥ latch.at` (`:297-298, 330-335`); latch persisted in the state file; stable keys; a missing reference pages at 30 min, both at once; references cached 5 min (`:283-294`); `guardian_pause=false` honoured (`:330`); config refuses D8 without both references per collection (`config.mjs:133-135`). The latch cannot clear itself: only an ack at or after the latch time does; a failed or pending pause leaves no latch, so it is retried. Stale or absent reading: no pause, by design (and nothing is being posted then). Round-1 PoCs ×3 fail. |
| **D-M-2** canary blind to EthNftWrapper events, Undeliverable vs Bitcoin | Med | **Closed** | `judgeLogs` (`evm.mjs:83-109`) reads `Wrapped`/`Deposited`/`Delivered`/`Undeliverable` from NFT wrappers against `recordIndex`'s `ethMints`/`ethDeliveries` (`records.mjs:87-104`, from `eth_nft_state` read with the records, `ic.mjs:37-47`, after the logs). Recording/Recorded carry the Ethereum delivery tx (`ethflow.rs:1000, 1761, 2093-2095`), which is what `recordDelivery` emits. Buys and deliveries are never pruned from `eth_nft_state`, so a replayed range still matches. See the PoC note above. |
| **D-M-3** no Ethereum outbound canary | Med | **Closed** (but see **D2-M-1**) | Nonce: chain `eth_getTransactionCount` on both providers at `minHead−2` against `max(last_mined+1, line.nonce+1)`, records read after the chain (`ethnft.mjs:346-357`); `ownerOf` of Bought/Final/Minting buys and Seen/Mint/Minting/Held/Return deposits (`:262-276`); balance vs Σ earmarks (Arrived `wei` + Buying `arrived`, the canister's own `earmark()`, `ethflow.rs:148-154`) (`:358-361`); halted line SEV1 (`:223-227`). Every one fires on its attack (fixer's regression) — and two of them also fire on honest operation (D2-M-1). |
| **D-M-4** runner re-hands a refused listing for ever | Med | **Closed** (note **D2-I-3**) | `ethNftOnce` (`ops/src/ethnft.ts:337-364`) remembers each hand-in per buy for 2 h and moves to the next; pre-checks `min(arrived − 0.003, reading × (1 + premium))`. Refused hand-ins are marked too. |
| D-L-1 | Low | Fixed (note D2-I-4) | `guarded` (`:196-201`): 15 s deadline, `redirect: "manual"`, 4 MB cap; every `eth_deposit` in its own try; each runner step caught (`runner.ts:176`). The cap is checked after the whole body is buffered. |
| D-L-2 | Low | **Partly** (**D2-L-3**) | `ethStepProblems` refuses to start; `/healthz` 503 on 40 min without accepted evidence. Checked only once at start, and skipped when `eth_nft_config()` cannot be read then (`runner.ts:163-170`). |
| D-L-3 | Low | Partly, as stated | The monitor remembers an unjudged inbound transfer past its window (`ethnft.mjs:314-324`); the runner's discovery window is unchanged. |
| D-L-4 | Low | Fixed | `ownerOn` (`:197-204`): an error is unread; one stable SEV2 after 3 polls; two failing providers are no owner. |
| D-L-5 | Low | Fixed (residual **D2-L-4**) | Each collection in its own `try` (`:250-341`). A failed collection drops its `pools`/`refMissing`/`ownerUnread` timers (D2-L-4). |
| D-L-6 | Low | Partly, as stated (**D2-L-5**) | premium 500 and chain 1 held; `--canister-evm` ties the custodian, but under `--phase1` its absence is only a WARN (`golive-check.sh:635-636`). |
| D-L-7 | Low | Fixed | `checkNftSlugs` (`StackChecks.sol:99-109`). |
| D-L-8 | Low | Fixed except one sentence (**D2-L-1**) | The Trust page now matches the code, except its A-M-1 claim. |
| D-I-2 / D-I-3 / D-I-4 | Info | Fixed | `guardian_pause=false` honoured; the reconciler excludes transit units (`backing.ts:87-91`); the OpenSea key in `envSecrets` (`log.ts:19-21`), no redirect with the key. |
| nft-pool: `poolAlerts`, indexer, site | — | Reviewed | `poolAlerts` (`ethnft.mjs:106-115`) pages a pool due with no open buy for 24 h, per collection (timer loss: D2-L-4). Indexer reads the factory's targets and `buffer()`; the site shows `convertibleUnits` against `1 + buffer` (D2-I-5). No safety issue. |

## 2. New findings

### Medium

#### D2-M-1: the outbound canary and the custody check page SEV1 "pause, then look" on every honest buy and every honest redemption

- **Where:** `monitor/src/ethnft.mjs:358-361` (the earmark rule) and `:253-260` with `:247-248` (the wrapper's held items against
  `ownerOf`); the canister's timing in `ethflow.rs:1637-1660` (`mined_eth`: Buying → Bought only when a tick reads the receipt) and
  `src/nft/EthNftWrapper.sol:379-392` (`itemOf` is deleted only by `recordDelivery`).
- **Mechanism (a), buys:** the rule is `balance < Σ(Arrived.wei + Buying.arrived)`. When our fill mines, the address pays the price
  and gas at once, but the canister moves the buy out of `Buying` (so the earmark drops) only at its next tick, which the runner
  sends every `RUNNER_TICK_EVERY_S` (default 600 s) unless it has just handed something in. For those minutes the balance is
  `pot + arrived − total − gas < arrived`, and the monitor (every 60 s) pages
  `eth-arrived-gone`: "ETH left without a fill of ours. Pause, then look." With several collections the other buys' earmarks make it
  no better.
- **Mechanism (b), redemptions:** the custody loop checks every item whose `itemOf` is still set. After a delivery mines on Ethereum
  the redeemer owns the token, but `itemOf` is cleared only when the canister's `recordDelivery` lands on 4663, after Ethereum
  finality (~13–15 min), a tick (≤ 10 min) and a 4663 transaction. The loop ignores the canister's delivery records, so every
  redemption pages `eth-not-held` SEV1: "An NFT left custody without a recorded delivery or return: pause, then look." This half
  predates round 1 (it is in `57160b0`'s loop) and was not reported then.
- **Impact:** on every honest buy and every honest redemption, a SEV1 custody alarm with the words "pause, then look", repeated
  while it lasts. This is the round-1 D-M-2 pattern (a false SEV1 on an honest action, rated Medium): the operator either pauses on
  noise or learns to ignore `eth-not-held` and `eth-arrived-gone`, which are the two pages that mean an NFT or the arrived ETH was
  actually taken. No guardian action is wired to them, so nothing pauses by itself.
- **PoC:** `poc/D/monitor/test/audit2_nft_d.test.mjs` "D2-M-1a" (a Buying fill mined, the canister not yet ticked) and "D2-M-1b"
  (a delivery `Moved`, the wrapper still holding the item): both **pass** (SEV1 raised).
- **Fix:** (a) count a buy's earmark only until its fill is visible: for a `Buying` buy, read its fill's receipt (every version in the
  line's `rivals`) and, if mined, earmark nothing; or compare against `arrived − total − gas` for a Buying buy whose `ownerOf` is
  already us. (b) Skip (or check against the destination) an item whose token is in a canister delivery in `Sent`, `Moved`,
  `Record` or `Recording` and whose `ownerOf` equals that delivery's destination; keep SEV1 for any other owner. Add both honest
  cases to the regression suite.

#### D2-M-2: 12 cheap wash sales crowd every real sale out of the runner's hand-in; the canister counts < 3 and posts nothing (D-H-1 / B-M-1 residual)

- **Where:** `ops/src/ethnft.ts:255-281` (`recentSaleTxs`: the 12 newest sale transactions) and `:233-246` (`isSale`, no price, no
  parties); `ethnft.rs:72` (`MAX_SALES` = 12), `:524-541` (`counted_sales`: one per token, per seller, per buyer, ±30 % of `L`),
  `:597-601` (`min_sales` 3 or no post).
- **Mechanism:** the runner stops at the 12 newest transactions that look like a sale, whoever traded and at whatever price. The
  canister then keeps one sale per token, per seller and per buyer, within ±30 % of the listing floor. Two wallets trading one token
  back and forth on Seaport or Blur (fees ~0 on Blur; any price, since the NFT returns to the attacker) make sales the runner hands in
  and the canister counts at most twice (at a far price: never). Kept as the 12 newest (≈ 10 wash sales for every real one; Milady has
  ~5 real sales a day, MAYC ~26, per round 1's measurement), they leave the canister with < 3 counted sales on every pass. One
  multicall transaction can carry a wash sale of several collections and take a slot in each (and D2-L-2 lets it take slots it is not
  even a sale for).
- **Impact:** no reading, so no post: after `FLOOR_TTL` the collection's module is stale (mints and burns revert `Stale`,
  graduations are declined, an `Arrived` buy cannot price its bound), until the per-collection poster pause (`pause_on_no_good_post_h`,
  off by default) or the 7-day escape. Outsider cost: gas only, roughly $50–300 a day per collection at a couple of gwei. The monitor
  pages "no good post"; D8 does not fire (no reading). Redemptions and custody are unaffected, hence Medium. An honest market where
  a few Blur bidders take most sales can land in the same place without an attacker (not measured here).
- **PoC:** `poc/D/ops/test/audit2_nft_d.test.ts` "D2-M-2": 3 real sales, then 12 Seaport sales between two wallets; the runner hands
  in exactly the 12, which the canister's dedupe (mirrored in the test) counts as 2; the 3 real sales alone count 3. **Passes.**
- **Fix:** have the runner apply the canister's rule before it chooses: parse the price and parties of each sale (as `sales_in`),
  drop self-trades, keep one per token, seller and buyer and within ±30 % of its own rank-2 listing, and hand in the 12 newest that
  **count**; keep scanning (within the receipt budget) until 12 count or the window ends. On the canister, raise `MAX_SALES` or accept
  the runner's hand-in as candidates only. The durable fix is round 1's "better" one: the canister chooses its own sample (`eth_getLogs`
  in the canister), which also closes A-M-1's runner residual.

### Low

#### D2-L-1: the Trust page says deposit-then-burn cannot pay out a high floor; under a malicious runner it can

- **Where:** `web-rook/src/rook/phase1Trust.ts:143` ("a deposit is minted only while the module's floor is at most 5 % over the
  median of the sales the canister verified, so a floor above the market is not paid out through deposit then burn").
- **Mechanism:** the same bullet says the runner chooses which sales to hand in. The canister's deposit band uses exactly those
  sales, so a runner handing in the week's highest sales lifts both the post (≤ +10 % over their median) and the band's median: a
  floor above the market does get paid out through deposit then burn (A-M-1's residual, which `fixes-round1.md` §4 names).
- **Fix:** say "while the runner is honest" or state the residual; the canister choosing its own sample removes it.

#### D2-L-2: the runner's sale verdicts are cached per transaction, not per collection

- **Where:** `ops/src/ethnft.ts:257, 269-274` (`verdicts` keyed by tx hash; `isSale(r, collection)` is per collection); `runner.ts:161`
  (one map for every collection).
- **Mechanism / impact:** a transaction is judged under the first collection whose logs contain it. A real Azuki sale that also moves
  a Milady (a bundle, an aggregator sweep) is cached "no sale" under Milady and never handed in for Azuki; a wash sale of one
  collection that moves another collection's token is cached "sale" and takes a slot in both (it amplifies D2-M-2 across collections).
- **PoC:** `audit2_nft_d.test.ts` "D2-L-2": **passes**.
- **Fix:** key the cache by `collection:tx`.

#### D2-L-3: the runner's "refuse to start half-configured" runs once, and not at all if the canister's config is unread at start

- **Where:** `ops/src/runner.ts:163-170`.
- **Mechanism:** `ethStepProblems` runs only when `eth_nft_config()` answers at start (`.then(…, () => null)`); a transient failure,
  or a canister not yet configured, starts the runner with `served = []`: no refusal, and `staleEvidence` never flags anything. A
  collection added later (an upgrade) is never checked for a slug.
- **Impact:** D-L-2's scenario (no slugs or no `ETH_LOGS_RPC`: no evidence, the floor never posted) returns quietly after a restart
  that met a blip; the monitor's "no good post" still pages. Low.
- **Fix:** re-read `eth_nft_config()` each pass; recompute `served` and the problems; `/healthz` 503 on any problem.

#### D2-L-4: one failed read of a collection resets its pool-idle, missing-reference and owner-unread timers

- **Where:** `monitor/src/ethnft.mjs:229-233` (`next.pools`, `next.refMissing`, `next.ownerUnread` rebuilt from empty each poll) with
  the per-collection `catch` at `:337-341`.
- **Mechanism / impact:** a collection whose read throws once (any 4663 or `eth_getBalance` error) carries none of its timers into the
  next poll. A pool due a buy is paged only after 24 h of consecutive clean polls, a missing reference only after 30 min of them: one
  blip a day keeps the pool-idle page silent for ever.
- **PoC:** `audit2_nft_d.test.mjs` "D2-L-4" (due from hour 0, one failed poll at hour 12, silent at hours 25 and 30; the control
  without the blip pages). **Passes.**
- **Fix:** start `next.pools/refMissing/ownerUnread` from `prev` and overwrite per collection only on success (as `refs`, `latch` and
  `unclaimed` already are).

#### D2-L-5: nothing that holds the launch values is compared with the launch file; `--canister-evm` is only a WARN under `--phase1`

- **Where:** `script/golive-check.sh:495-514` (`targetcheck` compares the factory and module with the **stack file**, which the same
  deploy wrote), `:635-636` (WARN), `deployments/launch/nft-pilot.json` (read only by the dry run, `scripts/golive-dryrun.sh:184-197`).
- **Mechanism / impact:** the real deploy takes `ETCH_NFT_NORMAL_RAISES` / `_EARLY_RAISES` as hand-typed comma lists in collection
  order. A shifted list (BAYC's 9 ETH on Pudgy, 5 ETH on BAYC) passes `DeployStack` (only the list length is checked,
  `DeployStack.s.sol:484-489`) and passes golive-check (it matches the stack file). BAYC would then pool three graduations per ape.
  Likewise a single-collection `ETCH_NFT_NORMAL_RAISE` set with a multi-collection list applies to all five. Economic drift, not
  custody. Separately, a stack whose custodian is not the canister's key (an EOA that can then `mint`/`mintDeposit`) passes
  `--phase1` with a WARN when `--canister-evm` is omitted.
- **Fix:** golive-check `--launch deployments/launch/nft-pilot.json` compares each collection's targets and module parameters with
  the launch file; make `--canister-evm` required under `--phase1` for an NFT stack.

### Info

- **D2-I-1 (D8 scope):** D8 now compares only the canister's *reading* with the references. A compromised canister key (trusted by
  the threat model) walking `accepted` inside the bounds is not auto-paused: `floorAlerts` uses `accepted` only for the 10 % SEV2,
  and `floorWalk`'s pause needs `pause_on_floor_walk` (default false, `config.example.json:57`). Manipulation of the listing book
  moves OpenSea's and CoinGecko's floors too, so D8 is a guard against a broken reading, not against the book; the sale veto is that.
- **D2-I-2 (the ack):** the ack is a config value (`floor_pause_ack`), so acknowledging needs a config change and a restart; an ack set
  far in the future silently disarms D8 for that module (every later latch is cleared at once). Consider an ack bound to the latch
  (its `at` exactly) or to the pause tx.
- **D2-I-3 (D-M-4 bound):** the runner pre-checks against its reading × premium; the canister bounds by `min(reading, accepted)` ×
  premium and by what is available after other buys' earmarks, so some listings the runner hands in are refused (each once per 2 h).
  The per-buy `tried` map, `saleVerdicts` and the monitor's `unclaimed` entries for our own fills are never pruned (D-I-5 class).
- **D2-I-4 (D-L-1):** `guarded` reads the whole body before the 4 MB check; a provider streaming a huge body within 15 s is buffered.
  Read with a byte counter and abort.
- **D2-I-5 (site):** the pool figure is the module's `convertibleUnits` against `1 + buffer`, while the canister and the monitor use
  `netUnits()` (net of transit and reserve): the page can say "due" while an advance is out. The collection page's floor sentence still
  says "the keeper's fresh cap" for an NFT nest.
- **D2-I-6 (earlier-round rules on the canister's posts):** `floorWalk`'s edge-run and 18 %/day SEV1 are keeper-era rules; the canister
  posting at the band's edge in a real NFT move pages them (pause only with `pause_on_floor_walk`). Expected, but worth a line in the
  runbook for the NFT collections.

## 3. The launch configuration (five collections, BAYC at 9 ETH, deposits on, D1–D9)

| Item | Finding |
| --- | --- |
| Launch list (D1) | golive-check's `NFT_LAUNCH` holds the five (`golive-check.sh:446`); `ethcheck` holds deposits on (D5), the D7 rule, Relay 300 bps, premium 500, chain 1, and that the canister serves exactly the stack's collections. OK. |
| Per-collection targets | Bounded (`StackChecks.checkNftCollection`), read back by `DeployStack`, compared by golive-check with the stack file. **Not compared with the launch file** (D2-L-5). |
| BAYC at 9 ETH with defaults (J1) | Pools two graduations per ape with ~2.6 % floor-rise headroom (the design's own numbers). Safe (units stay ETH-backed and burnable); a liveness and expectation matter, operator-decided. |
| Stage-0 custody caps (J7) | 3 items / 10 ETH per collection and 6 items / 10 ETH globally, shared with deposits (J4). One ape (~6.8 ETH) takes two thirds of the global value; while it is held Pudgy (~3.4 ETH) never fits, and if the three cheap collections take ~3.2 ETH first BAYC never fits. A depositor can fill the global item cap with six cheap NFTs and stop every collection's buys (each buy is dropped before its advance, then re-tried; `eth-pool-idle` pages after 24 h). **Not unsafe for funds** (pools stay ETH-backed), but at stage 0 with five collections and deposits on, most pooled ETH will not become NFTs. The operator's J7 decision is still open and should be taken before launch. |
| Canister `max_purchase_wei` | Only in the launch file's `_canister` note; golive-check does not read it (D-L-6 remainder). Bounded anyway by H2's per-item cap and by the advance itself. Info. |
| Monitor | `validate` refuses an `eth_nft` config without an armed guardian and without `opensea_slug` + `coingecko_id` per collection. No launch monitor config is in the repo (only `config.example.json`), so the five collections' reference ids are not reviewable here. The two false SEV1s of D2-M-1 fire from the first buy and the first redemption. |
| Runner | `RUNNER_TICK_EVERY_S` 600 by default: every canister timer (re-pricing 30 min, fill cancel 6 h, readings 1 h, urgent post 40 min) moves at that granularity; fine, but it is also the D2-M-1(a) window. |

**Launch blockers from area D:** none for custody or backing. D2-M-1 and D2-M-2 are Mediums and, by this review's bar (0 C / 0 H /
0 M), must be fixed before launch: D2-M-1 because the custody pages it breaks are the incident signal for the whole Ethereum side;
D2-M-2 because any outsider can hold a collection's floor stale for the price of gas. J7 (stage-0 caps against BAYC and Pudgy with
deposits on) should be decided before launch.

## 4. PoC results

| File | Test | Result |
| --- | --- | --- |
| `poc/D/monitor/test/audit2_nft_d.test.mjs` | D2-M-1a, D2-M-1b, D2-L-4 | 3/3 pass (weakness present) |
| `poc/D/ops/test/audit2_nft_d.test.ts` | D2-M-2, D2-L-2 | 2/2 pass (weakness present) |
| Round-1 `poc/D/monitor/test/audit_nft_d.test.mjs`, unmodified | 10 | 9 fail on their attack assertions; 1 (D-M-2 Undeliverable) passes only for want of production wiring (see top) |
| Round-1 `poc/D/ops/test/audit_nft_d.test.ts`, unmodified | 6 | does not load (`recentTransferTxs` removed); the attacks re-checked by hand and by the fixer's regressions |

To run: copy `poc/D/monitor/test/audit2_nft_d.test.mjs` into `monitor/test/` (`node --test test/audit2_nft_d.test.mjs`) and
`poc/D/ops/test/audit2_nft_d.test.ts` into `ops/test/` (`node --experimental-sqlite --no-warnings --import tsx --test
test/audit2_nft_d.test.ts`).
