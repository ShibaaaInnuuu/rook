# v8: collection airdrops passed through to holders — design

Status: **design, before any code**, 2026-09-27, branch `ideas-v8` (from `nft-side` `a37f54b`). Nothing built or deployed.
Read-only calls only: `eth_getCode` of the delegate.xyz registries on Ethereum; public reporting and documentation.

**Legend.** **[M]** measured 2026-09-27. **[R]** this repository. **[A]** assumption. **[D]** public source, cited.

**Phase: after launch.** Nothing here is needed to launch; it touches the custody key's signing surface and must wait for the
round-3/4 fixes and its own review.

---

## 0. Summary

- **The problem.** The canister's Ethereum address holds real Milady, Azuki, MAYC, Pudgy and BAYC tokens. Projects reward their
  holders (tokens, NFTs, mint passes). Today the canister claims nothing: "an NFT of a collection the canister does not serve is
  never touched" and it never signs anything but its judged transactions [R, `phase1-nft.md` §2.4]. The holders' units forgo
  that value.
- **The design choice: delegate, do not claim.** The custody key makes **one judged on-chain transaction per collection**: a
  **delegate.xyz v2** `delegateContract(claimsAddress, collection, rights, true)` naming a separate **claims address**. Claim sites
  that honour delegation let the claims address claim for the vault; the custody key never signs a claim site's message and never
  calls a claim contract. On-chain claim contracts that ignore delegation get, at most, **one judged call each**, added by a
  timelocked upgrade (§3.3).
- **Proceeds: sold and returned to that collection's own module as surplus** (`fundReserve`), recorded publicly per claim. Not
  distributed pro rata to holders at first (§4: most wX sits in locked pools, and a distribution raises the securities question).
- **Main risk:** the claims address is an operator-controlled key. It cannot move the NFTs (the registry is only a registry), but
  it controls the proceeds until they are returned. Public accounting, a per-claim record and a deadline bound it.
- **Build: ≈ 1 week** (the delegation judge and transaction in the canister, the claims runbook, the site's ledger), plus a
  review of the one new transaction target.

---

## 1. Real airdrop patterns for the five collections [D]

| Collection | Event | How it was claimed | What it means for a vault |
|---|---|---|---|
| BAYC / MAYC | **ApeCoin**, 2022-03-17: 10,094 APE per BAYC | an on-chain claim contract the holder called; it checked `ownerOf` **at claim time** (no snapshot) | a vault could claim by one contract call. The same design let an attacker flash-borrow five apes from the NFTX vault, claim, and return them in one transaction (≈ 60k APE, $1.1M) [D: Amber Group; The Block] — a warning for any vault with instant redemption |
| Azuki | **ANIME**, 2025-01-23 (Ethereum and Arbitrum): allocations to Azuki, Elementals and Beanz holders by "Collector's Status" (time held and rarity) | claimed on anime.xyz by connecting and signing with the holding wallet [D: Decrypt; CoinDesk] | an off-chain allocation and a wallet signature: a canister cannot safely sign a site's message; delegation is the only route |
| Pudgy Penguins | **PENGU**, 2024-12-17 (Solana): ≈ 1.7M PENGU per Pudgy | connect the Ethereum wallet, sign a message to prove ownership, name a **Solana** address; 88-day window, unclaimed locked for good [D: Decrypt; CoinMarketCap] | a cross-chain claim with a deadline: a vault that cannot sign loses it outright |
| Milady | **CULT**, 2024-12 (≈ $207M airdropped to Milady holders); later seasonal distributions via RemiliaNET | a claim flow with an "IQ test" whose failure burned the allocation [D: Decrypt; CryptoTimes] | an interactive, human-only claim: no automated path is possible; delegation to a human-held address is |

**The patterns:** (1) **on-chain claim contracts** reading `ownerOf` at claim time; (2) **off-chain allocations claimed by a
wallet signature**, often paying out on another chain; (3) **points by holding time**, which a long-holding vault may score well
on; (4) **pushed** tokens or NFTs sent to holders unasked; (5) **delegation-aware claims** via delegate.xyz or warm.xyz, which let a
"cold" holder claim from a "hot" wallet [D: delegate.xyz docs]. Patterns 2 and 5 dominate recent large drops.

---

## 2. Goal and why

- **Goal:** capture what a holder of the vault's NFTs is entitled to, without widening what the custody key signs beyond one
  judged registry transaction per collection, and return the value to that collection's holders.
- **Why:** the vault's items are its holders' backing. An unclaimed airdrop is value holders of the underlying would have had
  and holders of the units do not. It is also a product story ("wX holders get the collection's airdrops"), which §4 caps honestly.

---

## 3. Mechanism

### 3.1 Delegation (the default path)

- **The registry:** delegate.xyz **v2**, `0x00000000000000447e69651d841bD8D104Bed493` on Ethereum (10,046 bytes of code, code hash
  `0x9decc…1ba9` [M]); the v1 registry `0x00000000000076A84feF008CDAbe6409d2FE638B` (6,230 bytes [M]) is still read by some older
  claim sites. V2 supports per-contract and per-token delegation and **sub-delegation of rights** (e.g., "airdrop" rights only)
  [D: delegate.xyz, v1 → v2]. A delegate "has permission to claim airdrops or other utility on behalf of the vault, but it cannot
  touch anything within the vault" [D].
- **The transaction:** `delegateContract(address to, address contract_, bytes32 rights, bool enable)` from the custody address,
  for each configured collection, `to` = that collection's **claims address**, `rights` = the configured rights (`0` = all
  rights, or a named sub-right; §7 D-A2). Optionally the same on v1 (`delegateForContract`) for sites that only read v1.
