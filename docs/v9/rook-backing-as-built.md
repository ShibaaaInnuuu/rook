# rook_backing — as built (2026-10-09; audit fixes 2026-10-10, §8)

The Meteora DBC partner integration and the per-coin backing vaults of `docs/v9/solana-migration.md` §2.1 / §3. Code:
`solana/programs/rook_backing` (Anchor 1.2.1, program id `8VqSJrKwQjFAXY5RXFxb69wYXmcCzTTuct4chu7FxbGf`, a local key). Evidence:
`docs/v9/spikes/q1-meteora-dbc.md`. Not deployed anywhere.

## 1. What it does

Rook is the DBC **partner** of every coin. For each collection Rook creates DBC configs (off-chain, `create_config`) whose
`fee_claimer` (and `leftover_receiver`) is this program's PDA `["claimer", listing]`, then registers them here. At graduation
DBC keeps `migration_fee.fee_percentage` (50 % to start) of the raise for the claimer; anyone cranks `back(coin)`: the claimer
withdraws it, unwraps it and converts it through `rook_core::convert` (signed by this program's ONE `["converter"]` PDA, paid by
the claimer) into wX held by the coin's vault. Any holder burns coins for
their pro-rata wX (`redeem_backing`). Partner trading fees (curve and locked DAMM v2 position) are swept to rook_core's treasury.

## 2. Accounts

