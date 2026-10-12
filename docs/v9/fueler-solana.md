# The fueler on Solana: the ops share pays for the canister's cycles

> **Retired in part (2026-10-11, decision D-l, `canister-only-solana.md` §0):** the programs' half of this design — rook_core's `Treasury`,
> the permissionless `allocate`, `set_ops` / `apply_ops`, `FUEL_HOME=solana`, `FUELER_SOL_PROGRAM`, `FUELER_SOL_ALLOCATE_MIN` — is gone
> with rook_core. The fueler's Solana home is now `FUEL_HOME=books` (the default): the canister's own cranks pay the ops share to its
> `treasury.ops` (set with `treasury_set_ops`; `canister-only-solana.md` §9.2), and the fueler reads the canister's `treasury_status` for
> the ops key and the income mark. The Solana leg from the hot key on — §2 keys, §3 caps, Relay SOL → ETH (S1–S12), the forward to the
> cold payee, §6 monitor — stands as written below, read `rook_core Treasury.ops` as the canister's `treasury.ops`.

Written 2026-10-10 on branch `golive-2026-10-08`. Ports the fueler of `docs/v7/cycles-from-fees.md` §4.2 / §4.2.1 from Robinhood Chain to
Solana (`docs/v9/solana-migration.md` § "Fueler"). **Nothing has been deployed, signed or sent.** The network reads made while building it
were read-only: Relay `POST /quote/v2` (no key, nothing signed), Solana mainnet `getAccountInfo` / `getBalance`, CoinGecko and Coinbase
prices, and the CMC's rate on the IC.

## 1. The route

```
rook_core Treasury (PDA ["treasury"])
  │  allocate (permissionless; the canister also cranks it past 0.05 SOL)  → ops_bps (10 %) to Treasury.ops = the fueler's Solana hot key
  │  distribute (the canister's crank) → a collection's share above its reserve cap also goes to Treasury.ops
  ▼
Solana hot key ──── everything above FUELER_SOL_BUFFER + FUELER_SOL_DAY_MAX ────► the cold payee (default: the fund wallet GGgX…6Wuy)
  │  when the buyer is under its trigger: up to the run / day / week caps of SOL
  │  Relay deposit_native (SOL on Solana → ETH on Ethereum), judged by the rules S1–S12 of relaysol.rs
  ▼
the fueler's Ethereum address (FUELER_EVM_KEY)              ┐
  │  ckETH helper depositEth(own principal)                  │  unchanged from the Robinhood fueler
  ▼                                                          │  (ops/src/fuel/engine.ts phases bridging → done)
ckETH → ICPSwap (ICP/ckETH pool) → ICP → CMC notify_top_up  ┘  for the buyer (and the timelock)
```

- **ETH, not USDC.** The design's §2 USDC → ckUSDC route was already replaced by ETH → ckETH in the as-built fueler (§4.2.1: runs of at most
  20 T ≈ $27 are far under the $200 where the USDC leg pays off; no ERC-20 approve; the deposit's gas comes out of the bridged ETH). The
  Solana leg keeps that: Relay SOL → **native ETH** on Ethereum, so every leg after the bridge is the same code.
- **Not straight to the ckETH helper.** Relay could pay a contract call on Ethereum, but the judge refuses any order with `calls` (rule S9,
  as in the canister). The bridged ETH lands at the fueler's own Ethereum address, and the fueler then calls `depositEth` itself, as before.
- **No `claim_platform_fees` step.** On Solana the partner fees of the DBC configs go to rook_backing's claimer PDAs and on to the
  treasury through the canister's cranks; the fueler's only Solana income step is `allocate`, and it is permissionless.

## 2. Keys

| Key | Variable (sealed) | What it can sign | Holds |
| --- | --- | --- | --- |
| Solana hot key = `Treasury.ops` | `FUELER_SOL_KEYPAIR` (the `solana-keygen` JSON) or `FUELER_SOL_KEYPAIR_FILE` (a 0600 file on `/data`) | `allocate` naming itself as ops; Relay `deposit_native` with itself as sender, depositor and refund; a System transfer to the cold payee. Nothing else (`ops/src/fuel/sol.ts` `allowedSol`) | its fee buffer (0.05 SOL) and at most a day's cap |
| Ethereum key | `FUELER_EVM_KEY` | the ckETH helper's `depositEth` naming the fueler's own principal | bridged ETH for minutes |
| IC identity | `FUELER_IC_IDENTITY_PEM` | the pinned pool's approve/deposit/swap/withdraw, the ICP transfer to the CMC's top-up account of the buyer or the timelock | ckETH / ICP for minutes |

