# Round 3, part E: arrival accounting, the deposits hold, concurrency and upgrade safety

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `4949da0`; the
round-2 fix delta `79455e1..4949da0` (canister: `ethflow.rs`, `ethnet.rs`, `ethnft.rs`, `canister.rs`, `Cargo.toml`, the `.did`, the
tests and `tests/fixtures/eth-state-round2.hex`). `27c0962` and `79455e1` have the same `autobuyer/canister-rs` tree. Toolchain 1.98.1.
Scope: C2-M-1 (`unsettled`, `arrive`), C2-M-2 (`buy_holds_mint`), C2-L-1, C2-L-2 (`stranded_by`), C2-L-3 (`lost_opts`), the new state's
upgrade safety, and concurrency in the delta. Part C of this round covers the advance gate and the reverse Relay leg in depth; overlaps
are noted, not repeated.

**Nothing in the worktree was modified except this report and `review3/poc/E/`; nothing committed, pushed, deployed or signed.**
Every PoC ran in a private copy (`autobuyer/canister-rs` rsynced to a scratch dir). PoCs (`review3/poc/E/`):

- `audit3_nft_e.rs` (5 tests, all **pass** = the weakness is shown) and `audit3_nft_e.output.txt`. Harness: `tests/audit_nft_r2.rs`'s fake
  world copied whole, plus two switches: `pin_back` (the Ethereum pin one bucket lower: the balance there is the one before a spend of
  ours that mined above it, receipts above it are unseen, the nonce is one lower) and `fee_of` (a mined transaction's gas).
- `audit3_nft_e_upgrade.rs` (3 tests, pass) and its output: the probe on real saved shapes, its gaps, its cost.
- `saved-79455e1-rich.hex`, written by the **79455e1 build itself** (`dump_state_79455e1.rs`, run on a `git archive 79455e1` export):
  every nested `opt` of the Ethereum side present, every buy, delivery and deposit phase of that build.
- `didcheck/` (a scratch crate: `candid_parser` 0.1.4 + the canister crate) and `didcheck.output.txt`, with a negative control.
- `r2_audit2_nft_c_rerun.output.txt`: round 2's `poc/C/audit2_nft_c.rs` with the fixer's adaptations applied unchanged;
  `r2_audit2_nft_c_m2_adaptation.diff` + `r2_audit2_nft_c_m2.output.txt`: one more line so the C2-M-2 PoC reaches its assertions.

Severity scale: the README's.

## Verdict

Counts (this part: new findings, plus round-2 items not actually fixed):

| | Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- | --- |
| New (E3-*) | 0 | 0 | 1 | 3 | 5 |
| Round-2 items in scope not closed | 0 | 0 | 0 | 0 | 0 |

C2-M-1 is **partly closed**: the path the reviewer found (a receipt unread at the pin) is closed, but the watch is still relative and is
still read at a pin that may be below a spend already counted; the same phantom arrival comes back through a pin that moves back one
bucket (**E3-M-1**, Medium, the round-2 finding's impact class, counted once as new). The `arrive` cap does not bound that phantom (the
balance at the lower pin holds the spent ETH), and its hard wait turns C2-I-1's drift into a deadlock (E3-L-2). C2-M-2, C2-L-1, C2-L-2 and
C2-L-3 are closed for the reported paths, with a Low residual each for C2-M-2 (E3-L-1) and C2-L-1 (E3-L-3). The upgrade probe holds: no
false refusal on the audited `e04f49b` state or on a rich state written by `79455e1`, the new fields and cases are all `opt` / appended,
and its cost is small at the sizes the code allows.

## 1. Round-2 items in scope

Round 2's C PoC re-run (`r2_audit2_nft_c_rerun.output.txt`): **4 of 5 fail, 1 passes by design**; checked one by one:

| ID | Sev (R2) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **C2-M-1** a mined fill unread at the pin read as another collection's arrival | Med | **Partly closed** (new residual **E3-M-1**) | `unsettled` (`ethflow.rs:645-651`) is tick-local (reset every tick), one flag for every collection; set on a failed receipt or nonce read and whenever the line is kept (`resolve_eth` `:1737`, `:1748-1751`, `:1755`, `:1759-1762`: `mined_eth` put it back, `superseded` could not tell); the Funded step waits on it (`home_act` `:2257`). A line signed this tick is after the pin (the pin is cached per tick, `ethnet.rs:113-121`), a line unmined at the pin is in neither the balance nor `eth_out`, a receipt above the pin is `null` (`ethnet.rs:57-73`), a provider that answers `null` against one that answers a receipt is a disagreement (`nftnet.rs:63-69`) = `unsettled`; both `null` with the nonce used is `superseded` → halt, `unsettled` again. The way back's `BackDeposit` is a line transaction like any other (covered), and its own watch is taken in `eth_act` with the line empty (`:1673`, `:1689`). **Not closed:** nothing ties the balance reads to a pin at or above the last counted spend: `eth_act` refuses to SIGN below `last_mined`'s block (`:1532-1538`, B-M-4), but the watch start (`:2272`) and every arrival read (Bridging `:1093`, Stuck `:1124`, BackBridging refund `:1161`) run at whatever pin the tick has. `arrive`'s cap (`:1196-1207`) does not help there: the balance at the lower pin still holds the spent ETH. The fixer's `r2_c2_m1_an_arrival_is_never_earmarked_over_the_balance` passes only because Milady's buy is still `Arrived` (its earmark fills the room); with that buy past its fill (earmark 0) the same injected delta arrives. R2 PoC: fails at its step "Azuki is Depositing" (the quote now waits): that IS the fixed step, acceptable. |
| **C2-M-2** an Arrived buy holds its collection's deposits indefinitely | Med | **Closed** for the reported path (residual **E3-L-1**) | `buy_holds_mint` (`:508-515`): `Arrived` for `ARRIVED_HOLD_SECS` (6 h), nothing for `Stuck`/`Back*`/`Returning`; 48 h → `back_now`; `eth_give_back` on `Arrived`. The R2 PoC does **not** fail on its attack assertion as `fixes-round2.md` §11 says: it panics on a harness `unwrap()` of a hand-in refused once the buy went back at 48 h. With that one line tolerated (`r2_audit2_nft_c_m2_adaptation.diff`) it reaches its assertions: after 3 days the deposit is **Minted** and the buy `BackFunded`: closed. Residuals: every re-entry into `Arrived` (a reverted fill `:1954`, a mined cancel of the fill `:1993`, a way back refunded on Ethereum `:1164` via `arrive` `:1204`) restarts `since`, i.e. both the 6 h hold and the 48 h clock (E3-L-1); `Funded` (waiting for an acceptable quote) and `Bridging` still hold with no time bound (E3-I-5), and E3-L-2 leaves a buy `Bridging` for ever. Deposits during `Back*` mint (by design; the buy's own mint, if it ever comes, waits for room). |
| **C2-L-1** a Stuck after arrival keeps its watch | Low | **Closed** (residual **E3-L-3**) | `arrive` clears `last` (`:1205`); the L-2 sweep's `Stuck`-without-watch branch is live (`:1022-1033`) and releases `eth:{id}`. R2 PoC fails on its attack assertion ("the watch survives the arrival"). New cost: the post-arrival `Stuck` now has no exit at all (`give_back_now` `:2501` wants a watch): it never finishes, its collection never buys again, its module stays in transit, and no upgrade of this build may drop or re-point the collection (E3-L-3). |
| **C2-L-2** an upgrade dropping a collection with a buy on the Relay leg | Low | **Closed** | `stranded_by` (`:2515-2537`) refuses in `post_upgrade` (`canister.rs:490-496`), fed with the argument's `eth_nft` (which is what `restore_init` keeps, `canister.rs:242`, so no false refusal of an unchanged config); if a collection is gone anyway, `NeedAdvance` is dropped (`:2203-2206`) and `Funded` returns to its own module (`:2247-2254`). R2 PoC fails on its attack assertion (Azuki's buy starts). It cannot refuse every upgrade: an upgrade that keeps the configuration always passes; within this build it is not bypassable (a configuration that does not parse gives no collections, so everything open is "stranded": conservative). It can pin a collection's entry for ever when a record never finishes (a `Stuck` buy without watch, a `Held` stray whose return always reverts): part of E3-L-3. |
| **C2-L-3** the probe misses an `opt` nested in the state | Low | **Closed** | `lost_opts` (`canister.rs:415-446`) walks the saved bytes and the re-encoded restored state as untyped values: opt inside vec, inside records inside variants (same case id), at any depth; a field or case the new type lacks is skipped. Checked for **false refusals**: none on the audited `e04f49b` fixture, none on a rich state **written by the 79455e1 build** (16 buys in every phase with `ask` and `last`, deliveries and deposits with `refused_since`, a `CancelFill` line with `first_signed_at`, `pending`, `last_mined`, `halted`, `sale_cache`, readings with `module_posted_at`), each padded to a 64 KiB page as `stable_bytes()` returns it (trailing zeros are ignored: `get_value` reads one value, as `stable_restore` does). The migration in `post_upgrade` runs after the probe, so an `opt` it clears is not compared. No maps in the state (only `Vec`s), so the index-wise walk cannot misalign. Cost on the largest state the code allows for the Ethereum side (sale cache 2,048 × 5 with sales, scan queues 512 × 5, 3,000 buys, 6,000 orders and outbounds, a 400 kB log: 3.3 MB of stable memory): one typed decode 155 ms, the probe 395 ms native (release) — a few billion instructions against the 300 B upgrade limit; no bricking at the sizes the code allows. The fixer's `r2_c2_l3_…` does call `lost_opts` (not vacuous); the R2 C-L-4-style PoC passes by design (it asserts Candid's decode, `line = None`, and never calls the probe: verified). Gaps: E3-I-1, E3-I-3. |
| New state's upgrade safety | — | **Holds** | Every new field is `Option` + `#[serde(default)]`: `FloorPost.held` (`ethnft.rs:641-642`), `EthState.sale_scan` (`:436-437`; `sale_cache` was already); `SaleScan` lives only inside that `opt vec`; new cases appended (`BPhase::BackFunded/BackSending/BackBridging` `:113-117`, `EthKind::BackDeposit` `:314`); Candid variants are matched by hash, so order is moot anyway. No config type changed in the delta. `79455e1` → HEAD decodes with every record kept and the new fields absent (test). `tests/fixtures/eth-state-round2.hex` is the canister's own encoder's output, held by `r2_the_round_2_state_fixture_is_what_the_canister_encodes` (compares bytes). The `.did` matches the Rust types for every case of `BPhase`, `DPhase`, `XPhase`, `EthKind` and `Step` and the new records (`didcheck`, with the opt-loss comparison; a negative control without `BackDeposit` is caught). |
| C2-I-1 no gas reserve in `available`; deliveries use the whole balance | Info | **Unchanged, now worse** | Deliveries still check only the total balance (`ethnet.rs:294-307`); with the round-2 hard wait in `arrive`, gas drawn from an earmark is no longer self-correcting: E3-L-2. |
| L-2 stale reservations (the sweep's Stuck branch) | carried | **Closed** | Live since C2-L-1 (fixer's test checks the release). |

