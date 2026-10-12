# Round 3 re-review, area D: services, the launch monitor template and the Trust page (R3-services)

Reviewer: independent AI re-review (part D), 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at
`4949da0`; the round-2 fix delta `79455e1..4949da0`. Read: `review2/R2-services.md`, `fixes-round2.md` (§0, §4, §6, §7 — its claims
were checked against the code, not taken), and at HEAD `monitor/src/{candid,config,ethnft,evm,alerts}.mjs`,
`monitor/test/audit_nft_d_r2.test.mjs`, `ops/src/{ethnft,ic,runner}.ts`, `ops/test/{audit_nft_d,audit_nft_d_r2,ethnft}.test.ts`,
`web-rook/src/rook/phase1Trust.ts` (+ `web-rook/test/phase1.test.ts`), `deployments/launch/{monitor-nft,nft-pilot}.json`,
`autobuyer/canister-rs/etch_autobuyer.did`, `tests/fixtures/eth-state-round2.hex` and the Rust types it encodes
(`src/ethflow.rs`, `src/ethnft.rs` `FloorPost`), and, where a service relies on them, `ethflow.rs` (`resolve_eth`, `superseded`,
`mined_eth`, `delivery_pass`, `back_now`, `give_back_now`) and `src/nft/EthNftWrapper.sol` (`recordDelivery`, `refund`).

**Nothing in the tracked tree was modified; nothing was committed, pushed, deployed or signed.** Every test ran in a private copy under
the scratch dir (monitor, ops, the canister crate). Network: read-only GETs/`eth_*` reads of the template's public references and
Ethereum providers (`poc/D/logs/live_probe_template.out`). PoCs assert the weakness: a pass means it is present.

## Verdict for area D

The round-2 service fixes do what they say for the cases they were written for: an honest fill no longer pages "ETH gone", a
redeemer who keeps the NFT no longer pages "left custody", the per-collection timers survive a failed read, the runner hands in
listings only and re-checks its configuration every pass, the monitor refuses D8 without the no-good-post pause, and the monitor's
and runner's Candid types match the `.did` and the Rust types field by field for every case this round added.

But trying to hide a real loss inside the new custody allowance led to a **new High, outside the services' own code**: the canister
records a delivery only if, at finality, the **destination still owns the token** (`ethflow.rs:1238-1241`). A redeemer who moves the
NFT on before that (an arbitrageur selling into a bid, a second wallet, a sweeping smart wallet) leaves the ticket `Resolved` for good:
never recorded, never marked. `EthNftWrapper.refund()` then mints the redeemer's unit back 30 days after `deliverBy`, while the NFT is
gone: one unbacked unit per redemption, repeatable, and without an attacker (**D3-H-1**, canister + wrapper; flagged to part C). The
monitor pages it (SEV1 `eth-not-held` and later `eth-deliver-deadline`) but no pause stops `refund()`; only a timelocked canister
upgrade within the 30 days does.

The rest is Low/Info: a stale `Recorded` delivery masks a later loss of the same token to the same address (D3-L-1), an arrival race
and a constant alert key (D3-L-2), and the launch template's first Ethereum provider refuses the monitor's 7,200-block deposit scan
on its free plan (D3-L-3, measured).

## Counts (new in round 3, area D)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 1 | 0 | 3 | 7 |

Round-2 items in this area not actually fixed: none at Medium or above (D2-M-1(b) has the residuals D3-H-1 / D3-L-1, counted as new).
Carried, unchanged, not recounted: D-L-3 (partial), D-L-6 (partial, part A), D2-I-1…D2-I-6.

## 1. Round-2 items in area D

