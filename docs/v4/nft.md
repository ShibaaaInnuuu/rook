# v4 — coins whose fees sweep an NFT floor (any EVM chain)

Status: design, 2026-09-23. Contracts built on this design are in `src/nft/`, and their tests are in `test/nft/`.

A creator launches a Pons coin paired, for good, with an **NFT listing**. The listing names a collection on **any EVM chain**
(a chain id plus a contract address) and **one fate** for everything bought: `Raffle` or `Burn`. The coin's creator fees buy that
collection's floor. What is bought is raffled to the coin's holders (drand plus a challengeable snapshot) or burned. Nothing else
can happen to it.

There are two cases of the same listing:

| listing's chain | who buys | where the NFT sits | vault |
|---|---|---|---|
| 4663 (Robinhood Chain) | **anyone**, through Seaport, checked by the contract | the vault, on 4663 | `NftSweepVault` |
| any other EVM chain | the **canister** (agent C), whose EVM key nobody holds | the canister's address on that chain | `NftRemoteVault` |

Both vaults share the same money-in, attribution, price cap, raffle and burn code (`NftVaultBase`). They differ only in how a
purchase happens and how an NFT leaves.

## 1. Research: what was read

**Seaport 1.6 is on 4663.** `0x0000000000000068F116a894984e2DB1123eB395` has 23,981 bytes of code and `name()` = `"Seaport"`. This
was read with `eth_getCode` / `eth_call` against `https://robinhood.drpc.org` on 2026-09-23. The canonical 1.5 address, the
conduit controller and the older addresses below have **no code** on 4663:

- `0x00000000000000ADc04C56Bf30aC9d3c0aAF14dC` (Seaport 1.5)
- `0x000000000000AAeB6D7670E522A718067333cd4E`
- `0x1E0049783F008A0085193E00003D00cd54003c71`

Only 1.6 is there, so it is the only sweep target the deploy allow-lists.

**NFT activity on 4663 is real, and small.**