`fixes-round2-poc/canister-pocs-rerun.output.txt` is honest except for the C2-M-2 line (a harness panic, not the attack assertion; the
fix holds anyway, above) and the C2-M-1 line (fails at the fixed step, not at the final assertion; acceptable).

## 2. New findings

### E3-M-1 (Medium): the arrival watch is read at a pin with no floor: a pin one bucket back turns a counted spend into another buy's arrival, or hides a real arrival for ever

- **Where:** `ethflow.rs:2272` (Funded: `eth_before = eth_balance()`, `eth_out_before = st.eth_out`), `:1093-1096` (Bridging),
  `:1124-1133` (Stuck), `:1161-1164` (BackBridging's Ethereum refund); `:1196-1207` (`arrive`); the guard that exists only for signing:
  `:1532-1538` (`eth_act`, B-M-4); `pin.rs:54-61` (`choose_pin`: lowest rounded head less one bucket, 8 blocks on Ethereum).
- **Mechanism:** the watch is relative: `got = bal + (eth_out − eth_out_before) − eth_before` assumes the balance is read at a pin at or
  above every spend `eth_out` already counts. `eth_out` is raised when a receipt is read at pin P1; the pin is recomputed every tick and
  moves back a bucket whenever one provider's head falls back across a bucket boundary (load-balanced endpoints do; B-M-4 was fixed for
  this very case, for signing only). A fill counted at P1 is typically in the last bucket under P1 (the first pin that covers it), so the
  next tick's lower pin P0 is under it:
  - (a) a Bridging (or Stuck, or BackBridging) buy reads `bal(P0)` = the balance BEFORE the fill, and `eth_out` WITH it: `got` is the
    fill's spend. If that is at least the order's minimum out (any fill of a pricier collection, or equal-sized buys), the buy is
    `Arrived` on ETH already spent. `arrive`'s cap does not stop it: at P0 the balance still holds that ETH, and the fill's own buy is
    past `Buying` (earmark 0).
  - (b) a quote judged at P0 (the line is empty, so `unsettled` is false) takes `eth_before` = the balance BEFORE the fill and
    `eth_out_before` WITH it: the real arrival later reads short by the fill's spend and is never seen; past the deadline the buy is
    `Stuck` with its watch, on the one Relay leg, for ever.
