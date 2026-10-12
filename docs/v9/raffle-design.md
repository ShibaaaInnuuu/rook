# Holder raffle (rook_core, 2026-10-10)

Operator decisions of 2026-10-10:

1. **Prizes are paid from reserve overflow.** In the treasury's `distribute(i)`, the part of collection `i`'s share above its
   reserve cap (§9.10 of `rook-core-as-built.md`) is split: `raffle_bps` of it goes to that collection's raffle pot, the rest
   to operations as before. Default `raffle_bps = 5_000`; the owner changes it with the 24 h propose/apply pattern. If no
   operations address is set, the operations part stays in the reserves pot (as before).
2. **Weekly draws per collection.** The winner is drawn among the holders of that collection's graduated coins, weighted by
   holdings (backing-weighted, below); the prize is **one whole wX** of that collection.

This document is the contract a canister agent builds against: seeds, account order, arguments, hashing, the weight rule
and the cadence. Program: `solana/programs/rook_core` (`instructions/raffle.rs`, `state/raffle.rs`).

## 1. Accounts

| Account | Seeds (rook_core) | Bytes | Rent (SOL) | Notes |
|---|---|---|---|---|
| `Raffle` | `["raffle", listing]` | 327 | 0.003167 | holds the pot's lamports itself (program-owned) |
| raffle wX token account | `["raffle_wx", raffle]` | 165 | 0.002039 | SPL token account of the collection's wX mint, authority = the `Raffle` PDA |
| `Treasury` | `["treasury"]` | 1,485 (was 1,465) | 0.011226 | gains `raffle_bps`, `pending_raffle_bps`, `pending_raffle_at`, `total_to_raffles` |

`Raffle` layout (Borsh, after the 8-byte discriminator, in this order):

| Field | Type | Meaning |
|---|---|---|
| `listing` | Pubkey | the collection's `Listing` |
| `module` | Pubkey | its floor module `["module", listing]` |
| `wx_mint` | Pubkey | its wX mint |
| `wx_account` | Pubkey | `["raffle_wx", raffle]` |
| `bump`, `wx_bump` | u8, u8 | |
| `phase` | u8 enum | 0 `Idle`, 1 `Committed`, 2 `Drawn` |
| `pot` | u64 | lamports earmarked for prizes (= the account's lamports above rent; `fund_prize` re-syncs it, absorbing plain transfers) |
| `round` | u64 | draws committed so far; the current/last round's number (1-based) |
| `root` | [u8; 32] | the committed snapshot's Merkle root |
| `total_weight` | u64 | the sum of every leaf's `weight` |
| `entries` | u32 | number of leaves (published, not checked on chain) |
| `snapshot_slot` | u64 | the Solana slot the balances were read at |
| `committed_at` | i64 | unix time of the last `commit_raffle` (the weekly cadence counts from it) |
| `drand_round` | u64 | the drand evmnet round pinned at commit |
| `ticket` | u64 | the winning ticket (valid in `Drawn`) |
| `drawn_at` | i64 | unix time of `draw_raffle` |
| `winner` | Pubkey | the last claimant (zero until the first claim) |
| `claimed_at` | i64 | |
| `total_in` | u64 | lamports received from `distribute`, ever |
| `total_spent` | u64 | lamports spent on prizes (`fund_prize`), ever |
| `prizes_funded` / `prizes_claimed` / `rounds_expired` / `rounds_cancelled` | u64 × 4 | history counters |

Invariants: `raffle.lamports − rent ≥ pot`; the module's vault holds at least its books after `fund_prize` (the pot pays the
mint like any buyer); the wrapper ledger `supply = (held − open_tickets)·ONE + eth_backed` holds (the prize is minted
ETH-backed by the normal mint path and is held by the raffle's token account as by any holder); the treasury keeps
`lamports − rent ≥ reserves_pot`.

## 2. Instructions

Accounts are listed in order; (w) writable, (s) signer. "keeper" = `Registry.keeper` while not paused (the canister).

### `init_raffle` — anyone, once per collection

`payer` (w, s) · `listing` · `module` (`module.listing == listing`) · `wrapper` (= `module.wrapper`) · `wx_mint`
(= `wrapper.wx_mint`) · `raffle` (w, init `["raffle", listing]`) · `raffle_wx` (w, init `["raffle_wx", raffle]`) ·
`token_program` · `system_program`. No arguments. Payer pays 0.005206 SOL of rent (0.003167 + 0.002039). Event `RaffleInitialized`.

Until a collection's raffle exists, `distribute` treats its raffle share as 0 (the whole overflow goes to operations, or
stays in the reserves pot without an operations address, exactly as before the raffle). The canister should create every
listing's raffle at once; anyone may.

