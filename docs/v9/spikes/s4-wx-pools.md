# Spike S4: a wX/SOL pool the canister can re-centre (canister-only Rook on Solana)

Status: **run 2026-10-10** on branch `golive-2026-10-08` (uncommitted). Question: open item S4 in
`docs/v9/canister-only-solana.md` §5.4. In DAMM v2 the price range belongs to the **pool**, so which pool type lets the canister
keep the desk's liquidity concentrated around the floor and move it when the floor moves? **No mainnet transaction was sent.**
Mainnet was only read: `solana program dump` of Meteora DLMM, `getMultipleAccounts` for its preset accounts, Meteora's docs and
APIs, and Jupiter's quote API. Legend as in the design: **[M]** measured here, **[W]** from documentation or a live API,
**[A]** assumption.

Code (new files only):

- `solana/tests/s4_wx_pools.rs`: 5 tests, all passing.
  - DLMM instructions are hand-encoded from the IDL (Anchor discriminator + Borsh), the way the canister's `solix2` builders
    would be.
  - `programs/rook_backing/tests/s4_wx_pools.rs` is the 4-line shim, as for S1.
  - Run: `cd solana && cargo test -p rook_backing --test s4_wx_pools -- --nocapture --test-threads 1`. The harness's
    `Env::new()` needs `target/deploy/rook_backing.so`, as S1 does.
- `solana/tests/fixtures/dlmm.so`: DLMM `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`, mainnet bytes dumped 2026-10-10. The
  binary carries every instruction of IDL `lb_clmm` 0.12.0 (MeteoraAg/dlmm-sdk `main`), including `RebalanceLiquidity`.
- `solana/tests/fixtures/s4_dlmm_presets.json`: six mainnet `PresetParameter2` accounts (indexes 262, 256, 286, 72, 83, 68).

Stand-ins, as in S1:

| Stand-in | Plays |
|---|---|
| the env payer | `K_main`: fee payer and nonce authority |
| `desk` | `K_desk[i]` |
| `canister` | `K_coll[i]`, the mint authority of wX |

