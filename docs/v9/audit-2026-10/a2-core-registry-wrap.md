# A2: rook_core's registry, wrappers, redemption tickets, treasury and wake desk, and account validation

| | |
|---|---|
| Scope | `solana/programs/rook_core/src`: `lib.rs`, `seeds.rs`, `hashing.rs`, `clock.rs`, `instructions/{registry,wrap,treasury,wake}.rs`, `state/{registry,wrap,treasury,wake}.rs`, and the account constraints of every instruction (including `instructions/floor.rs`, whose arithmetic belongs to A1); `solana/crates/rook_layouts`; wake.rs's parsing of rook_backing's `Vault` |
| Baseline | commit `f2c24e0`, branch `golive-2026-10-08` |
| References | `src/nft/EthNftWrapper.sol`, `src/Wrapper.sol`, `src/ListingRegistry.sol`, `src/floor/ProtocolTreasury.sol`, `src/floor/WakeDesk.sol`, `docs/v9/rook-core-as-built.md`, `docs/v9/solana-migration.md` (D-g, Q4, Q17) |
| Method | Threat model, then a line-by-line review against the A2 checklist (owner, discriminator, PDA and canonical bump, signer and writable checks; account substitution; duplicate mutable accounts; init front-running; closing and revival; mint and freeze authority; token-account owner and mint; Merkle domain separation; redemption ordering and refund windows; destination validation; mint-once and per-day limits; bounds; treasury rounding and pagination; wake bond accounting; timelocks; events; CU and size). Each EVM original was diffed against its port. Every High has a failing LiteSVM proof of concept, and some Medium and Low findings have demonstration tests |
| PoC file | `solana/programs/rook_core/tests/audit_a2_core.rs` (new; no existing source was modified) |

## Threat model

**Assets.** The NFTs and inscriptions held in custody, recorded as `Item`, `HeldMarker` and `SourceMarker` accounts. The wX mint authority (the wrapper PDA), which no freeze authority accompanies. Redemption tickets: their stage, their pinned round, their destination, and the one unit each ticket burned. The treasury's lamports (`reserves_pot`, the operations share). Wake bonds, each held in its `Request` account. Registry roles and timelocks.

**Actors.**

| Actor | Powers |
|---|---|
| anyone | redeem, resolve, refund, the treasury cranks (`allocate`, `begin_distribution`, `distribute`), wake request, reclaim, forfeit, mark_served |
| holder | owns the wX token account used to redeem, and receives refunds |
| custodian (the canister) | mint_item, note_sent, mark_undeliverable, record_delivery, advance and settle |
| keeper | post_floor for inscriptions, and **prove_round, which carries no on-chain verification until Q4** |
| owner | today a single fund-wallet key that is also the upgrade authority (D-g, Q17) |
| guardian | stop-only |
| attacker | may pass crafted accounts to any instruction |

**Trust assumptions taken as given.** Under D-g the owner and upgrade authority can do anything, including upgrade the program. The custodian is trusted for mint and delivery truth (H11 bounds its mint rate). The keeper is trusted for prices (A1) and, until Q4, for randomness.

## Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| A2-01 | `begin_distribution` restarts an unfinished round for anyone: collections already paid are paid again, and the whole reserves pot can be steered into one collection | **High** | `poc_a2_01_*` (2 tests): **FAIL** (the bug is reproduced) |
| A2-02 | Keeper-attested drand (deferred Q4), assessed: the keeper picks each ticket's item and may publish future rounds early | Medium | `demo_a2_02_*`: passes (the attack works) |
| A2-03 | An Inscriptions redemption may name the custody address: a relist ticket the canister never delivers, refunded after 37 days (A-M-2's free, repeatable option, reopened for inscriptions) | Medium | `demo_a2_03_*`: passes (the attack works) |
| A2-04 | A queue blocked for `PIN_TIMEOUT` lets a redeemer read a proven outcome and refund instead (a free re-roll) | Low | `demo_a2_04_*`: passes |
| A2-05 | The owner's powers are instant: transfer, guardian replacement, treasury shares and operations address; the guardian cannot cancel a pending owner, keeper or root | Low | n/a |
| A2-06 | `allocate` fails while `ops` is an unfunded account and the operations share is under its rent-exempt minimum | Informational | n/a |
| A2-07 | Rent is never reclaimed: `Ticket`, `Item`, `SourceMarker`, `DrandRound` and `Spent` are never closed | Informational | n/a |
| A2-08 | `begin_distribution` with `MAX_COLLECTIONS = 32` modules needs an address lookup table (and about 170k CU) | Informational | `measure_begin_distribution_cu` |
| A2-09 | `record_delivery` does not check that the `HeldMarker` points to this item; events omit fields the EVM emitted | Informational | n/a |

---

### A2-01: `begin_distribution` restarts an unfinished round, so already-paid collections are paid again (High)

**Where.** `instructions/treasury.rs:141-160` (`handle_begin_distribution`, especially lines 155-158) and `:174-190` (`handle_distribute`).

**Description.** The EVM `ProtocolTreasury.distribute()` paid every collection in one call. The Solana port splits it into `begin_distribution`, which snapshots `round_pot = reserves_pot`, the weights and `round_paid = 0`, and `distribute(index)`, which pays one collection per call. Both are permissionless. `begin_distribution` never checks whether the current round is finished: it overwrites `round_pot` with what is left in the pot and clears `round_paid`. A collection paid before the restart is therefore payable again, at its full weight, out of the remainder. The doc comment says "A later call restarts the round with the pot as it then is", but the double payment this causes was not considered.

**Impact.** Anyone can redirect the protocol's whole reserves pot into the surplus of one chosen floor module, starving every other collection. The lamports stay inside the protocol, but they move between collections: from other collections' wX holders and their floors to the chosen one, typically one the attacker holds. Honest cranks hit the same bug without any malice: two keepers or bots racing, or one retrying a round, pay the first indices twice.

**Exploit.** With equal weights and a 40 SOL pot, the attacker loops `begin_distribution(all modules)` then `distribute(0)`. After k iterations collection 0 holds `40·(1 − 2^−k)` SOL. The only cost is transaction fees.

**PoC.** `poc_a2_01_restarting_a_distribution_round_drains_the_pot_into_one_collection` fails as expected. After 30 iterations collection 0 received **39 999 999 962 lamports** and collection 1 received **19 lamports**; with equal weights each should have received 20 SOL. `poc_a2_01_a_racing_crank_pays_a_collection_twice` also fails as expected: the second `distribute(0)` after a second `begin` succeeds.

**Recommendation.** Refuse `begin_distribution` while a round is open, meaning some index with non-zero weight is still unpaid and `round_pot` has not been fully paid out. Alternatively, rate-limit rounds (for example one per `DAY`, with a round stamp), or let a new round start only from what was *not* earmarked: keep `round_pot − paid_so_far` committed to the unpaid indices of the open round. A simple, robust shape: `begin` requires `round_paid == mask_of_nonzero_weights || round_total == 0`, plus a timeout (say 1 day) after which an unfinished round may be restarted. Mint-volume weights (A1) remain manipulable within one block; snapshotting them at `begin` is fine once restarts are bounded.

---

### A2-02: Keeper-attested drand, assessed (Medium; the deferral itself is by design, Q4)

**Where.** `instructions/wrap.rs:444-467` (`ProveRound` / `handle_prove_round`); its use in `resolve` at `:299-312`.

**Description.** The Q4 deferral is known. This entry assesses what a malicious or compromised keeper can do with it. The keeper is a hot key, the same one that posts inscription floors.

1. **It chooses each ticket's item.** `resolve` draws `keccak(randomness ‖ wrapper ‖ id) mod |free|`. The keeper sees the free set and the pinned round, and can grind `randomness` in about |free| tries so that a given ticket, for example an accomplice's, receives the rarest item. One `DrandRound` serves every wrapper's tickets for that round, which constrains grinding only a little when several tickets share it.
2. **It can publish rounds before they exist.** `prove_round` does not require `round <= round_at(now)`. Randomness written for future rounds is public, so any redeemer can time a redemption: `round_at(t) + 20` changes every 3 s. Choosing the moment then chooses the item. An honest but buggy keeper causes this too.
3. **It can withhold rounds.** Tickets then sit pinned until `PIN_TIMEOUT` (7 days) and are refunded (see A2-04 for the option this creates). Strict ordering means one withheld round blocks every later ticket of that wrapper.
4. **Each round is written once.** The `init` of `["drand", round]` means the keeper cannot rewrite a round.

**Bounded blast radius.** The keeper cannot move or mint units, change a destination, take an item out of custody, or alter ledger counters. The damage is limited to *which* held item each ticket gets and to delay (refundable). The value at stake is the spread between the most and least valuable held items, times the number of tickets.

**PoC.** `demo_a2_02_a_keeper_picks_the_item_a_ticket_receives` passes. The keeper proves a round 100 000 rounds ahead, then grinds the ticket's still-future round so that ticket 0 draws item 3.

**Recommendation.** Ship Q4 (the BLS check over alt_bn128). Until then, at little cost: require `round <= round_at(now)` in `prove_round` (this kills the pre-publication variant), and keep the keeper's prove-round path in the canister fed only from drand's HTTP relays, with an off-chain check (anyone can verify a published round against drand's public key and alert on a mismatch). Consider a separate, narrowly scoped `prover` role, so that rotating or pausing the poster key does not also stop draws.

---

### A2-03: An Inscriptions redemption to the custody address is a free, repeatable lock (Medium)

**Where.** `instructions/wrap.rs:242-247` (Inscriptions destination check), together with `refund` at `:421-430`. Canister side: `autobuyer/canister-rs/src/redeem.rs:407-424` (`discover`: a destination equal to our script becomes `RPhase::Relist`, never delivered and never marked undeliverable).

**Description.** For EthNft the port refuses `Destination::Eth(custody_evm)`, matching EthNftWrapper's fix for audit A-M-2. For Inscriptions any taproot address on the network is accepted, including the listing's own `custody`. The canister treats such a ticket as a relist: "nothing is delivered; the sale side lists the item". Nothing on chain marks it as non-refundable, so once `deliver_by + LATE_DELIVERY_WINDOW` passes (resolve + 37 days) anyone can refund it. The item returns to the set and the unit is minted back. This is exactly the free, repeatable option A-M-2 described. (The EVM `Wrapper.sol` had the same shape through `redeemToRelist`. The Solana port dropped `redeemToRelist`, but the custody address still works as a plain destination.)

**Impact.** A holder of k units can tie up k held items for about 37 days at a time, at no cost beyond ticket rent (about 0.0026 SOL each), and get every unit back. Once k ≥ held − open tickets, every other holder's `redeem` fails with `NoItem`. With custody capped at 48 items across all collections at step 1, a handful of units closes a collection's redemptions indefinitely. A second concern goes to A5: if the canister's sale side actually sells a "relisted" item while its ticket later refunds on chain, the item goes back into the draw set although it has left custody.

**PoC.** `demo_a2_03_an_inscription_ticket_to_custody_locks_items_and_refunds` passes. On signet, two units are redeemed to the custody address and accepted; both items become promised (`held == open_tickets == 2`); after 37 days both are refunded and the attacker holds both units again. The ledger holds throughout.

**Recommendation.** In `handle_redeem` for `Inscriptions`, refuse `s == listing.custody`, mirroring the EthNft branch. If a relist exit is ever wanted, add an explicit variant whose ticket is never refundable to its burner, as A-M-2 recommended.

---

### A2-04: A blocked queue gives redeemers a free re-roll (Low)

**Where.** `instructions/wrap.rs:413-420` (pinned refund) and `:293` (strict order).

**Description.** A pinned ticket refunds after `PIN_TIMEOUT` whether or not its round has been proven. If an earlier ticket's round is never proven (a keeper outage, a guardian `pause_keeper` lasting at least 7 days, or A2-02's withholding), later tickets cannot resolve even though their randomness is already public. Once 7 days pass, each such owner computes their outcome and chooses between resolving (permissionless) and refunding. On the EVM anyone could prove any round, so the queue never blocked on a missing round. On Solana only the keeper can.

**PoC.** `demo_a2_04_a_blocked_queue_gives_redeemers_a_free_option` passes: ticket 1's round is proven, ticket 0's is not, and after 7 days ticket 1 refunds.

**Recommendation.** Refuse the pinned refund once the ticket's round is proven (pass the optional `DrandRound` to `refund`, and require it to be absent or to have a different round). Alternatively, let `resolve` step past a stale pinned ticket whose round was never proven, so the queue cannot stall. This resolves fully with Q4, when anyone can prove a round.

---

### A2-05: The owner's powers are instant, and the guardian cannot cancel pending changes (Low)

**Where.** `instructions/registry.rs:79-95` (`propose_owner` / `accept_owner`: two-step, no delay), `:98-109` (`set_guardian`: instant), `instructions/treasury.rs:45-59` (`set_shares` / `set_ops`: instant, and `ops_bps` may be 10 000).

**Description and risk.** Transferring ownership has no public delay. Neither does replacing the guardian, or sending 100 % of all future treasury income to any `ops` address. Today this changes nothing in practice: under D-g the owner key is also the program's upgrade authority with no time lock, so its compromise is already total, and the risk is accepted until Q17. A new owner does **not** bypass the 7-day delays on keeper, converter and root changes, because those apply to whoever is owner. Once Q17 moves upgrades behind a Squads time lock, though, the gap becomes material: the owner role would still act instantly where upgrades no longer can. The guardian (stop-only) can cancel a pending converter, but not a pending owner, keeper or membership root, and it cannot stop `set_shares` / `set_ops`.

**Recommendation.** When Q17 lands, put `propose_owner`, `set_guardian` (when it replaces a non-default guardian), `set_shares` and `set_ops` behind `CHANGE_DELAY`, or keep the Squads time lock on the owner role itself. Let the guardian cancel `pending_owner`, `pending_keeper` and `pending_root` (stop-only powers). Consider capping `ops_bps`.

---

### A2-06: `allocate` can fail while the operations account is unfunded (Informational)

**Where.** `instructions/treasury.rs:125-129`. `pay_out` credits `ops` directly. If `ops` is a fresh system account with 0 lamports and `to_ops` is below the rent-exempt minimum (about 0.00089 SOL, which means income under about 0.0089 SOL at a 10 % share), the transaction fails with an insufficient-funds-for-rent error. It clears by itself as income accumulates. **Recommendation:** fund `ops` before `set_ops`, or skip the operations transfer and keep it pending while it is under rent.

### A2-07: Rent is never reclaimed (Informational)

**Where.** `state/wrap.rs`, `state/wake.rs`. Accounts that are never closed:

- `Ticket`, paid by the redeemer, about 0.0026 SOL.
- `Item`, `HeldMarker` and `SourceMarker`, paid by the custodian, about 0.004 SOL per mint. `HeldMarker` is reused; the other two must stay because `SourceMarker` is mint-once forever.
- `DrandRound`, paid by the keeper, 0.0012 SOL per round.
- `Spent`, paid by the reclaimer.

For tickets this is the EVM's storage cost turned into rent. **Recommendation:** add a `close_ticket` (to its owner) for Delivered or Refunded tickets below `next_to_resolve`, and budget the canister's SOL for about 0.004 SOL per wrap.

### A2-08: `begin_distribution` at `MAX_COLLECTIONS` (Informational)

Measured: 11 342 CU for 2 modules, with each `Module` 957 bytes, so roughly 5k CU per module and about 170k CU for 32 modules. That fits a 200k budget but with little margin. Thirty-two read-only module keys plus the fee payer and the treasury exceed a legacy transaction's 1 232 bytes, so the crank needs a v0 transaction with an address lookup table. **Recommendation:** document this in the crank, and add a 32-module test.

### A2-09: Minor defense in depth and events (Informational)

- `record_delivery` (`instructions/wrap.rs:361-377`) zeroes the `HeldMarker` of `item.key` without checking `held.item_plus_one == item.item_id + 1`. Its current invariants make a mismatch unreachable, because a key cannot be re-minted while an item still holds it. An explicit check would cost nothing.
- `Redeemed` does not emit the destination, unlike the EVM, and `Refunded` does not emit the item. The resolve that steps past a refunded ticket emits nothing. An indexer has to read ticket accounts.
- `Destination::Eth(0x…dEaD)` is accepted on an EthNft listing but does not count in `burned`; only `Destination::Burn` does.

## Checked and found sound

**Registry**

- `init_registry` is gated on the upgrade authority read from `ProgramData`. The account is pinned as the loader's PDA `[program_id]` (`seeds::program = bpf_loader_upgradeable`) and owner-checked. The offsets are correct (tag 3 @0, slot @4, option @12, key @13..45), and `None` is returned for an immutable program. It cannot be front-run, and the `["registry"]` `init` means it runs once.
- The `initial_converters` loop rejects default and duplicate keys and caps their number.
- Role checks: the keeper cannot be the guardian or the owner, the guardian cannot be the keeper, the pending keeper, the owner or the pending owner, and the owner cannot transfer to the guardian.
- There is no renounce path: `accept` requires a pending owner, and `propose(default)` only cancels.
- The 7-day `CHANGE_DELAY` holds for keeper, converter and root changes; only the very first keeper applies at once, as in the EVM. Removing a converter is instant and stop-only.
- `rook_layouts::registry::owner` matches the struct (a unit test covers it).
- `list_collection` checks lengths, a non-zero root, taproot custody on the registry's network, an EVM custody address, a custodian, and a poster for EthNft listings.

**Account validation across every rook_core instruction**

- Every `Account<T>` is checked for owner and discriminator. Listing, wrapper, module, item set, ticket, item and `DrandRound` accounts can only be created through `init` with fixed seeds, so an attacker cannot forge one.
- Every cross-link is constrained: `wrapper.listing == listing`, `item_set.wrapper`, `ticket.wrapper`, `item.wrapper && item_id`, `module.listing`, `address = module.vault / module.wrapper / wrapper.wx_mint / module.module_wx`, and `custodian == listing.custodian`.
- Stored bumps are the canonical ones from `init`; `Withdraw`'s `wake_slot` and `RecordDelivery`'s `held` derive their bumps.
- Token accounts are checked for mint, and for authority where the program burns from them.
- No duplicate-mutable-account path changes balances or books in A2's instructions.
- Every `init_*` is owner-only, and pre-funding a PDA does not block Anchor's `init`.

**Wrap**

- The wX mint has 9 decimals, mint authority = wrapper PDA, and **no freeze authority**. Only `mint_item` (custodian), `refund` (stage-gated, one unit) and the floor's `module_mint` sign as the wrapper.
- Mint-once per (item, tx) through `SourceMarker`, and held-once per key through `HeldMarker`.
- The H11 ring of 64 timestamps is correct for any limit ≤ 64. Lowering the limit is instant; raising it waits `LIMIT_DELAY` and anyone may then accept it. `init_mint_limit` only works before the first mint.
- The `MAX_HELD = 256` cap bounds `free + pending ≤ held ≤ 256` within the item set's fixed space.
- Merkle membership: the leaf is `keccak(keccak(id))` over 32 bytes and an inner node is `keccak(a‖b)` over 64 bytes, so the domains are separated and no second preimage exists. Pairs are sorted as in OpenZeppelin. Proof length is bounded by transaction size and CU only.
- Taproot validation: exactly 62 characters, `bc1p`/`tb1p` per the network, a bech32m checksum over the expanded prefix, zero padding bits, lowercase only. The NUMS burn constants check out.
- EVM address parsing and the zero and custody refusals are correct.
- The redemption invariant `held ≥ open + 1` guarantees `resolve` never hits `NothingToDraw`: items stamped ≤ id cover every ticket pinned before it.
- Resolution is strictly ordered. A refunded pinned ticket is stepped over. The draw `h mod |free|`, computed in byte steps, equals `uint256 % size`.
- Refund windows match EthNftWrapper: pinned after 7 days; resolved after `deliver_by + 3 days` when marked undeliverable, or `+ 30 days` with no `note_sent`. Double refunds and double deliveries are impossible because of the stage checks. A refunded item re-enters `pending` stamped at `ticket_count`, so earlier tickets cannot draw it.
- The ledger `supply == (held − open)·ONE + eth_backed` holds in every test, including the PoCs.
- Overflow checks are on in release builds (`overflow-checks = true`).

**Treasury**

- Shares always sum to `BPS`. `allocate` books only lamports above `rent + reserves_pot`; rounding favours the reserves; the operations share joins the reserves when no ops account is set.
- `distribute` pays at most `reserves_pot`, and the sum of `mul_div` floors never exceeds the round pot. Only registered modules are paid; the `remaining_accounts` order and keys are checked; `1 << i` is in range for i < 32. (The restart is A2-01.)

**Wake desk**

- The bond equals `desk.bond` exactly and is held in its own `Request`. `pay_out` never takes a request below its rent.
- One open request per collection, using `wake_slot` and the last request's id check. A request is refused while the floor is fresh.
- `mark_served` uses the latest post within `[at, at + window]`. `forfeit` sends the bond to the treasury only if the request was served, and adds it to `total_received`.
- `reclaim`:
  - The vault is owner-checked against `ROOK_BACKING_ID`, which equals `Anchor.toml` and rook_backing's `declare_id!`.
  - The vault is parsed by exact length and discriminator through the shared `rook_layouts` layout.
  - The coin's partner must equal `partner_of(listing)`, its creator must be the requester, and it must have graduated.
  - `launched_at >= at`. `launched_at` is the DBC pool's own `activation_point`, and rook_backing refuses configs without timestamp activation (`admin.rs:122`), so a fake or earlier launch cannot qualify.
  - One bond per coin, through `Spent`.
- `withdraw` closes the request to its requester (Anchor's `close`, so no revival) and frees the slot only when this request was the last one.

**Guardian** — can only call `pause_keeper`, `pause_poster` and `remove_converter`; it cannot unpause anything.

## Test results

- Existing suite: `cargo test -p rook_core --test rook_core`: **70 passed**. Unit tests: **7 passed**. The SBPF v2 binary was rebuilt from `f2c24e0` with `cargo build-sbf --arch v2 --sbf-out-dir target/deploy-sbpfv2`.
- `cargo test -p rook_core --test audit_a2_core`:

| Test | Result | Meaning |
|---|---|---|
| `poc_a2_01_restarting_a_distribution_round_drains_the_pot_into_one_collection` | **FAILED** | as intended: 39.99999996 SOL vs 19 lamports |
| `poc_a2_01_a_racing_crank_pays_a_collection_twice` | **FAILED** | as intended: a second payment succeeds |
| `demo_a2_02_a_keeper_picks_the_item_a_ticket_receives` | passed | the attack works |
| `demo_a2_03_an_inscription_ticket_to_custody_locks_items_and_refunds` | passed | the attack works |
| `demo_a2_04_a_blocked_queue_gives_redeemers_a_free_option` | passed | the option exists |
| `measure_begin_distribution_cu` | passed | 11 342 CU for 2 modules |

Once the fixes land, the `poc_*` tests should pass, and the `demo_*` tests should be inverted (made to expect the refusals).

## Verification (independent, 2026-10-10)

Method: I re-read the cited code at `f2c24e0` and ran `cargo test -p rook_core --test audit_a2_core` on a fresh SBPF v2 build.
The result was 4 passed and 2 failed (both `poc_a2_01_*`), with identical numbers: 39 999 999 962 lamports vs 19, a second
`distribute(0)` accepted, and 11 342 CU. I traced every use of `Module.surplus` to establish what A2-01 actually moves.

| ID | Verdict | Final severity | Reason |
|---|---|---|---|
| A2-01 | CONFIRMED-ADJUSTED | **Medium** (was High) | The bug is real and free to trigger. `begin_distribution` never checks for an open round, and `distribute` pays from a fresh `round_pot` with `round_paid = 0`. But the lamports land in the chosen module's `surplus`, which **no burn ever pays** (`release` pays from backing/reserve only). Surplus is used only to fund item purchases above backing (`take_one`), to cover an advance shortfall, and to go back to the treasury through an escaped `skim`. Nothing leaves the protocol and the attacker cannot extract it. The harm is misallocation of the protocol's buffer away from other collections. |
| A2-02 | CONFIRMED | Medium | `handle_prove_round` has no `round <= round_at(now)` bound (`wrap.rs:458-467`). Grinding is inside the Q4 trust assumption. Pre-publishing future rounds lets **any** redeemer choose an item through a keeper bug, and the one-line guard removes that. |
| A2-03 | CONFIRMED | Medium | The `Inscriptions` branch (`wrap.rs:236-241`) does not compare against `listing.custody`. The canister's `discover` (`redeem.rs:407-424`) turns such a ticket into `RPhase::Relist`, never delivers it, never marks it undeliverable, and never calls `note_sent`, so `refund` opens at `deliver_by + 30 d` (resolve + 37 d). The demo passes. |
| A2-04 | CONFIRMED | Low | A pinned refund after `PIN_TIMEOUT` never checks whether the round now exists. |
| A2-05 | CONFIRMED | Low | `propose_owner`, `set_guardian`, `set_shares` and `set_ops` are instant. This is moot while D-g gives the same key upgrade authority. |
| A2-06 | CONFIRMED | Informational | `pay_out` credits `ops` without a rent top-up. |
| A2-07 | CONFIRMED | Informational | Only `wake::Withdraw` uses `close =`. |
| A2-08 | CONFIRMED | Informational | Measured 11 342 CU for 2 modules. |
| A2-09 | CONFIRMED | Informational | `RecordDelivery.held` is derived from `item.key` without an `item_plus_one` check. Unreachable today. |

**A2-01 adjustment.** High overstates the impact, because no value can be extracted by anyone. It is still a must-fix before the
treasury holds anything: honest racing cranks trigger it too, and it makes the documented volume-weighted split meaningless.
The recommended fix stands. Refuse `begin_distribution` while some non-zero-weight index is unpaid, with a timeout after which
a stale round may be restarted.

**Cross-check with A3-01.** The wake desk's `v.creator == r.requester` is A3-01's transferable role holder. A2's sound-list
entry "its creator must be the requester" should read "the DBC creator role at `open_vault`".

No new issue found in A2's scope.

## Fixes (2026-10-10)

Paths under `solana/programs/rook_core/`. `cargo test -p rook_core --test audit_a2_core`: **7 passed** (before: 4 passed, 2
failed); the LiteSVM suite 71/71 (70 + the metadata test). The `demo_*` tests of A2-02/03/04 are inverted into `fixed_*`
regressions that expect the refusals.

| ID | Status | Fix | Where | Test |
|---|---|---|---|---|
| A2-01 | **Fixed** | `begin_distribution` refuses (`RoundOpen`) while any index with a weight is still owed a non-zero share (`Treasury::round_owed`), unless `ROUND_TIMEOUT` = 1 day passed since the round began (`round_started_at`, appended after `bump`: earlier offsets unchanged); `distribute` pays `round_owed(i)` | `src/instructions/treasury.rs:149, 163, 186`; `src/state/treasury.rs:9, 37, 43-59` | both `poc_a2_01_*` now **pass** (each collection 20 SOL; the second `begin` refused, a finished round restarts at once); `fixed_a2_01_a_stale_round_restarts_after_a_day` |
| A2-02 | **Fixed (the pre-publication part)** | `prove_round` requires `round <= round_at(now)` (`RoundInFuture`). Keeper-chosen randomness of a live round remains the Q4 trust assumption | `src/instructions/wrap.rs:543` | `fixed_a2_02_the_keeper_cannot_prove_a_round_that_is_not_live`; the suite's helpers move the clock to the round (`Env::reach_round`) |
| A2-03 | **Fixed** | `redeem` on an Inscriptions listing refuses `destination == listing.custody` (`CustodyDestination`) | `src/instructions/wrap.rs:305` | `fixed_a2_03_an_inscription_redemption_to_custody_is_refused`; `wrap::inscriptions_membership_outpoints_and_taproot_destinations` (custody moved off the burn address it used to redeem to) |
| A2-04 | **Fixed** | `refund` takes the ticket's `["drand", round]` PDA (seeds-checked, may not exist) and refuses a pinned ticket whose round account exists (`RoundProven`). No trap: every earlier ticket is resolvable (round proven) or refundable no later than this one (pinned earlier, round unproven); `refund` is permissionless and advances the cursor, so the proven ticket then resolves. +1 account, refund 22,859 → 26,380 CU | `src/instructions/wrap.rs:464-467, 491` | `fixed_a2_04_a_proven_pinned_ticket_resolves_instead_of_refunding` (the blocking ticket is refunded by anyone, the proven one resolves) |
| A2-09 | **Fixed** | `record_delivery`'s `held` must satisfy `held.item_plus_one == item.item_id + 1` (`WrongHeldMarker`); `Redeemed` emits `destination`; `Refunded` emits `item: Option<u32>` and `destination` | `src/instructions/wrap.rs:426, 330, 510`; `src/events.rs:135, 167-168` | `wrap::delivery_and_burn` (a second record now hits `WrongHeldMarker`) |
| A2-05 | Not changed (Low; moot under D-g until Q17) | — | — | — |
| A2-06, A2-07, A2-08 | Not changed (Informational) | — | — | `measure_begin_distribution_cu`: 11,342 → 11,404 CU for 2 modules |

Also in A2's scope: the wX mints get Metaplex token metadata at `init_wrapper` (`src/metadata.rs`, `src/instructions/wrap.rs:49-103`;
update authority = the wrapper PDA, changed by the owner-only `set_wx_metadata`): `rook-core-as-built.md` §9.6, test
`wrap::wx_metadata_at_init_and_the_owner_updates_it`. The sound-list entry "its creator must be the requester" now reads "the DBC
creator role holder when the coin was backed" (A3-01 fix).

Interface changes: `refund` +1 account (`drand_round`); `init_wrapper(meta)` +2 accounts (`metadata`, `token_metadata_program`);
new `set_wx_metadata`; `Treasury` +8 bytes; errors appended. The canister builds none of the changed instructions and decodes no
changed account; its IDL fixture was regenerated (`autobuyer/canister-rs/tests/fixtures/solana/rook_core.json`) and
`tests/solana.rs` passes (30/30).
