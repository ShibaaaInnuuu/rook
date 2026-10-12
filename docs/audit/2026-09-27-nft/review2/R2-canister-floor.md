# Review round 2, part B: the canister's floor and judges (after fix round 1)

Reviewer: independent AI review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `27c0962`
(the round-1 delta `8c107e9..27c0962` on top of `nft-pool`). Pre-fix tree for comparison: `Rook-nft-pool` at `8c107e9`.
**Nothing in the code was modified. Nothing was deployed, pushed or signed.** No network calls; everything ran in the canister's
fake world (`cargo test`, toolchain from `~/.cargo/bin`).

Scope: `ethnft.rs` (the floor, `sales_in`, `counted_sales`, `floor_from`, `ListingTerms::fillable`), `ethflow.rs` (`floor_pass`,
`reading_of`, `post_reading`, `deposit_floor_ok`, `home_act`'s ordering), `ethnet.rs` (`fees`, `eth_listing`, `eth_sale_receipt`,
the size classes), `pin.rs` (`json_rpc_norm_body`, `said_reverted`), `nftnet.rs` (`agreed`), and the runner's sale hand-in
(`ops/src/ethnft.ts` `recentSaleTxs`) where it decides what the canister sees. Line numbers are at `27c0962`.

PoCs (`poc/B/`), removed from the tree after the run so the suites stay green:
- `audit2_nft_b.rs`: 5 tests on the fixer's own regression fake (`tests/audit_nft.rs`'s harness, copied unchanged). Every test
  asserts the WEAKNESS and **passes** (`audit2_nft_b.output.txt`).
- `audit_nft_b.round1-adapted.rs`: round 1's `poc/B/audit_nft_b.rs`. It **does not compile unmodified** at HEAD: the `EthIo` trait
  gained `home_finalized_time` (H9 carried over), `floor_from` takes `Sale`s instead of prices, and the canister now reads
  `floorBounds()` before a post. The three minimal adaptations are in `audit_nft_b.round1-adapted.diff` (the home chain final at
  once; three wash sales with a distinct token, seller and buyer each, the strongest form for the attacker; `floorBounds()`
  answered `(0, max)`). Result: **0 of 11 pass** (`audit_nft_b.round1-rerun.output.txt`).

## Verdict (part B)

Every round-1 High and Medium in this area is closed as reported **except A-M-1**, whose canister-side gate compares the module's
floor with a sale median that anyone can set with gas-only wash sales (PoC), and which gates only the mint, not the later burn.
New: **B2-M-1** (Medium): the "sales only veto" rule makes the veto itself the attack. Ten gas-only ping-pong sales of one NFT
between two wallets, or seven in-band washes along a wallet chain, stop every post of a collection for as long as they are
repeated. A stale module refuses burns and mints until the D8 poster pause or the 7-day escape. Two Lows and three Infos.

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| New in round 2 (B2-) | 0 | 0 | 1 | 2 | 3 |
| Round 1, not closed | 0 | 0 | 1 (A-M-1) | 0 | 0 |
| **Open, part B** | **0** | **0** | **2** | **2** | **3** |

## 1. Round-1 items in this area

