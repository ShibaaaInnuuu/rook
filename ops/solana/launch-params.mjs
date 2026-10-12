#!/usr/bin/env node
// The Solana launch parameters (decision D-c, docs/v9/solana-migration.md §2.9): rook_core holds no SOL/ETH rate, so every value the
// EVM stack denominated in ETH is converted HERE, at the live rate, into the lamports the init instructions take, and written into a
// reviewed JSON with each value's ETH origin and the rate used. Nothing is sent anywhere; this only reads public tickers.
//
//   node ops/solana/launch-params.mjs --out <file.json> [--bitcoin-network mainnet|signet|testnet4] [--cluster mainnet-beta|devnet|localnet]
//        [--stage 0] [--wake-window <seconds>] [--max-spread-bps 100] [--min-sources 2]
//   node ops/solana/launch-params.mjs --check <file.json>        (re-reads the rate; prints the drift of every value; exit 1 over 5 %)
//   node ops/solana/launch-params.mjs --meteora                  (prints the `meteora` section alone: no rate read, no file)
//
// Defaults are the production ones: --cluster mainnet-beta, --bitcoin-network mainnet. The file also carries the `meteora` section
// (Rook's DBC partner config, decision D-e: every create_config field with its value and source) and the `listing_order`
// (rook_core listing ids, which the day-1 DBC configs' fee claimers are derived from: ops/solana/ceremony-day1.sh).
//
// The rate: SOL per ETH from >= 2 public sources (Coinbase ETH-USD / SOL-USD, OKX ETH-USDT / SOL-USDT, Kraken SOLETH, KuCoin
// ETH-USDT / SOL-USDT), REFUSED if any two that answered differ by more than 1 % (--max-spread-bps); the median of those is used.
// The file is written with "review": { "reviewed_by": null } — the launch ceremony fills it in by hand after reading every line.
// Bitcoin network: mainnet unless named; the file always names it (rook_core's `init_registry(owner, bitcoin_network,
// initial_converters)`; initial_converters = [rook_backing's ["converter"] PDA] comes from rook_backing's program id, not this file).
import { writeFileSync, readFileSync, existsSync } from "node:fs";

export const LAMPORTS_PER_SOL = 1_000_000_000n;
export const WEI_PER_ETH = 1_000_000_000_000_000_000n;
/** Rates are carried as integer lamports per ETH (9 decimals of SOL): exact, no float past the parse. */
export const RATE_DECIMALS = 9;

/** The ETH-denominated values of the EVM stack the Solana programs take as parameters (wei, with where each came from). */
export const ETH_VALUES = [
  { key: "revenue_target", wei: WEI_PER_ETH, into: "init_module Params.revenue_target",
    origin: "StackChecks.defaultParams: p.revenueTarget = 1 ether (script/StackChecks.sol:184)" },
  { key: "wake_bond", wei: 2_000_000_000_000_000n, into: "init_desk DeskParams.bond",
    origin: "ETCH_WAKE_BOND = 0.002 ETH (deployments/launch/nft-pilot.json _wake; the operator, 2026-10-09)" },
  { key: "wake_min_bond", wei: 500_000_000_000_000n, into: "init_desk DeskParams.min_bond",
    origin: "WakeDesk.MIN_BOND = 0.0005 ether (src/floor/WakeDesk.sol:63)" },
  { key: "wake_max_bond", wei: WEI_PER_ETH, into: "init_desk DeskParams.max_bond",
    origin: "WakeDesk.MAX_BOND = 1 ether (src/floor/WakeDesk.sol:64)" },
];

/** Values fixed in SOL or in units, carried for the review (not conversions). */
export const FIXED = {
  frog_purchase: { min_lamports: "1000000000", default_max_lamports: "10000000000", ceiling_lamports: "250000000000",
    origin: "decision D-b (docs/v9/solana-migration.md §2.9): 1 SOL min, 10 SOL default max, 250 SOL ceiling (Relay SOL->BTC per order, spike Q5); the canister's frogsol.rs constants" },
};

// ---- Meteora DBC: Rook's partner config (decision D-e, docs/v9/solana-migration.md §2.9; values: docs/v9/spikes/q1-meteora-dbc.md §3) ----

/** Where the confirmed values come from (every leaf below cites one of these). */
const Q1 = "spike Q1 (docs/v9/spikes/q1-meteora-dbc.md §3; solana/tests/harness/mod.rs rook_config_params(50))";
const D_E = "operator decision D-e, 2026-10-09 (docs/v9/solana-migration.md §2.9)";
const leaf = (value, source) => ({ value, source });

