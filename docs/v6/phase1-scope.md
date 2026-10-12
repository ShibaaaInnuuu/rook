# Phase 1: the exact launch scope

> **2026-09-27: the operator moved Phase 1 to Ethereum NFTs** (`docs/v7/phase1-nft.md`). This document stays the scope of the
> Bitcoin-frog stack (now Phase 2). Everything below still holds for it; the Ethereum NFT side's own paths, caps, install
> argument (`eth_nft`), services and go-live checks are in `docs/v7/phase1-nft.md` §4–§6 and §11.

Written 2026-09-24 on branch `phase1-scope` (from `5443aca`). Nothing here is deployed. Nothing was sent to a public chain,
and no key was used. Every ON/OFF below was checked against the code at `5443aca`, with file and line references. Where a
design doc says something different from the code, the code wins and §7 lists the difference.

**Phase 1** means the shared contracts, the **Bitcoin Frogs** stack and the site, with custody at **stage 0**:

- caps: 3 items and 10 ETH per collection; 6 items and 10 ETH globally; 10 ETH per item;
- outflow: 3 items a day per collection, 3 mints a day, 10 ETH a day, with a tripwire at 3×.

Only paths whose code gates are closed are switched on.

## 0. The honest risk statement (read this first)

- **The stage-0 caps bound custody, not the contracts.** `custody_caps` (H2) and `outflow_limits` (H3) bound what the
  canister holds, signs and pulls. They do **not** bound the ETH that sits in the 4663 contracts, and none of those contracts
  is audited (G1 is waived, §2). At stage 0 that ETH is:
  - each curve's raise, up to 30 ETH (5 normal, 8 early; `DeployStack.s.sol:364-367`). Anyone can launch, so there is no
    limit on how many curves exist;
  - the Floor Module's ETH-backed units, up to 40 items' worth (≈ 25 ETH at a 0.63 ETH frog floor). This number is not
    scaled by stage (`StackChecks.sol:132-133`);
  - the locked pools. These can never be withdrawn, and have no cap;
  - sweep-vault balances, the treasury, MintDesk escrow and OfferBoard bonds.

  The protection for all of this is the test suites, the rehearsals and the per-contract constants. An outside review is not
  part of it.
- **"Both devices compromised ⇒ at most 10 ETH lost" (G2 solo rule 2) is only true of custody.** Someone holding both devices
  holds the Safe. The Safe can:
  - change `ProtocolTreasury.setShares` and `setOps` **at once, with no delay** (`ProtocolTreasury.sol:98-115`), which
    redirects the treasury's income and its unallocated balance;
  - transfer ownership with no delay (Ownable2Step).

  Through the timelock with no veto, they can also upgrade the canister, and then mint unbacked wX up to H11's 3 a day and
  sell it into the pools. The site's Trust page has to say this, not the shorter sentence.
- **Redemption cannot be switched off** (`Wrapper.sol:429-458` has no pause, by design). So once any frog is in custody, the
  delivery path is live. That is why H8 is a must-do before go-live (§3.1), not a Phase-2 item.

## 1. Every path, ON or OFF in Phase 1

Gate references: C1–C11 and G1–G10 are from `docs/v5/launch-gates.md`; H1–H14 are from `docs/v5/custody-hardening.md`.
"Before handover" means the canister's controller-only setters. They can only be called while the deployer still controls the
buyer, because the timelock's only actions are Upgrade, Controllers and Roles (`timelock-rs/src/rules.rs:106`).

| Path | P1 | Why: the gates it rests on, and their status | How it is kept that way (verified) |
| --- | --- | --- | --- |
| **Curve launches** (`CurveFactory.launch`, `FrogCurve` trading) | **ON** | Contract-only; no custody key is involved before graduation. H11 and H12 are in code (`605aab0`). G1 is waived (§2) | Permissionless on chain, with no owner and no pause (`CurveFactory.sol:139-179`). It is live as soon as `deploy-stack.sh` wires the vaults (`DeployStack.s.sol:217-218`). The site launches only through a nest that has a factory (`Launch.tsx:48-49,124`), which means frogs only |
| **Graduation → item buying** (curve → `FloorModule.enqueue` → canister buys a frog → `wrapDeposit` to the module → `itemArrived`) | **ON, at stage-0 caps** | C1/H1 `97178ad`; C2/H2, C3/H3 `2562c0e`; C9/H11 (wrapper, 3 mints a day) `605aab0`; the module's `maxItemsPerDay` = 3. Two ordinals providers must agree (`f6ae169`), which **needs the UniSat key** (§3.2) | The last buy puts the item budget into the module and mints ETH-backed wX at once (`FrogCurve.sol:264-268`, `FloorModule.sol:583-597`). The canister converts that only through `set_floor_sink(module)` (`floornet.rs:32`, controller, **before handover**). Each conversion is: `itemArrived` custodian-only; cost ≤ `maxItemCost` = 1.03 × floor; 3 a day (`FloorModule.sol:459-487,527`); H2 room and reserve (`frogflow.rs:238,798`). **What stage 0 means here:** at most 3 frogs in custody (≈ 2 ETH at the frog floor). Past that the module keeps the ETH and the wX stays ETH-backed, and so burnable at the module. A graduation that would push ETH-backed units past 40 waits, and the curve refunds after its 3-day window (`FrogCurve.sol:347`). Frogs are bought only from offers the **operator** hands in (`frog_offer`, `frognet.rs:430`); the OfferBoard is OFF. **Since 2026-09-26 (`docs/v6/relay-conversion.md`) the module pays first**: `advance(maxItemCost)` (custodian-only, one open at a time, counted by H11 and H3 at the advance, H2 room reserved then) → the runner hands in a Relay quote → the canister judges it by the order hash and deposits → the BTC arrives at the float (3 deep) → the frog is bought with it → `wrapDeposit` → `settleAdvance(cost)`; a failed conversion is retried with a new quote (3 at most) or returned (`returnAdvance`). No operator float |
| **FloorModule mint / burn** | **ON** | Contract-only; the floor's move limit (±5% an hour) is in bytecode. It needs a **live keeper sender**: built as K3 (§9), not deployed | No owner, setter or pause (`FloorModule.sol:36`). Mint and burn revert `Stale` unless the floor was posted less than an hour ago with confidence ≥ 30% (`:372,409,830-832`). Limits: 5 items an hour each way; public ETH-backed ≤ 10 items; 25% kept as ETH reserve. If the keeper stops, holders of ETH-backed wX cannot exit at the module and have only the pools |
| **Redemption + delivery** (`redeem`, `redeemToBurn`, drand, custody send) | **ON** (cannot be off) | C3/H3 and C4/H4 `2562c0e`, with `require_finalized` and `require_event` hard-coded true (`custodynet.rs:348-350`). C6/H9 ticket read `2562c0e`. **C5/H8 done `72531db`** (K1): a delivery whose saved bytes are refused has its fee input spent back to custody before it settles (`conflict.rs`, `redeem.rs` `Conflicting`) | No pause exists on chain. A ticket needs an unpromised item (`Wrapper.sol:449`). Deliveries need UniSat to verify contents; without it they are refused as unverified and the ticket refunds by the 7-day rule (`Wrapper.sol:548-551`) |
| **Sweeps: RAFFLE and BURN** (`sweep` at the router, `openDraw`, `retire`) | **ON** | H12 step bound `605aab0` (`SweepVault.sol:247-258`). Nothing is signed by custody unless a winner redeems. It needs the keeper sender: built as K3 (§9), not deployed | `sweep` is anyone's, but needs a fresh keeper cap and the listing unpaused (`:297-302`). `openDraw` is keeper-only (`:370`). The owner proposes cap ceilings at deploy (`ETCH_CAP_CEILING`; they apply after 7 days). `sweepViaDesk` is anyone's; with the desk unserved, its escrow can be collected after the 6-hour TTL (`MintDesk.sol:106`) |
| **YOYO** (`relist`, `redeemToRelist`, relist tickets, protocol sales `itemSold`) | **OFF** | Custody-signed listings are a new outbound path (`ac88446`) that has had no review of its own (C5/H8 is done at `72531db`, but covers deliveries and returns only), and there is no Phase-1 need for it | `set_sale_policy` is **not called before handover**. Its policies are empty by default, so `saleflow::read` returns before reading relist tickets or sales (`saleflow.rs:75`). The keeper posts no cap to the YOYO vault, so its `sweep` and `relist` are refused (`SweepVault.sol:515-518`). The site hides the YOYO policy. A creator can still pick YOYO on chain: its fees then wait in the vault. A holder's `redeemToRelist` ticket is not delivered and refunds after 7 days |
| **MintDesk mint** (`request` → `fulfill`) | **OFF** | Not a gate problem. It would be a second inflow competing for the 3 slots, and the module already mints instantly | The desk is deployed, and `request` is permissionless with no pause (`MintDesk.sol:65`). The canister is given no desk: `set_item_config(factory, desk = None, [])` before handover (`frognet.rs:386`). Requests are never fulfilled, and `collect` refunds them after the 6-hour TTL. The site shows no desk |
| **OfferBoard hand-ins** | **OFF** | The ingress cost of refused `submit_offer` calls is open (`offer-handins.md` §7.1, no `inspect_message`). The reward has no funding route (§7.2) | `set_offer_board` is **not called before handover** (`handinnet.rs:31`). The board is deployed unconditionally for a Bitcoin stack (`DeployStack.s.sol:222-225`) but nothing funds it. Bonds refund after 24 h (`OfferBoard.sol:149`). The site hides Bots |
| **Holder deposits** (EWD1 → `wrapDeposit` to a holder) | **OFF** | C6/H9 for deposits' EVM side is **open**: they are still read at the fast pin | Anyone can send a frog with EWD1 to custody (`frogflow.rs:976`, `deposits_pass`), but it mints only after the operator supplies `frog_proof` (`frognet.rs:452`). The operator never does. A stray deposit is returned with `refuse_deposit`: H3 allows 10 returns a day, and the H4 `DepositReturn` policy applies. The site has no deposit UI |
| **Wrap** (v2: an AutoBuyerVault acquisition → `Wrapper.wrap` → `WrapClaim`) | **OFF** (unreachable) | — | `open_purchase` is refused on 4663 (`canister.rs:1203`). The policy loop is inert without a venue and a bridge (`looper.rs:95`), so no acquisition ever exists for `wrap` to use (`Wrapper.sol:374-376`). Caveat: `Launcher.launch` (Pons) is permissionless on the unpaused listing (`Launcher.sol:57-60`). Such a coin's fees go into the AutoBuyerVault, and **no path takes them out** until a v2 policy exists. The site offers no Pons launch on this listing; set `maxInFlight` to the minimum |
| **Bootstrap loans** | **OFF** | Not a gate problem. Half of each loan is a float that makes conversion demand the 3 slots cannot serve | The Safe calls `setBootstrapLoan(0)`. The bootstrap share is set to 0 (next row). `bootstrap` is then worth nothing (`ProtocolTreasury.sol:200`) |
| **Treasury split** | **ON, as 90/0/0/10** | Contract-only. Setters are instant (§7) | Before any income: `setShares(9000, 0, 0, 1000)` (reserves, floats, bootstrap, ops) and `setOps(<ops address>)`. Floats are 0 for the same reason as loans. `allocate` and `distribute` are anyone's. Restore 40/30/20/10 at stage 1 |
| **NFT collections** (NftWrapper, NFT sweeps, `nft_*`) | **OFF** | C10 (NFT delivery) is **not built**: nothing calls `verify_evm`, and there is no `transferFrom`. H9 for NFT arrivals is open | Not deployed: leave `ETCH_NFT_*` unset (`DeployStack.s.sol:79,138`) and do not run `deploy-nft.sh`. The canister gets `evm_chains = None`, so every `nft_*` path is inert |
| **Home-chain vaults** (Ethereum HomeVault; the 4663 HomeListing, adapter and escrow) | **OFF** | Separate stack. No 4663 deploy script exists (`home-chain-vault.md` §10.6) | Not deployed. `script/homechain/DeployHome.s.sol` requires chain 1 and is marked never-broadcast |
| **Rentals** | **OFF** | The canister side is spec only (`floor-module.md` §9.6) | There is no canister path. The Rent page is a description only |

