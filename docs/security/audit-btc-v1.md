# The Bitcoin purchase side, audited

2026-09-22. Scope: `autobuyer/canister-rs` (the buyer canister) and the drivers that call it. Method: the same rubric as
`audit-method-v1.md`, with the platform category pointed at the Internet Computer instead of Arbitrum. Every finding below
was reproduced — the two criticals on a live local replica, before and after the fix.

**The frame.** The design documents are careful: `autonomous-buyer.md` says "spike", `state-machine.md` says "nothing here
is deployed or audited". Both are true. What this pass found is that the distance between the design and the code is larger
than those caveats convey, and in two specific places the code contradicts a sentence the design leans on. That is worth
separating from "it is unfinished", because an unfinished thing gets finished and a contradicted premise gets shipped.

## Summary

**2 critical (both fixed), 1 high, 2 medium, 2 low.** Five further defects were found and four fixed during the parallel
rehearsal work; they are listed at the end rather than re-derived.

| # | Sev | Where | One line | Status |
|---|---|---|---|---|
| **B1** | critical | the whole canister | No caller authentication anywhere: two methods signed arbitrary 32-byte digests for anybody who asked | **fixed, verified on a replica** |
| **B2** | critical | `post_upgrade` | The state the design calls "stable memory" was heap; an upgrade discarded it and silently lifted the pause | **fixed, verified on a replica** |
| **B3** | high | the signing path | The sighash is computed off-canister, so `psbt.rs::verify` — the entire safety argument — is not on the path that signs | **fixed in `etchwrap`** |
| **B4** | medium | `rpc()` | The canister cannot broadcast an EVM transaction at all | open |
| **B5** | medium | `Psbt::verify` | Returns `ours`, a superset of `to_sign` | open, recorded |
| **B6** | low | `Psbt::verify` | `ChangeNotOurs` cannot fire on any transaction that buys anything; the comment claims a check the code does not perform | **test added**, code unchanged |
| **B7** | low | the interface | No way to read whether the buyer is paused | **fixed** |

---

## B1 — a key anybody could use

**Severity: critical.** Direct loss of every coin the canister holds, available to any principal on the Internet Computer,
for the price of one call.

**The defect.** There was no caller check anywhere in the crate — no `msg_caller`, no `is_controller`, no principal
allowlist. The candid interface *was* the access-control policy, and every method in it was open:

- `rehearse_sign_sighash(network, sighash_hex)` returns a BIP340 Schnorr signature over any 32 bytes. A BIP341 key-path
  sighash is 32 bytes. So anyone could build a transaction spending the canister's UTXOs to their own address, compute its
  sighash, hand it over, and be given the signature.
- `rehearse_sign_evm(chain_id, hash_hex)` does the same with ECDSA, and an EVM transaction's signing hash is 32 bytes. The
  v2 vault releases ETH **only to the buyer's address** — which is exactly the address this oracle can spend from.
- `set_paused(bool)`, whose own doc comment calls it "the owner's veto key", could be toggled by anybody. An emergency
  pause could be lifted by the party it was pausing.
- `tick()` could be driven by anybody, at the canister's expense in cycles.

**Why it was not noticed.** Both signing methods carry three guards each, carefully written and well commented, and the
comments call them guards. Every one is about **which network** the key derives for, not about **who is asking**. Once
something is called a guard it is easy to stop counting how many kinds of guard a function needs. Live exposure today was
testnet only — 212,861 signet sats and one rune — because the allowlists name `signet` and chain `46630`. Promoting to
mainnet is a two-constant change, and it would have promoted an open signing oracle with it.

**The fix, and its verification.** `only_controller()` on every update method. Measured on a local replica:

```
$ dfx canister call autobuyer set_paused '(false)' --identity anonymous
  Canister called `ic0.trap` with message: 'refused: 2vxsx-fae is not a controller of this canister.
  Every update method here is the operator's.'
$ dfx canister call autobuyer rehearse_sign_sighash '("signet","00…01")' --identity anonymous
  … same refusal …
$ dfx canister call autobuyer log            # the pause the anonymous caller tried to lift
  (vec { "installed"; "paused" })
```

