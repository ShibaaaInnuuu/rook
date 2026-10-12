# The canister after the Floor Module: fills both ways, protocol sales, relist tickets (v6)

Written 2026-09-23 on branch `canister-floor-followups` (from `9c6df31`), **before its code**. The Floor Module as merged
(docs/v5/floor-module.md §13: a standing reserve, floats the treasury funds, a fills-based price, `itemSold`) and the YOYO policy
(docs/v5/policies-and-fees.md §1: `redeemToRelist`, `recordSale`, `reclaimListing`) changed what the custodian has to do. This
branch makes the canister follow both. It touches `autobuyer/canister-rs/` and this file only: no Solidity (what the contracts
would need is §11), nothing in `src/homechain/`, `web/` or `script/`. Nothing here is deployed; no transaction is sent to any
public chain; the canister never handles a key or a secret it did not already hold.

## 0. What changed on chain, in the canister's terms

| the module | before | now (docs/v5/floor-module.md §13.6) | the canister |
|---|---|---|---|
| `netUnits()` | net ETH-backed units | the **convertible** units `C = E − R` (reserve units are never converted) | the netting signal, unchanged in form: `want = ⌊(netUnits − buffer) / 1e18⌋` once `netUnits ≥ 1e18 + buffer` |
| `escrow()` | wei held for items and burns | the convertible pot **`B`** alone (the reserve `S` is `escrowBalance() − escrow()`) | `budget_left`, unchanged |
| `floatUnits()` | — | units the module minted to itself from the treasury's float allocation; convertible, so **already inside `netUnits`**; after conversion they are the module's item-backed inventory, what `itemSold` sells | read, never added to the demand (§1) |
| `price()` | — | the fills TWAP clamped to the accepted floor's band; what mints and burns price at | read, for the ask of a protocol sale (§3) |
| `minSaleProceeds()` | — | `F_acc × (1 − convBps)`: the least `itemSold` accepts | read: a sale whose net would fall under it is not reported |
| `itemSold(uint256 proceeds)` payable | — | the custodian pays `proceeds` (`msg.value == proceeds`), the module hands it one whole unit of its float inventory, the ETH joins `S`, a sale fill is recorded at `proceeds / (1 − convBps)` | the last step of a protocol sale (§3) |
| `maxItemCost()`, `itemsArrived()`, `itemArrived(uint256)`, `floor()`, `buffer()`, `postFloor`, `floorBounds` | as built | unchanged | unchanged |

The wrappers: `redeemToRelist()` (both) creates a ticket whose destination is the custody address (Bitcoin) or the custodian
(EVM). Such a ticket is never delivered anywhere: it is **listed** (§4).

## 1. The sink view: ten reads, and float units counted once

`floorsink::reads()` grows from six to ten, in this order: `netUnits`, `escrow`, `buffer`, `floor`, `maxItemCost`,
`itemsArrived`, then `price`, `floatUnits`, `minSaleProceeds`, `escrowBalance`. `floorsink::view` is unchanged in what it
answers (`SinkView`); `floorsink::view_full` answers the `SinkView` and a `FloorExtra { price, float_units, min_sale_proceeds,
reserve }` beside it, `reserve = escrowBalance − escrow = S`.

**The trigger does not double-count float units.** A float is a mint to the module with no reserve share; its units are
convertible, so the module's `netUnits()` already carries them (`_stressUnits` in the contract subtracts them only for the
public cap and the stress term, which are not the canister's concern). The canister therefore derives `want` from `netUnits`
alone: a module reading `netUnits = 1.2 items, floatUnits = 1 item` wants **one** item, not two, and `floatUnits` is read
for one purpose only — whether the module has inventory to sell (§3). `test_float_units_are_not_added_to_the_demand`.

## 2. Fill reporting: what each report carries, and why

The module normalises every fill to the floor it implies (§13.4): a purchase's `cost` by `/(1 + convBps)`, a sale's `proceeds`
by `/(1 − convBps)`. So a cost must be **all-in** and proceeds **net**:

- **`itemArrived(cost)`** — unchanged. `cost = wei_for_sats(spend_sats, rate)`, and `frogbuy::wei_for_sats` already adds the
  canister's `CONVERSION_BPS` (200) to the sats' ETH value: the floor the frog cost plus the marketplace's sats plus the
  ETH→BTC conversion the reimbursement must cover. That is the all-in number the module divides by 1.03. (The module's
  allowance is 3%, the canister's estimate 2%: the 1% between them is the canister's margin on a conversion, absorbed on the
  purchase side as today.)