| ID | Sev (R1) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **B-H-1** self-trades steer the post | High | **Closed** (residuals **B2-M-1**, **B2-L-1**) | Both round-1 PoCs fail. With three distinct-wallet washes at 1.5 ETH, the sales are outside ±30 % of `L`: 0 counted, so there is no post. The 12 self-trades are not counted, so there is no post either. `floor_from` (`ethnft.rs:577-621`) always posts `L`, the rank-2 verified listing (one per offerer, one token once), or nothing. No path posts a value that is not a verified, fillable listing. Self-trades, one-per-token/seller/buyer and the zero seller are dropped (`:529-535`). But the veto that remains is cheap to trigger (B2-M-1) and cheap to disarm (B2-L-1). The fix doc's claim "a wash ring needs as many wallets as the real sellers it outvotes" is false for the veto: far sales are dropped *before* the dedup, so one NFT and two wallets suffice. |
| **D-H-2** (canister half) unclamped post | High | **Closed** | `post_reading` (`ethflow.rs:1721-1742`) reads `floorBounds()` at the 4663 pin and posts `clamp(reading, lo, hi)` (`:1736`), with `(0, max)` before the first post. I checked the fix's premise that the bounds only widen between the read and the transaction against `FloorModule.floorBounds()` (`FloorModule.sol:419-429`). `step` grows with elapsed time up to an hour. When the day window rolls, the anchor resets to `accepted`. Worked through for a floor that rose or fell during the day, that never narrows `[lo, hi]`, because `prev ± step` (≤ 5 %) binds before the ±20 % day band. Only posts change `accepted`, the canister is the only poster, and home transactions are one at a time. The fixer's regression `d_h_2_…` passes. The code comment at `:1719` says the bounds "only narrow … (they widen with time)", which contradicts itself (B2-I-2). |
| **B-H-2 / C-H-1** one wait stops every post | High | **Closed** | `home_act` (`ethflow.rs:1789-1998`): every step's error is collected into `waits` and never returned early. An urgent post (module ≥ 40 min old) goes first, and every due post is tried in step 4. A `NeedAdvance` buy without value or room is dropped before its advance (`:1872-1876`). Round 1's two B-H-2 PoCs are inconclusive in the old fake: their control fails because the old fake's "honest" Azuki sales are self-trades (now not counted), and the deposit fixture now goes back as a stray (A-I-3). The fixer's regressions `b_h_2_c_h_1_…` and `c_h_1_a_l_1_…` pass and test the right thing. |
| **B-M-1** one bad receipt voids the pass | Medium | **Closed** (residual **B2-L-2**) | Round-1 PoC fails: "five good sales and three good listings, and still no post" is now a post. `reading_of` (`ethflow.rs:782-797`) counts an `Err` as unread and goes on. The crowding half of B-M-1 ("12 plain transfers crowd out every sale") is closed at the runner (it hands in only sales, `isSale`) but comes back through sales themselves (B2-M-1). |
| **B-M-6** replicas miss consensus on provider shapes | Medium | **Closed** | `json_rpc_norm_body` (`pin.rs:154-182`): keys sorted, strings lower-cased, errors reduced to `{code, reverted}`, and every unusable body is one error. I looked for a normalization that makes two *different* chain answers agree. Every normalized read (`eth_call`, `eth_getBalance`, `eth_getTransactionCount`, `eth_chainId`, `eth_getCode`, `eth_estimateGas`) is hex or a quantity, where case has no meaning. Error text is dropped, but an error is an error either way. The "error dressed as a result" rule needs both `code` and `message`/`error` keys, which no state read returns. Every `nftnet` state read uses `agreed` (`nftnet.rs:71-102, 188, 215, 241, 244`). `fees` refuses only on all-revert (`ethnet.rs:136-142`). |
| **B-M-7** evidence pass cycles | Medium | **Closed for honest load** (residual **B2-L-2**) | Cache by (collection, hash) once 128 blocks deep (`ethflow.rs:788-791`). Receipts are asked at 32 kB, and at 200 kB only on the IC's size reject (`ethnet.rs:373-386`). Listings stop at `min_listings + 2` distinct verified offerers (`:760-773`). Small reads are 8 kB, `getApproved` is skipped when approved-for-all, and there is an 8 T floor (`:728-731`). The fixer's `b_m_7_…` passes. Residual: an unreadable (over 200 kB) receipt is never cached and pays both classes again on every pass (B2-L-2). |
| **A-M-1** (canister side) deposit → burn at a high floor | Medium | **Not closed** | See §2. `deposit_floor_ok` (`ethflow.rs:1771-1784`) compares `accepted` with `r.post.sale_median × 1.05`. That median is the median of the counted sales among the 12 the runner handed in. Seven in-band wash sales (gas only) move it to 1.20 while the post stays 0.93, and the fixer's own A-M-1 state (accepted 1.00, +10 % over the real 0.91) then mints (PoC). Separately, the gate binds the mint, not the burn: a unit minted while the floor is in the band can be burned after the floor rises, and nothing on either side ties the two. |
| **B-L-1** unfillable listings as evidence | Low | **Closed** | `ListingTerms::fillable` (`ethnft.rs:182-189`), called in `eth_listing` before any read (`ethnet.rs:341-343`). |
| **B-I-1** `sale_nft_listing` on the eth chain | Info | **Closed** | `salenet.rs:84-86`. |
| **B-I-5** depth by listings, not offerers | Info | **Closed** | `floor_from` counts `best` (one per offerer) within 5 % (`ethnft.rs:617`). |

