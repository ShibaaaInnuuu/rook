# Rook Phase-1 audit, area E: off-chain services and end-to-end workflows

Reviewer E, 2026-09-25. Checkout `/Users/jlohayon/Documents/Rook-audit-E`, detached at `origin/main` =
`e04f49b2bb94597e0107a607c5bc7ac3d3e3bbce` ("Merge branch 'p1-keeper'", 2026-09-25 01:27 -0400). No source or doc was modified.
Five PoC test files were added (listed in §6). Nothing was committed, pushed, deployed or signed. No key, wallet, Railway service
or identity was touched, and no `.env` was read. The only network use was a few read-only JSON-RPC calls to 4663's two public RPCs
(`eth_blockNumber`, `eth_getLogs`) to measure limits, plus fetching the methodology pages.

## 1. Method

**References followed, and what each was used for:**

- **OWASP API Security Top 10 (2023):** https://api-security.owasp.org/editions/2023/en/0x11-t10. Used on the indexer API, the
  keeper's `/status` and `/snapshots`, the reconciler, and the indexer's catalogue fetches. The relevant items were API4
  (unrestricted resource consumption), API6 (unrestricted access to sensitive business flows), API7 (SSRF), API8 (misconfiguration)
  and API10 (unsafe consumption of third-party APIs, which covers UniSat, Ordinals Wallet, Coinbase/Kraken and the RPCs).
- **OWASP ASVS 5.0, V13 Configuration:** https://github.com/OWASP/ASVS/blob/master/5.0/en/0x22-V13-Configuration.md. Used for hot
  keys and secrets:
  - 13.3.1: a vault or key store;
  - 13.3.3: an isolated module for crypto operations;
  - 13.3.4 and 13.1.4: rotation.
- **OWASP Top 10 (web):** used on web-rook for XSS sinks, CSP and what a user is asked to sign (A01, A03, A05, A08).
- **Chainlink, data feeds, "monitor and check timestamps / heartbeat / reasonable limits":** https://docs.chain.link/data-feeds.
  Used for the floor oracle's staleness, deviation and circuit-breaker behaviour.
- **OpenZeppelin Defender Relayer:** https://docs.openzeppelin.com/defender/module/relayers. Used as the keeper benchmark:
  - keys generated inside and kept in a KMS;
  - atomic nonce management;
  - gas-price caps;
  - automatic resubmission at +10% every minute, and replace or cancel by nonce.
- **Trail of Bits, Building Secure Contracts, incident response:** https://secure-contracts.com/development-guidelines/incident_response.html.
  Used for:
  - monitoring the system's own invariants;
  - documenting how each special-role wallet is reached;
  - not concentrating roles on one address;
  - treating off-chain components as common incident sources.

  The main secure-contracts.com index has no section specific to off-chain components; that page is its guidance for them.
- **Railway health checks:** https://docs.railway.com/reference/healthchecks. The page says: "Railway does not monitor the
  healthcheck endpoint after the deployment has gone live." (used in E-08).

**Passes:**

1. **Design docs:**
   - `docs/v6/phase1-scope.md` §0–§3.4, §8, §9;
   - `docs/v6/p1-services.md`;
   - `docs/deploy/services-v1.md`;
   - `docs/v6/k6-watcher-canary.md`;
   - `docs/v6/g10-incident-drill.md`;
   - `docs/v6/ordinals-apis.md`;
   - and, as needed, `docs/v5/custody-threat-model.md` T4, `docs/v5/floor-module.md` §5 and `docs/v5/launch-gates.md` G10.
2. **Each Phase-1 workflow traced end to end:** §3 below.
3. **Code review:** `keeper/src` (the floor path), `keeper/service/*`, `ops/src/*`, `monitor/src/*`,
   `indexer/src/{api,rpc}.ts`, `indexer/src/wrapped/*`, `indexer/src/rook/{routes,catalogSources}.ts`, and `web-rook/src`
   (TxButton, phase, Ticket, Redeem, Trust, server.mjs, Dockerfile, vite config). The contracts and the canister were read only
   where a workflow or a service depends on them: FloorModule, SweepVault, Wrapper, CurveFactory, DrandEvmnet, and the canister's
   `delivery.rs` and `redeem.rs`.
4. **PoCs** against the packages' own fakes, and the real recorded Ordinals Wallet book.
5. **Test gaps.**

**Baseline, before my PoCs:** every suite passes.

| Suite | Pass |
| --- | --- |
| keeper | 96 |
| ops | 20 |
| monitor | 48 |
| indexer | 99 |
| web-rook | 43 |
| tools/snapshot | 12 |

After adding the PoCs: keeper 103, ops 22, monitor 49, indexer 102 and web-rook 45. All pass, because each PoC asserts the
vulnerable behaviour.

## 2. Scope

**In scope:**

- `keeper/` (the service sender and the floor feed);
- `ops/` (the runner, the offer tool and the reconciler);
- `monitor/` (the watcher v2, and the Bitcoin and 4663 canaries);
- `indexer/` (the API, the wrapped/reserves code and the catalogue fetches);
- `web-rook/` (Phase-1 mode, TxButton, Trust/Reserves, `server.mjs`, `security.txt`);
- the Dockerfiles and `railway.json` files;
- the runbooks in §1.

Contracts and canister internals are other reviewers' areas and are cited only where a service or workflow relies on them.

## 3. Workflow map (Phase 1) and its trust points

Legend: **[K]** keeper hot EVM key (Railway env var) · **[O]** operator hot IC identity (Railway env var + operator laptop) ·
**[S]** Safe 2-of-2 (devices A+B) · **[U]** UniSat API key (canister heap, visible to subnet replicas) · **[OW]** Ordinals Wallet
book (single marketplace) · **[R]** 4663 RPCs · **[I]** indexer (feeds the site) · **[M]** monitor/reconciler (Telegram).

**Flow 1: launch, curve, graduation, custody.**

1. A user launches a curve with `CurveFactory.launch`. It is free and permissionless (`CurveFactory.sol:139-179`). The site
   launches through the stack's factory.
