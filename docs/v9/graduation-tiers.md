# v9: graduation tiers per collection

**Approved by the operator on 2026-10-10:** tiers 85 / 170 / 255 / 510 SOL with the assignment below; floors are re-measured on the morning of day 1 and any collection past its tier boundary moves up before the configs are created.

Status: **built, for the operator's approval before day 1**, 2026-10-10, branch `golive-2026-10-08`. Nothing was signed or sent.
Read-only, keyless sources only: CoinGecko's NFT API, Ordinals Wallet's `turbo` API, the launch-params rate tickers (Coinbase, OKX,
Kraken, KuCoin), and one mainnet `getMinimumBalanceForRentExemption`.

**Legend.** **[M]** measured on 2026-10-10 (≈ 07:30 UTC). **[R]** read from this repository. **[A]** assumption or estimate.

## 0. The decision and the rule

**Operator decision (2026-10-10):** every collection config graduates at a size whose backing buys **at least one NFT at once**. A
coin should never graduate and then wait for other coins of its collection before anything is bought.

What one graduation can spend on NFTs [R]:
- The migration fee is 50 % of the threshold. It goes to the collection's claimer and `rook_backing::back` moves it into the module.
- rook_core's convert keeps `reserve_bps` 25 % of it as SOL.
- It charges `conv_bps` 3 %.

So the **NFT buying power** of one graduation is `threshold × 0.5 × 0.75 / 1.03 ≈ 0.364 × threshold`.

**Rule.** A collection gets the **smallest tier whose buying power is at least 1.33 × its floor in SOL**. The 1.33 gives 33 %
headroom for the floor to move. The code is `launch-params.mjs` `tierFor()`, and `ops/test/launch_params.test.ts` holds every
assignment to it.

**Floor reference.** This is CoinGecko's floor, which is also the monitor's D8 reference [M].
- For Ordinals collections the canister buys from Ordinals Wallet's book. It does so only after the wake check
  (`ordinals-collections.md` §5) puts that book within 20 % of CoinGecko's floor. So 1.33 × the CoinGecko floor is at least ≈ 1.11 ×
  the book.
- Identity MD has no keyless live source. CoinGecko does not list it, and OpenSea's API needs a key. Its floor is the 1.78 ETH
  recorded on 2026-10-09 (`docs/v7/dormant-collections.md`) [R].

**Rates [M].**
- 22.693637936 SOL per ETH: the launch-params median of 4 sources, spread 12 bps.
- 752.2 SOL per BTC: Coinbase BTC-USD 82,570.71 / SOL-USD 109.77. Kraken's SOLXBT of 0.0013294 agrees.

## 1. The tiers

| tier | threshold | NFT buying power / graduation | covers a floor up to | start mcap | graduation mcap | migration fee to the claimer |
|---|---|---|---|---|---|---|
| **85** | 85 SOL | 30.95 SOL | 23.27 SOL | 53.1 SOL (≈ $5.8k) | 212.5 SOL (≈ $23.3k) | 42.5 SOL |
| **170** | 170 SOL | 61.89 SOL | 46.53 SOL | 106.3 SOL (≈ $11.7k) | 425 SOL (≈ $46.7k) | 85 SOL |
| **255** | 255 SOL | 92.84 SOL | 69.80 SOL | 159.4 SOL (≈ $17.5k) | 637.5 SOL (≈ $70.0k) | 127.5 SOL |
| **510** | 510 SOL | 185.68 SOL | 139.61 SOL | 318.8 SOL (≈ $35.0k) | 1,275 SOL (≈ $140.0k) | 255 SOL |

Market caps are on the fixed 1B supply. USD is at SOL = $109.77. The graduation mcap is always 2.5 × the threshold (50 % migration
fee, 20 % of supply to the pool).

**Why these four.** They are the smallest set that covers today's floors:
- 85 covers 21 collections;
- 170 covers Identity MD and Taproot Wizards;
- 255 covers Pudgy Penguins;
- 510 covers BAYC.

