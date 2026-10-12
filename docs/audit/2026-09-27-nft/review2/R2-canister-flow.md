# Round 2, part C: the canister's Ethereum flow, concurrency and upgrade

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `27c0962`
(`nft-pool` delta `09a680a..8c107e9` and round-1 delta `8c107e9..HEAD`). The pre-fix tree `Rook-nft-pool` (`8c107e9`) was read
for before/after. Scope: `autobuyer/canister-rs/src/ethflow.rs` (the sequencer), its wiring in `ethnet.rs` and `canister.rs`
(`post_upgrade`, the paused pass), and the upgrade safety of the new state. Toolchain 1.98.1 (`~/.cargo/bin`).

**Nothing existing was modified; nothing committed, pushed, deployed or signed.** PoCs were written as
`autobuyer/canister-rs/tests/audit2_nft_c.rs`, run, then moved to `review2/poc/C/` so the tree stays clean:

- `poc/C/audit2_nft_c.rs` (5 tests, all **pass** = the weakness is shown; each turns red once fixed) and its log
  `poc/C/audit2_nft_c.output.txt`. The harness is `tests/audit_nft.rs`'s fake world copied verbatim, plus one switch
  (`drop_milady`: the configuration no longer names Milady).
- `poc/C/r1_audit_nft_c_rerun.output.txt`: round 1's `poc/C/audit_nft_c.rs` re-run against HEAD. It no longer compiled
  unmodified (a new `EthIo` method and six new `EthTx` fields); the two compile-only adaptations are in
  `poc/C/r1_audit_nft_c_compile_adaptations.diff`. No assertion was touched.

Severity scale: the README's.

## 1. Round-1 items in this area

Round 1's C PoC re-run: **7 of 8 fail** on the fixed code; the 8th (C-L-4) passes because it tests Candid, not the probe (see the
row). One of the 7 (C-H-1) fails at its *control* assertion only because the old fake has no `floorBounds()`; the fix is judged on
the round-1 regression test instead.