/**
 * Every field of DBC 0.2.1's `create_config` (`ConfigParameters`, field order of the IDL in solana/programs/rook_backing/idls/
 * dynamic_bonding_curve.json), with its value and source. Integers wider than 53 bits are decimal strings. One config per
 * collection, identical except for its accounts: `fee_claimer` = `leftover_receiver` = rook_backing's PDA ["claimer", listing].
 * `ops/solana/ceremony-day1.sh` builds the create_config transactions from THIS object (the tool refuses a launch file without it).
 */
export const METEORA_CONFIG = {
  pool_fees: {
    base_fee: {
      cliff_fee_numerator: leaf("500000000", `${D_E}: anti-sniper schedule as in Q1 — 50 % fee at launch (of 1e9). ${Q1}`),
      first_factor: leaf(60, `${D_E}: 60 periods (number_of_period). ${Q1}`),
      second_factor: leaf("10", `${D_E}: one period = 10 s (period_frequency, timestamp activation). ${Q1}`),
      third_factor: leaf("8166666", `${D_E}: −0.8166666 pt per period (reduction_factor) → 1.000004 % after 10 min (DBC min 0.25 %). ${Q1}`),
      base_fee_mode: leaf(0, `${D_E}: 0 = linear fee scheduler (rate-limiter mode is deprecated in DBC). ${Q1}`),
    },
    dynamic_fee: leaf(null, `${Q1}: none`),
  },
  collect_fee_mode: leaf(0, `${Q1}: 0 = quote token (every fee in SOL); rook_backing register_partner_config requires it`),
  migration_option: leaf(1, `${D_E}: DAMM v2 graduation (1; 0 = DAMM v1 is refused by DBC 0.2.1). rook_backing requires it`),
  activation_type: leaf(1, `${Q1}: 1 = timestamp; rook_backing requires it (Vault.launched_at in unix seconds, as-built §7.2)`),
  token_type: leaf(0, `${Q1}: plain SPL coin, no Token-2022 extensions (§2.1); rook_backing requires it`),
  token_decimal: leaf(6, `${D_E}: 6 decimals (Q1's curve). DBC: 6-9`),
  partner_liquidity_percentage: leaf(0, `${D_E}: LP permanently locked — nothing claimable by the partner`),
  partner_permanent_locked_liquidity_percentage: leaf(50, `${D_E}: LP permanently locked; partner half (the claimer PDA holds the position NFT, fees only). ${Q1}`),
  creator_liquidity_percentage: leaf(0, `${D_E}: LP permanently locked — nothing claimable by the creator`),
  creator_permanent_locked_liquidity_percentage: leaf(50, `${D_E}: LP permanently locked; creator half (the creator claims its position's fees). Sum = 100. ${Q1}`),
  migration_quote_threshold: leaf("85000000000", `${D_E}: 85 SOL migration threshold (lamports)`),
  sqrt_start_price: leaf("134452599321738966", `${D_E}: Q1's curve — solana/tests/q1/design_curve.mjs 1000000000 20 85 50 6 (start ≈ 5.31e-8 SOL/coin)`),
  locked_vesting: {
    amount_per_period: leaf("0", `${Q1}: no locked vesting (migration needs no create_locker)`),
    cliff_duration_from_migration_time: leaf("0", `${Q1}: none`),
    frequency: leaf("0", `${Q1}: none`),
    number_of_period: leaf("0", `${Q1}: none`),
    cliff_unlock_amount: leaf("0", `${Q1}: none`),
  },
  migration_fee_option: leaf(6, `${Q1}: 6 = Customizable (DAMM v2 config A8gMrEPJkacWkcb3DGwtJwTe16HktSEfvwtuDh2MCtck, cloned from mainnet in Q1)`),
  token_supply: {
    pre_migration_token_supply: leaf("1000000000000000", `${D_E}: fixed supply 1,000,000,000 coins × 10^6 (Q1's curve validates only at a 50 % migration fee, spike §6.1)`),
    post_migration_token_supply: leaf("1000000000000000", `${D_E}: pre = post: nothing is burned (DBC can burn at most pre − post = 0, state/config.rs:931); the curve tail (≈ 12.78 coins) and any unsold remainder go to the leftover receiver (the claimer PDA). Supply stays 10^15`),
  },
  creator_trading_fee_percentage: leaf(0, `${D_E}: partner curve-fee split Rook 80 % / creator 0 % — of the trading fee DBC keeps its protocol share (20 %), and 0 % of the rest goes to the creator`),
  token_update_authority: leaf(1, `${Q1}: 1 = immutable metadata`),
  migration_fee: {
    fee_percentage: leaf(50, `${D_E}: 50 % of the 85 SOL threshold (42.5 SOL) to the config's fee_claimer = rook_backing's claimer PDA (DBC max 99)`),
    creator_fee_percentage: leaf(0, `${D_E}: none of the migration fee to the creator (rook_backing requires 0)`),
  },
  migrated_pool_fee: {
    collect_fee_mode: leaf(0, `${Q1}: DAMM v2 pool collects fees in SOL (OnlyB)`),
    dynamic_fee: leaf(0, `${Q1}: none`),
    pool_fee_bps: leaf(100, `${Q1}: 1 % pool fee after graduation (DBC 10-1000 bps)`),
  },
  pool_creation_fee: leaf("0", `${Q1}: no pool creation fee charged to creators`),
  partner_liquidity_vesting_info: {
    vesting_percentage: leaf(0, `${D_E}: no vesting (LP permanently locked)`), bps_per_period: leaf(0, "none"), number_of_periods: leaf(0, "none"),
    cliff_duration_from_migration_time: leaf(0, "none"), frequency: leaf(0, "none"),
  },
  creator_liquidity_vesting_info: {
    vesting_percentage: leaf(0, `${D_E}: no vesting (LP permanently locked)`), bps_per_period: leaf(0, "none"), number_of_periods: leaf(0, "none"),
    cliff_duration_from_migration_time: leaf(0, "none"), frequency: leaf(0, "none"),
  },
  migrated_pool_base_fee_mode: leaf(0, `${Q1}: flat pool fee after graduation`),
  migrated_pool_market_cap_fee_scheduler_params: {
    number_of_period: leaf(0, `${Q1}: no market-cap fee scheduler`), sqrt_price_step_bps: leaf(0, "none"),
    scheduler_expiration_duration: leaf(0, "none"), reduction_factor: leaf("0", "none"),
  },
  enable_first_swap_with_min_fee: leaf(false, `${D_E}: false — a creator's bundled first buy pays the anti-sniper fee like everyone`),
  compounding_fee_bps: leaf(0, `${Q1}: none`),
  padding: leaf([0, 0], "DBC 0.2.1 layout"),
  curve: leaf([
    { sqrt_price: "268905193275807567", liquidity: "215124158914782432327345953095692" },
    { sqrt_price: "79226673521066979257578248091", liquidity: "3435308751277378593870180" },
  ], `${D_E}: Q1's curve (design_curve.mjs 1000000000 20 85 50 6): 20 % of supply to DAMM v2, graduation ≈ 2.125e-7 SOL/coin (mcap ≈ 212.5 SOL), 799,999,984 coins sold on the curve`),
};

