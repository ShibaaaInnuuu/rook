# v8: buying Ordinals Wallet protected listings (passthrough v4) — design

Status: **design, written before its code**, 2026-10-09, branch `golive-2026-10-08`. Nothing deployed, signed or sent. Every
network read made for it was read-only (Ordinals Wallet's public API and docs, its open-source SDK). The operator's decisions of the
same day: buy Bitcoin inscriptions automatically; use UniSat's API without its written permission; and **support Ordinals Wallet's
protected listings**, which make up 8 of the 10 cheapest Bitcoin Puppets and 5 of the 10 cheapest Bitcoin Frogs [M, 2026-10-09].

**Legend.** **[M]** measured 2026-10-09. **[R]** read from this repository. **[OW]** Ordinals Wallet's docs, its engineering reference
(`ordinalswallet.com/learn/snipe-protection/technical`) or its MIT SDK (`github.com/ordinals-wallet/ow-cli` at `f991b1f`).

---

## 1. What a protected listing is [OW]

The seller signs, without broadcasting:

- **TX1, the passthrough**: version 2, one input (the inscription's UTXO, key path, `SIGHASH_DEFAULT`), one output of `postage − d`
  sats (`d` = 0 or 12) to an **escrow** `tr(NUMS, {sale_leaf, recovery_leaf})`:
  - `NUMS` = `50929b74…3ac0` (BIP-341's unspendable internal key: no key path);
  - `sale_leaf` = `<S> OP_CHECKSIG <C> OP_CHECKSIGADD OP_2 OP_NUMEQUAL` (seller `S` and co-signer `C` both);
  - `recovery_leaf` = `<144> OP_CSV OP_DROP <S> OP_CHECKSIG` (the seller alone, a day after TX1 confirms);
  - `C` = Ordinals Wallet's co-signer, **pinned**: `1d08b7c71f6f1e97a0a4cf005db7a977c85e34652a0c9365842aee25997c7dee`
    (`GET /market/secure-purchase/capabilities`).
- **A sale signature**: on `sale_leaf`, `SIGHASH_SINGLE|ANYONECANPAY` (`0x83`), for an input spending TX1:0 and the output **at the
  same index** paying the seller exactly the listed price.

A buyer asks `POST /wallet/secure-purchase/build` (`outpoints`, `protocol: "ordinal"`, `from`, `public_key`, `to`, `fee_rate`; no key)
and gets per item a **sale PSBT**, its **parent** (TX1), `miner_fee_sats` and `economics`, plus a **setup** PSBT when the buyer has
fewer than two coins. The sale's layout:

- inputs `0..k−1`: the buyer's funding; input `k`: the escrow (TX1:0); inputs `k+1..`: the buyer's fee input(s);
- outputs: `0` buyer change; `k` the seller's payout (exactly the price); `k+1` the inscription to `to`, carrying the escrow's whole
  value; then the marketplace fee (to `bc1p6yd49679azsaxqgtr52ff6jjvj2wv5dlaqwhaxarkamevgle2jaqs8vlnr`, 2 %, min 1,000 sats), creator
  royalties per collection, and a last change output.

The buyer signs **only its own inputs**, `SIGHASH_DEFAULT`, leaves the PSBT unfinalized, and posts it to
`POST /market/secure-purchase/submit`. Ordinals Wallet checks it against its template, adds `C`'s signature on the escrow input, and
relays `submitpackage([TX1, sale])`: TX1 pays (almost) nothing and confirms only with the sale. **Ordinals Wallet broadcasts; the buyer
cannot.**

## 2. Why the canister can do this safely

- **The canister's signature covers the whole sale.** `SIGHASH_DEFAULT` on a Taproot key path commits to every input's outpoint,
  amount and script and to every output. Ordinals Wallet's co-signature cannot change a byte of what the canister signed: it can only
  complete it or not.
- **What the canister judges is what can confirm.** The sale's txid is fixed before any witness (segwit), so the canister knows the
  exact transaction it is waiting for and recognises it on chain by txid.
- **The worst case is a sale that never broadcasts**: the canister's float coins stay unspent but a signed spend of them exists. The
  canister ends that the way it already ends a stale buy (`abandon`, `conflict.rs`): it spends one of its own signed inputs back to the
  float at a higher fee. If the sale confirms first, it bought; if the conflict does, nothing was bought and the reservation is freed.
- **The seller's terms are the seller's signature.** If Ordinals Wallet misstated the seller's address or price, the seller's `0x83`
  signature would not verify and no node would accept the sale. The canister does not need to verify it.

## 3. The canister: what changes [R]

### 3.1 The judge (`owbuy.rs`, new)

A port of the SDK's `verify_sale` (`rust/ordinalswallet/src/signing/passthrough.rs`), held to the canister's own rules, given:
the sale PSBT, the parent's raw bytes, the listing (`outpoint`, `seller_address`, `satoshi_price`, `escrow_price`), the facts
`accept` already reads (float and custody scripts, the inscription's location, offset and value, `cap_sats`).

| # | Rule | Source |
|---|---|---|
| P1 | the parent parses and hashes to its txid; one input, the listed outpoint, which is the inscription's located output; one output | SDK `parent_*` |
| P2 | the parent's output is exactly `tr(NUMS, {sale(S, C_pinned), recovery(S)})` for the `S` its leaf names, and at least 330 sats; `postage − value` is 0 or 12; the inscription's offset is below it | SDK `sale_escrow_mismatch`, [OW] |
| P3 | exactly one sale input spends `parent:0`, at index `k ≥ 1` with at least one input after it; its `witness_utxo` is the parent's output | SDK `sale_input_count`, `sale_prevout_mismatch` |
| P4 | the escrow input carries the sale leaf (with `NUMS` as its internal key) or a complete 2-of-2 witness whose seller signature is `0x83`; the leaf's co-signer is the pinned key | SDK `sale_leaf_unpinned`, `sale_template_internal_key`, `sale_witness_sighash` |
| P5 | every other input is the float's (by script), unsigned, and asks for no sighash but `DEFAULT` (the SDK also admits `ALL`; the canister signs only `DEFAULT`, so it refuses rather than sign another type) | SDK `funding_not_yours`, `funding_prefilled`, `buyer_sighash`; canister `ForeignInputToSign` |
| P6 | output `k` pays the listing's seller exactly `escrow_price` | SDK `sale_payout_mismatch` |
| P7 | the inscription's sat (FIFO: the inputs before `k`, then its offset) lands in **custody**, in an output worth the escrow's whole value; exactly one custody output | canister `NotToCustody`, `CustodyOutputs`; SDK `sale_asset_mismatch` |
| P8 | every other output is the float (change), the pinned marketplace fee address, or the listing's creator address; no `OP_RETURN` anywhere; seller payout + marketplace fee ≤ `satoshi_price`; royalties ≤ 10 % of it | SDK `sale_unknown_output`, `sale_overcharge`, `sale_royalty`; canister `OpReturn` |
| P9 | `spend` = the float's inputs − the float's outputs ≤ `cap_sats`; the network fee (the sale's, which pays for the parent too) ≤ the canister's `fee_bound(spend)` | canister `OverCap`, `FeeTooHigh` |
| P10 | a setup, when present: only the float's inputs, exactly two outputs to the float; the sale's float inputs spend only the setup's outputs, **at their values and script** (the SDK checks only the outpoints); spend = the sale's spend + the setup's fee ≤ `cap_sats`, and the sale's fee + the setup's ≤ `fee_bound(spend)` | SDK `verify_setup`, `chain_funding_mismatch` |

The SDK's 36 sale cases, 9 setup cases and 28 chain cases (`tests/fixtures/ow/purchase-verify.json`, MIT, generated by Ordinals
Wallet's own code from throwaway keys) are the judge's tests: each honest case passes P1–P8 and P10 when the fixture's buyer stands in
for the float and its recipient for custody; each tampered case is refused by the rule the SDK names, or a stricter one of the
canister's (listed by name). P9 is the canister's own and is tested with its own cases.

**As built (phase 1, 2026-10-09):** `src/owbuy.rs` (`judge_protected`, `judge_setup`, 40 typed refusals), `src/taptree.rs` (leaf, branch and
tweak hashes; the escrow rebuilt and held to every vector of `escrow.json`), `src/psbt.rs` additions (the Taproot input fields, a segwit-aware
raw transaction parser, and a writer that adds `0x13`/`0x17` to named inputs and copies every other byte). No transaction id is taken on
the API's word: the parent is whatever the escrow input spends, if its raw bytes hash to it. `tests/ow_protected.rs` decides every SDK case
the SDK's way or by a stricter rule named in its `DIFFERENT` list (an exact `escrow_price`, setup values, `DEFAULT` only, custody
distinct from the float); the fixture's own "with setup" case misstates a setup output's value and could never confirm. canister
`cargo test` 803 passed.

### 3.2 Signing and the hand-back

- The canister signs exactly the float's inputs of the sale (and of the setup), key path, `SIGHASH_DEFAULT`, with the existing
  `sign_float` (path `b"float"`, BIP-341 tweak by the subnet). The digest needs every input's prevout, which the PSBT carries.
- It does **not** finalize: it writes `PSBT_IN_TAP_KEY_SIG` (`0x13`) and `PSBT_IN_TAP_INTERNAL_KEY` (`0x17`) into its own inputs and
  leaves every other field as Ordinals Wallet built it, which is what the SDK's `sign_own_inputs` produces.
- A new operator query returns the signed sale (and setup) PSBTs for the runner to submit. A new operator method returns the float's
  **internal** key, which the build asks for (today only the tweaked output key is exposed).

### 3.3 The state machine

- A new phase **`Handed{buy, sale_txid, setup_txid, psbts, at}`** after a protected buy is signed. The canister does not broadcast in
  it; it watches `sale_txid` (`btc_tx_seen`) each tick. Seen → `Sent{buy}` as today (depth 6, then `custody_holds`, then the mint).
- Not seen within **`PROTECTED_STALE_SECS` = 6 h** (Ordinals Wallet relays at submit; its quotes live 24 h): `abandon` with the sale's
  largest **float** input named from the unsigned transaction (outpoint and value from its `witness_utxo`), the leaked fee the sale's
  network fee. The existing conflict machinery does the rest. (`abandon` today takes the largest non-seller input of a fully signed
  raw transaction: it gains a variant that is handed the input, and checks it is the float's.)
- The H2 reservation and the H3 outflow check are taken before signing, exactly as for a classic buy, and released when the
  conflict wins.

### 3.4 Entry points

- `frog_offer_protected(frog_id, inscription_id, sale_psbt_hex, parent_hex, listing, setup_psbt_hex?, proof_hex)`, operator-only,
  paused-refusing, stored beside `frog_offer`'s `OFFERS`. `every_update_method_here_is_guarded` counts it.
- `frog_protected_signed(frog_id)` (query, operator) and `float_internal_key()` (operator).

## 4. The runner [R]

A step beside the Ethereum one, on for the Bitcoin collection when `RUNNER_OW_SLUG` is set: for a frog `Wanted` with its cap,
read `GET /market/secure-purchase/capabilities` (refuse if the co-signer is not the pinned one), `GET /collection/:slug/escrows`, take
the cheapest `protected: true` rows under the cap, read each one's `GET /market/escrow/:id` (`escrow_price`, `seller_address`), build
with `from` the float, `public_key` the float's internal key, `to` custody, then hand in. After the canister signs, read the signed
PSBTs and `POST /market/secure-purchase/submit`. It never signs anything.

## 5. Limits and what this does not cover

- **Ordinals Wallet must be online** to complete a sale; if it is not, nothing happens and the 6 h stale path ends the buy.
- **Package relay**: the sale reaches miners only as a package with its zero-fee parent. Ordinals Wallet relays it; the canister's own
  broadcast path is not used.
- **The parent can confirm alone** (review P-08): at Core's 0.1 sat/vB minimum relay, a parent that drops 12 sats pays about
  0.108 sat/vB and may relay and confirm without the sale, so "confirms only with the sale" is not strictly true. Nothing changes for
  the canister: the sale stays valid, and the seller's 144-block recovery is usable only after the 6 h abandon has already ended it.
- **One item per buy**, as today. No basket, so there is never a chain of more than one sale (plus an optional setup).
- **Standard (unprotected) Ordinals Wallet listings** go through the existing `frog_offer` and judge; their build endpoint
  (`/wallet/purchase-bulk`) is a separate runner step.
- **A real buy is the only end-to-end test.** The SDK's fixtures prove the judge; a funded rehearsal (one Frog, about 375,000 sats
  [M]) proves Ordinals Wallet's submit path with the canister's real signatures.
- **Review**: P0 code on the custody signing path. Its own review round before it is installed, like every change here.