### `set_raffle_bps(bps)` — owner; `apply_raffle_bps` — anyone after 24 h

Same accounts as `set_shares` (`TreasuryAdmin`: `owner` (s), `registry`, `treasury` (w)) and `apply_shares`
(`TreasuryApply`: `treasury` (w)). `bps ≤ 10_000` (`BadRaffleBps`). Waits `TREASURY_DELAY` (24 h); a new proposal replaces
the pending one. Events `RaffleBpsProposed { bps, ready_at }`, `RaffleBpsSet { bps }`. `init_treasury` sets 5,000.

### `distribute(index)` — changed: one more account

`treasury` (w) · `module` (w) · `module_vault` (w) · `ops` (w, optional: `treasury.ops` or the program id) · **`raffle`
(w, the PDA `["raffle", module.listing]`, whether or not it exists yet)**.

With `r` the index's share and `to_surplus` as before (§9.10): `over = r − to_surplus`; if the raffle account is an
initialized `Raffle` (of this listing), `to_raffle = ⌊over × raffle_bps / 10_000⌋`, else 0 (the address must still be the
PDA: `WrongCollection` otherwise, so nobody can skip an existing raffle by passing another account); `rest = over −
to_raffle` goes to `ops` when one is set, else stays in the reserves pot. `reserves_pot −= to_surplus + to_raffle + to_ops`; `total_to_raffles += to_raffle`;
`raffle.pot += to_raffle`, `raffle.total_in += to_raffle`. `ReserveDelivered { module, amount, to_ops, kept }` is unchanged
(`kept` = what stayed in the pot); a raffle payment also emits `RafflePotFunded { raffle, listing, lamports, pot }`.

### `fund_prize` — anyone (the canister's keeper crank)

`registry` · `listing` · `module` (w) · `module_vault` (w) · `wrapper` (w) · `wx_mint` (w) · `raffle` (w,
`["raffle", listing]`) · `raffle_wx` (w, = `raffle.wx_account`) · `token_program`. No arguments.

Mints exactly one whole wX (`ONE` = 10⁹ units) into the raffle's token account through the floor module's public `mint`
book-keeping: `price = mint_price()` = ⌈max(price TWAP, up-target) × (1 + mint spread)⌉ lamports, `units = Module::mint(price,
min_units = ONE)` (spreads, the hourly bucket, `public_cap` and `max_eth_backed` all apply; the fresh-floor and poster checks
refuse with `Stale` / `Paused`); the pot pays `price` to the module vault by a direct lamport move; `wrapper.eth_backed += ONE`.
Refused: `PrizeHeld` while the raffle already holds ≥ 1 whole wX; `PotTooSmall` if `pot < price`. Event
`RafflePrizeFunded { raffle, round, lamports, units }`. It counts in the module's seven-day volume like any mint.

### `commit_raffle(root, total_weight, snapshot_slot, entries)` — keeper

`keeper` (s) · `registry` · `raffle` (w) · `raffle_wx` (= `raffle.wx_account`).

