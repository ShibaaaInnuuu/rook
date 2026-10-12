# Rook sim: one simulated frog purchase, Bitcoin half (regtest, no real money)

```bash
ops/rehearsal/sim/run.sh        # about 1 minute after the first build; exit 0 and "SIM: PASS" on success
```

Needs `bitcoind`/`bitcoin-cli` 31 and `ord` 0.29 (`brew install bitcoin ord`), node 22 with `npm ci` done in `ops/`, and cargo.
Everything runs on 127.0.0.1 (bitcoind regtest on 18743, ord on 18780; override with `SIM_RPC_PORT`, `SIM_ORD_PORT`). The data
directory is `$TMPDIR/rook-sim` (`SIM_DIR`), recreated each run; the log is `sim.log` and the bid PSBT `bid.psbt.hex` in it. The
script stops bitcoind and ord when it exits.

**Scope (operator, 2026-10-09):** Rook is moving from Robinhood Chain to Solana, so the EVM side is not simulated. The run stops
when **custody holds the inscription six deep**. The advance, the Relay ETH leg, and the mint/settle on 4663 are left out.

## What runs for real and what is a stand-in

| Leg | In this sim |
| --- | --- |
| Bitcoin | **Real**: bitcoind regtest with txindex. Every transaction is checked by consensus and policy: the seller's 0x83 signature, the float's three key-path signatures, fees. |
| The frog | **Real**: inscribed with `ord wallet inscribe` and indexed by `ord server`. The canister's flow reads ord's `/inscription/<id>` and `/output/<outpoint>`. |
| The seller's listing | **Real signature**: the seller's own wallet (ord's bitcoind wallet) signs a 1-in/1-out listing with `walletprocesspsbt … "SINGLE\|ANYONECANPAY"` and finalizes it. |
| The marketplace's bid PSBT | **Stand-in for UniSat's `create_bid`**: `market.mjs` (@scure/btc-signer) builds the classic padded shape (2 × 600 float padding, the seller's input finalized, float payment; out: 1,200 padding, the frog to custody, the price to the seller, change). This marketplace hands over the seller's signature. Whether UniSat's does is Part A's open question. |
| The runner's hand-in | **Stand-in**: the harness inserts the `Offer` exactly as `frog_offer` stores it (hex PSBT decoded, `to_sign` [0,1,3], membership proof). |
| The canister's decisions | **The production code, compiled natively** (`harness/`, a path dependency on `autobuyer/canister-rs`). This is `frogflow::tick_full`, the same function `frognet::tick` calls: the membership proof against the root, ord/explorer reads of the seller's output, `frogbuy::judge`, `input_clean`, caps, `sighash::taproot_key_path`, `psbt::finalize`, save-then-broadcast, `Signed → Sent → Held` with the six-deep and ord-custody checks. |
| The canister's keys | **Stand-in**: local BIP86 keys (internal key, TapTweak, BIP340) replace the subnet's threshold Schnorr `sign_with_schnorr` with Bip341 aux. The digest in and the 64-byte signature out are the same. |
| BTC arriving at the float | **Stand-in for Relay's BTC leg**: a regtest faucet wallet pays the float directly (600 + 600 padding seed, as R10 in docs/v8/ordinals-wave.md, plus 500,000 sats). |
| The frog record | **Stand-in for the floor trigger**: frog 0 is created `Wanted { cap 0.3 ETH }` directly. |
| EVM / 4663 | **Stub, reads only**: the wrapper names us custodian, and the registry's listing carries the sim's members root. Every EVM write errors. Nothing is minted. |
| The IC | **Not run.** There is no replica, no outcalls (the reads go straight to bitcoind and ord, one source rather than two agreeing explorers), and no cycles. ops/rehearsal/ic-outcall covers the outcall side. |
| Reference rate / fee rate | Fixed: 3,190,000 sats/ETH and 2 sat/vB. |

## The run (2026-10-09, abridged)

```
== 3. frog 0 is Wanted …; a tick with no offer
    | frog 0: wanted, for at most 300000000000000000 wei: no offer yet
== 4. the listing and the runner's hand-in …
  seller's listing witness (sighash 0x83): 96d80333bee673c8bc3f1fe0…
  bid PSBT: 619 bytes, to sign [0,1,3], seller paid to bcrt1p…
== 4b. the same bid WITHOUT the seller's signature …: one tick
    | frog 0: offer refused, nothing signed: refused: input 2 is not ours and the PSBT carries no witness for it
  float digests signed for it: 3 (signatures discarded; nothing broadcast: 0 sends so far)
== 5. the canister's tick: judge, sign the float's inputs, finalize, save, broadcast
    [bitcoind] sendrawtransaction accepted: 0ebf19c4…923d
    | frog 0: <frog> bought for 201500 sats (64429467084639500 wei), signed, saved and sent in 0ebf19c4…923d
== 6. mine 1: the buy confirms                   | frog 0: Bitcoin has it          => Sent
== 7. mine 5 more                                | frog 0: 6 deep and in custody: held  => Held
== 8. ord: <frog> now at 0ebf19c4…923d:1:0 (10000), owner bcrt1pmg55…f4w (custody); buy tx 6 confirmations, 4 in, 4 out, vsize 413
SIM: PASS
```

## Findings

1. **The classic judge does not require the seller's signature.** `frogbuy::judge` passes a bid whose seller input has no
   final witness. Only `psbt::finalize` refuses it, and that runs *after* the float key has signed every one of our inputs
   (step 4b: 3 digests signed, then discarded). The tick then logs "offer refused, nothing signed", which is wrong: the float
   key did sign, but the signatures were thrown away. For an operator offer this happens once, because offers are taken. For a
   **permissionless hand-in** the result is `Later`, so the canister signs again every tick and never rules the hand-in
   Invalid (no slash). Failing test: `tests/handins.rs::a_hand_in_without_the_sellers_witness_is_invalid_before_anything_is_signed`
   (`#[ignore]`d, known failing; run it with `--include-ignored`). The fix is to refuse a non-final foreign input inside the
   judge, before anything is signed. Production code is unchanged in this pass.
2. **Padding is required**, as docs/v8/ordinals-wave.md §1.3 R10 says. The float needs two small UTXOs ahead of the seller's
   input. A single Relay output cannot buy a classic padded listing.
3. The production flow, fed real ord and bitcoind answers, bought, confirmed and held the frog with no change. The sighash
   digests matched what bitcoind verifies, and the 0x83 seller signature survived being moved from index 0 of the listing to
   index 2 of the bid.

The bid PSBT from this run is kept as a fixture with its facts: `autobuyer/canister-rs/tests/fixtures/unisat/regtest-classic-bid-2026-10-09.json`.
`tests/unisat_bid.rs` runs it through the judge, both with the seller's signature and without it.