- **The canister's judge (`ethclaim::judge_delegation`):** the target is exactly the configured registry (address and code hash);
  the function is exactly `delegateContract` (or the v1 equivalent); `contract_` is a configured collection; `to` is that
  collection's configured claims address; `rights` is the configured value; `enable` true to add, false to revoke; simulates at the
  pin; the Ethereum line's rules; nothing else. The configuration is in the install argument, so adding or changing a delegate is a
  published, timelocked act.
- **Revocation:** the same call with `enable = false`, signed when the configuration drops it, or at once on the guardian's pause
  (§7 D-A5).

### 3.2 The claims address and who operates it

- **One claims address per collection** (per-collection isolation of proceeds). Recommended: a **Safe** (the operator plus a
  second signer), used only for claims of that collection, holding nothing else.
- **What the claims operator does per airdrop:** checks the claim is real (the project's own announcement, the contract verified,
  a simulation), claims with the claims Safe, records the claim (§3.4), converts proceeds to ETH (§4), and sends the ETH back
  (Relay to 4663, then `fundReserve` on that collection's module). Every step's transaction is public.
- **What it can never do:** move a vault NFT (the registry grants no transfer right; the NFTs stay at the custody address, and the
  canister signs transfers only for judged deliveries); sign anything with the custody key.
- **Why not the canister as the claimer:** claim sites ask for **personal_sign / SIWE messages** and sometimes EIP-712 typed data.
  A canister cannot judge an arbitrary message: a "login" can be a Permit2 or a Seaport order in disguise. The custody key must
  never sign one. Claims need a human-grade key with nothing to lose.

### 3.3 On-chain claim contracts that ignore delegation (the exception path)

For a large on-chain claim that checks `ownerOf` of the caller (the ApeCoin pattern), delegation does not help. Then:
- a timelocked upgrade adds **one claim entry**: `{ contract, code_hash, calldata_template, token_ids, recipient = the claims
  address, deadline }`; the canister's judge signs exactly that call once per listed token id (simulated; value 0; no approvals),
  with the proceeds sent to the claims address if the contract allows a recipient, or swept to it by a second judged ERC-20
  `transfer` of the named token;
- the 72 h timelock means this path cannot serve a claim window shorter than about four days;
- each entry expires at its deadline and is removed by the next upgrade.

This path is expensive in review and should be used only when the value is large.

### 3.4 Pushed airdrops (tokens or NFTs sent to the custody address)

- **Unchanged rule:** the canister never calls a contract it does not serve [R]. A pushed ERC-20 or NFT stays at the custody
  address; the reconciler lists it.
- **Recovery, when worth it:** a timelocked upgrade adds one judged `transfer` of that token to the collection's claims address
  (the exception path of §3.3). Default: leave it, and say so on the reserves page.
- **Malicious pushed tokens** (fake "claim" tokens with phishing names, transfer hooks, approval traps): never touched by the custody
  key; the site never links to them.

### 3.5 The public ledger of claims

Per collection, the site's reserves page gets an "airdrops" section: each claim (project, date, what was claimed, the claim
transaction), each conversion (the swap), the return (the Relay order and the `fundReserve` transaction on 4663), and any
airdrop the vault was eligible for and did not claim, with why.