- **`itemSold(proceeds)`** — new. `proceeds = wei_for_sats_net(sold_sats, rate) = ⌊sold_sats × 1e18 / rate × (1 − CONVERSION_BPS)⌋`:
  the sats the sale paid custody, at the reference rate read when the fill confirmed, **less** the BTC→ETH conversion the
  canister will pay to bring them to 4663. That is the net number the module divides by 0.97. The report is sent with exactly
  that much ETH (`msg.value == proceeds`), from the canister's 4663 balance, once that balance holds it (§3.4).

Neither number is the custodian's free word: a cost is bounded by `maxItemCost()`, a sale's proceeds by `minSaleProceeds()`
below, and by the sale's own sats above (the canister never reports more than the rate says the sats are worth).

## 3. Protocol sales: turning the module's inventory back into reserve

### 3.1 When: the reserve-low read

The module has no "reserve is low" view. The canister defines it from two reads it makes anyway and one number the timelock
sets:

```
S            = escrowBalance() − escrow()                      the standing reserve, wei
reserve_low  = S < policy.reserve_floor_wei                    policy: set_sale_policy(module, reserve_floor_wei, markup_bps, max_open, fee_bps)
sell_now     = reserve_low
            && floatUnits() ≥ 1e18                              the module holds inventory to sell
            && the posted floor is fresh for more than CAP_MARGIN_SECS (the same rule a purchase starts under)
            && wrapper.balanceOf(canister) ≥ 1e18               the canister holds a seed unit (§3.2)
            && open sales for this module < policy.max_open     default 1
            && custody has room (H2) and the limiter allows an item out (H3), and the canister is not paused (H1)
```

Why a controller-set floor and not a module read: a threshold is a policy, and every policy number in this canister is code or
the timelock's (`set_purchase_limit`, the caps, the limits), never the operator's. Without a policy for a module there are **no
sales** for it (fail closed). What the contracts could add to make this on-chain is in §11.

### 3.2 The seed, and why proceeds are real

`itemSold` hands the custodian the unit **after** it pays. A sale on the protocol's behalf needs an item to sell, an item needs a
ticket, a ticket needs a whole wX burned, and the canister has none until `itemSold` gives it one. The loop is closed with
**one unit of lag**: the canister holds one wX of the collection — the **seed** — and a sale is

```
redeemToRelist()                 burns the seed; a ticket to custody; drand draws the item
  → list the drawn item          a seller-signed listing at the ask (§3.3); recordDelivery(ticket, keccak(listing)) = delivery-by-listing
  → sold                         the item's outpoint spent by a confirmed fill paying custody the ask
  → bridged                      the sats come to 4663 as ETH (operator-assisted, §3.4)
  → itemSold{value: proceeds}    the module takes the ETH into S and hands back one unit: the seed for the next sale
```

So every `itemSold` reports the **real net proceeds of the sale just made**, and the canister ends where it began, holding one
unit. The seed is a bootstrap per collection: the operator sends one wX to the canister's 4663 address (minted at the module
or bought in the pool). The canister never fronts a report with a number it did not realise, and never sells an inventory frog
(a frog the float paid for is reimbursed only by a conversion; selling it would leave the float unpaid).

### 3.3 The listing: a seller-signed PSBT, spec'd

