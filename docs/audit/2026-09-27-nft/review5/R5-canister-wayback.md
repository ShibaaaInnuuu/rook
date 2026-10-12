# Round 5, part C: the way back after C4-M-1 (the late order, `eth_credit_payout` and its judge)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1dc2a61`
(delta `2735dac..1dc2a61`: fix round 4, canister commit `57032dd`). Scope: C4-M-1's fix and the way back around it: `ethflow.rs`
`step_buy` (`Bridging`, `Stuck`, `BackBridging`), `give_back_now`, `put_payout`, `judge_payout`, `payout_pass`, `LateOrder`,
`PayoutTx`, `EthState.credited`, `Inbox.payouts`, the tick order (`tick` step 4); `relayeth.rs` (`EthDeposit.solver`); `ethnet.rs`
(`eth_credit_payout`, `payout_tx`); `canister.rs` (`home_payout_tx`); `evmsend.rs` (`payout_view`); the `.did`; the re-rated Lows
C4-L-1, C3-L-1, E3-L-2 and B-L-6; and the upgrade safety of this round's state in that area. `src/floor/FloorModule.sol` (unchanged)
was read for `returnAdvance` and `fundReserve`. Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review5/poc/C/`; nothing committed, pushed, deployed or signed.**
Every PoC ran in a private copy of `autobuyer/canister-rs` (`/tmp/rook-r5/C`, an APFS clone, checked identical to the worktree's
`src/` and `tests/` with `diff -rq` before anything was added; removed afterwards). The harness is the current
`tests/audit_nft_r4.rs` copied whole, with one addition (`poc/C/r5_c.harness.diff`: a switch that fails one Ethereum balance read
after the payout judge reads its transaction). The 22 round-4 tests of the copied harness still pass with it (27 of 27).

Severity scale and threat model: the README's.

## Verdict

**C4-M-1 is not closed. It is narrowed and moved.**

- **What was fixed:**
  - A way back past its deadline, grace and `BACK_LATE_SECS` stays watched (`BackBridging`, holding the Relay leg) until its payout
    is read or the operator writes it off. The write-off is refused until a day past deadline + grace.
  - A payout that lands while the order is still watched is read by its own watch and returned to its own module. The fixer's
    `r4_c4_m1_…` and `r4_c4_m1b_…` show this, and my copy re-ran them.
  - `eth_give_back` on a watched forward `Stuck` no longer pays the raw 4663 balance.
- **What is not fixed (C5-M-1):**
  - The write-off gives the Relay leg back at once (`b.last.take()`, `Stuck` unwatched, `in_relay` false). `eth_credit_payout`
    exists for a payout that lands after the write-off, and that payout still lands in whatever watch is open by then.
  - The fixer's statement "A payout is never read by another order's watch, because no other order is watched while it could land"
    holds only before the write-off.
  - Round 4's PoC `c4_m1b` with the write-off step added still passes: Azuki reads Milady's late payout as its refund and bridges
    twice, and `eth_credit_payout` for Milady's payout is then refused, because the ETH is "Azuki's". Milady's module books the whole
    advance as a loss, and 0.51 ETH at our Ethereum address is nobody's.
- **The payout judge does not identify a payout (C5-L-1).**
  - `orderData.solver` is Relay's single solver: `0xf70d…dbef`, `solverChainId: base`, the same in both measured quotes and both
    directions. So "sent by the order's solver" is true of every Relay payment to us, for any order.
  - "Names the order id in its input" is true of any transaction whose calldata carries the id, at an amount the operator states.
    That includes our own `depositNative(us, id)` and a stranger's 0-value call.
  - What decides is the balance check. On 4663 it counts as free every wei that is not an NFT buy's `Funded` / `Returning`: the frog
    side's ETH (B-L-6), and an open watch's refund that has landed but not been read yet (`payout_pass` runs before `buy_pass`).
  - A wrong hand-in therefore pays a module from the frog side's ETH, or from another collection's refund. In the second case it
    also leaves that collection `Funded` on ETH that is gone, holding the Relay leg.
- **The dedupe itself holds.** It is one credit per written-off order, and a transaction hash is credited once canister-wide. It
  records only hand-ins, though, not the balance rises that watches read (C5-L-1).
- **One ordering bug (C5-L-3).** An Ethereum credit to an open buy is marked credited before the fallible balance read in its
  `arrive`. A transient read failure then consumes the payout for good.

## Counts (this part: new, plus round-4 items not actually fixed)

| Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- |
| 0 | 0 | 1 | 2 | 3 |

- The Medium is **C5-M-1**, C4-M-1's residual, re-numbered here because its trigger changed.
- The Lows are **C5-L-1** and **C5-L-3**.
- The carried Lows are re-rated in the table below and not counted again: C4-L-1, C3-L-1, E3-L-2, B-L-6.

## Round-4 items in scope

| ID | Sev (R4) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **C4-M-1**: a late way-back payout is nobody's, or is read as another order's | Med | **Narrowed, not closed** (**C5-M-1**, **C5-L-1**, **C5-L-3**) | See below. (a) A payout while the order is still watched is its own (`step_buy` `BackBridging` `:1713`, `Stuck` `:1667`, and `:1747` keeps the watch). A payout after the write-off can be credited by transaction, but only through a judge that does not bind it to the order (C5-L-1, C5-I-1). (b) A payout after the write-off is still read by the next order's watch (C5-M-1, PoC). |
| C4-M-1, forward exit: `give_back_now` on a watched `Stuck` paid `balance − keep` | (folded) | **Fixed** | `:3444-3449`: only `refunded(balance, balance_after, value)`, and then `rise.min(value).min(advanced)`. Refused otherwise until deadline + grace + 24 h (`:3451-3455`), and then written off with nothing paid (`:3456-3462`). The `balance` is the canister's floored 4663 pin (C4-H-1's `PIN_FLOORS`). The rise can still be the frog side's ETH (B-L-6, below). |
| **C4-L-1**: the way back's bound is the order's deadline | Low | **Re-rated: stays Low, longer** | The leg is now held until the operator acts (no automatic release: `:1747-1755`), at the earliest deadline + 1 h + 24 h (`:3407`, `:3451`), which is 8 to 9 days with Relay's 7-day deadlines. Burns and redemptions are unaffected, and `eth-back-late` / `Stuck` page. The write-off that ends the hold is itself C5-M-1's trigger, so the operator trades liveness against attribution with no safe point in between. |
| C3-L-1: the gate assumes 50 bps where the item cap binds | Low | **Unchanged, Low** | `advance_covers` `:1480-1491` still uses `ADVANCE_FEE_EST_BPS` 50 (`:75`), and `advance_bound` is capped at `item_cap` (`:1514`). Nothing in the delta touches it. A short arrival goes back after 48 h, and that way back can now hold the leg longer only if Relay also fails it (C4-L-1). |
| E3-L-2: gas drift deadlocks an arrival | Low | **Unchanged, Low** | `arrive` `:1787-1807` is unchanged. The new credit path `(false, open)` calls `arrive` after a check that subtracts earmarks and leftover (`:3570`), so it cannot fail for room (C5-L-3's failure is the read, not the room). The late order's `Stuck` earmark is 0, as `BackBridging`'s was. |
| B-L-6: the shared 4663 address (frog side and NFT side) | Low | **Partly fixed, stays Low; one new facet** | `eth_give_back` no longer pays the raw balance. But the automatic 4663 reads still take a rise of half the value as a refund, whoever's ETH it is: `Bridging` `:1628`, a watched `Stuck` `:1667`, and `BackBridging` (99 % of `min_out`, `:1713`). New: the payout judge's 4663 balance check counts the frog side's ETH as free (`:3559-3566`), so a wrong hand-in pays a module from it (C5-L-1(a), PoC). |

## C4-M-1's fix in detail (what was tried)

**1. The watch until payout or write-off.**

- `BackBridging` past deadline + grace + `BACK_LATE_SECS` returns "LATE … still watched" and keeps `last` (`:1747-1755`).
- `give_back_now`'s `BackBridging` arm refuses until `deadline + 1 h + 24 h` (`:3407-3411`). Then it takes the watch into
  `EthBuy.late` (`:3412-3415`) and leaves the buy `Stuck` with `last = None`.
- A forward order moves to `Stuck` at deadline + grace with its watch kept (`:1642-1646`). That watched `Stuck` reads a late Ethereum
  fill (`:1655`), a refund on Ethereum of at least half (`:1661`) and a late 4663 refund (`:1667-1672`). Its write-off has the same
  day's wait (`:3451-3455`).
- **Holds**, up to the write-off.

**2. `eth_credit_payout` / `judge_payout`.**

- **Access.** The endpoint is operator-only, refused while paused or mid-tick (the source-reading test counts 8 guarded updates).
  The chain is 4663 or the configured Ethereum id (1), and nothing else. `put_payout` requires `late`, refuses one already credited
  and a transaction already in `st.credited`, keeps one claim per buy, and holds at most 8. A claim is judged within
  `HANDIN_TTL_SECS` (600 s), one a tick.
- **Reading the transaction.** On 4663 the receipt is read at the floored pin, every provider agreeing (`canister.rs:982-1001`). On
  Ethereum it is the pinned receipt plus finality (`payout_pass` `:3550-3553`). The transaction body comes from
  `eth_getTransactionByHash` through the `payout_view` projection (from, to, value, input). **Holds.**
- **"Names the order" (`:3508-3512`).** Either of these passes:
  - The order id appears in any 32-byte window of the input, unaligned. The calldata is the sender's to choose.
  - The sender is `watch.solver`. That is `orderData.solver` as parsed from the handed-in quote (`relayeth.rs:219`), and it is bound
    by the id: rule 6 recomputes the id over `orderData`, whose EIP-712 type includes `address solver` (`relayconv.rs:260`). So the
    runner cannot choose a solver without changing the id, and the operator cannot choose one at all.
  - **But the solver is not per order.** Both measured quotes (4663 → 1 and 1 → 4663) name `0xf70da978…dbef` on `base`. Being bound
    by the id buys nothing when every order has the same solver.
- **"Pays us" (`:3514-3522`).** A plain transfer to our address at its `value`. Or, when the input carries the id, any successful
  call at the amount the operator states (`named`), to any address from any sender. Relay paying through an internal transaction
  (from inside the depository, or a solver contract) is creditable only through that second path, and only if the id is in the
  input (C5-I-1).
- **Bounds (`:3523-3532`).** `paid ≤ value` and `paid ≥ value / 2`. The `o.back && !home` clause repeats the general half check, and
  every chain combination passes (C5-I-2). There is no check that the transaction mined after the order's deposit (the watch's
  `pin`), and none that it is not ours (`from == us`).
- **"Beside every other claim".**
  - Ethereum (`:3568-3573`): `balance − every earmark − every collection's leftover`. **Holds**, except that a watched order's unread
    Ethereum arrival is free on the tick it lands.
  - 4663 (`:3557-3566`): `balance − Σ(Funded, Returning)` of open NFT buys. It does not subtract:
    - the frog side's, vault loop's or custodian's ETH at the same address;
    - a `Bridging` / watched `Stuck` refund or a `BackBridging` arrival that landed but is not read yet (`payout_pass`, `:905`, runs
      before `buy_pass`, `:908`);
    - the gas keeps.
  - These are C5-L-1(a) and (c), both with PoCs.
