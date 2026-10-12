# The buyer's loop: from a decision to buy to a receipt on chain, with nobody calling each step

Written 2026-09-23, **before its code**. `docs/v2/state-machine.md` settled what the buyer does at every state and what it does
when it wakes up in the middle of one; `canister-rs/src/state.rs` encodes that and is tested against every interruption.
What did not exist: anything that *reads the chains* (`observe` returned `VaultState::None` whatever the chain said) or
*does* what the state machine decides (no `Act` had a performer). So nothing ran a purchase on its own. This document is
that missing half. It changes no rule of the state machine; where the code disagreed with the state machine document, the
difference is named below.

## Shape

Three pieces, and the split is what makes the loop testable without a replica:

1. **`evmtx.rs` — pure.** The vault's ABI (the five calls the buyer makes and the one view it reads), RLP, and the EIP-1559
   envelope: the signing hash and the signed bytes. No network, no keys. Checked against Foundry's `cast` for every selector
   and calldata, and against the transaction the rehearsal actually got mined on chain 46630 for the envelope.
2. **`looper.rs` — the loop, over a trait.** `Io` is everything the loop can do to the world: read the vault, read our
   balance and nonce and the fee, sign a hash, save our state, send EVM bytes and read them back, read Bitcoin, send
   Bitcoin, and ask a venue or a bridge. `observe` and `perform` are written against `Io` and nothing else, so every decision
   the loop makes is unit-tested with a fake world — including the fake world dying between any two calls.
3. **`canister.rs` — the real `Io`.** Replicated outcalls, the threshold keys, stable memory. Thin by design: it translates
   and holds no decisions.

## Reading (observe)

- **The vault:** `purchaseCount()` and `purchase(id)` by `eth_call`, through `rpc` — every provider must return identical
  bytes. A purchase id at or past `purchaseCount` is `VaultState::None` (the vault reverts on an out-of-range id; the loop
  never asks). The fields acted on are `status`, `readyAt`, `maxEth` and `returned`; the others are decoded and kept in the
  log, not acted on.
- **Our EVM balance:** `eth_getBalance(latest)`, providers must agree.
- **Bitcoin:** our confirmed sats, and whether the transaction we signed is confirmed, through the Bitcoin module
  (`btcnet.rs`, built beside this; see "Seams"). **Delivered rune units** need a runes indexer, which no keyless replicated
  source offers; the `Io` asks for it and the canister's implementation says it cannot answer (see "Not done").
- Any read that fails or disagrees makes the whole observation fail, and the purchase **waits** this tick. The state machine
  is never fed a guess. (Before, `observe` fed it `None`, which `next` treats as "the vault does not know this purchase".)

## Doing (perform)

### The four EVM calls: announce, pull, settle, returnFunds

These spend or move money on 4663, so they follow the Bitcoin side's rule exactly:

1. **Build.** The nonce is `eth_getTransactionCount(ours, latest)`, and every provider must agree. The fee is
   `maxFeePerGas = 4 × baseFee` of the latest block, taking the **highest** base fee any provider reports (a fee cap, not a
   fact: overpaying the cap costs nothing, the chain charges the actual base fee), `maxPriorityFeePerGas = 0` (Arbitrum has no
   tip market). Gas is `eth_estimateGas` from our address, highest across providers, plus 30%. **An estimate that fails is a
   refusal**: it means the call would revert now (`pull` before `readyAt`, `announce` over the cap), and the loop waits.
2. **Sign** the EIP-1559 hash with the threshold key (`sign_evm_recoverable`: parity derived, low `s`).
3. **Save the signed bytes in the purchase, and let that save be committed, before anything is sent.** On the IC, state
   written before an `await` of an outcall is committed at the `await`. So the order `save → send` means a canister that dies
   during the send wakes up holding the bytes.
4. **Send** (`broadcast_evm`: one node sends, every node reads back) and **act only on the read-back**.
5. On any later tick while those bytes are pending, **send the same bytes again** and read back. Same nonce, so at most one of
   them is ever mined; the B4 rehearsal showed a resend of mined bytes changes nothing.

**One EVM transaction in flight at a time, across all purchases.** The buyer has one EVM key and nonces are sequential: a
second transaction signed while the first is pending takes the next nonce and cannot mine until the first does, and if the
first is ever replaced the second is invalid. So while any purchase holds pending EVM bytes, every other purchase's EVM act
waits. This costs a tick of latency per call and removes nonce bookkeeping entirely.

