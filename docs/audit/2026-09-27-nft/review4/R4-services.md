# Round 4 re-review, area D: services, the launch monitor template and the Trust page (R4-services)

Reviewer: independent AI re-review (part D), 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at
`1eb5788`; the delta `git diff 55d8d32..HEAD -- monitor ops web-rook deployments/launch/monitor-nft.json` (the operator's round
`a37f54b` and fix round 3). Read: `review3/R3-services.md`, `bayc10-decisions.md` §3-§5, `fixes-round3.md` §1, §4 (F3-1), §6, §7,
§10 and `fixes-round3-poc/` (the claims were checked against the code, not taken); at HEAD `monitor/src/{alerts,argdiff,canary,
candid,config,ethnft,evm,ic,liveness,main,records}.mjs`, `ops/src/{ethnft,ic,runner,stage}.ts`, `web-rook/src/rook/phase1Trust.ts`,
`deployments/launch/monitor-nft.json`, `autobuyer/canister-rs/etch_autobuyer.did`, and where a service relies on it
`ethflow.rs` (`note_sent`, `give_back_now`, the `Step::Sent` outcome, `DPhase`), `custodycap.rs` (`stage0`, `first_of`,
`first_global`), `outflow.rs` (`STAGE0_VALUE_WEI`), `src/nft/EthNftWrapper.sol` (`Ticket`, `DeliverySent`, `noteSent`, `refund`).

**Nothing in the tracked tree was modified except this report and `review4/poc/D/`; nothing was committed, pushed, deployed or
signed.** Every PoC ran in a private copy (monitor, ops and the canister crate under the scratch dir; the first copy was hit by a
scratch-disk cleanup and everything was re-run from a fresh, verified copy). No network was used by this part.

Severity scale: `../README.md`. A PoC that passes shows the weakness is present (or, for a "what holds" test, that the fix holds).

## Verdict for area D

The round-3 service fixes hold. **F3-1 is fixed and has no live sibling**: a state written by the canister's own encoder, decoded by
the monitor's IDL and made plain exactly as `ic.mjs` does, is read correctly by every check that consumes it (the delivery deadline
page, the record index, the 4663 event judge including `DeliverySent`, the outbound canary's earmarks, the custody check, the floor
reading). **The monitor half of D3-H-1 is sound**: a wrapper-held item owned by someone else is exempt only for a canister delivery of
that collection, token and wrapper whose ticket, read on the wrapper, resolves to that very item and destination, and only once
mined (any owner) or while mining (owner = destination); a buy's, deposit's or stray's token is never exempt, a failed ticket read
pages, and a stale `Recorded` can no longer mask a later loss (D3-L-1 fixed; item ids are never reused). The URL redaction does what
it says for the Telegram channel, the checker errors and the argument diff. The IDLs match the `.did`; the one new variant case
(`EthStep::Sent`) is in no type the services decode.

No Critical, High or Medium. Two Lows: the one-tick false SEV1 the fixer flagged, re-rated Low (**D4-L-1**), and a new regression in
the monitor's deposit scan, which since the operator's round reads each block once and never again, so a block its provider answered
short is never re-read (**D4-L-2**). Five Infos.

## Counts (new in round 4, area D)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 2 | 5 |

Carried, not recounted: D3-I-2 (unchanged, re-rated Info, below), D3-I-5 (partly: the monitor's pair now has one provider the
canister does not use), D3-I-6 (unchanged: the template still does not name `MONITOR_COINGECKO_KEY`), D-L-3, D-L-6, D2-I-*.

## 1. Round-3 items in area D (and the fixer's own F3-1)