wX is a classic SPL mint with 9 decimals and no freeze authority. Milady's floor is 18.61 SOL per wX. Sizes are legacy
transactions behind a durable-nonce advance plus CU limit and price (the canister's send line), against the 1,232-byte limit.

## 1. Verdict

| Option | Re-centres? | Result |
|---|---|---|
| **(A) Meteora DLMM** | **yes, in place.** A position owns its bins. `rebalance_liquidity` withdraws and re-deposits **the same position** around a new active bin in **one instruction**. The position is a PDA, so **no throwaway key signs** | **PASS: recommended.** Every step landed on mainnet bytes. Each transaction fits with a nonce (the tightest is exactly 1,232 B; §2.4). At 10 SOL a side in ±10 %, 0.1 / 0.5 / 1 SOL buys pay +0.81 / +0.80 / +1.03 % all-in, and both Jupiter tests pass |
| (B) DAMM v2 on Meteora-issued dynamic configs | only by **opening a new pool** each time | **works only with Meteora's help, and runs out.** Each re-centre uses up one config. Not testable: no such config exists |
| (C) DAMM v2 `[p/4, 4p]`, never re-centred | no | **fallback only.** At 10 SOL a side, 0.1 / 0.5 / 1 SOL buys pay **+1.00 / +3.00 / +5.50 %**. Both Jupiter tests pass (test 2 only just: +18.3 % against a 20 % limit) |

## 2. (A) Meteora DLMM [M unless marked]

### 2.1 Who can create a pair, and how many there can be (test `s4a`)

Each `PresetParameter2` is created by Meteora. A pair is created on a preset by **anyone**, with `initialize_lb_pair2`. The pair's
address is `[preset, min(mint), max(mint)]`, so there is **one pair per (preset, mint pair)**. Mainnet has ≈ 300 presets; indexes
0–299 were scanned.

| # | Check | Result |
|---|---|---|
| 1 | A stranger **holding no wX** creates wX/SOL on preset 262 (bin step 50, base fee 0.4 %, `collect_fee_mode` 1 = fees in Y only) | **OK.** 47–56 k CU, 835 B with a nonce. Rent 0.034647 SOL (pair + 2 reserves + oracle), paid by `funder`, which is only a payer and can be any key. `pair_type` 3 (PermissionlessV2), active bin 586 |
| 2 | A second wX/SOL pair on the same preset, in either X/Y order | refused (the account already exists) |
| 3 | wX/SOL on presets 256, 286, 72, 83, 68 | **OK each time.** The same two mints can have **many pairs**, one per preset |
| 4 | `initialize_customizable_permissionless_lb_pair2`: the creator chooses bin step, base fee (0.5 % exactly) and fee mode | refused without wX (`MissingTokenAmountAsTokenLaunchProof`, 6060). **OK when the creator holds 1 base unit of wX.** Only one such pair can exist per mint pair (a second is refused) |
| 5 | Token X with a **freeze authority** | accepted (DLMM does not check; Rook's own rule stays, as with DBC) |
| 6 | **Squatting**: a stranger opens an **empty** pair at 3× the floor | anyone can call `go_to_a_bin(floor)`: **OK**, no bin arrays needed, and the price moves back. If someone else's liquidity lies in between, the call is refused (`BinRangeIsNotEmpty`, seen in §2.4). The desk then swaps through it, which sells the squatter's liquidity at the wrong price |

**The squatting problem of §5.1 goes away:**

- the desk can use any preset that is still free for wX/SOL;
- an empty squatted pair can be moved back by anyone.

The "first mint inside the pool-creation transaction" ordering is no longer needed.

Presets with fees in SOL only (`collect_fee_mode` 1), close to the design's pool:

| Index | Address | Bin step | Base fee | Notes |
|---|---|---|---|---|
| **262** | `2E9Z9za4vEynZVHKpJH9GemaH8qdioAt9Fs6xYiJvA9S` | 50 | 0.4 % | filter / decay 120 / 1,200 s, variable-fee control 10,000. **The preset S4 used** |
| 286 | `J3u2ygYtE4B4xJSuYPtitpf8zQiB5gWomxWJKPTR9Krd` | 30 | 0.3 % | |
| 256 | `E1VmZqvtJA9DoewQR2BC5mnrG3tYq3aFZdGy3FfWVuXk` | 50 | 0.3 % | |

All of these have a protocol share of 10 %, so the LP keeps 90 % (DAMM v2: 80 %). No preset has 0.5 % with fees in SOL. A
customizable pair can have 0.5 % (#4).

### 2.2 Lifecycle of the desk's pool (test `s4b`, preset 262, ±10 % in price, spot shape, 10 SOL a side)

| Step | CU | Size with nonce | Signers | Rent / result |
|---|---|---|---|---|
| `initialize_lb_pair2` | 47–56 k | 835 B, 16 accounts | 2 | 0.034647 SOL, not refundable |
| `initialize_bin_array` (1 array covers 70 bins = ±17 % at bin step 50) | ≈ 261 k each | 411 B (466 B for 2) | 1 | **0.071437 SOL each, not refundable**: `close_bin_array` is admin-only (`InvalidAdmin`). Anyone can pay |
| `initialize_position_pda` (base = owner = `K_desk`) | 12–17 k | 576 B | 2 (`K_main` pays, `K_desk`) | 0.057406 SOL for ≤ 70 bins, **refunded on close**. **No position keypair**: the address is `["position", pair, base, lower, width]` |
| `add_liquidity_by_strategy2` (`SpotImBalanced`, bins 567…605) | 203 k | 899 B, 18 accounts | 2 | 10.0000 SOL + 0.537883 wX, deposited exactly |
| `swap2`, a 0.5 SOL buy (1 bin array) | 46 k | 852 B, 19 accounts | 2 | |
| `claim_fee2` after 11 swaps (≈ 21 SOL of volume) | 71 k | 842 B | 2 | **0 wX and 0.078411 SOL: fees arrive in SOL only.** A stranger is refused (`ConstraintRaw`) |
| **arb up**: `MintToChecked` (K_coll) + `swap2` sell, one transaction | | **997 B**, 21 accounts | 3 | |
| close: `remove_liquidity_by_range2` 100 % + `claim_fee2` + `close_position2` | 251 k | 899 B | 2 | 0.057406 SOL back to `K_main`. What stays in the reserve is the protocol's share and rounding |

A swap that runs past the last bin with liquidity is refused (`BitmapExtensionAccountIsNotProvided`). It does not fill
partially, and a DAMM v2 range edge behaves the same way.

### 2.3 Depth (test `s4c`)

Each buy is sold back before the next. Before each buy, the clock moves 1 h so that the variable fee has decayed. "avg" is the
buyer's average price against the floor of 18.61, all fees included. "move" is the active bin after the buy.

| Pool | Base fee | SOL a side | 0.1 SOL avg / move | 0.5 SOL | 1 SOL | $500 round trip (< 30 %) | $1k vs $500 unit price (< 20 %) |
|---|---|---|---|---|---|---|---|
| **preset 262, bin step 50, ±10 %, spot** | 0.4 % | 10 | +0.81 % / +0.50 % | **+0.80 %** / 0 | **+1.03 %** / +0.50 % | 0.9 % PASS | +2.2 % PASS |
| … curve | 0.4 % | 10 | +0.81 % / +0.50 % | +0.80 % / 0 | +0.80 % / 0 | 0.8 % PASS | +1.5 % PASS |
| … bid-ask | 0.4 % | 10 | +1.03 % / +1.00 % | +1.76 % / +1.51 % | +2.36 % / +2.53 % | 1.1 % PASS | +1.9 % PASS |
| … ±21 % (≈ DAMM `[p/1.21, 1.21p]`), spot | 0.4 % | 10 | +0.81 % / +0.50 % | +1.01 % / +0.50 % | +1.44 % / +1.51 % | 1.2 % PASS | +4.3 % PASS |
| … ±10 %, spot | 0.4 % | 5 | +0.81 % / +0.50 % | +1.03 % / +0.50 % | +1.48 % / +1.51 % | 1.2 % PASS | $1k refused (FAIL) → passes on test 1 |
| … ±10 %, spot | 0.4 % | 1 | +1.03 % / +1.00 % | +2.88 % / +4.07 % | +5.39 % / +8.85 % | refused FAIL | refused FAIL |
| preset 286, bin step 30, ±10 %, spot | 0.3 % | 10 | +0.59 % / +0.30 % | +0.70 % / +0.30 % | +0.90 % / +0.90 % | 0.7 % PASS | +2.3 % PASS |
| customizable, bin step 50, ±10 %, spot | 0.5 % | 10 | +0.92 % / +0.50 % | +0.90 % / 0 | +1.13 % / +0.50 % | 1.4 % PASS | +2.8 % PASS |
| customizable, bin step 10, ±10 %, spot | 0.5 % | 10 | +0.59 % / +0.10 % | +0.77 % / +0.40 % | +1.00 % / +0.90 % | 1.2 % PASS | +2.0 % PASS |
| customizable, bin step 10, ±10 %, curve | 0.5 % | 10 | +0.59 % / +0.10 % | +0.66 % / +0.20 % | +0.78 % / +0.40 % | 1.1 % PASS | +1.5 % PASS |

**Reading.**

- **DLMM has a structural half-spread of about one bin step** on top of the fee. The active bin holds SOL, so a buy starts one
  bin up. At bin step 50 that is 0.5 %, which is why a 0.1 SOL buy pays +0.81 % with a 0.4 % fee. At bin step 30 it pays
  +0.59 %, and at bin step 10 +0.59 % with a 0.5 % fee.
- At ≈ 10 SOL a side in ±10 %, DLMM matches DAMM v2 `[p/1.1, 1.1p]` (§4: +0.55 / +0.74 / +0.97 %) to within one bin step.
- It beats DAMM `[p/4, 4p]` by 3–5× on 0.5–1 SOL buys.
- Small bins cost CU and rent. ±10 % at bin step 10 is 191 bins:
  - 4 bin arrays (0.286 SOL, not refundable);
  - a 0.152 SOL position;
  - **902 k CU for one add**, so a rebalance of that width would not fit 1.4 M CU [A].

  Bin step 30–50 keeps a ±10 % position under 70 bins: one position account, 1–2 bin arrays, ≈ 200–330 k CU per add.
- **The variable fee.** After the arb crosses many bins, DLMM adds a volatility fee:
  - a 0.5 SOL buy right after a 13-bin arb paid **+1.48 %**, and **+2.51 %** after a 28-bin move;
  - an hour later it paid +0.90 %;
  - the fee decays within `decay_period` (1,200 s at bin step 50).

  The fee goes to the LP, which is the desk. Buyers pay more for some minutes after a big floor move.

### 2.4 Re-centring (test `s4b`)

A re-centre takes two steps:

1. the arb loop moves the pool to the new floor;
2. the desk moves its **one** position around it.

The ask side needs wX that the arb has just bought out. So the same transaction mints it at the module: a SOL transfer from the
desk to the module (a stand-in key) and a `MintToChecked` signed by `K_coll`.

| Case | Transaction | CU | Size with nonce | Signers | Result |
|---|---|---|---|---|---|
| **R1**: floor +7 % (inside the band; the §5.4 trigger is 70 % of the way to an edge) | SOL→module + mint wX + **`rebalance_liquidity`** (remove bins 567…605 at 100 %, claim fees, add spot ±19 bins around active 600) | **284 k** | **1,165 B**, 21 accounts, 1 bin array | **3** (`K_main`, `K_desk`, `K_coll`) | position `[567, 605]` → `[581, 619]`, **the same account**. Afterwards 10.32 SOL + 9.32 SOL of wX in the pool; a 0.5 SOL buy pays +0.90 % once the variable fee has decayed |
| **R2**: floor +15 % (past the top of the band) | the arb buys the pool to its top bin 605 (a further buy is refused). Then SOL→module + mint + `remove_liquidity_by_range2` 100 % + **`go_to_a_bin(614)`** + `rebalance_liquidity` (add only) | 336 k | **1,232 B: exactly at the limit**, 22 accounts, 2 bin arrays | 3 | position → `[587, 633]`, the same account. One new bin array (0.0714 SOL) was needed for the new range. A 0.5 SOL buy pays +0.90 % after the decay |
| R3: R2 with plain instructions | (1) `remove_liquidity_by_range2` + `claim_fee2` + `close_position2`; (2) `go_to_a_bin` + SOL→module + mint + `initialize_position_pda` (new lower bin, so a new PDA) + `add_liquidity_by_strategy2` | 251 k + 229 k | 899 B + 1,174 B | 2, then 3 | works. Two transactions, and the position rent comes back and goes out again |

Notes:

- **R2 has no margin.** Move the mint leg (transfer + `MintToChecked`) into the transaction before it, leaving the wX in the
  desk's ATA, to recover ≈ 100–130 B [A]. Or re-centre at the R1 trigger, before the floor leaves the band, which is the design's
  rule anyway.
- `rebalance_liquidity` with `shrink_mode` = ShrinkBoth grows the position to the new range. `rent_payer` pays (`K_main` here).
  In R2 the position did not shrink fully on the left (`[587, 633]` against an asked `[595, 633]`); the empty bins only cost rent.
- `RemoveLiquidityParams { min_bin_id: None, max_bin_id: None }` means **the active bin**, not "all". Pass the range explicitly.
  The first attempt failed with `InvalidRebalanceParameters` before this was fixed.
- Spot amounts in `AddLiquidityParams`, per the SDK's `getAmountInBinsAskSide` and `…BidSide`:
  - `y0` = lamports per bid bin;
  - `x0` = Y-value per ask bin, so ask bin `i` gets `x0 · (1+s)^−i` of X;
  - `delta_x` = `delta_y` = 0;
  - `bit_flag` = 0.
- **Swap bin arrays must be passed in the swap's direction, starting at the active bin's array.** A sell given `[13, 14]` while
  the active bin sat in array 14 was refused (`AccountNotEnoughKeys`).

**Keys.** No re-centre needs a new key. The canister signs with `K_desk`, plus `K_coll` when it mints. The S1 item "every new
DAMM v2 position needs a fresh NFT mint that signs" (`K_nft[n]`, ≈ $0.035 a time) **does not apply to DLMM**.

### 2.5 Routing [W, M]

- **Jupiter routes thin, new DLMM pools** (`lite-api.jup.ag/swap/v1/quote`, `dexes=Meteora DLMM`, `onlyDirectRoutes=true`, 0.01
  SOL, 2026-10-10). Each of these was one hop on its own pool:
  - SWITCHED/SOL `DSZ96htZ…` ($297 TVL);
  - SIB/SOL `22PDYJXh…` (**$2 TVL**, price impact 90 %);
  - Pepper/SOL `7TEc47X8…` ($1.7k).

  All were created 2026-10-10 (Meteora data API). S2 also saw SOL → EMBER (DLMM) → DBC as a two-hop route. "Meteora DLMM" is on
  Jupiter's instant-routing list.
- The grace period and liquidity test are **per token**, whatever the venue (S2 §3.2). So the depth rows of §2.3 decide whether
  wX stays routed.
- **The site's own two-hop** (test `s4e`, sizes only; the DBC and COIN/wX accounts are stand-in keys). The DAMM v2 calibration
  reproduces S2's 1,033 B and 903 B exactly.

| Hop 1 | On the curve (DLMM → DBC `swap2`) | Graduated (DLMM → DAMM v2 COIN/wX) |
|---|---|---|
| DLMM `swap2`, 1 bin array | **1,104 B**, 26 accounts | 1,070 B, 25 accounts |
| DLMM `swap2`, 2 bin arrays | 1,137 B, 27 accounts | 1,103 B, 26 accounts |

Both fit with 95–160 B to spare and no lookup table. CU stays well under 200 k (DLMM `swap2` ≈ 46 k against DAMM v2's 26 k).

## 3. (B) DAMM v2 with a dynamic config [W]

Source: `docs.meteora.ag/developer-guides/damm-v2/pool-fee-configs.md`, read 2026-10-10.

- **What a dynamic config is.** A config account (`config_type = 1`) that stores only an `index`, a non-default
  `pool_creator_authority` and a `permission` bitmask.
  - The creator supplies `pool_fees`, `sqrt_min_price`, `sqrt_max_price`, activation and liquidity per pool, in
    `initialize_pool_with_dynamic_config`.
  - The authority must sign.
  - `pool_creator_authority` may be a normal key or a PDA, but not a program address.
- **Who creates it.** Only Meteora's operator: "please reach out to the team on discord". Even for static configs,
  "integrators cannot change the configuration". There is no self-serve path, no form and no published turnaround.
- **One pool per config per pair.** The pool PDA is `["pool", config, sorted mints]`. A re-centre therefore means:
  - remove all liquidity, claim, and close the position (S1 measured this);
  - open a **new pool on an unused config**, with new pool rent (≈ 0.03 SOL) and a new `K_nft` signature;
  - leave the old pool and its vaults behind, empty but alive, where anyone can still trade or deposit.
- **N configs give N re-centres per collection** (shared across collections if one desk key is the authority). Each re-centre
  also creates a new pool address, so routers and the site have to follow the desk's current pool. The design's builder
  `solix2::damm::initialize_pool_with_dynamic_config` still cannot land: no config exists.

So B gives re-centring, but it depends on Meteora granting a batch of configs, wears them out at a steady rate, and leaves a
trail of empty pools. Meteora would also see it as unusual: dynamic configs are meant for launchpads.

## 4. (C) DAMM v2 fixed ranges at 10 SOL a side (test `s4d`) [M]

Settings: fee 0.5 %, fees in SOL, the same method as S1's §2.4. "avg" is against the floor, fee included.

| Range | SOL a side | 0.1 SOL avg / move | 0.5 SOL | 1 SOL | $500 round trip | $1k vs $500 |
|---|---|---|---|---|---|---|
| **`[p/4, 4p]`** | **10** | **+1.00 % / +1.00 %** | **+3.00 % / +5.04 %** | **+5.50 % / +10.20 %** | 1.0 % PASS | **+18.3 % PASS (2 points of margin)** |
| `[p/4, 4p]` | 20 | +0.75 % / +0.50 % | +1.75 % / +2.50 % | +3.00 % / +5.04 % | 1.0 % PASS | +10.1 % PASS |
| `[p/1.21, 1.21p]` | 10 | +0.59 % / +0.18 % | +0.96 % / +0.91 % | +1.41 % / +1.82 % | 1.0 % PASS | +3.9 % PASS |
| `[p/1.1, 1.1p]` | 10 | +0.55 % / +0.09 % | +0.74 % / +0.46 % | +0.97 % / +0.93 % | 1.0 % PASS | +2.0 % PASS |

The design's estimate for C ("≈ 2.5–3 % on a 0.5 SOL buy" at 10 SOL) is confirmed: **+3.00 %**. A 1 SOL buy pays 5.5 %. Never
re-centring also means:

- a floor that moves 4× strands the pool;
- inside the range, the desk's inventory drifts toward whichever side the floor moved away from.

## 5. Recommendation: (A) DLMM, preset 262 (bin step 50, 0.4 %, fees in SOL), ±10 % spot, ≈ 10 SOL a side

| | DLMM ±10 % bin step 50 | DAMM v2 + dynamic configs | DAMM v2 `[p/4, 4p]` |
|---|---|---|---|
| Re-centre | in place, one instruction, unlimited | new pool each time, limited by the configs Meteora grants | none |
| Depends on Meteora | no (public presets, permissionless) | yes (configs over Discord) | no |
| Buy 0.1 / 0.5 / 1 SOL at 10 SOL a side | +0.81 / +0.80 / +1.03 % | ≈ the narrow rows of §4 | +1.00 / +3.00 / +5.50 % |
| Jupiter tests at 10 SOL | PASS / PASS (+2.2 %) | PASS / PASS | PASS / PASS (+18.3 %) |
| Keys per re-centre | `K_desk` (+ `K_coll` when minting) | + `K_nft[n]` | n/a |
| Sunk rent per collection | 0.0346 (pair) + 0.0714 per bin array (1–2, plus one each time the floor walks into a new 70-bin array, ≈ every ±41 %) | ≈ 0.03 per re-centre (a new pool each time) | ≈ 0.03 |
| Refundable rent | position 0.0574 | position | position |

**Why preset 262.**

- Fees arrive in SOL only, as the design requires.
- ±10 % is 39 bins: one non-extended position, usually one bin array, ≈ 280–340 k CU per re-centre.
- It is a public preset, so any unused preset (256 at 0.3 %, 286 at bin step 30 / 0.3 %) is a drop-in fallback if 262's wX/SOL
  pair gets squatted. The empty squat can also simply be moved back with `go_to_a_bin`.
- Preset 286 (bin step 30) is ≈ 0.2 points cheaper for buyers. It needs 65 bins and 2 bin arrays, and its rebalance was not
  measured [A: ≈ 400–450 k CU].
- A customizable pair would give exactly 0.5 %, but only one can exist per mint pair, and it needs wX in hand at creation (§2.1
  #4).

**Target depth.** Unchanged from the revision, about 10 SOL a side:

- ≈ 5 SOL a side already passes Jupiter's test 1 (1.2 % round trip) and gives ≈ 1 % on 0.5 SOL buys;
- 10 SOL also passes test 2 with margin.

### 5.1 What changes in W7 (pool desk) and in the design

1. **§5.1 pool type: DLMM**, not DAMM v2 customizable. Settings:
   - `initialize_lb_pair2` on a pinned preset (262);
   - X = wX, Y = wSOL, so price = SOL per wX and fees arrive in Y = SOL;
   - activation is immediate (permissionless pairs have no activation choice).

   Drop the anti-squat "pool with the first mint" ordering and the tiny anti-squat pool option of O-1 (§2.1).
2. **§5.4 re-centring.** Replace the "new pool per dynamic config" text with:
   - `rebalance_liquidity` on the desk's single PDA position, at the 70 %-to-edge trigger (R1, 1,165 B);
   - the `go_to_a_bin` path only when the floor has jumped past the band (R2, 1,232 B: move the mint leg into an earlier
     transaction).

   **Close S4** and drop the request to Meteora.
3. **The cadence watch.** Read `active_id` (4 bytes at offset 76 of `LbPair`) with `getMultipleAccounts` + `dataSlice`, instead
   of DAMM's `sqrt_price`. Price = (1 + bin_step/10⁴)^active_id. The band and arb maths become bin arithmetic: an up-arb sells
   wX until the active bin reaches `m/(1−f)`. The "closed form for a concentrated constant-product position" is replaced by a
   walk over the bins (known amounts per bin).
4. **Keys (§1.1).** No `K_nft` for the desk; remove it from the re-centre cost.
   - Position: `["position", pair, K_desk, lower_bin_id i32 LE, width i32 LE]`, base = owner = `K_desk`.
   - The rent payers (`funder`, `payer`, `rent_payer`) are plain payers, unlike DAMM v2's `payer`, so `K_main` *can* pay. Booking
     the rent to the desk (§5.2) stays a policy choice.
5. **Rent (§5.2, §12.2).** Per collection:
   - **0.0346 SOL** for the pair (sunk);
   - **0.0714 SOL per bin array** (sunk: `close_bin_array` is admin-only);
   - 0.0574 SOL for the position (refundable).

   About 0.16 SOL to open, against DAMM's ≈ 0.03 SOL, plus about 0.07 SOL each time the floor moves into a new bin array.
6. **The variable fee.** For ≈ 20 min after a large re-centre or arb, buyers pay up to +1.6 points more (§2.3). It accrues to the
   desk. The site's quote should read it from the pair (the variable parameters at offset 40).
7. **The site's route (W12).** Hop 1 becomes DLMM `swap2`:
   - 1,104 B on the curve, 1,070 B graduated, with 1 bin array;
   - pass the bin arrays from the active bin upward (buy direction);
   - for slippage control keep `min_amount_out`; `swap_with_price_impact2` exists if wanted.

### 5.2 Builders the canister needs (`solix2::dlmm`, hand-encoded like the S4 test)

All instructions carry `event_authority` = `["__event_authority"]` and the program id. Optional accounts are passed as the DLMM
program id. `RemainingAccountsInfo { slices: [] }` (4 zero bytes) suffices for classic SPL.

| Instruction | Discriminator | Use |
|---|---|---|
| `initialize_lb_pair2` | `493b2478ed536cc6` | open the pair (once per collection) |
| `initialize_bin_array(index i64)` | `235613b94ed44bd3` | before the first add, and whenever a range enters a new 70-bin array |
| `initialize_position_pda(lower, width ≤ 70)` | `2e527d92558de499` | the desk's position. Base = owner = `K_desk`, payer = `K_main` |
| `increase_position_length2(min_upper)` | `ffd2cc477389e171` | only for ranges over 70 bins; ≤ 91 bins per call |
| `add_liquidity_by_strategy2` | `03dd95da6f8d76d5` | first deposit and fee top-ups (`SpotImBalanced` = 6) |
| `rebalance_liquidity` | `5c04b0c177b95309` | **re-centre**, and claim fees in the same instruction (`should_claim_fee`) |
| `remove_liquidity_by_range2` | `cc02c391359191cd` | partial withdrawals (burns, rebalancing beyond the arb), and the R2 pre-step |
| `go_to_a_bin(bin)` | `9248aee028fd54ae` | move an empty pool's price (R2, or an empty squat) |
| `swap2` | `414b3f4ceb5b5b88` | the arb's buy and sell, and the site's hop 1. Bin arrays go in swap order |
| `claim_fee2(min, max)` | `70bf65ab1c907fbb` | fee sweeps (SOL only) |
| `close_position2` | `ae5a2373ba2893e2` | wind down (the position rent comes back) |

PDAs:

| Account | Seeds |
|---|---|
| pair | `[preset, min(mint), max(mint)]` |
| reserve | `[pair, mint]` |
| oracle | `["oracle", pair]` |
| bin array | `["bin_array", pair, i64 LE]`, index = ⌊bin / 70⌋ |
| preset | `["preset_parameter2", u16 LE index]` |

Decoders: `LbPair` (`active_id` at 76, `bin_step` at 80, `status` at 82, variable parameters at 40), `PositionV2` (lower and
upper bin at 7,912 and 7,916). The fixtures for a byte-for-byte check, as in W2, can be cut from this test's instructions.

## 6. Still open

- Rebalance CU and size at bin step 30 (preset 286), if the lower buyer cost is wanted.
- R2's trimmed size once the mint leg moves to an earlier transaction (estimated, not built).
- The pair's `creator_pool_on_off_control` and status rules for presets: whether Meteora can disable a permissionless pair.
  Meteora's operator can (`set_pair_status`); the same trust in Meteora as with DAMM v2.
- Jupiter's post-grace test on a DLMM pool is assumed to be run the same way as on DAMM v2 (per token, from simulated quotes).
  Not observed directly.
