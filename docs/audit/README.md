# Audit package: etched v2 (the autonomous buyer), v3 (the wrapper), v4 (EVM NFTs, curves, queues, pools) and v5 (custody)

Written 2026-09-23 for an external auditor, at branch `golive-prep` (`8258c77`, which merges `main` at `01a1e5d`). **Updated
2026-09-23 on branch `custody-security` at `9022920`** (the merge of `v4-pools`) to cover everything merged since: the curve, the
per-collection buy queue, the mint desk, the pools and their lock, the router, the sweep vaults, the NFT registry, vaults and
wrapper, the Seaport guard, and the canister's Seaport, SignedZone, per-chain, frog-flow, deposit and item-sink modules. The
update adds **a map of every trust assumption** ("Trust assumptions, mapped", below) and the custody threat model
(`docs/v5/custody-threat-model.md`). **Nothing here has been audited by anyone outside the project.** Four internal passes found
two critical defects in the canister and one in the v1 vault, all since fixed. The custody review on this branch found nine more
issues, recorded under "Known issues" (T-numbers), with their fixes specified in `docs/v5/custody-hardening.md`. Take that as the
prior for what is left.

`docs/security/audit-brief.md` is the v1 brief (the bonded vault, 1,565 lines) and still stands for v1. This package covers what
has been built since: the vault whose purchaser is a program, the Internet Computer canister that is that program and the
custodian of every wrapped item, its timelock, the wrappers, and the curve, queue and pool stack that buys items and makes
markets in them. Read `docs/security/threat-model.md` and `docs/trust-model.md` for v1's actors, `docs/v5/custody-threat-model.md`
for custody, and the additions below.

**What must be true before value enters custody** is `docs/v5/launch-gates.md`. Its gate G1 is this audit, and it asks that the
audit be run on a commit with H1–H4 of `docs/v5/custody-hardening.md` wired.

## Scope

### Current scope: commit `9022920` (updated 2026-09-23)

Priorities: **P0** can move, mint, pay or sign for custody or for the ETH that becomes custody; **P1** decides what P0 does, or
strands value if wrong; **P2** covers interfaces, helpers and paths that are off by default. "New" means merged after `8258c77`.
Rust lines include inline tests; "code" excludes the `#[cfg(test)]` module.

**Solidity: 7,064 lines in `src/`** (`wc -l src/*.sol src/*/*.sol`):

| File | Lines | Priority | New? | What it is |
| --- | --- | --- | --- | --- |
| `src/Wrapper.sol` | 309 | P0 | changed | wFROG: `wrap`, the custodian's `wrapDeposit`, `redeem`/`redeemToBurn`, drand resolution, delivery (keeper or custodian), refund |
| `src/nft/NftWrapper.sol` | 266 | P0 | **new** | ERC-721 → ERC-20 per `(chainId, collection)`: custodian `mint`/`mintDeposit`, random redemption, `recordDelivery` (custodian only) |
| `src/WrapClaim.sol` | 184 | P0 | — | a wrapped frog's units by Merkle root |
| `src/curve/ItemQueue.sol` | 317 | P0 | **new** | per-collection FIFO buy queue; `recordItem(cost)` pays the custodian up to the keeper's fresh cap |
| `src/curve/MintDesk.sol` | 135 | P0 | **new** | public mint on demand; `fulfill(id, cost)` pays the custodian |
| `src/curve/FrogCurve.sol` | 427 | P0 | **new** | the launch curve; graduation feeds the queue and seeds the pools |
| `src/pools/LiquidityLock.sol` | 638 | P0 | **new** | owns every Uniswap v4 position (shared wX/ETH, layered; coin pools); no withdrawal path |
| `src/SweepVault.sol` | 436 | P0 | **new** | fees sweep into wX under the keeper's cap; raffle or burn |
| `src/nft/NftVaultBase.sol` | 356 | P0 | **new** | shared NFT vault: keeper `postCap`, draws |
| `src/nft/NftSweepVault.sol` | 134 | P0 | **new** | 4663-native NFT sweeps through the guard |
| `src/nft/SeaportGuard.sol` | 102 | P0 | **new** | decodes and re-encodes Seaport calldata: no tips, one listed NFT per order, per-order cap |
| `src/nft/NftRemoteVault.sol` | 316 | P0 | **new** | ETH to the canister for other-chain purchases; receipts on its word |
| `src/AutoBuyerVault.sol` | 497 | P0 | — | v2: ETH only to its immutable buyer |
| `src/ListingRegistry.sol` | 370 | P0 | changed | roles, listings, 7-day changes, `reserveBuyer` for NFT buyers |
| `src/nft/NftRegistry.sol` | 171 | P1 | **new** | NFT listings on any EVM chain, sweep targets |
| `src/curve/CurveFactory.sol` | 160 | P1 | **new** | deploys curves; `freshCap()`, the cap the queue and the lock read |
| `src/pools/EtchRouter.sol` | 119 | P1 | **new** | buy and sell through v4 |
| `src/Distributor.sol` | 434 | P1 | — | draws and pro-rata claims |
| `src/AssetVault.sol` | 502 | P1 | — | v1 bonded vault |
| `src/lib/Wait.sol` | 72 | P1 | changed | the dual clock; parent block time is configurable |
| `src/rand/DrandEvmnet.sol`, `src/rand/BN254HashToCurve.sol` | 205 | P1 | — | the beacon |
| `src/curve/CollectionDirectory.sol` | 57 | P2 | **new** | per-collection stack directory |
| `src/nft/NftFeeRouter.sol`, `src/nft/NftLauncher.sol` | 186 | P2 | **new** | NFT listings' fee routing and launches |
| `src/FeeRouter.sol`, `src/Launcher.sol`, `src/Treasury.sol` | 298 | P2 | — | as in the v1 brief |
| `src/pools/LiquidityAmounts.sol`, `src/nft/Seaport16.sol`, `src/nft/INft.sol`, `src/curve/CurveToken.sol`, `src/lib/BtcAddress.sol` | 180 | P2 | mostly new | helpers |
| `src/interfaces/*` (incl. new `IWrappedCollection`, `ICapSource`, `ILiquiditySeeder`, `ISweepListing`) | 223 | P2 | partly new | interfaces |
| `script/*.sol`, `script/*.sh` (incl. new `DeployNft.s.sol`) | 1,151 | P1 | changed | deploy scripts and refusals |

