# Rook security review, area C: treasury, sweep policies, roles and access control, deploy and go-live workflow

Reviewer: C (one of five). Date: 2026-09-25. Checkout: `/Users/jlohayon/Documents/Rook-audit-C`, detached at `e04f49b2bb94597e0107a607c5bc7ac3d3e3bbce` (origin/main, "Merge branch 'p1-keeper'").
Nothing under `src/`, `script/`, `docs/` or the canister code was changed. The only additions are PoC tests under `test/audit/C/`. Forge rewrote `foundry.lock` on its own; I reverted that. I made no commit, push, deploy or transaction. The only contact with 4663 was read-only `eth_getBlockByNumber` calls on the public RPC.

## 1. Methodology

Sources I fetched and followed:

- **Trail of Bits, Building Secure Contracts**:
  - development guidelines: https://github.com/crytic/building-secure-contracts/blob/master/development-guidelines/guidelines.md (design, implementation, deployment and admin-key items);
  - secure-development workflow: `development-guidelines/workflow.md` (Slither first, then special features, code structure, security properties, then manual review for front-running, ordering and privileged-key risk);
  - code-maturity rubric: `development-guidelines/code_maturity.md` (access controls, decentralization, transaction ordering, auditing/monitoring).
- **OWASP Smart Contract Top 10 (2025)**: https://owasp.org/www-project-smart-contract-top-10/. Every item was used as a pass: SC01 access control, SC02 oracle (the keeper's cap and floor), SC03 logic, SC04 input validation, SC05 reentrancy, SC06 unchecked calls, SC07 flash loans / just-in-time, SC08 arithmetic, SC09 randomness, SC10 DoS.
- **SWC registry**: https://swcregistry.io/. Used: SWC-100/108 visibility, SWC-105 unprotected ETH withdrawal, SWC-114 transaction-order dependence, SWC-115 tx.origin, SWC-116 block values as time, SWC-120 weak randomness, SWC-121 signature replay, SWC-128/113 DoS, SWC-132 unexpected ETH balance.
- **Consensys smart-contract best practices**: https://consensys.github.io/smart-contract-best-practices/. The `/attacks/` page answered 404 when fetched, so I used its standard categories from the site index: front-running, timestamp dependence, DoS, force-feeding ETH, griefing and insecure arithmetic.
- **Cyfrin / Solodit audit checklist**: https://github.com/Cyfrin/audit-checklist. The README was fetched; its categories (access control and centralization, DoS, randomness, front-running, signature, ERC-20, ETH handling) were applied.

Passes, in order:

1. **Design docs.** In this order: `docs/v6/phase1-scope.md`, `docs/v5/launch-gates.md` (by reference), `docs/v6/deploy-stack.md`, `docs/deploy/contracts-mainnet.md`, `docs/deploy/key-ceremony.md`, `docs/trust-model.md`, `docs/snapshot-v1.md`, `docs/v6/p1-services.md` §1, and the site's `web-rook/src/rook/phase1Trust.ts` and `web-rook/src/content/trust-model.md`.
2. **Invariants and trust assumptions.** Written down in §3.
3. **Line-by-line review** of:
   - `src/floor/ProtocolTreasury.sol` and `src/SweepVault.sol`;
   - every privileged function in `src/` that is live in Phase 1 (grep for owner, keeper, custodian and deployer checks, then each one read in full);
   - `script/deploy.sh`, `Deploy.s.sol`, `DeployChecks.sol`, `chain-guard.sh`, `list-auto.sh`, `ListAuto.s.sol`, `deploy-stack.sh`, `DeployStack.s.sol`, `StackChecks.sol`, `golive-check.sh` and the relevant parts of `scripts/golive-dryrun.sh`;
   - the keeper service's call surface (`keeper/service/abis.ts`).
4. **Breaking invariants with PoCs**: 9 Foundry tests, all passing (§6).
5. **Test gaps** (§8).

Chain facts checked read-only on 2026-09-25. The latest block's timestamp was 0.6–1.5 s behind wall-clock. `block.number` inside the EVM is the parent chain's (26,058,684 vs L2 72,732,6xx), as `src/lib/Wait.sol` says.

## 2. Scope

**In scope:**

- `ProtocolTreasury`: shares 90/0/0/10 in Phase 1, `setShares`/`setOps`, bootstrap OFF.
- `SweepVault`: RAFFLE and BURN, `postCap`, the H12 ceiling, `openDraw`, `requestDraw`/`redraw`/`resolve`/`claim`, the desk route.
- Every contract's access control and ownership: `ListingRegistry`, `WrapClaim`/`Wrapper` wiring, `CollectionDirectory`, the `CurveFactory`/`FloorModule`/`LiquidityLock` deployer wires, `AutoBuyerVault`, `MintDesk`, `Launcher`/`FeeRouter`, `Treasury.sol` (not deployed in Phase 1).
- The deploy scripts and go-live steps 6–8 of `phase1-scope.md` §3.4.
- The Trust page claims.

Phase-1 ON paths relevant here: curve launches; sweeps (RAFFLE, BURN); draws; the treasury split; redemption. OFF paths checked for on-chain reachability: YOYO, MintDesk, bootstrap loans and floats, Pons launches, OfferBoard.

## 3. Invariants and trust assumptions (as I understood them)

**Invariants (I1–I9):**