2. The user buys and sells on the curve with `buy(minOut)` / `sell` (`Ticket.tsx:77`). `minOut` is quoted from chain reads.
3. At the last buy, the curve calls `FloorModule.enqueue` and ETH-backed wX is minted at `gradPrice()`, which is
   `max(price, target) × 1.03`. This needs a fresh floor, which needs **[K]** to post and the price to come from **[OW]**.
4. The canister buys a frog. Every step depends on another party:
   - the runner **[O]** must tick;
   - the operator **[O]** hands in a PSBT through the offer tool;
   - the canister checks the offer against **[U]** plus ord (two providers);
   - the cost must be ≤ `maxItemCost` = 1.03 × the floor that **[K]** posted.
5. The frog is delivered to custody.
6. The canister calls `wrapDeposit` and `itemArrived`, which are custodian-only and checked by **[M]**'s 4663 canary.

   Break points:
   - no floor, which happens without **[U]** (feed §6.2), with the keeper down (E-08), or with an unbounded pass (E-04);
   - a manipulated floor (E-01);
   - operator hand-ins are manual (K4) and the runner's liveness is not paged (E-08).

**Flow 2: the floor post.**

1. `OrdinalsWalletFeed` reads the OW escrows and sales.
2. The 40 cheapest outpoints are checked by `VerifiedOrd`, which needs two providers and so needs **[U]**.
3. `robustFloor` computes the floor. Sales can only pull it DOWN.
4. The price is converted at the Coinbase/Kraken "agreed" rate.
5. `planPost` decides, within 30 min / 0.5% triggers.
6. The sender simulates, checks its caps, signs with **[K]** and sends via **[R]**.
7. The module accepts at most ±5%/h (`FloorModule.sol:330-360`).

   Break points:
   - wash-sale manipulation (E-01);
   - staleness is silent (E-08);
   - pass starvation (E-04);
   - a stuck nonce with no fee bump (E-10);
   - a compromised key is permanent for the module (E-02).

**Flow 3: raffle and burn sweeps, and draws.**

1. **[K]** posts `postCap(floor × 1.1)` to the RAFFLE and BURN vaults.
2. Anyone can `sweep`.
3. **[K]** calls `openDraw(root, uri)` with a snapshot built by the keeper and served from the keeper's own host.
4. The 15-min challenge window opens; only **[S]** can cancel.
5. `requestDraw`, `resolve` (drand proof) and `claim` (Merkle proof from the file) are open to anyone. In Phase 1, **no service
   or page sends them** (E-03).

   Nothing checks the root against an independent recomputation, and DrawOpened is not monitored (E-02).

**Flow 4: redemption back to Bitcoin.**

1. A holder calls `Wrapper.redeem(bc1p…)`. The site validates bech32m (`Redeem.tsx:48`).
2. The ticket is Pinned to a future drand round.
3. **Somebody must call `Wrapper.resolve(id, sig)`.** Nothing in Phase 1 does (E-03). The canister only waits (`delivery.rs:91`).
4. The canister delivers, needing **[U]** and the runner **[O]** ticking.
5. The canary matches the delivery txid against `redemptions()`.
6. `recordDelivery` is called.
7. The refund path exists only from Resolved (`Wrapper.sol:548-551`).

**Flow 5: monitoring.**

- **What the monitor checks:**
  - the timelock and its proposals, including argument diffs;
  - certified controllers and module hashes;
  - cycles;
  - custody, float and buyer addresses on two esploras;
  - 4663 Wrapper and FloorModule custodian events on two RPCs.
- **What the reconciler checks:** reserves hourly, plus `custody_caps`.
- **Not watched:**
  - keeper, runner and indexer liveness;
  - floor and cap freshness;
  - FloorPosted, CapPosted and DrawOpened;
  - unresolved tickets;
  - a missing UniSat key;
  - a reconciler that is "unverified forever".

  See E-06 and E-08.

**Flow 6: the reserves page.**

- Green means: items in custody minus tickets in flight, plus ETH-backed units, equals supply.
- The ETH-backed term is taken at face value; only an escrow of exactly 0 is red.
- A stale module does not count against green.
- See E-05.

## 4. Findings

