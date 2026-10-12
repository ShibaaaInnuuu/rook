# The operator's decisions of 2026-09-27 on the Ethereum NFT side, as built (branch `nft-bayc10`)

Branch `nft-bayc10`, from `nft-side` `4949da0` (rebased onto `55d8d32`, the round-3 review's documents, so the merge is a fast-forward).
Nothing was deployed, pushed or signed with a real key; the only chains written to were local anvils. The only network reads were
read-only JSON-RPC probes of public Ethereum providers (§5) and the reproducible build's crate fetch.

The round-3 review (`review3/`) was read. Its findings are left to the round-3 fixer, who works on this branch's code once it lands;
§7 says what this branch changes for two of them (C3-H-1, A3-L-4) and what it hands on.

## 1. J1: BAYC at 10 ETH, `convBps` 1,000, `reserveBps` 0

**Decided:** BAYC's graduation target is 10 ETH (normal = early; more needs a `CurveFactory` change). "Find a percentage above to
use": the conversion and reserve that maximise the floor rise from today's 6.79 ETH that still lets **one** graduation buy **one**
ape, within `StackChecks` (conv 300–1,600, reserve ≤ 2,500), without making the holders' ETH side unsafe.

**The math** (`docs/v7/phase1-nft-pooled.md` §1.1, every step read from the code): item budget 8.49 ETH → convertible units
(1 − reserve) × 8.49 ÷ (F × (1 + conv)) → a buy is due at 1.05 units → the advance → the canister's gate (Relay 0.5 %, the floor
+ 1 %, 0.003 ETH) → the buy (floor × 1.05, what arrived − 0.003, `max_purchase_wei`). Two edges: the floor at graduation above which
one graduation is under 1.05 units (pooling), and the rise after a graduation at 6.79 past which the gate refuses. A reserve only
lowers the first; the conversion trades one against the other; they meet at conv ≈ 997.

**Chosen:** conv **1,000**, reserve **0**. One graduation = **1.137 units**. It pools above a **7.3506 ETH** floor at graduation
(**+8.26 %**); a graduation at 6.79 is still advanced and bought up to **7.3549** (**+8.32 %**). So **+8.26 %** of floor rise from
6.79, before, after or split around the graduation, still buys one ape with one graduation. Both edges are reproduced on the
contracts (`Pooled.t.sol`: the gate passes at 7.3548 and refuses at 7.3550; 7.3505 gives 1.05002 units, 7.3507 gives 1.04999).

| BAYC at 10 ETH | Units / graduation at 6.79 | Pools above | Still advanced up to | Headroom |
|---|---|---|---|---|
| defaults (300 / 2,500) | 0.91 | 5.888 (pooled today) | 6.887 (+1.42 %) | pooled |
| 700 / 500 (best with a 5 % reserve) | 1.11 | 7.179 (+5.73 %) | 7.154 (+5.36 %) | +5.4 % |
| **1,000 / 0 (decided)** | **1.137** | **7.3506 (+8.26 %)** | **7.3549 (+8.32 %)** | **+8.26 %** |
| 1,100 / 0 (the earlier recommendation) | 1.127 | 7.284 (+7.28 %) | 7.422 (+9.30 %) | +7.28 % |
| 1,600 / 0 (the bound) | 1.078 | 6.970 (+2.66 %) | 7.756 (+14.2 %) | +2.66 % |

**Every collection at its current target** (floors of 2026-09-27; conv 300 / reserve 2,500 unless named):

| Collection (floor) | Target early / normal | Items / graduation early / normal | Pools above (early / normal) | Still advanced after a graduation | One-graduation headroom |
|---|---|---|---|---|---|
| Milady (0.930) | 8 / 5 | 5.28 / 3.23 | 4.676 / 2.857 | +1.13 % | +1.13 % |
| Azuki (0.670) | 8 / 5 | 7.33 / 4.48 | 4.676 / 2.857 | +1.00 % | +1.00 % |
| MAYC (0.8735) | 8 / 5 | 5.62 / 3.43 | 4.676 / 2.857 | +1.11 % | +1.11 % |
| Pudgy (3.353) | 8 / 5 | 1.46 / 0.89 | 4.676 (+39 %) / 2.857 (pooled today) | +1.38 % | early +1.38 %; normal pooled |
| **BAYC (6.79), 1,000 / 0** | 10 / 10 | **1.137** | 7.3506 (+8.26 %) | **+8.32 %** | **+8.26 %** |

