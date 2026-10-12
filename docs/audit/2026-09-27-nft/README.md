# Security review, 2026-09-27: Phase 1 on Ethereum NFTs (branch `nft-side`)

Four parallel AI reviews of the Ethereum-NFT side of Phase 1, following the method of the 2026-09-26 review
(`docs/audit/2026-09-26/`): scoped areas, the threat model and invariants written down first, a line-by-line reading, the
same severity scale, one proof of concept per finding where feasible (run against the repository's own harnesses: forge, the
canister's fake world, the services' node tests), a recommended fix per finding, and a re-check that none of the earlier
review's fixed findings regress in the code this branch touched. **Report only: nothing here is fixed.** Nothing was deployed,
pushed or signed; the only chains written to were local anvils; the network calls were public and read-only.

**The launch configuration reviewed** is the operator's decisions of 2026-09-27 (`docs/v7/phase1-nft.md` §9): **D1** Milady,
Azuki and MAYC at once (Pudgy Penguins and BAYC later, branch `nft-pool`, not reviewed); **D2** drand-random redemption; **D3**
≈ 0.01 ETH gas seed; **D4** stranded ETH waits (page at 24 h); **D5 holder deposits ON**; **D6** the OpenSea key in the runner
(depth measured: Milady 3 listings within +5 %, Azuki 14 from 4 sellers, MAYC 2); **D7** rank 2 of ≥ 3 listings, ≥ 3 sales,
+10 % / −15 %; **D8** the guardian pauses the keeper at > 20 % from both references; **D9** Blur not bought.

## Areas, commits and reports

| Report | Area | Commit read | PoCs |
| --- | --- | --- | --- |
| `A-contracts.md` | `EthNftWrapper`, `PosterSource`, the changed `FloorModule` (advance/settle/return), `DeployStack` NFT mode, `StackChecks`, golive-check; deposits + burns; cross-collection isolation on 4663 | `b48b3ba` | `poc/A/` (forge, 13 tests) |
| `B-canister-judges.md` | `relayeth.rs`, `ethnft.rs` (floor evidence, Seaport and Blur v2 sale parsing, the buy cap, the transfer judge), `ethnet.rs` (finalized reads, transforms, consensus), `canister.rs` wiring; the canister as floor poster; the D6 depth questions | `b48b3ba` | `poc/B/audit_nft_b.rs` (11) |
| `C-canister-flow-deposits.md` | `ethflow.rs` (arrival, finality, undeliverable marks, cancels, nonces, double spend/mint/delivery), holder deposits as built for D5 (claim judge, stray returns, discovery), economics, cross-collection flow | `e90471c` | `poc/C/audit_nft_c.rs` (8) |
| `D-services-deploy.md` | runner, monitor, keeper, indexer, site; what a malicious runner can do; the multi-collection deploy and golive-check of D1 | `57160b0` | `poc/D/` (ops 6, monitor 10, a mainnet measurement) |

The commits: `b48b3ba` is the branch before the operator's decisions; `e90471c` is D5 (holder deposits on); `57160b0` is the
launch configuration (several collections, golive-check and monitor holding D1/D5/D7/D8). A and B cite `b48b3ba` line numbers
and name the function each time: in `ethflow.rs`, `ethnft.rs` and `ethnet.rs` the function names are authoritative at HEAD.
A "passing" PoC shows the weakness; each turns red once it is fixed.

**Severity scale** (as 2026-09-26): **Critical** loses custody or mints unbacked units at scale with no one able to stop it;
**High** loses value or stops a path that cannot be switched off (redemption, burns, the floor) for everyone, cheaply or
without an attacker; **Medium** a bounded loss, or a stall that needs a condition or a privileged mistake; **Low** a narrow,
bounded or operational weakness; **Info** a note, a hardening, or a doc gap.

**Threat model** (each report states its own in full): an outsider with cheap NFTs, wash trades and self-listings; a
depositor (now live); a malicious or compromised runner/operator (hand-ins, timing, withholding); lying or lagging Ethereum
providers; Relay failing to fill or refund; reorgs before finality; gas spikes; a guardian pause; one collection's trouble
reaching another (D1). The canister's key and a timelocked upgrade are trusted as in the earlier review.

## Counts

| Area | Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- | --- |
| A contracts | 0 | 0 | 2 | 4 | 6 |
| B canister judges / floor | 0 | 2 | 7 | 9 | 6 |
| C canister flow / deposits | 0 | 1 | 3 | 6 | 9 |
| D services / deploy | 0 | 2 | 4 | 8 | 5 |
| **As reported** | **0** | **5** | **16** | **27** | **26** |
| **Distinct, open at HEAD (`57160b0`)** | **0** | **4** | **14** | **21** | **25** |

