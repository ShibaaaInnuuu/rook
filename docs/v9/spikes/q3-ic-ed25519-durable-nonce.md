# Spike Q3: a canister signs Solana transactions with threshold Ed25519 and a durable nonce

Status: **run 2026-10-09 on the LOCAL replica (dfx 0.32.0's PocketIC), against Solana devnet and a local `solana-test-validator`;
the IC-mainnet run is prepared for the operator, not run.** Question: `docs/v9/solana-migration.md` §2.4 **[Q3]**, §4 (`solkey.rs`,
`soltx.rs`, `solsend.rs`, `solrpc.rs`). Code: `ops/rehearsal/sol-sign/` (the `canister/` crate, `run.sh`, `drive.py`). Same legend as
the migration page: **[M]** measured here.

## 1. Verdict

| Claim | Status | Where proven |
| --- | --- | --- |
| A canister holds a Solana identity: `schnorr_public_key(ed25519)` → 32-byte key → base58 address | **proven [M]** | local replica, every run |
| `sign_with_schnorr(ed25519)` signs the Solana message bytes and Solana accepts the signature | **proven [M]** | devnet: both providers verified the signatures (§3.2); local validator: they landed |
| Message bytes built by hand (no solana-sdk in the wasm) are correct | **proven [M]** | byte-for-byte equal to the Solana CLI 4.3.0's `--sign-only` message (native test) |
| Durable-nonce account created by the canister alone (one signature, seed-derived account) | **proven [M]** | local validator: finalized 33 s after the send |
| Durable-nonce transfer signed once, bytes saved, the **same bytes** sent 4 times, **exactly one** landing | **proven [M]** | local validator: recipient holds 1,000,000 lamports exactly |
| Fallback lifetime (finalized `getSlot` rounded to 20 → `getBlock` → blockhash); proven expiry → re-sign the same intent; one of the two signatures lands | **proven [M]** | local validator (landed); devnet (signed, sent, expired, re-signed; unfunded) |
| Both behind one trait (`solsend::Lifetime`) | **done** | `canister/src/solsend.rs` |
| The same on **devnet with funds** | **not done: faucet refused** | `requestAirdrop` → 429 "airdrop limit today or the faucet has run dry", the CLI's airdrop too, OnFinality's 429, on three tries over ~40 min |
| The same on **IC mainnet** (`test_key_1`, real subnet, real HTTPS outcalls through 13 replicas) | **prepared, not run** | `run.sh --ic …` (§6); creating canisters is the operator's |

**One correction to §2.4:** "There is no local Ed25519 test key" is wrong for dfx 0.32. Its PocketIC holds
`[ecdsa:Secp256k1:key_1, schnorr:Bip340Secp256k1:key_1, schnorr:Ed25519:key_1, vetkd:Bls12_381_G2:key_1]` (the reject message
listed them when asked for `dfx_test_key`, which does NOT exist there). The local key is **`key_1`**, priced locally like mainnet's
`key_1`. So the whole send line is testable without the IC; only the subnet's own behaviour (13 replicas, latency, NAT64) needs mainnet.

## 2. What was built (`ops/rehearsal/sol-sign/canister`, 1.31 MB wasm, ic-cdk 0.19.0 as the buyer)

- `soltx.rs` (pure): base58; the legacy message compiler (keys ordered payer / signer-writable / signer-readonly / writable /
  readonly, sorted inside each class as the SDK's `CompiledKeys` BTreeMap does); System Program `transfer`, `advance_nonce`,
  `create_with_seed`, `initialize_nonce`; `address_with_seed`; `assemble`; the **1,232-byte check** (`check_size(wire_size(msg))`
  **before** signing, again in `assemble`); the nonce account decoder (80 bytes: version, state, authority, nonce, fee).
- `solrpc.rs`: one transform `sol_tf` whose context byte selects what is kept; `each` (both providers concurrently, `join_all`)
  and `agreed` (all answered and equal after the transform, else `Refused::Disagree` / `Refused::Failed`; never guessed).
- `solsend.rs`: `trait Lifetime { name, anchor, can_never_land }`, `DurableNonce { account }`, `RecentBlockhash`; the reads
  (`finalized_slot`, `finalized_height`, `rounded_blockhash`, `account`, `nonce_state`, `rent_exempt`, `balance`, `sig_status`) and
  `send_bytes` (broadcast to every provider, no agreement needed: the signature is known locally).
- `lib.rs` endpoints (controller-only updates): `sol_address`, `create_nonce_account`, `send_transfer(to, lamports)` (sign once,
  save, send), `hold_transfer` (sign + save, don't send: the expiry drill), `resend(opt preflight)` (the saved bytes; under the
  fallback, re-sign only on **proven** expiry), `status` / `sig_status` (`getSignatureStatuses` with history, both providers, the
  agreed verdict, the nonce now, the balance), `set_mode(Nonce|Blockhash)`, `slot_agreement(bucket)`, `metrics` (query), `saved_tx`.
- 13 native tests: the CLI byte-equality, the seed address (= `solana create-address-with-seed`), the 1,232 refusal, short-vec,
  nonce decoding, every transform on real devnet answers, the agreement rule, the `.did` held to the code.

## 3. Measurements [M]

### 3.1 Cycles (local PocketIC, priced as a 13-node application subnet)

| | cycles | note |
| --- | --- | --- |
| `sign_with_schnorr(ed25519, key_1)`: what `ic0.cost_sign_with_schnorr` returned and ic-cdk attached | **26,153,846,153** | constant, 6 of 6 signatures recorded; = §2.4's "≈ 26 B ≈ $0.035" **[W]** confirmed for `key_1` |
| the same, the canister's balance delta around the call | 26,160,040,171 – 26,160,355,903 | +6.2–6.5 M of the canister's own execution; no refund observed |
| one outcall **pair** (both providers), small answer (`getSlot`, `getBlockHeight`, `getBalance`, rent) | 133.1 – 134.1 M | `max_response_bytes` 1,000 |
| pair, `getSignatureStatuses` (2,000) | 155.4 – 155.8 M | |
| pair, `getAccountInfo` / `getBlock` / `sendTransaction` (4,000) | 196.9 – 204.0 M | the cap drives the price, not the answer |
| **one durable-nonce transfer**, send line only | **≈ 26.55 B** | nonce read 0.197 + sign 26.16 + send 0.200; + 0.156 per status poll |
| one fallback transfer | ≈ 26.69 B | getSlot 0.133 + getBlock 0.197 (+0.197 per skipped slot) + sign + send |
| a fallback re-sign after proven expiry | ≈ 27.0 B | height 0.133 + status 0.156 + a new anchor + a new signature |

On IC mainnet `test_key_1` is priced lower than `key_1` and a 34-node subnet higher; the `--ic` run prints the real numbers.
The signature dominates (≈ 98 %); outcalls are noise. Re-signing is what costs, so durable nonces also save cycles: a resend
costs one send pair (0.2 B), a fallback re-sign ≈ 27 B.

### 3.2 Latency (local replica; the IC's would differ: its rounds and its threshold protocol)

| | local |
| --- | --- |
| `schnorr_public_key` (first `sol_address`) | 609–625 ms in canister (1.75 s wall through dfx) |
| `sign_with_schnorr(ed25519)` | **1,218–1,259 ms** (6 signatures) |
| one outcall pair, concurrently | 2.43–2.49 s median (PocketIC's round cadence, not the providers'); 3.1 s max; `getBlock` once 4.3 s |
| `send_transfer` under the durable nonce (nonce read + sign + send) | **6.1 s** |
| `send_transfer` / `hold_transfer` under the fallback (slot + block + sign [+ send]) | 6.1 s held, 12.5 s sent (wall, with dfx) |
| `resend` that re-signs (height + status + slot + block + sign + send) | 13.4 – 13.7 s in canister |
| send → `finalized` (local validator) | 20.8 s (nonce transfer), 25.7 – 33.3 s (others) |
| fallback validity left at signing (devnet) | lastValidBlockHeight passed ≈ 50 s after the hold was signed: finalized (≈ 32 slots behind the tip) plus the rounding (≤ 19) eat ≈ 50 of the 150 blocks |

### 3.3 Sizes (the 1,232-byte check)

| transaction | bytes |
| --- | --- |
| durable-nonce transfer (`AdvanceNonceAccount` + `Transfer`, 1 signature, 5 keys) | **289** |
| plain transfer (fallback) | 215 |
| create nonce account (`CreateAccountWithSeed` + `InitializeNonceAccount`) | 416 |
| advance + N transfers to distinct recipients | N = 22 fits, 23 refused (native test) |

The durable-nonce prefix costs **74 bytes** (one instruction + the nonce account + the RecentBlockhashes sysvar). The check runs on
the compiled message **before** `sign_with_schnorr`, so an oversized intent costs 0 cycles of signing. The real program
instructions (Anchor, lookup tables) should be held to it the same way.

### 3.4 Devnet without funds: what still proved something

The RPC verifies a transaction's signatures **before** it simulates it (`-32003 signature verification failure` vs simulation
errors). Both devnet providers answered every send of the canister's bytes with `AccountNotFound` (the payer had no lamports):
the threshold Ed25519 signatures and the bytes were accepted as valid by two independent devnet nodes, 5 sends, 3 signatures
(the create-nonce transaction and both fallback signatures). The expiry drill ran fully on devnet (signed at height ≤ 496,489,115,
expired at 496,489,265, finalized height 496,489,286 read through both providers, re-signed on a new blockhash, the same intent,
history of two signatures, not re-signed again on the next resend). Nothing could land.

## 4. Failure modes seen or designed for

1. **The faucet.** Devnet `requestAirdrop` was refused all afternoon (`429 … airdrop limit today or the faucet has run dry`;
   `-32603 Internal error` once; the CLI's airdrop the same; OnFinality's public endpoint 429). `run.sh` falls back to the CLI, then
   waits `--fund-wait` seconds for a manual top-up at faucet.solana.com, then runs the **unfunded proof** (§3.4) and says
   `PARTIAL`. The landing half was run on a local `solana-test-validator` (Agave 4.3, same RPC) with `--rpc-urls
   http://127.0.0.1:8899,http://localhost:8899`: the local replica's outcalls accept plain-http localhost (the IC will not).
2. **Durable-nonce resend after landing says `BlockhashNotFound`, not `AlreadyProcessed`.** Once the transaction lands the nonce
   has moved, so the preflight's age check fails before the status cache is consulted. Seen 6 s after the first send already.
   The same class is what a nonce moved *by someone else* would produce, so **the send's answer is never evidence of anything**;
   only `getSignatureStatuses` is. Without preflight the providers answer `accepted` and the leader drops the duplicate.
3. **Ordering race in "can never land".** Read the lifetime fact FIRST (the nonce value at finalized / the finalized height), the
   signature status SECOND. Reading the status first lets a landing between the two reads look like "unknown signature + nonce
   moved/height passed" and triggers a second, valid signature: two can mine. Implemented and commented in `solsend.rs`.
4. **Nonce moved under a live line** (durable nonce, signature unknown, nonce changed): the spike **refuses** to re-sign and calls
   it an incident; the real canister should alert and only re-sign after a human looks (with one nonce account per line and the
   canister its only authority, this means a bug or a compromised key).
5. **One transaction in flight per nonce account.** The next intent reads the nonce at `finalized`; reading it before the
   previous transaction finalized would sign on a nonce already consumed. `send_transfer` refuses while the line's last signature
   is not `finalized`. More throughput = more nonce accounts (one per line), as §2.4 says.
6. **Skipped slots in the fallback.** `getBlock` on a skipped slot answers `-32007`/`-32009`; the transform turns these into
   `{"skipped": code}` (equal across providers) and the canister walks back ≤ 20 slots (one pair, 0.197 B, each).
7. **Fallback validity is short.** Reading at `finalized` and rounding costs ≈ 50 of the 150 blocks: ≈ 40–60 s left to land. The
   saved bytes must be resent quickly and expiry is common under congestion; every expiry is a 27 B re-sign. Another reason
   durable nonces stay the default.
8. **Replicated `sendTransaction` (IC only, not observable locally).** On a 13-node subnet each replica sends; their answers can
   differ (`accepted` vs `blockhash_not_found` vs `already_processed`), the transform cannot make them equal, and the outcall then
   fails consensus *although the transaction was sent*. Recommended: **non-replicated** send (`is_replicated: false`, `run.sh
   --non-replicated-send`, cheaper too) since the answer is advisory; the `--ic` run should try both.
9. **Local replica housekeeping** (fixed in `run.sh`, applies to `ic-outcall/run.sh` too): `dfx canister stop` **before** `dfx
   canister delete` makes dfx's withdrawal fail (IC0509/IC0508: it installs its temporary wallet into a stopped canister) and leaves
   the canister alive running wallet code; the next run's `create` then reuses it and the install fails. `dfx canister delete` on a
   **running** canister stops, withdraws and deletes cleanly (measured). `run.sh` no longer stops first, retries "start, wait for
   Running, delete", and refuses to start while an old `solsign` exists. `ops/rehearsal/ic-outcall/run.sh` still stops first.

## 5. Two disagreeing devnet providers

Keyless devnet providers checked 2026-10-09: `api.devnet.solana.com` ✓, `solana-devnet.api.onfinality.io/public` ✓;
`solana-devnet.drpc.org` (paid plan only), `devnet.helius-rpc.com` and `rpc.ankr.com/solana_devnet` (key required),
`solana-devnet-rpc.publicnode.com` (no answer). Mainnet should use keyed providers (§2.4: Helius + Triton/dRPC/PublicNode).

How they are kept from disagreeing, and what happens when they do:
- **Transforms** drop what moves between two requests: `context.slot`, `apiVersion` (the two ran different versions), `id`,
  `confirmations`, simulation logs and units; send answers become a class; slots and heights round down to 20.
- **Measured:** 29 agreed read pairs over the two devnet runs, 0 refusals; `getSlot(finalized)` raw agreed 6 of 6 when asked concurrently
  (sequential `curl`s 1 s apart differed by 8 slots: hence the rounding for anything sequential or replicated).
- **Disagreement** on any read is `Refused::Disagree` (both answers in the message), the tick ends, the next tick retries. Status
  polls that disagree are counted in the report (0 seen). A provider that is merely **behind** (one says `confirmed`, the other
  `finalized`) is a disagreement too; it resolves itself within a slot or two. Never take the "better" answer.
- **Sends** need no agreement: both providers are asked, either one forwarding is enough, and the signature is known locally.

## 6. Run it

```
cd ops/rehearsal/sol-sign && dfx start --clean --background        # local (key_1)
./run.sh --local [--fund-wait 600]                                 # devnet; unfunded → PARTIAL (exit 3)
./run.sh --local --rpc-urls http://127.0.0.1:8899,http://localhost:8899   # with solana-test-validator running: the full scenario
! ops/rehearsal/sol-sign/run.sh --ic --next-to <the buyer's canister id> [--non-replicated-send] [--fund-wait 900]   # OPERATOR
```

The `--ic` run (operator, with `!`): creates a throwaway canister (1.5 T default; creation fee ≈ 0.5 T on 13 nodes + ≈ 0.15 T of
signatures and outcalls), key `test_key_1`, prints the address and waits for funding (fund it at faucet.solana.com if the
airdrop is refused), then the whole scenario; the canister is deleted by a trap and the rest withdrawn. Each canister id derives
its own address, so it cannot be funded ahead of the run: keep faucet.solana.com open and use `--fund-wait 900`. Expected: everything as locally, plus the subnet's signing latency, real cycle prices for `test_key_1`,
and whether the replicated send fails consensus (failure mode 8). Devnet only; no real money.

## 7. The recommended `solsend` shape for the real canister

```rust
// solsend.rs: the lifetime is the only thing that changes when durable nonces go.
pub trait Lifetime {
    fn name(&self) -> &'static str;
    /// recent_blockhash + prefix instructions (+ last valid height) for a transaction signed NOW
    async fn anchor(&self, rpc: &SolRpc) -> Result<Anchor, Refused>;
    /// PROVEN that the saved bytes can never land: lifetime fact first, signature status second (§4.3)
    async fn can_never_land(&self, saved: &Saved, rpc: &SolRpc) -> Result<bool, Refused>;
}
pub struct DurableNonce { account: Pubkey }   // default; one per line; authority = the canister; seed-derived address
pub struct RecentBlockhash;                    // fallback; finalized slot rounded to 20 → getBlock → blockhash, +150 blocks

pub struct Saved {                              // in stable memory, written BEFORE the first send
    intent: IntentId, lifetime: LifetimeId, tx: Vec<u8>, signature: Signature,
    history: Vec<Signature>,                    // > 1 only after a proven expiry (fallback)
    recent_blockhash: Hash, last_valid_height: Option<u64>, sends: u32,
}

/// One tick of one line (no `await` holds state across a trap: every decision re-reads):
///   status(sig) agreed at finalized → Finalized(ok) / Failed(err): done, the intent is settled
///   else lifetime.can_never_land(saved) → DurableNonce: INCIDENT (alert, never auto re-sign)
///                                       → RecentBlockhash: re-sign the SAME intent, push to history, save, send
///   else resend saved.tx (non-replicated outcall, skipPreflight after the first send), Pending
///   any Refused (disagreement, outcall failure) → nothing changes, retry next tick
pub fn tick(line: &mut Saved, l: &impl Lifetime, rpc: &SolRpc) -> Step;
```

Plus: `check_size` before signing; the send's answer only logged, never decided on (§4.2); one in-flight transaction per nonce
account; `sol_seen` = the set of every signature in every `history` (a landed transaction from an old signature is still ours).
Default lifetime **DurableNonce**; `RecentBlockhash` selectable by an Init/upgrade flag, as built here.

## 8. Not covered (and where it goes)

- IC mainnet (13-node consensus on the reads and the send, NAT64 to the providers, `test_key_1` latency and price): the `--ic` run.
- Devnet **landing** and mainnet-beta: a funded devnet run (`--fund-wait`, faucet by hand), then §2.4's mainnet step.
- v0 messages and address lookup tables, Anchor instruction encoding, priority fees / compute-budget instructions (the
  durable-nonce transaction must still start with `AdvanceNonceAccount`; compute-budget instructions come after it).
- The SOL RPC canister as the second read route (§2.4).

Raw reports of the runs (local, not committed): `$TMPDIR/rook-sol-sign-local-report.json`; logs `/tmp/solsign-*.log`.
