# Cycles from fees: the platform's fee income pays for the canister's cycles

Written 2026-09-28 on branch `cycles-from-fees`, from `main` at `f8db5ef`. **Docs only: no code, nothing deployed, signed or sent.**
The only network reads were public and read-only: CoinGecko's price API, ICPSwap's pool statistics API and Relay's quote API
(`POST /quote`, user `0x…dEaD`, nothing signed). Web facts are cited in §7. Prices on 2026-09-28: **ETH US$2,675, ICP US$3.00**
(CoinGecko), **1 T cycles = 1 XDR ≈ US$1.35** (golive-runbook §6), so 1 T ≈ 0.45 ICP.

**The operator's decision (2026-09-28):** after a small first seed, Rook's own fee revenue pays for its IC cycles, automatically,
with caps and pages. The operator does not keep paying.

**Short answer.**

- **The platform's money arrives as ETH on Robinhood Chain (4663) at one code-less EOA, the treasury's `ops` address.** It gets
  10 % of `ProtocolTreasury`'s income at the stage-0 shares (reserves 90 / floats 0 / bootstrap 0 / ops 10). The other 90 % has no
  discretionary exit. The treasury's income is the platform's 20 % of every trade fee (0.2 % of volume at the default 1 % tier),
  the graduation fee (5 % of the raise, capped at 0.25 ETH), the modules' skims and the protocol liquidity's ETH fees.
- **At stage-0 shares the ops share covers the normal burn from about US$2M a month of volume**, or about US$0.6M a month plus one
  graduation a week. It covers the worst-case burn only from about US$12M a month (§1.3).
- **KongSwap sunset on 2026-04-06.** ICPSwap is the only IC DEX route checked here. Its ICP/ckUSDC pool is deep (TVL ≈ $742k) and
  its ICP/ckETH pool is thin (≈ $26k). **The CMC mints cycles from ICP only.** No route was found that turns a ck-token into cycles
  without going through ICP (§2.4).
- **Recommendation:** v0 now is a runbook plus the monitor, with no code. v1, after 30 days of measured burn and income, is a small
  capped off-chain "fueler" service. It is the ops address itself: it claims and allocates the fees, keeps a capped buffer,
  forwards the rest to the operator's cold address, and runs **4663 ETH → Relay → USDC on Ethereum → ckUSDC → ICPSwap → ICP →
  CMC `notify_top_up(buyer)`**, only when the buyer is under 100 T. A later v2 can replace the hot key with a separate keyless
  "fuel" canister. **Never the buyer:** fee ETH must never reach the buyer's addresses (§2.1).

---

## 1. Where the money is

### 1.1 The fee streams, the asset and the chain

| Stream | Rate (launch template, `script/DeployStack.s.sol` `_template`) | Lands in | Asset / chain |
| --- | --- | --- | --- |
| Curve trade fee, platform share | 20 % of the fee (`vault 40 / creator 30 / reserve 5 / liquidity 5`, the platform takes the rest, `docs/v5/floor-module.md` §13.2): **0.2 % of volume** at the default 1 % tier (up to 1.6 % at 8 %) | `FrogCurve.claimPlatformFees()` → `platform` = `ProtocolTreasury` | ETH, 4663 |
| Coin-pool LP fees, platform share | same split, ETH side only (`onPoolFees`, `forwardWrappedFees`) | treasury | ETH, 4663 |
| Graduation fee | `min(raise × 5 %, 0.25 ETH)` (`platformBps 500`, `platformFeeCap 0.25 ether`): **0.25 ETH** at the 5, 8 and 10 ETH raises | treasury (same route) | ETH, 4663 |
| Module spread revenue | `FloorModule.skim()` → `platform` | treasury | ETH, 4663 |
| Protocol liquidity's ETH fees | `LiquidityLock` (`wireTreasury`) | treasury | ETH, 4663 |

**The treasury pays out only by fixed shares** (`src/floor/ProtocolTreasury.sol`). It has no withdraw and no recipient
parameter. `allocate()` (anyone) splits what arrived into reserves, floats and bootstrap, **and pays the ops share at once to `ops`**.
At stage 0, `golive-check.sh` holds the shares to **9000 / 0 / 0 / 1000** and `ops` must be a code-less key (`nocode`). An EIP-7702
delegation is flagged. So the only fee money the platform can spend is **10 % of treasury income, as ETH on 4663, at one EOA**.

**Who controls it:**

