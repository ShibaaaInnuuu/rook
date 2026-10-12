# v8: the seven ideas the operator approved on 2026-09-27 — index

Branch `ideas-v8` (from `nft-side` `a37f54b`). **Design documents only**: no code, nothing deployed, nothing signed, nothing
pushed. Each document is self-contained: goal, why, mechanism, trust and security model, interaction with the invariants (every
unit 1:1 backed by ETH or an NFT; per-collection isolation; the caps; the timelock), phasing, open decisions, and a build and review
estimate. Live facts are measured or cited in each.

## The documents

| # | Document | The key design choice | The main risk | Build / review |
|---|---|---|---|---|
| 1 | [`bids.md`](bids.md) — buy with bids as well as asks | A **hybrid**: after arrival, a 12 h window of 1-hour, FULL, quantity-1 Seaport collection offers (WETH, OpenSea SignedZone) stepping −4 / −3 / −2 % under the canister's fresh floor, then the unchanged ask path; the ask path starts only once every order is **proven dead at finality**, so the ETH never pays twice | a signed order is a bearer instrument: a stale bid filling after a floor crash, and a pause that cannot recall a signature; bounded by the 1 h life, the stale guard, the off-chain cancel and `incrementCounter` | ≈ 2–3 weeks; **its own review round** of the signing path |
| 2 | [`redemption-draw.md`](redemption-draw.md) — redemption shown as a draw | No contract change: the site shows the vault's items and traits with **exact odds for the ticket about to be burned** and publishes each draw's proof (round, signature, free set, index) with a keyless verifier | **regulatory framing**: "a draw of rare traits" reads like a paid loot box; recommended wording is "composition, equal odds" until counsel answers | ≈ 1 week; half a day + legal |
| 3 | [`airdrop-passthrough.md`](airdrop-passthrough.md) — collection airdrops | **Delegate, do not claim**: one judged delegate.xyz v2 `delegateContract` per collection to a per-collection claims Safe; proceeds sold and returned to that collection's module as surplus | the claims Safe is an operator key over proceeds (not NFTs); malicious "airdrop" sites and contracts | ≈ 1 week; ≈ 1 day |
| 4 | [`proof-of-reserves.md`](proof-of-reserves.md) — a live reserves page | The Ethereum NFT delta to the existing v5 page: finalized `ownerOf` on two independent providers, each arrival transaction checked, the module's `C + R + T = E`, the advance's whereabouts; a **browser verifier and a CLI** that need no Rook service | a green shown on stale or single-source data; prevented by the G5 status rules and per-number ages | ≈ 1–1.5 weeks; half a day |
| 5 | [`mainnet-rehearsal.md`](mainnet-rehearsal.md) — a real-money rehearsal | A **throwaway rehearsal deployment** from the release commit (same wasm hashes, `key_1`, real timelock): Azuki alone at a 1 ETH target and reserve 0 (≈ 1.33 units, one Azuki), through purchase, mint, redemption and delivery, each step with its evidence and a pass check | a stuck state holding the operator's ETH: every state has a written exit, which makes the round-3 exit fixes prerequisites | ≈ 1–2 days to prepare; ≈ 5–7 days to run; net ≈ 0.3–0.4 ETH + 25–50 T cycles |
| 6 | [`index-token.md`](index-token.md) — a blue-chip index token | An **immutable, ownerless fixed-units basket** of the five wX (capped floor-value weights: BAYC 30, Pudgy 30, MAYC 20.4, Milady 11.3, Azuki 8.2 %), minted and redeemed in kind; rebalancing by new versions only; a stateless zap | economic: every collection's risk in one token, and zap slippage on thin pools | ≈ 1 week; ≈ 2 days |
| 7 | [`referrals.md`](referrals.md) — referrals as the growth engine | A stateless referral router in front of the unchanged curves, short codes, an opt-in creator fee splitter, rewards for **retained buyers** in the coin vesting 30 days with forfeiture, off-chain published scoring, weekly Merkle claims with a 48 h challenge window; no pay-per-engagement, no X-verified paid posts | the budget: at the default 1 % tier a 30 % creator budget is 0.09 % of volume, too small to pay promoters; and disclosure law | ≈ 2–3 weeks; ≈ 3–4 days |

## Live measurements taken for these documents (2026-09-27)

- **OpenSea collection-offer books** (keyless, 22:24 UTC): best offer vs floor — Milady −6.8 %, Azuki −6.4 %, MAYC −5.8 %, Pudgy −5.1 %,
  BAYC −3.3 %; offers within −5 %: 0 / 0 / 0 / 0 / 2; within −10 %: 4 / 4 / 6 / 3 / 2. Against the operator's listing depth within
  +5 % (3 / 14 / 2 / 7 / 5), the bid side near the floor is thinner still. Every order: WETH, OpenSea's SignedZone, FULL_RESTRICTED,
  OpenSea's conduit.
- **Seven days of fills on chain** (blocks 26,021,390–26,071,790): about half of all sales are bid fills; Seaport collection-offer
  fills a day — Milady 2.1, Azuki 1.1, MAYC 3.7, Pudgy 1.1, BAYC 0.6; their median time from `startTime` to fill 0.1–5 h; their
  prices about 1–5 % under the floor of their day (`bids.md` §1).
