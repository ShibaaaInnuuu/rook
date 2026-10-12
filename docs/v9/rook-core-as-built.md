# rook_core as built (2026-10-09; audit fixes 2026-10-10, §9; holder raffle 2026-10-10, §9.11)

`solana/programs/rook_core`, Anchor 1.2.1, program id `GepazuDZfS6zjwQ3U2zHCgk93xpRkp34AkmEFZKrAd7T`. One program, a module per
role (registry, floor, wrap, treasury, wake), so the tightly coupled parts make no CPI to each other. The EVM's
per-collection contracts are PDAs of the collection's `Listing`. Ported from the Solidity at tag `robinhood-stop-2026-10-09`
(`ref-robinhood-contracts.md`); plan: `solana-migration.md` §2.1, §3.

Build: `anchor build` (SBPF v3, 1,033,336 bytes since §9.11). Tests: `cargo test -p rook_core` (11 unit + 88 LiteSVM + 13 audit, 2026-10-10). LiteSVM 0.10 refuses SBPF v3
ELFs (`InvalidAccountData`), so the tests load the same source built for SBPF v2 when `target/deploy/rook_core.so` is v3:
`cargo build-sbf --manifest-path programs/rook_core/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2` (the convention
rook_backing's harness already uses).

## 1. Instructions

Signers in **bold**. "owner" = `Registry.owner` (the Squads vault in production), "guardian" = `Registry.guardian`,
"keeper" = `Registry.keeper` (not paused), "custodian" = `Listing.custodian` (the canister's Ed25519 key), "poster" =
`Listing.poster` for an EthNft listing (the canister) or the keeper for an Inscriptions listing.

| Role | Instruction | Signer | Source |
|---|---|---|---|
| registry | `init_registry(owner, bitcoin_network, initial_converters)` | **upgrade authority** + **payer** (§8.1, §8.5, §8.8) | ListingRegistry constructor |
| | `propose_owner` / `accept_owner` | **owner** / **pending owner** | Ownable2Step |
| | `set_guardian` | **owner** (never the keeper or owner) | `setGuardian` (N-8) |
| | `propose_roles(keeper)` / `accept_roles` | **owner** (first instant, later +7 d) | `proposeRoles` / `acceptRoles` |
| | `pause_keeper` / `unpause_keeper` | **owner or guardian** / **owner** | `pauseKeeper` / `unpauseKeeper` |
| | `propose_converter` / `accept_converter` | **owner** (+7 d, §8.2; the initial ones at `init_registry`, §8.8) | new (the `convert` allow-list, ≤ 8) |
| | `remove_converter` | **owner or guardian** (instant; also cancels a pending one) | new |
| | `list_collection(kind, name, custody, custodian, poster)` | **owner** + **payer** | `listInscriptions`, NFT listing |
| | `set_paused(bool)` | **owner** | `setPaused` |
| | `pause_poster` / `unpause_poster` | **owner or guardian** / **owner** | `PosterSource.pause` / `unpause` (D-M-1); effective for both kinds since §9 (A1-01) |
| | `propose_root` / `accept_root` | **owner** (+7 d, Inscriptions only) | `proposeRoot` / `acceptRoot` |
| floor | `init_module(params)` | **owner** + **payer** | FloorModule constructor (every parameter immutable) |
| | `post_floor(price, confidence_bps, depth_items)` | **poster** | `postFloor` |
| | `mint(lamports, min_units)` | **buyer** | `mint` |
| | `burn(units, min_lamports)` | **owner of the wX** | `burn` |
| | `convert(lamports, min_units)` | **converter** + **payer** | `enqueue` → `_tryFill` |
| | `advance(amount)` | **custodian** | `advance` |
| | `settle_advance(cost)` | **custodian** | `settleAdvance` |
| | `return_advance(lamports)` | **custodian** | `returnAdvance` |
| | `item_arrived(cost)` | **custodian** | `itemArrived` |
| | `snapshot_cap` | anyone | `snapshotCap` |
| | `fund_reserve(lamports)` | **payer** | `fundReserve` |
| | `skim` | anyone (fee payer) | `skim` → the treasury |
| wrap | `init_wrapper(meta: WxMetadata { name, symbol, uri })` | **owner** + **payer** | EthNftWrapper constructor + `wireFloorModule`; creates the wX mint's Metaplex metadata (§9.6) |
| | `set_wx_metadata(meta)` | **owner** | new: the wX's name, symbol, uri (image), signed by the wrapper PDA (§9.6) |
| | `init_mint_limit` / `propose_mint_limit` | **owner** | `initMintLimit` / `proposeMintLimit` |
| | `accept_mint_limit` | anyone, after 7 d | `acceptMintLimit` |
| | `mint_item(args)` | **custodian** (pays the item's rent) | `mint`, `mintDeposit`, `Wrapper.wrapDeposit` |
| | `redeem(destination)` | **holder** (pays the ticket's rent) | `redeem`, `redeemToBurn` |
| | `resolve` | anyone | `resolve` (+ `_skipRefunded`) |
| | `note_sent(tx)` / `mark_undeliverable` / `record_delivery(tx)` | **custodian** | same |
| | `refund` (+ the ticket's `["drand", round]` PDA) | anyone (unit back to the ticket's owner) | `refund`; a pinned ticket whose round is proven is refused (§9.4) |
| | `prove_round(round, randomness)` | **keeper** — TODO(drand on alt_bn128, Q4); `round ≤ round_at(now)` (§9.3) | `DrandEvmnet.prove` |
| treasury | `init_treasury` | **owner** + **payer** | constructor (90/10; reserve cap 20,000 bps, §9.10) |
| | `set_shares(reserves, ops)` / `set_ops` / `set_reserve_cap(bps)` / `add_collection` | **owner** | same; shares, a named ops address and the cap wait `TREASURY_DELAY` (24 h) |
| | `apply_shares` / `apply_ops` / `apply_reserve_cap` | anyone, after the 24 h | new |
| | `receive(lamports)` | **payer** | `receive()` |
| | `allocate` | anyone | `allocate` |
| | `begin_distribution` (all modules as remaining accounts) / `distribute(index)` (+ optional `ops`, + the collection's `raffle` PDA) | anyone | `distribute`, paginated; no restart of an open round for a day (§9.2); each module's surplus capped, the excess split between the raffle pot and ops (§9.10, §9.11) |
| | `set_raffle_bps(bps)` / `apply_raffle_bps` | **owner** / anyone after 24 h | new: the raffle's share of the excess (§9.11) |
| raffle | `init_raffle` | **payer** (anyone) | new (§9.11, `raffle-design.md`) |
| | `fund_prize` | anyone | new: the pot buys one whole wX through the module's public mint |
| | `commit_raffle(root, total_weight, snapshot_slot, entries)` | **keeper** | new: weekly snapshot, pins a future drand round |
| | `draw_raffle` (+ the pinned `DrandRound`) | anyone | new |
| | `claim_raffle(cum_start, weight, proof)` | **claimer** (pays the winner's ATA) | new: Merkle proof of the winning leaf |
| | `expire_raffle` | anyone | new: unclaimed after 30 d / undrawn after 7 d |
| wake | `init_desk(DeskParams { bond, min_bond, max_bond, window })` | **owner** + **payer** | constructor (§8.6) |
| | `request(bond)` | **requester** | `request` |
| | `mark_served` / `reclaim` / `forfeit` | anyone | same |
| | `withdraw` | **requester** | `withdraw` (see §5) |

### `convert` — the interface rook_backing calls

`convert(lamports: u64, min_units: u64)`, accounts in this order:

| # | Account | |
|---|---|---|
| 0 | `converter` | signer; must be in `Registry.converters` (rook_backing signs with its one program-wide `["converter"]` PDA via `invoke_signed`; its per-collection claimer PDA is the `payer`) |
| 1 | `payer` | signer, writable; the lamports leave it by a system transfer; may equal `converter` (then it must be system-owned) |
| 2 | `registry` | PDA `["registry"]` |
| 3 | `listing` | the collection's `Listing` |
| 4 | `module` | writable; PDA `["module", listing]` (`module.listing == listing`) |
| 5 | `module_vault` | writable; PDA `["module_vault", module]` (`== module.vault`) |
| 6 | `wrapper` | writable; PDA `["wrapper", listing]` (`== module.wrapper`) |
| 7 | `wx_mint` | writable; PDA `["wx_mint", wrapper]` (`== wrapper.wx_mint`) |
| 8 | `dest` | writable; an SPL token account of `wx_mint` (any owner) |
| 9 | `token_program` | SPL Token |
| 10 | `system_program` | |

Refused unless: the converter is allowed (`NotConverter`), the listing is not paused (`ListingPaused`), the floor is fresh
(`Stale`: posted < 3 h ago with confidence ≥ the minimum), the poster is up (`Paused`), `lamports > 0` (`BadInput`).
`gradPrice = ⌈max(price, upTarget) × (1 + convBps)⌉` lamports per whole unit; `units = ⌊lamports × 10⁹ / gradPrice⌋`, refused
if 0 or `< min_units` (`Slippage`) or if `eth_backed + units > maxEthBacked` (`OverBacked`). `reserveBps` of the lamports
and of the units go to the reserve tier (never advanced), the rest to the convertible backing; `units` are minted to `dest`
as ETH-backed. No return data; emits `Converted { module, converter, dest, lamports, units, unit_price }`. Errors are
`6000 + RookError` index (`src/error.rs`).

## 2. Accounts

All PDAs of rook_core. Size includes Anchor's 8-byte discriminator; rent = the rent-exempt minimum (LiteSVM, current mainnet rates).

| Account | Seeds | Bytes | Rent (SOL) | Paid by |
|---|---|---|---|---|
| `Registry` | `["registry"]` | 488 | 0.004287 | deployer, once |
| `Listing` | `["listing", id u32 le]` | 284 | 0.002868 | owner, per collection |
| `Module` | `["module", listing]` | 957 | 0.007552 | owner, per collection |
| `ModuleVault` (holds the lamports) | `["module_vault", module]` | 8 | 0.000947 | owner, per collection |
| module wX token account | `["module_wx", module]` | 165 | 0.002039 | owner, per collection |
| `Wrapper` | `["wrapper", listing]` | 662 | 0.005498 | owner, per collection |
| `wX` mint (9 decimals, authority = wrapper, no freeze) | `["wx_mint", wrapper]` | 82 | 0.001462 | owner, per collection |
| `ItemSet` (free set + pending queue, ≤ 256 each) | `["items", wrapper]` | 4,144 | 0.029733 | owner, per collection |
| `Item` | `["item", wrapper, item_id u32 le]` | 197 | 0.002262 | custodian, per item |
| `HeldMarker` | `["held", wrapper, key]` | 12 | 0.000974 | custodian, per distinct NFT |
| `SourceMarker` | `["source", wrapper, source]` | 9 | 0.000954 | custodian, per item |
| `Ticket` | `["ticket", wrapper, id u64 le]` | 241 | 0.002568 | redeemer |
| `DrandRound` | `["drand", round u64 le]` | 48 | 0.001225 | keeper |
| wX Metaplex `Metadata` (Metaplex-owned; update authority = the wrapper) | `["metadata", MPL, wx_mint]` of Metaplex | 607 | 0.005116 + Metaplex's 0.01 create fee | owner, per collection (§9.6) |
| `Treasury` (holds its lamports) | `["treasury"]` | 1,485 | 0.011226 | owner, once |
| `Raffle` (holds its pot) | `["raffle", listing]` | 327 | 0.003167 | anyone (`init_raffle`), per collection (§9.11) |
| raffle wX token account | `["raffle_wx", raffle]` | 165 | 0.002039 | anyone (`init_raffle`), per collection |
| `Desk` | `["desk"]` | 49 | 0.001232 | owner, once |
| `Request` (holds its bond) | `["request", id u64 le]` | 105 | 0.001622 | requester (back on `withdraw`) |
| `WakeSlot` | `["wake_slot", listing]` | 16 | 0.001002 | first requester |
| `Spent` | `["spent", backing vault]` | 9 | 0.000954 | reclaim caller |

Per collection ≈ 0.065 SOL (≈ 0.050 of rent plus the wX metadata's 0.0151, §9.6); per custody item ≈ 0.0042 SOL (canister);
per redemption 0.0026 SOL (redeemer). The program itself is 901,880 bytes (SBPF v3, `anchor build` of 2026-10-10; it was
867,112 before §9): ≈ 6.28 SOL of rent for the program-data account, twice that while a deploy buffer exists.

Module layout (`state/floor.rs`): `listing, wrapper, vault, module_wx`, bumps, `params` (§3), the floor (`accepted, target,
posted_at, move_acc, confidence_bps, depth_items, day_anchor, day_anchor_at, last_good_at`), the buckets (`mint_level,
mint_at, burn_level, burn_at`), the books (`backing, reserve, surplus, reserve_units, revenue`), conversions
(`items_arrived, arrivals, arrived_at[32], snap_cap, snap_at`), the advance (`transit_units, advanced, advance_basis,
advanced_at`), fills (`fills[8], fill_count`), volume (`day_volume[7], day_of[7]`), `mint_low[2]`, totals
(`converted_lamports, converted_units`). The books: `vault.lamports − rent ≥ backing + reserve + surplus + revenue`.

Wrapper ledger: `wX supply = (held − open_tickets) · 10⁹ + eth_backed`, and a redemption needs `held ≥ open_tickets + 1`.

## 3. Units

| EVM (source) | value | here | value |
|---|---|---|---|
| `ONE`, `ONE_ITEM` (FloorModule:111, EthNftWrapper:58) | 1e18 | `ONE` | 10⁹ (wX has 9 decimals) |
| `mintPerHour`, `burnPerHour` (StackChecks:141-142) | 5e18 | `5 · ONE` | |
| `maxPublicEthBacked` (StackChecks:143) | 10e18 | `10 · ONE` | |
| `maxEthBacked` (StackChecks:144) | 40e18 | `40 · ONE` | |
| `revenueTarget` (StackChecks:184) | 1 ETH | `Params.revenue_target` (lamports) | **the launch file's**, converted at the live SOL/ETH rate (§8.6) |
| wake bond at launch (`ETCH_WAKE_BOND`) | 0.002 ETH | `DeskParams.bond` (lamports) | the launch file's |
| `WakeDesk.MIN_BOND` / `MAX_BOND` | 0.0005 / 1 ETH | `DeskParams.min_bond` / `max_bond` (lamports, stored in `Desk`) | the launch file's |
| floor price | wei per item | lamports per whole unit | posted in lamports (§2.3) |
| every bps constant, every duration | — | unchanged | `FLOOR_TTL` 3 h, `STALE_ESCAPE` 7 d, `MAX_DAY_MOVE_BPS` 2,000, … |

No exchange rate is in the program (decision D-c, `solana-migration.md` §2.9): `units.rs` holds only unit conversions (`ONE`,
`WX_DECIMALS`, `LAMPORTS_PER_SOL`, `BPS`); every ETH-denominated value is a parameter of the instruction that fixes it, written by
`ops/solana/launch-params.mjs`. Defaults (`params::default_params(revenue_target)`): spreads
base 15 / vol weight 1× / stale 100 / stress 300 / max 1,000, maxMove 500/h, convBps 300, buffer 500, low-confidence 100 to
full 8,000, min confidence 3,000, maxItemsPerDay 3 (stage 0), reserveBps 2,500, fillBand 300. Math is checked `u128`
`mul_div` / `mul_div_ceil`, the rounding direction of each Solidity `mulDiv` kept.

## 4. Compute units

Measured in LiteSVM on the SBPF v2 build (`tests/suite/flow.rs`), default 200k per-instruction budget:

| Instruction | CU |
|---|---|
| `post_floor` | 13,915 |
| `convert` | 35,320 |
| `mint` | 36,287 |
| `burn` | 35,240 |
| `advance` | 18,062 |
| `mint_item` (EthNft, creates 3 accounts) | 41,718 |
| `settle_advance` | 32,063 |
| `redeem` (creates the ticket) | 26,572 |
| `resolve` (1 item) | 11,669 |
| `resolve` (64 items promoted at once) | 16,770 |
| `record_delivery` | 14,231 |

(+≈ 70–75 CU on the instructions that load the `Registry` since it grew by 40 bytes for the pending converter, §8.2.)

A `convert` CPI from rook_backing adds the CPI overhead (≈ 1k) plus rook_backing's own work; `convert` itself is one
system transfer and one SPL `mint_to`, both CPI depth +1 from rook_core.

## 5. What differs from the Solidity, and why

Dropped (OFF at launch, §2.7, or replaced by the DBC design, §2.1):
- **Rentals** (`rentIn`, `endRental`, `rentalReturned`, `settleRental`, `payDebt`, `claim`, `postRentalRoot`, renter pool,
  `owed`, `debt`): OFF at launch.
- **Floats, `itemSold`, `fundFloat`, bootstrap loans** (`bootstrap`, `repay`, `forgive`, `ITreasuryLoans`): floats were 0 and
  the treasury 90/0/0/10 at launch; the treasury keeps two shares (reserves, ops).
- **The graduation queue** (`enqueue` / `fill` / `release` / `cancel` / `isFilled` / `hasRoomFor` / `holdsRefunds`,
  `gradEth`, `gradUnits`, `RESUME_GRACE`): DBC owns the curve; `convert` converts at once or refuses, and the caller
  (rook_backing) retries. B-07's room rule becomes `maxEthBacked − eth_backed`.
- **`fundLiquidity`**: no liquidity lock (DBC graduates into locked DAMM v2).
- **`reclaimStray`**: the module's token account only ever holds an item's unit between `mint_item` and settlement.
- **Registry**: runes, `AssetVault` / `AutoBuyerVault` listings, `purchaser` / `attestor` roles, `isAutoBuyer`, `btcVault`
  (the custody address is fixed per listing), `bondBps`, `proposeTreasury`, `wire` / `wireNft` / `reserveBuyer`. The roles
  are the keeper (7-day rotation) and the guardian.
- **CollectionDirectory**: the directory is the PDA derivation (listing → wrapper, module). `wire` / `wireFloorModule` and
  the deploy-key powers are gone: the registry owner holds `init_mint_limit` / `propose_mint_limit` (closes the EVM's
  "deployer keeps a mint-limit power" gap).
- **Dual clocks** (`Wait`, `parentBlockSeconds`): unix time only (§2.4).
- **`redeemToRelist`** (reverted in the EVM too), `Wrapper.wrap` (v2 acquisition path, OFF), `WrapClaim`.

Changed:
- **`price()` keeps the fills TWAP** (8-fill ring, 24 h, within `fillBandBps`); fills come from `settle_advance` /
  `item_arrived` only (no sales).
- **`convert` refuses on a paused listing** (new: the task's spec); `mint` / `burn` read only the poster's switches, as the
  module did.
- **`resolve` over a refunded ticket steps the cursor** (the EVM's `_skipRefunded` loop needs every later ticket's
  account); a pinned ticket refunded at the cursor moves it at once.
- **Bounded collections**: ≤ 256 items held per wrapper (`MAX_HELD`), H11 limit ≤ 64/day, `maxItemsPerDay ≤ 32`, ≤ 8
  converters, ≤ 32 treasury collections.
- **Converters wait 7 days** (new: no EVM counterpart; §8.2) — except the initial ones `init_registry` sets (§8.8) — and the registry is initialized by the upgrade authority only
  (the EVM constructor ran in the deploy transaction; §8.1). Custody caps stay in the canister (48 items at step 1).
- **Treasury `distribute`** is paginated: `begin_distribution` reads every module's `volume7d` (remaining accounts) and fixes
  the round; `distribute(i)` pays one module's surplus, up to its reserve cap (§9.10). C-12's try/catch is unnecessary: one module's call cannot fail
  another's. A round cannot be restarted while an index is still owed its share, except a day after it began (§9.2).
- **WakeDesk**: the requester's coin is a rook_backing `Vault` (owner = rook_backing, `partner == ["partner", listing]` of
  rook_backing, `creator == requester` — the holder of DBC's transferable creator role when the coin was backed, not
  necessarily its launcher (§9.7) —, `graduated`); "launched after the request" (`coinIndex ≥ coinsAt`) becomes
  `launched_at ≥ request.at`, `launched_at` being the DBC pool's own creation time (§8.3); sweep-only coins do not exist. A lamport credit cannot be refused, so DI-L-2's `owed` path
  never arises: refunds and reclaims pay the requester directly and `withdraw` closes a closed request to return its rent.
- **Deferred: drand verification** — `prove_round` is keeper-attested until the BN254 pairing is ported to the alt_bn128
  syscalls (Q4). Until then the keeper chooses randomness for draws: a trust addition, flagged in code
  (`TODO(drand on alt_bn128, Q4)`).

## 6. Tests

`tests/rook_core.rs` (one LiteSVM binary, suites in `tests/suite/`): `floor` 22, `convert` 8, `wrap` 13, `registry` 10,
`treasury_wake` 17, `raffle` 10 (§9.11), `flow` 3 (the full Ethereum → wX → redeem → deliver path with a fake custodian keypair; CU; sizes),
`props` 5 (four 250-step random sequences of posts, mints, burns, conversions, advances/settles/returns, pauses, funding,
skims and lone mint-then-burn trades, checking after every step: vault ≥ books, the wX ledger, burns ≤ backing per unit,
convert-at-stale refuses, a lone trader never profits; plus a 3-seed × 200-step wrap/redeem/resolve/deliver/refund mix).
Each test names the Solidity test it ports. Unit tests: `math`, `hashing` (taproot checksum, sorted-pair Merkle, EVM address
parsing), `state::registry` (the `Registry` against the shared layout `rook_layouts::registry`), `state::layouts_tests`
(`Listing`, `Module`, `ModuleVault`, `Wrapper` and the seeds against `rook_layouts`, §9.8). Audit suites: `tests/audit_a1_core.rs`
(6), `tests/audit_a2_core.rs` (7). The LiteSVM harness loads Metaplex Token Metadata (`solana/tests/fixtures/mpl_token_metadata.so`). The wake suite builds its
backing vaults with `rook_backing::Vault` itself (a dev-dependency) and parses them with the shared layout.

## 7. Open questions

1. ~~**`init_registry` is first-come**~~ — closed 2026-10-09, §8.1.
2. ~~**Converters are instant**~~ — closed 2026-10-09, §8.2 (propose → accept after 7 days; removal instant).
3. ~~**Wake "launched after the request"**~~ — closed 2026-10-09, §8.3 (`Vault.launched_at` from the DBC pool; shared layout).
4. ~~**`SOL_PER_ETH = 20`** sets `revenueTarget` and the wake bond bounds~~ — closed 2026-10-09, §8.6 (no rate in the program).
5. **SBPF v3 vs LiteSVM 0.10**: the tests exercise the v2 build of the same source. LiteSVM ≥ 0.18 may load v3. The v3
   artifact itself is now checked on `solana-test-validator` (deploy + init of both listings + a keeper post, a public mint, a
   custodian's `mint_item` with a proof, a redemption to a signet taproot address and its delivery record): §8.7 and
   `docs/v9/rehearsal-sol-devnet.md`. (rook_backing's harness now loads
   `target/deploy-sbpfv2/rook_core.so` under `ROOK_CORE_REAL=1`.)
6. **drand on alt_bn128 (Q4)** replaces `prove_round`'s keeper attestation.
7. **Program size** (865 KB): ≈ 6 SOL of rent (≈ 12 during an upgrade buffer); the floor/wrap split into two programs would
   not shrink the total.
8. ~~**Converter onboarding takes 7 days**~~ — closed 2026-10-09, §8.8: rook_backing converts with ONE program-wide
   `["converter"]` PDA, allowed at `init_registry`; a new collection needs no converter change at all.

## 8. Fixes after review (2026-10-09)

### 8.1 `init_registry` is the upgrade authority's

`InitRegistry` (`src/instructions/registry.rs`) takes `authority` (signer), `payer`, and the program's ProgramData account
(seeds `[program_id]` under the BPF upgradeable loader, owner = the loader); the handler reads `upgrade_authority_address`
by hand (`upgrade_authority()`: tag 3 @0, option tag @12, key @13 — `Account<ProgramData>` would pull in bincode, +53 KB of
program) and refuses anything but `Some(authority)` with `NotUpgradeAuthority` (new error, appended: earlier codes unchanged).
A program without an upgrade authority can never be initialized, so initialize before finalizing (Q8). The payer may differ
(e.g. the Squads vault as authority, a hot key paying).

Tests (`registry::only_the_upgrade_authority_initializes_the_registry`): no authority → refused; a stranger (alone, or with
the authority only paying) → refused; a substituted ProgramData naming the stranger → refused (`ConstraintSeeds`); the
authority succeeds with a third-party payer; once only. LiteSVM 0.10 deploys every program as upgradeable with
`upgrade_authority_address: None` and has no API to set one, so the harness (`Env::set_upgrade_authority`) writes the
authority into the ProgramData account it created; `Env::bare()` makes the owner the authority.

### 8.2 Converters: propose → accept after 7 days

`add_converter` is gone. `propose_converter(converter)` (owner; refused if already allowed or the list is full; one pending,
a new proposal replaces it and restarts the clock) records `Registry.pending_converter` / `pending_converter_at = now +
CHANGE_DELAY`; `accept_converter` (owner, from that time) moves it into `converters`. `remove_converter` is instant and
stop-only: the owner **or the guardian**, for an allowed converter or the pending one. A proposed converter is not in
`converters`, so `convert` refuses it (`NotConverter`). Events `ConverterSet { converter, ready_at, allowed }`,
`ConverterRemoved { converter, by }`. `Registry` grows 447 → 487 bytes.

Tests: `registry::a_converter_waits_seven_public_days` (too early at 7 d − 1 s, owner-only accept, re-proposal restarts the
clock), `registry::the_guardian_removes_a_converter_at_once` (stranger refused; guardian removes an allowed one and cancels a
pending one, which can then not be accepted), `convert::a_proposed_converter_cannot_convert` (refused while pending and while
ready-but-not-accepted; converts after accept; refused again after the guardian's removal). The suites' `allow_converter`
helper proposes, jumps the clock 7 days for the accept and back (so mid-scenario floors stay fresh).

### 8.3 Wake `reclaim` checks the coin's launch time; the Vault layout is shared

rook_backing records `Vault.launched_at` = the DBC pool's `activation_point` at `open_vault` (see the rook_backing as-built
doc §7.2): the pool's creation time, not the backing time and not the clock at `open_vault`. `reclaim` requires
`launched_at >= request.at` (and still `graduated`, `creator == requester`, the listing's partner). A coin launched before
the request that graduates after it no longer pays the bond back.

The hand-kept offsets are gone: `crates/rook_layouts` (no dependencies, both programs depend on it) holds the offsets of
rook_backing's `Vault` and rook_core's `Registry.owner`; `BackingVaultView::parse` (`src/state/wake.rs`) delegates to
`rook_layouts::backing_vault::parse` (exact length and discriminator). The contract is tested on both sides: rook_backing's
`state::tests::vault_matches_the_shared_layout` serializes its real `Vault` and checks every named offset; rook_core's wake
suite builds every test vault with `rook_backing::Vault` (dev-dependency) and asserts the parser returns each field. A field
reorder in either struct fails CI. Test: `treasury_wake::a_coin_launched_before_the_request_never_reclaims` (launched 10 min
before, backed after → `NotALaunch`; a truncated vault → `NotALaunch`; launched in the request's second → paid).

### 8.4 rook_backing's admin is this registry's owner

rook_backing no longer stores an admin: its admin instructions read `Registry.owner` through `rook_layouts::registry`
(discriminator, `owner` the first field — keep it first: `state::registry::tests` fails otherwise). See the rook_backing
as-built doc §7.3.

### 8.5 The registry names its Bitcoin network (2026-10-09, for the devnet rehearsal)

`init_registry(owner, bitcoin_network)`; `Registry.bitcoin_network: BitcoinNetwork` (`Mainnet` | `Testnet4` | `Signet`, a
one-byte enum before `bump`; `Registry` 487 → 488 bytes), fixed for good. `list_collection` checks an Inscriptions listing's
custody address with `hashing::is_taproot_on(custody, network.hrp())` (`bc` mainnet, `tb` testnet4/signet: both 62-character
witness-v1 addresses, bech32m, zero padding); `redeem` checks a `Taproot` destination against the prefix of the listing's own
custody address (`taproot_hrp`), which `list_collection` tied to the registry's network, so `redeem` needs no registry account.
`is_taproot` stays mainnet-only. Mainnet is the type's default and production's value; the launch file must name it.
The canister refuses a frog listing of another network than its `btc_network` (`frogsol::network_ok`: the registry's network on the
frog flow's reads, the listing's custody prefix on the custodian's), and its burn destination follows the network
(`frogsol::burn_address`: the NUMS output with the `tb` prefix on signet/testnet4).

Tests: `hashing::tests::taproot_follows_the_network`, `wrap::the_registry_bitcoin_network_decides_the_taproot_prefix`
(mainnet refuses a `tb1p` custody; signet refuses a `bc1p` custody and a `bc1p` destination, takes `tb1p` ones and the burn,
resolves and records the delivery; testnet4 lists a `tb1p` custody); the vectors carry `taproot_tb` (checked by the canister's
`the_ported_floor_rules_and_hashing_match_rook_cores_own_vectors`).

### 8.6 No SOL/ETH rate in the program (decision D-c, 2026-10-09)

`SOL_PER_ETH`, `milli_eth`, `REVENUE_TARGET`, `WAKE_BOND`, `MIN_BOND` and `MAX_BOND` are gone from `units.rs`. `Params` moved to
`params.rs` (the IDL type is unchanged); `default_params(revenue_target)` takes the lamports. `init_desk` takes
`DeskParams { bond, min_bond, max_bond, window }` and refuses `min_bond = 0`, `min_bond > max_bond`, a bond outside
`[min_bond, max_bond]` and a window outside `[1 d, 30 d]`; `Desk` stores `min_bond` and `max_bond` (after `request_count`;
33 → 49 bytes) so the go-live check reads the bounds the bond was checked against. All immutable, as the WakeDesk's were. The
values come from `ops/solana/launch-params.mjs` (≥ 2 public SOL/ETH sources agreeing within 1 %), which writes each value with
its ETH origin and the rate. Test: `treasury_wake::the_desk_takes_its_bond_bounds_as_parameters`. The treasury holds no
ETH-denominated value (two shares in bps), so `init_treasury` is unchanged.

### 8.7 The SBPF v3 artifact on a local validator (2026-10-09)

`ops/rehearsal/sol-devnet/run.sh --local-validator --dbc` deployed the `anchor build` (SBPF v3, `e_flags` 3) binaries of both programs
to `solana-test-validator` 4.1.2 and ran there: the init from a live launch file (registry on Signet, treasury, desk, three listings),
five refusals, the program drill (post, mint, advance / return / advance, `mint_item` with a Merkle proof ×2, `settle_advance`, a
`bc1p` redemption refused on the signet registry, two `tb1p` redemptions, `prove_round`, `resolve` ×2, `note_sent`,
`record_delivery`, `mark_undeliverable`, a burn; the books and the wX ledger exact afterwards) and Meteora DBC cloned from devnet up
to `back`'s converter gate: **56 PASS, 0 FAIL**. The v3 binaries load and init works. v3 CU per instruction and the run's details:
`docs/v9/rehearsal-sol-devnet.md` §6. Tests: 7 lib + 69 LiteSVM (`cargo test -p rook_core`). Rerun after §8.8: `init_registry`
with rook_backing's `["converter"]` PDA as initial converter (14,677 CU), and `back` → `convert` landed in the same run: **59 PASS,
0 FAIL**.

### 8.8 Initial converters at `init_registry`; rook_backing's one converter (2026-10-09, solana-migration.md §8 Q14)

`init_registry(owner, bitcoin_network, initial_converters: Vec<Pubkey>)`: the keys (at most `MAX_CONVERTERS` = 8 → `Full`;
none default, no duplicates → `BadInput`; may be empty) enter `Registry.converters` at once, with no `CHANGE_DELAY`, each with a
`ConverterSet { converter, ready_at: 0, allowed: true }` event. Safe because only the upgrade authority initializes (§8.1),
before anything is live, and the list is in the init transaction for anyone to read. Every later addition keeps propose →
accept after 7 days (§8.2); removal stays instant (owner or guardian). The `Registry` layout is unchanged (488 bytes): only the
instruction's arguments grew (the IDL gains `initial_converters: vec<pubkey>`; the canister's IDL fixture is regenerated).

rook_backing now signs `convert` with ONE program-wide PDA `["converter"]` of rook_backing (the per-collection
`["claimer", listing]` PDA stays the DBC fee claimer and is the `payer`), so the launch passes that single key at init and a new
collection needs no converter change. The deploy order is therefore: deploy both programs, derive rook_backing's
`["converter"]` PDA from its program id, `init_registry(owner, network, [that PDA])`.

Tests: `registry::init_registry_allows_its_initial_converters_at_once` (9 keys → `Full`; a default key and a duplicate →
`BadInput`; two keys allowed at once with nothing pending; a third still waits — `TooEarly` at once; removal instant; a full
list of 8 then refuses a proposal with `Full`). The harness gains `Env::bare_with` / `Env::with_converters` /
`init_registry_with_ix`. `cargo test -p rook_core`: 7 lib + 70 LiteSVM, green.

## 9. Audit fixes (2026-10-10)

The October audit (`docs/v9/audit-2026-10/a1-core-floor.md`, `a2-core-registry-wrap.md`, `a3-backing.md`; each report's
"Fixes (2026-10-10)" section maps finding → code → test). `cargo test -p rook_core`: 8 lib + 6 A1 + 7 A2 + 71 LiteSVM, green.

### 9.1 `pause_poster` stops an Inscriptions collection (A1-01)

`Listing::keeper_down` for `Inscriptions` is `poster_paused || reg.live_keeper().is_none()` (was the keeper switch only), so the
guardian's per-collection switch stops the frogs' floor (`post_floor` refused `NotKeeper`; `mint`, `convert`, `advance` refused
`Paused`) without `pause_keeper`, which stops every listing and drand proving. `prove_round` reads `live_keeper` directly and is
not stopped by a collection pause. The canister's mirror (`autobuyer/canister-rs/src/solread.rs` `keeper_down`) still has the old
rule: it only decides whether the canister tries to post; to be aligned when the canister is fixed.

### 9.2 A distribution round is not restarted while it is open (A2-01)

`Treasury` gains `round_started_at: i64`, appended after `bump` (1,389 → 1,397 bytes; earlier offsets unchanged).
`begin_distribution` refuses (`RoundOpen`) while some index with a weight is still owed a non-zero share
(`Treasury::round_owed`: `round_pot × weight / total`, bounded by the pot, 0 once paid), unless `ROUND_TIMEOUT` (1 day) has
passed since the round began, when a stale round may restart with the pot as it then is. Anyone can finish a round
(`distribute(i)` for each unpaid index), so honest cranks never wait for the timeout. `begin_distribution` with 2 modules:
11,342 → 11,404 CU.

### 9.3 `prove_round` refuses a future round (A2-02)

`round ≤ round_at(now)` (`RoundInFuture`): no pre-published randomness for a round that does not exist yet. The keeper still
chooses a live round's randomness until Q4 (the BLS check).

### 9.4 No refund of a pinned ticket whose round is proven (A2-04)

`refund` takes the PDA `["drand", ticket.round]` (checked by seeds, whether or not it exists; +1 account, +≈ 3.5k CU for the
derivation: 22,859 → 26,380). A pinned ticket whose round account exists is refused (`RoundProven`): its outcome is public, so
the refund was a free re-roll. Nobody is trapped: every earlier ticket is either resolvable (its round proven) or refundable no
later than this one (pinned earlier, round unproven), `refund` is permissionless (the unit goes to the ticket's owner) and a
refund at the cursor advances it, so the proven ticket then resolves.

### 9.5 Redemptions, deliveries and events (A2-03, A2-09)

- `redeem` on an Inscriptions listing refuses `destination == listing.custody` (`CustodyDestination`), as the EthNft branch
  refuses `custody_evm`.
- `record_delivery`'s `held` must point to the ticket's item (`held.item_plus_one == item.item_id + 1`, `WrongHeldMarker`).
- `Redeemed` carries the `destination`; `Refunded` carries `item: Option<u32>` (the item returned to the set; none for a pinned
  ticket) and the `destination`. Events stay non-load-bearing (§2.4).

### 9.6 wX token metadata (Metaplex)

Without it wallets show a wX as "Unknown token". `init_wrapper(meta)` now creates the mint's Metaplex Token Metadata (classic
program `metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s`, `create_metadata_accounts_v3` built by hand in `src/metadata.rs`):
`name` ≤ 32 bytes ("Wrapped Milady"), `symbol` ≤ 10 (`deployments/launch/nft-pilot.json`: wMILADY … wBFROG … wNATCAT), `uri` ≤ 200 (may be
empty and set later), no creators, 0 royalty, `is_mutable = true`. Refused otherwise (`BadMetadata`). The wrapper PDA signs as
mint authority **and is the update authority**; `set_wx_metadata(meta)` (owner only, `update_metadata_account_v2` signed by the
wrapper PDA) changes name, symbol and uri. Why the wrapper PDA rather than the registry owner's key: the right to change a wX's
identity then follows `Registry.owner` through `propose_owner`/`accept_owner` (read on every call, like every owner power),
with no separate Metaplex hand-over to forget, and no key outside the registry can rename a wX or point its image elsewhere.

Accounts added to `InitWrapper`: `metadata` (the Metaplex PDA, checked by seeds) and `token_metadata_program`. Cost:
`init_wrapper` 76,604 CU in all (LiteSVM); the metadata account is 607 bytes, 0.0051156 SOL of rent plus Metaplex's protocol
create fee of 0.01 SOL (held in the account), so the owner pays 0.0518 SOL for the whole `init_wrapper`. `set_wx_metadata`:
35,930 CU. Test: `wrap::wx_metadata_at_init_and_the_owner_updates_it`.

### 9.7 What `Vault.creator` means (A3-01, cross-ref)

rook_backing's `back` now stores DBC's `pool.creator` at backing (the coin's graduation for Rook); the wake desk's `reclaim`
credits that key. DBC lets the creator role move freely until then (`transfer_pool_creator`), so "the requester's own coin" means
"a coin whose creator role the requester held when it was backed". Bounded by one bond per coin (`Spent`).

### 9.8 Shared layouts for rook_backing

rook_backing now reads four more rook_core accounts (`register_partner`, `back`). `rook_layouts` gains `listing` (discriminator,
id), `module` (discriminator, `listing`, `wrapper`, `vault`, `params.conv_bps` at 151, `accepted` at 207, length 957),
`module_vault` (discriminator), `wrapper` (discriminator, `listing`, `wx_mint`) and `core_seeds`; `state::layouts_tests`
serializes the real structs and fails if any of these moves.

### 9.9 Smaller

- `price()` uses `elapsed()` for every fill interval (A1-04), as its final term did.
- Interface changes for callers: `init_wrapper` takes `meta` and two more accounts; `refund` one more account; new
  `set_wx_metadata`; `Treasury` 8 bytes longer; new errors appended after `NotUpgradeAuthority` (`RoundOpen` … `BadMetadata`; earlier codes
  unchanged). The canister builds none of the changed instructions and decodes no changed account; its IDL fixture
  (`autobuyer/canister-rs/tests/fixtures/solana/rook_core.json`) was regenerated from this build.
- Sizes: SBPF v2 922,936 → 956,624 bytes; v3 901,880. CU of the unchanged paths moved by ≤ 80 (`redeem` 26,591 → 26,671,
  `record_delivery` 14,231 → 14,242 with the new constraint).

### 9.10 Each collection's reserve is capped (operator decision 2026-10-10)

The surplus a module receives from `distribute` only pays for purchases above backing per unit and for shortfalls; it is never
paid to holders, so beyond a cushion it sat idle. `Treasury` gains `reserve_cap_bps: u32`, `pending_reserve_cap_bps: u32` and
`pending_reserve_cap_at: i64`, appended after `pending_ops_at` (1,449 → 1,465 bytes; earlier offsets unchanged).

- **The cap** of a module is `reserve_cap_bps × accepted / 10_000` (its accepted floor price per whole unit; saturating).
  `init_treasury` sets 20,000 (two floors: the next ten purchases each about 20 % above backing). `set_reserve_cap(bps)`
  (owner, `1_000..=200_000`, else `BadReserveCap`; no "no cap") proposes; `apply_reserve_cap` (anyone) applies it once
  `TREASURY_DELAY` (24 h) has passed (`NothingPending` / `TooEarly`), as `set_shares` / `apply_shares`.
- **`distribute(i)`** computes the share `r` as before, then: no accepted floor (`accepted == 0`) → all of `r` to the surplus;
  else `to_surplus = min(r, cap − surplus)` (saturating), and the rest goes to `treasury.ops` by a direct lamport move from the
  treasury account (as `allocate` pays ops). With no ops address set the rest stays in `reserves_pot` for a later round. Index
  `i` is marked paid either way (a call moving nothing succeeds), so `round_open` cannot stay true because of a capped module.
  `total_to_reserves += to_surplus`, `total_to_ops += to_ops`. `ReserveDelivered` gains `to_ops` and `kept` (`amount` is what
  reached the surplus). New events `ReserveCapProposed { bps, ready_at }`, `ReserveCapSet { bps }`.
- **Accounts of `distribute`**: `treasury` (w), `module` (w), `module_vault` (w), `ops` (w, optional: `treasury.ops` when set,
  else the program id). `ops` is required only when something is paid to it (`BadInput` if missing or not `treasury.ops`).
- Invariants kept: treasury lamports − rent ≥ `reserves_pot` after every step; lamports leave only to registered modules' vaults
  and to `ops`. Tests: `treasury_wake` (under the cap, partial, at and over the cap, no floor, no ops with the round closing,
  the 24 h change and the bounds). CU of `distribute` (SBPF v2): two collections under the cap 17,383 → 18,107; one collection 17,440 (at the cap, kept) to 18,556 (partial: surplus + ops).

### 9.11 The holder raffle (operator decision 2026-10-10)

Full contract (accounts in order, arguments, hashing, the weight rule, the canister's cadence): `raffle-design.md`. Code:
`state/raffle.rs`, `instructions/raffle.rs`, `distribute` in `instructions/treasury.rs`.

- **Funding.** `distribute(i)` splits what exceeds collection `i`'s reserve cap: `⌊over × raffle_bps / 10_000⌋` to the
  collection's `Raffle` (a direct lamport move from the treasury account; `raffle.pot`/`total_in` and
  `treasury.total_to_raffles` grow; event `RafflePotFunded`), the rest to ops as in §9.10 (kept in the reserves pot without an
  ops address). `distribute` takes one more account, last: the raffle PDA `["raffle", module.listing]`, whether or not it
  exists; if it is not initialized the raffle share is 0 (all to ops, as before), and any other account is refused
  (`WrongCollection`), so an existing raffle cannot be skipped. `Treasury` gains `raffle_bps: u16` (5,000 at `init_treasury`),
  `pending_raffle_bps: u16`, `pending_raffle_at: i64`, `total_to_raffles: u64`, appended (1,465 → 1,485 bytes; earlier offsets
  unchanged). `set_raffle_bps(bps ≤ 10_000)` (owner; `BadRaffleBps`) + `apply_raffle_bps` (anyone, after `TREASURY_DELAY`).
- **Prize.** `fund_prize` (anyone) re-syncs `pot` to the raffle's lamports above rent, requires a fresh floor and the poster
  up, prices one whole wX at `mint_price` and runs `Module::mint(price, ONE)`, the public mint's own book-keeping (spreads,
  hourly bucket, public cap, `max_eth_backed`; volume counted), paying the module vault from the pot and minting ETH-backed to
  the raffle's wX account through `module_mint`. No converter path, no bypass. Refused while ≥ 1 whole wX is held
  (`PrizeHeld`) or `pot < price` (`PotTooSmall`).
- **Draw.** `commit_raffle` (keeper): phase Idle, ≥ 7 d since the previous commit, a prize held, `snapshot_slot` in the past,
  ≤ 432,000 slots old and newer than the previous one; pins `round_at(now) + 20`. `draw_raffle` (anyone) once that
  `DrandRound` exists: `ticket = u128(keccak("rook-raffle-draw" ‖ randomness ‖ raffle ‖ round)[0..16]) mod total_weight`.
  `claim_raffle` (anyone): `cum_start ≤ ticket < cum_start + weight` and a sorted-pair proof of
  `keccak(keccak("rook-raffle-leaf" ‖ owner ‖ cum_start ‖ weight))`; one whole wX to the owner's ATA (created, claimer
  pays). `expire_raffle`: drawn and unclaimed 30 d → the prize stays as the next round's; committed and undrawn 7 d → cancelled.
- **Invariants** (checked after every step in the tests): treasury lamports − rent ≥ `reserves_pot`; module vault ≥ books;
  `wX supply = (held − open_tickets)·ONE + eth_backed` (the prize is an ordinary holder's ETH-backed unit); raffle
  lamports − rent ≥ `pot`.
- **Trust.** The program checks only the proof; the snapshot's correctness is the keeper's (the canister publishes it). Until
  the drand BLS check is ported (Q4) `prove_round` is keeper-only too, as for redemptions.
- **Interface changes for callers.** `distribute` one more account (the canister's builder now passes it); `Treasury` +20
  bytes (the canister's decoder and `TREASURY_BYTES` updated; `ops/solana/fund-status.mjs` and `web-rook/src/rook/sol/fund.ts`
  `LISTING_CEREMONY_SIZES` 1,465 → 1,485); errors appended after `BadReserveCap` (`BadRaffleBps` … `RaffleNotExpired`).
- **Cost.** CU (SBPF v2): `init_raffle` 32,001, `fund_prize` 39,706, `commit_raffle` 7,686, `draw_raffle` 4,639,
  `claim_raffle` 42,351 (3 proof nodes, ATA created), `expire_raffle` 3,464; `distribute` with the raffle split 21,619
  (20,733 when the raffle does not exist yet). Program size: SBPF v3 1,033,336 bytes, v2 1,091,720.
- **Tests.** `tests/suite/raffle.rs` (10): the overflow split (with and without ops; under the cap; an impostor raffle
  account refused), no raffle → all to ops, the 24 h `raffle_bps` change and bounds, `fund_prize` at the mint price and its
  refusals (pot, stale, poster paused, hourly bucket, prize held), the full cycle with a 5-holder tree (keeper-only commit,
  unproven round, wrong leaf / wallet / weight / proof refused), every ticket boundary (first, last, a middle leaf's first and
  last with its neighbours refused) and a single holder, the commit rules (prize, inputs, snapshot age and order, cadence,
  paused keeper), expiry after 30 d and cancellation after 7 d, and treasury overflow → prize → winner end to end. Unit tests in
  `state::raffle` (leaf/tree, ticket range and keying, sizes).