- **Dedupe.**
  - `st.credited` (hashes, never pruned) plus `late.credited`: no transaction is credited twice, and no written-off order twice.
  - It is keyed by the hash alone, which is fine across chains: the chain id is in every signed EIP-155/1559 transaction.
  - Two claims for different buys with the same hash: the second is judged against `st.credited` at its own tick and refused.
  - **Gap:** balance rises read by watches have no transaction. A Relay transaction that a watch already consumed as an arrival or
    refund is not in `st.credited`, and the solver path accepts it for a written-off order (C5-L-1).
- **Where a credit goes.**
  - 4663, buy open: `Returning { paid.min(advanced) }`, then `returnAdvance`.
  - 4663, buy closed: a new sweep record, `Returning { paid }`, then `fundReserve`. The module's `NoAdvance` does not apply;
    `fundReserve` takes any value above 0.
  - Ethereum, buy open: `arrive`, i.e. `Arrived`, then a listing or the way back.
  - Ethereum, buy closed: the collection's leftover, swept back later.
  - Sweeps are handled apart.
  - Every destination is the buy's own collection and module. **Holds**, except C5-L-3's ordering.

**3. What each `give_back_now` path returns.**

| Path | Returns | Measured? |
| --- | --- | --- |
| `Funded { wei }` | `wei.min(balance − keep)` | Own: `wei` was measured when it arrived. |
| `Arrived` | `BackFunded { wei }` | Own earmark, after the 6 h hold. |
| `BackFunded` | Back to `Arrived` | Nothing signed. |
| `BackBridging` | Write-off only | After deadline + grace + 24 h. |
| Watched `Stuck` | Only `rise.min(value).min(advanced)` | Else refused, then a write-off with nothing. |
| Unwatched `Stuck` | `Returning { 0 }` | A sweep: `Returned { 0 }`. |