// ---- graduation tiers (operator decision 2026-10-10, docs/v9/graduation-tiers.md) --------------------------------------------------
const D_T = "operator decision 2026-10-10 (docs/v9/graduation-tiers.md): graduation size per collection in tiers, so that every graduation buys at least one NFT at once";
/**
 * The collection configs' graduation tiers: every field of METEORA_CONFIG except migration_quote_threshold, sqrt_start_price and the
 * curve, which `design_curve.mjs 1000000000 20 <tier> 50 6` gives (a fixed-supply DBC curve validates only for the threshold and fee it
 * was designed for: spike Q1 §6.1, A3). 85 is METEORA_CONFIG itself. The anti-sniper schedule, the 50 % migration fee, the supply and
 * the locked LP are the same in every tier. Each tier's Borsh encoding is pinned in day1-pins.json (`tiers`).
 */
export const GRADUATION_TIERS = {
  85: { sqrt_start_price: "134452599321738966",
    curve: [{ sqrt_price: "268905193275807567", liquidity: "215124158914782432327345953095692" },
      { sqrt_price: "79226673521066979257578248091", liquidity: "3435308751277378593870180" }],
    start_mcap_sol: "53.125", graduation_mcap_sol: "212.5" },
  170: { sqrt_start_price: "190144688344867500",
    curve: [{ sqrt_price: "380289371323205464", liquidity: "304231501351788061953802646828752" },
      { sqrt_price: "79226673521066979257578248091", liquidity: "3434578513360188981331421" }],
    start_mcap_sol: "106.250", graduation_mcap_sol: "425.0" },
  255: { sqrt_start_price: "232878731269108314",
    curve: [{ sqrt_price: "465757457172827529", liquidity: "372605970030573352902104940717502" },
      { sqrt_price: "79226673521066979257578248091", liquidity: "3433848552104604036791622" }],
    start_mcap_sol: "159.375", graduation_mcap_sol: "637.5" },
  510: { sqrt_start_price: "329340259036093404",
    curve: [{ sqrt_price: "658680512710218671", liquidity: "526944414457749482192302864017669" },
      { sqrt_price: "79226673521066979257578248091", liquidity: "3431658944516985920937941" }],
    start_mcap_sol: "318.750", graduation_mcap_sol: "1275.0" },
};
export const TIER_SOL = Object.keys(GRADUATION_TIERS).map(Number).sort((a, b) => a - b);
/** rook_core's convert keeps reserve_bps 25 % as SOL and charges conv_bps 3 %: the NFT buying power of one graduation (in SOL). */
export const buyingPower = (tierSol) => (tierSol * 0.5 * 0.75) / 1.03;
/** The headroom the operator asked for: a tier's buying power covers 1.33 x the collection's floor. */
export const TIER_HEADROOM = 1.33;
/** The smallest tier whose buying power is >= 1.33 x `floorSol`, or null when even the largest does not reach it (pure). */
export function tierFor(floorSol) {
  return TIER_SOL.find((t) => buyingPower(t) >= TIER_HEADROOM * floorSol) ?? null;
}