| ID | Sev (R3) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **D3-H-1, monitor half** | High (with C) | **Closed on the monitor side** (residuals **D4-L-1**, **D4-I-1**) | `ethnft.mjs:464-484`: a suspect must be wrapper-held (`x.item`, `:466`); candidates must match wrapper, collection and token and be in `DELIVERY_MINED` (`:48`) or `Sent`/`Cancelling` with owner = destination; `ticket(id)` is read on THAT wrapper (`:477`) and must name the same `itemId` and the delivery's destination. Tried: a Recorded delivery of the same token when it came back as a new item (item ids are array indices, never reused: pages); another ticket's delivery of the same token (item differs: pages); a ticket to another destination (pages); a ticket the wrapper will not answer (pages); a bought or deposited token at a thief (never exempt, pages). The canister-encoded state run (`audit4_nft_d` "F3-1") exempts the resold mined delivery and pages the bought token. Not checked: the ticket's `stage` (D4-I-1). |
| DeliverySent against the records | new | **Holds** | `evm.mjs:104-111`: a `DeliverySent` is backed only by the canister's delivery of that wrapper and ticket in `Moved`/`Record`/`Recording`/`Recorded` naming that tx (`word(l.data, 0)`, lower-case hex, as `hexb`). The canister signs `noteSent` only at `Moved` (`ethflow.rs:2370-2377`); a reverted note whose ticket left `Resolved` sets `sent` without a note (`:2846-2855`), which the monitor never needs to match. Controls: a note naming another tx, a mint to another address, both SEV1. |
| Deadline page for a mined delivery | new | **Holds** (text note in D4-I-3) | `ethnft.mjs:89-99`: SEV1 while `sent` is not `[true]`, SEV2 once it is (`optTrue`, `:46`, reads plain `opt bool` as `[]`/`[true]`). Verified on the canister's bytes. |
| **D3-L-1** stale Recorded masks a loss | Low | **Closed** | Review3's PoC fails on its attack assertion (`review3-pocD-rerun-on-HEAD.output.txt`, test 4). |
| **D3-L-2** balance/records race, constant key | Low | **Closed** | Balance and nonce read after the records at a later block (`ethnft.mjs:432-445`); the key names the earmarked buys. Review3's PoC fails on its attack assertion (test 5). On the canister's bytes the key is `eth-arrived-gone:<custodian>:2,4` (a buy and a sweep). |
| **D3-I-1** `since` 0 | Info | **Closed** | Test 6 fails on its attack assertion. |
| **D3-I-4** no `BackSending` page | Info | **Closed** | `ethnft.mjs` `eth-back-sending`, first seen + `BUYING_LATE_S` (the phase has no `since`). |
| **D3-I-7** `Cancelling` not exempt | Info | **Closed** | `Cancelling` with owner = destination is exempt (fixer's diag, and the control that a thief still pages). |
| **D3-I-2** a foreign tx at a cancel's nonce | Info | **Open, unchanged; stays Info** | `superseded()` is unchanged by the delta; review3's PoC still passes (test 7). It needs the canister's key, which the threat model trusts. |
| **F3-1** blobs of `plain()` state read as UTF-8 | (fixer: Medium) | **Fixed; no live sibling** | See §2. |
| Keyed-URL redaction (bayc10 §5, fixes-round3 §6) | new | **Holds for the channel; two defence-in-depth gaps** (D4-I-2) | See §3. |

## 2. F3-1 and its siblings

**What changed.** `ic.mjs` reads every query through `plain()` (`candid.mjs:248-255`): `Uint8Array` → hex text without `0x`, nat/nat64
→ decimal string, nat32/16/8 → number, principal → text, `opt` → `[]`/`[x]`. Before round 3, `ethnft.mjs` and `records.mjs` turned a
blob with `Buffer.from(b)`, which for a string is its UTF-8 bytes, so no blob of a live state compared equal. Both helpers now take
either shape (`ethnft.mjs:44`, `records.mjs:89`).

**The end-to-end check (PoC "F3-1 (what holds)").** `poc/D/canister/d4_fixture.rs` has the canister's own `candid::encode_one` write
a live-shaped `EthState` (deliveries in `Moved` with and without `sent`, `Sent`, `Recorded`; a `Bought` buy, an `Arrived` buy with the
line `Buy{2}` at a used nonce, a `Minted` buy, a sweep in `BackFunded`; a `Minted` and a `Mint` deposit; `last_mined`; a reading;
`leftover`). The monitor decodes it with its own `EthState` IDL, applies `plain()`, and runs every consumer on it:

| Consumer | Result on the canister's bytes |
| --- | --- |
| `ethNftAlerts` (deadline, late, way-back pages) | Right phase, token (`9180`), destination, tx; SEV2 with `sent`, SEV1 without |
| `recordIndex` (`ethDeliveries`, `ethMints`, `advances`) | Keys `0x<wrapper>:<ticket>`, `0x<collection>:<token>`; `to` = the module / the recipient; the sweep is not an advance |
| `judgeLogs` (`DeliverySent`, `Wrapped`, `Deposited`) | All backed, no page; the controls page |
| `earmarkOf` / the outbound canary | The plain line (`kind`, `nonce`, `value`, `max_fee`, `gas`) is read; the allowance applies; a short balance pages with the buys' ids |
| `checkEthNft` custody | Mined delivery exempt, bought token at a thief paged, deposit held |
| The floor reading (D8) | Found (`hexOf(r.collection)`); `rd.post.price` and `rd.at` are decimal strings, read with `BigInt` / `Number` |

**Siblings looked for.** Every read of canister state in `monitor/src` (`grep` for `Buffer.from`, `.eth`, `ethState`, `hexOf`,
`hexb`) and every IDL-decoded argument: `argdiff.mjs` reads `Init` (texts, `opt` through its own `opt()`; `chain_id` as a string is
only interpolated), `canary.mjs`/`records.mjs` Bitcoin paths (texts and hex already), `custodyhealth.mjs` (no blobs), `evm.mjs`
(reads logs, not state), `draws.mjs` (chain data only). `records.mjs:86` builds `module: 0x${String(b.module)}`: right for plain hex
text (production), wrong only for raw bytes, which no production path hands it. `ops` does not use `plain()`: its runner and
reconciler read the actor's raw decode with their own `hex()` over `Uint8Array` (`ops/src/ethnft.ts:250`), consistently. Nothing
else compares a blob, principal or `opt` the wrong way. **No sibling found.**

**IDLs vs the `.did`.** Monitor `candid.mjs:171-213` and runner `ops/src/ic.ts:36-67` against `etch_autobuyer.did`: the new fields
(`EthBuy.sweep`, `Delivery.sent`, `EthNftConfig.gas_pot_target_wei` / `logs_max_blocks`, and in `Init`/`INIT_FIELDS`) are present
with the `.did`'s types; `Watch.pin`, `EthBuy.arrived_at/back_at/ended_at`, `SaleScan.pend/rescan/judged`, `EthState.leftover/
sweep_after` are not decoded by the services (record subtyping skips them). `EthStep::Sent` (the one appended variant case) is only
in `HomeTx`/`Inbox`, which neither service's IDL names, so no reachable state is a decode failure.

## 3. The Dwellir key: redaction

`config.mjs:43-80`: `expandSecrets` replaces `${NAME}` in `eth_nft.eth_rpc`, `eth_nft.home_rpc`, `evm.rpcs` and
`draws.providers[].url` with the environment's value (unset refuses the config) and registers each value; `redact` replaces every
registered value; `redactUrl` prints `scheme://host[:port]` and `/…`. Where the key could surface:

- **Telegram:** `alerts.mjs` `format` redacts the whole body. Holds.
- **Checker errors** (`/status` `lastError`, the stale-checker page, `console.error("poll: …")`): redacted at `main.mjs:100`. Holds.
- **Provider errors:** every Ethereum/4663 error text names `new URL(u).host` or `host(u)` (`ethnft.mjs:261`, `abi.mjs:127-130`,
  `evm.mjs`, `draws.mjs`, `guardian.mjs`); `rpc()` errors print the JSON-RPC error object, not the URL. Node's `fetch` errors do not
  carry the URL. I found no path that puts the full URL into an alert text today.
- **Config refusals:** no message prints an `eth_rpc` URL; `draws.providers` and liveness print `redactUrl`.
- **Argument diff:** top-level `rpc_urls`/`btc_api_urls` are compared in full and printed redacted; `evm_chains` (where the canister's
  Ethereum pair lives) prints no URL at all ("reconfigured"). Holds.
- **Runner** (`ETH_LOGS_RPC`): `guarded()` errors print the host; the URL is never logged.
- **Gaps (D4-I-2):** `deliver()` keeps each alert's first line unredacted in `state.log`, which `/status` serves as
  `alerts_last_24h[].head` and the state file stores; and `redact` matches the whole value only, so a text cut (`.slice(0, 80)`,
  `.slice(0, 160)`) before it reaches `format` would show the key's prefix. Neither is reachable with today's texts; both would be the
  first leak if an error text ever echoed the URL.

## 4. New findings

### Low

#### D4-L-1: a delivery that mined but is still `Sent` in the canister pages SEV1 when the redeemer has already moved the NFT on (the fixer's "one-tick false SEV1", re-rated)

- **Where:** `monitor/src/ethnft.mjs:467-472` (a `Sent` candidate is exempt only when the owner is the destination);
  `ops/src/runner.ts:45` (the canister ticks only when the runner calls it, every `RUNNER_TICK_EVERY_S` = 600 s by default; the
  canister has no timer of its own, so `Sent → Moved` waits for the next tick).
- **Mechanism:** the transfer mines about 12 s after it is sent. Until the canister's next tick (up to 10 min) the record still says
  `Sent`. A redeemer who sells the NFT into a bid or sweeps it to another wallet in that window (what an arbitrage bot does, and the
  expected use when a unit trades under the floor) leaves the owner ≠ destination, so `eth-not-held` pages SEV1 "pause, then look".
  The page clears itself at the next tick.
- **Impact:** a false SEV1 on an honest redemption, recurring with fast resellers: alert fatigue on the one page that means "custody
  lost". It is narrower than round 2's D2-M-1 (every redemption; Medium): only redeemers who move within one tick. Muting is bounded:
  the key is per token (`eth-not-held:<collection>:<token>`) and a persistent key repeats after 6 h, so a real loss is muted only if
  the same token comes back into custody and is stolen within those 6 h.
- **PoC:** `poc/D/monitor/test/audit4_nft_d.test.mjs` "D4-L-1" (passes: SEV1, and no receipt read).
- **Fix:** for a `Sent` (or `Cancelling`) candidate whose owner is not the destination, read `eth_getTransactionReceipt(tx)` on both
  providers and exempt when it succeeded and holds the collection's `Transfer(custodian → destination, token)`, as the canister does
  at finality; or grace a `Sent` suspect for one tick interval before paging.

#### D4-L-2: the incremental deposit scan never re-reads a block its provider answered short (a lagging node behind the provider's load balancer, a reorg at the tip)

- **Where:** `monitor/src/ethnft.mjs:390-397` (since `a37f54b`: `from = done + 1`, `next.scanned[key] = head`); `head` is
  `eth_blockNumber` on `eth_rpc[0]` (`latest`), and the logs are asked of the same URL in 500-block ranges (`logsInRanges`, `:29-36`).
  Before the operator's round the whole 7,200-block window was re-read every poll, so a short answer healed a minute later.
- **Mechanism:** `eth_blockNumber` and `eth_getLogs` are separate requests. Behind one URL (Dwellir, the template's first provider,
  is a load-balanced service) they can be answered by different nodes; a node a block or two behind returns the logs up to its own
  head without an error, and the cursor still moves to the head the other node reported. Likewise, a deposit included in a block that
  replaced one at a height already scanned (a tip reorg) is never read. The transfer is then never in `unclaimed`, and
  `eth-unclaimed` never fires for it.
- **Impact:** D5's backstop (an NFT sent to the custodian that the runner never hands in) can miss a deposit for good. The runner's
  own discovery still re-reads its whole window every pass (`ops/src/ethnft.ts:236-246`), so this needs a second failure (the runner
  down or misconfigured, the case the watch exists for). No value moves.
- **PoC:** `audit4_nft_d.test.mjs` "D4-L-2": the head from a node at H, the logs from one at H − 2, a deposit at H − 1; 700 blocks
  later nothing pages. Control: a whole-window read (a fresh start, or the code before `a37f54b`) pages it.
- **Fix:** scan only up to `head − N` (N ≥ 64, or the finalized tag), or re-read an overlap of N blocks each poll (`unclaimed` already
  deduplicates by tx hash), or re-read the whole window every hour.

### Info

- **D4-I-1 (the exemption does not read the ticket's stage):** `ethnft.mjs:477-478` checks the ticket's `itemId` and `destination`,
  not `stage` or `sentTx`. A ticket refunded after the late window (stage `Refunded`; its item back in the set, still wrapper-held)
  whose canister delivery is still `Moved` exempts that item while the NFT is elsewhere (PoC "D4-I-1"). That needs the 30-day refund
  of a delivery whose record never landed (D3-H-1's loss, now closed on the contract by `noteSent`), and the deadline SEV1 pages it
  meanwhile; the canister moves such a delivery to `Gone`, which is not exempt. Hardening: require `stage == Resolved`, and for a mined
  delivery `sentTx(ticket)` either 0 or the delivery's tx.
- **D4-I-2 (redaction, defence in depth):** see §3. `alerts.mjs:97` keeps `head` unredacted in `state.log` (served on `/status`, stored
  in the state file); `redact` recognises the whole value only. Fix: `redact` the head in `deliver`, and redact before any
  `.slice`. The template's comment ("scrubbed from every alert and status line") should then hold as written. PoC "D4-I-2".
- **D4-I-3 (page texts):**
  - the `Stuck` page (`ethnft.mjs:64-65`) says every stuck buy "is no longer watched and is off the Relay leg" and that
    `eth_give_back` closes it "as a loss … once it has been stuck 24 h". A forward-leg `Stuck` (Bridging past its deadline) is still
    watched, holds the leg, and `eth_give_back` returns ETH for it (`ethflow.rs:3012-3019`). No 24 h wait exists in `give_back_now`
    for an unwatched `Stuck`;
  - the `Arrived` page counts hours from the phase's `since`, while the canister's 48 h are counted from `arrived_at` (E3-L-1), so
    after a failed fill the page understates the wait;
  - a sweep's `BackFunded` / `Returned` pages say "is going BACK (it could not fill)" and "nothing was bought";
  - `Delivery.sent = true` also means "nothing to note: the ticket left `Resolved`" (`ethflow.rs:2372-2374, 2847-2851`). The deadline
    page reads it as "noteSent is on 4663" (only while the delivery is not yet `Recorded`/`Gone`).
- **D4-I-4 (ops stage labels above stage 0):** `ops/src/stage.ts:22-40` mirrors J7 as 24 × 4^s items and 15 × 4^s ETH per Ethereum
  collection at every stage, so the reconciler calls 60 ETH / 96 items "stage 1". `launch-gates.md` G7 changed only stage 0's row
  (stage 1 is still "12 items, ≤ 40 ETH"). At stage 0 the mirror is exact (15 ETH, ≤ 24 items; the global the collections' sum,
  61 items / 75 ETH at launch). Pick one stage-1 rule for Ethereum collections before the first raise.
- **D4-I-5 (Trust page wording):** `phase1Trust.ts:148` says "a delivered NFT can never be refunded as well". That holds once
  `noteSent` has landed and while the canister does not mark a mined delivery undeliverable (the wrapper still refunds after
  `markUndeliverable`, by design). Say "once noted sent". `:65` says "at most the custody caps lost (75 ETH for the five)". The caps
  count items at their floor when admitted, so the NFTs' market value can exceed that after a rise. The remaining changed statements
  match the code: J7's counts, the 12/24 deliveries a day of the NFT install, mints 3, 25 ETH a day, deposits counted at the floor,
  the way back's 2 h, 24 h and one-day cooldown, "Relay's estimated fee", "48 hours after it first arrived", and the first post's
  three counted sales. The sale-scan sentence ("a flood of transfers cannot hide a sale") is part B's to confirm.

## 5. The operator's round, service side (J7, H3, Dwellir, the sweep)

| Item | Finding |
| --- | --- |
| `argdiff.mjs` J7/H3 mirror | `STAGE0_EVM_CAP` (6 default, 24 first, 15 ETH), `firstGlobal` (the sum, at least 6 / 10 ETH) and `STAGE0_LIMITS.global_value_wei` 25 ETH equal `custodycap.rs:23-37, 185-206` and `outflow.rs:27, 108-110`. The EVM key `<chain>:0x<lower>` matches `evm_key`. |
| `ops/src/stage.ts` | Exact at stage 0; above it D4-I-4. |
| Runner `ETH_LOGS_RPC` / `ETH_LOGS_MAX_BLOCKS` | The URL is only fetched, never logged; the range is bounded 10-10,000 (default 500); the discovery re-reads its 7,200-block window each pass (15 requests per collection, 75 a pass for five). |
| Monitor `logs_max_blocks`, the deposit scan | Bounded 10-10,000; ranges of 500. The incremental cursor is D4-L-2. |
| The sweep | Not an advance in the record index (`records.mjs:84`); `fundReserve`'s `ReserveFunded` is not judged (inbound value); `returnAdvance(0)` of a closed loss matches `AdvanceReturned` by phase (`evm.mjs:155-163`); a sweep's ETH is earmarked while `BackFunded`/`BackSending`, correctly (it is at our address, and a sweep starts only when nothing else there is anyone's). |
| Template `monitor-nft.json` | Dwellir (`${DWELLIR_API_KEY}`) + `xrpc.cl/eth`; `logs_max_blocks` 500. The canister's pair in the dry run is `xrpc.cl` + `publicnode`: the monitor shares `xrpc.cl` with it, so exactly one of its two providers is independent of the canister (D3-I-5 partly addressed; the owner check needs both to agree, so a lying shared provider shows up as disagreement, SEV2). `MONITOR_COINGECKO_KEY` is still unmentioned (D3-I-6). |

## 6. PoCs and outputs (`review4/poc/D/`)

| File | Findings | Result |
| --- | --- | --- |
| `canister/d4_fixture.rs` (+ `d4_fixture.output.txt`) | the live-shaped state from the canister's own encoder | writes `monitor/test/d4-state.hex` (6,520 hex chars; byte-identical on the re-run) |
| `monitor/test/audit4_nft_d.test.mjs` | F3-1 (what holds), D4-L-1, D4-L-2, D4-I-1, D4-I-2 | **5/5 pass** (`monitor/audit4_nft_d.output.txt`): F3-1 holds; each weakness present |
| review3's `audit3_nft_d.test.mjs` + the fixer's `audit3_nft_d.diag.test.mjs`, on HEAD | D3-M-1, D3-L-1, D3-L-2, D3-I-1, D3-I-2 | `monitor/review3-pocD-rerun-on-HEAD.output.txt`: D3-L-1, D3-L-2 and D3-I-1 fail on their attack assertions (fixed). D3-M-1 (a)/(b) still pass for the harness reason the fixer gives (no `ticket()`), and the diag with `ticket()` answered passes. D3-I-2 passes (open, Info) |

Run: copy `monitor/` whole, put the two files in its `test/`, `node --test test/audit4_nft_d.test.mjs`. The fixture: copy
`autobuyer/canister-rs` (no `target/`), add `d4_fixture.rs` to `tests/`, `D4_OUT=<file> cargo test --test d4_fixture` (toolchain
1.98.1).