- **On chain:** OpenSea's conduit `0x1E00…3c71` (from `ConduitController`), Seaport 1.6's `information()`, the delegate.xyz v2 and v1
  registries' code, the five collections' `totalSupply` and `tokenURI` hosting (Milady and MAYC on the projects' web servers, the
  others on IPFS).
- **X's oEmbed** answers keylessly at `publish.x.com/oembed`.

## Recommended order

1. **`bids.md` and `mainnet-rehearsal.md` first.** Bids touch the canister's signing path (a new signature type, four new transaction
   targets, a second balance) and **need their own security review after the round-3/4 fixes land**; the rehearsal is the first time
   any of the Ethereum path runs with real money and must come before anyone else's. Run the rehearsal on the ask path first; add the
   bid path as its optional second item once bids are reviewed.
2. **The site pieces — `redemption-draw.md` (composition wording) and `proof-of-reserves.md` — can start now.** They are read-only,
   depend on no canister change, run against the dry run's fork, and the reserves page is launch gate G5 and a rehearsal pass check.
3. **`referrals.md` alongside** the launch work: the contracts do not touch custody, and scoring can run in shadow mode from day one.
4. **`airdrop-passthrough.md` and `index-token.md` after launch:** the first adds a transaction target to the custody key (after the
   fixes and a review); the second needs live wX supply in all five collections.

## Every open operator decision, in one list

**Bids (`bids.md` §9)**
- **B1.** Which collections bid at launch (recommended: MAYC and Milady; Pudgy and BAYC a week later; Azuki off).
- **B2.** The window and steps (recommended: 12 h, BAYC 24 h; −4 / −3 / −2 %).
- **B3.** Order life and stale guard (recommended: 1 h; 100 bps).
- **B4.** OpenSea's SignedZone (recommended) vs open orders.
- **B5.** One bidding buy canister-wide first (recommended) before concurrent bids.
- **B6.** Allow exactly two signing acts while paused — OpenSea's off-chain cancel and `incrementCounter()` (recommended yes; update the
  guardians' standing instructions).
- **B7.** The deposit hold for a bidding buy starts at the fill, not at arrival (recommended).
- **B8.** Optional creator royalties on bids (recommended: not paid).

**Redemption shown as a draw (`redemption-draw.md` §8)**
- **R1.** Framing: composition with equal odds (recommended) vs trait odds on the redeem flow (after counsel).
- **R2.** Rarity ranks only on the inventory, not the redeem flow (recommended).
- **R3.** Fund legal review of the mechanic before launch (recommended).
- **R4.** The sequencer-timestamp flag in the verifier (recommended), and whether to propose the pinning change for a later wrapper.

**Airdrops (`airdrop-passthrough.md` §7)**
- **D-A1.** Adopt delegation after launch (recommended).
- **D-A2.** Full contract-scoped delegation vs a narrow "airdrop" sub-right (recommended: full, revisit).
- **D-A3.** Per-collection claims Safes, their co-signer, and a 14-day return deadline.
- **D-A4.** Proceeds: sell and return as surplus (recommended) vs pro-rata distribution (after counsel).
- **D-A5.** Revoke delegations on a pause (recommended: no).
- **D-A6.** The exception path for on-chain claims, and its value threshold (e.g., > 5 ETH).

**Proof of reserves (`proof-of-reserves.md` §8)**
- **P1.** Cadence 10 min; unverified after 1 h, red after 3 h.
- **P2.** The site's default provider pair, independent of the canister's and the monitor's.
- **P3.** Publish the CLI verifier as an npm package (recommended).
- **P4.** Show the canister's own records, labelled (recommended).

**The mainnet rehearsal (`mainnet-rehearsal.md` §7)**
- **M1.** A separate rehearsal deployment (recommended) vs the production stack at its 8 ETH early target.
- **M2.** The subnet (13-node vs fiduciary), decided once for both.
- **M3.** Deposits on or off in the rehearsal.
- **M4.** Run the optional bids-and-way-back item before the public launch.
- **M5.** Retire the rehearsal canister by proposal, or let it run out of cycles.
- **M6.** The second person who reproduces the build.

**Index token (`index-token.md` §8)**
- **I1.** Weights: capped floor-value at 30 % (recommended), floor-value, or equal.
- **I2.** Basket size (recommended: 1 ETH per rBLUE at launch floors).
- **I3.** Fees (recommended: none in the vault; optionally a small zap fee).
- **I4.** Rebalancing by version only (recommended) or a later auction.
- **I5.** The name, and listing it before counsel.

**Referrals (`referrals.md` §9)**
- **F1.** The budget: creators' 0–30 % only, or a platform co-fund (how much of its 20 %).
- **F2.** Stickiness per buyer across coins (recommended) or per coin; 30 days.
- **F3.** The rebate (recommended 20 bps) and whether it needs a reputation-qualified code.
- **F4.** Rewards in the coin (a market buy each epoch) or in ETH.
- **F5.** The root poster and who may veto in the 48 h window (recommended: the guardians).
- **F6.** The Telegram bot at launch or after the site flow (recommended: after).
- **F7.** Shadow scoring for 2–4 weeks before paying (recommended).
- **F8.** No X-verified sponsorship tier (recommended), or only after counsel and X's written confirmation.

**For counsel (collected):** the draw's framing (R1–R3), airdrop proceeds and distributions (D-A4), the index as a fund-like product
(I5), referral disclosure under the FTC Endorsement Guides and SEC §17(b), rewards paid in the coin (F4), and the existing questions of
`docs/legal/prize-draw-memo.md`.