The four at conv 300 have about 1 % of rise between a graduation and its buys (the gate needs the floor + 1.5 % and the fixed
0.0032 ETH); past it their pools wait on 4663, ETH-backed, until later graduations or the fee slices lift them. Not changed: their
own `convBps` is the operator's call.

**Why a zero reserve is safe for the ETH side:** every ETH-backed unit is still 1:1 backed and burns pay at most their tier's backing;
nothing in the module needs a reserve; a returned advance's loss is covered by revenue and surplus first and then shared by all the
ETH-backed units. What it gives up is a standing ETH pot for exits (after an ape, ≈ 0.14 units per graduation stay ETH-backed).
**What the 10 % costs:** a graduation's wX is priced at 1.10 × the floor (a burn pays about the floor); A2-L-1's leftover is now
closed (§3). The settlement records a fill of cost ÷ 1.10, so BAYC's fills sit at the fill band's lower edge (0.97 × the floor);
at conv 300 they sat near it already (Info, for the review).

**Changes:** `deployments/launch/nft-pilot.json` (BAYC 10 / 10 ETH, 1,000, 0; `_baycAlternative` removed; the purchase limit stays
8 ETH: it covers the item cap 7.47 and a buy at the edge, 7.72); `scripts/golive-dryrun.sh` (passes the conversion and reserve lists
from the launch file, which it did not before; reads BAYC's factory and module back; expects 1.14 items per graduation for BAYC and
the pooled note for Pudgy; the shifted-list refusal moves all four of BAYC's values); `script/golive-check.sh` (comments);
`test/ethnft/Pooled.t.sol` (three J1 tests); `test/StackChecks.t.sol`; `docs/v7/phase1-nft-pooled.md` (J1 decided, §1.1, §8).

## 2. D4: the automatic return after 48 h — confirmed

Recorded in `docs/v7/phase1-nft.md` §9 D4 (dated 2026-09-27): ETH stuck on Ethereum goes back automatically after 48 h `Arrived`
through the reverse Relay leg built in round 2; `eth_give_back` still does it at once; the 24 h page is kept. No code change.

## 3. A2-L-1 closed: the advance is the buy's bound, and the leftover goes back to its own module

1. **The advance** (`ethflow::advance_bound`): the floor + the premium (our fresh reading, never above the module's accepted floor),
   within `max_purchase_wei`, plus the 0.003 ETH gas reserve, grossed up for the Relay fee the judge accepts (`relay_max_fee_bps`,
   3 %, rather than the 0.5 % estimate, so a judged quote at its worst still leaves the buy's most: review round 3 C3-L-1), plus the
   0.0002 ETH gas keep, never above the item cap. The round-2 gate still judges the item cap against the module's floor, and the
   amount advanced must reach Ethereum covering the buy's floor + 1 %. **The module is unchanged:** `advance(amount)` already takes
   any amount up to `maxItemCost`; the unit releases its whole backing and what was not advanced is booked as that module's revenue.
   BAYC at 6.79: **7.353 ETH advanced, not 7.469** (0.116 ETH never leaves). The conv-300 collections' item caps are under their
   bound, so their advances are unchanged. The headroom table is unchanged (the bound only binds where the gate passes anyway).
2. **The gas-pot sweep** (`ethflow::sweep_pass`): a fill's leftover (what arrived less the price and the fill's gas) and a way back's
   kept reserve (less its gas) are booked to their collection (`EthState.leftover`). When nothing at our Ethereum address is
   anyone's (no buy holding or awaiting ETH, nothing on the line, no delivery open, no stray going back) and the balance is over the
   target (`eth_nft.gas_pot_target_wei`, 0.05 ETH, within [0.01, 1] ETH; D3's 0.01 seed and 0.005 page unchanged), the excess, up
   to the largest collection's leftover (at least 0.02 ETH), goes back: a `sweep` record takes the one Relay leg through the way
   back's own phases (`BackFunded` → `BackSending` → `BackBridging` → `Returning` → `Returned`, the quote judged by
   `relayeth::judge` `FROM_ETHEREUM`), then `FloorModule.fundReserve()` on **that collection's** module: its surplus, which tops up
   its conversions and never pays a burn. A sweep never becomes an arrival (a refund ends it and books the leftover back); one
   with no quote for 2 h is abandoned (it holds new buys back while it waits), then a 24 h cooldown; `eth_give_back` refuses it.
   BAYC's leftover per ape is ≈ 0.54 ETH at a listing at the floor, ≈ 0.20 at the floor + 5 %.

**Tests.** Canister (`tests/audit_nft_r2.rs`, `a2_l_1_*`): the bound's arithmetic at BAYC's numbers; the canister advances only
the bound and reserves custody for it; two collections' leftovers each go to their own module (Milady's to Milady's, then Azuki's
to Azuki's), `fundReserve` with no advance open, the kept reserve credited; the sweep never spends an arrived buy's ETH, never
while a delivery is open, only the balance over the target, nothing under the minimum; an unquoted sweep is abandoned and a refunded
one ends, both booking the leftover back, never `Arrived`; a fill's leftover is its collection's. Forge (`Pooled.t.sol`,
`test_A2L1_*`): a partial advance keeps the rest in the module as revenue, the ape settles, and a return restores everything but
Relay's fees; a swept leftover is that module's surplus only, and it lets BAYC buy after a rise past the headroom. Monitor: a
change of the target is worded; a sweep is never read as an advance.

## 4. J7 and H3: 15 ETH a collection with no binding total, 25 ETH a day

- **Custody (J7):** an Ethereum collection's stage-0 cap is 15 ETH, one item up to 15 ETH, and its item count what fits 15 ETH at
  its floor: Milady 16, Azuki 22, MAYC 17, Pudgy 4, BAYC 2 (`nft-pilot.json` `custodyItems`, `custodyWei`). **No canister-wide
  total:** the code needs a global cap (a collection's cap may not exceed it), so the global caps are the collections' sum (61
  items, 75 ETH, never binding before theirs). In the canister: from nothing, an Ethereum collection may start at up to 24 items
  and 15 ETH (`FIRST_EVM_*`) and the global at stage 0's or the collections' sum when more (`first_global`); `custody_caps = null`
  gives each Ethereum collection 6 items and 15 ETH and the sum in all. The Bitcoin wrapper's inscriptions keep 3 items and 10 ETH.
  Upgrades keep the ×4 rule.
- **Deposits count at their floor** (review round 3 A3-L-4): a deposit is checked and admitted at the module's accepted floor, not
  0 wei, so the 15 ETH bounds deposited value too.
- **The outflow consequence:** every item must be able to leave within the timelock's 72 h less a day (`outflow::exit_fits`, which
  the canister enforces at install), so 22 Azukis need 11 deliveries a day: the NFT launch install names `outflow_limits` of 12 a
  day per collection and 24 in all (×4 stage 0, the most an install may start at), mints 3 / 6, value 25 ETH, trip factor 3.
- **H3 (the operator):** stage 0's value a day is **25 ETH** (`outflow::STAGE0_VALUE_WEI`; 10 ETH before). For C3-H-1 (§7):
  BAYC's advance is now 7.353 ETH, so **3 BAYC advances fit a 25 ETH day** (1 fitted 10 ETH), and the tripwire (3 × 25 = 75 ETH
  asked) is reached after about **10** re-asks of one waiting BAYC advance (4 at 10 ETH and 7.469).
- **Go-live and services:** golive-check holds each Ethereum collection's cap against the launch file (`--launch`; J7's 15 ETH and
  ≤ 24 items without it) and FAILs a total under the collections' sum; the dry run's stand-in canister answers J7's caps and two
  refusals are shown (a binding total, BAYC at 3 items); the monitor's argument diff (`argdiff.mjs`) and the reconciler's stage
  (`ops/src/stage.ts`) mirror the new stage 0; the Trust page (`web-rook/src/rook/phase1Trust.ts`) states it, with custody's
  worst case now 75 ETH for the five; `docs/v5/launch-gates.md` G7's stage 0 row.