Requires: phase `Idle` (`RafflePhase`); `committed_at == 0` or `now ≥ committed_at + 7 days` (`RaffleTooEarly`); the raffle
holds ≥ 1 whole wX (`NoPrize`); `root ≠ 0`, `total_weight > 0`, `entries > 0` (`BadInput`); `snapshot_slot < Clock.slot`,
`Clock.slot − snapshot_slot ≤ 432_000` (≈ 2 days) and `snapshot_slot > raffle.snapshot_slot` (`BadSnapshot`).
Effects: `round += 1`, phase `Committed`, stores `root/total_weight/entries/snapshot_slot`, `committed_at = now`,
`drand_round = round_at(now) + 20` (a round ≈ 60 s in the future, which does not exist yet when the snapshot is fixed).
Event `RaffleCommitted { raffle, round, root, total_weight, entries, snapshot_slot, drand_round }`.

### `draw_raffle` — anyone

`raffle` (w) · `drand_round` (the `DrandRound` PDA `["drand", raffle.drand_round u64 le]`, written by `prove_round` as for
redemptions; missing → the instruction fails with `AccountNotInitialized`).

Requires phase `Committed`. `h = keccak256("rook-raffle-draw" ‖ randomness ‖ raffle ‖ round u64 le)`;
`ticket = u128::from_be_bytes(h[0..16]) mod total_weight`. Phase `Drawn`, `drawn_at = now`. Event `RaffleDrawn { raffle,
round, drand_round, ticket }`. (The randomness is keyed by the raffle and round so two collections drawn on the same drand
round get independent tickets.)

### `claim_raffle(cum_start, weight, proof)` — anyone (e.g. the canister, paying the ATA rent)

`claimer` (w, s; pays the winner's associated token account if it does not exist) · `owner` (the winner's wallet,
unchecked) · `raffle` (w) · `raffle_wx` (w) · `wx_mint` (= `raffle.wx_mint`) · `owner_wx` (w, the associated token account
of `owner` for `wx_mint`, created if needed) · `token_program` · `associated_token_program` · `system_program`.

Requires phase `Drawn`, `now < drawn_at + 30 days` (`ClaimWindowOver`), `proof.len() ≤ 32`, `weight > 0`,
`cum_start ≤ ticket < cum_start + weight` (`NotTheWinner`), and `verify(proof, root, leaf(owner, cum_start, weight))`
(`BadProof`). Transfers `ONE` wX from the raffle's account (signed by the raffle PDA) to `owner_wx`. Phase `Idle`,
`winner = owner`, `prizes_claimed += 1`. Event `RaffleClaimed { raffle, round, winner, ticket, cum_start, weight }`.

### `expire_raffle` — anyone

`raffle` (w). Either:
- phase `Drawn` and `now ≥ drawn_at + 30 days`: the unclaimed prize stays in the raffle's token account and becomes the next
  round's prize (no new `fund_prize` is needed; the pot keeps accumulating). `rounds_expired += 1`; or
- phase `Committed` and `now ≥ committed_at + 7 days` (the pinned round was never proven): the round is cancelled.
  `rounds_cancelled += 1`.

Phase `Idle` either way. Otherwise `RaffleNotExpired`. Event `RaffleExpired { raffle, round, drawn }`.

## 2a. Compute units (LiteSVM, SBPF v2 build)

| Instruction | CU |
|---|---|
| `init_raffle` (creates 2 accounts) | 32,001 |
| `fund_prize` | 39,706 |
| `commit_raffle` | 7,686 |
| `draw_raffle` | 4,639 |
| `claim_raffle` (3 proof nodes, creates the ATA) | 42,351 |
| `claim_raffle` (single holder, empty proof, creates the ATA) | 46,210 |
| `expire_raffle` | 3,464 |
| `distribute` (surplus + raffle + ops) | 21,619 |
| `distribute` (no raffle yet: one `find_program_address` to check the address) | 20,733 |

Each keccak level of a proof costs ≈ 150 CU; a 20-level proof (a million holders) stays well under 200k. Transaction size, not
CU, bounds the proof: `claim_raffle` has 9 accounts, so about 24 proof nodes fit in 1,232 bytes (16M leaves).

## 3. Hashing

- **Leaf** = `keccak256(keccak256("rook-raffle-leaf" ‖ owner (32 bytes) ‖ cum_start u64 le ‖ weight u64 le))`. Hashed twice,
  as `hashing::leaf_of`, so a 64-byte inner node can never be presented as a leaf.
