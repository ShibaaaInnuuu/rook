# Security review, 2026-09-26 (commit e04f49b)

Five parallel AI reviews of the Phase-1 stack. Totals: 1 Critical, 6 High, 18 Medium, 26 Low, 24 Informational.
Summary and fix plan: the "Rook Phase-1 Security Review" doc. Area reports:

- `A-curve.md` — bonding curve, factory, graduation, liquidity lock
- `B-floor.md` — FloorModule, wrapping, redemption, reserve accounting
- `C-treasury-roles-deploy.md` — treasury, sweeps and draws, roles, deploy scripts, go-live sequence
- `D-canister-timelock.md` — the ICP custody canister and the timelock (Rust)
- `E-services-workflow.md` — keeper, runner, reconciler, monitor, indexer, site; end-to-end workflows

`poc/` keeps each reviewer's proof-of-concept tests exactly as written against e04f49b (they are not part of any build).
Fixes turn the relevant ones into regression tests that assert the attack no longer works.
