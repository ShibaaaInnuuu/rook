# Round 6, part C: the way back after C5-M-1 (the late hold, `eth_late_release`, the payout judge)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `e04fa16`
(delta `8ef82d6..e04fa16`; canister commit `e811911`). Scope: C5-M-1's fix and what it touches: `ethflow.rs` `EthState::late_hold` /
`relay_busy`, `buy_pass`, `sweep_pass`, `eth_act` (the way back), `give_back_now`, `put_payout`, `judge_payout`, `payout_pass`,
`late_release`; `relayeth.rs` `EthDeposit.request_id`; `ethnet.rs` `eth_credit_payout`, `eth_late_release`; C5-L-1, C5-L-3; the
carried Lows C4-L-1, C3-L-1, E3-L-2, B-L-6 and C5-I-1. Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review6/poc/C/`; nothing committed, pushed, deployed or signed.** The
PoCs ran in a private APFS clone of `autobuyer/canister-rs` at `/tmp/rook-r6-C` (checked identical to the worktree with `diff -rq`
before anything was added; afterwards only `tests/audit_nft_r5.rs` differed; removed at the end). They are appended to the fixer's own
`tests/audit_nft_r5.rs` harness, unchanged otherwise: the 33 round-5 tests still pass beside them (38 of 38). Network use: two
read-only `eth_call`s to Relay's depository (on 4663 and on Ethereum), `poc/C/depository_probe.output.txt`.

Severity scale and threat model: `../README.md`'s. A PoC that passes shows the weakness.

## Verdict

**C5-M-1 is closed.** While a written-off order is neither credited nor released, `relay_busy()` is true (`ethflow.rs:655-662`), and
every path that opens a Relay watch checks it: new buys (`buy_pass` `:1781-1789`), the way back and a sweep's way back (`eth_act`
`:2539`, which covers every `BackFunded`, sweeps included), and new sweeps (`sweep_pass` `:3846`). At the write-off there is no other
watch open (one leg canister-wide), so no watch exists that can read the late payout. Round 5's PoC fails (re-run in
`fixes-round5-poc/C…`, and the fixer's `r5_c5_m1_a_…` holds). What the hold does **not** stop is right: burns, redemptions,
deliveries, posts, deposits (no Relay leg), and Arrived buys' fills. The only way back into C5-M-1's shape is an operator act:
`eth_late_release` (documented, §9 call 4), or a credit of a transaction that is not the real payout (C6-L-1).

**C5-L-3 is closed.** `payout_pass` (`:4070-4157`) reads the transaction, finality, our address and the balance first; the one step
that can refuse (`arrive`) runs before `st.credited` and `late.credited` are written. The fixer's `r5_c5_l3_a_…` holds.

**C5-L-1 is only partly closed (C6-L-1).** The solver-only path is gone and a plain transfer must now name the order, be from someone
else and mine after the deposit. But the depository path accepts any successful call to Relay's depository that is not
`depositNative` and carries the order id anywhere in its input, at the amount the **operator states**. The real depository answers
`allocator()` with trailing bytes (probed): a stranger can mint such a "payout" for gas. The request id is whatever the runner's quote
says. So the judge is still not a check on the operator's hand-in, only on its shape.

**Two new Lows in the credit path itself** (C6-L-2, C6-L-3): a credit does not look at what the buy has out on 4663, and a credited
forward payout revives a buy whose custody reservation was already released.

No finding lets anyone take funds without the operator handing in a transaction, and none freezes the canister.

## Counts (part C, new)

| Severity | Count | IDs |
| --- | ---: | --- |
| Critical | 0 | |
| High | 0 | |
| Medium | 0 | |
| Low | 3 | C6-L-1, C6-L-2, C6-L-3 |
| Info | 3 | C6-I-1, C6-I-2, C6-I-3 |

## Round-5 findings in scope: verdicts

| ID | Verdict | Basis |
| --- | --- | --- |
| **C5-M-1** the write-off frees the leg; the late payout is read as the next order's | **Closed** | `late_hold` (`:660-662`) finds any buy whose `late` is neither credited nor released, whatever its phase (a buy closed as a loss still holds). Every watch-opening path checks `relay_busy()` / `late_hold()` (above). `buy_pass`'s early return at `:1781` is filtered on "no buy in relay", and falls through to `relay_busy()` at `:1786` otherwise, so both cases stop. The hold ends only by a credit (`payout_pass` `:4148-4150`) or `late_release` (`:4165-4177`). Residual by design: after a release, a late payout lands in the next watch (C6-I-2) |
| **C5-L-1** the judge does not bind a payout to the order | **Partly closed** → **C6-L-1** | Closed: the solver path; our own `depositNative` (`t.from == us`); "mined after the deposit" (`:4040-4044`) where the pin was read. Open: the depository path (`:4050-4051`) at the stated amount; the request id is not bound; rule 3 is silently skipped when the deposit's 4663 pin read failed (C6-I-1) |
| **C5-L-3** a credit recorded before a fallible read | **Closed** | Reads first; `arrive` (the only fallible write) before any record (`:4099-4150`). A refused claim is dropped and can be handed in again (C5-I-3, unchanged) |
| C5-I-1 the judge's shapes are not measured against real Relay payouts | **Open, Info; a launch precondition** | Unchanged; a runbook task (`phase1-nft.md` §5.1). Note: if Relay's refund is an internal transfer from its depository's `execute`, the depository path is the only one that can credit it, so C6-L-1's fix and this measurement go together |
| C5-I-3 a claim's TTL 600 s, dropped on a transient refusal | Unchanged, Info | `payout_pass` `:4071` removes the claim before judging it |

## New findings

### C6-L-1 (Low): the depository path credits a stranger's gas-only call at the operator's stated amount

- **Code:** `ethflow.rs:4050-4051` (`judge_payout`, the depository branch); `:4034-4039` (names the order: any 32-byte window of the
  input); `relayeth.rs:224-225` (`request_id` from the quote's `steps[0].requestId`, not bound by the order hash); `payout_pass`
  `:4088-4100` (on 4663 the balance check subtracts only the open buys' `Funded` / `Returning` wei).
- **What passes:** `t.to == RELAY_DEPOSITORY_4663`, first four bytes not `depositNative`, the order id (or request id) anywhere in the
  input, `t.from != us`, mined after the deposit, `named` between half and the full value. `paid = named`: the amount comes from the
  operator, not the transaction.
- **Why a stranger can make one:** the order id is public (it is in our own `depositNative` calldata). The depository has no fallback,
  but its view functions ignore trailing calldata: `eth_call` of `allocator()` + 32 bytes succeeds on 4663 and on Ethereum
  (`poc/C/depository_probe.output.txt`). A 0-value transaction to the depository with that input costs only gas.
- **Also:**
  - a plain ETH transfer to us of at least half the order's value, with the order id as data, is credited at its value. It costs the
    sender real ETH, but it ends the hold;
  - a runner that hands in a quote whose `requestId` is, for example, our own address word makes every call carrying our address
    "name" the order (unit PoC).
- **Effect (needs the operator to hand in the wrong transaction):**
  - on 4663 the stated amount is paid to the module (`returnAdvance` / `fundReserve`) out of whatever else sits at the shared address
    (the frog side's float, gas keeps);
  - the credit ends the C5-M-1 hold, so the next order's watch reads the real payout when it lands: C5-M-1's shape, re-opened.
- **Why Low:** only the operator can hand in a payout, and it is meant to take the hash from Relay's own status. The judge is defence
  in depth against a misled operator (a phished hash, a compromised status page), and against that it checks only the shape. With
  C6-L-1 the depository path still accepts what C5-L-1 showed, minus the solver.
- **PoC:** `poc/C/r6_c.tests.rs`:
  - `r6_c_l1_a_…`: a stranger's `allocator()`+order-id call, handed in with 0.4855 ETH stated, is `CREDITED`. `returnAdvance` pays
    0.4855 ETH out of the other flows' 2 ETH, the hold ends, and Azuki's new order starts;
  - `r6_c_l1_b_…`: the judge accepts an ERC-20 `transfer(us, 1)` sent to the depository through a runner-chosen request id, and a
    half-value plain transfer carrying the order id.
- **Fix:**
  - Bind the depository path to Relay's own record. Require the receipt to hold the depository's execution event whose indexed or
    data fields name the order or request id, from the depository's address. Require the rise of our balance across that block
    (`eth_getBalance(us, block-1)` against `(us, block)` where the provider serves that state) to be at least `named`.
  - Refuse `named` otherwise, and do the C5-I-1 measurement first, to learn which log Relay emits.
  - Bind the request id: take it only when it is the order id itself, or when a second source agrees (Relay's `/intents/status` by
    order id, read by the operator), and never a word that is our own address, or zero-padded.
  - On 4663, subtract the frog side's accounted ETH in the balance check (with B-L-6).

### C6-L-2 (Low): a credit taken while the loss-closing `returnAdvance(0)` is out is recorded as returned and never paid

- **Code:**
  - `payout_pass` runs at tick step 4 (`:978`) whatever `st.pending` holds;
  - `(true, true)` rewrites the phase to `Returning { paid }` (`:4125-4129`); `(false, true)` calls `arrive` (`:4130`);
  - the `GiveBack` resolution then takes `wei` from the current phase, not from the transaction (`:3718-3726`);
  - the resolution also settles the custody reservation (`:3722`).
- **Path (the documented operator flow):**
  1. the write-off;
  2. `eth_give_back` again ("closes it as a loss"), so `Returning { 0 }`, and the next tick signs `returnAdvance(0)`;
  3. the late payout lands and the operator hands it in;
  4. at the next tick the `returnAdvance(0)` is not yet resolved (its receipt `Unsure`, not yet visible at the pin, or any of
     F5-M-1's unseen cases), so `payout_pass` credits it.
- **Effect:**
  - **4663:** the buy is recorded `Returned { wei: paid }`, the module received 0 (and booked the whole advance as a loss), the payout
    sits at our address attributed to nobody, and the credit is spent (the same transaction can never be handed in again).
  - **Ethereum:** the buy becomes `Arrived` with the payout earmarked, the `returnAdvance(0)` then resolves as "not returning", and the
    module's advance is closed. The Arrived buy later either fills and mints to a module with no open advance, or goes back.
    `home_act`'s `Returning` then sees `!transit_open` (`:3250-3254`) and records it `Returned` without paying. Either way the ETH ends
    at our address, belonging to nobody.
- **Why Low:** it needs a pending home transaction that stays unresolved across a tick, in the window the operator hands the payout in.
  No theft: the ETH stays at our address. But it is exactly the "a payout is nobody's" outcome C4-M-1 set out to remove, reached
  through the operator's documented sequence.
- **PoC:**
  - `r6_c_l2_…`: `Returned { wei: 485535083200866034 }`, 0 wei returned to Milady's module, `late.credited` set, and a second hand-in
    refused;
  - `r6_c_l2_b_…`: `Arrived { 0.4984 ETH }`, module transit closed, the resolution line "not returning".
- **Fix:** in `payout_pass`, refuse (keep the claim) while `st.pending` is a step of that buy (`GiveBack`, `Settle`, `Mint`), and
  judge it after the pending transaction resolves. Also make the `GiveBack` resolution use the value the saved transaction carried
  (the raw's value), not the phase's.

### C6-L-3 (Low): a credited forward payout buys an item the custody book never counts

- **Code:**
  - the write-off of a forward order leaves `Stuck` with `last = None` (`:3977-3978`);
  - `buy_pass`'s reservation sweep then releases `eth:{id}` as "a buy that can no longer land" (`:1765-1776`, the test `:1770`);
  - a late fill credited on Ethereum makes it `Arrived` again through `arrive` (`:4130`) with no reservation;
  - the reservation is made only at the advance (`:3302-3306`, `:3612-3614`);
  - at the mint `custodycap::settle("eth:{id}", held)` fails "not in flight" (`:3743-3745`), and the line only notes it.
- **Effect:**
  - the item is in custody and on the wrapper, but not in the custody book: H2's count and the exit rule's `held_fits` (A5-L-1) are
    one item short per such buy;
  - a collection at its cap can go one item over; the upgrade-time exit check reads one item fewer than custody holds;
  - the release at delivery ignores the missing entry, so nothing wedges.
- **Why Low:** it needs Relay to miss a forward order past deadline + grace + a day, then fill it, and the operator to credit it while
  the buy is open. It is bounded to one item per such event, with no loss of funds. It breaks an H2 invariant the fixes promise to
  keep ("nothing is weakened").
- **PoC:** `r6_c_l3_a_…`: the reservation is gone after the write-off; `CREDITED`, `Arrived`, fill, mint, `Settled`; the custody book
  has 0 entries, the line reads "the custody book: eth:0 is not in flight".
- **Fix:** keep the reservation while the buy's `late` order is uncredited and unreleased (add
  `b.late.as_ref().is_none_or(|l| l.credited.is_some() || l.released_at.is_some())` to the sweep's `Stuck` test). Or re-reserve in
  `payout_pass`'s `(false, true)` before `arrive`, refusing the credit into leftover when the cap is full.

### Infos

- **C6-I-1: rule 3 depends on a read that may silently be missing; the request id is unbound.**
  - `home_pin` is read `ok().flatten()` at both deposit sites (`:2559`, `:3356`). A failed read leaves `Watch.home_pin = None`, and
    `judge_payout` then skips "mined after the deposit" on 4663 for that order (unit PoC: a block-1 transaction passes). Pre-round-5
    watches have `None` too.
  - Fix: refuse to sign the deposit without the pin (the Ethereum `pin` is already mandatory), and require it in the judge.
  - The request id: C6-L-1.
- **C6-I-2: a release cannot be undone by crediting afterwards.**
  - The runbook and `late_release`'s text say a payout "can still be credited" after a release. But the next order's watch took its
    baseline (`eth_before` / `balance_after`) before that payout landed, so it counts the payout even when the operator credits it to
    the written-off order first. The new order's `arrive` then takes that much out of un-earmarked ETH (gas pot, leftover is
    subtracted), or waits.
  - Say in `phase1-nft.md` §5.1: release only once Relay's records show the order's ETH will never reach our addresses (refunded
    elsewhere, or definitively lost), never to "free the leg" while a payout may still come.
- **C6-I-3: the hold covers the NFT side only.**
  - `convflow.rs:135` (the frog side's Relay conversion watch) reads any rise of half its value at the shared 4663 address as its
    refund, `bal − GAS_KEEP` raw. A late NFT payout landing while a frog conversion is watched is taken there.
  - This is B-L-6's "vice versa", unreachable in the NFT-only launch stack. Carried there, not counted again.

## The operator endpoints

| Endpoint | Guard | What it authorizes | Abuse checked |
| --- | --- | --- | --- |
| `eth_late_release(buy)` (`ethnet.rs:604-609`) | `only_operator`, not paused, not ticking | Sets `released_at` on an uncredited `late` order. Ends the hold, nothing else: moves no ETH, signs nothing | Refused for no late order, credited, released twice (`:4165-4177`). Cannot pay twice: a later credit still needs the judge, `st.credited` (per transaction) and the balance check with earmarks. A premature release re-opens C5-M-1's attribution error (C6-I-2): an operator call, bounded by the order's value, no double pay found |
| `eth_credit_payout(buy, chain, tx, wei)` (`ethnet.rs:587-600`) | `only_operator`, not paused, not ticking; the chain must be 4663 or the NFT chain | Queues a claim (at most 8, one per buy), judged next tick | C6-L-1 (what the judge accepts), C6-L-2 (when it runs), C6-L-3 (what a forward credit revives) |
| `eth_give_back(buy)` (`ethnet.rs:573-579`) | `only_operator`, not paused, not ticking after the balance read | The write-off (`BackBridging` / watched `Stuck` past deadline + grace + 1 day), then the loss close | Unchanged from round 5; the write-off now sets the hold |

## Carried Lows: re-rated

| ID | Re-rating | Why |
| --- | --- | --- |
| **C4-L-1** (the Relay leg held by a late order) | **Low, unbounded in time, operator-bound** | The leg is held from the watch's deadline + 1 h + 24 h (8–9 days with Relay's 7-day deadlines) until the operator credits or releases; there is no automatic end. Meanwhile no collection starts a buy, no Arrived buy goes back (it cycles `BackFunded` ↔ `Arrived` every `BACK_QUOTE_SECS` / `BACK_RETRY_SECS`, harmless) and no sweep runs. Burns, redemptions, deliveries, deposits and posts go on. The monitor pages the written-off order (SEV3 with the hold) and `Stuck`. Holders' exits are unaffected, so it stays Low |
| C3-L-1 (the advance gate assumes 50 bps where the item cap binds) | Unchanged, Low | `ADVANCE_FEE_EST_BPS` 50 (`:77`); not in the delta |
| E3-L-2 (gas drift deadlocks an arrival) | Unchanged, Low | `arrive` (`:2023-2040`) unchanged; the credit path's `arrive` subtracts earmarks and leftover first |
| B-L-6 (the shared 4663 address) | **Partly fixed, Low; unreachable NFT-only** | The judge now needs the order named, but on 4663 `payout_pass` still counts the frog side's ETH as free (C6-L-1's effect), and `convflow.rs:135` still reads an NFT inflow as a frog refund (C6-I-3) |

## Upgrade safety (this area)

**Holds.** `Watch.request_id` / `home_pin` and `LateOrder.released_at` are `opt` with `#[serde(default)]`. A round-4 state's
uncredited `LateOrder` decodes with `released_at = None`, so after the upgrade it holds the leg at once, as intended. Its watch has no
`home_pin`, so rule 3 is off on 4663 for it (C6-I-1). No round-4 build was deployed, so this is moot in production.

