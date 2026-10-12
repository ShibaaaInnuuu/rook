# The EVM and runes rehearsals — pre-registration (2026-09-22)

A sibling to `docs/v2/2026-09-22-signet-rehearsal.md`, written the same way and for the same reason: **committed before the
code**, so that what counts as a pass is fixed in advance and cannot be adjusted afterwards to match whatever happened.

The signet rehearsal closed item 7 of `autonomous-buyer.md` — a real Bitcoin broadcast and read-back — and in closing it
found a bug of a particular shape: `taproot_key()`'s doc comment said "output key" when the subnet returns the *internal*
key. No test in this repository could have caught it, because a test that signs and then verifies reuses the assumption it
is testing. Only a Bitcoin node, which shares none of our assumptions, could disagree.

That rehearsal's closing section named the two places where the same shape is still unexercised. This document
pre-registers both.

---

## Rehearsal one — the EVM half

### Why this one first

`docs/v2/2026-09-22-signet-rehearsal.md` § "The distance to v2 as the launch path" states it: **the EVM side has never met
anything that could reject it.** As of this writing, confirmed again by `grep`:

- `sign_evm` exists at `canister-rs/src/canister.rs:324` and **has exactly one occurrence in the repository — its own
  definition.** No caller, no test, no driver.
- `Announce`, `Pull`, `Settle` and `ReturnFunds` are all EVM writes and none is ever built, signed or sent.
- `observe()` returns `Observed { vault: VaultState::None, .. }` — a default.
- `grep -rniE 'recovery|recid|parity|eip.?155|low.?s|normali[sz]'` over `canister-rs/src` and `canister-rs/tests` returns
  **one hit, and it is the word "allows" inside a test name.** There is no recovery-id code, no parity search, no EIP-155
  `v`, and no low-`s` normalisation anywhere.
- `grep -rni rlp` over `canister-rs/src` returns nothing. No EVM transaction has ever been serialised here.

### The specific assumption to be tested

> `sign_with_ecdsa` returns 64 bytes, `r‖s`, and no recovery parameter. An EVM transaction needs `v`.

This is the taproot bug's family. A signature with the wrong `v` recovers to a **different address**, and a different
address is a perfectly valid address: nothing is malformed, nothing throws, and a test that verifies the signature with the
same public key it signed under will pass whichever `v` it chose. Only a node — which recovers the sender from the
signature and charges *that* account — can disagree.

Two sub-assumptions ride along with it and are tested at the same time:

- **Low-`s` (EIP-2).** For any valid `(r, s)` the pair `(r, n − s)` is equally valid. Ethereum rejects the high half.
  Whether the IC's threshold ECDSA already returns a normalised `s` is not documented anywhere we can rely on, so it will
  be **measured, not assumed**, and normalised regardless.
- **Which parity.** Negating `s` flips the recovery parity, so normalisation and recovery must be done in that order or the
  `v` is wrong in a way that only appears after the fix.

### The network

`https://rpc.testnet.chain.robinhood.com` answers `eth_chainId` with `0xb626` = **46630**, Robinhood's own public testnet.
It is a Nitro (Arbitrum Orbit) chain — `web3_clientVersion` reports `nitro/v3.12.0-rc.3` — whose parent is **Ethereum
Sepolia**: the latest block's `l1BlockNumber` is `0xb36f2f` and Sepolia's tip at the same moment was `0xb36f34`, five
blocks apart.

**46630 is not 4663.** Chain 4663 is Robinhood Chain mainnet, where the real `AssetVault` lives, and it is out of scope
here in the same way Bitcoin mainnet was out of scope for the signet rehearsal.

### The steps, and what each one has to show

