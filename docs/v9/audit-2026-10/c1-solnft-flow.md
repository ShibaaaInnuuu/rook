# C1: the Solana NFT flow

| | |
|---|---|
| Scope | `autobuyer/canister-rs/src/solnftflow/` (`mod.rs`, `floor.rs`, `buy.rs`, `deliver.rs`, `run.rs`, `api.rs`: the floor from Magic Eden hand-ins verified on chain, the advance + purchase in one transaction, the custody check and `mint_item` through the deferred writer, the delivery to Solana wallets, the undeliverable rules; `sol_nft_evidence`, `sol_nft_configure`, `sol_nft_set_off`, `sol_nft_status`); its wiring into the money line (`LineOwner::SolNft`, `LineStep::SolNft`, `SolLines.solnft_state`, the upgrade hooks, the timer and the runner's tick, `books_view`); how it uses `solnft*.rs`, `books/`, `chainlink/` (`OwedWrite::SolNft*`, `write` / `apply_owed`, `sign_linked`) and `custodycap.rs`. The primitives themselves (`solnft*`, the judges) were B2's and are not re-reported. Design: `docs/v9/solana-nfts-canister.md` §1–§9, `docs/v9/canister-only-solana.md` §8. |
| Commit | `79b4e60` on branch `golive-2026-10-08` |
| Reference | Round B (`b1-desk-links.md`: B1-06 deferred writes, F1 `collection_busy`; `b2-primitives.md`: B2-01, B2-09), the README's method |
| Method | Threat model (§1); line-by-line review of the scope against it and against the checklist of the assignment; proofs of concept against the fake chain (`tests/desk_common`, the `tests/solnft_flow.rs` setup, the real mainnet accounts of `tests/fixtures/solnft/`) in `autobuyer/canister-rs/tests/audit_c1_solnft.rs`, for the High and, because they were cheap, three of the Mediums, each **passing by exhibiting the bug** on the current code (its comment says what a fix must refuse); no `src` file changed. |
| Result | 0 Critical, **1 High**, **4 Medium**, 3 Low, 6 Informational. Nothing found that overpays beyond the books' item cap, buys a non-member, pays twice, or mints an item not in custody at finalized. |

## 1. Threat model

**Assets**
- The SOL at each Solana NFT collection's module address `M[i]` (backing, revenue, surplus): an advance moves up to the books' `max_item_cost` of it per purchase.
- The items in `K_main`'s custody, and the redeemers' right to one of them per unit burned (a ticket, delivered within `DELIVERY_DEADLINE`, else refunded).
- The books' truth: `held`, `held_markers`, `transit_units`, the tickets' stages, matched to what `K_main` holds on chain.
- `K_main`'s SOL (fees, rents) and the flow's daily budget (`STEPS_PER_DAY` = 24 signed steps, purchases and deliveries together).
- The canister-wide custody limits (the custody caps, the exit-room rule A4-L-1, the outflow limits).

**Actors and what each controls**

| Actor | Controls | Bounded by |
|---|---|---|
| Runner / operator (hand-in) | `sol_nft_evidence`: which Magic Eden listings (any 64 KiB of JSON, the cheapest 40 by its own `price` kept), and the reference `floorPrice`; `sol_nft_configure`, `sol_nft_set_off` | `only_operator`, `not_paused`, `not_ticking`; every candidate verified at finalized; reference gap ≤ 20 %; the books' floor bounds (5 %/h, 20 %/day), freshness (3 h), `max_item_cost` = min(floor × 1.03, backing per unit + revenue + surplus); a widening knob waits 24 h |
| Seller | Lists, edits the price, cancels, relists at any moment (between the read and the landing too); chooses the Tensor maker broker and rent payer | The judge (canonical accounts, `max_amount` / exact M2 price), simulation at finalized, one transaction (a failure moves nothing) |
| Redeemer | The destination (any 32 bytes but zero or the custodian); what they do with the item once delivered (transfer, list, delegate, stake), their ATA before it (delegate, close authority) | `check_destination`, `send_instructions`' on-curve rule |
| Griefer | Cheap listings; redemptions (one wX each) | The verification, the item cap |
| RPC provider (one of two) | Lies, lags, fails | Both must agree at finalized; a disagreement is a wait |
| Magic Eden / Tensor programs | Fee and rule changes, M2's notary, the AH's accounts | The judge; the advance is exactly the all-in at read time |
| Collection update authority | Royalties, verification, rule sets | Out of the model (noted where it matters) |
| The IC | Interleaves at awaits; a trap rolls back to the last await | `hold_tick` / `TICKING`; the endpoints refuse while ticking |

**Trust.** Two providers agreeing at finalized are the chain. The books are right until a link's undo says otherwise. The hand-in is a hint for addresses; its omissions cannot be detected.

## 2. Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| C1-01 | A landed delivery whose destination no longer holds the item at the canister's read is only `note_sent`, re-checked for ever; `next_ticket` takes the first resolved ticket only, so one redeemer moving the item on (a transfer, a listing, a staking delegate) stops every later delivery of the collection; their redeemers get refunds after 37 days, never items, and the gone item stays `held` (and in the custody cap) for good | **High** | `poc_c1_01_one_redeemer_moving_the_item_on_stops_every_later_delivery_of_the_collection` |
| C1-02 | The install's caps name no Solana NFT collection; the flow falls back to a stage-0 policy of the one collection, its global cap sized from it and checked against its own items only: no global item / value cap, no exit-room rule, no outflow limit counts Solana NFTs | Medium | `poc_c1_02_a_full_global_custody_cap_does_not_stop_a_solana_nft_buy` |
| C1-03 | An M2 pNFT purchase composed with the advance, the memo and the nonce is 1,376–1,442 bytes (> 1,232): with M2 switched on, the cheapest (M2) ask is picked, booked, refused for size and undone every pass; the flow never falls through to the buyable Tensor ask | Medium | `poc_c1_03_m2_switched_on_blocks_every_buy_of_a_pnft_collection`, `poc_c1_03_the_composed_m2_pnft_purchase_is_over_the_packet_size` |
| C1-04 | A landed purchase booked through the deferred writer is forgotten by the flow as soon as `book_outcome` OWES it; if the owed `SolNftArrived` is refused when applied (a paused listing, H11), it is only an incident: the item is never booked, the advance stays in transit for ever and the collection never buys again | Medium | `poc_c1_04_an_owed_arrival_refused_at_apply_is_lost_and_the_advance_never_settles` |
| C1-05 | The floor and the purchase are wholly the hand-in's choice among real listings: the reference it is checked against comes from the same hand-in, and nothing canister-sourced cross-checks it (the Ethereum side has the canister's own sale scan); a compromised runner walks the floor up 20 %/day on its own listings and steers every buy to them | Medium | none |
| C1-06 | The floor post (`floor::pass`) writes the books with no `collection_busy` check (review F1, which `home_books` and `frogsol::books` keep): a post made while a desk link or the flow's own advance is open is wiped by that link's undo | Low | none |
| C1-07 | Any simulation refusal of a delivery (but `insufficient_funds`) counts toward `DELIVERY_TRIES`: three transient refusals mark the ticket undeliverable and the redeemer is refunded instead of delivered | Low | none |
| C1-08 | A delivery signed for a ticket past `deliver_by + LATE_DELIVERY_WINDOW` races the public refund: the refund lands first, the delivery lands too: the redeemer has both the item and the unit; `record_delivery` is refused and the gone item returns to the draw (then C1-01's block) | Low | none |
| C1-I1 | The premium knob is inert under the default parameters (`conv_bps` 300: `max_item_cost` ≤ floor × 1.03 always binds below floor × (1 + 10 %…30 %)); a combined change (a narrower premium with M2 on) delays the narrowing 24 h too | Info | |
| C1-I2 | The stable blob carries a transient cache (`asks`, ≤ 120 s useful) typed by other modules (`solnftbuy::Ask`, `Member`, `ListState`, `solnftm2::AuctionHouse`): a change there makes the blob undecodable and refuses the upgrade | Info | |
| C1-I3 | The books' custody proof is the first 32 bytes of a 64-byte signature; the full signature is only in the flow's `held`, dropped at delivery | Info | |
| C1-I4 | The advance is the all-in at read time: a landing that costs less (a seller's price cut) leaves the difference at `K_main` while the books count the full cost (and its fill); a venue fee increase is paid by `K_main` beyond the advance | Info | |
| C1-I5 | The standard can be switched at once while nothing is held, but the cached asks (≤ 120 s) were verified under the old one: a buy from them lands an item the new standard's custody check never confirms (arrival and advance stuck) | Info | |
| C1-I6 | `run::settle` saves the line after taking the outcome and before `deliver::landed`'s await; a trap after that await loses the delivery's booking (the ticket then reads "not in K_main's custody", C1-01's block). The same for a collection authority unverifying an item after its purchase | Info | |

---

### C1-01: one redeemer moving the item on stops every later delivery of the collection (High)

**Where:**
- `src/solnftflow/deliver.rs:117-139` `landed`: a delivery that landed `Some(true)` is `RecordDelivery` only if `check_on(.., to)` passes on a later read; otherwise `NoteSent`.
- `src/solnftflow/deliver.rs:59-67` `pass`: a noted-sent ticket is re-checked; `check_on(..).ok()?` returns `None` when the destination does not hold it (or a read fails): nothing else is tried.
- `src/solnftflow/deliver.rs:17-25` `next_ticket`: `find_map` over the tickets, the FIRST resolved, not undeliverable, `Destination::Sol` ticket of the collection, whatever its `sent_tx`.
- `src/solnft.rs:738-756` `in_custody`: the destination's custody also fails on a delegate or a close authority on its token account, or a pNFT token record not `Unlocked` / delegated (a listing, a staking program).
- `src/books/ledger.rs:426-433` `note_sent` keeps the ticket `Resolved`; `:471-483` `refund` refuses a noted-sent ticket (`DeliverySentNotRecorded`, D3-H-1) and refunds the others only after `deliver_by + LATE_DELIVERY_WINDOW` (37 days after the draw); `:444-458` only `record_delivery` lowers `held` and removes the marker.

**Description.** The delivery is a canonical transfer judged by `judge_send`; when it lands without error, the item has left `K_main` for `to`. The flow nevertheless asks the chain, on a later read at finalized, whether `to` still holds it, and if not books only `note_sent`. From then on the ticket is `Resolved` with `sent_tx` set for ever: `pass` re-reads the destination each tick and gives up silently, and because `next_ticket` always returns that same first ticket, no later ticket of the collection is ever signed. The destination's check is strict (no delegate, no close authority, an unlocked token record), so the redeemer does not even need to race: listing the pNFT on Tensor (it moves to an escrow), staking it (a locked token record with a delegate), or any transfer before the read suffices; for a legacy item an ATA prepared with a delegate or a close authority before the delivery makes the check fail deterministically. Innocent behaviour (an auto-lister, a wallet that stakes on receipt) triggers it too.

**Impact.** The collection's redemptions stop delivering for good, for the cost of one redemption (the attacker keeps the item). Every later redeemer waits 37 days and gets the unit back instead of an item. The gone item stays `held` with its marker: it counts in the custody cap (stage 0: 3 items, C1-02) for ever, so three such events also stop the collection's purchases. No funds are lost, but the collection's core function (wX → NFT) needs an upgrade to recover.

**PoC.** `poc_c1_01_…`: a Tensor purchase of a real Mad Lads pNFT lands and is booked; ticket 0's delivery lands; the redeemer moves the item to another wallet before the flow's read → "noted sent". A second real Mad Lads item B is put in `K_main`'s custody and drawn by an honest ticket 1: twelve ticks over three days sign nothing and never mention ticket 1, B never leaves; after the late window ticket 0 cannot be refunded (`DeliverySentNotRecorded`), ticket 1 can (wX back, no item), and the books still hold the gone item (`held = 2`, its marker present).

**Fix.** Decide the delivery on the transaction, not on the destination's later state: a delivery that landed successfully (the line's verdict at finalized on the canonical, judged transfer), or `K_main` no longer holding the item at finalized after it, is `record_delivery`. Keep `note_sent` only for an unknown outcome, and re-check it against `K_main`'s side (the item gone) rather than the destination's. Independently, `next_ticket` must not let one ticket block the others: iterate over every actionable ticket (skip a noted-sent one whose re-check fails this pass, and one whose item is "not in K_main's custody" after raising the incident).

**Fixed (2026-10-12):** `deliver::landed` books `record_delivery` on the line's verdict alone (a landed, judged canonical transfer; no read of the destination, so no await either: C1-I6), and never `note_sent`; a ticket an older build left noted sent (the only `note_sent` of a SolNft listing was a landed delivery) is recorded at the next pass. `next_ticket` became `due`: `deliver::pass` walks every actionable ticket (up to `TICKETS_PER_PASS` = 8 a pass), skipping one whose item is not in `K_main`'s custody (logged for the operator), one the judge refuses or one past its late margin (C1-08), and signs the first that can go. Regression: `fixed_c1_01_one_redeemer_moving_the_item_on_no_longer_stops_any_delivery_of_the_collection`, `fixed_c1_01_a_ticket_noted_sent_by_an_older_build_is_recorded_delivered`.

---

### C1-02: no global custody limit sees a Solana NFT (Medium)

**Where:**
- `src/canister.rs:398-427` `caps_of`: the default policy lists the wrappers' inscriptions, the live Inscriptions listings (`frognet::live_cap_keys`) and the Ethereum collections; never a `ListingKind::SolNft`; the global item cap is then re-sized by the exit-room rule (A4-L-1).
- `src/solnftflow/api.rs:141-143` returns that policy; `src/solnftflow/buy.rs:127-137`: `io.caps().filter(|p| p.cap(&key).is_some()).unwrap_or_else(|| stage0(&[key]))` and a `Book` of this collection's `held_markers` only.
- `src/custodycap.rs:197-221` `stage0` of one non-EVM key: 3 items, 10 ETH, and a global of max(3, 6) items, 10 ETH sized from it alone.
- No `outflow_check` / `outflow_record` in `src/solnftflow/` (deliveries are bounded only by `STEPS_PER_DAY`).

**Description.** Each Solana NFT collection is checked against a private policy that knows no other collection, with a book that holds none of their items. The canister-wide `global_max_items` / `global_max_value_wei` (whose default is cut to what the outflow limits let out within the timelock's delay, A4-L-1) are never applied to a Solana NFT buy; and since the Ethereum and Ordinals flows' books do not include Solana NFTs either, their global checks do not count them. Even an install that names `sol:<collection>` in its caps only gets its global cap checked against that collection's items. `validate_custody`'s exit rule and the outflow limits ignore them.

**Impact.** Every Solana NFT collection adds 3 items / 10 ETH (stage 0) outside every global bound; the review-round-4 guarantee that the whole default custody can leave within the delay no longer holds once Solana NFT collections are listed. Defence in depth, not a direct loss.

**PoC.** `poc_c1_02_…`: the install's policy has a global cap of 0 items (any acquisition under it is refused); the flow still signs the purchase.

**Fix.** List every live `SolNft` listing in `caps_of` (stage 0, `sol:<collection>`), refuse a buy whose key the install's policy does not name (as `check_acquire` refuses an unlisted collection), and check against one canister-wide book (the cap book plus the books' held Solana NFTs). Record deliveries in the outflow limits like the other flows.

**Fixed (2026-10-12):** `canister::caps_of` lists every SolNft listing of the books home at stage 0 (`solnftflow::live_cap_keys`, `sol:<collection>`), so the default global cap and its exit-room re-size (A4-L-1) and `validate_custody` count them; the buy checks the install's policy as it is (no per-collection fallback: an unnamed collection is `NotListed`, so an install that names its `custody_caps` must name `sol:<collection>` for each Solana NFT listing) against ONE book: the canister's (`SolNftIo::cap_book`, the custody book) plus every SolNft listing's items held and landed-not-booked (`solnftflow::cap_entries`, ids `solnft:<mint>`); `canister::with_caps` adds the same entries for the call (never kept), so every other flow's global checks and the `custody_caps` headroom count Solana NFTs too. Deliveries ask `outflow_check(Flow::Item)` under `solnft:<coll>:deliver:<ticket>` before signing and `outflow_record` once signed (listed in the H3 review of `audit_nft_r5.rs`). Not changed: the arrival is not counted as a `Flow::Mint` (it mints to the module, not to a holder), and `held_fits` at `post_upgrade` still reads the custody book alone (the desk state is restored after it). Regression: `fixed_c1_02_a_full_global_custody_cap_stops_a_solana_nft_buy`, `fixed_c1_02_the_flows_items_are_in_the_canister_wide_book_and_the_default_policy`, `fixed_c1_02_a_delivery_waits_on_the_outflow_limits_and_is_recorded`.

---

### C1-03: M2 switched on blocks every buy of a pNFT collection (Medium)

**Where:**
- `src/solnftflow/buy.rs:29-45` `pick`: with `m2`, the cheapest M2 ask with its auction house is a candidate, chosen when cheaper all in; no memory of a refusal.
- `src/solnftflow/buy.rs:161-185`: the transaction is the venue's instructions + the advance (a second signer, `K_coll[i]`) + the memo, on the money line's durable nonce; `sign_linked` → `solsend::sign_intent` → `soltx::assemble` → `check_size` refuses it; the link is undone and the error is a "waits".
- `src/solnftm2.rs:103-150` `m2_buy`: `deposit`, `buy_v2`, `mip1_execute_sale_v2` (27 accounts + creators) for a pNFT.
- `tests/solnft.rs:452-463` `wire_size_fits` measures the venues' instructions alone and leaves M2 out.

**Description.** Measured on the fixtures: an M2 pNFT purchase alone is 1,135–1,201 bytes; composed as the flow sends it (nonce advance, the advance transfer and its second signature, the memo) it is 1,409 bytes for the Mad Lads listing (1,376–1,442 across the six pNFT fixtures, one to three creators; the legacy M2 fixture: 1,204, fits). The send line uses legacy messages, no lookup table. `pick` chooses the M2 ask every pass while it is the cheapest within the limit (the usual case: Magic Eden's listings of pNFT collections are mostly M2), books the advance, fails at the size check, undoes it, and returns a "waits". It never tries the next ask. The same holds for any ask whose purchase deterministically fails (a refusal by simulation): it is retried for its 120 s each hand-in, and the flow never falls through.

**Impact.** For a pNFT collection the operator's M2 switch (meant to widen the market) stops all purchases while an M2 listing is the cheapest; the advance is booked and undone every pass (link records, reads of the chain at finalized, cycles). No funds at risk.

**PoC.** `poc_c1_03_…`: a Mad Lads Tensor ask (10.62 SOL all in) and a Mad Lads M2 ask (8.87 SOL) under a 10.7 SOL floor, M2 on: three passes log "transaction is 1409 bytes; Solana's limit is 1232", nothing is signed; with M2 off, the same hand-in buys the Tensor ask at once.

**Fix.** Build and size the whole transaction before choosing (skip what cannot be signed); remember a refused ask for the rest of its hand-in and fall through to the next; for M2 pNFTs, a v0 message with a canister-read lookup table (the send line supports `compile_v0`), or keep M2 to legacy items. Add the composed transaction (nonce, advance, memo, two signatures) to `wire_size_fits` for every venue and standard.

**Fixed (2026-10-12):** `buy::plan_and_sign` builds, judges and sizes each candidate as it will be signed BEFORE anything is booked: `buy::composed_size` (the durable nonce's advance whatever the lifetime, `buy::compose`'s venue instructions + advance + the longest memo a link of the collection can carry, both signatures) must pass `soltx::check_size`; an ask that cannot be built, is refused by the judge or does not fit is skipped and remembered for the rest of the read (`Cache::refused`), as is one every provider refuses by simulation at signing, and the next ask is tried. M2 pNFT asks are therefore never picked (no v0 message); an ask bought from also leaves the read's asks. Regression: `fixed_c1_03_m2_switched_on_no_longer_blocks_the_buy_of_a_pnft_collection`, `fixed_c1_03_the_composed_size_is_measured_before_an_ask_is_picked`.

**Fixed further (2026-10-12): M2 pNFT purchases fit.** The operator turned Magic Eden M2 on by default (`ColState.m2` defaults to
`true` for a collection the flow holds no state for; a state saved before keeps its value; `sol_nft_configure(.., m2 = Some(false))`
still applies at once, switching it back on still waits 24 h). An M2 purchase is now the same ONE transaction (the nonce's advance, the
advance `M[i] → K_main`, the purchase, the memo; judged whole by `judge_tx` with `solnftm2::judge`, only `SetComputeUnitLimit` beside
it, B2-01; signed by `K_main` and `K_coll[i]`; the link booked before and undone on any non-landing) compiled as a v0 message that loads
M2's fixed accounts (the auction house, its notary, authority and treasury, M2's signer PDA, `K_main`'s escrow, the Token, Token
Metadata, Auth Rules and ATA programs, the Instructions and Rent sysvars, the two common rule sets; nothing of a seller or an item) from
the canister's own address lookup table (`solnftflow/table.rs`). The table is made once, on the money line, by `K_main` alone
(`SolNftStep::Table`: `CreateLookupTable` on a finalized slot + `ExtendLookupTable`; failed or never landed: forgotten and made again),
the first time an M2 ask waits for it; it is read at finalized before every use (the ALT program's, `K_main` its authority, never
deactivated, holding exactly what it was made with, its last extension older than the finalized slot) and never changed by the canister,
so a signed message's indexes cannot be repointed; `solsend::compile_with` refuses a v0 message that does not decompile, against the
table as read, to the judged instructions, and `Saved.tables` keeps it for a fallback re-sign. `buy::composed_size` sizes the v0 message
before anything is booked: every M2 pNFT fixture 1,028–1,094 bytes (Mad Lads 1,428 → 1,061; 1,042 as signed in LiteSVM), the legacy M2
one 980; Tensor and MMM purchases stay legacy messages. An M2 ask seen while no usable table is there waits (skipped this pass, not
remembered as refused, nothing booked); one that does not fit even with it is refused as before. The two-transaction alternative (the
advance as its own link, then the purchase signed by `K_main` alone) was measured and rejected: the purchase alone with its nonce and
memo is 1,282–1,348 bytes (1,242–1,308 without its compute limit). Residual: an MMM pNFT purchase is 1,356 bytes as a legacy message and
is still skipped before anything is booked. Regression: `fixed_further_c1_03_mad_lads_is_bought_from_m2_by_default_end_to_end`,
`fixed_further_c1_03_a_failed_m2_purchase_returns_the_advance_to_the_module`, `fixed_further_c1_03_a_lookup_table_not_as_made_is_never_used`,
`fixed_further_c1_03_m2_is_on_by_default_off_at_once_on_after_the_delay`, the two `fixed_c1_03_…` tests (nothing oversized is ever
booked; every M2 fixture fits as a v0 message with 128 bytes to spare), and `autobuyer/solnft-svm/tests/m2_table.rs` (the table and the
flow's v0 transaction landing on M2's mainnet bytes).

---

### C1-04: an owed arrival refused at apply is lost and the advance never settles (Medium)

**Where:**
- `src/solnftflow/buy.rs:221-234` `arrived`: `desk::book_outcome(coll, SolNftArrived)`; on `Ok(())` the arrival is cleared and the item put in the flow's `held`.
- `src/chainlink/ledger.rs:447-453` `write`: with a link of the collection open, the write is OWED and `Ok(())` is returned; `:457-469` `apply_owed`: a refusal is an incident and the write is dropped.
- `src/chainlink/ledger.rs:212-220` `SolNftArrived`: `mint_item` (refused on a paused listing, under H11, `CustodyFull`, `NotEnoughBacked`) then `settle_advance`.
- `src/books/ledger.rs:280-281` `mint_item`: `ListingPaused`.

**Description.** In the direct path a refusal is retried: `arrived` keeps the arrival and tries again next pass. In the deferred path the flow treats "owed" as "booked": it forgets the arrival at once. When the owed write is later applied and refused, nothing holds it any more. A paused listing is enough: pausing stops the public mint, not burns or redemptions, so desk links of the collection run while it is paused, and a halt pauses the listing too (`Ledger::halt`; `clear_halt` leaves it paused).

**Impact.** The item is in `K_main`'s custody but never in the books; `transit_units` stays 1 for ever, so every later `advance` is `AdvanceOpen` and the collection never buys again; the flow's status shows the item as held. An upgrade is needed to recover.

**PoC.** `poc_c1_04_…`: a purchase lands and the flow takes its outcome; the listing is paused and a desk burn link of the collection is open; the next pass confirms custody, the write is owed and the arrival forgotten; the burn lands, the owed write is refused (incident); after unpausing nothing books the item: `held = 0`, `transit_units = ONE`, `advance` → `AdvanceOpen`.

**Fix.** Keep the arrival until the books HOLD the item (the flow checks `held_markers` for the asset and `transit_units == 0` before clearing it), or have `apply_owed` keep a refused `SolNftArrived` owed (and retried at each decision) rather than drop it. Consider letting `mint_item` to the module ignore the public-mint pause: the purchase is already paid.

**Fixed (2026-10-12):** `buy::arrived` keeps the arrival until the books HOLD the item (its held marker): while its `SolNftArrived` is owed it waits (never writes it twice); once the books hold it the arrival is cleared and the item held by the flow; if the owed write was dropped (refused at apply), the custody is read again and the write made again, each pass, until accepted, so the advance settles (after an unpause, here). `mint_item`'s pause rule is not changed. Regression: `fixed_c1_04_an_owed_arrival_refused_at_apply_is_retried_and_the_advance_settles`, `fixed_c1_04_an_owed_arrival_applied_later_clears_the_arrival`.

---

### C1-05: the floor and the purchase are the hand-in's choice (Medium)

**Where:** `src/solnftflow/floor.rs:13-32` (`put_evidence`: the listings and the stats both from the caller), `:41-94` (`pass`: `floor_from(asks, ev.reference, …)`), `src/solnftfloor.rs:54-91` (`floor_from`: rank 2 of ≥ 3 sellers among the handed-in asks, gap to the hand-in's reference ≤ 20 %), `src/solnftflow/buy.rs:115-120` (the cheapest of the same asks under the limit).

**Description.** Every fact the canister uses is read on chain, but which facts it reads is the hand-in's choice, and so is the reference the floor is checked against. Omitting the cheap listings is undetectable. A compromised runner key (or operator) with three seller wallets lists items above the market, hands in only those and a matching `floorPrice`: the floor moves up within the books' bounds (5 %/h, 20 %/day, compounding), and every purchase goes to the attacker's listings at up to `max_item_cost` (floor × 1.03, and backing per unit + revenue + surplus). The Ethereum flow's floor has a canister-sourced second signal (its own sale scan, `ethflow.rs` `reading_of`); this flow has none.

**Impact.** A runner compromise drains the modules' revenue and surplus into chosen listings over days, and buys at the inflated floor; detectable by the monitor only if it compares with an independent source.

**Fix.** A reference the canister fetches itself (an HTTPS outcall to Magic Eden's or Tensor's stats, two sources agreeing), or a second on-chain signal (the canister's own scan of recent sales of the collection's members); bound the buy limit by the floor posted a day earlier; alarm on a floor that only moves up on hand-ins naming few sellers.

**Fixed (2026-10-12):** bounded, not sourced: a buy's limit is now also at most the LOWEST floor the books accepted from this flow's posts over the last `FLOOR_WINDOW_DAYS` (7) days and today, plus the collection's premium (`buy::window_limit`; the day's lowest kept in `ColState::floor_days`, an optional field so an older blob decodes). A floor walked up on chosen listings (the books allow 20 %/day) therefore carries no buy above the week's low + 10 % until it has held a whole week: the compromise's pace drops from ≈ 20 % a day to the premium a week, in plain view of the monitor. Residual: the hand-in still chooses which listings are read (omissions are undetectable) and the floor itself still follows it within the books' bounds (the public mint's price); an independent source (an HTTPS outcall to Magic Eden / Tensor stats, or a canister scan of the collection's sales) remains the full fix. Regression: `fixed_c1_05_a_floor_walked_up_carries_no_buy_above_the_weeks_low`.

---

### C1-06: the floor post ignores `collection_busy` (Low)

**Where:** `src/solnftflow/floor.rs:72-81` writes `s.books.post_floor` with no `desk::collection_busy` check; compare `src/ethflow/home_books.rs:139-141` and `src/frogsol/books.rs:132` (review F1). `src/chainlink/ledger.rs:367-375` the undo snapshots (`Undo::Module` holds the module, `Undo::Collection` the whole collection).

**Description.** `run::tick` reads each collection's evidence and posts its floor while the flow's own advance may be out (it is `Signed` until the line's verdict) or a desk link is open. If that link is undone (the purchase failed, never landed; the desk's operation refunded), its snapshot is restored and the post is wiped: the floor reverts and may go stale (the public mint closes) until the next hand-in is read.

**Fix.** Skip the post (keep the evidence unread) while `collection_busy(coll)`, as the other floor writers do.

**Fixed (2026-10-12):** `floor::pass` returns before any read or post while `desk::collection_busy(coll)`; the evidence stays unread (its `read_at` unchanged) and is read and posted at the first pass the collection is free (within its 30 min). Regression: `fixed_c1_06_no_floor_is_posted_while_a_link_of_the_collection_is_open`.

---

### C1-07: transient simulation refusals make a ticket undeliverable (Low)

**Where:** `src/solnftflow/deliver.rs:104-111`, `:31-42` (`refused`), `:140`.

**Description.** Any simulation refusal not classed `insufficient_funds` counts as a refusal of the destination; three mark the ticket undeliverable (the redeemer is refunded, not delivered). A refusal may come from the canister's side (a stale read of the source token account, a nonce or blockhash issue both providers report, a provider-side simulation fault). The pNFT veto of B2-09 is the case meant; the rule does not tell it apart.

**Fix.** Count only refusals whose error is the transfer instruction's (index of the Token Metadata / Core / SPL instruction, with the program's error), and only when the read at finalized shows `K_main` still holds the item; spread the tries over time.

**Fixed (2026-10-12):** a simulation refusal counts toward `DELIVERY_TRIES` only when its error names one of the delivery's own instructions (`deliver::names_destination`: the `InstructionError` index, less the durable nonce's advance, is the transfer or, for a legacy item, the destination's token account or the source's close; never the compute limit); every other refusal (the nonce, a provider's fault, no instruction named) is a wait. The custody read at finalized before each signature already holds. Not changed: the tries are not spread over time, and a delivery that landed and failed on chain still counts. Regression: `fixed_c1_07_a_refusal_that_is_not_the_destinations_never_counts` (the fake chain now reports message indexes, `tests/solnft_flow.rs` too).

---

### C1-08: a late delivery races the public refund (Low)

**Where:** `src/solnftflow/deliver.rs:52-112` signs a delivery for any resolved ticket, however late; `src/books/ledger.rs:471-483` lets anyone refund it after `deliver_by + LATE_DELIVERY_WINDOW` while it is not noted sent; `src/solnftflow/deliver.rs:135-138` only logs the books' refusal of `record_delivery`.

**Description.** After a stall of the flow (off, paused, the line stuck) of more than 37 days, the flow signs a delivery for a ticket that the cranks may refund before it lands. Both land: the redeemer has the item and the re-minted unit; the books return the item to the draw although it left; the next ticket drawing it blocks the collection (C1-01).

**Fix.** Do not sign a delivery for a ticket within a margin (a day) of its refund time: past it, let the refund rule decide. (`note_sent` before signing would instead leave a never-landed delivery unrefundable.)

**Fixed (2026-10-12):** `deliver::pass` signs no delivery for a ticket within `LATE_MARGIN_SECS` (one day) of its late refund (`deliver_by + LATE_DELIVERY_WINDOW`) or past it; the ticket is left to the refund rule and the next one is tried. Regression: `fixed_c1_08_no_delivery_is_signed_near_or_past_the_late_refund`.

---

### Informational

- **C1-I1** `src/solnftflow/buy.rs:22-25`, `src/books/floor.rs:545-551`, `src/books/params.rs:53`: `limit_of` = min(floor × (1 + premium), `max_item_cost`), and `max_item_cost` ≤ floor × (1 + `conv_bps`) = floor × 1.03 by default: the premium (10 % by default, up to 30 %) and its 24 h delay never bind. `api.rs:69-78`: a change that narrows the premium and switches M2 on is delayed whole.
  **Fixed (2026-10-12):** documented at `DEFAULT_PREMIUM_BPS` (over the current floor `conv_bps` binds) and made effective by C1-05 (the premium is what binds over the 7-day low); `api::configure` applies the narrowing half of a combined change at once and delays only the widening half. Regression: `fixed_c1_i1_a_narrower_premium_with_m2_switched_on_narrows_at_once`, `fixed_c1_05_a_floor_walked_up_carries_no_buy_above_the_weeks_low`.
- **C1-I2** `src/solnftflow/mod.rs:144-165, 197-247`: `ColState.asks` (useful for 120 s) and `Evidence` are persisted with types of `solnftbuy` / `solnftm2` / `solnft`; candid's `V1` decode traps the upgrade if those change. Persist neither (re-read at the next hand-in) or version them here.
  **Fixed (2026-10-12):** neither is persisted: `ColState.evidence` and `ColState.asks` moved to a transient per-collection `Cache` (with the read's refused asks, C1-03), lost at an upgrade (the flow waits for the next hand-in); a blob of the build before (with both fields) still decodes (candid skips the extra fields); the new persisted fields (`floor_days`, `rate`) are optional. Regression: `fixed_c1_i2_the_asks_are_not_persisted_and_an_older_blob_decodes`.
- **C1-I3** `src/solnftflow/mod.rs:297-305`, `src/chainlink/ledger.rs:212-216`: the item's `source_tx` is half a Solana signature; `Held.signature` is removed at delivery (`deliver.rs:129-133`). Keep the full signature in the books' view (or in a kept log) so the custody proof can be looked up.
  **Recorded, not changed:** a books-side view change, outside this round's smallest-change scope; the full signature stays in the flow's `held` (and `sol_nft_status`) while the item is held.
- **C1-I4** `src/solnftflow/buy.rs:126, 161`, `src/chainlink/ledger.rs:212-218`: the advance is `ask.total + rent` at read time and `settle_advance(cost)` books it whole; a cheaper landing leaves the change at `K_main`. Tensor's taker fee is outside `max_amount` (`solnftbuy.rs` J3): a fee raise by the program is paid by `K_main`.
  **Recorded, not changed:** the change is bounded by the judge's exact amounts and costs `K_main` at most the venue's fee delta; reconciling the advance with the landed cost needs a read of the landed transaction's balances (a later round).
- **C1-I5** `src/solnftflow/api.rs:53-68`: the standard changes at once while nothing is held, but `ColState.asks` still holds members decided under the old one (≤ 120 s); clear `asks` (and `evidence`) on a standard change.
  **Fixed (2026-10-12):** `api::configure` clears the collection's cache (the hand-in, the asks, the refused) and its `read_at` when the standard changes; nothing is bought until a hand-in is read under the new standard. Regression: `fixed_c1_i5_a_standard_switch_drops_the_cached_asks`.
- **C1-I6** `src/solnftflow/run.rs:85-117`: the outcomes are taken and saved before `deliver::landed`'s await; a trap after it loses the booking. A collection authority unverifying (or re-ruling) an item after its purchase leaves the arrival unconfirmable (advance stuck) or the delivery "not in custody" (C1-01's block); out of the model, recorded.
  **Fixed (2026-10-12):** `deliver::landed` is no longer async (C1-01: no read of the destination), so `run::settle` takes the line's outcomes and books every one of them in the same message, with no await between. The authority case stays recorded; its "not in custody" ticket no longer blocks the others (C1-01). Regression: `fixed_c1_01_one_redeemer_moving_the_item_on_no_longer_stops_any_delivery_of_the_collection`.

## 3. Checked and found correct

- **The advance before the signature, undone on every non-landing.** `buy.rs:143-159` books the advance (`Books::advance`, `Scope::Module`, `Effects { lamports_out }`) before anything is signed; `judge_tx` refusal (`:164-170`), the guard, an unreadable chain, `pre_sign` (with a halt), the line not ready, a signing or simulation error (`coins/link.rs:43-105`), a failure on chain and a proven non-landing (`run.rs:104-113`) each undo it. One transaction: a failed purchase moves nothing from `M[i]` (verified by `tests/solnft_flow.rs::a_failed_landing_undoes_the_advance`).
- **No overpay, no double pay.** The judge (`judge_tx`, `buy.rs:65-81`) admits exactly one System instruction (the advance `M[i] → K_main` of exactly the ask's all-in, ≤ the limit), exactly the link's memo, and the venue's judge on the rest (J1–J4, only `SetComputeUnitLimit`: B2-01 holds, `soltx::compute_budget_ok`). The books re-check `amount ≤ max_item_cost` and refuse a second advance while one is in transit (`AdvanceOpen`); an arrival blocks new buys; assets the books hold are never picked. A seller raising the price between read and landing makes the purchase fail (Tensor `max_amount`, M2's exact price, mmm `max_payment_amount`); lowering it costs less (C1-I4).
- **No non-member, no fake.** Every candidate is verified at finalized, both providers agreeing (`floor.rs:51-56`, `solnftbuy::verify_all`): the listing account's program, its derivation, the escrow or the seller holding exactly the asset, live and in SOL; membership (`solnft::is_member`: verified collection, one-of-one, master edition, no Token-2022, Core plugins). The judge's J4 binds the member to the asset and the collection.
- **`mint_item` only with the item in custody at finalized.** `arrived` reads `check_keys` for `K_main` at finalized and decides `check_on` before `SolNftArrived`; `mint_item` + `settle_advance` are applied on a copy, together or not at all (`chainlink/ledger.rs:212-220`); the source is `keccak(mint ‖ tx)`, unique per (item, transaction).
- **No double delivery, no wrong address.** One step of the flow out at a time (`s.out`, the line's pending); `K_main`'s custody is read at finalized before each signature; `send_instructions` / `judge_send` admit only the canonical transfer to the ticket's destination; the destination cannot be zero or the custodian (`books/ledger.rs:350-353`), nor off the curve (marked undeliverable at once); `Destination::Burn` is refused for a SolNft listing.
- **Open links (B1-06, F1).** The buy and the delivery wait while the collection has an open link (`collection_busy`); every outcome write goes through `desk::book_outcome` (except the floor post, C1-06).
- **The knobs.** The premium is bounded at 3,000 bps; a raise or M2 switched on waits 24 h (applied in `run.rs:20-30`), a narrowing applies at once and cancels a pending widening; `sol_nft_set_off` stops new signatures only, outcomes are still booked.
- **Hand-ins.** Operator-only, refused while paused or ticking; 64 KiB / 4 KiB; the cheapest 40 kept; read once (`e.at > read_at`), not at all past 30 min; asks bought from for 120 s after the hand-in at most. A read is at most ≈ 40 × 6 keys in pieces of `MAX_KEYS`, accounts capped at `MAX_NFT_ACCOUNT`: no unbounded work.
- **pNFT quirks.** The token record is created by the purchase and must be `Unlocked` with no delegate for custody; the delivery is Token Metadata `TransferV1` with both token records and the item's rule set; a rule set veto (B2-09) is undeliverable after three refusals (C1-07 for the edge).
- **The custody cap's valuation.** Items in custody at their cost, the new one at its all-in, converted at the oracle's lamports per ETH (`rates::lamports_to_wei`, overflow refused by `check_acquire`); stage 0 for `sol:` keys is 3 items / 10 ETH (C1-02 for what it does not count).
- **Wiring and upgrade.** `LineOwner::SolNft` maps to the money line; `LineStep::SolNft` carries its link (`chainlink/driver.rs:147`); the state rides in `SolLines.solnft_state` (`#[serde(default)]`) across an upgrade, saved after the tick guard and restored after the lines; a blob that does not decode refuses the upgrade; the timer and the runner's tick run the flow after the pool desk, never while paused; `books_view` shows SolNft keys in base58.

## 4. Test results

- `cargo test --locked --test audit_c1_solnft`: 5 passed (the PoCs of C1-01–C1-04, each exhibiting its bug, and C1-03's size measure over seven M2 fixtures).
- `cargo test --locked --no-fail-fast` (the whole canister, this file included): 1,209 passed, 0 failed.
- Fix round (2026-10-12): the PoCs inverted into `fixed_c1_0x_…` tests and the fixes without a PoC tested: `cargo test --locked --test audit_c1_solnft` 16 passed; the whole canister 1,229 passed, 0 failed.
- Fixed further (2026-10-12, M2 pNFT purchases fit, M2 on by default): `cargo test --locked --test audit_c1_solnft` 20 passed; the whole canister 1,256 passed, 0 failed; `autobuyer/solnft-svm` `m2_table` 1 passed (the suite 17 passed, the two `m2_explore` tests that need `M2_FIXTURE` failing as before).