A 340 tier would only move Pudgy from 1.57 to 2.1 NFTs a graduation. An 850 tier has no collection. An unused tier is one more
pinned config and nothing else, so neither is included. A later tier, or a later move of a collection to another tier, is a **new
DBC config** for that listing (rook_backing allows `MAX_CONFIGS` 8 per partner). It is not an edit of a day-1 config, which is
permanent.

**What stays the same in every tier:**
- the anti-sniper schedule (50 % → 1 % over 10 min, linear, 60 × 10 s), for everyone but the creator's first buy: since 2026-10-11
  (canister-only collection configs, `enable_first_swap_with_min_fee`) a buy in the launch transaction itself pays the schedule's
  end fee, 1.000004 % (`canister-only-solana.md` §3.1);
- the 50 % migration fee, with none to the creator;
- the fixed 1B supply, 6 decimals;
- the LP permanently locked 50/50;
- DAMM v2 with a 1 % pool fee in SOL;
- immutable metadata, no creation fee.

Only `migration_quote_threshold`, `sqrt_start_price` and the curve differ. A fixed-supply DBC curve validates only for the threshold
and fee it was designed for (spike Q1 §6.1, A3). So each tier's curve is `solana/tests/q1/design_curve.mjs 1000000000 20 <tier> 50 6`:

| tier | sqrt_start_price | curve (sqrt_price / liquidity) | leftover to the claimer |
|---|---|---|---|
| 85 | 134452599321738966 | 268905193275807567 / 215124158914782432327345953095692; MAX / 3435308751277378593870180 | 12.78 coins |
| 170 | 190144688344867500 | 380289371323205464 / 304231501351788061953802646828752; MAX / 3434578513360188981331421 | 9.03 coins |
| 255 | 232878731269108314 | 465757457172827529 / 372605970030573352902104940717502; MAX / 3433848552104604036791622 | 7.37 coins |
| 510 | 329340259036093404 | 658680512710218671 / 526944414457749482192302864017669; MAX / 3431658944516985920937941 | 5.21 coins |

