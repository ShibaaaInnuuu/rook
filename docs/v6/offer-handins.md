# Permissionless offer hand-ins with a reward (v6)

Written 2026-09-23 on branch `btc-offer-handins` (from `feb928b`), **before its code**. The Bitcoin analogue of PunkStrategy's
permissionless `buyPunk` trigger: anyone — a bot, a seller, a stranger — may hand the canister a fillable listing of a frog in
the collection; the canister verifies it exactly as it verifies the operator's offers today, buys the cheapest valid one when a
sink wants an item, and once the purchase is six blocks deep the hand-in earns a fixed reward. Nothing here is deployed, and no
transaction was sent to any public chain.

This branch touches: `src/handin/` and `test/handin/` (new Solidity), `docs/v6/` (this file), and in the canister the new
modules `handin.rs`, `handinflow.rs`, `handinnet.rs` with wiring listed under "Shared-file edits". It does not touch
`src/floor/`, `src/curve/`, `src/pools/`, the wrappers, `docs/v5/` or `src/homechain/`.

## 0. Why

Today the canister's only source of marketplace offers for its own purchases (queue and curve graduations, desk requests,
Floor Module buys) is the operator: `frog_offer(id, inscription, psbt, to_sign, proof)` (docs/v3/curve.md, "Buying one frog").
The listing sources behind that are thin — ordinals.com's JSON API is off and Magic Eden answered 503 on the last rehearsal —
and every purchase waits on one person. The canister's checks were never about who hands the offer in: `frogbuy::judge`
believes only ord, the explorers and the PSBT's own bytes. So the hand-in can be opened to anyone, and the operator becomes
the fallback rather than the path.

The decision (the user's, 2026-09-23): **permissionless hand-ins, with a bond against spam and a fixed reward on a confirmed
purchase.**

## 1. The flow

```
  bot                          Robinhood Chain (4663)                 canister                         Bitcoin
   │  submit(hash, ask, id) {bond}  ┌───────────────┐                    │                                │
   ├──────────────────────────────►│  OfferBoard    │                    │                                │
   │                                │  (per wrapper) │◄── openIds / ─────┤ tick: read at the pin          │
   │  submit_offer(psbt, to_sign,   └───────────────┘    submissionOf    │                                │
   │     proof)  [update call]                                           │                                │
   ├────────────────────────────────────────────────────────────────────►│ sha256(psbt) == hash? keep     │
   │                                                                     │ a sink wants an item:          │
   │                                                                     │  cheapest open hand-in with    │
   │                                                                     │  bytes, ask ≤ cap → judge      │
   │                                                                     ├── sign float inputs, save ────►│
   │                                                                     │◄── six deep, ord: custody ─────┤
   │                                ┌───────────────┐   claim(id)        │                                │
   │◄── withdraw(): bond + reward ──┤  OfferBoard    │◄───────────────────┤ (after a fresh depth read)     │
   │                                └───────────────┘   slash(id)        │ invalid, and slashDelay past   │
   │                                                     refund(id)      │ (submitter, after expiry)      │
```

1. **On chain, the commitment.** `OfferBoard.submit(offerHash, askSats, inscriptionId)` with exactly `bond` wei attached.
   `offerHash = sha256(psbt bytes)`; `askSats` is what the submitter says will leave the canister's float, fee included (the
   judge's `spend`); `inscriptionId` is the frog. The board records the submitter and both clocks, and gives the submission an
   `id`. One open submission per hash; at most `maxOpenPerSubmitter` per address and `maxOpen` in all.
2. **To the canister, the bytes.** `submit_offer(psbt_hex, to_sign, proof_hex)` — an update call **open to anyone** (design
   (a) of the brief; §4 says why not a relay). The canister hashes the bytes and keeps them only if the hash is an open
   submission it has read, or, for a bounded number, until its next read shows one. Whoever delivers the bytes does not
   matter: the on-chain record names who is paid. Size- and rate-limited (§3.3).
3. **At the tick**, the board is read at the tick's pin (`openIds`, then `submissionOf` for ids not seen before, bounded per
   tick). When a frog record is `Wanted` and the operator handed in nothing for it, the canister takes the **cheapest** open
   hand-in that has bytes, no verdict yet, `askSats ≤ cap_sats`, and more than a purchase's latency left before its expiry —
   and judges it with `frogflow::accept`, i.e. every check of docs/v3/curve.md "Buying one frog": the membership proof, ord's
   location and contents, the explorers' account of the seller's output, `frogbuy::judge` (sat flow, one custody output, no
   `OP_RETURN`, no script paths, fee bound), the policy limits, the custody caps (H2) and the day's value limit (H3). Passed:
   signed with the float key, saved, broadcast; the hand-in's verdict is `Bought { frog }`. Refused: the verdict is
   `Invalid` (a fault of the bytes, §2.2), `Gone` (the listing is no longer fillable: sold, or under the policy minimum) or a
   wait (a read failed; tried again next tick).