| # | Step | Passes only if |
|---|---|---|
| 1 | **Derive** the canister's EVM address through `evm_address()`, the same threshold path a real buyer would use | a 20-byte address, printed. An address is not a secret |
| 2 | **Fund** it | the operator sends testnet ETH. Not automated, not simulated. The rehearsal waits. No faucet is scripted |
| 3 | **Build** an EIP-1559 (type `0x02`) transaction | a zero-value self-send. The point is the signature, not the payload. Its `chainId` field must be 46630 and is checked against the node's own `eth_chainId` before the transaction is built |
| 4 | **Sign** through the buyer's own ECDSA path, and **derive** `v` rather than guess it | the canister returns `r`, `s`, and a recovery id it computed by recovering the public key and comparing the resulting address to `evm_address()`. If neither parity reproduces the address, the run fails rather than picking one |
| 5 | **Recover independently, before broadcasting** | an *independent* implementation in the driver (`@noble/curves`, not the canister's `k256`) recovers the sender from the assembled raw transaction and gets the derived address. Both parities are tried and reported: exactly one must match |
| 6 | **Broadcast** and **confirm** | the node accepts the raw transaction and returns a hash; that hash later appears in a block, and `eth_getTransactionByHash(...).from` — **the node's own recovery, computed by a party that shares none of our assumptions** — equals the derived address |

Step 6's `from` field is the whole point. Steps 1–5 are all done inside this repository and all of them would pass with a
wrong `v` if the wrongness were consistent. The node's `from` is the only number in the table that we did not compute.

### What a pass and a failure each mean

- **Pass:** the recovery id is derived, not guessed, and a real node agrees the transaction came from the address the
  canister derived. The EVM signing path is then in the state the Bitcoin one reached yesterday.
- **Failure, and it is a result not an error:** if `v` has to be found by trying both parities and keeping whichever one
  works, that will be **said plainly**, because it means the code was previously guessing — and now does not have to.
- The measured answer to "was `s` already low?" is recorded either way.

### The mainnet guard

Same rule as the signet rehearsal: mainnet must be **refused**, not merely not-selected, and a boolean defaulting to false
is not a guard.

- The signing endpoint accepts an **allowlist of exactly one chain id**, `46630`. Not "anything but 4663" — an allowlist of
  one refuses a typo, a zero, a string, and a chain invented next year. `4663` and `1` are refused by the same rule that
  refuses everything else, and are additionally named in the error message so the refusal is legible.
- The **installed** `chain_id` must also be 46630. Both the install and the call must say it; either alone is not enough.
- Every RPC URL is checked against an allowlist of exactly one endpoint before a request is made, and against mainnet
  markers (`rpc.chain.robinhood.com`, `chain_id 4663`, `mainnet`) besides.
- The chain id is verified by **asking the node**, never by reading the variable that was supposed to hold it — the same
  discipline that `assertSignetAddress` applies by decoding the address instead of trusting the string.

The honest limit, unchanged and restated because it is not hypothetical: **the canister signs a 32-byte hash and never sees
a network.** Nothing it can check would stop those bytes being broadcast to 4663 by the caller. The guards make mainnet
unreachable by default and by typo; they do not make it unreachable by intent.

### What rehearsal one will NOT cover, whatever its result

Fixed in advance so a green run cannot be read as more than it is:

1. **The vault.** This is a self-send to an EOA. No contract is called, no `AssetVault` exists on 46630, and `Announce`,
   `Pull`, `Settle` and `ReturnFunds` remain unbuilt and unsent.
2. **`observe()`.** It still returns a default. The state machine still has never been given a real reading.
3. **Replicated broadcast.** The driver broadcasts over plain `fetch` from this machine, exactly as the signet driver did.
   The canister's own `rpc()` helper requires every provider to return an identical body, which a broadcast cannot satisfy
   — see "Further findings" below. That gap is *named* by this rehearsal and not closed by it.
4. **Chain 4663.** Nothing here is evidence about the chain the vault actually lives on beyond "the signature format is
   right". 46630 is Nitro; 4663 is also Nitro; that is an argument, not a proof.
5. **A multi-node subnet.** The key is the local replica's, held by one node agreeing with itself.
6. **Gas under contention, reorgs, and L1 settlement.** A testnet with an empty mempool tests none of them.

---

## Rehearsal two — runes on signet

### Why, and what is different from the first attempt at this

The signet rehearsal moved satoshis. Runes are the other half of what Etch buys, and `runestone.rs` — 348 lines whose
whole job is deciding what a Bitcoin transaction actually moves — has **only ever been run against payloads this
repository wrote in its own test helpers.** That is the same closed loop as before.

The signet pre-registration said, correctly at the time, that no rune was reachable on signet without a marketplace. That
turns out to be wrong in a useful way, and the correction is recorded here rather than quietly used:

- `open-api-signet.unisat.io/v1/indexer/runes/info-list` **answers without an API key** (`code: 0`, real data). UniSat's
  *marketplace* endpoints 502 on signet; its *indexer* does not. Signet carries **2,271 runes**.
- Among them, **UNCOMMON•GOODS, rune id `189444:15`**, has open mint terms: `amount 1`, `cap
  340282366920938463463374607431768211455`, no height or offset bounds, divisibility 0, 20,822 mints so far.

An open mint is reachable **with no marketplace, no counterparty and no permission**: a mint is a runestone and nothing
else. So the fallback the operator authorised — "prove only that the encoder produces an `OP_RETURN` a node relays" — is
not the ceiling. The full loop is reachable, and this document registers the full loop as the target and the `OP_RETURN`
result as the floor.

### The claim to be tested

> The buyer's runestone encoding is what the Runes protocol says it is — not merely what this repository's decoder will
> read back — and its decoder reads correctly from bytes that came off a chain.

### The steps, and what each one has to show

| # | Step | Passes only if |
|---|---|---|
| 1 | **Encode** a mint runestone for `189444:15` **in the canister's own Rust**, reached the same way the buyer would reach it | a script beginning `OP_RETURN OP_13`, produced by `Runestone::encode`, not by the driver |
| 2 | **Broadcast** it on signet, spending the existing UTXO | a signet node relays it and a block contains it |
| 3 | **Ask a third party what it means** | UniSat's signet **runes indexer** — a real `ord` implementation that shares none of our assumptions — credits our address with **1 unit of UNCOMMON•GOODS**. This is the step that can disagree, and it is the reason this rehearsal exists |
| 4 | **Encode an edict** transferring that unit, and broadcast it | a second block contains it |
| 5 | **Verify delivery by output, not by address** | the indexer reports the unit on the **specific UTXO the edict named**, and no longer on the one it came from |
| 6 | **Decode the chain's own bytes** | the raw transaction is re-read from the signet API and fed to the canister's `Runestone::decode`, which must report exactly the edict that was encoded. The bytes decoded are the ones a block contains, not the ones we held in memory |

### The floor, if the ceiling is unreachable

If the mint is refused, unindexed, or the indexer will not answer, the run falls back to proving **only** that the encoder
produces an `OP_RETURN` a node relays and a block contains, with the rune it names uncredited. Which of the two was
achieved will be stated explicitly and in those words. A partial result will not be written up as a whole one.

### What rehearsal two will NOT cover

1. **The marketplace round trip.** Still nothing to buy on signet; a mint is not a purchase.
2. **The seller's pre-signed input.** Every input in both transactions is ours.
3. **A PSBT we did not construct.** `psbt.rs` has still only ever parsed PSBTs this repository wrote. The runestone inside
   one will now have been on a chain; the PSBT around it will not.
4. **Etching.** A *named* etch needs a commit/reveal with a tapscript, and `sign_taproot` is key-path-only by design
   (`merkle_root_hash: vec![]`). Out of reach, and correctly so.
5. **Divisibility and large amounts.** UNCOMMON•GOODS is divisibility 0 and we will move 1 unit.

---

## Where the code goes

`autobuyer/` and `docs/v2/` only. Gates before each commit: `cd autobuyer && npm test`, and `cargo test` in `canister-rs`
(`~/.cargo/bin/cargo`, which is not on the default `PATH`). Baselines at the time of writing: **Node 10 (9 pass, 1
skipped)**, **Rust 55 unit + 2 e2e**.

`npm run e2e:v2` must not be run: it calls `dfx start --clean`, which destroys the replica's threshold key, and that key
owns 213,421 signet satoshis and is the subject of both rehearsals.

## Further findings to be reported either way

The operator asked for more of the same shape — an assumption the code and its tests hold in common. Whatever the two
rehearsals return, the write-up will state what was examined and found **sound** as well as what was found broken, because
a list of only bugs is not a report on where the assumptions are.

---

# Results (2026-09-22)

Written after the fact against the table above. Where a step was not reached, it is in "What remains unproved", not here.

## Rehearsal one — the EVM half

**Block 122,882,989 of chain 46630 contains a transaction signed by the canister's threshold key, and the node recovered
the buyer's own address from it.** Every step was real. The claim was in fact settled twice: once before any funding
existed, from a node's refusal, and again from a mined transaction once the operator funded the address.

| # | Step | What actually happened |
|---|---|---|
| 1 | Derive | `evm_address()` → **`0x5b0b0649f0306d08b4c0ad37406a7082fc25ebc2`**. The node at `rpc.testnet.chain.robinhood.com` was asked `eth_chainId` first and answered `46630`; the address was not printed until it had |
| 2 | Fund | the operator sent **0.01 ETH** (`10^16` wei). Not automated: the faucet sits behind a browser challenge and no faucet is scripted here |
| 3 | Build | EIP-1559 type `0x02`, chainId 46630, nonce 0, zero value, to itself, gas 40,956 at up to 40,000,000 wei (base fee 10,000,000). 107 bytes |
| 4 | Sign + derive `v` | the mined transaction: `rehearse_sign_evm(46630, 66dca7bb…5fb2)` → `r = ed058429…d774`, `s = 36676166…0e86`, **`recovery_id = 0`, derived by recovery and checked against `evm_address()`**. `s_was_high = false` |
| 5 | Recover independently | `@noble/curves` (not the canister's `k256`) recovered **parity 0 → `0x5b0b0649…ebc2` (ours)** and **parity 1 → `0xbbbbd575c73a11ca7f16212d8ada87e09c05d832`** — a different, entirely valid address. Exactly one parity is us, and the two libraries agreed on which, independently |
| 6 | Broadcast + confirm | the node accepted **`0xb0de664d…95ff`**, and block **122,882,989** (`0x84efb546…449a`) contains it — listed in that block's own transaction list. `status 0x1` (success), 25,399 gas, `yParity 0x0`. And **`eth_getTransactionByHash(...).from` = `0x5b0b0649f0306d08b4c0ad37406a7082fc25ebc2`**, the address the canister derived. A second session ran the same driver minutes earlier and put **`0x5c104811…ee3f`** in block **122,882,924** with the same result |

The `from` field is the one number in that table this repository did not compute. It is the node's own recovery, in Go,
in a Nitro sequencer that has never heard of this code.

### The same evidence, obtained before the funding arrived

A node recovers the sender from the signature **before** it looks up a balance, so it has to name an address in order to
say the balance is too low. Submitting the fully signed transaction from an empty account is therefore not a wasted call;
it is the experiment, run for free:

```
eth_sendRawTransaction -> {"code":-32000,"message":"insufficient funds for gas * price + value:
  address 0x5B0b0649F0306D08b4C0aD37406a7082fc25Ebc2 have 0 want 1638240000000"}
```

That address is the one `evm_address()` derives, and the node computed it from the signature alone — **before the account
had any balance at all**. Worth keeping as a technique: it makes the experiment repeatable at zero cost and without
waiting on a faucet.

The outcomes are distinguishable, which is what makes this evidence and not a formality: for that transaction `v = 0`
would have recovered `0x6f542acff38ff24cd8c4de725641f2797a4f75cc`, an address equally valid, equally empty, and not ours.
A wrong `v` would have produced the same error message naming that address instead.

### Was `v` searched for, or known?

**Searched for, and it had to be.** Stated plainly because the pre-registration said it would be.

`sign_with_ecdsa` returns `r‖s` and nothing else, so there was no `v` to be wrong about before this work — there was no
`v` at all, and no code anywhere in `canister-rs` that could have produced one. `sign_evm_recoverable` now recovers a
public key under each parity and keeps the one whose address is `evm_address()`, erroring if neither is.

The search is not a formality. Over **24 signatures from the subnet, the parity was `0` twelve times and `1` twelve
times.** A hardcoded `v` would have been wrong for half of all transactions the buyer ever sent, and every test in this
repository would still have passed, because a test that verifies a signature against the key it signed with agrees with
itself under either parity.

### Low `s`

**Measured, not assumed.** Across 25 signatures (24 in the survey plus the one that was broadcast), `s_from_subnet` was in
the lower half of the curve order **every time**: the IC's threshold ECDSA appears to normalise already. That is evidence
and not a guarantee, so the normalisation stays, and `s_was_high` is reported on every call so the day it changes is a
number in a log rather than a surprise.

One measured fact settles how the two interact, and it came out the opposite way from the guess in the code comments:
k256's recovery **refuses a high-`s` signature under both parities** rather than quietly returning the other key. So if
the subnet ever does return a high `s` and normalisation were skipped, the parity search would find nothing and the call
would error — loud, not wrong. The reverse order is the dangerous one, and it is the order the code does **not** use:
negating `s` after fixing `v` leaves a signature that recovers perfectly well, to somebody else.

### The guards, as exercised against the running canister

Not asserted in a unit test alone — these were run against the installed canister:

```
rehearse_sign_evm(4663,  …) → refused: caller asked to sign for chain 4663. The only accepted value is
                              46630 (Robinhood Chain testnet), and it is an allowlist of one on purpose --
                              'anything but 4663' would accept a typo. That is Robinhood Chain MAINNET,
                              where the vault and the money are. Never.
rehearse_sign_evm(1,     …) → refused (same allowlist, no mainnet clause)
rehearse_sign_evm(46631, …) → refused: the near miss a denylist would have waved through
rehearse_sign_evm(0,     …) → refused
rehearse_sign_evm(46630, "a5a5") → an EVM signing hash is 32 bytes; got 2
```

## Rehearsal two — runes on signet

**A rune was really minted.** An `ord` indexer — a party that shares none of this repository's assumptions — credits the
buyer's address with 1 UNCOMMON•GOODS as a result of a runestone the canister encoded and the threshold key signed. That
is the pre-registered **ceiling**, not the floor: the claim is not "a node relayed an `OP_RETURN`", it is "the bytes meant
what the buyer thought".

| # | Step | What actually happened |
|---|---|---|
| 1 | Encode | `rehearse_runestone_encode({"mint":"189444:15","pointer":0})` → **`6a5d081484c80b140f1600`**, by the canister's own Rust, and byte-identical to `runestone.mjs`'s independent encoding |
| 2 | Broadcast | a signet node relayed **`deee13a785e33a743b7c85e46dcbc7e275eb1d9eb265f123ba21423dbfa5a66c`** — in 211,526 sat, out 546 sat to ourselves, the runestone at 0 sat, 210,814 sat change, 166 sat fee. Mined in block **323,261** (`0000000688eb07baa73467a1db2582fbf1039b31dcd89af83741bbe9bd46c687`), and listed in that block's own txid list |
| 3 | **Indexer** | UniSat's signet runes indexer: **`{"rune":"UNCOMMONGOODS","runeid":"189444:15","amount":"1","divisibility":0}`** at `tb1pa7h6…s0z7`, placed on `deee13a7…a66c:0` — the output the pointer named |
| 4 | Encode an edict | `{"edicts":[{"id":"189444:15","amount":"1","output":0}],"pointer":2}` → **`6a5d0916020084c80b0f0100`**, both encoders agreeing |
| 5 | Broadcast the transfer | **`8730ef716246308292ba4710a5363baa6366b8191a7bb7ee339cdc1e55040bf9`** — two inputs (the rune-bearing 546 sat and a 210,814 sat fee UTXO), out 0 the edict's named recipient at 546 sat, out 1 the runestone, out 2 the change the pointer names, 225 sat fee |
| 5b | Confirm the transfer | mined in block **323,262** (`0000000044578d8ede9e9a374539fb2fae0157da59361817d5bccfb90c189dcd`), listed in that block's own txid list |
| 6 | **Delivery by output, not by address** | the indexer puts **1 UNCOMMON•GOODS on `8730ef71…0bf9:0`** — output 0, *the output the edict named*, and not output 2, which is where the pointer sent the remainder. Address-level accounting could not tell those apart; this can |
| 7 | Decode the chain's own bytes | the canister's `Runestone::decode`, given the output scripts **re-read from the block**, returned `{"cenotaph":false,"edicts":[{"amount":"1","id":"189444:15","output":0}],"has_spread":false,"pointer":2}` — exactly the edict that was encoded, recovered from bytes a block holds rather than from anything this process remembered |

### Two encoders, written from different sources, agreeing

The bytes are not the canister's word alone. `autobuyer/runestone.mjs` is a second implementation written from the
protocol description rather than from the Rust, and the driver refuses to broadcast unless the two agree:

```
                      mint                        edict
canister-rs (Rust)    6a5d081484c80b140f1600      6a5d0916020084c80b0f0100
runestone.mjs (JS)    6a5d081484c80b140f1600      6a5d0916020084c80b0f0100
```

Its tests assert hand-computed constants rather than a round trip — `14` (tag 20, Mint) `84 c8 0b` (block 189,444) `14`
`0f` (tx 15) `16 00` (tag 22, Pointer, output 0) — for the specific reason that a round trip between an encoder and its
own decoder passes whichever convention the two share, which is the exact failure the taproot mislabel was.

**Worth deflating, though:** both implementations were written by the same author in the same afternoon, which is weaker
than two written by different people. Two of the hand-computed constants were wrong on the first pass (`leb128(840000)`
and `leb128(2**64)`) and the implementation was right both times; the constants were corrected against an independent
calculation. The genuinely external party is the `ord` indexer, and that is what step 3 is.

### What the decoder cannot see

`Runestone::decode` acts on exactly two tags, Body and Pointer, and reads past everything else — which the mint proves
directly: the transaction that minted a rune decodes to `edicts: []`, because the Mint tag is invisible to it. For the
buyer's own purpose that fails in the safe direction (a PSBT that mints instead of delivering fails
`AssetNotDelivered`), but it is the doorway to finding 5 below.

---

## Further findings: more of the same shape

The operator asked for more assumptions held in common by the code and its tests. Four were found by reading, all of the
same family — a check that the tests could not fail because every fixture satisfied it by construction. Three are fixed
here; one is recorded because fixing it belongs to whoever writes the first real caller.

### 1. A runestone script with a non-push opcode was read as a partial transfer, not a cenotaph — **fixed**

`payload_of` stopped at the first byte that was not a data push and returned the pushes it had already read, with the
comment *"anything else ends the payload; the protocol treats it as a cenotaph, caught by the caller"*. The caller did not
catch it and could not: nothing was passed back to catch. `Runestone::decode` called `payload_of(s).unwrap_or_default()`
and carried on.

`ord` treats such a script as a **cenotaph and burns the runes in it**. So the divergence is not cosmetic: a venue could
hand us a PSBT whose runestone this code reads as *"delivers 5,000 units to the winner's output"* while every indexer in
the world reads it as a burn. `Psbt::verify` would have found the delivery satisfactory and signed.

Why no test could find it: **every fixture in the repository builds its runestone scripts out of pure data pushes**,
because that is what a correct encoder emits. The encoder, the decoder and all nineteen tests shared the assumption that a
runestone script contains only pushes. Nothing inside the loop disagrees with it.

Now `payload_of` returns `Valid` or `Invalid`, `Invalid` is a cenotaph, `OP_0` and `OP_PUSHDATA4` are recognised as the
pushes they are, and a push that runs off the end of the script is a cenotaph too. Nine new assertions cover it.

### 2. Out-of-range rune ids were truncated into different, valid-looking runes — **fixed**

An edict's id fields are `u128` on the wire and were narrowed with `as u64` / `as u32`. `as` wraps. A block delta of
`u64::MAX + 1` became `0`, which names **a different rune**, and the decoder would then report a delivery of something
nobody sent — which `Psbt::verify` would accept. `ord` calls an out-of-range id a cenotaph. Now so does this, via
`try_from`.

### 3. "Two RPC providers" counted entries without checking they were different — **fixed**

`init` asserted `rpc_urls.len() >= 2`, and `rpc()` accepts an answer only when every provider returns an identical body.
One URL listed twice satisfies both, trivially and always: the consensus check becomes a tautology while still looking
like a check. This was not hypothetical — `e2e-v2.mjs` listed `https://rpc.mainnet.chain.robinhood.com` twice and
`https://mempool.space/signet/api` twice, and it was the path of least resistance when installing for this rehearsal too.
`init` now requires the entries to be distinct, on both lists.

### 4. Every unrecognised even tag was read past, where `ord` burns the runes — **fixed, and measured with a control**

`Runestone`'s `cenotaph` field is documented as *"True when a tag we do not understand appeared … the protocol calls such
a runestone a cenotaph and **burns** the runes in it. The buyer must never sign one."* The code did not do that. `decode`
acted on Body and Pointer and read past everything else — so the doc comment described the protocol correctly and the
implementation had disagreed with its own documentation since it was written.

Rather than implement a tag allowlist from memory of a specification, this was **put to a real indexer**. Two
transactions, both in signet block 323,262, byte-identical but for three bytes:

```
deee13a7…a66c   6a5d08        1484c80b140f1600    → indexer credited 1 UNCOMMON•GOODS
904e55c3…8f4a   6a5d0b fe0100 1484c80b140f1600    → indexer credited nothing
```

`fe 01` is tag 254 and `00` is its value; the mint tag, the rune id and the pointer are identical in both. The first is
the **control** and it minted. The second did not. Runes follows the usual forward-compatibility rule — unrecognised
**odd** tags ignored, unrecognised **even** tags a cenotaph.

The exploit is the `payload_of` one again: a venue adds one even tag to an otherwise honest runestone, this code reports
*"delivers N units to the winner"*, `Psbt::verify` signs it, and every indexer in the world burns the asset. No test could
have found it — every fixture used tags the decoder knew.

Fixed, and **validated against the chain rather than against itself**: replayed over the three real transactions, the
decoder now returns `cenotaph: false` for the mint, the edict for the transfer, and `cenotaph: true` for the probe, which
is what the indexer says about each of them.

One deliberate narrowing, flagged because it is a trade and not an oversight: the even-tag allowlist holds only **Body,
Mint and Pointer** — the three that were measured. `ord` also recognises the etching tags, so a legitimate runestone
carrying one is **refused** here rather than accepted. Refusing a valid runestone costs a purchase; accepting an invalid
one costs the asset. Widening it is a change to make against `ord`'s source.

### 5. `Psbt::verify` returns every input of ours, not the ones the venue asked us to sign — **recorded, not fixed**

Its doc says it will "say which inputs may be signed" and its signature takes `to_sign`, but it returns `ours` — a
superset. A caller that signs what it returns would sign inputs the venue never asked about. The exposure is bounded
(`spend = ours_in − ours_out` counts every input of ours and is checked against the cap), so this is a trap rather than a
hole, but it is exactly the shape in question: the e2e fixture has **one** input of ours, so `ours` and `to_sign` are the
same list in every test that exists, and the divergence is invisible.

Not fixed here because `Psbt::verify` **has no non-test caller** — `grep` finds it in `tests/e2e.rs` and `psbt.rs`'s own
tests and nowhere else. The right time to settle whether it returns `ours` or `to_sign` is when something real calls it.

### What was examined and found sound

Listed because a report of only defects says nothing about where the assumptions are:

- **The delta encoding of edict ids.** The rule — block delta always, tx delta only within one block, absolute across
  blocks — is implemented the same way on both sides, and the new encoder's output decodes back to the same ids under
  reordering. Confirmed against an independent JS implementation as well as the round trip.
- **The cap arithmetic in `Psbt::verify`.** `spend = ours_in − ours_out` counts every input of ours and every output
  carrying our script, so an extra input or a stranger's output is bounded by `max_spend_sats` rather than escaping it.
  The e2e suite's four hostile-PSBT cases each fail on the most specific refusal available.
- **`has_spread`.** `ord` distinguishes `output == num_outputs` (spread) from `output > num_outputs` (cenotaph); this
  refuses both. Conservative in the safe direction.
- **Duplicate-key rejection in PSBT maps**, and the refusal of an unsigned transaction that carries scriptSigs or uses the
  witness serialisation marker. Correct per BIP 174 and the right failure direction.
- **The BIP341 signing path**, re-exercised: every signature in the runes rehearsal verifies under the tweaked output key
  and fails under the internal key, which is the check `checkKeyKind` installed after the taproot bug.
- **`assert_chain`**, which compares the node's `eth_chainId` against the installed one on every read. It was already the
  right discipline; the EVM driver now applies the same one from the outside.

### A gap this rehearsal names but does not close

**The canister cannot broadcast an EVM transaction at all**, and nothing had noticed because nothing had tried.

`rpc()` asks every configured provider and refuses the answer unless all bodies are identical — correct for reads, which
is all `observe()` does. But an ICP HTTPS outcall is made **by every node in the subnet**, and `eth_sendRawTransaction` is
not idempotent: the first node to arrive gets a transaction hash and the rest get *"already known"*. The bodies differ,
consensus fails, and nothing is sent. `strip_volatile` drops headers, not this.

So `Act::Announce`, `Pull`, `Settle` and `ReturnFunds` have no path to a chain even once someone writes the code to build
them. Both rehearsals broadcast over plain `fetch` from this machine, as the signet one did. Closing this needs a
transform that normalises the "already known" reply into the success reply, and it is the next piece of EVM work after
this.

---

## What remains unproved

For rehearsal one, beyond the six items pre-registered above (all of which stand):

- ~~**No block contains a transaction from this canister.**~~ **Closed, 2026-09-22, after the operator funded the
  address.** Chain 46630 block **122,882,924** contains
  `0x5c1048110ba1ca609d359de1d36a83e921fb04090934d2a548a45372ddcaee3f`, status `0x1`, 25,399 gas, `yParity` `0x0`, and the
  transaction is listed in that block's own transaction list rather than only answering a by-hash lookup.
- ~~**`eth_getTransactionByHash(...).from` has not been read.**~~ **Closed.** The node says
  `from = 0x5b0b0649f0306d08b4c0ad37406a7082fc25ebc2`, and `evm_address()` derives the same. A Nitro sequencer recovering
  the sender in Go from the signature alone, and the canister's own k256 recovery, now agree on a *mined* transaction and
  not only on a refusal message. `@noble/curves` recovered parity 0 to the same address and parity 1 to
  `0x74b0cafe930ad9d2b7951d5a4934fea40244e0ee`, so the parameter discriminates.
- The canister still calls `ecdsa_public_key` once per signature to check its own recovery. Correct, and wasteful; a
  production buyer should cache its own address.
- **One parity per transaction, and both have now been through a node.** The two mined transactions each needed `v = 0`;
  the unfunded preflight needed `v = 1`. So both parities have been accepted by a node, but only `v = 0` has been through
  a block.
- **The address was not exclusively this rehearsal's, and that is worth saying rather than tidying away.** Two mined
  transactions exist, one per agent session, at nonce 0 and nonce 1 — see "A note on the environment" below. Each session
  built, signed and broadcast its own, and each verified its own hash against what the node returned, so neither result
  rests on the other. But an experiment that quietly assumes exclusive control of its subject should say when it did not
  have it.

For rehearsal two, **the pre-registered ceiling was reached** — a rune was minted, transferred by edict, and placed by an
indexer on the output the edict named. The four items pre-registered as out of scope all stand unchanged: no marketplace
round trip, no seller's pre-signed input, no PSBT this repository did not construct, no etching. Beyond those:

- **`Psbt::verify` has still never seen a real runestone.** The runestone inside a PSBT has now been on a chain; the PSBT
  around it has not. The verifier's 17 tests still only parse bytes this repository wrote, which is the one remaining
  closed loop of exactly the kind this exercise is about.
- **One indexer, not two.** UniSat's signet indexer is the only `ord` implementation consulted. It is genuinely a third
  party and it disagreed with us twice, which is the point — but a second one would be better.
- **Divisibility and large amounts are untested.** UNCOMMON•GOODS is divisibility 0 and one unit moved.
- **The even-tag allowlist is narrower than `ord`'s** (finding 4), so some valid runestones are now refused.

---

## A note on the environment these were run in

A second agent session was working in this repository throughout, and it matters to how these results should be read.

It **audited the canister and found a real bug in what this rehearsal added** (commit `3862c6d`): `rehearse_sign_sighash`
and `rehearse_sign_evm` had no caller check, and neither did anything else — the candid interface *was* the access-control
policy, so **every method was callable by any principal on the Internet Computer**, two of them signing. A blind signing
oracle over 32 arbitrary bytes, offered to the world. Its `only_controller()` fix is kept here unchanged, and the
rehearsal was re-run on top of it.

That is worth stating plainly rather than burying: this document spent its length on assumptions the code and its tests
held in common, and the code it added carried one of its own — *"only we will call this"* — which nothing in the file
checked and no test could fail. It was caught by a reader, which is the other way these get caught.

It also **closed rehearsal one independently, minutes earlier**, using this driver: commit `b366ccd` records
`0x5c104811…ee3f` in block 122,882,924 at nonce 0, and this document's own result is `0xb0de664d…95ff` in block
122,882,989 at nonce 1. Two transactions, two sessions, the same conclusion reached twice by the same method. That is a
replication and not a conflict — but it is also why the nonce jumped, and guessing at it would have been worse than
finding the commit that explained it.

Two other practical consequences: the canister was reinstalled several times by both sessions — the threshold key
survives that, and `taproot_key()` returned `52888c6d…6c54` before and after every one, with the replica's root key
unchanged throughout, so nothing was ever run with `--clean` — and one session's install named `dfx_test_key`, a key this
replica does not have, which is what surfaced the collision. A shared replica and a shared canister id are a real
hazard for two agents working at once, and neither had any way to see the other coming.

## B4: the canister broadcasts (pre-registered 2026-09-23, before it runs)

**The finding.** The canister could sign an EVM transaction but not send one. A replicated HTTPS outcall is made by every node
and their replies must agree; to `eth_sendRawTransaction` the provider answers the first copy with the hash and every later
copy with "already known", so consensus fails and the canister is told the call failed about a transaction that was sent.

**The change** (`autobuyer/canister-rs/src/evmsend.rs`, `canister.rs`). `ic-cdk` 0.18 → 0.19, which brings
`ic-management-canister-types` 0.5 and its `is_replicated` field. The send is made by **one** node (`is_replicated: false`) to
each provider, and its reply is only classified and reported. What is believed is the read-back: `eth_getTransactionReceipt`
and `eth_getTransactionByHash`, replicated, every provider agreeing, for the hash the canister computed itself (the keccak of
the bytes). The bytes must be signed for the chain the canister was installed for; the RPC must be that chain too. Every
existing read is now explicitly `is_replicated: true`. `rehearse_broadcast_evm` is the rehearsal's entry point, behind
`rehearse_sign_evm`'s guards; `evm_tx_status` asks again later.

**The rehearsal.** `node evm-rehearsal.mjs canister-send` on chain 46630: the same zero-value self-send as `send`, built and
signed the same way (nonce 1 now), but handed to the canister to broadcast. Then the canister is asked, replicated, until it
reports the transaction mined.

**Pass**, all four:
1. the canister's call returns (no consensus failure) with a report from both providers;
2. the canister's replicated read-back reports `Mined` with `success = true`;
3. the hash it reports equals the keccak the script computed, independently;
4. the node's own `eth_getTransactionByHash(...).from` is the canister's address — read by the script, not the canister.

**Fail** is any of: a consensus error on the send (B4 not fixed); a `Mined` that the script's node does not confirm; a different
hash; a different sender. A `NonceTooLow` or `Refused` from one provider is recorded, not a failure, if the other has it and
the chain mines it.

### B4 result (2026-09-23): **pass**, after one fix the rehearsal found

The canister broadcast `0x2eebfa6e…2a6b00` (nonce 2, a zero-value self-send on chain 46630) through `rehearse_broadcast_evm`.
The call returned without a consensus failure. Against the four pre-registered criteria:

1. **Returned, no consensus failure — pass.** The per-provider report of this first send was not recorded: the script
   misread `dfx`'s JSON for a candid tuple record and stopped before printing it (fixed in the script, not the canister).
2. **Canister's replicated read-back reports `Mined`, `success = true` — pass, after a fix.** The first read-back said
   `Unsure: providers disagree on eth_getTransactionReceipt`. Both providers had the same receipt, but Robinhood Chain's own
   node adds Arbitrum's `gasUsedForL1` and `l1BlockNumber` and its explorer's RPC does not, and `rpc()` compared whole bodies.
   It could never have agreed on a receipt across these two providers. Now the providers must agree on the fields the buyer
   acts on (`transactionHash`, `blockHash`, `blockNumber`, `from`, `to`, `status`), lower-cased; a missing field is a
   disagreement, not a pass. Tested on these two real receipts. After the upgrade: `Mined { block 122968985, success }`.
3. **Same hash as the script's keccak — pass.**
4. **The node's own `from` is the canister's address — pass** (`0x5b0b0649…25ebc2`, read by the script).

**And the property the design leans on, shown rather than asserted:** the identical bytes (rebuilt from the chain's record
and checked to hash to the mined transaction) sent again through the canister. Both providers answered `NonceTooLow`, which
the canister does not read as success; its read-back still said `Mined`, block 122968985; the nonce count stayed at 3.
Resending the same bytes changed nothing.

What B4 does not yet include: the buyer's own EVM steps (announce, pull, settle, return) still have no performer — `observe`
is a stub — so nothing calls `broadcast_evm` except the rehearsal. When they are built, the rule is the Bitcoin side's: save
the signed bytes in the phase before sending, and after a restart send the same bytes.