**What the fix moves rather than removes.** The trust anchor is now the controller set, and a controller can replace the
wasm with one that signs anything. So "a key nobody holds" is exactly as true as "nobody controls the canister", and today
one dfx identity does. `autonomous-buyer.md` already names the answer — a timelocked controller canister, an upgrade
proposed by wasm hash, a public delay, a guardian key whose only power is veto, and a reproducible build so the running hash
can be checked against the published source. None of that is built. Until it is, the honest sentence for the site is *the
operator can change what the buyer does, with a delay of zero*.

---

## B2 — the stable memory that was not stable

**Severity: critical.** The premise of the entire state machine, contradicted by the code implementing it.

**What the design says.** `docs/v2/state-machine.md`, first paragraph:

> A canister can stop at any point — an upgrade, a subnet hiccup, a trap, running out of cycles — and resume with only what
> it wrote to **stable memory**.

Its whole interruption table answers "the canister dies here; what does it do on restart?", and the sharpest row is
`Signed`: *"`Signed(txid, rawTx)` in stable memory → broadcasts the **saved bytes** → no double-spend: same inputs, same
txid."*

**What the code did.** `PURCHASES`, `PAUSED` and `LOG` lived in `thread_local! { RefCell<…> }` — heap. There was no
`pre_upgrade`, no `stable_save`, no stable-structures dependency. `post_upgrade(arg)` called `init(arg)`. An upgrade
therefore reset the buyer to nothing.

Measured on a replica, before the fix — a paused canister with a two-line log, then `dfx deploy --mode upgrade` with a
genuinely different module hash:

```
before:  paused = (true)   log = (vec { "installed"; "paused" })
after:   paused = (false)  log = (vec { "installed" })
```

Two consequences, and the second is worse than the first. The bytes of a signed, unbroadcast Bitcoin transaction do not
survive a restart, so they cannot be rebroadcast; a canister that re-requests a PSBT instead may be handed a *different*
transaction spending the same UTXOs, and one of them may already be in a mempool. That is precisely the double-spend the
table says cannot happen. And **the pause was silently lifted by every upgrade** — an operator who hits the veto because
something is wrong, then ships the fix, restarts the buyer by deploying it, at the one moment they are most certain it is
stopped.

**The fix, and its verification.** A real `pre_upgrade` writing `(Init, Vec<Purchase>, paused, log)` to stable memory, and
a `post_upgrade` that restores it — still validating the install argument, so an upgrade cannot smuggle in a mainnet network
name or a single RPC provider, but taking what the buyer *knows* from before. A `stable_save` that fails traps, because a
refused upgrade is recoverable and a forgotten purchase is not; a restore that finds nothing says so loudly, because the
first upgrade after this ships legitimately finds an empty stable memory and that must not look like a clean boot.

Measured on the replica afterwards, with the module hash confirmed to have changed
(`0x609accb9…` → `0x1b840e7e…`):

```
before:  paused = (true)   log = (vec { "installed"; "paused" })
after:   paused = (true)   log = (vec { "installed"; "paused"; "upgraded: state restored" })
```

---

## B3 — the verifier is not on the path that signs

**Severity: high.** Open.

`psbt.rs` is 716 lines and its module comment is the safety argument for the whole venue integration: we sign only our own
inputs; what leaves us is at most the cap recorded *before* the PSBT was requested; the fee is bounded; the runestone must
deliver the asset to the recipient's script. "A node that lies fails one of these."

Nothing calls it. `Psbt::verify` has no non-test caller. The sighash the canister actually signs is computed **in
JavaScript, off-canister** — `runes-rehearsal.mjs:157`, `tx.preimageWitnessV1(...)` — and handed to
`rehearse_sign_sighash` as 32 opaque bytes. The canister's own comment is honest about the consequence and stops one step
short of naming it: *"the canister signs a hash; it never sees a network."* It never sees the transaction either.

