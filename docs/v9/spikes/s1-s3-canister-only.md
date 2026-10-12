# Spikes S1–S3: canister-only Rook on Solana (D-l)

Status: **run 2026-10-10** on branch `golive-2026-10-08` (HEAD `d43b69a`, uncommitted). Questions: `docs/v9/canister-only-solana.md`
§15 (S1, S2, S3) and the decisions in §14. **No mainnet transaction was sent and no real funds were used.** S1 and the size/CU half of
S2 ran in LiteSVM 0.10 against the mainnet program bytes of Meteora DBC 0.2.1, DAMM v2 0.2.4 and Metaplex Token Metadata
(`solana/tests/fixtures`, the Q1 dump). The routing half of S2 used read-only public APIs. S3 ran on a **local** dfx 0.32 replica
(PocketIC, `schnorr:Ed25519:key_1`); no IC-mainnet canister was created. Legend as the design: **[M]** measured here, **[W]** from
documentation or a live API, **[A]** assumption.

Code (new files only):

- `solana/tests/s1_canister_only.rs`: the S1 spike, the pool-depth and Jupiter liquidity-test measurements, and the S2 bundle (6 tests, all passing). `programs/rook_backing/tests/s1_canister_only.rs`
  is a 4-line shim so Cargo discovers it without touching `Cargo.toml`. Curves: `solana/tests/fixtures/s1_wx_curves.json` (from
  `tests/q1/design_curve.mjs 1e9 20 <T_wX> 50 6` for every `T_wX` of §3.2).
  Run: `cd solana && cargo test -p rook_backing --test s1_canister_only -- --nocapture --test-threads 1`.
- S3: `ops/rehearsal/sol-multisig/` is a copy of the Q3 `sol-sign` rehearsal (sol-sign itself is unchanged). It holds:
  - the canister `solmulti` (crate `rook_sol_multisig`, ic-cdk 0.19.0; `soltx.rs` copied as is);
  - `run.sh` and `drive.py`;
  - `offline/`, a native tool built on `ic-ed25519` 0.6.0 and `ed25519-dalek` 2.

  Run: `ops/rehearsal/sol-multisig/run.sh`. It needs a local replica and a local `solana-test-validator`.

Stand-ins in LiteSVM: ordinary keypairs play the canister's derived keys. All of them are on-curve, so they sign like wallets.

| Stand-in | Plays |
|---|---|
| `canister` | `K_coll[i]`: mint authority and metadata update authority of wX. Per the spike brief, it is also `K_fc` (`fee_claimer` = `leftover_receiver`) |
| the env payer | `K_main` (fee payer of every transaction) |
| `desk` | `K_desk[i]` |
| `coin_owner` | `K_coin[c]` |

## 1. Verdict

| Spike | Result | One line |
|---|---|---|
| **S1** DBC + DAMM v2 with a wX quote and a real-key claimer | **PASS** | Every step lands, and every fee lands in wX. DBC enforces nothing about the quote mint's authorities, and every `T_wX` in §3.2 validates. **But the wX/SOL pool sizing in §5.3 and O-1 is wrong by ≈ 20×** (§2.4) |
| **S2** routing | **PASS, with a time limit** | Jupiter routes DBC curves quoted in an arbitrary SPL token and thin DAMM v2 pools, and quotes SOL → X (DAMM v2) → coin (DBC) in one route today (live example). **But only during a token's grace period (≈ 14–30 days from the mint)**: after it, the wX/SOL pool must pass Jupiter's liquidity test (0.5 SOL a side at `[p/4, 4p]` fails it), and a coin still on its curve is dropped. The site's own two-hop transaction fits easily (1,033 B, 136 k CU) |
| **S3** multi-key signing from the canister | **PASS** (local replica) | 3 derived keys sign one transaction, which landed 3 of 3 times on a local validator. 26.15 B cycles per signature. Parallel signing takes one signature's latency. Derived keys can be computed offline and survive upgrades. **New bound: 20 queued signature requests per key** |

## 2. S1: DBC and DAMM v2 with wX quote (LiteSVM, mainnet bytes) [M]

### 2.1 Checks