**When pending bytes resolve:** mined and succeeded → cleared, and the next observation sees the vault move; mined and
**reverted** → cleared and the purchase is `Stuck(Reverted)`, because a buyer that keeps re-signing a call the vault rejects
spends gas for ever; the owner looks, in public. Our nonce is used and our bytes are not visible → **wait**, not rebuild. Only
this loop signs for this key, so that is almost surely a provider behind; and if it is not, rebuilding is how a bridge gets paid
twice. (Changed while writing the code: this line first said "cleared and rebuilt".)

**Settle** is `settleRune(id, amountReceived, btcTxid, satsPaid)`. Inscription purchases (`settleInscription`, with a
collection proof) are not built: every quote in the canister is a rune quote.

**returnFunds** sends what we hold of the purchase, up to `maxEth − returned`, keeping a gas reserve; an announcement that was
never pulled is cancelled with value 0, as the vault expects.

### Announce, and where a purchase comes from

The vault assigns ids; the buyer never invents one. A purchase starts when the operator calls `open_purchase(amount_wei,
quantity, cap_ref)`: the canister reads `purchaseCount()` (providers agree) and that is the id the next `announce` will get,
because only the buyer can announce and the buyer announces one at a time. `open_purchase` is refused while **any** purchase
is unfinished -- one purchase end to end at a time, because the buyer's Bitcoin balance is one balance and the state machine
reads "funded" from it -- so two opens can never be given the same id. After the announce mines, the
loop checks `purchase(id)` has **our** `maxEth` and `quantity`; anything else is `Stuck`.

When to buy — the policy that would call `open_purchase` on its own — is not here. This document is the machinery a policy
drives.

### Bridge, quote, PSBT: behind their traits, refused when absent

`Bridge` and `Venue` are traits (`bridge.rs`, `venue.rs`) with no production implementation: no bridge is chosen and UniSat's
permission for commercial use has not arrived (`venue.rs`). The loop's rule:

> **No purchase is announced unless a venue and a bridge are configured.** ETH that leaves the vault must have a path to BTC
> and a place to spend it. Checked at `open_purchase` and again before `announce` is signed.