4. **Once the frog is `Held`** (six deep on the lowest tip, the block still the block, ord showing custody holding exactly it),
   the canister re-reads the depth immediately before sending `claim(id)`: the board credits the submitter `bond + reward`
   (§2.1), to be pulled with `withdraw()`. A frog whose purchase ends `Stranded` (custody holds something other than exactly the
   frog) or `Failed` (nothing of ours was spent) earns nothing.
5. **An invalid hand-in is slashed** by `slash(id)`, sent only once `slashDelay` has passed on both clocks since the
   submission (so a canister slashing wrongly is visible for that long before any bond moves — the guardian's pause stops it,
   H1) and only while the submission is open. Its bond goes to the board's reward pool.
6. **A valid hand-in nobody bought** (a cheaper one won; the sink stopped wanting; its listing was sold elsewhere) gets its bond
   back: `refund(id)` by the submitter once `expiry` has passed on both clocks, or by the custodian at any time. No reward.

The operator's `frog_offer` keeps working and is tried **first** for a wanted frog; the operator's `frog_proof` and
`refuse_deposit` are unchanged.

## 2. `OfferBoard` (src/handin/OfferBoard.sol)

One per wrapped collection, constructed with the wrapper and its parameters, all immutable:

| parameter | suggested (4663) | what it is |
|---|---|---|
| `wrapper` | the collection's `Wrapper` | `wrapper.custodian()` is the only address that may claim or slash |
| `bond` | 0.01 ETH | attached to every `submit`, exactly |
| `reward` | 0.01 ETH | paid on a confirmed purchase, on top of the bond back |
| `slashDelay` | 1 hour | no slash before this has passed on both clocks since the submission |
| `expiry` | 24 hours | after this on both clocks the submitter may take the bond back |
| `rewardDayCap` | 0.2 ETH | the most reward paid in one day (both clocks roll the day) |
| `maxOpen` | 64 | open submissions in all: bounds the canister's read |
| `maxOpenPerSubmitter` | 4 | open submissions per address |
| `parentBlockSeconds` | 12 | `lib/Wait.sol`'s block time, as every waiting contract here |

### 2.1 The ABI the canister assumes

