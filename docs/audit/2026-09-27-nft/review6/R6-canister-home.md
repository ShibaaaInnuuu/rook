# Round 6, part F: the home line after F5-M-1 (nonce identification, effect read-backs, `eth_home_clear`, adoption)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `e04fa16`
(delta `8ef82d6..e04fa16`, canister commit `e811911`).

**Scope:**
- F5-M-1's fix:
  - `ethflow.rs`: `resolve_home` (`:3472-3602`), `home_effect` (`:3645-3683`), `transit_open`, `wrapper_holds`, `adopt_advance`
    (`:3606-3643`), `record_done`, `mark_done`, `note_sent` (`:2985-3048`), `home_act`'s `Final`, `Returning` and `NeedAdvance` arms
    and step 3 (`:3216-3455`), `home_clear` (`:4183-4196`);
  - `canister.rs` `home_nonce_tx` / `nonce_at` / `our_txs_view` (`:1040-1085`), `evmsend.rs` `our_txs_body` (`:139-155`), `ethnet.rs`
    `eth_home_clear` (`:615-620`);
- E5-L-1 (step 3's `own`), F5-L-1 (`chain_moved`, `canister.rs:568-576`), F5-I-1 carried;
- the monitor's `eth-home-unseen` (`monitor/src/ethnft.mjs:110-118`) where it bears on the operator's exit;
- the contracts read for what the read-backs rely on (`EthNftWrapper.sol` `_admit`, `recordDelivery`, `noteSent`; `FloorModule.sol`
  `advance`, `settleAdvance`, `returnAdvance`). Contracts unchanged.

**Nothing in the worktree was modified except this report and `review6/poc/F/`. Nothing committed, pushed, deployed or signed.**
- **PoCs:** ran in `/tmp/rook-r6-F` (an APFS clone of `autobuyer/canister-rs`, `diff -rq` identical before the PoC file was added;
  removed afterwards).
- **Network (read-only):** `eth_blockNumber`, `eth_getTransactionCount` and `eth_getBlockByNumber` probes of the canister's 4663 pair
  (`rpc.mainnet.chain.robinhood.com`, `rpc.ordofi.network`) and `robinhood.drpc.org`.

Severity scale and threat model: `../README.md`.

## Verdict

**F5-M-1 is closed.**
- A value-bearing home step is never done again on absence of evidence.
- What decides is one of:
  - a positive identification (our nonce count and the block's transactions, every provider agreeing);
  - a read-back of the step's own effect at a pin past the nonce, which is stable while the step is pending (below);
  - the operator's word, for the two steps that leave nothing readable.
- No single provider, wrong or malicious, can cause a double pay or a wrong adoption. Every read that decides goes through `rpc` /
  `rpc_agreed` / `rpc_view` and must agree on every provider; a disagreement is an error, and an error is a wait.

**Two new Lows are residuals of the fix, not regressions:**
- **F6-L-1:** at the runner's default cadence, the identification is out of the canister's 4663 provider's state depth. A hidden Relay
  deposit or `fundReserve` therefore always waits for the operator.
- **F6-L-2:** the likelier index failure, ONE provider missing or differing on the receipt, reads as `Unsure`. That path never
  identifies, never starts the clock, has no operator exit and pages nothing specific.

**The rest:**
- E5-L-1 is closed, with Info F6-I-3.
- F5-L-1 is closed for a direct move, with Info F6-I-1 (a two-step move).
- F5-I-1 is unchanged (Info).

| ID | Sev | Title |
| --- | --- | --- |
| F6-L-1 | Low | The nonce identification is out of the providers' state depth at the runner's cadence: every hidden Relay deposit / `fundReserve` goes to the operator |
| F6-L-2 | Low | A receipt the providers disagree on (`Unsure`) halts the 4663 line with no identification, no clock, no `eth_home_clear`, no `eth-home-unseen` page |
| F6-I-1 | Info | F5-L-1's reset misses a two-step move (drop `eth_nft`, re-add on another chain) |
| F6-I-2 | Info | `eth_home_clear` accepts, and the SEV1 page asks for, a word on a state-bearing step that the canister never applies; refused while paused, when the paused pass's records and marks wait behind the same pending |
| F6-I-3 | Info | E5-L-1's `own`: a mint estimate that refuses for the whole collection (a paused listing, `MintRateLimited`) admits every waiting deposit to the custody book |
| F6-I-4 | Info | `adopt_advance` goes on when the custody room cannot be reserved again (the minted item is then outside the book), and re-records H3 outside the window |

## F5-M-1: verified point by point

1. **Identification (`canister.rs:1053-1085`).**
   - What it does:
     - `nonce_at` is `rpc(…)`, all providers must agree (`:804-816`);
     - the bisection runs from `HomeTx.pin` (the signing tick's pin: EthNet caches it at the tick's start, `canister.rs:1304-1311`) to a
       fresh `pin()`, which the pin floor keeps at or above the tick's;
     - the block is read with full transactions through `our_txs_view`, filtered by `from == us`, with `[hash, nonce]` compared across
       providers.
   - Wrong answers:
     - `hi` holding no transaction of ours at the nonce is an error, never a decision;
     - `Ok(None)` ("not used at the search's pin") only restarts the clock and re-sends the same bytes.
   - **Ours ⇒ MINED; another of ours ⇒ ours can never mine.** Both are sound, since one nonce means at most one mined transaction and
     the hash is the keccak of our own raw bytes.
   - Cost: at most ~13 nonce reads per unseen tick at the real depth (below), then one block; acceptable.
2. **The effect read-backs (`home_effect`).** Each is a read at the tick's pin. `resolve_home` enters this branch only when
   `home_nonce()` at that same pin is past the nonce (`:3482-3483`), so the transaction, if mined, is at or below the pin.
   - **Can an effect read "absent" although the step happened (a double pay)?** Only if something undid it between its block and the
     pin. Nothing can while the step is pending: the home line has one transaction out, and the undoing calls are ours alone and come
     after resolution.

     | Step | What undoes it | Only by |
     | --- | --- | --- |
     | `advance` | `settleAdvance` / `returnAdvance` (`FloorModule.sol:715-742`) | the custodian (us) |
     | a mint's `itemOf` | `recordDelivery` (`EthNftWrapper.sol:392-399`) | us |
     | a ticket's `Delivered` | nothing | (terminal) |
     | `sentTx` | a later `noteSent` | us |
     | `undeliverable` | the holder's refund, after which `mark_done` sees "not Resolved" and ends it | `Gone` (safe) |
   - **Can an effect read "present" although the step did not happen (stuck, or ETH unaccounted)?** Only if another actor produced the
     same state:
     - `transitUnits` is opened only by the custodian's `advance`, which reverts `AdvanceOpen` if one is open;
     - `itemOf(token) != 0` for a token we bought and still own on Ethereum could come only from our own mint (`sourceUsed` also
       dedupes it);
     - for a deposit mint, the item's `sourceTx` must be this deposit's.

     One advance per module and at most one open buy per collection (`open_buy_for`, `buy_pass:1793`; `Stuck` is not `finished`), so
     `transit_open(module)` is this buy's.
3. **Adoption of an earlier build's wrong drop.**
   - `NeedAdvance` with `transit_open` ⇒ `adopt_advance` at the module's own `advanced()`.
   - `Returning` with transit closed ⇒ `Returned`, nothing paid.
   - `Final` with the token on the wrapper ⇒ minted.
   - `Record` with the ticket `Delivered` ⇒ recorded and released.
   - A mark or note already on the wrapper ⇒ done.

   **Can adoption give an advance or token to the wrong buy or collection?** Not found:
   - the advance is the collection's own module's, and only this collection's one open buy can be `NeedAdvance`;
   - the C4-M-1 credit paths that set `Returning` either have the advance still open (`(true, true)`, `:4125-4128`) or are sweeps
     (`fundReserve`, excluded from the transit check);
   - a buy dropped by `retain` never had an advance.

   Residual: F6-I-4.
4. **The clock.** `Pending` and `Unsure` reset `used_since` (`:3583-3600`), and "seen nowhere" is every read in a row. Confirmed by the
   fixer's `r5_f5_m1_d` and by my F6-L-2 run: `used_since` stays `None` through a day of `Unsure`.
5. **`eth_home_clear` (`ethnet.rs:615-620`, `ethflow.rs:4183-4196`, applied at `:3565-3572`).**
   - **Who can call it:** `only_operator` (the operator principal or a controller), refused while paused or mid-tick. The source test
     `every_update_method_here_is_operator_only…` counts it.
   - **Preconditions:** a pending transaction at exactly that nonce, unseen with its nonce used for ≥ 1 h.
   - **What it can authorize:** only a step with no readable effect (a Relay deposit or a sweep's `fundReserve`; a post is dropped by
     itself), and only on a tick where the chain still cannot tell: identification errs and the effect is `None`. A chain answer on a
     later tick wins over the operator's word.
   - **Stale words:** a word left behind when the chain answers first lingers in `inbox.home_clear`. It can never match again, because
     the nonce floor (`note_home_nonce`) keeps every later home transaction above it.
   - **A wrong word:**
     - a wrong "not mined" pays one deposit (at most the collection's `max_purchase_wei`, 8 ETH for BAYC) or one sweep twice. This is
       the fixer's judgment call 1, bounded;
     - a wrong "mined" for a deposit that did not go out self-corrects: the watch reads the unspent value as a refund
       (`relayconv::refunded` over `balance_after`) and the buy goes back.

   **It cannot create or widen a state, and it is not usable by anyone but the operator.** Residual: F6-I-2.
6. **Provider answers.** A single provider that lies or lags only produces disagreement, which is an error and a wait. Both providers
   agreeing on a false chain is outside the threat model, as every read.

## F6-L-1 (Low): the nonce identification is out of the providers' state depth at the runner's cadence

- **Code:**
  - `canister.rs:1059-1062`: `lo = from` (`HomeTx.pin`), and `nonce_at(&us, lo)` must succeed;
  - `ethflow.rs:2976`: `HomeTx.pin` is the signing tick's pin, cached at the tick's start;
  - `ethflow.rs:3496`: the identification runs only from the first tick that finds the nonce used;
  - `ops/src/runner.ts:192-195`: the runner sleeps `RUNNER_TICK_EVERY_S` (default 600 s) after each cycle ends.
- **Measured (read-only, `poc/F/depth_probe.output.txt`, 2026-09-28):**
  - 4663 makes **10 blocks a second** (6,200 blocks = 622 s).
  - `rpc.mainnet.chain.robinhood.com`, the canister's first 4663 provider (`scripts/golive-dryrun.sh:115`), serves state **6,100
    blocks back and not 6,300** ("historical state … not available"), i.e. about **10 minutes**. It serves block bodies at any depth.
  - `rpc.ordofi.network`, the second, is an archive.
  - (`robinhood.drpc.org`, which `scripts/golive-dryrun.sh:728` notes was dropped, answers no state read at any block number.)
- **So:**
  - the first resolving tick comes 600 s plus one runner cycle after `HomeTx.pin`: at least 6,000 blocks, more with the tick's own
    duration and the pin's lag;
  - that is at or past the edge on the first tick, and past it on every later one;
  - `nonce_at(lo)` errors on the first provider, and `rpc` fails the whole read.
- **Effect:** the positive identification, the fix's primary path, practically never succeeds at the default cadence.
  - State-bearing steps fall through to their read-back, which is correct.
  - **A hidden Relay deposit or `fundReserve` is always `UNIDENTIFIED`:**
    - the whole 4663 line waits (the frog side's too, `others_pending`) until the SEV1 page is answered with `eth_home_clear`;
    - a wrong word pays twice (bounded, F5-M-1 call 1).
  - `fixes-round5.md` §4 names the 6,200-block depth, but not that it is ~10 minutes, under one runner interval.
- **Trigger:** our transaction mined and neither provider shows its receipt or its transaction by hash. That is provider failure, with
  no attacker, and rare. Liveness only: no double pay without an operator's wrong word. Hence Low.
- **PoC:** `poc/F/r6_f.rs` (harness diff):
  - `r6_f6_l1_at_the_runners_cadence_…` (600 s + 60 s ticks, 6,200 blocks of state: a day of `UNIDENTIFIED`, the chain able to tell
    throughout);
  - control `r6_f6_l1_control_…` (a 60 s cadence, or an archive provider: MINED at the first tick).
- **Fix (any one):**
  - Identify from block bodies, which every provider serves at any depth, instead of state: record the pin right after the broadcast
    and scan `eth_getBlockByNumber(n, true)` forward from `HomeTx.pin` through `our_txs_view` for our `[hash, nonce]`. The sequencer
    includes a transaction within seconds; bound the scan (e.g. 600 blocks a tick, resumable).
  - Or bisect only within the reachable depth: start at `max(HomeTx.pin, pin − 5,000)`, and when the nonce is already used there, fall
    back to the block scan.
  - Or run the identification in the same tick as the broadcast's first unseen read (it is cheap).
  - And state the depth in minutes in `phase1-nft.md` §5.1.

## F6-L-2 (Low): a receipt the providers disagree on halts the 4663 line, with no identification, no exit and no specific page

- **Code:**
  - `canister.rs:992-997`: one provider's receipt differs from the other's, or one is null, gives `EvmSeen::Unsure`;
  - `ethflow.rs:3589-3600`: `Unsure` resets `used_since` and returns;
  - `:3481`: the identification runs only on `NotYet`;
  - `ethflow.rs:4188-4191`: `home_clear` needs `used_since` ≥ 1 h;
  - `monitor/src/ethnft.mjs:114-118`: `eth-home-unseen` also keys on `used_since`.
- **Why it matters:**
  - F5-M-1's trigger is "no provider shows it". An index failure on ONE of the two providers is the likelier case.
  - While it lasts, the pending transaction is `Unsure` every tick. The step is never resolved (not even when mined and identifiable),
    `eth_home_clear` is refused ("not yet seen nowhere"), and nothing else is signed on 4663:
    - posts: the module goes stale and mints and burns wait;
    - deposit mints;
    - records and marks, including the paused pass's.
  - The only exit is a timelocked provider change (72 h).
  - Detection: generic symptoms only, e.g. the no-good-post pause after 6 h.
  - Unchanged in kind from round 4. Round 5's fix covers "seen nowhere" but not "seen differently".
- **PoC:** `r6_f6_l2_one_provider_that_disagrees_…`. A mined deposit, `unsure` for a day with `identify` on: `used_since` never set,
  `eth_home_clear` refused, no `postFloor`, `mintDeposit` or `recordDelivery` signed, identification never tried.
- **Fix:**
  - On `Unsure` with the nonce used at the pin (`home_nonce() > p.nonce`), run the same identification and effect read-back as
    `NotYet`. Keep the clock rule, so a drop still needs an hour of "seen nowhere".
  - And/or let `eth_home_clear` (and the page) apply after an hour of any non-`Mined` view with the nonce used.

## F6-I-1 (Info): F5-L-1 misses a two-step chain move

- **Code:** `canister.rs:571-576` passes `from = saved_init.eth_nft.chain_id` and `to = arg.eth_nft.chain_id`, and `chain_moved` resets
  only on `(Some(a), Some(b)), a ≠ b`.
- **The case:**
  - An upgrade that drops `eth_nft` is allowed when nothing is open (`stranded_by` with `next = None`, B-L-5's check). It reads
    `(Some(1), None)`.
  - A later upgrade that adds it on another chain reads `(None, Some(b))`.
  - Neither resets, so the old chain's pin and nonce floors and `last_mined` refuse every pin of a chain whose head is lower, for good.
    Undoing it takes another timelocked move.
- **Rating:** Info, an operator configuration sequence.
- **PoC:** `r6_f6_i1_…`.
- **Fix:** keep the floors' chain id beside them (`PinFloors.eth_chain`), and reset whenever it differs from the configured chain,
  whatever the previous Init said.

## F6-I-2 (Info): `eth_home_clear`'s scope, as the endpoint and the page state it

1. **The endpoint over-promises for state-bearing steps.**
   - For a state-bearing step (an advance, a mint, a record…) whose effect read keeps failing, or whose identification succeeded but
     whose effect is unreadable, `used_since` ages past the hour. The page then says "if it mined, `eth_home_clear(n, true)`".
   - The endpoint accepts and answers "is taken as MINED at the next tick", but the word is applied only on the `Ok(None)`-effect
     branch (a deposit or a `fundReserve`). It is never used for that step and lingers.
   - The page and the endpoint should say which steps it applies to, and refuse the others.
2. **It is refused while paused.**
   - The paused pass signs only when no home transaction is out (`ethflow.rs:955`).
   - An `UNIDENTIFIED` deposit during an H1 pause therefore holds every record and mark (holders' refund path) until the canister is
     resumed. Only then can the operator clear it.
   - Allowing `eth_home_clear` while paused would sign nothing by itself.

## F6-I-3 (Info): E5-L-1's `own` admits every waiting deposit when the whole collection's mint refuses

- **Code:** `ethflow.rs:3429`: the custody `admit` comes before `send_home`. `:3434-3443`: any `send_home` error is `own`, so the next
  deposit of the collection is tried.
- **When** every mint estimate of the collection reverts, the loop admits each waiting deposit into the custody book and mints none.
  Causes:
  - a registry pause of the listing (`ListingPaused`);
  - the wrapper's `MintRateLimited`, whose day runs on two chain clocks while H3's runs on the canister's;
  - also a transient `home_sign` failure (fees, nonce, disagreement).
- **Effect:**
  - the room is held by unminted deposits, so the collection's purchases see no room (`check_acquire`);
  - a later deposit past the room is sent back;
  - the admissions are the same deposits' eventual room, so nothing is lost or over-admitted beyond the cap.
- **PoC:** `r6_f6_i3_…` (4 waiting, 4 admitted, 0 minted).
- **Fix:** treat only an estimate revert naming the deposit itself (`AlreadyHeld` / `AlreadyMinted`) as `own`, and anything else as
  the collection's wait. Or admit after the signature succeeds (release on failure).

## F6-I-4 (Info): `adopt_advance` with no custody room, and H3 recorded again

- **Code:** `ethflow.rs:3606-3625`. When `custodycap::reserve` fails, the error goes into the line only, and the buy is funded and
  bought anyway. At its mint, `settle(eth:{id}, Some(held))` fails (`mined_home` Mint arm), so the item is custody the book does not
  count, and the exit rule's premise loses one item.
- `outflow_record` under the original key is idempotent only while that act is still in the limiter's window.
- **Reachable only from a state an earlier build's wrong drop left.** Nothing is deployed.
- **Fix:** on a failed reservation, send the adopted advance back (`Returning`) instead of buying.

## Other items re-checked

- **E5-L-1 closed.**
  - A mint that fails its own estimate moves to the next deposit.
  - A deposit whose token is on the wrapper from its own transaction is adopted: `wrapper_holds(…, Some(tx))` compares the item's
    `sourceTx`, and H3 is recorded once under the same `mint_key`.
  - The drop path decides by `home_effect`, and a read error is a wait.
- **F5-I-1 unchanged (Info).** `looper.rs:320`, `redeem.rs:610-613` and `:1064` and `frogflow.rs:2012` still wait with no exit on the
  same trigger. Through `others_pending` they also hold the NFT line. `home_nonce_tx` could serve them, together with F6-L-1's block
  scan.
- **F5-I-2 .. F5-I-5:** not re-attacked beyond confirming the code is as round 5 left it (F5-I-5's dangerous drops are tested by the
  fixer's `r5_f5_m1_*`).
- **Upgrade safety of this area:** `HomeTx.pin` and `used_since`, `Inbox.home_clear`, `Watch.home_pin` and `LateOrder.released_at` are
  all `Option` with `#[serde(default)]`. A pending transaction saved without `pin` searches `HOME_NONCE_SEARCH_BLOCKS` (100,000 blocks
  ≈ 2.8 h at 4663's rate) back. That is beyond the first provider's depth, so it identifies nothing and decides by effect or the
  operator (F6-L-1).

## PoC results

`cargo +1.98.1 test --test r6_f r6_`: **5 passed** (each asserts its weakness). The whole copied harness: 38 of 38. The switches leave
the fixer's 33 tests unchanged. Outputs: `poc/F/r6_f.output.txt`, `poc/F/depth_probe.output.txt`.