/** METEORA_CONFIG's tier-dependent leaves for `tierSol` (85: METEORA_CONFIG's own values). */
export function tierOverrides(tierSol) {
  const t = GRADUATION_TIERS[tierSol];
  if (!t) throw new Error(`no graduation tier of ${tierSol} SOL (tiers: ${TIER_SOL.join(", ")})`);
  const how = `design_curve.mjs 1000000000 20 ${tierSol} 50 6`;
  return {
    migration_quote_threshold: leaf(String(BigInt(tierSol) * LAMPORTS_PER_SOL), `${D_T}: ${tierSol} SOL migration threshold (lamports)`),
    sqrt_start_price: leaf(t.sqrt_start_price, `${D_T}: ${how} (start mcap ≈ ${t.start_mcap_sol} SOL)`),
    curve: leaf(t.curve, `${D_T}: ${how} — 20 % of supply to DAMM v2, graduation mcap ≈ ${t.graduation_mcap_sol} SOL; migration fee ${tierSol / 2} SOL to the claimer`),
  };
}

/** deployments/launch/graduation-tiers.json: each listing's tier (by slug). */
export function graduationTiers(path = new URL("../../deployments/launch/graduation-tiers.json", import.meta.url)) {
  return JSON.parse(readFileSync(path, "utf8"));
}

const D_G = "operator decision D-g, 2026-10-09 (docs/v9/solana-migration.md §2.9: the community-funded deploy, the genesis ROOK coin)";
/** The operator's fund wallet (decision D-g): a plain system account, its key held only by the operator. Overridable everywhere. */
export const FUND_WALLET = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy";

/**
 * The genesis ROOK coin's DBC config (decision D-g): NOT a collection config and no backing. Every field is the collection config's
 * (METEORA_CONFIG) except these overrides. Same family: 1B supply, 6 decimals, 85 SOL threshold, the same anti-sniper schedule and
 * locked LP. The curve differs only because the migration fee is 0: a fixed-supply DBC curve validates only for the fee it was designed
 * for (spike Q1 §6.1), so design_curve.mjs is rerun with fee 0 — all 85 SOL go into the DAMM v2 pool (graduation mcap ≈ 425 SOL).
 */
export const GENESIS_ROOK_OVERRIDES = {
  migration_fee: {
    fee_percentage: leaf(0, `${D_G}: no backing on ROOK — no migration fee (DBC allows 0 when the creator share is 0 too: process_create_config.rs MigrationFee::validate)`),
    creator_fee_percentage: leaf(0, `${D_G}: must be 0 when fee_percentage is 0 (DBC)`),
  },
  creator_trading_fee_percentage: leaf(100, `${D_G}: the MAXIMUM DBC 0.2.1 allows (≤ 100, process_create_config.rs:389): the creator (= the fund wallet) receives all of the trading fee after Meteora's 20 % protocol share, i.e. 80 % of every curve fee; trading ROOK funds the deploy`),
  sqrt_start_price: leaf("95072344172433750", `${D_G}: solana/tests/q1/design_curve.mjs 1000000000 20 85 0 6 (start ≈ 2.66e-8 SOL/coin, mcap ≈ 26.6 SOL)`),
  curve: leaf([
    { sqrt_price: "380289371323205464", liquidity: "101410499496546307411360885487802" },
    { sqrt_price: "79226673521066979257578248091", liquidity: "3434578513360188981331421" },
  ], `${D_G}: design_curve.mjs 1000000000 20 85 0 6 — 20 % of supply to DAMM v2 with the whole 85 SOL, graduation ≈ 4.25e-7 SOL/coin (mcap ≈ 425 SOL), ≈ 800M coins sold on the curve`),
};

