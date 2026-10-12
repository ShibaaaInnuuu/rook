# Pinned reads: every EVM state read names one agreed block

Written 2026-09-23, before its code. Applies to the buyer canister's reads of Robinhood Chain (`autobuyer/canister-rs`).

## The problem

The buyer believes an EVM answer only when every provider in `rpc_urls` returns the same one (`rpc`, `rpc_view`). Two things
make that impossible for any read taken at `"latest"` on chain 4663, and both were measured, read-only, on 2026-09-23:

1. **Providers are never on the same head.** Robinhood Chain makes a block every **0.104 s** (1,208 blocks in 125 s). Polled
   together every 2 s for two minutes, the three keyless providers' heads were apart by a median of **7 blocks** and at most
   **9**; `rpc.mainnet.chain.robinhood.com` was the one behind, by up to 9. An `eth_getBalance(…, "latest")` asked of two
   providers is two different questions.
2. **A single provider does not agree with itself across IC nodes.** A replicated outcall is made by every node of the subnet
   at a slightly different moment and the bodies must match. At ten blocks a second, `eth_blockNumber` — and anything read at
   `"latest"` — differs between nodes of the same subnet asking the same provider. It cannot reach consensus at all. This is
   true of `base_fee` and `estimate_gas` today too, which read `"latest"` through `rpc_max`.

So "latest" is out. Every state read must name a block that every node and every provider will name identically.

## What the providers will serve (measured)

| Provider | head | state served back to | `safe` / `finalized` |
|---|---|---|---|
| `rpc.mainnet.chain.robinhood.com` (Robinhood) | up to 9 behind | **~6,220 blocks (≈ 647 s)** | agrees |
| `robinhood.drpc.org` (dRPC) | ≤ 1 behind | ≥ 20,000 blocks (archive) | agrees |
| `rpc.ordofi.network` (ordofi) | — | ≥ 20,000 blocks (archive) | agrees |
| `robinhood-rpc.publicnode.com` (publicnode) | ≤ 1 behind | **~92 blocks (≈ 10 s)**; deeper needs a paid token | agrees |

All four returned the same hash for block 67,108,864, for the block 1,500 behind the head, and for `safe` (70,161,201) and
`finalized`. Robinhood, dRPC and ordofi also answered `eth_getBalance`, `eth_call`, `eth_getTransactionCount` at a block
**hash** (EIP-1898) and `eth_estimateGas` at a block number, 1,500 blocks back.

Two consequences:

- **publicnode cannot be a pinned provider.** A tick is several outcalls in sequence, each a consensus round of seconds; a pin
  10 s deep is gone before the tick ends. It stays useful for nothing the buyer does.
- **`safe` is not usable either,** though it is the natural pin: it was **6,418** blocks behind the head, and Robinhood's own RPC
  keeps state only **~6,220** back. Pinning to `safe` would leave Robinhood unable to answer. `finalized` (10,426 back) is
  further still.

## The rule

1. **Head, rounded.** Each provider is asked `eth_blockNumber`, replicated, through a transform that rounds the answer **down to a
   multiple of B = 1,024 blocks**. Nodes asking within the same 1,024-block window (≈ 106 s) return identical bodies; nodes
   straddling a boundary disagree, the read fails, and the tick waits. With nodes' requests a second or two apart, that is a
   ≈ 2% chance per read — a retry, never a wrong answer.
2. **Pin.** `pin = min(rounded heads) − B`. Every node computes the same number from the same agreed inputs, and it lies between
   **B and 2B + 9** blocks behind every provider's real head: **≈ 107 – 214 s**. Robinhood's 6,220-block window leaves ≈ 430 s
   for the rest of the tick. If the rounded heads are more than one bucket apart, a provider is stale or lying: nothing is
   believed.
3. **The block is agreed.** Every provider is asked `eth_getBlockByNumber(pin)`, projected to `{number, hash, baseFeePerGas}`,
   and all must match. A different hash at the same number — a reorg in progress, or a lie — and nothing is believed.
4. **Every state read names the pin by hash** (EIP-1898 `{"blockHash": …}`): `eth_call` for the vault, `eth_getBalance`,
   `eth_getTransactionCount`. If the chain reorganised under the pin after step 3, a provider fails the read rather than answering
   from another fork.
5. **Numbers that are not state.**
   - **Base fee**: the pinned block's `baseFeePerGas`, from step 3 — no extra call, identical everywhere. It is 2–4 minutes old;
     the loop already caps the fee at 4× the base fee (`FEE_MULTIPLE`), which is the margin for it moving in between.
   - **Gas estimate**: `eth_estimateGas` at the pin's number, the highest across providers (`rpc_max`, as now). Deterministic.
   - **Nonce**: `eth_getTransactionCount` at the pin — the nonce as of the pin — plus the loop's own saved in-flight transaction.
     Only this canister signs for its key and only one EVM transaction is in flight, so the pending nonce is never needed.
6. **Confirmation is at the pin too.** A receipt counts only if its block is at or below the pin: a transform maps a missing
   receipt and a receipt above the pin to the same `null`, and projects the rest to `receipt_view`'s fields. Every node then
   answers identically (the pin is ≥ 1,024 blocks behind every head), and "mined" and "the nonce moved" become true at the same
   pin, so the loop's rule "nonce used but our transaction not visible → wait" never sees one without the other. A transaction
   mined above the pin reads as not yet seen; the loop sends the same saved bytes again, which is harmless.

