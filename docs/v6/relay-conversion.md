# Relay conversion: a graduation's own ETH pays for its frog

Written 2026-09-26 on branch `relay-float` (from `main` at `de5fb8f`), **before its code**. Nothing here is deployed; no key was
used; nothing was sent to a public chain. The only calls made were read-only: Relay's public API (`/chains`, `/quote`), the
Robinhood Chain RPC (`eth_getCode`, `eth_call`, `eth_getStorageAt`) and Relay's documentation.

## 0. The decision, and what it changes

Today a frog is bought with **operator money**: the canister signs the seller's PSBT with the **float** key, spending sats the
operator pre-funded (`docs/v6/phase1-scope.md` O7, "the float: plain sats for about one frog plus fees";
`autobuyer/canister-rs/src/frogbuy.rs`, "Buying one frog … from the float"). Only after the frog is six deep, wrapped to the
Floor Module and reported (`itemArrived(cost)`, `src/floor/FloorModule.sol`) does the module pay `cost` in ETH to the canister's
EVM address (`payee`). Nothing turns that ETH back into sats, so the float is spent once and the operator funds it again.

The operator chose **option (b), no operator money** (2026-09-26): the module's own ETH pays for its frog. The order is reversed:

```
 today:     float sats ──buy──▶ frog ──6 conf──▶ wrapDeposit ──▶ itemArrived(cost) ──ETH──▶ payee (reimbursement)
 option b:  advance(amount) ──ETH──▶ payee ──Relay──▶ float sats ──buy──▶ frog ──6 conf──▶ wrapDeposit ──▶ settleAdvance(cost)
```