---

## 4. Proceeds: what to do with them

| Option | How | For | Against |
|---|---|---|---|
| **A. Sell, return as the collection's surplus (recommended)** | the claims Safe sells for ETH, Relay to 4663, `fundReserve` on that module (surplus: tops up conversions, never pays a burn) | simple; no new contract; per-collection; lifts that collection's headroom (`phase1-nft-pooled.md` §1.1), so the next graduations buy more items; no payment to holders | value reaches holders indirectly (more NFTs per ETH over time), not as a visible distribution |
| B. Buy more items | the same ETH, but the canister buys an item and mints a unit… to whom? | a visible "the vault grew" | every item must back a unit (the wrapper's ledger); minting a unit to the module turns this into A with extra steps |
| C. Distribute pro rata to wX holders | a Merkle claim of the proceeds by a snapshot of wX balances | visible, direct | most wX sits in each coin's **locked** pool (`LiquidityLock`) and the module, not with people; a fair snapshot needs look-through of LP positions; a distribution of value to token holders from the efforts of others is the securities fact pattern the memo flags (`docs/legal/prize-draw-memo.md` §5) |
| D. Pass the claim itself to redeemers | nothing to build: whoever redeems the NFT after a snapshot has already lost the snapshot | — | not a pass-through at all; stated on the site so nobody expects it |

**Recommendation: A**, with the public ledger. Revisit C only after counsel (§8).

---

## 5. Trust and security model

- **What the canister must judge:** one new transaction kind (the registry call, §3.1), and optionally per-claim entries (§3.3).
  Nothing else changes in its signing.
- **What can go wrong:**
  - **a malicious "airdrop"** (a fake claim site, a claim contract that asks for approvals, a pushed token with a phishing name):
    the custody key never interacts; the claims Safe holds nothing else and is operated by a runbook (verify from the project's own
    channels, simulate, never sign a permit or `setApprovalForAll`, never interact from the custody address);
  - **the claims operator keeps the proceeds**: bounded by the public ledger, the monitor (any token movement from a claims address
    not matched to a ledger entry is SEV2), and a deadline (proceeds returned within 14 days of the claim);
  - **a delegation abused by a site**: a delegation-aware site might let the delegate do more than claim (e.g., register the vault's
    NFTs in a program). It still cannot move them. Sub-delegating a narrow right (§7 D-A2) limits which sites honour it;
  - **a flash-loan-style claim against the vault** (ApeCoin 2022): with redemption, anyone can burn a unit, receive an item and claim
    its airdrop; that is fair (they now hold the NFT), but a claim contract that reads `ownerOf` at claim time can be claimed by the
    vault first only if the vault is fast. No instant loan of vault items exists (redemption takes a drand round and an Ethereum
    delivery), so the NFTX-style flash claim is not possible against Rook;
  - **deposits of an NFT with an unclaimed airdrop**: the vault may claim it; the depositor's loss, stated on the deposit page.
- **New attack surface:** one registry transaction target from the custody key; one or more claims Safes outside custody.

## 6. Interaction with the existing invariants

- **1:1 backing:** unchanged; nothing mints or burns units. Proceeds enter the module as surplus, which backs nothing and pays no
  burn [R, `fundReserve`].
- **Per-collection isolation:** one delegation and one claims address per collection; each collection's proceeds return only to its
  own module; the ledger is per collection.
- **Caps:** no custody, outflow or mint cap changes; the registry transaction is a zero-value call, not an H3 flow. It uses the
  Ethereum line (seconds).
- **The timelock:** the registry, the rights and each claims address are install configuration (72 h). An exception-path claim is an
  upgrade argument.
- **Redemption:** an NFT delivered out is no longer delegated (the delegation is by contract, and the vault no longer owns the token);
  nothing to undo.

## 7. Open decisions for the operator

- **D-A1.** Adopt delegation at all (recommended: yes, after launch).
- **D-A2.** Rights: full delegation (`rights = 0`, honoured by the most sites) vs a narrow "airdrop" sub-right (safer, honoured by
  fewer). Recommended: start with full contract-scoped delegation, revisit.
- **D-A3.** The claims addresses: per-collection Safes (recommended), who co-signs, and the 14-day return deadline.
- **D-A4.** Proceeds policy A (recommended) vs C (after counsel).
- **D-A5.** Revoke delegations on the guardian's pause (recommended: no; a delegation cannot move NFTs, and revoking needs signing
  while paused).