The keys are read only in `ops/src/fuel/keys.ts` (a guard test); the Solana secret stays in a closure (`node:crypto` ed25519), every log line
is scrubbed of it in four spellings, and a malformed or mismatched keypair is refused without repeating it. The one Solana signing site is
`Engine.sendSol`: `allowedSol` → `simulateTransaction` → sign → journal (bytes, signature, last valid block height) → send.

## 3. Caps and checks (the first month)

| | Default | Variable |
| --- | --- | --- |
| Trigger / target (buyer) | under 40 T, toward 60 T | `FUELER_BUYER_LOW_T`, `FUELER_BUYER_TARGET_T` |
| Per run / per 7 days (cycles) | 20 T / 60 T | `FUELER_RUN_MAX_T`, `FUELER_WEEK_MAX_T` |
| SOL per day / per 7 days | 1 SOL / 2.5 SOL | `FUELER_SOL_DAY_MAX`, `FUELER_SOL_WEEK_MAX` |
| Never spend below | 0.01 SOL (rent and fees) | `FUELER_SOL_FLOOR` |
| Fee buffer | 0.05 SOL | `FUELER_SOL_BUFFER` |
| Forward to the cold payee | everything above buffer + day cap, when it exceeds 0.01 SOL | `FUELER_SOL_FORWARD_MIN` |
| Smallest run | 0.1 SOL | `FUELER_SOL_MIN_RUN` |
| Allocate when unallocated ≥ | 0.05 SOL | `FUELER_SOL_ALLOCATE_MIN` |
| One run per | 6 h | `FUELER_MIN_GAP_S` |
| Relay minimum vs oracle (S10) | ≥ 98 % of the oracle's ETH for the SOL | `FUELER_SOL_RELAY_MAX_FEE_BPS` (200), Relay slippage `FUELER_SOL_SLIPPAGE_BPS` (50) |
| Priority fee | 20,000 µlamports/CU × 100,000 CU ≈ 2,000 lamports | `FUELER_SOL_PRIORITY_MICROLAMPORTS`, `FUELER_SOL_CU_LIMIT` |

The trigger is the as-built dormant-model value (34 T at the handover). The design's 100 T trigger is `FUELER_BUYER_LOW_T=100` with a
target above it; the run and week caps still bound it.

Measured 2026-10-10 (keyless quotes, SOL $110, ETH $2,496): at 0.5 % slippage Relay's guaranteed minimum was 1.04 % under the oracle for
0.26 SOL and 1.27 % under for 0.1 SOL, so the 2 % bound holds with room; Relay's automatic slippage (2 %) would not.

Prices: CoinGecko and Coinbase must agree within 2 % on ETH, ICP **and SOL**. The pool and end-to-end checks are unchanged (the end-to-end
check now values the run's SOL, not ETH).

**The Relay judge** (`judgeSolQuote`) is `relaysol.rs judge_sol` in TypeScript: S1 one deposit step / item / instruction (lookup tables
never used: the fueler writes its own message); S2 `paymentDetails` on solana, the pinned depository, native SOL, the amount; **S3 the
depository program pinned**: the `RelayDepository` state's allocator `HuAj…eW6Z` and the program's `ProgramData` (upgrade authority
`6Bz8…aMWD`, deploy slot 386,211,280) read from every provider (they must agree) on each judgement, any change refuses; S4 the amount; S5
the exact five accounts with the hot key as sender **and** depositor; S6 the instruction's id = `hashStruct(Order)` (Solana addresses as 32
raw bytes) = `orderId`; S7 one native-SOL input of that amount; S8 every refund ours (solana → the hot key; ethereum → the fueler's
Ethereum address), at least one on solana; S9 one native-ETH payment to the fueler's Ethereum address, no calls, no fees; S10 the minimum;
S11 deadlines between a day and eight days away; S12 an order id never accepted before (kept in the journal).