export function genesisRookSection(fundWallet = FUND_WALLET) {
  return {
    _: "The genesis ROOK coin (decision D-g): a separate DBC config whose partner fee claimer, leftover receiver and pool creator are the fund wallet; no backing (migration fee 0). config_parameters = meteora.config_parameters with these overrides.",
    fund_wallet: leaf(fundWallet, `${D_G}: the operator's own wallet (key never generated or held by Rook's tooling); --fund-wallet overrides`),
    accounts: {
      fee_claimer: leaf("the fund wallet", `${D_G}: partner fee claimer (receives the partner's 0 % of curve fees and holds the partner LP position after graduation)`),
      leftover_receiver: leaf("the fund wallet", `${D_G}`),
      creator: leaf("the fund wallet (the launch is signed by the ceremony payer and handed over by transfer_pool_creator in the same transaction)", `${D_G}; DBC transfer_pool_creator is allowed before the curve completes`),
    },
    metadata: { name: leaf("ROOK", `${D_G}; operator decision 2026-10-10: the final name (--rook-name overrides)`),
      symbol: leaf("ROOK", `${D_G}; operator decision 2026-10-10: the final symbol (--rook-symbol overrides)`),
      uri: leaf("https://res.cloudinary.com/ddwebzwrk/raw/upload/v1791607907/rook-token/rook.json", `${D_G}; operator decision 2026-10-10: the hosted metadata JSON (name ROOK, symbol ROOK, image …/rook-token/rook.png on the same Cloudinary account; 80 bytes ≤ Metaplex's 200). The mint's metadata is IMMUTABLE (token_update_authority 1): the two Cloudinary files must never be deleted or renamed. --rook-uri overrides (rook-ceremony refuses an empty or non-permanent uri with --send)`) },
    overrides: GENESIS_ROOK_OVERRIDES,
  };
}

/** The Meteora section of the launch file: the programs, the accounts of every create_config, the parameters. */
export function meteoraSection() {
  return {
    _: "Rook's Meteora DBC partner config, one per collection (decision D-e). Confirmed by the operator 2026-10-09. The parameters are identical for every collection except the graduation tier (threshold, start price, curve: `tiers`, operator decision 2026-10-10); fee_claimer / leftover_receiver differ (rook_backing's PDA [\"claimer\", listing]).",
    dbc_program: leaf("dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN", `${Q1} §2: DBC 0.2.1 (f552f20, deployed mainnet slot 445503633); rook_backing reads its accounts by offset — re-check on every Meteora release`),
    damm_v2_program: leaf("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG", `${Q1} §2: DAMM v2 0.2.4`),
    instruction: "create_config (DBC); signers: payer, the config keypair (a keypair account of 1,048 bytes, not a PDA)",
    accounts: {
      config: leaf("a fresh keypair per collection (generated by ceremony-day1.sh, kept with the program keypairs)", `${Q1} §6.3`),
      fee_claimer: leaf("rook_backing PDA [\"claimer\", listing]; listing = rook_core PDA [\"listing\", u32 LE listing id] — derived from the RESERVED mainnet program ids", `${D_E}; rook-backing-as-built.md §2`),
      leftover_receiver: leaf("= fee_claimer", `${Q1} §3`),
      quote_mint: leaf("So11111111111111111111111111111111111111112", `${D_E}: SOL (wSOL)`),
    },
    config_parameters: METEORA_CONFIG,
    tiers: {
      _: `${D_T}. A listing's config is config_parameters with tiers[<its listing_order graduation_sol>] merged in (leaf by leaf); "85" is config_parameters itself.`,
      ...Object.fromEntries(TIER_SOL.map((t) => [String(t), tierOverrides(t)])),
    },
    genesis_rook: genesisRookSection(),
  };
}

/**
 * The listing order: rook_core assigns listing ids in `list_collection` order (Registry.listing_count, from 0), and each DBC
 * config's fee claimer is derived from its listing's PDA. The ceremony MUST list in exactly this order (the day-1 configs pin it).
 * Order: the eleven Ethereum collections of deployments/launch/nft-pilot.json in file order, then Bitcoin Frogs (`bitcoin`, id 11),
 * then the further Ordinals collections of `inscriptions[]` in file order (ids 12+, docs/v9/ordinals-collections.md). Append only:
 * an id once used for a day-1 config is never reused or moved.
 */
export function listingOrder(pilotPath = new URL("../../deployments/launch/nft-pilot.json", import.meta.url),
  tiersPath = new URL("../../deployments/launch/graduation-tiers.json", import.meta.url)) {
  const p = JSON.parse(readFileSync(pilotPath, "utf8"));
  const tiers = graduationTiers(tiersPath).collections ?? {};
  const eth = p.collections.map((c) => ({ slug: c.slug, kind: "EthNft", name: c.name, symbol: c.symbol, chain_id: c.chainId, collection: c.collection }));
  const ins = (b) => ({ slug: b.slug, kind: "Inscriptions", name: b.name, symbol: b.symbol, members_root: b.membersRoot });
  const btc = [...(p.bitcoin ? [ins(p.bitcoin)] : []), ...(p.inscriptions ?? []).map(ins)];
  // docs/v9/graduation-tiers.md: each listing's graduation tier (its day-1 config's threshold), pinned with the order
  const tierOf = (slug) => {
    const t = tiers[slug]?.tier;
    if (!GRADUATION_TIERS[t]) throw new Error(`graduation-tiers.json: ${slug} has no tier (one of ${TIER_SOL.join(", ")})`);
    return t;
  };
  return [...eth, ...btc].map((c, i) => ({ listing_id: i, ...c, graduation_sol: tierOf(c.slug) }));
}

