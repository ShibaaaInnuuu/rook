# The buyer broadcasts its own Bitcoin transaction

Pre-registered 2026-09-23, before its code and before the rehearsal runs. The EVM half is B4
(`89283b5`, `src/evmsend.rs`); this is the same shape on the Bitcoin side, where the broadcast has
until now been the driver script's (`signet-rehearsal.mjs` step 5, `runes-rehearsal.mjs`).

## What is wrong today

The canister signs a Bitcoin spend and never sends it. Every signet transaction this project has
put in a block (323256, 323261, 323262) was posted by a script on the operator's machine. A buyer
whose last step is a person pasting bytes is not autonomous, and the state machine's
`Act::Broadcast` has nothing to perform it.

A replicated outcall cannot fix that. Every node would `POST /tx` the same bytes; the explorer
accepts the first and answers the rest with an error, the replies differ, consensus fails, and the
canister is told the send failed about a transaction that was sent. That is B4, unchanged.

## The change

- **`src/btcsend.rs`, pure and unit-tested.** It parses a signed transaction (segwit or not) and
  refuses anything that does not parse completely: no trailing bytes, at least one input and one
  output. It computes the **txid itself**, as the double SHA-256 of the non-witness serialisation,
  displayed reversed, so the transaction the canister watches is the one it holds, not one a
  provider names. It classifies an esplora `POST /tx` reply: `Accepted` (our txid), `WrongTxid`,
  `AlreadyConfirmed`, `AlreadyInMempool`, `MissingInputs`, `Refused`, `Unreadable`. `MissingInputs`
  is **never read as success**: an input already spent is either this very transaction, confirmed,
  or someone else's, and only the read-back can say which. (Measured: resending the confirmed
  rehearsal spend `3d3ed273…` to both explorers answers `bad-txns-inputs-missingorspent`, not
  "already in block chain", because its outputs have since been spent.)
- **`src/btcnet.rs`.** One single-node `POST /tx` (`is_replicated: false`) per configured
  explorer. Then the **replicated read-back**, where every node asks every explorer and all must
  agree: `GET /tx/{txid}/status` projected to `{confirmed, block_height, block_hash}`, and, when not
  confirmed, `GET /tx/{txid}` to tell "in a mempool" from "never heard of it". That second call is
  needed because both explorers answer `{"confirmed":false}` with HTTP 200 for a txid that does not
  exist.
- **The network is checked, not assumed.** Every explorer must return a pinned block hash at a
  fixed height for the installed network before anything is sent or believed. Genesis cannot do
  this job, because every signet shares one genesis block. Signet at height 300000 is
  `000000073002e4e1…01b02c` (mempool.space and blockstream.info agree); testnet4 at height 50000 is
  `00000000e2c8c94b…eff2cb`.
- **Endpoints.** `broadcast_btc(raw)` and `btc_tx_seen(txid)` are what the loop calls.
  `rehearse_broadcast_btc(network, raw_hex)` has `rehearse_sign_sighash`'s guards: the canister is
  installed for signet, and the caller names signet again. `btc_tx_status(txid_hex)` asks again
  later. Both are operator-only.

**A configuration finding, made before any code.** The rehearsal canister was installed with
`btc_api_urls = [mempool.space/signet/api, mutinynet.com/api]`. **Mutinynet is a different signet**:
its tip is at height 3,448,455 against default signet's 323,306, and its block at height 300000 is
`0000035739a7…`. A replicated read-back across those two could never agree on a default-signet
transaction, and nothing so far would have noticed, because nothing read Bitcoin back through the
canister. The rehearsal reinstalls the canister with `blockstream.info/signet/api` as the second
source; the network check above is what makes the same mistake fail loudly next time. Reinstalling
keeps the canister id, so the threshold keys, and therefore the funded address, are unchanged.

## The rehearsal (signet only)

The funded canister is the local replica's `uxrrr-q7777-77774-qaaaq-cai`. Its output key
`efafa803…71d` is the address `tb1pa7h6…s0z7` that holds the rehearsal's signet coins. It spends
`904e55c3…8f4a:2` (1,180 sat, no runes) to itself: one P2TR input, one P2TR output. The script
builds and signs through `rehearse_sign_sighash`, as before. **The canister broadcasts**, through
`rehearse_broadcast_btc`.

**Pass**, all five:
1. the canister's call returns, with no consensus failure, and a report from both explorers;
2. the txid it watches equals the one the script computes, independently (`@scure/btc-signer`);
3. the canister's replicated read-back reaches `Confirmed { height, block_hash }`;
4. mempool.space, asked by the script and not the canister, shows the same txid in the same block
   at the same height;
5. the identical bytes sent again through the canister change nothing: no second transaction, and
   the read-back still says `Confirmed` at the same height.

**Fail** is any of: a consensus error on the send; a different txid; a `Confirmed` that the script's
explorer does not show; a read-back that never agrees; anything new appearing on the resend. A
`MissingInputs` or `AlreadyConfirmed` from an explorer on the resend is expected, and recorded.

## Result (2026-09-23): the code passes on the live explorers; the funded broadcast is blocked, and why

