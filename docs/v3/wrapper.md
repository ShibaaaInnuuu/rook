# The wrapper: what one wFROG is, and why it needs no price

Written 2026-09-22, **before its code**, in the fork `etchwrap` taken from `etch` at the fourth audit pass. It is the design
for the one route that produces a token an ALIEN/wFROG pool can hold. Nothing here is deployed or audited.

## The decision this document exists to make

Pairing is not the hard part. Pons V2's `launchToken(params, configId, **pairToken**, exemptions)` already takes a pair
asset, and `Launcher.sol` passes `address(0)` on purpose — native ETH only, because a coin quoted in an ERC-20 would pay its
fees in that token and the vault does not hold it. One argument stands between here and a custom pair.

The hard part is **what one wFROG entitles you to**, because that single choice decides whether the peg holds itself or has
to be propped up by somebody publishing a number.

## The answer, and the sentence it rests on

> **One wFROG is one random inscription from this vault, and the way you find out what that is worth is to take one.**

A whole token burns for a claim on one frog. Which frog is decided by the drand beacon this repository already uses for
draws — not by the redeemer, and not by the vault. That is the entire design, and everything below is consequence.

**Redemption is the oracle.** Nobody publishes a price. Anyone who thinks wFROG is cheap burns one and takes a frog; anyone
who thinks it is dear sells one into the pool. The value of the token is discovered by people acting on it, not asserted by
a feed somebody has to defend. That is the difference between a peg that survives a bad week and one that does not, and it
is the reason this design is possible at all — an NFT price oracle is thin, manipulable, and is the most common cause of
death in this category (OWASP SC02).

### Why not the two obvious alternatives

**The redeemer chooses.** Then wFROG prices to the *best* frog in the vault, everyone redeems exactly those, and the vault
is left holding what nobody wanted while the token still claims to be worth one frog. The peg inverts immediately.

**First in, first out, published.** Atomic, simple, no beacon — and it prices the token to *whatever happens to be next in
line* rather than to the vault. A reader can see a good frog is next and burn to take it; the token's price then oscillates
with queue position, which is a fact about ordering rather than about backing. Targeted redemption in a slower costume.

Random redemption prices the token to the vault's **mean**, which is a statement anybody can check by looking at what the
vault holds. Note this is deliberately *not* floor. Floor is what a basket with a published mark converges to, and it needs
the oracle. A mean over a visible set needs nothing.

## What this changes, and what it does not

Less than it first appears. The vault already receipts an inscription, and the Distributor already hands a rune out
**pro rata by Merkle root** while handing an inscription out **by draw**. The wrapper turns an inscription into a divisible
quantity, so it takes the path a rune already takes:

| | before | after |
|---|---|---|
| a receipted inscription | drawn among holders, one winner | minted as 1 wFROG, distributed pro rata |
| a receipted rune | distributed pro rata | unchanged |
| the draw | allocates a lot to one holder | allocates **a frog to a burner**, same beacon |

So the draw machinery is not thrown away; it moves. `openDraw`/`requestDraw`/`resolve` pinned a future drand round and
resolved a winner from it — a redemption pins a future round and resolves *which frog leaves*. The Merkle claim path
(`openDistribution`, `claimShare`) already exists and is what a frog's 1e18 units are handed out by.

**What does retire is the draw as a distribution mechanism.** Once a frog is a divisible balance, holders exit
independently and there is nothing left for a per-lot draw to allocate. This is a replacement, not an addition, and calling
it additive would be wrong.

## The shape

`ONE_ITEM = 1e18`. The token has 18 decimals and one whole token is one frog, so the ERC-20 does the fractionalising and
there is no second unit to keep straight.

### Minting
A frog is minted **when it is receipted, to the vault**, and distributed from there by the existing Merkle root. Never to a
person directly: who owns a coin changes block by block, and a snapshot taken at settle time would be a different set from
the one that paid for it.

### Redeeming — three phases, because Bitcoin is not here
A burn cannot atomically send a Bitcoin transaction, so a redemption is a **ticket**:

1. **Burn.** One whole token is burned and a ticket is opened, pinned to a drand round in the future. The tokens are gone
   from supply at this moment, which is what keeps supply and backing in step.
2. **Resolve.** Once the round exists, anyone proves it. The randomness picks a frog from the vault's *currently held* set,
   and that frog is marked spoken-for. The ticket now names an inscription and a destination.
3. **Deliver.** The buyer sends it on Bitcoin and the delivery is receipted, exactly as a draw's delivery is today.

**And the fourth phase, which is the one that matters.** If delivery never happens by a deadline, the ticket is
**refundable**: the frog is returned to the held set and the burned token is **re-minted to the ticket's owner**. Without
this, a canister that stops destroys tokens for nothing and the supply silently stops matching the backing. A burn that
cannot be undone when the thing it bought never arrives is not a redemption, it is a donation.

### The ledger invariant
At every moment, and asserted by the invariant campaign:

```
totalSupply == (frogs held and not spoken for + frogs on open tickets) × ONE_ITEM
```