- **D-A6.** Whether to use the exception path (§3.3) at all, and a value threshold for it (e.g., only above 5 ETH per claim).

## 8. For legal review

Receiving and selling airdropped tokens on behalf of holders, and routing proceeds to a module that benefits them: custody,
tax (who received the airdrop: the entity? the holders?), and, for option C, distributions to token holders
(`docs/legal/prize-draw-memo.md` §5). Claims on other chains and in other jurisdictions may carry KYC or geo-restrictions the
claims operator must respect.

## 9. Phasing

1. **After launch and after the round-3/4 fixes:** the delegation judge and one delegation per collection; the claims Safes; the
   site's airdrop ledger.
2. **Per airdrop, as they come:** the runbook; the exception path only above the threshold.
3. **Later, after counsel:** option C, if wanted.

## 10. Build and review estimate [A]

- Canister: `ethclaim.rs` (the registry judge, the configuration, the line transaction, revocation), upgrade safety, tests:
  ≈ 3 days.
- Monitor: the delegation state per collection (read `DelegateRegistry.getOutgoingDelegations(custody)`), claims-address movements
  against the ledger: ≈ 1 day.
- Site and indexer: the airdrop ledger: ≈ 1–2 days. The claims runbook: ≈ half a day.
- Review: the one transaction target and its judge (≈ 1 day) — can ride with the bids review.

## Sources

- ApeCoin: [Amber Group, reproducing the APE flash-loan claim](https://medium.com/amber-group/reproducing-the-ape-airdrop-flash-loan-arbitrage-exploit-93f79728fcf5),
  [The Block](https://www.theblock.co/post/138410/someone-borrowed-5-bored-apes-to-claim-1-1-million-of-ape-tokens).
- ANIME: [Decrypt](https://decrypt.co/301080/anime-azuki-ethereum-token-airdrop-tokenomics), [CoinDesk](https://www.coindesk.com/business/2025/01/23/nft-project-azuki-s-animecoin-debuts-at-usd1-2b-fdv-amid-airdrop).
- PENGU: [Decrypt](https://decrypt.co/296507/pengu-pudgy-penguins-solana-token-airdrop-tokenomics), [CoinMarketCap](https://coinmarketcap.com/academy/article/how-to-claim-the-pengu-airdrop-a-detailed-guide).
- CULT: [Decrypt](https://decrypt.co/295800/milady-cult-why-it-took-so-long), [CryptoTimes](https://www.cryptotimes.io/2024/12/10/cult-memecoin-soars-after-207m-airdrop-to-milady-nft-holders/).
- delegate.xyz: [docs](https://docs.delegate.xyz/), [contract addresses](https://docs.delegate.xyz/technical-documentation/delegate-registry/contract-addresses),
  [IDelegateRegistry](https://docs.delegate.xyz/technical-documentation/delegate-registry/idelegateregistry.sol), [v1 → v2](https://docs.delegate.xyz/upgrade-to-v2/v1-v2-migration).
- warm.xyz and delegate.xyz compared: [Boring Security](https://boringsecurity.com/articles/wallet-delegations-explained).