`BackSending`, `Returning` and `Returned` are refused. **Holds**: nothing pays out raw balance. The only caveat is that "the rise"
cannot tell the frog side's inflow from a refund (B-L-6).

**4. "No other order is watched while it could land."**

- `relay_busy` is one watch canister-wide, and `in_relay` covers every phase with a watch (`:230-237`; `last` is set only by the
  `Bridging` and `BackBridging` entries, both `in_relay`).
- `eth_act` signs no way back while another buy is on the leg (`:2298`), and `sweep_pass` starts no sweep then (`:3329`). While
  watched, two watches never coexist. **Holds.**
- **After the write-off:** the late order is `Stuck` with `last = None`, so it is not `in_relay`. A forward buy of any other
  collection (`buy_pass` `:1550`), another collection's way back, or a gas-pot sweep can start and be watched while the written-off
  order's payout is still out. **Fails: C5-M-1.**

**5. Upgrade safety.**

- Every new field is `Option` + `#[serde(default)]` and `opt` in the `.did`: `EthBuy.late`, `Watch.solver`, `EthState.credited`,
  `Inbox.payouts`, `HomeTx.used_since`, `home_floor`, `pin_floors`, and `SaleScan.bad` / `skipped`.
- No enum case was added.
- `LateOrder.credited : opt record { blob; nat; bool }` matches `Option<([u8;32], u128, bool)>`. `PayoutClaim.wei` is `nat` (`u128`).
  `eth_credit_payout : (nat64, nat64, text, nat)` matches the Rust signature.
