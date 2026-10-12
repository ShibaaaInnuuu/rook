# Rook on Solana: the migration — design and plan

Status: **design, 2026-10-09**, branch `golive-2026-10-08`. Nothing deployed anywhere. The operator's decision the same day: "there is
no volume on Robinhood, we have to launch on Solana." The Robinhood Chain launch stopped at tag `robinhood-stop-2026-10-09`
(`docs/v9/robinhood-stop-2026-10-09.md`); it stays as the fallback.

**Legend.** **[R]** read from this repository (`ref-robinhood-contracts.md`, `ref-robinhood-offchain-deps.md`, both made from the tag).
**[W]** web research of 2026-10-09 (`ref-solana-research-2026-10.md`, sources there). **[M]** measured live that day. **[D]** a decision
this document makes; **[Q]** an open question with a named spike.

---

## 0. The one sentence, and the invariant that must survive

Rook is a memecoin launchpad whose coins are **backed by real blue-chip NFTs**: every coin raises on a bonding curve; at
graduation the raise's item budget is converted, at the collection's floor, into units of a wrapped-NFT token (`wX`, one unit = one
NFT); the coin's liquidity is paired with `wX`, not with the gas coin; an off-chain custodian (an Internet Computer canister with
threshold keys) turns those units into real NFTs on Ethereum and real inscriptions on Bitcoin, held in custody; a `wX` holder can
redeem a unit for the delivery of a real NFT. **[R]**

The invariant: **a coin's liquidity is `wX`, and every `wX` unit is either an NFT in custody or SOL in the floor module's books at
the floor.** Everything in this plan is judged by whether it keeps that. The Ethereum side (buying with Seaport, custody of ERC-721s)
and the Bitcoin side (PSBT judges, Taproot custody, ordinals marketplaces) do not touch the home chain and carry over unchanged
**[R]**; what moves is the home chain: the contracts, the canister's home-chain modules, the services that read the home chain, and
the site's wallet.

## 1. What moves, what stays

| Layer | Lines **[R]** | Stays | Moves |
|---|---|---|---|
| Contracts `src/` | 14,146 Solidity (≈6k live logic) | nothing (frozen at the tag as reference) | rewritten as Solana programs (§4) |
| Contract tests `test/` | 31,348 | the invariants, as a checklist | rewritten (Anchor / LiteSVM / fuzz) |
| Canister `autobuyer/canister-rs` | 41,259 src + 24,615 tests | ≈11–12k src: Bitcoin core, both PSBT judges, ord/UniSat/OW, Seaport, Ethereum pin, pure rules, timelock | ≈14–17k src + ≈10–12k tests: the EVM layer in `canister.rs`, `evmread/pin/evmsend/evmtx/abi/ticketlog`, home steps of `ethflow`, `convflow/relayconv` origin, `frogflow` adapters, `redeem/custodynet/delivery`, `deposits` claim, `rates/oracle`, `chains`, `custodycap` keys |
| Services | ≈9k touched | OpenSea/ordinals/UniSat fetching, drand, Bitcoin canary, IC watcher, alerts, indexer API/db | keeper signer+posts, runner Relay legs, fueler, reconciler, indexer follower, monitor home canary + guardian pause, `golive-check`, dry run |
| Web `web-rook/` | ≈3–4k touched | UI, charts, v8 core (drand, odds), content, API types | wagmi/viem → `@solana/kit` + Wallet Standard; `TxButton`; quotes; deployment loaders |
| CI | — | `rust`, `depth-probe`, node suites of the kept services | `contracts`, `fork`, size limits, `golive-check` harness |

## 2. The decisions

### 2.1 The curve: Meteora DBC, paired with SOL; Rook is the partner **[D, operator 2026-10-09]**

**Superseded decision.** This section first chose our own curve program so the coin could trade against `wX`. The operator decided
the same day that **a coin paired with SOL is fine**, and chose **Meteora's Dynamic Bonding Curve** after weighing pump.fun (no
partner configuration: nothing could force any share of a raise into NFTs) and Raydium LaunchLab (trading fees only, no share of the
raise) **[W]**.

How DBC carries the backing:
- **Rook's partner config** (one per collection, or one shared; §3 `rook_backing`): the curve shape, SOL as the quote mint, plain SPL
  coins (no Token-2022 extensions), Rook's partner trading-fee share, an anti-sniper fee schedule at launch, graduation to **DAMM v2**
  with the LP locked.
- **The backing share**: DBC's `migration_fee.fee_percentage` (≤ 99 % of the migration threshold **[W]**) is paid out of every raise
  at graduation to the config's `fee_claimer`, which is a **PDA of `rook_backing`**. Starting value **50 %**, a setting to be modelled
  (pool depth against backing per coin) before launch **[Q9]**.