/** StackChecks.stageOf(stage).itemsPerDay (script/StackChecks.sol:53-58). */
export const ITEMS_PER_DAY = [3, 6, 12, 24];
const ONE = 1_000_000_000n; // one wX unit (9 decimals): rook_core units.rs ONE

/** StackChecks.defaultParams(1e18, stage) in rook_core's `Params` field names; `revenue_target` in lamports (a string). */
export function defaultParams(revenueTargetLamports, stage = 0) {
  if (!(stage in ITEMS_PER_DAY)) throw new Error(`stage ${stage}: 0 to 3`);
  return {
    base_spread_bps: 15, vol_weight_bps: 10_000, stale_spread_bps: 100, stress_spread_bps: 300, max_spread_bps: 1_000, max_move_bps: 500,
    conv_bps: 300, buffer_bps: 500, low_confidence_spread_bps: 100, full_confidence_bps: 8_000, min_confidence_bps: 3_000,
    max_items_per_day: ITEMS_PER_DAY[stage], reserve_bps: 2_500, fill_band_bps: 300,
    mint_per_hour: String(5n * ONE), burn_per_hour: String(5n * ONE), max_public_eth_backed: String(10n * ONE), max_eth_backed: String(40n * ONE),
    revenue_target: String(revenueTargetLamports),
  };
}

/** A decimal string → integer scaled by 10^d (rounded half up). No float. */
export function scaled(s, d = RATE_DECIMALS) {
  const m = String(s).trim().match(/^(\d+)(?:\.(\d+))?$/);
  if (!m) throw new Error(`not a positive decimal: ${s}`);
  const frac = (m[2] ?? "").padEnd(d + 1, "0");
  let v = BigInt(m[1]) * 10n ** BigInt(d) + BigInt(frac.slice(0, d) || "0");
  if (Number(frac[d]) >= 5) v += 1n;
  return v;
}

/** a / b as lamports per ETH, from two decimal strings (ETH-USD over SOL-USD), half up. */
export function ratio(ethUsd, solUsd) {
  const e = scaled(ethUsd, 18), s = scaled(solUsd, 18);
  if (s === 0n) throw new Error("a zero SOL price");
  return (e * 10n ** BigInt(RATE_DECIMALS) * 2n + s) / (2n * s);
}

/** 1 / x (x: ETH per SOL, a decimal string) as lamports per ETH, half up. */
export function inverse(ethPerSol) {
  const x = scaled(ethPerSol, 18);
  if (x === 0n) throw new Error("a zero ETH-per-SOL price");
  return (10n ** 18n * 10n ** BigInt(RATE_DECIMALS) * 2n + x) / (2n * x);
}

export const fmt = (lamports) => {
  const l = BigInt(lamports), w = l / LAMPORTS_PER_SOL, f = (l % LAMPORTS_PER_SOL).toString().padStart(9, "0").replace(/0+$/, "");
  return f ? `${w}.${f}` : `${w}`;
};

/**
 * Pure. `quotes`: [{ name, lamports_per_eth (bigint) } | { name, error }]. Refuses with fewer than `minSources` answers or a spread
 * (max/min − 1) over `maxSpreadBps`; returns the median (the lower of the middle two for an even count, the conservative side for a
 * revenue target: fewer lamports per ETH) and the spread.
 */
export function combine(quotes, { maxSpreadBps = 100, minSources = 2 } = {}) {
  const ok = quotes.filter((q) => typeof q.lamports_per_eth === "bigint" && q.lamports_per_eth > 0n);
  if (ok.length < minSources) {
    throw new Error(`refused: ${ok.length} rate source(s) answered, ${minSources} needed (${quotes.map((q) => `${q.name}: ${q.error ?? "ok"}`).join("; ")})`);
  }
  const v = ok.map((q) => q.lamports_per_eth).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const lo = v[0], hi = v[v.length - 1];
  const spreadBps = Number(((hi - lo) * 10_000n + lo - 1n) / lo); // rounded up
  if (spreadBps > maxSpreadBps) {
    throw new Error(`refused: the sources differ by ${spreadBps} bps (> ${maxSpreadBps}): ${ok.map((q) => `${q.name} ${fmt(q.lamports_per_eth)}`).join(", ")} SOL per ETH`);
  }
  const median = v[Math.floor((v.length - 1) / 2)];
  return { lamports_per_eth: median, spread_bps: spreadBps, used: ok.length };
}

