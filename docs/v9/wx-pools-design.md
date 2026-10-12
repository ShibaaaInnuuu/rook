# wX/SOL pools — design (2026-10-10)

Operator decision (2026-10-10): every collection gets a wX/SOL pool on Meteora, opened at its first graduation, LP locked
forever, fees to Rook's treasury, routed by Jupiter; every later graduation deepens it. Seed = "the 25 % reserve".

**Status: design + spike. Implementation waits for one decision (§3): the reserve cannot go into the pool as written without
breaking the backing invariant. §3 explains why and proposes option A; §4–§10 assume A and do not change if the operator
picks B or C (only where the pool's SOL comes from changes).**

Evidence: `solana/tests/q2_wx_pool.rs` (spike, mainnet DAMM v2 0.2.4 bytes in LiteSVM; `cargo test -p rook_backing --test
q2_wx_pool -- --nocapture`). Tags: **[M]** measured in the spike, **[R]** read in this repo, **[W]** from Meteora/Jupiter docs,
not checked here.

## 1. What the module does today [R]

`back` (rook_backing) withdraws a coin's DBC migration fee `L` (50 % of the raise: 42.5 SOL on the 85 SOL tier) and calls
rook_core `convert(L)`. `convert` mints `U = L / g` wX units (`g = gradPrice = max(price, upTarget) × (1 + conv_bps)`), **all
of them into the coin's backing vault**, and `intake` books the lamports: `reserve_bps` (25 %) as `reserve` (with 25 % of the
units as `reserve_units`, never advanced: the canister's buy signal is `netUnits = live − transit − reserve_units`), and 75 %
as `backing` (convertible into NFTs).

The reserve units are not a separate stock: they are a label on 25 % of the units the coin's vault holds. The reserve SOL is
what backs them. Invariants (rook-core-as-built §9.11):

- `wX supply = (held − open_tickets) · 10⁹ + eth_backed` — every wX is an NFT in custody or a claim on the module's books;
- `module vault − rent ≥ backing + reserve + surplus + revenue`;
- a burn releases exactly its tier's backing per unit and pays at most that (`release`: `paid ≤ rel`).

## 2. Why "reserve SOL + reserve units into the pool" does not hold

Moving the reserve lamports `0.25 L` out of the module into the pool, together with `0.25 U` units, counts the same value
twice. The units still exist and are still burnable at the module; the SOL that backed them is gone. Anyone buys the pool's
units for SOL and burns them, and the module pays them out of the convertible backing that belongs to every other holder:
backing per unit falls from `g` to `0.75 g` for everyone. The pool would look like `0.5 L` of value (`0.25 L` SOL plus `0.25 U`
units at `g`) on `0.25 L` of real money. Keeping the units outside the books (a "pool tier" not counted in `eth_backed`) fails
the same way: wX is fungible, so a unit that leaves the pool can't be told apart from any other.

**Rule that follows:** every wX unit in the pool must be backed in the module like any other unit, so **the SOL side of the
pool has to be SOL that no unit claims**. A graduation's pool slice of value `V` therefore has to be split: `V/2` stays in the
module as the backing of the units that go into the pool, and `V/2` goes into the pool as SOL. This is the case the operator
asked me to propose before building ("the reserve SOL stays in the module and only freshly minted wX plus SOL from elsewhere go
to the pool").

## 3. The decision: where the pool's value comes from

The wX supply invariant and the burn rule hold under all three options below, because the module's books don't change: the
pool is just one more holder of ordinary ETH-backed units. What changes is **who pays for the locked liquidity**. A pool
that's locked forever, with its fees going to the treasury, is money taken out of someone's hands for good.

### Option A (recommended): the pool slice comes out of the migration fee before `convert`

With `q` = the pool share of `L` (25 %, the reserve's share):

| per graduation | today | option A |
| --- | --- | --- |
| SOL into the pool (no unit claims it) | — | `S = q·L/2` = 12.5 % of `L` |
| lamports converted by `convert` | `L` | `L − S` = 87.5 % |
| units minted | `U` | `(L − S)/g` = 0.875 `U` |
| — to the pool | — | `W = S/g` = 0.125 `U` (backed in the module) |
| — to the coin's vault | `U` (25 % of them labelled reserve) | 0.75 `U` |
| module convertible backing (NFT buying power) | 0.75 `L` | 0.75 `L` (unchanged, with `reserve_bps` = 1,429) |
| module reserve | 0.25 `L` | 0.125 `L` = exactly the backing of the pool's `W` units |

In words: the reserve is split in half. One half stays in the module as SOL backing (never advanced) for the wX that goes
into the pool; the other half goes into the pool as SOL. Every wX unit stays backed at `g` and NFT buying power doesn't
change. **The cost: a graduated coin's vault gets 0.75 `U` instead of `U`.** On the 85 SOL tier the backing behind 10⁹
coins drops from 42.5 SOL of wX to 31.9 SOL of wX, and the 25 % that used to sit in the vault as SOL-backed reserve units now
sits in the locked pool. The coin's holders lose a claim they have today, and the treasury gets the pool's fees.

How it's built: all in **rook_backing** (`back` splits the fee; the pool's desk holds the slice; `open_pool` / `deepen_pool`
move it into DAMM v2). **rook_core is unchanged**, apart from `reserve_bps` going from 2,500 to 1,429 in the launch file for
modules not yet initialized (nothing is deployed: it's an `init_module` parameter, and only `ops/solana/launch-params.mjs` and
its buying-power line change). If `reserve_bps` stays at 2,500, NFT buying power drops to 0.656 `L` (`0.75 × 0.875`).

### Option B: coin holders keep everything, the treasury seeds the pool

`back` stays as it is. At the first graduation `open_pool` takes `T` lamports from the treasury: `T/2` mints wX through the
module's public `mint` (ordinary book-keeping, spreads and caps), and `T/2` goes in as SOL. Later deepenings come from the same
source. Holders are whole. The catch: the treasury only holds partner trading fees (curve + coin LP), which are small at a
first graduation, so pools start shallow or late. This needs a new treasury share (rook_core `Treasury` + `distribute`, which
the other agent is editing now).

### Option C: a smaller slice

Option A with `q` = 10 % (or any value): the coin's vault gets `1 − q`, and the pool gets `q·L/2` per side. One constant in
rook_backing.

**Ask:** A at `q` = 25 % (the operator's "the reserve seeds the pool", read so it can actually be built), or A at a smaller
`q`, or B. Everything below holds for all three.

## 4. Pool type: DAMM v2, full range, 0.5 % fixed fee, fees in SOL

| | DAMM v2 (cp_amm 0.2.4) | DLMM |
| --- | --- | --- |
| depth near the floor | constant product over a pool-wide price range fixed at creation; full range: ±1 % ≈ 0.5 % of a side **[M]** | bins concentrated around the floor: far deeper at first |
| when the floor moves | full range never strands | locked bins can't follow the floor: a ±35 % range (70 bins at 1 %) strands within months of normal NFT floor moves |
| permanent lock | native `permanent_lock_position` (DBC's migration uses it); liquidity added later can be locked too **[M]** | position lock up to a release point; not native-permanent **[W]** |
| fees | fixed base fee plus optional dynamic fee; `collect_fee_mode = 1` puts every fee in SOL **[M]**; Meteora keeps 20 % of trading fees **[M]** (599,927 of ≈ 750,000 lamports reached the LP) | dynamic fees, often higher capture |
| CPI | already declared in rook_backing (`declare_program!(cp_amm)`), already used by `sweep_lp_fees`; init 110 k CU, add 38 k, lock 6.6 k, claim 25 k **[M]** | bin arrays, a strategy call, more accounts and CU; no fixture in the repo |
| Jupiter | DAMM v2 is routed **[W]** | routed **[W]** |

**Choice: DAMM v2**, one position per collection, full range (`MIN_SQRT_PRICE..MAX_SQRT_PRICE`), base fee 0.5 % fixed (no
scheduler; the dynamic fee stays off at first), `collect_fee_mode = 1` (SOL only), timestamp activation, active at once. The
range is fixed forever when the pool is created, and wX's SOL price follows the NFT floor (several-fold moves in a year), so a
concentrated range would end up idle. `[P/2, 2P]` would give 3.4× the depth and `[P/4, 4P]` 2×, if the operator prefers depth
and accepts stranding.

**Depth is the module's job, not the pool's.** The pool is thin: on the 85 SOL tier (option A) there are 5.3 SOL a side, and a
0.1 SOL buy moves it ≈ 3.8 %. At 10.6 a side (the spike) it moves +1.9 % **[M]**, and at 21 a side a 1 SOL buy moves it
+9.6 % **[M]**. The module's mint and burn are the deep venue (default 5 units/hour each way, ≈ 250 SOL/hour at a 50 SOL floor,
at a spread of 0.15 % and up), and arbitrage ties the two together (§7). The pool exists so that wX has a market price, Jupiter
routes, and holders can exit in size without waiting for an NFT sale.

## 5. Who creates the pool, and the squatting problem

**Squatting [M].** DAMM v2's customizable-pool address is `["cpool", max(mint), min(mint)]`: one per pair, with any price and
any amounts. Once wX can be minted publicly (the first fresh floor), anyone holding a millionth of a unit creates the wX/SOL
customizable pool first at any price, and Rook's `initialize_customizable_pool` then fails ("already in use"). The spike does
this at 10× the floor. Public static configs (`["pool", config, mints]`) can be squatted the same way, one config at a time.

**Fix [M]:** a **dynamic config with `pool_creator_authority` = a rook_backing PDA**. Only that PDA can create a pool on it
(`initialize_pool_with_dynamic_config`, the instruction DBC's migration uses with its own authority). A stranger is refused,
and the pool address `["pool", rook_config, max, min]` belongs to Rook alone. One config serves every collection, because the
address includes the mints. Meteora creates configs (`create_dynamic_config` is operator-gated): **Rook has to ask Meteora for
one** (index, `pool_creator_authority` = rook_backing's `["pool_creator"]` PDA). Fallback if Meteora won't: an owner-pinned
list of public static configs, with `open_pool` taking the first one that's free. That's weaker, since a squatter has to take
them all, but it works.

**Who: a separate permissionless `open_pool(listing)`, not `back`.** `back` already has 26 accounts. Pool creation adds about
20, which goes past a legacy transaction (an address lookup table would be needed) and roughly 100 k CU more. Splitting it also
keeps a failed pool step (price out of band, config missing) from blocking a coin's backing. `back` only parks the slice in the
collection's desk (2 more accounts, ≈ +10 k CU), and the canister cranks `open_pool` / `deepen_pool` right after.

## 6. Price, lock, deepening

**Initial price = the graduation's own conversion price** `u = (L − S) · 10⁹ / units_minted`, which is what the pool's units
cost. So the pool holds `S` lamports and `S · 10⁹ / u` units, exactly balanced, with no value moving between the vault and the
pool. `u` sits within `back`'s bound (`≥ 0.9 × accepted × (1 + conv)` worth of units, audit A3-06). At the defaults `u` is
`mint_base × 1.03`, up to about 2.85 % above the module's mint price (base spread 0.15 %). The arbitrage this invites (mint at
the module, sell into the pool, down to `mint / (1 − 0.5 %)`) costs the locked position about `V · δ² / 8` ≈ 0.008 % of the
pool's value, inside the hourly mint cap. Initializing at the exact mint price would need rook_core's spread logic duplicated
in rook_backing; the gain isn't worth that. The spike opens at exactly the requested price (50.000000 SOL/wX) and leaves
≤ 0.01 % of either side behind **[M]**.

**Lock.** The position NFT is minted to DAMM v2's `["position_nft_account", nft_mint]`, owned by the desk PDA. The NFT mint is
a PDA `["pool_nft", listing]`, which signs its own creation. In the same instruction, `permanent_lock_position(all of it)`.
After that `remove_all_liquidity` fails (`InsufficientLiquidity`) **[M]**. rook_backing has no instruction that removes
liquidity, closes the position or moves the NFT, so the lock rests on DAMM v2's permanent lock plus our code having no exit.

**Deepening.** Each later `back` parks a new slice in the desk. `deepen_pool(listing)` adds it at the **pool's current price**
(DAMM v2 adds in proportion), but only if that price is within ±3 % (`DEEPEN_BAND_BPS = 300`) of the reference: the slice's own
`u`, kept in the desk with its time, while ≤ 3 h old; after that, the module's accepted floor, which must be fresh. This keeps
someone from sandwiching the add at a price they pushed: inside ±3 % the cost is about `δ² ×` the amount added. Liquidity is
limited by the scarcer side, the rest stays in the desk for the next round, and then `permanent_lock_position(delta)`. Spike:
add 38 k CU, lock 6.6 k; the pool's whole liquidity is the locked position afterwards **[M]**.

## 7. The pool and the module's spreads

The module mints at `m = mint_base × (1 + s_mint)` and burns at `b = min(price, burnTarget) × (1 − s_burn)`, capped by
backing per unit. With pool fee `f`, arbitrage pays when the pool price `p` leaves `[b / (1 + f), m / (1 − f)]`: above the top,
mint and sell into the pool; below the bottom, buy from the pool and burn. Each arbitrage is bounded by the hourly buckets
(`mint_per_hour`, `burn_per_hour`, 5 units by default), the public cap (10 net units) and `max_eth_backed`. Inside the band
there's no arbitrage and the pool trades freely. Arbitrage profit comes out of the locked position (LVR), not out of the
module: mint and burn are book-neutral by construction (`intake` / `release`). So **the books and invariants don't depend on
the pool's price.**

## 8. Pause and stale floor

- Stale floor or poster down: `convert` refuses, so `back` waits (as today), and `deepen_pool` refuses without a fresh
  reference. The pool keeps trading. DAMM v2 has no pause Rook could use, and none is needed. Mint closes, so the pool can rise
  above the old band without an arbitrage pulling it back.
- Poster switch on (`kd`): burns stay open at `pause_floor` (and at backing once escaped), so arbitrage still holds a floor
  under the pool price.
- Listing paused: `convert` refuses `ListingPaused`, so nothing new reaches the pool. The existing pool and fee sweeps
  continue.
- Guardian or Meteora action: Meteora's operator can `set_pool_status` (disable) and `update_pool_fees` on any pool **[R,
  IDL]**. This is a Meteora trust assumption, the same one the coin pools already carry (rook-backing-as-built §6).

## 9. Instruction contract (rook_backing; option A)

New state (rook_backing, new files `state/pool.rs`, `instructions/pool.rs`):

- `WxPool` PDA `["wx_pool", listing]`: `listing`, `pool` (DAMM v2), `position`, `nft_mint`, `config`, `bump`, `nft_bump`,
  `pending_lamports`, `pending_units`, `ref_price` (lamports per whole wX) + `ref_at`, totals `lamports_in`, `units_in`,
  `fees_swept`, `opened_at`. Created by `register_partner` (or lazily by the first `back`).
- desk `["pool_desk", listing]`: system-owned and data-less (like the claimer). It holds the pending SOL, owns the position NFT
  account, signs DAMM v2 and pays the pool's rent (the cranker tops it up first).
- desk wX `["pool_desk_wx", listing]` (token account of `wx_mint`, authority = the desk), and a transient desk wSOL
  `["pool_desk_wsol", listing]`, created and closed inside an instruction.
- `["pool_creator"]`: the program-wide `pool_creator_authority` of Rook's DAMM v2 dynamic config (pinned in `Settings` as
  `damm_config` + `pool_creator_bump`, set by a new admin `set_damm_config`).

| Instruction | Signer | Accounts (in order) | Does / refuses |
| --- | --- | --- | --- |
| `back(min_units)` (changed) | anyone | as today + `wx_pool` (w), `pool_desk` (w), `pool_desk_wx` (w) | converts `L − S`, sends `S` lamports to the desk and `W = S·10⁹/u` units from the vault's mint to the desk wX (the coin vault keeps the rest); `pending_* += …`; `ref_price = u`, `ref_at = now`. The price bound applies to `L − S` |
| `open_pool` | anyone (cranker pays rent ≈ 0.03 SOL) | cranker (s, w), settings, partner, wx_pool (w), pool_desk (w), pool_desk_wx (w), pool_desk_wsol (w, init), pool_creator, damm_config, pool_nft_mint (w, PDA), position_nft_account (w), damm_pool (w), position (w), token_a_vault (w), token_b_vault (w), wx_mint, native_mint, module (r: accepted floor and freshness when `ref_at` is old), damm_pool_authority, token_program, token_2022_program, system_program, damm_event_authority, damm_program (≈ 25) | refuses if open (`PoolOpen`), nothing pending (`NothingPending`), reference stale (`Stale`), config unset (`NoDammConfig`); wraps `pending_lamports`; `initialize_pool_with_dynamic_config` (wX = token a, wSOL = token b, full range, 0.5 %, fees in b, `sqrt_price = √(ref_price)`); `permanent_lock_position(all)`; unwraps leftovers back to the desk; emits `WxPoolOpened { listing, pool, position, lamports, units, price }` |
| `deepen_pool` | anyone | cranker, settings, partner, wx_pool (w), pool_desk (w), pool_desk_wx (w), pool_desk_wsol (w, init), damm_pool (w), position (w), position_nft_account, token_a_vault (w), token_b_vault (w), wx_mint, native_mint, module (r), token_program, system_program, damm_event_authority, damm_program (≈ 20) | refuses if not open, nothing pending, or pool price outside ±3 % of the reference (`PoolOutOfBand`); `add_liquidity` (scarcer side), `permanent_lock_position(delta)`; emits `WxPoolDeepened` |
| `sweep_wx_pool_fees` | anyone | cranker, settings, wx_pool (w), pool_desk, pool_desk_wx, pool_desk_wsol (w, init), damm_pool, position (w), position_nft_account, vaults (w), mints, treasury (w, `= settings.treasury`), damm_pool_authority, token_program, system_program, damm_event_authority, damm_program (≈ 19) | `claim_position_fee` → desk wSOL → unwrap → **treasury only** (plain transfer, which rook_core counts as income), rent back to the cranker; the wX side is always 0 (fee mode 1); emits `WxPoolFeesSwept { listing, lamports }` |
| `set_damm_config` | admin (registry owner) | admin, settings (w), registry, damm_config | pins Rook's dynamic config after checking it: owner DAMM v2, `config_type = 1`, `pool_creator_authority = ["pool_creator"]` |

Estimated CU (the spike's DAMM v2 cost plus wrapper overhead as in `sweep_lp_fees`): `open_pool` ≈ 150 k, `deepen_pool` ≈
70 k, `sweep_wx_pool_fees` ≈ 60 k, `back` ≈ +10 k. All fit in legacy transactions. CPI depth: rook_backing → DAMM v2 → Token
(2 of 4). One thing the spike doesn't prove: that `add_liquidity` and `permanent_lock_position` accept a CPI caller (DAMM
v2 has a `CpiDisabled` error; DBC's migration shows that init and lock do). The first build step checks it.

Invariants the tests check after every step: the wX supply ledger; module vault ≥ books; desk lamports − rent ≥
`pending_lamports`; desk wX ≥ `pending_units`; position `unlocked = vested = 0`, `permanent_locked = pool.liquidity` (only
Rook's position); `vault_wx + pool slice = units minted`; every swept lamport lands in the treasury; and an arbitrage round
trip (pool → burn, mint → pool) never takes more from the module than backing per unit times units.

## 10. Canister, site, indexer

- **Canister** (`sol_line` crank): after a successful `back`, run `open_pool` if the collection's `WxPool.pool` is unset,
  else `deepen_pool`. On `PoolOutOfBand` / `Stale`, retry hourly (the slice waits in the desk, so nothing is lost).
  `sweep_wx_pool_fees` daily per open pool (skip if the position's `fee_b_pending` plus accrual is under 0.01 SOL). The canister
  decodes `WxPool` (new IDL fixture). One-time: the operator asks Meteora for Rook's dynamic config, then `set_damm_config`.
- **Site**: on the collection and coin pages, the wX pool (price vs the module's mint and burn, both sides' depth, fees swept
  to the treasury, a "liquidity locked forever" badge linking the position and the lock transaction), a Jupiter swap link for
  wX, and a 24 h chart. The coin page's backing figure follows option A (the vault holds 0.75 `U`).
- **Indexer**: follow every `WxPool` account (rook_backing, discriminator), decode DAMM v2 `EvtSwap2` on those pools for
  candles in SOL and USD (as it does for graduated coin pools), take TVL from the two vault balances, and read rook_backing
  events (`WxPoolOpened`, `WxPoolDeepened`, `WxPoolFeesSwept`). Serve `/v1/rook/wx/{listing}/pool` and candles.

## 11. Open

- The decision in §3 (A / A at a smaller `q` / B).
- Meteora: a dynamic config for Rook (`pool_creator_authority` = rook_backing `["pool_creator"]`). Without one, use the
  static-config fallback (§5).
- Fee 0.5 % and full range are proposals (§4).
- Jupiter routing thresholds for thin new pools **[W]**: Jupiter may not route a 5 SOL-a-side pool at once.
- Meteora's 20 % protocol share of trading fees **[M]** applies to Rook's pool as it does to the coin pools.