Every wFROG in existence is backed by a frog this vault still holds, and every frog it holds is either backing tokens or
already promised to a ticket.

**Correction (2026-09-23, before the code): the formula above double-counts.** The sentence under it is right and the
formula is not. A frog on an open ticket backs *no* tokens — they were burned to open the ticket — so adding it to the
right-hand side counts it once as backing and once as promised. After one burn from a vault of `H` frogs the supply is
`H − 1`, and the formula as written asks for `H`. What the contract asserts, and the invariant campaign checks, is:

```
totalSupply == (held − openTickets) × ONE_ITEM
```

where `held` is every frog wrapped and not yet delivered, and `openTickets` is every burn neither delivered nor refunded.
Equivalently `totalSupply + openTickets × ONE_ITEM == held × ONE_ITEM`: each held frog backs exactly one whole token or
exactly one open ticket, never both.

## What this does NOT protect against, because nothing on this chain can

- **A frog that leaves Bitcoin without this chain knowing.** A key compromise, a lost canister, an inscription transferred
  by something other than a redemption. Supply and backing diverge and the contract cannot see it. This is the irreducible
  custody risk of every wrapper, and it is why the custodian matters more than the wrapper.
- **The buyer failing to deliver.** Bounded, not eliminated: the ticket refunds and the token comes back, but the redeemer
  has waited and the vault has been frozen for that frog in the meantime.
- **A vault holding frogs nobody wants.** wFROG then trades below one frog and nobody redeems. That is not a failure — it
  is the mechanism reporting honestly, and it self-corrects when the contents change.
- **The mean being the wrong number for a particular buyer.** Somebody who wants one specific frog is not served by this and
  should buy it on a marketplace. The token is a claim on a basket and says so.

## Why this custodian and not another

Every wrapped-NFT product has the same weak point: somebody holds the real thing, and it is usually a multisig. Bitcoin
makes it sharper, because there is no escrow contract on L1 — the holder is a key.

`etch`'s v2 buyer is already a program on the Internet Computer whose signing key is held by a subnet and by no person, and
it was built to remove a bonded human from the *buying* path. A custodian that is published code compelled to release a
frog when a burn lands on chain is a materially stronger claim than any multisig can make.

It is not finished, and the gap is named rather than glossed. From `docs/security/audit-btc-v1.md`:

- **B3.** The sighash is computed off-canister today, so `psbt.rs::verify` — 716 lines that are the whole safety argument —
  is not on the path that signs, and has no non-test caller. A wrapper needs the canister to send Bitcoin **out** on demand;
  it can barely send it in.
- **B4.** The canister cannot broadcast an EVM transaction at all.
- And the two DFINITY recommendations nothing answers: a **timelocked controller** and a **reproducible build**. For an
  accumulator, one identity being able to replace the wasm with no delay is a nuisance. For a custodian it is the whole
  question, and until both exist the honest sentence is *the operator can change what the buyer does, with a delay of zero*.

Nothing in this document holds real money until those are closed. What can be built and tested first is the contract, which
is what the next commit is.

## Built: what the contract had to decide that this document did not (2026-09-23, before the code)

`src/Wrapper.sol` is step 1 below. Writing it forced these, each taken on the conservative reading and recorded here first.

**1. One wrapper per inscriptions listing, and the wrapper is the token.** An ERC-20 with 18 decimals and `ONE_ITEM = 1e18`.
Deploying it for a rune listing reverts: a rune is already divisible.

**2. What may be wrapped, and who says so.** The design says a frog is minted "when it is receipted". Receipting alone is
not enough today, because the Distributor still exists and a receipted inscription can still be drawn or named — the same
frog would then be delivered to a winner *and* back tokens. Until step 2 retires the draw, `wrap(acquisitionId)` requires:
the acquisition is one inscription of this listing's vault; the creator of the coin that paid for it is the caller; and that
creator has already chosen `Mode.Never` for it in the Distributor, a choice the Distributor makes final. So a frog is wrapped
only when the one party entitled to decide its fate has decided it will never be drawn. A paused listing wraps nothing new.

**3. Minted to the distribution contract, not to the vault.** "Minted to the vault" read literally would strand the tokens:
neither vault has a function that moves an ERC-20, and adding one to a vault that has no withdraw is the wrong trade. The
wrapper mints to an immutable `distribution` address fixed at construction — step 2's pro-rata claim contract, which hands a
frog's `1e18` out by the existing Merkle-root pattern. Until it exists, tests use a plain address.

**4. The draw must not be steerable after the beacon speaks.** "Picks a frog from the vault's currently held set" is
grindable as written. Between a round becoming public and a ticket being resolved, three things can change the held set:
another ticket's resolution (anyone chooses the order), a refund (anyone chooses when), and a new wrap (the creator chooses
when). Each changes which frog a known random number lands on. So:

- tickets resolve **strictly in order**: ticket `n` cannot resolve before ticket `n − 1`;
- every frog carries a **stamp**, the number of tickets that existed when it last became free (wrapped, or returned by a
  refund). Ticket `n` draws only among free frogs stamped `≤ n` — frogs that were already free when it was burned.
  A frog wrapped or refunded after the burn waits for a later ticket.