```solidity
contract OfferBoard {
    enum Status { None, Open, Claimed, Slashed, Refunded }
    struct Submission {
        address submitter; uint64 at; uint64 atBlock; Status status;
        uint256 bond; uint256 askSats; bytes32 offerHash; string inscriptionId;
    }
    function submit(bytes32 offerHash, uint256 askSats, string calldata inscriptionId) external payable returns (uint256 id);
    function claim(uint256 id) external returns (uint256 paid);     // custodian only; Open → Claimed; credits bond + paid
    function slash(uint256 id) external;                            // custodian only; Open, slashable → Slashed; bond → pool
    function refund(uint256 id) external;                           // submitter after expiry, or custodian; Open → Refunded
    function withdraw() external returns (uint256);                 // pull what claim / refund credited
    function fund() external payable;                               // anyone: the reward pool (the collection's fees)
    function openIds(uint256 start, uint256 max) external view returns (uint256[] memory);
    function openCount() external view returns (uint256);
    function submissionOf(uint256 id) external view returns (Submission memory);
    function params() external view returns (uint256 bond, uint256 reward, uint256 slashDelay, uint256 expiry,
                                             uint256 rewardDayCap, uint256 maxOpen);
    function slashable(uint256 id) external view returns (bool);    // Open and slashDelay past on both clocks
    function expired(uint256 id) external view returns (bool);      // expiry past on both clocks
    function wrapper() external view returns (address);
    function custodian() external view returns (address);           // wrapper.custodian()
    function rewardPool() external view returns (uint256);
    function owed(address) external view returns (uint256);
}
```

Selectors, pinned in `handin.rs` and checked against `cast sig` in its tests: `submit` `7f3b74cc`, `claim` `379607f5`,
`slash` `45bc4d10`, `refund` `278ecde1`, `openIds` `c3f95de6`, `openCount` `cfaf380e`, `submissionOf` `74505d71`, `params`
`cff0ab96`, `wrapper` `ac210cc7`, `slashable` `4bff9a80`, `expired` `ba065e1f`.

**Money.** `claim` pays `paid = min(reward, rewardPool, rewardDayCap − paidToday)`: the pool is never overdrawn and a dry pool
never blocks a claim (the bond still comes back; a bot can read `rewardPool()` before it bothers). Payments are **pull**
(`owed[submitter]`, `withdraw()`), so a submitter that refuses ETH cannot make the canister's claim revert forever. The
books: `address(this).balance == bonded + rewardPool + owedTotal` at all times; a slashed bond moves from `bonded` to
`rewardPool`; a refund or claim moves the bond from `bonded` to `owedTotal`.

**Invariants** (`test/handin/OfferBoard.invariant.t.sol`): the balance identity above; `Σ rewards paid ≤ Σ funded + Σ
slashed` (rewards never exceed the budget); every id is paid at most once (status is a one-way `Open → {Claimed, Slashed,
Refunded}`); bonds are conserved (`bonded == Σ bond of open submissions`); `paidToday ≤ rewardDayCap`; `openCount ==
Σ openBy`.

### 2.2 What is invalid (slashed), what is merely not bought

| the hand-in | verdict | bond |
|---|---|---|
| bytes never delivered (a hash-only submission) by `slashDelay` | Invalid | slashed |
| the PSBT does not parse; a final witness cannot be read | Invalid | slashed |
| the proof does not verify against the listing root; ord does not know the inscription | Invalid | slashed |
| the seller's output carries runes or a second inscription | Invalid | slashed |
| `frogbuy::judge` refuses: an input to sign that is not the float's, a custody input, no or two seller inputs, the seller's value wrong, an `OP_RETURN`, the inscribed sat not landing on the custody output, foreign sats in it, two custody outputs, dust, a script-path witness, fee over the bound | Invalid | slashed |
| the spend exceeds `askSats` (it was tried because `askSats ≤ cap`) | Invalid | slashed |
| over the cap or the collection's limit while the spend is within `askSats` (the cap or the rate moved between the choice and the judgement) | none: tried again | — |
| the seller's output is spent (sold elsewhere, or a dead listing handed in) | Gone | back at expiry |
| the cost is under the policy's minimum purchase (`MIN_PURCHASE_WEI`) | Gone | back at expiry |
| `askSats` above the cap of every sink that wants an item | not tried | back at expiry |
| valid, but a cheaper valid hand-in was bought | not tried | back at expiry |
| ours was broadcast and the listing vanished (the seller's outpoint spent by another transaction) | Gone (the frog record fails; a new record wants again) | back at expiry |
| a read failed (ord, an explorer, the rate) | none: tried again | — |

The line is: **slashed if the fault is in the bytes the submitter committed to**, whatever happens later on the market. A
dead listing costs the submitter only the bond's time and the canister only one round of reads (a `Gone` hand-in is never
tried again).