| Account | Seeds | Fields |
| --- | --- | --- |
| `Settings` | `["settings"]` | `registry` (= rook_core PDA `["registry"]`, derived at init; its `owner` is the admin, §7.3), `treasury` (= rook_core PDA `["treasury"]`, derived at init), `bump`, `converter` + `converter_bump` (this program's `["converter"]` PDA, derived at init, §7.4) |
| `Partner` | `["partner", listing]` | `listing` (rook_core Listing = the collection), the pinned `convert` accounts `registry`, `module`, `module_vault`, `wrapper`, `wx_mint`; `claimer` + `claimer_bump`; `configs: Vec<Pubkey>` (≤ 8 DBC configs); totals `vaults`, `lamports_backed`, `lamports_swept` |
| converter | `["converter"]` | no data, never funded; program-wide; the one key in rook_core's `Registry.converters` for rook_backing (set at `init_registry`); signs `convert` only |
| claimer | `["claimer", listing]` | no data, system-owned; DBC `fee_claimer` and `leftover_receiver`, holder of the partner DAMM v2 position NFT, the `payer` of rook_core `convert`; ends every instruction with the lamports it started with (0 unless someone sent it lamports, which then stay there: no instruction spends them, A3-08) |
| claimer wSOL | `["claimer_wsol", listing]` | SPL token account (wSOL, authority = claimer), created and closed inside `back` / `sweep_*` |
| `Vault` | `["vault", coin_mint]` | `partner`, `coin_mint`, `dbc_pool`, `dbc_config`, `creator` (the holder of DBC's transferable creator role when the coin was backed, written by `back`; not necessarily who launched it, §8.4), `wx_account`, `graduated`, `lamports_backed`, `wx_units` (credited by `back`), `coin_supply_at_backing`, `backed_at`, `launched_at` (the DBC pool's creation time, §7.2), `coins_burned`, `wx_paid`, bumps (259 bytes; layout shared with rook_core: `rook_layouts::backing_vault`) |
| vault wX | `["vault_wx", coin_mint]` | token account of `wx_mint`, authority = the `Vault` |

## 3. Instructions

| Instruction | Signer | Does / refuses |
| --- | --- | --- |
| `init_backing` | the program's **upgrade authority** (ProgramData checked) | creates `Settings` once (pins rook_core's registry and treasury PDAs) |
| `register_partner(PartnerParams { listing, registry, module, module_vault, wrapper, wx_mint })` (accounts: admin, settings, registry, partner, listing, module, module_vault, wrapper) | admin = rook_core `Registry.owner` | creates `Partner`, derives the claimer; `params.registry` must be the pinned registry (`WrongRegistry`); the listing, module, module vault and wrapper must be rook_core's (owner, discriminator) and linked as rook_core wrote them: `module.listing == listing`, `module.vault`, `module.wrapper`, `wrapper.listing`, `wrapper.wx_mint` (`WrongCoreAccount`, §8.3); emits `PartnerRegistered { listing, listing_id, … }` |
| `register_partner_config` (accounts: admin, settings, registry, partner, dbc_config) | admin = rook_core `Registry.owner` | reads the DBC `PoolConfig` (owner DBC, 1048 bytes, discriminator) and refuses unless `version` = 0 (DBC 0.2.1, `WrongConfigVersion`), `fee_claimer == claimer`, `leftover_receiver == claimer` (`WrongLeftoverReceiver`), quote = wSOL, migration = DAMM v2, SPL coin, `collect_fee_mode` = quote, **timestamp activation** (`WrongActivationType`), `migration_fee_percentage > 0`, creator migration share 0, partner and creator LP permanently locked (no unlocked share, no vesting: `UnlockedLiquidity`), the migrated pool collecting SOL only without compounding (`WrongMigratedFeeMode`), no pool creation fee (`PoolCreationFee`) (§8.2); no duplicates, ≤ 8 |
| `open_vault` | anyone (pays rent ≈ 0.0047 SOL) | reads the DBC `VirtualPool` (owner DBC, 424 bytes, discriminator; TransferHookPool refused): `pool.config ∈ partner.configs` (`NotRookPool`), `pool.base_mint == coin` (`WrongCoinMint`); records `launched_at = pool.activation_point` (`NoLaunchTime` if 0); creates `Vault` + vault wX |
| `back(min_units)` | anyone | refuses if `vault.graduated` (`AlreadyBacked`) or `pool.is_migrated != 1` (`NotMigrated`); CPI DBC `withdraw_migration_fee(0)` signed by the claimer → claimer wSOL; close into the claimer; rent back to the cranker; CPI rook_core `convert(fee, max(min_units, floor))` where `floor` = 90 % of what `fee` buys at the module's accepted floor plus its conversion allowance (read from rook_core's `Module`; `NoFloor` if never posted; §8.1) (converter = the `["converter"]` PDA, checked against `Settings.converter`; payer = the claimer; dest = vault wX; registry/listing/module/module_vault/wrapper/wx_mint must equal the pinned ones); requires `units ≥` that bound; records units, supply, time and `creator` = DBC's `pool.creator` now (§8.4) |
| `redeem_backing(amount)` | holder | refuses before backing (`NotBacked`), `amount == 0` (`ZeroAmount`), another mint (`WrongCoinMint`), a payout of 0 (`ZeroPayout`: never burns for nothing); burns `amount` from the holder's account (holder signs), pays `⌊vault_wx_balance × amount / live_mint_supply⌋` to any wX account |
| `sweep_partner_fees` | anyone | `treasury == settings.treasury` (`WrongTreasury`); CPI DBC `claim_trading_fee(0, u64::MAX)` signed by the claimer; unwrap; lamports → treasury by plain transfer (rook_core counts plain transfers as income), rent → cranker |
| `sweep_lp_fees` | anyone | same, with DAMM v2 `claim_position_fee` on a position whose NFT the claimer holds (`WrongPositionOwner` otherwise) |

Redemption rounding: always down, so a redemption never over-pays and the backing per remaining coin never falls (property test
`payout_never_overpays`). Burning the whole live supply pays the whole balance (the last redeemer takes the dust); in practice the
coins locked in the DAMM v2 pool are never burned, so their pro-rata share and the dust stay in the vault and back the coins still
out (an arbitrageur who buys them from the pool can redeem them).

## 4. CU and sizes (LiteSVM, SBPF v2 build) [M]

| | CU | accounts |
| --- | --- | --- |
| `back` (mock convert) | 82,756 (+1,500 on some runs: PDA bump search) | 26 |
| `back` (real rook_core convert, fresh floor) | 100,901 | 26 |
| — of which DBC `withdraw_migration_fee` | 17,777 | 10 |
| `sweep_partner_fees` | 64,431 | 20 |
| `sweep_lp_fees` | 67,948 | 21 |
| `redeem_backing` | ≈ 27,100 | 9 |
| `open_vault` | ≈ 24,600–29,100 | 9 |
| `register_partner_config` | 7,606 | 5 |
| `register_partner` | ≈ 11,900–16,400 | 5 |
| `init_backing` | 22,864 | 5 |