If a configured one disappears mid-flight (an upgrade that drops it), the purchase is `Stuck(Unconfigured)` rather than
improvised around. The quote is recorded through `Purchase::record_quote` (#23): once, from `Funded`, before any PSBT. The PSBT
is verified by `Psbt::verify` against that recorded quote and signed only if it passes, with the same binding as
`sign_purchase` (one quote, one transaction).

### Bitcoin: broadcast and read-back

`Act::Broadcast` sends the **saved** signed bytes through the Bitcoin module; `Act::ReadBackBitcoin` reads Bitcoin for the
txid. Neither believes an HTTP answer about a send.

## Seams with the other work in progress

- **`btcnet.rs` / `btcsend.rs`** (Bitcoin broadcast) were built on another branch and merged into this one (`4652700`); the
  loop calls `broadcast_btc` and `btc_tx_seen` through the canister's `Io`. The loop carries txids in Bitcoin's display
  order (`Psbt::txid`), the Bitcoin module in internal order; the `Io` reverses between them, and a test checks the loop's
  finalised bytes against `btcsend::parse_signed`. A `MissingInputs` reply, like every send reply, is a report: only the
  read-back is believed. `broadcast_btc` refuses a network with no checkpoint, which includes Bitcoin mainnet today.
- **Our Bitcoin balance** is read here, not in the Bitcoin module: esplora's `/scripthash/{sha256(script)}` from every
  configured explorer, replicated, through a transform (`chain_stats_only`) that keeps only confirmed funded and spent sums,
  because mempool figures move between the moments different replicas ask. The explorers must agree.
- **`delivery.rs`** is someone else's and untouched.

## Corrections to the state machine code

- `next()` for `Broadcast` contained `o.now > SETTLE_WINDOW_SECS + o.now.saturating_sub(o.now)`, which is `o.now > 259200` —
  true for any real clock. Its intent was "Bitcoin has rejected it outright". It is now just
  `o.btc_tx_confirmed == Some(false)`, with `Some(false)` meaning Bitcoin knows of the txid being **rejected** (its inputs spent
  by a different transaction), never "not seen yet". Not seen yet is `None`, and the loop keeps reading and rebroadcasting.
- `Stuck` gains `Reverted` (one of our EVM calls was mined and reverted) and `Unconfigured` (a venue or bridge vanished
  mid-purchase). Both are terminal, like the others.

## Tests

Unit tests in `looper.rs` with a fake world, one per row of `state-machine.md`'s interruption table that involves an act, plus:
the canister dies after saving signed bytes and before sending → the restart sends **the same bytes**; the send's reply is lost
→ the chain is believed; a read that disagrees → wait, nothing sent; a second purchase's EVM act while one is pending → waits;
a mined revert → `Stuck(Reverted)`; the nonce taken by something else → rebuilt; no venue or bridge → nothing announced; a
failed gas estimate → wait, nothing signed; `open_purchase` twice → refused.

Integration: the loop on a local dfx replica against a local anvil chain with `AutoBuyerVault` deployed there, if the replica
can make outcalls to it. What that run does and does not prove is written into the result section when it has run.

## Not done, said plainly

- **Delivery of rune units cannot be read yet.** No keyless source serves a runes balance through replicated outcalls, so the
  loop can take a purchase to "our Bitcoin transaction is confirmed" and no further on its own. `settle` is built and tested;
  what it cannot yet get is the number it settles with.
- **No bridge, no venue.** Therefore, by the rule above, the live canister never announces. That is the intended behaviour
  until both exist.
- **No fee bump.** A pending EVM transaction whose fee cap falls below the base fee waits for the base fee to fall. Arbitrum's
  base fee has a floor and moves little; `4 ×` is the margin. Replacing a pending transaction means re-signing the same nonce,
  which this does not do.
- **No policy.** Nothing decides on its own to open a purchase.

## Result (2026-09-23): what ran, and what it proves

**Unit tests** (`tests/loop.rs`, 14; `evmtx.rs`, 4; `psbt.rs`, +1): a fake chain that executes our signed transactions against a
model of the vault takes a purchase from open to settled with nobody calling a step, and every send is checked to have been
saved as pending first, with the same bytes. Plus: a death between save and send resends the saved bytes and signs nothing
new; lost replies are rebuilt from the vault, never re-sent; disagreeing providers mean wait, with nothing signed or saved;
one EVM transaction in flight; a mined revert stops; a used nonce with our transaction invisible waits; no venue or bridge,
nothing announced; a vanished venue stops; a call that would revert now is not signed; a vault purchase that is not ours is
refused; the deadline return. The loop's finalised Bitcoin bytes are read by the Bitcoin module's own parser (`btcsend.rs`),
and the two txid byte orders agree. Selectors and calldata match Foundry's `cast` byte for byte; the envelope rebuilds the
transaction the rehearsal got mined on 46630.

**On a local replica against a local chain** (`script/LocalLoop.s.sol`, anvil on 8601, chain 31337; a new canister
`autobuyer_loop`, `kgth3-wx777-77777-aaafq-cai`, installed for 31337 with two local RPC URLs), against the real
`AutoBuyerVault` with the canister's threshold-key address as its buyer:

- `open_purchase` refused with no venue or bridge; with the rehearsal fixtures on it opened purchase 0 at the vault's next id,
  read live; a second open was refused (one at a time).
- `tick` signed `announce` with the threshold ECDSA key, sent it, and read back the mined purchase (`readyAt` from the vault).
- `pull` was refused while the vault's hour had not passed on **the chain's** clock (its gas estimate reverted `TooEarly`), with
  nothing signed; after anvil moved an hour and 300 blocks, `pull` was signed and mined and the canister held the 1 ETH.
- Purchase 0: the vault's deadline was already behind the canister's clock (anvil started ten days back), so the loop took the
  past-deadline branch on its own: `returnFunds`, 1 ETH, vault `Returned`.
- Purchase 1 (anvil two hours behind real time, so the deadline was ahead): announce, pull, then at the bridge step the loop
  **refused** -- "no reference rate is configured: a bridge quote cannot be judged, so none is taken" -- after reading our
  Bitcoin balance from the two signet explorers by script hash, replicated, and getting agreement. `abandon` → `returnFunds`
  signed and mined → vault `Returned`.

**Not shown on a replica:** bridge, quote, PSBT, Bitcoin broadcast and settle. They need a bridge, a venue, a reference rate and
a runes indexer, none of which exist; they run only in the fake world above. Settle's calldata is checked against `cast`.

**Found on the way, outside this work:** `ListingRegistry`'s runtime bytecode is **39,540 bytes**, 14,964 over EIP-170's 24,576
(it embeds both vaults' creation code to deploy them). anvil refused it until started with `--disable-code-size-limit`. Whether
chain 4663 accepts it depends on its configured maximum code size (Arbitrum's default is EIP-170's); it needs answering before
any deployment.