The canister's own switches, checked in code:

- `tick` is operator-only, and there is no timer (`canister.rs:1129`). Nothing acts until the operator's runner calls it.
- The pause stops the tick between passes.
- `custody_caps = None` and `outflow_limits = None` at install mean exactly stage 0. `validate_change(None → arg)` refuses
  anything higher (`custodycap.rs:167,205`; `outflow.rs:95`).
- Unlisted collections are denied by default.
- Every `rehearse_*` update method is dead on 4663 plus Bitcoin mainnet (tested since `df2f13c`: `every_rehearsal_guard_refuses_a_production_install`).

## 2. The operator's decisions, recorded

**No cap on open curves in Phase 1. Decided 2026-09-24.** Asked whether to limit how many curves may be filling at once (so the ETH
in unaudited curve contracts stays near 10–16 ETH), the operator declined: curves stay unlimited, each bounded only by its own raise
target (5 ETH normal, 8 ETH early, 30 ETH ceiling). §0's statement stands, and the Trust page must say plainly that the stage-0 caps
bound custody only and that the number of open curves, and the ETH in them, is not capped.

**G1: no outside audit. Accepted for stage 0 only, 2026-09-24.** The operator is solo and has decided not to commission an
audit. In its place:

- **Tests:**
  - forge suites (628 at `923ef11`; run the full count on the go-live commit);
  - canister `cargo test`: **406 pass, 0 fail**, run for this document; **432 pass, 0 fail** at `df2f13c` (K1 and K2 added 18);
  - timelock: **8 pass, 0 fail**;
  - the fuzz and invariant campaigns (`FOUNDRY_PROFILE=deep`).
- **Rehearsals:** `scripts/golive-dryrun.sh all` and `stack` (result in §6), and the replica rehearsals in `autobuyer/`.
- **Internal passes:** `docs/phase3.md`, `docs/security/`, and the custody threat model with H1–H4 wired.
- **Caps bound the loss of custody:** 3 items and 10 ETH per collection. §0 says what they do not bound.

**Consequences, accepted:**

- No stage above 1 without an outside review. G7 stage 2 requires "audit fixes re-reviewed", and stage 3 requires a second
  independent review, so without G1 the ladder stops at stage 1.
- The bug bounty (G6) is the only outside-eyes mechanism left. It is listed as an operator action (§3.3).

**G2: the solo ceremony, as written in `launch-gates.md` §G2 (solo variant), with one correction the code forces.**

- Device A holds the proposer. Device B, a different machine, holds the guardian. Both are software keys: there is no
  hardware wallet, which is accepted at stage 0 (rule 5).
- The Safe is 2-of-2, one key on A and one on B.
- The only controller is the timelock, with 72 h. The deployer identity is deleted on the log.
- The public ceremony log stands in for the witnesses.
- Caps stay at stage 0, and never go past stage 1 until two outside guardians hold identities.

**The correction.** The production timelock refuses fewer than 2 proposers, fewer than 3 guardians, a `k` outside
`2 ≤ k ≤ g − 1`, and a principal in both roles (`rules.rs:44-80`). So "k = 1 while solo" cannot be installed. The operator
therefore holds:

| Role | Identities | Where they live |
| --- | --- | --- |
| Proposers | p1, p2 | p1 on device A; p2 on device A as an encrypted offline backup (losing every proposer ends upgrades) |
| Guardians | g1, g2, g3, with k = 2 | all on device B |

Any one guardian still pauses. A veto or a resume takes two identities, both on B. This satisfies the code floors and adds no
security over one identity: the bound stays the 72 h delay plus the stage-0 caps. The alternative is a "solo" build flag,
which would change the module hash and weaken the floors. It is rejected.