- **Impact:** (a) is C2-M-1's impact exactly: one collection's spent ETH backs another's buy; the earmarks exceed the balance (PoC:
  0.495 ETH earmarked, 0.013 ETH held), starving every other arrived buy and every stray return; if Relay then refunds the real order on
  4663 nothing notices it (the refund stays at our 4663 address); 48 h on the way back needs ETH that is not there and waits for ever,
  and while a buy is `BackFunded` no buy starts for any collection (`:1036`); a wrong `cost_of` if the real payout lands and fills. The
  same misread in BackBridging reads as "refunded on Ethereum" and loses the real 4663 payout from view. (b) holds the Relay leg for
  every collection with the whole arrival at our Ethereum address earmarked to nobody; the operator's exit (`eth_give_back` on a
  watched `Stuck`) returns the home balance (4663 gas money, 0.0022 ETH in the PoC) to the module, not the ETH that arrived. Needs no
  attacker: one pin regression in the ticks around a counted fill while another collection's buy is on the Relay leg.
- **PoC:** `audit3_nft_e.rs` `e3_m1_a_pin_that_moves_back_reads_a_counted_fill_as_another_collections_arrival` (**passes**: Azuki
  `Arrived { wei: 495464962449863321 }` = Milady's fill; the same tick `eth_act` refuses to sign at that pin; then earmarks > balance, the
  4663 refund unseen, the way back stuck) and `e3_m1_a_watch_taken_at_a_pin_that_moved_back_never_sees_the_real_arrival_and_holds_the_relay_leg`
  (**passes**: Stuck, watched, `relay_busy`, 0.498 ETH at our address earmarked to nobody).
