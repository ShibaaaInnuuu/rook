# Round 3, part C: the advance gate (A2-M-1), the way back (Relay Ethereum → 4663) and C2-M-2

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `4949da0`
(round-2 fix delta `79455e1..4949da0`). Scope: `ethflow::advance_covers` and where it is checked; the new phases `BackFunded`,
`BackSending`, `BackBridging` (and `Returning` on the way back); `EthKind::BackDeposit`; `back_now`, `give_back_now` / `eth_give_back`;
`ARRIVED_BACK_SECS` / `ARRIVED_HOLD_SECS`; `eth_act` step 3, `mined_eth`, `step_buy`, `buy_pass`, `home_act`; `relayeth::judge` with
`FROM_ETHEREUM`; `ops/src/ethnft.ts` `preJudgeEth` for both legs; the `.did` additions; the fixer's tests `r2_a2_m1_*`, `r2_c2_m2_*`.
`src/floor/FloorModule.sol` (unchanged) was read for `advance` / `returnAdvance`. Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review3/poc/C/`; nothing committed, pushed, deployed or signed.**
Every PoC ran in a private copy of `autobuyer/canister-rs` (scratch dir), on the fake world of `tests/audit_nft_r2.rs` (copied whole),
with one addition for C3-H-1: the canister's real H3 limiter (`outflow::Outflow::check` / `record`, as `canister::outflow_check`
calls it, a freeze setting `paused`).

Severity scale: the README's.

## Verdict

The gate does what it says: a module whose `item_cap` (after 0.5 % and the keeps) does not cover `accepted × 1.01 + 0.003 ETH`
never starts a buy and never advances, and a buy whose floor rose after it started is dropped before the module pays. The way back's
judge, destination, amounts, earmarks, replay and nonce handling hold: I found no path that returns ETH twice, spends another buy's
earmark, lets a fill and the way back both spend the same ETH, or routes the deposit anywhere but our own address. **C2-M-2 is
closed.**

But the round leaves or adds four problems in this path:

- **C3-H-1 (High):** an advance that waits on H3's daily value limit is dropped and re-opened every tick under a new outflow key
  (`buy:eth:{id}`). Each new key is a new ask in the tripwire's demand, so within about four ticks the tripwire freezes custody. That
  pauses the whole canister, and it resumes only through the controller (the timelock and k guardians after handover). While it is
  paused no floor is posted, so every module goes stale within its 1 h TTL and refuses mints and burns. At stage 0 this takes one
  BAYC advance (7 ETH) that comes back within 24 h (three Relay refunds, or the operator's `eth_give_back`, which the monitor's own
  page text suggests) while the module still wants its ape. The code is from fix round 1 (`ed76e9d`), not the round-2 delta; the
  round-2 way back and its `eth_give_back` add triggers.
- **C3-M-1 (Medium):** the way back has no exit. `BackFunded` blocks every collection's new buys (`buy_pass`), and `BackBridging`
  holds the canister-wide Relay leg. `eth_give_back` refuses both, and a `BackFunded` buy can no longer take a listing. So one way
  back that gets no judgeable quote (Relay has no route or inventory for the amount, the fee is over the bound, or the runner
  withholds it), or that Relay neither fills nor refunds, stops every collection's conversions until an upgrade.
- **C3-L-1:** the gate assumes Relay costs 0.5 %, but the judged quote may take up to 3 % (`relay_max_fee_bps`). Relay's own measured
  quote promises only 97.7 %, so an order filled at its minimum lands under the floor that the gate passed. The ETH then goes back
  after 48 h (two fees), and the gate lets the next advance through unchanged.
- **C3-L-2:** the runner holds the operator identity, so it can call `eth_give_back` on any `Arrived` buy at once. With no cooldown,
  it can churn a module's ETH through Relay round trips at about 1 % of an item each. H3's value limit bounds this, and then C3-H-1
  freezes the canister.
- **C3-L-3:** any inflow to our 4663 address of at least half the order's minimum, arriving during `BackBridging`, is taken as the
  arrival and returned to the buy's module. Relay's real payout is then orphaned. This extends B-L-6: a frog-side advance does it
  whenever both sides run.

## Counts (this part: new, plus round-2 items not actually fixed)

| Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- |
| 0 | 1 | 1 | 3 | 6 |

## Round-2 items in scope

| ID | Sev (R2) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **A2-M-1**: pooled advance under the floor, stranded on Ethereum | Med | **Closed as reported.** New issues in the fix: C3-M-1 and C3-H-1 (triggers); residuals C3-L-1, C3-L-2, C3-L-3 | `advance_covers` (`ethflow.rs:989-1000`) is checked in `buy_pass` before a buy exists (`:1055-1059`) and again right before `advance` (`home_act` `:2212-2217`, where a buy that fails is dropped with nothing paid). `accepted` is `floor()`'s first word (`floorsink::reads()[3]`, and `FloorModule.floor()` returns `accepted`); `item_cap` = `min(maxItemCost, max_purchase_wei)`. The r2 test's two cases are real (pooled floor over the blend; the floor rising between the buy and the advance). Launch numbers: Pudgy's `maxPurchaseWei` of 4 ETH covers the gate up to a 3.94 ETH floor (it is 3.353); BAYC's 8 ETH covers up to 7.88 (it is 6.79). Relay's real fee vs the 0.5 % estimate: C3-L-1. Whether the gate can starve a pool: C3-I-1 (liveness only; burns and redemption are unaffected). |
| **C2-M-2**: an `Arrived` buy holds its collection's deposits indefinitely | Med | **Closed** | `buy_holds_mint` (`:508-515`): an `Arrived` buy holds for 6 h at most; `Stuck` and `Back*` hold nothing. `r2_c2_m2_*` is real (the deposit mints between 6 h and 6 h 40 min; the buy is `BackFunded` at 48 h). The review2 PoC re-run in `fixes-round2-poc/` fails at a **harness** `put_listing(..).unwrap()` (the buy leaves `Arrived` at 48 h), not at the attack assertion (C3-I-5). With that one call made tolerant, the PoC prints `deposit Minted` and fails its attack assertion: the fix holds. The new operator exit (`eth_give_back` on `Arrived`) is C3-L-2's lever. |
| B-L-6: the shared 4663 balance, frog side vs eth side | Low | **Open, now wider** | The way back reads its **arrival** from the same balance and pays it out with `returnAdvance` (C3-L-3). Still unreachable in the NFT-only stack. |
| B-L-3 / C-M-2: an unmined line transaction and cancels | Med | **Not extended to the new kind** | `BackDeposit` is re-priced but never cancelled (`resolve_eth` `:1766-1780`): C3-I-2. |

Re-run of `review2/poc/C/audit2_nft_c.rs` with `fixes-round2-poc/audit2_nft_c.compile-adaptations.diff`
(`poc/C/review2-poc-C-rerun.output.txt`): 1 pass (C2-L-3's probe, as the fixer reports) and 4 fail. Three fail on their own
attack or mechanism assertions: C2-L-1 ("the watch survives the arrival"), C2-M-1 (the quote now waits: `Funded`, not
`Depositing`), and C2-L-2 (Azuki's buy starts). C2-M-2 fails on the harness, as above. Part E owns C2-M-1 and C2-L-1/-2/-3.

## New findings

### C3-H-1 (High): an advance waiting on H3's value limit is re-asked under a new key every tick; the tripwire freezes and pauses the canister

- **Where:**
  - `ethflow.rs:2222-2230` (`home_act` `NeedAdvance`: `outflow_check("buy:eth:{id}", …, Flow::Value, amount)`; on a wait the buy is
    **dropped**);
  - `ethflow.rs:1039-1072` (`buy_pass` opens a new buy, with a new id, on the next tick: it checks H2, not H3);
  - `outflow.rs:194-195` (each new key is a new ask) and `:206-220` (the tripwire: demand ≥ `trip_factor` × `global_value_wei`,
    Value having no per-collection scope);
  - `canister.rs:1600-1614` (`Frozen` sets `PAUSED`) and `:1580-1595` (only a controller resumes: after handover, k guardians
    through the timelock's `resume_target`).
- **Mechanism:**
  - `Outflow::check` counts an ask "once per key, so that the same act asked every tick counts once" (`outflow.rs` `Act`).
  - The NFT side defeats that. The advance's key carries the buy id, and a buy whose advance waits is dropped, then re-created by
    `buy_pass` on the next tick with the next id.
  - Every tick therefore adds `amount` to the demand. With stage 0's 10 ETH a day and `trip_factor` 3, demand reaches 30 ETH
    within `(30 − done) / amount` ticks.
  - The tripwire then freezes custody: `canister::outflow_check` pauses the whole buyer, the frog side included.
  - While paused, `home_act_paused` only records, so **no floor is posted**. Every NFT module goes stale at its 1 h `FLOOR_TTL`, and a
    stale module refuses mints and burns for every holder (`ethflow.rs:52-54`). No delivery is signed.
  - The resume needs the controller, which is the timelock after handover. After a resume the module still wants its item, so demand
    builds again under fresh keys and the tripwire re-trips until the 24 h window lets the advance through.
- **Reachability at stage 0 (the launch numbers), no attacker:**
  - The global custody cap (10 ETH) normally keeps a second big advance from being opened at all. So the trigger is custody room
    coming back while the day's Value record stays: **an advance that comes back within 24 h**, while the module still wants.
  - How it comes back: Relay refunds three times (`Bridging` → `Returning`; "Relay failing to fill" is in the threat model), or the
    operator's `eth_give_back` on a `Funded` or `Arrived` buy.
  - The round-2 monitor pages say "eth_give_back(id) sends it back" and "(eth_give_back(id) does it now)" (`monitor/src/ethnft.mjs:45,47`).
  - For BAYC, 7 + 7 > 10, so the canister freezes about four ticks after the return. For Pudgy (3.4) it takes three advances in a day.
  - If the pilot raises the custody caps (J7) while H3 stays at 10 ETH, BAYC plus Pudgy on the same day is enough, with no return at
    all.
- **Impact:** the burns, the floor and redemption deliveries stop for every holder of every collection, reached without an attacker,
  and cleared only by a timelocked resume, possibly more than once in a day. High. The round-2 way back adds a new operator lever
  (`eth_give_back` on `Arrived`), and C3-L-2's churn ends here.
- **PoC:** `poc/C/r3_c.rs`
  `c3_h1_an_advance_waiting_on_the_value_limit_is_re_asked_under_a_new_key_every_tick_and_trips_the_freeze`, **PASS** (the weakness):
  - Setup: the real limiter, with the value limit scaled so that one 0.5002 ETH advance fits and two do not (as 7 + 7 against 10).
  - Relay refunds three times, and the advance is returned (`Returned`).
  - Four ticks later H3 has been asked under `buy:eth:0` … `buy:eth:5`, and it reports
    `Tripwire { flow: Value, demanded: 3.0012 ETH, limit: 0.9 ETH }`.
  - `paused` is set, and the next tick is the paused pass ("nothing new is signed").
- **Recommended fix:**
  - Key the advance's H3 ask by collection, not by buy (for example `advance:eth:{collection}`, kept until it goes out). Or keep the
    buy and do not drop it on an H3 wait: leave it `NeedAdvance` off the Relay leg (`in_relay` false while it waits) and ask again
    under the same key.
  - Or check H3 in `buy_pass`, with a stable key, before any buy is opened.
  - Add a regression test with the real limiter.
  - The same pattern should be checked everywhere an outflow key embeds an id that a retry renews.

### C3-M-1 (Medium): the way back has no exit; a `BackFunded` or `BackBridging` buy stops every collection's buys until an upgrade

- **Where:**
  - `ethflow.rs:1036-1038` (`buy_pass`: no new buy while any buy is `BackFunded`);
  - `:160-167` (`in_relay`: `BackSending`, `BackBridging` and `Returning` hold the one Relay leg);
  - `:1148` (`BackFunded` waits for a quote with no bound);
  - `:1166-1169` (`BackBridging` past the deadline: "the operator looks; the leg stays watched", and nothing else happens);
  - `:2492-2503` (`give_back_now` refuses every `Back*` phase);
  - `:2615-2618` (`put_listing` refuses a non-`Arrived` buy).
- **Mechanism:**
  - An `Arrived` buy goes `BackFunded` at 48 h, or at once through `eth_give_back`.
  - From then on no buy of **any** collection starts, until its deposit is signed.
  - The deposit is signed only if a runner-supplied Ethereum → 4663 quote passes `relayeth::judge`. Nothing bounds the wait, no
    operator call moves the buy, and it can no longer take a listing if the market comes back.
  - Once deposited, `BackBridging` holds the Relay leg until 4663 rises by half the minimum, or Ethereum rises by half the value.
    If Relay does neither (the threat model's "Relay failing to fill or refund"), the buy stays there for good.
  - The forward leg's analogue (`Bridging` past its deadline goes to `Stuck`, still watched) has `eth_give_back` as its exit. The way
    back has none.
  - Other ways to wedge `BackFunded` (C3-I-3): `give_back_now` on an `Arrived` buy with `wei` ≤ 0.003 ETH makes `back_now` fail every
    tick; and after a reverted `BackDeposit` the buy keeps its old ask while `wei` shrinks by the revert's fee.
- **Reachability:**
  - The back leg has to carry a whole item: about 7 ETH for BAYC, 3.4 ETH for Pudgy.
  - It needs Relay solver inventory on Robinhood Chain for that amount, at a fee within `relay_max_fee_bps`. Only a 0.2 ETH back
    quote was measured (the fixture).
  - A runner that withholds the back quote does the same (it holds the operator identity).
- **Impact:** a stall of every collection's conversions: modules keep pooling ETH and burns still pay, but nothing is bought. The
  item's ETH sits on Ethereum with no path back, and only a code upgrade clears it. Medium: a stall that needs a condition, with no
  operator exit.
- **PoC** (`poc/C/r3_c.rs`, both **PASS**):
  - `c3_m1_a_way_back_with_no_quote_stops_every_collections_buys_and_nothing_moves_it`: 7 days `BackFunded`. Azuki wants an item and
    has room, but no buy starts. `put_listing`, `eth_give_back` and `eth_line_clear` are all refused. Control: the same state with
    Milady `Arrived` starts Azuki's buy in one tick.
  - `c3_m1_a_way_back_past_its_deadline_with_no_fill_or_refund_holds_the_relay_leg_for_good`: 3 days past the order's deadline the buy
    is still `BackBridging` and `relay_busy`, Azuki never buys, and `eth_give_back` is refused. Control: the forward leg's `Stuck` state
    can be given back.
- **Recommended fix:**
  - Give the way back the forward leg's exits. `BackFunded` with no deposit for N hours returns to `Arrived` (or the operator's
    `eth_give_back` does that), so the ETH is buyable again.
  - `BackBridging` past its deadline plus the grace goes to a watched `Stuck`, off the new-buy gate, with an operator exit.
  - Do not block other collections' new buys on a `BackFunded` buy that has not had a judgeable quote for a bounded time.
  - Before sending anything back, check that Relay quotes Ethereum → 4663 at the launch's item sizes (7 ETH, 3.4 ETH).

## Low

### C3-L-1: the gate's 0.5 % is an estimate the judged quote is never held to (the judge allows 3 %; Relay's own quote promises 97.7 %)

- **Where:**
  - `ethflow.rs:73`, `:989-1000` (the `ADVANCE_FEE_EST_BPS` estimate);
  - `:2261-2267` (the forward quote is judged by `relayeth::judge` alone: rule 10, `min_out ≥ value × (1 − 300 bps)`, with no
    re-check against `accepted`);
  - `relayeth.rs:201-206`.
- **Mechanism:**
  - The measured forward quote (`relay-quote-4663-1-2026-09-27.json`) pays 99.69 %, but its `minimumAmount` is 97.7 % (230 bps under).
  - A solver may fill at the minimum, and the canister counts that as the arrival (`got ≥ min_out`).
  - For a module at the gate's edge (pooled BAYC or Pudgy after a small rise), what arrives, less the 0.003 ETH reserve, is then under
    the floor the gate passed. The rank-2 listing cannot be bought.
  - After 48 h the ETH goes back (a second fee, the reserve and gas), and the gate passes the next advance unchanged.
- **PoC:** `c3_l1_a_quote_the_judge_accepts_arrives_under_the_floor_the_gate_passed`, **PASS**. The floor is 0.4896 ETH and
  `advance_covers` is Ok. The judged order promises 488,475,083,200,866,055 of 0.5 ETH, Relay fills at that minimum, and a listing at
  the floor is refused "over cap". The buy goes `BackFunded` at 48 h, and the gate still passes.
- **Fix:**
  - At the forward quote's judge, re-read `accepted` and require `min_out − ETH_GAS_RESERVE ≥ accepted × (1 + ADVANCE_MARGIN_BPS)`.
    Otherwise hold the quote and ask again, or return the advance (`Funded` → `Returning`: gas only, no Relay fee).
  - Or compute the gate with `relay_max_fee_bps` instead of 50.

### C3-L-2: the runner's operator key can churn a module's ETH through Relay round trips at will (about 1 % of an item each)

- **Where:**
  - `ethnet.rs:476-482` (`eth_give_back`: `only_operator`);
  - `ops/src/runner.ts:155-156` (the runner runs as the operator identity, `OPERATOR_IDENTITY_PEM`);
  - `ethflow.rs:2494-2497` (`Arrived` goes back at once);
  - `buy_pass` (no cooldown after a `Returned` buy).
- **Mechanism:** a compromised runner never hands in a listing and calls `eth_give_back` as each buy arrives. The ETH goes back:
  4663 → Ethereum → 4663, then `returnAdvance`. The module still wants its item, and the gate passes again. Every cycle is short by
  two Relay fees, the 0.003 ETH reserve (left in the Ethereum gas pot), the 4663 keep and gas; `returnAdvance` takes the shortfall
  from revenue, then surplus, then the convertible units.
- **PoC:** `c3_m2_the_runners_operator_key_churns_a_modules_eth_through_relay_round_trips_at_will`, **PASS**: 3 round trips in 180 s of
  canister time, no NFT bought, the module short 0.005053 ETH per cycle (1.01 % of 0.5002).
- **Why Low:** it needs the operator identity and gains the attacker nothing. H3's value limit bounds it to about 1 % of 10 ETH a day
  at stage 0, and then C3-H-1 freezes the canister. Before round 2 the operator key could not touch an `Arrived` buy's ETH.
- **Fix:** limit `eth_give_back` on `Arrived` to buys older than `ARRIVED_HOLD_SECS`, or to the controller. Do not re-advance a
  collection for some hours after its advance came back.

### C3-L-3: any 4663 inflow during `BackBridging` is read as its arrival and returned to the buy's module (B-L-6 wider)

- **Where:** `ethflow.rs:1152-1159`. The arrival is `home ≥ balance_after + min_out / 2`, and `wei = min(home − balance_after,
  value)`, sent to `returnAdvance`. The forward leg's arrival needs `got ≥ min_out`.
- **Mechanism:** ETH that is not Relay's reaches our 4663 address before Relay's payout. That can be a frog-side advance or conversion
  refund, with both sides on one canister (B-L-6), or an outsider's transfer. The canister returns that ETH to the NFT module as its
  way back. Relay's real payout then lands unwatched and stays at the canister's 4663 address, unattributed. The same holds on
  Ethereum: an outsider's ETH of at least half the value during `BackBridging` reads as the way back's refund (`Arrived` again), and
  Relay's 4663 payout is orphaned.
- **Impact:** in the NFT-only stack only a donor can do it. The donor loses D, and the module is short `basis − D`, taken from
  revenue, surplus, then holders; the real payout sits at the canister. That is costly griefing with no profit. With a frog side it
  becomes a real misallocation between modules.
- **PoC:** `c3_l2_a_4663_inflow_during_the_way_back_is_returned_as_the_arrival_and_the_real_arrival_is_orphaned`, **PASS**:
  0.2428 ETH of someone else's ETH is returned to Milady's module, and Relay's 0.4951 ETH lands orphaned.
- **Fix:** require `rise ≥ min_out` (as the forward leg does) and a rise no larger than the order's `value`. Better, read Relay's fill
  (the order id) on 4663. B-L-6's fix covers the frog side.

## Info

- **C3-I-1: pooled liveness.**
  - The gate needs a pooled module's per-unit backing (1.03 × the average graduation floor, plus revenue and surplus) to reach
    `1.0151 × accepted + 0.0032 ETH`. So a pool converts only while the current floor is within about 1.5 % of its graduations' average.
  - In a steadily rising market, BAYC or Pudgy can wait indefinitely.
  - That is liveness only: the ETH stays in the module on 4663, burnable. No Relay leg or mint room is held, deposits are unaffected,
    and the pool page fires at 24 h. It is not a stall of burns, redemption or the floor.
- **C3-I-2:** `BackDeposit` is re-priced but never cancelled (`resolve_eth` `:1766-1780`). B-L-3 / C-M-2's six-hour cancel was not
  extended to the new kind. An unmined `BackDeposit` holds the one Ethereum line and blocks deliveries; above the fee cap this is
  C-I-6's case.
- **C3-I-3: wedges into C3-M-1's no-exit state.**
  - `give_back_now` accepts an `Arrived` buy without checking `wei > ETH_GAS_RESERVE_WEI`; `back_now` then errors every tick
    (`:1212`).
  - A reverted `BackDeposit` keeps `ask.amount_wei` (the old `wei − 0.003`) while `wei` drops by the revert's fee (`:2000`). After
    enough reverts `cost > own` forever (`:1676`).
- **C3-I-4:** `relayeth.rs:7-8, :29` still say `FROM_ETHEREUM` is "not wired in Phase 1".
- **C3-I-5:** `fixes-round2-poc/canister-pocs-rerun.output.txt` reports the C2-M-2 PoC as failing. It fails at the harness's
  `put_listing(..).unwrap()`, not at the attack assertion, so that output does not prove the fix; the tolerant re-run above does
  (`poc/C/review2-poc-C-tolerant.diff`).
- **C3-I-6: what holds, checked.**
  - Fills and the way back share the one Ethereum line and its nonce order. An `Arrived` buy never has its fill on the line: a
    signed fill makes it `Buying`, and `eth_give_back` and the 48 h rule act only on `Arrived`. So the NFT cannot be bought and the
    advance also returned; the guard at `:1143` is unreachable.
  - After `eth_line_clear` on a halted fill whose receipt was only lagging, the back deposit is refused because the balance lacks
    the ETH (`own = wei.min(bal − others)`, `:1674`).
  - A double `returnAdvance` reverts at the module (`transitUnits == 0`, `NoAdvance`), and the canister cannot reach it. A return
    larger than the basis goes to revenue; a smaller one draws revenue, then surplus, then a shared loss.
  - The judge pins, for `FROM_ETHEREUM`:
    - the origin chain 1 and the depository and allocator (read on Ethereum);
    - our address as the depositor, the output recipient and every refund recipient, with refunds on Ethereum or 4663;
    - native ETH, no calls or fees, and the 97 % minimum;
    - the deadlines, and rule 12 (checked before signing, so the same quote cannot be signed twice).
  - A runner can only pick among quotes Relay really issues (C3-L-1). `preJudgeEth` handles both legs and the canister judges
    again.
  - Frog-side use of `FloorModule` is unaffected by the new calls, apart from the shared 4663 balance (B-L-6, C3-L-3).
  - The `.did`, the runner's and the monitor's IDL carry the three new phases, and the monitor carries `BackDeposit`. The runner's
    `EthState` IDL has no `line`, so it never decodes `EthKind`.
  - The fixer's `r2_a2_m1_*` and `r2_c2_m2_*` tests are real, not vacuous, but they cover the happy paths only: no test covers a
    way back with no quote, one past its deadline, a fill at the order's minimum, or H3.

## PoCs

| File | Result |
| --- | --- |
| `poc/C/r3_c.rs` (+ `r3_c.output.txt`) | **6 pass** (each shows its weakness): C3-H-1, C3-M-1 ×2 (with controls), C3-L-1, C3-L-2, C3-L-3 |
| `poc/C/review2-poc-C-rerun.output.txt` | review2 C PoCs with the fixer's adaptations: 1 pass / 4 fail (as `fixes-round2-poc`); C2-M-2 then re-run tolerant of the harness `unwrap`: the deposit is `Minted`, and it fails on its attack assertion (closed) |
| `poc/C/review2-poc-C-tolerant.diff` | the one-line tolerant change |
| `poc/C/audit_nft_r2.output.txt` | the fixer's `audit_nft_r2`: 16 pass |