## 3. What must be DONE before Phase 1 goes live

### 3.1 Code

| # | What | Size | Why |
| --- | --- | --- | --- |
| K1 | **H8 conflict-out (C5)**: a delivery or return that was broadcast and then refused gets its fee input spent back to custody. Entry point named under H8 in `custody-hardening.md`; H4's `ConflictOut` already exists. **Done `72531db`** (the spec's open points resolved first, `6d3bc4e`) | M (days) | Redemption cannot be switched off (§0) |
| K2 | **Bitcoin mainnet install (C7)**: add `"mainnet"` to `KNOWN_NETWORKS` (`canister.rs:138`) — the exact string `ordnet::providers()` matches (`ordnet.rs:34`) — and a mainnet checkpoint in `btcsend::checkpoint` (`:230`). **Done `df2f13c`** (the check published first, `fb7bc26`) | XS code + verification (§3.2) | A mainnet install panicked in `validate` before it |
| K3 | **Production keeper sender**: `FloorModule.postFloor` at least every hour, `SweepVault.postCap` for RAFFLE and BURN, `openDraw` with a published snapshot. `keeper/src/` stays the dry run and `keeper/live/` the one-transaction tier-2 test; the sender is new. **Built 2026-09-25 on `p1-keeper`** (`keeper/service/`, design `docs/v6/p1-services.md` §1, deploy `docs/deploy/services-v1.md` §2): the floor from the existing two-provider feed only (every 30 min, never an unagreed floor), caps floor + 10% inside H12's step, gap and ceiling, never the YOYO vault, draws at a published daily slot with a `tools/snapshot` file it serves; `KEEPER_CHAIN_ID` required and pinned per RPC, the key read in `service/signer.ts` alone and refused unless it is `registry.keeper()` and every module's keeper with no code, per-tx and per-day gas caps, one transaction in flight, bytes journalled before the send, dry run unless `KEEPER_MODE=send`. Fork run ALL PASS (§9). **Not deployed** | M | Without it the module is stale (mint and burn refused, graduations stuck) and sweeps are refused. ~~The module's keeper is **immutable**~~ Since 2026-09-26 the module reads `registry.keeper()` (rotated in 7 days, paused at once by the Safe or the guardian; `docs/audit/2026-09-26/fixes-B-C-keeper-draws.md`) |
| K4 | **Operator runner**: call `tick` on a schedule, plus a tool that turns a venue listing into `frog_offer(id, inscription, psbt, to_sign, proof)`. **Built 2026-09-25 on `p1-keeper`** (`ops/`, design `p1-services.md` §2, deploy `services-v1.md` §3): the runner ticks every 10 min as the operator (identity from `OPERATOR_IDENTITY_PEM`, read in `ops/src/identity.ts` alone), never overlapping; the offer tool mirrors `frogflow::accept` / `frogbuy::judge` rule for rule in TypeScript, prints each input the canister would sign with its key-path sighash, and calls `frog_offer` only with `--send`. Replica run ALL PASS (§9). The venue input is a seller-signed PSBT or UniSat's `create_bid` answer (swagger shape, unverified); nothing calls a venue's bid endpoint (F3). **Not deployed** | S–M | Without it no frog is ever bought |
| K5 | **Site Phase-1 mode** (`web-rook/`): hide Bots, Rent and the YOYO policy, with no desk and no Pons launch; bundle the 4663 stack deployment (`TxButton` refuses without one, `ux-and-reserves.md` §7); the Trust page states §0's two bullets; `security.txt` | S–M | So the site does not offer OFF paths. **Built 2026-09-25 (§8.3, §8.5)**: `VITE_ROOK_PHASE=1`; the stack file must be copied into the build (`npm run sync`) once `deploy-stack.sh` has written it; `security.txt` is written only once O9 names a contact |
| K6 | **Watcher v2 and custody canary (H5, G9)**: decode and diff upgrade arguments; alert on any custody-address outbound not in a canister record. **Built 2026-09-25 on `p1-monitor`** (`monitor/`, design `docs/v6/k6-watcher-canary.md`): every proposal's argument decoded (type table checked for unknown fields) and diffed against the last executed one, loosening/tightening in words, module hash against the published builds; certified controllers and module hashes; custody, float and buyer addresses on mempool.space + blockstream (agreeing) matched by txid against `redemptions`/`holder_deposits`/`frog_buys`/`sale_state`/`purchases`; the Wrapper's and FloorModule's custodian-gated events on both C8 RPCs; Telegram, deduplicated, daily heartbeat, `--test-alert all`; Dockerfile + `railway.json` + `/status`. 48 tests, fixtures from the canister's own Rust encoding and real mainnet transactions. **Wave 2 (2026-09-26, `docs/audit/2026-09-26/fixes-W2-monitor.md`):** the keeper's `FloorPosted`/`CapPosted`/draw events, the pause, the guardian and role rotations; every opened draw's root recomputed on both RPCs; an optional guardian key that pauses the keeper and cancels an unpinned draw on a confirmed trigger; stale floor and caps, unresolved tickets and draws, and every service's `/healthz` paged; a disputed RPC range no longer blinds it (E-06); Telegram fails closed (E-08); the audit-D canister types. **Not deployed** | M | G9 |
| K7 | **Reserves reconciliation run hourly (G5)**: the page and API exist but have never met a real chain. Add `custody_caps()` and the stage, show red when older than 3 h, and wire it to an alert. **Built 2026-09-25 on `p1-keeper`** (`ops/src/reconcile.ts`, design `p1-services.md` §3, deploy `services-v1.md` §4): the indexer's `WrappedService` over the stack file, hourly, plus `custody_caps()` read anonymously and the G7 stage it implies (above the stack's stage is red); a report older than 3 h is served red, and a watchdog alerts it; Telegram through the monitor's channel (`monitor/src/alerts.mjs`) on red, on new reasons, every 6 h while red, once on recovery; its own page at `/`. Met a fork of 4663 and a local replica: GREEN (§9). The site's reserves page (`web-rook/`) does not show the caps, stage or staleness yet (a K5 follow-up). **Wave 2 (2026-09-26):** the module's real ETH against its books, units and floor, and a stale floor, are red (E-05); locations need two ordinals providers (E-13); cannot-read and 6 h unverified are paged (E-08a); `--test-alert` of its own. **Not deployed** | S | G5 |
| K8 | **4663 deploy signer**: only if the operator does not buy a hardware wallet. Add a password-keystore (`--account`) path to `deploy.sh`, `deploy-stack.sh`, `deploy-wrapclaim.sh` and `chain-guard.sh`, plus their refusal tests. Until 2026-09-25 every 4663 script required `--ledger` or `--trezor` and refused everything else | S | The deploy cannot run otherwise. **Built 2026-09-25 (§8.1, §8.5)**: `--account <name>`, checked in `chain-guard.sh`'s `send_4663`; `deploy-nft.sh` stays hardware-only (NFT is OFF) |
| K9 | `golive-check.sh` in stack mode: compare `listing.btcVault` with `wrapper.custodyAddress` (until 2026-09-25 only in wrapper-file mode), and print the canister's `custody_caps()` next to the stack's stage | XS | A mismatch fails silently. **Built 2026-09-25 (§8.2, §8.5)**: `--canister <buyer>` (optional) reads `custody_caps()` with dfx and fails a collection whose cap is not the stage's |
| K10 | Doc fixes (§7) | XS | Stale claims. **Done 2026-09-25**: all nine resolved in the docs; three point at code that could move (§7 items 3 and 9, §8.5) |

### 3.2 Ops

