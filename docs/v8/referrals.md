# v8: referrals as Rook's growth engine — design

Status: **design, before any code**, 2026-09-27, branch `ideas-v8` (from `nft-side` `a37f54b`). Nothing built or deployed.
Read-only calls only: X's public oEmbed endpoint (one post), public policy pages and reporting.

**Legend.** **[M]** measured 2026-09-27. **[R]** this repository. **[A]** assumption. **[D]** public source, cited.

**For legal review (§8):** promoter disclosure (FTC), touting (SEC, if a coin is a security), and rewards paid in the coin.

---

## 0. Summary

- **What (from the operator's discussion):** a **referral router in front of the curves and the pools** — the audited curves
  unchanged — that emits `Referred(buyer, referrer, token, amount)`; **short codes** mapped to addresses, typed on the site or in a
  Telegram buy bot; a **small buyer rebate** for using a code; **the first referral sticks 30 days**; a **per-launch creator
  promotion budget** (0–30 % of creator fees) funds the rewards; rewards **in the coin, vesting 30 days, forfeited** if the
  promoter or wallets linked to them sell while their referred buyers hold; **promoter reputation** from referred-buyer quality (hold
  time, not sybils or bots); **indexer attribution and weekly Merkle claims**; **no pay-per-engagement**.
- **The key design choice:** everything that judges (attribution, linking, reputation, forfeiture) is **off chain and published**;
  on chain there is only a router that forwards and logs, a code registry, a fee splitter the creator opts into at launch, and a
  Merkle distributor that pays only from the promotion pools. The worst an attribution bug can do is misallocate a promotion pool;
  it can never touch a trade, a curve, a module or custody.
- **The honest number (§3.2):** at the default 1 % fee tier the creator earns 0.30 % of volume, so a 30 % budget is **0.09 % of the
  coin's volume** — about 0.009 ETH per 10 ETH of volume. That does not pay promoters. Referrals work at the higher fee tiers
  (up to 8 %), with post-graduation pool fees, or with the platform co-funding from its own 20 % share. This is the operator's
  main decision.
- **Build ≈ 2–3 weeks** (contracts ≈ 1 week, indexer ≈ 1 week, site and bot ≈ 1 week); **review ≈ 3–4 days** (the router sits in
  the buy path of every referred buyer; the splitter sits in the creator's fee path).

---

## 1. Goal and why

- **Goal:** pay the people who bring buyers who stay, out of the launch's own fees, in a way bots and self-dealers cannot farm.
- **Why:** launches live or die by distribution in their first hours. Paying for posts is banned on X (§5) and was the model of the
  InfoFi apps X shut down; paying for **retained buyers** is ordinary affiliate marketing and aligns promoters with holders.

---

## 2. Mechanism

### 2.1 On chain (4663)

```solidity
contract ReferralCodes {                           // short codes → addresses
    function register(bytes16 code) external;      // [a-z0-9]{3,12}, lowercase, first come; one code per address; not transferable
    function ownerOf(bytes16 code) external view returns (address);
}

contract ReferralRouter {                          // stateless; holds nothing between calls
    event Referred(address indexed buyer, address indexed referrer, address indexed token, uint256 ethIn, uint256 tokensOut, bytes16 code);
    function buy(address coin, uint256 minOut, uint256 deadline, bytes16 code) external payable returns (uint256 out);
}
```

- **`buy`**, while the coin's curve trades: `curve = factory.routerOf(coin)` [R, `CurveFactory.sol:61`]; `curve.buy{value}(minOut,
  deadline)` sends the coins to the router, which forwards **exactly the balance increase** to `msg.sender`; a closing buy's excess
  ETH (the curve returns it to its caller [R, `FrogCurve._buy`]) is forwarded too. Once graduated: `EtchRouter.buy{value}(coin,
  minOut, msg.sender, deadline)` [R, `EtchRouter.sol:58`], which already delivers to `to`. Then `Referred(...)` with the referrer
  resolved from the code (zero for an unknown code: the buy still goes through, unreferred).
- **Invariants of the router:** no storage but immutables; `nonReentrant`; its ETH and coin balances are the same after every call
  as before (asserted); it never approves anything; the buyer's `minOut` and `deadline` are passed through unchanged. Sells do not
  go through it.
- **What changes for the curve:** nothing; its `Bought` event names the router as the buyer, which is why `Referred` names the real
  buyer.
- **`PromoSplitter`** (opt-in at launch): the creator launches through it, so it is the coin's `creator` [R, `creatorOf = msg.sender`].
  It claims the creator's fees (ETH before graduation; ETH, wX and coin after, `claimCreatorFees`, `forwardWrappedFees`,
  `forwardTokenFees` [R]) and splits them: `promoBps` (0–3,000, fixed at launch and public) to the coin's **promotion pool**, the rest
  to the human creator. Immutable; per coin; no owner.
- **`PromoDistributor`**: holds each coin's promotion pool; pays weekly epochs by Merkle root (`claim(epoch, coin, account, amount,
  vestStart, proof)`), with 30-day linear vesting per epoch; a root becomes claimable **48 h after it is posted** (a challenge
  window, §4); unclaimed and forfeited amounts roll into the next epoch of the same coin. The root poster is a Safe; it can post roots,
  never withdraw.

### 2.2 Off chain (the indexer)

- **Attribution:** a buyer's **first** `Referred` sets their referrer for **30 days** (per buyer, across coins, recommended; §9 F2);
  later codes in that window are ignored; after it, the next code starts a new window. Buys made without the router in the window
  still count for the sticky referrer only if the site made them (the site always routes through the router); direct curve buys do not.
- **Rewards per epoch and coin:** from that coin's pool, `reward_i ∝ Σ (referred ETH bought) × quality(referred buyer) × reputation(i)`,
  after the **buyer rebate** (below) is set aside; paid **in the coin** (post-graduation pool fees arrive partly in the coin; ETH in
  the pool is used to buy the coin at the epoch — a market buy, disclosed — or paid in ETH if the operator prefers, §9 F4).
- **Buyer rebate:** `ρ` of each referred buy (recommended 20 bps, capped by the pool), paid to the buyer in the same epoch's claim.
  Only when the referrer is not linked to the buyer (otherwise everyone would refer themselves for the rebate).
- **Quality of a referred buyer** (reusing the pons-swarm wallet-scoring idea the operator described): hold time (share still held
  at 24 h, 7 d, 30 d), wallet age and history, funding source (a fresh wallet funded from the same source as others in the same
  minute scores zero), bot signatures (same-block buys, identical amounts, known sniper contracts), and whether the buyer sells into
  the promoter's own sells.
- **Linking** (promoter ↔ wallets): funding graph (common funders, sweeps to common addresses), co-timed trades, shared deposit
  addresses. Linked wallets are one party for rewards and for forfeiture.
- **Forfeiture:** a promoter whose own or linked wallets **sell the coin while their referred buyers still hold it** forfeits the
  unvested rewards of that coin (the dumping-on-your-referrals pattern). Selling after the referred buyers have exited, or after 30 days,
  forfeits nothing.
- **Reputation:** a promoter's rolling score from their referred buyers' quality across coins; it scales their share and gates
  whether their code earns buyers the rebate (new promoters start at a neutral score).
- **Publication:** every epoch's inputs (the `Referred` events used, each buyer's quality inputs, each link and its evidence, each
  forfeiture and its trigger) and the computation code are published with the root, so anyone can recompute it.

### 2.3 The site and the Telegram bot

- **Site:** `?ref=code` in any coin link sets the code (stored per visitor, shown before signing: "code `alice` — Alice earns a reward
  from this coin's promotion budget; you get a 0.2 % rebate"); the buy button calls the router.
- **Telegram buy bot:** **non-custodial.** The bot never holds a user's key: it answers `/buy <coin> <amount> [code]` with a signed-in
  deep link to the site's prefilled buy (or a WalletConnect request) that the user signs in their own wallet. A custodial bot (keys on
  the bot's server) is a honeypot and a money-transmission question; it is not proposed.
- **Promoter page:** register a code (one on-chain transaction), the disclosure terms (§5), a dashboard of referred buyers' quality,
  vesting and forfeitures.

---

## 3. Economics

### 3.1 Where the budget comes from [R]

A coin's trade fee `feeBps` (the creator's tier, 100–800 bps, `policies-and-fees.md` §3) splits **40 % vault / 30 % creator / 5 %
reserve / 5 % liquidity / 20 % platform** [R, `DeployStack` template]. The launch fee (50 % decaying over 10 minutes) goes entirely to
the vault [R]. After graduation the coin's pool fees are split the same way [R, `FrogCurve.onPoolFees` et seq.].

### 3.2 What that pays [A]

| Fee tier | Creator's share of volume | Budget at 30 % | Per referred ETH if half the volume is referred | A promoter who brings 10 ETH of buys |
|---|---|---|---|---|
| 1 % (default) | 0.30 % | 0.09 % of volume | 0.18 % | ≈ 0.018 ETH |
| 3 % | 0.90 % | 0.27 % | 0.54 % | ≈ 0.054 ETH |
| 8 % (max) | 2.40 % | 0.72 % | 1.44 % | ≈ 0.144 ETH |
| 1 % + the platform co-funds 50 % of its 20 % share | 0.30 % + 0.10 % | 0.19 % | 0.38 % | ≈ 0.038 ETH |

Less the buyer rebate (20 bps of referred ETH). **At the default tier it is not a growth engine.** Options: promote the higher tiers
for creators who want promoters; co-fund from the platform's share (the curves' split is immutable, so this is the platform treasury sending part of what it
receives to the promotion pools, a public transfer by the Safe); count post-graduation pool volume, which continues for the coin's life.

---

## 4. Trust and security model

- **What the canister must judge:** nothing. Referrals are entirely on 4663 and off chain.
- **What can go wrong:**
  - **the router in the buy path**: a bug could lose a buyer's ETH or coins → the router is minimal, stateless, balance-neutral,
    reviewed, and optional (the site can fall back to direct buys);
  - **the splitter in the fee path**: a bug could strand the creator's fees → immutable, per coin, tested against the real curve, and
    opt-in;
  - **a wrong or malicious Merkle root** (the indexer is wrong, or the root poster is compromised) → it can misallocate only the
    promotion pools; the 48 h challenge window lets anyone who recomputes flag it, and the guardians (or the Safe) can veto a root
    before it activates;
  - **sybils and self-referral** → linking, quality scoring, rebate only across unlinked parties, reputation; imperfect by nature,
    published so disputes can be argued;
  - **false-positive linking** (an honest promoter forfeits) → an appeal path: the promoter shows evidence, the next epoch corrects;
  - **wash volume to farm rewards** → rewards follow retained buyers, not volume; wash buyers sell and score zero;
  - **code squatting and impersonation** (`@famousperson` codes) → codes are not identities; the promoter page shows the code's address
    and optional links the promoter proves (a signed message from the address, published).
- **New attack surface:** four small contracts on 4663 (codes, router, splitter, distributor), none holding custody or wX backing;
  the indexer's scoring; a Telegram bot that holds no keys.

## 5. Platform rules and disclosure

- **No pay-per-engagement.** X's Developer Policy: "Your service shouldn't compensate people to take actions on X", and "you may
  not sell or receive monetary or virtual compensation for any X actions … Posts, follows, unfollows, reposts, likes, comments, and
  replies" [D: X Developer Policy]. On 2026-01-15 X's head of product announced it "will no longer allow apps that reward users for
  posting on X (aka 'infofi')" and revoked their API access [D: the post, read through X's oEmbed, below]; **Kaito sunset Yaps** and
  **Cookie DAO shut down its Snaps rewards** the same day [D: CoinDesk; CotiNews]. Rook's rewards are for referred buyers who hold,
  never for posts, likes, follows or replies, and the product never reads X's API to decide a reward.
- **The optional sponsorship tier (later), verified through X's oEmbed:** measured [M], `https://publish.twitter.com/oembed?url=…`
  redirects to `https://publish.x.com/oembed`, which answers **without a key** with the post's author, text and date. But paying a
  sponsor **for a post** and checking the post through X's own endpoint is precisely "compensating people to take actions on X"; X's
  embed terms also add display and privacy-notice duties [D: X Developer Policy, embedded posts]. **Recommendation: do not build a
  paid-post tier on X verification.** If sponsorships are wanted, they are ordinary disclosed paid partnerships (a contract, X's own
  paid-partnership label, reviewed manually), outside the referral system, after legal review.
- **Disclosure (flag for legal review):**
  - **FTC Endorsement Guides (16 CFR Part 255):** a promoter who earns a reward has a material connection that must be disclosed
    clearly with the endorsement. The promoter terms require it ("I earn rewards if you buy with my code"), and the site shows it at
    the point of purchase [D].
  - **SEC Securities Act §17(b) (anti-touting):** if a coin were a security, anyone promoting it for consideration from the issuer must
    disclose **the receipt and the amount**; the SEC's EthereumMax settlement with Kim Kardashian ($1.26M, 2022) is the reference case
    [D]. Whether a coin on Rook is a security is counsel's question (`docs/legal/prize-draw-memo.md` §5); the rewards being paid in the
    coin and funded by fees from others' trading make it sharper.
  - Consumer-protection, tax (rewards are income to promoters) and, for the rebate, whether it is a price change that must be shown to
    all buyers equally.

## 6. Interaction with the existing invariants

- **Every unit 1:1 backed:** untouched; the router buys coins, which are not wX; the splitter moves only the creator's fees.
- **Per-collection isolation:** promotion pools are per coin; nothing crosses collections.
- **Caps, custody, the canister, the timelock:** untouched. The router makes no conversion faster than the curves already allow.
- **The audited curves, pools and routers are unchanged**; the new contracts only call their public functions.

## 7. Phasing

1. **Alongside the launch work (no dependency on the canister):** the contracts and their tests; the indexer attribution (reading
   `Referred`) with scoring in shadow mode (computed, published, not paid) for the first weeks, to tune thresholds on real behaviour.
2. **At or soon after launch:** codes and the router live; promotion pools accumulate; the first paid epoch after 2–4 weeks of shadow
   scoring and after counsel's answers on disclosure.
3. **Later:** reputation-gated perks; sponsorships only as §5 describes.

## 8. For legal review

FTC material-connection disclosure; SEC §17(b) if applicable; rewards and rebates paid in the coin (a distribution? an inducement to
buy?); tax reporting for promoters; the rebate's fairness to non-referred buyers; X's terms for any use of oEmbed; whether a
non-custodial Telegram bot that builds transactions is itself regulated anywhere Rook operates.

## 9. Open decisions for the operator

- **F1.** The budget: creators' 0–30 % only, or with a platform co-fund (and how much of the platform's 20 %).
- **F2.** Stickiness: per buyer across coins (recommended) or per buyer per coin; 30 days.
- **F3.** The rebate: 20 bps (recommended), and whether it needs a reputation-qualified code.
- **F4.** Rewards in the coin (the operator's idea; a market buy at each epoch) or in ETH (simpler, no buy pressure to disclose).
- **F5.** The root poster (a Safe) and who may veto during the 48 h window (the guardians, recommended).
- **F6.** Whether to build the Telegram bot at all at launch (recommended: after the site's flow is live).
- **F7.** Shadow-mode length before paying (recommended: 2–4 weeks).
- **F8.** No X-verified sponsorship tier (recommended), or only after counsel and X's written confirmation.

## 10. Build and review estimate [A]

- Contracts: `ReferralCodes`, `ReferralRouter`, `PromoSplitter`, `PromoDistributor` with forge tests against the real curve, pool and
  routers (balance neutrality, the closing buy's refund, graduated and trading paths, the splitter's three fee kinds, vesting,
  roll-over): ≈ 1 week.
- Indexer: attribution, linking, quality, reputation, forfeiture, epoch computation, publication: ≈ 1 week (the scoring reused from the
  pons-swarm idea).
- Site (codes, disclosures, dashboard) and the non-custodial bot: ≈ 1 week.
- Review: ≈ 3–4 days (the router and the splitter first; the distributor; the scoring's publication and the challenge window).

## Sources

- X: [Developer Policy](https://docs.x.com/developer-terms/policy), [Developer Agreement](https://docs.x.com/developer-terms/agreement),
  [Nikita Bier, 2026-01-15](https://x.com/nikitabier/status/2011825522817270230) (read via `publish.x.com/oembed`).
- InfoFi shutdowns: [CoinDesk, Kaito sunsets Yaps](https://www.coindesk.com/business/2026/01/15/kaito-to-sunset-yaps-as-x-cracks-down-on-infofi-apps-token-falls-17),
  [CotiNews, Cookie DAO and Kaito](https://www.coti.news/news/x-cracks-down-on-infofi-forcing-cookie-dao-and-kaito-to-shut-down-reward-programs),
  [Unchained](https://unchainedcrypto.com/x-bans-incentivized-posting-apps-prompting-shakeup-in-crypto-engagement-platforms/).
- Disclosure: [FTC Endorsement Guides / SEC §17(b) and the Kardashian settlement (Davis+Gilbert)](https://www.dglaw.com/sec-penalizes-kim-kardashian-over-1-million-for-paid-crypto-post/),
  [Holland & Knight on touting](https://www.hklaw.com/en/insights/publications/2023/03/tout-tout-let-it-all-out-sec-continues-crackdown-on-celebs).
- Repository: `src/curve/FrogCurve.sol` (`_buy`, `_takeFee`, the creator's claims), `src/curve/CurveFactory.sol` (`routerOf`,
  `creatorOf`), `src/pools/EtchRouter.sol`, `script/DeployStack.s.sol` (the fee template), `docs/v5/policies-and-fees.md`.
