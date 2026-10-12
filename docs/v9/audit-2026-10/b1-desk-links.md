# B1: the intent desk and the chain links

| | |
|---|---|
| Scope | `autobuyer/canister-rs/src/desk/` (`intent.rs`, `quote.rs`, `build.rs`, `run.rs`, `state.rs`, `api.rs`: the direct mint / burn / redeem desk, its deposit addresses `K_int[n]`, fee, minimums, rate limits, the 300-open cap, the tick, refunds, expiry, cleanup, reconciliation) and `autobuyer/canister-rs/src/chainlink/` (`ledger.rs`, `gate.rs`, `memo.rs`, `recon.rs`, `driver.rs`). `src/books/` is read as the reference the desk writes into, not audited (its own regressions stand). Design: `docs/v9/canister-only-solana.md` §4.4, §6, §10.2. |
| Commit | `e4d4d34` on branch `golive-2026-10-08` (the B0 fixes are in) |
| Reference | `docs/v9/audit-2026-10/b0-review-2026-10-11.md` (F1–F12 are not re-reported; F1 and F2 are verified below), the README's method |
| Method | Manual line-by-line review against the threat model below; every book writer outside the desk located (`desk::with_mut` call sites) for the F1 check; a proof-of-concept test for every High (and, because they were cheap, for every Medium) in `autobuyer/canister-rs/tests/audit_b1_desk_links.rs`, each **passing by exhibiting the bug** on the current code (its header says what a fix must make it refuse); no `src` file changed. |
| Result | 0 Critical, **2 High**, **4 Medium**, 6 Low, 6 Informational. F2 verified. F1 verified at every decision site and found incomplete at the outcome side (B1-06). |

## 1. Threat model