| Item | What | Status |
| --- | --- | --- |
| Canister mainnet install | `docs/deploy/canister-mainnet.md` steps 1–7, with the install argument below | unblocked by K2 (`df2f13c`); not done |
| **C7** (`KNOWN_NETWORKS` and checkpoint) | Candidate height 900,000 → `…0d96968a` (`canister-mainnet.md`). Solo, there are no two people with their own nodes. Check mempool.space, blockstream.info and a third explorer, and publish all three in the ceremony log for anyone to repeat. This is a deviation, accepted with G2 | **done** 2026-09-25: five explorers agree and the header's hash and work are checked in a test; published in `canister-mainnet.md` ("The published C7 check", `fb7bc26`), to be copied into the ceremony log. A node check is still to be appended |
| **C8** (providers) | See the provider list below this table | 2 of 3 met |
| **G8** (cycles) | Buyer 20 T and timelock 2 T, freezing threshold 90 days, set **before handover**. Top-up automation. Alarms at 5 T, 2 T and frozen. Drill 1 (`pre_upgrade` at 10× stage 3) and drill 2 (out of cycles) on a replica | not done |
| **G9** (monitoring) | `scripts/timelock-watch.mjs` (built) plus K6 on an always-on host that is not the keeper's, posting to the operator's phone. A test alert of each kind is received | **K6 built, not deployed** (2026-09-25; wave 2 2026-09-26). Left for the operator: a Railway project apart from the keeper's (`k6-watcher-canary.md` §7, §10), the bot and chat, the config (ids, addresses, `install_arg_hex`, `builds` in `deployments/canister-ic.json`, the stack file, `draws.exclusions`, the services' `/healthz` URLs, and the guardian if O11 says yes), then `--test-alert all` on the monitor (**16 kinds**) and `--test-alert` on the reconciler (3) |
| **G5** (reserves) | K7, live at a public URL | **K7 built, not deployed** (2026-09-25): the reconciler in the monitor's Railway project with `CANISTER_ID`, the bot and chat, and a public domain (`services-v1.md` §4) |
| **G10** (incident) | One end-to-end drill: pause from B, veto a harmless proposal with two guardian identities, resume | not done; the runbook, with the alert expected at each step, is `docs/v6/g10-incident-drill.md` |
| UniSat key | `set_unisat_key` after install and **after every upgrade** (it lives on the heap only, `ordapi.md` §2.1). Use a read-only plan: the subnet's replicas can see it | not held |
| Custody fee pool (audit D, H-4) | Send custody one plain-sat output of **at least 20,000 sats** (the operator's minimum, 2026-09-26: not 100,000, ≈ $84, held for fee spikes; one setting, `golive-check.sh --min-fee-pool-sats` and the monitor's `min_fee_pool_sats`) (nothing else in it) after the gates allow value in, then `add_custody_fee_input("<txid>:<vout>", <sats>)`; `golive-check.sh --canister` FAILs without one usable entry that large. The canister's `MAX_CONFLICT_FEE_SATS` stays 100,000 sats, a ceiling, not a requirement (round 3, `fixes-R3-canister.md` §2.2: only an entry that large lets a re-priced conflict-out reach it): In a large fee spike a stuck delivery is cancelled more slowly; the gated Wrapper (refunds only after markUndeliverable or 30 days) keeps that from becoming a loss. Deliveries and returns take their fee from the pool and verify it on every explorer and on ord at use; the change of each confirmed spend refills it. With an empty pool they fall back to listing the custody address, which a stranger breaks by sending it more than 500 outputs | new 2026-09-26 |

**C8, the providers:**

