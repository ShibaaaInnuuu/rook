# v8: a live proof-of-reserves page for the Ethereum NFT side — design

Status: **design, before any code**, 2026-09-27, branch `ideas-v8` (from `nft-side` `a37f54b`). Nothing built or deployed. No
network calls beyond those recorded in the other v8 documents.

**Legend.** **[R]** this repository. **[A]** assumption. **[D]** public documentation.

**What already exists [R].** A reserves page and its API were built for v5 (`docs/v5/ux-and-reserves.md` §1–§6; launch gate G5,
`docs/v5/launch-gates.md`): `indexer/src/wrapped/` (`ledger.ts` folds the wrapper's events, `invariant.ts` computes green / red /
unverified, `verify.ts` reads `ownerOf` on every configured provider, `floorModule.ts` reads the module), `/v1/reserves[/:slug]`,
and `web-rook/src/pages/Reserves.tsx`. This document is the **delta for the Ethereum NFT side as built in `nft-side`** (the
`EthNftWrapper`, the canister's buys, deposits and ways back, the advance in transit, the gas pot and leftovers, finality) and the
**self-verification** a user runs without trusting Rook's indexer.

---

## 0. Summary

- **One status per collection** — green, red or unverified — computed from public reads only, with each number's source, block
  and age on the page; unchanged semantics from G5 (green only when every item is found where it should be by every source, every
  read is fresh, and the books balance exactly).
- **The new numbers for Ethereum collections:** items held by `ownerOf` at an Ethereum **finalized** block on two independent
  providers; each item's arrival transaction checked (a Seaport fill or a holder's deposit, to the custodian, final); units on
  4663 by `totalSupply` and the wrapper's books; the ETH side at the module (`C + R + T = E`), the unit in transit and where its
  ETH is (the canister's buy record: Relay order, Ethereum balance, way back); the gas pot and each collection's leftover; strays
  held; and, once `bids.md` ships, open bids and WETH.
- **"Check it yourself":** a button that re-runs every custody read **from the visitor's browser** against providers the visitor
  chooses, and a keyless command-line verifier that needs no Rook service at all.
- **Cadence:** 10 minutes; unverified after 1 hour stale, red after 3 hours (G5's bound).
- **Build: ≈ 1–1.5 weeks** (indexer delta, page, browser verifier, CLI); can start now: it reads contracts and public records only
  and runs against the dry run's fork.

---

## 1. Goal and why

- **Goal:** anyone can confirm, at any moment and without trusting Rook, that every wX unit of a collection is backed 1:1 — by an
  NFT sitting at the canister's Ethereum address or by ETH at the collection's module or in a traceable transit — and see the
  instant it is not.
- **Why:** custody is a honeypot (`ux-and-reserves.md` §1). The canister's key and the timelock are the trust; the page is how that
  trust is watched by the people who bear it. It is also launch gate G5.

---

## 2. What the page shows, per Ethereum collection, and how each number is checked

| # | Number | Source (public, keyless) | Independent check |
|---|---|---|---|
| 1 | **wX in circulation** | `EthNftWrapper.totalSupply()` at a 4663 block B | two 4663 providers (`rpc.mainnet.chain.robinhood.com`, `rpc.ordofi.network`) agree at B |
| 2 | **The wrapper's books**: `held`, `openTickets`, items (`item(i)`), tickets (`ticket(i)`) | the wrapper's views at B | the indexer's fold of `Wrapped`, `Deposited`, `Redeemed`, `Resolved`, `Delivered`, `Undeliverable`, `Refunded` must equal the views (else red: the books disagree with themselves) |
| 3 | **Items in custody** | `ownerOf(tokenId)` of the collection, for every held item, at Ethereum block N = the lower `finalized` of two providers | two providers **not** the canister's pair and not Rook's own node (§2.1); all must name the custodian; a token owned elsewhere is **red** with its owner and the transfer that moved it |
| 4 | **Each item's arrival** | the `purchaseTx` / `depositTx` in `Wrapped` / `Deposited`; its Ethereum receipt | the receipt is final (block ≤ N), status 1, contains `Transfer(x, custodian, tokenId)` of the collection; for a purchase, a Seaport `OrderFulfilled` with the price paid (a listing we filled, or our own offer once bids exist); for a deposit, the claim data naming chain 4663, this wrapper and the recipient |
| 5 | **The purchase's cost** | the module's `AdvanceSettled(item, advanced, cost)` | `cost = ⌈advanced × paid / arrived⌉` against the receipt's price (4) and the canister's record (7); shown per item |
| 6 | **The ETH side** | the module: `ethBackedUnits`, `escrowBalance`, reserve, surplus, revenue, transit (`transitUnits`, `advanced`, `advancedAt`) | `C + R + T = E` (the module's own ledger, `phase1-nft.md` §11.1) recomputed from the views; the module's ETH balance by `eth_getBalance` |
| 7 | **The canister's records** | `eth_nft_state()` (buys, deliveries, deposits, the line, leftovers), an ICP query | each `Wrapped` item has a `Settled` buy naming the same token and transaction; each `Deposited` a minted deposit; an open advance on 4663 has exactly one open buy, and its phase says where the ETH is (below) |
| 8 | **Where an advance's ETH is** | the buy's phase: `Funded` (at our 4663 address), `Bridging` (a Relay order id), `Arrived` (at our Ethereum address, earmarked), `Back*` (on its way back) | Relay's public order status for the order id; `eth_getBalance` of the custodian on Ethereum ≥ Σ earmarks; the 4663 balance of the custodian ≥ `Funded` asks |
| 9 | **The gas pot and leftovers** | `eth_getBalance(custodian)` on Ethereum at N; the canister's `leftover` per collection | balance = earmarks + leftovers + pot (± fees of mined transactions since the canister's last read); shown, not part of the 1:1 equation (it backs no unit) |
| 10 | **Tickets in flight** | tickets `Pinned` / `Resolved` not delivered | each resolved ticket's delivery (the Ethereum transfer, when signed) and its deadline; overdue in red; a refund shown with its reason |
| 11 | **Not in the books** | `Transfer(*, custodian, *)` of the collection (the indexer's scan) | tokens of the collection owned by the custodian but not held by the wrapper: a stray being returned or `Held`, a deposit awaiting finality, a bid fill awaiting its mint; each listed with its state (never counted as backing) |
| 12 | **Caps** | `custody_caps()` and the stage | items and value against J7 (15 ETH per collection); H3's day so far |
| 13 | **Open bids** (after `bids.md`) | the canister's live-order ledger; Seaport `getOrderStatus`; WETH `balanceOf` and `allowance(custodian, conduit)` | each live order's price, `endTime`, funding; WETH outside a bid is flagged |

**The equation** (unchanged from `ux-and-reserves.md` §2, now with Ethereum finality):

```
(items owned by the custodian at finalized N − tickets in flight) × 1e18  +  ETH-backed units (module at B)  =  wX totalSupply (wrapper at B)
```

Status rules as G5: **red** if any held item is owned elsewhere by every provider, if the books disagree with themselves (2), if an
arrival does not check out (4), or if the equation fails even counting every unverified item as present; **unverified** if any read
failed, disagreed or is stale; **green** only otherwise.

### 2.1 Which providers

- **The site's defaults:** a pair independent of the canister's install pair (`evm_chains[1]`) and of the monitor's Dwellir key:
  e.g. `ethereum-rpc.publicnode.com` and `mainnet.gateway.tenderly.co` (both answered 500-block `eth_getLogs` in the
  measurements of `bayc10-decisions.md` §5; their state depth for `ownerOf` at the finalized block is a check to run at build). `eth.drpc.org`, the v5 default in `indexer/src/wrapped/config.ts`, refuses
  500-block log ranges (round 3 D3-L-3) and should be replaced for the Transfer scan (11).
- **Finality:** the v5 verifier reads at "the lowest head less 8 blocks" (`verify.ts`). The Ethereum NFT side mints only past
  finality, so the page reads custody at the **finalized** tag (the lower of the providers'), and says so.

### 2.2 The canister's word, labelled

`eth_nft_state()` is an ICP **query**: one replica's answer, not certified. The page labels it "the canister's own records"
and uses it only to explain (7, 8, 9), never to turn anything green: every green verdict rests on Ethereum and 4663 reads. A
verifier who wants a consensus answer can call the same method as an **update** (a replicated call; any query method can be called
that way), at the canister's cycles.

---

## 3. Refresh cadence, freshness and what a mismatch shows

- **Cadence:** the indexer recomputes every **10 minutes** (Ethereum finality advances every ≈ 6.4 minutes; a 4663 block every
  ≈ 0.1 s). Each number shows its block and age.
- **Freshness:** older than 1 h → that number is unverified; the report older than 3 h → red (G5). Every failure lists the source
  and its error.
- **A mismatch** shows, at the top of the collection's page and the reserves index: what failed (which item, which source said
  what, at which block), since when, what it means for a holder ("custody of #9180 is not where the books say; the operator and
  guardians are paged; redemptions of other items continue; do not deposit"), and a link to the incident log (G10). The monitor
  already pages SEV1 on a custody miss (`phase1-nft.md` §6); the page's red is the public face of the same check and is itself a
  canary alert (G5, G9).
- **Transient states are not mismatches:** a purchase between its fill and its mint, a deposit before finality, a delivery mined
  but not yet recorded, a stray going back: shown in (11) with their state and age; they turn amber only past their own timeouts
  (the monitor's thresholds: `Arrived` 24 h, a resolved ticket 1 h, an unjudged inbound transfer 2 h).

---

## 4. How a user verifies it themselves

1. **In the browser ("Check it yourself").** The page ships a small verifier that runs in the visitor's browser: it reads
   `totalSupply`, `held`, `openTickets` and the item list from a 4663 provider, and `ownerOf` for each item from an Ethereum
   provider — **defaulting to two public providers, with a field to paste any RPC URL** (the visitor's own node or wallet
   provider). It recomputes the equation locally and shows each call it made. No Rook server is involved beyond serving the page.
2. **From a terminal.** `npx @rook/reserves verify --collection milady --eth-rpc <url> --rh-rpc <url>` (the indexer's pure
   `ledger`, `invariant` and `verify` modules, packaged; keyless; exits non-zero on red). The same code the page runs.
3. **By hand.** Every item row links to its Etherscan token page (owner), its arrival transaction, the wrapper on the Robinhood
   Chain explorer, and the module's balance. A reader who trusts nothing can check one item in a minute.
4. **The math, spelled out.** The page explains the equation in one paragraph and shows its terms with their sources, so a reader
   knows what "green" asserts and what it does not (§6).

---

## 5. Trust and security model

- **What the canister must judge:** nothing; the page is read-only.
- **What can go wrong:** the indexer lies or is compromised (hence the browser and CLI verifiers, which do not use it); a provider
  lies (two must agree; the visitor can add their own); a collection's contract changes `ownerOf` semantics (the shortlist has no
  proxy, `phase1-nft.md` §1.1); a page shows green on stale data (every number carries its age; stale is never green).
- **New attack surface:** a public endpoint (`/v1/reserves`) that already exists; the browser verifier makes RPC calls from the
  visitor's machine to the providers they choose (no secrets involved). Phishing clones of the page: the page asks for no wallet
  connection.

## 6. Interaction with the existing invariants, and what the page does not prove

- It **checks** "every unit 1:1 backed by ETH or an NFT" per collection (the equation), per-collection isolation (each collection's
  books and module are separate rows; the custodian address is shared and each token is attributed by collection), the caps (12)
  and, via the monitor's argument diff, the timelock's pending changes (a link to the pending proposal, if any).
- It **does not prove** that nothing can leave between two reads, that a timelocked upgrade will not move custody (it shows
  pending proposals, the 72 h, the guardians), or that the floor is right (the floor's own checks are the monitor's D8). It says so.

## 7. Phasing

1. **Now:** the indexer delta (EthNftWrapper events are already folded, `ledger.ts`; add finality, the arrival checks 4 and 5, the
   canister's records 7–9, the not-in-books list 11, the provider pair), the page's Ethereum rows, and the browser verifier; test
   against the dry run's five-collection fork and fixtures.
2. **At the mainnet rehearsal** (`mainnet-rehearsal.md`): the page's first real green, recorded as a pass criterion.
3. **After `bids.md`:** row 13. **After `airdrop-passthrough.md`:** the airdrop ledger.

## 8. Open decisions for the operator

- **P1.** The cadence (10 min) and the stale thresholds (1 h unverified, 3 h red).
- **P2.** The default provider pair for the site (recommended: two keyless providers independent of the canister's and the
  monitor's).
- **P3.** Publish the CLI verifier as an npm package (recommended) or as a script in the repository only.
- **P4.** Show the canister's records (7–9) at all (recommended: yes, labelled as the canister's word).

## 9. Build and review estimate [A]

- Indexer: ≈ 3–4 days (finality reads, arrival receipts, the canister query, the Transfer scan, the new report fields, tests).
- Site: ≈ 2 days (rows, the mismatch banner, the browser verifier).
- CLI packaging: ≈ 1 day.
- Review: ≈ half a day (that green cannot be reached with a failing or stale source; the tests exist for v5 and are extended).