The unit is **backed by the ETH until the frog arrives, then by the frog**. While the ETH is on its way (at the canister's EVM
address, in Relay's depository, as sats in the float) it is **in transit**: the module counts it apart, no burn can reach it, and
the stage-0 caps and the daily item limit bound it exactly as they bound `itemArrived` today.

### 0.1 What was measured today (2026-09-26), and what is assumed

| Fact | Status | How |
| --- | --- | --- |
| Relay lists Robinhood Chain (4663, protocol id `robinhood`, native ETH `0x0…0`) and Bitcoin (8253038, protocol id `bitcoin`, native BTC `bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8`) | **measured** | `GET https://api.relay.link/chains` |
| Relay's depository on 4663 is `0x4cd00e387622c35bddb9b4c962c136462338bc31` | **measured, three ways** | `/chains` → `protocol.v2.depository`; every quote's `paymentDetails.depository` and deposit `to`; the address holds 8,628 bytes of code on 4663 |
| The depository is not a proxy; `owner()` = `0xF61A305199fa1135d76FFaB3752D42F55cBd775A`, `allocator()` = `0x63C1d3E9C646184529C5694630a01C00dF171b56`; code hash `0xc8b9128d5bc2534ff2b2d7a45fd35a657ec345f5b9ad1d7a05b47b73d1ef4b07`; `depositNative(address,bytes32)` (`0x49290c1c`) in its code | **measured** | `cast code`/`cast storage` (both EIP-1967 slots zero)/`cast call` against `rpc.mainnet.chain.robinhood.com` at block 73,241,920 |
| A quote for 0.16 ETH → BTC at a fresh `bc1p…` recipient: **one** step `deposit` of kind `transaction`, `depositNative(user, orderId)` to the depository, value 0.16 ETH, 505,655 sats expected, **495,542 sats minimum** (2 % slippage), ≈ 304 s | **measured** | `POST /quote` (fixture `autobuyer/canister-rs/tests/fixtures/relay-quote-4663-btc-2026-09-26.json`) |
| **The deposit id is `hashStruct(Order)`, the EIP-712 struct hash of `protocol.v2.orderData`**, and the order names the **Bitcoin recipient, the minimum sats, the output currency and chain, the deadline and the refund recipients** | **measured** | Relay's "Input Validation" page (docs.relay.link, read today); recomputed with `@relay-protocol/settlement-sdk@0.0.143`'s `getOrderId` for the 2026-09-26 quote **and both 2026-09-23 fixtures**: all three equal `orderId` and the calldata's `id` |
| The order's deadline is the quote's time + 604,800 s (7 days) | **measured on one quote** (09-26: 1,791,046,471 − 1,790,441,670) | not relied on (§3.1 rule 11 takes a band) |
| The order's refunds name the quote's `user` (4663) and the recipient (Bitcoin) | **measured** | both fixtures; `refundTo` set to `user` today |
| Relay refunds on the origin chain to the refund recipient when it cannot fill, less the refund's gas; "no refund" if the gas exceeds it; a quote regenerated below `minimumAmount` is refunded | **documented, not observed** | docs.relay.link "Refunds", "How it works → Fast refund" |
| A deposit nobody fills can be reclaimed by its depositor through `/withdrawals/*`, signed by the depositor's key; EVM only (not Bitcoin) | **documented, not observed** | docs.relay.link "Programmatic Withdrawals" |
| Relay has no testnet route from 46630 or to signet | **measured 2026-09-23** (`docs/v2/venue-bridge-policy.md` §2a.5) | so **no end-to-end rehearsal exists**; the first real conversion is the first test |
| Relay fills in minutes | **assumed** from `timeEstimate` (304 s today, 402 s in the operator's quote) | the monitor pages at 30 min (§6) |

### 0.2 The correction to `docs/v2/venue-bridge-policy.md` §2a

On 2026-09-23 this repository concluded that Relay's Bitcoin recipient "is not in the calldata; it is bound to `id` in Relay's
database", so a relayer-supplied quote could substitute its own recipient, and a safe use needed the canister to fetch the quote
itself (a non-replicated outcall trusting one replica). **That is superseded.** Since then Relay publishes the full order in
`protocol.v2.orderData`, and `id = hashStruct(Order)`. Recomputing the hash binds the deposit we sign to the recipient, the
minimum out, the currency, the chain, the deadline and the refund addresses we inspected. Relay's settlement pays according to
the order whose hash is the deposit id (the oracle checks the fill against it). So:

- **No HTTPS outcall to Relay is needed**, and none is made: the runner fetches the quote and hands it in; the canister trusts
  nothing in it that it does not recompute or check against its own reads.
- `relayquote::judge` (the 2026-09-23 judge, which trusted `details.recipient`) is replaced by `relayconv::judge` (§3.1).

## 1. The flow

One frog, one conversion, **one at a time** (contract and canister both enforce it).

1. **Want.** A graduation (or public mints) raises the module's net ETH-backed units past one item plus the buffer; the canister's
   trigger (`floorsink.rs` → `itemsink::may_start`) starts a frog purchase `Wanted { cap_wei }` for the module, `cap_wei` =
   `maxItemCost()` (the accepted floor + 3 %, never more than one unit's backing plus the revenue and surplus pots), capped by
   the per-purchase maximum. Unchanged.
2. **Advance** (new, contract). If the funding is Relay (the default, §4), the canister sends `advance(cap_wei)` to the module —
   custodian-only, one EVM transaction like every other. The module moves one convertible unit **into transit** and pays
   `cap_wei` to `payee` (the canister's `evm_address`). Accounting: §2.
3. **Ask.** The canister's conversion record says `Funded`, with the exact ask: `amount = advanced − GAS_KEEP_WEI` (0.0002 ETH
   kept for this purchase's own gas), `user = refundTo = evm_address`, `recipient = float_address`, 4663 → 8253038, native →
   native, `EXACT_INPUT`.
4. **Quote** (runner). The runner (ops/) reads `frog_conversions()`, calls `POST https://api.relay.link/quote` with exactly the
   ask, pre-judges the answer with the same rules in TypeScript, and hands the raw JSON in: `relay_quote(frog, json)`. Then it
   calls `tick` at once.
5. **Judge and deposit** (canister). At the next tick the canister judges the quote (§3.1) against its own reads — the
   reference rate (`oracle::read_rate`, the canister's own replicated price reads), the depository's `allocator()` at the pin,
   its balance — and, if every rule passes, signs `depositNative(evm_address, orderId)` with `value = amount` and sends it
   through the existing one-transaction path (`frogflow::send_value`: nonce, EIP-1559 fee, estimate, bytes saved before the
   send, the same bytes re-sent until mined). The order id joins the used list **before** the send.
6. **Watch.** Mined: `Converting`, with the balance at that pin. Each tick:
   - **arrival**: the float's UTXOs (every explorer agreeing, `custodynet::utxos_of`) show an output that was not there at the
     deposit, worth at least the order's `minimumAmount`, three blocks deep with the block still the block → `Arrived`;
   - **refund**: our 4663 balance rose by at least half the deposit since the deposit mined → the ETH is back (§5.1);
   - **overdue**: past the order's deadline + 1 h with neither → `Stuck` (§5.3).
7. **Buy.** The frog is now fundable. The operator's offer tool builds the PSBT over the float's UTXOs (the arrival among them)
   and hands it in (`frog_offer`, unchanged). `frogflow::accept` judges it exactly as today, except that the most that may leave
   the float is **the sats that arrived** (not a cap re-derived from ETH), and the frog's recorded ETH cost is its **share of the
   advance**: `cost = ⌈advanced × spend / arrived⌉` (≤ `advanced`).
8. **Arrive.** Six deep and ord-verified → `Held` → `wrapDeposit` to the module (unchanged) → `settleAdvance(cost)` instead of
   `itemArrived(cost)`: the transit unit becomes item-backed (one wX burned from the module's own balance, as `itemArrived`
   does), the fill is recorded at `cost`, and no ETH moves. The sats the frog did not use stay in the float as **spare**, recorded
   (`spare_sats`) and shown on the reserves page (§2.4).

## 2. The contract: `FloorModule`

### 2.1 New state, functions and events

```solidity
uint256 public transitUnits;   // 0 or ONE: the unit whose backing left as an advance
uint256 public advanced;       // wei paid to payee for it
uint256 public advanceBasis;   // the backing that unit released (what itemArrived would have released)
uint64  public advancedAt;

function advance(uint256 amount) external;          // custodian: pay `amount` (≤ maxItemCost()) to payee; one unit in transit
function settleAdvance(uint256 cost) external;      // custodian: the frog is wrapped here; the transit unit becomes item-backed
function returnAdvance() external payable;          // custodian: the conversion failed; the ETH comes back, the unit is ETH-backed again

event Advanced(uint256 amount, uint256 released);
event AdvanceSettled(uint256 indexed item, uint256 advanced, uint256 cost);
event AdvanceReturned(uint256 returned, uint256 basis, uint256 loss);
error AdvanceOpen();
error NoAdvance();
```

### 2.2 The rules

- **`advance(amount)`**: `msg.sender == wrapper.custodian()`; `transitUnits == 0` (else `AdvanceOpen`: **one in flight**);
  `0 < amount ≤ maxItemCost()` (which is 0 without a whole convertible unit, without arrival room today, or with the floor stale
  or the keeper paused; `OverItemCap` otherwise). The money side is exactly `itemArrived`'s: `released = backing × ONE / c`
  (the last unit takes the dust); `amount ≤ released` → the rest is revenue; `amount > released` → the difference comes from
  revenue, then the surplus (never the reserve). **H11 counts here** (`_countArrival`: at most `maxItemsPerDay` advances plus
  arrivals in any 24 h), because this is when ETH leaves. Then `transitUnits = ONE`, `advanced = amount`,
  `advanceBasis = released`, and `amount` is sent to `payee`.
- **`settleAdvance(cost)`**: custodian; an advance is open (else `NoAdvance`); `0 < cost ≤ advanced`; the module holds one more
  wX than the graduations and floats it owes (the frog was wrapped to it: `NoWrapped` otherwise, as `itemArrived`). Then
  `transitUnits = 0`, `itemsArrived++`, the fill is recorded at `cost` net of the conversion allowance (skipped while the keeper
  is paused, as no conversion is priced during an incident), `moduleBurn(ONE)`. No ETH moves. **H11 is not counted again.**
- **`returnAdvance()`** (payable): custodian; an advance is open. `v = msg.value`; the unit goes back to the convertible pot:
  `v ≥ basis` → `backing += basis`, the rest is revenue; `v < basis` → `backing += v`, and the shortfall is made up from the
  revenue pot, then the surplus; whatever is still short is a **loss** (`AdvanceReturned(v, basis, loss)`), shared by the
  convertible units. The daily count is not given back.
- **Everything that divides by convertible units excludes the transit unit**: `netUnits() = E − R − transitUnits` (so
  `maxItemCost`, `betaConvertible`, the stress term and the public cap see it gone), the convertible tier's backing per unit in
  burns (`_tierPrices`, `_release`) and `beta()` divide by `E − R − transitUnits` (resp. `E − transitUnits`), and a burn may
  take at most `E − transitUnits` units (`NotEnoughBacked`). `ethBacked` (the wrapper's) still counts the transit unit — its wX
  is in somebody's wallet — so `maxEthBacked`, `hasRoomFor` and `_gradRoom` count it, which is conservative.

### 2.3 Why the books stay exact

- **The module's books** (`balance ≥ backing + reserve + surplus + revenue + gradEth + rentalEscrow + renterPool + owedTotal`,
  the invariant suites' `_books()`, the reconciler's check) hold across `advance`: the ETH that leaves is exactly `released`
  from `backing` plus/minus revenue/surplus moves, as in `itemArrived`.
- **No burn is paid from the transit unit's backing**, and no other unit's backing per unit moves: the pot loses `released`
  and one unit at the same time (the per-unit backing of the rest is unchanged, as at an arrival).
- **The supply identity** `totalSupply == held × ONE_ITEM + ethBacked` is untouched until `settleAdvance`, which is
  `itemArrived`'s wrap-then-burn.
- **The ledger's tiers**: `convertible + reserve + transit = ethBacked` (the indexer's invariant gains the transit term).

### 2.4 What the unit is backed by, in transit

| Where | Backed by | Who holds it | How long |
| --- | --- | --- | --- |
| `Funded` | ETH at `evm_address` | the canister (threshold ECDSA) | until the next ticks judge a quote |
| `Converting` | ETH in Relay's depository, then Relay's solver | **Relay** (§7) | ≈ 5 minutes measured; bounded by the order's deadline (7 days) |
| `Arrived` | sats in the float | the canister (threshold Schnorr) | until the frog is bought |
| `Held`…`Minted` | the frog in custody | the canister | until `settleAdvance` |
| after `settleAdvance` | the frog | custody | as every item |

The spare sats (arrived − spent) are not a unit's backing: they are the difference between the advance and the frog's cost,
kept in the float (the ETH equivalent `advanced − cost` appears on chain in `AdvanceSettled`). At stage 0 (3 frogs) that is at
most a few percent of three floors. They are reported on the reserves page and are the operator's decision (§9, open).

### 2.5 What does not change

`itemArrived` stays, for a frog paid from an operator float (§4, `float_first`); the graduation lane (`enqueue`, `fill`,
`release`, `cancel`), minting, the fills price, rentals and sales are untouched. The module still has no owner and no setter.

## 3. The canister

### 3.1 `relayconv::judge` — every rule, and why

Pure (`autobuyer/canister-rs/src/relayconv.rs`). Input: the quote's JSON as the runner handed it in, and `Want` — every field
the canister's own:

```rust
pub struct Want<'a> {
    pub chain_id: u64,              // 4663 (the install's)
    pub our_evm: [u8; 20],          // evm_address
    pub float_address: &'a str,     // the float's bech32m address (from the float key and the installed network's HRP)
    pub amount_wei: u128,           // the ask: advanced − GAS_KEEP_WEI
    pub sats_per_eth: u64,          // oracle::read_rate(), read this tick
    pub max_fee_bps: u16,           // the install's (default 300, ceiling 500)
    pub allocator: [u8; 20],        // depository.allocator() read at this tick's pin
    pub now: u64,                   // the canister's clock
    pub handed_in_at: u64,          // when relay_quote stored it
    pub used: &'a [[u8; 32]],       // every order id this canister ever judged acceptable
}
```

Refused, each with the field named, unless **all** hold:

1. **One deposit transaction**: exactly one step, `id == "deposit"`, `kind == "transaction"`, exactly one item.
2. **Chain**: the item's `chainId == 4663`.
3. **Allowlisted `to`**: `to == RELAY_DEPOSITORY_4663` (`0x4cd0…bc31`, pinned in code, §0.1), and `allocator()` read at the
   pin `== RELAY_ALLOCATOR_4663` (`0x63C1…1b56`): a rotation by Relay's council is a change of the trust we accepted, so it stops
   conversions until a reviewed upgrade. `protocol.v2.paymentDetails.depository` must name the same address (a cross-check, never
   the source).
4. **Value**: `value == amount_wei` (the ask exactly), and `paymentDetails.amount` the same.
5. **Calldata**: exactly 68 bytes, `depositNative(address depositor, bytes32 id)`, `depositor == our_evm`. (`from`, if present,
   is ours.)
6. **The order is the order we inspect**: `id == hashStruct(normalize(orderData)) == protocol.v2.orderId`. EIP-712 struct hash
   with Relay's types (`Order`, `Input`, `InputPayment`, `InputRefund`, `Output`, `OutputPayment`, `Fee`), addresses encoded as
   Relay's settlement SDK does: EVM 20 bytes; Bitcoin bech32/bech32m → witness version byte ‖ program; Base58 → `0xff` ‖ payload
   (no checksum). Only the chains `robinhood` (EVM) and `bitcoin` may appear in addresses; anything else is refused (never
   guessed).
7. **Input**: one input; `payment.chainId == "robinhood"`, native currency, `payment.amount == value`.
8. **Refunds are ours**: at least one refund; every refund names `robinhood` → `our_evm` or `bitcoin` → `float_address`, native
   currencies; at least one on `robinhood`.
9. **Output**: `output.chainId == "bitcoin"`, exactly one payment, `recipient == float_address`, currency native BTC, **no calls**,
   no fees (`fees == []`).
10. **Min out** (the rate bound): `minimumAmount ≥ ⌊amount × sats_per_eth / 10^18⌋ × (10,000 − max_fee_bps) / 10,000`. With the
    default 3 %, today's quote (2.0 % below expected, fees ≈ 1 %) passes by ≈ 0.4 %; a solver pricing worse than 3 % under our
    reference rate is refused.
11. **Freshness**: `now − handed_in_at ≤ QUOTE_TTL_SECS` (300: the runner hands in and ticks at once) and the order's
    `output.deadline` and every refund's deadline lie in `[now + 1 day, now + 8 days]` (a quote made days ago, or an order that
    could expire before Relay acts, is refused). The price itself is re-checked at signing by rule 10 on this tick's rate.
12. **Replay**: `id` is not in `used` (every order id this canister ever accepted, kept in stable memory, never pruned).

Output: `ConvDeposit { to, value, data, order_id, min_sats, deadline, request_id }`.

### 3.2 State (stable, in `FrogState`)

```candid
type ConvAsk = record { amount_wei : nat; user : text; recipient : text; refund_to : text;
  origin_chain_id : nat64; destination_chain_id : nat64; origin_currency : text; destination_currency : text };
type CPhase = variant {
  NeedAdvance;                                               // the advance is the next EVM act
  Advancing;                                                 // advance(amount) is out
  Funded : record { wei : nat; since : nat64 };              // ETH at evm_address; the runner is asked for a quote
  Depositing : record { order_id : blob; value : nat; min_sats : nat64; deadline : nat64 };
  Converting : record { order_id : blob; value : nat; min_sats : nat64; deadline : nat64; since : nat64; balance_after : nat };
  Arrived : record { order_id : blob; sats : nat64 };
  Returning : record { wei : nat };                          // returnAdvance{value: wei} is the next / current EVM act
  Returned : record { wei : nat };
  Settled : record { cost : nat; spare_sats : nat64 };
  Stuck : record { why : text };
};
type ConvRec = record {
  frog : nat64; module : blob; advanced : nat; attempts : nat32; phase : CPhase;
  orders : vec blob;             // every order id accepted for this conversion, in order
  deposits : vec blob;           // every deposit transaction hash, in order
  arrival : opt record { outpoint : Outpoint; sats : nat64 };
  ask : opt ConvAsk;             // Some while Funded: exactly what the runner must quote
  note : text;                   // the last refusal or wait, in words
};
```

`FrogState.convs : opt vec ConvRec` (serde default: an older state decodes to none). At most one record is not finished
(`Returned`, `Settled`); `Stuck` is not finished and holds the line (nothing new starts).

### 3.3 Where it runs in the tick

- **assign** (step 5): a new frog for a Bitcoin floor sink, with Relay funding, creates its `ConvRec { NeedAdvance }`. A frog with
  an advance out is never released or failed by "the target no longer needs it": the module paid, the purchase is committed.
- **the conversion pass** (new, after step 4): `Converting` → arrival / refund / overdue; `Funded` with a handed-in quote → judge,
  and on success mark it for the EVM act.
- **evm_act** (step 7), after the reports and mints: `NeedAdvance` → `advance(amount)`; a judged quote → the deposit;
  `Returning` → `returnAdvance{value}`. One EVM transaction canister-wide, as always.
- **resolve/mined**: `Advancing` mined → `Funded` (reverted → `NeedAdvance` again, the estimate guards the retry; the frog fails
  if the module no longer wants it); the deposit mined → `Converting` (reverted → `Funded`, attempt counted); `returnAdvance`
  mined → `Returned`, and the frog `Failed` ("the conversion was returned: nothing bought").
- **accept** (step 4): a frog whose conversion is not `Arrived` takes no offer ("waits for its conversion"); once `Arrived`, the
  cap on what leaves the float is the arrived sats.
- **the report**: a frog with a settled-able conversion reports `settleAdvance(cost)`; mined → frog `Settled`, conversion
  `Settled { cost, spare_sats }`.
- **floorsink** reads `transitUnits()` too, and the demand counts it (`wanted(net + transit, buffer)`), so the frog on its way
  still has its unit.

### 3.4 Endpoints

- `relay_quote(frog : nat64, quote_json : text) -> Result<text, text>` — operator; stored on the heap for that frog only while its
  conversion is `Funded` (≤ 64 KiB); judged at the next tick. A refused quote costs nothing but the call.
- `relay_give_back(frog : nat64) -> Result<text, text>` — operator; `Funded` → `Returning`: the ETH goes back to the module
  instead of a new quote (the operator's exit when Relay will not quote or fill). It can only send ETH **to the module**.
- `frog_conversions() -> vec ConvRec` — query, for the runner, the monitor and the reserves page.

### 3.5 Install argument

`frog_funding : opt record { max_fee_bps : nat16; float_first : bool }` — operational, taken from every upgrade's argument (so
changing it is a published, timelocked act the watcher decodes). `null` = **Relay, 300 bps, no float** (the default, and Phase 1's
value). `max_fee_bps` must be in `[50, 500]`. `float_first = true` keeps an optional operator float: when the float's confirmed,
ord-clean balance covers the frog's cap, the frog is bought from it and reimbursed by `itemArrived` as today; otherwise Relay.

## 4. Limits (stage 0)

| Bound | Where | Value |
| --- | --- | --- |
| One advance in flight | contract (`AdvanceOpen`) and canister (one unfinished `ConvRec`) | 1 |
| Per purchase | contract: `maxItemCost()` (floor + 3 %, ≤ one unit's backing + revenue + surplus); canister: `policy::MAX_PURCHASE_WEI` (0.5 ETH) and the collection limit | ≈ 0.65 ETH at a 0.63 ETH floor |
| Per day | contract H11 `maxItemsPerDay` (3, counted at the advance); canister H3 value limit (10 ETH/day, counted at the advance) | 3 |
| In custody | canister H2: the advance **reserves** the item's room at `cap_wei` when it is sent, settled to the item at arrival | 3 items / 10 ETH per collection |
| Relay fee | `max_fee_bps` (min-out ≥ rate × (1 − 3 %)) | 3 % |
| Retries | `MAX_ATTEMPTS` quotes per advance, then the ETH goes back | 3 |
| Gas | `GAS_KEEP_WEI` of each advance stays at `evm_address` for that purchase's gas | 0.0002 ETH |

**At most one advance (≤ ≈ 0.65 ETH at today's floor) is outside the module at any time, and at most three a day.** That is
the whole new exposure.

## 5. Failure modes

### 5.1 Relay refunds (it could not fill; a regenerated quote fell under the minimum; screening)

The refund lands at `evm_address` (the order's refund recipient, which rule 8 made ours), less Relay's refund gas. The canister
sees its balance rise by ≥ 50 % of the deposit since the deposit mined, and moves to `Funded { wei = min(balance − GAS_KEEP_WEI,
advanced − GAS_KEEP_WEI) }` with `attempts + 1`: a new quote, a new order id, a new deposit. After `MAX_ATTEMPTS` (3) the
conversion goes `Returning` and `returnAdvance{value: wei}` gives the ETH back; the module takes any shortfall from its revenue
and surplus pots, else records a loss (§2.2). The frog fails ("nothing bought").

Honest limit: the canister cannot tell Relay's refund from a stranger's gift of the same size. A gift taken for a refund leads to
one extra conversion **paid by the gift**; if the original order then also fills, two arrivals reach the float and the second is
spare. No module ETH is lost either way.

### 5.2 Partial fills and price moves

Relay's settlement pays at least the order's `minimumAmount` or refunds (documented); a regenerated quote below it is refunded.
The canister only counts an arrival of at least `minimumAmount`; a smaller output to the float is ignored (and shown). The
budget buffer is the conversion allowance already in `maxItemCost` (floor + 3 %) against the Relay fee bound (3 %) and today's
real cost (≈ 1 %). An offer over the arrived sats cannot be signed (rule: the float may lose at most the arrived sats); if frogs
move above that, the frog waits with its sats (§5.4).

### 5.3 Nothing happens (no fill, no refund)

Past the order's deadline + 1 h with no arrival and no refund: `Stuck { why }`, the monitor pages SEV1, and nothing new starts.
Relay documents a depositor-signed withdrawal for stuck EVM deposits (`/withdrawals/*`); the canister holds the depositor's key
but has **no endpoint that signs Relay's withdrawal message** — recovery is by Relay's support, or a reviewed upgrade adding a
narrowly-scoped signer (§9). The exposure is one advance.

### 5.4 The BTC arrives and no frog can be bought

The frog stays `Wanted` with its sats in the float; the unit stays in transit (burns cannot reach it; it is backed by sats). The
monitor shows `Arrived` age. The operator hands in an offer within the arrived sats, or (open, §9) the sats wait for a cheaper
frog. There is no automatic Bitcoin → ETH reverse conversion in this change.

### 5.5 Timeouts and reorgs

- The deposit transaction: the existing path (the same saved bytes re-sent; a nonce used but the hash unseen waits).
- A 4663 reorg that drops the deposit: `evm_seen` sees it not mined; the bytes are sent again (same nonce, same order id — the
  order is still valid until its deadline).
- A Bitcoin reorg under the arrival: the arrival needs three blocks with the block hash re-checked (`frogflow::depth`); the buy
  then needs its own six. A buy whose input vanished is the existing "listing vanished / never confirmed" path.

## 6. Monitoring and the reserves page

- **Watcher, 4663 events** (`monitor/src/evm.mjs`): `Advanced(amount, …)` must match a conversion record for that module with
  `advanced == amount` (SEV1 "AN ADVANCE NO CONVERSION BACKS" otherwise); `AdvanceSettled(item, advanced, cost)` a record whose
  frog is minted or settled with that cost; `AdvanceReturned` a record `Returning`/`Returned`.
- **Conversion states** (`frog_conversions()`, polled with the other records): `Converting` for more than 30 min → SEV2
  "late"; `Stuck` → SEV1; `Funded` for more than 2 h (the runner is not quoting) → SEV2; `Arrived` for more than 24 h (no
  frog bought) → SEV2; `Returned` → INFO, with the loss if any; every refund (attempt > 1) → INFO.
- **The Bitcoin canary**: a conversion's arrival is a known inbound to the float (not "unexplained").
- **Reconciler** (`ops/src/backing.ts`): reads `transitUnits`, `advanced`, `advancedAt`; the page says "1 unit in transit: X ETH
  advanced at T"; an advance older than 48 h is **red**.
- **Indexer / reserves page**: `convertible + reserve + transit = ethBacked`; the ledger gains the row "ETH-backed, in transit
  (being converted to a frog)".

## 7. Trust and threat model

**New trust: Relay's solver and settlement, for about five minutes per purchase, for at most one advance at a time.** Between
the deposit and the fill, the advance is in Relay's depository (released only by Relay's allocator, an MPC key governed by
Relay's security council multisig) and the solver's word. Relay's code, oracle and allocator could fail to fill and fail to
refund. The bound: one advance (≤ floor + 3 %) at a time, three a day, the monitor paging at 30 minutes.

| Adversary | Can | Cannot |
| --- | --- | --- |
| **The runner** (hot operator identity) | hand in any quote; withhold quotes (the conversion waits, the monitor pages at 2 h); pick among Relay's valid quotes within the 3 % bound; call `relay_give_back` (the ETH goes back to the module) | name another recipient, refund address, chain, currency, amount, depository or calldata (rules 3–9 recompute or compare every one against the canister's own values); replay an order (rule 12); deposit twice for one advance (one `Funded` → one `Depositing`; the id is recorded before the send); take ETH anywhere but Relay's pinned depository or the module |
| **Relay's API** (a lying or compromised endpoint) | serve a bad quote | pass the judge with an order whose hash does not match, or whose fields are not ours; name a depository that is not the pinned one. **No outcall is made to it**: nothing depends on one replica's answer |
| **Relay's solver / allocator** | keep the advance (fail to fill and to refund); fill late | take more than one advance; reach module ETH beyond it |
| **A stranger** | send ETH to `evm_address` (taken for a refund: an extra conversion paid by the gift, §5.1); send sats to the float (spare; an output under the minimum is ignored) | make the canister deposit module ETH it does not hold |
| **The custodian key** (a compromised canister) | exactly what it can today through `itemArrived`: take up to `maxItemCost()` a day-count at a time — `advance` gives no new power, only earlier timing | exceed `maxItemCost`, the daily count, or one open advance |
| **Reorgs** | undo a deposit (re-sent, same order) or an arrival (three blocks, hash re-checked) | make one advance count twice |

**Double spend of the budget**: the contract pays one advance per transit unit and refuses a second while one is open; the
canister converts at most `advanced − GAS_KEEP_WEI` per attempt, from `Funded` only, and after a refund at most what came back.

**Screening**: Relay screens addresses (`BLOCKED_WALLET_ADDRESS` seen for well-known ones). If Relay blocks the canister's
addresses, quotes are refused (no deposit is made) or a deposit is refunded; after three attempts the ETH goes back.

## 8. What changes, file by file

| Area | Change |
| --- | --- |
| `src/floor/FloorModule.sol` | §2; tests in `test/floor/` (every rule, the books, the tiers, the caps, the invariant handler) |
| canister | `relayconv.rs` (pure judge, EIP-712, address encoding), `FrogState.convs`, the flow in `frogflow.rs`, endpoints in `frognet.rs`, `floorsink.rs` (transit read), `canister.rs` (`frog_funding`), `.did`; tests for every judge rule and each failure path |
| `ops/` | `src/relay.ts` (ask → quote → pre-judge → `relay_quote`), the runner calls it after each tick; `backing.ts` reads the transit |
| `monitor/` | the three events, the conversion alerts, the canary's known inbound, the `frog_funding` field in the watcher's type table |
| `indexer/`, `web-rook/` | the transit tier; the Trust page's Relay section |
| docs | this; `phase1-scope.md` (O7: no float; the canister's gas), `canister-mainnet.md` (install argument), `services-v1.md` (runner config) |

## 9. Open questions for the operator

1. **Spare sats.** The float keeps what each frog did not use (≈ 1–2 % of a floor). Leave them (they cover a later frog's fee), or
   sweep them somewhere by a future, reviewed path?
2. **Stuck deposits.** Accept that recovery of a never-filled, never-refunded deposit is Relay support (documented path), or add a
   reviewed canister endpoint that signs only Relay's withdrawal message with recipient = `evm_address`?
3. **The allocator pin.** A Relay rotation stops conversions until an upgrade (72 h). Keep that (safer), or pin only the
   depository?
4. **`max_fee_bps` = 3 %.** Today's cost is ≈ 1 % at 0.16 ETH, ≈ 2 % at 0.02 ETH (the operator's measurement). 3 % leaves room
   for a smaller advance or a worse day; lower it to 2 % if frogs stay above 0.1 ETH.
5. **Gas.** The canister's EVM address needs ≈ 0.002 ETH once for its first transactions (not a float: L2 gas only); each advance
   keeps 0.0002 ETH for its own gas afterwards.

## 10. As built (2026-09-27)

What the code does, where it differs from the text above, and what was run.

- **Contract** (`src/floor/FloorModule.sol`): §2 exactly. `advance(uint256)` `0x71d6ddd6`, `settleAdvance(uint256)` `0x1597691c`,
  `returnAdvance()` `0xf9044bf0`, `transitUnits()` `0x12bc60d6`, `advanced()` `0xa0dd2c03`, `advanceBasis()` `0xb9ddd086`,
  `advancedAt()` `0xb578e434`. The transit unit is left out through one helper (`_liveUnits()` = `ethBacked − transitUnits`) used
  by `netUnits`, `burn`, `_tierPrices`, `beta` and `burnCapacity`. Runtime 30,224 bytes (4663's limit 98,304). Tests:
  `test/floor/Advance.t.sol` (every rule, the pots, the tiers, burns, H11 at the advance only, a fuzz of advance then settle or
  return) and the module's invariant handler now advances, settles and returns.
- **Canister**: `relayconv.rs` (the judge, the EIP-712 hash, Relay's address encoding, the money helpers), `convflow.rs` (the
  steps in the frog tick), and the hooks in `frogflow.rs`, `floorsink.rs` (the eleventh read, `transitUnits()`), `frognet.rs`
  (`relay_quote`, `relay_give_back`, `frog_conversions`) and `canister.rs` (`frog_funding`). Differences from §3:
  - Relay's native-BTC id `bc1qqq…mql8k8` is **not a valid segwit address** (a 19-byte "program"); Relay's SDK encodes it as
    `0x00` ‖ 19 zero bytes. The canister's encoder follows the SDK (bech32/bech32m regrouping without segwit's length rules), and
    refuses Base58 addresses instead of guessing.
  - `ConvRec` carries three more optional fields than §3.2: `float_before` (the float's outpoints just before the deposit: an
    arrival is an output not among them), `balance_before`, and `last` (the deposit being watched, kept through `Stuck` so a
    late fill or refund is still seen).
  - H2's room and H3's value are counted **at the advance** (when module ETH leaves); the frog's buy then counts neither again.
  - A frog with an advance is never released as "no longer needed", and its reservation is never swept as stale.
  - `relay_give_back` works from `Funded` or `Stuck`; from `Stuck` it returns only what is at our address.
- **Runner** (`ops/src/relay.ts`): reads `frog_conversions()`, quotes exactly the ask, pre-judges rules 1–9 and 11 (rule 10 needs
  the canister's own rate), hands the raw JSON in, and ticks again at once. `RUNNER_RELAY` (default on), `RELAY_API_URL`.
- **Monitor**: the three events matched against `frog_conversions()`; the conversion alerts of §6; the arrival as a known float
  inbound; `frog_funding` decoded and diffed. **Reconciler**: the transit unit and a 48 h stale-advance red
  (`RECONCILE_ADVANCE_STALE_S`). **Indexer / site**: `E = C + R + T`; the ledger's "in transit" row; the Trust page's Relay
  section.
- **Rehearsal**: `scripts/golive-dryrun.sh stack` runs a new phase `advance` after the launch: the custodian advances
  `maxItemCost()` on the fork, the unit is in transit, the books hold, then `returnAdvance` restores the module exactly.
- **Results (2026-09-27, this branch):** `forge test --no-match-path "test/fork/*"` 1,061 passed, 0 failed (120 suites; the
  FloorModule invariants included); canister `cargo test` 506 passed, 0 failed (333 unit + 173 integration, 5 of them this
  flow end to end in `tests/frogs.rs`); timelock 15 passed; ops 38, monitor 143, keeper 150, indexer 135, web-rook 69, all
  passing; `scripts/golive-dryrun.sh stack` **ALL PASS**, including the advance phase on the fork (advanced 0.6178 ETH to the
  custodian, one unit in transit, returned, the module whole). Main's `ops-config` had left the dry run expecting a refusal at
  99,999 sats after lowering the fee-pool minimum to 20,000; the expectation now tests 19,999.
- **Reproducible build**, twice (the second with `--no-cache`), identical:
  `buyer.wasm` `905c6fc42f57d9e4653c164fc4145642fb43386d3d57f9602a3b951bca069b2d`,
  `timelock.wasm` `b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41` (unchanged),
  `timelock-rehearsal.wasm` `2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4` (unchanged). The buyer's hash
  replaces the one in `docs/audit/2026-09-26/review4/fixes-golive-lows.md` for any install of this branch.
