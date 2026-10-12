# A3: rook_backing, rook_layouts and the Meteora DBC / DAMM v2 integration

| | |
|---|---|
| Area | A3 (see `README.md`) |
| Baseline | commit `f2c24e0`, branch `golive-2026-10-08` |
| Date | 2026-10-10 |
| Scope | `solana/programs/rook_backing/src/**` (admin, open_vault, back, redeem, sweep, dbc.rs, state, constants, errors); `solana/crates/rook_layouts`; the CPI into rook_core `convert` (interface and assumptions only; rook_core's side is A1); the DBC 0.2.1 (`f552f20`) and DAMM v2 0.2.4 (`a85c926`) instructions rook_backing calls, and the Meteora accounts it trusts |
| Meteora source read | `MeteoraAg/dynamic-bonding-curve@f552f20` (Release 0.2.1), `MeteoraAg/damm-v2@a85c926` (Release 0.2.4) |
| PoC file | `solana/programs/rook_backing/tests/audit_a3_backing.rs` (new; no source file modified) |
| Result | **No Critical, no High.** 0 Medium, 4 Low, 7 Informational. |

## 1. Threat model

**Assets.**

- The migration fee of every graduated coin: 50 % of the 85 SOL raise (42.5 SOL), held in the DBC quote vault until `back`.
- Each coin's `Vault` wX balance, which only `redeem_backing` may pay out.
- Rook's partner fees: DBC curve trading fees, the partner DAMM v2 position's fees, and the partner surplus and creation fees.
- Signing power. The claimer PDA `["claimer", listing]` is DBC's `fee_claimer`, holds the partner position NFT and pays `convert`. The converter PDA `["converter"]` is the one key in rook_core's `Registry.converters`.
- The `Vault` fields rook_core trusts: `creator`, `graduated` and `launched_at`, which the wake desk reads to return a bond.

**Actors.**

| Actor | Capability assumed |
|---|---|
| Anyone / cranker | calls `open_vault`, `back`, `sweep_*` with any accounts, any `min_units` and any timing; sends lamports to any address; creates DBC configs and pools freely, including configs that name Rook's claimer PDA as `fee_claimer` |
| Coin holder | redeems; buys or sells on DBC / DAMM v2; burns |
| Coin creator | signs DBC creator instructions: `transfer_pool_creator`, `claim_creator_trading_fee`, `creator_withdraw_surplus`, and the creator LP position |
| Malicious pool creator | launches under a look-alike config, or under a Rook config with chosen timing |
| Registry owner (admin) | `register_partner` and `register_partner_config`; trusted, but mistakes are in scope |
| Upgrade authority (rook_backing) | `init_backing` only; can replace the program, which is trusted |
| Meteora upgrade authority | can change DBC / DAMM v2 code and layouts; a trust assumption, considered for layout drift |

**Trust boundaries.**

- Accounts owned by DBC and DAMM v2 are read by fixed offset (`dbc.rs`) or passed through to those programs, which check them.
- rook_core's `Registry` is read by offset through `rook_layouts::registry`. rook_core reads `Vault` by offset through `rook_layouts::backing_vault`.
- Every CPI target is pinned: DBC, DAMM v2, rook_core, SPL Token and System.

## 2. Findings

| ID | Title | Severity | Location | PoC |
|---|---|---|---|---|
| A3-01 | `Vault.creator` is a snapshot of DBC's transferable `pool.creator`, taken when anyone opens the vault, so wake-bond credit can be handed over | Low | `open_vault.rs:61`; consumed by `rook_core/src/instructions/wake.rs:153` | `a3_01_vault_creator_is_a_snapshot_of_a_transferable_field`: passes, behaviour shown |
| A3-02 | `register_partner_config` does not enforce the LP-lock, fee-mode, creation-fee and leftover invariants the design relies on | Low | `admin.rs:113-129`; fields parsed but unused in `dbc.rs:33-50` | `a3_02_register_partner_config_accepts_configs_outside_the_rook_invariants`: passes, 255,835,938,175 coin base units stranded in the claimer |
| A3-03 | `Partner` pins the rook_core accounts without checking them and can never be changed; configs can never be removed (at most 8) | Low | `admin.rs:75-92`, `admin.rs:126-128`, `state.rs:24-48` | analysis (fails closed or misroutes; see text) |
| A3-04 | No claim path for the partner surplus, the partner creation fee, coins that land in the claimer, or lamports donated to the claimer | Info | `lib.rs` (instruction set), `sweep.rs:102-105, 205-207` | `a3_03_partner_surplus_has_no_claim_path`: passes; surplus measured as 0 |
| A3-05 | Fixed-offset reads of Meteora accounts depend on DBC 0.2.1's layout; the `version` byte is not checked | Info | `dbc.rs:78-129` | offsets verified against source (§4) |
| A3-06 | `back`: the cranker chooses the moment of conversion and passes any `min_units`, so it picks the worst legal grad price for the coin's holders (up to ≈20–33 %) | Low | `back.rs:109, 166-218` | analysis |
| A3-07 | Redemption is pro rata over the live supply, which includes the coins locked forever as DAMM v2 liquidity | Info | `redeem.rs:36-49` | covered by `sound_redeem_binds_vault_mint_and_holder` |
| A3-08 | Pre-funded PDAs: a donation to `claimer_wsol` becomes backing (and the cranker is refunded rent it never paid); a donation to the claimer is stranded | Info | `back.rs:56-65, 141-162`; `sweep.rs:24-43` | `sound_prefunded_claimer_pdas_do_not_block_back`: passes |
| A3-09 | `ROOK_CORE_ID` and rook_backing's `declare_id!` are local development keys and must change in lockstep with the reserved ids | Info | `constants.rs:5`, `lib.rs:17`; `rook_core/src/state/wake.rs:76` | analysis (fails closed) |
| A3-10 | The documentation overstates three invariants: the claimer "ends every instruction at 0 lamports", the LP fees are "OnlyB", and the vault records "who launched the coin" | Info | `docs/v9/rook-backing-as-built.md` §2; `sweep.rs:205`; `rook_layouts/src/lib.rs:56`; `state.rs` | the A3-01, A3-02 and A3-08 PoCs |
| A3-11 | `back` takes 26 accounts in a legacy transaction, leaving about 165 bytes of headroom | Info | `back.rs:19-104` | arithmetic (§A3-11) |

---

### A3-01: `Vault.creator` is a snapshot of a transferable DBC field, taken when anyone opens the vault (Low)

**Location.**

- `programs/rook_backing/src/instructions/open_vault.rs:61` (`v.creator = pool.creator;`)
- `crates/rook_layouts/src/lib.rs:56` (doc comment: "who launched the coin")
- Consumer: `programs/rook_core/src/instructions/wake.rs:153` (`require!(v.creator == r.requester)`)

**Description.** DBC 0.2.1 `transfer_pool_creator` (`instructions/creator/ix_transfer_pool_creator.rs`) lets the current creator set `pool.creator` to any key. This is allowed in `PreBondingCurve` with no condition, and in `CreatedPool` for DAMM v2 with no condition. `open_vault` is permissionless and may run at any time, even long after graduation: the deploy-later launch backs coins that graduated before rook_backing existed (`launch-day1-solana.md` §2). The vault copies whatever `pool.creator` is at that moment and never refreshes it.

rook_core's wake `reclaim` returns a requester's bond when a graduated vault of the collection has `creator == requester` and `launched_at >= request.at`. The intended meaning is "the requester launched a coin". What the code actually checks is "the requester held DBC's creator role when somebody first opened the vault".

**Impact.**

- A requester R who launched nothing can have any graduated coin's creator C, launched after R's request, transfer the role to R before the vault is opened. R then reclaims the bond, which would otherwise go to the treasury when the wake was served. Each coin works once (`Spent`), and C has to cooperate, but the creator role of a graduated coin is cheap to give away (its creator fees are mostly already claimed).
- The other way round, a creator who transferred the role for a legitimate reason before the vault was opened (the genesis coin moves its creator to the fund wallet in the launch transaction) loses the credit.

The loss is bounded by the bond, so the severity is Low.

**Exploit.**

1. R calls `request` on listing L.
2. C launches a coin under a config of L, and it graduates.
3. C calls `dbc.transfer_pool_creator(new_creator = R)`.
4. Anyone calls `open_vault`, then `back`.
5. R calls `reclaim`.

**PoC.** `a3_01_vault_creator_is_a_snapshot_of_a_transferable_field`, which **passes** (the behaviour is shown). After the transfer, `Vault.creator == R` and `rook_layouts::backing_vault::parse(..).creator == R`. On a second coin the vault is opened first and the role is transferred afterwards: `Vault.creator` keeps the old creator while DBC's `pool.creator` has changed.

**Recommendation.** Decide what the wake desk should credit.

- To credit the launcher: the launcher is not stored by DBC once the role moves. Either record the creator only while `migration_progress == PreBondingCurve`, at the earliest moment (a Rook keeper opening every vault in the block after launch narrows the window), or accept the role-holder semantics.
- To credit the role-holder: re-read `pool.creator` at `back` (DBC's live value, owner and discriminator already checked) and store that. Change the `rook_layouts` and rook_core docs to "creator at backing".

Either way, document that the role is transferable. A2 should weigh this on rook_core's side.

---

### A3-02: `register_partner_config` does not enforce the invariants the design relies on (Low)

**Location.** `programs/rook_backing/src/instructions/admin.rs:113-129`. In `dbc.rs`, `parse_config` reads `leftover_receiver`, `partner_liquidity_percentage`, `creator_liquidity_percentage`, both permanent-lock percentages, `migration_fee_option`, `creator_trading_fee_percentage` and `migration_quote_threshold`, but nothing uses them. It does not read `migrated_collect_fee_mode` (offset 360), `pool_creation_fee` (offset 368), the two liquidity-vesting infos, `fixed_token_supply_flag`, the pre/post supplies or `token_update_authority`.

**Description.** The as-built page and spike Q1 rest on several properties:

- LP is permanently locked: "the claimer ... can only ever claim fees".
- DAMM v2 fees are SOL-only (OnlyB): `sweep.rs:205` says "nothing lands here".
- Pre-migration supply equals post-migration supply, so there are no leftover coins.
- The pool-creation fee is 0.

`register_partner_config` checks only `fee_claimer`, the quote mint, the migration option, the token type, `collect_fee_mode` (curve only), the activation type, a non-zero migration fee and a zero creator migration share. A config that breaks any of the properties above registers without complaint.

**Impact.** Each consequence needs an admin mistake (or a config built for another purpose), which is why this is Low.

| Config value | What happens | Recovery |
|---|---|---|
| `migrated_collect_fee_mode = 1` (BothToken) | `sweep_lp_fees` puts the coin half of the partner LP fees into the claimer's coin account | none: no instruction moves them |
| `migrated_collect_fee_mode = 2` (Compounding) | nothing is claimable | none |
| `partner_liquidity_percentage > 0`, or partner vesting | the claimer PDA holds removable liquidity | no instruction removes it |
| `creator_liquidity_percentage > 0` | the creator can pull liquidity, which contradicts "LP locked" | none |
| `pool_creation_fee > 0` | the creation fee is claimable only by the claimer | no instruction (A3-04) |
| `leftover_receiver` not the claimer, with leftover coins | those coins go to an arbitrary key, which may redeem them | none |

**PoC.** `a3_02_register_partner_config_accepts_configs_outside_the_rook_invariants`, which **passes**. A config with 50 % unlocked partner LP, DAMM `collect_fee_mode = BothToken`, a 0.1 SOL creation fee and a stranger as `leftover_receiver` registers. The test then launches a coin under it, graduates it, backs it, makes a 20 SOL DAMM buy and runs `sweep_lp_fees`: 255,835,938,175 coin base units are left in the claimer's ATA, and the 0.1 SOL creation fee sits in the DBC pool.

**Recommendation.** Make the on-chain check match the documented shape:

- `partner_liquidity_percentage == 0` and `creator_liquidity_percentage == 0`.
- Both vesting infos uninitialized (byte 184 and byte 200 equal 0).
- `migrated_collect_fee_mode == 0`.
- `pool_creation_fee == 0`, or a sweep for it.
- `leftover_receiver == claimer`.
- Optionally `fixed_token_supply_flag == 1` with pre == post supply.

Add each new offset to the layout test, which uses the real DBC bytes.

---

### A3-03: `Partner` pins unvalidated rook_core accounts and is immutable; configs cannot be removed (Low)

**Location.** `admin.rs:75-92` (`register_partner`), `admin.rs:126-128` (configs only ever pushed), `state.rs:24-48`.

**Description.** `register_partner` stores `module`, `module_vault`, `wrapper` and `wx_mint` exactly as the admin passes them. It checks only `params.registry`. It does not check that `listing` is a rook_core `Listing`, that `module.listing == listing`, that `wrapper == module.wrapper` or that `wx_mint == wrapper.wx_mint`. The `Partner` PDA is `init` on `["partner", listing]` and no instruction updates or closes it. The claimer derives from the listing, so a collection can never get a second `Partner`. Configs can only be appended, up to `MAX_CONFIGS = 8`, and never removed.

**Impact.**

1. **A wrong pin fails closed, but only an upgrade fixes it.** A wrong module, module vault, wrapper or wx mint makes every `back` for the collection fail inside rook_core (`WrongCollection` / `ConstraintAddress`). The 42.5 SOL fee of every graduated coin stays in DBC, claimable only through rook_backing's claimer signature, until rook_backing is upgraded. The program is upgradeable, so this is recoverable, but it is not an admin operation.
2. **Listing-order mistakes are silent.** The runbook relies on `register_partner_config` refusing a misordered listing (`launch-day1-solana.md` §2). That refusal happens only if the admin registers the Partner under the listing of the intended collection. If the admin registers by listing id (`PDA(["listing", id])`) and ids were assigned in the wrong order, the claimer still matches the day-1 config. Every check passes, and that collection's backing is converted into another collection's floor and wX. rook_backing has no notion of collection identity to catch this.
3. **No retirement.** A config that should be retired (for example after A3-02) cannot be removed. The ninth curve shape for a collection can never be added.

**Recommendation.**

- In `register_partner`, read rook_core's `Listing`, `Module` and `Wrapper` (owner = `ROOK_CORE_ID`, discriminators, through `rook_layouts`) and require `module.listing == listing`, `module.vault == module_vault`, `module.wrapper == wrapper` and `wrapper.wx_mint == wx_mint`. Also record the listing's collection identity (contract address or id) in the event, so the ceremony can verify the mapping.
- Add an admin `set_partner_accounts` (same checks), and `remove_partner_config` / `retire` that blocks new `open_vault` calls only, so existing vaults are not stranded.

---

### A3-04: No claim path for partner surplus, the partner creation fee, coins in the claimer, or claimer donations (Info)

**Location.** The program's instruction set (`lib.rs`); `sweep.rs:102-105`, `sweep.rs:205-207`.

**Description.** DBC pays the following only to a signature of `config.fee_claimer` (the claimer PDA), and rook_backing signs only `withdraw_migration_fee`, `claim_trading_fee` and DAMM `claim_position_fee`:

- `partner_withdraw_surplus`: 80 % of `quote_reserve − threshold`, split partner/creator.
- `claim_partner_pool_creation_fee`.

The claimer's coin ATA can receive leftover coins (it is `leftover_receiver`) and coin-side LP fees (A3-02), and no instruction moves them. Lamports sent to the claimer stay there (A3-08). The as-built page already lists surplus and leftover as open questions.

**Impact.** Measured on DBC 0.2.1 with the Rook curve, the surplus is **0**. Swaps cannot cross `migration_sqrt_price` (`virtual_pool.rs`: ExactIn requires `amount_left == 0` with `stop = migration_sqrt_price`; ExactOut requires `next_sqrt_price <= migration_sqrt_price`), so the reserve tops out at the threshold. The creation fee is 0 in the Rook config. The value at stake today is negligible; it becomes real only under A3-02 or a future DBC change.

**PoC.** `a3_03_partner_surplus_has_no_claim_path`, which passes. After graduation, `quote_reserve == THRESHOLD`, `is_partner_withdraw_surplus == 0`, and DBC refuses a non-claimer signer.

**Recommendation.**

- Fold `partner_withdraw_surplus` into `back`, as backing, or into `sweep_partner_fees`, to the treasury (mind A3-11 for `back`).
- Add a `sweep_creation_fee` if `pool_creation_fee` may ever be non-zero, or refuse such configs (A3-02).
- Add a permissionless `burn_claimer_coins`, which raises every holder's backing.

---

### A3-05: Fixed-offset reads of Meteora accounts depend on DBC 0.2.1's layout (Info)

**Location.** `dbc.rs:78-129`.

**Description.** All offsets were checked against `state/config.rs` and `state/virtual_pool.rs` at `f552f20` (§4). `parse_*` checks the owner, the exact length (1048 / 424) and the discriminator, so a `ConfigWithTransferHook` (1128 bytes) and a `TransferHookPool` (its own discriminator) are refused.

The residual risk is a Meteora upgrade that keeps the account size, repurposes padding or reorders fields, and keeps the discriminator. `open_vault` would then read `config`, `base_mint`, `activation_point` or `creator` from the wrong bytes, and `back` would read `is_migrated` from the wrong byte. `back` and the sweeps are still re-checked by DBC itself, so funds are not exposed. The risky read is `open_vault`'s `pool.config ∈ partner.configs`: a mis-read can at worst refuse a pool, or accept one only if 32 attacker-controlled bytes happen to equal a registered config key, which is not attacker-reachable in today's layout.

**Recommendation.**

- Also require `PoolConfig.version` (byte 236) to equal the value DBC 0.2.1 writes.
- Keep the ProgramData-hash monitoring and the rerun of these tests that the as-built page proposes, and pause `open_vault` / `back` (an admin flag) on a Meteora upgrade until the rerun passes.

---

### A3-06: The cranker of `back` chooses when the conversion happens and passes any `min_units`, so it picks the worst valid price for the coin's holders (Low)

This finding answers the cross-area lead from A1.

**Location.**

- `back.rs:109` (`min_units` is the cranker's argument) and `back.rs:166-218`.
- rook_core `state/floor.rs:148` (`fresh`), `:268` (`up_target`), `:287` (`grad_price`), `:409` (`priced`), `:509` (`convert`).

**Description.** `back` is permissionless, and `min_units` comes straight from the cranker (0 or 1 is allowed). rook_core's `convert` has no other price protection. It converts at:

`gradPrice = ⌈max(price, upTarget) × (1 + conv_bps)⌉`

- `price` is the module's 24-hour fill TWAP, clamped to `accepted × (1 ± fill_band_bps)`, with `fill_band_bps ≤ 10 %`.
- `upTarget = min(target, max(day_anchor × 1.2, accepted))`, where `target` is the keeper's raw, unclamped last posted price.
- `conv_bps ≤ 20 %`.

`convert` first requires `fresh`, the poster up, and the listing not paused.

What the cranker can and cannot do:

| | Possible? | Why |
|---|---|---|
| Convert at a stale or low-confidence price | **No** | `priced()` refuses unless the floor is fresh: posted less than 3 h ago with `confidence ≥ min_confidence_bps`, and the keeper not down (`Stale` / `Paused`) |
| Move the price itself | **No** | `accepted`, `target` and the fills are written only by the keeper and canister flows (A1 and A5 scope) |
| Choose the moment | **Yes** | every legal price between about `accepted × (1 − band)` and `max(accepted, anchor × 1.2)`, times `(1 + conv)`, is reachable |

The cranker can wait for a keeper post whose `target` spikes toward the top of the day band, then call `back` with `min_units = 1`. It can also bundle the permissionless DBC `migration_damm_v2` with `back` in one transaction, so Rook's keeper cannot get in first. It cannot postpone conversion against Rook's will: anyone, including Rook's keeper, may crank first whenever the moment is good.

**Impact.**

- **Holders of that coin.** Their backing is minted at up to about (1.2 / (1 − band)) − 1, roughly 20–33 %, fewer wX units than the most favourable legal moment. On 42.5 SOL that is up to about 7–10 SOL of backing value.
- **Where the value goes.** It is not stolen. The SOL goes to the floor module and backs the existing wX, so it is a transfer from the coin's holders to wX holders.
- **Attacker profit.** Only indirect: the attacker would need to hold wX, and its gain is diluted across all of wX.
- **Design intent.** Conversions intentionally price at `upTarget` (EVM parity, A-10). The cranker only selects among prices the trusted keeper posted.

Severity is Low: the price range is bounded and keeper-set, and there is no direct profit.

**Recommendation.** Deriving `min_units` from the module's current state inside the same transaction is circular: it is the same `gradPrice`, so it bounds nothing. A useful bound needs an independent reference. Options, in order of preference:

1. rook_backing reads rook_core's `Module` (owner, discriminator, `module.listing == partner.listing`, through a `rook_layouts::module` reader tested on rook_core's side, as is done for `Registry`). It then floors the slippage with
   `min_units_eff = max(min_units, ⌊lamports × 10⁹ / (accepted × (1 + conv_bps)) × (1 − TOL)⌋)`,
   where the reference is the **accepted floor without the target premium**. `TOL` could be about 5 % or a `Settings` value. This refuses a conversion while the keeper's `target` sits far above `accepted`, and the next post retries it.
2. Or give `convert` a converter-specific price of `max(price, accepted) × (1 + conv)`, ignoring `upTarget`. This changes the economics, so it is A1 / operator territory.
3. Or restrict `back` to the keeper (or `Registry.keeper`) for a grace period after `is_migrated` (for example 1 h, using `finish_curve_timestamp` at pool offset 344), then open it to anyone for liveness. Rook's keeper should crank promptly in any case.

Restricting `back` permanently is not recommended: it would make every coin's floor depend on one key.

**Liveness.** While rook_core refuses (`Stale`, `Paused`, `ListingPaused`, `OverBacked`), `back` reverts and the fee stays safely in DBC. Meanwhile `redeem_backing` stays closed for that coin, because `graduated` is false. Surface `OverBacked` in monitoring, since it can block backing indefinitely.

---

### A3-07: Redemption is pro rata over the live supply, including coins locked forever in the DAMM v2 pool (Info)

**Location.** `redeem.rs:36-49`.

**Description.** `payout = ⌊vault_wx × amount / mint.supply⌋`. The supply includes the coins in the DAMM v2 pool vault, which back permanently locked LP and are never burned. Their pro-rata share of the wX can only be realised by someone buying them out of the pool. Rounding is down and per-coin backing never decreases (the property is tested). The last full-supply redeemer would take the dust, which cannot happen in practice.

DBC revokes the SPL mint authority at launch, and the mint has no freeze authority; the PoC checks both. So the supply only ever decreases.

**Impact.** As designed and documented. The "backing per coin" a UI shows should use the live supply, not `coin_supply_at_backing`.

---

### A3-08: Pre-funded PDAs: donations are absorbed and the cranker is refunded rent it never paid (Info)

**Location.** `back.rs:56-65, 141-162`; `sweep.rs:24-43`.

**Description.** Anchor `init` accepts a `["claimer_wsol", listing]` address that already holds lamports: it allocates and assigns without charging the cranker. SPL `InitializeAccount3` then counts `lamports − rent` as the wSOL amount. `back` converts that amount with the fee, and a sweep sends it to the treasury. `rent_back = lamports − amount` (the rent reserve) is then paid to the cranker, who paid nothing. Lamports sent to the claimer itself stay there forever (no instruction spends them), which breaks the documented "ends at 0 lamports".

**Impact.** No DoS and no theft: the donor's own money is routed to backing, the treasury or the cranker. Two accounting details follow. `lamports_backed` and `lamports_swept` include the donations. `Partner.lamports_backed` no longer equals the sum of DBC migration fees.

**PoC.** `sound_prefunded_claimer_pdas_do_not_block_back`, which passes. With a 3 SOL donation to `claimer_wsol` and 1 SOL to the claimer, `back` succeeds, `lamports_backed = 42.5 SOL + 3 SOL − rent`, the claimer keeps 1 SOL, and the cranker gains exactly the rent reserve.

**Recommendation.** Optional:

- Record the fee DBC transferred (the wSOL delta or the `withdraw_migration_fee` event), not the account balance. The difference is the donation.
- Doc fix (A3-10).

---

### A3-09: Program-id constants are local development keys (Info)

**Location.** `constants.rs:5` (`ROOK_CORE_ID`), `lib.rs:17` (`declare_id!`), and rook_core's `ROOK_BACKING_ID` (`rook_core/src/state/wake.rs:76`).

**Description.** The day-1 configs pin claimer PDAs derived from the **reserved** rook_backing id. If the deployed binary's `declare_id!` or `ROOK_CORE_ID` differs from the reserved ids, the failure is closed rather than open:

- Anchor refuses instructions on a program-id mismatch.
- `init_backing` would pin the wrong registry and treasury, so every admin call fails `NotRegistry`.
- The claimer PDAs would not match the configs (`WrongFeeClaimer`).

Fees stay in DBC until a corrected build is deployed.

**Recommendation.** Before deploy, have the ceremony assert that the binary's embedded ids equal the reserved keypairs' public keys, for both programs and in both directions. A6 is cross-checking this.

---

### A3-10: Documentation overstates three invariants (Info)

The following statements are not enforced by the code:

- **"The claimer ends every instruction at 0 lamports"** (`rook-backing-as-built.md` §2). This holds only if nobody sends it lamports (A3-08).
- **"The migrated pool collects fees in SOL only (OnlyB), so nothing lands here"** (`sweep.rs:205`). This holds only because of the off-chain config (A3-02).
- **"The DBC pool's creator (who launched the coin)"** (`rook_layouts/src/lib.rs:56`, `state.rs` `Vault.creator`). The field is the role-holder at `open_vault` (A3-01).
- `launch-day1-solana.md` §2 says a wrong listing order makes `register_partner_config` refuse. That is true only for one way of registering (A3-03).

---

### A3-11: `back` has little transaction-size headroom (Info)

**Location.** `back.rs:19-104`.

**Description.** `back` takes 26 accounts. A legacy transaction carrying `back`, a compute-unit limit and a compute-unit price instruction comes to about 1,065 bytes out of 1,232:

- 28 keys: 896 bytes
- 1 signature: 65 bytes
- header and blockhash: 36 bytes
- the three instructions: about 68 bytes

That leaves room for roughly 5 more accounts. Adding `partner_withdraw_surplus` to `back` (A3-04: +3 or 4 accounts) still fits; anything larger needs a v0 transaction with a lookup table.

**Recommendation.** Keep the surplus in `sweep_partner_fees` rather than in `back`, or plan an address lookup table for the cranker.

## 3. Checked and found sound

**Ownership and identity of Meteora accounts**

- `parse_config` / `parse_pool` require owner == DBC, the exact length and the discriminator. A look-alike config created by an attacker that names Rook's claimer PDA as `fee_claimer` (DBC allows this) is never in `partner.configs`. `open_vault` refuses its pools with `NotRookPool` (PoC `sound_lookalike_config_with_rooks_claimer_is_refused`).
- `register_partner_config` binds a config to exactly one collection through `fee_claimer == claimer(listing)`. The claimer is unique per listing, so the same config cannot be registered under two partners. DBC never changes `fee_claimer` (written once in `PoolConfig::init`; no update or close instruction).
- `open_vault`: the `Vault` PDA is `["vault", coin_mint]`, so there is one vault per coin globally. The coin mint is the DBC base mint, which DBC creates as a signer keypair in pool creation, so a second pool for the same mint is impossible and a vault for a fake or foreign coin is refused. `base_mint == coin_mint` is checked, `launched_at = activation_point` and is non-zero. DBC sets `activation_point` only at pool creation (`get_current_point`; no other writer in 0.2.1), and registered configs require timestamp activation.
- Token type: registered configs require `token_type == SPL`. DBC refuses an SPL-path pool creation under a non-SPL config and the reverse, and transfer-hook configs and pools have a different size or discriminator. So every Rook coin is plain SPL, its mint authority is revoked at launch (`set_authority(MintTokens, None)`), and it has no freeze authority.

**`back`**

- It requires `is_migrated == 1` (stronger than DBC's "curve complete"). It is idempotent through `!vault.graduated`, and DBC's partner bit (0b100) also prevents a second withdrawal.
- Every account that matters is pinned: the pool and config to the vault, the quote mint, DBC's pool authority, the claimer by seeds, the converter by `Settings`, rook_core by address plus executable, and the registry, listing, module, module vault, wrapper, wx mint and vault wX to `Partner` / `Vault`.
- The CPI targets are fixed: `Program<DynamicBondingCurve>` and the hard-coded `ROOK_CORE_ID` / `token::ID` / `system_program::ID`. The claimer and converter seeds are only ever given to these fixed programs with fixed instruction data: DBC `withdraw_migration_fee(0)`, SPL `close_account`, System `transfer` to the cranker (rent only), and rook_core `convert` (discriminator constant, verified). Neither PDA can be made to sign for an attacker-chosen program or instruction.
- The transient wSOL account is created by this instruction (`init`, authority = claimer, SPL Token) and closed into the claimer within the same instruction, with the rent returned to the cranker. `units ≥ min_units` is re-checked against the vault's balance delta.
- The `convert` interface matches rook_core's `Convert` accounts and order (converter readonly signer; payer writable signer; registry, listing, module, module_vault, wrapper, wx_mint, dest, token_program, system_program) and its data `(u64 lamports, u64 min_units)`. The real-rook-core suite passes: `back` through the real `convert`.

**`redeem_backing`**

- The vault, coin mint and vault wX are pinned to the vault. `holder_coin` requires `authority == holder` (signer), so nobody else's coins can be burned. The wX mint is pinned to the vault's account, and the token programs are constrained to the accounts' owners.
- The payout is computed before the burn, on the pre-burn supply, and rounds down in u128. `amount == 0` and a zero payout are refused. A redeem against another coin's vault is refused (PoC `sound_redeem_binds_vault_mint_and_holder`).

**Sweeps**

- The destination is `settings.treasury` (`has_one`, settings by seeds); it cannot be redirected.
- DBC `claim_trading_fee` is checked by DBC (`is_partner_fee_claimer`, pool/config/vault has-one).
- DAMM `claim_position_fee` checks that `position.pool == pool`, that the pool's mints and vaults match, that the NFT mint matches and its amount is 1, and that the signer is the NFT owner or a delegate. rook_backing adds NFT owner == claimer, so the creator's position cannot be swept (existing test).
- DAMM v2 0.2.4 position delegates are set only by the NFT owner (the claimer never signs an approve).

**Admin and settings**

- `init_backing` is gated on the ProgramData upgrade authority and is a one-time `init` at `["settings"]`. `back` skips the seeds check on `Settings`, which is safe: `Account<Settings>` checks owner and discriminator, and only `init_backing` can create one, at a fixed PDA.
- Admin = rook_core `Registry.owner`: the address is pinned to `Settings.registry`, the owner program is rook_core, the discriminator is checked, and the `owner` field (offset 8) matches rook_core's struct (first field).
- The `rook_layouts::backing_vault` offsets equal the Borsh layout of `Vault` (259 bytes; unit test).

**Other**

- PDA seed families cannot collide. `"partner"/"claimer"/"claimer_wsol"/"vault"/"vault_wx"` + 32 bytes, and `"settings"/"converter"`, differ in length or prefix.
- Rent-state transitions: the claimer starts and ends at its prior balance, and the treasury and cranker only receive.
- Compute: `back` uses about 100k CU with the real `convert`; CPI depth is 2.

## 4. DBC 0.2.1 offsets, verified against `f552f20` source

| Account | Field | rook_backing offset | Source (8 + field offset in the `#[repr(C)]` struct) |
|---|---|---|---|
| PoolConfig (1048) | quote_mint / fee_claimer / leftover_receiver | 8 / 40 / 72 | ✓ |
| | pool_fees (80), partner and creator vesting (16 + 16), padding_0 [14], padding_1 u16 | 104..232 | ✓ |
| | collect_fee_mode, migration_option, activation_type | 232, 233, 234 | ✓ |
| | token_decimal, version, token_type, quote_token_flag | 235, 236, 237, 238 | ✓ (237 used) |
| | partner_perm_locked, partner_liq, creator_perm_locked, creator_liq | 239–242 | ✓ |
| | migration_fee_option, fixed_supply_flag, creator_trading_fee_pct, token_update_authority | 243, 244, 245, 246 | ✓ |
| | migration_fee_percentage, creator_migration_fee_percentage | 247, 248 | ✓ |
| | migration_quote_threshold | 264 | ✓ (swap_base_amount at 256) |
| VirtualPool (424) | config, creator, base_mint, base_vault, quote_vault | 72, 104, 136, 168, 200 | ✓ (volatility tracker 64 bytes) |
| | quote_reserve | 240 | ✓ (base_reserve 232) |
| | activation_point | 296 | ✓ (sqrt_price u128 at 280) |
| | is_migrated, migration_progress, migration_fee_withdraw_status | 305, 308, 311 | ✓ (pool_type 304, partner surplus 306, protocol surplus 307, leftover 309, creator surplus 310) |

## 5. Test results

Environment: `export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"`. The rook_backing SBPF v2 build in `target/deploy-sbpfv2` is newer than every source file at `f2c24e0`.

| Command | Result |
|---|---|
| `cargo test -p rook_backing --test audit_a3_backing -- --nocapture` | **6 passed** (A3-01, A3-02 and A3-03 demonstrate the behaviour; 3 "sound" checks) |
| `cargo test -p rook_backing` (unit + `backing` + `q1_meteora_dbc` + audit) | 1 + 9 + 1 + 6 passed |
| `cargo test -p rook_backing --features real-rook-core --test real_rook_core` | 2 passed |

Output from the PoCs:

- `A3-02: coin-side LP fees stranded in the claimer's coin account: 255835938175 base units`
- `A3-03: quote_reserve 85000000000 vs threshold 85000000000: total surplus 0`

There was no High or Critical finding, so no failing-PoC test was required. The A3 tests are written to pass while the behaviour is present; turn them into regressions (expect refusal) when the fixes land.

## Verification (independent, 2026-10-10)

Method: I re-read `admin.rs`, `open_vault.rs` and `back.rs` at `f2c24e0`. I rebuilt rook_backing with `cargo build-sbf --arch v2`
and ran `cargo test -p rook_backing --test audit_a3_backing`: 6 passed, with the same outputs (255 835 938 175 stranded base
units; surplus 0). I did not re-fetch the Meteora sources. The offsets in §4 are exercised end to end by the existing
`q1_meteora_dbc` / `backing` suites against the real DBC binary, and those suites pass.

| ID | Verdict | Final severity | Reason |
|---|---|---|---|
| A3-01 | CONFIRMED | Low | `open_vault.rs:61` copies `pool.creator` once, and `open_vault` is permissionless at any time. The loss is bounded by one wake bond per coin and needs the creator's cooperation. |
| A3-02 | CONFIRMED | Low | `register_partner_config` (`admin.rs:113-129`) checks only the 8 listed fields. `migrated_pool_fee.collect_fee_mode` is also unchecked at run time by `rook-ceremony` (a unit test only, `ceremony.rs:854`), so A6-05's tampered-file path reaches this consequence with no layer catching it. It stays Low because the admin is trusted and A6-05 is the place to fix it. |
| A3-03 | CONFIRMED | Low | `register_partner` stores the module, vault, wrapper and wx mint unchecked. A mismatch fails closed in rook_core's `Convert` constraints (`module.listing == listing`, `address = module.vault/wrapper`). The silent misrouting is the listing-order problem (A6-02). |
| A3-04 | CONFIRMED | Informational | Surplus measured at 0, creation fee 0. The leftover is about 12.8 coins per coin (A6-14). |
| A3-05 | CONFIRMED | Informational | Exact length and discriminator are checked. A same-size relayout is the residual risk. |
| A3-06 | CONFIRMED | Low | `min_units` is the cranker's argument, and `convert` prices at `max(price, upTarget)·(1+conv)`. With defaults (`conv_bps` 300, `fill_band_bps` 300) the legal spread is about 1.2/0.97 − 1 ≈ 24 %. Value moves to wX holders, not to the cranker. |
| A3-07 | CONFIRMED | Informational | As designed. |
| A3-08 | CONFIRMED | Informational | Demo reproduces it. The cranker gains exactly the rent reserve. |
| A3-09 | CONFIRMED | Informational | Fails closed. A6 confirms the swap covers all five places. |
| A3-10 | CONFIRMED | Informational | The doc statements are contradicted by the A3-01, A3-02 and A3-08 demos. |
| A3-11 | CONFIRMED | Informational | Arithmetic checked. |

No finding disputed or adjusted, and no new issue in A3's scope. The report's "no High/Medium" stands.

## Fixes (2026-10-10)

Paths under `solana/programs/rook_backing/`. `cargo test -p rook_backing`: 2 unit + **8 audit** + 9 backing + 1 spike, green
(before: 1 + 6 + 9 + 1); `--features real-rook-core --test real_rook_core`: 2/2. The A3-01/02/03/06 tests are now `fixed_*`
regressions that expect the new behaviour.

| ID | Status | Fix | Where | Test |
|---|---|---|---|---|
| A3-06 (+A1-08) | **Fixed** (the report's option 1) | `back` reads rook_core's `Module` (owner, exact layout through `rook_layouts::module`, `module.listing == partner.listing`) and applies `max(min_units, ⌊⌊fee·10⁹ / ⌈accepted·(1+conv_bps)⌉⌋·(1 − TOL)⌋)`, so a cranker's lower value is ignored; `accepted == 0` → `NoFloor`. `TOL` = `BACK_PRICE_TOLERANCE_BPS` = 10 %: `fill_band_bps ≤ 10 %` (rook_core `validate_params`), so the fill TWAP alone never trips it; it trips only when the target premium exceeds ≈ 11 % (the day band allows 20 %). A refusal leaves the fee in DBC and retries after the next posts. Real `back` 100,901 → 101,610 CU, no new account | `src/instructions/back.rs:110-123, 194-198, 250`; `src/constants.rs:49` | `fixed_a3_06_back_bounds_the_price_by_the_accepted_floor` (1.7 SOL accepted vs a 2 SOL conversion: refused even with `min_units = 0`; 1.85: accepted; no floor: `NoFloor`); `instructions::back::tests`; `real_rook_core` |
| A3-02 | **Fixed** | `register_partner_config` refuses `leftover_receiver != claimer` (the launch's choice), `partner_liquidity_percentage`/`creator_liquidity_percentage` ≠ 0 or an initialized vesting info (`UnlockedLiquidity`), `migrated_collect_fee_mode ≠ 0` or a compounding fee (`WrongMigratedFeeMode`), `pool_creation_fee ≠ 0` (`PoolCreationFee`). `fee_claimer == claimer` and timestamp activation were already enforced. Fixed supply is not enforced (one test runs a 30 % fee on DBC's dynamic supply; leftovers go to the claimer either way) | `src/instructions/admin.rs:176-190`; `src/dbc.rs:109-133` | `fixed_a3_02_register_partner_config_enforces_the_rook_invariants` (each variant refused with its error; the launch config registers) |
| A3-05 | **Fixed (the version check)** | `PoolConfig.version` (byte 236) must be 0, what DBC 0.2.1 writes (measured on the mainnet binary) | `src/dbc.rs:32`; `src/instructions/admin.rs:176` | same test (byte 236 asserted 0; a copy with 1 refused `WrongConfigVersion`) |
| A3-03 | **Fixed (validation)** | `register_partner` takes `listing`, `module`, `module_vault`, `wrapper` as accounts: each owned by rook_core with its discriminator (`rook_layouts`), and `module.listing == listing`, `module.vault == module_vault`, `module.wrapper == wrapper`, `wrapper.listing == listing`, `wrapper.wx_mint == wx_mint` (`WrongCoreAccount`); emits `PartnerRegistered { listing, listing_id, module, wrapper, wx_mint, claimer }` for the ceremony's order check. Not done: `set_partner_accounts` / config retirement | `src/instructions/admin.rs:75-89, 97-117` | `fixed_a3_03_register_partner_checks_the_rook_core_accounts` (another listing's module, vault, wrapper or mint; a non-rook_core listing: refused) |
| A3-01 | **Fixed (decision: the role holder at backing)** | `back` re-reads DBC's `pool.creator` and stores it in `Vault.creator`; the wake desk credits that key. Requiring `open_vault` before migration was rejected (deploy-later backs coins that graduated before rook_backing existed). The role stays transferable on DBC until backing (bounded by one bond per coin); docs now say "creator role holder at backing", not "who launched the coin" | `src/instructions/back.rs:256`; `src/state.rs:55-58`; `crates/rook_layouts/src/lib.rs:67`; `rook_core/src/instructions/wake.rs` (doc) | `fixed_a3_01_vault_creator_is_the_role_holder_at_backing` |
| A3-10 | **Docs fixed** | the claimer's "ends at 0 lamports", the OnlyB statement (now enforced), "who launched the coin", the listing-order sentence in `launch-day1-solana.md` §2 | `docs/v9/rook-backing-as-built.md` §2, §8; `docs/v9/launch-day1-solana.md` | — |
| A3-04, A3-07, A3-08, A3-09, A3-11 | Not changed (Informational) | — | — | — |

Harness: the mock rook_core signs as `["wrapper", listing]` (rebuilt `--arch v2` into `tests/fixtures/mock_rook_core.so`), and
`setup_collection` writes rook_core-shaped `Listing`/`Module`/`ModuleVault`/`Wrapper` accounts at rook_core's PDAs
(`tests/harness/mod.rs` `mock_core_accounts`, `mock_set_accepted`).

To re-check on devnet: the devnet DBC is newer than mainnet's 0.2.1. If its `create_config` writes another `version`, the devnet
rehearsal's `register_partner_config` now refuses (`WrongConfigVersion`), and the new offsets (184, 200, 360, 366, 368) have not
been read from a devnet-written config. Interface change: `register_partner` +4 accounts (the rehearsal tool's builders).
