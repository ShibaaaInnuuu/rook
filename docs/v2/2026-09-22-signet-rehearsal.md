# The signet rehearsal — pre-registration (2026-09-22)

A sibling to `docs/v2/autonomous-buyer.md`. **Written and committed before the code**, so that what counts as a pass is
fixed in advance and cannot be adjusted afterwards to match whatever happened.

`autonomous-buyer.md` closes with seven things that must be true "before this could hold real money". Number 7 is *"A real
Bitcoin broadcast and read-back"*. Nothing in this repository has ever touched a real Bitcoin network: the threshold key has
only ever signed a sighash for a transaction built out of `11…11` and `33…33` txids, verified by a library in the same
process, and thrown away. A signature verifying in a test and a Bitcoin node accepting a transaction are different claims,
and only the second one is evidence that the buyer's Bitcoin half works.

This document pre-registers the rehearsal that closes number 7, on **signet and only signet**.

## The claim to be tested

> The buyer's own key-derivation and signing path — the code that would sign a real purchase — produces a taproot key-path
> spend that a Bitcoin network accepts into a block.

## The steps, and what each one has to show

| # | Step | Passes only if |
|---|---|---|
| 1 | **Derive** the buyer's signet address through `taproot_key()`, the same call the real buyer uses | a `tb1p…` address whose bech32m HRP decodes as `tb`, witness version 1, 32-byte program. A `bc1…` address at any point is a failure, not a warning |
| 2 | **Fund** it | the operator sends signet coin from a faucet. Not automated, not simulated. The rehearsal waits |
| 3 | **Detect** the UTXO from chain data | the txid, vout, and value come from a public signet source, not from anything this repo wrote down |
| 4 | **Build** a transaction spending it | a self-send. Mechanics, not destination |
| 5 | **Sign** it through the buyer's own signing path, with the BIP341 tweak | 64 bytes from `sign_with_schnorr`, `aux = #bip341` |
| 6 | **Broadcast** and **confirm** | a signet node accepts the raw transaction and returns a txid; that txid later appears in a block with height and hash recorded |

Step 6 is the whole point. Steps 1–5 have all been done before in one form or another against fixtures; none of them is
evidence on its own.

## The mainnet guard

The rule is that mainnet must be **refused**, not merely not-selected. A boolean that defaults to false is not a guard: it is
a default that nobody has changed yet. So:

- the Bitcoin network is an explicit parameter, never a default;
- the signing endpoint used for this rehearsal accepts an **allowlist of exactly one value**, `signet`. Not "anything but
  mainnet" — an empty string, a typo, and a future network name all refuse too;
- the derived address is checked by decoding it, not by reading the variable that was meant to produce it. An address whose
  HRP is `bc` fails the run;
- every HTTP endpoint the rehearsal touches is checked against a signet allowlist before the request is made.

The honest limit, stated here so it is not discovered later: **none of this can stop a determined operator**, in exactly the
way `accept_production_key_name` cannot tell a local replica from mainnet. The canister cannot know which network the bytes
it signed were broadcast to. What the guards buy is that reaching mainnet requires editing code and passing a value that no
default supplies — and that it cannot be reached by leaving a field alone. That is all they buy.

## What this rehearsal will NOT cover, whatever its result

Stated in advance so that a green run cannot be read as more than it is. This rehearsal is the **Bitcoin mechanics**, and the
mechanics are the smaller half of a real purchase:

1. **The marketplace round trip.** UniSat's marketplace does not exist on signet — nine POSTs across `runes_types`,
   `runes/auction/list` and `collection/auction/list` on `open-api-signet.unisat.io` return **502 from a gateway with no
   backend**, while `GET /v1/indexer/blockchain/info` on that same host returns `200 ok`. Signet gives chain reads and
   nothing to buy. No amount of signet work will prove a purchase.
2. **The seller's pre-signed input.** A real purchase spends a UTXO we do not own, signed by somebody else, in a transaction
   we did not build. A self-send has one input and we own it.
3. **Verification of a PSBT we did not construct.** `psbt.rs` has 17 tests against PSBTs this repository wrote. A venue's
   real PSBT — its field ordering, its optional fields, its sighash flags, whatever it does that the spec permits and we did
   not anticipate — has never been parsed.
4. **Runes.** A self-send moves satoshis. No runestone is decoded, no asset is delivered, nothing is checked for delivery.
5. **A multi-node subnet.** The key is the local replica's, held by one node agreeing with itself.
6. **The bridge leg**, and everything downstream of it.

So a green run proves the buyer can move Bitcoin. It does not prove the buyer can buy anything.