De-duplication (the same root found by two reviewers counts once, at the higher severity): B-H-2 = C-H-1 (its remaining
half, see below); B-M-2 = C-M-1; B-M-3 = C-M-3; B-L-3 = C-M-2 (Medium); B-M-5 = C-L-3 (Medium); B-L-4 = C-L-1; B-L-2 = C-L-6;
B-L-8 = C-L-2; B-I-4 = C-I-7. **A-L-4** (DeployStack could not build the three-collection stack) is resolved by `57160b0`; its
follow-up is D-L-7.

**What the later commits already changed in A's and B's findings:** B-H-2's deposit trigger (a deposit waiting on a limit or
without custody room stopped every floor post) is gone in `e90471c`; its buy trigger remains (C-H-1). A-L-1 is narrowed (no
deposit mints while a purchase is between its fill and its mint) but not closed (bridging and `Arrived` buys still share the
room). Everything else in A and B is open as written.

## Every Critical, High and Medium (distinct)

No Critical. No path was found that lets an outsider, a depositor or the runner take an NFT out of custody, mint a unit that
no NFT backs, or make the canister sign a transaction its judges did not build.

**High (4)**

- **B-H-1** — anyone steers the posted floor with self-trades: the sale band clamps the post above or below every real listing
  (±19 % holdable) — `autobuyer/canister-rs/src/ethnft.rs:507-542` `floor_from` (clamp `:533-536`), sale parsing `:403-477`;
  `ops/src/ethnft.ts:179-193` `recentTransferTxs` (@b48b3ba).
- **B-H-2 / C-H-1** — one refusal in `home_act`'s buy step (a purchase's mint waiting on H3; the advance's H2/H3 checks)
  returns before the floor step, so no collection's floor is posted for up to 24 h; deposits use up the shared mint limit on
  demand — `ethflow.rs:1281-1285` `home_act`, floor step `:1412-1428` (@e90471c).
- **D-H-1** — the honest runner's "sales" are the latest 12 transfer transactions: measured on mainnet, no floor post for
  hours a day (Milady 45 of 73 points) — `ops/src/ethnft.ts:196-205,286`; `ethnft.rs:528-530` (@57160b0).
- **D-H-2** — the canister posts its raw reading with no `floorBounds()` clamp; the monitor's keeper-era runaway rule reads a
  clamped post as a stolen key and, with D8 armed, pauses the whole registry — `ethflow.rs:1424-1425`;
  `monitor/src/evm.mjs:264-280,772-790`; `monitor/src/config.mjs:127-130` (@57160b0). `57160b0` sharpened it: the monitor now
  refuses an `eth_nft` configuration without an armed guardian (D8).

**Medium (14)**