**Idempotency on Solana.** A Solana transaction has no nonce: it is valid until its blockhash's last valid height. The fueler journals the
signed bytes with that height before sending, sends the same bytes again each tick until a provider shows it **finalized**, and declares it
expired only when no provider has it and every provider's finalized height is past that height (it can then never land). An expired Relay
deposit gives its SOL back to the caps and the run takes a fresh quote and order; a deposit that fails on chain fails the run (nothing left
but the fee) and pages. Tested: a crash after signing and before sending, an expired deposit, a failed deposit.

## 4. What the operator sets

1. **The hot key**: `solana-keygen new --no-bip39-passphrase -o fueler-hot.json` on a clean machine; put its JSON in the fueler's Railway
   service as the sealed `FUELER_SOL_KEYPAIR` (or the file on `/data`, 0600, and `FUELER_SOL_KEYPAIR_FILE`). Used for nothing else.
2. **Fund it with about 0.05 SOL** (its fees; it converts nothing until fee income arrives: a run needs 0.1 SOL above the floor).
3. **`set_ops` at the listing ceremony**: `rook-ceremony list … --ops <the hot key's address>` (§5).
4. **The cold payee**: `FUELER_SOL_PAYEE`, default the fund wallet `GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy` (decision D-g: the
   operator's own wallet, whose key Rook's tooling never holds). The design's forward-to-cold applies, so it is on by default.
5. **rook_core's mainnet id**: `FUELER_SOL_PROGRAM` (the reserved id of `day1-plan.json`); the treasury is derived from it. Until the
   programs are deployed and `init_treasury` has run, the fueler says "treasury not on chain" and only watches the cycles.
6. **Solana providers**: `FUELER_RPC_SOL`, two at least (D-h: Helius free + PublicNode).
7. **Carried over unchanged**: `FUELER_EVM_KEY` (a fresh EOA; on Solana only its Ethereum address is used and it needs no ETH),
   `FUELER_IC_IDENTITY_PEM` (a fresh IC identity), `RELAY_API_KEY`, `FUELER_RPC_1`, `IC_HOST`, `FUELER_BUYER`, `FUELER_TIMELOCK`, the T caps.
   The pinned IC pieces (ckETH minter and helper `0x1890…0E68`, the ICP/ckETH pool `angxa-baaaa-aaaag-qcvnq-cai`, the ICP ledger, the CMC)
   are re-checked each run as before.
8. **Railway**: the fueler's own service (`ops/Dockerfile.fueler`, `ops/railway.fueler.json`, one replica, a `/data` volume), variables
   from `deployments/launch/railway/fueler.env.example` (`FUEL_HOME=solana` is the default). `FUELER_MODE=dry` for a day, then `send`.
9. **Monitor**: the `fueler` block with `sol_ops` (§6) and its `/healthz` in `liveness`.

Before running it on mainnet anywhere, a plan with nothing signed:

```sh
cd ops && FUELER_BUYER=<buyer> FUELER_SOL_PROGRAM=<rook_core> FUELER_RPC_SOL=<a>,<b> FUELER_RPC_1=<a>,<b> \
  FUELER_SOL_WATCH_ADDRESS=<any pubkey> FUELER_WATCH_ADDRESS=<any 0x address> \
  node --import tsx src/fueler.ts --dry-run [--assume-cycles-t 34] [--assume-sol 0.6]
```

Run 2026-10-10 against mainnet with a throwaway pubkey, `--assume-cycles-t 34 --assume-sol 0.6`: the treasury absent (programs not
deployed), the decision "run: 0.25967 SOL for 20.00 T", and a live Relay quote judged by S1–S12 with S3 read from mainnet — order
`0x68e2…6873`, 259,671,005 lamports in, at least 0.011328 ETH out (expected 0.011385; the oracle 0.011447). Without `--assume-sol` the same
key holds 0 SOL and the plan is "blocked: … under a run's minimum of 0.1 SOL" (SEV3 `cycles-autofund-short`).

## 5. The ceremony

`rook-ceremony list` (`ops/rehearsal/sol-devnet/tool/src/ceremony_list.rs`):

