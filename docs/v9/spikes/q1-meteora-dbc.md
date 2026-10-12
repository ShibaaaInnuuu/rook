# Spike Q1: a Rook partner config on Meteora DBC, a PDA that claims the backing share

Status: **run 2026-10-09 in LiteSVM 0.10 against the mainnet program bytes** of Meteora DBC and DAMM v2 (dumped that day) and a
DAMM v2 migration config cloned from mainnet. No mainnet transaction was sent. Question: `docs/v9/solana-migration.md` §2.1 and
P0 **[Q1]** (the table row in §7 still carries the older wording "DBC with `wX` as quote"; the operator's decision of 2026-10-09
made the quote SOL). Code: `solana/tests/q1_meteora_dbc.rs` (the spike), `solana/tests/harness/` (shared with the program tests),
`solana/tests/q1/design_curve.mjs` (the curve), `solana/programs/rook_backing` (the program, see `docs/v9/rook-backing-as-built.md`).
Same legend as the migration page: **[M]** measured here, **[W]** web, **[R]** read from Meteora's source.

Run it: `cd solana && anchor build -p rook_backing && cargo build-sbf --manifest-path programs/rook_backing/Cargo.toml --arch v2
--sbf-out-dir target/deploy-sbpfv2 && cargo test -p rook_backing --test q1_meteora_dbc -- --nocapture` (why the v2 build: §6).

## 1. Verdict

| Claim | Status | Where |
| --- | --- | --- |
| DBC accepts a **PDA (off-curve) `fee_claimer`** | **proven [M][R]** | `create_config` takes `fee_claimer` as an `UncheckedAccount` and stores the key; the config was created with `["claimer", listing]` of `rook_backing` |
| `withdraw_migration_fee` authenticates the claimer as a **`Signer`** whose key equals `config.fee_claimer` → our program must `invoke_signed` | **proven [M][R]** | `rook_backing::back` signs it with the claimer seeds; 42.5 SOL of an 85 SOL raise moved |
| The SOL lands in our PDA and goes on into `rook_core::convert` | **proven [M]** | wSOL → transient account → closed into the claimer → `convert(lamports, min_units)` with converter = payer = claimer; the claimer ends at 0 lamports |
| … against the **real** rook_core `convert` (not only the mock) | **proven [M]** | `programs/rook_backing/tests/real_rook_core.rs`: 42.5 SOL → 206,310,679 wX base units at a 200 SOL floor (+3 % conv) |
| Partner trading fees (curve) claimable by the PDA → treasury | **proven [M]** | `sweep_partner_fees` (DBC `claim_trading_fee`, PDA-signed) |
| Partner LP fees after graduation claimable by the PDA → treasury | **proven [M]** | `sweep_lp_fees` (DAMM v2 `claim_position_fee`, PDA-signed; the PDA holds the position NFT) |
| LP permanently locked | **proven [M]** | both DAMM v2 positions: 0 unlocked (creator: 1 unit of rounding), 0 vested; the creator's `remove_all_liquidity` refused |
| Migration is permissionless | **proven [M]** | `migration_damm_v2` cranked by a random funded keypair |
| Anti-sniper schedule works as configured | **proven [M]** | 1 SOL buy at launch paid 0.5 SOL fee (50 %); the 85 SOL fill 10 min later paid ≈1 % |
| GMGN / Axiom list DBC coins pre-graduation | **partly** | GMGN: the operator saw "Dynamic BC" / "Meteora" in its launchpad filter (2026-10-09). Axiom: not confirmed by a web search (§5) |

**No blocker.** One design consequence: DBC lets the partner withdraw the migration fee as soon as the curve is *complete*
(`quote_reserve ≥ threshold`), before migration **[R]**; `rook_backing` adds "and migrated" itself (`is_migrated == 1`).

## 2. Versions