(2026-10-09 fixes: `register_partner*` +1 account and ≈ +400 CU for the registry read; `init_backing` +3,000 for the registry
PDA derivation; `back` +132 through the real `convert` because rook_core's `Registry` grew 40 bytes. The ranges are PDA
bump-search variance between runs. §7.4: `back` +1,365 (real) / +1,321 (mock) and +2 accounts (`settings`, `converter`): the
`Settings` read and the runtime's derivation of the second signer PDA in `invoke_signed`; no `find_program_address`, the bump
comes from `Settings`, and `settings` is not re-derived by seeds.)
| (DBC `migration_damm_v2`, for reference) | ≈ 273,000–279,000 | 26 |

All fit legacy transactions without lookup tables. CPI depth: rook_backing → DBC → SPL Token (2 of 4); rook_backing → rook_core →
SPL Token (2). The program binary is 512,880 bytes (v3) / 538,272 (v2).

## 5. Tests

`cd solana && anchor build -p rook_backing && cargo build-sbf --manifest-path programs/rook_backing/Cargo.toml --arch v2
--sbf-out-dir target/deploy-sbpfv2 && cargo test -p rook_backing`: **11 tests, green** (2026-10-09, after §7.4) — the unit test
`state::tests::vault_matches_the_shared_layout` (§7.2), `tests/backing.rs` (9: upgrade-authority gating; the registry owner as
admin (§7.3); config checks (incl. slot activation refused); `open_vault` refuses a non-Rook pool, a wrong mint, a non-pool, and records the pool's creation time even when opened an hour
later; `back` refuses at launch and with a complete
but unmigrated curve, refuses a substituted module and a substituted converter (a stranger key, or the claimer), honours `min_units`, backs once and refuses the second call with nothing moved;
another share (30 %); the payout property test; redeem rules; sweeps go to the treasury only and never touch the creator's
position) + `solana/tests/q1_meteora_dbc.rs` (the spike, full flow). Opt-in, against the **real** rook_core:
`cargo test -p rook_backing --features real-rook-core --test real_rook_core` (needs rook_core's v2 test binary; 2 tests, green
2026-10-09: `back` through the real `convert` with rook_backing's `["converter"]` PDA passed to `init_registry` — **no clock
jump**: nothing proposed, the test asserts the clock stayed under `CHANGE_DELAY` — with a stranger refused as admin; and the admin following a real
`propose_owner` / `accept_owner`). `ROOK_CORE_REAL=1` makes the harness's `Env::new()` load the real rook_core
(`target/deploy/rook_core.so` if it is not SBPF v3, else `target/deploy-sbpfv2/rook_core.so`); the mock-account suites
(`setup_collection`) refuse to run under it with a pointer to `real_rook_core.rs`, which builds real rook_core state.