- **Node** = keccak256 of the two children, the smaller (bytewise) first (OpenZeppelin sorted pairs, `hashing::verify_proof`).
- **Tree** (the canister's convention; any tree consistent with sorted pairs verifies): leaves in ascending order of `owner`
  bytes; each level pairs neighbours left to right; an unpaired last node moves up unchanged. One leaf: the root is the leaf
  and the proof is empty.

## 4. The weight rule (computed off-chain by the canister)

For a collection `C` and a snapshot slot `S`:

1. **Coins**: every rook_backing `Vault` of a partner of `C` (`["partner", listing]`) with `graduated = true` at `S`.
2. **Backing of a coin**: `wx(c)` = the balance at `S` of the vault's wX account `["vault_wx", coin_mint]` (the live figure;
   `Vault.wx_units` is historical and ignores redemptions). Coins with `wx(c) = 0` are skipped.
3. **Circulating supply of a coin**: `circ(c)` = the coin mint's supply at `S` minus the balances of excluded token accounts:
   - the coin's DAMM v2 pool token vaults (the locked liquidity after migration) and any DBC pool base vault still holding coins;
   - every token account whose owner is **not on the ed25519 curve** (PDAs: program-owned vaults, lockers, escrows, Rook's own
     PDAs) — such owners cannot sign for a prize and are not holders;
   - the incinerator `1nc1nerator11111111111111111111111111111111` and token accounts owned by the system program address.
   Excluded accounts receive no weight. Coins with `circ(c) = 0` are skipped.
4. **Weight** of a wallet `w` (an on-curve owner; balances of all its token accounts of the coin mint summed):
   `weight(w) = Σ_c ⌊balance_c(w) × wx(c) / circ(c)⌋` (u128 intermediate, floor per coin). In wX base units, so different
   coins are comparable by the backing they carry. Wallets with weight 0 are omitted.
5. **Leaves**: sorted by owner; `cum_start` = the sum of the weights before it; `total_weight` = the sum of all weights
   (≤ Σ wx(c), fits u64). Ticket `t` wins for the leaf with `cum_start ≤ t < cum_start + weight` — exactly one leaf.
6. **Publication**: before `commit_raffle`, the canister publishes the snapshot (slot, coin list with `wx(c)` and `circ(c)`,
   excluded accounts, every leaf `(owner, cum_start, weight)` and the root) where anyone can fetch it (the canister's HTTP
   interface / the site), so anyone can recompute the root from chain data at slot `S` and check the winner after the draw.

The program checks only the Merkle proof and the ticket range. Correctness of the snapshot is the keeper's (and its
publication makes it auditable). Until drand verification is ported (TODO Q4), `prove_round` is also keeper-only, so the
keeper is trusted for the randomness as for redemptions; pinning the round after the commit means an honest keeper cannot
see the outcome before fixing the snapshot.

## 5. Cadence for the canister (per collection, every crank pass)

1. Raffle missing → `init_raffle` (0.005206 SOL once).
2. Phase `Idle`, raffle wX balance < 1 whole wX, `pot ≥ mint_price` and the floor fresh → `fund_prize` (simulate first).
3. Phase `Idle`, prize held, `committed_at == 0 || now ≥ committed_at + 7 d` → take the snapshot at a recent finalized
   slot, publish it, `commit_raffle`.
4. Phase `Committed`, `round_at(now) ≥ drand_round` → `prove_round(drand_round, …)` if the `DrandRound` PDA is missing (the
   existing resolve path), then `draw_raffle`.
5. Phase `Drawn` → `claim_raffle` for the leaf containing `ticket` (the canister pays the winner's ATA rent, ≈ 0.00204 SOL,
   when missing).
6. Phase `Drawn` for 30 days or `Committed` for 7 days → `expire_raffle`.
7. Treasury `distribute(i)`: pass `raffle = PDA ["raffle", module.listing]` as the last account; `apply_raffle_bps` after its
   24 h wait, like `apply_reserve_cap`.