Also re-checked and unchanged in behaviour: the Seaport/Blur parsing in `sales_in` (`ethnft.rs:484-511`). The lowest price is kept
per token per receipt, and a sale needs the collection's own `Transfer`. Seller and buyer are the first `from` and the last `to`.

## 2. A-M-1, as it stands

Round 1's mechanism is unchanged on the contract side. A fresh burn pays `min(price(), _burnTarget()) × (1 − sBurn)`, capped at
the tier's backing β (`FloorModule.sol:525-539`). β is what graduations paid at `gradPrice`, which comes from the posted floor. The
fix adds one canister gate before a deposit's `mintDeposit`: the fresh reading's sale median × 1.05 must be ≥ `accepted`. It has
two gaps:

1. **The median is the washer's.**
   - `counted_sales` (`ethnft.rs:524-558`) keeps one sale per token, seller and buyer, within ±30 % of `L`. The median is taken
     over at most 12 hashes, the newest first (`ops/src/ethnft.ts:376`, `recentSaleTxs`).
   - Seven sales along a chain of eight attacker wallets (w1→w2 token 1, w2→w3 token 2, …) are seven distinct sellers, buyers
     and tokens. The price only moves between the attacker's own wallets, so the cost is gas plus any enforced royalty.
   - A Seaport order with no fee recipient, filled directly, still emits `OrderFulfilled` and counts. Milady, BAYC and MAYC do
     not enforce royalties on-chain.
   - Seven of 12 sets the median anywhere in [`L`, 1.3 `L`].
   - PoC `audit2_a_m1_seven_in_band_wash_sales_open_the_deposit_band_at_a_floor_10_percent_over_the_market`: at the fixer's own
     A-M-1 state (accepted 1.00, real sales' median 0.91), seven washes at 1.20 set the median to 1.20. The deposit is minted,
     and the post is still 0.93.
   - A malicious runner reaches the same result without washes, by choosing which real sales to hand in (the documented residual).
2. **Mint time is not burn time.**
   - A depositor mints while `accepted ≤ 1.05 S`, keeps the wX, and burns after the floor has walked up.
   - The walk can come from the runner withholding listings, from B2-L-1's sweep, or from an honest rise in the rank-2 listing.
   - Round 1's `test_A_M1b_…` arithmetic (graduation before the inflation, β = 1.03 F caps the burn) gives ≈ +0.084 ETH/day
     at +10 % for Milady at stage 0. Where a graduation lands during the inflation, it is round 1's +0.27 ETH/day.
   - Units bought in the coin pools are not gated at all, as round 1 noted.

**Severity: Medium (open).** It is bounded as round 1 bounded it: `burnPerHour`, H11's 3 mints a day, 3 items of custody room,
and β. **Recommended fix:** round 1's option 1, on the module: cap the fresh burn price by the lowest `accepted` of the trailing
24 h. That closes both gaps, because the burn is what pays. If the canister gate is kept, compare `accepted` with the **lowest**
recent counted sale (or the p25) rather than the median, and only over sales whose buyer and seller have not traded that token
with each other in the window. Either way, correct `fixes-round1.md` §4: the band is not "at most about 5 % of the sales' median"
against an outsider.

## 3. New findings

### Medium

#### B2-M-1: the sales veto is the attack. Gas-only wash sales stop a collection's floor post for as long as they are repeated

- **Where:** `autobuyer/canister-rs/src/ethnft.rs:597-613` (`floor_from`: fewer than `min_sales` counted, `l > hi` or `l < lo`
  is an `Err`, no post). `:524-535` (`counted_sales`: a far sale is dropped before the one-per-token/seller/buyer rule).
  `ops/src/ethnft.ts:255-282, 376` (`recentSaleTxs`: the 12 newest sales, with no price filter).
  `autobuyer/canister-rs/src/ethflow.rs:745` (an `Err` keeps no reading).
- **Mechanism.** The post needs at least 3 counted sales among the ≤ 12 handed in, and `S25 × 0.85 ≤ L ≤ S × 1.1`. Two cheap ways
  to break it:
  - (a) **Crowd:** make ≥ 10 of the 12 newest sales far from `L`, for example one NFT ping-ponged between two wallets at
    0.001 ETH. Far sales are "no vote", so the count falls under 3. Nothing in the dedup applies, because the band check runs
    first.
  - (b) **Veto from inside the band:** seven sales at 0.70–0.84 `L` (or above 1.18 `L`) along a wallet chain make the median or
    the p25 veto `L`. This defeats even a runner that pre-filters by the band.
  - In both cases the price moves only between the attacker's own wallets. The cost is gas: about 0.001–0.002 ETH per sale on
    a fee-less Seaport order.
  - The newest-12 sample means the attacker must keep up with honest volume: about one sale per honest sale, plus a margin.
    For Milady, that is tens of transactions a day, well under 0.1 ETH/day.
- **Impact.**
  - The collection's module gets no post. After `FLOOR_TTL` (1 h) it is not `fresh()`. Mints and graduation fills (`_priced`)
    revert `Stale`, and **burns revert `Stale`** (`FloorModule.sol:535`) until either the monitor's D8 `pause_on_no_good_post_h`
    pauses that collection's poster (burns then pay the pause floor) or `STALE_ESCAPE` (7 days) passes.
  - There is no fresh reading, so the canister's buys (`buy_cap`) and deposit mints (`deposit_floor_ok`) wait too.
  - The attacker can do this per collection or to all five, and choose the moment, for example during a real move so that
    exits are at the frozen pause floor. Redemptions are unaffected.
  - Round 1 graded the same class (B-M-1, crowding the sample to stop posts) Medium, and the monitor's poster pause is a switch
    that reopens burns. So: **Medium**.
- **PoC** (`poc/B/audit2_nft_b.rs`, passing):
  - `audit2_b2_m1_ten_dust_ping_pong_sales_stop_every_floor_post`: in the control, the five real sales post 0.93. Ten
    ping-pong sales of one token at 0.001 ETH plus the two real ones that still fit give "2 verified sales count", no reading
    and no post for 2 h.
  - `audit2_b2_m1_seven_in_band_washes_under_the_listings_veto_the_post_too`: seven chain sales at 0.70 give "more than 10 %
    over the sales' median 0.70: no post".
- **Recommended fix.**
  - Make a veto cost as much as moving the market. Take the sample by **time**, not by count: every sale of the last 24–72 h,
    one per seller and per buyer, weighted so that one wallet cluster cannot be a majority. Or, better, have the canister read
    the collection's `Transfer` logs itself (round 1's "better" fix, which also closes A-M-1's runner residual).
  - Do not turn a veto into "no post" when the last post was good. Keep posting `L` at the module's minimum good confidence
    for a bounded time (for example, 6 h) and page. The module's ±5 %/h and ±20 %/day limits still bound the walk.
  - At least have the runner hand in only in-band sales (the canister's `SALE_BAND_BPS` around the rank-2 listing it hands
    in). That removes form (a), the cheapest one.

### Low

#### B2-L-1: the upward veto is wash-disarmable, so a sweep of a thin book is posted

- **Where:** `ethnft.rs:604-609` (the `l > hi` veto, with `hi` from the counted median) and `:524-558`.
- **Mechanism.**
  - The only defence against a posted floor above the market is the veto `L > S × 1.1`. `L` is a real listing, so it can rise
    only when the cheaper listings are bought.
  - The depth is thin. D6 measured 3 offerers within +5 % for Milady and 2 for MAYC. The canister's own buys also take the
    cheapest listing each time.
  - An attacker buys the few honest listings under their target, keeps two wallets listing at the target, and hands the
    "market" seven chain washes near it. The median follows, so the veto passes.
  - PoC: after a sweep, the honest sales (median 0.91) veto a book of 1.10 / 1.12 / 1.30. Seven washes at 1.30 make the post
    1.12, +23 % over every real sale.
  - The module takes it at 5 %/h, up to 20 %/day. D8's references (OpenSea's and CoinGecko's floors) read the same swept book
    and agree, so the guardian does not pause.
- **Impact.**
  - Graduations during the inflation overpay (`gradPrice` = accepted × 1.03). The canister then buys the attacker's own
    listing at up to `min(reading, accepted) × 1.05` with that ETH.
  - Burns pay up to β (A-M-1).
  - This needs real capital (the swept NFTs, recovered by selling them into the inflated floor) and is bounded by the day band
    and stage-0 custody. So: **Low**. It is a market-power attack that the sales check was meant to catch and does not.
- **PoC:** `audit2_b2_l1_after_a_sweep_seven_wash_sales_let_the_post_follow_the_attackers_own_listings` (passing).
- **Fix:** as B2-M-1 (a time-based, wallet-diverse sample read by the canister). Also cap a post's rise over the **p25** of that
  sample, not the median.

#### B2-L-2: an oversized sale receipt is never cached; it pays both size classes on every pass

- **Where:** `ethflow.rs:782-797` (only an `Ok(Some)` receipt is cached; an `Err` is only counted) and `ethnet.rs:373-386`
  (32 kB then 200 kB).
- **Mechanism.**
  - Anyone can make a sale whose receipt is over 200 kB: a sale bundled with a few hundred junk logs from the attacker's own
    contract, costing well under 1 M gas.
  - The honest runner hands it in (it is a sale). The canister reserves the 32 kB class, is refused for size, reserves 200 kB,
    is refused again, and marks it unread, which is correct for B-M-1.
  - It is final, but it is never cached, so each later pass pays both classes again for as long as it stays among the newest
    12.
  - Twelve such receipts cost ≈ (32 kB + 200 kB) × 2 providers ≈ 4.9 B cycles each on 13 nodes. That is ≈ 59 B a pass, and
    ≈ 4.2 T a day per collection attacked at the runner's 20-minute cadence. They also crowd out every real sale (no post,
    B2-M-1).
  - Below 8 T the evidence pass is skipped **for every collection** (`:728-731`). A drain in one collection therefore stops all
    five floors until a top-up.
