# The autonomous buyer's state machine, and why an interrupted canister cannot buy twice

Written 2026-09-21, **before its code**, as `docs/v2/autonomous-buyer.md` requires. It is the contract between the vault on
chain 4663 and the buyer canister: every state, every transition, what is written before what, and what happens when the
canister halts between any two lines. Nothing here is deployed or audited.

## Why this document exists

A canister can stop at any point — an upgrade, a subnet hiccup, a trap, running out of cycles — and resume with only what it
wrote to stable memory. A buyer that resumes wrongly buys the same rune twice with two vault releases, or pays a seller and
forgets it delivered. Neither is recoverable: Bitcoin does not refund. So the rule below is absolute, and every transition is
tested by replaying it from a saved state:

> **Nothing that spends is attempted until the intent to spend is durably recorded, and every spend is idempotent against
> something the outside world already holds.**

Two outside-world facts do the work. On 4663 the vault's own `State` is the record: `pull` moves `Announced → Pulled` in the
same transaction that sends the ETH, so the chain cannot be in a state where we were paid and it does not know. On Bitcoin the
**signed transaction is its own idempotency key**: the same inputs can be spent once, so re-broadcasting is free and
re-signing the same inputs produces a transaction that either is already confirmed or replaces nothing.

## States

`Purchase` in stable memory, keyed by the vault's `id` (the vault assigns it; we never invent one).

| State | Means | The canister's next act |
|---|---|---|
| `Watching` | a listing can afford a purchase and recipients are known | call `announce` on the vault |
| `Announced(id, amount, readyAt)` | the vault reserved the amount and started the public hour | wait for `readyAt`, then `pull` |
| `Pulled(id, amount, deadline)` | ETH is at our EVM address | get BTC for it |
| `Bridging(id, quoteHash, sentTxHash, expectSats)` | a bridge transaction is signed and broadcast on 4663 | watch **Bitcoin**, not the bridge, for `expectSats` |
| `Funded(id, utxo)` | a confirmed UTXO of ours can pay for the asset | ask the venue for a PSBT |
| `Quoted(id, quote)` | a price and the exact asset are fixed, recorded before any PSBT is requested | request the PSBT |
| `Signed(id, txid, rawTx)` | a PSBT was verified and signed; the txid is known **before** anything is broadcast | broadcast, then read Bitcoin back |
| `Broadcast(id, txid)` | it went out at least once | read back until confirmed or the deadline |
| `Delivered(id, txid, assetId, amount, sats)` | Bitcoin shows the asset at the recipient | call `settle` on the vault |
| `Settled(id)` | the receipt is on 4663 | done; nothing further |
| `Returning(id)` | the purchase failed before any BTC was spent | call `returnFunds` with the full amount |
| `Returned(id)` | the vault has its ETH back | done |
| `Stuck(id, why)` | a bridge that never paid, or a venue that never delivered | stop pulling for this listing; the owner writes off, visibly |

Terminal: `Settled`, `Returned`, `Stuck`. Nothing leaves a terminal state.

## The one-way rule, per transition

Each row is the answer to "the canister dies here; what does it do on restart?" Every one is a test in
`canister-rs/src/state.rs` (`interrupted_*`).

| Interrupted between | On restart the canister finds | It does | Can it double-spend? |
|---|---|---|---|
| deciding and `announce` | no purchase on chain with our id | decides again from scratch | no: nothing was written |
| `announce` sent, reply lost | the vault's log shows `Announced(id)` with our amount | adopts `id` from the log; never announces a second time for the same listing while one is open | no: `announce` reserves against `maxInFlight`, and the canister keys off the **vault's** id |
| `pull` sent, reply lost | the vault says `Pulled` and our EVM balance rose | moves to `Pulled` from the chain's state, not its own memory | no: `pull` reverts on any state but `Announced` |
| `Pulled`, before bridging | ETH at our address, no bridge tx | re-quotes and bridges | no |
| bridge tx signed, reply lost | a nonce may or may not be used | reads our nonce and the tx by hash; if mined, `Bridging`; if not, re-signs **the same nonce**, which replaces rather than duplicates | no: EVM nonces are the idempotency key |
| `Bridging`, BTC not yet seen | no UTXO | keeps watching until `deadline`, then `Stuck(bridge)` | no |
| `Funded`, before quoting | a UTXO, no quote | quotes | no |
| `Quoted`, before the PSBT request | a recorded quote | requests a PSBT and **checks it against the recorded quote** | no |
| PSBT received, before signing | nothing signed | verifies and signs | no |
| **signed, before broadcast** | `Signed(txid, rawTx)` in stable memory | broadcasts the **saved bytes** | no: same inputs, same txid |
| broadcast, reply lost | `Broadcast(txid)` | reads Bitcoin for the txid and for its inputs being spent | no: re-broadcast is a no-op |
| `Delivered`, before `settle` | Bitcoin shows the delivery, vault still `Pulled` | calls `settle` | no: `settle` reverts unless `Pulled` |
| `settle` sent, reply lost | vault says `Settled` | marks `Settled` | no |