## 3. Anti-spam and anti-grief

### 3.1 Bond sizing

The bond must exceed what a bad submission costs the system, so that slashing is net positive and spam is net negative:

- **the canister's cost to judge one** — one board read at the pin (a handful of `eth_call`s), one `locate`/`contents`/
  `output` round to ord and every explorer, and one `slash` transaction (L2 gas, paid by the canister's EVM address). On 4663 a
  transaction is well under 0.001 ETH; the outcalls are cycles, on the order of 0.001 ETH-equivalent for the round;
- **the slot** — a submission occupies one of `maxOpen` for up to `slashDelay` (if slashed) or `expiry` (if merely useless).

0.01 ETH covers both by an order of magnitude and stays below what any bot earns on one honest hand-in (§5), so the bond is
not a barrier to the behaviour we want. Every parameter is a constructor argument; another chain or collection sets its own.

### 3.2 Rate limits, on chain

`maxOpenPerSubmitter` (4) bounds one address; `maxOpen` (64) bounds everyone, so the canister's per-tick read is bounded
whatever happens (`MAX_SUBMISSIONS_READ_PER_TICK = 16` new records a tick, the rest next tick). Neither is a Sybil defence —
addresses are free — the bond is: 64 slots cost 0.64 ETH to fill, and every hash-only slot is slashed after an hour, so a
full board of junk is 0.64 ETH into the reward pool every hour. A board full of *valid but overpriced* junk (§3.4) is refunded
at expiry and merely costs the attacker the capital's time, while an honest submitter has to wait for a slot: the residual
DoS, bounded to 24 hours per 0.64 ETH, and lifted by the custodian's any-time `refund`, which the canister uses on
`Gone` verdicts so their slots free at once.

### 3.3 Rate limits, on the canister (`submit_offer`)

An update call open to anyone costs the canister cycles for every byte it receives, before it can refuse. So:

- **size**: at most `MAX_PSBT_BYTES` (8,192) of PSBT, at most 32 proof hashes, at most 64 `to_sign` indexes; anything
  larger is refused before parsing;
- **matched first**: bytes whose sha256 is an open submission the canister has read are kept (at most one set per hash, the
  newest wins), up to `maxOpen`;
- **unmatched** bytes (the bot sent them before the canister's next tick read its submission) go to a small buffer
  (`MAX_UNMATCHED` = 32, first in first out, dropped after `UNMATCHED_TTL` = 2 hours), matched at the next read;
- **per caller**: at most `SUBMITS_PER_HOUR` (6) accepted per principal, anonymous callers refused; the caller is not who is
  paid, so an honest bot uses any principal it likes;
- what is refused costs the caller nothing and the canister an ingress message. The IC's per-canister ingress limits and
  cycles balance are the last line; `inspect_message` (refusing before execution) is not built here (§7).

Bytes are **stable** (the tenth element of the upgrade tuple, with the rest of the hand-in state): a bot that paid a bond is
not asked to notice an upgrade and resend.

### 3.4 Cheapest first, and the overpriced-but-under-cap hand-in

A bot could hand in a listing (its own, say) priced just under the sink's cap and collect both the price and the reward. Two
things bound this:

- **the canister buys the cheapest valid open hand-in** for a wanted frog, by `askSats` (ties by age). An overpriced hand-in is
  bought only when nothing cheaper is on the board — and if nothing cheaper is on the board and the price is under the cap, it
  is exactly the purchase the sink is funded to make;
- **the cap is not the bot's**: it is the sink's per-item cap (`itemCap` from the keeper's floor for a queue, `maxItemCost`
  for a Floor Module, the request's `amount` at the desk), less what ours on the way already commit (`itemsink::per_item_cap`),
  and never over the policy's `MAX_PURCHASE_WEI` or the collection's own limit. The reward is **fixed**, not a share of the
  price, so a submitter gains nothing by inflating the price beyond selling its own frog at the cap — which any seller could
  do today by listing at the cap and waiting for the operator.

So the residual is "the system pays up to its cap when the floor is thin", which the floor feed and the keeper's cap exist to
bound (docs/v5/floor-feed.md §1), not this module.

### 3.5 The rest of the threat list

| threat | answer |
|---|---|
| a lying `askSats` (low, to be picked first; the PSBT spends more) | the judge's `spend > askSats` is Invalid: slashed |
| the same PSBT handed in twice | one open submission per hash; the second `submit` reverts (`Duplicate`) |
| two hand-ins for the same frog at different prices | both valid; the cheapest is bought; the other is `Gone` at the next try (the seller's outpoint is spent) and refunded |
| the submitter front-runs the canister's broadcast with another spend of the seller's outpoint (it holds the seller key) | "listing vanished": nothing of ours is spent, the record fails, a new one wants again; the hand-in is `Gone`, not slashed (indistinguishable from an honest sale); the attacker paid gas and a bond's time for nothing |
| a PSBT that spends float inputs into anything but change | `judge`: every float input's sats not in the custody output must return to the float's script within `spend ≤ cap`; a foreign output is paid only from `spend` |
| a PSBT with a runestone, or a reveal | no `OP_RETURN` at all; no script-path witness |
| the submitter refunds while the canister's purchase is confirming | `refund` needs `expiry` (24 h) past; a purchase is started only with more than `PURCHASE_LATENCY + MINT_MARGIN + CAP_MARGIN` (≈4.25 h) left before the hand-in's expiry, as for a sink's deadline |
| a reorganisation after the claim | the claim goes only after a **fresh** depth read shows six on the lowest tip with the block still the block; a deeper reorganisation than six is the system's standing assumption (docs/v3/sweep.md 1a) |
| the canister slashes a valid hand-in (a bug, a forged read) | `slashDelay` on both clocks before any slash: the verdicts are a public query (`handin_verdicts`), the bytes are public (the submitter has them), and the guardian's pause (H1) stops the `slash` step like every other outbound act; bonds are small by design |
| draining the reward pool | a reward needs a confirmed purchase, and purchases are demand-driven (a sink must want an item, one purchase at a time, ≈3 h each) and capped by stage caps (docs/v5/launch-gates.md G7: 3 mints a day a collection at stage 0); `rewardDayCap` on chain and the H3 day value limit in the canister bound it again |
| a board that is not ours | `set_offer_board` (a controller's act, like `set_item_config`) is refused unless `board.wrapper()` is our wrapper; the read checks it again at the first tick |
| the operator's power | unchanged: `frog_offer` is still judged in full; the operator cannot claim, slash or refund on its own (the canister decides from verdicts; the guardian can only pause) |

## 4. Why (a), not a relay

A relay (b) is one more keyed, off-canister service that must be up for anyone to be paid — the situation this replaces.
(a) puts the commitment on chain, where the reward is paid, and lets the bytes come from anywhere: the submitter, a mirror, the
operator. Its cost is the ingress budget of §3.3, which the bond and the size and rate limits bound. The flaw looked for and
not found: a hash on chain reveals nothing about the PSBT (a seller-signed PSBT is public on the marketplace anyway), and the
bytes are worthless to anyone else (they can only ever be signed by the float key into the custody output the judge
requires).

## 5. Reward economics: what a bot earns, what it costs

Per confirmed purchase: **+ reward (0.01 ETH)**, bond returned. Costs: two L2 transactions (`submit`, `withdraw`; on 4663 a
few thousandths of a cent each), one canister update call (free to the caller), and 0.01 ETH of capital held from the
submission to the claim — a purchase plus six confirmations, 1–4 hours; up to 24 hours if not bought. Gas on Bitcoin is the
seller's and the canister's, never the submitter's.

How much is there to earn: purchases are one at a time canister-wide and ≈3 hours each, so at most ~8 a day in principle;
stage-0 caps allow 3 mints a day a collection; a queue wants an item only when it holds `itemCap + buffer`. So a bot's ceiling
is 0.03–0.08 ETH a day a collection, against near-zero marginal cost: enough to run a listing scraper, not enough to make
manipulation (§3.4) pay. `rewardDayCap` of 0.2 ETH is above that ceiling, so it binds only when something is wrong.

What the bot has to do well: know the collection's floor (to be cheapest), have the seller-signed PSBT (the marketplace's), and
the membership proof (from the listing's published tree). A seller can be its own bot and treat the reward as a rebate.

## 6. Canister side (autobuyer/canister-rs)

| module | pure? | what |
|---|---|---|
| `handin.rs` | yes | the board's ABI (selectors, encoders, decoders of `Submission` and `uint256[]`), `HandinState` (boards, submissions seen, bytes, verdicts), the intake rules (size, hash match, the unmatched buffer, per-caller rate), the choice (`cheapest`), which verdicts are due a claim or a slash |
| `handinflow.rs` | no network | the sequencer's pieces over `FrogIo`: read the boards at the pin, offer the cheapest to a wanted frog, the claim and slash steps |
| `handinnet.rs` | — | `HANDINS` (stable, tenth element), the endpoints (`set_offer_board`, `submit_offer`, `offer_board_state`, `handin_verdicts`), the per-principal rate limiter (heap) |

**State** (stable): `HandinState { boards: Vec<BoardRec>, seen: Vec<Seen>, bytes: Vec<Bytes>, unmatched: Vec<Bytes>,
verdicts: Vec<VerdictRec> }`. Saved as the **tenth** optional element of the upgrade tuple, after the nine, nothing reordered.
The EVM steps `Claim { board, id }` and `Slash { board, id }` are two new `FStep` variants (frogflow's one-transaction rule
covers them: they queue after reports, mints, deposits and triggers).

**Verdict flow**: `Bought { frog }` → the frog's `Held` (a fresh six-deep read) → `Claim` sent → mined success, or the board
already says `Claimed`, → `Claimed`. `Invalid { why }` → `slashable(id)` true at the pin → `Slash` sent → `Slashed`. `Gone`
→ the custodian's `refund` is not driven (it would cost a transaction per dead listing; the submitter takes it at expiry) — the
slot just waits; open problem 3 says when to revisit.

**Where the tick changes** (`frogflow::tick_full`, wrapping the existing `tick_with`): the boards are read after the sinks
(step 3); `step_frog`'s `Wanted` arm falls through to the cheapest hand-in when `take_offer` is `None`; `evm_act` gains a
fifth group; `mined` handles the two steps. `accept`'s error is now `Why::{Invalid, Gone, Later}` so a verdict can be told from
a wait; the operator path flattens it to the same line as before.

**Endpoints**: controller: `set_offer_board(address)` (append only; refused unless its wrapper is ours). Anyone:
`submit_offer(psbt_hex, to_sign, proof_hex)` (paused: refused). Queries: `offer_board_state()`, `handin_verdicts()`.

## 6.1 As built (2026-09-23, commits `923ef11` Solidity, `f83fc1d` canister)

Everything above is what was built, with these refinements found in the code:

- **`accept` refuses in three kinds** (`frogflow::Why`): `Invalid` (the bytes), `Gone` (the listing: the seller's output spent,
  or a cost under `MIN_PURCHASE_WEI`), `Later` (a read failed, the cap or the rate moved, the caps or limits say wait). The
  operator's path prints all three as before. An `OverCap` from the judge is `Invalid` only when the spend exceeds the
  hand-in's `askSats`; otherwise it is the cap moving, and a wait.
- **`tick_with` is unchanged for callers**: it is `tick_full` with an empty `HandinState`; `frognet::tick` calls `tick_full`
  with `HANDINS`. `FrogIo` gained one defaulted method, `save_handins`, so the existing fake worlds compile untouched.
- **Ordering in `evm_act`**: reports, mints, EVM claims, deposits, triggers, then hand-in claims and slashes — a claim
  never delays a report or a mint; in practice the claim goes out the tick after `recordItem`.
- **The claim's depth re-read** uses `frogflow::depth` (the lowest explorer tip, the block still the block); a reorganisation
  sends the frog back to `Sent` through the mint's own check, and the claim is not due again until `Held`.
- **Bytes** are kept in `HandinState` (stable). The per-caller rate table is heap.
- **The board's `refund` by the custodian is not driven** by the canister (§7.3).

**Shared-file edits** (everything else is new files): `autobuyer/canister-rs/src/lib.rs` (three `pub mod` lines);
`src/frogflow.rs` (two `FStep` variants and their text; `save_handins` on `FrogIo`; `word`, `addr`, `depth`, `send` made
`pub(crate)`; `tick_full` wrapping `tick_with`; the boards read after the desk; `step_frog`'s hand-in fallthrough; `accept`'s
`Why` and `ask`; the fifth `evm_act` group; `resolve` routing `Claim`/`Slash` to `handinflow::mined`); `src/frognet.rs`
(`save_handins`, `tick_full`, `HANDINS` in and out); `src/canister.rs` (the tenth element of `Saved`, `pre_upgrade`,
`post_upgrade`, the existing restore tests' literals, and one new restore test); `etch_autobuyer.did` (four endpoints, four
types). No Solidity outside `src/handin/` and `test/handin/`.

**Tests**: `test/handin/OfferBoard.t.sol` (16: the exact ABI against the pinned selectors and the `cast abi-encode` fixture the
canister decodes; submit, claim, slash, refund, withdraw, the day cap on both clocks, a submitter that refuses ETH, the
custodian following the wrapper); `test/handin/OfferBoard.invariant.t.sol` (4 invariants: the books, rewards ≤ budget, paid at
most once, bonds conserved). Canister: `src/handin.rs` (9 unit tests: selectors, decoders, intake, reconcile, choice,
verdicts, the rate); `tests/handins.rs` (15 against a fake world: bought and claimed; the operator first; invalid slashed after
the delay on both clocks; a lying ask; cheapest over dearer with the dear one refunded at expiry; a dead listing gone, not
slashed; hash-only slashed; buffered bytes matched at the read; bounded reads; reorganisation after held; the listing vanished
after signing; a refund before the claim read from the board's revert; too near expiry; a board that is not ours; the day's
value limit and the boardless tick). Suites before and after each commit: forge 628, cargo all green, the release wasm
builds.

## 7. Open problems

1. **Ingress cost of refused `submit_offer` calls.** A flood of oversized or unmatched messages costs cycles before the refusal.
   An `inspect_message` that refuses anonymous callers and oversized payloads before execution is the next step; the
   canister's cycles alarm (G8) is the backstop.
2. **Who funds the reward pool.** `fund()` is anyone's; nothing routes fees to it yet. The natural source is the collection's
   fee router; that is the Floor Module branch's file, not this one's.
3. **`Gone` slots.** A board full of dead listings is refundable only at expiry (or by the custodian). If that becomes a real
   nuisance, drive `refund` for `Gone` verdicts from `evm_act` (one transaction each, lowest urgency).
4. **The board is read only when a wrapper is configured**, like everything on the items side; a Floor Module for EVM NFTs has
   no hand-in path (Seaport orders are the NFT line's, docs/v4).
5. **Deep reorganisations after a claim** are the system's assumption, not this module's fix.

## 8. What stays operator-only

`frog_offer` (the fallback offer for a wanted frog, tried first), `frog_proof`, `refuse_deposit`, `tick`, `set_paused` (the
guardian's pause covers `submit_offer`, `Claim` and `Slash` as it covers every other act). The board address is the
controller's (`set_offer_board`), as sinks are. Nobody but the canister can claim or slash; nobody but the submitter (or the
canister) can refund.