## Where the code goes

`autobuyer/` only. The rehearsal driver, a signet-network guard module with its own tests, and the signing endpoint on the
Rust canister. Gates: `cd autobuyer && npm test`, and `cargo test` in `canister-rs`.

## Result (2026-09-22, signet)

**A signet node accepted a transaction signed by the threshold key, and block 323256 contains it.** Every step below was
real. Nothing in this section was simulated; where something could not be done, it is in "What this did not prove", not here.

| # | Step | What actually happened |
|---|---|---|
| 1 | Derive | `taproot_key()` → `52888c6d…6c54`; tweaked output key `efafa803…071d`; scriptPubKey `5120efafa803…071d`; address **`tb1pa7h6sqckxnagsapfj4czg3v20yl96309dx3xj63y420a6q6jquws5cs0z7`**, checked by bech32m decode (HRP `tb`, witness v1, 32-byte program) |
| 2 | Fund | the operator sent 2,006 sat from a signet faucet. `eadc2cec…aa5d:1`, confirmed in block **323255** |
| 3 | Detect | read from `mempool.space/signet/api/address/…/utxo`, not from anything this repo wrote down |
| 4 | Build | self-send, 2,006 → 1,895 sat, fee 111 sat at 1 sat/vB. The 111 vB estimate was exact: the mined transaction is 444 weight units |
| 5 | Sign | `rehearse_sign_sighash("signet", …)` → 64 bytes over sighash `f5a15f49…8c21`. Verifies under the tweaked output key; **does not** verify under the internal key |
| 6 | Broadcast + confirm | node returned txid **`3d3ed2734b97895f5a4d32f8b74ebbcc8273803df65c8bbc0fd9832dd40f7f9f`**; block **323256**, hash `00000013aac5e485f63a06e1480576df1a0f70be2e451b5f49c083257ed1b7b8`, `2026-09-22T17:28:24Z` |

Read back from the block rather than believed from the broadcast reply: the transaction appears in that block's own txid
list; its input spends a `v1_p2tr` output of our address; its witness is **one item of 64 bytes**, which is a key-path spend
with `SIGHASH_DEFAULT` and no script path at all.

<https://mempool.space/signet/tx/3d3ed2734b97895f5a4d32f8b74ebbcc8273803df65c8bbc0fd9832dd40f7f9f>

### The bug this caught, and why no test could have

`taproot_key()` returns the **untweaked internal key**. Its own doc comment said "the buyer's taproot output key", and the
results table in `autonomous-buyer.md` said "`taproot_key()` returns the x-only output key". Both were wrong.
`schnorr_public_key` hands back the canister's raw BIP340 key; the subnet applies BIP341's tweak later, at signing time,
through `aux = #bip341`. An address built by treating that key as the output key commits to P while every signature is made
under Q, and is unspendable — the coins would be gone, with no error anywhere to say so.

**No test in this repository could have caught it, and adding more of the same kind would not help.** A test that signs and
then verifies uses the same assumption it used to build the address, so it agrees with itself whichever key it had. The
existing `bip341.test.mjs` and `replica.test.mjs` both pass, both always would have, and both are correct as far as they go.
Only a party that did not share the assumption could disagree, and the only such party is a Bitcoin node.

What replaced the assumption is `checkKeyKind()`: sign a probe hash, ask which key the signature verifies under, and refuse
to print an address unless the answer is the tweaked one. It asks the subnet rather than the documentation.

**This is the general lesson, and it is not about taproot.** An assumption shared by the code and its tests is invisible to
those tests by construction. The EVM half of v2 has not yet met anything that could disagree with it — see below.

### The guards, as exercised

Both refusals were triggered against the running canister, not asserted in a unit test alone:

```
install with btc_network = "mainnet"
  → refused: btc_network = "mainnet". This canister installs for ["signet", "testnet4"] and nothing
    else. Mainnet is not on that list, and adding it is a code change and a review, not an argument.

rehearse_sign_sighash("mainnet", …)
  → refused: caller asked to sign for "mainnet". The only accepted value is "signet", and it is an
    allowlist of one on purpose -- 'anything but mainnet' would accept a typo.
```

The limit pre-registered above held and is worth repeating now that it is not hypothetical: **the canister signs a 32-byte
hash and never sees a network.** Nothing it can check would stop those 64 bytes being broadcast to mainnet by the caller.
The guards make mainnet unreachable by default and by typo; they do not make it unreachable by intent.

### Two operational facts found while running it