**Tested:** 96 unit tests and 2 end-to-end tests pass, and the wasm (`8fb63744…`) builds without warnings. `btcsend` computes
the txid of four real transactions exactly as the chain did: three segwit signet spends from this project, and block 170's
pre-segwit payment.

**The funded canister refused to broadcast, and that was right.** `uxrrr-q7777-77774-qaaaq-cai` (the one that owns
`tb1pa7h6…s0z7`) was **upgraded** in place, with state restored and the key unchanged (`efafa803…71d`). Module `f3e8c14d…` →
`8fb63744…`. An upgrade keeps the saved install configuration, and that turned out to be neither of the two lists above: it
was `[mempool.space/signet/api, open-api-signet.unisat.io]`. UniSat's signet API is not an esplora explorer: it serves
different paths, behind a key, and answers `404 page not found` at `/block-height/300000`. The new network check refused it
before anything was sent:

    rehearse_broadcast_btc("signet", …) → Err "refused: https://open-api-signet.unisat.io is not "signet": block 300000
                                               there is "404 page not found" (HTTP 404), not 000000073002…01b02c"

So two misconfigurations were found without spending anything: the saved list (UniSat), and the list the operator had been
passing on upgrades (mutinynet, which is another chain). Neither would ever have let a replicated read-back agree, and nothing
had checked until now. Refused live on the same canister: a caller naming another network, and trailing bytes after the
locktime.

**Not run: a new funded signet broadcast.** It needs `uxrrr` to have two real signet explorers. That takes a reinstall, which
wipes its stored state; the reinstall was refused as an irreversible deletion and is left to the operator. The alternative is
to let an upgrade's argument replace `btc_api_urls`, as the timelock already lets it replace the operator (the argument is
public for the whole delay). That is a change to `canister.rs`'s `post_upgrade`, so it is left to whoever owns that file.

**What was shown instead, on a fresh, unfunded canister** (`kpqmh-a7777-77777-aaaea-cai`, the same wasm, installed with
`[mempool.space/signet/api, blockstream.info/signet/api]`), with nothing that could change the chain:

| Call | Canister, replicated | The script, mempool.space directly |
| --- | --- | --- |
| `btc_tx_status(904e55c3…8f4a)` | `Confirmed { 323262, 0000000044578d8e…9dcd }` | confirmed, 323262, same hash |
| `btc_tx_status(3d3ed273…7f9f)` | `Confirmed { 323256, 00000013aac5e485…b7b8 }` | confirmed, 323256, same hash |
| `btc_tx_status(00…aa)`, a txid nobody has | `NotYet` | the status says `{"confirmed":false}`, the trap the presence check exists for |

And the send path, end to end, with the **already-confirmed** spend `3d3ed273…`, whose inputs are long spent, so no node can
accept it: `rehearse_broadcast_btc` returned without a consensus failure. Both explorers answered `MissingInputs`, which is
not read as success; the replicated read-back said `Confirmed { 323256 }`; the txid the canister computed is the mined one.
The address's transaction count was 6 confirmed and 0 in the mempool before, and the same after.

Against the five pass criteria: 1 and 2 are shown on the send path; 3 is shown on the read-back, for transactions confirmed
earlier; 5 is shown (a resend changes nothing). **Criterion 4, a new transaction confirmed after the canister sent it, is not
shown,** and cannot be until `uxrrr` has two signet explorers.

## Result, criterion 4 (2026-09-23): **pass** — a new signet spend, broadcast by the canister

Unblocked by letting an upgrade replace the buyer's endpoints (commit `2e9f894`: `rpc_urls` and `btc_api_urls` follow the
upgrade argument, as `operator` does; chain, network, key and vault stay as saved). `uxrrr-q7777-77774-qaaaq-cai` was upgraded
in place (`8fb63744…` → `3f2a8931…`) with `btc_api_urls = [mempool.space/signet/api, blockstream.info/signet/api]`; its output
key stayed `efafa803…071d`, so the funded address is unchanged.

`node signet-rehearsal.mjs canister-spend` (with `CANISTER=uxrrr-…`, `UTXO=904e55c3…8f4a:2`): the pre-registered 1,180-sat
coin, self-sent for 1,069 sat at 111 sat fee, signed by the threshold key, **broadcast by the canister**.

1. **Returned, no consensus failure, a report from both explorers — pass.** Both answered `Accepted`; the immediate read-back
   said `Mempool`.
2. **Same txid as the script's (@scure) — pass.** `12d45d42b83b2196c1b99fdaed5e6a7f1609ce15237a599d590243691ad153f7`.
3. **The canister's replicated read-back reached `Confirmed` — pass.** Height 323,310, block `00000008bfca720b…e899`.
4. **mempool.space, asked by the script, agrees — pass.** Confirmed at 323,310 in the same block, and listed in that block's own
   txids.
5. **The identical bytes again changed nothing — pass.** Both explorers answered `AlreadyConfirmed`; the read-back still said
   `Confirmed` at 323,310; the address's confirmed transaction count stayed at 7.

All five criteria are met. The Bitcoin broadcast is no longer the driver script's.