- OpenSea added Robinhood Chain and reported about 80K secondary sales and 50K mints in its launch period
  ([OpenSea blog](https://opensea.io/blog/articles/robinhood-chain-is-live-on-opensea), [OpenSea on X](https://x.com/opensea/status/2080741551941071243)).
- KuCoin reported seven 4663 projects past 1,500 ETH of combined volume on 2026-07-23
  ([KuCoin](https://www.kucoin.com/news/flash/robinhood-chain-nfts-surge-in-activity-seven-projects-hit-1500-eth-in-trading-volume)).
- For scale: Ethereum did about $720M of NFT volume in Q1 2026 and Base about $122M in 2025
  ([coinlaw](https://coinlaw.io/nft-market-growth-statistics/), [CoinGecko](https://www.coingecko.com/en/nft)).
  The blue chips, and most of the liquidity, are on Ethereum, with Base second.

**Which collections exist on 4663.** Nosh's `CollectionRegistry` on 4663 (`0x83B35632Ef70ab85DD5B01180FdD7856b68E4B0f`) was read
end to end on 2026-09-23 (`count()`, `collectionAt(i)`, `vaultOf(collection)`). It lists **53 collections**:

- **41** have no code on 4663. These are Nosh's "external" collections on other chains.
- **12** are real ERC-721s on 4663 (`supportsInterface(0x80ac58cd)` = true):

| collection | address | supply | Nosh vault holds (NFTs / ETH) |
|---|---|---:|---|
| Hashcats | `0xca75df55cc9c476db27a7375d1fc8e794cf80721` | — | 0 / 0.069 |
| StonkBrokers | `0x539cdd042c2f3d93ebc5be7dfff0c79f3b4fabf0` | 4,444 | 0 / 1.608 |
| Chain Mancers | `0x797a2e030b7e49107c8f07bf0300ea9cae88ca57` | 5,000 | 0 / 0.012 |
| CASHCAT | `0xe3b34c4bb0f12c82143745eee6a6cf4e3154b1fa` | 10,000 | 0 / 0.007 |
| Rare Friends Genesis | `0x116eaa62241751e0c98da43d458600c6c17cd361` | — | 0 / 0.101 |
| RH MACHINES | `0x8c71d170fbd94bcba93bb08fc2cfd0e8620cd9ce` | 10,000 | 0 / 0.001 |
| Opencatz AI | `0xf0474980e09c3023655a9ca3e71a763358214efb` | 5,000 | **35** / 0.004 |
| QUOTRONS | `0x027aca2794e44f24950d81227dcd516ffbb49d6e` | — | 0 / 1.362 |
| The Royal Mechanica | `0x99374499930204e1a7f76299e5e8c71fcd0e3aac` | 4,242 | **22** / 0.001 |
| AGNTS | `0x57efd86cf3ab1812cb47f0fd3c3d5bf0efb1f2d5` | — | 0 / 0.003 |
| Rekt Tradooors | `0x7b3ecfa33657de415ff269dc97dfa82954cee706` | 10,000 | 0 / 0.057 |
| Ape Hood | `0x47484196a99df52bf8fe53fb664c54821c13219d` | 3,333 | **6** / 0.010 |

Across all twelve of Nosh's 4663 vaults there are 63 swept NFTs and about 3.2 ETH not yet spent. These collections are listed on
OpenSea through Seaport 1.6. On 4663, OpenSea is the only venue whose protocol has code on chain.

**Conclusion, stated plainly.** 4663 has native collections worth sweeping, and a contract-enforced Seaport vault is the best
trust model this system has ever had for them: nobody holds the money at any point. But 4663 NFT liquidity is thin, a few dozen
floor NFTs across a dozen collections, and most volume is on Ethereum and Base. **The canister path (agent C) will carry most of
the volume.** `NftRemoteVault` is the 4663-side accounting for that path. `NftSweepVault` is the path with the stronger guarantee,
for the collections that live here.

## 2. The listing: any EVM chain

`NftRegistry` is a **new contract**. It is not a new `Kind` in `ListingRegistry`. `ListingRegistry.sol` changes only by one
**additive** hook that reserves the canister's NFT buyer address against role rotations (§2.1). The listing itself stays out of it
for three reasons:

1. **ABI stability.** `ListingRegistry.listing(id)` returns a struct that three decoders read: `web/src/lib/abis.ts`, the
   canister's `policy::ret_listing`, and `script/golive-check.sh`. A new enum value would put `kind = 2` in front of each of them.
   An NFT listing's fields (chain id, collection address, policy) also do not fit the Bitcoin-shaped struct (`btcVault`,
   `membershipRoot`, `divisibility`).
2. **Size.** ListingRegistry is about 41.4 KB against 4663's 98,304-byte limit, because it embeds both Bitcoin vaults' creation
   code. The NFT vaults live behind their own contract instead.
3. **Ownership.** Agents A and C also read ListingRegistry. The only change is additive (§2.1): no existing function, struct, event
   or error moves, so nothing A or C decode changes.

`NftRegistry` borrows **authority** from ListingRegistry and keeps no roles of its own:

- the owner (multisig): `listingRegistry.owner()`
- the keeper that posts caps and snapshots: `listingRegistry.keeper()`
- the treasury: `listingRegistry.treasury()`

A role rotation therefore goes through ListingRegistry's existing seven-day public delay, and applies to both systems at once.

```solidity
enum Policy { Raffle, Burn }
struct NftListing {
    uint64  chainId;      // EIP-155 chain id where the collection lives: 4663, or 1, 8453, …
    Policy  policy;       // fixed for ever: what happens to everything this listing buys
    bool    paused;       // stops buying (sweep / announce / pull); never stops money in, raffles or burns
    address collection;   // the ERC-721 contract on `chainId`
    address vault;        // NftSweepVault if chainId == 4663, else NftRemoteVault (on 4663 either way)
    address buyer;        // the canister's EVM address for a remote listing; zero for a 4663 listing
    string  name;
}
```

- `listNft(chainId, collection, policy, name, buyer, maxInFlight)`: owner only. It refuses a duplicate
  `(chainId, collection, policy)`. A native listing's collection must have code, and a remote listing's buyer must be a code-less
  account, as `golive-check` checks for v2 buyers. **A collection may be listed twice, once per policy**, so that burn money and
  raffle money are in **different vaults** and cannot mix, by construction rather than by bookkeeping.
- `ref(id)` returns the CAIP-10-style string `eip155:<chainId>:<collection>` for UIs and the canister.
- Seaport allow-list, `isSweepTarget(target)`:
  - Seaport 1.6 is allow-listed at construction.
  - A new target takes `CHANGE_DELAY` (7 days, both clocks) in public.
  - Removing a target is instant, because removal can only stop sweeps.

**Pairing and policy are fixed at launch.** `NftLauncher.launch(p)` names the listing. The listing's policy is immutable in its
vault. The coin's router is an EIP-1167 clone initialised once with that vault. Nothing afterwards can change any of the three.

### 2.1 The canister's buyer address is reserved in ListingRegistry

A remote listing's `buyer` can pull ETH to itself (§6). It must therefore never also be a key that posts the cap it pulls against,
pauses the pulling, or writes it off. That is ListingRegistry's existing rule for v2 buyers, kept as the set `isAutoBuyer`. The NFT
side now joins that set:

- `ListingRegistry.wireNft(nftRegistry)`: owner only, **once**, like `wire`. It names the one contract allowed to reserve buyers.
- `ListingRegistry.reserveBuyer(buyer)`: callable only by that contract. It refuses (`SameKey`) an address that is the owner, the
  pending owner, the keeper, purchaser or attestor, a **pending** keeper, purchaser or attestor, the treasury or the pending
  treasury. Otherwise it sets `isAutoBuyer[buyer] = true` and emits `BuyerReserved(buyer)`. A buyer named twice is fine.
- `NftRegistry.listNft` calls `reserveBuyer` for every remote listing. An unwired ListingRegistry refuses (`NotNftRegistry`), so a
  remote listing cannot be made before the hook is in place.
- Rotations **onto** a reserved address are refused wherever a role is proposed:
  - `proposeRoles` (unchanged);
  - `proposeTreasury` (new check);
  - `transferOwnership`, Ownable2Step's first step (new check).
- The reverse order is covered too. Because `reserveBuyer` refuses an address that is **pending** for any role, a rotation that
  was proposed first can never land on an address reserved later. The acceptances therefore need no second check.
- The v2 path gets the same pending-role check: `listInscriptionsAuto` / `listRuneAuto` also refuse a pending role, the pending
  owner and the treasury.

Nothing is loosened and no delay is shortened. A reservation can only **refuse** a later rotation, so it needs no delay of its own.
It is permanent, as a v2 buyer's is: the canister's key is immutable in its vault, so there is nothing to un-reserve. Every
existing ABI (`listing(id)`, `isAutoBuyer`, the roles, the events) is unchanged; three functions, one error and two events are
added. ListingRegistry stays over EIP-170's 24,576 bytes, as it already was, and well under 4663's 98,304 (§11).

## 3. Money in, and attribution by fees earned

`NftFeeRouter` is a clone per coin with a constant split: 80% to the vault, 20% to the treasury.

- `harvest()` is **permissionless**. It sweeps the curve (`try`), calls `escrow.claim()`, and splits **only what the escrow paid**.
- **Attribution is the Pons escrow's record:**
  - The vault is credited `claim × 80%`, attributed to the coin.
  - ETH that reaches the router any other way (a direct payment) is **not attributed**. It goes to the treasury.
  - A plain payment to a vault reverts: there is no `receive` except for Seaport's refund during a sweep.
  - `vault.deposit(coin)` accepts only the coin's own router, and only for a coin whose listing is this vault's.
- ETH forced in by `selfdestruct` is never counted and never spendable.

The vault keeps the AssetVault ledger: `unspentOf[coin]`, a FIFO queue of coins with `MIN_ENTRY = 0.01 ether`, `MAX_PAYERS = 64`,
and `attributed` / `waiting` totals. A purchase **charges** coins first in, first out. Each NFT bought is attributed to one coin,
**the coin that paid most of that NFT's price** (`fundedBy`). That coin's holders are the raffle's ticket holders. The same rule
applies in both vaults.

## 4. The floor cap

- The keeper posts `capWei`, the most that may be paid per NFT, with `capRef` saying where the number came from.
- The cap expires after **1 hour on both clocks** (`capExpiry` and `capExpiryBlock`, see `lib/Wait.sol`). A sequencer that stamps a
  block an hour ahead cannot keep a stale cap alive.
- The cap is the keeper's word about a market. It bounds loss; it does not prove a price.

## 5. The 4663 sweep: `NftSweepVault`, contract-enforced

`sweep(target, data, candidates, minBought)` is **permissionless**. Anyone may send it. The vault **decodes** `data` as Seaport 1.6
calldata, checks every order in it, **re-encodes** what it checked and forwards only that (§5.1). It then measures the outcome and
checks that too.

1. The listing is not paused, `target` is allow-listed (`isSweepTarget`), and the cap is fresh on both clocks.
2. `candidates` is 1–`MAX_CANDIDATES` (50) token ids, with no duplicates and none already owned by the vault. `minBought ≥ 1`.
3. Before the call it records `nftBefore = balanceOf(vault)` and `ethBefore = address(this).balance`.
   Before that, `data` is decoded and checked (§5.1), which also gives `total`, the most ETH the orders can take.
4. It calls `target.call{value: min(total, spendable)}(reencoded)` inside the vault's reentrancy lock. The same
   lock covers `deposit`, so a seller re-entering a router's `harvest` cannot move ETH in mid-measurement. During the call,
   `receive` accepts ETH only from `target` (Seaport's refund of what an order did not use). `onERC721Received` accepts only the
   listing's collection, and only during a sweep.
5. After the call:
   - `arrived` = the number of candidates the vault now owns.
   - It requires `balanceOf(vault) − nftBefore == arrived`. So **no id outside the candidate list arrived** (`UnlistedArrival`),
     and nothing arrived twice.
   - It requires `arrived ≥ minBought` (`NotEnoughBought`).
   - `spent = ethBefore − address(this).balance` must satisfy `spent ≤ capWei × arrived` (`OverCap`) and `spent ≤ spendable`.
6. `spent` is charged to the coins FIFO. Each arrived id is recorded as an acquisition with `fundedBy`, `pricePaid` and `txRef`
   (for a native acquisition, the block number).

The vault never approves anything to anyone. A Seaport order that names the vault as **offerer** can therefore move nothing out:
Seaport skips the signature for a caller-offerer, but the transfer still needs an approval the vault never gives, and native ETH
cannot be offered. All the ETH that can leave during a sweep is `value`. The only NFTs that can leave are the listing's, and only
through `claimWin` or `burn` below.

### 5.1 What the vault accepts in `data`

Only three Seaport 1.6 entry points. Everything else is refused with `UnsupportedCall(selector)`: `fulfillOrder`,
`fulfillAdvancedOrder`, `fulfillAvailableOrders`, `matchOrders`, `matchAdvancedOrders`, `cancel`, `validate`, `incrementCounter` and
any other selector.

| selector | what it buys | why it is in the subset |
|---|---|---|
| `fulfillBasicOrder` (`0xfb0f3ee1`) | one NFT | OpenSea's usual single fill; a fixed layout |
| `fulfillBasicOrder_efficient_6GL6yc` (`0x00000000`) | one NFT | the same parameters, Seaport's cheaper entry |
| `fulfillAvailableAdvancedOrders` (`0x87201b41`) | up to `candidates.length` NFTs | OpenSea's sweep ("buy N") call |

`matchAdvancedOrders` is refused because in a match the fulfiller writes the fulfillments and may add orders of its own, including
one whose offerer is the vault. Basic and "available" fills cover every sweep a vault needs.

For **every order**, the vault requires:

- **Offer:** exactly one item, an ERC-721 (not criteria-based), token = the listing's collection, identifier ∈ `candidates`, amount 1
  (basic: `basicOrderType` 0–3, the ETH-for-ERC-721 route; `offerAmount` = 1).
- **Consideration:** native ETH only (item type `NATIVE`, token 0, identifier 0).
- **No tip:** `consideration.length == totalOriginalConsiderationItems` (basic: `additionalRecipients.length ==
  totalOriginalAdditionalRecipients`). Seaport lets the **fulfiller** append items past that count, and the offerer's signature does
  not cover them. That is the whole of the tip channel, and it is closed.
- **Per-NFT price:** the sum of the order's ETH (for each item, the larger of its start and end amount) ≤ `capWei`. This is stricter
  than the old aggregate bound: one expensive NFT can no longer ride on a cheap one.
- **Nobody paid is the caller:** the offerer and every consideration recipient differ from `msg.sender`, from `tx.origin` and
  from the vault itself. The vault cannot be an offerer, which would let Seaport skip the signature for its caller.
- **Plain orders:** order type 0–3 (`CONTRACT` orders, whose offerer generates the items at fill time, are refused); for advanced
  orders `numerator == denominator` (a full fill), no criteria resolvers, and `recipient` is the vault or zero (then set to the vault);
  `fulfillerConduitKey == 0`; at most `candidates.length` orders.

**Re-encoded, not forwarded.** The vault decodes with Solidity's ABI decoder, which reverts on truncated data, out-of-range offsets,
dirty address bits and out-of-range enums. It then forwards `abi.encodeWithSelector(selector, decoded…)`, never the caller's bytes.
Seaport therefore executes exactly the values that were checked, and a parser differential (non-canonical offsets that Seaport and
the vault would read differently) has nothing to act on. For basic orders, the canonical encoding is also the only one Seaport
accepts.

Restricted orders (a zone) are accepted, because OpenSea may use its signed zone. A zone is called by Seaport under the vault's
reentrancy lock and cannot move the vault's ETH or NFTs.

**NFTs leave only by raffle or burn.** There is no withdraw, no `release`, no owner path, and no `approve`.

- In a `Burn` vault, anyone may call `burn(acqId)`, which transfers the NFT to `0x…dEaD`.
- In a `Raffle` vault, a claimed winner receives the NFT in `claimWin`.

An NFT that was sent to the vault outside a sweep (a plain `transferFrom`) is not an acquisition. It is stuck for ever. That is
safe, but wasteful.

## 6. Other chains: `NftRemoteVault`, canister-bought

This is AutoBuyerVault's shape with an EVM receipt.

- `buyer` is immutable: the canister's threshold-ECDSA address. That **same address** holds what it buys on the collection's
  chain.
- `maxInFlight` bounds what can be out at once, and `MAX_OPEN = 3`.
- **Purchase flow:**
  1. The buyer **announces** `(maxEth, count, capRef)`. `maxEth` must be at most `capWei × count` and within `spendable` and
     `maxInFlight`.
  2. After **one public hour on both clocks**, the buyer may **pull** the ETH, to itself only.
  3. It must **receipt** each NFT within `SETTLE_DEADLINE` (3 days): `(tokenId, purchaseTxHash)`. Token ids are unique per vault.
  4. It **settles** when all `count` have arrived, or settles short once ETH for the missing ones is **returned**
     (`returnFunds`, given back to the paying coins pro rata, clamped).
- **Overdue handling:**
  - An overdue purchase blocks new announcements.
  - The owner may write it off after the deadline. **Anyone** may, 7 days later.
  - An unpulled announcement may be cancelled by the owner, or by anyone 7 days after it became ready.
- What this does not protect against: a false receipt. The contract checks what this chain can check (uniqueness, count, price per
  unit). Whether `purchaseTxHash` really bought `tokenId` on chain `chainId` into `buyer` is checkable by anyone with that chain's
  RPC, and by nothing here. The canister's own code (agent C) is the other half of that trust.

## 7. Settlement: raffle or burn, never both

The fate is per **vault** (§2), so one vault's money only ever buys NFTs for one fate. There is no function that moves an
acquisition from one fate to the other.

**Raffle** (both vaults, the Distributor's machinery rebuilt around an ERC-721 lot):

1. **Snapshot.** The keeper `openDraw(acqId, root, totalTickets, uri)`. Leaves are `(holder, firstTicket, endTicket)` for holders of
   `fundedBy`, balance-weighted, with the same leaf encoding as `Distributor.ticketLeaf`.
2. **Challenge window.** 15 minutes on both clocks. During it, the owner may `cancelDraw`. It is the owner's only lever.
3. **Pin.** Anyone may `requestDraw`, which pins `beacon.roundAt(now) + 20`, a drand round that did not exist yet. If drand skipped
   the round, anyone may `redraw` 24h later.
4. **Resolve.** Anyone calls `resolve` with the round's signature. The winning ticket is
   `keccak(randomness, vault, acqId) mod totalTickets`.
5. **Claim.** Anyone calls `claimWin(acqId, winner, first, end, proof)`.
   - **On 4663** the NFT is transferred to `winner` in the same call.
   - **Elsewhere** the win is recorded, and delivery goes to the **winner's same EVM address on the collection's chain**. The
     winner may redirect once, to another address, within the first hour (`setDestination`, both clocks). This is for a winner
     whose 4663 account is a contract with no twin on that chain. After that hour, the buyer `recordDelivery(acqId, txHash)`.
     After 7 days anyone may `flagOverdue`.
6. **Void.** If the winning ticket cannot be claimed for 7 days (the list had a gap), the owner may `voidDraw`, which returns the
   lot to a new public snapshot.

**Burn:**

- **On 4663**, `burn(acqId)` is permissionless: `transferFrom(vault, 0x…dEaD, id)`.
- **Elsewhere**, the buyer sends the NFT to `0x…dEaD` on that chain and `recordBurn(acqId, txHash)`. After 7 days without it,
  anyone may `flagOverdue`.

## 8. Optional: an ERC-721 wrapper (a collection as an ERC-20)

Written down, not built. It would be an ERC-20 minted 1e18 per deposited NFT of one collection and redeemable for any held id.
That is the NFTX/"floor token" shape, and it would let a coin be **paired in a pool** against the collection itself. Reasons not to
build it now:

- **It is not cheap.** Redeem-any creates adverse selection, where the best id goes first. It needs a random-redeem or fee design.
- **It needs liquidity.** A pool needs NFTs that 4663 does not have in depth (§1).
- **It would change the vault's promise.** "NFTs leave only by raffle or burn" becomes "…or by redemption", a third exit.

If it is built later, it should be its own contract that the **Burn** vault could sell into, never a path out of a Raffle vault.

## 9. What agents A and C get from this

- **`ListingRegistry` changes only additively (§2.1).** Nothing A or C decode moves: `listing(id)`, the roles and `isAutoBuyer`
  keep their ABI. New: `nftRegistry()`, `wireNft(address)`, `reserveBuyer(address)`, `event NftRegistryWired`, `event
  BuyerReserved`, `error NotNftRegistry`.
- **For C (the canister):**
  - Read `NftRegistry.listing(id)` for `chainId`, `collection`, `policy`, `vault` and `buyer`, and `NftRegistry.ref(id)` for
    `eip155:<chain>:<addr>`.
  - Call on the remote vault:
    - `announce(maxEth, count, capRef)`
    - `pull(id)`
    - `receipt(id, tokenId, txHash)`
    - `settleShort(id)`
    - `returnFunds(id)` (payable)
    - `recordDelivery(acqId, txHash)`
    - `recordBurn(acqId, txHash)`
  - All of these are `onlyBuyer`, except `returnFunds`, which anyone may call.
  - The canister's buyer address must be code-less and **not** a ListingRegistry role, current or pending. `listNft` checks it and
    reserves it in `ListingRegistry.isAutoBuyer`, so no later rotation can land a role on it (§2.1).
  - Sweeps on 4663 must be sent as Seaport 1.6 `fulfillBasicOrder`, `fulfillBasicOrder_efficient_6GL6yc` or
    `fulfillAvailableAdvancedOrders` calldata, with no tips and with the vault as recipient (§5.1).
- **For A:** nothing in `NftRegistry` touches the Bitcoin wrapper path. If A wants a raffle machine for ERC-721 lots, the one in
  `NftVaultBase` is self-contained.

## 10. Open problems

1. **Tip skimming by a permissionless sweeper: the tip channel is closed; self-dealing within the cap is bounded, not closed.**
   Before, `sweep` forwarded any calldata and checked only balances, so a sweeper could fill a real listing at price `P` and append
   a Seaport tip to itself, taking `cap − P` per NFT. Now the vault decodes and re-encodes the calldata (§5.1). Each measure,
   judged by whether it actually stops someone who wants the vault's money:
   - **No appended consideration (`length == totalOriginal…`): closes the tip.** A fulfiller can add value to itself only through
     items past the signed count. With none, every wei goes to a recipient the **offerer signed for**.
   - **Offer bound to the collection and a candidate id, one ERC-721 per order: closes paying for nothing.** Before, an order with an
     empty offer (or another collection's token) paying an accomplice rode inside the aggregate `cap × arrived` bound next to a
     cheap real NFT. Every order must now deliver one listed NFT.
   - **Per-order price ≤ cap: closes averaging.** Before, one NFT above the cap could be bought if a cheap one in the same sweep
     made up the average.
   - **Re-encoding: closes parser differentials.** Seaport runs exactly what was checked.
   - **`CONTRACT` orders, criteria, partial fills, conduits, other recipients, other selectors refused:** these are surface
     reduction. They close paths that would need separate reasoning (for example, a contract offerer generating different items at
     fill time).
   - **Offerer and recipients ≠ `msg.sender` / `tx.origin`: theater against anyone deliberate.** It stops a naive self-dealer and
     costs nothing, but a second address, or a contract whose deployer is not the sender, gets around it. It is kept as a speed
     bump and not counted as a defence.
   - **A minimum order age ("listed before the cap was posted"): pure theater, not built.** An order's `startTime` is whatever the
     offerer signs. A listing created a second ago can carry `startTime = 0`. Nothing on chain records when an order was
     published.

   **What stays open, and its bound.** A sweeper who owns an NFT of the collection (or colludes with a holder) can list it at up to
   `capWei` and fill that listing through the vault from another address. The vault then buys a **real NFT of the listed
   collection** at a price ≤ the cap, which is the purchase it exists to make, but possibly above the true floor. No on-chain
   check can tell this seller from an honest seller asking the same price. Only the keeper's cap separates them, because the chain
   cannot see the rest of the order book. The loss is bounded:
   - per NFT: `capWei − floor`, where `floor` is the cheapest honest ask at the time;
   - per sweep: `(capWei − floor) × arrived`, with `arrived ≤ 50`, and never more than `attributed`;
   - per cap: the cap lives for an hour on both clocks (§4), and the keeper can post a tighter one at any time.

   The same premium was always available to any honest seller who asked `capWei`, so the permissionless sweeper adds no loss
   beyond the cap. What it could add before, taking value **without** delivering an NFT (tips, empty offers, averaging), is gone.
   Closing the rest would need either an on-chain order book (the vault would need to know the cheapest ask) or a trusted
   sweeper. A trusted sweeper would be a keeper able to self-deal in the same way, which is not better.
2. **A malicious collection contract** can lie in `ownerOf` / `balanceOf`. Collections are owner-listed, and a native listing must
   have code. The fork test pins two real ones.
3. **4663 liquidity** is thin (§1). A sweep will often find nothing under the cap. Money then waits, attributed.
4. **Remote receipts are the canister's word** (§6). Checking them against the other chain needs a light client or an oracle.
   Neither exists here.
5. **Snapshot correctness** is the keeper's word, open to challenge for 15 minutes, as in v1. Pools and contracts should be
   excluded from tickets. That is the keeper's job, and the challenge window is where it is checked.
6. **Closed: the canister's NFT buyer address is reserved in `ListingRegistry.isAutoBuyer` (§2.1).** Before, `listNft` refused a
   buyer that was currently a role, but ListingRegistry did not know about NFT buyers, so a later rotation could land on one. Now
   `listNft` reserves the buyer through `ListingRegistry.reserveBuyer`. It checks current **and pending** roles, owner and treasury,
   and every place a role is proposed (`proposeRoles`, `proposeTreasury`, `transferOwnership`) refuses a reserved address. The seven-day delays are unchanged. `golive-check` reads the reservation back for every remote listing.

## 11. Verification

### 2026-09-23, branch `nft-harden` (§2.1 and §5.1, closing §10.6 and the tip in §10.1)

- **`forge build`:** clean. `forge build --sizes`, runtime / initcode in bytes: ListingRegistry 41,353 / 41,715 (was 40,845);
  NftRegistry 42,799 / 43,109 (was 35,514, since it carries the sweep vault's creation code); NftSweepVault 19,300; NftRemoteVault
  16,054. ListingRegistry and NftRegistry were already over EIP-170's 24,576 and are deployed only to 4663 (limit 98,304, every
  script passes `--code-size-limit 98304`; `docs/deploy/contracts-mainnet.md`). Both keep more than 55 KB of margin there.
- **ABI:** ListingRegistry's compiled ABI before and after was diffed entry by entry. No entry is missing or changed. Added:
  `nftRegistry()`, `wireNft(address)`, `reserveBuyer(address)`, `NftRegistryWired`, `BuyerReserved`, `NotNftRegistry`. NftRegistry's
  and NftRemoteVault's ABIs are unchanged. NftSweepVault's functions are unchanged (`sweep(address,bytes,uint256[],uint256)`
  included); its ABI gains only SeaportGuard's seven errors. `test/nft/NftSurface.t.sol` now also
  asserts ListingRegistry's exact function list, and the `listing(uint256)` selector the three decoders use.
- **`forge test --no-match-path "test/fork/*"`:** 507 passed, 0 failed. Of these, 100 are in `test/nft/`:
  - `NftBuyerReservation.t.sol`: rotations onto a reserved buyer, a buyer named while it is a pending role, the wiring;
  - `NftSeaportGuard.t.sol`: the subset, the attack tests, and three decoder fuzzers (arbitrary bytes behind each accepted
    selector; valid calldata with a byte overwritten or truncated anywhere; structured orders accepted exactly when no rule is
    broken). Every forwarded call is re-decoded and re-checked independently of the guard.
- **Attack tests, against the old vault:** with `src/nft/NftSweepVault.sol` reverted to `40735bf` and nothing else changed, every
  `test_attack_*` fails, as does `test_aTipWithinTheCapIsRefused`, formerly `test_knownOpenProblem_aTipWithinTheCapIsPaid`. The
  money went out: self-listed order, fee to the caller, fee to `tx.origin` behind a contract, tip to an accomplice, empty offer,
  averaging over the cap, another collection, other selectors. One case was already caught by the old balance check under a
  different error: an id that is not a candidate (`UnlistedArrival`). With the new vault, all of them pass. Likewise the
  rotation test in `NftBuyerReservation.t.sol` fails before the ListingRegistry hook, because the rotation was accepted.
- **`forge test --match-path "test/fork/*" --fork-url http://127.0.0.1:8623`** (a local anvil fork of 4663, `--code-size-limit
  98304`, nothing sent to any public chain): 17 passed, 9 of them in `NftSweep.fork.t.sol`. Real signed orders for real
  StonkBrokers go through the decoder and real Seaport 1.6 accepts the re-encoding:
  - `fulfillAvailableAdvancedOrders`, with one order and with two orders including a signed marketplace fee;
  - both basic selectors.
  A real order with an appended tip is refused by the vault, and the same order sent to Seaport directly **is** filled with the
  tip paid, which shows the attack was real. `fulfillOrder`, which the old fork test used, is now refused.
- **`DRYRUN_PORT=8624 scripts/golive-dryrun.sh bare`:** ALL PASS, including step 7 (`wireNft` in the local deploy) and the new
  `golive-check` lines: `registry.nftRegistry`, and each remote buyer reserved.

### 2026-09-23, branch `nft-sweep`

- **`forge build`:** clean.
- **`forge test --no-match-path "test/fork/*"`:** 358 passed, 0 failed. Of these, 68 are in `test/nft/`:
  - unit tests
  - security tests
  - fuzz tests
  - an invariant suite of 5 invariants
  - an exact-ABI surface test
- **`forge test --match-path "test/fork/NftSweep*" --fork-url http://127.0.0.1:8607`:** 6 passed. These ran on a local anvil fork of
  4663, using real Seaport 1.6, real StonkBrokers and Chain Mancers, and a real Pons launch and escrow.
- **`DRYRUN_PORT=8617 scripts/golive-dryrun.sh all`:** ALL PASS, bare, fork and refusals, including step 7 (`script/deploy-nft.sh`)
  and the NFT read-back in `script/golive-check.sh`.
