# Round 2 re-review, 2026-09-27 (nft-side), part A: contracts, pooled graduations, deploy

Reviewer: independent AI review (round 2), 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at
**`27c0962`**. Diffs read: `09a680a..8c107e9` (`nft-pool`) and `8c107e9..HEAD` (fix round 1), for `src/`, `script/`, `test/ethnft/`,
`test/StackChecks.t.sol` and `deployments/launch/nft-pilot.json`. The canister was read only where the contract side depends on it
(`ethflow.rs` `NeedAdvance`, `buy_pass`, `give_back_now`; `floorsink.rs` `view`; `custodycap.rs` `stage0` / `check_acquire`).

**Nothing existing was modified. Nothing was committed, pushed, deployed or signed. No key was used. No chain was contacted:** every
run was a local forge test, with `FOUNDRY_OUT` / `FOUNDRY_CACHE_PATH` in the reviewer's scratch directory. PoCs were run from
`test/audit_nft_a2/` and removed afterwards; they are in `poc/A/` with their output:

- `poc/A/AuditNftA2.t.sol` (8 tests; it imports `./AuditNftABase.sol`, which is round 1's `../../poc/A/AuditNftABase.sol`, unchanged);
- `poc/A/AuditNftA2.output.txt` (the run);
- `poc/A/round1-pocA-rerun.output.txt` (round 1's `poc/A/`, unmodified, re-run at HEAD).

## Verdict (part A)

No new Critical or High. **One new Medium (A2-M-1)**: a pooled collection whose floor rises more than ≈ 5 % between its
graduations advances its whole pooled budget below the floor. The ETH leaves 4663 and then waits on Ethereum with no way back, and
the module's conversions stop meanwhile. The design says the opposite ("nothing is advanced … no D4-style stranding"), and the test
that claims to prove it is vacuous. It concerns BAYC and Pudgy, the two collections that pool in the launch configuration.

Every round-1 contract fix holds: A-M-2, D-M-1 / A-L-3 (the contract half) and D-L-7. The round-1 PoCs turn red where they should.
Two new Lows and three Infos.

## Counts (new in round 2, part A)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 1 | 2 | 3 |

Still open from round 1 (not new): **A-L-2** (Low, not fixed, sharpened below). A-I-1, A-I-2, A-I-4, A-I-5 and A-I-6 are unchanged.

## 1. Round-1 items (contract side)

| ID | Sev (R1) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **A-M-2** `redeemToRelist` free option | Medium | **Closed** | Round 1's `test_A_M2_…` and `test_A_M2b_…`, re-run unmodified, now **fail** with `NotSupported()`. `EthNftWrapper.sol:322-324` is `pure` and reverts. **Other ways to get the same effect were looked for:** (a) a ticket the canister cannot deliver. Delivery is a plain `transferFrom` (`ethnft.rs:757`), so a contract destination gets no callback and cannot revert it. `redeem` refuses zero and the custodian (`:308-311`). `judge_transfer` (`ethnft.rs:761-775`) refuses only those two, a ticket inside its last day, and a token we do not own. None of these can be picked by the redeemer. (b) `redeemToBurn` goes to `DEAD`, which is delivered and counted in `burned`. No cheap refund loop is left. The regression `test/ethnft/EthNftWrapper.t.sol` `test_A_M2_…` is a real test: it checks the revert, nothing burned, and another holder redeeming. |
| **A-M-1** deposit → burn at a high floor | Medium | **Contract unchanged**. Closed only as far as the canister's `deposit_floor_ok` holds (canister part) | Round 1's six `A_M1*` PoCs still **pass** at HEAD (+0.2719 / +0.0837 / +0.5235 / +0.0516 ETH). That is expected: the fix is canister-side (a deposit waits while `accepted` > 1.05 × the verified sales' median). On chain, `mintDeposit` still mints whenever the custodian asks, and a deposit made during a `PosterSource` pause burns at the pause floor, never above the posted floor (`test_A2_holds_aDepositDuringAPosterPause_…`: 0.9007 at F 0.93). |
| **D-M-1 / A-L-3** one switch for every collection | Medium / Low | **Closed** (contract half) | `PosterSource.sol:42-58`: `pause()` is the registry's owner or guardian (a zero guardian cannot be impersonated), and `unpause()` is the owner only. `keeper()`, `keeperPaused()` and `keeperResumedAt[Block]()` combine both switches (`:61-76`). The module reads everything through `registry` = the PosterSource (`FloorModule.sol:387, 878-881, 1142-1144`). So the per-collection pause triggers exactly what the registry's did: `_keeperDown`, the pause floor, `maxItemCost` 0, `hasRoomFor` false, and refunds held. Checked in `poc/A`: (1) mint → the poster is paused → burn is a **loss** (1 ETH in, 0.9578 out); the A3-M1 / A4-L1 pause floor applies unchanged. (2) The refund hold follows the **later** resume: a registry pause and resume inside a poster pause does not release it; the poster's owner resume starts `RESUME_GRACE`; an `unpause()` of an unpaused poster stamps nothing, so it gives no free grace extension. (3) The guardian cannot unpause. Round 1's `test_A_L3_…` still passes because it exercises the **registry** pause, which is intentionally still canister-wide. The regressions `EthNftFlow.t.sol` `test_D_M_1_…` and `Pooled.t.sol` `test_A_L3_…` are real tests. Immutables and the constructor are unchanged, and the new state is fresh storage in a new contract, so there is no upgrade surface. |
| **D-L-7** slugs | Low | **Closed** | `StackChecks.checkNftSlugs` (`:99-109`) is called in `DeployStack.s.sol:114-117` before any broadcast. The test covers a repeat, an empty slug and upper case. |
| **A-L-1** shared mint room | Low | Canister-side (`buy_holds_mint`), not re-checked here | Round 1's PoC still passes on chain, as expected: the wrapper is unchanged. |
| **A-L-2** the deploy key keeps the NFT wrapper's mint limit | Low | **Not fixed** (as the fixer says), **sharpened** | `EthNftWrapper.sol:51, 174, 188-201`: the deploy EOA is the permanent `deployer`. golive-check reads `maxMintsPerDay` for NFT wrappers (`golive-check.sh:647`), but `pendingMintLimit` only for `inscriptions` (`:640-642`) and `deployer()` only for the claim (`:595`). So a raise proposed by the deploy key before go-live (e.g. 1,000 a day, acceptable by anyone 7 days later) is **not** caught for an NFT wrapper. The module's `maxItemsPerDay` 3 and the canister's custody caps still bound purchases, but deposits are bounded only by the wrapper limit and the canister's caps. Fix as in round 1, plus a `pendingMintLimit() == 0` check for `KIND=nft`. |
| **Round 1's ledger fuzz** (W1, W3, W4, M2) | holds | **Holds at HEAD**, pooled | The round-1 fuzz fails at HEAD only because it calls the now-refused `redeemToRelist`. Re-run without relist, with pooling graduations (0.7 ETH at F 0.93 = 0.55 units each), deposits, `redeem` / `redeemToBurn`, advance / settle / return, public mints and burns: 512 runs pass, and the 20-seed non-vacuity run takes every action (deposits 88, advances 35, settles 25, returns 6, graduations 106, burns 99, redeems 24, resolves 7, delivers / marks 6, refunds 14). `supply == (held − open) × ONE + ethBacked`, `C + R + T = E`, and the module's books hold at every step. |

**Carried-over 2026-09-26 contract fixes:** unchanged by either delta (`git diff 09a680a..HEAD -- src/` touches only `PosterSource`
and `EthNftWrapper.redeemToRelist`). Round 1's §7 table stands. N-3 / A3-L1 (`RESUME_GRACE` from `keeperResumedAt`) now reads
the later of two stamps, re-checked above.

## 2. The pooled-units backing invariant

**Claim** (`phase1-nft-pooled.md:32-34`): every wX unit is 1:1 backed at all times, by ETH at its module or by an NFT in custody.

| Situation | What holds | Evidence |
| --- | --- | --- |
| Pooling (a graduation under one item) | Holds. The units are ETH-backed at their own module, `maxItemCost` is 0 under one convertible unit, and nothing is advanced | `Pooled.t.sol` `test_BAYC_…` (checked), fuzz |
| Deposit while pooled | Holds. `ethBacked` and `netUnits` are unchanged, and the item-backed side grows by one | `Pooled.t.sol` `test_aDepositWhilePooled_…`, fuzz |
| An ETH-backed (pooled) unit redeems a deposited NFT | The ledger holds. In aggregate the module's ETH still backs exactly `ethBacked` units. Economically, a pooled holder can take the depositor's specific NFT (the adverse selection of round 1's A-I-4, now also across pooled / deposit holders) | fuzz; `test_aDepositWhilePooled_…` |
| The advance → settle window | **The claim is literally false**, as documented in relay-conversion §2.2: the transit unit's backing is at Relay or on Ethereum, not at the module, and is excluded from burns (`_liveUnits`). With pooling this window is **no longer "seconds"**: see A2-M-1 | `FloorModule.sol:684-712`; A2-M-1 |
| A Relay refund / `returnAdvance` short | The shortfall comes from revenue, then surplus, then is a shared loss to the convertible units (`:735-757`), as designed | fuzz (returns at 99 %) |
| A cancelled fill / `markUndeliverable` | No contract state changes on a cancelled fill (it is canister-side until `settleAdvance` / `returnAdvance`). `markUndeliverable` only enables the refund at the grace, and the refund re-queues the item (`:425-431`) | fuzz |
| Cross-collection | Holds on chain. Each module pays only its own `backing`, and `settleAdvance` counts only its own wrapper's wX (`:720`). A BAYC-minted unit sent to Milady's module does not settle Milady's advance. `returnAdvance{value}` accepts any ETH, so which module gets returned ETH is the canister's word (round 1 §6a, unchanged) | `Pooled.t.sol` `test_oneCollectionsPoolNeverFundsOrBacksAnothers` (checked: it is a real test) |

**`Pooled.t.sol` read critically:** four of its five §5 claims are proved.

The exception is "a floor rise after the first graduation: **no advance while the average backing is short**"
(`test_aFloorRiseWhilePooled_…`, `:232-249`). It asserts only that `advance(7.5 ether)` reverts, which is **above** `maxItemCost()`
(7.3267). It never tries `advance(maxItemCost())`, which is what the canister sends, and which succeeds. See A2-M-1.

The suite also runs `initMintLimit(1_000)` instead of stage 0's 3 (round 1's test gap 1, repeated).

## 3. New findings

### Medium

#### A2-M-1: a pooled collection whose floor rises between graduations advances its pool below the floor; the ETH then waits on Ethereum with no way back, and the collection's conversions stop

- **Where:**
  - `src/floor/FloorModule.sol:1329-1333` (`maxItemCost` = min(item cap, **average** backing + revenue + surplus): no floor
    comparison) and `:684-712` (`advance`);
  - `autobuyer/canister-rs/src/ethflow.rs:1866-1890` (`NeedAdvance`: `amount = v.item_cap`, advanced whenever
    `itemsink::may_start` passes) and `:880-894` (`buy_pass` starts a buy on `wanted`, not on the price);
  - `floorsink.rs:118-128` (`item_cap = min(maxItemCost, limit)`);
  - `ethflow.rs:2132-2137` (`give_back_now` refuses an `Arrived` buy);
  - claims `docs/v7/phase1-nft-pooled.md:176-178` and the vacuous test `test/ethnft/Pooled.t.sol:232-249`.
- **Mechanism:**
  - A pooled collection needs two graduations per item: BAYC at 9 ETH (0.82 units each) and Pudgy at 5 ETH (0.89 each), in the
    launch config. The two graduations can be days apart.
  - The first graduation's units are backed at `1.03 × F₁`, and the second's at `1.03 × F₂`.
  - After the second, `netUnits ≥ 1.05`, so the module "wants" one item, and `maxItemCost` is the **blend** plus revenue and
    surplus.
  - The canister advances exactly that (`amount = v.item_cap`) with no check that it covers the posted floor, let alone the rank-2
    listing plus Relay's fee.
  - What arrives is `≈ maxItemCost × 0.9964 − gas margin`. The canister's buy bound then needs a listing at or under that.
  - For a rise of more than about 5.1 % between the two graduations (PoC scan, BAYC): arrives/floor is 10,106 bps at +3 %, 10,008
    at +5 %, 9,959 at +6 %, 9,863 at +8 %, and 9,769 at +10 %. So even the rank-2 listing (the posted floor itself) is out of
    reach.
  - The buy sits `Arrived`: the D4 path, paged at 24 h. It has no give-back (`give_back_now` accepts only `Funded` or `Stuck`
    while bridging), so the ETH waits on Ethereum until a listing under it appears or the floor falls.
  - Meanwhile the module has an open advance (`AdvanceOpen`). **Every later graduation of that collection pools and cannot
    convert**, and the transit unit's backing is off 4663 (excluded from burns). The monitor's pool page (SEV2 "due a buy with no
    open buy for a day") does **not** fire, because there is an open buy. Only the D4 page does.
- **Impact:**
  - No loss of principal: the ETH is at the canister's address, earmarked to the buy, and a floor dip or a cheaper listing ends it.
  - But it is exactly the stranding the design says pooling cannot cause: "**nothing is advanced** … No ETH leaves 4663, so there
    is no D4-style stranding on Ethereum".
  - It blocks one collection's conversions for as long as the floor stays up.
  - It needs only an ordinary market move (> 5 % over the days between two BAYC or Pudgy graduations) and no attacker.
  - The unpooled collections are practically unaffected, because the canister buys minutes after the graduation.
  - Medium: a stall of one collection's path that needs a condition, and the documented safety property of the pooled design is
    false.
- **PoC:** `poc/A/AuditNftA2.t.sol` `AuditNftA2_M1_PooledRiseAdvancesBelowTheFloor`, **PASS** (the weakness):
  - `test_A2_M1_aFloorRiseBetweenPooledGraduations_…`: BAYC 9 ETH; graduate at 6.79, the floor walks +10 % to 7.469, graduate
    again. `maxItemCost` = 7.3267, under the floor. `advance(maxItemCost())` succeeds: the module pays 7.3267 ETH. About 7.2972
    ETH arrives, under the 7.469 floor. A third graduation then cannot advance (`AdvanceOpen`). The books hold throughout.
  - `test_A2_M1_theThreshold`: the scan above.
- **Recommended fix:**
  - In the canister, do not start a buy, and do not advance, unless `item_cap × (1 − relay_max_fee) − GAS_KEEP` covers the
    canister's own bound at the fresh reading (at least the rank-2 floor `L`, better `L × (1 + premium)`). Otherwise leave it
    `wanted` and waiting on 4663 (the pool page then fires after 24 h, as the design intends).
  - Or give the module an on-chain floor gate: `advance` refuses `amount < accepted × (1 + minMarginBps)`.
  - Correct `phase1-nft-pooled.md` §3.4.
  - Make `test_aFloorRiseWhilePooled_…` try `advance(maxItemCost())` (the canister's call), or the canister-side gate.
  - Consider a give-back path for an `Arrived` buy older than N days (Relay back to 4663, then `returnAdvance`), so D4 has an end
    other than the market.

### Low

#### A2-L-1: J1's alternative (BAYC 10 ETH, `convBps` 1,100, reserve 0) leaks ≥ 0.41 ETH per ape to the canister's gas pot, not the module's revenue as the doc says

- **Where:** `FloorModule.sol:715-731` (`settleAdvance` moves no ETH); claim `docs/v7/phase1-nft-pooled.md:93-94` ("the part not
  spent on the ape becomes the module's revenue, as every conversion's leftover does"); `deployments/launch/nft-pilot.json`
  `_baycAlternative`.
- **Mechanism:**
  - On the advance path the module pays out the whole `maxItemCost` (= `accepted × 1.11`) at `advance`.
  - `settleAdvance(cost)` only records the fill, so `advanced − cost` stays at the canister's Ethereum address as the gas pot
    (phase1-nft §5; round 1's A-I-2).
  - At `convBps` 300 that is the documented 1–3 %. At 1,100 it is ≥ 5.7 % of every ape even when the canister pays its full +5 %
    premium, and ≈ 11 % at the floor.
- **Impact:** not the committed config (9 ETH, defaults), but it is the doc's recommended alternative. If the operator takes it,
  about 0.41–0.75 ETH per ape leaves BAYC holders' backing for the operator-controlled gas pot, unaccounted and never returned.
- **PoC:** `AuditNftA2_L_J1LeftoverLeaves`, **PASS**: 7.5369 advanced, 7.1295 paid at +5 %, 0.4074 ETH out of the module, revenue
  unchanged but for dust.
- **Fix:**
  - Correct the doc.
  - Before recommending `convBps` > 300: return the leftover over the gas pot's target to the module (Relay back plus
    `returnAdvance`-like revenue credit), or advance only the canister's buy bound (`min(maxItemCost, L × (1 + premium) +
    fees)`), which is round 1's A-I-2 recommendation.

#### A2-L-2: launch config: stage-0 custody caps across five collections make BAYC and Pudgy mostly unbuyable, and let a depositor hold them off at no cost

- **Where:** `autobuyer/canister-rs/src/custodycap.rs:23-28, 167-174, 206-227` (3 items / 10 ETH per collection, **6 items /
  10 ETH globally**); `ethflow.rs:885` (`check_acquire` at the item's value before a buy starts); `phase1-nft-pooled.md` J4 / J7
  (operator-pending).
- **Mechanism:**
  - A BAYC buy needs ≈ 7 ETH of global value room, so it can start only while every other collection together holds ≤ 3 ETH
    (for example, three Miladys plus nothing else). Pudgy needs ≈ 3.46 ETH of room.
  - With deposits on (D5), any holder of one or two cheap NFTs (an Azuki at 0.67, a Milady at 0.93) can deposit them and keep
    the global value above that line.
  - A deposit mints them a fungible unit they can sell or redeem, so the room costs them nothing.
  - Once the six global items or 10 ETH are used, **every** collection's buys stop and every new deposit is returned.
- **Impact:** no loss. BAYC and Pudgy pools stay ETH-backed and burnable, and the pool page fires after 24 h. But the five-collection
  launch is in effect a ≈ 10 ETH, 6-item pilot in which the two pooled collections rarely buy. This is the operator's pending J7.
  It is flagged here because the launch list includes BAYC at stage 0, and golive-check passes it.
- **Fix:**
  - The operator decides before launch. Either raise the global value cap (timelock G7) to fit one ape beside the others' stage-0
    items, or leave BAYC and Pudgy out until stage 1.
  - At minimum, reserve room for a pooled buy against deposits (J4's "reserve one slot for buys").

### Info

- **A2-I-1. The transit window is no longer "seconds".** `phase1-nft.md` §5 ("one advance … in Relay's hands for seconds") and
  `phase1-nft-pooled.md:32` ("every unit is 1:1 backed … at all times") assume the advance buys at once. With pooling (A2-M-1), and
  with D4 generally, the transit unit's backing can sit on Ethereum for days, outside burns, as ETH at the canister's address.
  State it on the Trust page and the reserves page.
- **A2-I-2. Per-collection deploy lists: silent misreads.**
  - `DeployStack.s.sol:271-272` casts `_perCol(…)` to `uint16` before `checkNftCollection` bounds it. A typo of 65,836 reads as 300
    and passes.
  - A single-collection variable (`ETCH_NFT_NORMAL_RAISE`) applies to **every** collection of a multi-collection run when the list
    is unset (`_perCol`, `:484-489`).
  - golive-check's `targetcheck` compares the factory to the stack file only, never to `deployments/launch/nft-pilot.json`. So
    "BAYC at 9 ETH, the others at the defaults" is not held by the go/no-go read.
  - Fix: bound before the cast (`require(v <= type(uint16).max)`), and have golive-check hold each collection's targets against
    the launch file.
- **A2-I-3. `PosterSource.unpause()` emits `PosterUnpaused` when nothing was paused** (`PosterSource.sol:50-58`). It is harmless (no
  stamp), but the monitor or indexer should not read the event as a resume. `pause()` likewise re-emits on a paused poster.

## 4. Deploy and go-live changes read

- **`StackChecks.checkNftCollection`** (`:139-150`) holds the bounds of `phase1-nft-pooled.md` §2. `checkParams` is re-run per
  collection with its own `convBps` / `reserveBps` (`DeployStack.s.sol:298-303`).
- **The read-backs** of the module's conversion and reserve and the factory's four targets are `require`s (`:344-346`).
- **The stack file** records `targets` / `moduleParams` per slug (`:433-442`).
- **golive-check:**
  - holds, for each collection, `posterSource.paused() == false` (D-M-1);
  - holds the premium at 500 and chain 1 (D-L-6, with `--canister`);
  - ties `--canister-evm` to the custodian: a **WARN** only under `--phase1` when it is not passed (D-L-6 "partly", as stated);
  - the launch list now includes Pudgy and BAYC (`NFT_LATER` empty);
  - `targetcheck` compares and prints the pooling (A2-I-2 for what it does not hold).
- **J5** (the opening target ignores the reserve share): confirmed harmless under pooling. The first BAYC coin converts 0.82 of an
  item and waits for the next graduation. Left as the doc says.

## 5. Tests run (part A)

| Run | Result |
| --- | --- |
| Round 1's `poc/A/` unmodified at HEAD | 9 pass, 4 fail. `A_M2`, `A_M2b` and the two ledger runs fail with `NotSupported()` (relist refused: **A-M-2 closed**). `A_M1*` ×6, `A_L1`, `A_L2` and `A_L3` still pass: canister-side fixes, not fixed, and the registry pause by design (see §1) |
| `poc/A/AuditNftA2.t.sol` | **8 pass**: 3 weaknesses shown (A2-M-1 ×2, A2-L-1); 5 holds (poster switch ×3, pooled ledger fuzz 512 runs and non-vacuity) |
| Full forge suite, fork proof | Not run here (the lead runs the test bar) |
