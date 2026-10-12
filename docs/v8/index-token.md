# v8: a blue-chip NFT index token over the five collections' units — design

Status: **design, before any code**, 2026-09-27, branch `ideas-v8` (from `nft-side` `a37f54b`). Nothing built or deployed.
Read-only calls only: each collection's `totalSupply()` on Ethereum and the OpenSea floors of 22:24 UTC (`bids.md` §1.1).

**Legend.** **[M]** measured 2026-09-27. **[R]** this repository. **[A]** assumption.

**Phase: after launch.** It needs live wX supply in all five collections and changes nothing in custody.

---

## 0. Summary

- **What:** one ERC-20 on 4663, **rBLUE** (working name), each unit a fixed claim on a basket of the five wrappers' units (wMIL,
  wAZUKI, wMAYC, wPPG, wBAYC) held by an **immutable, ownerless index vault**. Mint by depositing the basket in kind; redeem for the
  basket in kind. No oracle, no price, no manager, no rebalancing inside the contract.
- **The key choice: a fixed-units basket, set once from capped floor-value weights at deployment, never rebalanced by the
  contract.** A change of weights is a new index version with an opt-in migration. That keeps the attack surface to "hold five
  ERC-20s and give them back", which is the only thing that can be made as safe as the wrappers themselves.
- **A zap router** (separate, stateless) lets a user buy rBLUE with ETH: it buys each wX through `EtchRouter.buyWrapped` (the
  shared pool or the module, whichever is better) and mints, and the reverse.
- **Main risk:** economic, not custodial — rBLUE carries every collection's risk (a custody failure in one collection devalues
  rBLUE by that weight) and the zap's slippage on thin wX pools. In-kind redemption means a holder can always exit into the four
  healthy collections.
- **Build ≈ 1 week, review ≈ 2 days** (a small contract; the zap is the larger part).

---

## 1. Goal and why

- **Goal:** one token for "blue-chip NFTs", backed 1:1 through the units by real NFTs or ETH, with no new trust beyond the wrappers.
- **Why:** each wX is one collection; most buyers want exposure to the category and cannot buy five things. An index also gives
  the wX pools a steady source of two-sided flow (every mint and redeem through the zap trades each pool).

---

## 2. Weights [M, A]

The five collections at the floors of 2026-09-27 22:24 UTC and their `totalSupply()` read on Ethereum:

| Collection | Supply | Floor (ETH) | Floor × supply (ETH) | Floor-value weight | **Capped at 30 %** | Equal weight |
|---|---|---|---|---|---|---|
| Milady | 10,000 | 0.929 | 9,290 | 7.4 % | **11.3 %** | 20 % |
| Azuki | 10,000 | 0.674 | 6,739 | 5.4 % | **8.2 %** | 20 % |
| MAYC | 19,570 | 0.856 | 16,749 | 13.4 % | **20.4 %** | 20 % |
| Pudgy Penguins | 8,888 | 3.286 | 29,208 | 23.4 % | **30.0 %** | 20 % |
| BAYC | 10,000 | 6.300 | 62,999 | 50.4 % | **30.0 %** | 20 % |

**Units per 10 ETH of basket** at those floors: capped — 1.22 wMIL, 1.22 wAZUKI, 2.39 wMAYC, 0.91 wPPG, 0.48 wBAYC; equal — 2.15,
2.97, 2.34, 0.61, 0.32.

- **Floor-value** (a market-cap analogue) makes rBLUE half BAYC: one collection's risk dominates.
- **Equal** over-weights the cheap collections and drifts with every floor move.
- **Capped floor-value at 30 %** (recommended): the category's shape without a single collection above 30 %.
- **Whatever is chosen is converted once into fixed unit amounts** at deployment (e.g., per 1 rBLUE: 0.1220 wMIL, 0.1220 wAZUKI,
  0.2388 wMAYC, 0.0913 wPPG, 0.0476 wBAYC for a 1 ETH rBLUE at those floors). After that the weights drift with the floors, as a
  buy-and-hold basket does. That is disclosed, not corrected.

---

## 3. Mechanism

### 3.1 The index vault (the only contract that holds anything)

```solidity
contract IndexVault is ERC20 {                 // rBLUE
    IERC20[5] public immutable parts;          // the five EthNftWrapper tokens (wX), fixed at deploy
    uint256[5] public immutable perUnit;       // wX wei per 1e18 rBLUE, fixed at deploy
    function mint(uint256 amount, address to);     // pulls ceil(amount × perUnit[i] / 1e18) of each part; mints `amount`
    function redeem(uint256 amount, address to);   // burns `amount`; sends floor(amount × perUnit[i] / 1e18) of each part
    function redeemSkip(uint256 amount, address to, uint8 skipMask); // the same, leaving out parts the caller cannot receive
}
```

- **No owner, no fee, no oracle, no upgrade, no pause.** The parts are standard ERC-20s (the wrappers' units) [R]; the vault calls
  only their `transferFrom` / `transfer`.
- **The invariant:** each part's vault balance ≥ `totalSupply × perUnit[i] / 1e18` (rounding in the vault's favour both ways). A
  forge invariant suite asserts it under random mints, redeems and direct donations.