| ID | Sev (R1) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **B-H-2 / C-H-1** one refusal in `home_act` stops every floor post | High | **Closed** | `home_act` (`ethflow.rs:1789-1998`) collects every step's wait (`waits`), none returns before step 4; an urgent post (module ≥ 40 min old, `due_readings` `:1744`) goes first. R1 PoC fails at its control (fake lacks `floorBounds()`: every post waits on "ABI data ends before word 1", which is the harness, not the flow). `audit_nft.rs` `b_h_2_c_h_1_…` is a real test (Milady's mint waiting on H3, both collections' posts go). New side effect of the mint-room half: **C2-M-2**. |
| **B-M-2 / C-M-1** a transport failure of the simulation marks / holds | Med | **Closed** | `EthNet::fees` (`ethnet.rs:128-159`): only all-providers-revert is `refused (simulation)`; the flow needs an agreed revert standing `SIM_REFUSAL_SECS` (`ethflow.rs:1340-1347`, `:1235-1244`). R1 PoC fails (`Waiting`, not marked). Note: "over several ticks" is really "two agreed reverts an hour apart" (a transient read in between does not clear `refused_since`); acceptable. |
| **B-M-3 / C-M-3** `superseded` clears the line before its reads | Med | **Closed** | `superseded` (`:1583-1631`) clears only after a rival's receipt is read (`?` keeps the line); a nonce taken with no version of ours halts (`halted`). R1 PoC fails ("the line was cleared before the read"). `b_m_3_c_m_3_…` and `b_m_3_a_nonce_taken_…` are real. Residuals (operator exit trusts the operator; a Cancel/CancelFill line whose nonce a foreign tx takes has no exit): C2-I-2. |
| **B-M-4** nonce read at a pin that moves back | Med | **Closed** | `last_mined` / `nonce_floor` (`:459-465`), `sign_eth` signs at `max(plan, floor)` (`:1270`), `eth_act` refuses a pin below the last mined block (`:1291-1296`). `b_m_4_…` real (stale nonce injected). Reorg residual (a `note_mined` on a receipt later reorged out) needs a reorg deeper than the bucketed pin: B-I-2 class. |
| **B-M-5 / C-L-3** one collection blocks every other's buys | Med | **Closed** for the reported paths | Per-collection open buy, `continue` past refusals (`buy_pass` `:864-896`), H2 at the item's value before a buy starts (`:885-888`), a NeedAdvance buy that cannot reserve is dropped (`:1881-1886`). R1 PoC fails (Azuki's buy starts). **Not closed for two new paths** that hold the one Relay leg or poison the earmarks: C2-L-1, C2-M-1 (and C2-L-2 by an upgrade). |
| **B-L-3 / C-M-2** an unmined fill/return holds the line; no marks | Med | **Closed** | Re-pricing every 30 min (`reprice` `:1520`), fill cancel after `FILL_CANCEL_SECS` (`:1468-1472`, `cancel` `:1544`), `mark_pass` every tick whatever holds the line (`:1039-1060`). R1 PoC fails (the other collection's delivery is marked). `c_m_2_…` asserts a disjunction ("marked or served") but does show the cancel. Unchanged from R1: above the chain's fee cap neither re-price nor cancel is signed (C-I-6), so the line waits; marks still come in the last day. |
| B-L-7 refund on Ethereum under `min_out` | Low | **Closed** | Stuck step (`:954-960`); `b_l_7_…` real. Residual: a Stuck buy whose Relay refund lands on **4663** (not Ethereum) is never noticed by the Stuck step (only `Bridging` reads the home refund, `:923-937`); it holds the Relay leg until the operator's `eth_give_back`. Info (operator exit exists). |
| B-L-8 / C-L-2 head-of-line | Low | **Closed** | Per-delivery and per-deposit waits (`eth_act` `:1299-1369`, `home_act` `:1958-1989`). R1 PoC fails. |
| B-L-9 urgent post | Low | **Closed** | `URGENT_POST_SECS`, step 0 of `home_act`. |
| A-L-1 deposits use the purchase's mint room | Low | **Closed**, with a new cost | `buy_holds_mint` (`:468-471`). The cost is C2-M-2. |
| C-L-5 no exit for a held stray | Low | **Partly** (as stated) | `rearm_stray` (`:2185-2199`) re-judges; `c_l_5_…` real. Unreadable-claim re-open not built. |
| B-L-5 upgrade drops the chain with work | Low | **Partly** (as stated) | `post_upgrade` now refuses a chain change with a line, a pending home tx or an open buy (`canister.rs:451-458`). **Removing one collection** (chain kept) is not guarded: C2-L-2. |
| H9 (B-L-2 / C-L-6) ticket facts final | carried | **Closed** | `eth_act` signs only when `deliverBy − 7 d` < the lowest provider's finalized time (`:1316-1330`); `DELIVERY_DEADLINE` is a wrapper constant (`EthNftWrapper.sol:57`). `b_l_2_c_l_6_…` real. |
| R3 M-1(c) paused pass (B-L-4 / C-L-1) | carried | **Closed** | Paused `tick` (`:592-610`): line read, final delivery recorded (`home_act_paused`), last-day delivery cancelled; no re-send of a Deliver whose ticket left `Resolved` (`:1428-1435`). R1 PoC fails (the refunded ticket's bytes are not re-broadcast). Marks are not sent while paused, as on the Bitcoin side (R3 §2.3). |
| L-2 stale reservations | carried | **Partly** | The sweep (`:851-862`) releases `eth:{id}` of a buy gone, finished, or `Stuck` **with no watch**. Every `Stuck` at HEAD keeps its watch (C2-L-1), so the Stuck branch is dead code: a Stuck buy's reservation (up to one item's value against the stage-0 10 ETH global cap) is released only through `eth_give_back`. No dedicated test ("exercised by the flow tests"). |
| L-5 a signed buy never abandoned | carried | **Closed** | The fill cancel (C-M-2). |
| **C-L-4** opt element decodes as nothing | Low | **Partly (not closed for nested `opt`)** | The probe (`canister.rs:395-421`) compares the thirteen elements only. Candid's opt rule applies at the **innermost** `opt` that does not fit: an `nft-side`-shaped `EthState` whose `line` holds the old `EthTx` decodes with `line = None` and everything else present, so the probe sees element 13 present and the upgrade **proceeds**, silently forgetting a signed in-flight transaction (**C2-L-3**, PoC). The fixer's statement that such an `EthTx` "inside the `opt` element … restores as nothing" and "the probe now refuses such an upgrade" is wrong for this case. The regression test's last check ("a state saved without them restores") encodes the *current* default, not an older shape: it does not test what it says. |
| nft-pool's non-`opt` `EthTx` fields | found by fixer | **Confirmed, latent** | `to`, `data`, `gas`, `earlier`, `rivals` (`ethflow.rs:306-317`). nft-pool → HEAD adds only `opt` fields and one appended case (`EthKind::CancelFill`), so that path decodes. nft-side → HEAD loses the line (above). Nothing is installed from either branch. |

## 2. New findings

Counts: **0 Critical, 0 High, 2 Medium, 3 Low, 3 Info.**

### C2-M-1 (Medium): one collection's mined fill can be read as another collection's Relay arrival

- **Where:** `ethflow.rs:1913-1923` (the Funded step takes `eth_before = eth_balance()` and `eth_out_before = st.eth_out`),
  `:916-921` (arrival: `got = bal + (eth_out − eth_out_before) − eth_before`); the two paths that leave a mined spend uncounted
  at that pin: `resolve_eth` `:1440` (`eth_receipt(v)?` fails → the tick goes on) and `mined_eth` `:1642-1648` (ownerOf not read
  → `eth_out` is taken back and the line restored).
- **Mechanism:** the arrival watch is relative: it assumes `eth_out` already counts every transaction of ours mined at the pin
  where `eth_before` was read. When collection Y's fill is mined at the pin but its receipt read fails that tick (a 429, providers
  not alike, a size reject), `home_act` still judges collection X's Relay quote in the same tick (step 7), so X's watch takes the
  balance **after** Y's fill and `eth_out` **before** it. At the next tick the receipt is read, `eth_out` rises by Y's value + fee,
  and X's `got` rises by the same amount with nothing from Relay. If Y's fill is at least X's `min_out` (e.g. any BAYC or Pudgy
  fill against a Milady / Azuki / MAYC buy, or equal-sized buys), X is `ARRIVED` with Y's spent ETH as its earmark.