## PoCs

| File | Result |
| --- | --- |
| `poc/C/r6_c.tests.rs` | Appended to `tests/audit_nft_r5.rs` in the private copy. **5 of 5 pass** (each shows its weakness): `r6_c_l1_a_…`, `r6_c_l1_b_…` (C6-L-1, C6-I-1), `r6_c_l2_…`, `r6_c_l2_b_…` (C6-L-2), `r6_c_l3_a_…` (C6-L-3). The harness's 33 round-5 tests still pass (38/38) |
| `poc/C/r6_c.output.txt` | The run's test lines and the PoCs' printed state (tick lines elided) |
| `poc/C/depository_probe.output.txt` | The read-only `eth_call`s: `allocator()` + 32 bytes succeeds on Relay's depository on 4663 and Ethereum; an unknown selector reverts |

## Recommended fixes (order)

1. **C6-L-3:** keep the custody reservation while a buy's late order is open (one line in the sweep's test).
2. **C6-L-2:** no credit while that buy has a home transaction out; the `GiveBack` resolution takes the transaction's value.
3. **C6-L-1 with C5-I-1:** measure a real Relay fill and refund each way. Then bind the depository path to Relay's own execution event
   and to our balance's rise at that block, and bind or drop the runner's request id.
4. **C6-I-1:** a deposit's 4663 pin is required, not optional. **C6-I-2:** the runbook's release rule.