Mocked: `rook_core::convert` in the default suite (`solana/tests/mock_rook_core`, built with
`CARGO_TARGET_DIR=$PWD/target/mock cargo build-sbf --manifest-path tests/mock_rook_core/Cargo.toml --sbf-out-dir tests/fixtures`;
2 SOL per unit, converter must be listed in the registry account; the mock registry sits at rook_core's real `["registry"]` PDA
with the `Registry` discriminator and `owner` = the harness's `rook_owner`, converter keys from byte 64). Real: Meteora DBC, DAMM v2, Token Metadata (mainnet bytes), the
DAMM v2 migration config (mainnet account). The mock's registry/module/wrapper are raw accounts; the treasury in the default suite
is a funded system account at rook_core's `["treasury"]` address.

## 6. Open questions

- **Q9, the backing share.** 50 % to start. Each (share, threshold) needs its own DBC curve (the fixed 1B supply fails
  otherwise, spike §6.1); with 50 % / 85 SOL the DAMM pool opens with 42.5 SOL and 200M coins (≈ 212.5 SOL mcap) and each coin is
  backed by 42.5 SOL of wX per 10⁹ coins. Model pool depth against backing before launch.
- ~~**Admin binding.**~~ Closed 2026-10-09 (§7.3): the admin is rook_core's `Registry.owner`, read on every call.
- **Surplus.** `partner_withdraw_surplus` (the share of overshoot above the threshold) is not wired; it belongs in `sweep_*`
  (→ treasury) or in `back` (→ backing).
- **Leftover coins.** `leftover_receiver` is the claimer; with pre = post supply nothing is left over, but any coins that do land
  at the claimer (leftover, or base-token fees if a config ever collects them) have no instruction yet (burning them would raise
  every coin's backing).
- **Creator trading-fee share** is 0 in the spike config; creators still earn their LP position's fees after graduation.
- ~~**SBPF v3**: the production build (`anchor build`'s default) has not run these CPIs~~ — run 2026-10-09 on
  `solana-test-validator` 4.1.2 with Meteora DBC, DAMM v2, Token Metadata and the "Customizable" DAMM config cloned from
  **devnet** (`ops/rehearsal/sol-devnet/run.sh --local-validator --dbc`): the v3 binaries of both programs deployed; `init_backing`,
  `register_partner`, DBC `create_config` (claimer PDA as fee claimer, a 2 SOL threshold), `register_partner_config`, a launch,
  `open_vault`, a buy to the threshold, the permissionless `migration_damm_v2` all landed; `back` simulated: DBC's
  `withdraw_migration_fee` CPI signed by the claimer PDA **ran**, then rook_core's `convert` refused `NotConverter` (the claimer is
  proposed, accepted only after 7 days), so the whole transaction reverted (62,135 CU). The devnet DBC (deployed slot 508,712,305,
  newer than mainnet's 0.2.1) accepted rook_backing's pinned pool/config offsets. **Rerun after §7.4 (same evening): `back`
  landed on v3 in the same run** — the `["converter"]` PDA allowed at `init_registry`, a keeper floor post on listing 0, then
  `back` (80,289 CU, whole transaction: withdraw_migration_fee → unwrap → rook_core `convert`; 121,359,223 wX base units into
  the vault) and a holder's `redeem_backing` (30,863 CU): **59 PASS, 0 FAIL** (`docs/v9/rehearsal-sol-devnet.md` §6).
- **Meteora upgrades.** DBC and DAMM v2 are upgradeable by one Meteora key; `rook_backing` reads `PoolConfig` / `VirtualPool` by
  offset (DBC 0.2.1). Pin the ProgramData hash in monitoring and rerun these tests on every Meteora release.
- ~~A permissionless `back` lets anyone pick `min_units`~~ — closed 2026-10-10 (§8.1): the bound is derived on-chain from the
  accepted floor; a cranker's lower value is ignored.
- `partner_withdraw_surplus` and leftover coins in the claimer still have no instruction (A3-04, measured 0 surplus; the
  leftover is a few coins per coin with pre = post supply).

## 7. Fixes after review (2026-10-09)

### 7.1 `init_backing` was already the upgrade authority's

`InitBacking` already required `program_data.upgrade_authority_address == Some(authority)` (`Unauthorized`); unchanged except
that it no longer takes an `admin` argument (§7.3) and pins `Settings.registry`. Test: `init_backing_only_by_upgrade_authority`
(stranger refused, authority succeeds, once; the harness writes the authority into LiteSVM's ProgramData, which LiteSVM 0.10
creates with none).

### 7.2 `Vault.launched_at` from the DBC pool

DBC 0.2.1's `VirtualPool` carries `activation_point: u64` (offset 296, after `sqrt_price`); `initialize_virtual_pool_*`
takes no activation argument and sets it to the current point, i.e. the pool's creation time in the config's activation unit.
`register_partner_config` now also requires `activation_type == Timestamp` (offset 234 of `PoolConfig`; Rook's curve already
uses it), so every Rook pool's `activation_point` is unix seconds. `open_vault` copies it into `Vault.launched_at` (refusing 0:
`NoLaunchTime`), independent of when the vault is opened. The test checks against the real DBC bytes that
`activation_point == the clock at launch` and that a vault opened an hour later still records the launch. rook_core's wake
`reclaim` checks `launched_at >= request.at` (rook_core as-built §8.3). Layout: `crates/rook_layouts::backing_vault`, tested
here by `state::tests::vault_matches_the_shared_layout` (every offset, length = `8 + Vault::INIT_SPACE` = 259, discriminator).

### 7.3 The admin follows rook_core's `Registry.owner`

`Settings.admin` and `set_admin` are gone. `register_partner` and `register_partner_config` take rook_core's registry account:
address = `Settings.registry` (rook_core PDA `["registry"]`, derived at `init_backing`), owner program = rook_core (both
`NotRegistry`), the `Registry` discriminator, and the signer must equal its `owner` field (`Unauthorized`), read through
`rook_layouts::registry` (rook_core tests its struct against it). `register_partner` also refuses `params.registry` other than
that PDA (`WrongRegistry`). A registry ownership change (Ownable2Step in rook_core) carries over at once, with no rook_backing
transaction. Tests: `the_admin_is_the_registry_owner` (owner succeeds; stranger refused for both instructions; after the owner
field changes the old owner is refused and the new one succeeds; foreign registry pin refused; a look-alike account at another
address, a foreign-owned account at the right address, and a wrong discriminator are all refused) and, against the real
rook_core, `the_backing_admin_follows_the_real_registry_owner` (the pending owner is refused until `accept_owner`; then the old
owner is refused and the new one registers).

### 7.4 One program-wide converter, allowed at `init_registry` (rook_core §8.2, §8.8; solana-migration.md Q14)

`back` no longer signs `convert` with the per-collection claimer. It signs with ONE program-wide PDA `["converter"]`
(`CONVERTER_SEED`) and the claimer pays: `convert`'s `converter` (readonly signer) and `payer` (writable signer, the claimer, a
system-owned PDA, so rook_core's system transfer moves the fee straight out of it) are already separate accounts, so no lamports
move to the converter and it is never funded. `invoke_signed` carries both seed sets. `Settings` stores `converter` and
`converter_bump` (derived once at `init_backing`); `back` takes `settings` (owner + discriminator checked by `Account`, no seeds
re-derivation: only `init_backing` creates one) and `converter` (`address = settings.converter`: a 32-byte compare), so there is
no `find_program_address` in `back`.

rook_core's `init_registry` takes `initial_converters`, so the launch passes this one PDA at init and `back` converts from the
first graduation: no 7-day wait and no per-collection converter change ever (a new collection's claimer is only the DBC fee
claimer). Deploy order: deploy both programs; `init_registry(owner, network, [PDA "converter" of rook_backing])`; `init_backing`.
A later converter (another program) still waits 7 days; the guardian can still remove it at once (then `back` refuses with
`NotConverter` and can be retried; the migration fee stays claimable in DBC).

Tests: `back_refuses_before_migration_then_backs_once` refuses a substituted converter (a stranger key and the claimer,
`ConstraintAddress`) and checks `Settings.converter`; the mock registry lists the `["converter"]` PDA (not the claimers);
`real_rook_core::back_converts_through_the_real_rook_core` initializes the real registry with it (`core::Env::with_converters`),
checks nothing is pending, and backs with no clock jump.

## 8. Audit fixes (2026-10-10)

From `docs/v9/audit-2026-10/a3-backing.md` (its "Fixes (2026-10-10)" section maps finding → code → test). `cargo test -p
rook_backing`: 2 unit + 8 audit + 9 backing + 1 spike, green; `--features real-rook-core --test real_rook_core`: 2, green.

### 8.1 `back` bounds the conversion price on-chain (A3-06, A1-08)

`back` reads rook_core's `Module` (owner rook_core, exact 957-byte layout through `rook_layouts::module`, `module.listing ==
partner.listing`) and computes `floor = ⌊⌊fee · 10⁹ / ⌈accepted · (1 + conv_bps)⌉⌋ · (1 − 10 %)⌋`; the bound passed to `convert`
and re-checked on the vault's balance is `max(min_units, floor)`, so a cranker's lower `min_units` (0, 1) is ignored. The
reference is the **accepted floor without the target premium** `convert` may add (`max(price, upTarget)`); the A3 report's
preferred option 1. `BACK_PRICE_TOLERANCE_BPS = 1_000` (10 %): rook_core's `validate_params` caps `fill_band_bps` at 10 %, so the
fill TWAP alone (clamped to `accepted × (1 ± band)`) never trips the bound; it trips only when the keeper's raw target runs more
than ≈ 11 % ahead of the accepted floor (the day band allows 20 %, ≈ 24 % fewer units with the defaults). A refused `back`
leaves the fee in DBC (nothing moves) and succeeds after the next posts lift `accepted`. A module never posted (`accepted == 0`)
refuses with `NoFloor`. Cost: +≈ 700 CU (real `convert`: 100,901 → 101,610), no new account. Tests:
`fixed_a3_06_back_bounds_the_price_by_the_accepted_floor` (mock), `instructions::back::tests` (the arithmetic, the band and
day-band edges), `real_rook_core` (through the real `convert`).

### 8.2 `register_partner_config` enforces the design's invariants (A3-02, A3-05)

Refused: a config `version` other than 0 (what DBC 0.2.1 writes; the offsets read are 0.2.1's), `leftover_receiver` other than
the claimer (the launch: `fee_claimer = leftover_receiver = claimer`), an unlocked partner or creator LP share or an initialized
vesting info (bytes 184 / 200), a migrated DAMM v2 pool collecting base-token fees or compounding (offsets 360 / 366), a pool
creation fee (offset 368). The launch config (`ops/solana/launch-params.mjs`) meets all of them. Offsets verified against DBC
0.2.1's IDL (`idls/dynamic_bonding_curve.json`, `PoolConfig`). **To re-check on devnet**: the devnet DBC is a newer build than
mainnet's 0.2.1; if its `create_config` writes another `version`, the devnet rehearsal's `register_partner_config` now refuses
(`WrongConfigVersion`) until that version's layout is verified. Fixed supply (pre = post) is not enforced (one test runs a 30 %
fee with DBC's dynamic supply); the leftover goes to the claimer either way. Test:
`fixed_a3_02_register_partner_config_enforces_the_rook_invariants`.

### 8.3 `register_partner` checks the rook_core accounts it pins (A3-03)

Four accounts added (`listing`, `module`, `module_vault`, `wrapper`), each owned by rook_core with its discriminator (so rook_core
created it at its PDA), and the links rook_core wrote must hold: `module.listing == listing`, `module.vault == module_vault`,
`module.wrapper == wrapper`, `wrapper.listing == listing`, `wrapper.wx_mint == params.wx_mint`. A wrong pin used to fail closed
at every `back` with only an upgrade to fix it; now it is refused at registration. The event `PartnerRegistered` carries the
listing id, for the ceremony's listing-order check (A6-02; the order mistake itself is off-chain). `register_partner` ≈ 15–19k
CU (bump-search variance). Still open: no instruction changes a `Partner` or retires a config (≤ 8). Test:
`fixed_a3_03_register_partner_checks_the_rook_core_accounts`.

### 8.4 `Vault.creator` = the creator role holder at backing (A3-01)

Decision: the wake desk credits whoever holds DBC's creator role when the coin is backed (its graduation for Rook). `back`
re-reads `pool.creator` (DBC's account, owner/length/discriminator checked) and stores it; `open_vault`'s copy is provisional
(nothing reads it before `graduated`). Requiring `open_vault` before migration was rejected: the deploy-later launch backs coins
that graduated before rook_backing existed. DBC's `transfer_pool_creator` still lets a creator hand the role (and so the wake
credit) to anyone before backing; the loss is bounded by one bond per coin. Docs updated: the field is "the role holder at
backing", not "who launched the coin". Test: `fixed_a3_01_vault_creator_is_the_role_holder_at_backing`.

### 8.5 The test harness

The mock rook_core (`tests/mock_rook_core`) now signs as the wrapper PDA `["wrapper", listing]` (rook_core's seeds), rebuilt
`--arch v2` into `tests/fixtures/mock_rook_core.so`; `setup_collection` writes rook_core-shaped `Listing`/`Module`/`ModuleVault`/
`Wrapper` accounts at rook_core's PDAs (`Env::mock_core_accounts`; the mock module's `accepted` is the mock's 2 SOL price with
`conv_bps` 0; `Env::mock_set_accepted` moves it).

### 8.6 Interface changes

`register_partner` takes four more accounts (the rehearsal tool's builders need them); `back`, `open_vault`, the sweeps and
`redeem_backing` are unchanged. New errors appended (`WrongConfigVersion` … `NoFloor`). Sizes: SBPF v2 538,272 → 555,424 bytes;
v3 528,184.