**The buyer / custodian canister: 16,183 lines, 12,097 code at `9022920`; 17,760 / 13,035 with the three unwired modules of
`custody-security`** (`autobuyer/canister-rs/src`):

| Module | Lines (code) | Priority | New? | What it is |
| --- | --- | --- | --- | --- |
| `canister.rs` | 2,044 (1,636) | P0 | changed | entry points, guards, `restore_init`, keys (paths `btc`, `custody`, `float`, `evm`), pause, saved state |
| `psbt.rs`, `sighash.rs`, `runestone.rs` | 1,924 (998) | P0 | — | the Bitcoin signing checks |
| `custody.rs`, `redeem.rs`, `custodynet.rs` | 1,476 (1,274) | P0 | **new** | custody: locate on ord and esplora, build, sign with the custody key, send, record |
| `delivery.rs` | 242 (121) | P0 | — (now wired) | the deadline margin |
| `frogflow.rs`, `frogbuy.rs`, `frognet.rs` | 2,384 (2,174) | P0 | **new** | frog buys from the float into custody; the one-EVM-transaction sequencer |
| `deposits.rs` | 606 (411) | P0 | **new** | holder deposits (`EWD1`), mint via `wrapDeposit`, 50 a day, returns |
| `seaport.rs` | 1,029 (718) | P0 | **new** | Seaport 1.6 basic and advanced (SignedZone) orders: verify, rebuild calldata |
| `nftnet.rs`, `chains.rs` | 911 (746) | P0 | **new** | per-chain pinned reads, lines, `nft_set_target` / `nft_sign` / `nft_send` / `nft_status` |
| `evmtx.rs`, `evmsend.rs`, `pin.rs` | 775 (507) | P0 | partly new | EVM envelopes, send rule, pinned reads |
| `listing.rs`, `state.rs`, `looper.rs`, `policy.rs` | 1,590 (1,142) | P0 | partly new | quote checks, state machine, loop, purchase policy |
| `itemsink.rs`, `buyqueue.rs`, `curvebuy.rs`, `mintdesk.rs` | 785 (522) | P1 | **new** | rules for the queue, curve and desk |
| `btcsend.rs`, `btcnet.rs` | 524 (419) | P1 | — | Bitcoin broadcast |
| `rates.rs`, `oracle.rs`, `runeindex.rs` | 997 (765) | P1 | **new** | reference rate, runes indexer |
| `custodycap.rs`, `outflow.rs`, `outbound.rs` | 1,577 (938) | P0 | **new, wired** (`2562c0e`) | custody caps (H2), outbound limits and tripwire (H3), outbound purpose proof (H4); the hooks are in `canister.rs`, `nftnet.rs`, `redeem.rs`, `frogflow.rs`, `custodynet.rs`, `pin.rs` |
| `relay.rs`, `relayquote.rs`, `bridgecall.rs`, `bridge.rs`, `venue.rs`, `abi.rs`, `lib.rs` | 1,029 (664) | P2 | partly new | hand-ins, bridge judges (not configured), ABI helpers |

**The timelock: 794 lines** (`rules.rs` 541, `canister.rs` 245, `lib.rs` 8): P0, unchanged in design since the trust points (sets,
a k-of-n veto, delayed roles changes).

**Out of scope** is as below, plus Seaport, OpenSea's SignedZone and Uniswap v4 (their code is pinned by address and code hash,
and read, not audited). **Off-chain, lighter review:** the watcher, the keeper's hand-in paths (`keeper/src/opensea.ts`), and the
web app's redemption and approval flows.

### The original scope: commit `8258c77`, kept for the diff

Commit `8258c77`. Solidity 0.8.28, **via-IR**, optimizer 200, EVM `cancun`, OpenZeppelin 5; target Robinhood Chain (4663), an
Arbitrum Nitro L2 whose code-size limit is 98,304 bytes (measured, `docs/deploy/contracts-mainnet.md`) and where EIP-7702 is live.
Rust canisters: `ic-cdk` 0.19, `wasm32-unknown-unknown`, toolchain pinned in `autobuyer/rust-toolchain.toml`.

**In scope — Solidity, 2,829 lines** (`wc -l src/*.sol src/*/*.sol`):

| File | Lines | What it is | New since the v1 brief |
| --- | --- | --- | --- |
| `src/AutoBuyerVault.sol` | 486 | v2 vault: releases ETH only to one immutable `buyer` (the canister), bounded by `maxInFlight`, on the dual clock | **new** |
| `src/Wrapper.sol` | 228 | v3: one whole token per receipted frog; burn to a ticket, resolve by drand in order, deliver or refund | **new** |
| `src/WrapClaim.sol` | 178 | v3 step 2: deploys its own Wrapper, holds its supply, pays holders by keeper-published Merkle roots | **new** |
| `src/lib/Wait.sol` | 52 | the dual clock: a wait must pass on both `block.timestamp` and the parent chain's block number | **new** |
| `src/ListingRegistry.sol` | 313 | listings, roles, slow changes; deploys both vault kinds; `listInscriptionsAuto` / `listRuneAuto` | changed |
| `src/Distributor.sol` | 411 | draws and pro-rata claims | changed |
| `src/AssetVault.sol` | 492 | v1 bonded vault | changed (C1 fix) |
| `src/Treasury.sol` | 94 | the 20% share's destination, for the protocol token | not in v1 brief |
| `src/FeeRouter.sol`, `src/Launcher.sol`, `src/rand/*`, `src/lib/BtcAddress.sol`, `src/interfaces/*` | 537 | as in the v1 brief | — |