- The **Safe** (2-of-2, the registry's owner) sets `ops` and the shares with `setOps` and `setShares`. Both take effect **at once**,
  with no 7-day delay (`docs/v5/launch-gates.md`, corrected 2026-09-25).
- Whoever holds the `ops` key spends what reaches it. Today the docs do not name that key's holder beyond "the ops address"
  (`ETCH_OPS`, and the deployer's leftovers go back to it, `docs/deploy/key-ceremony.md` step 5). Unset, the ops share joins the
  reserves pot and **the platform receives nothing spendable**.

**Nothing claims or allocates automatically today.** No service in `ops/`, `keeper/` or `monitor/` calls `claimPlatformFees`,
`flushProtocolFees` or `allocate` (grep on `f8db5ef`). All three are anyone-callable. Until someone calls them, ops receives nothing.

**Not platform money:**

- The canister's **Ethereum gas pot** holds purchase leftovers, topped up to 0.05 ETH (`gas_pot_target_wei`). Above that it is
  swept back through Relay to **each collection's own module** (`fundReserve`, D3 amended, A2-L-1). It is booked per collection
  and belongs to the holders' side. It is not a source for cycles.
- The creator's 30 % goes to each coin's creator.
- The decaying launch fee goes wholly to the coin's sweep vault.

### 1.2 Volume → ops income vs cycles burn

The burn figures are from golive-runbook §6: 13-node subnet, five collections, 6–10 T/day normal and ≈ 60 T/day worst. A 34-node
subnet is 16–27 T normal and ≈ 160 T worst. "Volume" means ETH-side trade volume on the curves and coin pools at the 1 % tier. The
route cost is ≈ 2 % (§2).

| Monthly volume | Volume / day | Treasury (0.2 %) / day | **Ops (10 %) / day** | Ops as cycles (after ≈ 2 %) | Normal burn 6–10 T ($8–14) | Worst 60 T ($81) |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| $1M | $33.3k | $66.7 | **$6.67** | ≈ 4.8 T | **short** (48–81 %) | short (8 %) |
| $5M | $167k | $333 | **$33.3** | ≈ 24 T | covered 2.4–4× | short (40 %) |
| $20M | $667k | $1,333 | **$133** | ≈ 97 T | covered ≈ 10× | covered 1.6× |

**Each graduation** adds 0.25 ETH ($669) to the treasury: **0.025 ETH ($67 ≈ 49 T) to ops**, or about five days of normal burn.

Higher fee tiers scale the curve and pool rows in proportion (a 3 % coin pays 3×).

### 1.3 Break-even

- **Normal burn (10 T ≈ $13.8/day with conversion):** ops must earn about $13.8/day. That is treasury income of about $138/day, or
  about **$2.1M a month of volume** with no graduations. One graduation a week (≈ $9.6/day to ops) brings it down to about
  **$0.6M a month**.
- **Worst case (60 T ≈ $83/day):** about **$12M a month**.
- On a **34-node subnet**, multiply by 2.7.

**If income falls short**, the Safe can raise `opsBps` (for example to 2000). The change is immediate and public. It takes money
from the collection reserves, so the site's Trust page must say so. Recommend it only after 30 days of measured income and burn,
and not before launch (§5.2).

---

## 2. Routes from 4663 ETH to cycles

Facts used by every route:

- The **CMC** (`rkp4c-7iaaa-aaaaa-aaaca-cai`) mints cycles **from ICP only**. `notify_top_up({block_index, canister_id})` names an
  ICP-ledger block, a transfer to the CMC's account under the target canister's subaccount with memo `TPUP`. The `.did` has no
  other currency.
- **Anyone can top up any canister**, with no controller rights (runbook §6). A route therefore never needs a role on the buyer or
  the timelock.
- **ckETH / ckUSDC:** the minter is `sv3dd-oaaaa-aaaar-qacoa-cai` and the ckETH ledger is `ss2fx-dyaaa-aaaar-qacoq-cai`.
  - Deposits go through the helper contract on Ethereum (`0x18901044688D3756C35Ed2b36D93e6a5B8e00E68`: `depositEth` / `depositErc20`
    with a `bytes32` principal and a subaccount). **Verify the address with the minter's `get_minter_info` before use.**
  - Minting takes about 20 minutes.
  - The ckETH transfer fee is 0.000002. Withdrawals have a 0.03 ETH minimum; the routes here never withdraw.
- **ICPSwap** (read 2026-09-28):

  | Pool | Canister | TVL | 24 h volume |
  | --- | --- | ---: | ---: |
  | ICP/ckUSDC | `mohjv-bqaaa-aaaag-qjyia-cai` | ≈ $742k | ≈ $407k |
  | ICP/ckETH | `angxa-baaaa-aaaag-qcvnq-cai` | ≈ $25.6k | ≈ $8.2k |

  The flow is quote → ICRC-2 approve → `depositFrom` → `swap({amountIn, zeroForOne, amountOutMinimum})` → `withdraw`. `quote` is a
  query with the same arguments.
- **KongSwap sunset on 2026-04-06.** Its site and API return HTTP 530 today. Do not build on it.
- **Relay 4663 → Ethereum** (quote 2026-09-28, 0.05 ETH in, $133.66): ETH out 0.049659 (−0.72 %, ≈ 4 s), or **USDC out 132.34
  (−0.99 %)**. Fees ≈ $0.9–1.1. The canister's Relay judge already allows up to 3 %.
- **Ethereum gas** is negligible at the measured 0.07 gwei base (phase1-nft §8): approve plus deposit is well under $0.10.

**Cost of converting $100** is about 2 %, whichever way the ETH reaches ICP:

- **ETH → USDC path:** Relay ≈ 1 %, then ckUSDC→ICP at 0.3 % plus small impact, then the ICP fee 0.0001.
- **ETH → ETH path:** Relay ≈ 0.7 %, then ckETH→ICP at 0.3 % plus about 0.5–1 % impact at $100–200 in the thin pool.

Use the USDC leg above about $200 a trade, or when the ckETH pool's quote impact is over 1 %.

### 2.1 (a) The canister does it itself

The idea is that the buyer uses its threshold-ECDSA address: `depositEth` naming its own principal, then an ICPSwap swap, then
`notify_top_up(self)`.

**Rejected for the buyer.**

1. **The income isn't at the canister.** `ops` would have to be the buyer's EVM address. The buyer's Ethereum and 4663 flows are
   *balance-based* (arrivals, refunds, watch starts, phase1-nft §2.4). Phase1-nft already notes "a stranger's gift can be mistaken
   for an arrival". Fee ETH landing there would corrupt that accounting.
2. It adds new signing paths, new callees (the ckETH helper, ICPSwap, the ICP ledger, the CMC) and new state to the custody
   canister. Every change is a 72 h timelocked upgrade plus a full re-review of the canister that holds up to 75 ETH of NFTs.

**Kept as a variant, (a′): a separate "fuel" canister.**

- Its own threshold-ECDSA key gives one code-less address on both chains, so it passes `nocode`. That address *is* `ops`.
- It holds no custody and is controlled by the timelock.
- Each run, when the buyer is under 100 T:
  1. It signs a Relay deposit 4663 → Ethereum to itself, judged by a copy of `relayeth::judge`.
  2. It signs `depositErc20` or `depositEth` to the helper, naming its own principal.
  3. It swaps on ICPSwap with `amountOutMinimum` from its own oracle: the IC's exchange-rate canister (XRC) for ETH/ICP, about 1 B
     cycles a call.
  4. It pays the CMC and calls `notify_top_up` for the buyer and the timelock.
  5. It forwards ETH above its buffer to one install-time payee (the operator's cold address).
- **Pros:** no hot key anywhere; rules in code, visible and timelocked.
- **Cons:** a new canister and a new audit. The signing code is patterned on the audited buyer but is new, and the ICPSwap and CMC
  integrations are new. It burns cycles itself: outcalls, signatures at 26 B each, and about 4 signatures a run. Weeks of work.

### 2.2 (b) An off-chain service with an operator-provided key

A Railway service, the "fueler":

- It has its own sealed EOA key, which is `ops`, and its own IC identity. **Not** the runner's `OPERATOR_IDENTITY_PEM`.
- Each run it:
  1. calls `claimPlatformFees` on each curve and `allocate` on the treasury (4663 gas is small);
  2. above its buffer, forwards the excess ETH to the operator's cold payee;
  3. when the buyer is under 100 T: Relay 4663 → USDC on Ethereum to its own address, then approve plus `depositErc20` naming its
     own principal, then an ICPSwap swap to ICP, then an ICP transfer to the CMC plus `notify_top_up(buyer)`, with `@dfinity/agent`.
- It reuses `ops/src/relay.ts` (the quote encoder and its judge rules) and the monitor's cycles reads.
- **Pros:** no canister change and no timelock cycle. TypeScript in a stack the repo already runs and tests. Days, not weeks.
- **Cons:** it is a **hot key on money and trust**.
  - A stolen key takes the buffer and future ops income until the Safe calls `setOps` (immediate).
  - A buggy or compromised service can waste up to its caps.
  - Bound: the buffer cap plus the weekly cap (§3). Watch: every outgoing transaction of `ops` must match the fueler's journal (§3.4).

### 2.3 (c) Semi-manual: the monitor pages, the operator runs a runbook

The monitor's `burnCheck` already pages on days-left, rate and balance. The operator converts from the ops wallet by hand, weekly
or on a page. The v0 runbook is in §4.1.

- **Pros:** zero code, zero new keys, available today.
- **Cons:** the operator's time, about 20 minutes a week, and the IC side needs the operator's dfx identity. It is not automatic,
  which is the point of the decision. It is the right v0 and the permanent fallback.

### 2.4 (d) Buying cycles with ck-tokens directly

**Not found as of 2026-09-28.**

| Candidate | Finding |
| --- | --- |
| CMC | Takes ICP-ledger blocks only (`cmc.did`) |
| Cycles ledger | Holds and moves cycles (`deposit`, `withdraw` to a canister, `create_canister`). Minting cycles into it is ICP → CMC `notify_mint_cycles` |
| CycleOps | Pays top-ups in **ICP or cycles**, with a 7 % margin (FAQ). No ck-tokens. It is useful as a v2 *executor*: fund a CycleOps account in ICP and it tops up every 6 h (its balance checker is black-holed), but the ETH → ICP step remains |
| Cycle.Express | Card via Stripe to a canister. Fiat only |
| CyclePay | Stripe; the project **deleted** its ck-USDC rail. Pre-production |
| Newer IC mechanisms | No DFINITY doc found that lets a canister pay for cycles in ckETH or ckUSDC |

The OISY wallet has a `CyclesMint` transaction type (PR #14129); it was not examined here and is a wallet feature in any case.

**So every route goes through ICP and the CMC.** The choice is only who holds the key between 4663 ETH and ICP.

### 2.5 Comparison

| | (a′) fuel canister | (b) fueler service | (c) runbook | (d) ck-tokens direct |
| --- | --- | --- | --- | --- |
| Hot key | none | yes (ops EOA + IC identity) | the operator's own wallets | — |
| Touches the buyer | no | no | no | — |
| Build | weeks + audit | days + review | none | does not exist |
| Loss if compromised | a timelocked upgrade's worth (visible 72 h) | buffer + caps (§3) | the operator's wallet | — |
| Automatic | yes | yes | no | — |
| Cost per $100 | ≈ 2 % + ≈ 0.1 T of its own cycles a run | ≈ 2 % | ≈ 2 % (or a CEX's spread) | — |

---

## 3. Recommendation: the caps and the rules

**v1 is (b), the fueler, with these caps.** They are held in code, persisted on its `/data` volume, and published on `/status`.

### 3.1 When it acts

| Rule | Value |
| --- | --- |
| Top up the **buyer** | only when the buyer's balance is **under 100 T**, and only **to 250 T** (at most 150 T a top-up). That is the runbook §6 policy, now automatic |
| Top up the **timelock** | only when it is under 1.5 T, to 2 T |
| Minimum between runs | 6 h. Check every 30 min; act at most once in 6 h |
| On a 34-node subnet | ×2.5 on every amount, as the runbook says |

### 3.2 How much

| Cap | Value | Why |
| --- | --- | --- |
| ETH converted a day | **≤ 0.10 ETH** (≈ $267 ≈ 195 T) | above one full top-up; below a day of the 34-node worst case |
| ETH converted over 7 days | **≤ 0.25 ETH** (≈ 490 T) | ≈ 8 days of the 13-node worst case. Hitting it pages (runaway burn, a bug or an attack on the scan); it stops and a human decides |
| Floor on the ops wallet | **never spend below 0.02 ETH** | its own 4663 gas for claims, allocates and forwards |
| Buffer | keep **≤ 0.15 ETH** at `ops` (≈ $400 ≈ 290 T: about two full top-ups); forward everything above it to the cold payee each run | bounds a stolen key's take |
| Hot IC balances | at most one run's worth (ckUSDC, ckETH, ICP); anything left after a run is retried next run, never accumulated | |

### 3.3 Price and slippage

- **Oracle sanity:** two sources for ETH/USD and ICP/USD (CoinGecko plus Coinbase's public ticker) must agree within 2 %. The
  median is the reference. If either is missing, the run does nothing and pages.
- **Relay leg:** min-out ≥ 98.5 % of the oracle value. The existing judge allows 3 %; the fueler is stricter because it has no
  deadline pressure.
- **ICPSwap leg:**
  - `amountOutMinimum` = quote × 99.5 %, and the quote must be within **1.5 %** of the oracle's ICP.
  - If the ckETH pool's impact is over 1 %, route through ckUSDC.
  - Refuse a trade above 2 % of the pool's TVL.
- **End to end:** refuse if the expected T per ETH is below **95 %** of `ETH/USD ÷ XDR/USD`, using the CMC's
  `get_icp_xdr_conversion_rate` for the cycles side.
- **Check after the top-up:** the buyer's balance rose by the expected T, within the burn over the elapsed time. Otherwise page.
  `notify_top_up` with the same block is idempotent at the CMC, so a retry is safe.

### 3.4 When the route is down

Every leg is resumable from the journal on `/data`: Relay in flight, the helper deposit mined, ckUSDC minted, the swap, the ICP
sent, notified.

| Event | Page | What the operator does |
| --- | --- | --- |
| a leg fails twice, or a leg is stuck > 2 h (ckUSDC mint > 1 h) | **SEV2 `cycles-autofund-failed`** | the §4.1 runbook by hand from the fallback reserve |
| a cap hit (7-day) | **SEV2 `cycles-autofund-cap`** | read `/status` → `cycles`: is the burn real? Raise the cap in config (a redeploy) or fix the cause |
| ops income < the burn over 14 days | **SEV3 `cycles-income-short`** | §1.3: the opsBps question, or keep topping up by hand |
| an `ops` transaction not in the fueler's journal | **SEV1 `ops-unexpected-tx`** (persistent) | treat the key as stolen: the Safe `setOps(<new>)` at once, rotate the service key |

**The fallback reserve:** keep **≥ 100 T** in the operator's cycles ledger (≈ $135; the runbook's 300 T until v1 has made two
clean top-ups). Also keep the `dfx cycles top-up` command with the operator and the guardians.

### 3.5 How it fits the existing pages (`burnCheck`, `monitor-nft.json` `cycles`)

The thresholds are ordered so the fueler acts first and the pages mean "the fueler did not":

| Level | What happens |
| --- | --- |
| **100 T** | the fueler tops up |
| 7 days left at the measured rate | `burnCheck` SEV2 (≈ 70 T at 10 T/day) |
| 20 T | `burnCheck` SEV2 |
| 2 days left | `burnCheck` SEV1 |
| 8 T | the canister stops its own sale scan |
| 5 T | the canister's own level page |

- **A top-up is never counted against the rate**, as already tested in `monitor/test/cycles_burn.test.mjs`.
- At the worst case (60 T/day), `burnCheck` pages on the rate (> 30 T/day) whatever the fueler does. That is intended: the fueler
  keeps the canister alive, and the human decides whether the burn is legitimate.
- The fueler reads balances from the same source as the monitor (`canister_status` through the blackhole, or the monitor's
  `/status`), never its own guess.
- Add three rows to the monitor configuration in v1, with their tests: the fueler's `/healthz`, `ops` balance and journal.

---

## 4. Security review scope and the phased plan

### 4.1 v0: now, before and at launch (no new code)

**Before the handover:**

1. The Safe sets `ops` to an operator-held EOA: `ETCH_OPS` at deploy, or `setOps`. Check it with `golive-check.sh`: no code, not
   the owner.
2. Seed the cycles (§5).

**Weekly, on a Monday, or on any `burnCheck` page:**

1. **Claim and allocate** (anyone; from the ops key, 4663 gas only):
   - `cast send <curve> "claimPlatformFees()"` for every curve with fees, as listed by the indexer's curve list;
   - then `cast send <treasury> "allocate()"`.
   - Check: `ops()` received 10 %, from the `Allocated` event.
2. **If the buyer is under 100 T** (`/status` → `cycles`), convert about 150 T (≈ $203 ≈ 0.076 ETH):
   1. **Relay 4663 → Ethereum**, ETH → USDC to the operator's own Ethereum address (relay.link). Accept min-out ≥ 98.5 %.
   2. **ckUSDC:** `dfx canister call sv3dd-oaaaa-aaaar-qacoa-cai get_minter_info --network ic` and confirm the helper address.
      Then approve USDC to it and call `depositErc20(USDC, amount, <operator principal as bytes32>, 0x0…0)`. Wait about 20 min.
   3. **ICPSwap:** swap ckUSDC → ICP in pool `mohjv-bqaaa-aaaag-qjyia-cai`, `amountOutMinimum` at 99.5 % of the quote, and withdraw
      to the operator's dfx principal.
   4. **Cycles:** `dfx cycles convert --amount <ICP> --network ic`, then
      `dfx cycles top-up <buyer> 150000000000000 --network ic`.

   The alternative for steps 1–3 is a CEX that lists ICP: send ETH, buy ICP, withdraw to the dfx account.
3. **Log it** in `ops/cycles-log.md`: date, ETH in, T out, and the effective rate.
   - This log is v1's evidence (actual cost per T and actual weekly burn).
   - `ops/cycles-log.md` is the operator's file, created on the first run. This branch does not add it.
4. **After 30 days,** compare the ops income with the measured burn (§1.3). Set `burn_warn_t_per_day` to three times the measured
   normal (runbook §6).

### 4.2 v1: the fueler (automated route (b))

**Start** after v0 has run 30 days, the measured burn is known, and ops income covers at least the normal burn (or the operator
decides to fund the gap).

**Build:** `ops/src/fueler.ts` and its own Railway service in the monitor's project, with one replica and a `/data` volume. It gets
its own sealed variables: `FUELER_EVM_KEY` (= `ops`), `FUELER_IC_IDENTITY_PEM` and `FUELER_PAYEE` (cold). Add it to
`deployments/launch/railway/` and `check-env.mjs`.

**Security review scope (v1):**

1. **Destinations are pinned, not read from anywhere mutable.** The only allowed destinations are:
   - on 4663: the Relay depository (the existing judge rules, one payment to its own Ethereum address, no calls), the curves'
     `claimPlatformFees`, the treasury's `allocate`, and `FUELER_PAYEE`;
   - on Ethereum: USDC `approve` / `depositErc20` to the helper pinned in config and re-checked against `get_minter_info` each run
     (a mismatch halts and pages);
   - on the IC: the ICPSwap pool canister(s) pinned by id, the ICP ledger, and the CMC with `canister_id` ∈ {buyer, timelock}.

   Any other calldata is refused. There must be no generic "send" in the code.
2. **Every intermediate recipient is itself.** Relay's recipient, the helper's principal and the ICPSwap withdraw target are all its
   own. Test that no code path can name a third party except `FUELER_PAYEE` and the CMC.
3. **The caps are held in code and on disk:** §3.2, across restarts and in the journal, with tests for each cap, the floor, the
   buffer forwarding and the 6 h spacing.
4. **Price sanity (§3.3):** oracle disagreement, stale quotes, the pool-impact limit and the end-to-end rate check. Include a
   manipulated-pool test (the quote far from the oracle → refuse).
5. **Idempotency and resume:** every leg is recorded before and after. A crash between "sent" and "noted" never pays twice:
   Relay by request id, the helper deposit by transaction hash, the swap by ICPSwap's per-user balance, and `notify_top_up` by block
   index. This is the reconciler's lesson of phase1-nft §5 ("a wrong 'not mined' pays twice").
6. **Keys:** sealed service variables and never logged, extending the scrubber in `ops/src/log.ts`. The key is not shared with the
   runner, keeper or guardian. Check for EIP-7702 delegation on `ops` at start (refuse if delegated). Write the rotation runbook:
   the Safe `setOps(new)`, then swap the service variable.
7. **Monitor integration:** the four pages of §3.4, and the `ops-unexpected-tx` watch in the monitor's project, not the fueler's.
8. **Trust statement:** the site's Trust page states that the ops share (10 %) is operator income, partly converted to cycles by an
   automated hot key with these caps.

**Out of scope for v1** (unchanged): the buyer, the timelock, `ProtocolTreasury`, the curves, and every canister signing path.

### 4.2.1 As built (2026-10-09, branch `golive-2026-10-08`)

The operator asked for the fueler at launch rather than after 30 days of v0. It is built with the first month's small caps, so the
30 days of measured burn and income tune the caps instead of gating the build. **Nothing has been deployed, signed or sent.** Every
network read made while building it was read-only (Relay quotes from `0x…dEaD`, IC queries, Sourcify, JSON-RPC reads).

**Where it is:** `ops/src/fueler.ts` (the service), `ops/src/fuel/` (`pinned.ts` every destination, `plan.ts` the caps and price checks,
`engine.ts` the journal, the allowlist and the run, `io.ts` the chains, Relay, the prices and the IC, `keys.ts` the two secrets),
`ops/Dockerfile.fueler`, `ops/railway.fueler.json`, `deployments/launch/railway/fueler.env.example`, `monitor/src/fueler.mjs`, and a
Trust page section (`web-rook/src/rook/phase1Trust.ts`, id `fueler`).

**Where it differs from the design above, and why:**

| Design | As built | Why |
| --- | --- | --- |
| ETH → USDC on Ethereum → ckUSDC → ICP | ETH → ETH on Ethereum → `depositEth` → ckETH → ICP (pool `angxa-baaaa-aaaag-qcvnq-cai`, token0 ICP, token1 ckETH) | A run is at most 20 T ≈ 0.012 ETH ≈ $30, far under §2's $200 threshold for the USDC leg. No ERC-20 approve on Ethereum, and the deposit's gas comes out of the bridged ETH, so the Ethereum address keeps nothing. The thin pool is bounded by the quote-against-oracle check (1.5 %), which also stands in for the "2 % of TVL" rule |
| Relay `POST /quote` | `POST /quote/v2` with `slippageTolerance: "50"` and `x-api-key` | `/quote` is deprecated in Relay's spec; `/quote/v2` needs a key from 2026-10-02. **Measured 2026-10-09:** with Relay's automatic slippage the guaranteed minimum is 2.3 % under the amount and §3.3's 1.5 % bound refuses every quote; at 0.5 % it is 0.74–0.91 % under (0.005–0.02 ETH) and the bound holds. The judge is the runner's own `preJudgeEth` at 150 bps |
| Act under 100 T, to 250 T, ≤ 150 T a top-up | Act under 40 T, toward 60 T, ≤ 20 T a run, ≤ 60 T in 7 days (environment overrides) | The dormant model's launch (§11 of dormant-collections.md: 34 T at the handover). The design's ETH caps (0.10 ETH a day, 0.25 ETH in 7 days), floor (0.02 ETH) and buffer (0.15 ETH) are kept |
| Gas | 4663: tip 0, max fee 3× base. Ethereum: tip 0.5 gwei, max fee 3× base + tip, refused over 50 gwei; the deposit's gas limit at most 120,000 | A transaction is never re-signed (§4.2 item 5), so its fee must be generous once |

**Facts it rests on, read 2026-10-09:**

- `get_minter_info`: `deposit_with_subaccount_helper_contract_address` = `0x18901044688D3756C35Ed2b36D93e6a5B8e00E68` (pinned, re-read every run;
  a mismatch halts the fueler and pages SEV1); no minimum is listed for ETH deposits (the ckERC20 minimums do not apply);
  withdrawals' minimum is 0.005 ETH (the fueler never withdraws).
- The helper's ABI (Sourcify): `depositEth(bytes32 principal, bytes32 subaccount)`. The principal's 32 bytes are its length then its bytes
  (dfinity/ic `encode_principal`).
- The ICP account identifier and the CMC's top-up account (`Subaccount::from(&PrincipalId)`) are held to vectors `dfx ledger account-id`
  printed. The top-up is the ICP ledger's legacy `transfer` with memo `TPUP` and a journaled `created_at_time`, then `notify_top_up`.
- The CMC's rate: 22,322 ten-thousandths of an XDR per ICP (2.2322 T cycles an ICP).

**The review scope of §4.2, item by item:**

1. Destinations are constants in `pinned.ts`; `engine.ts` `allowed()` is the only gate to `sign`, and it admits five transactions: the Relay
   deposit naming the fueler itself, a curve's `claimPlatformFees()` (value 0), the treasury's `allocate()` (value 0), a plain transfer to
   `FUELER_PAYEE`, and the helper's `depositEth` naming the fueler's own principal. Everything else is refused before signing (tested).
2. Every intermediate recipient is the fueler: Relay's recipient and refunds, the helper's principal, the pool's withdraw (to the caller).
   A top-up naming a canister other than the buyer or the timelock halts.
3. The caps are in `plan.ts`, applied from the journal's conversions, which survive restarts (tested per cap).
4. Prices: CoinGecko and Coinbase within 2 %; the pool's quote within 1.5 % of the oracle, `amountOutMinimum` the quote less 0.5 %; end to end
   at least 95 % of the ETH's worth at the oracle and the CMC; the canister's balance read again after the top-up.
5. Idempotency: an EVM transaction is signed with the next nonce and journaled before its broadcast, and only the same bytes are broadcast
   again; the run's phase is written in the same save as the signed transaction; the pool's legs are read back from its per-user balance;
   the ICP transfer is deduplicated by the ledger on its created-at time; `notify_top_up` is idempotent. A crash at each of eight points is
   tested: one Relay deposit, one helper deposit, one ICP transfer, one top-up, every time.
6. Keys: read in `ops/src/fuel/keys.ts` only (a guard test), its own EOA and IC identity, every log line scrubbed of the key; an address with
   code (an EIP-7702 delegation) is refused at start.
7. The monitor (`monitor/src/fueler.mjs`, in the monitor's project): `ops-unexpected-tx` SEV1 when either chain's nonce for the ops address
   passes the fueler's journaled one (the chain read first, so its own transactions never look foreign); the fueler's alerts relayed; the
   ops wallet under its floor SEV3; the fueler's `/healthz` in `liveness`.
8. The Trust page says the ops share is operator income, partly converted by an automated hot key, with these caps.

**Tests:** ops 99 (the fueler's 25 among them), monitor 254, web-rook 99, the seven Railway templates, the image built and started to its
configuration refusal.

**Before it holds money (the operator):**

1. A review of this section's code by someone other than its author, like every other change here.
2. A fresh EOA and a fresh IC identity, sealed in the fueler's Railway service; `RELAY_API_KEY` from dashboard.relay.link.
3. The Safe calls `treasury.setOps(<the fueler's address>)` (instant).
4. `FUELER_MODE=dry` for a day: `/status` shows each tick's decision and what it would sign.
5. Fund the address with a little 4663 ETH (its floor and first run, ≈ 0.04 ETH) until fees arrive, then `FUELER_MODE=send`.
6. Keep the §3.4 fallback reserve (≥ 100 T in the operator's own cycles ledger) until two clean runs.

### 4.2.2 Solana (2026-10-10)

Rook moved to Solana (`docs/v9/solana-migration.md`). The fueler's first leg moved with it; **`docs/v9/fueler-solana.md` is the reference**
(its programs half retired with rook_core on 2026-10-11, decision D-l; the books home: `docs/v9/canister-only-solana.md` §9.2).
In short: `FUEL_HOME=books` (the default; `robinhood` keeps the leg above). The ops share arrives as SOL at a Solana hot key, which is
the canister's `treasury.ops` (set by the operator with `treasury_set_ops`); the canister's own cranks pay it once 0.1 SOL is owed, and
the fueler reads the canister's `treasury_status` for the ops key and the income mark. The fueler keeps a 0.05 SOL fee buffer, forwards everything above the buffer plus a day's cap (1 SOL) to the cold payee (default the fund wallet), and,
when the buyer is under its trigger, takes up to the caps of SOL through Relay (SOL on Solana → native ETH at its own Ethereum address,
judged by the canister's Solana rules S1–S12 with the depository program pinned). From the ETH on Ethereum onwards the legs above are
unchanged (helper → ckETH → ICPSwap → ICP → CMC). Nothing deployed, signed or sent.

### 4.3 v2 (optional): the fuel canister (route (a′))

**When:** the hot key becomes the weakest link (stage above 1, outside guardians), or income makes the buffer material.

**Audit scope:**

- the fuel canister's two signing paths: a Relay deposit, patterned on and held byte-for-byte to `relayeth::judge`, and the helper
  deposit with fixed calldata;
- its ICRC-2 approve and ICPSwap calls, XRC use and the CMC notify;
- the forward-to-payee rule;
- controller = the timelock, and its install argument (payee, caps, pinned ids) published like the buyer's;
- its own cycles: it must top itself up first, within the same caps.

**Retire the fueler** once the canister has made two clean top-ups. Then the Safe's `setOps` moves `ops` to the canister's address.

---

## 5. The bootstrap

### 5.1 The minimum seed before any fee exists

| Item | Amount | ≈ US$ | Source |
| --- | --- | ---: | --- |
| Buyer cycles, 2 weeks normal (13 nodes, 6–10 T/day) | 84–140 T | 113–189 | runbook §6 |
| Canister creation (buyer + timelock) + timelock float | ≈ 1 T + 2 T | 4 | runbook §1, §6 |
| IC rehearsal, both paths (net, mostly returned) | ≈ 1.2 T (13 nodes) | 2 | runbook §1 |
| **Cycles, minimum** | **≈ 145 T** | **≈ 196** | |
| Deployer gas on 4663 | ≈ 0.03 ETH | 80 | key-ceremony step 4 |
| Keeper, resolver, guardian gas on 4663 (estimate: well under 0.001 ETH/day each, services-v1 §"keeper") | ≈ 0.005 ETH each | ≈ 40 | estimate |
| Ethereum gas pot seed (D3) | 0.01 ETH | 27 | phase1-nft §9 |
| Relay rehearsal (C5-I-1; mostly returned) | < 0.01 ETH | < 27 | runbook §2 |
| **Total minimum** | ≈ 145 T + ≈ 0.06 ETH | **≈ $350–380** | |

- The runbook's full posture is 250 T at the handover plus a 300 T operator reserve, about **$750 of cycles** and ≈ $920 in all.
- With v0 running, the reserve can be ≈ 100 T. With v1, the 250 T start can be 150 T once fee income is measured.
- **The Robinhood Chain gas waiver** (Robinhood Wallet transactions only) ended on **2026-09-29**. Budget 4663 gas as above.
- The **Ethereum gas pot is not the operator's cost after the seed.** Purchases' leftovers refill it to 0.05 ETH, and the excess
  goes to the modules.

### 5.2 Ways the operator avoids paying it

| Option | What | Verdict |
| --- | --- | --- |
| **Graduation fee (exists today)** | Each graduation sends 0.25 ETH to the treasury, so 0.025 ETH ≈ 49 T to ops at stage-0 shares. Three graduations repay the whole cycles seed | **Use it.** The only zero-change source |
| **Creator share on the launch coins** | A creator earns **30 %** of the fee (0.3 % of volume), 15× what ops earns from the same trades (0.02 %). If a Rook-owned address launches the five launch collections' first coins, that income is Rook's. `launch` is permissionless, so no contract change | **Strong.** Must be disclosed on the coin pages and the Trust page (the platform as creator) |
| **Temporary `opsBps` raise** | The Safe `setShares(8000,0,0,2000)` or higher for the first 30 days (immediate, public) | Possible. It takes from the reserves; the Trust page must state the shares in force |
| **A coin-creation fee in ETH** | `CurveFactory.launch` is **not payable** today, so it is a contract change, an audit and a redeploy. It would land in the treasury (10 % to ops) unless routed to ops | **Not before launch.** Revisit with the next contract round |
| **DFINITY Developer Grants** | $5k / $25k / $50k / $100k tiers, about 12–16 weeks; apply via the DFINITY support site. The grants page returned HTTP 503 on 2026-09-28, so whether it is open in 2026 is **unconfirmed** | Worth one application (Chain-Fusion NFT custody is on-theme). Too slow for day one |
| **DFINITY cycles faucet** | Historically 20 T free for new developers via a coupon from the DFINITY Discord. 2026 availability is unconfirmed | Covers the rehearsals, not the operation |
| **Robinhood Chain** | No standing grant program found. Robinhood sponsored the **2026 Arbitrum Open House** with $1M (buildathons in NYC, London, Singapore and Dubai, plus founder houses; announced 2026-02-11). 2 percentage points of the chain's net revenue go to an Arbitrum developer fund | Enter a buildathon if one is open; do not plan on it |

**Recommended bootstrap:**

1. The operator seeds the minimum (≈ $350–380).
2. Rook launches the first coin of each launch collection from a disclosed Rook address, so the creator share plus graduation fees
   repay the seed in weeks at modest volume.
3. Apply to DFINITY in parallel.

The operator decides on the creator-address option. It changes what the coin pages say.

---

## 6. Open items for the operator

1. **Who holds `ops`?** Name the key: a new EOA, not the deployer or any service key. Record it in `docs/deploy/key-ceremony.md`.
2. Decide the creator-address option (§5.2) before the first launches.
3. Run v0 from the handover. After 30 days, read `ops/cycles-log.md` and `/status` → `cycles`, then decide v1 against §1.3.
4. **Watch ICPSwap's health:** it is now the only IC DEX checked here. If it goes the way of KongSwap, the CEX leg of §4.1 is the
   fallback.

## 7. Sources (read 2026-09-28)

- ckETH minter, helper contract, fees and ids: [dfinity/ic `cketh.adoc`](https://github.com/dfinity/ic/blob/master/rs/ethereum/cketh/docs/cketh.adoc);
  [ICP docs: chain-key tokens](https://docs.internetcomputer.org/guides/digital-assets/chain-key-tokens/)
- CMC interface (ICP-only `notify_top_up`): [dfinity/ic `rs/nns/cmc/cmc.did`](https://github.com/dfinity/ic/blob/master/rs/nns/cmc/cmc.did)
- Cycles and the cycles ledger, 1 T = 1 XDR: [ICP docs: cycles](https://docs.internetcomputer.org/concepts/cycles/)
- ICPSwap swap flow: [ICPSwap-Labs/docs, Executing a Trade](https://github.com/ICPSwap-Labs/docs/blob/main/02.SwapPool/Swap/02.Executing_a_Trade.md);
  pool statistics from `https://api.icpswap.com/info/pool/all`
- KongSwap sunset (2026-04-06): [What We Learned from KongSwap (postmortem)](https://dev.to/sssdefi/what-we-learned-from-kongswap-product-governance-trust-and-exit-design-in-defi-1g25);
  LP fee 0.3 %: [KongSwap whitepaper](https://github.com/KongSwap/documentation/blob/main/kong_whitepaper.md)
- CycleOps (ICP or cycles, 7 % margin, 6 h checks, black-holed checker): [CycleOps FAQ](https://docs.cycleops.dev/docs/faq)
- Cycle.Express (fiat via Stripe): [cycle.express](https://cycle.express/);
  CyclePay (Stripe, ck-USDC rail deleted): [marc0olo/cyclepay #12](https://github.com/marc0olo/cyclepay/issues/12)
- DFINITY grants and the faucet: [dfinity.org/grants](https://dfinity.org/grants) (503 on the day);
  [Claim free cycles (DFINITY)](https://medium.com/dfinity/claim-free-cycles-to-kick-off-your-project-on-the-internet-computer-e97e1e50dc1c)
- Robinhood Chain: [Arbitrum blog, testnet and the $1M commitment](https://blog.arbitrum.io/robinhood-chain-testnet/);
  [gas waiver end, revenue share](https://cryptoticker.io/en/robinhood-chain-ecosystem-end-of-free-gas/);
  [Relay on 4663](https://defiway.com/blog/top-robinhood-chain-bridges)
- Prices: CoinGecko `simple/price`. Relay: `api.relay.link/quote`, 0.05 ETH 4663 → Ethereum (ETH and USDC).