- **Impact:** exactly what §3.6 of the pooled design says cannot happen: one collection's ETH backs another's buy.
  - X's earmark exceeds the real balance (PoC: 0.495 ETH earmarked, 0.013 ETH at the address): every other arrived buy's
    `available` (`:1388`) and every stray return (`:1249-1254`) is starved until X's buy finishes.
  - X's real Relay fill, when it lands, is unearmarked; `cost_of(advanced, total, arrived)` (`ethnft.rs:660`) divides by the
    phantom, so `settleAdvance(cost)` books a wrong cost and feeds a wrong fill price to the module (`FloorModule.sol:725`).
  - If X's Relay order instead refunds on 4663, nothing notices (X is no longer `Bridging`): the refund stays at the canister's
    4663 address, X sits `Arrived` on ETH that is not there with no exit (`give_back_now` refuses `Arrived`), its module stays in
    transit (no more buys for that collection) and `buy_holds_mint` holds its deposits. A bounded loss of one item's budget for
    that module plus a collection-wide stall.
  - Needs no attacker, only a transient read failure in the one tick a quote is judged while another collection's fill is mined.
    With five collections and fills and Relay legs interleaving, that is a matter of time.
- **PoC:** `audit2_nft_c.rs` `c2_a_fill_mined_at_the_pin_but_not_yet_counted_when_the_next_deposit_is_watched_is_read_as_that_buys_arrival`
  (**passes**): Azuki is `Arrived { wei: 495464962449863321 }` while the Ethereum balance never moved (Milady's 0.4954 ETH fill).
- **Recommended fix:** make the watch absolute, not relative: do not judge a quote (take `eth_before`) while the line holds a
  transaction whose outcome is unread at this pin, i.e. while `st.line` is `Some` and not resolved this tick, or while `mined_eth`
  put a mined transaction back; or better, detect the arrival from Relay's own fill on Ethereum (the order id in the fill
  transaction's calldata/logs, read by the canister) instead of a balance delta. At minimum cap `got` at the order's quoted
  `currencyOut.amount` and require `got ≤ bal − Σ earmarks`.

### C2-M-2 (Medium): an `Arrived` buy waiting for a listing holds every deposit of its collection unminted, indefinitely, with no exit

- **Where:** `ethflow.rs:468-471` (`buy_holds_mint`: every phase from `Advancing` to `Minting`, including `Arrived` and `Stuck`),
  `:1962-1964` (deposit mints wait on it), `:2132-2138` (`give_back_now` refuses `Arrived`), `:2185-2189` (`rearm_stray` only
  for `Held`).
- **Mechanism:** round 1's A-L-1 / C-H-1 fix stops a collection's deposit mints from the buy's advance until its own mint. An
  `Arrived` buy waits for a listing within `min(arrived − margin, reading × 1.05, max_purchase)` (D4). The advance buys at
  most `floor × (1 + convBps)`: the headroom is ≈ 2.3–2.6 % for every launch collection at the default `convBps` 300
  (`phase1-nft-pooled.md` §1, BAYC at 9 ETH 2.6 %). A floor that rises past that between the advance and the fill (minutes of
  Relay, then the listing hand-in) keeps the buy `Arrived` until the market comes back, possibly days. Meanwhile every deposit
  of that collection that is final and ours stays `Mint`: not minted, not returned, and no operator path moves it.
- **Impact:** holder deposits (D5 ON) of a volatile collection are frozen for as long as the collection's own buy waits, with no
  bound and no exit short of a timelocked upgrade (a BAYC deposit is ≈ 7 ETH of a third party's NFT). The fixer listed this as
  judgment call 3 "delays that collection's deposit mints as long"; with the launch config's headroom it is a likely state, not an
  edge. It also covers `Stuck` (C2-L-1) and a stuck `Bridging` for its whole deadline.
- **PoC:** `c2_an_arrived_buy_that_cannot_fill_keeps_every_deposit_of_its_collection_unminted_indefinitely` (**passes**: 3 days,
  deposit still `Mint`, no exit).
- **Recommended fix:** reserve the purchase's mint instead of blocking deposits: let a deposit mint when the H3 per-collection
  mint window and the H11 count have room for it **and** one more (the buy's); or release the hold once a buy has been `Arrived`
  for longer than a bound; or return a deposit that has waited more than N days (it then goes back to its depositor, which D5
  already does for "no custody room"). And give `Arrived` an operator exit (send the ETH back through Relay, bounded by H3).

### C2-L-1 (Low): a buy that goes `Stuck` after its arrival keeps its Relay watch: it holds the one Relay leg, then "re-arrives" ETH it already spent

- **Where:** `ethflow.rs:139-146` (`in_relay`: `Stuck` counts while `last` is `Some`), `:918-920` (the arrival never clears
  `last`), `:970-973` and `:1650-1652` (the two post-arrival `Stuck`s), `:945-953` (the Stuck step recomputes `got` from the stale
  watch), `:851-858` (the L-2 sweep then never releases the buy's `eth:{id}`).
- **Mechanism:** after an arrival and a paid fill, `bal + (eth_out − eth_out_before) − eth_before` still equals what arrived (the
  fill's value is in both terms). A post-arrival `Stuck` (the fill mined but `ownerOf` at the pin is not us; or `Bought` and not
  ours after finality) is `in_relay`, so in the same tick buy_pass steps it and sets `Arrived { wei: arrived }` again. For a
  reverted re-inclusion after a reorg the phantom is `arrived + total`.
- **Impact:** the phantom earmark exceeds the balance (PoC: 0.498 ETH earmarked, 0.058 ETH held): every other collection's
  arrived buy is capped at nearly nothing (PoC: Azuki's own 0.498 ETH arrival refuses a 0.45 ETH listing, "cap
  55422962449863321"), all stray returns wait, the collection's deposits wait (C2-M-2), and no operator exit exists for `Arrived`.
  Before the re-arrival the `Stuck` holds the canister-wide Relay leg. Reachability is low: both post-arrival `Stuck`s need the
  token not to be ours after a successful fill (an exotic collection or a reorg deeper than the pin, B-I-2 class), hence Low.
- **PoC:** `c2_a_stuck_after_arrival_holds_the_relay_leg_then_re_arrives_eth_already_spent_and_stalls_every_other_collection`
  (**passes**).
- **Recommended fix:** clear `last` on arrival (`Bridging`/`Stuck` → `Arrived`); make `in_relay` and the Stuck step require a
  phase that was never past arrival; give a post-arrival `Stuck` (and `Arrived`) an operator exit; let the L-2 sweep release a
  `Stuck` buy that is not on the Relay leg.

### C2-L-2 (Low): an upgrade that drops a collection whose buy is on the Relay leg stops every collection's buys for good

- **Where:** `canister.rs:451-458` (the B-L-5 guard compares only the chain id); `ethflow.rs:1866-1867` (`NeedAdvance` and
  every other home step error "no longer configured" before any drop), `:1383` (`Arrived`), `:418-420` (`relay_busy`).
- **Mechanism / impact:** a timelocked upgrade that removes one collection from `eth_nft.collections` (or changes its module or
  wrapper) while its buy is `NeedAdvance`/`Funded`/`Bridging`/`Returning` leaves that buy open for ever: it keeps `relay_busy`
  true, so no other collection's buy ever starts; an `Arrived` one keeps its earmark. A privileged mistake, hence Low; B-L-5's
  "not built" note covers the caps, not this.
- **PoC:** `c2_an_upgrade_dropping_a_collection_whose_buy_holds_the_relay_leg_stops_every_other_collections_buys_for_good`
  (**passes**: 20 ticks, Azuki wants and has room, no buy).
- **Recommended fix:** in `post_upgrade`, refuse removing or re-pointing a collection that has an open buy, a non-finished
  delivery, an open deposit/return or a `Held` stray; or let `home_act` drop a `NeedAdvance`/`Funded` buy whose collection is gone
  (return the ETH to the module named in the buy).

### C2-L-3 (Low): C-L-4's probe does not see an `opt` nested inside the Ethereum state

- **Where:** `canister.rs:395-421` (`SavedProbe`, `lost_elements`); the nested `opt`s of `EthState`: `line: Option<EthTx>`,
  `pending: Option<HomeTx>`, `EthBuy.last`/`ask`, `readings`' `module_posted_at`, etc.
- **Mechanism / impact:** Candid nulls the innermost non-fitting `opt`. An `EthTx` of another shape inside `line` restores as
  `line = None` while the rest of element 13 decodes, so the probe passes and the upgrade proceeds without the signed in-flight
  Ethereum transaction. The delivery it carried stays `Sent` with no line (orphaned: never marked, the holder waits for the
  30-day window, and if the transaction mines the holder has both the NFT and a refund); a fill's buy stays `Buying`; the nonce
  may be signed again. Latent: nothing is installed from `nft-side` or `nft-pool`; it bites the next upgrade that changes a type
  under a nested `opt` (the probe is presented as the guard for exactly that).
- **PoC:** `c2_an_nft_side_line_inside_a_decodable_eth_state_is_what_the_probe_sees` (**passes**: `line = None`, delivery
  `Sent`).
- **Recommended fix:** decode the thirteenth element a second time as a mirror type in which every nested `opt` is
  `opt reserved` (or re-encode the decoded `EthState` and compare its `line`/`pending`/per-buy `last` presence with the probe's),
  and trap on any difference; add a golden fixture of an `nft-pool`-era state as the frog side has (`saved-e04f49b.hex`).

### Info

- **C2-I-1** The pooled design (§4 table) says a buy's spend is ≤ "balance − Σ other earmarks − the gas reserve"; the code
  (`:1387-1389`) subtracts no gas reserve, and the fill's gas is paid on top of `cap` from the shared balance. Deliveries check only
  the total balance (`ethnet.rs:292-296`), so they can spend an arrived buy's ETH on gas. Self-correcting through `available`,
  small amounts; doc/code mismatch.
- **C2-I-2** `eth_line_clear` (`:2152-2180`) does not re-read the chain: it trusts the operator to have checked that no version
  mined. Cleared wrongly (a receipt a provider did not index), a `Deliver` goes back to `Waiting`, can no longer be signed
  (ownerOf ≠ us), and `mark_pass` marks it in its last day while the NFT was delivered (NFT + refund); a `Buy` goes `Arrived` on
  spent ETH. Also: a `Cancel`/`CancelFill` line whose nonce a foreign transaction took is neither halted nor clearable
  (`superseded` returns `Err` for ever, `line_clear` refuses cancels): only an upgrade frees the line. Re-read the versions'
  receipts at a later pin inside `line_clear`, and let it clear a cancel line the same way.
- **C2-I-3** Reorg classes (all need a reorg deeper than the bucketed Ethereum pin, B-I-2): `MarkAfter` marks at the finality of a
  block **number** without re-reading which rival mined (`:1003-1010`): a cancel reorged out for a delivery version would mark a
  delivered ticket (NFT + refund); `Moved`/`Bought` record the hash that was first seen (`recordDelivery(ticket, v1)` although
  `v2` mined); `note_mined` on a receipt later reorged out leaves `nonce_floor` above the chain and nothing re-broadcasts the lower
  nonce. Re-read the receipt (and, for a mark, `ownerOf == us`) at a finalized pin before acting.

## 3. What was checked and holds

- One Relay leg: every path into `NeedAdvance`…`Bridging`/`Returning` is gated by `relay_busy` (`:864`); arrival and home refund
  are the only balance-delta reads, and no other buy moves either balance meanwhile in the NFT-only stack (B-L-6 needs a frog side).
- Earmarks otherwise: `earmark()` counts `Arrived`/`Buying` only; `buy_cap` gets `wei.min(bal − others)` (`:1388-1390`); returns
  keep every earmark (`:1249`). A reverted fill gives back `arrived − fee` (`:1638-1640`); a mined cancel `arrived − fee` (`:1678`).
- Re-pricing: same nonce, `to`/`value`/`data`/`gas` kept (`:1528-1529`), every version kept in `earlier` and read first
  (`versions()`), `retag` follows the mined version, re-price never while paused and never over the chain cap; a cancel's rivals are
  the full version list; a cancel line is itself re-priced with its rivals kept (`..t.clone()`).
- Deliveries first by `deliver_by`, then arrived buys round-robin, then one return; each wait is its own.
- Upgrade: nft-pool → HEAD adds only `opt` fields and an appended variant (checked by diff of every record in `ethflow.rs` /
  `ethnft.rs`); `FloorPost` and the config types are unchanged.
- No double mint or double delivery found outside the reorg and operator-mistake classes above.