/** wei × (lamports per ETH) / 1e18, half up. */
export function convert(wei, lamportsPerEth) {
  return (BigInt(wei) * BigInt(lamportsPerEth) * 2n + WEI_PER_ETH) / (2n * WEI_PER_ETH);
}

export const NETWORKS = { mainnet: "Mainnet", signet: "Signet", testnet4: "Testnet4" };
export const CLUSTERS = ["mainnet-beta", "devnet", "localnet"];

/** Pure: the launch file from a combined rate and the quotes behind it. */
export function build({ rate, quotes, at, bitcoinNetwork = "mainnet", cluster = "mainnet-beta", stage = 0, wakeWindow = 604_800, maxSpreadBps = 100, listings = [] }) {
  const net = NETWORKS[bitcoinNetwork];
  if (!net) throw new Error(`refused: bitcoin network ${bitcoinNetwork}; one of ${Object.keys(NETWORKS).join(", ")}`);
  if (!CLUSTERS.includes(cluster)) throw new Error(`refused: cluster ${cluster}; one of ${CLUSTERS.join(", ")}`);
  if (cluster === "mainnet-beta" && net !== "Mainnet") throw new Error("refused: a mainnet-beta launch file names Bitcoin mainnet");
  if (!(wakeWindow >= 86_400 && wakeWindow <= 30 * 86_400)) throw new Error("refused: the wake window is 1 to 30 days (rook_core MIN_WINDOW..MAX_WINDOW)");
  /** @type {Record<string, any>} */
  const values = {};
  for (const v of ETH_VALUES) {
    const l = convert(v.wei, rate.lamports_per_eth);
    values[v.key] = { lamports: String(l), sol: fmt(l), eth: fmt(v.wei / 1_000_000_000n), into: v.into, eth_origin: v.origin };
  }
  const bond = BigInt(values.wake_bond.lamports), lo = BigInt(values.wake_min_bond.lamports), hi = BigInt(values.wake_max_bond.lamports);
  if (!(lo > 0n && lo <= bond && bond <= hi)) throw new Error("refused: the converted bond is outside its converted bounds");
  return {
    _: "Rook on Solana: launch parameters (decision D-c, docs/v9/solana-migration.md §2.9). Every ETH-denominated value of the EVM stack, converted at the SOL/ETH rate below; the programs hold no rate. Review every line before the ceremony and fill in `review`.",
    generator: "ops/solana/launch-params.mjs",
    generated_at: at,
    cluster,
    bitcoin_network: net,
    rate: {
      sol_per_eth: fmt(rate.lamports_per_eth), lamports_per_eth: String(rate.lamports_per_eth), method: "median of the sources that answered",
      spread_bps: rate.spread_bps, max_spread_bps: maxSpreadBps,
      sources: quotes.map((q) => (q.error ? { name: q.name, error: q.error } : { name: q.name, sol_per_eth: fmt(q.lamports_per_eth), legs: q.legs })),
    },
    values,
    fixed: { ...FIXED, wake_window_s: { value: wakeWindow, origin: "ETCH_WAKE_WINDOW = 604800 (7 days; docs/v9/ref-robinhood-contracts.md)" } },
    init: {
      init_registry: { bitcoin_network: net },
      init_module: { stage, params: defaultParams(values.revenue_target.lamports, stage), origin: "StackChecks.defaultParams(1e18, stage), units at 9 decimals (rook_core params.rs)" },
      init_desk: { bond: values.wake_bond.lamports, min_bond: values.wake_min_bond.lamports, max_bond: values.wake_max_bond.lamports, window: wakeWindow },
    },
    listing_order: {
      _: "rook_core listing ids in list_collection order. The day-1 DBC configs (ceremony-day1.sh) derive each collection's fee claimer from its listing id: list in EXACTLY this order at the ceremony, with nothing listed before.",
      listings,
    },
    meteora: meteoraSection(),
    review: { reviewed_by: null, reviewed_at: null, note: null },
  };
}

// ---- the sources (public tickers, no key) ----------------------------------------------------------------------------------------