| # | Invariant (intended) | Holds? |
| --- | --- | --- |
| I1 | The deployer key owns nothing after the deploy; nothing is initialisable by a third party at any step | **Holds.** The registry is Safe-owned from its constructor. `WrapClaim` is the wrapper's deployer. The deployer's one-shot wires (`CurveFactory.wire/wireYoyo`, `FloorModule.wire`, `LiquidityLock.wire/wireTreasury`) are made in the same broadcast and then `AlreadyWired` |
| I2 | `WrapClaim.initMintLimit` / `wireFloorModule` can't be front-run or made impossible before step 7 | **Holds.** They are owner-only, and `NotFresh` (supply, held or tickets ≠ 0) can only be triggered by the custodian or the module. Neither can mint before step 9 (canister) and step 7 (module wiring) |
| I3 | A sweep never pays above the keeper's cap, and a cap cannot jump (H12) | Holds only while fresh (±20% per 10 min, which compounds to 3×/h). A stale re-post is bounded only by an **accepted** ceiling, and no process accepts it (**C-01**) |
| I4 | A raffle's winner is fixed by a drand round nobody has seen when it is pinned | Holds for the first pin. `redraw` after 24 h lets a party that has *seen* the pinned round's outcome discard it (**C-02**) |
| I5 | The ticket list is the published snapshot | Only if someone recomputes it and the owner cancels within 15 min. No watcher exists, and `totalTickets` is not committed in the root (**C-03**) |
| I6 | Phase 1 runs the treasury at 90/0/0/10, with floats and bootstrap OFF | This is a manual Safe step, never scripted or checked, and it is not retroactive to pots already allocated (**C-07**) |
| I7 | The owner has no withdraw path ("There is no function that takes a recipient, no withdraw") | False for `ProtocolTreasury`: the Safe drains every pot at once (**C-06**) |
| I8 | Stage-0 caps bound custody only (the doc's own statement) | Agreed. Also, the keeper hot key's reach (sweep credit, prizes, false deliveries, the module floor) is not on the Trust page (**C-09**) |
| I9 | OFF paths are unreachable on chain | Not all of them (see §5 and C-07, C-10, C-14): the MintDesk route is reachable by anyone; floats/bootstrap are reachable through ordering; Pons launches are permissionless. YOYO is inert without a keeper cap |

**Trust assumptions:**

- The Safe is honest (2-of-2, both keys with one person).
- The keeper (an EVM hot key on Railway) is honest, or the owner reacts within 15 minutes / 7 days.
- The canister is correct.
- drand is honest.
- The 4663 sequencer's timestamps stay within about 60 s of real time (for the drand pin).

## 4. Privilege table

The same keeper EOA is `registry.keeper()`, which the SweepVaults, Wrapper and WrapClaim read live. It is also the **immutable** `FloorModule.keeper` (`FloorModule.sol:112`).

| Role (who holds it in Phase 1) | What it can do (Phase-1 contracts) | Worst case if compromised |
| --- | --- | --- |
| **Safe 2-of-2** (registry owner; `CollectionDirectory.admin` immutably; all `registry.owner()` checks) | **At once:**<br>• `ProtocolTreasury.setShares/setOps/setBootstrapLoan/addCollection/forgive`<br>• `ListingRegistry.setPaused` (any listing), `listInscriptions*`/`listRune*` (new vaults, a new buyer), `transferOwnership` (2-step, no delay), `setBondBps` (raise), `wire`/`wireNft` once<br>• `SweepVault.cancelDraw` (15 min), `proposeCapCeiling` (lowering is instant only once a ceiling is active), `setMarkup`<br>• `WrapClaim.wireFloorModule`/`initMintLimit` (once, before first mint), `proposeMintLimit` (lower instantly), `cancel` (15 min)<br>• `AutoBuyerVault.returnFunds`/`writeOff` (owner paths)<br>• `CollectionDirectory.add`<br>**After 7 days:** `proposeRoles`/`acceptRoles` (a new keeper for the SweepVaults and the Wrapper), root, btcVault, treasury, bond decrease | **The whole ProtocolTreasury balance, instantly, including allocated pots** (C-06, PoC: 14 of 14 ETH).<br>Pause every sweep and draw (not redemption).<br>Cancel every draw.<br>With a 7-day wait: become the keeper and then do everything in the keeper row except the FloorModule, whose keeper is immutable.<br>Before step 7 only: wire a fake floor module to the wrapper, which then mints unbacked wFROG. After step 7 it cannot.<br>It cannot touch curves, locked LP, the FloorModule's ETH, sweep-vault ETH directly, or any vault's buyer |
| **Keeper** (hot EOA on Railway, `keeper/service`) | `FloorModule.postFloor` (±5%/h clamp; low confidence makes the module stale), `postRentalRoot`<br>`SweepVault.postCap` (fresh: ±20%/10 min; stale: anything unless a ceiling is **accepted**), `openDraw` (any root and `totalTickets`), `buyback` (YOYO)<br>`Wrapper.recordDelivery` (any resolved ticket, any txid)<br>`WrapClaim.open` (OFF), `Distributor` (v1, OFF), `AutoBuyerVault.postCap` (unbounded, but the buyer is the canister, which checks)<br>`Treasury.fund` (not deployed) | Drain all RAFFLE/BURN credit through an attacker-sandwiched pool route. Making the module stale forces pool-only routing (C-01, PoC).<br>Steal every raffle prize unless the solo operator cancels each draw within 15 min (C-03, PoC).<br>Burn redeemers' wFROG with false deliveries: redemption is unpausable and rotation takes 7 days (C-05).<br>**Permanent** control of the FloorModule floor, because the keeper is immutable there: walk it ±5%/h, or keep the module stale, which blocks mint/burn and graduations (C-11). Floor-walk extraction from the module's ETH is for the FloorModule reviewer |
| **Deployer EVM** (keystore on device A) | Nothing after `deploy.sh`/`deploy-stack.sh` complete. If a stack deploy is interrupted, the remaining one-shot wires of that half-deployed set | Look-alike deployments before the log is published. Environment inputs such as `ETCH_PAYEE`, `ETCH_OPS` and `ETCH_STACK_ITEM_UNITS` that `golive-check` does not read back (C-08). A wrong payee is a DoS: the canister refuses a sink whose payee is not itself (`frogflow.rs:298`) |
| **Custodian** (canister EVM address; immutable in Wrapper, FloorModule, MintDesk, OfferBoard) | `wrap`/`wrapDeposit` (mint), `recordDelivery`, `itemArrived`/`itemSold`, `fulfill`, `recordSale`/`reclaimListing`, `reclaimStray` | Custody-level (out of area C). Bounded by the canister's caps and 72 h timelock; mints bounded by H11 (3/day) |
| **Registry `treasury` EOA** (`ETCH_TREASURY`) | Receives 20% of every **Pons** router's fees and any `forwardToken` tokens; `AutoBuyerVault.depositFrom` | Loses what it has received. Pons launches are permissionless on chain in Phase 1 (C-14) |
| **Ops EOA** | Receives `opsBps` of ProtocolTreasury allocations | Loses its share only |
| **Purchaser / attestor** | v1 `AssetVault` only. No v1 listing exists on 4663 in Phase 1 | None in Phase 1. It becomes live if the Safe lists a v1 listing later |
| **Payee** (default: the custodian) | Receives `itemArrived` cost and MintDesk `fulfill` cost | Only if misconfigured (C-08) |
| **Coin creator** | `claimCreatorFees`, fee tier at launch, policy choice | None beyond its own fees |
| **Anyone** | launch (curves, and Pons on the auto listing), `sweep` (caller's route, bounded by cap), `sweepViaDesk`/`collectDesk`, `requestDraw`/`redraw`/`resolve`/`claim`/`voidDraw`, `retire`/`reclaim`, `acceptCapCeiling`, `acceptMintLimit`, `allocate`/`distribute`/`bootstrap`, `claimPlatformFees`, `flushProtocolFees` | Raffle grinding (C-02); snapshot JIT (C-04); desk griefing (C-10); running the OFF bootstrap path at defaults (C-07) |

## 5. Findings

| ID | Severity (L×I) | Title | File:line | Phase-1 reachable? | PoC |
| --- | --- | --- | --- | --- | --- |
| C-01 | **Medium** (Low-Med × High) | The H12 cap ceiling proposed at deploy is never activated: `acceptCapCeiling` is unowned, and a stale re-post is unbounded. A compromised keeper drains RAFFLE/BURN credit | `src/SweepVault.sol:247-289`; `script/DeployStack.s.sol:264-268`; `script/golive-check.sh:269` | Yes | PASS |
| C-02 | **Medium** (Med × Med) | Raffle grinding: anyone who sees a losing pinned round can decline to resolve and `redraw` after 24 h. No Phase-1 component calls `requestDraw`/`resolve`/`claim` | `src/SweepVault.sol:407-444` | Yes | PASS (a 20% holder won after 10 re-rolls) |
| C-03 | **Medium** (Low-Med × Med) | The draw's only guard (a 15-min owner cancel) has no watcher. `totalTickets` is not committed in the root, and `snapshotAt/Block` are unbound claims | `src/SweepVault.sol:370-405` | Yes | PASS (correct root, truncated total ⇒ only the lowest range wins) |
| C-04 | **Medium** (Med × Med) | Snapshot at a published, predictable block (daily 15:00 UTC, first block ≥ T). Just-in-time holding captures raffle EV | `docs/v6/p1-services.md` §1 (Draws); `docs/snapshot-v1.md`; `SweepVault.openDraw` | Yes | Arithmetic only |
| C-05 | **Medium** (Low-Med × Med) | The keeper can mark any resolved redemption delivered with a made-up txid (known T4, still open). Redemption is unpausable; keeper rotation takes 7 days | `src/Wrapper.sol:524-545` | Yes, once any frog is in custody | Not written (direct from code; documented as T4) |
| C-06 | Low (Low × High, centralization) | The Safe drains the **entire** ProtocolTreasury at once (allocated pots included) through `addCollection(fake)` + `distribute`/`bootstrap` + `setShares`/`setOps`. The NatSpec and Trust page understate this | `src/floor/ProtocolTreasury.sol:14-21,98-124,163-216` | Yes | PASS (14/14 ETH) |
| C-07 | Low | The Phase-1 treasury config (`setShares(9000,0,0,1000)`, `setBootstrapLoan(0)`) is unscripted and unchecked, and not retroactive. Income before it keeps floats ON and strands the bootstrap pot; at defaults anyone opens a bootstrap loan | `src/floor/ProtocolTreasury.sol:34-41,143-216`; `script/DeployStack.s.sol:151-171,277-287` | Yes (sweep-only launches trade right after `deploy-stack.sh`) | PASS |
| C-08 | Low | `golive-check.sh` passes with Safe step 7 entirely undone, and does not verify several Phase-1 configs | `script/golive-check.sh:177-287` | n/a (process) | n/a |
| C-09 | Low | Trust page claims vs contracts: Safe powers understated, "absence of a withdraw path" is false for ProtocolTreasury, and keeper-compromise reach is absent | `web-rook/src/rook/phase1Trust.ts:43-56`; `web-rook/src/content/trust-model.md:62-63` | n/a | n/a |
| C-10 | Low | `sweepViaDesk` griefing: anyone parks a coin's whole credit at the unserved desk for 6 h, repeatedly | `src/SweepVault.sol:328-364`; `src/curve/MintDesk.sol:65-125` | Yes (desk deployed, OFF, unserved) | PASS |
| C-11 | Low | One hot key is both `registry.keeper` and the **immutable** `FloorModule.keeper`. A leak is permanent for the module and bundles four powers | `src/floor/FloorModule.sol:112`; `script/DeployStack.s.sol:102,204` | Yes | n/a (documented design) |
| C-12 | Info | `distribute` NatSpec is wrong: one reverting `fundReserve` blocks every collection | `src/floor/ProtocolTreasury.sol:18-19,182` | Needs an owner-added bad module | PASS |
| C-13 | Info | The drand pin assumes the sequencer timestamp is within 60 s (`PIN_AHEAD` 20 × 3 s) of real time. Nitro permits up to 24 h behind | `src/SweepVault.sol:82,428` | Sequencer only | Measured lag ~1 s |
| C-14 | Info | The Pons launch (OFF) can only be stopped by pausing the listing, which also stops the stack's sweeps and draws (same `listingId`) | `src/Launcher.sol:58`; `script/DeployStack.s.sol:214-216` | Yes | n/a |
| C-15 | Info | `CollectionDirectory.admin` is immutable and does not follow a registry ownership transfer | `src/curve/CollectionDirectory.sol:21,31-34` | n/a | n/a |
| C-16 | Info | `retire` (BURN) is not pause-gated, so pausing a listing does not stop BURN redemptions | `src/SweepVault.sol:486-495` | Yes | n/a |

**Counts:** Critical 0, High 0, Medium 5, Low 6, Informational 5.

---

### C-01 (Medium): the H12 cap ceiling is never activated; a stale re-post is unbounded

**Description.** `postCap` bounds a new cap only while the last one is fresh: ±20% per step, with at least 10 minutes between posts on both clocks (`SweepVault.sol:249-257`). Once the cap is an hour old, the next post is bounded only by `capCeiling`, and only if it is non-zero (`:258`).

- The go-live step "proposeCapCeiling ×3" (DeployStack `_ownerSteps`/`_printOwnerSteps`, `:264-268`, `:280-286`) only sets `pendingCeiling`.
- `capCeiling` becomes non-zero only when someone calls `acceptCapCeiling()` after 7 days (`:282-289`).
- No script, service, checklist row, runbook or monitor mentions `acceptCapCeiling`: `grep -rn acceptCapCeiling docs script scripts keeper ops monitor web-rook` returns nothing.
- `golive-check.sh:269` prints a "note" and never fails.
- `phase1-scope.md` §7 item 3 treats the proposed ceiling as *the* bound "7 days after it is proposed", but it is not the bound until someone accepts it.
- While `capCeiling == 0`, the owner's instant-lower branch (`:270`) is unavailable, so the only emergency lever is pausing the listing.
- Even while fresh, the step compounds to 1.2⁶ ≈ 2.99× in one hour and the cap stays fresh (PoC C01b).

**Exploit.** A compromised keeper key (a hot key on Railway):

1. Makes the FloorModule stale, by posting a floor with confidence below `minConfidenceBps` or by not posting. The router then fills pool-only.
2. Lets the RAFFLE/BURN cap lapse for an hour, or ramps it 3× in an hour while fresh.
3. Posts a cap at 1000× the floor.
4. In one bundle: pushes the wX/ETH pool up, calls the permissionless `sweep(coin, credit, swapData, 1)`, and sells back.

`minOut` is the caller's, so the only bound is the cap. Each coin's whole `credit` moves to the attacker, less pool fees. Repeat per coin. Even with the suggested 1.5× ceiling accepted (`deploy-stack.md` §6 step 4), about one third of every sweep is extractable under the same conditions.

**PoC.** `test/audit/C/C_SweepVault.t.sol`:

- `test_C01_…`: a ceiling was proposed, 8 days passed, and `capCeiling == 0`. The keeper posted 1000× and a sweep spent the full 1 ETH credit for 1/1000 of its value. **PASS.**
- `test_C01b_…`: the cap reached 2.99× in 6 fresh posts over 1 h. **PASS.**

**Fix.**

- (a) Make the first ceiling part of construction: pass `ETCH_CAP_CEILING` to the constructor, active at once. Or have `postCap` refuse when `capCeiling == 0` on 4663.
- (b) Bound a stale re-post by a step from the last cap (as §7 item 3 already suggests).
- (c) Until then: add `acceptCapCeiling` ×3 as a dated go-live row, make `golive-check` **fail** on `capCeiling == 0` after the delay, and have the keeper service or the monitor call or alert it.
- (d) Consider a per-hour cumulative bound instead of a per-post step.

---

### C-02 (Medium): raffle grinding through `redraw`

**Description.** `redraw` re-pins a new round if the pinned one is "unproven" (`beacon.randomnessOf(round) == 0`) after 24 h on both clocks (`:417-424`). A round is "proven" on chain only when someone calls `resolve` or `beacon.prove`. The drand signature is public off-chain about 60 s after the round's time, so anyone can compute the winner from it (`keccak(randomness, vault, id) % totalTickets`) long before it is on chain.

In Phase 1 **nothing** calls `requestDraw`, `resolve` or `claim` for SweepVault draws:

- the keeper's call list is `postFloor`, `postCap` and `openDraw` only (`keeper/service/abis.ts`; `p1-services.md` §1);
- the site has no SweepVault draw UI (`web-rook/src/lib/abisV3.ts` has only the Wrapper's `redeem`/`refund`/`redraw`);
- the monitor does not watch SweepVault.

**Exploit.** A holder with a small share:

1. calls `requestDraw`;
2. reads each pinned round's outcome off drand;
3. if the outcome is not theirs, does nothing, and after 24 h calls `redraw`;
4. when it is theirs, calls `resolve` and `claim`.

With no automated resolver, the honest winner has no tool that tells them to resolve. Expected re-rolls ≈ 1/share, one per day. Also, a resolved-but-unclaimed prize voids after 7 days (`voidDraw`), and the site offers no claim path.

**PoC.** `test_C02_aTicketHolderRerollsTheRaffleUntilTheyWin`: a holder of 20% of tickets won after 10 re-rolls. **PASS.** The existing `testFuzz_theRaffleCannotBeAimed` bounds the wait to ≤ 23 h, so this path is untested.

**Fix.**

- Have the keeper service (or a separate public bot) `requestDraw` and `resolve` every draw as soon as its round exists, and `claim` for the winner (claims pay the leaf's holder).
- In the contract, make `redraw` require the pinned round to be unprovable rather than merely unproven: for example, allow `redraw` only if `block.timestamp > timeOf(round) + X` **and** the beacon cannot verify. Or remove `redraw` and rely on anyone proving; the drand signature is always available.
- Or pin the replacement round deterministically, for example `round + N` rather than "now + 20", so a re-roll gains nothing.
- The same pattern exists in `Wrapper` redemption `redraw`, where the redeemer themselves can re-roll which frog they get. That belongs to the Wrapper reviewer; flagged there.

---

### C-03 (Medium): draw inputs are the keeper's word, and the 15-minute challenge is unwatched

**Description.** `openDraw` takes `root`, `totalTickets`, `uri`, `snapshotAt` and `snapshotBlock` from the keeper (`:370-394`).

- `snapshotAt` and `snapshotBlock` are only checked for being under an hour old. They are not bound to the root, so they bind nothing.
- `totalTickets` is **not committed in the root**. A correct root published with a smaller `totalTickets` makes only the lowest ranges winnable, because ranges are assigned by ascending holder address (`snapshot-v1.md`).
- A verifier that recomputes and compares the root (the fork test compared "the same root and the same bytes") passes.
- The only defence is `cancelDraw` by the Safe within 15 minutes (`:397-405`). The Safe is 2-of-2 with two devices held by one person. No component compares `DrawOpened` with the published file: the monitor watches only Wrapper and FloorModule custodian events, and the indexer only records.

**Exploit.** A compromised keeper opens a draw at the daily slot with a root in which it holds all tickets, or with the honest root and `totalTickets` equal to the end of an address range it controls. With no alert and no second signer on hand within 15 minutes, it takes the prize: one whole wFROG, which redeems for a real frog. It can repeat this every day for every coin.

**PoC.** `test_C03_correctRootWithTruncatedTotalTicketsHandsTheDrawToTheLowestRange`: 8 of 8 draws went to the lowest range. **PASS.**

**Fix.**

- Commit `totalTickets` (and the snapshot block) inside the root, for example as a leaf or by hashing `(root, totalTickets, block)`. At minimum, have the verifier compare `DrawOpened.totalTickets` with the file.
- Add a monitor rule: recompute every `DrawOpened` with `tools/snapshot` against a second RPC, and page on mismatch within 5 minutes.
- Consider a longer challenge window, which a solo operator can actually meet.
- The Trust page should state that raffles rely on the keeper plus the operator's 15-minute cancel.

---

### C-04 (Medium): just-in-time capture of raffle tickets at a predictable snapshot block

**Description.** The snapshot block is "the first block whose timestamp is at least T", with T = daily 15:00 UTC, published in advance (`p1-services.md` §1 Draws). Tickets are one per whole coin held at that single block (`weighted`). Blocks on 4663 are about 0.1–0.25 s apart.

**Exploit.**

1. Buy the coin seconds before T and sell right after.
2. Round-trip cost ≈ 2 × fee × position. Price impact is mostly recovered on an AMM. At the default 1% fee, 40% of that fee is paid back into the same raffle vault.
3. Expected gain = (position / eligible supply value) × prize. The prize is 1 wFROG, about 0.63 ETH at the frog floor.
4. So the trade is profitable whenever the coin's eligible market value is under about prize / (2 × fee) ≈ 31 ETH, which covers most coins from a 5–8 ETH raise.

Honest holders' expected value moves to the sniper. Across coins it is repeatable daily.

**PoC.** Economic; not written as a Foundry test.

**Fix.**

- Pick the snapshot block *after the fact* from randomness. For example: the first block ≥ `T − 24 h + (drand(T) mod 24 h)`. This is still named by a public rule, so the keeper does not choose it.
- Or use time-weighted balances: the average over k random blocks in the prior day, or a minimum holding time.

---

### C-05 (Medium): false `recordDelivery` by the keeper (T4, open)

**Description.** `Wrapper.recordDelivery` accepts the keeper **or** the custodian (`Wrapper.sol:525`). It marks any resolved ticket `Delivered` with any txid, decrements `held`, and clears `inscriptionHeld`. `refund` then reverts (stage ≠ Resolved), and the canister refuses to deliver a `Delivered` ticket (`custody-threat-model.md` T4).

The canister already records its own deliveries (`autobuyer/canister-rs/src/redeem.rs`), so the keeper power is not needed in Phase 1. Redemption cannot be paused. Replacing the keeper takes 7 days (`proposeRoles`). The monitor alerts (`monitor/src/evm.mjs:78`) but cannot stop it.

**Exploit.** For every redemption, the leaked keeper key front-runs the canister's delivery with a fake txid. Each redeemer loses one whole wFROG, and the frog stays in custody uncounted.

**PoC.** Not written: it follows directly from the code and is already listed as T4 "open".

**Fix.** Before the Wrapper is deployed (nothing is deployed yet), make `recordDelivery` custodian-only. The contracts are not yet on chain, so this costs a redeploy of nothing.

---

### C-06 (Low): the Safe drains the whole ProtocolTreasury at once

**Description.** `addCollection` accepts any contract whose `treasury()` returns this treasury (`:118-124`). `distribute` weights by a self-reported `volume7d()` (`:167-170`). `bootstrap` pays `min(bootstrapLoan, bootstrapPot)` to any registered module (`:200-216`). `setShares`, `setOps` and `setBootstrapLoan` are instant (`:98-115`).

A Safe holder can therefore, in one sitting:

1. add a fake module reporting `uint128.max` volume;
2. `setShares(0,0,0,10000)` and `setOps(self)`, then `allocate`: all unallocated ETH goes to them;
3. `distribute`: almost all of the reserves and floats pots go to the fake module;
4. `setBootstrapLoan(max)`, then `bootstrap(fake)`: the whole bootstrap pot.

The contract comment admits "the timelock registering a module that is not one of ours". But:

- the owner is a Safe, not a timelock;
- the NatSpec says "no withdraw";
- the Trust page says the Safe "redirects the treasury's income and its unallocated balance", which is less than the whole balance.

**PoC.** `test/audit/C/C_ProtocolTreasury.t.sol::test_C04_theSafeTakesEveryPotNotOnlyUnallocated` (named C04 in the file). The attacker took 13.999999999999999998 of 14 ETH, 9 ETH of it already allocated to pots. **PASS.**

**Fix.**

- Make `addCollection` require a module from a known factory, or a code-hash allowlist. Or delay `addCollection` and share changes (7 days, like the registry's loosenings).
- Correct the NatSpec and the Trust page: "the Safe can take everything in the treasury, at once".

---

### C-07 (Low): the Phase-1 treasury configuration is manual, unchecked, and not retroactive

**Description.**

- Defaults are 40/30/20/10 with a 2 ETH `bootstrapLoan` (`:34-41`).
- `phase1-scope.md` §1 turns floats and bootstrap OFF via `setShares(9000,0,0,1000)` and `setBootstrapLoan(0)`, the **last** bullet of go-live step 7.
- `DeployStack.s.sol` neither makes these calls locally nor prints them (`:151-171`, `_printOwnerSteps` `:277-287`). The dry-run rehearsal therefore always runs at defaults, and `golive-check` never reads shares, `bootstrapLoan` or pots.
- Income can arrive before step 7. `CurveFactory.wire` happens inside `deploy-stack` (`:217`), and a `sweepOnly` launch needs no keeper cap (`CurveFactory.raiseFor`). Its trades accrue `platformOwed`, and anyone can `claimPlatformFees` to the treasury and `allocate`.
- Pots are not re-split by `setShares`. So pre-step-7 income:
  - keeps `floatsPot` paying floats after floats are "OFF";
  - leaves `bootstrapPot` stranded while `bootstrapLoan == 0`.
- If the bullet is simply forgotten, anyone can open the 2 ETH loan (half of it a float).

**PoC.** `test_C05_preSetSharesIncomeKeepsTheOffPathsAlive` and `test_C05b_defaultsLetAnyoneOpenABootstrapLoan` (named C05 in the file). **PASS.**

**Fix.**

- Deploy `ProtocolTreasury` with Phase-1 shares and loan as constructor arguments.
- Or make `setShares` re-split undelivered float and bootstrap pots into reserves when their share goes to 0.
- Add both to DeployStack's printed owner steps, and make `golive-check` fail if shares ≠ the stage's.

---

### C-08 (Low): gaps in `golive-check.sh` (the go-live gate)

**Step 7 can be skipped and the check still passes.** For a mainnet stack (`ownerStepsMade=false`), each of these prints a "note" rather than a FAIL:

- an unwired wrapper (`:231-232`);
- a collection not yet in the treasury or the directory (`:281-286`);
- a missing cap ceiling (`:269`);
- unset ops (`:193`).

Step 14's "every row Yes" is then pure procedure.

**Configs the check never reads:**

- `ProtocolTreasury` shares, `bootstrapLoan` and pot balances (C-07);
- `capCeiling` value vs `ETCH_CAP_CEILING`, and whether it was accepted (C-01);
- `registry.pendingOwner()`: a lingering `transferOwnership` can be accepted later with no delay;
- `pendingRoles`, `pendingTreasury`, `pendingRoot`, `pendingBtcVault`;
- the Safe's `getThreshold()`/`getOwners()`: `DeployChecks` accepts any non-7702 contract as owner;
- role EOAs' code: keeper, purchaser, attestor and treasury are not checked for EIP-7702 delegations, although the docs require it;
- `FloorModule.payee()` and `MintDesk.payee()`: `ETCH_PAYEE` is an unverified environment input;
- `maxEthBacked`, `maxPublicEthBacked`, `mintPerHour` and `burnPerHour`: these scale with the unverified `ETCH_STACK_ITEM_UNITS`;
- listing paused state and `AutoBuyerVault.maxInFlight` against the intended minimum (printed only).

**Fix.** In `--phase1` mode, turn every "still to be made" note into a FAIL and add the reads above.

---

### C-09 (Low): Trust page vs contracts

- **Understated Safe power.** `phase1Trust.ts` lists only `setShares`/`setOps` ("income and unallocated balance") and ownership transfer. The Safe can also:
  - take all allocated pots (C-06);
  - pause every listing;
  - cancel every draw;
  - list new vaults;
  - in 7 days, install its own keeper.
- **False "not trusted" claim.** `trust-model.md:62` lists "the absence of a withdraw path" as not trusted; that is false for `ProtocolTreasury`.
- **Keeper reach absent.** The Phase-1 statement says nothing about the keeper hot key, which can:
  - steal each raffle prize without a 15-minute human response (C-03);
  - drain sweep credit until a ceiling is accepted (C-01);
  - burn redeemers' units (C-05);
  - control the module floor permanently (C-11).
- The general trust-model's "Owner: void a resolved draw" describes the v1 Distributor. For SweepVault, `voidDraw` is anyone's after 7 days.

**Fix.** Add these sentences. `phase1.test.ts` already pins the Trust page's required sentences, so add them there too.

---

### C-10 (Low): `sweepViaDesk` griefing

**Description.** The MintDesk is deployed (OFF, and the canister is given no desk). Anyone can call `SweepVault.sweepViaDesk(coin, amount ≤ cap)` repeatedly while the cap is fresh (`:328-341`). This moves a coin's entire credit into desk escrow for the 6 h TTL, so `sweep` reverts `Insufficient`. After 6 h anyone can `collectDesk` and park it again. The cost is gas only, and a coin's sweeps can be blocked indefinitely.

**PoC.** `test_C07_anyoneParksACoinsCreditAtTheUnservedDesk` (named C07 in the file). **PASS.**

**Fix.** Deploy the Phase-1 vaults with `desk = address(0)`. The constructor allows it when a venue exists. Or gate `sweepViaDesk` to the keeper.

---

### C-11 (Low): one hot key with a permanent grip on the FloorModule

`DeployStack` passes `registry.keeper()` as the FloorModule's immutable `keeper` (`:102,204`). The same EOA posts SweepVault caps, opens draws and can `recordDelivery`. A leaked key:

- can be rotated for the vaults and the Wrapper only after 7 days;
- can **never** be rotated for the module.

**Fix.** Give the module its own keeper address, or a registry-read keeper. Pre-plan a module redeploy procedure. Keep the draw-opening key separate from the cap/floor key.

---

### Informational

- **C-12.** `ProtocolTreasury` NatSpec (`:18-19`) says a module whose `fundReserve` reverts leaves its share in the pot and "the others are paid". `distribute` calls `fundReserve` without `try` (`:182`), so the whole call reverts. **PoC PASS** (`test_C10_…` in the file). Wrap it in `try`, as `fundFloat` is.
- **C-13.** `_pin` uses `roundAt(block.timestamp) + 20`, which is 60 s ahead. Nitro lets the sequencer stamp up to 24 h behind. With a lag over 60 s, a `requestDraw` caller pins an already-public round and can time the call to pick a favourable one. Measured lag today is about 1 s. Consider deriving the round from the parent-chain block (`block.number × 12 s`) or using a larger `PIN_AHEAD`.
- **C-14.** The stack's vaults use the same `listingId` as the auto listing that Pons launches target. The only on-chain way to stop the OFF Pons path, `setPaused`, also halts sweeps and draws.
- **C-15.** `CollectionDirectory.admin` is fixed at deploy. After a Safe rotation, the old Safe keeps `add`.
- **C-16.** `retire` lacks `_notPaused()`. That is consistent with "redemption cannot be paused", but a paused listing still burns.
- **Other checks** (SWC-105, 115, 132; OWASP SC05, SC06), all fine:
  - no `tx.origin`;
  - no unprotected ETH withdrawal besides C-06;
  - force-sent ETH in `ProtocolTreasury` is treated as income (fine), and in `SweepVault` stays unaccounted (fine);
  - `Ownable2Step` is present and `renounceOwnership` is disabled;
  - `FeeRouter.initialize` is `launcher`-only and runs in the same transaction as the clone;
  - there are no signatures to replay in these contracts.

## 6. Proof-of-concept tests

The files are `test/audit/C/C_SweepVault.t.sol` and `test/audit/C/C_ProtocolTreasury.t.sol`. Run with `forge test --match-path 'test/audit/C/*'`: **9 passed, 0 failed.** The test function names use the IDs the findings had while I worked, which differ from the final report IDs for four of them:

| Test | Report finding | Result |
| --- | --- | --- |
| `test_C01_ceilingProposedAtDeployIsNeverActiveWithoutAccept_andStaleRepostIsUnbounded` | C-01 | PASS |
| `test_C01b_freshStepCompoundsToThreeTimesInAnHour` | C-01 | PASS |
| `test_C02_aTicketHolderRerollsTheRaffleUntilTheyWin` | C-02 | PASS (10 re-rolls for a 20% holder) |
| `test_C03_correctRootWithTruncatedTotalTicketsHandsTheDrawToTheLowestRange` | C-03 | PASS |
| `test_C07_anyoneParksACoinsCreditAtTheUnservedDesk` | C-10 | PASS |
| `test_C04_theSafeTakesEveryPotNotOnlyUnallocated` | C-06 | PASS (13.999… of 14 ETH) |
| `test_C05_preSetSharesIncomeKeepsTheOffPathsAlive` | C-07 | PASS |
| `test_C05b_defaultsLetAnyoneOpenABootstrapLoan` | C-07 | PASS |
| `test_C10_oneRevertingModuleBlocksDistributeForAll` | C-12 | PASS |

Note for anyone extending these tests: under `via_ir`, `block.number` read inside one test function can be stale. `test/utils/Clock.skip` rolls from `block.number`, so the PoCs use their own `_skip` with `vm.getBlockNumber()`.

## 7. Slither

**Run.** `slither . --filter-paths "lib|test|script"`, Slither 0.11.6. crytic-compile ran `forge build --build-info --skip test script --force`. A first attempt with `--ignore-compile` failed because forge 1.8's default build-info lacks outputs.

Outputs:

- `/private/tmp/claude-501/-Users-jlohayon-Documents-SmartVid/6cc0ea2d-ac78-4270-84d0-11af74f0a96f/scratchpad/rook-audit/slither-all.json`
- `/private/tmp/claude-501/-Users-jlohayon-Documents-SmartVid/6cc0ea2d-ac78-4270-84d0-11af74f0a96f/scratchpad/rook-audit/slither-all.txt`

**Whole repo (all of `src/`): 457 results.**

| Impact | Count | Breakdown |
| --- | --- | --- |
| High | 38 | reentrancy-eth 15, arbitrary-send-eth 8, reentrancy-balance 5, uninitialized-state 5, unchecked-transfer 3, arbitrary-send-erc20 1, weak-prng 1 |
| Medium | 143 | incorrect-equality 42, unused-return 39, uninitialized-local 34, reentrancy-no-eth 21, divide-before-multiply 6, out-of-order-retryable 1 |
| Low | 191 | timestamp 84, reentrancy-benign 38, shadowing-local 35, calls-loop 18, reentrancy-events 11, missing-zero-check 4, events-maths 1 |
| Informational | 81 | |
| Optimization | 4 | |

Files with the most results: FloorModule 72, LiquidityLock 39, HomeVaultBase 31, NftVaultBase 28, ProtocolTreasury 22, AssetVault 21, SweepVault 20, EtchRouter 18, Distributor 18, FrogCurve 16. Other reviewers triage their own files.

**Triage for area C files.** `ProtocolTreasury` has 22 results (3 High, 5 Medium). `SweepVault` has 20 (6 High, 2 Medium). `ListingRegistry` has 8 (Low/Info only). `CollectionDirectory` and `WrapClaim` have none. All are false positives or covered by findings above:

- **`arbitrary-send-eth` in `ProtocolTreasury.bootstrap`.** The module must be `isCollection`, which is owner-added. It is a true centralization point, reported as C-06; not an unprotected send.
- **`reentrancy-eth` / `reentrancy-balance` in `SweepVault.sweep`/`buyback`.** False positive: `nonReentrant` applies, `venue` is immutable, and ETH and wX are measured by balance difference. No non-guarded function changes `credit`, `proceeds`, `units` or the wrapper balance. The existing `test_theVenueCannotReenter` covers it.
- **`reentrancy-eth` in `ProtocolTreasury.distribute`/`bootstrap`.** False positive: `nonReentrant` applies. The unguarded `repay` only books a repayment, and the pots are decremented before the call and restored in `catch`. In `bootstrap`, `l.principal` is written after the calls deliberately, so `_repay` sees 0 due. Confirmed by reading `FloorModule._repay`.
- **`unchecked-transfer`, `wrapper.transfer`/`transferFrom` in `SweepVault.claim`/`depositWrapped`.** False positive: the Wrapper is OpenZeppelin ERC20 and reverts on failure. Wrapping it in `SafeERC20` would be tidier.
- **`reentrancy-no-eth` in `SweepVault.resolve`** (beacon is immutable), **`incorrect-equality`** (`amount == 0` guards), **`uninitialized-local`** (`total`, zero-initialised by design) and **`unused-return`** (`fundFloat`, `desk.collect`; the vault measures balances instead): all false positives.
- **Low `timestamp`.** All dual-clock waits (`Wait.elapsed`). Acceptable. The C-13 note covers the drand pin.
- **Low `missing-zero-check`, `setOps`.** Intended: zero means the ops share goes to reserves.
- **Low `calls-loop` / Informational `costly-loop`, `distribute`.** The number of collections is owner-bounded. The DoS risk is C-12.
- **Observations on other areas' files**, not triaged in depth:
  - `arbitrary-send-erc20` in `EtchRouter.unlockCallback` is a false positive: only the PoolManager calls it, and only in response to the router's own `unlock`, so `payer` is always the router's caller;
  - `uninitialized-state` in `AssetVault._charges` should be checked by its owner (`AssetVault` is not live in Phase 1).

## 8. Test gaps

- No test, rehearsal or check ever calls `acceptCapCeiling`, and no failing check exists for `capCeiling == 0` (C-01).
- `testFuzz_theRaffleCannotBeAimed` bounds the wait at ≤ 23 h, so the `redraw` grinding path (C-02) is untested. There is also no end-to-end test that anything resolves or claims SweepVault draws in Phase 1.
- No test or verifier compares on-chain `DrawOpened.totalTickets` / `snapshotBlock` with the published snapshot file (C-03).
- No test documents the Safe's full power over `ProtocolTreasury` with a hostile registered module (C-06).
- The Phase-1 treasury config is never rehearsed: `golive-dryrun.sh stack` and `DeployStack` local owner steps never call `setShares`/`setBootstrapLoan`. No invariant covers "a pot with share 0 is never delivered" (C-07).
- `deploy.test.sh` / `golive-dryrun.sh` do not assert that `golive-check` **fails** when step 7 is skipped or misconfigured (C-08).
- No test for sequencer timestamp lag against the drand pin (C-13).

## 9. Not covered

- FloorModule economics: floor-walk extraction by a keeper, spreads, reserve and backing.
- FrogCurve math, LiquidityLock and EtchRouter internals beyond the sweep route.
- Wrapper redemption internals beyond access control; the Wrapper's own `redraw` re-roll was flagged to its reviewer.
- OfferBoard; NFT and home-chain contracts (OFF and not deployed).
- The canister (Rust) and the timelock, except the `payee` check.
- Keeper, ops and monitor code beyond their call surfaces and alerts.
- The web app beyond the Trust page.
- The deep fuzz/invariant profile; the full forge suite (only the 9 PoCs were run).
- Economic modelling of C-04 on real pool fee tiers.