| ID | Severity | Title | Where (file:line) | Phase-1 reachable? | PoC |
| --- | --- | --- | --- | --- | --- |
| E-03 | **High** | Nothing in Phase 1 resolves a redemption ticket or progresses a raffle draw. Redemptions stall at Pinned with no refund path; a watcher can grind outcomes by redrawing | `Wrapper.sol:479-480,548-551,461-468`; `delivery.rs:91`; K3 call list `p1-services.md` §0; `Redeem.tsx:51,88-89` | Yes (redemption cannot be switched off) | static PoC, passes |
| E-02 | **High** | A stolen keeper hot key is unmonitored and, for the FloorModule, unrecoverable. It can freeze module exits indefinitely, walk the floor 5%/h, and open forged draw roots over every RAFFLE unit | `FloorModule.sol:112,330-350,830-832`; `SweepVault.sol:204,370-393,397-405`; `floorpost.ts:70`; `launch-gates.md:410` | Yes | PoC (freeze and keeper does not repair), passes; forged root by inspection |
| E-01 | Medium | The floor oracle's sales input is attacker-writable. Wash sales at ⌈L/4⌉ drag the posted floor to 0.775·L with confidence still over the minimum; one marketplace, thin book | `floor.ts:248,263`; `floorsources.ts:79`; `feed.ts:56-81` | Yes | PoC on the real recorded book, passes |
| E-04 | Medium | Every keeper pass makes ≥2 sequential RPC reads per coin ever launched. Launches are free, so passes grow without bound and starve the floor, caps and draws; rate-limits are not failed over | `keeper.ts:128-137`; `rpc.ts:62-73`; `CurveFactory.sol:139-179` | Yes | PoC (34 → 4,034 calls/pass at 2,000 coins) plus measured RPC limits |
| E-05 | Medium | The reserves page and K7 reconciler are GREEN with 10 ETH-backed wX against 1 wei of escrow, never compare `escrowBalance()` to the module's real balance, and ignore a stale floor | `invariant.ts:50-56,70-82`; `floorModule.ts:88-102` | Yes | PoC, passes |
| E-06 | Medium | One persistent 4663 RPC disagreement freezes the EVM canary's cursor for good. One non-repeating SEV2 is sent, the checker reports healthy, and a later unbacked mint is never alerted | `evm.mjs:131-148`; `main.mjs:63-81` | Yes | PoC, passes |
| E-07 | Medium | The Phase-1 send guard checks only the target. The sell ticket asks the wallet to `approve(router)` with `router` taken from the indexer, so a compromised indexer gets token approvals | `phase.ts:114-132`; `Ticket.tsx:41,71`; `wallet.tsx:47-50` | Yes | PoC, passes |
| E-08 | Medium | Monitoring coverage gaps, and alerting that fails open. See the detail below the table | `reconcile.ts:55,134-146`; `alerts.mjs:48`; `railway.json`; `services-v1.md:175` | Yes | PoC (E-08a, E-08b), passes |
| E-09 | Low | The "agreed" rate picks the lower sats/ETH, which gives the HIGHER wei floor; the design doc says the opposite | `feed.ts:48-50`; `p1-services.md` §1.4 | Yes | PoC, passes |
| E-10 | Low | No fee bump or replacement: a pending transaction whose re-broadcast is refused (fee < base fee) blocks all keeper sends while it lasts, and it is not alerted | `sender.ts:63-79,101-105` | Yes | PoC, passes |
| E-11 | Low | Draw snapshots scan Transfer logs from `fromBlock` in 50k-block chunks with no split on the provider's 10,000-log cap; a failed scan silently skips the slot. Snapshots live only on the keeper volume | `draws.ts:40-53,67-76`; `keeper.ts:154-155` | Yes | measured RPC limit; by inspection |
| E-12 | Low | The indexer's outbound-fetch SSRF guard is hostname-only: no DNS or IP check after resolution, redirects followed, `metadataOf`/`getJson` unguarded, unbounded JSON bodies | `catalogSources.ts:25-30,223-235,316-321,342-345` | Yes (catalogue page) | by inspection |
| E-13 | Low | Reserves "in-custody" rests on ordinals.com alone for the inscription's location; the two-provider rule is not applied | `verify.ts:30-47` | Yes | by inspection |
| E-14 | Low | Snapshot exclusions silently become `[]` if `exclusions.json` is unreadable; `KEEPER_PUBLIC_URL` accepts `http:`; the keeper's `/status` is public | `draws.ts:58`; `config.ts:100`; `status.ts:36` | Yes | by inspection |
| E-15 | Info | Runbook and code mismatches | see detail | — | — |
| E-16 | Info | Dependency audit: web-rook has 25 advisories (2 high), none in the shipped bundle's code paths; the keeper image runs as root; `^` ranges in web | `web-rook/package.json`; `keeper/Dockerfile` | — | `npm audit` |

**E-08 in detail.** Unwatched:

- keeper, runner and indexer liveness. Railway checks `/healthz` only at deploy;
- floor and cap freshness;
- FloorPosted, CapPosted and DrawOpened;
- a missing UniSat key;
- a reconciler "unverified" for a week.

A missing Telegram token turns alerts into log lines that are recorded as sent.

### E-03 (High): nothing resolves redemption tickets or raffle draws in Phase 1

**Description.** A Wrapper ticket goes Pinned → Resolved only when someone calls `Wrapper.resolve(id, signature)` with the drand
evmnet BLS signature of the pinned round (`Wrapper.sol:479-484`). `DrandEvmnet.randomnessOf` is non-zero only after someone calls
`prove` (`DrandEvmnet.sol:52-62`).

No Phase-1 component sends either call:

- the K3 keeper's closed call list is `postFloor`, `postCap` and `openDraw` (`p1-services.md` §0; `keeper/service/keeper.ts`);
- the canister's delivery logic only waits: `Stage::Pinned => Wait { "not drawn yet: the ticket's round is still unproven" }`
  (`delivery.rs:91`), and `redeem.rs:270` stops its walk at the first Pinned ticket;
- the runner only ticks;
- the site's Redeem page tells the holder "anyone proves it on chain" (`Redeem.tsx:51`), but offers only Refund (from Resolved)
  and Redraw (after 24 h unproven);
- the only code that resolves is the staging operator (`keeper/staging/operator.ts:284`, the v1 Distributor) and the go-live
  rehearsal script (`golive-dryrun.sh:175-185`), which did it by hand.

The consequences:

- `refund` requires `Stage.Resolved` (`Wrapper.sol:548-551`), so a Pinned ticket has **no refund path**. The burnt wX is neither
  delivered nor refunded.
- Resolution is strictly in order (`Wrapper.sol:480`, `OutOfOrder(nextToResolve)`), so **the first unresolved ticket blocks every
  later one**.
- `redraw` (`Wrapper.sol:461-468`) is allowed to anyone once the pinned round is unproven for a day. With no prompt resolver, any
  watcher can:
  1. read the pinned round's randomness from api.drand.sh as soon as it is published;
  2. compute which free item the ticket would get;
  3. if they dislike it, wait 24 h and `redraw`.

  This is outcome grinding across the free set. The honest holder can only prevent it by resolving first, and the site gives them
  no way to.
- The same gap applies to the RAFFLE vault. `requestDraw`, `resolve` and `claim` (`SweepVault.sol:408-460`) are sent by nothing,
  and the site has no raffle UI. Every opened draw sits in Open with its 1 wX reserved (`totalReserved`). Open draws have no
  timeout; `voidDraw` needs Resolved. The keeper opens another draw at the next daily slot, so over time the vault's units move
  into never-resolved draws, and a winner is never paid unless someone builds the Merkle claim by hand from the published file.

**Scenario.** A holder redeems one wFROG on launch day. Days later it is still Pinned. The site shows "Pinned" and a Redraw button
after 24 h; pressing Redraw re-pins it, which still resolves nothing. Every later redeemer is stuck behind the first ticket. G5's
reserves stay green, because tickets in flight are subtracted.

**PoC.** `keeper/test/audit_e.test.ts`, "E-03 PoC (static)". It scans all 152 Phase-1 service and site source files for any
`resolve`, `requestDraw`, `claim` or `prove` call or ABI. The only hit is `web-rook/src/lib/abis.ts`, the unused v1 Distributor
ABI. The test also asserts:

- the canister's Pinned → Wait;
- `refund` requires Resolved;
- the in-order rule.

**Result:** passes.

**Fix.**

1. Add a resolver to the keeper service, keyless: `resolve` and `requestDraw` are permissionless and need no special key.
2. The resolver fetches the round's signature from drand (two relays) as soon as the round exists, and calls `resolve` for every
   Pinned ticket in order, plus `requestDraw`/`resolve` for SweepVault draws.
3. Serve per-holder claims from the snapshot, and add a Resolve / Claim button to the site.
4. Page when any ticket is Pinned for more than one hour past its round, or a draw is Open more than one hour past its challenge
   window.
5. Consider letting `refund` also cover a Pinned ticket after a deadline, and bounding redraws (for example, the next round only,
   or only by the owner). These are contract changes.

### E-02 (High): keeper hot-key compromise is unmonitored and, for the FloorModule, permanent

**Description.** The keeper key is a plain Railway service variable (`services-v1.md:48`). It is in the same Railway project as
the runner's operator identity (`p1-services.md` §6.6). This does not meet ASVS 13.3.1 or 13.3.3 (KMS/HSM), nor Defender's "keys
never leave KMS".

It controls:

- **The FloorModule, whose keeper is immutable** (`FloorModule.sol:112`). There is no owner, setter or pause
  (`phase1-scope.md` §1). `proposeRoles` rotates only `registry.keeper()`. The recovery row in `launch-gates.md:410` ("Safe pauses
  listings and proposes new roles (7 days)") therefore does not recover the module, and "stop the keeper service" removes the
  honest poster. With the stolen key, the attacker can:
  - (a) post the current price with `confidenceBps = 0`. `fresh()` then requires confidence ≥ min (`FloorModule.sol:830-832`), so
    mint, burn, `fundFloat` and the grad/arrival paths that need a fresh floor stop. Re-posting every ~50 minutes keeps it that way.
    The honest keeper **does not repair it**: `planPost` decides on price and TTL only, and ignores the posted confidence
    (`floorpost.ts:67-73`). The ETH backing ETH-backed wX (up to 40 items' worth by `phase1-scope.md` §0, about 25 ETH) then has no
    exit at the module indefinitely;
  - (b) walk the accepted floor 5%/h down (`floor-module.md` §5 prices this at ≈1.4 ETH for frogs), then mint up to `publicCap`
    and sell into the pools, or redeem custody frogs.
- **SweepVault draws.** `openDraw` is `onlyKeeper` against `registry.keeper()` (`SweepVault.sol:204,370`). The attacker can post
  any root with any `uri`, and can call it repeatedly: the contract has no per-slot or per-period limit, only `units[coin] ≥ 1e18`
  and the snapshot's age (`:376-383`). Every RAFFLE unit of every coin can therefore be put into forged draws in one block. The
  only defence is `cancelDraw` by `registry.owner()` (the 2-of-2 Safe) within **15 minutes** (`SweepVault.sol:81,397-405`).
  Nothing watches DrawOpened or recomputes roots (`k6-watcher-canary.md` §8 defers "CapPosted steps" and 4663 governance), so in
  practice nobody would be at two devices within 15 minutes. The snapshot is also served from the attacker-held keeper host.
  Claims need the resolve/claim steps (E-03), which the attacker can script.
- **Caps.** Steps of ±20% per 10 minutes up to `capCeiling`. This is bounded, and documented under T4.

**Scenario.** The Railway account or project is phished, and the attacker reads `KEEPER_PRIVATE_KEY`:

1. **Minute 0:** openDraw ×N with a root listing only the attacker.
2. **Minute 15:** requestDraw.
3. **Minute ~16:** resolve and claim. The RAFFLE vault is empty.
4. **In parallel:** a confidence-0 post every 50 minutes.

The operator notices days later (E-08), rotates the registry keeper after 7 days, and the module stays frozen for as long as the
attacker cares.

**PoC.** `keeper/test/audit_e.test.ts`, "E-02 PoC". The fake chain takes the thief's confidence-0 post at the same price, and the
honest keeper's next pass says "keep — within 50 bps and 3000 s of its TTL left". **Result:** the module is stale for 180 of 180
minutes while the honest keeper runs. The forged-root drain is shown by inspection of `SweepVault.sol:370-393` (no rate limit) and
`:397-405`; no forge test was written, since contracts are another reviewer's area.

**Fix.**

- **Now, in the keeper:** re-post when `!fresh()` or when the posted confidence is below the minimum (planPost must consider
  confidence and `fresh`).
- **Monitoring:**
  - watch FloorPosted (a confidence below the minimum, or a target/accepted move with no keeper plan);
  - watch DrawOpened;
  - recompute each draw's root from chain with `tools/snapshot` in the **monitor's** project (not the keeper's), and page on any
    mismatch or on a second draw per coin per slot.
- **Key custody:** a KMS-held signer for the keeper; separate Railway projects/accounts for the keeper and the runner.
- **Contracts, before stage 1:**
  - a per-coin-per-slot limit on `openDraw`;
  - a longer challenge window, or a permissionless challenge that verifies against a recomputed root;
  - a way to replace the FloorModule's keeper (timelocked), or a module-level pause.
- **Runbook:** state plainly that "Keeper key stolen" cannot be recovered for the module.

### E-01 (Medium): the floor's sales input is attacker-writable; one marketplace; thin book

**Description.**

- `robustFloor` blends the listing floor L DOWN toward the sales median S: `price = min(L, 0.7·L + 0.3·S)` (`floor.ts:263`).
- The only sales filters are:
  - self;
  - same cluster (two-way trading, or a sale under L/4);
  - bundles;
  - outliers under L/4 (`floor.ts:248`);
  - one sale per buyer and per seller.
- An attacker with one frog and fresh wallets can sell it around 11 unlinked wallet pairs at ⌈L/4⌉+1 sats on Ordinals Wallet.
  Both sides are theirs, so the cost is OW fees plus miner fees. The newest 11 of the 20 counted sales are then theirs,
  S = L/4, and the price is 0.775·L.
- Confidence stays above 3,000 bps, because the agreement term is clamped at 5,000 (`floor.ts:273-274`), so the keeper posts it.
- The whole book is one marketplace (OW escrows, `feed.ts:56-81`). Listing freshness is asserted, not checked:
  `verifiedAt: fetchedAt, fillable: true` (`floorsources.ts:79`), so `L-stale` and `L-unfillable` can never fire on OW.
- Burst-detection state (`BookDiff`) is in memory and lost on restart.
- Only the 40 cheapest outpoints are checked (`withContents`).

On the recorded 2026-09-23 book the bottom is very thin: 433k, 565k, 565k, 606k, 693k sats, then more than 1M. The PoC measures
**depth = 1** listing within +10% of the floor, and 998,331 sats buys enough of the bottom to lift the listing floor 10%. Upward
pumping only helps against sweeps and caps (burns are β-capped), but the numbers show how little liquidity the oracle stands on.

**Scenario.**

- Over one afternoon, 11 wash sales move the posted floor down 13% on the real book (22.5% of L in general). The module follows
  in 3 hourly steps.
- The attacker then mints at the depressed `mintPrice`, up to `publicCap` = min(10, depthItems). On this book that is about one
  item: a small gain per round. They sell in the pools or redeem.
- Meanwhile `maxItemCost` = 1.03 × the depressed floor sits under the market, so the canister cannot buy any frog. Conversions and
  graduations stall, which is a cheap denial of service against Flow 1.

**PoC.** `keeper/test/audit_e.test.ts`, "E-01" and "E-01b", on `fixtures/floor/ow-bitcoin-frogs-*-2026-09-23.json`.

**Result:** listing floor 565,262 sats; honest floor 503,683 at 4,900 bps confidence. After 11 wash sales at 141,316 sats the
floor is 438,078 (−13.0%) at 3,847 bps, and the module reaches it in 3 hourly steps. Passes.

**Fix.**

- Take sales from on-chain Bitcoin transfers with a real PSBT-sale shape, or from at least two marketplaces.
- Weight sales by recency and distinct counterparties' age; require sales wallets to have history.
- Make a sale-driven drop larger than X% need listing confirmation.
- Refuse to post (or alert) when a low `depthItems`/`upCost10` shows the book cannot support the price.
- Alert on a posted floor that deviates from a second reference (for example, ME's `stat` floor) by more than X%. This is
  Chainlink's "reasonable limits" monitor.

### E-04 (Medium): unbounded keeper pass from permissionless launches; rate limits are not failed over

**Description.**

- `draws()` loops over **every coin the factory ever launched**, sequentially: `coins(i)`, then `policyOf`, then (for RAFFLE)
  `units` and `routerOf` (`keeper.ts:128-137`). This happens every pass (default 60 s).
- `CurveFactory.launch` costs only gas (a few million gas at about 0.01 gwei, so about 0.00005 ETH per launch).
- The operator declined to cap open curves (`phase1-scope.md` §2).
- Each pass posts the floor and caps first, then walks the draws. The next pass starts `KEEPER_INTERVAL_S` after the previous one
  ends. Once a pass takes more than about 30 minutes, the 30-minute refresh and the 1-hour TTL are missed, and the module and caps
  go stale.
- `ChainRpc.call` treats **any** JSON-RPC error as final and does not fall back to the next endpoint (`rpc.ts:62-73`). Both public
  RPCs return rate limiting as JSON-RPC errors over HTTP 200. Measured today:
  - Robinhood: `{"code":429,"message":"Too Many Requests"}` and `"logs matched by query exceeds limit of 10000"`;
  - ordofi: `{"code":-32005,"message":"the network is busy"}`.

  So one throttled read fails the whole pass, and the second RPC is never tried.
- The indexer's `coins()` does about 10 parallel reads per coin per snapshot. The reconciler runs it with `ttlMs: 1`, which makes
  it a heavy client of the same RPCs.

**PoC.** `keeper/test/audit_e.test.ts`, "E-04 PoC". **Result:** `eth_call` per pass is 34 with 0 coins, 1,034 with 500 and 4,034
with 2,000. 10,000 launches (about 0.5 ETH of gas) mean about 20k sequential calls every pass. Passes.

**Fix.**

- Index the Launched events once, and keep a set of RAFFLE coins with `units ≥ 1e18` from the vault's events.
- Only walk the draws at the draw slot.
- Parallelise with a concurrency cap.
- Time-box each pass, so the floor post is never behind the draws.
- Fall back to the next RPC on 429 or -32005 JSON-RPC errors, with backoff.

### E-05 (Medium): reserves can be green while the ETH-backed half is not backed, and while the module is stale

**Description.**

- `computeInvariant` counts `ethBackedUnits` 1:1 as backing, and reds only when `escrow === 0` (`invariant.ts:56`).
- `escrowWei` is `escrowBalance()`, which is `backing + reserve`: the module's own storage accounting (`FloorModule.sol:1044`).
  Nothing compares it with `eth_getBalance(module)` (`floorModule.ts:88-102` never reads it), or with the ETH that the units
  imply (for example `E × β ≥ some fraction of floor`).
- `floorFresh=false` is not a reason at all.

So an accounting bug in the unaudited module (§0), or a keeper-driven freeze (E-02), leaves the G5 page, the K7 reconciler and the
go/no-go criterion "reserves page green" (`phase1-scope.md` §3.4, step 14; §4 stage-1 entry) all green.

**PoC.** `indexer/test/audit_e.test.ts`. **Result:** status=green with 10 ETH-backed units against 1 wei of escrow; green with a
6 ETH escrow and no balance check; green with `floorFresh=false`. Passes.

**Fix.**

- Read `eth_getBalance(module)` and require `balance ≥ backing + reserve + revenue + renterPool + …`.
- Add unverified or red when escrow per ETH-backed unit falls below a fraction of the floor.
- Add amber/red when the module or the caps have been stale for more than 1 hour, and page it.

### E-06 (Medium): the 4663 canary can be silenced by one persistent RPC disagreement

**Description.**

- `checkEvm` breaks at the first chunk where the two RPCs' logs differ, and does not advance the cursor (`evm.mjs:136-142`). The
  alert key is `evm-disagree:<range>:<counts>` and is not persistent, so it is sent **once**.
- The checker still "completes": `lastOk` is updated in `pollOnce` (`main.mjs:66-70`). So there is no `watcher-error`, and
  `/healthz` stays 200.
- Every later Deposited, Delivered or ItemArrived, including an unbacked mint, is never read.
- A lagging, pruning or lying RPC, or an attacker able to influence one RPC's answer for a single range, therefore blinds the
  canary. `k6-watcher-canary.md` §4 promises G9's "still disagreeing after an hour" re-alert only for Bitcoin.

**PoC.** `monitor/test/audit_e.test.mjs`. RPC A returns one extra harmless log at block +10; both return an unbacked mint at
+6,000. **Result:** over 6 simulated hours (12 polls), one 4663 message (the disagreement), zero UNBACKED MINT alerts, the cursor
stuck at `from_block`, `evm.lastOk = now`. Passes.

**Fix.**

- Make the disagreement key persistent (repeat every 6 h), and add "still disagreeing" after 1 h.
- Treat a cursor that has not moved for more than N minutes while the heads move as a `watcher-error`.
- Consider judging logs that both RPCs agree on beyond the disputed chunk, while flagging the disputed range.

### E-07 (Medium): the Phase-1 guard checks where a transaction goes, not what it says

**Description.**

- `targetRefusal` (`phase.ts:114-132`) allows any stack contract, or any coin or curve the stack's factory launched.
- The sell flow's approval is `coin.approve(router, amount)` with `router = col.contracts.router` from the indexer's
  `/v1/wrapped/<slug>` (`Ticket.tsx:41,71`).
- The target (the coin) passes, and the spender argument is never compared with the bundled stack's router.
- A compromised indexer, its domain or DNS, or a MITM of `VITE_ETCH_API` therefore turns "Allow X to be sold" into an approval
  for the attacker.
- The subsequent `sell` to a non-stack router is refused, but the approval already exists.
- The same pattern would apply to any future call whose arguments carry addresses from the API.

**PoC.** `web-rook/test/audit_e.test.ts`. **Result:** the guard returns `null` (allowed) for
`approve(0x…bad0bad, 1e24)` on the coin; the decoded spender is the indexer's. Passes.

**Fix.** Take `router` (and every address used as an argument) from the bundled stack file, and make `targetRefusal` a
`requestRefusal` that also validates spender and recipient arguments (spender ∈ stack routers; recipient = the connected account).

### E-08 (Medium): monitoring gaps and fail-open alerting

**Description.** Nothing pages on any of the following:

- **Keeper, runner or indexer down or refusing.** Their `/healthz` 503 is consumed only by Railway, and Railway "does not monitor
  the healthcheck endpoint after the deployment has gone live". The Dockerfile `HEALTHCHECK` does not restart anything on Railway.
  `restartPolicyType: ALWAYS` restarts only on exit.
- **The FloorModule or caps going stale.** So the keeper being down for a day is silent, while mint and burn are refused,
  conversions stop, sweeps are refused and draws are skipped.
- **FloorPosted, CapPosted and DrawOpened.** See E-02.
- **The UniSat key being absent.** It is lost at every canister upgrade (`ordinals-apis.md` §2.1); deliveries and buys then refuse
  as "unverified", and nothing tells the operator.
- **Pinned tickets aging.** See E-03.

The reconciler also fails open in two ways:

- **(a) Permanent failure never alerts.** A reconciler that cannot read anything reports "unverified" and never alerts
  (`reconcile.ts:55,140`). Every hourly run refreshes `computedAt`, so the 3-hour "stale is red" never fires either. It also runs
  its watchdog in the same loop as the job (`reconcile.ts:221-227`), so a hung `runOnce` silences it.
- **(b) A missing token is swallowed.** `Telegram` silently becomes a dry run when the token or chat id is missing
  (`alerts.mjs:48`). The reconciler then records the red as sent (`alert()` stores `redKey`). The runbook's alert test,
  `node monitor/src/main.mjs --test-alert all` (`services-v1.md:175`), exercises the **monitor's** variables, not the
  reconciler's, so a typo in the reconciler service goes unnoticed.

**PoC.** `ops/test/audit_e.test.ts`:

- **E-08a:** 168 hourly runs with nothing readable. Served "unverified", `stale=false`, 0 alerts posted.
- **E-08b:** with no token, a RED is served, `dryRun=true`, and `redKey` is recorded. The red is only in the log.

Both pass.

**Fix.**

- Add a monitor checker, in the monitor's project, that pages on:
  - `FloorModule.fresh()==false` or `capFresh()==false` for more than 15 minutes;
  - no FloorPosted for more than 45 minutes;
  - keeper, runner and reconciler `/status` unreachable or older than 3 intervals;
  - "unverified" for more than 6 hours;
  - `ord_providers()` not listing UniSat;
  - any ticket Pinned more than 1 hour past its round.
- Make Telegram fail closed: refuse to start in production without both variables, and add `--test-alert` to the reconciler.

### E-09 (Low): the rate choice is the reverse of what is documented

`AgreedRate` returns the lower sats/ETH (`feed.ts:48-50`). Since wei = sats × 1e18 / (sats per ETH), that is the **higher** floor
in wei, up to +1% (the 100 bps band). `p1-services.md` §1.4 says "The lower is used, which gives a lower floor in wei".

A higher floor means higher `maxItemCost`, higher caps, and higher burn prices (β-capped).

**PoC:** E-09: at 480k sats, +0.95%. Passes.

**Fix:** take the higher sats/ETH (the conservative floor) or the mean, and correct the doc.

### E-10 (Low): no fee bump or replacement for a stuck transaction

**Description.**

- `resolvePending` re-broadcasts the same bytes forever. A refusal ("max fee per gas less than block base fee", "insufficient
  funds") is only logged (`sender.ts:74-77`).
- `submit` then refuses every other call with "a transaction is in flight" (`:101-105`).
- If an ambiguous broadcast is followed by a base-fee rise past the signed `2×base+tip`, the keeper posts nothing until the base
  fee falls back. There is no alert, and no replace-by-nonce, which Defender does automatically (+10% per minute).

**PoC:** E-10: after 3 simulated hours the keeper still reports "still waiting for its receipt" and `module.fresh()=false`. Passes.

**Fix:** after N passes, sign a replacement for the same nonce at a higher fee (within the caps), or a zero-value self-transfer
to clear the nonce, and alert.

### E-11 (Low): draw snapshots are fragile and stored in one place

**Description.**

- `transferLogs` reads `[fromBlock … snapshotBlock]` in 50,000-block chunks and never splits a chunk
  (`draws.ts:40-53`).
- Robinhood's RPC refuses a query matching more than 10,000 logs (measured). A busy coin's chunk, or a -32005/429, throws; see
  E-04.
- `draws()` turns that into "no snapshot", and the slot is **skipped** with no alert (`keeper.ts:154-155`).
- Each draw re-scans from genesis (4663 makes about 850k blocks a day).
- Published snapshots, which are needed for every claim's Merkle proof, live only on the keeper's `/data` volume and are served by
  the keeper host (`status.ts:37-41`). Losing the volume or the domain makes claims impossible to prove, and the host is the hot
  key's host.

**Fix:**

- Split chunks adaptively on "exceeds limit".
- Cache balances incrementally.
- Publish snapshots also to content-addressed storage (and put the hash in `uri`).
- Serve them from a host without the key.
- Alert on a skipped slot.

### E-12 (Low): indexer outbound fetches: hostname-only SSRF guard, unbounded bodies

**Description.**

- `isPublicHost` rejects private IPv4 literals and `.internal`/`.local`/bare names (`catalogSources.ts:316-321`). It does not:
  - resolve DNS and check the IP (for example `10.0.0.5.nip.io`, or a public name pointing at Railway's `fd12::/16` private IPv6);
  - disable redirects (`fetch` follows them to any host);
  - cover 100.64/10 or IPv6 ULA and link-local.
- `metadataOf` (tokenURI) and `getJson` fetch any `https://` URL with no host check at all (`:223-235`, `:25-30`).
- `r.json()` and `r.arrayBuffer()` read whole bodies before the size check (`:352-353`).
- The URLs come from marketplace or tokenURI metadata that collection creators control (API10). The impact is limited: https only,
  images only are served back, and the indexer holds at most `OPENSEA_API_KEY`.

**Fix:** resolve and pin the IP (reject private, loopback, link-local, ULA and CGNAT), set `redirect: "manual"` and re-check each
hop, stream with a byte cap, and apply the same guard to `metadataOf`.

### E-13 (Low): reserves location check is single-provider

`checkInscription` believes ordinals.com's `/r/inscription` alone for the inscription's address and satpoint (`verify.ts:30-47`).
The esploras only confirm that the named output pays custody and is unspent. This is contrary to `ordinals-apis.md` §3 ("two
providers, agreeing, or nothing"). A wrong or compromised ord can:

- place a held frog in any unspent custody output (for example, the G10 drill deposit), keeping green;
- or say "elsewhere", making the page red on demand.

**Fix:** use `VerifiedOrd` (ord plus Ordinals Wallet, plus UniSat) for the location.

### E-14 (Low): small keeper hardening items

- **Exclusions fail silently.** `exclusionsFor` swallows any error reading `exclusions.json` and uses `[]` (`draws.ts:58`). The
  published snapshot then silently includes the Pons contracts as "holders". Fail closed instead.
- **`http:` accepted.** `KEEPER_PUBLIC_URL` accepts `http:` (`config.ts:100`), so an on-chain `uri` could be plain HTTP. Require
  https.
- **Public `/status`.** `/status` is public with CORS `*`: balance, refusals, pending transaction and history (`status.ts:36`).
  This is low sensitivity, but it hands attackers the timing of E-10 and E-04. Consider restricting it.
- **Rehearsal price above the purchase limit.** The offer tool's `MAX_PURCHASE_WEI` = 0.5 ETH (`judge.ts:16`, mirroring
  `policy.rs`). The rehearsal's 0.6 ETH fixture floor would be refused. This is not an issue at today's measured floor (about
  0.13 ETH), but no test covers a floor above 0.5 ETH.

### E-15 (Info): runbooks and code disagree

1. **`services-v1.md` §2, §3, §4 and `k6-watcher-canary.md` §7, §10:** they say `/healthz` is "Railway's health check" as if it
   were continuous. It is deploy-time only (E-08).
2. **`services-v1.md` §5 step 1:** the reconciler's alert test uses the monitor's command (E-08b).
3. **`launch-gates.md:410`:** "Keeper key stolen → propose new roles (7 days)" does not recover the FloorModule, whose keeper is
   immutable (E-02).
4. **`p1-services.md` §1.4:** the rate direction is stated backwards (E-09).
5. **`web-rook/Dockerfile:1`:** it says `railway up ./web --path-as-root`; the Phase-1 site is `web-rook/`. In Docker,
   `sync-content.mjs` copies nothing, so `web-rook/src/deployments/stack-4663.json` must be **committed**. Go-live step 13 says
   "npm run sync", which is local only. Without the commit, the build fails closed and cannot send; this is safe but a surprise.
6. **The redemption flow as documented** (`Redeem.tsx:51`: "anyone proves it on chain") has no prover (E-03).
7. **`p1-services.md` §6.5:** it acknowledges that "unverified is not alerted"; E-08a shows this is permanent silence, not "a
   provider's bad hour".

### E-16 (Info): dependency audit

`npm audit` results, run after `npm ci`:

| Package | Result |
| --- | --- |
| keeper | 0 |
| ops | 0 |
| monitor | 0 |
| indexer | 0 |
| tools/snapshot | 0 |
| staging | 0 |
| web-rook | 25 advisories (23 moderate, 2 high); with `--omit=dev`, 23 (22 moderate, 1 high) |

**Detail for web-rook:**

- **`vite` (high, dev and build only):**
  - GHSA-4w7w-66w2-5vf9 (path traversal in optimized-deps `.map` handling), dev server only;
  - `esbuild` GHSA-67mh-4wv8-2f99, dev server only.

  Neither ships.
- **`ws` 8.x (high):** GHSA-58qx-3vcg-4xpx and GHSA-96hv-2xvq-fx4p. It is transitive through viem and the WalletConnect, Reown
  and MetaMask SDK packages. `Web3.tsx` configures only `injected()` from `@wagmi/core`, so those connectors are installed but not
  bundled, and `ws` does not run in the browser.
- **Moderates:** `@metamask/*`, `@walletconnect/*`, `@reown/*`, `@gemini-wallet/core`, `decode-uri-component`, `query-string`,
  `uuid`. All come through wagmi's connector dependency tree.

**Recommendations:**

- Pin exact versions for the shipped bundle: `wagmi`, `viem`, `react`, `marked` and `lightweight-charts` are all `^` ranges.
- Upgrade vite (5 → a fixed line).
- Consider `@wagmi/core` plus viem without the `wagmi` connectors package, to shrink the supply-chain surface.

**Image hardening:**

- The keeper and monitor images run as root (no `USER`).
- The keeper image installs dev dependencies (`npm ci` without `--omit=dev`), because it runs under `tsx`.
- The web image uses `USER node`.

**Other checks:**

- No secret was found committed. The 32-byte hex constants in `autobuyer/scripts/*-rehearsal.sh` are block-hash checkpoints and an
  anvil dev key.
- CSP on `web-rook/server.mjs` is strict: `script-src 'self'`, `frame-ancestors 'none'`.
- `dangerouslySetInnerHTML` is used only on build-time repository markdown (Trust, Docs).
- Token logos are cleaned (`indexer/src/logo.ts`); React escapes names and symbols. No XSS was found from token metadata.

## 5. What held up (checked, no finding)

**Keeper (key handling, chain and send safety):**

- Key isolation: `signer.ts` is the only reader, and the scrubber and tests hold.
- The chain id is pinned per RPC and re-asked before a send.
- The key must equal `registry.keeper()` and every module's keeper, and must have no code.
- YOYO is never posted.
- The fixture floor is refused on 4663.
- Journal-then-send, with the same bytes on restart.
- Gas and fee caps refuse rather than lower.

**Runner, offer tool and monitor:**

- The runner never overlaps ticks.
- The offer tool's TypeScript `judge` mirrors the canister rules, and nothing is sent without `--send`.
- The monitor's argument diff, certified controllers and Bitcoin canary logic (txid-matched, two esploras, one-sided outbound →
  SEV1).

**Site:**

- The Phase-1 site hides the OFF paths.
- The Phase-1 site validates the bundled stack.
- The Phase-1 site checks bech32m before redeem.

**Snapshots:** they are deterministic and reproduced byte for byte by `tools/snapshot`, confirmed in the repo's own fork run. The
slot block rule is published in advance.

## 6. PoCs added (all pass; run from each package after `npm ci`)

| File | Tests | Command |
| --- | --- | --- |
| `keeper/test/audit_e.test.ts` | E-01, E-01b, E-02, E-03 (static), E-04, E-09, E-10 | `cd keeper && npx tsx --test test/audit_e.test.ts` |
| `indexer/test/audit_e.test.ts` | E-05 ×3 | `cd indexer && node --experimental-sqlite --no-warnings --import tsx --test test/audit_e.test.ts` |
| `monitor/test/audit_e.test.mjs` | E-06 | `cd monitor && node --test test/audit_e.test.mjs` |
| `web-rook/test/audit_e.test.ts` | E-07 ×2 | `cd web-rook && npx tsx --test test/audit_e.test.ts` |
| `ops/test/audit_e.test.ts` | E-08a, E-08b | `cd ops && node --experimental-sqlite --no-warnings --import tsx --test test/audit_e.test.ts` |

**Measured on 4663 (read-only, 2026-09-25/26):**

- `rpc.mainnet.chain.robinhood.com` returns `-32000 "logs matched by query exceeds limit of 10000"` and `429 Too Many Requests` as
  JSON-RPC errors in HTTP 200.
- `rpc.ordofi.network` returns `-32005 "the network is busy, please try again in a moment"` for 50k-block `eth_getLogs`.

## 7. Test gaps

**Keeper:**

- no test of `planPost` with a posted confidence below the minimum, or with `fresh()==false` at the same price (E-02);
- no adversarial sales in the floor tests (wash sales at ≥ L/4, many fresh wallets) (E-01);
- no test of a pass with many launched coins, or of a JSON-RPC 429 / -32005 fallback (E-04);
- no test of a rebroadcast refusal (E-10);
- no test of a `transferLogs` "exceeds limit" error (E-11).

**Monitor:** no test of a disagreement persisting across polls, or of the cursor stall (E-06).

**Indexer / reconciler:**

- no test that compares escrow to the module's balance or treats a stale floor (E-05);
- no test of permanent "unverified", of Telegram being unset, or of a hung `runOnce` versus the watchdog (E-08).

**Web:** no test that `request()` arguments (spender, recipient) come from the stack (E-07).

**End to end:** no test that a redeem on a fork reaches Resolved without the rehearsal script's manual `resolve` (E-03); the fork
run does resolve and claim by hand.

**Site:** no browser or e2e test of the Phase-1 TxButton against a hostile indexer answer.

## 8. Not covered

- The canister internals: frogflow, frogbuy, redeem signing, H8 conflict-out, ordnet transforms. The timelock.
- Contract correctness beyond what a workflow needs. In particular FloorModule's pricing math, SweepVault's sweep route validation,
  and CurveFactory/FrogCurve. I read them only for the lines cited.
- Live UniSat behaviour: no key, and the shapes are unverified per the docs.
- The Ordinals Wallet escrow mechanics: whether an "escrow" can be spent while listed.
- Railway project and account configuration, DNS/registrar (H14), and the operator devices.
- `web/` (the legacy site), `staging/` beyond the resolve search, `autobuyer/` scripts, and `tools/snapshot` beyond its use by
  the keeper.
- No fork or replica runs: `scripts/keeper-fork-test.sh` and `ops/scripts/runner-replica-test.sh` were not re-run.