- **Recommended fix:** give the balance reads the floor the signer has: skip the watch start and every arrival/refund read (and
  `arrive`) in a tick whose pin is below `last_mined`'s block, or better record in `EthState` the pin at which `eth_out` was last raised
  and in `Watch` the pin it was taken at, and read deltas only at a pin ≥ both. The robust fix is still the round-2 reviewer's: detect
  the arrival from Relay's own fill (the order id in the payout transaction), not from a balance delta.

### E3-L-1 (Low): a fill that reverts restarts the Arrived hold and the 48 h clock: a seller can hold a collection's deposits, and defer the way back, indefinitely

- **Where:** `ethflow.rs:1954` (a reverted fill → `Arrived { since: now }`), `:1993` (a mined cancel of the fill, same), `:1204`
  (`arrive`, e.g. the way back refunded on Ethereum); `:512` (the hold measured from `since`), `:1140` (the way back from `since`).
- **Mechanism / impact:** the bound of C2-M-2 is measured from the latest re-entry into `Arrived`, not from the arrival. The runner hands
  in the cheapest listing under the cap (`ops/src/ethnft.ts:287-316`; it skips only an order hash it already tried, and a new listing is
  a free signature). A holder of one NFT of the collection lists it at the top of the book; when the canister's fill is in the mempool
  they cancel the order (or move the token) ahead of it: the fill reverts, the buy is `Arrived` again with `since` now. Once every ≤ 6 h
  keeps every deposit of the collection unminted and the buy from ever going back; each cycle costs the attacker a cancel and the buy a
  reverted fill's gas (taken from what arrived). No loss of custody; holders' deposits frozen and the module's ETH parked on Ethereum for
  as long as the attacker pays. Also a pathological, attacker-free version: the way back refunded on Ethereum repeatedly (6 h in every
  48 h), and `MAX_ATTEMPTS` does not bound the way back.
- **PoC:** `e3_l1_a_fill_that_reverts_restarts_the_arrived_hold_so_a_seller_can_hold_deposits_and_defer_the_way_back_indefinitely`
  (**passes**: 5 days, 24 reverted fills, the deposit still `Mint`, the buy still `Arrived` and never went back).
- **Recommended fix:** keep the first arrival's time across re-entries (a `first_arrived` in the phase, or on `EthBuy` as an `opt`) and
  measure both `ARRIVED_HOLD_SECS` and `ARRIVED_BACK_SECS` from it; bound the fills a buy may try (or the reverts) before it goes back.