**Assets**
- The SOL at each module address `M[i]` (the collections' backing, reserve, surplus, revenue) and the wX supply of each collection: the books say what they are, the chain holds them, the links tie them (§4.4).
- `K_main`'s SOL: the fee payer of every transaction of the canister and the recipient of the desk's fee; the threshold-signature cycles.
- Users' deposits at `K_int[n]` (SOL) and at `K_int[n]`'s token accounts (wX, coins): only the canister's keys move them.
- The desk's availability: the one direct, poolless route to mint and burn; for holders of a halted or paused collection, the only exit.

**Actors and what each controls**

| Actor | Controls | Bounded by |
|---|---|---|
| Anonymous caller | `open_intent` (any `coll`, `recipient`, `refund_to`, `min_out`, destination), as often as the limits allow; reads every intent's addresses through `intent_status` | `MAX_OPEN` 300, 5 an hour a principal (120 shared by anonymous callers), 3 open a `refund_to`; `inspect_message` accepts it |
| Depositor / stranger on chain | Sends SOL to any `K_int[n]`, wX or anything to any token account (an intent's, `M[i]`, the recipient's future ATA), at any time: before, between the read and the landing, after expiry; burns their own wX | The balance rule at finalized; the books' minimums and dust |
| Holder of wX | `Burn` of their own tokens (owner-signed), transfers between accounts | nothing of the canister's |
| Provider (one of two) | Lies, lags, fails | Both must agree, at finalized; a disagreement is a wait |
| Operator | `desk_pause_collection`, `desk_set_paused`, `desk_clear_halt`, `desk_add_collection`, `desk_reconcile`, `desk_sweep_unbooked`, `sol_clear_line_incident` | `only_operator`, `not_paused`, `hold_tick` on the ones that read the chain |
| The IC | Interleaves ingress calls at every `await`; traps roll back to the last await; a message may be lost | `hold_tick` / `TICKING` serialise the ticks and the operator's chain methods; the desk's state is written only in synchronous sections |
| Solana | A transaction lands late, fails, or never lands (its nonce moved) | The line's verdicts (`solsend::tick`), `can_never_land`, the operator's clear |

**Trust.** Two providers agreeing at finalized are the chain. The books are right until a link's undo says otherwise. A memo is a label. `K_main` is honest and funded.

## 2. Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| B1-01 | A holder's own on-chain `Burn` of any wX puts the chain's supply under the ledger's; `supply_matches` is exact, so the next `pre_sign` halts the collection and the reconciliation strikes it; no operator method books the burn: the collection's desk and treasury links stop for good | **High** | `poc_b1_01_a_holders_own_burn_on_chain_halts_the_collection_for_good` |
| B1-02 | A token refund charges no fee and has `K_main` pay the destination's token-account rent, the fee and two signatures; one burn intent with an unmeetable `min_out`, cycled, drains `K_main` at ≈ 0.002 SOL + the signatures per cycle with no rate limit | **High** | `poc_b1_02_a_token_refund_is_paid_by_k_main_and_charges_no_fee` |
| B1-03 | `execute_next` tries the five oldest funded intents and stops: five intents waiting on a transient refusal starve every other collection's intents for up to 6 h | Medium | `poc_b1_03_five_waiting_intents_of_one_collection_starve_every_other_collections_intents` |
| B1-04 | A base unit of wX sent to an intent's token account between the read and the landing fails the burn / redemption / token refund (`CloseAccount` of a non-empty account): three failures → refund, five → the operator; each costs `K_main` a fee | Medium | `poc_b1_04_a_dust_transfer_to_the_intents_token_account_fails_the_burn_three_times_and_then_its_refund` |
| B1-05 | Opening is free and anonymous: 300 openings close the desk to everyone for 24 h, renewably; expired intents stay watched for 7 days, so the per-tick batched reads grow beyond anything the cap bounds | Medium | `poc_b1_05_three_hundred_free_openings_close_the_desk_and_stay_watched_for_a_week` |
| B1-06 | Review F1 is incomplete: the book writes made at a line OUTCOME (`home_books::landed`, `frogsol::books::landed`, the cranks' landed prize claim) run with no `collection_busy` check, inside `looper::step` before the desk's pass, and are wiped by an open desk link's undo | Medium | `poc_b1_06_a_book_write_at_a_line_outcome_under_an_open_desk_link_is_wiped_by_the_links_undo` |
| B1-07 | The partial fill of §6.1 is not implemented: a deposit over the hourly bucket (or the public cap's room) is `OverCap` / `OverBacked` for ever, waits 6 h and goes back less the fee | Low | none |
| B1-08 | A system account planted at the recipient's ATA address (≥ rent, by anyone) makes `create = false`; the `MintTo` fails simulation three times and the deposit goes back less the fee: a cheap block of one recipient's mints | Low | none |
| B1-09 | The per-recipient limit is keyed on `refund_to()`: three free openings naming a victim's wallet lock that wallet out of the mint for 24 h; for burns it limits `refund_to`, never the SOL recipient | Low | none |
| B1-10 | `desk_sweep_unbooked` checks the desk's links only: lamports of a money-line step landed at `M[i]` but not yet booked by its flow are swept as surplus, then booked again by the flow → `BooksUnbacked` → halt | Low | none |
| B1-11 | A `Stuck` link has no resolution: it holds the collection `Busy` and its effects in `totals` for ever (every operation waits 6 h and refunds; the reconciliation halts); `settle`'s lookup by signature reads a transaction with no `meta` as landed OK | Low | none |
| B1-12 | The memo scan's cursor jumps to the newest signature: over `SCAN_LIMIT` (100) signatures of `K_main` between two scans are never read, and anyone can append `K_main` to their transactions to push it past; unexplained memos are only listed | Low | none |
| B1-I1 | While the desk is paused a deposit is neither executed nor returned; once unpaused, anything funded over 6 h earlier is refunded, not executed | Info | |
| B1-I2 | A burn whose `paid − fee` is under the rent-exempt minimum, to a recipient with no account, fails simulation three times then refunds (the fee check should include the recipient's rent) | Info | |
| B1-I3 | Recipients and `refund_to` are not checked against the canister's own keys or executable accounts: wX refunded into a token account owned by the System Program id are lost; a burn paid to `M[i]` is a gift | Info | |
| B1-I4 | A trap between the line's save and the link's `signed()` leaves the intent `Funded` with `op = None`; the recovered link lands unattributed (`minted_units` not counted, the intent reads `Open` with its deposit gone): cosmetic | Info | |
| B1-I5 | 0.01 SOL sent to `M[i]` by anyone is an `Unbooked` incident every 10 min until the operator sweeps it (`INCIDENTS_KEPT` 64) | Info | |
| B1-I6 | `recon::compare`'s `if !matches!(…) \|\| out.is_empty()` is always true inside `if out.is_empty()` (dead condition) | Info | |

---

### B1-01: A holder's own burn halts the collection for good (High)

**Fixed (2026-10-11, the same day, after the report):** `supply_matches` accepts a chain supply UNDER the ledger's (over it stays `SupplyMismatch`); the reconciliation files a supply under the ledger as the soft finding `HolderBurn` and books it at once as the ledger's new `holder_burned` (`Books::book_holder_burn`, never under an open link), which `expected_supply` subtracts; the module keeps the burned units' backing. Regression: `fixed_b1_01_a_holders_own_burn_on_chain_is_booked_and_stops_nothing`.

**Where:**
- `src/books/mod.rs:614-618` `supply_matches`: `want == supply`, exact; called by `check_collection` (`:647-654`).
- `src/chainlink/gate.rs:60-61`: `pre_sign` ends with `check_collection` on the eventual numbers; `gate.rs:7-13` says so: "exact: a chain that drifted from the books in either direction stops here".
- `src/chainlink/driver.rs:91-103`: a gate refusal undoes the link and `link.halt(books, c, …)` the collection.
- `src/chainlink/recon.rs:66-70`: the reconciliation's `SupplyMismatch` is a hard finding; `recon.rs:92-107` two in a row halt.
- `src/chainlink/ledger.rs:283-286`: a halted collection refuses every link; `src/desk/api.rs:116-118`: and every opening.
- No method writes `ledger.sol_backed`, `held` or `open_tickets` from the chain (the only writers are the books' own operations and the undo, `ledger.rs:262`).

**Description.** wX is a classic SPL mint with no freeze authority. SPL `Burn` needs the token account owner's signature and nothing else: every holder can reduce the supply of their collection's wX by any amount, from one base unit (10⁻⁹ wX) up. The ledger's `expected_supply` does not change. §4.4 rule 2 (the supply guard) is deliberately one-sided (`≤`), but `check_collection` also requires equality, in the gate and in the reconciliation. After a holder's burn of one base unit:
1. the next link of the collection with a chain effect (a desk mint, burn or redemption; the treasury's surplus or prize link) is refused at `pre_sign` with `SupplyMismatch`, undone, and the collection is **halted** (`Ledger::halt`: no link, public mint paused);
2. the reconciliation finds the same every 10 minutes (two strikes → halt again);
3. `desk_clear_halt` lifts the halt; the next operation halts it again. No operator method books the burn; the books cannot be brought back to the chain without a code change.

**Impact.** A permanent denial of the collection's desk (mint, burn, redemption), of its treasury links, and of every opening (`open_intent` refuses a halted collection, so a holder's burn intent is refused at the door). It costs the attacker one public mint of the minimum (0.05 SOL, of which they keep the wX) and one `Burn` of one base unit. Every collection is exposed; the attack is anonymous and repeatable. Funds are not taken, but holders' exit through the desk is closed until an upgrade, and the monitor pages `SupplyMismatch` as SEV1.

**PoC.** `poc_b1_01_…`: a 5 SOL mint lands; the holder's ATA is reduced by one base unit and the fake mint's supply with it; a second user's 1 SOL mint ends `Refunded` with the collection halted (`halted[0].what` contains `SupplyMismatch`); `driver::reconcile` reports `SupplyMismatch`; opening a burn is refused "stopped by an incident"; after `clear_halt` + unpause, the next mint halts it again, and a burn opened in the clear window is refused and its wX sent back.

**Fix.** Make the chain seam one-sided wherever the books meet the chain: `supply ≤ expected` in `check_collection` (as `supply_guard` already is), with the shortfall reported by the reconciler as a soft finding (`HolderBurn { units }`), never a strike. Then book it: an operator (or automatic, after two reads agreeing) `book_holder_burn(coll, units)` that lowers `sol_backed` by the burned units and leaves their backing where it is (it becomes surplus at the next burn's release, as `release` already does for a unit whose backing exceeds its price); the burned units are fungible, so taking them from `sol_backed` is conservative (`live_units` falls, the per-unit backing of the rest rises). Keep the guard against a supply **above** the ledger's exact: that one is the SEV1.

---

### B1-02: A token refund is paid by `K_main` and charges no fee (High)

**Fixed (2026-10-11, the same day, after the report):** `refund_token_tx` closes `A`'s token account to `K_main` instead of to the refund's destination: its rent is the token refund's fee and covers the destination account `K_main` creates, so a refund costs `K_main` the transaction fee only and every repeat costs the depositor a fresh `A` account (0.002 SOL). Regression: `fixed_b1_02_a_token_refund_is_paid_by_the_deposits_own_rent`.

**Where:**
- `src/desk/build.rs:71-77` `refund_token_tx`: `create_ata_idempotent(a.main, to, a.mint)`, the transfer, `close_account(from, to, …)`, the memo. No fee instruction (`refund_sol_tx` at `:66-69` does charge it).
- `src/desk/run.rs:579-596`: the token branch of `refund_op`; `:346-354` `refused`: a non-transient books refusal (`Slippage`) is a refund at once.
- `src/desk/run.rs:206-241` `observe`: a `Refunded` intent that is not expired takes a new deposit as the next operation (no limit).
- `src/solsend.rs` / `solhome::sign_keys`: `K_main` is the fee payer; `KeyPath::Intent(n)` and `K_main` sign (two threshold signatures).

**Description.** A burn intent with `min_out = u64::MAX` is funded with the minimum 0.01 wX. `exec_burn` books `Slippage` (not transient) → `set_refund`. The refund transaction moves the wX to `refund_to`'s associated token account, **creating it with `K_main`'s lamports** (2,039,280) when absent, closes the intent's token account with its rent to `refund_to`, and pays the 5,000-lamport fee from `K_main`; the desk charges nothing. The attacker then closes the account they were given (its rent is theirs), sends the wX back to the same intent (its token account is recreated with their rent, which the close returns to them) and the same intent refunds again. One intent, no new opening, no limit but the desk line's pace (one transaction a tick, `BUSY_SECS` 10 s while anything is funded): ≈ 0.002 SOL of `K_main` plus two threshold signatures (≈ 0.07 USD of cycles) per cycle, around 3 SOL + 200 USD a day, for ever, at a cost of two Solana fees a cycle. The same holds for a redemption's leftover refund and for every token refund after a halt or the 6 h window.

**Impact.** `K_main` is the fee payer and nonce authority of every line of the canister: draining it stops everything, and the drain is funded by the canister's own cycles too. The amount per cycle is small, the rate is bounded by the line, the attacker's cost is near zero, and the attack is anonymous. Separately, every legitimate token refund costs the canister the rent it was designed to recover through the 0.0015 SOL fee (§6.1: "the fee covers 3 signatures").

**PoC.** `poc_b1_02_…`: `K_main` falls by `RENT165 + 5_000` on the first refund (the attacker's fresh wallet gains a token account worth `RENT165` plus the intent's closed account's rent), and by the same again after the attacker closes the account and re-deposits into the same intent; `done_ops` stays 0.

**Fix.** Never create the destination's token account in a refund: refund only into an existing token account of `refund_to` (read it in the same batch; absent → the intent waits with a note, and pages after the retry window). Count refunds in the limits (per `refund_to`, per intent: after N refunds the intent is closed and further deposits are left in place). Keep a token refund's fee out of `K_main`: either take the intent's own token-account rent (close it to `K_main` rather than to the user, covering the 5,000 fee and part of the signatures) or require a small SOL balance at `K_int[n]` for a token intent.

---

### B1-03: Five waiting intents starve every other collection (Medium)

**Fixed (2026-10-11, the same day, after the report):** `execute_next` orders candidates by `(skipped_at, funded_at, id)`: an intent the pass tried and skipped (a refusal that may pass, or any other skip) is stamped `Intent::skipped_at` and goes behind every never-skipped intent, then the least recently skipped come first. Each tick therefore reaches intents it has not tried yet, still at most `TRIES_PER_TICK` (5) of them; among intents that can go the FIFO is kept, and the stamp is cleared when a transaction goes out. Regression: `fixed_b1_03_five_waiting_intents_of_one_collection_no_longer_hold_another_collections_intent`.

**Where:** `src/desk/run.rs:261-284` `execute_next`: `TRIES_PER_TICK = 5`; candidates sorted by `(funded_at, id)`; a `Skip` moves to the next; after five the pass ends. `run.rs:346-356` `refused`: a transient refusal (`Stale`, `Paused`, `OverCap`, `CapStale`, `OverBacked`, `NotKeeper`, `NoItem`, `ArrivalRateLimited`, `NotEnoughBacked`) keeps the intent `Funded` with its `funded_at`.

**Description.** A waiting intent keeps its place at the head of the order (its `funded_at` never moves). Five intents of one collection whose floor is stale (its poster paused, its source down, no post in 3 h), or whose hourly bucket or public cap they exceed, are the five oldest for up to `RETRY_WINDOW_SECS` (6 h); every tick tries exactly those five and stops. Every funded intent of every other collection waits, unsigned, until the five are refunded, and then the next five. No attacker is needed; with one, five deposits over the bucket of a cheap collection (refunded less the fee every 6 h) hold the desk indefinitely.

**PoC.** `poc_b1_03_…`: two collections; collection 0's floor is stale; five 2 SOL deposits on it and one on collection 1; four rounds sign nothing; with one of the five taken out of the running, collection 1's mint lands at once.

**Fix.** Order candidates by `(last_tried_at, funded_at, id)` (a tried intent goes to the back), or skip a collection known to refuse transiently this tick, or try at most one intent per collection per pass. Keep the FIFO among executable intents.

---

### B1-04: Dust to an intent's token account fails the burn and then its refund (Medium)

**Fixed (2026-10-11, the same day, after the report):** `burn_tx` and `refund_token_tx` no longer end with `CloseAccount`, so they land whatever dust a stranger sends to `A` (the dust stays at `A`). The close is a transaction of its own (`build::close_tx`, `Purpose::Close`, `OpKind::Close`, memo `rook:v1:-:close:<id>:i<n>`): after a burn or a token refund landed, a later tick reads `A`'s token account at finalized and closes it to `K_main` only if it is empty. A non-empty read or a failed close counts as a try, and after `MAX_CLOSE_TRIES` (2) the account is left open with its rent at `A`. The rent goes to `K_main` in both cases: it pays for the extra transaction and, for a refund, for the destination account `K_main` created, so B1-02's economics hold (a burn's recipient no longer receives the 0.002 SOL rent). B1-02 stays closed under the split: `refund_op` lets `K_main` create an absent destination only when no close is pending or given up for the intent; otherwise the intent goes to the operator. A depositor who keeps `A` open can therefore make `K_main` front at most one destination per intent, and they lose `A`'s rent to do it. Regression: `fixed_b1_04_a_dust_transfer_to_the_intents_token_account_no_longer_fails_the_burn_or_its_refund` (with `fixed_b1_02_…` updated for the close's own fee).

**Where:** `src/desk/build.rs:41-50` `burn_tx` (`close_account` last), `:73-77` `refund_token_tx` (`close_account` last); `run.rs:475-508` reads `units` at finalized then signs; `run.rs:127-151` three `Failed` → `must_refund`, five → `attention`. The intent's token account is public (`api.rs:322-324`).

**Description.** `CloseAccount` requires an empty account. The transaction burns (or transfers) the amount read before signing; a transfer of one base unit of wX by anyone, landing between that read and ours, leaves the account non-empty and the whole transaction lands failed (fee paid by `K_main`, nonce advanced, books undone). The intent is retried with the new amount, which the stranger dusts again. Three failures put the deposit on the refund path, whose transaction closes the account too; five refund failures leave it to the operator. The stranger's cost is a few base units and Solana fees; the canister's is three to eight fees and as many signatures, the user's is their exit.

**PoC.** `poc_b1_04_…`: a 5 SOL holder's burn is dusted by one base unit (from a second holder) after each signature: three `Failed` landings (`attempts` 1..3, the supply unchanged, `K_main` down 3 × 5,000), then the refund is dusted and fails too (`refund_attempts` 1).

**Fix.** Do not close in the same transaction: burn / transfer the read amount and leave the account; close it in a later, separate step only when a fresh read shows it empty (or never: the rent is the depositor's). Alternatively burn the full balance with `BurnChecked` of the account's current amount read on chain… which Solana cannot do atomically; the separate close is the fix.

---

### B1-05: Free openings close the desk; expired intents stay watched (Medium)

**Fixed (2026-10-11, the same day, after the report):** intents that never saw a deposit (`Intent::unfunded`) have their own cap, `MAX_UNFUNDED_OPEN` (200), and no longer count toward `MAX_OPEN` (300), which now counts only intents with a deposit. Free openings therefore never refuse funded work; they can still fill their own cap, but they cannot take one of the 300 slots. An hour after expiry (`UNFUNDED_GRACE_SECS`), a never-funded intent leaves the tick's batch (`Intent::rotated`) for a rotation: one `getMultipleAccounts` a tick covers up to 100 of them in turn (`DeskState::rotation`) for the rest of their 7-day grace. Free openings add at most one read a tick, and a late deposit is still found and returned. Funded intents keep the 7-day grace in the batch. The 24 h TTL is unchanged: a 1 h TTL for never-funded intents would refund a slow exchange withdrawal instead of executing it, and it would not stop a refilling attacker. Regression: `fixed_b1_05_free_openings_have_their_own_cap_and_expired_ones_are_read_in_rotation`.

**Where:** `src/desk/api.rs:114-141` `open`: the counts; `:78` `ANON_PER_HOUR = 120`; `src/canister.rs:292` `may_ingress` accepts `open_intent` from anyone; `src/desk/intent.rs:191-193` `watched`: until `expires_at + REFUND_GRACE_SECS` (7 days); `run.rs:179-204` `detect`: one `getMultipleAccounts` per 100 watched, both providers, every tick.

**Description.** An opening costs the caller nothing: no deposit, no stake, and the ingress check accepts anonymous callers. Self-authenticating principals are free to make (60 of them × 5 an hour), recipients are any 32 bytes (3 each). 300 openings in an hour fill `MAX_OPEN`; every other user is refused "300 intents are open" for the 24 h until they expire, and the attacker refills. Expired, never-funded intents are watched for seven more days, so the batched reads grow by two outcalls per 100 per tick with no bound from the cap (the design's "one read per tick for up to 100 open intents" assumed the cap bounded the set). A funded attacker widens it further: a 0.05 SOL mint per intent leaves `Done` intents that stop counting toward the cap but are watched for 8 days.

**PoC.** `poc_b1_05_…`: 300 free openings refuse a real user; 24 h later all 300 are `Expired`, 300 more are opened, the user is refused again, 600 addresses are watched and one tick issues 12 `getMultipleAccounts` (6 chunks × 2 providers).

**Fix.** Tie an opening to something scarce: a tiny deposit at `K_int[n]` before it counts (the quote and the address can be given for free, the slot taken only when funded), or a much smaller free cap (per principal and in all) with the 300 reserved for funded intents. Bound the watched set: a never-funded intent watched for a short grace (1 h) after expiry, not 7 days; or rotate the expired set through one chunk per tick. Refuse anonymous callers at the ingress, or count them per source in `inspect_message`.

---

### B1-06: F1 is incomplete at the outcome side (Medium)

**Fixed (2026-10-11, the same day, after the report):** the three outcome-side writers (`ethflow::home_books::landed`, `frogsol::books::landed`, the cranks' landed prize claim in `settle`) book through `desk::state::book_outcome`, i.e. `Ledger::write`: applied at once when the collection has no open link, else owed in `Ledger::owed` (an `OwedWrite` per write, `ReturnAdvance` / `FundReserve` / `ClaimRaffle`, saved with the desk state across an upgrade) and applied by `Ledger::apply_owed` the moment the collection's last open link is decided (`landed`, and `undo` after the snapshot is restored), before any new link's snapshot (`begin`), and by the desk's pass as a backstop; an owed write the books refuse then is an incident. Deferral was chosen over an inverse-delta undo: three writers instead of every booking closure, and the snapshot undo stays as it is. Regression: `fixed_b1_06_a_book_write_at_a_line_outcome_under_an_open_desk_link_survives_the_links_undo`.

**Where:**
- `src/ethflow/home_books.rs:229-250` `landed`: `return_advance` (an advance that never landed), `fund_reserve` / `return_advance` (a way back that landed): `desk::with_mut` with no `collection_busy` check. Its `send` at `:146` has the check.
- `src/frogsol/books.rs:183-205` `landed`: the same three writes; its `send` at `:119` has the check.
- `src/cranks.rs:842-848` `settle`: `claim_raffle` at a landed prize claim (writes `c.raffle`, which a `Scope::Collection` snapshot holds); `receive` writes the treasury (not snapshotted: safe).
- `src/chainlink/ledger.rs:254-273` `restore`: a whole-module or whole-collection snapshot; `:287` `Busy` only stops **links**.
- `src/canister.rs:1828` `looper::step` (the Ethereum / frog passes, which process the money line's outcomes) runs before `:1871` the lines' pass and `:1880` the desk's pass.

**Description.** A desk link signed in tick N is open through tick N+1's `looper::step`. The NFT flows' and the cranks' outcome handlers write the collection's books there, without the F1 wait. When the desk's transaction then fails on chain or never lands, `Ledger::undo` restores the snapshot taken at `begin`: the outcome-side write is gone. For a landed way back: `M[i]` holds the lamports, the books do not (the reconciler reports `Unbooked`; the operator's sweep would re-book them as surplus instead of as the advance's return, and `transit_units` stays `ONE` → `AdvanceOpen`: the collection's NFT pipeline is stopped for good). For an advance that never landed: the books keep it in transit and its backing released, against a chain that never moved.

**PoC.** `poc_b1_06_…`: a mint link is open (`collection_busy`); `fund_reserve` is called as `home_books::landed` would; the mint lands failed; the link is `Failed` and the surplus is back to 0.

**Fix.** Either an inverse-delta undo (the link records the operation's delta and reverses only it; the ledger's module doc already names this as the refinement), or route every outcome-side write through the same wait: defer the `landed` books write (keep it in the flow's state as "owed to the books") while `collection_busy`, applying it at the next pass. The cranks' landed claim can use the F4 wait it already has.

---

### B1-07: No partial fill over the hourly bucket (Low)

**Fixed (2026-10-11, the same day, after the report):** `desk::run::fit_bucket`: a deposit whose wX would overflow the hourly bucket mints what fits now (its `min_out` scaled alike), the rest staying at `A` as the next deposit; a fit under the least mint or under a tenth of the deposit is not taken (the whole waits for the bucket, as before), so a refilling bucket is not chased in slivers. Regression: `fixed_b1_07_…` (`tests/audit_b1_lows.rs`).

`src/desk/run.rs:441-444`: `b.mint(coll, to_module, min_units, …)` is all-or-nothing; `floor.rs:463-464` refuses `OverCap` when `level + units > mint_per_hour`, `:461-462` `OverBacked` over the public cap or the room. §6.1 promises "a mint over the hourly bucket mints what fits and keeps the rest". A deposit above the bucket (5 wX, or `⌈250 SOL / floor⌉` units) can never pass, waits 6 h (`transient`) and goes back less the fee; an exchange withdrawal of that size is the likely case. Fix: mint `min(deposit, room × unit_price)` and leave the rest at `A` as the next deposit (the intent machine already treats a leftover as the next operation).

### B1-08: A planted system account at the recipient's ATA blocks the mint (Low)

**Fixed (2026-10-11, the same day, after the report):** the recipient's token account is created unless the address holds a token account of the mint (a planted system account is allocated over, as the ATA program does). Regression: `fixed_b1_08_…` (`tests/audit_b1_lows.rs`; the fake chain now allocates over a pre-funded address).

`run.rs:419-435`: `create = accs[1].is_none()`. Anyone can transfer ≥ 890,880 lamports to the ATA address of (recipient, wX mint), creating a system account there. Then `create` is false, `MintTo` to a non-token account fails simulation (`SIMULATION_REFUSED` → `attempts`), three times → refund less the fee. The recipient cannot be minted to by the desk until they create the account themselves (the ATA program handles a pre-funded address). Fix: `create` unless `accs[1]` decodes as a token account of the mint.

### B1-09: The per-recipient limit is a lock-out lever, and misses the burn's recipient (Low)

**Fixed (2026-10-11, the same day, after the report):** the per-recipient bound counts the caller's own open intents for that recipient, so a stranger's free openings never lock a wallet out (`the_limits_hold_per_recipient_per_caller_and_in_all`). The burn's SOL recipient stays unbounded by it: the per-caller bound binds.

`api.rs:132-136`: `same` counts intents whose `refund_to()` equals the new one's. For a mint that is the recipient: three free openings naming any wallet refuse that wallet's own mint intents for 24 h ("3 intents are already open for this recipient"), repeatable. For a burn `refund_to()` is `refund_to`, which the caller chooses freely, so the limit never binds the SOL `recipient`. Fix: count by caller **and** recipient; or require a funded intent to take a recipient's slot (as B1-05).

### B1-10: `sweep_unbooked` sees only the desk's links (Low)

**Fixed (2026-10-11, the same day, after the report):** the sweep is refused while the money line is busy, holds an outcome not yet taken by its flow, or the collection has deferred book writes (B1-06's queue). Regression: `fixed_b1_10_…` (`tests/audit_b1_lows.rs`).

`driver.rs:286-308`: refused while `open_of(coll)`; the Ethereum and frog money-line steps (an advance's way back, a late credit) are not links. Between their landing on chain (finalized) and their flow's `landed` (the next `looper::step`), `M[i]` holds lamports the books do not yet count. A sweep in that window (`hold_tick` keeps the tick off, not the chain) books them as surplus; the flow then books them again (`return_advance` / `fund_reserve`): the books exceed the chain → `BooksUnbacked` → two strikes → halt. Operator-only, a narrow window, self-detecting. Fix: refuse the sweep while the money line holds a pending step of the collection, and while any flow has an unprocessed outcome.

### B1-11: A `Stuck` link never resolves (Low)

**Fixed (2026-10-11, the same day, after the report):** `desk_resolve_link(id, landed)` (`driver::resolve_stuck`): the chain is asked by the signature first and a finalized verdict decides; unknown to the chain, only `landed = false` is taken (undone, the books restored); a transaction answered without `meta` is unread, here and in the crash-recovery pass. Regression: `fixed_b1_11_…` (`tests/audit_b1_lows.rs`).

`ledger.rs:362-371`, `driver.rs:219-222`: a `Signed` link absent from every line and unknown to the chain after `ORPHAN_SECS` is `Stuck`: open for ever (`Phase::open`), so `Busy` for its collection (every later operation waits 6 h and refunds) and in `totals` (`eventual` is off by its effects: every reconciliation is a hard finding → halt). No operator method marks it landed or undone; `clear_halt` does not touch it. The path to it is narrow today (the lines' outcomes are drained each tick), but once there the collection is lost until an upgrade. Also `driver.rs:208`: `tx["meta"]["err"].is_null()` reads a transaction answered without `meta` as succeeded. Fix: an operator `desk_resolve_link(id, landed | undone)` that re-reads the signature's status first; refuse a missing `meta`.

### B1-12: The memo scan skips what it cannot read (Low)

**Fixed (2026-10-11, the same day, after the report):** the scan pages back with `before` (`sol_signatures_before`, `solrpc::signatures_between`) until it meets the cursor, at most `SCAN_PAGES` (5) pages of `SCAN_LIMIT`; past the budget it files an incident naming the gap and still moves the cursor (the reconciliation is the check). Unexplained memos were already the monitor's SEV1. Regression: `fixed_b1_12_…` (`tests/audit_b1_lows.rs`).

`driver.rs:235-255`: `sol_signatures(cursor, SCAN_LIMIT)` returns the newest 100 after the cursor; the cursor then jumps to the newest. More than 100 signatures of `K_main` between two scans (ten minutes; four lines; or anyone's transactions that list `K_main` as a read-only account, which `getSignaturesForAddress` returns) leave a gap that is never read. Unexplained memos are only listed in the report (no strike, no page). The §10.2 reconciler is the real check; this one should page through (`before`) until it meets the cursor, bounded per pass, and page on an unexplained **mint**.

### Informational

- **B1-I1.** `run.rs:302-305`: the pause check precedes the 6 h check; deposits made during a pause wait, then are refunded (not executed) if the pause lasted over 6 h. State it, or count the window from the unpause.
- **B1-I2.** `run.rs:492-495`: `paid > FEE_LAMPORTS` lets `net` be under 890,880 lamports; to a recipient with no account the transfer fails simulation three times, then refunds. Check `net ≥ rent(0)` when the recipient account is absent.
- **B1-I3.** `api.rs:86-110`: no recipient or `refund_to` is checked against `K_main`, `M[i]`, `K_int[n]`, program ids or executable accounts. The user's loss alone.
- **B1-I4.** `driver.rs:192-195`: a link recovered as signed on a line has no intent waiting for it (`op = None`); when it lands the intent is never told (`minted_units`, `done_ops`, `history` stay), and the next read finds the deposit gone (`Open`). Attribute the recovered link to its `intent` field.
- **B1-I5.** `recon.rs:75-78`, `:108-110`: an `Unbooked` incident per reconciliation per collection with ≥ 0.01 SOL above the books; a stranger makes one every 10 min for 0.01 SOL until the operator sweeps. **Fixed the same day:** a soft finding already in the incidents with the same text is not filed again (`a_repeated_soft_finding_is_one_incident`).
- **B1-I6.** `recon.rs:79-85`: dead condition; the invariant finding is always pushed. **Fixed the same day:** the condition is gone, the comment says what the branch does.

## 3. Review F1 and F2 in the desk's paths (verified)

**F1 (non-link writers wait while a link is open).** Every `desk::with_mut` outside the desk was read:
- `ethflow/home_books::send` (`:146`), `frogsol/books::send` (`:119`): wait, except `Call::Deposit`, which is not a book step (it returns `Err` before any write).
- `cranks`: `distribute` (`:434-437`, all collections, retried in 5 min), `resolve` (`:506-508`, per collection, decided in the same synchronous section as the read; the `drand_round` await between the decision and the write runs inside the serialised tick, so no link can begin meanwhile), the raffle pass (`:565`), the surplus link (`:713`), the pause-lift / treasury parameters (`:359-390`: pause flags and the treasury, neither in a snapshot).
- The pool desk and the coins write the books through links (`pooldesk/run.rs:230`, `coins/redeem.rs:137`, `coins/fees.rs:144`) or write the treasury only (`pooldesk/run.rs:572`, `coins/fees.rs:239`), or add a collection (`coins/open.rs:199`).
- Inside the desk: `sweep_unbooked` refuses while open; `halt` writes `paused` (kept by `restore`); the poster's fields are kept by `FloorFields`; `desk_pause_collection` writes `paused`.

**Found incomplete** at the outcome side: B1-06 (`home_books::landed`, `frogsol::books::landed`, the cranks' landed `claim_raffle`).

**F2 (the distributed surplus is a chain link).** `books::distribute_owed` / `distribute_all_owed` (`books/mod.rs:488-532`) book the raffle part and the treasury only; the module's surplus is `fund_reserve`d inside `link.begin` of the `TreasuryStep::Surplus` link (`cranks.rs:712-726`), with `Effects { lamports_in }`, so `pre_sign` (`gate.rs:49-61`) requires `K_tre → M[i]` to leave the eventual balance covering the books, and the reconciliation's `eventual` (`gate.rs:28-33`) counts it while open. Landed → `link.landed`; failed or never landed → `link.undo` (`cranks.rs:833-876`): the surplus is removed from the books again. The desk's own gate and reconcile read the same ledger, so a surplus in flight never reads as missing lamports. **Holds.**

## 4. Checked and found correct

- **Book first, then sign, then confirm.** `begin_link` (`run.rs:336-343`) books and records in one synchronous section before any await; `sign_recorded` reads the chain, gates, takes the turn, signs, and sets `Signed` in the synchronous section after the last await; `send` sets `Executing` in the same section. Anything short of a signature undoes the link.
- **Idempotence.** The desk's keys are `i<id>:<kind>:<seq>` with `seq` bumped before `begin`; the intent's `op` is the one link it waits for; `begin` with a known key returns it unchanged; `prune` keeps the keys of open links; `sweep:<coll>:<now>` repeats harmlessly.
- **One books-changing link per collection** (`Busy`), refund links outside it (`coll = None`, `Scope::None`), the gate skipped for them.
- **The gate's arithmetic.** Pending burns are taken off the finalized supply before the supply guard; the other undecided payouts before the payout guard; `eventual` in `i128`; `supply_guard` is `≤`; `payout_guard` keeps the books covered after this and every pending payout; `WrongCollection` on a mismatched view.
- **Reads at finalized, two providers agreeing** (`solrpc::accounts`, `agreed`): a lag or a lie is a `Disagree` → the desk waits ("the deposits are not readable"); `read_view` refuses a non-system `M[i]` and a missing mint; a failing read undoes the recorded link.
- **Detection.** One batched read per 100 (`MAX_KEYS`); `observe` ignores `Executing`; a SOL deposit counts only on a system account with no data, a token deposit only on a token account of the intent's address and the right mint; dust below the minimum never signs; the amount is re-read before the operation and before the refund.
- **Fee and rent arithmetic.** `to_module = deposit − fee − rent` must be positive, else a refund of `deposit − fee` (≥ 1,000,000 lamports ≥ the rent-exempt minimum of a new recipient account); a deposit that exactly covers rent + fee is refunded (the rent back); `A` ends at exactly 0 in a mint; the burn pays `paid − fee` + the fee = `paid` = `Effects.lamports_out`; `DUST_REFUND` ≥ fee + rent; `MIN_BURN` > `DUST_REFUND_UNITS`.
- **Overflow.** `checked_sub` on the deposit; `saturating_*` on the effects and the counters; `u128` in the quotes; `i128` in `eventual` and `payout_guard`; `expires_at.max(now + TTL)`.
- **Traps on chain data.** `solrpc::accounts` refuses an answer of the wrong length, so `accs[0]`, `accs[1]`, `v[0]`, `a[0]`, `a[1]` are safe; the decoders return errors; `Value` indexing never panics; the only `expect` is on a filtered list.
- **Interleaving.** The ticks are serialised (`begin_tick` / `TICKING`); `desk_reconcile` and `desk_sweep_unbooked` hold the tick; the other operator methods and `open_intent` write only non-ticked fields (`paused`, `desk_paused`, `coin_redeem`, `halted`, a new collection, a new intent) in synchronous sections; `open` reserves the number, the rate-limit entry and the counts before its one await; the state is never held across an await; a trap after an await rolls back to a state the next `settle` repairs (a `Recorded` link not on a line is undone; one on a line is adopted).
- **Expiry and cleanup.** Only a never-funded `Open` intent expires; a deposit after expiry is returned (never minted) for 7 days, then left alone; records drop 30 days after the grace; the rate-limit log is pruned hourly; a served redemption returns every later deposit.
- **Refusals.** Transient ones wait and are retried every tick for 6 h; the user's own bound, a paused or halted collection, a bad destination (checked at opening), an unknown collection, a deposit under its fees go back at once; three failed landings or three simulation refusals go back; five failed refunds page.
- **Halting the right collection.** The gate halts the link's collection; the reconciliation strikes per checked collection; a read failure is an incident, not a strike; two strikes in a row.
- **Crash recovery** (`settle` step 2) looks at every line (F9), adopts a `Recorded` link the line carries, undoes one it does not, asks the chain by signature for a `Signed` link no line carries, and marks `Stuck` only after `ORPHAN_SECS`; `sol_clear_line_incident` re-reads every signature before abandoning (NEW-A4-01), and an abandoned step is undone through the line's `ok = None` outcome.
- **Memos** are labels: `by_memo` requires the link's exact text; the scan only reports.
- **Custody of deposits.** Only `K_int[n]` moves `A`'s lamports and its token accounts; a third party cannot withdraw, only add (B1-04 is what adding can do); the nonce line, the Relay key (A5-03) and the multi-signer rule (`signer_paths`) are B2's.
- **The operator's switches.** `desk_set_paused` stops openings and executions, not refunds; `desk_pause_collection` stops mints only; `desk_clear_halt` keeps the mint paused; `desk_add_collection` validates the mint key and the parameters; `desk_sweep_unbooked` needs `UNBOOKED_MIN` and no open link.
- **The views.** `intent_status`, `desk_status`, `ledger_log` (bounded 200), `books_view` read only; nested `with` are shared borrows.

## 5. Test results

- `cargo test --locked --test audit_b1_desk_links`: **6 passed** (the six PoCs, each exhibiting its finding).
- `cargo test --locked` (the whole crate, the new file included): **1,207 passed, 0 failed** (55 suites).
- No file under `src/` was modified.
