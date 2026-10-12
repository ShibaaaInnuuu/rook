# v8: a real-money rehearsal on mainnet before the public launch — runbook (design)

Status: **a runbook written before it is run**, 2026-09-27, branch `ideas-v8` (from `nft-side` `a37f54b`). Nothing here has been
run. No key was used. **The operator runs every signing step with their own wallet, Safe or HSM identity; this runbook never asks
for a private key, a seed phrase or an API key's value**, and no step requires pasting one anywhere but the service's own
environment on the operator's machine.

**Legend.** **[R]** this repository. **[M]** measured (dates given). **[A]** estimate.

---

## 0. Summary

- **What:** one Azuki graduation, end to end, on the real chains with real money, before anyone else's money is in: a coin
  launched and bought to graduation on 4663 → the advance → Relay to Ethereum → a real Seaport listing bought by the canister →
  finality → `EthNftWrapper.mint` → `settleAdvance` → a redemption by the operator → the delivery to the operator's own Ethereum
  address. Every step has its on-chain evidence and a pass/fail check (§3).
- **The design choice: a throwaway rehearsal deployment, built from the release commit, with one Azuki collection at a 1 ETH
  target** (§1). Same `buyer.wasm` and `timelock.wasm` hashes, same `key_1`, same providers, same services, same caps; only the
  4663 stack is separate (its own wrapper, module and factory at small targets) and so is the canister (a different canister id,
  so a different custody address). The production install happens after it, from the same commit.
- **Main risk:** a stuck state with the operator's ETH in it. Every stuck state has a written exit (§4) — which is why the
  round-3 fixes that add exits (C3-M-1 for the way back, E3-L-2 / E3-L-3) are **prerequisites**, not follow-ups.