### E3-L-2 (Low): C2-I-1's drift + the round-2 arrival cap: an arrival waits on an earmark that waits on the Relay leg that the arrival holds (deadlock)

- **Where:** `ethflow.rs:1196-1202` (`arrive`: less room than `min(got, value)` → `Err`), `:1095-1096` (Bridging: `arrive(...)?` returns
  before the refund and deadline checks, so the buy never even becomes `Stuck`), `:1660-1661` (the way back waits while another buy holds
  the Relay leg), `:1036` (no new buy while one is `BackFunded`); `ethnet.rs:294-307` (a delivery checks only the total balance).
- **Mechanism:** deliveries (and a fill's gas beyond its reserve) are paid from the shared balance; once the gas pot (the ETH earmarked to
  nobody) is spent, they spend an `Arrived` buy's earmarked ETH (C2-I-1). Then another collection's real arrival finds
  `bal − Σ other earmarks < min(got, value)` and waits, for ever: `got` is right and the room is short by exactly the drift. If the
  earmarked buy cannot fill (A2-M-1's case: the floor rose) it goes `BackFunded` at 48 h and waits for the Relay leg, which the
  waiting arrival holds (`Bridging` is on the leg). Neither moves. Before round 2 the arrival happened (earmarks over the balance by the
  drift, self-correcting); now it is a deadlock. The waiting buy stays `Bridging` past its deadline, which also holds its collection's
  deposit mints (`buy_holds_mint`), and neither buy has an operator exit (`give_back_now` refuses `Bridging` and `BackFunded`).
- **Impact:** every collection's buys stop, both buys' ETH (≈ 0.99 ETH in the PoC) sits at our Ethereum address, one collection's
  deposits wait, until someone sends ETH to the canister's Ethereum address (the drift or more; control in the PoC: a 0.01 ETH top-up
  releases it). Needs the pot drained into an earmark (an operational condition: deliveries outpacing the per-buy 0.003 ETH leftovers,
  or a gas spike) and an arrived buy that cannot fill.
- **PoC:** `e3_l2_gas_drift_into_an_earmark_deadlocks_the_next_arrival_with_the_way_back` (**passes**: a 0.001 ETH pot, one delivery at
  0.004 ETH gas; after 10 days Milady `BackFunded` "the way back waits: another buy holds the Relay leg", Azuki `Bridging` past its
  deadline, 720 arrival refusals; then the top-up control).
- **Recommended fix:** make deliveries and returns respect earmarks (sign only while `bal − Σ earmarks ≥ gas`, and page on a low pot),
  so earmarks never exceed the balance by construction; in `arrive`, when `got` is consistent but the room is short, earmark what is
  there and flag rather than wait for ever; never let a refused arrival skip the deadline/refund checks; let a `BackFunded` buy take the
  leg from a buy that cannot arrive, or give both an operator exit.

### E3-L-3 (Low): a post-arrival `Stuck` buy is open for ever with no exit: its collection never buys again, its module stays in transit, and no upgrade may drop or re-point the collection

- **Where:** `ethflow.rs:1966` and `:1180` (the two post-arrival `Stuck`s), `:2501` (`give_back_now` wants a watch, cleared at arrival
  since C2-L-1), `:151-153` (`finished`: only `Settled`/`Returned`), `:1042` (`buy_pass` skips a collection with an open buy),
  `:2518-2521` (`stranded_by` counts it).
- **Mechanism / impact:** before round 2 such a buy kept its watch, which gave it the (flawed) `eth_give_back` exit; now it has none, and
  `Stuck` is never finished. The collection's module keeps its advance in transit (never settled or returned), no new buy for that
  collection ever starts, and `stranded_by` refuses every upgrade of this build that drops or re-points the collection (a `Held` stray
  whose return can never succeed pins its collection the same way). Reachability as C2-L-1's (a fill mined whose token is not ours
  after finality: an exotic collection or a reorg deeper than the pin), hence Low. The ETH of that item is already lost in that case;
  this is the missing way to close the books.
- **PoC:** `e3_l3_a_post_arrival_stuck_buy_is_open_for_ever_with_no_exit` (**passes**: 3 days with the module wanting, still one Milady
  buy, the module in transit, `eth_give_back` refused, an upgrade dropping Milady refused).