The invariant behind the table: **the canister's memory is a cache of two chains, never the source of truth.** On any doubt it
re-reads 4663 (through two RPC providers that must agree) and Bitcoin, and believes them over itself.

## The duplicate-call hazard, answered

UniSat's spec has no idempotency key, and a replicated outcall is made by every node on the subnet — "potentially 13 times"
(ICP docs). Our answer has three parts, and only the first is novel:

1. **Nothing that spends is trusted from an HTTP response.** `create_bid_prepare` / `create_bid` are non-replicated (one node)
   and return an unsigned PSBT. The canister then **verifies that PSBT against the quote it recorded before asking**: the
   asset's own runestone edict must send at least the quoted amount to the recipient's script; our inputs must not pay more
   than `cap`; our change must return to us; the fee must be under a bound; and only our inputs are signed. A signature is
   produced **only** for a PSBT that passes. See `psbt.rs` and `runestone.rs`.
2. **The broadcast is idempotent by Bitcoin's own rule.** `confirm_bid` (or a direct broadcast) can be sent any number of
   times: the transaction spends fixed inputs. The canister **ignores the HTTP answer entirely** and learns what happened by
   reading Bitcoin.
3. **The read-back is the only source of "it happened".** Bitcoin says whether our inputs are spent and whether the
   recipient's output exists. On mainnet ICP that read is the Bitcoin canister (consensus-grade). On this branch, against
   Bitcoin **testnet**, it is a replicated HTTPS outcall to public explorers with a transform that strips volatile fields, and
   **that is weaker** — it is two or more explorers agreeing, not a consensus view. Recorded as such, not glossed.

### What a single malicious node can and cannot do

It serves the non-replicated call, so it sees the request and writes the response.

| It can | It cannot |
|---|---|
| return a PSBT that fails our checks → the purchase fails and retries | make us sign a PSBT that pays more than `cap` |
| return a worse-but-legal price within `cap` → we overpay up to the cap | redirect the asset: the runestone edict must name the recipient's script |
| stall or drop the response → the purchase times out into `Stuck` | take our change: it must return to our address |
| drop the response **after** the venue broadcast the tx → we think it failed | make us believe a delivery happened: only Bitcoin says that |
| learn which rune we are buying and front-run us in the open market | spend our other UTXOs: we sign only the inputs we chose |

The residual is an **overpay up to the cap** and **availability**. Both are bounded, neither is custody. The honest sentence
for the site: *a single node can waste a purchase or make it cost up to its cap, and can never take the money or the asset.*

## What the vault bounds, now that there is no bond

A bond existed because a person could leave with the ETH. A threshold key cannot leave. What remains is a bug, a bridge that
never pays, a venue that lies, or a stuck state, so the bound is structural and lives in `AutoBuyerVault.sol`:

- `pull` sends **only** to the buyer address fixed at construction. Changing it means a new vault.
- **Per purchase**: `amount ≤ capWei`, and the cap expires an hour after it is posted.
- **In flight**: `inFlight + amount ≤ maxInFlight`. One bug costs at most that.
- **Public hour**: `announce` then `pull`, an hour apart, so anyone can see what is about to be bought and the owner can pause.
- **Overdue freezes the vault**: anyone may flag a purchase past its deadline; while any flag stands, no new announcement.
- **Return is permissionless**: anyone may send the ETH back for a pulled purchase; the buyer exposes a public `refund` whose
  destination is fixed to the vault, so no permission is needed to make it whole.
- **The owner may pause and write off, and may never receive or redirect funds.**

## What this pass does not attempt

Inscriptions (runes first — their edicts are checkable arithmetic; an inscription's provenance is an indexer's word and needs
its own design), the timelock controller and reproducible builds, a multi-node subnet, and a real purchase at a venue, which
needs UniSat's written permission (§3.1) that the operator has asked for and not yet received.