- **No first-depositor or inflation attack:** there is no share price; a mint's amount is set by the caller and the pulls are exact.
- **Why `redeemSkip`:** if a wrapper ever reverts transfers to some address or is itself in trouble, a holder can still take the
  other four parts (the skipped part's share stays in the vault for the remaining holders — it is forfeited by the caller, stated in
  the name and the site).

### 3.2 The zap router (stateless, holds nothing between transactions)

- `buyIndex(minOut, to, deadline)` payable: for each part, `EtchRouter.buyWrapped` for the needed units (the router takes the better
  of the collection's shared pool and its Floor Module's mint [R, `EtchRouter.sol:80`]), then `IndexVault.mint`; returns leftover ETH.
- `sellIndex(amount, minEthOut, to, deadline)`: `redeem`, then `sellWrapped` each part.
- **Exposure:** slippage on thin pools (each wX pool is small at stage 0), the module's public mint caps (10 items ETH-backed, 5 an
  hour [R, `phase1-nft.md` §5]) that bound how much wX can be minted at the floor, and MEV (sandwiching). `minOut` and a
  per-part slippage check bound them.

### 3.3 Rebalancing

- **Not in the contract.** A basket that rebalances needs prices (the canister's posted floors, `module.floor()`) and a trade path
  (burn one wX at the floor less the spread, mint another at the floor plus the spread): both a new trust in the floor for this purpose
  and a steady cost of the two spreads.
- **By version.** When drift is large (e.g., a weight over 40 %), deploy rBLUE-2 with new `perUnit` and a migration helper
  (redeem rBLUE, zap the difference, mint rBLUE-2) that each holder calls when they choose.
- **A later option, if wanted:** a keeper-free rebalance auction that lets anyone swap parts into the vault at the modules' posted
  floors within a band — it would put the floor's integrity on the index's path and is out of scope for v1.

---

## 4. Trust and security model

- **What the canister must judge:** nothing. The index lives entirely on 4663 and never touches custody.
- **What can go wrong:**
  - a part's wrapper fails (custody loss): rBLUE loses that part's value; the reserves page of that collection is red; `redeemSkip`
    lets holders exit the rest;
  - a part's module is stale or paused: its burns may stop (`Stale`, `phase1-nft.md` §4.2); rBLUE still redeems in kind;
  - the zap is sandwiched: bounded by `minOut`; the vault itself cannot be;
  - rounding dust accrues to the vault (by design, in holders' favour).
- **New attack surface:** one ~150-line contract holding five ERC-20s; one stateless router. No new signer, no new oracle.

## 5. Interaction with the existing invariants

- **Every unit 1:1 backed:** rBLUE is 1:1 backed by wX, which is 1:1 backed by an NFT or ETH; nothing mints wX but the wrappers and
  modules.
- **Per-collection isolation:** the index **pools** the collections' risk for its holders by design; it does not connect the
  collections' contracts (no module, wrapper or canister state is shared). A collection's trouble reaches rBLUE's price, never the
  other collections' backing.
- **Pooling:** unaffected. ETH-backed and item-backed wX are the same token; the index does not care which.
- **Caps:** no custody or outflow cap changes. Demand for wX through the zap raises module mints (bounded by `maxPublicEthBacked`),
  which raises conversions, which fills custody toward J7's 15 ETH per collection; past that, wX stays ETH-backed (as today).
- **The timelock:** not involved (immutable contracts on 4663).

## 6. For legal review

A token backed by a basket of other tokens, with published weights, is the fact pattern of a fund or collective investment scheme
in many jurisdictions, and of an index product where it is marketed as tracking "blue-chip NFTs". Flag with the rest of
`docs/legal/prize-draw-memo.md` §5; no marketing of returns.

## 7. Phasing

1. **After launch**, once each of the five collections has wX supply and a shared pool (Pudgy and BAYC via `nft-pool`).
2. The vault and the zap, with a forge invariant suite and a fork test through the real routers.
3. rBLUE/ETH liquidity (optional): a pool seeded by protocol-owned liquidity.

## 8. Open decisions for the operator

- **I1.** Weights: capped floor-value at 30 % (recommended), floor-value, or equal.
- **I2.** The basket's size per rBLUE (1 ETH at launch floors, recommended, so rBLUE starts near 1 ETH).
- **I3.** A mint/redeem fee (recommended: none in the vault; the zap may carry a small fee to the treasury).
- **I4.** The rebalance policy: by version only (recommended) or an auction (later).
- **I5.** The name and whether it is listed on the site before counsel (§6).

## 9. Build and review estimate [A]

- `IndexVault` (≈ 150 lines) + invariant and unit tests: ≈ 2 days. The zap (≈ 250 lines) + fork tests through `EtchRouter`: ≈ 2–3
  days. Site: an index page (parts, weights drift, per-part reserves links): ≈ 1–2 days.
- Review: ≈ 2 days (the vault's rounding and the zap's slippage paths).