| ID | Sev (R2) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **D2-M-1 (a)** honest fill pages `eth-arrived-gone` | Med | **Closed** (residuals **D3-L-2**, **D3-I-2**) | `earmarkOf` (`monitor/src/ethnft.mjs:139-150`) takes a buy's earmark down only when the ONE line transaction is this buy's (`Buy`/`CancelFill`/`BackDeposit` with its id) AND the chain nonce is past it; by at most `max(line.value, body.total, body.value) + max_fee × max(gas, 300k)`. The line is cleared as soon as the canister reads the receipt (`ethflow.rs:1738-1740`), so the allowance lasts only until the canister's next tick, applies to one buy, and never sums. A tx spending more than its value is impossible for an EOA; a non-line tx at a nonce beyond the line is `eth-outbound-unknown` (unchanged). What it can hide: a foreign tx at the line's own nonce (needs the canister's key, trusted) — `Buy`/`BackDeposit` then halts the line (SEV1 `eth-line-halted` at the canister's next tick), but `CancelFill`/`Cancel` never halt (D3-I-2, PoC). The fixer's test has real controls (nonce unused + ETH gone pages; ETH gone beyond value + gas pages). |
| **D2-M-1 (b)** honest redemption pages `eth-not-held` | Med | **Closed for a redeemer who keeps the NFT**; residuals **D3-H-1**, **D3-L-1**, D3-I-7 | Suspects are judged after the records are read again (`:404-411`), exempt only if a delivery of that collection/token to that very owner is `Sent`/`Moved`/`Record`/`Recording`/`Recorded`. A redeemer who moves the NFT on inside the ~15–25 min before `recordDelivery` lands still pages SEV1 — and that page is, today, right: the canister will never record that delivery (D3-H-1). `Recorded` is matched with no link to the current item, so an old record masks a later loss (D3-L-1). `Cancelling` is not in the set (D3-I-7). |
| **D2-M-2** wash sales crowd the runner's sale hand-in | Med | **Closed on the runner side** (the canister's own sample is part B's) | `ethNftOnce` hands in `eth_floor_evidence(coll, listings, [])` (`ops/src/ethnft.ts:328-330`); `recentSaleTxs`/`isSale` are gone. The round-2 PoC "no longer loads" (it imports `recentSaleTxs`) — that alone proves nothing; the fixer's replacement (`ops/test/audit_nft_d_r2.test.ts` "D2-M-2") is not vacuous: 12 wash transfers on the fake chain, and it asserts the runner hands in `[]` sales and makes **zero** chain reads. Whether the canister's own scan resists washes is B's (`floor_from`, `sale_scan`). A malicious runner can still withhold listings (stated on the Trust page). |
| D2-L-1 Trust page A-M-1 claim | Low | **Fixed** (text notes **D3-I-3**) | The claims now match `floor_from` / `deposit_floor_ok` (the lower quarter, one per token and wallet, the residual stated). Two sentences overstate (first post; 48 h; "after Relay's fee"). |
| D2-L-2 verdict cache per tx | Low | **Moot** | No runner-side sale reading remains. |
| D2-L-3 problems checked once | Low | **Closed** | `ethStepProblems` on every pass (`ops/src/ethnft.ts:259-266`, the call at `:263`), written into the shared `problems` that `/healthz` reads (`runner.ts:180`, 503 while any); the start-time unread config leaves a problem until a pass reads it (`runner.ts:171`); `served` refreshed each pass; a config that stays unreadable keeps the last problems and `staleEvidence` 503s after 40 min. The fixer's test covers the upgrade-adds-a-collection case. |
| D2-L-4 timers reset on a failed read | Low | **Closed** | `pools`, `refMissing`, `ownerUnread` start from `prev` (`ethnft.mjs:258-263`); `dueSince` of a collection is replaced only after its pool is read (`:344-345`); owner-unread counts only after the whole collection read (`:373-374`). A permanently failing read does not silence anything: `eth-collection-unread` SEV2 pages every hour (`:377`). (If the read always fails AFTER the `ownerOf` step, the owner-unread count never advances — covered by that hourly SEV2.) Round-2 PoC fails on its attack assertion. |
| D8 without `pause_on_no_good_post_h` | (launch note) | **Closed** | `config.mjs:134-135` refuses `eth_nft` with D8 armed and the value undefined; `:98-99` bounds it to 2–160 (`null`/strings refused). `evm.mjs:807-818` pauses the collection's own poster (`posterSource`). The canister's held posts use confidence 5,000 ≥ the module's `minConfidenceBps` 3,000, so a held post keeps `lastGoodAt` fresh (no false pause from B2-M-1's "hold"). |
| New way-back pages | — | **Fire as intended** (notes **D3-I-1**, **D3-I-4**) | `BackFunded` ≥ 2 h SEV2 (`:48`), `BackBridging` ≥ 30 min SEV2 (`:50`) keyed by the phase's own `since`; the `Arrived` page's "48 h" matches `ARRIVED_BACK_SECS`, counted from `since` (which a reverted or cancelled fill resets, `ethflow.rs:1954, 1993`). Right after `eth_give_back` the `BackFunded` page fires at once with a nonsense wait (since 0, D3-I-1); `BackSending` (the way back's deposit unmined, holding the line) has no page (D3-I-4). |
| IDL vs `.did` vs Rust | — | **Match** | Field by field: `BPhase::BackFunded {wei, since}`, `BackSending {wei, order_id, value, min_out, deadline, tx}`, `BackBridging {order_id, value, min_out, deadline, since}`; `EthKind::BackDeposit {buy}`; `EthTx.value/max_fee` nat, `gas` nat64; `FloorPost.held` opt text, `EthState.sale_scan` opt vec (both unread by the services: record subtyping). Monitor `candid.mjs:173-208`, runner `ic.ts:36-66`, `.did:29-135`, `ethflow.rs:85-118, 305-342, 395-440`, `ethnft.rs:628-646`. The fixture is `candid::encode_one(round2_state())` held by `r2_the_round_2_state_fixture_is_what_the_canister_encodes` (`tests/audit_nft_r2.rs:1071`) and decoded by both services' tests with their own IDL (pass in the private copies). Every variant of `BPhase`, `DelPhase`, `EthKind`, `XPhase` in the Rust types is present in both IDLs, so no reachable state is a decode failure. |

## 2. New findings

### High

#### D3-H-1: a redeemer who moves the NFT on before Ethereum finality is never recorded; 30 days later `refund()` mints the unit back — an unbacked unit per redemption

- **Where:** `autobuyer/canister-rs/src/ethflow.rs:1233-1242` (`delivery_pass`, `Moved`: at finality `ownerOf(token)` must equal
  `d.destination`, else `Err("final, but the token's owner is 0x…, not the destination: nothing is recorded; look")`, the phase stays
  `Moved`, every tick, for ever); `:1281-1302` (`mark_pass` marks only `Waiting`); `src/nft/EthNftWrapper.sol:380-393`
  (`recordDelivery`, custodian only), `:408-433` (`refund`: anyone, a `Resolved` ticket past `deliverBy + LATE_DELIVERY_WINDOW`
  (30 days) without the attestation → `_pending.push(itemId)` and `_mint(t.owner, ONE_ITEM)`; no pause check). Present since
  `bbb9478`; not reported in rounds 1–2.
- **Mechanism:** the delivery is a plain `transferFrom(custodian, destination, token)`. The canister moves it `Sent → Moved` when it
  reads the receipt, then waits for finality (~13–15 min) and, at its first tick after that (the runner ticks every ≤ 10 min), reads
  `ownerOf` at its pin. If the destination has already moved the token — sold it into a Blur/OpenSea bid (the natural arbitrage
  when a unit trades under the floor), sent it to another wallet, or a smart wallet that sweeps — the check fails and nothing ever
  records it. Nothing marks it either (`Moved` is not `Waiting`; the deadline cancel acts only on an unmined `Sent`). After
  `deliverBy + 30 days` the ticket owner calls `refund(id)`: one unit is minted back and the item returns to the draw set, although
  the NFT is at the buyer. The redeemer keeps (the proceeds of) the NFT and the unit. The phantom item is later drawn by an honest
  redeemer; the canister cannot transfer it (`eth_plan_transfer`: owner is not us), marks it, and that redeemer is refunded too, so
  the phantom circulates while `totalSupply` stays one unit over the NFTs held.
- **Impact:** unbacked units, one per redemption so treated, at gas cost, with or without intent (an honest arbitrage bot does it
  every time; a rational one then calls `refund`). Bounded per collection by the stage-0 caps (3 items / 10 ETH) and by the 37 days
  per cycle per unit, but repeatable and parallel. **Who can stop it:** the monitor does page (SEV1 `eth-not-held`, since the owner
  is not the destination; later SEV1 `eth-deliver-deadline`, whose text wrongly says "the holder waits 30 days" as if that were the
  safe outcome), but no guardian pause stops `refund()`, and the canister has no operator path to record a `Moved` delivery: only a
  canister upgrade through the 72 h timelock inside the 30 days. Hence High, not Critical.
- **PoC:** `poc/D/canister/d3_h1_append_to_tests_ethnft.rs` (appended to a private copy of `tests/ethnft.rs`, using its own
  `Fake`): a resolved ticket to ALICE is sent and mined; ALICE moves token 77 to BOB; finality; 40 days of ticks: the delivery is still
  `Moved` with the "not the destination" note, the ticket is still `Resolved`, nothing marked. **Passes** (`logs/audit3_canister.out`).
  The wrapper half is the repository's own `test/ethnft/EthNftWrapper.t.sol::test_withoutTheAttestationTheLateWindowIsThirtyDays_onBothClocks`
  (a `Resolved` ticket without the attestation refunds after 30 days, minting the unit). The monitor side:
  `poc/D/monitor/test/audit3_nft_d.test.mjs` "D3-M-1" (Sent/Moved/Record/Recording, owner = a buyer ≠ destination → SEV1) **passes**.
- **Fix (canister, part C):** judge the delivery by its own receipt, not by the current owner: at finality re-read the receipt at
  the finalized pin (success, block ≤ finalized, the same hash) and require the collection's `Transfer(custodian → destination,
  token)` log in it; then record. Keep `ownerOf` only as a diagnostic. Add the case "destination moved it on" to the tests. **Monitor
  (paired, or it becomes a false SEV1 on every such honest redemption once the canister is fixed):** for a wrapper item whose delivery
  of ours is mined (`Moved`/`Record`/`Recording`, or `Sent` with the delivery's receipt read by the monitor), accept any owner
  other than the custodian; keep SEV1 only for an item with no mined delivery of ours. Correct the `eth-deliver-deadline` text for a
  mined delivery.

### Low

#### D3-L-1: a stale `Recorded` delivery masks a later loss of the same token to the same address

- **Where:** `monitor/src/ethnft.mjs:404-411` (the exemption matches collection + token + destination + phase, including `Recorded`,
  with no link to the item held now) and `:296-309` (the same exemption applies to bought/deposited tokens not yet in the wrapper).
- **Mechanism / impact:** deliveries are never pruned. Token T redeemed to X (`Recorded`) and later back in custody (X deposits it,
  or the canister buys it again) is exempt for good if it ever leaves custody to X again without a delivery — a canister bug that
  re-sends an old ticket, or a key compromise that routes through a former redeemer. For a token in `Bought`/`Final`/`Minting` or a
  deposit, no delivery of the current custody can exist at all, so any match is stale. The custody check is the out-of-band detector
  for exactly these events. Low: it needs an out-of-model outflow plus a prior redemption to that address.
- **PoC:** `audit3_nft_d.test.mjs` "D3-L-1": wrapper item 9180 held, a `Recorded` delivery of 9180 to X, owner X → no SEV1; the same
  for a `Bought` 9180; control (owner THIEF) pages. **Passes.**
- **Fix:** exempt only a delivery whose ticket's item is the item held now (`d.ticket` → the wrapper's ticket → `itemId`, or the
  delivery's mined block after the item's arrival); accept `Recorded` only after re-reading `itemOf` and finding it cleared; never
  exempt a buy's or a deposit's token.

#### D3-L-2: the canary reads the balance at the poll's first block and the earmarks at its end: an arrival between them is SEV1 "ETH gone", and the constant key then mutes the real page for 6 h

- **Where:** `monitor/src/ethnft.mjs:270` (`block` = min head − 2, fixed before the per-collection loop), `:388-398` (balance at
  `block`, records `st2` read after the loop), key `eth-arrived-gone:${custodian}` (`:398`) with `alerts.mjs:36` (a persistent key is
  resent only every `REPEAT_MS` = 6 h).
- **Mechanism / impact:** the canister's pin is 8–16 blocks behind its head (`pin.rs`, bucket 8). When a poll runs longer than ~70 s
  (five collections, sequential `ownerOf` pairs, reference fetches, no per-request timeout) and the canister ticks during it, `st2`
  can show a Relay arrival (`Arrived wei`) that the balance at the poll's `block` does not include: SEV1 "ETH left without a fill of
  ours. Pause, then look." on an honest arrival. Because the key has no nonce/buy in it, that false page also suppresses a real
  `eth-arrived-gone` for up to 6 h. Round 1's canary had the same order; not new code, not reported before.
- **PoC:** `audit3_nft_d.test.mjs` "D3-L-2" (first `ethState` Bridging, second Arrived; balance = the pot) → SEV1. **Passes.**
- **Fix:** read the balance and nonce again after `st2` (at a block ≥ the canister's pin, or at `latest − 2` taken after `st2`), and
  judge the earmarks only against a balance read later than the records; put the shortfall (or the buy ids) in the key.

#### D3-L-3: the launch template's first Ethereum provider refuses the monitor's deposit scan (D5's unclaimed-deposit watch is dead at launch)

- **Where:** `deployments/launch/monitor-nft.json:34` (`eth_rpc[0]` = `https://eth.drpc.org`); `monitor/src/ethnft.mjs:21, 349-363`
  (`eth_getLogs` over `DEPOSIT_SCAN_BLOCKS` = 7,200 blocks on `eth_rpc[0]` only).