## 5. The Ethereum provider: Dwellir, 500-block log ranges, the key never in a file

- **Measured 2026-09-27** (read-only, no key): `xrpc.cl/eth` answers the finalized tag and `eth_getLogs` over 500 and 2,001 blocks
  (kept second, independent and keyless); `1rpc.io/eth` caps logs at 50 blocks; `mainnet.gateway.tenderly.co` and
  `ethereum-rpc.publicnode.com` answered 500 blocks. Dwellir with the operator's key: measured by the operator (500-block cap).
- **Monitor:** `deployments/launch/monitor-nft.json` `eth_rpc` = [`https://api-ethereum-mainnet.n.dwellir.com/${DWELLIR_API_KEY}`,
  `https://xrpc.cl/eth`]. `${NAME}` in an RPC URL is expanded from the environment at load (unset refuses the config) and every
  expanded value is scrubbed from alerts, the status body and log lines (`config.mjs` `expandSecrets`, `redact`). The deposit scan
  asks at most `eth_nft.logs_max_blocks` (500) a request, and after its first poll only the new blocks (it re-read the whole
  7,200-block window every minute before).
- **Runner:** `ETH_LOGS_RPC` (the Dwellir URL, a secret env value, never logged) and `ETH_LOGS_MAX_BLOCKS` (500; was 2,400).
- **Canister:** the sale scan asks at most `eth_nft.logs_max_blocks` (500, within [25, 2,000]); a chunk saved at 2,000 is cut to
  it; a pass covers as many blocks as before in narrower chunks, only while the window is being read.