**Checked [M]:**
- **In LiteSVM with the mainnet DBC 0.2.1, DAMM v2 and Metaplex bytes** (rook_backing's harness, mock rook_core), each of the four
  configs ran through every step: `create_config`, `register_partner_config`, a launch, a buy to the threshold, `migration_damm_v2`,
  and `rook_backing::back`. `back` moved exactly half the threshold (42.5 / 85 / 127.5 / 255 SOL) to the module vault. The scratch
  test was run and removed.
- **By `ops/test/audit_a6_day1_verify.py`'s port of DBC's fixed-supply validation:** all four pass, and the leftover is ≤ 13 coins.
- **ROOK's config is unchanged:** 85 SOL, migration fee 0, its own curve.

## 2. The assignment (live floors, 2026-10-10)

"NFTs / graduation" is buying power ÷ floor, before the conversion and purchase costs.
- On Ethereum these are the Relay SOL→ETH leg and Seaport gas.
- On Bitcoin they are Relay SOL→BTC and miner fees: ≈ 1-3 % at these sizes, see `ordinals-collections.md` §7.

| id | collection | floor | floor (SOL) | source | **tier** | NFTs / graduation |
|---|---|---|---|---|---|---|
| 0 | Milady Maker | 0.82 ETH | 18.61 | CoinGecko `milady-maker` | **85** | 1.66 |
| 1 | Azuki | 0.604 ETH | 13.71 | CoinGecko `azuki` | **85** | 2.26 |
| 2 | Mutant Ape Yacht Club | 0.818 ETH | 18.56 | CoinGecko `mutant-ape-yacht-club` | **85** | 1.67 |
| 3 | Pudgy Penguins | 2.6 ETH | 59.00 | CoinGecko `pudgy-penguins` | **255** | 1.57 |
| 4 | Bored Ape Yacht Club | 6.03 ETH | 136.84 | CoinGecko `bored-ape-yacht-club` | **510** | 1.36 |
| 5 | Doodles | 0.262 ETH | 5.95 | CoinGecko `doodles-official` | **85** | 5.20 |
| 6 | Moonbirds | 0.392 ETH | 8.90 | CoinGecko `moonbirds` | **85** | 3.48 |
| 7 | CloneX | 0.149 ETH | 3.38 | CoinGecko `clonex` | **85** | 9.15 |
| 8 | Lil Pudgys | 0.257 ETH | 5.83 | CoinGecko `lilpudgys` | **85** | 5.31 |
| 9 | Checks VV | 0.153 ETH | 3.47 | CoinGecko `checks-vv-originals` | **85** | 8.91 |
| 10 | Identity MD | ~1.78 ETH | 40.39 | repo, 2026-10-09 (OpenSea) [R] | **170** | 1.53 |
| 11 | Bitcoin Frogs | 0.004 BTC | 3.01 | CoinGecko `bitcoin-frogs` | **85** | 10.29 |
| 12 | NodeMonkes | 0.02318 BTC | 17.44 | CoinGecko `nodemonkes` | **85** | 1.77 |
| 13 | OMB | 0.0175 BTC | 13.16 | CoinGecko `ordinal-maxi-biz-omb` | **85** | 2.35 |
| 14 | Bitcoin Puppets | 0.01024 BTC | 7.70 | CoinGecko `bitcoin-puppets` | **85** | 4.02 |
| 15 | Bitcoin Punks | 0.00314 BTC | 2.36 | CoinGecko `bitcoin-punks` | **85** | 13.09 |
| 16 | Natcats | 0.00284 BTC | 2.14 | CoinGecko `natcats` | **85** | 14.49 |
| 17 | Runestone | 0.001388 BTC | 1.04 | CoinGecko `runestone` | **85** | 29.64 |
| 18 | Bitcoin Weirdos | 0.00065 BTC | 0.49 | CoinGecko `bitcoin-weirdos` | **85** | 63.30 |
| 19 | CENTS | 0.00515 BTC | 3.87 | CoinGecko `cents` | **85** | 7.99 |
| 20 | RSIC | 0.000745 BTC | 0.56 | CoinGecko `rsic-metaprotocol` | **85** | 55.20 |
| 21 | Pizza Ninjas | 0.00872 BTC | 6.56 | CoinGecko `pizza-ninjas` | **85** | 4.72 |
| 22 | Quantum Cats | 0.01045 BTC | 7.86 | CoinGecko `quantum-cats` | **85** | 3.94 |
| 23 | The Wizards of Ord | 0.0054 BTC | 4.06 | CoinGecko `the-wizards-of-ord` | **85** | 7.62 |
| 24 | Taproot Wizards | 0.047 BTC | 35.35 | CoinGecko `taproot-wizards` | **170** | 1.75 |
| 25 | Bitcoin Wizards | 0.0135 BTC | 10.15 | CoinGecko `bitcoin-wizards` | **85** | 3.05 |

**Close to a boundary:**
- **BAYC** (1.36 × at 510): a floor above 6.15 ETH would need a larger tier.
- **Milady and MAYC** (1.66-1.67 × at 85): they cross 1.33 × above ≈ 1.03 ETH.
- **NodeMonkes** (1.77 × at 85): it crosses above ≈ 0.031 BTC. Its Ordinals Wallet book, 0.0327, is above that today. It cannot
  wake until the book comes within 20 % of CoinGecko (§0), so this does not matter yet.

The assignment is fixed at the day-1 configs. **Re-measure on the morning of day 1.** If a floor has crossed its tier's ceiling,
change `graduation-tiers.json` and regenerate the pins (`rook-ceremony pins --write`) before the ceremony. Once created, a listing's
tier changes only through a new config.

**Throughput is a separate limit [R].** At stage 0 a module mints at most `max_items_per_day` 3 items a day. A tier that buys ≥ 1
NFT per graduation guarantees the first purchase. It does not guarantee that several graduations on the same day are all spent that
day.

## 3. Where the tier lives (pinned)

- `deployments/launch/graduation-tiers.json`: the tier of every slug, with the floor, its source, the rates and the rule.
- `ops/solana/launch-params.mjs`:
  - `GRADUATION_TIERS`, `TIER_SOL`, `buyingPower`, `tierFor` and `tierOverrides`;
  - `meteoraSection().tiers.<sol>`: the overrides for threshold, start price and curve;
  - `listingOrder()`: each entry gains `graduation_sol`.
- `ops/solana/day1-pins.json`:
  - `tiers.<sol>` pins each tier's Borsh `ConfigParameters`. `tiers.85` is `collection`, whose sha `8db45a9b…53ae1c` is unchanged.
    The others are 170 `5247529b…`, 255 `e95867b5…` and 510 `ce7c4ebc…`.
  - `listing_order` (sha `6faf8ed9…c74680`) commits each listing's `graduation_sol`.
- `rook-ceremony`:
  - `check_params` accepts only a tier threshold.
  - `check_pinned_tiers` byte-compares every tier and requires exactly the pinned set.
  - `day1` builds each listing's `create_config` from its tier and writes `graduation_sol` into each plan row.
  - `preflight` / `list` check each on-chain config's threshold against the listing's tier (`check_written`, `check_config_bytes`).
  - ROOK must stay at 85.
- `ops/test/audit_a6_day1_verify.py`:
  - checks each tier's parameters, including the DBC fixed-supply validation, once;
  - checks every other listing of that tier as byte-identical;
  - checks the simulated threshold per listing.
- `ops/test/audit_a6_launch_tamper.sh` refuses five new tampered launch files:
  - a listing moved to a cheaper tier;
  - a listing moved to a dearer tier;
  - a tier with another tier's curve;
  - an unpinned tier;
  - a tier threshold changed.
- `ops/solana/site-config.mjs`:
  - each site collection row gains `graduationSol`;
  - `--check-rpc` reads each config's threshold.
- The site: `web-rook/src/rook/sol/collections.ts` (`graduationSol`, `GRADUATION_TIERS_SOL`) and `src/config/solana.ts`.

## 4. Site text still saying "85 SOL" (files this pass does not own)

These show 85 SOL / 42.5 SOL as if it held for every collection. Each should read the collection's `graduationSol` (and half of it)
from `collections.ts`. ROOK stays 85.
- `web-rook/src/ui/sol.tsx:53` "until **85 SOL**", `:55` "50 % of the raise (42.5 SOL)".
- `web-rook/src/ui/market.tsx:45` the progress bar title `% of 85 SOL`.
- `web-rook/src/rook/sol/market.ts:44` (comment: "share of the 85 SOL raised"), `:78` `BACKING_SHARE_SOL = 42.5`.
- `web-rook/src/rook/sol/launchWizard.ts:95` `MAX_FIRST_BUY = 85 SOL`, `:99` "more than the 85 SOL the curve holds" (should be the
  chosen collection's threshold).
- `web-rook/src/rook/sol/backing.ts:2` (comment: "50 % of the 85 SOL").
- `web-rook/src/rook/sol/demo.ts:49` `THRESHOLD = 85n * LAMPORTS`, `:77`, `:88` "Drifting toward 85 SOL.", `:186` `lamportsBacked: sol(42.5)`.
- `web-rook/src/rook/sol/demoMarket.ts:8` ("a full curve ends at 85 SOL").
- `web-rook/src/content/docs/01-what-is-rook.md:14,16`, `02-how-it-works.md:13-17`, `03-launching.md:18,21,29,31`,
  `05-fees.md:18,22-24`, `08-risks.md:14`, `09-faq.md:33`, `10-glossary.md:18`.