- **Measured** (`logs/live_probe_template.out`, 2026-09-27): drpc's free plan answers `{"code":35,"message":"ranges over 10000 blocks
  are not supported on free plan"}` for 7,200, 5,000, 2,000, 1,000 and 500 blocks; 100 blocks answers. `xrpc.cl/eth` serves 7,200.
- **Impact:** every poll, per collection, the scan fails into a SEV3 "unclaimed deposits are not watched this round"; an NFT sent to
  the custodian that the runner never hands in is never paged (D-L-3's backstop, the only one for D5, is off). Nothing else in the
  template depends on large ranges; `ownerOf` at a pinned block works on both providers.
- **Fix:** put `xrpc.cl/eth` first (or a paid endpoint), or scan in chunks the provider serves; have `golive-check --monitor-config`
  (part A) probe `eth_getLogs` over `DEPOSIT_SCAN_BLOCKS` on `eth_rpc[0]`. The same free-plan cap applies wherever this pair is reused
  (the canister's `evm_chains[1]` per `phase1-nft.md` §8, and `ETH_LOGS_RPC`): see "for siblings".

### Info

- **D3-I-1 (way back, `since` 0):** `eth_give_back` on an `Arrived` buy stores `BackFunded { since: 0 }` (`ethflow.rs:2495`) until
  the next tick sets it (`back_now`, `:1218`). A poll in between pages `eth-back-funded:…:0` SEV2 at once, "has waited 30000000 min"
  (`ethnft.mjs:39, 48`). PoC "D3-I-1" passes. Fix: treat `since == 0` as "just asked" (use `seen()`), or set `since: now` in
  `give_back_now`.
- **D3-I-2 (a foreign transaction at a cancel's nonce):** `superseded()` returns `Err` every tick for `CancelFill` and `Cancel` when
  neither our cancel nor a rival is visible (`ethflow.rs:1920-1937`, and the `Cancel` arm at `:1900-1918`): the line is kept, never halted.
  With the round-2 allowance (the rival's `total` + `max_fee × 300k`, `ethnft.mjs:147-149`) the drained address then pages no SEV1
  (PoC "D3-I-2": 0.97 ETH earmarked, balance 0, cancel at 100 gwei → earmark 0, no SEV1). Needs the canister's key (trusted by the
  threat model); before round 2 the same drain paged `eth-arrived-gone` at once. Fix (C): halt after N ticks with the nonce used and
  nothing of ours visible; monitor: don't apply the allowance while the line's outcome is unread for longer than one tick interval.
- **D3-I-3 (Trust page text, D2-L-1 follow-up):** (i) "Recent sales can only hold the floor back … never stop a post … wash trading
  can slow a rise, never freeze the module" — true once a module has posted; its FIRST post needs `min_sales` counted sales and `L`
  inside the band (`ethnft.rs:719-721`), so thin or wash-lowered sales can keep a new collection unposted. (ii) "covers the module's
  floor plus 1% after Relay's fee": `advance_covers` uses a 0.5 % fee estimate (`ethflow.rs:989`, `ADVANCE_FEE_EST_BPS`), while
  `relayeth::judge` accepts up to `relay_max_fee_bps` (300). (iii) "48 hours without a listing": counted from the last `Arrived`
  `since`, which each reverted or cancelled fill and each refunded way back resets. Say "once posted", "Relay's estimated fee", "48 h
  after its last attempt".
- **D3-I-4 (no page for `BackSending`):** a way-back deposit signed and unmined (priced under Ethereum's fees, or over the chain cap)
  holds the one Ethereum line; `Buying` has `eth-buying-late`, `BackSending` has nothing (`ethnft.mjs:40-57`). Deliveries behind it page
  `eth-deliver-late` eventually. Add a 1 h SEV2.
- **D3-I-5 (independence of the monitor's providers):** the template's comment says the pair is the one "the canister's
  evm_chains[1] should use too", and `phase1-nft.md:569` recommends it for the canister, while `:753` calls the monitor's pair "a pair
  independent of the canister's". Lying providers are in the threat model; the custody check is only independent if the monitor uses
  at least one provider the canister does not.
- **D3-I-6 (CoinGecko keyless burst):** each reference is cached 5 min per module, so all five collections re-fetch CoinGecko in the
  same poll (5 calls in seconds, keyless; round 1 measured 429 from the 6th). A 429 is cached as 0 for 5 min; 30 min of it pages SEV2
  and D8 cannot pause meanwhile. The template does not mention `MONITOR_COINGECKO_KEY`; require it for launch or stagger the fetches.
- **D3-I-7 (`Cancelling` not exempt):** when a delivery's transfer wins the race against our deadline cancel, the redeemer owns the
  NFT while the canister still says `Cancelling` (until its next tick reads the rival, `ethflow.rs:1900-1918`): SEV1 `eth-not-held`
  meanwhile (PoC "D3-M-1 (b)" passes). Narrow (a delivery pending a day before its deadline). Add `Cancelling` (owner = destination).

## 3. The launch monitor template (`deployments/launch/monitor-nft.json`), monitor side

| Item | Finding |
| --- | --- |
| Collections and order | The five contracts are right and in `nft-pilot.json`'s order: Milady `0x5af0…25a5`, Azuki `0xed5a…c544`, MAYC `0x60e4…a7c6`, Pudgy `0xbd35…2cf8`, BAYC `0xbc4c…f13d`. |
| OpenSea slugs | `milady`, `azuki`, `mutant-ape-yacht-club`, `pudgypenguins`, `boredapeyachtclub`: each answers the keyless `/stats` the monitor calls (floors 0.929 / 0.674 / 0.858 / 3.29 / 6.30), and `/collections/milady` and `/boredapeyachtclub` name the right Ethereum contract (measured). |
| CoinGecko ids | `milady-maker`, `azuki`, `mutant-ape-yacht-club`, `pudgy-penguins`, `bored-ape-yacht-club`: each names the right contract on `ethereum` (measured); floors agree with OpenSea within 0.3 %. Keyless: D3-I-6. |
| Ethereum providers | Both serve chain 1 and `ownerOf` at a pinned block. `eth.drpc.org` (first) refuses the 7,200-block deposit scan: **D3-L-3**. Independence from the canister: D3-I-5. |
| Guardian | `auto_pause: true`, `pause_on_floor_walk: false`, `pause_on_no_good_post_h: 6` (in range; `validate` now requires it with D8); `guardian_pause: true`; `deposits_scan: true`. The no-good-post pause is per poster (`posterSource`) and skips a module never posted (`evm.mjs:810`). OK. |
| Placeholders | Every `<…>` is filled by the fixer's test before `validate`; `golive-check --monitor-config` (part A) must refuse a leftover. |

## 4. Runner: the way back and the per-pass checks

- `ethAskOf` quotes `Funded` and `BackFunded` buys with the canister's own ask (`ops/src/ethnft.ts:49-54`); `preJudgeEth` picks the leg
  from the ask's chain ids (`1>4663` / `4663>1`), the same depository for both (as `relayeth::TO_ETHEREUM`/`FROM_ETHEREUM`,
  `relayeth.rs:27-31`). It is advisory: the canister re-judges every quote with `relayeth::judge` (`FROM_ETHEREUM` for the way back),
  checks the plan's value, that the cost fits the buy's own earmark, and the one Relay leg (`ethflow.rs:1653-1700`). A malicious runner
  can hand in a bad quote (refused, noted) or none (the `BackFunded` 2 h page); it cannot make the canister sign something unjudged.
  The fixer's A2-M-1 test pre-judges a real Relay `1 → 4663` quote (fixture) and refuses the wrong leg and a foreign recipient.
- Listings only; `ethStepProblems` per pass, `/healthz` 503 on any (D2-L-3, closed). `ETH_LOGS_RPC` is now needed only with deposits
  on (correct: the runner no longer reads sales).

## 5. PoC results

| File (under `poc/D/`) | Test | Result |
| --- | --- | --- |
| `canister/d3_h1_append_to_tests_ethnft.rs` (appended to a private copy of `tests/ethnft.rs`) | D3-H-1 | **pass** (weakness present), `logs/audit3_canister.out` |
| `monitor/test/audit3_nft_d.test.mjs` | D3-M-1 (monitor half of D3-H-1), D3-M-1 (b) (= D3-I-7), D3-L-1, D3-L-2, D3-I-1, D3-I-2 | **6/6 pass** (weakness present), `logs/audit3_monitor.out` |
| round-2 `review2/poc/D/monitor/test/audit2_nft_d.test.mjs`, unmodified | D2-M-1a, D2-M-1b, D2-L-4 | 3/3 **fail on their attack assertions** (the fix; not setup or a control), `logs/r2poc_monitor_rerun.out` — matches the fixer's `fixes-round2-poc/monitor-pocs-rerun.output.txt` |
| round-2 `review2/poc/D/ops/test/audit2_nft_d.test.ts`, unmodified | D2-M-2, D2-L-2 | does not load (`recentSaleTxs` removed), `logs/r2poc_ops_rerun.out` — matches the fixer's output. Not by itself proof; the replacement test is (§1, D2-M-2). |
| suites at HEAD (private copies, no PoC files) | monitor, ops | monitor 169/169, ops 55/55 (`logs/monitor_suite_head.out`, `logs/ops_suite_head.out`) |
| live, read-only | template references and providers | `logs/live_probe_template.out` |

To run: copy `monitor/test/audit3_nft_d.test.mjs` into `monitor/test/` (`node --test test/audit3_nft_d.test.mjs`); append
`canister/d3_h1_append_to_tests_ethnft.rs` to `autobuyer/canister-rs/tests/ethnft.rs` (`cargo test --test ethnft d3_h1`).

## 6. For sibling parts

- **Part C (canister flow) / A (wrapper):** D3-H-1 (`ethflow.rs:1238-1241` + `EthNftWrapper.refund`); D3-I-2 (`superseded` never halts
  a `Cancel`/`CancelFill` whose nonce was taken by a transaction not ours).
- **Part B (the canister's own sale scan):** drpc's free plan refuses `eth_getLogs` over ~100+ blocks (measured). If the canister's
  `evm_chains[1]` uses the recommended `eth.drpc.org` + `xrpc.cl/eth`, every 2,000-block chunk fails on drpc and is halved to ~62
  blocks: the 7-day backfill (50,400 blocks) needs ~800 chunks at 13 a pass, i.e. days of passes before `min_sales` can confirm a first
  post, and costs cycles on every refused chunk.
- **Part A (golive-check):** `--monitor-config` should probe `eth_getLogs` over `DEPOSIT_SCAN_BLOCKS` on `eth_rpc[0]` (D3-L-3), and
  could require `MONITOR_COINGECKO_KEY` for D8 (D3-I-6).