- **The per-coin backing vault** (operator's choice of the recommended option): after graduation, anyone cranks `back(coin)`:
  `rook_backing` signs `withdraw_migration_fee` as the claimer, converts the SOL into `wX` through the floor module's `convert`
  (the EVM `enqueue` path: at `floor × (1 + convBps)`, minted as ETH-backed units, `reserveBps` of it never converted), and holds the
  units in the coin's vault. **Any holder can burn coins for their pro-rata share of the vault's `wX`** (`redeem_backing`), so every
  coin has a hard floor of "this much NFT per coin", and arbitrage holds the pool price above it.
- Partner trading fees (curve and, through the locked LP's fee share, after graduation) go to `rook_treasury`, which splits them
  between collection reserves and ops as before.

What the invariant becomes: **every `wX` unit is an NFT in custody or SOL in the floor module's books** — unchanged; and **every
graduated coin is redeemable for its vault's `wX`** — new, replacing "the coin's liquidity is `wX`". The visibility question (do the
trading terminals list DBC coins during the curve phase) is open **[Q1]**; after graduation every terminal indexes DAMM v2.

### 2.2 The DEX for graduation: Meteora DAMM v2 (DBC's migration target) **[D]**

With DBC (§2.1) graduation and the locked pool are Meteora's own; `rook_lock` and the shared `wX`/SOL pool's seeding below are
**not built at launch**. A `wX`/SOL pool for the wrapped tokens themselves is a later, separate step. The text below is kept as the
own-curve design.

`LiquidityLock` (829 lines) holds Uniswap v4 positions it never withdraws and collects fees from **[R]**. DAMM v2 has permanent
liquidity lock and configurable fee claimers natively, and is Jupiter-routed **[W]**. Plan: graduation makes a COIN/`wX` pool
(full range) and adds to one shared `wX`/SOL pool per collection, both owned by a lock PDA that can only claim fees. The EVM shared
pool's three concentrated layers (near/wide/far) **[R]** are dropped at first; if depth near the floor matters, Orca Whirlpool
positions held by the same PDA come back as a later step. **[Q2]**: a spike proves the CPIs — create pool, add locked liquidity,
claim fees — from a program-owned PDA within the CU and account limits (our program → DAMM v2 → Token → … is 3 of 4 CPI levels).

### 2.3 Denomination: SOL, with floors posted in lamports **[D]**

Coins raise SOL. NFTs are priced in ETH (Ethereum) and BTC (ordinals). The EVM Frogs module already takes a **BTC floor posted in
wei by the keeper** **[R]**, so "the poster converts to the home currency at post time, the 3 h TTL and the ±20 %/day clamp bound the
drift" is the existing design; it extends to every collection: the poster converts the ETH or BTC floor into lamports. The
canister's advance is in SOL and converts through Relay at execution; the existing `convBps` margin (3 %) and `returnAdvance`
shortfall path absorb the slip, and the advance gate (`ADVANCE_FEE_EST_BPS`, `ADVANCE_MARGIN_BPS` **[R]**) gets a SOL/ETH (and
SOL/BTC) term from `rates.rs`, which today reads ETH/BTC from tickers and the IC exchange-rate canister. Pyth has SOL/USD, ETH/USD and
BTC/USD **[W]**; an **on-chain sanity band** (refuse a post more than X % off Pyth's cross) is a later addition, not a dependency.
No NFT-floor oracle exists on Solana (Switchboard shut down 2026-09-25 **[W]**): the keeper and the canister post, as today.

### 2.4 The canister's home-chain layer: threshold Ed25519, saved bytes, durable nonces **[D, Q3]**

- **Keys.** A new threshold **Ed25519** key (`sign_with_schnorr`, algorithm `ed25519`, ≈26 B cycles ≈ $0.035 a signature **[W]**) is
  the canister's Solana identity (`sol_address()`); the secp256k1 `"evm"` key stays for Ethereum; the Bitcoin Schnorr paths are
  untouched. There is no local Ed25519 test key: the rehearsal is on IC mainnet with `test_key_1`, as the IC-outcall rehearsal was.
- **The invariant "sign once, save the bytes, rebroadcast until mined, at most one mines"** (`frogflow::send`, `ethflow::HomeTx`
  **[R]**) is what makes every home-chain step idempotent across ticks. Solana transactions expire with their blockhash (≈60–90 s)
  **[W]**, which would break it. **Durable nonces** keep it: one nonce account per transaction line (the home line is one line
  canister-wide today), the nonce advanced by the transaction itself, so the saved bytes stay valid until mined or explicitly
  replaced. Durable nonces are slated for removal with no date, after an unaudited replacement ships **[W]**; so `solsend.rs` hides
  the mechanism behind one trait and the fallback (`getSlot` rounded + `getBlock` blockhash, re-sign on expiry with the same
  intent id) is implemented but not default. **[Q3]** spike: a canister on IC mainnet signs and lands a durable-nonce transfer on
  devnet and on mainnet, and the "same bytes resent" path is exercised.
- **Reads.** Today every read is an `eth_call` at an agreed block hash through replicated outcalls to two providers **[R]**. Solana
  RPC has no historical account reads **[W]**, so agreement is on `finalized` state with the context slot stripped by the transform;
  a disagreement between providers is **refused and retried next tick**, never guessed (the pin-disagreement rule today). Two
  routes, both kept: our own outcalls to ≥ 2 providers (Helius and Triton/dRPC/PublicNode) with transforms, as now; or DFINITY's SOL
  RPC canister (3-of-N consensus, no keys, ≈1–5 B cycles a call **[W]**; its canister id must be pinned from the NNS, the docs
  disagree **[W]**). **[D]** start with our own outcalls (same shape as today's code, no new trust), evaluate the SOL RPC canister as
  the second route once the ids are confirmed.
- **Events.** Ticket and deposit scans read logs at finality (`ticketlog.rs`) **[R]**. Solana logs truncate **[W]**; the programs keep
  the state the canister needs **in accounts** (a ticket account per redemption, a deposit record per claim), read at `finalized`,
  so no event is load-bearing. Anchor events are emitted for the indexer only.
- **Clocks.** `Clock {time, parent_block}` (Arbitrum L1 blocks) **[R]** becomes `Clock {unix_time, slot}`; deadlines in the programs are
  in unix time (3 h TTL, 7-day delivery, 72 h refund windows) with slots only for ordering.

### 2.5 Governance: Squads v4 with a 72 h time lock; the canister's own timelock unchanged **[D]**

**Superseded for the launch by D-g (§2.9, 2026-10-09):** the upgrade authority and the registry owner are the operator's single fund
wallet. The Squads plan below is where control moves later.

- **Upgrade authority** of every Rook program: a **Squads v4** vault PDA, `time_lock = 259200`, `config_authority = default` (autonomous,
  so no single key can shorten the lock) **[W]**. This replaces "no contract is upgradeable" **[R]** with "upgradeable only through a
  public 72 h delay": a new chain and new code make a no-upgrade launch reckless; the published path is to burn the curve and token
  programs' upgrade authority after a quiet quarter, keeping the floor module and wrappers behind the timelock.
- **Owner powers** (today the Safe through `ListingRegistry.owner()` **[R]**) become the Squads vault signing instructions of the
  registry program; the 7-day delays on roles, roots and cap raises are kept as program-enforced `proposed_at + 7 d`.
- **Guardian**: a separate key (or a second, no-lock Squads) whose only instructions are `pause_keeper`, `pause_poster(collection)`
  and `cancel_draw` — stop-only, as today.
- **The canister's 72 h timelock** (`timelock-rs`, zero home-chain references **[R]**) stays exactly as it is.
- **Verifiable builds**: `solana-verify` + the OtterSec remote job, PDA written through Squads **[W]**; replaces the reproducible
  `buyer.wasm` story for the programs (the canister's stays).

### 2.6 Randomness: drand, verified on-chain with the alt_bn128 syscalls **[D, Q4]**

Redemption draws and raffle draws pin a drand evmnet round and verify its BLS signature with the BN254 pairing precompile **[R]**.
Solana has `alt_bn128` group and pairing syscalls **[W]**; `DrandEvmnet` + `BN254HashToCurve` (205 lines) port directly and the
off-chain tooling (`keeper/service/drand.ts`, the resolver, the site's verify) stays. **[Q4]** spike: the pairing check fits in a
transaction's CU budget (1.4 M **[W]**). Fallback: MagicBlock VRF (live, ≈0.0005 SOL a request **[W]**).

### 2.7 Scope for the Solana launch **[D]**

ON: curve launches and trading; graduation into locked DAMM v2 pools; the floor module (post, mint, burn, convert, advance / settle /
return); Ethereum NFT wrappers and the Bitcoin Frogs wrapper; redemption and delivery; the treasury (platform share, ops share);
dormant collections and the wake desk; the collection directory; poster pause and keeper pause. Collections: the eleven Ethereum
ones (Identity MD included, OpenSea-alone D8) and Bitcoin Frogs, with the same custody caps and A4-L-1 exit rule (`custodycap.rs`
is pure and stays **[R]**).

OFF at launch (as on 4663 or newly): sweep vaults, raffles and draws (they were half-off; dropping them removes the keeper's cap
posts and the draw machinery from the first audit), YOYO, mint desk, offer board, rentals, holder deposits (Ethereum deposits were
ON; they return in the first post-launch release: the claim format must change anyway, §5), the platform token (never existed).

### 2.8 Relay legs **[Q5]**

Today: home ETH → BTC (`relayconv`, EIP-712 order recomputed and bound by the `depositNative` id) and home ETH ↔ Ethereum
(`relayeth`) **[R]**. On Solana the deposit is an instruction to Relay's depository program (`deposit_native(amount, id)`), built from
the quote's `instructions[]` and lookup tables, ≤ 1,232 bytes **[W]**. The canister's judge must re-derive the binding: that the
instruction's `id` is the order hash whose recipient, minimum and refunds it recomputed. **Measured 2026-10-09 [M]:** SOL→ETH
quotes fine (0.21 % fee, ≈4 s); **SOL→BTC and ETH→BTC quotes failed** ("no UTXOs available") in that test — possibly the test
recipient, since 4663→BTC quotes were judged live in September. **[Q5]** spike with the Relay API key (required from 2026-10-02
**[W]**): SOL→BTC to a real taproot float address, SOL→ETH to the canister's Ethereum address, and a refund; and the Solana
depository program's `execute_transfer` / allocator read for payout recognition (`home_payout_tx` today **[R]**).

### 2.9 Decisions, 2026-10-09 (the operator, after canister phase B2) **[D]**

| # | Decision | What it changes |
|---|---|---|
| D-a | ~~**The registry keeper is a separate keeper service, not the canister.**~~ **Superseded by D-i (2026-10-10).** It posts the Inscriptions (frog) floors (`post_floor` on a listing of kind `Inscriptions` is signed by `Registry.keeper`) and will prove drand rounds (`prove_round`, keeper-attested until Q4, then the BLS proof). | The canister's `frog_floor_reading` → `FStep::Post` path stays as built but is **dormant**: it signs only when the registry names the canister's key, and the launch file names the keeper service's key. The keeper service gets a Solana signer (§5 "Keeper"). The devnet rehearsal plays the keeper with an operator keypair (`docs/v9/rehearsal-sol-devnet.md`). |
| D-b | **Frog purchase bounds on the Solana home: 1 SOL minimum, 10 SOL default maximum, 250 SOL ceiling** of any collection limit (Relay's SOL → BTC capacity per order, spike Q5). | Confirms `frogsol::MIN_PURCHASE_LAMPORTS` / `MAX_PURCHASE_LAMPORTS` / `RELAY_SOL_BTC_MAX_LAMPORTS` as built in B2; they are SOL amounts, not ETH conversions. |
| D-c | **`SOL_PER_ETH` is not a constant.** The launch file carries every ETH-denominated value already converted, computed from the live SOL/ETH rate at generation time. | rook_core no longer derives anything from a rate: `revenue_target` is a `Params` field of `init_module`; the wake desk's bond **and its bounds** (`min_bond`, `max_bond`) are `init_desk` parameters, stored and immutable; `units.rs` holds only unit conversions. The launch file is written by `ops/solana/launch-params.mjs` (≥ 2 public sources, refused if they differ by more than 1 %), each value with its ETH origin and the rate used. |
| D-d | **No wasm trims now; plan a chunked install** (`install_chunked_code`) before the gzipped canister module reaches the 2 MiB single-message limit (1.66 MB after B2, 79 %). | A work item for the canister's release tooling (the timelock's proposal path included), due before the gzipped module passes ≈ 1.9 MB. |
| D-e | **The Meteora config values are confirmed**: Q1's curve (1B fixed supply, 6 decimals), an **85 SOL** migration threshold, `migration_fee` **50 %** to rook_backing's claimer PDA, partner curve-fee split **Rook 80 % / creator 0 %** (DBC keeps its 20 % protocol share), the Q1 anti-sniper schedule (50 % → 1 % over 10 min, linear), DAMM v2 graduation with the LP **permanently locked** (50/50, nothing claimable, no vesting). | Closes Q9's value. The launch file carries a `meteora` section, written by `ops/solana/launch-params.mjs`: every `create_config` field with its value and source. It also carries the `listing_order` that the configs' claimers derive from. `rook-ceremony` builds the configs from that section and refuses a launch file that drifts from D-e. |
| D-f | **The operator does not provide the ≈ 10 SOL of program rent.** Day 1 runs **without deploying the programs**: the mainnet program ids are reserved (keypairs generated, kept offline), and the 12 DBC configs are created with `fee_claimer` = the claimer PDAs derived from those ids. | The backing share and the partner fees accumulate in DBC until `rook_backing` is deployed at the reserved id. Nothing expires, and only a program at that id can claim (verified against DBC 0.2.1's source and simulated on mainnet: `launch-day1-solana.md` §2). **Day 1 costs 0.098384680 SOL** (simulated). The risk: whoever holds the two program keypairs controls every coin's backing, and losing them loses it. |
| D-g | **The deploy is community-funded through one plain wallet**, the operator's own, `GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy` (the fund wallet; its key is never generated or held by Rook's tooling). Contributions are plain SOL transfers. A **genesis ROOK coin** on its own DBC config has creator trading-fee share **100 %** (DBC's maximum: 80 % of every curve fee after the protocol's 20 %), creator and partner fee claimer = the fund wallet, migration fee 0 (no backing). `deploy-later.sh` deploys from it and makes it **the programs' upgrade authority and the registry owner**. | **This replaces the Squads v4 / 72 h time-lock plan of §2.5 for now.** The fund wallet is a **single key and a single point of failure**: it can upgrade both programs at once with no delay, and it owns the registry. Control can move later. The upgrade authority moves with `solana program set-upgrade-authority` (the checked variant has the new key sign). The registry owner moves with rook_core's two-step `propose_owner` / `accept_owner` (it has no delay in the code; the "7-day" delays apply to roles, roots and converters, not to the owner). Public explainer: no promised returns; contributions fund a refundable deploy deposit (`launch-day1-solana.md` §5). |

Also recorded the same day, for the rehearsal: rook_core's registry takes a **Bitcoin network** at `init_registry` (`Mainnet` |
`Testnet4` | `Signet`, immutable; the launch file must name it, `Mainnet` for production), so an Inscriptions listing's custody
address and its redemptions' taproot destinations are checked against that network's prefix (`bc1p…` or `tb1p…`). The canister
refuses a frog listing whose registry names a network other than its own `btc_network`.

**D-i [operator 2026-10-10] The canister is the registry keeper (supersedes D-a).** No separate keeper service: the canister's
Solana key is `Registry.keeper`, and the canister cranks every permissionless instruction itself. Only claiming ROOK's creator and
partner fees into the fund wallet stays manual (the operator's wallet signs). As built (`autobuyer/canister-rs/src/solcrank.rs`,
`drand.rs`; tests `tests/cranks_sol.rs`):

| Crank | When | Rule |
|---|---|---|
| rook_backing `open_vault` | every pool of a registered config with no vault (listed per config through a sliced `getProgramAccounts` on DBC, every 6 h) | migrated pools first; others at most 12 a day (each costs the canister ≈ 0.0047 SOL of rent it never gets back) |
| rook_backing `back(0)` | every migrated pool whose vault is not backed (checked hourly) | `min_units = 0`: the program applies its own bound (`units_floor`, A3-06). A refusal (no floor, stale, over the cap) is not a line error: it waits for the module's next floor post, retried at most every 6 h |
| `sweep_partner_fees` / `sweep_lp_fees` | claimable ≥ **0.05 SOL** (DBC `partner_quote_fee`; DAMM v2 position: pending + liquidity × Δ accumulator >> 128) | a crank costs ≈ 0.000255 SOL all in (5,000 lamports + ≈ 26 B cycles of threshold signature), a coin's first sweep 0.00204 SOL more (the claimer's token account of the coin): ≤ 0.5 % (≤ 4.6 % the first time) |
| treasury `apply_shares` / `apply_ops`, then `allocate` | a pending `set_shares` / `set_ops` past its 24 h wait (72abb09); unallocated income ≥ 0.05 SOL | the change is applied before income is split |
| `begin_distribution` + `distribute(i)` | reserves pot ≥ 0.25 SOL, a day since the last round, some 7-day volume; no round open (`RoundOpen`) unless `ROUND_TIMEOUT` passed | an open round's unpaid indexes are distributed (6 per transaction), never restarted early |
| `resolve` (+ `prove_round`) | in ticket order, checked every 15 min | a pinned round only once `round ≤ round_at(now)`; its signature from ≥ 2 drand relays (api.drand.sh, api2/api3.drand.sh, drand.cloudflare.com; `Home::Solana.drand_urls`), identical and none different, **and verified by the canister itself** (BN254 pairing, RFC 9380 hash-to-curve with Keccak + SvdW, ported from `DrandEvmnet`/`BN254HashToCurve`; all 20 real beacons of `test/fixtures/drand-evmnet-20.json` verify); randomness = `sha256(signature)`; `prove_round` and `resolve` in one transaction |
| frog floor `post_floor` | every fresh reading the runner hands in (`frog_floor_reading`, the Ordinals Wallet book) | the dormant D-a path is now the normal one |

Fairness: a crank never queues for the line; it signs at the end of a tick, on a free line nobody waits for, one transaction a tick and
48 a day at most, and pays rent only above a 0.05 SOL reserve. State in `sol_line().crank` (schedule, waits, last action, drand
disagreement); the monitor pages a crank waiting ≥ 6 h (SEV3; 24 h SEV2), the registry naming another keeper, a drand disagreement,
and a keeper pass silent for 2 h (`monitor/src/ethnft.mjs` `crankAlerts`).

Cost: ≈ 0.12 T cycles a day of reads at rest (12 collections; ≈ 0.17 T with the 17 of docs/v9/ordinals-collections.md, the reads scale per listing), plus ≈ 27.5 B cycles per crank transaction (threshold Ed25519 26.15 B,
simulation, statuses): 10 cranks a day ≈ 0.4 T/day ≈ $0.53/day; the 48-a-day ceiling ≈ 1.45 T/day. SOL: 5,000 lamports a crank, plus
rent: 0.0047 SOL per vault, 0.0012 SOL per proven drand round (≈ one per redemption), 0.002 SOL per coin's first sweep. The BLS check is
≈ 1.6 ms natively per round (negligible cycles). Wasm: 1,787,593 bytes gzipped (85 % of the 2 MiB install limit; +96 kB with the BLS
check's +31 kB); the chunked install of D-d is still due before ≈ 1.9 MB.

What the launch changes (rook_core is unchanged): `propose_roles(keeper)` applies at once for the FIRST keeper (owner-signed, no 7-day
wait), so no `initial_keeper` argument is needed. The canister's Solana address is derived from its threshold key, so the order is:
create and install the canister (with `home`) → read `sol_address()` → deploy the programs and `init_registry` (`deploy-later.sh`) →
`rook-ceremony list` (the fund wallet signs) with `custody.json.custodian` = that address, which also signs `propose_roles(custodian)` as
its first step and checks `Registry.keeper` back. No keeper service key is ever named (naming one first would make the move to the
canister a 7-day change). Fund the canister's Solana address: ≈ 0.5 SOL covers a month at a few launches and redemptions a day (the
budgets' ceiling, 12 plain vaults and 48 cranks a day, is ≈ 0.06 SOL a day).
Q4 stays open ON CHAIN (rook_core's `prove_round` still trusts its keeper); with the canister's own BLS check, what must be trusted is the
canister's code (behind its 72 h timelock), not its operator or any relay.
Feasibility of the canister fetching the frog floor itself: the Ordinals Wallet book is an HTTP API like the ones `ordapi.rs` already
reads two at a time (replicated outcalls, a transform keeping the sorted listings); possible, but kept runner-supplied for now: the
canister signs whenever it holds a fresh reading (≤ 1 h old), and a stale module (3 h without a post) is what the monitor pages.

**D-h [operator 2026-10-10] RPC at start: free tiers.** Helius free plan (dedicated key with its credit cap; IPv6) plus a second free provider reachable over IPv6 (PublicNode, or Ankr free), since the canister requires two independent providers to agree. Upgrade to Helius Developer ($49/month) and a paid second provider when collections are active or the free limits refuse (≈0.8M requests/month per provider at rest is at the edge of Helius free). Needed only from the deploy on; day 1 does not use the canister.

## 3. The Solana programs

One Anchor 1.2.x workspace, `solana/`. **As built**: two programs, not one per role — `rook_core` (registry, floor, wrappers,
treasury, wake desk, drand: the tightly coupled parts, so no CPI between them) and `rook_backing` (the DBC integration and the per-coin
vaults, which only calls `rook_core`'s public `convert`). The table lists the roles; each is a module of one of the two:

| Program | Replaces **[R]** | Accounts (PDAs) | Instructions (signer) |
|---|---|---|---|
| `rook_registry` | `ListingRegistry`, `CollectionDirectory`, `PosterSource` | `Registry` (owner, keeper, guardian, keeper_paused, roles with `proposed_at`), `Listing` per collection (kind, paused, membership root, custody address, poster, poster_paused) | `list_*` (owner), `set_paused` (owner), `pause_keeper` (owner/guardian), `unpause_keeper` (owner), `propose_roles` / `accept_roles` (owner, 7 d), `propose_root` / `accept_root` (7 d), `pause_poster` (owner/guardian), `add_to_directory` (owner), `is_member` (view, keccak sorted-pair Merkle) |
| `rook_floor` | `FloorModule` | `Module` per collection (every immutable parameter, the books: backing, reserve, surplus, revenue, grad_lamports, owed; units: eth_backed, transit, reserve_units; floor: price, confidence, depth, posted_at, day anchor; buckets), `Vault` (lamports) | `post_floor` (poster), `mint` / `burn` (anyone, SOL in/`wX` out and back; FLOOR_TTL 3 h, the spread rules, hourly buckets, `publicCap`), `enqueue` / `release` / `cancel` (curve program CPI), `advance` / `settle_advance` / `return_advance` / `item_arrived` (custodian = the canister's Ed25519 key), `fund_reserve` / `fund_liquidity` / `skim` (anyone), `fill` |
| ~~`rook_curve`~~ (not built: DBC, §2.1) | `FrogCurve`, `CurveFactory`, `CurveToken` | `Factory` per collection (template, targets, raise rules, coins count), `Curve` per coin (state, the closed-form A/B/v0/t0, spent, fee ledgers), the coin's SPL mint (authority: the curve PDA; no Token-2022 extensions **[W]**) | `launch` (anyone; creates mint + curve), `buy` / `sell` (anyone; min-out, deadline, clip-and-close at the raise), `graduate` (anyone, once the module `is_filled`: CPI `release`, seed the pools), `open_refunds` / `refund`, `claim_creator_fees`, `claim_platform_fees`, `flush_protocol_fees` |
| ~~`rook_lock`~~ (not built: DBC graduates into locked DAMM v2) | `LiquidityLock`, `EtchRouter` |
| `rook_backing` | (new) | `Partner` (the DBC config keys Rook created, per collection), `Vault` per coin (the coin mint, the DBC pool, `wx_units`, `backed_at`, `launched_at` = the DBC pool's creation time, the claimer PDA) | `register_config` (rook_core's registry owner, read from the `Registry`), `back(coin)` (anyone, after graduation: CPI DBC `withdraw_migration_fee` as the claimer PDA, then `rook_core::convert`), `redeem_backing(coin, amount)` (any holder: burns coins, pays pro-rata `wX`), `sweep_partner_fees` (anyone: DBC/DAMM v2 partner fees → treasury) | `Lock` per collection (shared pool, positions), per-coin position records | `seed` (curve CPI: create COIN/`wX` DAMM v2 pool full-range, locked; add to the shared `wX`/SOL pool), `claim_fees` (anyone; fees to the curve / vault / treasury as today), `add_protocol_liquidity` (anyone) |
| `rook_wrap` | `EthNftWrapper`, `Wrapper`, `WrapClaim` | `Wrapper` per collection (chain, collection, custodian, listing, mint limit with `proposed_at`, held, eth_backed, open tickets), the `wX` SPL mint (authority: the wrapper PDA), `Item` per held NFT, `Ticket` per redemption (state, drand round, resolved item, deliver_by, sent tx) | `init_mint_limit` / `propose_mint_limit` / `accept_mint_limit` (owner), `mint` / `mint_deposit` (custodian; once per item, once per (item, tx), H11 per-day limit), `module_mint` / `module_burn` (floor CPI), `redeem` / `redeem_to_burn` (anyone; burns 1 unit, opens a ticket pinned at drand round +20), `resolve` (anyone, in order, drand signature → `rook_drand`), `note_sent` / `record_delivery` / `mark_undeliverable` (custodian), `refund` (holder, after the windows) |
| `rook_treasury` | `ProtocolTreasury` | `Treasury` (shares, ops, collections) | `set_shares` / `set_ops` / `add_collection` (owner), `allocate` / `distribute` (anyone), `repay` (floor CPI) |
| `rook_wake` | `WakeDesk` | `Desk` (bond, window), `Request` per wake | `request` (anyone, pays the bond), `mark_served` (anyone; the canister after its post), `reclaim`, `forfeit`, `withdraw` |
| `rook_drand` | `DrandEvmnet`, `BN254HashToCurve` | `Round` per proven round | `prove(round, sig)` (anyone; alt_bn128 pairing) |

Rules carried over verbatim (they are the audited behaviour **[R]**): the floor post clamps (±maxMove/h, ±20 %/day), `FLOOR_TTL` 3 h,
`STALE_ESCAPE` 7 d, burn-while-paused at `pauseFloor`, the reserve tier (`reserveBps` of every intake, never converted), one open
advance at a time with `maxItemCost = min(itemCap, backing/unit + revenue + surplus)`, `netUnits = ethBacked − transit − reserveUnits`
as the buy signal, the curve's clip-and-close, `openRefunds` after 3 d (+4 d if filled but stuck), the ticket order rule, the
7-day delivery and the 72 h / 30 d refund windows, the per-day mint limit, "custody caps live in the canister, not on-chain".

Amounts: `u64` lamports and SPL amounts (9 decimals for `wX`, 1 unit = 10⁹; coins 10⁹ supply × 10⁶ decimals), the curve's `u256
mulDiv` becomes checked `u128`; every EVM `1e18` constant is rescaled in one table (`solana/programs/common/src/units.rs`) with a
test that the closed form reproduces the Solidity fixtures (`test/FrogCurve.t.sol` values, exported once).

## 4. The canister on Solana

New modules (`autobuyer/canister-rs/src/`):
- `solrpc.rs` — the outcall layer: `getAccountInfo` / `getMultipleAccounts` (base64, `finalized`), `getSignatureStatuses`,
  `getTransaction`, `getSlot`, `getBlock`, `sendTransaction`, `getBalance`, with transforms that strip `context.slot` and
  normalise; ≥ 2 providers agreeing; the same `rpc_agreed` shape as today's `canister.rs:809-885` **[R]**.
- `soltx.rs` — message building (legacy and v0), Anchor instruction encoding (discriminators + Borsh), lookup tables, durable-nonce
  prefix, the 1,232-byte check; `solsend.rs` — the saved-bytes send line (§2.4) and `sol_seen`.
- `solread.rs` — typed decoders for the program accounts the canister reads (`Module`, `Wrapper`, `Ticket`, `Request`, `Curve`).
- `solkey.rs` — `sol_address()` from `schnorr_public_key(ed25519)`, `sign_sol(message)`.

Changed modules (estimates from `ref-robinhood-offchain-deps.md` §A): `canister.rs` (Init: `home = Solana { cluster, rpc_urls,
programs }`, chain guards, key), `ethflow.rs` home steps (Advance, Deposit, Mint, Settle, GiveBack, Record, Mark, Sent, Post,
MintDeposit, Fill, Served → `soltx` instructions), `convflow.rs` / `relayconv.rs` (Solana origin, lamports, SOL/BTC rate),
`relayeth.rs` (SOL↔ETH legs), `frogflow.rs` + adapters (`curvebuy`, `floorsink`, `floortrigger`, `floornet`; `buyqueue`, `mintdesk`,
`handin*`, `sale*` go dormant with their features), `redeem.rs` / `custodynet.rs` / `delivery.rs` (tickets from accounts, unix
clocks), `deposits.rs` (OP_RETURN claim `"EWD2" ‖ chain(4) ‖ wrapper(32) ‖ recipient(32)` = 72 bytes, fits 80), `custodycap.rs`
(keys `sol:<wrapper pubkey>`), `rates.rs` / `oracle.rs` (SOL/BTC, SOL/ETH), `chains.rs` (`validate(chains, home)` for a non-EVM
home), `dormancy.rs` reads. Deleted: the v2 vault buyer (`looper`, `state`, `policy`, `listing`, `bridge*`, `relayquote`, `venue`,
≈1,500 lines, AutoBuyerVault-only **[R]**).

Unchanged: everything in `ref-robinhood-offchain-deps.md` §A.8 (Bitcoin core, both judges, ord providers, Seaport, Ethereum pin,
pure rules, the timelock). Tests: `tests/fakerpc` gains a fake Solana provider pair (finalized accounts, signature statuses,
nonces) beside the Ethereum fakes; `frogs.rs`, `ethnft.rs`, `redeem.rs`, `dormant.rs`, the `audit_nft_r*` suites are re-pointed at
it. The regression tests of every review finding (C-, P-, B-, A-, D-series) are kept by name: a port that loses one fails CI.

## 5. Services, web, tests

- **Keeper**: ~~`@solana/kit` signer, `post_floor` in lamports (sources unchanged: Ordinals Wallet book, OpenSea), no cap posts or
  draws at launch; the resolver calls `resolve` with the drand signature.~~ **D-i (2026-10-10): the canister is the keeper** (§2.9); the
  runner hands it the frog floor readings.
- **Runner**: Relay quotes with a Solana origin (instructions + lookup tables judged like `relay.ts` judges calldata today); the
  Bitcoin steps (`owprotected`, `unisatbuy`) unchanged; the pending cadence tied to Robinhood's state window goes.
- **Fueler**: a Solana hot key; `claim_platform_fees` / `allocate`; SOL → ETH through Relay to the ckETH helper, then ICPSwap → ICP →
  cycles as today (`fuel/*` is chain-agnostic past the first leg).
- **Reconciler**: reads `Module` / `Wrapper` accounts; the backing page's arithmetic is unchanged.
- **Indexer**: Helius webhooks or Yellowstone gRPC (Helius Business, $499/mo, or Triton **[W]**) for program events + account
  snapshots; USD from Pyth SOL/USD; API, db, candles, views unchanged.
- **Monitor**: a Solana home canary (wrapper and module instructions signed by the custodian matched to canister records, as
  `evm.mjs` does **[R]**), guardian pause through the Solana key, liveness; the Bitcoin canary, IC watcher, alerts unchanged.
- **Web**: `@solana/kit` + Wallet Standard (Phantom, Solflare, Backpack) **[W]**; `TxButton` → one send path with simulation, CU
  limit and priority fee; `Launch.tsx` (wake request + launch), `Ticket.tsx` (buy/sell/graduate/refund), quotes from `Module` and
  the DAMM v2 pool, `Redeem.tsx`; the trust page rewritten for the Solana governance.
- **Go-live tooling**: `golive-check` becomes a Node script over RPC (program ids, upgrade authorities = the Squads vault with the
  time lock, registry roles, module parameters vs the launch file, caps and the A4-L-1 exit rule, canister install argument,
  monitor config); the dry run runs on a local validator (`solana-test-validator` with the programs and a DAMM v2 clone) instead of
  an anvil fork.
- **Launch file**: `deployments/launch/solana-pilot.json` — the same collections, caps and stages, amounts in lamports, program ids,
  the Squads vault, the guardian.

## 6. What changes in the security model (honest list)

- **Upgradeable programs** behind a 72 h public delay, where the EVM contracts were immutable (§2.5). A compromised Squads quorum
  can change the code after 72 h; the canister's timelock guards the canister only.
- **Reads without a pin**: provider disagreement is refused, not resolved; a provider that lies alone is caught by the second; two
  colluding providers could feed a stale `Module` — same class as today, with no block-hash anchor. Mitigation: three providers,
  the SOL RPC canister as a second route, the monitor's independent reads.
- **Durable nonces** are a Solana feature on its way out (§2.4): a migration is planned for, not a surprise.
- **Transaction expiry** removes the "stuck for 48 h in the mempool" class and the nonce-bisection machinery (`home_nonce_tx`) **[R]**.
- **Logs are not load-bearing** (§2.4): fewer ways for an RPC to hide an event from the canister.
- **FX exposure** on every advance (§2.3), bounded by the margin and the TTL; new, and quantified in the reserves page.
- **Audit**: every review round so far is void for the programs; the canister's Bitcoin and Ethereum findings stay valid. The
  programs need their own rounds (OtterSec, Neodyme, Zellic, Sec3 **[W]**) and the same house review rounds this repo runs.

## 7. The plan

Work is parallel where the interfaces are fixed first; the calendar is dominated by review rounds and rehearsals, not typing.

**P0 — spikes (first)**: Q1 a Rook DBC config on a local validator with mainnet DBC + DAMM v2 cloned: launch, buy to graduation,
`withdraw_migration_fee` signed by a `rook_backing` PDA, partner-fee claims; plus whether terminals list DBC coins pre-graduation; Q3 a canister on IC mainnet signing a durable-nonce Solana transfer and resending the same
bytes; Q4 drand pairing CU; Q5 Relay SOL→BTC / SOL→ETH / refund with the API key, and the depository payout read. Each spike ends
in a page under `docs/v9/spikes/` with measurements, the way `ow-protected.md` was written before its code.

**P1 — programs** (`solana/`): `common` (units, clocks, Merkle, errors), `rook_registry`, `rook_floor`, `rook_curve` (closed form
first, against the Solidity fixtures), `rook_wrap` (tickets), `rook_lock` (after Q2), `rook_treasury`, `rook_wake`, `rook_drand`
(after Q4). Tests: LiteSVM unit tests per instruction, property tests for the books invariants (balance ≥ Σ of the books; `wX`
supply = held − open tickets + eth_backed), the ported review PoCs as regression tests. Deliverable: a local validator deploy script
and `golive-check` v2 reading it.

**P2 — canister** (in parallel after P1's account layouts are fixed): `solkey`, `solrpc`, `soltx`, `solsend`, `solread`; then
`ethflow` home steps, `convflow`/`relayconv` origin, `frogflow` adapters, `redeem`, `deposits`, `rates`, `chains`, Init; fake Solana
providers; every named regression test green. Deliverable: the Ethereum NFT flow end to end on a local validator + Ethereum
fork (the `nft-fork-rehearsal` harness **[R]**), and the Bitcoin half of the simulated frog purchase (`ops/rehearsal/sim/`) pointed
at the Solana floor module.

**P3 — services and web** (parallel with P2): keeper, runner legs, fueler, reconciler, indexer, monitor, web, launch file,
`golive-check` v2, the local-validator dry run.

**P4 — review and rehearsal**: the house review rounds (programs, then the canister's new modules), two outside audits of the
programs, verifiable builds, a devnet end-to-end with real wallets, the IC mainnet rehearsal of the Ed25519 line (like the
IC-outcall rehearsal, ≈2 T cycles), Relay SOL legs live for small amounts, then `launch-gates.md` rewritten for Solana and the
ceremony (Squads vault, time lock, program deploys, registry roles, the canister reinstall naming the Solana home).

**The operator's items** (nothing else needs a second person, per the 2026-10-09 decision): a Rook Helius account plus Ankr
pay-as-you-go (§6 of `launch-day1-solana.md`; Triton is IPv4-only), a Relay API key, the guardian key, ≈ 0.11 SOL in the fund
wallet for day 1 (D-f). The program deploy (9.54 SOL at peak, 6.92 SOL locked as a refundable deposit, opt-level 3) is
community-funded (D-g) and no longer an operator item. A Squads v4 multisig comes later (Q17). Also: cycles for the Ed25519 rehearsal, the audit engagements, and the three decisions
flagged **[D]** above that are product calls: own curve (2.1), DAMM v2 (2.2), sweeps/raffles off at launch (2.7).

## 8. Open questions

| # | Question | Decided by |
|---|---|---|
| Q1 | Does DBC accept `wX` as a quote mint, and does Jupiter route SOL→`wX`→COIN during the curve? | spike, P0 |
| Q2 | DAMM v2 locked-add and fee-claim CPIs from a PDA: CU, accounts, lookup tables | spike, P0 |
| Q3 | Threshold Ed25519 + durable nonce from a canister: latency, cycles, the resend path | spike on IC mainnet, P0 |
| Q4 | drand BN254 pairing on Solana within the CU budget | spike, P0 |
| Q5 | Relay SOL→BTC quotes to a taproot float; the depository payout read | spike with the API key, P0 |
| Q6 | The SOL RPC canister's id and consensus modes, as a second read route | P2 |
| Q7 | Holder deposits' return (EWD2 claim; Ethereum deposits were ON at the Robinhood launch) | first post-launch release |
| Q8 | Program immutability: when the upgrade authorities are burned (`init_registry` / `init_backing` require the upgrade authority, so both run before burning) | after a quiet quarter |
| ~~Q9~~ | ~~The backing share of each raise (DBC `migration_fee`), against pool depth~~ | **decided 2026-10-09 (D-e)**: 50 % of an 85 SOL threshold, Q1's curve |
| ~~Q6~~ | ~~The SOL RPC canister as a second read route~~ | **evaluated 2026-10-09** (`launch-day1-solana.md` §6): `tghme-zyaaa-aaaar-qarca-cai` (NNS root controller) has no typed `getMultipleAccounts` / `getProgramAccounts` / `simulateTransaction`, costs ≈ 1.7 B cycles a read, and holds its own provider keys. It is a monitor cross-check at most. Primary: Helius + Ankr called directly (both IPv6) |
| Q16 | A DBC coin that never graduates leaves its rook_backing vault unbacked forever: its collection then stays "active" for the canister's dormancy rule, and a listing past 64 vaults makes each 6-hourly vault read cost the full ≈ 19 B cycles again | product rule before launch (count only vaults whose pool reached the threshold, or age them out) |
| Q17 | The single-key fund wallet as upgrade authority and registry owner (D-g) | move to a multisig (Squads) once the deploy is done; the date is the operator's |
| Q15 | Floor-post front-running: a public OpenSea floor rise lets a bot mint before the post and burn after (≤ max_move 5 %/h less 2×15 bps, ≤ 5 units/h each way, ≤ 20 %/day). **Accepted at launch [operator 2026-10-09]**: NFT floors rise slowly and volume is low. Revisit (burn at min(floor now, floor at mint) or a mint→burn lock across a post) if floors start moving fast; watch mint-then-burn pairs across posts in the monitor. | accepted |
| ~~Q10~~ | ~~Who posts the frog (Inscriptions) floor and proves drand rounds: the canister or a keeper service?~~ | **decided 2026-10-09 (D-a)**: a separate keeper service |
| ~~Q11~~ | ~~The frog purchase bounds on the Solana home~~ | **decided 2026-10-09 (D-b)**: 1 SOL min, 10 SOL default max, 250 SOL ceiling |
| ~~Q12~~ | ~~`SOL_PER_ETH = 20` (rook_core `units.rs`): confirm in the launch file~~ | **decided 2026-10-09 (D-c)**: no constant; the launch file carries converted values from the live rate (`ops/solana/launch-params.mjs`) |
| ~~Q13~~ | ~~The canister wasm near the 2 MiB single-message install (1.66 MB gzipped)~~ | **decided 2026-10-09 (D-d)**: no trims now; chunked install before 2 MiB |
| Q14 | Devnet rehearsal: Relay has no devnet (the SOL → BTC and SOL → ETH legs are stubbed there). ~~rook_backing's converter waits 7 days, so `back` on devnet runs a week after the init~~ | **converter half decided 2026-10-09**: rook_backing signs `convert` with ONE program-wide `["converter"]` PDA (the per-collection claimer stays the DBC fee claimer and pays), and `init_registry(owner, network, initial_converters)` allows it at once (upgrade-authority-gated, before anything is live); later additions keep the 7-day propose → accept, removal stays instant (`rook-core-as-built.md` §8.8). `back` → `convert` runs in the same rehearsal run, and at launch day. Relay half: `docs/v9/rehearsal-sol-devnet.md` |

None of Q1–Q9 closed with the 2026-10-09 review fixes; the programs' own open questions that did close (first-come
`init_registry`, instant converters, the wake launch time, rook_backing's admin binding) are recorded in
`rook-core-as-built.md` §7–8 and `rook-backing-as-built.md` §6–7.

## 9. Fallback

Everything on Robinhood Chain is intact at tag `robinhood-stop-2026-10-09`; `robinhood-stop-2026-10-09.md` §5 says how to resume it.
The Ethereum and Bitcoin sides are shared, so work on them (Identity MD, the simulated purchase, future review fixes) serves both.