Given the order of burns, the set each ticket draws from is then fixed before its round exists, and nothing anyone does
afterwards moves it. There is always at least one frog to draw: a burn needs a whole token, a whole token means
`held − openTickets ≥ 1` when it is burned, and every earlier unresolved ticket takes exactly one.

The cost is that one ticket whose round drand skipped holds up the tickets behind it until anyone re-pins it, 24 hours later
on both clocks (the Distributor's `redraw`, same constant).

**5. The destination is fixed at the burn.** A taproot address, checked as the Distributor checks a winner's. There is no
correction hour: the burner types their own address in the same transaction that burns, and a correction window would only
add a time the buyer must not send in.

**6. Delivery is recorded by the keeper**, as a draw's is today (`Distributor.recordDelivery`), within **7 days** of
resolution, on both clocks. Past that, **anyone** may refund: the frog goes back to the free set (stamped, per 4) and one
whole token is minted to the ticket's owner. A delivery may still be recorded after the deadline as long as nobody has
refunded yet.

**The race this leaves, stated rather than hidden.** A Bitcoin delivery sent near the deadline and a refund on this chain
can both happen: then the redeemer has the frog *and* the token, and the supply is backed by a frog the vault no longer
holds. No contract can see a Bitcoin transaction, so no contract can close this. The custodian must not **send** after
`deliverBy − 24 hours`, and must check the ticket is still `Resolved` immediately before sending. That is a rule for the
canister (step 3), and it is written here so step 3 cannot forget it.

**7. Nothing the owner does stops a redemption.** Pause gates `wrap` only. Redeem, resolve, deliver and refund are never
paused, because an owner who can halt redemption can break the peg, which is the one thing the design says it does not rely
on anybody to hold up.

**8. A ticket whose round is never proven is never refunded.** Refund is from `Resolved` only. A permanently dead beacon
would strand open tickets; that is the same exposure the Distributor's draws already accept.

**How it is tested** (`test/Wrapper.t.sol`, `test/security/WrapperSecurity.t.sol`, `test/Wrapper.invariant.t.sol`). The unit
suite walks every transition and every refusal, including that a frog in any Distributor mode but `Never` cannot be wrapped
and that a wrapped frog can never be drawn afterwards. The security suite runs each grinding attack in 4 as two worlds that
differ only by the attacker's move after the round is public — a late wrap, a timed refund, a resolution out of order — and
requires the same frog in both, fuzzed over the randomness (10,000 runs in the deep profile); a companion test shows the late
wrap WOULD have moved the draw without the stamp, so the check is not vacuous. Two more move only the sequencer's clock and
show neither a refund nor a re-pin opens early. And the wrapper's whole ABI is asserted exactly: no owner, no setter, no
sweep, nothing that mints but `wrap` and `refund`. The invariant campaign (200,000 calls in the deep profile) asserts the
corrected ledger, that every held frog is in exactly one place — drawable, waiting, or named by one open ticket — that
tickets resolve in order, that every token is held by someone we can see, and that a ticket whose turn it is always resolves.

## Steps 2 and 3: the claim contract, and the rule the custodian sends by (2026-09-23, before the code)

### Step 2: `WrapClaim`, the `distribution` address

`Wrapper.wrap` mints every frog's `ONE_ITEM` to an immutable `distribution`. Step 2 is that address: a contract that hands a
frog's tokens to the holders of the coin that paid for it, pro rata, by Merkle root — the path `Distributor.openDistribution`
/ `claimShare` already takes for rune units, with one difference that removes a whole stage: the thing being handed out is an
ERC-20 on this chain, so a claim **transfers the tokens** and is finished. There is no accrual, no minimum, no payout request
and no Bitcoin leg; the Bitcoin leg is the wrapper's redemption, which any holder uses when they choose.

**Binding.** `WrapClaim` deploys its `Wrapper` in its own constructor, passing `address(this)` as `distribution`. So
`wrapper.distribution() == address(claim)` holds by construction, both are immutable, and there is no setter, no owner step
and no window in which one points somewhere the other does not. `Wrapper.sol` is unchanged.

**Who can claim what.**
- **Opening.** The keeper publishes, per wrapped frog, a root over leaves `(holder, amount)` and a `uri` where the whole list
  is — exactly the Distributor's `shareLeaf` shape, so the same tooling builds both. `open(acquisitionId, root, uri)`
  requires the frog to be wrapped and not already opened. **The total is fixed by the chain, not the keeper:** one
  `ONE_ITEM`, plus whatever the same coin's earlier distributions left unclaimed (`carry[coin]`).
- **The challenge.** For 15 minutes on both clocks (`Wait`) nothing can be claimed, and the owner may cancel a root that is
  wrong. A cancel puts the carried part back in `carry[coin]` and lets the keeper open the frog again with a corrected root.
  After the 15 minutes nobody can cancel anything — the owner has no power over a root that has become claimable.
- **Claiming.** Anyone may submit a holder's proof; the tokens go to the **holder in the leaf**, never to the caller. One
  claim per holder per frog. A root that promises more than its total cannot pay past it (`OverTotal`), so a bad root can
  misallocate within one frog's tokens but can never reach another frog's.
- **Unclaimed tokens.** Claims stay open for 30 days (both clocks). After that anyone may roll the distribution over: what
  nobody claimed joins `carry[coin]` and is part of that coin's **next** frog's distribution. If the coin never has another
  frog, the carried tokens stay in this contract. They are still fully backed — the ledger counts them — and the frog behind
  them stays in the vault, available to every other redeemer's draw. Nothing is lost; the tokens are idle. A carry-only
  distribution for a coin that will never buy again is deliberately **not** built: it would be a keeper power to publish a
  root with no acquisition behind it, and idle-but-backed is the safer failure.

**What the owner can and cannot do.** Cancel a root inside its 15 minutes. Nothing else. There is no pause (a paused claim is
a peg someone can hold up, the same reason the wrapper's redemption cannot be paused), no sweep, no rescue, and no function
that moves this contract's tokens except `claim`. In particular this contract never burns, never redeems and never
transfers to anyone but a leaf's holder.

**The draw, retired.** The order-of-work item "retire the inscription draw" is met per frog, not by changing the Distributor:
a frog can only be wrapped after its creator's final `Mode.Never`, so a wrapped frog can never be drawn. The Distributor is
left untouched; its draws still serve any inscription that is *not* wrapped.

**The ledger, across both contracts.** The wrapper's invariant is unaffected by claims — a claim is a transfer, and
`totalSupply == (held − openTickets) × ONE_ITEM` does not see who holds a token. Step 2 adds its own, and the invariant
campaign asserts both together:

```
wrapper.balanceOf(claim) == unopened × ONE_ITEM  +  Σ(open, not rolled) (total − claimed)  +  Σ carry[coin]
```

where `unopened` counts wrapped frogs with no distribution yet. Every token this contract holds is in exactly one of those
three places; the campaign also asserts no holder receives more than their leaf and every claimed token left for a leaf's
holder. (A token someone *sends* to the contract unasked is not accounted, and is stranded; the contract never mints and never
counts it, so it cannot be claimed twice. Stated rather than guarded against.)

### Step 3: the rule the custodian sends by

The race recorded in "Built", 6: a Bitcoin delivery near the deadline and a refund on this chain can both happen, and no
contract can see Bitcoin. The fix is a rule in the canister, written here as a pure function (`autobuyer/canister-rs/src/
delivery.rs`) and wired into the buyer's loop when that exists. **Given the ticket as read from chain 4663 immediately
before sending, the canister answers Send, Wait or Refuse:**

1. **Stage.** `Resolved` is the only stage that can Send. `Pinned` → **Wait** (not drawn yet). `Delivered`, `Refunded` or
   no ticket → **Refuse**: sending now would send a frog nobody is owed.
2. **The ticket names what we are about to send.** The ticket's frog (acquisition id) and destination must equal the ones in
   the transaction about to be broadcast; anything else → **Refuse**.
3. **The read is fresh.** Read no more than **60 seconds** before the decision, on the canister's own clock, and not in its
   future. Older → **Wait** and read again. This is what "check immediately before sending" means as a number.
4. **The margin: one day, on EITHER clock.** Refund needs *both* the ticket's `deliverBy` timestamp and its
   `deliverByBlock` (parent chain) to have passed. The canister stops sending when **either** clock is within **24 hours**
   of its deadline — `deliverBy − chainTimestamp ≤ 86,400 s`, or `deliverByBlock − parentBlock ≤ 7,200 blocks`
   (`Wait.blocksFor(24 hours)` at 12 s) — after first taking off the seconds elapsed since the read. Inside the margin →
   **Refuse**, and it is final for that ticket: the ticket is left to refund, the frog returns to the set and the redeemer
   gets their token back. Losing a delivery to a slow day is recoverable; sending into a refund is not.
   Taking *either* clock is stricter than refund's *both*: whichever deadline is nearer is the one that binds. The sequencer
   can push the timestamp up to an hour ahead of the wall clock, which makes the timestamp clock read *nearer* — the safe
   direction for a rule that stops sending.
5. **What 24 hours buys.** A Bitcoin transaction confirmed and the keeper's `recordDelivery` landed on chain 4663, with a day
   to spare. `recordDelivery` stays valid after the deadline for as long as nobody has refunded, so a delivery sent inside
   the window is never lost to a slow confirmation.

What this does not fix: a canister that ignores its own rule, which is the custodian question the timelocked controller
and the reproducible build exist for; and a chain read that lies, which is why the canister reads 4663 through `rpc_view`,
every provider agreeing.

### Built: what the code settled (2026-09-23, with the code)

- **Counting is lazy.** `wrap` mints to the claim contract and calls nothing on it, so a wrapped frog becomes `unopened` the
  first time anyone looks — `count(id)`, or the keeper's `open` — and always from `wrapper.wrapped(id)`, never a caller's
  word. The joint invariant counts a wrapped-but-uncounted frog as its own place, so the books balance at every moment.
- **Both of step 2's windows are on both clocks.** The challenge and the 30-day claim period each end only when the
  timestamp **and** the parent block have passed (`Wait`). The Distributor's own rune `claimsEnd` is timestamp-only; this
  contract does not copy that, so a sequencer stamping an hour ahead can neither close claims early nor force a roll-over.
- **The margin's boundary refuses.** Exactly 86,400 s or exactly 7,200 blocks left is *inside* the margin. A read dated
  after the canister's own clock is treated as stale (Wait), not trusted.
- **Wait and Refuse are different answers.** A ticket not yet drawn, or a stale read, is Wait: read again. A ticket
  delivered, refunded, missing, malformed, naming a different frog or address, or inside the margin is Refuse, and final.

Tested in `test/WrapClaim.t.sol` (13), `test/security/WrapClaimSecurity.t.sol` (6: a proof carried to another frog, a
jumped timestamp, a pushed claim fuzzed over the pusher, a cancel that moves nothing, the contract's exact ABI, and that it
never redeems), `test/WrapClaim.invariant.t.sol` (the wrapper's ledger and step 2's books asserted together over every action
of both contracts, 200,000 calls in the deep profile, with a hand-driven run that reaches a cancel, an over-promising root,
a carry into the coin's next frog, a second coin, and claimed tokens burned, resolved, delivered and refunded), and
`autobuyer/canister-rs/src/delivery.rs` (10: each stage, each mismatch, freshness, each clock alone, the seconds since the
read, past the deadline, and that a margin refusal never turns into a send).

## Order of work

1. **The wrapper contract and its tests** — mint on receipt, burn to a ticket, resolve by beacon, deliver, refund on
   deadline. Testable today against the existing mocks, and it forces every decision above to be concrete.
2. **Retire the inscription draw** in favour of the pro-rata path, which already exists. *Built as `WrapClaim`; see "Steps 2 and 3".*
3. **The canister's outbound path** (B3), which is the gate on any of it being real.
4. **The timelocked controller and the reproducible build**, before a single frog is held for value.

## Step 3, the flow: how the custodian delivers a redeemed frog (2026-09-23, before the code)

`delivery::decide` is the rule. This is the flow around it: find the tickets owed a frog, find the frog, build and sign the
transaction in the canister, send it only when the rule says Send **at that moment**, believe only Bitcoin, and get the
delivery recorded on chain 4663. Code: `autobuyer/canister-rs/src/custody.rs` (pure: locating, choosing, building) and
`src/redeem.rs` (the flow, over its own `CustodyIo` trait, tested against a fake world in `tests/redeem.rs`).

### Which tickets
Every tick reads `Wrapper.ticketCount()` and walks forward from a saved cursor. Tickets resolve strictly in order
("Built", 4), so the walk stops at the first `Pinned` ticket: nothing after it can be `Resolved`. A `Resolved` ticket with no
redemption record gets one. A ticket already `Delivered` or `Refunded` when first seen is passed over. The wrapper address is
part of the canister's identity: `Init.wrapper`, which an upgrade may set once (from nothing) and never change.

### Which UTXO is the frog — custody, read, never remembered
The ticket names an acquisition; `vault.acquisition(id).assetRef` is the inscription id. Where that inscription is now is read,
not recalled, from two kinds of source that must both hold:
- **ord** (`ordinals.com`, the reference implementation, keyless, IPv6): `/r/inscription/<id>` gives its satpoint and output;
  `/r/utxo/<output>` gives everything on that output — its inscriptions and its runes.
- **esplora** (every configured explorer, agreeing, replicated): the output exists, is **unspent**, is confirmed, pays **our
  own taproot script**, and has the value ord reports.

The frog's output must carry **exactly this one inscription and no runes**. Anything else — another inscription riding on the
same output, a rune balance, an output that is not ours, a value that disagrees — is `Stuck(Custody)`, loudly: a frog that is
not where the canister can safely send it from is the "frog that leaves Bitcoin without this chain knowing" case above, and
nothing about it gets better by retrying.

**This only exists on Bitcoin mainnet.** There is no keyless ord for signet or testnet4 (`docs/v2/rates-and-indexer.md`), so
on a test network the flow refuses to locate a frog rather than trusting memory. The rehearsal is therefore fixtures plus
live read-only reads of mainnet.

### The transaction
```
in  0: the frog's output               (value V, the inscription on one of its sats)
in  1: one plain UTXO of ours          (value F: no inscription, no runes — checked on ord, per candidate)
out 0: the redeemer's taproot script   value V   ← every sat of input 0, so the inscription lands here wherever it sat
out 1: our own script                  value F − fee   (only if ≥ 330 sats; below that it would be dust and goes to fee)
```
Output 0 takes exactly input 0's value, so under ordinal theory's first-in-first-out assignment every sat of the frog's
output — including the inscribed one, at any offset — lands on output 0 and nothing else does. The fee comes only from
input 1. The fee rate is read from the explorers and bounded: at least 1 sat/vB, at most 100; a fee above
`MAX_DELIVERY_FEE_SATS` (25,000) is refused rather than paid. The digest is computed in the canister (`sighash::
taproot_key_path`, from the prevouts it just verified), both inputs are signed with the threshold key, and the transaction's
id is known before anything is broadcast. A fee UTXO already reserved by another redemption in flight is never chosen.

### The rule, at the moment of sending
`decide` runs on a ticket read **after** signing (signing awaits the subnet), and again before **every** re-send of saved
bytes. The read names one block — a few behind the lowest head any provider reports — and takes that block's own timestamp
and parent-chain number (`l1BlockNumber` on Nitro), so the ticket and the clocks it is judged against are the same moment.
- `Send` → save the signed bytes, then broadcast; only `btc_tx_seen` is believed.
- `Wait` → nothing is saved or sent; next tick.
- `Refuse` → final for that redemption. If signed bytes were saved and Bitcoin has never shown them, they are **not** sent
  again, and the reason is recorded; the residual risk — a node somewhere holding bytes we sent earlier that no explorer
  shows — is stated, not claimed away.

### Recording the delivery: the keeper question
`Wrapper.recordDelivery(id, txid)` is `registry.keeper()`'s alone, and `ListingRegistry.proposeRoles` refuses an
auto-buyer as keeper (`isAutoBuyer[keeper_] → SameKey`). **So as the contracts stand, the canister cannot record its own
deliveries.** The flow therefore does both, without changing `Wrapper.sol`:
1. after Bitcoin confirms, it reads the ticket; `Delivered` (someone recorded it) → done;
2. if our EVM address **is** `registry.keeper()` (not today), it records the delivery itself through the EVM path — build,
   sign, save the bytes, send, believe the read-back;
3. otherwise it holds the redemption in `AwaitingRecord`, and `pending_delivery_records()` (a public query) lists
   `(ticket, txid)` for the keeper to record — the txid in display order, as `keeper/live/cli.ts` takes it.

The consequence, named: until the record lands, the ticket is still `Resolved`, and after `deliverBy` anyone may refund it —
the redeemer then has the frog **and** the token. The one-day margin gives the keeper at least a day minus one confirmation
to record. That depends on a second key being awake. **The recommended fix is a contract change, not made here:** an
immutable `custodian` in `Wrapper` (the canister's EVM address, passed through `WrapClaim`'s constructor) allowed to call
`recordDelivery` alongside the keeper, so the party that sent the frog records it. It changes `Wrapper.sol`, `WrapClaim.sol`,
their tests and the deploy scripts, which is the operator's decision.

### If the ticket is refunded after the frog was sent
`LostRace`: the frog was delivered on Bitcoin and the token refunded on 4663. It is recorded and reported, never retried,
because the only "fix" would be to send a second frog nobody is owed.

### Restart safety, in the style of the buyer loop
The signed bytes are saved before the first broadcast; a restart re-sends the same bytes (after `decide` says Send again) or
reads Bitcoin to see they landed. A reply lost after a broadcast is covered by reading Bitcoin, not the reply. A confirmed
delivery whose record has not landed is re-checked each tick. The EVM record transaction is itself saved before it is sent,
and one EVM transaction of ours is in flight at a time across purchases and deliveries.

### Built: what the code settled, and what was shown (2026-09-23, with the code)

**Files.** `autobuyer/canister-rs/src/custody.rs` (pure: ord and esplora parsing, the custody verdict, fee choice, the
builder), `src/redeem.rs` (the flow, `CustodyIo`, the wrapper's ABI), `src/custodynet.rs` (the canister's `CustodianIo`: its own
replicated outcalls and transforms for ord and the explorers, and everything that signs, sends or reads 4663 delegated to the
buyer's `CanisterIo`, so deliveries use the same pinned reads, keys and send paths as purchases), `tests/redeem.rs`.
Shared files, each by a few lines: `canister.rs` (`CanisterIo`, `CanisterIo::new`, `CanisterIo::pin` and `ours()` made
`pub(crate)`; `Init.wrapper`; the fifth, optional element of the saved state; the custodian pass at the end of `tick`; the
purchases' one-EVM-transaction rule now counts a delivery's record too), `lib.rs`, `etch_autobuyer.did`.

**Decisions.**
- **The ticket read is pinned twice over.** The wrapper's `ticket()` is read at the tick's agreed pin; the rule's reads take a
  fresh pin each, and the pinned block's own timestamp and `l1BlockNumber` (read by hash, every provider agreeing) are its
  clocks. The pin is a bucket behind the head, so the chain's time is taken as at least the canister's and the parent block
  is advanced by the lag at 12 s — both make the deadline read nearer, the safe side for a rule that stops.
- **Signed goes straight to Confirmed** when Bitcoin already has it in a block; Sent is only "in a mempool".
- **An evicted transaction goes back to Signed**, so the rule runs again before the same bytes are re-sent.
- **The fee rate is a report, bounded** (1–100 sat/vB, the highest the explorers give); the fee input's contents are
  checked on ord per candidate, smallest sufficient first, at most 10 asked.
- **The wrapper is identity**: `Init.wrapper` is set once by an install or upgrade argument and never changed after.

**Tested.** `custody.rs` (6, on replies captured from ordinals.com, mempool.space and a real address's UTXO set:
inscription 0's location and contents, a plain UTXO, a runes UTXO; every custody refusal; fee eligibility; the builder and
its refusals). **An implementation that is not ours agrees byte for byte:** `autobuyer/gen-delivery-fixture.mjs` builds the
same delivery with @scure/btc-signer, and its txid (`c94a3138…ab41`) and both BIP341 digests equal `custody::build` +
`sighash::taproot_key_path`'s. `redeem.rs` (1: every selector and the `ticket()`/`acquisition()` returns against `cast`).
`tests/redeem.rs` (11): the whole path to `Done`; the margin refusing before anything is built; a refund between the first
read and the send (signed, never saved, never sent); death between save and send (the same bytes, nothing re-signed); saved
bytes never shown by Bitcoin and now inside the margin (not re-sent); a lost reply and an eviction; a refund after delivery
(`LostRace`, never retried); recording as the keeper (bytes saved first, one EVM transaction at a time with purchases);
custody refusals (ord behind = wait; not ours, runes = stuck); a runes UTXO and another delivery's input never paying a fee;
the ticket walk stopping at the first `Pinned` ticket without duplicates. `canister.rs` (2 more): a state saved by the
canister now deployed (four elements) restores into the new shape, and custody round-trips.

**Shown on a replica** (new canister `jrblj-hp777-77777-aaana-cai`, installed with `main`'s build `cf68cfb` gzipped, paused,
then upgraded to this branch): the upgrade restored the saved state (still paused, the log kept, `upgraded: state restored`)
and set the wrapper from nothing. `probe_frog`, read-only against Bitcoin **mainnet** through the canister's own replicated
outcalls: inscription 0 and a second inscription each came back with ord's satpoint, the output's contents, and both
explorers agreeing on value, script and unspent — verdict *not in custody*, as it must be; an id ord does not know →
"ord does not know"; `../etc/passwd` refused before any URL. A `tick` with the wrapper set ran the custodian pass and failed
closed with its reason.

**Not shown, and why.** A delivery sent and confirmed: there is no keyless ord for signet or testnet4, so no network other
than mainnet can locate a frog, and nothing here touches mainnet but reads. A ticket read from a deployed `Wrapper`: no
public-chain deploy, and the anvil route was not run.

**Found, outside these files.**
1. **A purchase can spend a frog.** The buyer and the custodian share one taproot address, and `psbt::verify` checks that the
   inputs it signs are ours and that the spend is under the cap — not that they carry no inscription. A venue's (or a
   relayer's) PSBT listing a frog's 546-sat output among "our" inputs passes, and the loop signs every one of our inputs it
   includes. Fix, structural: derive the custody key on its own path (a separate address that purchases never spend from);
   or, at least, refuse to sign any of our inputs that ord shows carrying an inscription or runes (`custody::fee_eligible`'s
   check).
2. **The keeper question** (above): `recordDelivery` is the keeper's alone and the registry refuses an auto-buyer as keeper.
   Recommended: an immutable `custodian` in `Wrapper`, allowed to record.
3. **Pinned reads against Blockscout's RPC.** The testnet explorer's `/api/eth-rpc` answers `eth_call` with a `{"blockHash": …}`
   block parameter "Invalid block number", so an install naming it cannot read contracts at the pin (seen here: the
   custodian pass stopped on it). The mainnet pair recommended in `docs/deploy/canister-mainnet.md` is unaffected; a testnet
   rehearsal needs two providers that take EIP-1898 parameters.
4. **The wasm is past 2 MiB.** `main` at `cf68cfb` is 2.24 MB, this branch 2.53 MB; gzipped 575 KB. An install must use the
   gzipped module (and its module hash is the gzip's, as #24 found).

## Custody on its own key, and a custodian that records its own deliveries (2026-09-23, before the code)

Two findings above are closed here: **a purchase could spend a frog** (the buyer and the custodian shared one taproot address)
and **the keeper question** (only the keeper could record a delivery, and the registry refuses an auto-buyer as keeper).

### 1. Custody is a different key, a different address
- **The custody key** is the canister's threshold Schnorr key on derivation path `b"custody"`; the buyer's stays on `b"btc"`.
  Different path, different key, different taproot output key, different address. `custody_output_key()` publishes it, as
  `taproot_output_key()` publishes the buyer's.
- **Structurally, a purchase cannot spend a frog.** `sign_purchase` and the loop sign only with the buyer's key, and only inputs
  whose prevout pays the buyer's script. A frog never sits at that script. A PSBT that lists a custody output as one of "ours"
  is refused (`ForeignInputToSign`) if it tells the truth about the script; if it lies, the BIP341 digest commits to the real
  prevout script (`sha_scriptpubkeys`), so the signature is for a transaction that does not exist, and it is by the wrong key.
- **Belt and braces.** Before the buyer signs any input as its own, ord's `/r/utxo` is asked about that exact output, and the
  signature is refused if it carries **any inscription or any rune**. On Bitcoin mainnet an ord that cannot be read fails the
  signature closed. On signet and testnet4 there is no keyless ord: the check is not made, the refusal reason says so in the
  log, and the structural separation is the only defence — acceptable because nothing of value sits on a test network, and
  stated rather than hidden.
- **Custody sends are signed with the custody key**; the delivery's fee comes from a plain UTXO **of the custody address**, so
  custody holds a small float of plain sats. **Who funds it:** the operator, by sending plain sats to the custody address
  (one delivery costs ~212 vB × the fee rate; 20,000 sats covers dozens at today's rates). A delivery with no clean fee UTXO
  waits and says so; nothing else is ever used for the fee.

### 2. Where a wrapped frog lives, and who can move that
- The Wrapper records it: `custodyAddress`, a taproot address fixed in the **constructor** (through `WrapClaim`'s), with no
  setter — immutable in effect (a string cannot be an `immutable`, so it is storage written once).
- A purchased inscription is delivered to the listing's `btcVault` (`policy::order_for`). So `wrap` now also requires the
  listing's `btcVault` **equal** `custodyAddress`: a frog is wrapped only if the listing's delivery address is custody. The
  owner can move `btcVault` only through `proposeBtcVault`, **7 days** public (`CHANGE_DELAY`), and the moment it differs,
  `wrap` refuses new frogs — frogs already wrapped are unaffected, because custody is the Wrapper's own record, not the
  listing's. This narrows `wrap`; it widens nothing.

### 3. The custodian records its own deliveries
- `Wrapper` gains an immutable `custodian` (an EVM address, the canister's), passed through `WrapClaim`'s constructor.
- `recordDelivery` accepts **the keeper or the custodian**. Nothing else changes: no other function reads `custodian`, and the
  custodian gains no other power — it cannot resolve out of order, refund, wrap, mint, or pause.
- The canister reads `wrapper.custodian()`; when it is ours, it records the delivery itself after Bitcoin confirms (bytes saved
  before sending, one EVM transaction of ours at a time), and the keeper's path stays as the fallback.
- Deploy: `script/DeployWrapClaim.s.sol` takes `CUSTODIAN` and `CUSTODY_ADDRESS`; `script/golive-check.sh` reads both back and
  checks the listing's `btcVault` equals `custodyAddress`.

### Built: custody's own key and the custodian's record (2026-09-23, with the code)

- **Contracts.** `Wrapper` takes `custodian` (immutable) and `custodyAddress` (written once, taproot-checked) in its
  constructor, through `WrapClaim`'s; `recordDelivery` accepts the keeper or the custodian; `wrap` refuses (`NotCustody`) while
  the listing's `btcVault` differs from `custodyAddress`. The ABI pin in `test/security/WrapperSecurity.t.sol` gains exactly
  `custodian()` and `custodyAddress()`. New tests: the custodian records and can do nothing else (not an unresolved ticket,
  not an early refund, not redeem); a zero custodian and a non-taproot custody address refuse construction; a listing whose
  `btcVault` is moved away (7 days) stops wrapping, and moved back resumes it. Both invariant handlers now record as either
  party; the deep profile (200,000 calls each) passes. `GoLiveChecks.checkWrapper` refuses a zero custodian, a non-taproot
  custody address, and one that is not the listing's `btcVault`; the bare dry-run deploys the pair and reads `custodian`,
  `custodyAddress` and the listing's `btcVault` back — **ALL PASS**.
- **Canister.** `BUYER_PATH = b"btc"`, `CUSTODY_PATH = b"custody"`; `custody_address()` / `buyer_address()` print each address
  and output key. The custodian's `our_script` and `sign_taproot` are the custody key's; `may_record` is true when the Wrapper's
  `custodian()` (or the registry's keeper) is our EVM address. The buyer's check runs in both signing paths — the loop's
  `RequestAndVerifyPsbt` and `sign_purchase` — before the quote is bound, so a refused PSBT leaves room for an honest one.
- **Tests.** `tests/loop.rs`: a venue PSBT whose "our" input ord shows carrying an inscription, a rune, or unknown is refused
  with nothing signed or bound, and signed once ord shows it clean; a PSBT listing a custody output truthfully is refused
  `ForeignInputToSign(0)`. `custody.rs`: the verdict on the real inscribed, runed and plain fixtures. `policy.rs`: the bech32m
  encoder reproduces BIP-86's first test vector.
- **Replica** (`jrblj-hp777-77777-aaana-cai`, upgraded in place, state restored): `buyer_address` →
  `tb1p4dntx3hq…2gqglt` (`ab66b346…`), `custody_address` → `tb1pudheuh9m…merkvw` (`e36f9e5c…`): two keys, two addresses.
  An upgrade naming `wrapper = null` left the saved wrapper in place, as the set-once rule says.
- **What remains.** Signet and testnet4 have no keyless ord, so there the buyer's belt-and-braces check is not made (it is
  logged); custody's separate key is the defence there. A delivery sent and confirmed is still not shown on any network.
