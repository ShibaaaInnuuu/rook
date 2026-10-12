# Launch gates: what must be true before real value enters custody

Written 2026-09-23 on branch `custody-security`. A gate is a yes/no fact that someone other than its author has checked, with
the evidence linked. **No item, sat or wei of anyone else's enters custody until every gate for the target stage is Yes.** That
includes the canister's custody, float, buyer and EVM addresses, and every wrapper, queue, desk and pool. The threats behind each
gate are in `docs/v5/custody-threat-model.md`, and the fixes in `docs/v5/custody-hardening.md`.

## The checklist

| Gate | What | Owner | Evidence |
| --- | --- | --- | --- |
| **G1** | An outside audit of the scope below. Every Critical and High finding fixed and re-reviewed; the report public | project lead | the report, the fix commits, the re-review letter |
| **G2** | The key ceremony done as written below, with its verification repeated by two people who hold no role | ceremony lead | the signed ceremony log, `deployments/canister-ic.json` |
| **G3** | Timelock parameters as below; the H6 announcement rule adopted by every proposer and guardian | proposers, guardians | the timelock's `status()` |
| **G4** | Code gates: H1–H4, H8 and H9 wired and tested; the blocked items of `docs/deploy/canister-mainnet.md` closed | canister agent | commits, test output |
| **G5** | The proof-of-reserves page live, reconciling hourly from public data | web agent | its URL and the reconciliation script |
| **G6** | The bug bounty live, funded and announced | project lead | the program page, the Safe balance |
| **G7** | The stage's caps and limits in the running install argument, matching the table below | proposers | `custody_caps()` and the install argument |
| **G8** | Cycles: top-up, freezing threshold, alarms, and the two drills | operator | the drill logs |
| **G9** | Monitoring: watcher v2 and the custody canary (H5) running, with the guardians in the channel | operator | a test alert received by every guardian |
| **G10** | The incident-response playbook below rehearsed once end to end | guardians, operator | the drill log |

---

## G1. The outside audit

### Scope

Priorities. **P0**: it can move, mint, pay or sign for custody or the ETH that becomes custody. **P1**: it decides what P0 does,
or a bug in it strands value. **P2**: interfaces, helpers and paths that are off by default. Lines are `wc -l` at `9022920`. For
Rust, "code" excludes the inline `#[cfg(test)]` module.

**Solidity (0.8.28, via-IR, Cancun; target Robinhood Chain 4663) — 7,064 lines in `src/`:**

| File | Lines | Priority | What it is |
| --- | --- | --- | --- |
| `src/Wrapper.sol` | 309 | **P0** | wFROG: wrap, custodian `wrapDeposit`, drand redemption, delivery, refund |
| `src/nft/NftWrapper.sol` | 266 | **P0** | the ERC-721 → ERC-20 wrapper: custodian `mint`/`mintDeposit`, redemption |
| `src/WrapClaim.sol` | 184 | **P0** | pays each wrapped frog's units by Merkle root |
| `src/curve/ItemQueue.sol` | 317 | **P0** | per-collection FIFO buy queue; `recordItem` pays the custodian |
| `src/curve/MintDesk.sol` | 135 | **P0** | public mint on demand; `fulfill` pays the custodian |
| `src/curve/FrogCurve.sol` | 427 | **P0** | the launch curve whose graduation buys items |
| `src/pools/LiquidityLock.sol` | 638 | **P0** | owns every pool position; no way out |
| `src/SweepVault.sol` | 436 | **P0** | fees buy wX; keeper `postCap` |
| `src/nft/NftVaultBase.sol` | 356 | **P0** | shared NFT vault logic; keeper `postCap`, draws |
| `src/nft/NftSweepVault.sol` | 134 | **P0** | 4663 NFT sweeps through the Seaport guard |
| `src/nft/SeaportGuard.sol` | 102 | **P0** | decodes and re-encodes Seaport calldata (no tips, one listed NFT per order, per-order cap) |
| `src/nft/NftRemoteVault.sol` | 316 | **P0** | releases ETH to the canister for other-chain NFT purchases; receipts on its word |
| `src/AutoBuyerVault.sol` | 497 | **P0** | v2 vault: ETH only to its immutable buyer |
| `src/ListingRegistry.sol` | 370 | **P0** | roles, listings, 7-day changes, `reserveBuyer` |
| `src/nft/NftRegistry.sol` | 171 | P1 | NFT listings, sweep targets |
| `src/curve/CurveFactory.sol` | 160 | P1 | deploys curves; `freshCap` (the queue's and lock's cap source) |
| `src/pools/EtchRouter.sol` | 119 | P1 | buy/sell routing through v4 |
| `src/Distributor.sol` | 434 | P1 | draws and pro-rata claims (v1/v2 path) |
| `src/AssetVault.sol` | 502 | P1 | v1 bonded vault |
| `src/lib/Wait.sol` | 72 | P1 | the dual clock every wait relies on |
| `src/rand/DrandEvmnet.sol`, `src/rand/BN254HashToCurve.sol` | 205 | P1 | the beacon every redemption draws from |
| `src/curve/CollectionDirectory.sol` | 57 | P2 | directory of per-collection stacks |
| `src/nft/NftFeeRouter.sol`, `src/nft/NftLauncher.sol`, `src/FeeRouter.sol`, `src/Launcher.sol`, `src/Treasury.sol` | 484 | P2 | fee routing and launches (Pons) |
| `src/pools/LiquidityAmounts.sol`, `src/lib/BtcAddress.sol`, `src/nft/Seaport16.sol`, `src/nft/INft.sol`, `src/curve/CurveToken.sol` | 180 | P2 | helpers and interfaces |
| `src/interfaces/*` | 223 | P2 | `IWrappedCollection`, `ICapSource`, `ILiquiditySeeder`, `ISweepListing`, … |
| `script/*.sol`, `script/*.sh` | 1,151 | P1 | deploy scripts and their refusals (`DeployChecks`, `GoLiveChecks`): a wrong constructor argument is permanent |