- The saved states of `e04f49b`, `4949da0`, `a37f54b` and `1eb5788` restore with `lost_opts = []` and the new fields absent. The
  round-4 fixture round-trips.
- A downgrade to a pre-round-4 build with a `late` or `credited` present is refused by `lost_opts`, as intended.
- **Holds.** Not counted: `credited` is never pruned, 32 bytes per credit.

## Findings

### C5-M-1 (Medium): the write-off gives the Relay leg back, so the late payout that `eth_credit_payout` exists for is still read as the next order's refund or arrival

**Where.**

- `ethflow.rs:3412-3415`: `BackBridging` write-off, `b.last.take()`.
- `:3456-3462`: the forward write-off, `b.last = None`.
- `:230-237`: `in_relay`, where an unwatched `Stuck` is not on the leg.
- `:1550`: `buy_pass` starts the next buy once `relay_busy()` is false.
- `:1628-1644`: another order's `Bridging` reads any 4663 rise of half its value as its refund.
- `:1667`: a watched `Stuck` does the same.
- `:1713`: `BackBridging` or a sweep reads 99 % of its `min_out` on 4663 as its arrival.
- `:1621-1624` and `:1654-1656`: an Ethereum rise of `min_out` is read as a forward arrival.
- `:3557-3566`: the credit is then refused, because the ETH is the other order's.

**How.**

1. Milady's way back is neither filled nor refunded. A day past deadline + grace the operator, on Relay's status, writes it off
   (`eth_give_back`). This is the step round 4 added, and C4-L-1's hold pushes the operator to take it, because every collection's
   buys wait until then.
2. The Relay leg is free. Azuki's buy starts, advances and deposits. Its order is `Bridging`.
3. Relay pays Milady's way back after all: a fill of 0.4855 ETH to our 4663 address. This is the event `eth_credit_payout` was built
   for.