- **The canister's providers stay keyless (flagged for the round-3 fixer):** a keyed URL in `evm_chains[1].rpc_urls` would be public
  (the install argument is published with every timelock proposal, and the monitor's argument diff prints provider URLs). The
  canister's query methods do not return `evm_chains`, and its errors name the method, not the URL; but the argument is enough.

## 6. Results

| Suite | Result |
|---|---|
| forge, `test/fork` excluded | **1,105 passed** (1,096 + J1's 3 and A2-L-1's 2 in `Pooled.t.sol`, the J1 alternative's test replaced) |
| canister `cargo test` | **600 passed** (`a2_l_1_*` 6, J7 1, the 500-block scan 1; the stage-0 tests updated to J7 and 25 ETH) |
| monitor | **176 passed** (`bayc10.test.mjs` 7 new) |
| ops | **56 passed**; `tsc --noEmit` clean |
| web-rook | **72 passed**; `tsc --noEmit` clean |
| keeper, indexer, timelock | not affected (no file changed) |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (536 golive checks ok on the five-collection NFT stack; BAYC's factory at 10 ETH and its module at 1,000 / 0 read back; 1.14 items per BAYC graduation, Pudgy pooled; J7's caps held against the launch file; refused: a binding total, BAYC at 3 items, and every earlier refusal) |

**Reproducible build** (`autobuyer/Dockerfile.build`, linux/amd64, two `git archive` exports of the canister commit, each
`docker build --no-cache`): the same hashes both times.

```
0e54b2cdd771db8994fb6041996c36d309d924d9b454dc59c0c4e48b15012830  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

`buyer.wasm` is new (6,624,970 bytes; 1,259,836 gzipped with `gzip -9 -n`, under the 2,000,000 proposal limit) and replaces round 2's
`8efa4bdd…6938`. The timelock is unchanged. Nothing was installed; the exports and build images were removed.

## 7. For the round-3 fixer

- **C3-H-1** (not fixed here, by instruction): the drop-and-reopen double count is unchanged; it builds on H3's 25 ETH a day and
  BAYC's 7.353 ETH advance (§4).
- **A3-L-4:** done here (§4). **C3-L-1:** the advance's gross-up now uses the judged fee bound; the gate itself still assumes 0.5 %.
- **C3-M-1** applies to a sweep as to any way back: a sweep past its order's deadline in `BackBridging` waits, holding the leg.
- The canister's Ethereum provider pair must stay keyless (§5).
- Info: BAYC's settlements record fills at the band's lower edge (§1).

## 8. The diff for the round-3 review

`git diff 55d8d32..nft-bayc10`:

- **Canister:** `autobuyer/canister-rs/{etch_autobuyer.did, examples/watch_fixtures.rs, src/{canister,custodycap,ethflow,ethnft,outflow}.rs,
  tests/{audit_nft,audit_nft_r2,ethnft,hardening}.rs, tests/fixtures/eth-state-round2.hex}`
- **Contracts tests:** `test/StackChecks.t.sol`, `test/ethnft/Pooled.t.sol` (no file under `src/` changed)
- **Launch and go-live:** `deployments/launch/{nft-pilot,monitor-nft}.json`, `script/golive-check.sh`, `scripts/golive-dryrun.sh`
- **Monitor:** `monitor/src/{alerts,argdiff,candid,config,ethnft,main,records}.mjs`, `monitor/fixtures/canister.json`,
  `monitor/test/{audit_nft_d_r2,bayc10,ethnft}.test.mjs`
- **Ops:** `ops/src/{ethnft,runner,stage}.ts`, `ops/test/{audit_nft_d_r2,ethnft,reconcile,runner}.test.ts`
- **Site:** `web-rook/src/rook/phase1Trust.ts`, `web-rook/src/content/launch-gates.md`, `web-rook/test/phase1.test.ts`
- **Docs:** `docs/v7/{phase1-nft,phase1-nft-pooled}.md`, `docs/v5/launch-gates.md`, this document