## All providers, or a quorum

**All must agree.** With providers P and quorum q, a false belief needs q providers lying consistently; a stall needs
P − q + 1 providers down. All-of-2 and 2-of-3 have the same safety — two liars — and 2-of-3 has better liveness. But that is
only true if the three are **independent**, and here they may not be: dRPC is an aggregator whose upstream nodes are not
published, and could be Robinhood's own; ordofi publishes nothing about its infrastructure. A quorum over correlated providers is
a quorum of one. The buyer holds money; a stall costs time and is visible, a false belief costs funds. So the rule stays
**every configured provider must agree**, and the recommendation is **two** providers — the fewest that are a check.

**Superseded 2026-09-24/25 (docs/v6/phase1-scope.md §3.2 C8, §7 item 7): dRPC is dead for this use.** Measured on 2026-09-24, `robinhood.drpc.org` refuses every state read at an explicit block ("Unknown state. First available state is 1"), and the canister reads only at pinned historical blocks (`pin.rs:123`). Install with `rpc_urls = vec { "https://rpc.mainnet.chain.robinhood.com"; "https://rpc.ordofi.network" }` (Robinhood keeps ≈ 6,220 blocks; ordofi is archive; both publish AAAA records). The measurements below are kept as they were taken.

~~**Recommended `rpc_urls`: `https://rpc.mainnet.chain.robinhood.com` and `https://robinhood.drpc.org`**~~ (2026-09-23), with
`https://rpc.ordofi.network` as the spare to swap in (an upgrade argument, public for the timelock's delay) if either stalls.
publicnode is out (state window).

## Not shown here

- **That IC nodes' requests are close enough in time for 1,024-block windows.** The local replica is one node; its outcalls
  come from this Mac. The disagreement rate on a real subnet is measured by the first mainnet reads, and B is a constant to
  raise if it is high (a larger B costs depth: Robinhood's 6,220-block window bounds B at about 2,000).
- **That the providers answer IC nodes at all** (all four sit behind Cloudflare).
- **Reorg depth.** A pin 2–4 minutes deep is well past the sequencer's soft confirmations, but short of Ethereum finality. Step 4
  makes a reorg fail loudly; it does not make one impossible.

## What the code settled, and the live evidence (2026-09-23)

Code: `src/pin.rs` (pure: rounding, the pin choice, block agreement, receipt-at-pin; 4 tests covering lag, a bucket boundary,
a reorg, a provider down, a stale or wild provider, a lying block, receipts above the pin), `canister.rs` (the transforms
`head_bucket`, `block_at`, `receipt_at_pin`; `pin()`, taken once per tick in `CanisterIo`; every read site moved to the pin;
`pinned_read`, an operator-only, read-only diagnostic of the same path).

**One cap corrected by the live run.** The head read was first capped at 1,000 response bytes. ordofi's headers alone exceed it
on a 60-byte answer, and every read was rejected ("Http body exceeds size limit of 1000 bytes"). The cap is 8,000.

**Live, read-only, against Robinhood Chain mainnet**, from a new canister on the local replica (`k5w36-mp777-77777-aaaha-cai`,
chain_id 4663). The local replica is one node and its outcalls leave from this Mac's network, not from IC nodes: this shows
the providers agree on pinned reads; it cannot show the node-skew disagreement rate or that the providers answer IC nodes.
The self-check: `pinned_read(0x…64, arbBlockNumber())` — ArbSys returns the block the call runs at, so every provider must
return exactly the pin.

| `rpc_urls` | reads | agreed | `arbBlockNumber == pin` | time a read |
|---|---|---|---|---|
| Robinhood + dRPC | 14 over 363 s (+1 before) | **14 / 14** | 14 / 14 | ≈ 26 s |
| Robinhood + dRPC + ordofi | 12 over 491 s | **12 / 12** | 12 / 12 | ≈ 38 s |
| Robinhood + publicnode (negative control) | 3 | 1 / 3 | 1 / 1 | — |

The pin stepped every 1,024 blocks (≈ 106 s) as designed: 4 distinct pins in the first run, 6 in the second. publicnode failed
2 of 3 with "historical state … is not available" and passed once — load-balanced nodes that sometimes keep the state, which is
worse than never: excluded.

**Confirmation at the pin**, through `evm_tx_status`: a mainnet transaction 5,000 blocks back (`0x98e01455…3a73`, block
70,177,553) read `Mined { block 70177553, success }`; one in the head block (`0x237b1644…d55a`, 70,182,553, above the pin) read
`Pending` — it exists, and is not counted until the pin passes it.

**Recommendation (2026-09-23, superseded 2026-09-24 — see above):** `rpc_urls = [https://rpc.mainnet.chain.robinhood.com, https://robinhood.drpc.org]`, all agreeing;
ordofi the spare. Today: Robinhood and ordofi. Left on the local replica: `k5w36-mp777-77777-aaaha-cai`, installed with the three-provider list.