- **4663:** `rpc_urls = ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"]`. The runbook's dRPC is
  **dead for this use**: measured today, it refuses every state read at an explicit block ("Unknown state. First available
  state is 1"), and the canister reads only at pinned historical blocks (`pin.rs:123`). Robinhood keeps about 6,220 blocks;
  ordofi is archive. Both publish AAAA records.
  **Re-measured 2026-09-26 (audit D, H-1):** Robinhood keeps 6,201 blocks of state, the finalized block is 8,096–12,023 behind
  the head, and ordofi refuses `eth_getLogs` over more than 10,000 blocks, so the delivery path as it was (state at the finalized
  block, one 8M-block log query) could never run on this pair. The canister now reads state at the fast pin only and takes
  finality from the wrapper's `Redeemed`/`Resolved` events, paged 8,000 blocks at a time (`docs/audit/2026-09-26/fixes-D-canister.md`).
  Re-measure all four numbers at go-live.
- **Bitcoin:** mempool.space and blockstream.info.
- **Ordinals:** UniSat (keyed), ordinals.com and Ordinals Wallet.
- **Not met:** H10's "≥ 3 providers, one our own node". The code requires 2, all agreeing (`canister.rs:244-248`). This is
  accepted in writing for stage 0. A Nitro node is the fix.

**The install argument:**

- `chain_id = 4663`, Bitcoin `"mainnet"`, `key_name = "key_1"`, `accept_production_key_name = true`;
- `rpc_urls` and `btc_api_urls` as under C8 above;
- `custody_caps = null` and `outflow_limits = null` (which mean stage 0);
- `holder_deposits = null` (off: the deposits pass reads and returns nothing; audit D, H-2) and `wrapper_from_block = null` (no
  ticket can exist before the canister names the wrapper, so the ticket-log scanner's first tick is idle; set it to the Wrapper's
  deployment block if the step-9 reinstall comes after any redemption could have happened). Both fields are new (`opt`) since
  2026-09-26: an argument encoded before then still decodes, but re-encode it with the new `.did` so the watcher's decoder (K6)
  sees every field;
- `evm_chains = null`, `bridge_allowlist = vec {}`;
- `frog_funding = null` (Relay at 300 bps, no operator float: `docs/v6/relay-conversion.md` §3.5);
- `operator` = the runner's identity;
- `vault` = the AutoBuyerVault from `list-auto`, and `wrapper` = the stack's Wrapper, both set by a pre-handover reinstall.

### 3.3 Operator actions

| # | Action | Needed for |
| --- | --- | --- |
| O1 | Register **rook.fun**; registrar lock and DNSSEC (H14) | the site |
| O2 | Get a **UniSat API key** (read-only plan) | deliveries and buys |
| O3 | **Rotate the OpenSea API key** (it is not used in Phase 1: the NFT side is OFF) | hygiene |
| O4 | **Rename the Railway project** | ops |
| O5 | **Buy one hardware wallet (≈ $80) for the EVM deployer**, or accept K8. This is recommended: it removes a code change, and G2 rule 5 makes it the first purchase anyway. K8 is now built (2026-09-25), so either works: `--ledger`/`--trezor` or `--account` (`docs/deploy/key-ceremony.md`, "The deployer as a keystore") | the 4663 deploy |
| O6 | Create fresh, code-less EOAs for treasury, keeper, purchaser, attestor, ops and (O11) the guardian; check each with `cast code` (EIP-7702 sweepers, `trust-model.md`; `golive-check.sh --phase1` fails on any with code). Create the Safe 2-of-2 on 4663 (A and B) | `deploy.sh` |
| O7 | **Fund:** | |
| | • the deployer: ≈ 0.03 ETH on 4663 | |
| | • the keeper: gas for hourly posts | |
| | • cycles: ≈ 25 T | |
| | • ~~the float: plain sats for about one frog plus fees~~ **Not needed since 2026-09-26** (`docs/v6/relay-conversion.md`, the operator's option (b)): the module advances each frog's own budget to the canister's EVM address and the canister converts it to BTC at the float through Relay; `frog_funding = null` in the install argument. **The operator funds neither a float nor more than the 20,000-sat custody fee pool below.** | |
| | • the canister's EVM address (`evm_address`): ≈ 0.002 ETH of 4663 gas, once, for its first transactions (not a float: each advance then keeps 0.0002 ETH for its own purchase's gas) | |
| | • the custody address: no value before the gates allow it (`launch-gates` G2 step 4); **then its fee pool** (audit D, H-4): send custody one plain-sat output of **at least 20,000 sats** (the operator's minimum, `golive-check.sh --canister --min-fee-pool-sats`, default 20000, requires it; the conflict-out's 100,000-sat fee ceiling is unchanged, and an entry under it is a note: in a large fee spike a stuck delivery is cancelled more slowly; the gated Wrapper (refunds only after markUndeliverable or 30 days) keeps that from becoming a loss) (nothing else in the transaction's output to custody, no inscription or rune), wait for it to confirm, and call `add_custody_fee_input("<txid>:<vout>", <sats>)` as the operator. Deliveries and returns take their fee from the pool; without it they fall back to listing the custody address, which a stranger breaks by sending it more than 500 outputs | |
| | • the guardian (O11, if chosen): a little ETH on 4663 for gas (two transactions at most) | |
| O8 | **Run the ceremony on two devices**: p1 and p2 on A, g1–g3 on B; `deployments/roles.json`; reproducible build on both machines; handover; blackhole; delete the deployer identity on the log | G2 |
| O9 | **Bug bounty (G6)**: publish a security contact. Either fund a reserve of at least the Critical floor (5 ETH) or accept its absence in writing. Recommended, since G1 is waived | G6 |
| O10 | Choose the second collection | **not needed for Phase 1** |
| O11 | **Decide on the guardian** (`fixes-B-C-keeper-draws.md` §8 item 2; `fixes-W2-monitor.md` §1.4). Recommended: yes — a fresh code-less EOA whose key is `MONITOR_GUARDIAN_KEY` in the monitor's own Railway project (never the keeper's or the runner's), named by the Safe's `setGuardian`. It can only pause the keeper and cancel an unpinned draw, and the monitor uses it only on a confirmed bad draw root (both RPCs) or a runaway floor. Without one, only the Safe (two devices) can pause, and the go/no-go runs `golive-check.sh --phase1 --no-guardian` with the choice written in the ceremony log | go-live step 7 |

### 3.4 Go-live checklist, in order

| # | Step | Who |
| --- | --- | --- |
| 1 | K1–K5 and K9 merged. Then pass: `forge test`, `cargo test` (canister and timelock), `script/deploy.test.sh`, `scripts/golive-dryrun.sh all` and `stack`. **The gated Wrapper and this canister build ship together** (`docs/audit/2026-09-26/review2/integration-R3.md`): the Wrapper refunds a resolved ticket early only on the custodian's `markUndeliverable`, else `LATE_DELIVERY_WINDOW` (30 days) after its deadline, and only a canister built from round 3 on sends that attestation and delivers as TRUC (version 3). Never deploy one without the other | developer |
| 2 | O1–O7: domain, UniSat key, OpenSea key, Railway, hardware wallet, role EOAs and Safe, funding | operator |
| 3 | Set up devices A and B; create p1, p2, g1, g2 and g3; publish `deployments/roles.json` | operator |
| 4 | Reproducible build on A and on B; the hashes must match each other **and the hashes recorded in `docs/audit/2026-09-26/review2/integration-R3.md`** (round 3: `buyer.wasm` `36ae6e24876d3b697f47c68bf3fcdd7e054cd94f4791fac60707f261c575ef59`, `timelock.wasm` `92248ca7ecb160c093665a8c19b7894cdf7f4a13f74746f3794f4a6e972c48f1`, `timelock-rehearsal.wasm` `742bf4748d805fe47c8ef081076054ce783f90e7774243fa554cd4561b9758b1`; if integration-R3.md records newer ones, those). Record them in `deployments/canister-ic.json` and in the ceremony log | operator |
| 5 | Create the buyer and the timelock (fresh ids, never a rehearsal id). Install the buyer with the argument above (`wrapper = null`, placeholder `vault`). Read `evm_address`, `custody_address`, `float_address` and `buyer_address`; they must be distinct | operator (deployer identity) |
| 6 | Contracts, in this order: | operator (hardware wallet or the deployer keystore, K8), then Safe A+B |
| | a. `deploy.sh` → the Safe calls `wire` and `proposeRoles` (the first roles apply at once) | |
| | b. `list-auto.sh` → the Safe sends `listInscriptionsAuto` (buyer = `evm_address`, btcVault = `custody_address`, minimum `maxInFlight`) | |
| | c. `deploy-stack.sh` with `ETCH_STAGE=0`, `ETCH_CAP_CEILING` set (required on 4663) and `ETCH_GUARDIAN` (O11; unset if declined), `ETCH_NFT_*` unset. Do **not** run `deploy-wrapclaim.sh` or `deploy-nft.sh` | |
| 7 | Safe owner steps (DeployStack prints each with its calldata): | Safe A+B |
| | • `WrapClaim.initMintLimit(3)` and `WrapClaim.wireFloorModule(module)`, **before any mint** (`Wrapper.sol:206,258`; the wrapper refuses a module that does not name it, B-05) | |
| | • `setOps` and `addCollection` | |
| | • `directory.add(factory)` | |
| | • `ListingRegistry.setGuardian(<the guardian EOA>)` (O11), unless declined in writing | |
| | • `proposeCapCeiling` ×3: the **first ceiling applies at once** (there is no acceptance to wait for) | |
| | • ~~`setShares(9000, 0, 0, 1000)` and `setBootstrapLoan(0)`~~: not needed since 2026-09-26, they are the treasury's constructor defaults (C-07); `golive-check.sh` fails if they differ at stage 0 | |
| 8 | `golive-check.sh --phase1` with the stack file, `--safe-owners <device A>,<device B>` and `--before-reinstall` (the canister names the wrapper only after step 9's reinstall); attach its output to the log. `--phase1` fails without the stack file, and on anything step 7 left undone, a keeper paused, a pending ownership transfer or rotation, a treasury split other than 9000/0/0/1000, an owner that is not a real 2-of-2 Safe (a canonical Safe 1.3.0/1.4.1 proxy on its canonical singleton, no module, no guard, code-less owners holding no role key, no fallback handler but a canonical CompatibilityFallbackHandler, and a `SafeSetup` event showing no `setup()` delegatecall but the canonical SafeToL2Setup: R3-check L-H; `SAFE_FROM_BLOCK` if the RPC cannot read its logs from block 0; created by a canonical SafeProxyFactory with `setup()` as its initializer, the `ProxyCreation` in the same transaction as the `SafeSetup`, whose owners and threshold are the chain's and `--safe-owners`: R4-check L-S1 — so create the Safe with the canonical factory in one transaction, as the Safe apps do), no `--safe-owners` (required), a role key with code (EIP-7702), a module or desk payee other than the stack file's `payee`, a wrapper `ONE_ITEM()` other than the stack's `itemUnits`, a module rate or ETH cap other than the Phase-1 defaults at that unit, a listing whose pause differs from the stack's `listingPaused` (and a stack file missing any of the three), any read that fails, and no guardian (`--no-guardian` if O11 declined one: a WARN). Run it again after step 9 **with `--canister <buyer>`** instead of `--before-reinstall`: every collection's cap must then read `ok` (fixes-R3-services.md §7), and, once step 11 has funded it, the custody fee pool must hold one usable entry of at least 20,000 sats (`custody_fee_pool()`; FAIL otherwise; `--min-fee-pool-sats` sets another minimum; an entry under 100,000 sats passes with a note) | operator |
| 9 | Pre-handover canister setup: | operator (deployer identity) |
| | • reinstall with the real `vault` and `wrapper` | |
| | • `set_floor_sink(module)` | |
| | • `set_item_config(factory, null, [])` | |
| | • **not** `set_offer_board`, `set_sale_policy` or a desk | |
| | • freezing threshold 90 days; top up to 20 T | |
| 10 | Install the timelock (2 proposers, 3 guardians, k = 2, 72 h). Check `status()`. Handover. Timelock at 2 T and 90 days. Blackhole. `dfx identity remove` on the log. Publish the verification block | operator |
| 11 | `set_unisat_key`. Fund the custody fee pool with one plain-sat output of **at least 20,000 sats** (the operator's minimum; §3.3) and `add_custody_fee_input` (O7), then run `golive-check.sh --phase1 --canister <buyer>` again: it FAILs until the pool holds that entry. **Before value enters custody (round 3, TRUC):** confirm both configured explorers accept a version-3 transaction on `POST /tx` (Bitcoin Core ≥ 28 behind them), and run one delivery built by this canister code through signet (`autobuyer/signet-rehearsal.mjs`), relayed and mined; if an explorer refuses version 3, do not go live on it. **Start the monitor's holder index at stack deploy** (its `draws.providers` back-fill from the stack file's first block, so a draw's snapshot is never older than the index; `monitor/scripts/measure-index.mjs` gives the times). Start the reserves job (K7) and the watcher and canary (K6, with the guardian key if O11); the keeper sender (K3) in a dry run, read its `/status`, then with the key and `KEEPER_MODE=send`; then the runner (K4), with its Relay step on (`RUNNER_RELAY`, default on; `docs/v6/relay-conversion.md`). **Before the first graduation:** send `evm_address` ≈ 0.002 ETH of gas (O7), and re-read the pinned Relay depository and its `allocator()` on 4663 (`cast code`/`cast call`, the design's §0.1): if either changed, stop and review before any conversion. Both alert paths are tested (`--test-alert` on the monitor and on the reconciler). The order and every variable: `docs/deploy/services-v1.md` §5 | operator |
| 12 | Drills: | operator |
| | • pause from B | |
| | • veto a harmless proposal (2 guardian identities) | |
| | • resume | |
| | • the canary sees a test inbound transfer to custody | |
| | • G8 drills on a replica | |
| | • the G10 drill, including its keeper step (pause and unpause the keeper; `g10-incident-drill.md` §5b) | |
| 13 | The site goes to Phase-1 mode with the stack deployment: copy `deployments/4663.json` and `deployments/stack-4663.json` in (`npm run sync` in `web-rook/`) **and commit them with `src/content/`** (the Docker build's context is `web-rook/` alone and ships what is committed), build with `VITE_ROOK_PHASE=1` and, under the dormancy, `VITE_COLLECTIONS_URL=<the runner's origin>/collections` (a Dockerfile `ARG`: the Launch page's "warming up" and "queued" words and its capacity line; without it the page still offers the wake from the chain alone, independent review DI-L-1), and check the Trust page names the Safe and the treasury from the file. The Trust page states §0 | operator |
| 14 | Go/no-go: every row above Yes, and the reserves page green | operator |

## 4. Phase 2 and later: which gates open which paths

A cap raise follows G7 on top of this table.

**Stage 1** needs:

- 4 weeks at stage 0 with no incident of severity High or above;
- every canary alert explained;
- the reserves page green throughout;
- the bounty live for 4 weeks;
- a hardware wallet for the roles (G2 rule 5).

**Never past stage 1** until two outside guardians hold identities (G2 rule 1). **Stage 2** also needs the audit fixes
re-reviewed, which is impossible without G1.

One problem affects several paths below. The canister's configuration setters (`set_offer_board`, `set_sale_policy`, the
desk) are **controller-only**, and after handover the timelock cannot call them. Turning any of these on needs either a
timelocked `propose_controllers` round-trip or a code change that lets an upgrade argument carry them. The code change is
**S, recommended**, and it is a prerequisite for the first four rows.

| Path | Opens when | Also needs |
| --- | --- | --- |
| Holder deposits | H9 for deposits' EVM side at the finalized pin (C6 complete) | an operator tool for `frog_proof`; stage 0 or 1 |
| OfferBoard | `inspect_message` for `submit_offer`; a funding route for the reward | the setter change; K6 watching slashes |
| MintDesk | nothing in the gates | the setter change; stage 1 (more slots) |
| YOYO and protocol sales | H8 (K1) live for a stage; a review of the custody-signed listing path | `set_sale_policy`; keeper caps on the YOYO vault |
| Floats and bootstrap loans | stage 1 (the slots can serve the demand) | `setShares(4000, 3000, 2000, 1000)`, `setBootstrapLoan(2 ETH)` |
| NFT collections | C10 (delivery with `transferFrom` and `verify_evm`); H9 for arrivals; H10 providers per chain | `evm_chains`; the `NftWrapper` deployed from the timelock (`deploy-stack.md` §6); per-collection caps by floor |
| Home-chain vaults | a 4663 deploy script; an Ethereum timelock; the bridge round-trip rehearsed | its own review (P0 Solidity) |
| v2 wrap (Pons) | a venue and bridge policy configured on 4663 | none planned |
| Stage 1 → 2 → 3 | the G7 entry criteria | G1 (§2) |

## 5. What this scope does not change

The pool LP is never withdrawable. Redemption is never pausable. The payee and treasury of the module are immutable; its keeper
is the registry's (amended 2026-09-26: rotatable in 7 days, pausable at once, `docs/audit/2026-09-26/fixes-B-C-keeper-draws.md`).
And the frog listing's `buyer` is permanent. Phase 1 commits to all four.

## 6. The rehearsal, re-pointed and re-run (2026-09-24)

`scripts/golive-dryrun.sh stack` forks 4663 from an archive RPC. The previous default, `robinhood.drpc.org`, stopped serving
historical state on 2026-09-24. Measured today, every state read at an explicit block number, head included, answers "Unknown
state. First available state is 1".

The new defaults:

- `STACK_FORK_RPC` = `https://rpc.ordofi.network`. It is archive, and served state 2,000, 13,000 and 50,000 blocks back.
- `STACK_FORK_BACK` = **2,000**. The endpoint is load-balanced, so the fork block has to be one every backend holds.

At ~0.1 s blocks, 2,000 blocks puts the fork's clock about 205 s behind the wall clock. The drand round a draw pins is then
about 13 minutes in the future, and the script waits for it. Setting `STACK_FORK_BACK=13000` removes the wait when the
upstream serves that deep.

**As run** (fork at block 71,126,701, head 71,128,701, under chain id 9466301, port 8651, all local):

**DRY RUN: ALL PASS**, exit 0, finished 2026-09-24 01:56 EDT.

| Step | Result |
| --- | --- |
| `deploy.sh`, `list-auto.sh`, `deploy-stack.sh` | ok; stack file written, one collection |
| Precompile warm-up | ok |
| `golive-check.sh` | **116 checks ok**, plus the three expected cap-ceiling notes |
| The five stack refusals | all refused |
| Launch | first buy paid the 50% launch fee (5000 bps) |
| Graduation | through the module, **10.909 ETH-backed units** |
| Module mint | 0.3249 wX, **equal to its quote** |
| Sweep | 0.8 ETH bought **1.2985 wX** |
| Draw | draw 0 opened and pinned to drand evmnet round 20,902,772, read back from the chain; the simulation pinned the same round |
| Wait | about 13 minutes for the round to exist, as expected with a 2,000-block fork-back |
| Resolve | with the round's real signature: winning ticket 997 |
| Claim | one whole wX moved to the winner |

**`refusals` mode, same day: DRY RUN: ALL PASS** (12 checks, all local):

- 4663 without a hardware wallet: `deploy.sh`, `deploy-wrapclaim.sh`, `deploy-nft.sh` and `deploy-stack.sh` all refused;
- `list-auto.sh` on 4663 sent nothing: the nonce and the listing count are unchanged;
- `deploy-stack.sh` on 4663 at a stage above 0: refused;
- the rehearsal on a node reporting 4663: refused;
- three refusals on 46630.

The hardware-wallet refusals are exactly why K8 or O5 is needed (§3). K8 is built since 2026-09-25; `refusals` mode now also holds the keystore's (§8.5).

## 7. Where the docs and the code disagreed (fixed under K10, 2026-09-25)

Each item as found on 2026-09-24, then what was done. No contract was changed.

1. **G2 solo "k = 1"**: the timelock cannot be installed that way (§2). *Done:* `launch-gates.md`'s solo table now holds p1, p2
   on A and g1–g3 on B with k = 2, with a dated correction note.
2. **The hardware wallet:** `launch-gates.md` accepts software keys; every 4663 script demanded `--ledger` or `--trezor`;
   `contracts-mainnet.md` and `key-ceremony.md` said a hardware deployer. *Done in code (K8)* and in both docs: a hardware wallet
   or a Foundry keystore; `key-ceremony.md` gains "The deployer as a keystore"; the solo table names the EVM deployer.
3. **`audit/README.md` known issues** listed H11 and H12 as open. Both are built (`605aab0`), but the code differs from the spec
   (H11: a default of 20 a day, set once by `initMintLimit`, raised after 7 days, not immutable, and `moduleMint` does not count;
   H12: with no ceiling, a stale re-post is unbounded). *Done:* both rows say "built, not as specified", with the details.
   *Flag for code:* H12's stale re-post could be bounded in `SweepVault.postCap` (e.g. a step from the last cap even when stale);
   until then go-live step 7's `proposeCapCeiling` is the bound, 7 days after it is proposed.
4. **"Loosening waits 7 days"** was false for `ProtocolTreasury`'s setters and for `Ownable2Step` ownership. *Done:*
   `launch-gates.md` (both tables and a note), `key-ceremony.md` ("What each can do"), `custody-threat-model.md` T15 and
   `contracts-mainnet.md`'s step table now say which changes are instant.
5. **`deploy-stack.md` §1** said an owner rotation goes through `proposeRoles` (7 days). *Done:* corrected — ownership moves with
   no delay; `proposeRoles` is the keeper, purchaser and attestor.
6. **G2 solo rules** said they were "checked by `golive-check`", which never read the canister. *Done:* K9 makes
   `golive-check.sh --canister` read `custody_caps()` against the stage (rule 1); `launch-gates.md` now names which script checks
   what (roles: `timelock-watch.mjs --expect-*`) and that the rest is procedure.
7. **`canister-mainnet.md`** recommended `robinhood.drpc.org`, dead for pinned reads. *Done:* the install argument and the
   recommendation name Robinhood + ordofi, with a dated supersession note there, in `pinned-reads.md` and in
   `custody-threat-model.md`'s provider list; the measurements are kept as taken.
8. **`deploy-stack.md` §6 step 3** said "the canister reads `queues()`". *Done:* corrected — nothing in the canister reads the
   directory; it is configured by `set_floor_sink` and `set_item_config`.
9. **The module's ETH-backed caps** (10 public, 40 hard) are not scaled by stage (§0). *Done:* stated under G7 in
   `launch-gates.md`. *Flag for code:* scaling them is a change to `DeployStack.s.sol`/`StackChecks.sol` (the module's `Params` are
   per-deployment immutables), not to a contract; not made, since the operator has accepted §0 as written.

## 8. K5, K8, K9, K10: the plan (2026-09-25, branch `p1-deploy-site`, written before the code)

Nothing here is deployed, pushed, signed or sent. The only chains touched are local anvil nodes; the stack rehearsal forks 4663
read-only from `rpc.ordofi.network`.

### 8.1 K8: a password-keystore signer on 4663

The operator has no hardware wallet (§2, G2 rule 5), so O5 is not bought and K8 is built.

- **What is added.** `--account <name>` beside `--ledger` / `--trezor` in `script/deploy.sh`, `script/deploy-stack.sh` and
  `script/deploy-wrapclaim.sh`. The check lives once, in `script/chain-guard.sh` (`send_4663`), which `deploy.sh` now sources too.
  Modelled on GitLaunch's `script/deploy.sh`: a keystore name of letters, digits, dot, dash and underscore; the keystore's
  address must be the deployer's; the chain id is still typed by hand; every other refusal stays.
- **Where forge reads the keystore, measured.** `forge script --account` reads `~/.foundry/keystores/<name>` and nothing else:
  its help says "the default keystores folder (~/.foundry/keystores)", and with `FOUNDRY_KEYSTORES_DIR` pointed elsewhere and
  `HOME` at an empty directory, `cast wallet address --account probe` looked for `$HOME/.foundry/keystores/probe`. GitLaunch's
  script checks `${FOUNDRY_KEYSTORES_DIR:-~/.foundry/keystores}/<name>`, so with that variable set it checks one file and forge
  signs with another. That is **not** mirrored: the scripts check `$HOME/.foundry/keystores/<name>`, and refuse when
  `FOUNDRY_KEYSTORES_DIR` names any other directory, so the file checked is the file that signs.
- **The address.** If the keystore file records an `address`, it must equal the deployer. If it does not (Foundry's own
  keystores may not), `cast wallet address --account <name>` opens it once, the operator types the password, and the address it
  prints must equal the deployer. No address either way: refused. The script itself never reads a key or a password; cast and
  forge prompt for the password on the terminal.
- **The environment.** `ETH_KEYSTORE`, `ETH_KEYSTORE_ACCOUNT` and `ETH_PASSWORD` are refused alongside the private-key variables:
  forge reads each by itself, which would pick a signer or a password nobody typed.
- **Not changed.** Local chains (31337, the staging fork) still refuse every signer flag, `--account` included.
  `script/deploy-nft.sh` stays hardware-wallet-only: the NFT side is OFF in Phase 1 (§1) and nothing should deploy it.
- **The spec (tests).** `script/deploy.test.sh` gains the local refusals of `--account` and unit tests of `send_4663` against a
  throwaway `HOME` with address-only keystore files (no key material anywhere). `scripts/golive-dryrun.sh refusals` keeps "no
  signer → refused" for all four scripts and adds, on a node reporting 4663: a keystore for another address, a missing keystore,
  a bad name, `FOUNDRY_KEYSTORES_DIR` elsewhere, a keystore variable in the environment, two signer flags — all refused — and a
  keystore with the right address **accepted up to the typed chain id** (a wrong id is typed, so it stops at "not confirmed"
  and the deployer's nonce is unchanged).

### 8.2 K9: `golive-check.sh` in stack mode

- For every inscriptions collection in a stack file, `listing(listingId).btcVault` from the registry must equal
  `wrapper.custodyAddress()`, as the wrapper-file mode already checks.
- `--canister <buyer id>` (optional; `--network ic|local`, default `ic`) reads the buyer's public `custody_caps()` query with
  `dfx` as the anonymous identity (as `scripts/timelock-watch.mjs` does) and prints it next to the stack's stage. For each
  collection, the canister's cap under `btc:0x<wrapper>` must equal the stage's items and wei per collection. A wrapper the
  canister does not name yet is a note (step 8 of §3.4 runs before the reinstall in step 9 that names it), not a failure.
  Without `--canister` the check prints a note that the canister was not read. Asked for and unreadable: a failure.
- The spec: `scripts/golive-dryrun.sh stack` runs the check with a stand-in `dfx` (the `DFX` variable) that answers stage 0,
  then one that answers a raised cap, which must fail.

### 8.3 K5: the site's Phase-1 mode (`web-rook/`)

- **The switch.** `VITE_ROOK_PHASE=1` at build (a Dockerfile `ARG` too). Any other non-empty value fails the build.
- **Hidden in Phase 1.** Bots (nav, footer, the reserves page's link; `#/bots` shows why it is off), Rent (`#/rent` likewise),
  the YOYO policy (not offered by the launch wizard; a YOYO coin's page says its policy is off and its fees wait in the vault),
  the desk (the site has no MintDesk UI; a test keeps it that way), and the Pons path (pons coins are left off the board, and a
  pons coin's page does not send anyone to trade it).
- **The deployment file.** `scripts/sync-content.mjs` also copies `deployments/stack-4663.json`. In Phase 1 `TxButton` refuses
  without a valid bundled 4663 stack file. ~~Before sending it checks only that the target is one of the stack's contracts, or a
  coin or curve its factory launched.~~ **Since wave 2 (audit E-07, `docs/audit/2026-09-26/fixes-W2-site.md` §2)** it checks what
  the transaction *says*, not only where it goes: `requestRefusal` (`web-rook/src/rook/phase.ts`, called from
  `web-rook/src/ui/wallet.tsx`) classifies the target (a stack contract, or a coin or curve the factory launched, read on chain
  with `CurveFactory.routerOf`), allows only the functions the site makes to that kind of contract (`CALLS`: `redeem`/`refund`/
  `resolve` on the wrapper, `launch` on the factory, `buy`/`sell` on the router, `approve` on a coin, the curve's own calls, and
  the raffle vault's keyless `requestDraw`/`resolve`/`claim`/`voidDraw`), matches the call to its ABI, and holds **every address
  argument**, in lists and tuples too, to its rule: an approval's spender is the stack's router, a trade pays the connected
  account, a coin is one the factory launched. The addresses a page sends to come from the stack file (`pinnedAddress`); when
  the indexer names another for the same role, the page shows both and sends nothing. The launch wizard launches through the
  stack file's factory and refuses if the indexer names another.
- **The Trust page.** In Phase 1 it opens with §0, stated plainly: the stage-0 caps bound custody only; the number of open curves
  and the ETH in them is not capped (the operator's decision of 2026-09-24); the contracts are unaudited (G1 waived for stage 0);
  what someone holding both operator devices could do (the Safe: `ProtocolTreasury.setShares` / `setOps` at once, ownership
  transfer with no delay; through the timelock, an upgrade after 72 h); redemption cannot be paused. Then the trust model as before.
- The spec: `web-rook/test/phase1.test.ts` (the switch, the nav, the policies, the board, the stack file's validation, the
  target check with a stand-in reader, the Trust statement's sentences, no desk UI) and `web-rook/test/audit_e07.test.ts` (the
  per-argument request check: a hostile spender, recipient or coin is refused). Screenshots of the Phase-1 build in
  `docs/v6/screens/`.

### 8.4 K10: the nine disagreements of §7

Docs are brought to the code; no contract changes. Where the code, not the doc, should move, it is flagged in §8.5 after the work.

### 8.5 As built and as run (2026-09-25)

All local. No deploy, push, transaction, key or Railway change. The keystore files in the tests hold an address and no key.

| Run | Result |
| --- | --- |
| `forge test` | 96 suites, **940 passed, 0 failed** |
| `script/deploy.test.sh` | **37 ok, 0 fail**: the 16 old refusals, 6 new local ones (`--account` on 31337, no name, two signer flags, `ETH_KEYSTORE` / `ETH_KEYSTORE_ACCOUNT` / `ETH_PASSWORD`), and 15 unit tests of `send_4663` (no signer refused; Ledger and Trezor accepted; the deployer's keystore accepted, in capitals with 0x too; another address, a missing file, `../` and `-` names, a file only in `FOUNDRY_KEYSTORES_DIR`, `FOUNDRY_KEYSTORES_DIR` elsewhere refused; set to forge's own directory accepted; no recorded address: cast names the deployer → accepted, another → refused, cannot open → refused) |
| `scripts/golive-dryrun.sh refusals` | **DRY RUN: ALL PASS**, 28 checks: the 12 before, plus on a node reporting 4663 a keystore for another address (`deploy.sh`, `deploy-wrapclaim.sh`, `deploy-stack.sh`), a missing keystore, a climbing name, `FOUNDRY_KEYSTORES_DIR` elsewhere, `ETH_KEYSTORE_ACCOUNT`, `ETH_PASSWORD`, two signer flags — all refused; the deployer's keystore accepted by all three scripts up to the typed chain id (a wrong id typed: "not confirmed"), the prompt naming the keystore; `deploy-nft.sh` refusing `--account`; the deployer's nonce unchanged and no 4663 file written |
| `DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` (fork of 4663 from ordofi, chain 9466301, port 8651) | **DRY RUN: ALL PASS** to the drand pin (resolve and claim not run, as the flag says). `golive-check.sh` **117 checks ok** (116 + the new `listing 2 btcVault is the wrapper's custody address`) and the note that the canister was not read; with a stand-in dfx: stage-0 caps read `ok` for items and wei, a stage-1 cap refused, a canister not naming the wrapper a note, an unreadable canister and a bad id refused |
| `web-rook`: `npm test` | **43 pass** (10 new in `test/phase1.test.ts`); `tsc` clean; `vite build` clean with and without `VITE_ROOK_PHASE=1`; `VITE_ROOK_PHASE=2` stops the build |
| Phase-1 screenshots (playwright, 400 and 1440, dark and light, 8 pages) | no horizontal scroll, no console errors; `docs/v6/screens/phase1-{trust,bots-off,launch}.{phone,desktop}.{dark,light}.jpg` |

Found on the way:

- **GitLaunch's keystore check reads `FOUNDRY_KEYSTORES_DIR`, forge does not.** Its `script/deploy.sh` checks
  `${FOUNDRY_KEYSTORES_DIR:-~/.foundry/keystores}/<name>`; forge's `--account` reads `~/.foundry/keystores/<name>` only. With the
  variable set, GitLaunch checks one file and signs with another (forge would still refuse a signer that is not `--sender`, so the
  practical risk is a confusing failure, not a wrong signer). Not changed here: another repository.
- **The Trust page needs the stack file to name addresses.** A Phase-1 build without it says so on the page and cannot send.
- **`security.txt` waits for O9.** With `SECURITY_CONTACT` empty the Phase-1 build warns and writes no file.

## 9. K3, K4, K7: as built and as run (2026-09-25, branch `p1-keeper`)

The design came before the code: `docs/v6/p1-services.md`, dated. Its §7 holds the results in full. The operator's page is
`docs/deploy/services-v1.md`. **Nothing is deployed**, no transaction went to a public chain, and no real key exists.

**Three services**, so that no two secrets share a process and the alarm does not share a host with what it watches:

- **keeper** (K3, `keeper/service/`, `keeper/Dockerfile`) holds the keeper's EVM key.
- **runner** (K4, `ops/src/runner.ts`, `ops/Dockerfile.runner`) holds the operator's IC identity.
- **reconciler** (K7, `ops/src/reconcile.ts`, `ops/Dockerfile.reconciler`) holds only the Telegram token. It goes in the
  monitor's Railway project.

**Refusals**, each tested:

- **Keeper.**
  - No `KEEPER_CHAIN_ID`.
  - An RPC or a stack file for another chain.
  - A key that is not `registry.keeper()` or a module's keeper, or that has code.
  - A RAFFLE or BURN vault of the wrong policy or registry. The YOYO vault is never posted to.
  - A fixture floor sent to 4663.
  - A floor the two-provider feed did not agree on.
  - Gas over the per-transaction or per-day caps.
  - A second transaction in flight.
  - A dry run unless `KEEPER_MODE=send`.
- **Runner.** No identity, and a stranger's identity, which the canister itself refuses.
- **Offer tool.** Every refusal of `frogbuy::judge`, plus the membership, location, contents, explorer, limit and cap checks.
  It runs dry by default.
- **Reconciler.** Caps above the stack's stage are red, and a report older than 3 h is red.

**Runs.**

- `scripts/keeper-fork-test.sh`: **ALL PASS**, on a fork of 4663 under the staging id:
  - the floor and both caps were mined;
  - restarts were idempotent;
  - the launch was filled by the module at the keeper's floor;
  - a draw was opened with a published snapshot, and `tools/snapshot` recomputed it byte for byte;
  - six refusals left the nonce unchanged;
  - a dry run on a node reporting 4663 signed nothing, and the keyless real feed posted no floor;
  - the reconciler was GREEN against the fork plus a local replica's `custody_caps()` at stage 0.
- `ops/scripts/runner-replica-test.sh`: **ALL PASS**.

**Tests.** keeper 96 (29 new), ops 20. Indexer 99, monitor 48 and `tools/snapshot` 12 are unchanged and still pass.

**Left for the operator**, in go-live checklist step 11:

- O2, the UniSat key. Without it no frog floor is posted.
- A public domain for the keeper (the draw URIs) and one for the reconciler (G5).
- The secrets, set as service variables.
- The start order in `services-v1.md` §5.

**Left for K5:** the site's reserves page should read the reconciler's `caps`, `stage` and staleness.