No keyless marketplace listing API could be confirmed (Magic Eden's needs a key and answered 503 on the last rehearsal;
Ordinals Wallet's escrows are its own contract). The canister therefore produces the one artefact every Bitcoin ordinals venue
takes as a listing, in the format their own tools write, and the operator (or anyone) posts it:

**The seller PSBT.** Version 2, locktime 0, exactly **one input** — the inscription's outpoint at custody, `nSequence
0xffffffff`, `witness_utxo` = custody's output as every explorer served it — and exactly **one output** — `ask_sats` to
custody's own taproot script. The input is signed by the custody key with **`SIGHASH_SINGLE | SIGHASH_ANYONECANPAY`
(`0x83`)**, the signature (64 bytes + the sighash byte) carried as the input's final witness. A buyer adds their own inputs
and outputs around it; BIP-341 under `0x83` commits to this input's outpoint, value, script and sequence and to the output at
the same index, so the only transaction the signature can complete is one in which that very input is spent and custody is
paid exactly `ask_sats` at the matching index. Nothing else about the buyer's transaction is ours to check, and nothing about
it can move the item without paying custody. `sighash::taproot_key_path_typed` computes the `0x83` digest (the existing
function is the `0x00` case); `sale::listing_psbt` builds it and `sale::check_listing` re-reads the bytes and refuses anything
but that shape before the signature is saved.

**The ask.** A protocol sale lists at `price() × (1 + policy.markup_bps)` (default 500 = 5%): the fills price, which mints
and burns use, plus enough that the net — the ask less the venue's fee and the 2% conversion — clears `minSaleProceeds()` =
`F_acc × 0.97`. A YOYO relist lists at the vault's own `listing(ticket).ask` (§4). Sats: `sats_for_wei_gross(ask_wei, rate)`,
the rate read at listing.

**The record.** `recordDelivery(ticket, keccak256(listing PSBT bytes))` (custodian or keeper). The item leaves `held`; it is
on the market; the hash names the listing without naming a venue.

**H2, H3, H4.** The item stays in the custody book until the fill (it is still ours); a listing asks the limiter for one
`Item` under the key `listing:<ticket>` before signing and records it once the signed bytes are saved — the signature is what
lets the item leave, so that is the act counted; `check_listing` is the outbound rule for the partial transaction (the full
`outbound::verify_btc` cannot judge a transaction the buyer has not built yet).

**Delisting.** A listing is a signature; cancelling it is spending the outpoint. `sale_delist(id)` (operator) has the canister
move the inscription to a fresh custody outpoint: input 0 the item, input 1 a clean fee UTXO, every output to custody,
verified as `outbound::Purpose::ConflictOut` — exactly a conflicting spend of a leaked partial signature. Six deep, the item is
re-deposited: `wrapDeposit(…, to = canister)` for a protocol sale (the seed is back; nothing else is owed), `wrapDeposit(…, to
= vault)` then `reclaimListing(ticket)` for a YOYO relist. A membership proof is needed for the re-deposit; the canister keeps
the ones it has and waits for `frog_proof` otherwise.

### 3.4 The fill, the sats, the bridge, the report

The canister watches the listed outpoint (`outspend`). Spent by a transaction that is `CONFIRMATIONS` (6) deep with its block
still the block, whose output **at the same index as our input** pays custody at least `ask_sats`: **Sold**, `sold_sats` =
that output's value (the signature commits to it, so it equals the ask), the rate read then, `proceeds_wei` as §2. A
reorganisation before six deep puts the sale back to watching; the item's cap-book entry is released only at Sold.

**The bridge is operator-assisted.** BTC → ETH on 4663 has no path the canister can run itself today (bridgecall.rs verifies
Across and Chainflip calls for the ETH → BTC direction and is not wired; the reverse would need a second chain's line). The
sats sit at custody; the operator brings `proceeds_wei` of ETH to the canister's 4663 address by whatever route they use for the
purchase reimbursements' return trip; the canister's rule is only: **`itemSold{value: proceeds}` is sent when its 4663 balance
at the pin is at least `proceeds + a fee margin`**, and `proceeds ≥ minSaleProceeds()` at that pin, else it waits and says
why. The report is `Flow::Value` out (ETH to the module), asked and recorded under `sale:<id>`. Mined: **Reported**, final;
the canister holds the next seed. Reverted: read again next tick, the estimate guards the retry.

### 3.5 EVM NFTs

Built here, pure and signed: `seaport::listing_order(...)` builds the `OrderComponents` of a plain full-open listing — offerer
the canister, offer the ERC-721, consideration the ask in native ETH to the canister plus, if the chain's configuration names a
fee recipient and the policy a `fee_bps ≤ chain.max_fee_bps`, that fee to `fee_recipients[0]`; zone zero, conduit zero, salt
from the ticket — and the canister signs its EIP-712 digest with its EVM key (`sale_nft_listing`, operator; the order hash and
the 65-byte signature come back, `seaport::offerer_signed` re-checks the signature recovers to us). That is a validate-free
Seaport listing any venue's API takes. **Not built, operator-assisted:** the collection's `setApprovalForAll(Seaport, true)`
from the canister's address (a transaction through the NFT line the line does not carry yet), posting the order, the
`NftWrapper` ticket walk (the canister has no NFT redemption delivery at all yet), the fill read and the proceeds' bridge from
the NFT chain to 4663. `recordDelivery(ticket, orderHash)` and `recordSale` / `itemSold` for an EVM sale follow the same
phases as Bitcoin once those exist (§11 lists the gaps).

## 4. YOYO relist tickets

A ticket read at the tick's pin that is `Resolved`, whose `destination` decodes to **custody's own script**, and whose `owner`
is **not the canister** is a relist by a YOYO vault: `owner` is the vault. The canister reads `vault.listing(ticket)` — stage
`Listed`, the `ask` — and runs §3.3–3.4 with that ask; the delivery is the listing; a fill is reported with
`recordSale{value: proceeds}(ticket)`, which the vault refuses under `ask × 0.95`: if the realised net (§2) is under that (the
rate moved more than the allowance between listing and fill), the sale is **Stuck** and a person looks — the custodian's word
is bounded below and the canister never tops it up. A delist re-deposits to the vault and calls `reclaimListing(ticket)`.

`redeem.rs` learns the one thing it must: `discover` classifies a resolved ticket whose destination is custody as
`RPhase::Relist`, finished as far as delivery goes. Before this, such a ticket would have been "owed" every tick, asked the
limiter every tick (`ticket:<id>` is one distinct ask, but forty relists are forty), and refused by `custody::build`
("the destination is the custodian itself") forever.

A ticket whose destination is custody and whose owner **is** the canister is a protocol sale's own ticket (§3.2): the sale that
sent `redeemToRelist` adopts it by the ticket count it read before sending.

## 5. Sequencing, caps, the pause

The sale side runs inside `frogflow`'s tick, over the same `FrogIo`, as the hand-ins do: its reads at the tick's pin (the
modules' ten reads; the wrapper's tickets from its own cursor, bounded per tick; a vault's listing), its Bitcoin pass (listings
to sign, outpoints to watch, moves to confirm), and its EVM steps as the **sixth group** of `evm_act` — after reports, mints,
claims, deposits, triggers and hand-ins — one transaction at a time canister-wide, saved before sent, settled by the read-back.
`FStep` gains `Relist`, `SaleRecord`, `SaleReport`, `SaleDeposit`, `SaleReclaim`; `frogflow::send` gains a value
(`send_value`); `FrogIo` gains `save_sales`, `btc_hrp` (to decode a ticket's destination) and `our_balance` (the 4663 balance
a report is sent from), each with a default so a world without sales needs none of them.

H1: every new update method is `only_operator` or `only_controller` and refuses while paused; the tick does not run paused.
H2: an item listed is still held; released at Sold. H3: a listing is one `Item` out; a report is `Value` out. H4: §3.3.

## 6. Stable memory

The upgrade tuple gains an **eleventh** element, `Option<sale::SaleState>`, after the ten, nothing reordered; a state saved by
the ten-element wasm restores with `None` (tested, `a_state_saved_before_sales_existed_still_restores_and_the_sale_state_round_trips`).

```
SaleState { policies: Vec<SalePolicy>, sales: Vec<SaleRec>, next_sale: u64, ticket_cursor: u64, extras: Vec<(module, FloorExtra)> }
SalePolicy { module, reserve_floor_wei, markup_bps, max_open, fee_bps }
SaleRec { id, module: Option<[u8;20]>, vault: Option<[u8;20]>, wrapper, ticket: Option<u64>, phase: SPhase }
SPhase: Redeeming { count_before } | Pinned | Listing { inscription, ask_wei } | Listed { inscription, outpoint, ask_sats, ask_wei, rate, listing, hash }
      | Recorded { … } | Sold { …, fill_txid, sold_sats, rate, proceeds_wei } | Reported { proceeds_wei }
      | Moving { …, txid, raw, fee_input } | Redeposit { inscription, outpoint } | Reclaiming | Failed { why } | Stuck { why }
```

## 7. Endpoints

| method | who | what |
|---|---|---|
| `set_sale_policy(module, reserve_floor_wei, markup_bps, max_open, fee_bps)` | controller (the timelock) | the reserve floor, the markup (≤ 2,000), open sales at once (≤ 4), the EVM fee share (≤ the chain's) |
| `sale_state()` | query | policies, every sale and where it is, the modules' last `FloorExtra` |
| `sale_listings()` | query | every listing waiting to be posted: `(sale, ticket, inscription, ask_sats, psbt_hex)` |
| `sale_delist(id)` | operator | §3.3: move the item, re-deposit, reclaim |
| `sale_nft_listing(chain_id, collection, token_id, ask_wei, life_secs)` | operator | §3.5: a signed Seaport listing |

## 8. Tests

- `floorsink`: the ten selectors are the signatures' keccak; `view_full` parses ten reads; six reads are refused; float units
  are not added to the demand.
- `sale`: the reserve-low rule (each condition alone stops it); the listing PSBT (one input, one output, the ask, `0x83`;
  `check_listing` refuses a second input, a second output, a foreign output, an under-ask); the `0x83` digest against a hand
  construction of BIP-341's SigMsg; proceeds net of conversion and never above the sats' value; the ask from the price and the
  markup; policy bounds; the ticket owner decode.
- `seaport`: a listing order hashes to the same value whichever side builds it, and a signature by a known key recovers as
  the offerer through `offerer_signed`.
- `tests/sales.rs` (fake world): the protocol sale lifecycle — reserve low + inventory + seed → `redeemToRelist` → the ticket
  drawn → the listing signed (`0x83`, custody key, one input) and `recordDelivery` sent with its keccak → a fill paying custody
  the ask, six deep → Sold → the report waits for the 4663 balance → the balance arrives → `itemSold{value}` with exactly the
  net proceeds → Reported; the world's module refuses a value under its minimum or without inventory. The YOYO lifecycle — a
  vault's relist ticket → listed at the vault's ask → `recordSale{value}` — and a delist → conflict-out move → six deep →
  `wrapDeposit(to = vault)` → `reclaimListing`. Reserve not low, no float, no seed, a stale floor, no policy: nothing
  happens. The limiter (H3) refuses a second listing in a day at a one-item limit; a frozen limiter signs nothing. A
  reorganisation of the fill before six deep goes back to watching. The state is saved before every send.
- `tests/redeem.rs`: a relist ticket is `Relist`, never built, never asked of the limiter.
- `canister`: the eleventh element restores from ten.

## 9. Results (recorded after the code, 2026-09-23)

**Built** (`autobuyer/canister-rs/`, commit after `b07bcde`): new files `src/sale.rs` (the ABI, the policy, the records and
phases, the reserve-low rule, the ask and proceeds arithmetic, the listing PSBT and its check, the fill test, the move),
`src/saleflow.rs` (the reads at the pin, the ticket walk, the Bitcoin pass, the sixth EVM group, the mined settlements),
`src/salenet.rs` (`SALES`, the five endpoints), `tests/sales.rs` (the fake world). **Shared-file edits:** `src/floorsink.rs`
(ten reads, `FloorExtra`, `view_full`; `view` unchanged in what it answers), `src/sighash.rs` (`taproot_key_path_typed`, the
`0x83` message; `taproot_key_path` is its `0x00` case), `src/redeem.rs` (`RPhase::Relist`, classified in `discover`),
`src/frogflow.rs` (five `FStep`s, `send_value`, `tick_all` — `tick_full` is `tick_all` with an empty `SaleState` — the two
hooks and the sixth group; `FrogIo` gains `save_sales`, `btc_hrp` and `our_balance`, all with defaults so the other worlds
compile; `flag` and `FrogState::proof` opened to the crate), `src/frognet.rs` (`ItemIo` implements the three; the tick carries
`SALES`; `reserved()` includes a move's fee input), `src/seaport.rs` (`listing_order`, `listing_json`), `src/canister.rs`
(the eleventh element and its restore test), `src/lib.rs`, `etch_autobuyer.did`, `src/floortrigger.rs` (its test fixture is
ten reads), `tests/frogs.rs` (its module answers the four new selectors), `tests/redeem.rs` (one test). Nothing in `src/`
(Solidity), `src/homechain/`, `web/` or `script/`.

**What the code changed in this document.**

1. **The ids.** At a tick the modules are read before the wrapper's tickets, so a protocol sale planned that tick takes the
   lower id than a relist adopted in the same tick (`tests/sales.rs`, the limiter test).
2. **A sale lifts the reserve.** The very sale `itemSold` reports adds its proceeds to `S`; with a floor of one item the
   first sale usually ends the "low" condition, and the next tick reports "the reserve holds … not under the floor" only in
   the page's `extras`, not in the log (the log carries every other refusal so an idle sale side does not fill it).
3. **`sale_nft_listing` uses counter 0.** The canister never calls Seaport's `incrementCounter`, so its counter is zero by
   construction on every chain; the endpoint makes no chain read (a venue validates the counter when the order is posted).
4. **The listing's ask must be at least the item's own sats**, or the seller PSBT would spend value into nothing; an ask
   under it is `Stuck` (it cannot happen at any real price: an item is a few hundred sats, an ask a few hundred thousand).
5. **The `0x83` digest has no differential fixture.** `gen-sighash-fixture.mjs` produces `SIGHASH_DEFAULT` cases only and
   `@scure/btc-signer` is not installed in this checkout; the digest is checked against a hand assembly of BIP-341's SigMsg
   for that type (`sale::tests::the_listing_digest_is_bip341s_single_anyonecanpay_message`). Before a listing is ever posted
   for real, one differential vector should be generated the way the default ones were.
6. **A refunded ticket under a live listing** (the record never mined for seven days) is not left listed: the sale goes to
   `Delisting` by itself and the item is moved; a refund after a fill is `Stuck`.

**Tests.** `cargo test`: **404 pass** (382 before): 296 unit (13 new: the ten selectors and `view_full`, float units not added
to the demand, the sale ABI and decoders, every reserve-low condition, the policy bounds and open counts, the ask and proceeds
arithmetic, the listing PSBT and its check against every tampered shape, the `0x83` message, a fill at the item's index, the
move through `ConflictOut`, a listing order hashing as the taker's basic order with a known key's signature recovering as the
offerer, the guards on every new update method, the eleventh element restoring from ten), 8 in `tests/sales.rs` (the protocol
sale end to end; nothing on no policy / a reserve at its floor / no inventory / no seed / a stale floor; a fill reorganised
out and a report that waits under the module's minimum; a vault's relist end to end with `recordSale{value}`; a delist by
conflict-out move, re-deposit to the vault and `reclaimListing`; a stranger's custody-destination ticket and a holder's
ticket left alone; the limiter making a second listing wait and a frozen one signing nothing; a restart between the send
and the save resuming from the saved step), 1 in `tests/redeem.rs` (a relist ticket is `Relist`, never built, never asked of
the limiter). `cargo build --release --target wasm32-unknown-unknown`: clean; **4,583,219 bytes** (4,266,294 at the base:
+317 KB). Nothing was deployed anywhere; no transaction was sent to any chain; no local replica was needed (every path is
exercised against the fake worlds).

## 10. What this does NOT do

- **Bridge sats to 4663.** Operator-assisted, §3.4. The canister only refuses to report what it cannot pay.
- **Post a listing.** The PSBT is produced and exposed; posting it to a venue is the operator's (or anyone's).
- **EVM sales end to end.** §3.5: the order is built and signed; approval, posting, the fill read, the bridge and the reports
  are spec.
- **Sell inventory.** §3.2.
- **Choose which item to sell.** drand does, through the ticket, as for every redemption.

## 11. For the contracts (interface gaps, exactly)

1. **A reserve target on the module** — `reserveTargetWei()` (or `reserveLow() → bool`), set by the timelock — would move the
   sale trigger's one policy number on chain, where every other module bound lives. Until then `set_sale_policy` carries it.
2. **`itemSold` hands the unit after the payment.** That is why a seed unit is needed (§3.2). An entry that lends the custodian
   one float unit against a listing it has recorded (`lendForSale(ticketId)` / `settleSale{value}(ticketId)`) would remove
   the seed and let the module see the listing hash; it would also let the module refuse a second lend while one is open.
3. **A ticket's owner is read from `ticket(id).owner`** to tell a vault's relist from ours. Fine as built; an event or a
   `relistedBy(ticketId)` view would be cheaper than decoding the struct.
4. **`NftWrapper` tickets** carry `address destination`; the canister has no NFT redemption delivery yet. Nothing to change on
   chain; the canister side is the gap.
5. **`recordSale` under `ask × 0.95` reverts with no way to settle a stuck listing** whose realised net fell short (a rate move).
   A custodian-only `settleShort(ticketId)` that returns the unit to the coin (like `reclaimListing`) without a re-deposit
   — the item having been sold — would need the vault to accept less than the allowance, which the doc rules out; the
   alternative is the operator topping up the difference, which the canister will not do. Stated, not solved.