| # | Check | Result | Evidence |
|---|---|---|---|
| 1 | Classic SPL wX, 9 decimals, **live** mint authority = canister key, freeze authority none, Metaplex metadata with update authority = canister. Made by the §2 ceremony in **one** transaction: a hot key creates the mint, creates the metadata naming the canister as update authority (not a signer), then `SetAuthority(MintTokens → canister)` | **PASS** | read back: mint authority = canister, freeze `None`, supply 0, decimals 9, metadata update authority = canister. Tx 753 B, 3 signatures (payer, hot key, mint keypair), 42–46 k CU. Metadata account 15,115,600 lamports (rent 0.0051156 + Metaplex fee 0.01), matching §2's 0.0165772 SOL with the mint |
| 2 | `create_config` with `quote_mint = wX` (live mint authority), `fee_claimer = leftover_receiver = ` the canister's on-curve key, Rook's Q1 parameters (anti-sniper schedule, fixed 1B supply, 50 % migration fee, DAMM v2 Customizable, LP 50/50 permanently locked) and the curve in wX | **PASS** | stored as given: quote = wX, claimer = leftover = K_fc, `collect_fee_mode` 0 (quote), fixed supply. 36.8 k CU |
| 3 | Curve validation for **every** `T_wX` in §3.2: 3, 4, 5, 7, 9, 10, 11, 12, 13, 15, 21, 22, 25, 26, 29, 37, 40, 82, **152 (RSIC), 174 (Weirdos)** | **PASS** | all 20 configs accepted. A 9-decimal wX makes the curve the same as a SOL curve with the same number, so `design_curve.mjs` works unchanged |
| 4 | Launch a coin under the wX config (`initialize_virtual_pool_with_spl_token`, quote = wX) | **PASS** | 117–123 k CU |
| 5 | Buy to graduation with wX (10 min after launch, past the anti-sniper window): a 0.1 wX exact-in buy, then a partial fill | **PASS** | T=5: spent 5.0505 wX (1 % fee), `quote_reserve` = 5.000000000 wX, `migration_progress` 2. T=174: spent 175.7576 wX, reserve 174.000000000. `swap2` 41.8–44.7 k CU |
| 6 | Partner curve fees: `claim_trading_fee`, signed by K_fc | **PASS: in wX** | T=5: 0.040404205 wX and 0 coin. T=174: 1.406066288 wX. A stranger is refused (`Unauthorized` 6053). 22.1 k CU |
| 7 | Migration to DAMM v2 **COIN/wX** (permissionless, any payer) | **PASS** | pool at `["pool", A8gMr…, max, min]`. T=5: 199,600,013,064,632 coin base units / 2.495 wX in the vaults. 276–280 k CU |
| 8 | The partner position NFT goes to K_fc, and its LP is permanently locked | **PASS** | owner = K_fc. Unlocked 0, vested 0, locked > 0 |
| 9 | `withdraw_migration_fee(flag 0)`, signed by K_fc, into a **third-party** ATA (K_coin[c]'s wX ATA), with a Memo in the same transaction | **PASS: in wX, exactly 50 %** | T=5: 2.500000000 wX. T=174: 87.000000000 wX. A stranger as `sender` is refused (`NotPermitToDoThisAction` 6022). A second withdrawal is refused (`MigrationFeeHasBeenWithdraw` 6040). Tx with an idempotent ATA + Memo: 749 B; the DBC instruction + memo: 24.9 k CU |
| 10 | `partner_withdraw_surplus` | **PASS (0)** | the partial fill stops at the threshold, so the surplus is 0. The call succeeds and moves nothing |
| 11 | `withdraw_leftover` → the leftover receiver (K_fc), then K_fc burns the coins | **PASS** | permissionless: no K_fc signature needed. T=5: 52.764 coins. T=174: 8.927 coins. K_fc's `burn_checked`: 4.7 k CU. The design's "≈ 12.8 per coin" is a curve-dependent figure: it is 53 at T=5 and 9 at T=174 |
| 12 | Trade on the graduated COIN/wX pool both ways, then `claim_position_fee` on the partner NFT, signed by K_fc | **PASS: in wX** | after a 0.5 wX buy and a sell: 0.003079343 wX and 0 coin (the migrated pool collects fees in quote only). A stranger is refused (`InvalidAuthority`). 24.6 k CU |
| 13 | Canister mint path: `MintToChecked` + Memo signed by K_main + K_coll | **PASS** | 11.5 k CU |

### 2.2 Refusals and bounds: what DBC checks and what it does not [M]

DBC 0.2.1 put **no** condition on the quote mint beyond its being an SPL mint:

| Variant | DBC |
|---|---|
| quote mint with a **live mint authority** | accepted (the main flow) |
| quote mint **with a freeze authority** | **accepted.** DBC does not check it. "Freeze authority none" is Rook's own rule, enforced by the ceremony read-back and the canister's config check (§3.1) |
| quote mint with 6 decimals (same raw curve) | accepted (the curve then means something else; not used) |
| `collect_fee_mode` 0 (QuoteToken) and 1 (OutputToken) with a non-SOL quote | both accepted |
| `migrated_pool_fee.collect_fee_mode` 0 and 1 with a non-SOL quote | both accepted |
| `leftover_receiver ≠ fee_claimer` | accepted |
| the quote's mint authority changes after the config exists (canister key rotation) | a later launch still works: DBC never reads the quote's authorities again |
| threshold 1 wX on the 3-wX curve | accepted. DBC does not bind the threshold to the curve; `rook-ceremony` must |

No refusal bites the design. The only refusals seen are the authorization checks, and those are correct: a stranger cannot
claim trading fees, cannot withdraw the migration fee, cannot claim LP fees, and nobody can withdraw the migration fee twice.

### 2.3 wX/SOL DAMM v2 pool, range [p/4, 4p], managed (not locked) [M]

Milady's floor is 18.61 SOL per wX. The seed is 0.5 SOL a side (O-1 b), so 0.026867275 wX. The pool is
`initialize_customizable_pool` with a 0.5 % fixed fee, fees in SOL only, timestamp activation and `[s₀/2, 2s₀]`.

| Check | Result | Evidence |
|---|---|---|
| **One transaction**: the canister's first wX mint goes to the desk, the desk wraps SOL, the pool is created, plus a Memo (§5.1, the anti-squat ordering) | **PASS, but at the size limit** | **4 signatures**: K_main, K_coll, K_desk, and the **position-NFT mint**. 1,224 B legacy of 1,232. 167–172 k CU. Seed used to within 26 base units / 512 lamports |
| … with a durable-nonce advance in front (the canister's send line, Q3) | **FAIL as built** | 1,298 B. Variants: drop the Memo and pre-make the wSOL ATA → 1,237 B (still over). Pre-make both desk ATAs and keep the Memo → 1,246 B. **Pre-make both ATAs and drop the Memo → 1,195 B (fits)** |
| Position unlocked and owned by the desk | **PASS** | unlocked > 0, locked 0. NFT owner = K_desk |
| Swaps both ways | **PASS** | 26.5 k CU (buy), 25.9 k CU (sell). Round trips return the price to 18.6100 |
| `claim_position_fee` (K_desk) | **PASS: SOL only** | 0 wX, 1,276,799 lamports after the round trips |
| `add_liquidity`, `remove_liquidity` (25 %), `remove_all_liquidity`, burn the recovered wX, `close_position` | **PASS** | 37.6–37.9 k CU each; `close_position` 18.5 k CU. Vaults left with 3 wX base units and 319,205 lamports of rounding/fees. wX supply after burning: 3 base units (the dust stuck in the vault) |
| DAMM v2 `payer` must be the owner of `payer_token_a/b` | **bound** | the instruction's `payer` is also the token-transfer authority, so **K_desk pays the pool rent** (≈ 0.03 SOL), not K_main. The transaction fee can still be K_main's |

### 2.4 Depth: the §5.3 and O-1 numbers are wrong by ≈ 20× [M]

The same pool at several ranges and seeds. Each buy is sold back before the next.

| range | SOL a side | buy 0.01 SOL: avg / price move | buy 0.1 SOL | buy 0.5 SOL |
|---|---|---|---|---|
| [p/4, 4p] | 0.5 | +1.50 % / +2.00 % | **+10.50 % / +20.89 %** | +50.5 % / +124 % |
| [p/4, 4p] | 5 | +0.60 % / +0.20 % | +1.50 % / +2.00 % | +5.50 % / +10.20 % |
| [p/1.44, 1.44p] | 0.5 | +0.84 % / +0.66 % | +3.84 % / +6.74 % | +17.2 % / +35.9 % |
| [p/1.44, 1.44p] | 5 | +0.54 % / +0.07 % | +0.84 % / +0.66 % | +2.17 % / +3.34 % |
| [p/1.21, 1.21p] | 0.5 | +0.68 % / +0.36 % | +2.32 % / +3.65 % | +9.59 % / +18.9 % |
| [p/1.21, 1.21p] | 5 | +0.52 % / +0.04 % | +0.68 % / +0.36 % | +1.41 % / +1.82 % |
| [p/1.1, 1.1p] | 0.5 | +0.60 % / +0.19 % | +1.43 % / +1.86 % | +5.16 % / +9.48 % |
| [p/1.1, 1.1p] | 5 | +0.51 % / +0.02 % | +0.60 % / +0.19 % | **+0.97 % / +0.93 %** |

"avg" is the buyer's average price against the floor, fee included. "price move" is the pool's own price after the buy.

**What this shows.** With `[p/4, 4p]`, a buy of Δ SOL moves the pool price by ≈ Δ / (SOL a side). The buyer then pays the fee plus
≈ half that move. §5.3 says "`[p/4, 4p]` halves" the full-range move, and that much is right. But §5.3 and O-1 then size the
seeds as if the move were 20× smaller:

| Claim in the design | Measured |
|---|---|
| O-1 b: 0.5 SOL a side gives "≤ 1 % impact up to ≈ 0.1 SOL" | 0.1 SOL pays **+10.5 %** and moves the pool +21 % |
| §5.3 / O-1 d: 2.5 SOL a side gives "≤ 1 % impact up to ≈ 0.5 SOL" | that needs **≈ 50 SOL a side** at `[p/4, 4p]` (1 % ≈ 0.5 % fee + 0.5 % slippage → Δ ≤ 1 % of a side) |

**Design changes (S1):**

1. **Narrow the range, and let the arbitrage loop re-centre it.** The position is unlocked (O-2) and the canister re-ranges
   anyway (§5.4).
   - `[p/1.1, 1.1p]` (±10 % in price) at 5 SOL a side gives ≤ 1 % all-in up to 0.5 SOL.
   - The same range at 0.5 SOL a side gives ≈ 1.4 % at 0.1 SOL.
   - Cost: the re-range trigger (§5.4 "70 % of the way to an edge") fires on a ≈ 7 % floor move instead of a 3× one, so re-ranges
     are routine rather than rare. Each re-range is `remove_all` + `close_position` + a new position + `add_liquidity`. A new
     position needs a new NFT-mint signer (point 3).
   - Re-ranging stays bounded by the arb's cadence and limits.
2. **Rewrite O-1's table.**
   - At 0.5 SOL a side, one-transaction buys are good only to ≈ 0.01–0.05 SOL, or to ≈ 0.1 SOL with the ±10 % range.
   - Larger buys should go through the direct mint (§6) or accept the slippage, which the arb recovers for the desk.
   - "Target depth" means ≈ 5 SOL a side with a ±10 % range for impact alone, not 2.5 SOL with `[p/4, 4p]`. Jupiter's
     post-grace test raises it to ≈ 10 SOL (§3.3).
3. **Every new DAMM v2 position needs a fresh position-NFT mint that signs.** That covers pool creation and every re-range.
   - In the canister this is **one more threshold signature** (≈ 26 B cycles ≈ $0.035).
   - It needs a throwaway derived path such as `sol/nft/<n>`. The key signs once, as a new account, and holds nothing.
   - Add it to §1.1's key table and to the cost of a re-range.
4. **The one-transaction "mint the seed and create the pool" does not fit with a durable nonce unless it is trimmed.** Any one
   of these works:
   - (a) pre-create the desk's wX and wSOL ATAs in an earlier transaction (an ATA can exist before any supply), and put the
     Memo in the next transaction (1,195 B);
   - (b) send this one transaction with a recent-blockhash lifetime (Q3's fallback);
   - (c) use a v0 message with a lookup table (`soltx` builds legacy only today).

   (a) keeps the anti-squat ordering of §5.1: no wX exists before the pool does.
5. The pool rent comes from **K_desk** (DAMM v2's `payer` is the token authority), so §12.2's per-pool rent is desk money.

Unchanged: everything in §3 (config fields, the `T_wX` rule, the canister actions of §3.3, which keys sign what) holds as
written.

## 3. S2: routing

Read-only, 2026-10-10. Jupiter's quote API was called without a key at `lite-api.jup.ag/swap/v1/quote`; `api.jup.ag` and Ultra
`/order` gave the same routes. The pools were decoded over public mainnet RPC. Jupiter's labels: `dbcij3…` = "Dynamic Bonding
Curve", `cpamd…` = "Meteora DAMM v2". Raw answers and scripts are in the session scratchpad; they are not committed. **[W]** below
means a live API or doc.

### 3.1 Checks [W]

| # | Check | Result | Evidence |
|---|---|---|---|
| a | Jupiter routes a **DBC curve pool whose quote is not SOL** | **PASS** | **JUP-quoted** coin `HqNtPF3w…` on curve `7gSz4tfP…` (config quote = JUP, `is_migrated` 0, created 2026-10-09). JUP → coin is one hop through `7gSz…`, the same under `dexes=Dynamic Bonding Curve` and `onlyDirectRoutes`. SOL → coin is 3 hops (SOL → USDC → JUP → curve). Curves quoted in EMBER and OTC (a Token-2022 token) also routed, one with only 31 raw units in it. **USDC-quoted curves: 6 sampled, all `TOKEN_NOT_TRADABLE`.** All were old curves that never graduated, which matches the documented removal rule (3.2). There is no sign that Jupiter restricts DBC routing to SOL or USDC quotes |
| b | A **thin DAMM v2 pool** | **PASS during grace** | USDP/USDC `7APx1dDS…` ($950 TVL, created 2026-10-03): chosen without a filter, one hop. A concentrated DAMM v2 range (SOL/USDC `8Pm2kZpn…`, ≈ 6.3× range) is used too. Thin pools on **old** tokens fail: `TOKEN_NOT_TRADABLE`, or `NO_ROUTES_FOUND` for pools of $19–42 |
| c | Multi-hop **SOL → X → coin** | **PASS** | **FLYWHEEL is Rook's setup today.** X = `FLYcmcBD…` is classic SPL, unverified (tag `unknown`), and created 2026-10-03. Its only SOL liquidity is one full-range DAMM v2 pool `7G6z3K7n…` (≈ $2–4k). SOL → coin `FLYTh4kH…` routes in **2 hops: DAMM v2 `7G6z…` SOL → X, then DBC curve `23WsxUFf…` X → coin**, and back the other way. Also SOL → EMBER (DLMM) → DBC, and SOL → OTC ($689 pool) → DBC |
| d | Listing and indexing requirements | documented | see 3.2 |
| e | A **live mint authority** on the quote token | **not an exclusion** | a token with a live mint authority routes as the quote of a DBC curve. Jupiter's token API shows `mintAuthority` and tags it only `unknown`. The docs call it an informational indicator ("supply can increase at any time"). How jup.ag displays it was not checked |
| f | Route size | bound | the 2-hop FLYWHEEL route needs `maxAccounts` ≈ 49–56 (it routes at 56, not at ≤ 48). A single DBC hop needs ≈ 31–40. `onlyDirectRoutes=true` on SOL → coin finds nothing. Integrators (wallets, aggregators that cap accounts) must allow ≈ 56 |

### 3.2 Jupiter's rules for new markets [W]

Source: `developers.jup.ag/docs/swap/routing/market-listing`, the current page.

- **No manual listing.** New markets on supported programs are routed automatically. "Meteora Dynamic Bonding Curve" and
  "Meteora DAMM V2" are both on the instant-routing list. A live mint authority or "unknown" status does not block routing.
- **Grace period.** There is no liquidity check during a grace period, and it is counted from the **token's age, not the
  market's**: a new pool for an old token does not reset it.
  - The page's example has day 20 inside the period and day 30 outside.
  - An older post (`discuss.jup.ag`, 2024) says 14 days; Meteora's FAQ snippet says 15.
  - Plan for **14 days**.
- **After the grace period**, a market must pass **one** of two tests, re-checked every 30 minutes:
  1. a $500 round trip loses < 30 %;
  2. the per-token price of a $1,000 buy is within 20 % of the price of a $500 buy.

  Jupiter gates **whole tokens**: once a token fails, every pair with it is dropped.
- **Bonding curves that have not graduated by the end of the grace period are removed**; the graduated pool is added once it
  exists.
- Not explained: LOTTO (29 days old, a $4.6k DAMM v2 pool) and MOONER (an $889 full-range pool) are not tradable. The real
  check may be stricter than the page reads.

### 3.3 Would a wX/SOL pool pass Jupiter's test? (LiteSVM) [M]

Test `s1e`: Milady wX at 18.61 SOL, fee 0.5 %, $500 ≈ 4.5 SOL and $1,000 ≈ 9 SOL at ≈ $110 per SOL. Each buy is sold back before the
next. Jupiter needs **either** test 1 **or** test 2 to pass.

| range | SOL a side | test 1: $500 round trip (< 30 % loss) | test 2: $1k vs $500 unit price (< 20 %) | passes? |
|---|---|---|---|---|
| [p/4, 4p] | 0.5 | buy refused (`PriceRangeViolation`) | refused | **no** |
| [p/4, 4p] | 5 | 1.0 % | +30.9 % | yes (1) |
| [p/4, 4p] | 10 | 1.0 % | +18.3 % | yes (1, 2) |
| [p/1.21, 1.21p] | 0.5 | refused | refused | **no** |
| [p/1.21, 1.21p] | 5 | 1.0 % | $1k refused | yes (1) |
| [p/1.21, 1.21p] | 10 | 1.0 % | +3.9 % | yes (1, 2) |
| full range | 0.5 | 1.0 % | +90 % | yes (1) [A: MOONER suggests Jupiter may judge it stricter] |
| full range | 10 | 1.0 % | +30.9 % | yes (1) |
| full range | 20 | 1.0 % | +18.3 % | yes (1, 2) |

On a constant-product pool, a round trip that stays inside the range loses only the two fees (1.0 %). Test 1 therefore fails only
when the $500 buy **runs off the end of a concentrated range**: the program refuses it. Range edges are not a problem for full
range.

To pass with margin, a concentrated pool needs enough depth to take ≈ 9 SOL without leaving its range: **≈ 10 SOL a side at
`[p/1.21, 1.21p]`**, and that also gives < 1 % impact on 0.5 SOL buys (§2.4). The design's O-1 b (0.5 SOL a side at `[p/4, 4p]`)
fails both tests and would make **wX itself** untradable on Jupiter after ≈ 14 days.

### 3.4 Can the site build the two-hop transaction itself?

**Yes** (3.5 below). It is needed in any case:

- for coins still on their curve after their grace period;
- for any wX that fails Jupiter's test;
- as the primary path that does not depend on Jupiter.


### 3.5 The site's own two-hop transaction (LiteSVM) [M]

A fresh wallet with no wSOL, wX or coin ATA signs one legacy transaction. The pool is deepened to 10 SOL a side here so 3 %
slippage holds. Size and CU do not depend on depth. The instructions, in order:

- CU limit and CU price;
- create the wSOL ATA, transfer, `sync_native`;
- create the wX ATA (idempotent); DAMM v2 `swap` SOL → wX with `minimum_amount_out`;
- create the coin ATA (idempotent); the coin hop;
- close the wSOL ATA.

| Bundle | Size (legacy, limit 1,232) | Accounts | CU | Result |
|---|---|---|---|---|
| **pre-graduation**: DAMM v2 wX/SOL + **DBC `swap2`** wX → coin (amount in = hop 1's `minimum_amount_out`) | **1,033 B** | 24 | **135,606** | **PASS.** 0.1 SOL bought 1,649,538 coins; 107,869 wX base units (≈ 0.002 SOL) of hop-1 surplus left in the user's wX ATA |
| **post-graduation**: DAMM v2 wX/SOL + DAMM v2 COIN/wX | **903 B** | 20 | **88,394** | **PASS** |

Both fit in one legacy transaction with ≈ 200–330 B to spare and well under 200 k CU. No lookup table is needed. To avoid the wX
dust, the site can use DAMM v2 `swap2` in exact-out mode for hop 1, or sweep the dust on the next buy.

## 4. S3: multi-key signing from the canister (local replica)

Local dfx 0.32 replica (PocketIC, `schnorr:Ed25519:key_1`), canister `uxrrr-q7777-77774-qaaaq-cai`, and a throwaway local
`solana-test-validator` (Agave 4.1.2). No IC-mainnet canister was created.

**The transaction.** `multi_sign` builds one legacy transaction:

1. `AdvanceNonceAccount(nonce, K_main)`;
2. transfer K_int → K_coll;
3. SPL `MintTo(mint, dest, K_coll)`;
4. a Memo v2.

The message header is (3, 0, 4) with 10 keys. The canister maps each header signer to its derivation path, signs each with
`sign_with_schnorr`, and assembles the transaction. It refuses before signing if any signer is not one of its own keys.

| # | Check | Result | Numbers |
|---|---|---|---|
| 1 | Derive several Ed25519 keys (different paths) with `schnorr_public_key` | **PASS** | `[sol]` 5YnN8RHb…, `[sol, intent, 7u64 LE]` AvLdweiP…, `[sol, coll, 3u32 LE]` APy8RtVb…, `[sol/intent/7]` GpsWc277…, `[sol/coll/3]` BVQHkRyp…, `[sol-relay]` Hx86fhac… (full list in the run's report). 615–645 ms each in the canister; no cycles attached (≈ 5.7 M of the canister's own execution), whatever the path depth |
| 2 | One transaction needing 3 canister keys: K_main fee payer + nonce authority, K_int, K_coll | **PASS** | 622 B (633 B with a longer Memo naming K_int as a signer; 614 B with flat paths). Compiling the message: ≈ 340 k instructions |
| 3 | Signatures verify off-chain | **PASS, 9 of 9** | 3 transactions × 3 signatures; each checked with `ed25519-dalek` `verify_strict` and with `ic-ed25519` |
| 4 | Lands on a local validator | **PASS, 3 of 3** (confirmed, `err` null) | two nonce transactions in a row both landed; the K_coll balances, the minted amounts and K_main's fee (15,000 lamports per transaction) all match |
| 5 | Cycles per signature | **PASS** | `cost_sign_with_schnorr` = **26,153,846,153** every time. Balance delta 26,160.05–26,160.74 M per signature. **≈ 78.48 B cycles per 3-signer transaction**, sequential or parallel, so §6.1's "≈ 80 B ≈ $0.11" fee basis holds |
| 6 | Latency | **PASS** | 1 signature ≈ 1.22–1.29 s; **3 sequential ≈ 3.65–3.75 s; 3 in parallel (`join_all`) ≈ 1.22–1.25 s**. The 3 parallel calls all reply in the same round |
| 7 | Derived public keys computable **offline** | **PASS, byte-equal** for 8 answers (7 paths + the empty path), keys and chain codes | `ic-ed25519` 0.6.0. Two methods match: (a) `PublicKey::derive_subkey_with_chain_code(&DerivationPath::new(path), &chain_code)` from the canister-level key (`schnorr_public_key` with an empty path); (b) `PublicKey::derive_pocketic_key(Key1, &canister_id, path)`. The canister id is the first path element. For mainnet the crate hardcodes `key_1` and `test_key_1` and offers `derive_mainnet_key(MasterPublicKeyId::{Key1, TestKey1}, &canister_id, path)`. Not checked on mainnet (no mainnet run); one comparison on the first mainnet run is advised |
| 8 | Keys stable across upgrades | **PASS** | 6 paths, identical keys and chain codes after `--mode upgrade` to a different module and after `--mode reinstall`. Derivation uses only the master key, the canister id and the path |
| 9 | Concurrency | **bound found** | 10 concurrent signatures: all succeeded. At 30 and 60, **exactly 20** succeeded and the rest were rejected at once: code 4, "request queue for key schnorr:Ed25519:key_1 is full". Rejected calls are refunded (a burst of 30 cost 20 × 26.16 B) |

**Path style must be pinned before launch.** Both styles landed, but they differ:

| Style | Example | Derivation | Consequence |
|---|---|---|---|
| Multi-element | `[b"sol", b"coll", i as u32 LE]` | one HKDF step per element: children of K_main | K_main's key and chain code derive every child offline (public keys only, which is harmless). The integer width and endianness are permanent |
| Flat string | `[b"sol/coll/3"]` | one step: a sibling of `sol` and `sol-relay` | the decimal and separator format is permanent, and identifiers must not contain `/` |

Changing either encoding later changes every address. Recommendation: multi-element with fixed-width little-endian integers.
Write it into §1.1.

**Signer order.** The legacy compiler puts the fee payer first, then the writable signers **sorted by key bytes**. The order came
out K_main, K_coll, K_int, not the role order. The canister must sign by `message.signers[i] → path`, as `multi_sign` does. The new
rule for `solkey.rs` ("every signer is a key of this canister, else refuse before any cycles are spent") works as §1.1 says.

## 5. What changes in the design

In order of weight. Nothing failed outright; the items below correct numbers and add bounds.

1. **wX/SOL pool sizing (§5.1, §5.3, O-1, §12.2): rewrite.**
   - At `[p/4, 4p]`, a buy of Δ SOL moves the pool price by ≈ Δ / (SOL a side). The design's seeds are ≈ 20× too thin for the
     impact it claims (§2.4).
   - The same seeds fail **Jupiter's post-grace liquidity test** (§3.3), and Jupiter drops a failing token everywhere. A thin
     pool would take **wX itself** out of Jupiter after ≈ 14 days.
   - Proposal: a **narrow, canister-re-centred range** (`[p/1.21, 1.21p]`, ±10 % in price) and **≈ 10 SOL a side** for each
     collection that should have Jupiter routing. That gives ≤ 1 % on 0.5 SOL buys and passes both Jupiter tests.
   - O-1 becomes a choice between:
     - (i) ≈ 20 SOL of desk money per routed collection;
     - (ii) a **full-range** pool at a small seed: it passes Jupiter's round-trip test as measured (a constant-product round trip
       loses only fees), but it has poor impact and MOONER's case suggests Jupiter may be stricter;
     - (iii) no pools, with the site's own path (6, below) and the direct mint as the only routes.
   - The re-range trigger (§5.4) fires on ≈ 7 % floor moves with a ±10 % range, so re-ranges are routine.
2. **Jupiter routing is time-limited.**
   - Each token gets a grace period of ≈ 14 days (docs: 14–30) from its own mint date: wX from its first mint, each coin from its
     launch.
   - After that:
     - wX needs the depth in item 1;
     - **a coin still on its DBC curve is removed from Jupiter until it graduates.** For the slow collections (large `T_wX`)
       that will be the normal case.
   - §5.1's "Jupiter routes SOL → wX → coin" holds only within these limits, so the site must carry its own route (item 6).
3. **One more threshold signature per new DAMM v2 position.**
   - Pool creation and every re-range mint a position NFT whose mint account must sign.
   - Add a throwaway path (`sol/nft/<n>` or the multi-element form) to §1.1, and ≈ $0.035 to each pool open or re-range.
4. **One-transaction "mint the seed + create the pool" with a durable nonce is over 1,232 B** (1,298 B).
   - Pre-create the desk's wX and wSOL ATAs (an ATA can exist before any supply) and move the Memo to the next transaction:
     1,195 B, and the anti-squat ordering of §5.1 holds.
   - Alternatively, use the recent-blockhash lifetime for this one transaction.
5. **K_desk pays the DAMM v2 pool rent.** The instruction's `payer` is also the token authority. Book it as desk money (§12.2).
6. **The site's own two-hop transaction** is cheap and fits easily, and becomes a required feature:
   - pre-graduation: DAMM v2 + DBC, 1,033 B, 136 k CU;
   - post-graduation: 903 B, 88 k CU.

   Use exact-out on hop 1 (or a dust sweep) for the ≈ 0.002 SOL of wX left over.
7. **Signing (§1.1, §6.1, §13): confirmed, with three additions.**
   - Sign the signers **in parallel**: 1 signature's latency (≈ 1.2 s locally) instead of 3, for the same 78.5 B cycles.
   - Map signatures by the compiled message's signer order, which sorts by key bytes, not by role.
   - Treat a "signature queue full" reject (**20 requests per key** locally; mainnet value unknown and shared across the signing
     subnet) as retry-next-tick; rejected calls are refunded. At 3 signatures per transaction, ≤ 6 transactions can sign at
     once, which bounds the intent desk's and arb loop's throughput per tick.
8. **Derivation-path encoding must be pinned before launch.** Write the chosen form into §1.1: multi-element with fixed-width
   little-endian integers is recommended. §1.1's "offline derivation **[W]**" becomes **[M]**: `ic-ed25519`'s
   `derive_mainnet_key(Key1, canister_id, path)` lets every K_* address be computed, and pinned in the launch file, before the
   canister runs. Keys are stable across upgrades and reinstalls. Check this once on the first mainnet run.
9. **Rook's own config checks must carry what DBC does not check** (§3.1, `rook-ceremony`):
   - freeze authority none (DBC accepts a quote with a freeze authority);
   - the threshold matching the curve (DBC accepts a mismatched one).
10. **Small corrections to the design's numbers.**
    - Leftover coins per graduated coin depend on the curve: ≈ 53 at `T_wX` = 5, ≈ 9 at 174 (the design says ≈ 12.8).
    - The Memo v2 id is `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` (as `relaysol.rs` already has).

**Unchanged:** §2 (wX mint, metadata, ceremony), §3.1–3.3 (DBC config with wX quote, `T_wX` thresholds including 152 and 174,
fee claims as K_fc into third-party ATAs, all fees in wX), and S3's cost basis (26.15 B cycles per signature).

**Still open (not testable without mainnet):**
- the IC-mainnet `test_key_1` run (latency, real pricing and queue size);
- how fast Jupiter indexes a brand-new pool;
- the exact grace-period length;
- why LOTTO and MOONER are untradable.