| | id | release | deployed (mainnet) |
| --- | --- | --- | --- |
| DBC | `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` | 0.2.1 (`f552f20`, repo HEAD, 2026-09-09 10:10 +07) | slot 445503633, 2026-09-09 03:02 UTC; upgrade authority `JADaUV8k…CVLd` |
| DAMM v2 | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` **(id verified: on-chain, and the id in DBC's code and SDK)** | 0.2.4 (`a85c926`) | slot 445230614, 2026-09-08 03:00 UTC; same authority |
| Token Metadata | `metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s` | — | dumped (DBC's pool creation CPIs it) |
| DAMM v2 config "Customizable" | `A8gMrEPJkacWkcb3DGwtJwTe16HktSEfvwtuDh2MCtck` | — | cloned (account JSON); its `pool_creator_authority` is DBC's pool authority `FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM` |

The IDLs (`programs/rook_backing/idls/`) are those of the deployed releases: DBC's from `@meteora-ag/dynamic-bonding-curve-sdk`
1.5.13 (metadata version 0.2.1); DAMM v2's identical to the repo's `idls/damm_v2.json` (0.2.4). The release commit timestamps match the
deploy slots to the minute, so the source read is the deployed code. Both programs are upgradeable by one Meteora key: the
`PoolConfig`/`VirtualPool` offsets `rook_backing` reads are pinned by these tests (a layout change fails them).

## 3. The Rook config (values chosen; Q9 still open)

| Parameter | Value | Why / bound |
| --- | --- | --- |
| quote | wSOL, 9 dec | operator, §2.1 |
| coin | plain SPL (`token_type 0`), 6 decimals, **fixed supply 1,000,000,000** (`token_supply` pre = post = 10¹⁵) | DBC: decimals 6–9 |
| `migration_quote_threshold` | **85 SOL** | pump.fun-like; Q9 |
| curve | `sqrt_start_price 134452599321738966`; points `(268905193275807567, 215124158914782432327345953095692)`, `(79226673521066979257578248091 = MAX, 3435308751277378593870180)` | BigInt port of Meteora's own `designCurve` (`tests/q1/design_curve.mjs 1000000000 20 85 50 6`): 20 % of supply to DAMM v2; start ≈ 5.31e-8 SOL/coin (mcap ≈ 53 SOL), graduation ≈ 2.125e-7 (mcap ≈ 212.5 SOL), 799,999,984 coins sold on the curve |
| `migration_fee` | **`{ fee_percentage: 50, creator_fee_percentage: 0 }`** | ≤ 99 **[R]**; creator share must be 0 when the fee is 0 |
| `fee_claimer` = `leftover_receiver` | `rook_backing` PDA `["claimer", listing]` | any pubkey **[R]** |
| base fee | linear fee scheduler: `cliff 500_000_000` (50 %), `first_factor 60` periods, `second_factor 10` s, `third_factor 8_166_666` → 1.000004 % after 10 min; `activation_type 1` (timestamp) | min fee ≥ 0.25 %, max ≤ 99 %, all three scheduler fields non-zero or all zero **[R]**; rate-limiter mode refused (deprecated) |
| `dynamic_fee` | none | |
| `collect_fee_mode` | 0 = quote (all fees in SOL) | `rook_backing` requires it |
| `creator_trading_fee_percentage` | 0 (the partner keeps 80 %, Meteora 20 %) | operator to decide whether creators get a cut |
| `migration_option` | 1 = DAMM v2 (0 = DAMM v1 → `DeprecatedMigrationOption`) | |
| `migration_fee_option` | 6 = Customizable (DAMM config `A8gMr…`) | |
| `migrated_pool_fee` | `{ collect_fee_mode 0 (→ DAMM OnlyB = SOL), dynamic_fee 0, pool_fee_bps 100 }`, `migrated_pool_base_fee_mode 0`, market-cap scheduler all 0, `compounding_fee_bps 0` | 10–1000 bps **[R]** |
| LP | partner 50 % **permanently locked**, creator 50 % **permanently locked**, 0 claimable, no vesting | sum = 100; ≥ 10 % locked at day 1 **[R]** |
| `locked_vesting` | none (so migration needs no `create_locker`) | |
| `pool_creation_fee` | 0 | 0 or 0.001–100 SOL |
| `token_update_authority` | 1 = Immutable | mint-authority variants only for transfer-hook configs |
| `enable_first_swap_with_min_fee` | false | (lets a creator's bundled first buy skip the anti-sniper fee) |

## 4. The flow, instruction by instruction [M]

| Step | Instruction (program) | Signer(s) | Accounts (order) / args | CU |
| --- | --- | --- | --- | --- |
| config | `create_config` (DBC) | payer, **config keypair** (the config is a keypair account, not a PDA) | config, fee_claimer, leftover_receiver, quote_mint, payer, system, event_authority, program / `ConfigParameters` | 36,817 |
| register | `register_partner_config` (rook_backing) | admin | admin, settings, partner, dbc_config | 7,207 |
| launch | `initialize_virtual_pool_with_spl_token` (DBC) | creator, base-mint keypair, payer | config, pool_authority, creator, base_mint, quote_mint, pool `["pool", config, max(mints), min(mints)]`, base_vault / quote_vault `["token_vault", mint, pool]`, mint_metadata, metadata_program, payer, token_quote_program, token_program, system, event_authority, program / `{name, symbol, uri}` | 113,933–122,933 |
| vault | `open_vault` (rook_backing) | anyone | see the as-built page | 26,049–27,549 |
| buy | `swap2` (DBC) | buyer | pool_authority, config, pool, input/output token accounts, base/quote vaults, base/quote mints, payer, token programs, referral (optional), event_authority, program / `{amount_0, amount_1, swap_mode}`; **`swap_mode 1` = PartialFill stops at the threshold** (85.000000001 SOL in the reserve) | 62,052–71,010 |
| graduate | `migration_damm_v2` (DBC) | **anyone** (payer) + two fresh position-NFT mint keypairs | virtual_pool, migration_metadata (deprecated, unused), config, pool_authority, DAMM pool `["pool", damm_config, max, min]` (DAMM), first/second position NFT mint, NFT account `["position_nft_account", nft]`, position `["position", nft]`, damm_pool_authority, amm_program, base/quote mint, DAMM token vaults, DBC base/quote vaults, payer, token programs ×3, damm_event_authority, system; **remaining[0] = DAMM v2 config** | 272,748–278,748 |
| back | `back(min_units)` (rook_backing) → DBC `withdraw_migration_fee(flag = 0)` → SPL `close_account` → system `transfer` (rent back) → rook_core `convert` | anyone | 24 accounts, legacy tx | **81,362** (mock convert) / **99,404** (real convert); the DBC CPI itself 17,777 |
| curve fees | `sweep_partner_fees` → DBC `claim_trading_fee(0, u64::MAX)` | anyone | | 64,427 |
| LP fees | `sweep_lp_fees` → DAMM v2 `claim_position_fee` | anyone | | 67,950 |
| redeem | `redeem_backing(amount)` | holder | | 27,059–27,083 |

CU ranges are over runs (PDA bump searches vary with random keys); "CU" excludes the compute-budget instruction. All were sent
as legacy transactions with a 1.4 M limit; nothing needed a lookup table.

`withdraw_migration_fee` exactly **[R]**: accounts `pool_authority (FhVo3…)`, `config`, `virtual_pool (w)`, `token_quote_account (w)`
(any wSOL token account: **no owner check**, so the PDA can direct it to its own transient account), `quote_vault (w)`, `quote_mint`,
`sender (signer)`, `token_quote_program`, `event_authority`, `program`; arg `flag: u8` (0 partner, 1 creator). Checks: config's
quote mint, pool's quote vault and config, `quote_reserve ≥ migration_quote_threshold`, `sender == config.fee_claimer`, the
partner bit (0b100) of `migration_fee_withdraw_status` not yet set (second call: `MigrationFeeHasBeenWithdraw`). Amount:
`threshold × fee% / 100` rounded so the pool keeps `⌈threshold × (100 − fee%) / 100⌉`; with 50 % exactly 42.5 SOL.

`claim_trading_fee` and `claim_partner_pool_creation_fee`, `partner_withdraw_surplus` are gated the same way
(`#[access_control(is_partner_fee_claimer)]` + `Signer`), so all of them are PDA-claimable.

