# Rook on Solana — programs

The Anchor 1.2.1 workspace for Rook's Solana launch. Design: `docs/v9/solana-migration.md`.

- `programs/rook_core` — registry, floor module, wrappers (`wX`), redemption tickets, treasury, wake desk.
- `programs/rook_backing` — the Meteora DBC partner integration and the per-coin backing vaults.
- `crates/rook_layouts` — the byte layouts one program reads of the other's accounts (rook_backing's `Vault`, rook_core's
  `Registry.owner`); each owning program tests its struct against them.

Build and test: `anchor build && cargo test` (LiteSVM tests load the built `.so` files from `target/deploy`).

Program keypairs in `target/deploy/` are local development keys only and are git-ignored; mainnet program ids are generated at
the ceremony.