- **`--canister-sol <pubkey>`** (required; or the launch file's `canister.sol_address`; decision D-i): the first owner-signed step after
  preflight is `propose_roles(keeper = the canister)`, immediate while `Registry.keeper` is unset, read back (keeper = the canister, not
  paused). It must equal `custody.json`'s custodian(s); another keeper already set is refused (a 7-day move); the canister's key may be
  neither the owner nor the guardian, and the guardian not the owner.
- **`--ops <pubkey>`** (optional): right after `init_treasury`, the owner's `set_ops(<pubkey>)`, read back from the Treasury account. The
  first operations address applies at once; replacing one is proposed, waits a day (`TREASURY_DELAY`) and is applied by anyone with
  `apply_ops` (the tool reports the pending change). Refused: the default key, either program, the treasury or registry PDA. Without
  `--ops` the operations share joins the reserves pot until it is set.

`ops/solana/fund-status.mjs` does not count the fueler's buffer: it measures the community deploy deposit, and the fueler's 0.05 SOL is an
operating cost that its forward returns to the fund wallet.

## 6. The monitor

`monitor/src/fueler.mjs` with `fueler.sol_ops` set (`rpcs_sol` replaces `rpcs_4663`):

- `sol-unexpected-tx` (SEV1, persistent): a transaction **signed** by the hot key that the fueler never journaled (its `/status` lists every
  signature it made; the chain is read before `/status`). The signatures already present at the first poll are the baseline. Response:
  stop the fueler, propose `set_ops(<a fresh key>)` (applies after a day; until then each `allocate` pays the stolen key its share), rotate
  `FUELER_SOL_KEYPAIR`.
- `ops-unexpected-tx` (SEV1) on Ethereum, as before (nothing on 4663).
- `fueler-sol-low` (SEV3): the hot key under `min_sol` (default 0.02 SOL).
- `fueler-not-fueling` (SEV2): the buyer under `trigger_t` (default 40 T) for `stuck_h` (default 12 h) with no run open and no conversion
  since.
- The fueler's own alerts relayed: a stuck leg (`cycles-autofund-failed`), a cap hit (`cycles-autofund-cap`), the oracle, `fueler-halt`.

```json
"fueler": { "ops": "<the fueler's Ethereum address>", "sol_ops": "<the hot key>", "status_url": "https://<fueler host>/status",
  "rpcs_sol": ["<helius>", "https://solana-rpc.publicnode.com"], "rpcs_1": ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"],
  "min_sol": 0.02, "trigger_t": 40, "stuck_h": 12 }
```

## 7. The bootstrap

Fees arrive only after the programs are deployed, collections are listed, and coins trade. Until then the canister lives on its seed:
**about 34 T at the handover** (the dormant model, `docs/v7/dormant-collections.md` §11: one collection awake ≈ 2.5–2.8 T a day plus the 8 T
scan floor), which is about 9–10 days of one awake collection, or months fully dormant. The fueler starts in `dry`, sees the buyer under
40 T at once and reports `cycles-autofund-short` (SEV3) until the hot key holds 0.1 SOL above its floor; the operator keeps the §3.4
fallback reserve (≥ 100 T in the operator's own cycles ledger) and tops up by hand until two clean fueler runs. At 10 % of the treasury's
income, one 20 T run (≈ 0.26 SOL) needs about 2.6 SOL of treasury income.

## 8. Where it is

`ops/src/fuel/sol.ts` (pinned Relay accounts, PDAs, the Treasury decoder, the order id with Solana addresses, the instruction builders, the
legacy message, `allowedSol`, `judgeSolQuote`), `ops/src/fuel/solio.ts` (Solana JSON-RPC), `ops/src/fuel/engine.ts` (the `solana` home:
`sendSol`, `settleSol`, `housekeepingSol`, `quoteSol`, `preflightSol`), `ops/src/fuel/keys.ts` (`fuelSolSigner`), `ops/src/fuel/plan.ts`
(`DEFAULT_SOL_CAPS`, the SOL asset), `ops/src/fueler.ts` (`FUEL_HOME`, `--dry-run`), `ops/test/fueler_sol.test.ts` (17 tests: the
encodings against the programs and a real quote, the keys, every S-rule, the allowlist, the guards, the SOL caps, an end-to-end run, the
buffer and forward, allocate, refused quotes, expiry, a failed deposit, a crash before the send, the dry run, preflight, the config),
`monitor/src/fueler.mjs` and `monitor/test/fueler.test.mjs`. The Robinhood leg is kept: `FUEL_HOME=robinhood`.