- **A-M-1** — with deposits ON, a floor posted above the market (within the module's limits) is paid out in ETH through
  deposit → burn (+0.27 ETH/day at +10 %, +0.52 at +19 %, Milady stage 0) — `src/nft/EthNftWrapper.sol:282-302`;
  `src/floor/FloorModule.sol:501-541, 1244-1246, 1329-1333`.
- **A-M-2** — `redeemToRelist` is a free, repeatable option that keeps every NFT on the attacker's own tickets and closes
  redemption to everyone else — `EthNftWrapper.sol:320-326, 407-430`; `ethnft.rs:688`.
- **B-M-1** — one unreadable or oversized sale receipt voids the evidence pass; 12 plain transfers crowd out every sale —
  `ethflow.rs:510-557` `floor_pass`; `ethnet.rs:286-291` `eth_sale_receipt` (@b48b3ba).
- **B-M-2 / C-M-1** — any outcall failure of the transfer simulation is "refused (simulation)": a good delivery is marked
  undeliverable at once, a stray held for good — `ethnet.rs:109-121` `EthNet::fees`; `ethflow.rs:1015-1021` `eth_act`,
  `:949-953` `return_act` (@e90471c).
- **B-M-3 / C-M-3** — `superseded` clears the line before its reads: one failed read orphans a delivery that moved, so the
  redeemer later refunds and keeps the NFT — `ethflow.rs:1124-1159` `superseded` (@e90471c).
- **B-M-4** — the nonce is read at a pin that can move backwards: a used nonce is reused, stranding a delivery or leaving a buy
  `Stuck` — `ethnet.rs:109-121` `fees`; `nftnet.rs:253`; `pin.rs:54-61`; `ethflow.rs` `superseded`.
- **B-M-5 / C-L-3** — a collection at its custody cap that still wants blocks every other collection's buys; an `Arrived` or
  `Stuck` buy blocks all with no exit — `ethflow.rs:577-612` `buy_pass`, `:1330-1343` `give_back_now` (@b48b3ba).
- **B-M-6** — `strip_volatile` passes provider bodies through: measured, `1rpc.io/eth` answers in varying shapes, so
  replicated reads miss consensus — `canister.rs:693-695`; `nftnet.rs:71-73` `agreed`.
- **B-M-7** — one evidence pass reserves ≈ 72 B cycles per collection (≈ 15.6 T/day for three at the 20-minute cadence) —
  `ethnet.rs:260-291`; `ethflow.rs:510-557`.
- **B-L-3 / C-M-2** — the one Ethereum line has no bump, cancel or timeout for a fill or a return: an unmined one stops every
  collection's deliveries and marks — `ethflow.rs:484`, `:1071-1096` `resolve_eth` (@e90471c).
- **D-M-1** — the D8 pause is registry-wide, fires on real moves the module must lag, re-fires within a poll of every unpause
  and rests on effectively one keyless reference — `monitor/src/ethnft.mjs:99-110,166-168,183-186`.
- **D-M-2** — the 4663 canary cannot read `EthNftWrapper`'s events and judges its `Undeliverable` against Bitcoin redemptions:
  every honest mark is a false SEV1 — `monitor/src/evm.mjs:20-30,80-106`; `monitor/src/records.mjs`.
- **D-M-3** — the design's Ethereum outbound canary was not built: NFTs bought but not minted, deposits, held strays and an
  arrived buy's ETH can leave unpaged — `monitor/src/ethnft.mjs:148-163`.
- **D-M-4** — `eth_listing` always answers Ok, so the runner stops at the cheapest listing; a refused one is handed in again
  every pass and the buy waits (blocking all collections) — `ops/src/ethnft.ts:259-274`; `ethflow.rs:1596-1608`.

## D5 (holder deposits on) in the review

The deposit claim judge and the stray returns held under every attack the reviewers tried: fake deposits, reorgs before
finality, another collection's or another wrapper's NFT, replay of an old deposit after the token left and came back, claims of
our own fills, returns of held or in-use tokens, the tripwire (returns never trip it; one deposit mint is asked at a time). What
D5 exposes is economic (A-M-1: deposit → burn at a high floor; A-I-4 adverse selection), shared limits (C-H-1, A-L-1), gaps
around the return path (C-M-1's return half, C-L-5 no operator exit for a held stray, C-I-1 bookkeeping keyed without the
collection, D-L-3 one shared 7,200-block window for discovery and its watchdog), and two of R3's preconditions for turning
deposits on that are not built (C-I-3: a per-sender return budget, a return-anomaly tripwire).

## Regressions of the 2026-09-26 fixes

Each report has its table. No fixed finding regresses in the Bitcoin-side code this branch touched, and the contract fixes
carried into `EthNftWrapper` (no redraw, pinned-ticket refund, the gated refund, custodian-only delivery, B-05, H11) hold.
**Not carried to the new Ethereum side:** the finality of the ticket read (H9 / fixes-D H-1: B-L-2 = C-L-6), R3's paused pass
(M-1(c): B-L-4 = C-L-1), the stale-reservation sweep of L-2 (B-M-4) and the stuck-buy abandon of L-5 (B-L-3), and the monitor
precondition of fixes-W2-monitor "Left open 2" (D-H-2). M-2's upgrade-decode fix has a residual for the new optional element
(C-L-4).

## Ready for fix rounds?

**Yes.** Every finding has a location, an impact, a recommended fix and, for all Highs and most Mediums, a PoC in the
repository's harnesses that turns red once fixed. **Not ready for launch:** the four Highs are all about the floor (who sets
it, whether it is posted at all, and how the monitor and D8 react to it) and must be fixed first; they also decide how much of
A-M-1 remains. Suggested order: the floor (B-H-1, D-H-1, D-H-2, B-M-1, D-M-1), then the cross-collection stalls (B-H-2/C-H-1,
B-M-5, B-L-3/C-M-2, B-L-8), then the read-failure classifications (B-M-2/C-M-1, B-M-3/C-M-3, B-M-4, B-M-6), then
`redeemToRelist` (A-M-2), the monitor gaps (D-M-2, D-M-3), cycles (B-M-7), and D5's economics (A-M-1: a deposit fee, a burn
spread for deposit-minted units, or deposits minted only below the posted floor).

## Later: dormant collections (2026-09-29)

`dormant-review.md`: the review of branch `dormant-collections` (wake-on-demand collections, the `WakeDesk`), 3 High, 4 Medium
and 5 Low, fixed on the same branch. Its "Scaling" section reviews the auto-scaling of the wake limits from the cycles balance
(`dormant-collections.md` §11): 2 Medium and 3 Low, fixed.