So the property "a single malicious node cannot redirect the asset" is currently enforced by a driver running on the
operator's laptop, not by the replicated program whose whole purpose is to be the thing that cannot be talked into signing.
For the rehearsals this is fine and is what a rehearsal is. It is the single largest piece of work between here and a buyer
that holds real money: the canister must parse the PSBT, verify it against a quote it recorded first, compute the BIP341
sighash itself from the amounts and scripts in that PSBT, and sign only that.

**Closed, 2026-09-22, in the `etchwrap` fork.** `src/sighash.rs` computes the BIP341 key-path digest for `SIGHASH_DEFAULT`
with no annex, and `sign_purchase(psbt_hex, quote, input_index)` parses, runs every check in `psbt.rs`, computes the digest
**from the bytes it just verified**, and signs only that. The loop closes because BIP341 commits to every input's amount and
script and to every output, so a digest derived that way cannot describe a different transaction from the one that passed —
a venue that lies about what our own input is worth now produces a signature that is invalid for the real UTXO rather than a
valid one for a transaction we did not agree to.

Tested against an implementation that is not this one. `gen-sighash-fixture.mjs` builds three transactions with
**@scure/btc-signer** — the library whose signatures signet has already accepted from this project, in blocks 323256 and
323262 — asks *it* for the digest, and the Rust is checked against those four answers across two inputs, a non-default
version, a non-zero locktime, a `vout` of `u32::MAX`, a value above 2^51 and a script long enough to need a two-byte compact
size. A test that checked this module against a second copy of my own reading of BIP341 would agree with itself and prove
nothing. Three more assert the commitment directly: one satoshi of difference on an input, or one bit of a redirected
output script, moves the digest.

`sighash-e2e.mjs` runs the whole path on a replica: the canister's own runestone encoder produces the edict, a transaction
is built around it by a library the canister has never seen, `sign_purchase` verifies and signs, and its digest equals
@scure's exactly. A quote the transaction does not satisfy is refused with `ForeignInputToSign(0)` before the key is touched.

Two things this does **not** close, stated rather than implied:

- ~~`quote` still arrives as an argument rather than out of the purchase's recorded state.~~ **Closed 2026-09-23.**
  `record_quote(id, quote)` fixes the quote on a purchase, only once it is `Funded` and only once: it is never replaced, so
  nothing that arrives after it (the venue's PSBT included) can change what the PSBT is checked against.
  `sign_purchase(id, psbt, input)` takes no quote; it reads the recorded one. And the first signature binds the purchase to
  its transaction (`Phase::Quoted { signing }`), bound before the signature is awaited so two calls in flight cannot both
  pass: one quote pays for one transaction, which "quote from state" alone would not have guaranteed. `sighash-e2e.mjs` shows
  on a replica: no quote, no signature; a re-quote at 100x the spend refused; a second, different transaction under the same
  purchase refused though it would pass every check; an unknown purchase refused; a quote the transaction does not satisfy
  refused before the key is touched. Today the controller supplies the quote; when the loop is wired, `Act::GetQuote`'s
  performer calls the same `Purchase::record_quote` with the venue's quote.
- ~~Nothing is broadcast. B4 is unchanged.~~ **B4 closed 2026-09-23** for the EVM side (`89283b5`): one node sends, every node
  reads back, shown on chain 46630. On the Bitcoin side the broadcast is still the driver's.

**And a footgun found while testing it, now removed.** `taproot_key()` returns the key the subnet derives — which is the
*internal* key — while `sign_taproot` passes `aux: Bip341` so the subnet tweaks before signing and the signature verifies
under a different number entirely. Reading one for the other is the mistake that once left a rehearsal's coins unspendable.
`taproot_output_key()` now returns the tweaked key under its own name, computed in the canister with k256 and checked
against @scure's `p2tr` on every e2e run.

---

## B4 — the canister cannot broadcast an EVM transaction

**Severity: medium.** Open. Found during the EVM rehearsal.

`rpc()` requires every configured provider to return an identical body, which is the right rule for a *read*. An ICP HTTPS
outcall is replicated across the subnet's nodes, and `eth_sendRawTransaction` is not idempotent: one node receives a
transaction hash and the rest receive "already known". Consensus fails and the call traps. So `announce`, `pull`, `settle`
and `returnFunds` have no path to chain 4663 at all, whatever the vault is willing to accept. Both rehearsals broadcast over
plain `fetch` from the driver.

The standard answer is a non-replicated outcall for the send plus a replicated read-back for the truth — the same shape the
design already uses for `create_bid_prepare` — and it is not written.

## B5 — `verify` returns a superset of what was asked for

**Severity: medium.** Open, recorded. `Psbt::verify(q, to_sign)` checks that every index in `to_sign` is ours, and then
returns `ours` — all of our inputs, not the subset requested. A caller that signs what it is handed signs inputs it did not
ask to sign. Bounded by the cap, so a trap rather than a hole, and it has no non-test caller yet (see B3). Settle it when
something real calls it.

## B6 — a check that cannot fire

**Severity: low.** The comment above the change loop says it checks "that we did not get change on a script that is not
ours". The condition ends in `spend == 0`, and `spend == 0` means we kept every sat we put in — which no purchase does. On
any transaction that pays for something the branch is unreachable, and the only thing between us and a stranger's output is
the cap.

The cap is a real bound and the module comment already states overpay-up-to-the-cap as the residual risk, so this is not a
hole. It is a comment describing a check the code does not perform, which is the kind of thing that gets relied on later.
`the_change_not_ours_check_cannot_fire_on_a_transaction_that_buys_anything` demonstrates it: a stranger takes 9,000 sats out
of our input on an otherwise honest transaction, and `verify` returns `Ok`. Left in the code as it is, with the test now
saying what the bound actually is.

## B7 — no way to read the veto

**Severity: low. Fixed.** There was no `paused` query. An operator who wanted to know whether the buyer was stopped had to
infer it from the last `paused`/`resumed` line in a log that keeps only the most recent 500 entries.

---

## Found and fixed during the rehearsals, not re-derived here

Five defects in the runestone and PSBT paths, four fixed, from `docs/v2/2026-09-22-evm-and-runes-rehearsals.md`:
`payload_of` reading past a non-push opcode and returning a partial runestone that `ord` would have burned; rune ids
narrowed with `as`, truncating an out-of-range id into a different valid-looking rune; "two RPC providers" counted without
checking distinctness, making the agreement rule a tautology for anyone listing one URL twice; every unrecognised even tag
read past, settled by a control experiment — two transactions in one block differing only by tag 254, where the control
minted and the variant credited nothing. B5 above is the fifth.

## What the rubric asked that this codebase has no answer to

- **Auditing/monitoring.** `log()` is the whole observability story, it is capped at 500 lines, and until B2 it did not
  survive an upgrade. Nothing watches it.
- **Decentralization.** One controller, no timelock, no reproducible build, so the module hash cannot be checked against
  published source. See B1's closing note.
  **Built 2026-09-23, not yet deployed beyond a local replica** (`docs/v2/timelock-controller.md`): a timelock canister that
  is the buyer's only controller — a public delay (72 h floor), the proposed module's hash readable by anyone, a guardian
  whose only powers are cancel and pause, execution by anyone after the delay — and a Docker build that two clean clones
  reproduce to the same hashes. The operator is now a separate principal from the controllers.
- **Testing and verification.** 74 Rust unit tests and 22 JS tests, all passing, and they are good tests — the runestone
  decoder is validated against a live indexer's verdict on real signet transactions, which is a genuinely strong form of
  differential testing. What is missing is a test that the *canister* refuses a bad PSBT, because the canister does not look
  at PSBTs yet (B3).
