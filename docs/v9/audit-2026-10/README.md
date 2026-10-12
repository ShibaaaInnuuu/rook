# Rook Solana audit, October 2026

Scope: everything written for the Solana launch, which no earlier audit covered. Baseline commit is in each report's header.

| Area | Report |
|---|---|
| A1 rook_core: floor module, convert, math, units, params | a1-core-floor.md |
| A2 rook_core: registry, wrap (mint, redeem, tickets, drand), treasury, wake; account validation | a2-core-registry-wrap.md |
| A3 rook_backing, rook_layouts, the Meteora DBC / DAMM v2 integration | a3-backing.md |
| A4 canister Solana primitives: solkey, soltx, solsend, solrpc, solread, solix | a4-canister-sol-primitives.md |
| A5 canister flows on a Solana home: solhome, ethflow home_sol, frogsol, relaysol, convflow, rates/oracle, upgrade | a5-canister-sol-flows.md |
| A6 launch operations: ceremony-day1, deploy-later, launch-params, fund-status, the Meteora configs | a6-launch-ops.md |

**Round B (2026-10-11, the canister-only build: `docs/v9/canister-only-solana.md`).** Nothing below was in A1–A6; the programs of
A1–A3 are no longer deployed (decision D-l).

| Area | Report |
|---|---|
| B0 the author's modules reviewed the same day (cranks, pool desk, coins, the NFT flows on the books) and the fixes | b0-review-2026-10-11.md |
| B1 the intent desk and the chain links: `desk/`, `chainlink/` (ledger, gate, memo, recon, driver) | b1-desk-links.md |
| B2 the Solana primitives of this phase: `solix2/` (builders, guards, decoders, routes), `solkey` (the key map, the multi-signer rule), `solhome` (four lines, the timer), `solnft*` (membership, custody, buys, the Magic Eden judge) | b2-primitives.md |
| B3 the pool desk's decisions against DLMM (`pooldesk/plan.rs`), the raffle snapshot (`raffle_books`), the surface (operator and guardian methods), the launch pins | b3-desk-raffle-surface.md |

Fixes of Round B, the same day, each recorded under its finding with its PoC inverted into a `fixed_` test: every High (B1-01, B1-02, B3-01), every Medium (B1-03–B1-06, B2-01, B3-02–B3-05), and every Low (B1-07–B1-12, B2-02–B2-04, B3-06 (by B1-06), B3-07–B3-11), plus B1-I5/I6. Open: B3-07's public snapshot hour and the other Informationals (recorded, not changed).

**Round C (2026-10-12, what landed on `golive-2026-10-08` after Round B; baseline `79b4e60`).**

| Area | Report |
|---|---|
| C1 the Solana NFT flow: `solnftflow/` (the floor from Magic Eden hand-ins, the advance + purchase, custody, `mint_item` through the deferred writer, deliveries, undeliverable rules, its endpoints), its money-line wiring, its use of `solnft*`, `books/`, `chainlink/`, `custodycap` | c1-solnft-flow.md (1 High, 4 Medium, 3 Low, 6 Informational; PoCs in `tests/audit_c1_solnft.rs`) |
| C2 Ordinals multi-collection: `frogsol/multi.rs`, per-collection frog and custodian state, custody routed by root, dormancy, caps, the minimum purchase, the migration; the default-live decision (§4) | c2-ordinals-multi.md (1 High, 3 Medium, 4 Low, 6 Informational; PoCs in `tests/audit_c2_ordinals.rs`) |
| C3 Robinhood Chain (4663): the per-chain EVM sides, the SOL <-> 4663 Relay legs, per-collection caps, `token_bound`, the reconciler and floor feed per chain, the measurement | c3-robinhood-chain.md (3 High, 2 Medium, 4 Low, 6 Informational; PoCs in `tests/audit_c3_robinhood.rs`, `ops/test/audit_c3_robinhood.test.ts`) |
| C4 opening on demand and the runner integration: `coins_request_open` and its queue, the ingress filter, `coins_set_auto_open`, the registry, `launch_pins`; the runner's Ordinals, Magic Eden and band-gate steps; the site's "Open it" and the board | c4-opening-runner.md (1 High, 2 Medium, 3 Low, 8 Informational; PoCs in `tests/audit_c4_opening.rs`, `ops/test/audit_c4_runner.test.ts`; fixed 2026-10-12: 13 fixed, C4-07 recorded, the PoCs inverted to `fixed_` tests) |

Fixes of Round C, 2026-10-12, each recorded under its finding in its report with the PoCs inverted into `fixed_c<n>_…` tests: every
High (C1-01, C2-01, C3-01, C3-02, C3-03, C4-01), every Medium and Low, and most Informationals. Bounded rather than closed: C1-05 (a
buy capped at the 7-day lowest accepted floor plus the premium; an independent price source is the full fix) and C4-07 (the CoinGecko
band only catches a broken feed). Recorded, not changed: C1-I3, C1-I4, C2-09, C2-11, C3-10, C3-12, C3-15, C4-07. With the fixes, the
operator's default-live decision (every Inscriptions listing live at install, its first floor bounded by its launch floor; C2 §4) and
the 38 pins at the ceremony are in. Suites at the merge: canister 1,252, ops 180, monitor 283, site 224, rehearsal 4.

Method: scope and threat model; manual line-by-line review against an area checklist; a failing proof-of-concept test for every
High or Critical; severities Critical / High / Medium / Low / Informational (impact × likelihood); independent verification of
every finding; fixes reviewed separately.

