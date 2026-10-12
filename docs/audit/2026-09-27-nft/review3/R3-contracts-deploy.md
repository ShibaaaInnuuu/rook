# Round 3 re-review, 2026-09-27 (nft-side), part A: contracts, the module under the way back, deploy and go-live, launch config

Reviewer: independent AI review (round 3), 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at
**`4949da0`**. The fix delta read is `79455e1..HEAD`: `src/`, `test/ethnft/Pooled.t.sol`, `script/DeployStack.s.sol`,
`script/golive-check.sh`, `scripts/golive-dryrun.sh`, `deployments/launch/{nft-pilot.json,monitor-nft.json}`, and the claims in
`fixes-round2.md` §2, §6 and §7. The canister was read where the module depends on it: `ethflow.rs` `advance_covers`, `buy_pass`,
`step_buy` (`Arrived`, `BackBridging`), `back_now`, the `Returning` act, `give_back_now`; `custodycap.rs` `check_acquire`.

**Nothing tracked was modified. Nothing was committed, pushed, deployed or signed. No key was used.** Every run was in a private copy
under the reviewer's scratch directory: forge (`test/audit_nft_a*/`), the canister's fake world (`tests/audit_nft_a3.rs`, a copy of
`tests/audit_nft_r2.rs` with one test appended), and golive-check's new functions extracted verbatim into a harness with a stubbed
chain. Network use: read-only GETs to CoinGecko's public API and opensea.io's collection pages (the D8 references). The dry run was
not run here (the lead runs it in the main tree); its partial output in `bar/dryrun.txt` was read.

PoCs and outputs are in `review3/poc/A/`:

- `AuditNftA3.t.sol` + `AuditNftA3.output.txt` (forge, 4 tests: 2 weaknesses, 1 control, 1 dead-branch probe);
- `audit_nft_a3_canister.rs` + `.output.txt` (canister fake world, 1 test);
- `golive_harness.sh` + `.output.txt` (golive-check `launchcheck` / `moncheck` / `monglobal`, lines 65-71 and 507-572, with a stub chain);
- `d8-references.output.txt` (each OpenSea slug and CoinGecko id checked against its contract);
- `review2-pocA-rerun.output.txt`, `round1-pocA-rerun.output.txt` (the earlier rounds' part-A PoCs, unmodified, at HEAD).

## Verdict (part A)

**No Critical, High or Medium.** `src/` is unchanged since `79455e1` (`git diff 79455e1..HEAD -- src/` is empty), so every contract
verdict of round 2 stands. A2-M-1 is closed by the canister's advance gate. Its constants match between `ethflow.rs` and
`Pooled.t.sol`, and it is applied both before a buy starts and again before the advance. The go-live changes do what they claim for
targets, conversion, reserve and purchase limits. They are keyed by collection address, compare exact integers and refuse a missing
or extra collection.

Four new Lows:

- **A3-L-1:** every trip of the new way back is paid by the collection's holders, and nothing bounds the repetition except the
  A2-M-1 gate.
- **A3-L-2:** `--launch` never holds a wrapper's name, symbol or slug.
- **A3-L-3:** `--monitor-config` accepts swapped D8 references, a guardian that is not the registry's, and placeholders outside
  two blocks.
- **A3-L-4:** C-I-2, sharpened. Deposits enter custody at 0 wei, so with BAYC and Pudgy launched, stage 0's 10 ETH value caps no
  longer bound what custody holds.

Three Infos.

## Counts (new in round 3, part A, and not actually fixed)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 4 | 3 |

Still open from earlier rounds (not new): A2-L-2 (Low, J7), A-L-2 (Low, partly), A2-L-1 (Low, partly). They are not counted
above.

## 1. Round-2 items in scope

| ID | Sev (R2) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **A2-M-1** pooled advance under the floor | Medium | **Closed** (canister gate). The contract is unchanged, by design | `ethflow.rs:989-1000` `advance_covers`: `item_cap − GAS_KEEP_WEI (0.0002 ETH, relayconv.rs:52) − item_cap × 50/10 000 ≥ accepted × 10 100/10 000 + ETH_GAS_RESERVE_WEI (0.003 ETH, ethnft.rs:77)`. `accepted` is `floor()`'s first word, i.e. the module's `accepted` (`FloorModule.sol:1294-1296`). It is called in `buy_pass` before `check_acquire` (`:1056`) and again in `NeedAdvance` right before `advance_call` (`:2214`). No other path calls `advance`. The advance amount is `v.item_cap = min(maxItemCost, max_purchase_wei)`. Round 2's `AuditNftA2.t.sol` still passes 8/8 at HEAD, as expected: it shows the MODULE pays `advance(maxItemCost())` under the floor, and the module is unchanged. The fixer's `contracts-poc-a-rerun.output.txt` is honest (the same 8 passes). It omits round 1's `poc/A`: re-run here, 9 pass / 4 fail, identical to round 2 (the 4 fail on `NotSupported()`, A-M-2 closed). **Residual:** the way back this fix added is not free (A3-L-1). |
| — `Pooled.t.sol` rewrite (`test_aFloorRiseWhilePooled_…`, `test_A2_M1_…`, `_canisterAdvances`) | (test) | **Not vacuous in its numbers, but a mirror** | `_canisterAdvances` (`Pooled.t.sol:235-240`) is `advance_covers` exactly: the same four constants and the same integer order of operations (it reverts instead of saturating under 0.0002 ETH, which is irrelevant here). But it tests the Solidity copy, not the canister: a change to `ethflow.rs` would leave both forge tests green. The canister-side regression is `audit_nft_r2.rs` `r2_a2_m1_a_module_whose_item_cap_does_not_cover_its_floor_never_advances` (read: it drives the real gate in both places). The floor-rise test's last branch never runs (A3-I-1). |
| **A2-L-1** J1's leftover goes to the gas pot | Low | **Partly** (as the fixer says) | The doc is corrected (`phase1-nft-pooled.md:93-98`). The behaviour is not changed: `settleAdvance` moves no ETH. Round 2's `test_A2_L_J1Alternative_…` still passes (0.4074 ETH out of the module at `convBps` 1,100). Harmless at the committed 300. |
| **A2-L-2** stage-0 custody caps vs BAYC and Pudgy | Low | **Not closed** (J7, operator-pending), **corrected and sharpened** | See §3 (launch configuration). One correction to round 2: deposits enter H2 at **0 wei** (`ethflow.rs:1432`, `:2333`), so a depositor cannot hold off BAYC through the global *value* cap. What a depositor fills at no cost is the 6-item global count (for example 3 Azuki and 3 Milady, ≈ 4.8 ETH of NFTs), and that stops every collection's buys and every further deposit. The value side is A3-L-4. golive-check still passes BAYC at stage 0 and does not read the global caps (A3-I-3). |
| **A2-I-1** the transit window | Info | Unchanged in substance | The transit can now last up to 48 h and then a way back. Not re-checked on the Trust page (part D). |
| **A2-I-2** deploy lists | Info | **Closed** for what it named; residual in A3-L-2 | `DeployStack.s.sol:484-487` `_bps` requires `v ≤ 65 535` before the cast (`:271-272`). `_perCol` (`:493-500`) refuses a one-collection variable when several collections are deployed and the list is unset (a set list wins silently over a set single variable: harmless). golive-check `launchcheck` (`golive-check.sh:522-530`) holds `normalRaise`, `earlyRaise`, `earlyUntilEth`, `maxRaise`, `convBps` and `reserveBps` against the launch file, keyed by the stack's `custody.collection`, which is tied on chain to `wrapper.collection()` (`:690`). The values are Python integers compared as exact strings, so there is no float or precision loss. `:833` refuses a launch file that misses or adds a collection. An absent or null value is refused, because `same` FAILs on an empty side (`:68-70`). Harness: 30 `ok` lines on the committed file. The dry run refuses the shifted 9 ETH. **Not held:** name, symbol and slug (A3-L-2). |
| **A2-I-3** `PosterSource.unpause()` event | Info | Unchanged (`src/` unchanged) | — |
| **A-L-2** the deploy key keeps the NFT wrapper's mint limit | Low | **Partly** | golive-check now holds `pendingMintLimit() == 0` for NFT wrappers (`golive-check.sh:694`, inside the NFT branch). The key's powers are unchanged: round 1's `test_A_L2_theDeployKeyCutsMintsAfterLaunch` still **passes** (an instant cut of the limit after go-live, a stall of every deposit and buy mint of that collection), and a later raise proposal is caught only by the monitor's SEV2 on `MintLimitProposed` (the NFT wrappers are in the monitor's `wrappers`, `evm.mjs:862`). |
| **D2-L-5** `--canister-evm` a FAIL under `--phase1` | Low | **Closed** | `golive-check.sh:713-716`: a FAIL, unless `GOLIVE_REHEARSAL=1`, which is refused on any chain but 31337 / 9466301 (`:82-85`). On 4663 `--phase1` is implied (`:80`). |
| — `--launch` / `--monitor-config` required under `--phase1` | (new) | **Holds** | `golive-check.sh:836-839`, only when the stack has NFT collections. Caveat: under `--phase1 --before-reinstall` (no `--canister`), every `ethcheck` item, `max_purchase_wei` included, is only a note, with a WARN to re-run (`:873-877`). That is by design. |

**The dry run's new refusals** (`scripts/golive-dryrun.sh:235-280`, output `bar/dryrun.txt` at 16:56, NFT section complete, the
run still in its drand wait) do exercise the new code:

- Each `expect` needs a non-zero exit AND a pattern that only the new FAIL line prints: `FAIL pudgypenguins factory.normalRaise =
  the launch file's`, `FAIL bayc canister max_purchase_wei`, `pause_on_no_good_post_h is not armed`, `lacks one of D8's
  references`, `--canister-evm is required under --phase1`, `without --launch`.
- The positive run counts at least 35 `= the launch file's` lines (5 × 6 targets + 5 purchase limits) and exactly 5 "both D8
  references" lines.
- All were `ok` in `bar/dryrun.txt`.
- The shifted-targets refusal edits the launch file rather than the deploy. That is equivalent, because the comparison is
  symmetric.
- The ports are relocatable (`DRYRUN_PORT`, `STACK_PORT`: `:15`, `:504`).

## 2. The module under the new way back (A2-M-1's `returnAdvance` path)

- **Accounting of a short return** (`FloorModule.sol:735-758`):
  - The transit unit becomes ETH-backed again at its released basis.
  - The shortfall `basis − msg.value` is taken from `revenue`, then `surplus`, then as a loss to `backing` shared by the
    convertible units.
  - Proved exactly in `test_A3_control_…`.
  - The unit ledger (`supply == (held − open) × ONE + ethBacked`, `C + R + T = E`, the module holding every wei it owes) holds
    after every short return (the `_books` checks in all four tests).
  - **What does not hold** is the economic "1:1 at the backing": each trip lowers the ETH per unit (A3-L-1).
- **The frog side:**
  - `returnAdvance` is the same function the frog conversions use, and it is unchanged.
  - Frogs are Phase 2 (off).
  - No new on-chain surface.
- **Another collection's money:**
  - On chain, `returnAdvance` accepts any value from the custodian. The custodian is one EOA for all five modules, so the chain
    cannot tell whose ETH it is.
  - The canister sends it to the buy's own module (`ethflow.rs` `Returning` act: `b.module`, `:2193-2199`), with
    `wei = min(rise on 4663, order value)` (`:1150-1156`) while the one Relay leg is held (`relay_busy` includes the `Back*`
    phases, `:158-165`).
  - The monitor matches every `AdvanceReturned` to a `Returning` / `Returned` record of that module (`records.mjs:81-86`), so a
    misrouted return would page SEV1.
  - One weakness for part C to weigh: the rise is measured on the canister's whole 4663 balance. Anyone can add to it
    (`FloorModule.burn(…, to = canister)`), and the canister's own 4663 gas lowers it. Either moves the returned amount by that
    much. There is no gain for an outsider.

## 3. The launch configuration

**Addresses.** The launch file (`nft-pilot.json`), golive-check's `NFT_LAUNCH` list (`golive-check.sh:455`) and
`monitor-nft.json` all name the five given contracts (Milady `0x5af0…25a5`, Azuki `0xed5a…c544`, MAYC `0x60e4…a7c6`, Pudgy
`0xbd35…2cf8`, BAYC `0xbc4c…f13d`), and golive-check compares them in lower case.

**D8 references (`monitor-nft.json`), all correct** (`d8-references.output.txt`):

| Collection | CoinGecko id | `contract_address` | OpenSea slug | Contract dominating the page |
| --- | --- | --- | --- | --- |
| Milady | `milady-maker` | `0x5af0…25a5` | `milady` | `0x5af0…25a5` |
| Azuki | `azuki` | `0xed5a…c544` | `azuki` | `0xed5a…c544` |
| MAYC | `mutant-ape-yacht-club` | `0x60e4…a7c6` | `mutant-ape-yacht-club` | `0x60e4…a7c6` |
| Pudgy | `pudgy-penguins` | `0xbd35…2cf8` | `pudgypenguins` | `0xbd35…2cf8` |
| BAYC | `bored-ape-yacht-club` | `0xbc4c…f13d` | `boredapeyachtclub` | `0xbc4c…f13d` |

CoinGecko's floors today: Milady 0.929, Azuki 0.674, MAYC 0.858, Pudgy 3.29 and **BAYC 6.3 ETH**. The docs use 6.79 for BAYC and
3.353 for Pudgy.

**Purchase limits against the advance gate** (item cap = min(1.03 × F, `maxPurchaseWei`); the gate needs cap × 0.995 − 0.0002 ≥
1.01 F + 0.003):

| Collection | Floor F | Item cap | What reaches Ethereum / what is needed | Largest floor that still buys |
| --- | --- | --- | --- | --- |
| BAYC, 8 ETH limit | 6.3 | 6.489 | 6.456 / 6.366 (+1.4 % slack) | ≈ 7.878 ETH (+25 %) |
| Pudgy, 4 ETH limit | 3.29 | 3.389 | — | ≈ 3.937 ETH (+20 %) |
| Milady, 2 ETH limit | 0.929 | — | — | ≈ 1.967 ETH (+112 %) |
| MAYC, 2 ETH limit | 0.858 | — | — | ≈ 1.967 ETH (+129 %) |
| Azuki, 2 ETH limit | 0.674 | — | — | ≈ 1.967 ETH (+192 %) |

- Above the last column the gate refuses: that collection never buys, and only the pool page (24 h) says so.
- A `maxPurchaseWei` under ≈ 1.015 × F makes a collection never buy. golive-check cannot see this before the first post: the
  NFT stack prints "no graduation price yet (no floor posted)".
- At convBps 300 the gate needs F ≥ ≈ 0.22 ETH. Every launch collection clears it.
- **Pooling at today's floors:**
  - BAYC at 9 ETH gives 0.88 units a graduation, and Pudgy at 5 ETH gives 0.91. Both need two graduations per item, as
    documented.
  - With the gate, a rise of +1.4 % between BAYC's two graduations already holds the pool.
  - After a +10 % rise, the pool needs **eight** graduations (six more than the two that pool one ape) before the gate opens.
    That is about 72 ETH of raises holding ≈ 5 apes' worth of ETH-backed units on 4663 (A3-I-2).

**Stage-0 custody caps (J7)** (`custodycap.rs:23-28`: 3 items and 10 ETH a collection, 6 items and 10 ETH global; H2 checks a buy
at its item cap, `ethflow.rs:1062`):

- **Buys.**
  - BAYC's 6.489 needs 65 % of the global value room.
  - BAYC and Pudgy together (9.878) leave 0.12 ETH for the other three collections.
  - If the three cheap collections hold ≥ 3.52 ETH, BAYC never fits.
- **Deposits** (D5 on) count **0 wei** (`ethflow.rs:1432`, `:2333`).
  - They cannot block by value.
  - Six cheap deposits fill the global item count, which stops every buy and every further deposit of all five collections
    (the buys are dropped before their advance; `eth-pool-idle` pages after 24 h).
  - The value caps do not bound deposited value at all (A3-L-4).
- **No loss for holders:** the pools stay ETH-backed and burnable.
- **In effect:**
  - a 10 ETH pilot in which BAYC and Pudgy rarely buy;
  - a 6-item pilot that any holder of six cheap NFTs can close;
  - a custody-value bound that deposits ignore.
- **Unsafe for launch as configured:** only in the A3-L-4 sense (the stated stage-0 exposure is not what holds).
- The operator's J7 decision should be taken before launch. golive-check passes all of it.

## 4. New findings

### Low

#### A3-L-1: the way back costs the holders a round trip each time, and nothing but the A2-M-1 gate bounds how often it repeats

- **Where:**
  - `src/floor/FloorModule.sol:735-758` (`returnAdvance`: a shortfall comes from revenue, then surplus, then the convertible
    units' `backing`);
  - `autobuyer/canister-rs/src/ethflow.rs:1137-1147` (`Arrived` → `back_now` after `ARRIVED_BACK_SECS`, 48 h);
  - `:1211-1223` (`back_now` keeps `ETH_GAS_RESERVE_WEI` on Ethereum);
  - `:1149-1158` (`Returning { min(rise, value) }`);
  - `:1036-1068` (`buy_pass`: the next buy of the same collection starts as soon as the returned one is finished; no cooldown,
    and no count of failed trips across buys);
  - `ethnet.rs:476-482` (`eth_give_back`: the operator sends an `Arrived` buy back at once);
  - docs `phase1-nft-pooled.md:194-198` and `phase1-nft.md:176-186`, `:612-617`, which say nothing of the cost.
- **Mechanism:**
  - A trip costs: the 4663 gas keep (0.0002 ETH, stays at the canister's 4663 address); Relay's fee on the forward leg; the
    Ethereum gas reserve (0.003 ETH, stays in the Ethereum gas pot); Relay's fee on the way back; and both legs' gas.
  - All of it is charged to the module at `returnAdvance`: revenue first (0 in a fresh pool), then surplus (0), then the units.
  - The module still "wants" the item, and the gate still passes while the per-unit backing covers `floor × 1.01` after the fee
    estimate. So the canister advances again, and the next trip starts.
- **Trigger:** no listing within the buy's bound for 48 h. This can happen in a thin book (MAYC had 2 listings within +5 %,
  D6), after a rise between advance and fill, or when a lazy or compromised runner withholds the fill listing. The operator's
  `eth_give_back` also triggers it, and then a trip takes minutes. The module's `maxItemsPerDay` (3) is the only rate bound.
- **What stops the loop:**
  - Only the A2-M-1 gate. It closes once the per-unit backing has fallen to about `1.015 × floor`, so about half of the
    conversion allowance is burnt.
  - Each later graduation brings new units at `1.03 × floor` and re-opens it.
  - At the judge's fee bound (`relay_max_fee_bps` 300 on each leg), one trip alone costs about 6 % of an ape and leaves the units
    backed **under** the floor.
- **Impact:**
  - A bounded loss per refill, charged to one collection's holders, silently: no doc states it, and no page fires before the
    first trip.
  - It is not an outsider's profit: the fees go to Relay's solvers and the reserve to the canister's gas pot.
  - Low: bounded by the gate at about 1.5 % of the pool's backing (plus one trip's overshoot), and it needs a condition or the
    operator.
- **PoC** (all PASS while present):
  - `AuditNftA3.t.sol` `test_A3_L1_repeatedWayBacks_eatTheBAYCPoolUntilTheGateStops_weakness`: BAYC at 9 ETH, F 6.3, two
    graduations, Relay at the measured 0.36 %.
    - Four trips run back to back before the gate refuses: 6.489 → 6.439, 6.461 → 6.411, 6.433 → 6.383, 6.404 → 6.355 ETH.
    - They cost 0.198 ETH in all, and every wei of it came from the pool.
    - The per-unit backing falls from 6.489 to 6.376.
    - Nothing is bought.
    - The next graduation re-opens the gate.
  - `test_A3_L1_oneWayBackAtTheJudgesFeeBound_weakness`: one trip, 6.489 → 6.102 ETH. The per-unit backing becomes 6.269, under
    the 6.3 floor.
  - `audit_nft_a3_canister.rs` `a3_m1_the_way_back_repeats_without_bound_each_round_trip_returned_short` (the canister's fake
    world):
    - four successive buys of the same collection, each advanced, arrived, sent back, and returned short by 0.00624 ETH (124 bps);
    - a **fifth** advance is already signed in the same ticks.
    - The fake's module does not model `maxItemsPerDay`; the real one allows 3 a day.
- **Recommended fix:**
  - After a buy of a collection comes back, hold that collection's next buy until the market condition has changed: a new
    floor post that differs, or a fresh listing within the bound. At least impose a cooldown (for example 24 h), and a count of
    consecutive way-backs that pauses the collection's buys and pages.
  - Refuse a way-back quote whose fee exceeds the measured band (for example 50 bps, not 300).
  - Document that the round trip is paid from revenue, then surplus, then the units, and have the Trust page say so.

#### A3-L-2: `--launch` does not hold a wrapper's name, symbol or slug; a rotated `ETCH_NFT_NAMES` / `_SYMBOLS` / `_SLUGS` list deploys "Wrapped Bored Ape / wBAYC" over another collection and passes the go/no-go read

- **Where:**
  - `script/DeployStack.s.sol:102-116`: four independent comma lists, and `checkNftSlugs` checks only format and uniqueness.
  - `script/golive-check.sh:522-530`: `launchcheck` reads only the six numbers.
  - `:729`: `wrapper.symbol` is compared with the **stack file**, written by the same deploy that read the lists (exactly the
    reasoning the fixer used for the targets, `:519-521`). The name is compared with nothing.
- **Impact:**
  - The wrapper's ERC-20 name and symbol are constructor-set, so they cannot be corrected after deploy.
  - The stack file's slug labels the collection for the site, the indexer and golive-check.
  - A rotation mislabels which NFT backs the token that users buy and hold (for example "wBAYC" backed by Miladys), and every
    check passes. Targets, purchase limits and the monitor are keyed by address, so the economics are right and the labels
    are wrong.
- **PoC:** `golive_harness.sh` case 1: the launch file's name, symbol and slug rotated by one. `launchcheck` still prints its 30
  `ok` lines and no FAIL, because it never reads them.
- **Fix:** have `launchcheck` compare `wrapper.name()`, `wrapper.symbol()` and the stack file's `slug` with the launch file's
  entry for that collection address. Better, have DeployStack take one list of `collection:slug` records.

#### A3-L-3: `--monitor-config` passes a monitor configuration whose D8 references are another collection's, whose guardian is not the registry's, or that still holds placeholders

- **Where:**
  - `script/golive-check.sh:531-548` (`moncheck`: each reference is only required to be non-empty with no `<`);
  - `:554-571` (`monglobal`: placeholders are looked for only in `evm.guardian` and `eth_nft`; providers are counted as distinct
    strings; `evm.guardian.address` is not compared with `registry.guardian()`, which `:277` reads; `evm.chain_id` and
    `evm.stack_file` are not read);
  - claim `fixes-round2.md` §6: "with no placeholder left and two Ethereum providers".
- **Impact:**
  - **References swapped between Milady and MAYC:** floors 0.93 / 0.86 ETH, within D8's 20 %. D8 then watches each collection
    against the other's market, silently, and loses its point for both. A BAYC ↔ MAYC swap would instead pause both posters at
    once.
  - **A guardian that is not the registry's:** caught only at run time (`chain.mjs:38`, SEV2). Until then no pause of D8 or of
    the no-good-post rule can be signed.
  - **Placeholders:** `<p1>`, `<g1>` (the timelock's expected proposers and guardians) and `<runner host>` (liveness) pass both
    golive-check and the monitor's own `validate`, which was checked by running it. They produce standing false alarms, or no
    liveness watch.
  - **One provider twice:** `eth.drpc.org` and `eth.drpc.org/` pass as "two providers".
- **PoC:** `golive_harness.sh` cases 2 and 3-5: no FAIL line for any of them. The control (no `pause_on_no_good_post_h`) FAILs.
- **Fix:**
  - Keep each collection's OpenSea slug and CoinGecko id in `nft-pilot.json`, and hold the monitor configuration to them by
    address.
  - Compare `evm.guardian.address` with `registry.guardian()`.
  - Refuse any `<` anywhere in the file.
  - Compare providers by host.
  - Hold `evm.chain_id == 4663` and `evm.stack_file` against the stack file given.

#### A3-L-4: deposits enter custody at 0 wei, so with BAYC and Pudgy in the launch, stage 0's 10 ETH value caps no longer bound what custody holds (C-I-2 sharpened)

- **Where:** `autobuyer/canister-rs/src/ethflow.rs:1432` (`check_acquire(…, 0)` for a deposit), `:2333` (`admit(…, 0)`),
  `custodycap.rs:206-227`. Round 1's C-I-2 called this "not binding at stage 0 for D1's collections", when D1 was Milady, Azuki
  and MAYC.
- **Impact:**
  - Stage 0's custody cap (launch-gates G7: 10 ETH a collection and 10 ETH in all) is the bound on what a flaw in custody or
    delivery could lose.
  - With deposits on, three BAYC (≈ 18.9 ETH) and three Pudgy (≈ 9.9 ETH) can be deposited within the per-collection item caps.
    That is ≈ 29 ETH in custody, about 3 × the stated bound, and the value caps never see it.
  - Purchases, meanwhile, are held to the 10 ETH.
  - Low: no path to a loss by itself. What changes is the stage-0 exposure the launch states.
- **PoC:** by reading. `check_acquire`'s value tests add `cost_wei` = 0 for a deposit, and the book entries of deposits carry 0.
- **Fix:** admit a deposit at the module's accepted floor (the value the wrapper mints a unit against), or state in the launch
  decision that deposits are bounded only by item counts and the wrapper's `maxMintsPerDay`.

### Info

- **A3-I-1. `Pooled.t.sol`'s floor-rise test has a dead branch, and its gate is a mirror.**
  - After the +10 % rise, `test_aFloorRiseWhilePooled_…` (`:248-271`) ends with `if (_canisterAdvances(bayc, max3)) {
    advance … }`.
  - At the third graduation the gate still refuses (maxItemCost 7.4449 ETH, and the gate needs ≈ 7.5848), so the body never
    runs. A trace shows no `advance` call in that test.
  - The test's comment ("once it covers the floor the canister advances it") is therefore asserted nowhere.
  - PoC: `test_A3_I1_pooledFloorRiseTest_theThirdGraduationBranchIsDead` (PASS: refused at the third graduation, converts at the
    eighth).
  - Make the assertion unconditional, at the graduation count where it holds, and assert the refusal before it.
  - The suite still uses `initMintLimit(1_000)`, not stage 0's 3 (round 2's note, repeated).
- **A3-I-2. The gate's recovery after a rise is slow, and the doc understates it.**
  - `phase1-nft-pooled.md:187-189` ("it converts once a later graduation lifts the blend").
  - After +10 %, BAYC needs six more graduations (≈ 54 ETH of raises) before the average backing crosses `≈ 1.015 × floor`
    after fees.
  - Meanwhile ≈ 4–5 apes' worth of ETH-backed units wait on 4663 (burnable).
  - This is not unsafe, but the Trust page and J1 should say it: it strengthens the case for J1's alternative, one graduation
    per ape.
- **A3-I-3. golive-check does not hold the canister's per-item or global custody caps.**
  - `capcheck` (`golive-check.sh:440-449`) compares only per-collection `max_items` and `max_value_wei` with the stage.
  - `max_item_wei` is not held. A value copied from a cheap collection (for example 2 ETH) makes BAYC and Pudgy refuse every buy
    (`ItemOverMax`) and passes.
  - `global_max_items` / `global_max_value_wei` are only printed.
  - `custodycap.rs:24` still says "an ape is ≈ 5.1 ETH all-in": BAYC's floor is 6.3 and its item cap 6.49.
  - Hold `max_item_wei ≥ max_purchase_wei` per collection, and hold the global caps against a stage value in the stack file.

## 5. Tests run (part A, private copies)

| Run | Result |
| --- | --- |
| `git diff 79455e1..HEAD -- src/` | empty. `test/`: only `test/ethnft/Pooled.t.sol` changed |
| Round 2's `poc/A/AuditNftA2.t.sol`, unmodified | **8 pass**, the same as the fixer's `contracts-poc-a-rerun.output.txt` (honest). A2-M-1's two PoCs still pass on chain, which is expected (the module is unchanged; the gate is canister-side) |
| Round 1's `poc/A/`, unmodified | 9 pass, 4 fail (`NotSupported()`: A-M-2 closed), identical to round 2. `test_A_L2_…` still passes (A-L-2 partly) |
| `test/ethnft/Pooled.t.sol` `test_aFloorRiseWhilePooled_…`, `test_A2_M1_…` with `-vvvv` | both pass; no `advance` in the floor-rise test (A3-I-1) |
| `poc/A/AuditNftA3.t.sol` | **4 pass**: A3-L-1 ×2 (weakness), the control, A3-I-1 |
| `poc/A/audit_nft_a3_canister.rs` (in a copy of `tests/audit_nft_r2.rs`) | **1 pass** (the weakness) |
| `poc/A/golive_harness.sh` | control clean; cases 1, 2 and 3-5 pass without a FAIL (weakness); case 6 (no `pause_on_no_good_post_h`) FAILs as it should |
| The monitor's `validate` on the template with only golive-check's fields filled | accepts `expect.*` and `liveness` placeholders; refuses `timelock`, `buyer` and `install_arg_hex` placeholders |
| The D8 references (CoinGecko API, opensea.io pages) | all five correct |
| `scripts/golive-dryrun.sh stack` | not run here; the lead's `bar/dryrun.txt` (partial, 16:56): the NFT section's positive run and all eight refusals `ok`, no FAIL line so far |