4. Azuki's watch reads it as "REFUNDED" (a rise of at least half of 0.5 ETH). Azuki becomes `Funded { 0.4855 }` and asks for a new
   quote.
5. The operator hands in Milady's payout. It is refused: "our 4663 balance … beside the open buys' ETH there, under the … to
   credit". Azuki's `Funded` claims it.
6. Azuki bridges again (2 Relay deposits). Its first order fills on Ethereum and is read as the second's arrival. The second fill is
   nobody's.
7. The operator closes Milady as a loss. `returnAdvance(0)`: Milady's module books its whole advance, and its units' holders cover
   it.

The same happens for a written-off forward order whose late fill lands on Ethereum while another forward order is `Bridging` (read as
its early arrival), and for a gas-pot sweep's `BackBridging`, which reads the rise as its arrival and returns only its own value.

**Impact.**

- As C4-M-1(b): one advance booked as a loss while its ETH sits in custody unattributed (Milady about 0.5 ETH; BAYC up to 7.35 ETH).
  Another collection bridges twice, and one arrival is orphaned.
- The new tool cannot repair it. The judge refuses the right transaction, because the watch already consumed the ETH. It could be
  "repaired" only through C5-L-1's loose paths, by crediting a different transaction.
- The monitor sees a normal refund on Azuki.

**Why Medium (unchanged).**

- It is a bounded loss, conditional on Relay failing an order and then paying after the operator wrote it off. That is the threat
  model's "Relay failing to fill or refund" plus Relay's own status being wrong or late.
- The fix narrowed the trigger. The deadline + 25 h cut-off is now the operator's write-off.
- The fix's own design assumes the trigger happens: `eth_credit_payout` and §9 call 4 exist for exactly this payout.
- The canister does not check Relay's status. Call 3 says the write-off is allowed "whatever Relay's status says" after the day.

**PoC** (`poc/C/r5_c.rs`, passes):

- `c5_m1_a_payout_after_the_write_off_is_still_read_as_another_collections_refund`
- Output: "Milady's late payout 485535083200866034 was read as Azuki's refund; Azuki bridged twice; Milady's module got 0;
  511352841698860608 wei at our Ethereum address is nobody's".

**Fix** (any one of these closes it; the first is the smallest):

- **Keep a written-off order on the leg for new orders.** While any `late` order is neither credited nor closed as a loss plus a
  quiet period, `relay_busy()` stays true for new orders and sweeps. Only burns and redemptions go on. This makes C4-L-1 longer; it
  is the honest form of "no other order is watched while it could land".