**The buyer / custodian canister (Rust, `ic-cdk` 0.19) — 17,760 lines, 13,035 of them code** (16,183 / 12,097 at `9022920`; the
rest is this branch's three unwired modules):

| Module | Lines (code) | Priority | What it is |
| --- | --- | --- | --- |
| `canister.rs` | 2,044 (1,636) | **P0** | entry points, guards, upgrade rules (`restore_init`), keys and paths, pause |
| `psbt.rs`, `sighash.rs`, `runestone.rs` | 1,924 (998) | **P0** | what the buyer will sign on Bitcoin |
| `custody.rs`, `redeem.rs`, `delivery.rs`, `custodynet.rs` | 1,718 (1,395) | **P0** | custody: locating, building and sending deliveries; the deadline rule |
| `frogflow.rs`, `frogbuy.rs`, `deposits.rs`, `frognet.rs` | 2,990 (2,585) | **P0** | frog buys from the float, holder deposits (mints), returns |
| `seaport.rs`, `nftnet.rs`, `chains.rs` | 1,940 (1,464) | **P0** | Seaport verification (incl. SignedZone), per-chain lines, `nft_set_target` |
| `evmtx.rs`, `evmsend.rs`, `pin.rs` | 775 (507) | **P0** | EVM envelopes, the send rule, pinned reads |
| `listing.rs`, `state.rs`, `looper.rs`, `policy.rs` | 1,590 (1,142) | **P0** | quote checks, the purchase state machine, the loop |
| `itemsink.rs`, `buyqueue.rs`, `curvebuy.rs`, `mintdesk.rs` | 785 (522) | P1 | the rules for queue, curve and desk sinks |
| `btcsend.rs`, `btcnet.rs` | 524 (419) | P1 | Bitcoin broadcast and read-back |
| `rates.rs`, `oracle.rs`, `runeindex.rs` | 997 (765) | P1 | reference rate, runes indexer |
| `custodycap.rs`, `outflow.rs`, `outbound.rs` | 1,577 (938) | P1 (P0 once wired) | **new, unwired**: H2–H4 |
| `relay.rs`, `relayquote.rs`, `bridgecall.rs`, `bridge.rs`, `venue.rs`, `abi.rs`, `lib.rs` | 1,029 (664) | P2 | venue hand-in, bridge judges (not configured), ABI helpers |

**The timelock (Rust) — 794 lines (617 code): P0.** `autobuyer/timelock-rs/src/rules.rs` (541), `canister.rs` (245), `lib.rs` (8).

**Off-chain, in scope for a lighter review:** `scripts/timelock-watch.mjs` and the canary (H5), `autobuyer/scripts/reproducible-build.sh`,
the keeper's hand-in paths (`keeper/src/opensea.ts`), and the web app's redemption and approval flows (T14).

**Out of scope:** the IC protocol and threshold signatures (DFINITY's, audited by Trail of Bits and NCC Group); Pons V2; Seaport,
OpenSea's zone and Uniswap v4; `lib/`; test code (read it: it is the specification).

### What the auditor must receive

`docs/audit/README.md` (updated on this branch, with the trust-assumption map); this directory (`docs/v5/`); the design docs in
`docs/v2`–`docs/v4`; and a commit that has H1–H4 wired. **Audit the wired code, not this branch.**

### Shape and pass condition

One firm with published IC canister work for the P0 Rust and the timelock (`docs/audit/README.md`, "Who could audit this"), and a
competitive review for the Solidity. The gate passes when every Critical and High is fixed, the fix is re-reviewed by the same
firm, and the report is public. A Medium left open needs a written acceptance in `docs/audit/README.md`'s known-issues table.

---

## G2. The key ceremony

This extends `docs/deploy/key-ceremony.md` and `docs/deploy/canister-mainnet.md`. It does not replace them. Nobody generates,
sees or transmits a key other than their own, and no secret is ever typed into a shared document.

### Who holds what

| Role | How many | Held by | Where | Power (and its bound) |
| --- | --- | --- | --- | --- |
| Safe signers (4663 owner) | 5, threshold 3 | 5 different people | one hardware wallet each, different cities | listings, roles, pause listings; the registry's loosenings wait 7 days, but `ProtocolTreasury`'s setters and ownership transfers are instant (see the note under the solo table) |
| Proposers (IC) | **3** (code floor 2) | 3 different people, none a guardian | HSM-backed `dfx` identity (PKCS#11), offline machine, stored apart | propose; one pending each; 72 h public |
| Guardians (IC) | **4** (code floor 3), veto k = 2 | 4 different people, none a proposer, in at least 3 time zones, reachable within 4 hours | HSM-backed identity | any one pauses; k veto; k resume |
| Operator (IC) | 1 | the loop's server | hot identity on that server, never exported | operate within the code; pause, never resume |
| Keeper (4663) | 1 | the keeper service | its own key | caps, snapshots, records: `docs/trust-model.md` |
| Deployer (IC and EVM) | 1 each | the ceremony lead | throwaway identity, deleted in front of the witnesses | everything before the handover; nothing after |
| Canary / watcher operator | 1 | not a proposer | its host's environment (Telegram bot token only) | alerts only |
| Cycles top-up account | 1 | the operator | a cycles-ledger account | adds cycles; can never remove them |
| Witnesses | 2 | people who hold none of the above | — | they verify and sign the log |

At least **9 distinct people**: 3 proposers, 4 guardians, 2 witnesses. Safe signers may overlap with proposers but not with
guardians.

### The solo variant (chosen 2026-09-23; the table above is where it grows to)

The operator has no other people to involve at launch. Rook launches under a solo ceremony that keeps every structural
protection and replaces the people with delay and small caps:

| Role | Solo holding | Bound |
| --- | --- | --- |
| Controller of every canister | **the timelock canister only**; the deployer identity is deleted in front of the log | nothing changes without a 72 h public wait (7 days at stage 3) |
| Proposers | two password-encrypted `dfx` identities, p1 and p2, both on device A (p2 an encrypted offline backup: losing every proposer ends upgrades); no hardware wallet, the operator has none | one pending proposal each |
| Guardians (veto and pause) | three identities, g1, g2 and g3, all on device B, a different machine, stored apart from A; veto k = 2 | any one pauses; a veto or a resume takes two identities, both on B |
| Safe (4663 owner) | 2-of-2 software keys, one on A and one on B (browser-extension wallets on different machines, each password-locked) | the registry's loosenings wait 7 days; `ProtocolTreasury.setShares` / `setOps` / `setBootstrapLoan` / `addCollection` and ownership transfers do not (below) |
| EVM deployer (4663) | a Foundry keystore on device A (`--account`, `docs/deploy/contracts-mainnet.md`), password-encrypted; no hardware wallet | owns nothing after its deployments; holds ≈ 0.03 ETH for gas |
| Operator, keeper, watcher, cycles account | as in the table above | unchanged |
| Witness | the public log itself: the ceremony transcript, module hashes and verification block are published before any value enters custody | anyone can repeat the verification |

**Corrected 2026-09-25 (docs/v6/phase1-scope.md §2, §7 items 1 and 4).** This table said "k = 1 while solo". The production
timelock cannot be installed that way: it refuses fewer than 2 proposers, fewer than 3 guardians, a `k` outside `2 ≤ k ≤ g − 1`
and a principal in both roles (`timelock-rs/src/rules.rs:44-80`). So the solo operator holds two proposers on A and three
guardians on B with k = 2. That meets the code's floors and adds no security over one identity per device: the bound stays the
72 h delay plus the stage-0 caps. It also said "loosening waits 7 days" for the Safe. That is true of `ListingRegistry`'s
roles, roots, BTC vaults and treasury (`CHANGE_DELAY`), and false for two things: `ProtocolTreasury`'s owner setters
(`setShares`, `setOps`, `setBootstrapLoan`, `addCollection`) take effect at once (`src/floor/ProtocolTreasury.sol:98-120`), and
ownership moves by `Ownable2Step` (`transferOwnership`, then `acceptOwnership`) with no delay. A Safe holder can therefore
redirect the treasury's income and its unallocated balance, and hand the contracts to another owner, immediately.

Rules that hold while solo. What a script checks is named in each; the rest is procedure, checked at the ceremony and on the
public log. `golive-check.sh --canister <buyer>` reads the buyer's `custody_caps()` and holds each collection's cap to the stack's
stage (K9, 2026-09-25); `scripts/timelock-watch.mjs --expect-proposers/--expect-guardians/--expect-veto` checks the roles.
Before 2026-09-25 this line said all of it was "checked by `golive-check`", which never read the canister.

1. **Caps stay at stage 0** (3 items and 10 ETH per Bitcoin collection; since the operator's J7 of 2026-09-27, 15 ETH per Ethereum
   collection with its item count set so 15 ETH fits at its floor, and no total beyond the collections' sum; read back by
   `golive-check.sh --canister`) and **may not rise past stage 1** until at
   least **two outside guardians** hold identities on devices the operator does not control. Adding a guardian is itself a
   timelocked change; the stage table in G7 gates on the guardian count as well as on time.
2. A compromise of device A can only *queue* a change; the 72 h wait and device B's veto are the defence. A compromise of
   both devices bounds the loss **of custody** to the stage-0 caps. It does not bound the rest: both devices hold the Safe,
   which can change `ProtocolTreasury.setShares` and `setOps` and transfer ownership at once, and through the timelock (no
   veto left) can upgrade the canister and mint unbacked wX up to H11's 3 a day. The ETH in the curves, the Floor Module and the
   pools is not bounded by the caps at all (docs/v6/phase1-scope.md §0). The site's Trust page states this in full, not the
   shorter sentence (K5, 2026-09-25).
3. The operator rehearses a veto and a pause from device B before the handover, and repeats the G8 drill monthly.
4. When guardians join, the ceremony above is run for the new roles; the solo identities become one proposer and one
   guardian among several, never more.
5. **No hardware wallet (2026-09-23).** Both identities are software keys. That is accepted at stage 0 because the caps bound
   the loss to 10 ETH; it is the reason rule 1 forbids rising past stage 1. Minimum hygiene, checked at the ceremony: each key
   lives on one machine only, password-encrypted at rest, never in a cloud drive, chat or screenshot; device B is a machine
   that is not used for daily browsing; both passwords differ from every other password the operator uses. A hardware wallet
   (about $80) is the first purchase before any cap raise, and moving a role to it is a timelocked change like any other.

### The runbook

**Day −7.** Each role holder sets up their own device (`docs/deploy/key-ceremony.md`, step 1) and sends only their principal or
address, over two channels. The ceremony lead publishes the list of expected principals in the repo (`deployments/roles.json`),
and each holder confirms their own line in a signed commit or message.

**Day 0, one sitting, witnessed (video call, recorded; screen-shared terminal).**
1. The reproducible build is run by the lead **and** by one witness on a different machine: `autobuyer/scripts/reproducible-build.sh`.
   **Waived by the operator, 2026-10-09:** no second person takes part in the launch. The lead's reproducible build is the only one;
   instead, a second build of the same commit is made in a fresh clone on a different machine or a clean CI runner (the
   `reproducible-build.sh` hashes, recorded in `docs/v7/golive-runbook.md`), and the hash is published with the release so anyone can
   rebuild and compare after the fact. What this loses: nobody independent of the operator vouches for the installed wasm before
   install.
   Both read out the three SHA-256s; they must match each other and the commit's `deployments/canister-ic.json`.
2. Create the buyer and the timelock (`docs/deploy/canister-mainnet.md`, step 1). **Fresh canister ids. Never an id that ever ran
   a rehearsal build** (T16). Choose the subnet (fiduciary recommended for custody).
3. Install the buyer with the production argument: `chain_id = 4663` (disables `rehearse_sign_evm`), Bitcoin mainnet (disables
   `rehearse_sign_sighash`), ≥ 3 independent providers per chain (H10), `custody_caps` and `outflow_limits` at **stage 0**
   (G7), and the operator's principal.
4. Read `evm_address`, `buyer_address`, `custody_address` and `float_address`. The three Bitcoin addresses must all differ. Write
   them into `deployments/canister-ic.json`. **Send nothing to any of them.**
5. Set the buyer's freezing threshold to 90 days (`dfx canister update-settings --freezing-threshold 7776000`) and top it up to
   20 T (G8). This must happen now: after the handover only the timelock could, and it cannot.
6. Install the timelock with `delay_secs = 259200` and the roles from `deployments/roles.json`. Check that `status()` shows exactly
   those principals, `veto_threshold = 2`, and `rehearsal_build = false`. Verify its module hash equals `timelock.wasm`.
7. **Handover**: set the buyer's controllers to `[timelock]` only. Freezing threshold 90 days and top-up to 2 T on the timelock.
   **Blackhole**: remove the deployer from the timelock's controllers.
8. Both witnesses independently run the verification block below and read out the results. The lead deletes the deployer identity
   on screen (`dfx identity remove`), and the witnesses sign the log.

**Day 0, contracts** (`docs/deploy/contracts-mainnet.md`): the wrappers are deployed with `custodian = evm_address` and
`custodyAddress = custody_address`. `golive-check.sh` passes, and its output goes into the log. The Safe owns everything; the
deployer EVM key holds nothing afterwards.

**Day +1 to +7.** The drill from `docs/deploy/key-ceremony.md`, step 7, plus G10's incident drill. Every guardian pauses once;
two resume; a harmless proposal is vetoed by two. The watcher must report every step, and the canary must report a deliberate
test transaction sent **to** custody as expected inbound.

### Verification anyone can repeat

```
git clone <repo> && cd <repo> && git checkout <commit in deployments/canister-ic.json>
cd autobuyer && scripts/reproducible-build.sh                      # prints buyer.wasm and timelock.wasm SHA-256
dfx canister --network ic info <buyer>                            # Controllers: <timelock> ONLY; Module hash = buyer.wasm
dfx canister --network ic info <timelock>                         # Controllers: (none); Module hash = timelock.wasm (NOT the rehearsal one)
scripts/reproducible-build.sh verify <buyer> buyer ic             # MATCH
scripts/reproducible-build.sh verify <timelock> timelock ic       # MATCH
dfx canister --network ic --identity anonymous call <timelock> status   # roles = deployments/roles.json; delay_secs ≥ 259200; no rehearsal
dfx canister --network ic --identity anonymous call <buyer> cycles      # balance ≥ 20 T
dfx canister --network ic --identity anonymous call <buyer> custody_caps  # the stage in G7 (policy, then headroom per collection)
cast call <wrapper> "custodian()(address)"        --rpc-url https://rpc.mainnet.chain.robinhood.com   # = the buyer's evm_address
cast call <wrapper> "custodyAddress()(string)"    --rpc-url …                                          # = the buyer's custody_address
cast call <nftwrapper> "custodian()(address)"     --rpc-url …                                          # = the buyer's evm_address
cast call <registry> "isAutoBuyer(address)(bool)" <evm_address> --rpc-url …                            # true
```

The freezing threshold is not publicly readable after the handover (`canister_status` answers controllers only). The witnesses'
signed log is the evidence for step 5.

### Guardians' standing instructions (read at the ceremony, signed by each guardian)

1. **Pause first, ask later.** Any canary alert of an unexpected outbound transaction, any controller or module-hash change
   without an `Executed` event, and any CRITICAL argument diff (H5): pause (`pause_target`) at once.
2. **Veto a proposal with no announcement** (H6), or whose announced hash does not match a reproducible build of the announced
   commit, or whose argument diff was not announced.
3. **Resume only** when the incident playbook's resume conditions (G10) are met, and only with a second guardian.

---

## G3. Timelock parameters

| Parameter | Stages 0–2 | Stage 3 and above | Why |
| --- | --- | --- | --- |
| `delay_secs` | 259,200 (72 h, the code floor) | **604,800 (7 days)**, by `propose_controllers` to a new timelock with a 7-day delay (the running one is blackholed and fixed) | holders must be able to leave before a change lands: `outflow::exit_fits(max_items, per_day, delay)` must hold for every collection (H3). Stage 3's 192 items fit 7 days at 32 a day and do not fit 72 h |
| proposers | 3 | 3 | one lost still leaves two; one stolen is bounded by one delay |
| guardians / k | 4 / 2 | 5 / 2 | one lost or stolen changes nothing; two are needed to veto or resume |
| announcement | H6, mandatory | the same | a delay protects only people who read it |

---

## G4. Code gates

| # | Gate | Where |
| --- | --- | --- |
| C1 | **H1** wired: every signing, sending or staging method refuses while paused, with the source test | `docs/v5/custody-hardening.md` H1 — **done** `97178ad` |
| C2 | **H2** wired: `custody_caps` in `Init`, `validate_change` in `post_upgrade`, the hooks in `nft_set_target`, `nft_sign`, `nft_status`, the frog flow and deposits, and `release` on delivery and return; `custody_caps()` public | H2 — **done** `2562c0e` |
| C3 | **H3** wired: `outflow_limits`, `exit_fits` enforced at install, the freeze mapped onto `PAUSED`, the hooks in `redeem`, `frogflow` and `nftnet` | H3 — **done** `2562c0e` |
| C4 | **H4** wired with `require_finalized = true` and `require_event = true`, in `redeem` and `frogflow` | H4 — **done** `2562c0e` (Bitcoin; the EVM side comes with C10) |
| C5 | **H8** conflict-out built and tested (a fake world where a delivery is evicted and then refused must end with the fee input spent to custody) | H8 — **done** `72531db` (spec resolved `6d3bc4e`): `tests/redeem.rs` `a_delivery_evicted_and_then_refused_ends_with_its_fee_input_spent_to_custody`, plus returns in `tests/frogs.rs` |
| C6 | **H9** finalized pins for tickets, deposits and NFT arrivals | H9 — **tickets done** `2562c0e`; deposits' EVM side and NFT arrivals open |
| C7 | Mainnet in `KNOWN_NETWORKS`, with the checkpoint verified by two people on their own nodes | **done** `df2f13c`, with a solo deviation (`docs/v6/phase1-scope.md` §3.2): checked against five public explorers and the header's proof of work, published in `docs/deploy/canister-mainnet.md` (`fb7bc26`); no own-node check yet |
| C8 | ≥ 3 independent providers on every chain the canister acts on, one of them our own node (H10) | ops |
| C9 | **H11** (mint rate limit) and **H12** (cap step bound) in the deployed wrappers and vaults | contracts agent |
| C10 | The NFT delivery path (`docs/v4/nft-wrapper.md` §5.3) built with `transferFrom` only and H4's `verify_evm`, before any `NftWrapper` holds value | canister agent |
| C11 | The known issues in `docs/audit/README.md` either closed or accepted in writing | project lead |

---

## G5. The proof-of-reserves page (built by the web agent)

Requirements, so that the page is a check and not a claim:
- **From public data, reproducibly.** A script in the repo (the canary's reconciliation, H5) computes the page. Anyone can run it
  and get the same numbers.
- **Per wrapper:** `totalSupply`, `openTickets`, `held`, and the ledger check `totalSupply + openTickets × 1e18 == held × 1e18`.
- **Per held item, with its location proof.** Bitcoin: the inscription id → ord's satpoint → an output paying the custody
  address, unspent, as two esplora explorers report it. EVM: `ownerOf(tokenId) == custodian` on its chain at a finalized block.
- **The counts agree.** Items located = `held`. Any item held on the contracts' books but not located, or located at custody but
  not on the books, is shown red with its id.
- **Caps and headroom** from `custody_caps()` (H2), and the current stage.
- **Freshness:** updated at least hourly, with the block heights and the time of the last run. Stale for more than 3 hours shows
  red.
- **Alarms:** a red state on the page is also a canary alert (G9).

---

## G6. The bug bounty

**Scope.** Everything in G1's scope at its deployed addresses and canister ids, including the unwired modules once wired. The
web app's redemption and approval flows. The watcher and canary (a way to hide an outbound transaction from them is in scope).

**Out of scope.** The known issues and accepted risks in `docs/audit/README.md`. Third parties: the IC, Pons, Seaport, OpenSea,
Uniswap, drand, RPC providers. Attacks that need a compromised Safe, proposer, guardian or keeper key (those are threat-model
rows, not bugs), **unless** the finding is that the damage exceeds the bound the threat model states. Spam, DoS by paying
cycles or gas, social engineering, and physical attacks.

**Payouts: a percentage of the value at risk.** The value at risk is what the exploit could take at the time of the report,
bounded by the stage's caps and limits plus any pool (P) or escrow (Q) it reaches (`docs/v5/custody-threat-model.md` §1).

| Severity | Examples | Payout | Floor | Ceiling |
| --- | --- | --- | --- | --- |
| Critical | loss of custody; an unbacked mint; bypassing the timelock, the pause, a cap or the rate limit; signing with a custody key for an unlisted purpose | **10%** of value at risk | 5 ETH | 10% of (the stage's global cap + P + Q), and at least 100 ETH from stage 2 |
| High | theft bounded by one item or one keeper cap; a delivery to a wrong destination that H4 should have refused; a re-roll or double delivery; freezing custody for longer than one timelock delay | **5%** | 2 ETH | half the Critical ceiling |
| Medium | griefing a redeemer or depositor; stranding an item until a refund; a false alarm the canary cannot distinguish | flat | 1 ETH | — |
| Low | a check that fails open where it should fail closed, with no path to value | flat | 0.2 ETH | — |

**Funding.** The Safe holds the Critical ceiling for the current stage in a bounty reserve, announced on the program page. Moving
to a higher stage requires the reserve for that stage first.

**Terms.** A published security contact (`security.txt` on the web app, and a PGP key in the repo). Safe harbour for good-faith
research. No testing against mainnet custody: a fork, or a local replica with a test key. First valid report wins; duplicates
share nothing. Payment within 14 days of the fix, in ETH on 4663 or Ethereum. A platform (Immunefi or equivalent) is recommended
for triage from stage 1.

---

## G7. Staged TVL caps

Every stage is ×4 the one before, which is also the most one proposal may raise a cap (`custodycap::MAX_RAISE_FACTOR`). A raise is
a timelocked upgrade argument, announced under H6. It is never faster than the delay, and never more than one stage per proposal.

| Stage | Custody caps (per collection / global) | Per-item max | Outbound limits (items a day per collection / global; mints a day per collection; value a day) | Timelock delay | Entry criteria |
| --- | --- | --- | --- | --- | --- |
| **0: canary** | 3 items, ≤ 10 ETH / 6 items, 10 ETH. **Ethereum collections (J7, 2026-09-27):** ≤ 15 ETH each, items as 15 ETH fits at the floor (≤ 24; 6 when the argument names none) / the sum of the collections' caps (no binding total) | ≤ 10 ETH (one blue-chip); ≤ 15 ETH for an Ethereum collection | 3 / 6; 3; **25 ETH** (the operator, 2026-09-27; 10 ETH before). The NFT launch installs 12 / 24 items a day and, because every item must be able to leave within the delay (A4-L-1: at 24 a day in all 48 can, so J7's 61 do not fit), the operator's option (c) (2026-09-28, fixes-round5.md §6; `deployments/launch/nft-pilot.json` `stages`): **step 1** installs 48 items (Milady 13, Azuki 17, MAYC 13, Pudgy 3, BAYC 2; since 2026-10-09 Azuki 14 and the Bitcoin wrapper Bitcoin Frogs' 3 inside the 48, step 3's global 62: `docs/v7/golive-runbook.md` §9); **step 2**, a timelocked upgrade queued once step 1 has executed, raises only `global_items` to 31 a day (the monitor pages it as EXPECTED, `golive-check --proposal --stage 2` holds its argument; any other upgrade executed in its 72 h makes it stale: queue it again); **step 3**, after two clean weeks, puts the item caps back to J7's 61 (caps only). Deliveries go backlog first (ethflow `delivery_order`, A5-L-2), so the rule's two tests are enough whatever order tickets resolve in | 72 h | G1–G10 all Yes |
| **1** | 12 items, ≤ 40 ETH / 24 items, 40 ETH | per collection, ≤ 1.2 × floor | 6 / 12; 6; 20 ETH | 72 h | **4 weeks** at stage 0 with no incident of severity High or above; every canary alert explained; the proof-of-reserves page green throughout; bounty live 4 weeks |
| **2** | 48 items, ≤ 160 ETH / 96 items, 160 ETH | as above | 24 / 48; 12; 80 ETH | 72 h | **8 weeks** at stage 1 without incident; a second drill (tripwire freeze and k-of-n resume) passed; audit fixes re-reviewed |
| **3** | 192 items, ≤ 640 ETH / 384 items, 640 ETH | as above | 32 / 64; 24; 320 ETH | **7 days** | **12 weeks** at stage 2 without incident; a second independent review (audit or contest) of everything changed since G1; the 7-day timelock in place; H10's key separation done or explicitly accepted |

At every stage: `trip_factor = 3` (the tripwire fires at three times a limit in distinct asks), and global mints are twice the
per-collection mints.

Rows 1–3 are the Bitcoin wrapper's numbers. An Ethereum collection's stage n is ×4 its own stage 0 (J7: `ops/src/stage.ts`
`EVM_STAGE0`), not the row's 12 items; the outbound limits follow the row (review round 5, A5-I-4).

Checked by code at install and upgrade: stage 0 from nothing, ≤ ×4 per raise (`custodycap::validate_change`), and every
collection's cap, and the whole custody, can leave within the delay less a day (`outflow::exit_fits`), on the caps named and (review
round 5, A5-L-1) on what custody holds at the upgrade. Every stage's numbers pass both.

**Not in this table (2026-09-25, docs/v6/phase1-scope.md §7 item 9).** The Floor Module's ETH caps — `maxPublicEthBacked`
(10 items for frogs) and `maxEthBacked` (40 items) — are constructor constants set by `script/DeployStack.s.sol` from the frog
defaults (`StackChecks.sol:132-133`), not from the stage. They bound basis risk, not custody (docs/v5/floor-module.md §6.2), so
at stage 0 up to 40 items' worth of ETH (≈ 25 ETH at a 0.63 ETH floor) can sit in the module. Scaling them by stage would be a
deploy-script change (the module's `Params` are immutable per deployment), not a contract change; it is not made here.

**An incident resets the clock.** Any incident of severity High or above restarts the current stage's waiting period, and the
post-mortem decides whether to step down a stage (lowering is immediate in effect once the upgrade lands, and always allowed).

---

## G8. Cycles and drills

- Top-up automation keeps the buyer ≥ 20 T and the timelock ≥ 2 T. The freezing threshold is 90 days on both (ceremony step 5 and
  step 7).
- Alarms at 5 T (warning), 2 T (page everyone) and frozen (incident), from the watcher.
- **Drill 1, `pre_upgrade` at scale** (H13): on a replica, fill the state to 10× stage 3 and upgrade; record the instructions used
  against the IC's upgrade limit. Pass: under 25% of the limit.
- **Drill 2, out of cycles** (H13): on a replica, let a buyer run dry. Confirm the IC uninstalls it and that the timelock's
  `upgrade` fails on the empty canister. Rehearse recovery through `propose_controllers`, and write the result into
  `docs/deploy/canister-mainnet.md`.

---

## G9. Monitoring

Watcher v2 and the custody canary (H5) run on an always-on host, independent of the keeper's host, and post to a channel that
every guardian and the operator are in. A test alert of each kind reaches every guardian's phone before stage 0. Alerts:
proposal (with argument diff), veto, execute, failed; controllers or module hash changed; `Propose*` on 4663; cap step > 20%;
**any outbound transaction from a custody address not matching a canister record**; a ledger mismatch; a tripwire freeze; cycles
levels; provider disagreement persisting over 1 hour; no drand round proven for 1 hour while tickets are pinned.

---

## G10. Incident response

### Severity

| Level | Examples | Response time |
| --- | --- | --- |
| **SEV1** | any unexpected outbound transaction from custody; an unbacked mint; a controller or module-hash change without `Executed`; a CRITICAL argument diff; a ledger mismatch on the reserves page | pause within **15 minutes** of the alert |
| **SEV2** | the tripwire fired; persistent provider disagreement; cycles under 2 T; a cap step above 20%; the keeper or operator host suspected | assess within 1 hour; pause if unexplained |
| **SEV3** | a stalled delivery, a skipped drand round, a degraded provider | next business day |

### 1. Pause (any one guardian; minutes)

```
dfx canister --network ic --identity <guardian> call <timelock> pause_target      # the buyer stops: tick, and after H1 every signing path
dfx canister --network ic --identity <guardian> call <timelock> veto '(<id>)'     # for each pending proposal not announced under H6
```

The operator stops the keeper service (it holds the operator identity and the keeper key). The Safe pauses the affected listings
(`ListingRegistry.setPaused`, `NftRegistry.setPaused`): no new wraps or sweeps. **Redemption on the wrappers cannot be paused, by
design.** Tickets burned while custody is paused are not delivered, so holders' tokens come back: a ticket still pinned refunds 7 days after its pin; a resolved ticket 72 h after its delivery deadline once the canister attests it undeliverable (`markUndeliverable`, sent after the resume), and in any case 30 days after its deadline (`LATE_DELIVERY_WINDOW`).
Say so in the first message. **Until H1 lands, the pause does not stop `sign_purchase` or `nft_sign`: stop the keeper host too
(T6).**

### 2. Assess (guardian on call, the operator, the lead)

Read the canary's alert, then the canister's `log()`, `redemptions()`, `holder_deposits()` and `nft_chain_status(c)`, the
timelock's `status()` and `history()`, and the reserves page. Classify the incident against the threat model's rows (T1–T16).
Keep an incident log from the first minute: times, actions, who did them.

### 3. Communicate

- **Within 1 hour of a SEV1:** a banner on the web app and a pinned post on every official channel. Template: *"Custody is paused
  as a precaution while we investigate <one line>. Redemptions made now will be refunded automatically after 7 days if not
  delivered; your tokens are not at risk from waiting. Do not approve any new transaction from a link; the only app is <domain>,
  whose build hash is <hash>."*
- Updates every 6 hours until resolved, even if nothing changed.
- A post-mortem within 7 days: timeline, root cause, value affected, fix, and what changes in the gates.

### 4. Recover, by case

| Case | Recovery |
| --- | --- |
| Operator key stolen (T5, T6) | Pause; stop the host; propose an upgrade whose argument names a new operator (72 h); resume after it lands |
| Canister bug | Pause; fix; reproducible build; propose (72 h) with the H6 announcement; resume after it lands and passes the resume conditions |
| Providers lying (T10) | Pause; propose an upgrade replacing them (72 h, the argument diff announced); review every outbound act since the first bad read |
| Keeper key stolen (T4) | Since 2026-09-26 every contract reads the keeper from the registry (`docs/audit/2026-09-26/fixes-B-C-keeper-draws.md` §0), so one pause stops it everywhere. In order: (1) the guardian (the monitor, automatically on a confirmed trigger) or the Safe calls `ListingRegistry.pauseKeeper()`: instant; floor, caps, draws, mints, fills, sweeps and item arrivals stop; burns continue, at the floor the keeper left (at most the backing per unit); (2) `SweepVault.cancelDraw(id)` for every draw that is Announced or Open (not yet pinned; the monitor's `/status` lists them); (3) the Safe `proposeRoles(<new keeper>, purchaser, attestor)`; (4) wait 7 days (both clocks), stop the old keeper service and set the new key in it (dry run first); (5) `acceptRoles()`; (6) `unpauseKeeper()`. Holders burn from the moment the keeper is paused at the floor the keeper left or the backing per unit, whichever is lower, less the base spread (`FloorModule.pauseFloor()`, `docs/audit/2026-09-26/review3/fixes-R4-contracts.md` §1 and `review4/fixes-golive-lows.md` §1: at most the fill band's low edge and the cheapest recent mint, so a mint made before a pause cannot exit above what it paid); the backing per unit itself is paid once a week has passed with no good post (`exitAtBacking()`, the stale escape), paused or not. The monitor pauses automatically only on evidence (a confirmed bad root, a floor walk when told to, a draw whose root a healthy index disagrees with: `docs/audit/2026-09-26/review3/fixes-R4-services.md` §3). The monitor may also pause after a long stretch with no good post (`evm.guardian.pause_on_no_good_post_h`, opt-in), which ends a burn freeze at once. A closing buy waits (`QueueFull`) and an unfilled curve's refunds hold until 6 hours after the unpause (`keeperResumedAt + RESUME_GRACE`, so the keeper posts and the runner fills first), at most until the curve's escape hatch. The canister's own caps (H2) bound custody meanwhile |
| Frontend / DNS (T14) | Take the site down at the host; restore DNS; republish a verified build; announce the hash; list the redemptions made during the window |
| Subnet or key compromise (T3) | Nothing on our side stops a key thief. Pause (it stops our code), communicate, and account for the loss against the stage cap. Plan a migration to new canisters on another subnet |
| Cycles exhausted (T13) | Top up (anyone can). If the canister was uninstalled: `propose_controllers` to a recovery controller (72 h), reinstall the reviewed build, reconcile every in-flight record from the chains before resuming |
| Unbacked mint (T7) | Pause; the Safe pauses listings; announce the affected wrapper and the mint transactions at once (pools cannot be paused, so warn holders and LPs); recover through the post-mortem's decision |

### 5. Resume (two guardians)

```
dfx canister --network ic --identity <guardian-1> call <timelock> resume_target
dfx canister --network ic --identity <guardian-2> call <timelock> resume_target    # k = 2: the second vote resumes
```

Only when: the root cause is known and written in the incident log; the fix is live (module hash verified) or the case needed
none; the canary has been clean for 24 hours; the reserves page reconciles; and the guardians' standing instruction 3 is met.
After H3 is wired, resuming also lifts a tripwire freeze. That is intended: the same people decide both.

### 6. After

The post-mortem; any gate the incident showed to be wrong is changed in this document; the stage clock resets (G7).