- **Impact:** a paid drain of about $6/day per attacked collection, and a cross-collection stall if the top-ups lag. The monitor's
  low-cycles alert covers it operationally. **Low.**
- **PoC:** `audit2_b2_l2_oversized_receipts_are_re_reserved_every_pass` (12 reads every pass, and no post).
- **Fix:** cache a final hash whose receipt the IC refused at 200 kB as "unreadable, final", never read again. Or have the runner
  drop sales whose receipt is over 32 kB.

### Info

- **B2-I-1: a veto does not retract the previous reading.**
  - An `Err` from `floor_from` leaves the last good reading in place (`ethflow.rs:745`). Its `due` post can still go out, and
    `buy_cap` and `deposit_floor_ok` keep using it, for up to `READING_TTL_SECS` (1 h) after the evidence said "no post"
    (`due_readings`, `:1744-1750`).
  - This is bounded by the TTL. Consider clearing `due` (and the reading's use for deposits) on a veto.
- **B2-I-2: the comment at `post_reading` (`ethflow.rs:1719`) contradicts itself** ("the bounds only narrow … (they widen with
  time)"). The code is right and the bounds only widen (§1). Fix the wording.
- **B2-I-3: fix judgment call #1 ("no post in a deep crash") understates what a stale module does.**
  - Burns do not "wait". They revert `Stale` until the poster is paused (the pause floor ≤ the last accepted × (1 − fillBand))
    or the 7-day escape (full backing, no floor).
  - Under honest trading, the rule lifts once about 3 of the 12 newest sales trade near the new level, so it is short. With
    B2-M-1 an attacker can hold it indefinitely.
  - State it on the Trust page. Confirm that the launch monitor config arms `pause_on_no_good_post_h` for every eth collection,
    which is the only thing that reopens burns before 7 days.

## 4. Commands and results

```
cd autobuyer/canister-rs
cargo test --test audit_nft_b_r1   # round-1 PoC, 3 minimal adaptations (diff in poc/B): 0 passed, 11 failed (each on its attack
                                   # assertion, except the two B-H-2 tests whose control no longer holds in the old fake)
cargo test --test audit2_nft_b     # this round: 5 passed (each shows the weakness)
cargo test --test audit_nft        # the fixer's regressions: 21 passed
cargo test --lib ethnft::          # 14 passed;  cargo test --lib pin::  7 passed
```

Both PoC files were removed from `tests/` after the run. The copies and outputs are in `poc/B/`.