- **Recommended fix:** an operator exit that closes a `Stuck` buy without a watch as a loss (a terminal `Abandoned { why }` appended to
  `BPhase`, counted as finished, releasing the reservation, and a module call that ends the transit with nothing returned, or a
  `returnAdvance` of what is left), and let `stranded_by` ignore terminal records.

### Info

- **E3-I-1** The probe compares `opt`s only where both sides keep one: a required field changed into an `opt` of a new shape, or an
  `opt` renamed, restores as nothing and is not reported (Candid's opt and field-drop rules; test
  `e3_i_probe_gaps_…`). The contract "new fields optional, new cases appended" is what prevents these, not the probe; the doc
  comment's "at any depth" should say "an `opt` kept by both types".
- **E3-I-2** The fixer's `.did` check (`fixes-round2.md` §11: "parses and decodes `eth-state-round2.hex` as its `EthState`") is vacuous
  for anything under an `opt`: a copy of the `.did` without `EthKind::BackDeposit` still decodes the fixture (its `line` becomes null).
  With the opt-loss comparison added (`didcheck/`), the `.did` at HEAD matches every case and the negative control is caught. There is
  no committed `.did` test; the services' decode of the fixture may share the blind spot (part D).
- **E3-I-3** `lost_opts` names a lost path by Candid hash and index (`saved.1_202_919_412`), not by field name: the trap message tells
  the operator little. Map hashes back through the build's own types (or `candid::types::internal::find_type`) for the message.
- **E3-I-4** The watches are balance deltas, so anyone can move them by sending ETH: a donation of half the minimum on 4663 during
  `BackBridging` makes `returnAdvance` send the donation and leaves Relay's real payout at our 4663 address unattributed (and it can
  later read as a forward leg's "refund"); on Ethereum a donation makes an early "arrival". The attacker pays at least what is
  displaced; griefing only. Detecting Relay's fill by order id (E3-M-1's fix) closes it too.
- **E3-I-5** `Funded` (waiting for an acceptable quote, no timeout; `unsettled` held for ever by a halted line also keeps it) and
  `Bridging` (to its deadline, and for ever under E3-L-2) still hold their collection's deposit mints; the round-2 text says only
  `Arrived` is bounded, which is accurate, but the claim "the hold is bounded" is not general. The operator's `eth_give_back` exits
  `Funded`; a withholding runner could already stall deposits through stale evidence, so no new power.

## 3. What was checked and holds

- **`unsettled`** per tick and per collection: see the table; two collections in the same tick cannot both judge a forward quote (one
  Relay leg), and a `BackFunded` buy's deposit is signed only with the leg free (`:1660`) and the line empty.
- **Nonces after `BackDeposit`:** `sign_eth` signs at `max(plan, nonce_floor)` (`:1512`); a mined `BackDeposit` is `note_mined`; a
  reverted one returns to `BackFunded` less its gas with the watch dropped (`:1999-2002`); `line_clear` of a halted `BackDeposit` goes back
  to `BackFunded` with its order kept in `used_orders` (rule 12). `superseded` halts a `BackDeposit` whose nonce a stranger took.
- **Earmarks:** `BackFunded`/`BackSending` keep what arrived earmarked until the deposit mines (`:171`); the way back spends only
  `wei.min(bal − others)` (`:1674`); returns keep every earmark (`:1491-1495`).
- **The `eth:{id}` sweep:** releases a buy gone, finished or `Stuck` without watch; a buy dropped by C2-L-2 (`retain`) is `None` →
  released; a `Returning`/`Back*` buy keeps its reservation until `returnAdvance` mines.
- **Upgrade:** `79455e1`/`27c0962` → HEAD and `e04f49b` → HEAD decode whole and the probe passes (no false refusal); the probe cannot be
  fooled by page padding; the cost is linear and small; the round-2 fixture is the encoder's; the `.did` matches.

## 4. For sibling parts

- **Part C (reverse leg):** `BackBridging` has no terminal state and no operator exit (`:1166-1168` only logs past the deadline;
  `give_back_now` refuses `Back*`), and a `BackFunded` buy stops every new buy while it waits for the leg (`:1036`) — E3-L-2 is one way
  to reach that for ever. E3-M-1(a) also applies to the way back's Ethereum refund read.
- **Part D (services):** E3-I-2 (a fixture decode is no proof for cases under an `opt`); the monitor's earmark allowance should expect
  earmarks above the balance (E3-M-1, E3-L-2) rather than page a loss.