- **Cost:** ≈ 1.9 ETH of outlay, of which ≈ 1.5–1.6 ETH comes back (an Azuki and the coin's value); **net ≈ 0.3–0.4 ETH plus
  ≈ 25–50 T cycles** (§5).
- **Time:** ≈ 5–7 days wall-clock, most of it the 72 h timelock handover and waiting on Ethereum finality and a drand round.

---

## 1. Why a separate deployment, and why Azuki

- **Why not the production stack itself:** a fresh Azuki factory's first coin is an **early** coin: the target is `earlyRaise`
  (8 ETH) while the collection's shared pool has taken in less than `earlyUntilEth` (2 ETH), and the "opening" rule only ever raises
  it [R, `CurveFactory.raiseFor`]. The operator's stake would be 8 ETH, and factory targets are immutable, so a smaller target means
  a separate stack.
- **The rehearsal stack:** NFT-only, **Azuki alone**, `normalRaise = earlyRaise = 1 ETH` (the `StackChecks` minimum), `convBps` 300,
  **`reserveBps` 0**. At a 0.674 ETH floor the opening target is `(cap × 1.05 + 0.25) ÷ 0.874` ≈ **1.12 ETH**; the item budget
  ≈ 1.12 − 0.056 (5 % platform fee) − 0.141 (12.6 % shared pool) = **0.923 ETH** → 0.923 ÷ (0.674 × 1.03) ≈ **1.33 units**, above
  the 1.05 buy trigger, so one graduation buys one Azuki. With the default 25 % reserve it would convert 0.997 units and pool: that
  is why the reserve is 0 here (the opening-target flaw, `phase1-nft-pooled.md` §1).
- **Why Azuki:** the deepest listings (14 within +5 %, D6), so the ask path has something to buy; the cheapest item.
- **What it does not rehearse:** the five-collection concurrency, the pooled BAYC path, deposits at scale, and the production 8 ETH
  targets. A second, optional rehearsal item covers bids and the way back (§6).

---

## 2. Prerequisites (each a pass/fail check before step 1)

| # | Prerequisite | Evidence | Pass |
|---|---|---|---|
| P1 | The round-3 fixes (D3-H-1, C3-H-1, E3-M-1, B3-M-1, C3-M-1) and any round-4 fixes merged on the release commit; the round-4 review's verdict "no Critical, High or Medium open" | the review's README | yes |
| P2 | The reproducible build of the release commit, twice, and by a second person | `autobuyer/Dockerfile.build` output | the three hashes match each other and the published ones |
| P3 | `scripts/golive-dryrun.sh stack` on the release commit, **with the rehearsal launch file** (Azuki, 1 / 1 ETH, 300 / 0) | the dry run's log | `DRY RUN: ALL PASS` |
| P4 | **The Safe** that owns the rehearsal's 4663 contracts (the G2 solo variant is acceptable for a rehearsal) | the Safe's address and signers | deployed on 4663; a test transaction executed |
| P5 | **The IC identities:** a throwaway deployer; ≥ 2 proposers and ≥ 3 guardians, HSM-backed (`docs/deploy/key-ceremony.md`) | `dfx identity list` on each machine | each principal written down; none in two roles |
| P6 | **The providers:** the canister's keyless pairs (4663: `rpc.mainnet.chain.robinhood.com`, `rpc.ordofi.network`; Ethereum: `xrpc.cl/eth` and the second keyless provider chosen in round 3/4), re-measured the same day (state depth, `finalized`, 500-block logs) | the measurement log | every number within the bounds `canister-mainnet.md` "Blocked today" 2 states |
| P7 | **The services' secrets in their environments only:** the monitor's `DWELLIR_API_KEY`, the runner's `OPENSEA_API_KEY` and `ETH_LOGS_RPC` | each service's start log shows the key is set (never its value) | set; `redact` masks it in a test alert |
| P8 | **The monitor and the guardian:** D8 armed (`guardian_pause = true`), every guardian in the alert channel | a test alert | received by every guardian |
| P9 | **Caps:** stage 0 in the install argument (Azuki 15 ETH / 22 items, H3 25 ETH/day, outflow 12 / 24 a day) | the argument, decoded by the monitor's `argdiff` | matches `docs/v5/launch-gates.md` G7's stage-0 row |
| P10 | **Budget:** ≈ 2 ETH on 4663 in the operator's wallet; ≈ 0.02 ETH on Ethereum; ≈ 50 T cycles | balances | enough |
| P11 | **Market:** Azuki's floor and depth the day of the run (OpenSea best listings): ≥ 5 listings within +5 % | the runner's depth print | yes, else wait |

---

## 3. The run, step by step

Each step: who signs (**O** = operator's own wallet, **S** = Safe, **D** = deployer identity, **P/G** = proposer / guardian HSM
identity, **C** = the canister, **K** = anyone / keyless), the expected on-chain evidence, and the pass check. **Abort** marks
the points where stopping is clean (§4).

### Stage A: install

| # | Step | Who | Evidence | Pass |
|---|---|---|---|---|
| A1 | Create `buyer` and `timelock` canisters with 2 T each (subnet as the operator chose: 13-node or fiduciary) | D | canister ids | created |
| A2 | Install `buyer.wasm` (the P2 hash) with `key_1`, Bitcoin mainnet inert, 4663 and Ethereum chains, `eth_nft` = Azuki only (the rehearsal stack's wrapper and module addresses are filled after A5; install first with the collection absent, as `canister-mainnet.md` describes for the placeholder-then-reinstall sequence) | D | `dfx canister info buyer` module hash | = P2's hash |
| A3 | Read the canister's EVM address (`evm_address()`), write it down twice | K | the address | two readers agree |
| A4 | Raise the freezing threshold to 90 days; top up the buyer to 20 T | D | `cycles()` | ≥ 20 T |
| A5 | Deploy the rehearsal stack on 4663 (`ETCH_STACK_KIND=nft`, the rehearsal launch file, `ETCH_NFT_POSTER` = A3's address) | O (the deployer key or keystore) | `deployments/…json`; `EthNftWrapper.custodian()` = A3 | `script/golive-check.sh --phase1` passes |
| A6 | The Safe wires the registry and roles (contracts runbook step 2) | S | the Safe transactions | executed |
| A7 | Reinstall the buyer with the complete `eth_nft` (A5's wrapper and module) | D | `eth_nft_config()` | names exactly A5's addresses; deposits as the operator chose (§7 M3) |
| A8 | `golive-check.sh --phase1 --canister --launch --monitor-config` | K | its output | every check ok |
| A9 | Install the timelock (`timelock.wasm`, P2 hash, `delay_secs` = 259,200), hand the buyer over, blackhole the timelock | D | `status()`; `dfx canister info buyer` controllers = [timelock]; timelock controllers = [] | as stated; **the deployer identity is then destroyed** |
| A10 | Start the services (`services-v1.md` §5 order): monitor, keeper (resolver), runner, reconciler, indexer | O | each service's health endpoint | all green; the monitor reads the canister and the module |

**Abort** anywhere in stage A: nothing holds value yet (cycles only).

### Stage B: the floor, the gas pot, a guardian drill

| # | Step | Who | Evidence | Pass |
|---|---|---|---|---|
| B1 | Seed the gas pot: send 0.01 ETH on Ethereum to A3's address (D3) | O | the transfer | balance ≥ 0.01 |
| B2 | The canister's first floor post on the Azuki module | C | `FloorPosted` from the poster; `module.floor()` fresh | within 5 % of OpenSea's floor; confidence ≥ 7,000; the monitor quiet |
| B3 | **Guardian drill:** one guardian pauses the buyer; watch one tick sign nothing; k guardians resume | G | the pause and resume in `history()` | nothing signed while paused; resumed (G10) |

**Abort:** clean.

### Stage C: one graduation

| # | Step | Who | Evidence | Pass |
|---|---|---|---|---|
| C1 | Read `factory.raiseFor(false)` | K | `(raise, early, opening)` | opening, raise ≈ 1.12 ETH (≤ 1.3) |
| C2 | Launch the rehearsal coin (`launch(name, symbol, BURN, false)`), policy BURN, the default fee tier | O | `Launched(coin, curve, creator = O, …, raise)` | raise as C1 |
| C3 | **Wait until `curve.launchExtraBps() == 0`** (the launch fee starts at 50 % and decays over 10 minutes, `policies-and-fees.md` §2); buying earlier gives up to half the ETH to the sweep vault | K | the view | 0 |
| C4 | Buy to the raise in one or two buys (`buy(minOut, deadline)`); the closing buy is filled to the raise and the rest returned | O | `Bought`; `GraduationStarted(…, itemBudget)`; the queue's `enqueue` | `itemBudget` ≈ 0.92 ETH |
| C5 | Graduate (keyless `graduate()` once the queue fills the coin) | K | the module's `netUnits()` | ≈ 1.33 units |

**Abort:** before C4's closing buy, sell the coins back (`sell`); after C4, the curve's own refund window (A-03) applies if it does not
graduate; after C5 the ETH is ETH-backed wX at the module, burnable at any time.

### Stage D: the canister buys the Azuki

| # | Step | Who | Evidence | Pass |
|---|---|---|---|---|
| D1 | `NeedAdvance` → `advance(amount)` on the module | C | `Advanced(amount, released)` on 4663; `transitUnits = 1` | `amount` = the canister's `advance_bound` (the floor + 5 %, fees, gas), ≤ `maxItemCost()` |
| D2 | The runner quotes Relay 4663 → Ethereum; the canister judges (order hash, recipient us, min-out ≥ 97 %) and deposits | C | `depositNative(us, orderId)` on 4663; the order id in `eth_nft_state()` | the judge's record matches Relay's quote |
| D3 | Relay fills | — | our Ethereum balance up by ≥ `minimumAmount` | `Arrived` within 30 min; fee ≤ 3 % (measured 0.31–2.3 %, `phase1-nft.md` §8) |
| D4 | The runner hands in OpenSea's best Azuki listings; the canister judges and signs `fulfillBasicOrder` / `fulfillAdvancedOrder` | C | the Ethereum transaction; `OrderFulfilled`; `Transfer(seller, us, tokenId)` | price ≤ min(floor × 1.05, arrived − 0.003 ETH); fees = OpenSea's 1 % only |
| D5 | Finality: the fill's block ≤ the lower finalized block; `ownerOf = us` on both providers | C | the canister's buy record `Final` | within ≈ 15–35 min of the fill |
| D6 | `EthNftWrapper.mint(tokenId, fillTx, module)` then `settleAdvance(cost)` | C | `Wrapped(itemId, tokenId, fillTx, module)`; `AdvanceSettled(item, advanced, cost)` | `cost = ⌈advanced × paid ÷ arrived⌉`; `transitUnits = 0` |
| D7 | The reserves page and the reconciler | K | `/v1/reserves/azuki` | **green**: 1 item, `ownerOf` = custodian on two providers, the equation exact |

**Abort / unwind** at each state: §4.

### Stage E: redemption and delivery

| # | Step | Who | Evidence | Pass |
|---|---|---|---|---|
| E1 | Get one whole wX: sell rehearsal coins into the coin's pool, or `EtchRouter.buyWrapped` | O | the swap | balance ≥ 1 wX |
| E2 | `redeem(operatorEthAddress)` (a fresh address the operator controls on Ethereum, not the custodian) | O | `Redeemed(id, O, round, destination)` | round = `roundAt(t) + 20` |
| E3 | `resolve(id, drandSignature)` (the keyless resolver, or the operator by hand) | K | `Resolved(id, itemId, tokenId, deliverBy)` | the only item |
| E4 | The canister's delivery: `transferFrom(us, destination, tokenId)` | C | the Ethereum transaction; `ownerOf = destination` | mined within 1 h of E3 |
| E5 | Finality, then `recordDelivery(id, txHash)` | C | `Delivered(id, tokenId, destination, txHash)` | judged by the final receipt (the D3-H-1 fix) |
| E6 | The reserves page | K | `/v1/reserves/azuki` | green: 0 items, 0 tickets, the equation exact |
| E7 | The draw proof (`redemption-draw.md` §2.3), recomputed by the verifier | K | the verifier's output | the index names the delivered item |

### Stage F: close out

| # | Step | Who | Evidence | Pass |
|---|---|---|---|---|
| F1 | The monitor's log for the whole run | O | the alert history | every alert explained; no SEV1 other than drills |
| F2 | The cycles used, the gas used, Relay's fee, the price vs the floor | O | the numbers | recorded in the run log (§5 filled in with actuals) |
| F3 | Retire the rehearsal: the canister holds nothing; a timelock proposal to stop the buyer (or leave it to run down its cycles) | P | the proposal; `status()` | executed after 72 h (or the operator's choice, §7 M5) |
| F4 | Write the run log into `docs/audit/` beside the reviews | O | the commit | — |

---

## 4. Abort points and how to unwind (the return paths)

| State | What is at risk | Exit | Who |
|---|---|---|---|
| Curve trading (before C4 closes) | the operator's ETH in coins | `sell` back to the curve | O |
| Graduating, not filled | the raise | the curve's refund window (3 days, A-03; the escape after `window + STUCK_GRACE`) | K |
| Module holds the ETH (after C5) | nothing leaves 4663 | burn wX at the module (≈ the floor − spread); or wait | O |
| `NeedAdvance` / `Funded` (ETH at the canister's 4663 address) | one advance | `eth_give_back` → `returnAdvance` (operator identity) | O via the runner's operator identity |
| `Bridging` (Relay has it) | one advance | Relay refunds on 4663 → returned; past the deadline + 1 h → `Stuck` → `eth_give_back` | O |
| `Arrived` (ETH on Ethereum, no listing within the bound) | one advance | the automatic way back after 48 h (D4), or `eth_give_back` at once → Relay Ethereum → 4663 → `returnAdvance`. **Requires C3-M-1's exits** for a way back that cannot be quoted (P1) | O |
| Fill mined, before finality | the NFT is ours or the fill reorgs away (the ETH is still ours) | wait; the canister re-reads | — |
| Minted, before settle | nothing | the canister retries `settleAdvance` | C |
| Ticket resolved, delivery failing | the redeemer's unit | the canister's cancel → `markUndeliverable` → `refund` after 72 h grace (the unit comes back) | C, K |
| Anything unexpected | — | **any guardian pauses** the buyer (G10); nothing is signed until k guardians resume | G |

Every exit keeps "every unit 1:1 backed": the advance returns to its module (less Relay's fees, charged to revenue, surplus, then
backing, A3-L-1), a refund mints a unit back only when no delivery can confirm.

---

## 5. Costs [A, at the measured prices of 2026-09-27]

| Item | Amount | Back? |
|---|---|---|
| The curve: raise ≈ 1.12 ETH + 1 % trade fee | ≈ 1.13 ETH | as coins; the creator's 30 % of the fee comes back to the operator as creator |
| — platform fee (5 %, capped 0.25) | ≈ 0.056 ETH | to the platform treasury (the operator's own entity) |
| — shared pool's ETH side (12.6 %) | ≈ 0.141 ETH | locked as liquidity forever; partly recoverable by trading against it |
| One wX for the redemption (E1) | ≈ 0.70 ETH | the Azuki itself (≈ 0.674 ETH at the floor) |
| Relay 4663 → Ethereum on ≈ 0.72 ETH | ≈ 0.002–0.017 ETH (0.31–2.3 %) | no |
| Ethereum gas: the fill (≈ 184k gas), the delivery, gas at 0.07–1.5 gwei | < 0.001 ETH | no |
| The gas pot seed | 0.01 ETH | stays at the rehearsal address (swept only above 0.05) |
| 4663 gas: the stack's deploy and the operator's transactions | small (the dry run's gas report gives the number) | no |
| ICP cycles: 2 canisters created (1 T), install, ≈ 2–5 T a day for ≈ 7 days, ≈ 10 signatures × 26 B | ≈ 25–50 T (≈ 25–50 XDR) | no |
| **Outlay** | **≈ 1.9 ETH + cycles** | |
| **Net cost** | **≈ 0.3–0.4 ETH + ≈ 25–50 T cycles** | |

## 6. Optional second item: bids and the way back

After `bids.md` is built and reviewed: a second 1 ETH graduation on the same rehearsal stack with bids on (2 h window, one step at
−3 %), expected to fall back to the ask path (a test of the death proof, §8 there); and, by withholding the listing hand-in for the
run, `eth_give_back` exercising the way back Ethereum → 4663 → `returnAdvance`. ≈ 1.2 ETH outlay, ≈ 0.15 ETH net.

## 7. Trust, invariants and open decisions

- **Trust and security:** the rehearsal uses the production code, keys and timelock; its new exposure is the operator's own stake,
  bounded by stage-0 caps. Its value is that the first real Relay leg, Seaport fill, Ethereum finality read and delivery happen with
  no one else's money (`relay-conversion.md` §0.1: "the first real conversion is the first test"). What can go wrong is listed in
  §4; nothing in the runbook widens what the canister may sign.
- **Invariants:** per-collection isolation is trivially held (one collection); the caps are stage 0; the timelock is real (72 h) and
  blackholed; 1:1 backing is checked by the reserves page at D7 and E6.
- **Open decisions for the operator:**
  - **M1.** Separate rehearsal deployment (recommended) vs rehearsing on the production stack at its 8 ETH early target.
  - **M2.** The subnet (13-node vs fiduciary) — the same choice as production, so decide it once.
  - **M3.** Deposits on or off in the rehearsal (recommended: on, and deposit one of the operator's own NFTs if they hold one; else
    off).
  - **M4.** Run the optional second item (§6) before the public launch (recommended once bids exist).
  - **M5.** Retire the rehearsal canister by a timelock proposal, or leave it to run out of cycles.
  - **M6.** Who is the second person reproducing the build (P2).

## 8. Build and review estimate [A]

- Writing the rehearsal launch file, a `golive-dryrun.sh` variant for it and a checklist script that prints each step's evidence
  (read-only): ≈ 1–2 days.
- The run itself: ≈ 5–7 days wall-clock, ≈ 1–2 days of the operator's attention.
- Review: a read of the run log by the reviewers of round 4 (≈ half a day).