**In scope — the buyer canister, 4,997 lines** (`autobuyer/canister-rs/src`):

| File | Lines | What it is |
| --- | --- | --- |
| `canister.rs` | 1,350 | entry points, guards (`only_controller`, `only_operator`), the tick guard, stable memory, signing, outcalls |
| `runestone.rs` | 777 | decodes what a Bitcoin transaction's runestone moves (a cenotaph where `ord` would burn) |
| `psbt.rs` | 754 | refuses to sign any PSBT that does not do what the recorded quote says |
| `state.rs` | 461 | the purchase state machine, recorded quote, one-quote-one-transaction binding |
| `btcsend.rs`, `btcnet.rs` | 524 | Bitcoin broadcast: one node sends, every node reads back; network checkpoints |
| `evmsend.rs` | 254 | EVM broadcast (B4): classify a provider's reply; the chain id a signed transaction commits to |
| `delivery.rs` | 242 | the custodian's send rule for the wrapper (stop before the deadline) |
| `sighash.rs` | 232 | BIP341 key-path digest computed in the canister (B3) |
| `bridge.rs`, `venue.rs`, `lib.rs` | 194 | interfaces and the bridge-quote check |

**In scope — the timelock, 493 lines** (`autobuyer/timelock-rs/src`: `rules.rs` 305, `canister.rs` 180, `lib.rs` 8), and **the
deploy scripts' refusals** (`script/DeployChecks.sol`, `script/GoLiveChecks.sol`, `script/Deploy.s.sol`, `script/ListAuto.s.sol`,
`script/DeployWrapClaim.s.sol`, `script/*.sh`) — a wrong constructor argument is permanent for most of these contracts.

**Out of scope:** Pons V2 (verified on a fork, `docs/phase0.md`, `docs/phase1.md`); the IC protocol, threshold ECDSA/Schnorr and the
Bitcoin integration (DFINITY's, audited separately — below); the web app, indexer, keeper and snapshot services (off-chain trust is
the design: `docs/trust-model.md`); `lib/`; test code (read it: it is the specification).

## What changed in the trust model

v1 trusted a bonded human purchaser and a separate attestor. v2 replaces the purchaser with a canister whose keys no person holds;
v3 makes that canister a custodian. New actors, and what stops them:

| Actor | Can take | Can block | What stops them |
| --- | --- | --- | --- |
| **The buyer canister** (its code) | whatever its code signs: ETH pulled from a vault up to `maxInFlight`; BTC it holds | purchases, by not acting (the vault's deadlines release the ETH) | `psbt.rs::verify` on the signing path against a quote recorded before the PSBT exists; one quote, one transaction; the vault's immutable `buyer` and `maxInFlight` |
| **The operator** (runs the loop) | up to 3% of a purchase over the released ETH's reference value, colluding with a venue, plus overpaying within the keeper's cap: every order and quote must pay the listing's own `btcVault`, for its rune and the vault's units (`docs/v2/trust-points.md` §1) | the buyer, by not acting | the chain checks on `open_purchase`, `record_quote`, the loop's quote step and `sign_purchase`; a guardian's pause (which the operator cannot lift); rotation through the timelock |
| **A proposer** (2 or more) | nothing without 72 public hours; anything after them, unless k guardians veto | nothing alone: its one pending slot does not block the others' | the delay, the published hash and argument, a k-of-n veto, the watcher; a roles change removes it after one delay (`docs/v2/trust-points.md` §3) |
| **A guardian** (3 or more, veto k = 2) | nothing | alone, only by pausing the buyer until a roles change removes it (one delay); k together can veto everything | separate custody, different people; no single guardian can veto |
| **An IC subnet** | the threshold keys, if a supermajority of its nodes collude | everything, if it halts | the IC's own security; out of scope |
| **RPC providers / Bitcoin explorers** | nothing directly: every read must agree across providers, replicated across the subnet's nodes | purchases, by disagreeing or going down | agreement on the fields the buyer acts on (`receipt_view`, `status_view`); checkpoints that pin the network |
| **The venue** | overpayment up to the recorded quote's cap | purchases | the quote is recorded before the PSBT; B6's bound is the cap |
| **The custodian** (the same canister, as frog holder) | the frogs, if its code sent them wrongly — but only with the **custody key** (derivation path `custody`), which no purchase path signs with | deliveries, by not acting (the ticket refunds after 7 days) | frogs sit at their own address: the buyer's key and `psbt::verify` cannot reach them; the buyer also refuses to sign any of its inputs ord shows carrying an inscription or a rune (fail closed on mainnet); every send is judged by `delivery::decide` on a fresh pinned read, with a one-day margin; the Wrapper's `custodian` may call `recordDelivery` and **nothing else** (`docs/v3/wrapper.md`, "Custody on its own key") |
| **The keeper, for WrapClaim** | nothing directly | payouts, by not publishing | a root cannot be used for 15 minutes, during which the owner may `cancel`; the list is published |
| **The owner, for WrapClaim** | nothing | a payout, by `cancel` — **only within the 15 minutes** after a root | after those 15 minutes the owner has no power over it |
| **A redeemer** | a frog, by burning a whole token | nothing | tickets resolve strictly in order, each against frogs free when it was burned (`docs/v3/wrapper.md`, decision 3) |
| **The sequencer** | nothing | — | every wait someone relies on is on both clocks (`Wait.sol`, audit S1); the last timestamp-only waits were closed 2026-09-23 ("Single-clock waits", below). Freshness expiries stay on the timestamp, by design |

## Trust assumptions, mapped (2026-09-23, at `9022920`)

Every place the system believes something it cannot check for itself, what that belief can cost if it is wrong, and what bounds
it. "Today" is `9022920`; "H" is `docs/v5/custody-hardening.md`. The T-numbers are `docs/v5/custody-threat-model.md`.

```
                        ┌──────────────── Safe (3/5): roles, listings, btcVault ── 7-day public delay ──┐
                        │                                                                               ▼
 keeper (hot) ── postCap (1 h TTL) ──► SweepVault / NftVaultBase ──► CurveFactory.freshCap ──► ItemQueue.itemCap, LiquidityLock
     │     └──── recordDelivery (Wrapper) ─────────────────────────────────────────────────────► Wrapper tickets
     │
 operator (same host) ── hand-ins: quotes, PSBTs, frog offers, Seaport orders, NFT targets+caps ──┐
                                                                                                 ▼
 timelock (72 h, k=2 veto) ── upgrade + ARGUMENT (providers, evm_chains, operator) ──► BUYER / CUSTODIAN CANISTER
                                                                                      │  keys: btc · custody · float · evm
 RPC providers (4663, 1, 8453) ── pinned reads, all must agree ─────────────────────► │  (one subnet holds them all)
 esplora ×2 + ord ×1 ── UTXOs, deposits, inscription locations ─────────────────────► │
 drand evmnet ── randomness ──► Wrapper / NftWrapper resolve                          │
                                                                                      ├─► Bitcoin: deliveries, returns, frog buys
                                                                                      ├─► chain c: Seaport fills (E_c), NFT custody
                                                                                      └─► 4663: wrapDeposit / mint (on its word),
                                                                                            recordItem / fulfill (paid on its word),
                                                                                            recordDelivery
```

| Who or what is trusted | For what | If it lies or is compromised | Bound today | Bound with H | Enforced at |
| --- | --- | --- | --- | --- | --- |
| **IC subnet (threshold keys)** | signing only what the canister's code asks | every custody key signs anything (T3) | none of ours | the stage cap (H2) | IC; `custodycap.rs` |
| **Timelock and its roles** | only reviewed code, after 72 h | a proposer plus k guardians can install anything, publicly (T1, T2) | 72 h, veto | + argument diff and announcement (H5, H6), ×4 per raise (H2) | `timelock-rs/src/rules.rs` |
| **The upgrade argument** | providers, operator, `evm_chains` | new providers can forge every read (T1, T10) | public for 72 h, not diffed | diffed and flagged CRITICAL (H5) | `canister.rs:159-170` |
| **Operator / relayer** | hand-ins | rune quote: 3% and within cap. Frog offer: keeper cap. **NFT target: any cap, any collection → Σ E_c** (T5) | per hand-in; **the pause does not stop signing** (T6) | policy cap per collection (H2); pause everywhere (H1) | `listing.rs`, `frogbuy.rs`, `seaport.rs`, `nftnet.rs:247` |
| **Keeper** | caps, snapshots, roots, `recordDelivery` | inflated cap → Q overpaid; false delivery → one redeemer's token (T4) | 1 h TTL, no step bound | ±20% per post (H12) | `SweepVault.sol:199`, `NftVaultBase.sol:202`, `Wrapper.sol:267` |
| **The canister as minter** | "this item arrived" | unbacked wX sold into P, Q claimed (T7) | per-item uniqueness; 50 deposits a day **in the canister** | an on-chain rate limit (H11), caps (H2), the canary (H5) | `Wrapper.sol:170`, `NftWrapper.sol:120-138`, `ItemQueue.sol:181`, `MintDesk.sol:79` |
| **4663 RPC providers** | tickets, sinks, desk, listings, vaults | forged tickets → items delivered to an attacker (T10) | all must agree; in practice ~1 operator; **no delivery rate limit** | 6 a day then frozen (H3), finalized reads plus event cross-check (H4, H9), 3 operators (H10) | `pin.rs`, `redeem.rs`, `outflow.rs`, `outbound.rs` |
| **Chain-c RPC providers** | Seaport checks, `ownerOf`, arrival | a bad fill or a mint against an arrival that did not happen | all must agree; the simulation | finalized arrival before a mint (H9) | `nftnet.rs`, `chains.rs` |
| **esplora ×2, ord ×1** | UTXOs, deposit transactions, where an inscription is | a forged deposit → an unbacked mint (T10) | 50 a day; 6 confirmations | + caps (H2) and mint limits (H3, H11) | `deposits.rs:20`, `custody.rs` |
| **Sequencer (4663)** | ordering, soft finality | a reorged burn after its delivery (T9) | nothing | finalized pins (H9) | `pin.rs` |
| **drand** | unbiased, live randomness | stalls redemptions; bias moves value between redeemers (T12) | re-pin after 24 h | — | `Wrapper.sol:219` |
| **OpenSea zone, transfer validators, ERC-2981** | authorising fills, allowing transfers, naming the royalty receiver | a revert (gas); up to `max_fee_bps` of a fill; a stuck NFT (T11) | pinned code hashes; fee cap | — | `seaport.rs` |
| **Web app and DNS** | the destination the user signs | the destination is swapped, final at the burn (T14) | none | H14 | `web/` |
| **Cycles** | the canister stays alive | frozen custody; an uninstall loses in-flight records (T13) | watcher at 5 T | top-up, 90-day threshold, drills (H13) | `canister.rs:1181` |

## Invariants we believe hold, and the tests that check them

v1's seven are in `docs/security/audit-brief.md`. Added since:

| # | Invariant | Checked by |
| --- | --- | --- |
| 1 | ETH leaves an `AutoBuyerVault` only as a pull to its immutable buyer, exactly what was pulled | `test/AutoBuyerVault.invariant.t.sol::invariant_ethOutIsExactlyWhatWasPulled` |
| 2 | the v2 vault's books equal its balance; every contributed wei is accounted for | `…::invariant_theBooksMatchTheBalance`, `…::invariant_everyContributedWeiIsAccountedFor` |
| 3 | no required state transition ever panics | `test/AutoBuyerVault.invariant.t.sol`, `test/Wrapper.invariant.t.sol::invariant_noRequiredTransitionEverPanics` |
| 4 | a wait cannot be shortened by moving `block.timestamp` alone | `test/security/SequencerClock.t.sol` (5 tests) |
| 5 | wrapper supply = (frogs held − open tickets) × 1e18 — the corrected formula (`docs/v3/wrapper.md`) | `test/Wrapper.invariant.t.sol::invariant_supplyIsHeldLessOpenTickets` |
| 6 | every held frog is in exactly one place; every token is somewhere visible | `…::invariant_everyHeldFrogIsInExactlyOnePlace`, `…::invariant_everyTokenIsSomewhereWeCanSee` |
| 7 | tickets resolve in order | `…::invariant_ticketsResolveInOrder` |
| 8 | WrapClaim: the wrapper's ledger holds; every token it holds is accounted once; no distribution pays past its total; claims pay exactly and nothing is stuck | `test/WrapClaim.invariant.t.sol` (5 invariants) |
| 9 | the canister signs only a PSBT that satisfies the recorded quote, and computes the digest from the bytes it verified | `autobuyer/canister-rs/src/psbt.rs`, `sighash.rs` tests; `autobuyer/sighash-e2e.mjs` against `@scure/btc-signer` on a replica |
| 10 | a quote is recorded once, only from `Funded`; one quote signs one transaction | `src/state.rs` tests; `sighash-e2e.mjs` refusals |
| 11 | every canister update method has a caller guard | the test in `src/canister.rs` that reads every `#[ic_cdk::update]` |
| 12 | a timelocked change is public for ≥ 72 h and vetoed by k of n guardians; a proposer withdraws only its own; a stale proposal fails; roles change only by a delayed, vetoable proposal; `execute` never reinstalls | `autobuyer/timelock-rs/src/rules.rs` tests; `rehearse.sh` and `rehearse-roles.sh` on a replica |
| 14 | an order and a quote pay only the listing's own `btcVault`, for its rune and at least the vault's announced units, within 3% of the released ETH's reference value; checked again before anything is signed | `src/listing.rs` tests (BIP-173/350 vectors, a `cast`-encoded listing); `tests/loop.rs` (three refusals) |
| 13 | nothing is believed from one provider, one node, or a whole body a provider decorates | `src/evmsend.rs` (two real 4663 receipts), `src/btcsend.rs` tests |
| 15 | NFT wrapper: `totalSupply + openTickets × 1e18 == held × 1e18`; every held item in exactly one place; no token id held twice; tickets resolve in order | `test/nftwrap/NftWrapper.invariant.t.sol`, `test/nftwrap/NftWrapperSecurity.t.sol` |
| 16 | the queue: every wei in is spent, queued, held, returned or released; units assigned are exactly items bought; FIFO, no coin starved by a later one | `test/ItemQueue.invariant.t.sol` |
| 17 | shared liquidity only grows and equals the units; coin-pool liquidity never moves; fees in equal fees credited; the lock covers what it owes; the router holds nothing | `test/pools/Pools.invariant.t.sol` |
| 18 | NFT sweeps: never over the cap; the ledger conserves and the balance covers it; NFTs in = bought − burned; the guard refuses tips, empty offers, averaging, other collections and self-listed orders | `test/nft/NftSweepVault.invariant.t.sol`, `test/nft/NftSeaportGuard.t.sol` |
| 19 | (H2–H4, wired at `2562c0e`) custody never takes in past its caps; outbound acts are rate-limited and a run freezes custody; a custody spend pays only the ticket's destination (or the sender, or custody). Against the wired flows: forged tickets drain six then trip the freeze; a delivery to a destination the finalized ticket does not name is refused before saving; an over-cap deposit is returned; the operator cannot target above the cap | the modules' 32 unit tests; `tests/hardening.rs` (9); `tests/redeem.rs` and `tests/frogs.rs` (the hardening sections) |

## Known issues and accepted risks

Collected from the internal passes; each links its source. **Open** means not fixed.

| Source | Issue | Status |
| --- | --- | --- |
| `docs/deploy/canister-mainnet.md` | the buyer cannot be installed for Bitcoin mainnet: `KNOWN_NETWORKS` is signet and testnet4, and mainnet has no checkpoint | **closed** 2026-09-25, `df2f13c`: `"mainnet"` and the block 900,000 checkpoint (its check: `canister-mainnet.md`, "The published C7 check") |
| `docs/deploy/canister-mainnet.md` | only one RPC provider for 4663 is reachable from the IC (Blockscout's is behind a Cloudflare challenge); the buyer needs two | open — operational |
| `docs/deploy/canister-mainnet.md` | the operator key holds the in-flight funds until quotes come from the venue and recipients from the vault | **closed** 2026-09-23, `docs/v2/trust-points.md` §1: the power that remains is in the actor table |
| `docs/deploy/canister-mainnet.md` | `Init.vault` is never read; an upgrade cannot change it; one field for possibly several vaults | open, recorded |
| `docs/deploy/canister-mainnet.md` | after the handover nobody can read the buyer's cycles (`canister_status` is controllers-only) | **closed** 2026-09-23: public `cycles()` on both canisters, the watcher alerts (§2) |
| `docs/deploy/canister-mainnet.md` | every replicated read reserves 64,000 response bytes, ~93% of its cost | open, cost only |
| from the WrapClaim work (via the coordinator) | `Distributor.claimShare` / `rollOver` check a rune distribution's `claimsEnd` on `block.timestamp` alone (`src/Distributor.sol:344`, `:364`): a sequencer an hour ahead can close rune claims up to an hour early, contrary to `Wait.sol`'s dual-clock policy | **fixed** 2026-09-23 with nine more of the same kind — "Single-clock waits", below |
| `docs/v3/wrapper.md` | a late delivery and a refund can both happen: the redeemer gets the frog and the token back, and supply is backed by a frog that is gone. No contract can stop it; the canister must stop sending a day before the deadline (`src/delivery.rs`) | accepted on chain; the rule is wired since (`src/redeem.rs`, `rule`, before every send and re-send). The leaked-bytes variant is open: T8 / H8 below |
| `docs/v3/wrapper.md` | one skipped drand round holds the tickets behind it for up to 24 h (the price of in-order resolution) | accepted |
| `docs/v3/wrapper.md` | a frog that leaves Bitcoin without this chain knowing breaks supply = backing | accepted — the irreducible custody risk |
| `docs/v3/wrapper.md` "Why this custodian" | its text still lists B3, B4 and the timelock as open; all three are closed | stale text, not a defect |
| `docs/security/audit-v2.md` S3 | the buyer signs its own receipt: no second opinion on what Bitcoin did | **open — the decision for an auditor** |
| `docs/security/audit-v2.md` S6 | no monitoring of contract events, no published security contact, no incident-response plan | open (the timelock watcher is the first piece) |
| `docs/security/audit-v2.md` S7 | `buyer` and `maxInFlight` are immutable: a lost buyer strands its vault for ever | accepted, deliberate |
| `docs/security/audit-v2.md` | M2, M3, M5, M7, M8, M9, L1–L9, I1–I5 | open, unchanged since the second pass |
| `docs/security/audit-btc-v1.md` B5 | `Psbt::verify` returns `ours`, a superset of `to_sign` | open, recorded |
| `docs/security/audit-btc-v1.md` B6 | `ChangeNotOurs` cannot fire on a transaction that buys anything; the bound is the cap | open, test documents it |
| `docs/v2/timelock-controller.md` | does not protect against: the operator acting within the code; a proposer and guardian who collude; nobody watching; the IC; a bad build that matches its hash. A module over ~2 MB needs chunked install, not built (the buyer is 1.6 MB) | accepted |
| `docs/deploy/key-ceremony.md` | a stolen proposer or a stolen guardian leaves the buyer permanently un-upgradable (the timelock is blackholed and cannot be reconfigured) | **closed** 2026-09-23 (§3): sets, a k-of-n veto and delayed roles changes bound any single key to one delay. What remains, accepted: k guardians colluding can freeze the code; losing every proposer ends upgrades |
| `docs/deploy/contracts-mainnet.md` | `ListingRegistry` is 40,845 bytes (39,540 before the single-clock fix): deployable on 4663 (published limit 96 KB; `eth_estimateGas` on 4663 accepts it), not on a chain with Ethereum's 24,576. Fix options (vault factories, CREATE2, clones) are listed there | accepted — portability; the coordinator's decision |
| "Single-clock waits", above | freshness expiries (`capExpiry` in both vaults, `Distributor.minimumExpiry`) read the timestamp alone: a sequencer running behind keeps a stale cap or minimum valid longer | open — needs an either-clock expiry and a staleness decision |
| `docs/deploy/contracts-mainnet.md` | the owner-is-a-multisig check accepted a single key with an EIP-7702 delegation | **fixed** 2026-09-23, `test/GoLiveChecks.t.sol` |
| custody review (T6) | the guardian's pause is read only in `tick`: `sign_purchase`, `record_quote`, `nft_set_target`, `nft_sign`, `nft_send` still work while paused | **closed** 2026-09-23, H1 at `97178ad`: `not_paused()` in every operator method that acts, with a source test |
| custody review (T5) | `nft_set_target`'s cap is the operator's word, for any collection: a stolen operator key buys its own junk NFT for the whole balance E_c on each chain | **closed** 2026-09-23, H2 at `2562c0e`: `nftnet::record_target` refuses an unlisted collection and a cap over the collection's per-item maximum; exposure is bounded by the stage caps |
| custody review (T10) | deliveries have no rate limit: forged 4663 reads empty custody at Bitcoin's pace | **closed** 2026-09-23, H3 at `2562c0e`: 3 items a day a collection at stage 0, the tripwire pauses the buyer at three times a limit; what remains is H10 (3 independent providers) |
| custody review (T8) | a delivery broadcast, evicted and then refused leaves valid signed bytes that can be mined after the refund | **open**, fix H8 (spec; H4's `ConflictOut` built and tested; H4 at `2562c0e` refuses a mis-built delivery before it is ever broadcast, which narrows T8 to the eviction case) |
| custody review (T9) | outbound decisions read 4663 at sequencer-soft pins, not L1-finalized blocks | **partly closed** 2026-09-23 (`2562c0e`): the ticket and its `Redeemed` log are read at the finalized pin before a delivery is signed (H4/H9); a deposit's EVM side and NFT arrivals are still at the fast pin (H9, open) |
| custody review (T7) | no wrapper bounds its mint rate on chain; a compromised custodian mints in one block | **built** at `605aab0` (H11), **not as specified** (2026-09-25, docs/v6/phase1-scope.md §7 item 3): the limit is `maxMintsPerDay`, a default of 20 a day, set once by the deployer's `initMintLimit` (through `WrapClaim` from the registry's owner) and raisable by `proposeMintLimit` + `acceptMintLimit` after 7 days — not an immutable. `moduleMint` (the Floor Module's ETH-backed mints) does not count against it. Stage 0 sets it to 3 at step 7 of the go-live, before any mint |
| custody review (T4) | `postCap` takes any value with no step bound | **built** at `605aab0` (H12), **with a gap** (2026-09-25, docs/v6/phase1-scope.md §7 item 3): a post while the cap is fresh is bounded to a step and a 10-minute gap (`SweepVault.sol:247-258`), but once the cap is stale a re-post is bounded only by `capCeiling`, and the ceiling is 0 (none) until the owner's `proposeCapCeiling` is accepted after 7 days. With no ceiling a stale re-post is unbounded. Go-live step 7 proposes the ceilings (`ETCH_CAP_CEILING`); closing the gap in code is a Solidity change, not made here |
| custody review (T13) | saved state grows without bound inside a `pre_upgrade` that must succeed; an uninstalled buyer cannot be upgraded by the timelock | **open**, fix H13 |
| custody review (T1) | the upgrade argument replaces every provider and all of `evm_chains`; the watcher does not diff it | **open**, fix H5 |
| `docs/v4/eth-nft-canister.md` §12.7 | does OpenSea issue SignedZone signatures to an unauthenticated fulfiller? | open, needs the user's live check |
| `docs/v4/nft-wrapper.md` §9 | the canister's NFT mint, delivery and reconciliation are spec only | open, launch gate C10 |

## Single-clock waits (2026-09-23, written before the fix)

**The finding.** `Wait.sol`'s policy is that any period someone relies on must pass on both clocks: the sequencer's
`block.timestamp`, which Nitro lets run up to an hour ahead of wall-clock, and the parent chain's `block.number`, which it
cannot run forward. The Distributor's rune `claimsEnd` broke that policy (above). A sweep of every `block.timestamp` read in
`src/` found nine more waits of the same kind — each one a period a person relies on, measured on the sequencer's clock
alone, so a sequencer stamping ahead can shorten it by up to an hour:

| Contract | Wait | Who relies on it | Worst case today |
| --- | --- | --- | --- |
| `Distributor` | `claimShare` closes at `claimsEnd` | rune holders claiming a share | claims close up to 1 h early |
| `Distributor` | `rollOver` opens at `claimsEnd` | the same holders (what is rolled over is no longer theirs to claim) | the same hour |
| `Distributor` | `redraw` after `REDRAW_AFTER` (24 h) | anyone who would resolve the pinned round | a redraw up to 1 h early |
| `Distributor` | `voidDraw` after `VOID_AFTER` (7 d) | a winner with 7 days to claim | the owner voids up to 1 h early |
| `Distributor` | `flagOverdue` after `DELIVERY_DEADLINE` (7 d) | the keeper's 7 days to deliver | flagged overdue up to 1 h early |
| `Distributor` | `flagPayoutOverdue` after `DELIVERY_DEADLINE` | the same, for payouts | the same |
| `AssetVault` | `writeOff` after the settle `deadline` (3 d), and the public write-off 7 d later | the purchaser's 3 days to settle | written off up to 1 h early |
| `AssetVault` | overdue blocks the purchaser (`BlockedByOverdue`, `blocked()`) | the purchaser | blocked up to 1 h early |
| `AutoBuyerVault` | `writeOff` after `deadline`, public write-off, `blocked()`, and `settle`'s `late` flag | the buyer's 3 days to settle | written off, blocked or marked late up to 1 h early |
| `AutoBuyerVault` | anyone may cancel after `readyAt + PUBLIC_CANCEL_AFTER` (7 d) | the buyer's 7 days | public cancel up to 1 h early |
| `ListingRegistry` | every `CHANGE_DELAY` (7 d): roles, membership root, BTC vault, a lowered bond, the treasury | everyone watching a governance change | a change takes effect up to 1 h early |

An hour of a 3- or 7-day wait is small; an hour of a 30-day claim period is small. None of these moves funds by itself. They
are fixed because the policy is only worth anything if it has no exceptions, and because an auditor would rightly ask why a
seven-day governance delay is the one wait the sequencer can shorten.

**The fix.** Each wait gets its parent-block half, recorded where the timestamp is recorded and checked with
`Wait.elapsed`; a strict `>` on the timestamp keeps its meaning (`elapsed(t + 1, b)`). The blocks go into **new storage
declared last in each contract** (side mappings and variables), so no existing struct, view, event or error changes shape:
the web app decodes `distribution()`'s tuple exactly (`web/src/lib/abis.ts`), the canister decodes the vaults', and
`script/golive-check.sh` reads the registry — none of them has to change. The ABI only gains read-only getters for the new
block fields.

**Deliberately not changed.** Freshness expiries — `capExpiry` in both vaults and the Distributor's `minimumExpiry` — are not
waits anyone relies on: a sequencer running ahead only makes them expire early, which refuses rather than harms. Their real
weakness is the other direction (a sequencer running up to 24 h *behind* keeps a stale cap "fresh" longer), which needs the
opposite composition — expired when *either* clock says so — and a decision about how stale a cap may be. Recorded as open,
below. The drand pin `roundAt(block.timestamp) + PIN_AHEAD` is not a wait at all (docs/phase3.md M2).

## What we most want attacked

1. **The canister's signing path** — `sign_purchase`, `psbt.rs`, `sighash.rs`, `runestone.rs` — against a hostile venue: anything
   that gets a signature on a transaction that pays other than the recorded quote says.
2. **The recorded quote and the binding** — `state.rs`, `record_quote`, the bind before the await — under interleaved calls
   (DFINITY: messages have no reliable ordering across an `await`).
3. **The read-back rules** — `receipt_view`, `status_view`, the checkpoints — anything that makes providers "agree" on something
   false, or lets one node's report be believed.
4. **The timelock** — any path to code on the buyer without 72 public hours, or to a `reinstall`.
5. **The wrapper and WrapClaim** — supply versus backing under every ordering of wrap, burn, resolve, deliver, refund, open,
   cancel and claim; and the draw's steerability.
6. **S3**: is a buyer that signs its own receipt acceptable for a custodian, and if not, what is the cheapest second opinion?
7. **Custody outflow** — every path by which the custody, float or EVM keys sign: `redeem.rs`, `frogflow.rs` (returns, frog buys),
   `nftnet.rs`. Anything that moves an item or value to anyone but a ticket's destination, a deposit's sender or ourselves.
8. **The canister as minter** — `wrapDeposit`, `NftWrapper.mint`, `ItemQueue.recordItem`, `MintDesk.fulfill`: any way to mint or
   be paid without an item arriving, and what that reaches in the pools (`LiquidityLock`).
9. **The keeper's cap as a lever** — `postCap` → `freshCap` → the queue's `itemCap`, the opening target, the pool-add guard and
   the center: the largest loss one keeper key can cause within an hour.
10. **The hardening modules** (`custodycap.rs`, `outflow.rs`, `outbound.rs`) and their wiring as specified: a way around a cap, a
    limit or the tripwire, or a transaction the outbound check accepts that pays someone the purpose does not name.

## How to build and run everything

```
# Solidity (Foundry; solc 0.8.28 via-IR)
forge build --sizes
forge test --no-match-path "test/fork/*"                        # 278 tests at 8258c77; 611 at 9022920
FOUNDRY_PROFILE=deep forge test --no-match-path "test/fork/*"   # 10,000 fuzz runs, deep invariant campaigns
forge test --match-path "test/fork/*"                           # against the public 4663 RPC and the live Pons factory
bash script/deploy.test.sh                                      # the deploy script's refusals
scripts/golive-dryrun.sh all                                    # the whole go-live on a bare node, a fork of 4663, and 4663/46630 refusals
slither . --filter-paths "lib/|test/|script/" --exclude-informational --exclude-optimization

# the buyer and the timelock (Rust)
(cd autobuyer/canister-rs && cargo test)                        # 106 unit tests + the e2e test at 8258c77; 315 at custody-security; 356 at 2562c0e (H1–H4 wired)
(cd autobuyer/canister-rs && cargo test --test hardening)       # H2–H4 integration, 9 tests (the modules' 32 unit tests run with the library's)
(cd autobuyer/timelock-rs && cargo test && cargo test --features rehearsal)
(cd autobuyer && scripts/reproducible-build.sh)                 # the canonical Docker build; prints the module hashes
(cd autobuyer && npm ci && npm test)                            # BIP341, signet, replica, EVM and runestone checks in JS

# on a local dfx replica
(cd autobuyer && node sighash-e2e.mjs)                          # the canister's digest against @scure/btc-signer, and #23's refusals
(cd autobuyer/timelock-rs && ./rehearse.sh)                     # the timelock: handover, delay, veto, execute, failed upgrade
(cd autobuyer/timelock-rs && ./rehearse-roles.sh)               # sets, a 2-of-3 veto, stale proposals, roles changes, cycles
node --test scripts/timelock-watch.test.mjs                     # the proposal and cycles watcher
```

Rehearsals on public test networks (`evm-rehearsal.mjs`, `signet-rehearsal.mjs`, `runes-rehearsal.mjs`) need funded test keys and
are for the operator; their results are recorded in `docs/v2/2026-09-22-evm-and-runes-rehearsals.md` and
`docs/v2/2026-09-22-signet-rehearsal.md`.

## Who could audit this

The work spans two stacks: Solidity on an Arbitrum chain, and a Rust canister that uses the IC's threshold signatures, HTTPS
outcalls and Bitcoin. Only what was read is written here.

| Who | Relevant work found | Source read | Price / lead time |
| --- | --- | --- | --- |
| **Trail of Bits** | DFINITY's own auditor for "Threshold ECDSA Integration and Bitcoin Canisters" (2022-09-05), "ckBTC and BTC Integration Review" (2023-10-06), and in 2025 **Orbit** (DFINITY's canister-governance product — the nearest thing to our timelock) and OISY; also the maturity rubric `docs/security/audit-method-v1.md` uses | [DFINITY: third-party security audits](https://dfinity.org/third-party-security-audits); [DFINITY forum, audit #5](https://forum.dfinity.org/t/threshold-ecdsa-integration-and-bitcoin-canisters-security-review-by-trail-of-bits-third-party-security-audit-5/15952) | not published; request a quote |
| **NCC Group** | "Threshold ECDSA Cryptography Review" (2022-06-16) and "VetKeys Cryptography Review" (2025-08-15) for DFINITY — the cryptography under our keys, not application code | [DFINITY: third-party security audits](https://dfinity.org/third-party-security-audits); [NCC Group's public report](https://www.nccgroup.com/research/public-report-threshold-ecdsa-cryptography-review/) | not published |
| **Code4rena** (contest) | a competitive review; since June 2025 no platform fee — the sponsor funds the prize pool (a 96% conditional pool, refunded if nothing significant is found) and the judge's fee | [Zellic's announcement, 2025-06-17](https://www.zellic.io/blog/code4rena-free-contests/) | the pool and judging fee; length not stated there |
| **Oak Security**, **Zellic**, **Halborn** | named in search results as Rust and cross-chain auditors (Oak: "30+ audits" in L1s, interoperability, light clients, bridges); **no ICP canister audit of theirs was confirmed** — Halborn's audit list refused the request (HTTP 429) | search result summaries only: [oaksecurity.io](https://oaksecurity.io/), [halborn.com/audits](https://www.halborn.com/audits) | not published |

A reasonable shape: one firm with published IC work (Trail of Bits is the only one confirmed here) for the canister, the timelock and
the vault; and a contest for the Solidity surface, which is where a crowd finds most. Ask any firm for an IC canister report they
have published before engaging them on the Rust.