## 5. Money flows measured [M]

- **Graduation**: of the 85 SOL raise, 42.5 SOL (50 %) → `back` → the floor module (`convert`); 42.5 SOL + 200,000,003 coins
  → DAMM v2 (minus Meteora's protocol liquidity migration fee). The coin supply stays exactly 10¹⁵ (DBC burns the unused curve
  tokens at migration).
- **Curve fees**: a 1 SOL buy at launch paid 0.5 SOL; the fill paid 0.8535 SOL (≈1 %). Partner share 80 % = **1.0828 SOL**
  swept to the treasury; Meteora 20 %.
- **Post-graduation pool fees**: the DAMM v2 pool charges 1 % in SOL (OnlyB). A 10 SOL buy → 0.1 SOL fee: **0.04 SOL to the
  partner position** (swept by `sweep_lp_fees`), **0.04 SOL to the creator position** (the creator claims it with
  `claim_position_fee` itself), 0.02 SOL (20 %) to Meteora. Two positions exist because both partner and creator have LP;
  the larger takes the dead liquidity (on a tie, the creator's position is the first).
- **LP lock**: `permanent_lock_position` at migration on both positions; nothing unlocked or vested (1 unit of rounding on the
  first position, not removable: `AmountIsZero`). The partner position NFT is owned by the claimer PDA; `rook_backing` has no
  instruction that touches liquidity, so it can only ever claim fees.
- **Surplus**: swaps may overshoot the threshold (ExactIn); the overshoot is shared partner/creator/protocol and claimable with
  `partner_withdraw_surplus` (not wired into `rook_backing` yet: open question in the as-built page). PartialFill left 1 lamport.

**Terminals.** Jupiter integrates DBC directly (Meteora docs). Codex's launchpad API indexes `MeteoraDBC` with lifecycle states
created → completing → completed → migrated. GMGN's launchpad filter lists "Dynamic BC" / "Meteora" (operator, 2026-10-09).
Axiom: a web search found no statement either way; check its Pulse filters by hand before launch. After graduation every
terminal indexes DAMM v2. Sources: <https://docs.meteora.ag/product-overview/dynamic-bonding-curve-overview>,
<https://docs.codex.io/launchpads/meteora>, <https://github.com/MeteoraAg/dynamic-bonding-curve>.

## 6. Bounds and surprises hit

1. **The fixed supply only validates for the share the curve was designed for**: the migration base amount depends on
   `fee_percentage`, so a 1B fixed-supply curve designed for 50 % fails at 30 % or 0 % with `InvalidTokenSupply` (6020). Every
   (share, threshold) pair needs its own curve from `design_curve.mjs`; the tests use DBC's dynamic supply for other shares.
2. DBC's `withdraw_migration_fee` needs only a complete curve, not a migration (§1).
3. `create_config` needs the config **keypair** to sign: configs are created off-chain by Rook's admin, then registered.
4. Migration needs the DBC pool authority to hold lamports ("flash rent": it pays DAMM's account rent and is refunded); it does on
   mainnet. And two **signer** NFT-mint keypairs from the cranker.
5. Meteora runs its own migration keeper (SDK docs); anyone else may crank it too (proven).
6. DBC's account lists need a coin token account even when only SOL moves (`claim_trading_fee`'s `token_a_account`): the
   claimer's coin ATA, created once per coin by anyone (`claimer_coin` in `rook_backing`).
7. **LiteSVM 0.10 cannot load SBPF v3** (Anchor 1.2.1's default `--arch v3`; v3 is live on mainnet since epoch 993):
   `InvalidAccountData`. The tests run the same source built `--arch v2` into `target/deploy-sbpfv2` (rook_core's suite does the
   same). Upgrading to LiteSVM 0.18 was not done (a second runtime build did not fit the disk). **The v3 artifact itself has not
   executed these CPIs**; a `solana-test-validator` run of the v3 build is the remaining check before devnet.
8. `MIN_LOCKED_LIQUIDITY_BPS` (10 % locked at day 1), curve ≤ 16 points (20 stored), migrated pool fee 10–1000 bps, base fee
   0.25–99 %, migration fee ≤ 99 %: none bound Rook's choices.

## 7. What remains for Q1

- ~~The SBPF v3 build on `solana-test-validator` (§6.7)~~ — done 2026-10-09 against DBC / DAMM v2 cloned from devnet, up to `back`'s
  converter gate (`rook-backing-as-built.md` §6); then devnet with the real DBC (devnet has the same program ids, and the "Customizable"
  DAMM config): `docs/v9/rehearsal-sol-devnet.md`.
- Axiom's pre-graduation listing, by hand.
- Q9: the share (50 %) against pool depth: with 50 % the DAMM pool opens with 42.5 SOL; each share needs its own curve (§6.1).