- **Or, while a `late` order is open, read no other watch by balance delta** on that order's payout chains. Such a watch moves only
  on a handed-in transaction (the same judge, with C5-L-1's fix).
- **Robust (E3-I-4, rounds 3 and 4): attribute every leg by transaction.**
  - Store Relay's `requestId` beside the order id.
  - Have the runner hand in Relay's status transactions for every fill and refund.
  - Credit by transaction, never by balance rise.

### C5-L-1 (Low): the payout judge does not bind a transaction to the order; the 4663 balance check counts other flows' ETH and unread refunds as free

**Where.**

- `ethflow.rs:3508-3512`: the solver path or the id anywhere in the input.
- `:3514-3522`: the `named` path, any call at an operator-stated amount.
- `relayeth.rs:219`: the solver, which is the same for every order.
- `:3557-3566`: the 4663 check, `balance − Σ(Funded, Returning)`.
- `:905-908`: `payout_pass` runs before `buy_pass`.
- No check of `t.block` against the watch's start, and none of `t.from != us`.

**What passes the judge for a written-off order.**

- (a) **Any Relay payment to us from the shared solver**, of any order. For example:
  - a refund of one of the frog side's 4663 → BTC conversions (`relayconv`, same depository and solver);
  - a fill or refund of another NFT buy that a watch already consumed by balance delta (never in `st.credited`);
  - one mined before this order existed.
  - PoC: a solver transfer of another request, mined in block 1, is credited to Milady's closed order. `fundReserve` pays 0.4855 ETH of
    the frog side's 2 ETH into Milady's module.
- (b) **Our own `depositNative(us, id)`** of the same order, or **anyone's 0-value call** with the id in its input, at the stated
  amount. PoC: both judged as payouts of 5 ETH.
- (c) **A credit paid from an open watch's unread refund.**
  - Azuki's forward order is `Bridging`. Relay refunds it on 4663 from the solver. On that tick the operator hands in a transaction
    for Milady (in the PoC, that very refund, which the solver path accepts).
  - `payout_pass` sees the refund as free and credits Milady (`Returning`). `buy_pass` then reads the same rise as Azuki's refund
    (`Funded`).
  - Milady's module receives Azuki's 0.495 ETH (`returnAdvance`). Azuki is `Funded` on ETH that is gone, holding the Relay leg, until
    the operator gives back what is left. Azuki's module then books a loss.

**Impact.**

- Value crosses from the frog side, or from another collection, into one module. It never leaves the protocol, and it is bounded by
  one order's value per written-off order (`late.credited`).
- In (c) it also stalls every collection's new buys until the operator acts.
- It needs a written-off order and a wrong hand-in (a mistake, or a compromised operator key, which is in the threat model).
- The judge was the control meant to catch a wrong hand-in ("judged against that order alone"). In practice only the balance check
  guards, and on 4663 that check is incomplete.
- Low: operator-gated, bounded, no gain to the actor. It is Medium-adjacent because (c) loses one collection's refund to another.

**PoC** (`poc/C/r5_c.rs`, all pass):

- `c5_l1_a_the_judge_credits_another_relay_payment_from_the_shared_solver_paid_from_the_frog_sides_eth`
- `c5_l1_b_the_judge_takes_our_own_deposit_or_any_call_carrying_the_id_at_the_amount_the_operator_states`
- `c5_l1_c_a_payout_is_credited_from_a_watched_orders_unread_refund_and_that_order_then_holds_the_relay_leg`

**Fix.**

- Name the order by what Relay puts in its payouts: the `requestId` (`0x1790…` in the fixture, which is not the v2 `orderId`
  `0xbce9…`), at the position Relay's fill calldata carries it. Store it in the watch.
- Drop the solver-only path, or require the solver and the id together.
- Refuse `from == us` and `t.block ≤` the watch's start.
- For the `named` path, require `to == depository` and the depository's own withdrawal or refund event naming the id and our address
  (read the receipt's logs, not an operator-stated amount).
- Run `payout_pass` after `buy_pass`, or refuse a credit while any other order is watched.
- On 4663, subtract the frog side's accounted ETH (with B-L-6's fix).

### C5-L-3 (Low): an Ethereum credit to an open buy is recorded before its fallible balance read; a transient failure consumes the payout for good

**Where.** `ethflow.rs:3576-3581` writes `st.credited` and `late.credited`. Then `:3600-3603` handles `(false, open)` with
`arrive(…, io.eth_balance().await?, …)?`. That is a second outcall, which on failure returns `Err` from the closure after the
writes. The claim was already removed from the inbox (`:3540`).

**Impact.**

- The log says "refused, nothing credited". But the order and the transaction are marked credited, the buy is still an unwatched
  `Stuck`, and a re-hand-in is refused ("credited already").
- The operator's only exit is the loss (`returnAdvance(0)`), with the payout sitting unattributed at our Ethereum address.
- The trigger is one transient outcall failure (a provider's 429 or disagreement) on the one tick the payout is judged. There is no
  attacker. Low: rare, bounded, and an upgrade repairs it.

**PoC** (`poc/C/r5_c.rs`, passes, with the harness switch):

- `c5_l3_a_failed_balance_read_after_the_judge_marks_a_payout_credited_that_was_never_applied`: a written-off forward order's
  0.498 ETH fill on Ethereum is handed in, and the second balance read fails once.
- `late.credited = Some(…)`, `st.credited` holds the hash, the buy is `Stuck`, nothing is earmarked, and the second hand-in is
  refused.

**Fix.** Read the balance once, before any write, and pass it to `arrive`. Or write `credited` only after the branch succeeded, and
make the whole closure write-free until its last fallible call.

### Infos

- **C5-I-1: the judge's shapes likely do not match Relay's real payouts.**
  - The fixture's `requestId` differs from `protocol.v2.orderId`. Relay's fills tag the request id, as far as I know; I could not
    measure one here.
  - `orderData.solver` is a Base address.
  - Relay v2 refunds from the depository are allocator-signed calls: `to` is the depository, the value moves in an internal
    transaction, and the input names a withdrawal request.
  - So a real refund may name neither the order id nor come from the solver. The operator then credits it only through C5-L-1's loose
    paths, for example by citing our own deposit transaction and stating the amount. That works because the balance check sees the
    refund. §9 call 4 names half of this.
  - Measure one real late fill and one real refund of each direction before launch, and pin the judge to their shapes.
- **C5-I-2: rule 4 is not enforced as written.**
  - `if o.back && !home && paid < value/2` (`:3526`) repeats the general check below it. Every chain combination is accepted.
  - A way-back fill on 4663 is held only to half its value, not to its `min_out`.
  - Harmless as written, since both refunds and fills may land on either chain under rule 8. The comment overstates it.
- **C5-I-3: the claim's TTL is 600 s and a refused claim is dropped.** A claim refused for a transient reason (Ethereum not final
  yet, a read error) must be handed in again. That is correct, but the Ethereum finality wait (about 13 min) is longer than the TTL:
  a payout handed in as soon as it mines is always refused once, and must be handed in again after finality. That is operational
  noise, not a loss.

## What holds (re-attacked, no finding)

- **Before the write-off**, a late payout is read only by its own watch. The watch is kept past deadline + grace + 24 h
  (`:1747-1755`), and no second watch can open while it holds the leg.
- **The write-off is refused** until the canister has watched the order a day past deadline + grace, on both legs.
- **No order is credited twice**, and no transaction is credited twice canister-wide. No credit exceeds the order's value, and none
  goes to another collection's buy or module. A closed buy's payout reaches its own module through `fundReserve` or its own
  collection's leftover.
- **The Ethereum side of the balance check** subtracts every earmark and all booked leftover. `arrive` cannot fail for room after it.
- **Every `give_back_now` path returns only ETH measured as the buy's own**, and none reads the raw 4663 balance. `returnAdvance` and
  `fundReserve` go only to the buy's own `b.module`.
- **The solver field is bound by the order id**: rule 6 recomputes the EIP-712 hash over `orderData`, `address solver` included. The
  weakness is that it is shared, not that it is forgeable.
- **The endpoint** is operator-only, refused while paused and mid-tick, limited to the two chains, and capped at 8 claims.
- **Upgrade**: every new field is optional and old states restore, as listed above.

## PoCs

`poc/C/r5_c.rs` holds the tests. `poc/C/r5_c.harness.diff` is the balance-read-failure switch against `tests/audit_nft_r4.rs`.
`poc/C/r5_c.output.txt` is the run, with the per-tick `  | ` lines removed.

Run: copy `tests/audit_nft_r4.rs` to `tests/r5_c.rs`, apply the diff, append `r5_c.rs`, then
`cargo test --test r5_c -- --nocapture --test-threads=1`.

| Test | Finding | Result |
| --- | --- | --- |
| `c5_m1_a_payout_after_the_write_off_is_still_read_as_another_collections_refund` | C5-M-1 | **Passes** (weakness present) |
| `c5_l1_a_the_judge_credits_another_relay_payment_from_the_shared_solver_paid_from_the_frog_sides_eth` | C5-L-1 | **Passes** |
| `c5_l1_b_the_judge_takes_our_own_deposit_or_any_call_carrying_the_id_at_the_amount_the_operator_states` | C5-L-1 | **Passes** |
| `c5_l1_c_a_payout_is_credited_from_a_watched_orders_unread_refund_and_that_order_then_holds_the_relay_leg` | C5-L-1 | **Passes** |
| `c5_l3_a_failed_balance_read_after_the_judge_marks_a_payout_credited_that_was_never_applied` | C5-L-3 | **Passes** |
| the 22 round-4 tests of the copied harness (`r4_*`) | control | **Pass** (the new switch is off by default) |
