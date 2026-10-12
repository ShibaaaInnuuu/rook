# Phase 0 findings

Started 2026-09-21. Every item says how it was checked. **Verified** means read from the live chain (chain id 4663, the public
RPC `rpc.mainnet.chain.robinhood.com`), run on a fork of it, or run against real external data. **Read from source** means the
Pons V2 source as vendored and byte-matched in our CardPaired repo — true of the deployment only as far as that byte-match holds;
the fork tests are what tie it to the live contracts. **Unverified** means exactly that. Nothing was deployed, signed or sent.

## 1. Pons V2: can a contract be the permanent creator-fee recipient?

**Yes, with one stated exception (Pons's own owner).**

| Question | Finding | How |
|---|---|---|
| How a launch names the recipient | `TokenParams.creatorFeeRecipient`, an argument of `launchToken(params, launchConfigId, pairToken)` on the factory `0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e`. Zero means the caller. | read from source; **verified** by launching through our Launcher on a fork (`test/fork/PonsLaunch.fork.t.sol`) |
| May it be a contract | Yes. No code-size or EOA check. It must be able to receive ETH, because the escrow pays by `call`. | source; **verified** on the fork |
| Who is "the creator" to Pons | `msg.sender` of `launchToken` — our Launcher. The person is recorded by us, not by Pons. | source |
| Is it permanent | From the creator's side, yes if the recipient has no way to call `transferCreatorFeeRecipient(token, new)` — only the *current recipient* may. Our router has no such call. **But the Pons protocol owner (`0x263ed295…19Dd`) can redirect any launch's recipient** through `setCreatorFeeRecipient` → 3-day timelock (`CREATOR_FEE_RECIPIENT_TIMELOCK = 259200`, **verified** by `eth_call`) → permissionless execute inside a 3-day window. The source says in its own words that this is "a standing protocol power over creator fee routing". | source + live read |
| Buyback | `buybackEnabled` sends part of the creator bucket to a buyback vault. Only the current recipient can turn it on; the owner can only turn it off. We launch with it off and the router cannot turn it on. | source |
| How fees are claimed | `PonsV2FeeEscrow` (`0xd3AFEB2a…Ac9e`, **verified** as the factory's `feeEscrow()`): `claim()` pays `msg.sender` its whole native balance; `balanceOf(recipient)` reads it. **Accounting is per recipient, not per coin** — which is why each coin needs its own router: the router's address *is* the per-coin ledger. `credit(recipient)` is permissionless (anyone can add ETH to a router's balance; it is split like any other). | source; **verified** on the fork |
| Launch fee | `launchFee()` = 0.0005 ETH, must be sent exactly (`msg.value != launchFee` reverts). | **verified** live |
| Creator tax | `creatorTaxBps`, 0–`maxCreatorTaxBps()` = 1000 (10%), **verified** live; charged on top of the base fee and paid **entirely** to the creator recipient. | source + live |
| Base fee | 100 bps on the curve, split 30 protocol / 70 creator (CardPaired Phase 0, measured on chain). | earlier measurement |
| Launch-second snipe tax | Starts at `snipeTaxStartBps()` = 9900 and decays over `snipeTaxSeconds()` = **3 s** (the source's default says 15; the live value is 3 — **verified** live). It is accrued **with the base fee**, so it is split 30/70 like the base fee: 70% of it reaches the router, not all of it. The launcher and the recipient are exempt. | source + live |
| After graduation | The pool's hook credits the same escrow the same way to the same recipient (`PonsV2MemeHook` → `feeEscrow.credit{value}(recipient)`); the factory forwards a recipient change to whichever of curve/hook is live. | source; on a fork: see §1a |
| Quote asset | A launch may be against native ETH (`pairToken = address(0)`) or an approved ERC-20; an ERC-20 launch's fees arrive as that token (`claimToken`). **Our Launcher launches native only.** | source |

### 1a. What the fork tests show — and one thing the source reading missed

Run on a fork of 4663 at the public RPC, 2026-09-21 (`forge test --match-path "test/fork/*" --fork-url robinhood`), 6 of 6 passing.

| Test | Shows |
|---|---|
| launch through our Launcher | the live factory accepts our `TokenParams` layout; it records **our router, a contract, as `creatorFeeRecipient`**, the Launcher as deployer, `pairToken` zero, buyback off |
| trade, harvest | a 1 ETH curve buy at a 2% creator tax yields **exactly 0.027 ETH** creator-side (70% of the 1% base fee + the whole 2% tax); `harvest()` lands 80% in the vault attributed to the coin and 20% at the treasury, and leaves nothing behind |
| launch-second snipe | a 1 ETH buy in the launch second yields **0.693 ETH** creator-side — 70% of the 99% snipe tax, as the source says, not all of it |
| the role cannot be handed on | neither the creator nor the Launcher can call `transferCreatorFeeRecipient`; only the router could, and it has no such call |
| a stranger cannot sweep | see the finding below |
| graduation | five 1 ETH buys cross the threshold (graduated, factory phase 1); the fees graduation swept reach the router (0.0729 ETH at a 1% tax); **the recipient is still our router afterwards** |

**FINDING (not visible from the escrow alone): creator fees do not arrive in escrow by themselves.** After a trade
`escrow.balanceOf(router)` is **zero**. Pons accrues fees inside the curve and credits the escrow only in `sweepFees`, which only two
callers may run: Pons's fee-sweep operator, and the launch's current creator-fee recipient (the curve stores it as `deployer`) — the
latter only while no buyback is pending, which for us is always. So **our router must sweep its own curve**, and `harvest()` does,
before it claims. Without that, every coin's fees would wait on Pons's operator. Graduation sweeps the curve by itself.

**Unverified: fees from pool trades after graduation.** The hook's `sweepPoolFees` has the same two callers, and a non-operator
may sweep only when no internal swap is needed — i.e. the ETH-side fees; the token-side half must be converted by a swap that
Pons reserves for its operator. The router exposes `sweepPool(poolId)` for the half it may do. This was read from source and **not
exercised on a fork** (it needs a v4 swap through the router); it is the first thing Phase 1 should test. If Pons's operator
stopped sweeping, the token-side half of post-graduation fees would sit in the hook.

## 2. Randomness

**Decision: (a) our own verifier — but not the one the brief expected.**

- **There is a drand oracle on 4663 and it is not open to us.** The live raffle contract of the product this design follows
  carries one immutable besides its registry: `0xa096500e4414199f65ba738f1728aabba438e786` (found by scanning its bytecode for
  address constants; unverified source, so its interface was read by resolving its function selectors against a public signature
  database). It exposes `requestRandomness(uint32)`, `fulfill`, `proveRound`, `roundRandomness`, `authorizedConsumers`,
  `setConsumerAuthorization`, `authorizedRelayers`, `requestFee` (0), `withdrawFees`, an `owner()`. **Verified** by `eth_call`:
  `authorizedConsumers(theirRaffle) = true`, `authorizedConsumers(a stranger) = false`, and a simulated
  `requestRandomness` from a stranger reverts `UnauthorizedConsumer`. It is an allowlisted service with an owner. Depending on it
  means asking to be listed and trusting its owner not to delist us mid-draw. Not used.
- **What it told us that matters:** its `CHAIN_HASH()` is `04f1e906…c8c3`, `PERIOD()` 3, `GENESIS()` 1727521075, and its `DST()`
  decodes to `BLS_SIG_BN254G1_XMD:KECCAK-256_SVDW_RO_NUL_`. That is drand's **`evmnet`** network (scheme
  `bls-bn254-unchained-on-g1`), made to be verified on the EVM: signatures are BN254 G1 points, checkable with the pairing
  precompile `0x08` that every EVM chain has had since Byzantium. **No BLS12-381 needed.**
- **EIP-2537 is present anyway.** **Verified** by `eth_call`: `0x0b` (G1ADD) answers 128 zero bytes to two points at infinity, and
  `0x0c`–`0x11` answer `invalid input length` to a malformed input rather than returning empty, i.e. the precompiles exist. So a
  quicknet (BLS12-381) verifier is possible too. evmnet is cheaper and simpler, so it is what was built.
- **What was built:** `src/rand/DrandEvmnet.sol` + `BN254HashToCurve.sol` — RFC 9380 `hash_to_curve` (expand_message_xmd over
  Keccak-256, the Shallue–van de Woestijne map with constants computed from the RFC's definitions: c1 = 4, c2 = (p−1)/2,
  c3 = sqrt(−12) with even parity, c4 = −16/3), then `e(−sig, G2) · e(H(round), pk) = 1`. No owner, no fee, no allowlist; anyone
  proves any round; the public key is a constant.
- **Evidence it is right: verified against real beacons.** Rounds 1000 and 20,823,703 fetched from `api.drand.sh` on 2026-09-21
  verify, and `sha256(signature)` equals the network's published randomness for both; the same signature for a neighbouring
  round, a tampered signature, and a valid curve point that is not a signature are all refused (`test/DrandEvmnet.t.sol`).
  About 140k gas a verification.
- **What no verifier fixes:** drand's own threshold assumption, and *when* a round is pinned. The raffle pins a round that does
  not exist yet and never lets it change.

## 3. The Bitcoin side (design only; nothing here was bought, signed or sent)

### 3.1 ETH on 4663 → BTC: a route exists. **Verified** (quotes, not fills).

LI.FI's public API lists both chains (`4663 Robinhood Chain`, `20000000000001 Bitcoin`) and quotes native ETH on 4663 → native BTC
to a taproot address, routed through **LayerSwap**, about 32 minutes. Quotes taken 2026-09-21, no key, nothing executed:

| Send | Arrives | Cost |
|---|---|---|
| 0.005 ETH ($13.64) | $12.98 | **4.79%** |
| 0.05 ETH ($136.37) | $134.87 | **1.10%** |
| 1 ETH ($2,727.45) | $2,707.86 | **0.72%** |

The fees are mostly fixed, so small purchases are expensive: below roughly 0.05 ETH the bridge eats more than a percent. This
argues for fewer, larger declarations, which is also what `MAX_OPEN = 3` pushes toward. A quote is not a fill: slippage, a failed
leg and a stuck 32 minutes are all the purchaser's problem, and a bridged amount that never arrives looks, on chain, exactly like
an overdue receipt. Relay, Chainflip and THORChain were **not checked**.

### 3.2 What delivery costs, which decides the rune minimum. **Computed from verified inputs.**

Inputs read 2026-09-21 from mempool.space: fee rates 1–3 sat/vB (fastest 3), BTC $85,213. A rune transfer is one transaction with
a runestone (an OP_RETURN of a few dozen bytes) and one output per recipient; each recipient output must carry its own bitcoin —
330 sats is the relay dust limit for a taproot output, 546 the amount ord and most wallets use as "postage".

| | Per extra recipient in a batch | One-off transaction |
|---|---|---|
| Size | ~43 vB (the output) + a few bytes of edict | ~180 vB (input, runestone, recipient, change) |
| At 3 sat/vB | ~130 sats fee + 546 postage = **~680 sats ≈ $0.58** | ~540 + 546 = **~1,090 sats ≈ $0.93** |
| At 50 sat/vB (a busy day) | ~2,150 + 546 = **~2,700 sats ≈ $2.30** | ~9,000 + 546 = **~9,550 sats ≈ $8.10** |

The postage is not a cost to the protocol in the accounting sense — the recipient receives it — but it is bitcoin the keeper must
hold and send. **The posted minimum should be worth a multiple of the busy-day figure**, on the order of $10–25 of runes, or
delivery eats the distribution. That is why the minimum is keeper-posted with a TTL and not a constant: it is a function of the
fee market and the rune's price, neither of which this chain can read. An inscription delivery is one ~150 vB transaction carrying
the inscription's own sat: a few hundred sats at today's rates.

### 3.3 Buying programmatically, and what the venues allow. **Mostly UNVERIFIED.**

- *Mechanism (general knowledge of the protocols, not re-verified here):* ordinals and rune listings are partially signed bitcoin
  transactions. A seller signs an input with `SIGHASH_SINGLE|ANYONECANPAY`; a buyer completes the PSBT with payment inputs and
  their own outputs, signs and broadcasts. Marketplaces wrap this in an API that returns a PSBT to sign. Nothing about it needs a
  browser.
- *Magic Eden:* its developer docs returned **HTTP 403** to our fetch, so neither its buy endpoints nor its terms on automated
  buying were read. **Unverified.**
- *UniSat:* the marketplace API page returned **404** and the public documentation corpus that did load contains wallet and product
  pages only — no marketplace endpoints, keys, limits or terms. **Unverified.**
- *OKX:* not reached. **Unverified.**
- **What this means:** Phase 1 cannot start buying until somebody has read each venue's API terms in a browser and written down the
  section that permits or forbids automated purchasing. The CardPaired work on Collector Crypt is the model: their terms reached the
  website and API, not the on-chain program. Bitcoin has no on-chain program to fall back on — a listing's PSBT comes from the
  venue's API — so **the venue's permission matters more here than it did there.**
- *Floor data:* the same APIs. A cap posted from one venue's floor is one venue's word; the `capRef` on every declaration is there
  so that whoever checks later knows which.

### 3.4 What a collection is. **General knowledge; decisive for the design.**

An ordinals "collection" has no on-chain existence of its own. It is a list of inscription ids kept by marketplaces (and, for some
newer collections, a parent inscription whose children are provably its own — parent/child provenance, which *is* on Bitcoin).
Where a parent exists the list can be derived by anyone from an indexer; where it does not, the list is whatever the venues agree
on. Either way 4663 cannot see it, which is why the registry commits to a Merkle root of ids and makes replacing it slow and
public. A rune needs none of this: its id (`block:tx`) is the asset.

### 3.5 Checking a receipt independently. **General knowledge.**

Anyone can run `ord` with `--index-runes` (or ask any public ordinals/runes indexer) what the listing's public Bitcoin address
holds: which inscription ids, how many units of which rune, and which transaction brought them. That is the check the attestor's
receipt is a claim about, and the reason each listing publishes one Bitcoin address and changes it only after seven public days.

### 3.6 Custody on Bitcoin. **A recommendation, not a finding.**

A single key holding every listing's assets is a single theft. A 2-of-3 taproot multisig (script-path, or MuSig2/FROST key-path
where the signers' tooling supports it) means no one person can move an asset, at the cost of coordinating signatures for every
delivery — which batching already wants. It does **not** make the purchase trustless: the purchaser still receives ETH on 4663
before anything is bought. Ordinals-aware coin control is mandatory whoever signs: a wallet that does not know about inscriptions
will spend one as fee.

## 4. The reference product's own documents

Read 2026-09-21 for understanding only (`usenosh.app/docs`; the trust-model page it mentions returned 404). It names no address for
its randomness oracle; §2 found it on chain. Its external vaults release ETH through "an announced withdrawal with a one-hour time
lock … the owner can cancel", with a viewing key for audit. **What we changed and why:** a time-locked withdrawal bounds nothing but
time — after the hour, any amount can go. Ours is bounded per declaration by a fresh cap × quantity, limited to three open at once,
and **stops itself** when a receipt is late, with a different key required to vouch for the spending. None of that makes the
Bitcoin purchase trustless; it makes the worst case a number.