- **`cargo` is present on this machine** (`~/.cargo/bin/cargo`, 1.98.1 via rustup, `wasm32-unknown-unknown` installed). An
  earlier agent reported it missing; it is simply not on the default `PATH`. The Rust gate is real and was run.
- **`npm run e2e:v2` destroys a funded address.** It calls `dfx start --clean`, which wipes the replica's threshold key. The
  key is otherwise stable across an ordinary stop/start — verified before the address was published — but a `--clean` run
  while coins sit at a derived address strands them permanently.

## What this did not prove

The four items pre-registered above all stand unchanged, and none of them was touched: **the marketplace round trip**
(UniSat has no signet marketplace to call), **the seller's pre-signed input** (this transaction has one input and we owned
it), **a PSBT we did not construct** (`psbt.rs` has still only ever parsed PSBTs this repository wrote), and **runes** (this
moved satoshis; no runestone was decoded and no asset was delivered). Also unchanged: the key is the local replica's, held
by **one node agreeing with itself**, so no multi-node consensus or transform failure can appear.

## The distance to v2 as the launch path

v2 is now the path to launch rather than a branch, so the gap matters more than it did this morning. Stated plainly:

**This rehearsal proves the canister can hold and spend Bitcoin. It does not prove the canister can be the registered
buyer on 4663.** Those are different halves, and only the Bitcoin one has now met a real network.

`AssetVault` today releases ETH to a purchaser named at declaration, bonded, after a public delay — a person who could walk
away, which is what the bond is for. §"What replaces the bond" has the canister become that registered buyer, fixed at
construction, with the bond gone because a threshold key cannot walk. Everything in that sentence is on the EVM side, and
the EVM side has never been run against anything that could reject it:

1. **Nothing signs an EVM transaction.** `sign_evm` exists and **has no callers**. `tick()` computes an `Act` and logs it;
   `Announce`, `Pull`, `Settle` and `ReturnFunds` are every one of them an EVM write, and none is ever built, signed or
   sent. The vault has never received a transaction from the canister.
2. **The recovery id is missing, and it is exactly the same shape of bug as the taproot one.** `sign_with_ecdsa` returns 64
   bytes, `r‖s`, and no recovery parameter. An EVM transaction needs `v`, and the canister must recover it itself by trying
   both parities and keeping the one that reproduces `evm_address()`. There is no such code — `grep` finds no recovery id,
   no parity, no EIP-155 `v` anywhere in `canister-rs`. Like the taproot mislabel, **no test that does not submit to a node
   will ever notice**: a signature with the wrong `v` recovers to a different address, and only the chain objects.
   EIP-2's low-`s` requirement is unchecked for the same reason.
3. **`observe()` reads nothing.** It returns `Observed { vault: VaultState::None, .. }` — a default. The state machine is
   thoroughly tested against every value that struct can take and has never been given a real one. The vault's logs and
   Bitcoin's view are both still unread.
4. **`refund(purchaseId)`, which the document says anyone may call, does not exist on the deployed vault.** It is in
   `AutoBuyerVault.sol`, compiled under its own Foundry profile so it cannot link into v1, and tested there. Nothing has
   been deployed, and the permissionless-return path has never been exercised against a chain.
5. **The per-purchase cap, `maxInFlight`, the public hour and the overdue freeze** are all proved by Foundry tests against a
   vault in a test harness — 15 of them, one a fuzz. That is a real check of the rules and not a check that the canister
   obeys them, because the canister has never called the vault.
6. **The bridge leg is a fixture behind a trait**, and remains the largest single residual: minutes of relayer custody per
   purchase, and the fee dominating small purchases.

The honest order of work, given (2): **the EVM half needs its own version of this rehearsal before anything else** — a
threshold-ECDSA signature that a real 4663 node accepts into a block, against a test vault, with the recovery id derived and
checked against `evm_address()`. Until something real has rejected or accepted an EVM transaction from this canister, the
EVM half is in the state the Bitcoin half was in yesterday: verified entirely against its own assumptions.

## Reproducing

```
cd autobuyer && npm ci
cd canister-rs && dfx start --background          # NOT --clean if an address is already funded
dfx deploy autobuyer --mode reinstall --yes --argument '(record { …; btc_network = "signet" })'
cd .. && node signet-rehearsal.mjs derive         # prints the address and the faucet, then stops
node signet-rehearsal.mjs watch                   # polls until the operator has funded it
node signet-rehearsal.mjs spend                   # build, sign, broadcast
node signet-rehearsal.mjs confirm                 # wait for the block, read it back
```

Gates: `npm test` (10 tests, 1 skipped for want of the Motoko replica canister) and `cargo test` in `canister-rs`
(55 unit + 2 e2e).