async function getJson(url, timeoutMs = 10_000) {
  const r = await fetch(url, { signal: AbortSignal.timeout(timeoutMs), headers: { accept: "application/json" } });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

export const SOURCES = [
  { name: "coinbase", async read() {
    const [e, s] = await Promise.all(["ETH-USD", "SOL-USD"].map((p) => getJson(`https://api.exchange.coinbase.com/products/${p}/ticker`)));
    return { lamports_per_eth: ratio(e.price, s.price), legs: { "ETH-USD": e.price, "SOL-USD": s.price } };
  } },
  { name: "okx", async read() {
    const [e, s] = await Promise.all(["ETH-USDT", "SOL-USDT"].map((p) => getJson(`https://www.okx.com/api/v5/market/ticker?instId=${p}`)));
    return { lamports_per_eth: ratio(e.data[0].last, s.data[0].last), legs: { "ETH-USDT": e.data[0].last, "SOL-USDT": s.data[0].last } };
  } },
  { name: "kraken", async read() {
    const j = await getJson("https://api.kraken.com/0/public/Ticker?pair=SOLETH");
    if (j.error?.length) throw new Error(j.error.join(", "));
    const last = Object.values(j.result)[0].c[0];
    return { lamports_per_eth: inverse(last), legs: { "SOL-ETH": last } };
  } },
  { name: "kucoin", async read() {
    const [e, s] = await Promise.all(["ETH-USDT", "SOL-USDT"].map((p) => getJson(`https://api.kucoin.com/api/v1/market/orderbook/level1?symbol=${p}`)));
    return { lamports_per_eth: ratio(e.data.price, s.data.price), legs: { "ETH-USDT": e.data.price, "SOL-USDT": s.data.price } };
  } },
];

export async function readQuotes(sources = SOURCES) {
  return Promise.all(sources.map(async (s) => {
    try { return { name: s.name, ...(await s.read()) }; } catch (err) { return { name: s.name, error: String(err.message ?? err).slice(0, 200) }; }
  }));
}

// ---- the command line --------------------------------------------------------------------------------------------------------------

function args(argv) {
  const o = {};
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i];
    if (!k.startsWith("--")) throw new Error(`unknown argument ${k}`);
    o[k.slice(2)] = argv[i + 1] && !argv[i + 1].startsWith("--") ? argv[++i] : true;
  }
  return o;
}

async function main() {
  const a = args(process.argv.slice(2));
  if (a.meteora) {
    console.log(JSON.stringify({ listing_order: listingOrder(), meteora: meteoraSection() }, null, 2));
    return;
  }
  const maxSpreadBps = Number(a["max-spread-bps"] ?? 100), minSources = Number(a["min-sources"] ?? 2);
  // audit A6-14: `--max-spread-bps abc` was NaN, and `x > NaN` is false: the spread refusal silently disabled
  if (!Number.isInteger(maxSpreadBps) || maxSpreadBps < 0 || maxSpreadBps > 500) throw new Error("refused: --max-spread-bps is an integer 0..500 (default 100 = 1 %)");
  if (!Number.isInteger(minSources)) throw new Error("refused: --min-sources is an integer");
  if (minSources < 2) throw new Error("refused: two sources at least (one is not a check)");
  const quotes = await readQuotes();
  for (const q of quotes) console.error(q.error ? `  ${q.name}: no answer (${q.error})` : `  ${q.name}: ${fmt(q.lamports_per_eth)} SOL per ETH`);
  const rate = combine(quotes, { maxSpreadBps, minSources });
  if (a.check) {
    const f = JSON.parse(readFileSync(a.check, "utf8"));
    const was = BigInt(f.rate.lamports_per_eth), now = rate.lamports_per_eth;
    const drift = Number(((now > was ? now - was : was - now) * 10_000n) / was);
    console.log(`${a.check}: written at ${fmt(was)} SOL per ETH (${f.generated_at}); now ${fmt(now)} (${drift} bps ${now > was ? "more" : "less"} SOL per ETH)`);
    for (const v of ETH_VALUES) console.log(`  ${v.key}: file ${fmt(f.values[v.key].lamports)} SOL, now ${fmt(convert(v.wei, now))} SOL`);
    process.exit(drift > 500 ? 1 : 0);
  }
  if (!a.out) throw new Error("refused: --out <file.json> (or --check <file.json>)");
  if (existsSync(a.out) && !a.force) throw new Error(`refused: ${a.out} exists (a reviewed launch file is never overwritten; --force for a scratch file)`);
  const file = build({ rate, quotes, at: new Date().toISOString(), bitcoinNetwork: a["bitcoin-network"] ?? "mainnet", cluster: a.cluster ?? "mainnet-beta",
    stage: Number(a.stage ?? 0), wakeWindow: Number(a["wake-window"] ?? 604_800), maxSpreadBps, listings: listingOrder() });
  writeFileSync(a.out, JSON.stringify(file, null, 2) + "\n");
  console.log(`${a.out}: ${file.rate.sol_per_eth} SOL per ETH (${rate.used} sources, spread ${rate.spread_bps} bps); revenue_target ${file.values.revenue_target.sol} SOL, ` +
    `wake bond ${file.values.wake_bond.sol} SOL in [${file.values.wake_min_bond.sol}, ${file.values.wake_max_bond.sol}]; bitcoin ${file.bitcoin_network}; cluster ${file.cluster}`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((err) => { console.error(String(err.message ?? err)); process.exit(2); });
}
