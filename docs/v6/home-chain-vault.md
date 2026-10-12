# v6 — the vault on the NFT's home chain: no key, no canister, anyone pulls the trigger

Written 2026-09-23 on branch `home-chain-vault` (from `feb928b`), **before its code**. Everything here lives in new directories
(`src/homechain/`, `test/homechain/`, `script/homechain/`, `docs/v6/`); no existing contract, test, doc or script is edited.
Nothing is deployed and nothing was sent to any public chain: every read below was `eth_call` / `eth_getLogs` against public
RPCs, and every transaction ran on a local anvil fork.

## 0. The decision, and what it replaces

`docs/v4/eth-nft-canister.md` and `docs/v4/nft-wrapper.md` put the EVM-collection custody in the **canister**: one threshold-ECDSA
key that buys through Seaport, holds the NFTs on Ethereum or Base, mints wX on 4663 on its own word, and delivers on its own
word. `docs/v5/custody-threat-model.md` then spent 470 lines on what that key can do wrong (T1, T3, T5, T7, T10, T16) and
`docs/v5/custody-hardening.md` on caps and rate limits to bound it.

The decision this document builds: **for an EVM collection the vault is an immutable contract on the collection's own chain**
(Ethereum for CryptoPunks and BAYC, Base for Base collections), with **no key of any kind**. Buying, relisting and settling are
**permissionless triggers** that pay whoever calls them a fixed reward, modelled on PunkStrategy (§1). Custody is the contract
itself, so:

- an NFT leaves only by a raffle win proven against a snapshot, a burn to `0x…dEaD`, a Seaport sale the vault itself listed
  (YOYO), or a wrapper redemption drawn by drand — never by a signature, because there is nothing that can sign;
- the wrapper's "backing" is checkable in one `ownerOf` on the **same chain**, not a reconciliation across two;
- `docs/v4/nft.md` §10.4 ("remote receipts are the canister's word") and `docs/v4/nft-wrapper.md` §9.2 ("mint attestations
  are the canister's word") disappear: the contract holds what it says it holds.

The canister stays for **Bitcoin only** (inscriptions and runes, where nothing but a key can hold the item).

What it costs: money and tokens must cross between 4663 and the home chain. That crossing is the whole new trust question, and
it is answered with the chain's **native bridge** (§2), which inherits Ethereum's security and needs no third party, at the price
of time: about 10 minutes down (Ethereum → 4663) and about a week up (4663 → Ethereum).

## 1. PunkStrategy, read from its verified source

`0xC50673eDb3a7b94e8caD8A7D4e0Cd68864e33eDf` on Ethereum, Sourcify **exact match**, `src/PunkStrategy.sol`, solc 0.8.30.
What was read (verbatim where it matters):

- `addFees()` — the Uniswap v4 hook (or the owner) pays trading fees in; `currentFees += msg.value`.
- `buyPunkAndRelist(punkId)`, `nonReentrant`: reads `punksOfferedForSale(punkId)`; refuses unless `isForSale` and
  `onlySellTo == address(0)`; `totalRequired = minValue + reward`; refuses if `currentFees < totalRequired`;
  `buyPunk{value: minValue}(punkId)`; checks `punkIndexToAddress(punkId) == address(this)`;
  `offerPunkForSale(punkId, minValue * priceMultiplier / 1000)`; sends `reward` to `msg.sender`;
  `currentFees -= totalRequired`; `lastPunkSalePrice = minValue`.
- `processPunkSale()`: `excessEth = balance − currentFees`; requires `excessEth > lastPunkSalePrice`; `_buyAndBurnTokens(excessEth −
  reward)` swaps ETH → PNKSTR on Uniswap v4 with **`amountOutMin = 0`**, recipient `0x…dEaD`; pays `reward` to the caller.
- `reward` and `priceMultiplier` are **owner-settable** (`setReward`, `setPriceMultiplier` bounded to 1100–10000 bps).
  Constructor: `reward = 0.01 ether`, `priceMultiplier = 2000`. Read on chain 2026-09-23: `priceMultiplier() = 1200` (1.2×),
  `reward() = 1e27` wei — the owner has set the reward to a billion ETH, which makes `buyPunkAndRelist` unsatisfiable
  (`currentFees < minValue + reward` always). The machine is switched off by its owner today.
- `transferEther(to, amount)` is `onlyOwner`: **the owner can drain the contract**.

What is kept: the shape (fees in, permissionless buy at the order book's ask, atomic relist at a markup, permissionless sale
processing, a fixed reward from the budget). What is not: an owner who can set the reward, the multiplier, or take the ETH;
a buyback with no slippage bound; a sale detector that trusts `balance − currentFees` (anyone can send ETH to it and "process a
sale" that buys and burns with the sender's own money, paying themselves the reward — harmless to others, but it shows the
ledger is not a ledger). Here every parameter is **immutable**, the budget is a ledger charged per ticket, and the sale is detected by
ownership of the exact token.

Also read: CryptoPunksMarket (`0xb47e3cd837dDF8e4c57F05d70Ab865de6e193BBB`) pays a seller by crediting `pendingWithdrawals[seller]`;
the seller must call `withdraw()`. PunkStrategy never does, so its `processPunkSale` cannot fire from a punk sale at all. The
vault here calls `withdraw()` itself in `settle`.

## 2. The bridge, verified read-only

### 2.1 What 4663 settles to

Read on 2026-09-23 (`cast` / raw JSON-RPC against `rpc.mainnet.chain.robinhood.com` and `eth.drpc.org`):

| fact | value | how |
|---|---|---|
| 4663's `block.number` in the EVM vs. its RPC | `l1BlockNumber` **26,040,897** at L2 block 70,599,360 | `eth_getBlockByNumber("latest")` |
| Ethereum mainnet head at the same moment | **26,040,896** | `eth_blockNumber` on `eth.drpc.org` |
| Rollup contract, on **Ethereum** | `0x23A19d23e89166adedbDcB432518AB01e4272D94`, 10,347 B of code | `eth_getCode` |
| `Rollup.chainId()` | **4663** | `eth_call` |
| `Rollup.confirmPeriodBlocks()` | **45,818** parent blocks | `eth_call` |
| `Rollup.challengeGracePeriodBlocks()` | 14,400 | `eth_call` |
| `Rollup.minimumAssertionPeriod()` | 75 | `eth_call` |
| `Rollup.stakeToken()` / `baseStake()` | WETH `0xC02a…6Cc2` / 1 ETH | `eth_call` |
| `Rollup.validatorWhitelistDisabled()` | **false** (permissioned validators) | `eth_call` |
| `Rollup.anyTrustFastConfirmer()` | zero (not AnyTrust fast-confirm) | `eth_call` |
| `Rollup.numBigStepLevel()` | reverts → **pre-BoLD** (classic challenge protocol) | `eth_call` |
| `Rollup.inbox()` / `bridge()` / `outbox()` / `sequencerInbox()` | `0x1A07cc4BD17E0118BdB54D70990D2158AbAD7a2D` / `0xDf8755334ce7A73cCF6b581C02eA649AE3E864b3` / `0xf0ce991ea4A0d2400A4AB49b20ae333f6Dce3DE9` / `0xBd0D173EEb87D57A09521c24388a12789F33ba96` | `eth_call`, each cross-checked: `Inbox.bridge()`, `Bridge.rollup()`, `Outbox.rollup()` all agree |
| `SequencerInbox.maxTimeVariation()` | delayBlocks **28,800**, futureBlocks 300, delaySeconds **345,600**, futureSeconds 3,600 | `eth_call` |
| `SequencerInbox.isUsingFeeToken()` | false (ETH-gas chain) | `eth_call` |
| `ArbSys(0x64).arbChainID()` / `arbOSVersion()` | 4663 / 116 | `eth_call` on 4663 |
| `ArbRetryableTx(0x6E).getLifetime()` | 604,800 s (7 days) | `eth_call` on 4663 |
| Ethereum block time | 12.10 s measured in `docs/v4/eth-nft-canister.md` §1; `lib/Wait.sol` measured 12.06–12.14 s from 4663's own `l1BlockNumber` | — |

**So 4663 is an Arbitrum Nitro chain whose parent is Ethereum mainnet (chain 1)**, on the classic (pre-BoLD) rollup protocol with a
permissioned validator set. The repo's assumption of 12-second parent blocks holds. The Robinhood docs
(`docs.robinhood.com/chain/bridging`) say the same in words: "Ethereum (L1)", "deposits typically confirm within 10 minutes",
withdrawals go through "a 7-day challenge period, a standard requirement of Arbitrum's fraud proof system", then a claim on L1;
the canonical UI is `portal.arbitrum.io/bridge?destinationChain=robinhood-chain`. The docs also name fast bridges: Stargate,
Transporter (CCIP), **Relay**, Across, LiFi, 0x.

One correction to the repo's own notes: `docs/security/audit-method-v1.md` says force inclusion through the delayed inbox is
"on a 24-hour window". Measured: `delayBlocks = 28,800` ≈ **4 days** at 12 s, `delaySeconds = 345,600` = 4 days. Force inclusion
on 4663 takes four days, not one.

### 2.2 The token bridge (standard ERC-20 gateway)

A web search returned an "L1 Gateway Router" of `0xF6F11aAEE80875776C264d93B37B34cE437382D1`. **It has no code on Ethereum.** The
real addresses were read from the chain itself, starting from the L2 router the same search gave, and every link was checked
both ways:

| contract | chain | address | checked by |
|---|---|---|---|
| L2GatewayRouter | 4663 | `0x1E324B9316138CA9a73F960213621AD1aaf01B89` (4,407 B) | `counterpartGateway()` = the L1 router below |
| L2ERC20Gateway (standard) | 4663 | `0xfd9b17206278C16DdaacF6AC8f05dBf97EdCb31e` | `L2GatewayRouter.defaultGateway()`; its `counterpartGateway()` = the L1 gateway below |
| L2 BeaconProxyFactory | 4663 | `0xc302CcbC357A39a7231A681C61943b2DC032Dd51` | `L2ERC20Gateway.beaconProxyFactory()` = `L1ERC20Gateway.l2BeaconProxyFactory()` |
| **L1GatewayRouter** | Ethereum | **`0x6a2E3a1e16FC29f27Ce61429746D558d656975bB`** (4,407 B) | `counterpartGateway()` = `0x1E32…1B89`; `inbox()` = the Inbox above |
| **L1ERC20Gateway** (standard) | Ethereum | **`0x85001CC4867C5e1C22dA4B79BB8852B9e2a06da0`** | `L1GatewayRouter.defaultGateway()`; `router()` = `0x6a2E…5bB`; `counterpartGateway()` = `0xfd9b…b31e` |
| L1 WETH gateway | Ethereum | `0xF7e12b9614b509C747ab4423bC4ACF923759Cf1B` | `L1GatewayRouter.getGateway(WETH)` |
| Bridged WETH on 4663 | 4663 | `0x0Bd7D308f8E1639FAb988df18A8011f41EAcAD73` | `calculateL2TokenAddress(WETH)` on **both** routers; equals the WETH on `docs.robinhood.com/chain/contracts` |

`L1GatewayRouter.getGateway(anyUnregisteredToken)` returns the standard gateway, so a fresh ERC-20 on Ethereum (a `HomeWrapper`)
bridges through the standard gateway with no registration: its 4663 twin is a `StandardArbERC20` beacon proxy at
`calculateL2TokenAddress(wX)`, **deterministic before the first deposit**, created by the gateway on the first deposit with the
L1 token's name, symbol and decimals.

### 2.3 Times and costs, per direction

| leg | mechanism | time | who pays what |
|---|---|---|---|
| ETH **down** (Ethereum → 4663) | `Inbox.createRetryableTicket` (a retryable that calls a 4663 contract with value) | included on L2 after the L1 tx is sequenced: minutes; the docs say "within 10 minutes" | the L1 sender: L1 gas + submission fee (`Inbox.calculateRetryableSubmissionFee(dataLen, baseFee)`; read: 35,680 gwei for 64 bytes at a 20 gwei base fee) + `gasLimit × maxFeePerGas` of L2 gas (4663 gas price read: 0.05 gwei). A failed auto-redeem can be redeemed by anyone for **7 days** (`ArbRetryableTx.getLifetime`) |
| ERC-20 **down** (wX → 4663) | `L1GatewayRouter.outboundTransfer(token, to, amount, maxGas, gasPriceBid, abi.encode(maxSubmissionCost, ""))` after `approve(L1ERC20Gateway)` | same as above | same as above; `extraData` must be empty for the standard gateway |
| ETH **up** (4663 → Ethereum) | `ArbSys.sendTxToL1{value}(destination, data)`; after the assertion containing it is confirmed, anyone calls `Outbox.executeTransaction(proof…)` on L1, which does `destination.call{value}(data)` | **`confirmPeriodBlocks` = 45,818 blocks ≈ 6.4 days at 12 s**, plus the time to the next assertion and its confirmation: **≈ 7 days**, as the docs say | L2 gas to send; L1 gas to execute (anyone; the keeper in practice) |
| ERC-20 **up** (wX 4663 → Ethereum) | `L2GatewayRouter.outboundTransfer(l1Token, to, amount, "")` (burns the L2 twin; the L1 gateway releases on execution) | ≈ 7 days | same |
| **Base**-home collections | Base is an OP-stack L2 whose parent is Ethereum; there is no native Base ↔ 4663 route. Native path is Base → Ethereum (7-day OP withdrawal) → 4663 (retryable): ≈ 7 days down and ≈ 14 days up | | a fast bridge (Relay: 4663 → Base quoted at ~2 s in `docs/v4/eth-nft-canister.md` §8) is the practical path; §5 |

Force inclusion (a sequencer that censors the L2→L1 send): 4 days (§2.1). Retryable lifetime: 7 days, then the ticket is dead
and its `l2CallValue` refunds to `callValueRefundAddress` **on L2** — always set to the receiver contract, never to a person.

## 3. `HomeVault`: one per collection, on the collection's chain

**One contract per policy** — `HomeVaultRaffle`, `HomeVaultBurn`, `HomeVaultYoyo`, `HomeVaultWrap` over an abstract
`HomeVaultBase` — so a deployment carries only its policy's code (a single contract with all four was 32.6 KB, over EIP-170 on
Ethereum; §9). Immutable at construction: `collection`, `isPunks` (the collection is CryptoPunksMarket, which is not an ERC-721),
`policy`, `seaport` (1.6, the same CREATE2 address on every chain), `beacon` (a `DrandEvmnet` deployed
on the home chain — BN254 precompiles exist on Ethereum and Base), `guard` (§6), `reward` (wei, e.g. 0.01 ETH), `buffer` (wei
kept back from every purchase), `markupBps` (YOYO ask over price paid, **11,000–12,000**), `listingTtl` (how long a YOYO ask
stands, e.g. 30 days), `wrapper` (§4, for `Wrap`), `inbox` and `outbox` (§2.1), `escrow` (the 4663 `FeeEscrow`, §5) and
`receiver` (the 4663 `HomeReceiver`, §5), `keeper` (posts caps and snapshots) and `owner` (the timelock: cancels a draw during
its challenge window and raises caps through the guard; nothing else), `parentBlockSeconds` (`lib/Wait.sol`; 12 on Ethereum,
2 on Base, where `block.number` is the chain's own).

### 3.1 Money in: an explicit budget, attributed to 4663 release tickets

The vault keeps a **ledger**, never `address(this).balance`: `attributed` (spendable, from queued tickets) and `waiting` (tickets
under `MIN_ENTRY`), the sum of which is the budget. ETH enters it three ways, and only three:

1. `fund(uint256 ticket)` payable, **anyone**. Credits `budget`, and `unspentOf[ticket]` in a FIFO queue of tickets with
   `MIN_ENTRY` and `MAX_PAYERS` exactly as `NftVaultBase` keeps coins (the queue is re-keyed from coins to tickets, because on
   the home chain a coin is only a number in a 4663 ticket). A ticket is the `FeeEscrow`'s release id (§5). The native path
   arrives as `Outbox.executeTransaction` → `fund(ticket)` with `msg.sender == outbox` and `outbox.l2ToL1Sender() == escrow`;
   that call is recorded as **`native`**. A fast bridge (Relay's destination call) arrives as a plain `fund(ticket)` from a
   solver; it is recorded as **`fast`**. Anyone may top a ticket up; nobody can take it out.
2. `receive()` from `seaport`: **sale proceeds**, credited to `proceedsHeld`, not to the budget. From the punks contract nothing is
   written at all: its `withdraw()` pays with a 2,300-gas `transfer`, which affords no storage write, so `settle` measures the
   balance around the `withdraw` instead (found by the tests; §9).
3. `receive()` from anyone else: credited to `budget` under ticket 0 ("unattributed"). Bridges that deliver by a plain
   transfer land here. It is still money for this collection; it just cannot be receipted to a 4663 ticket (§5).

ETH forced in by `selfdestruct` is never counted and never spendable.

### 3.2 Buying: two order sources, one check, anyone's call

Both entry points are `nonReentrant`, require the guard not paused (§6), a fresh keeper cap on both clocks (`postCap`, `CAP_TTL`
1 h, as `NftVaultBase`), and `attributed ≥ price + reward + buffer` (+ the policy's extra: Wrap's two bridge fees). Price, reward
and extra are **charged FIFO to the tickets** in one purchase; the reward is paid to `msg.sender` at the end; the acquisition's
`fundedBy` is the ticket that paid most of its price (attribution as `NftVaultBase._attribute`). `checkOrders(data, candidates)`
exposes the Seaport check as a view (it is also how the buy calls it: an external self-call, because the decoder of
`fulfillAvailableAdvancedOrders` inlined into the buy does not fit the stack under via-IR).

- **`buyPunk(punkId)`** (`isPunks` vaults only): reads `punksOfferedForSale(punkId)`; requires `isForSale`, `onlySellTo == 0`,
  `minValue ≤ capWei`; `buyPunk{value: minValue}(punkId)`; requires `punkIndexToAddress(punkId) == this`. Under `Yoyo` it then
  `offerPunkForSale(punkId, minValue × markupBps / 10,000)` in the same call, as PunkStrategy does.
- **`buySeaport(bytes data, uint256[] candidates, uint256 minBought)`** (ERC-721 vaults only): `SeaportGuard.check` from
  `src/nft/SeaportGuard.sol`, **imported, not copied**: only `fulfillBasicOrder`, `fulfillBasicOrder_efficient_6GL6yc` and
  `fulfillAvailableAdvancedOrders`; one ERC-721 of `collection` per order, a candidate id, amount 1; native ETH only; no item
  past the signed count (no fulfiller tip); per-order price ≤ `capWei`; offerer and every recipient ≠ caller, `tx.origin`,
  the vault; no contract orders, criteria, partial fills or conduits; recipient forced to the vault; **re-encoded**, never the
  caller's bytes. Then the same outcome checks as `NftSweepVault.sweep`: balance rise == candidates that arrived, `≥ minBought`,
  `spent ≤ capWei × bought`, `spent ≤ value`. During the call `receive` accepts only Seaport's refund and `onERC721Received`
  accepts only `collection`.

**The cheapest-ask hint** (punks vaults, branch `contract-fixes`, closing open problem 1): the order book is on chain, but
reading 10,000 offers is not affordable, so the vault keeps a **bounded list of hinted asks** (`MAX_HINTS` = 8 punk ids).
`hintAsk(punkId)` (anyone): the punk must be offered to anyone (`isForSale`, `onlySellTo == 0`, not by the vault, a non-zero
ask); it takes a free slot, or the slot of a hint that is no longer offered, or — the list full — the slot of the dearest hint
if it undercuts it (`HintNotCheaper` otherwise). `buyPunk(punkId)` then refuses `CheaperOffered(hintId, ask)` when any hinted
punk is offered to anyone right now at strictly less than the chosen punk's ask — each hint re-read from the order book at that
moment, so a hint that sold or was withdrawn blocks nothing. `cheaperHint(punkId, ask)` is the same loop as a view. A zero ask
is not an ask: it neither hints nor buys (`NotForSale`). What this buys: a trigger cannot take a dear listed punk while a cheaper
one has been named on chain by anyone (the keeper, a competing trigger, a holder); what it does not: name the cheapest of the
whole book — that stays the keeper's cap, an hour at a time.

What a caller can take: `reward`, once per successful buy, and nothing else. The guard forbids paying the caller in the order;
`value` is bounded by the checked total; the reward is fixed. What a caller can make the vault do: buy a real item of the
collection at up to the cap (the bounded self-dealing of `docs/v4/nft.md` §10.1; no on-chain check can see the rest of the
order book). The cap is the keeper's word, an hour at a time, and the guard's per-item cap bounds it further (§6).

### 3.3 Policies: what happens to what was bought

The policy is one per vault, immutable, so raffle money and YOYO money never share a balance (as `docs/v4/nft.md` §2).

| policy | after the buy | how the NFT leaves |
|---|---|---|
| `Raffle` | held | `NftVaultBase`'s draw, re-implemented here without the registry: keeper `openDraw(acqId, root, totalTickets, uri)` over the **paying ticket's coin's holders** (the keeper reads `FeeEscrow.ticket(id).coin` on 4663 and snapshots it there); 15-minute challenge (owner may cancel); `requestDraw` pins a future drand round; `resolve`; `claimWin` proves the winner's ticket range and **transfers to the winner in the same call** (`transferFrom`, or `transferPunk` for punks). The winner is the same 20 bytes as on 4663; the keeper's snapshot excludes contracts, which have no twin here. `voidDraw` after 7 days as before |
| `Burn` | held | `burn(acqId)`, anyone: `transferFrom(this, 0x…dEaD)` / `transferPunk(0x…dEaD)` |
| `Yoyo` | listed at `pricePaid × markupBps / 10,000` | a buyer fills the ask: punks through the order book, ERC-721s through Seaport (§3.4). `settle(acqId)` (anyone, reward) confirms the token is gone, collects the proceeds, and bridges them down for buyback (§5.3) |
| `Wrap` | wrapped into `HomeWrapper` (§4): 1e18 wX per NFT | as any wX: the vault bridges the wX down through the standard gateway to the 4663 `HomeReceiver`, which releases it to the ticket's `l2Recipient` (the coin's curve or queue) once the vault's report arrives (§5.4). Redemption is the wrapper's business |

### 3.4 YOYO relisting through Seaport without a signature

A contract cannot sign an EIP-712 order. Seaport 1.6 has two ways to accept an order from an offerer that never signs:

1. `validate(Order[])`: when `msg.sender == offerer`, Seaport skips signature verification and marks the order hash validated;
   every later fill of that hash skips the signature too (`_verifySignature` is bypassed for `isValidated` orders).
2. A `CONTRACT` order type (SIP-6 contract offerer) — refused here: it would make the vault an order generator, a much larger
   surface.

`relist(acqId)` (anyone, reward, `Yoyo` ERC-721 vaults) builds `OrderParameters` with offerer = vault, zone = 0, one ERC-721
offer item (`collection`, `tokenId`, 1..1), one consideration item (native, `ask`, recipient = vault), `FULL_OPEN`,
`startTime = now`, `endTime = now + listingTtl`, `zoneHash = 0`, `salt = keccak(acqId, relistCount)`, `conduitKey = 0`,
`totalOriginalConsiderationItems = 1`, and calls `seaport.validate`. It stores the order hash (`seaport.getOrderHash` over the
components with `getCounter(vault)`) so `settle` can check `getOrderStatus`. It is callable again only when the previous ask has
expired or the token is gone: one reward per listing, not per block.

For Seaport to move the token, the vault gives the collection-wide approval `setApprovalForAll(seaport, true)` — **only in a
`Yoyo` vault, at construction**. This is the one departure from `NftSweepVault`'s "the vault never approves anything", and it
is safe for this reason: Seaport moves an offerer's token only under an order that offerer signed or validated, or that the
offerer fills itself. The vault has no key (no ECDSA), does not implement ERC-1271 (`isValidSignature` is absent by ABI test), and
never calls a fulfil function with itself as offerer. So the only orders Seaport will ever honour with the vault as offerer are
the ones `relist` validated: one token, one ask, to the vault. `incrementCounter` (which would void them) is never called.
Raffle, Burn and Wrap vaults give no approval.

Punks relist in `buyPunk` itself (the order book takes an ask from the owner with no signature, §1), so `relist` is the fallback
for a punk whose ask somehow stands cancelled: it re-offers at the same ask and is refused while an ask stands.

### 3.5 Settling a YOYO sale

`settle(acqId)` (anyone, reward): the lot is `Listed`; the vault no longer owns the token (`ownerOf` / `punkIndexToAddress`);
for punks, `punks.withdraw()` first (§1); requires `proceedsHeld ≥ ask` (a validated full-fill order pays exactly `ask` to
the vault, and the punks market pays `minValue` in full); `proceedsHeld −= ask`; `Sold`; the guard counts the item out (§6);
then **bridge down**: `Inbox.createRetryableTicket(receiver, l2CallValue = ask − reward − bridgeFee, maxSubmissionCost,
excessFeeRefundAddress = receiver, callValueRefundAddress = receiver, gasLimit, maxFeePerGas, data = abi.encodeCall(
HomeReceiver.creditProceeds, (fundedBy)))` with `msg.value = l2CallValue + maxSubmissionCost + gasLimit × maxFeePerGas`.
`maxSubmissionCost` is `inbox.calculateRetryableSubmissionFee(data.length, block.basefee)` × 2 (a margin: the fee is judged at
the block that includes the L1 tx, which may be later than the one that built it); the L2 gas parameters are immutables
(`L2_GAS_LIMIT = 200,000`, `L2_MAX_FEE = 1 gwei`; 4663 charges 0.05 gwei today), so the bridge costs about
0.0003 ETH at a 20 gwei L1 base fee, against asks of several ETH. Both refund addresses are the receiver, so a ticket that dies
after 7 days unredeemed still refunds its value **to the receiver**, never to a person.

A settle whose token is still owned by the vault reverts (`NotSold`); one whose ask has expired unsold lets `relist` run again.

## 4. `HomeWrapper`: wX on the home chain, custody by the contract

`HomeWrapper is ERC20, IWrappedCollection`, one per collection, immutable `collection`, `isPunks`, `beacon`, `guard`,
`parentBlockSeconds`. **`custodian()` returns the wrapper itself**: the NFTs sit in this contract.

- **In**: `wrap(tokenId, to)`, anyone. ERC-721: `transferFrom(msg.sender, this, tokenId)` (the caller approved the wrapper).
  Punks: the owner first `offerPunkForSaleToAddress(tokenId, 0, wrapper)`; `wrap` then `buyPunk{value: 0}(tokenId)`, which the
  punks contract accepts because `onlySellTo == wrapper` and `minValue == 0`; the wrapper checks `punkIndexToAddress`. Mints
  `ONE_ITEM` to `to`. The guard's per-collection **item cap** applies (§6): a wrap past the cap reverts and the NFT stays with
  its owner. There is no attestation, no source tx, no `sourceUsed`: the NFT is here or the call reverted.
- **Out**: `redeem(destination)` / `redeemToBurn()` burn `ONE_ITEM` and pin `beacon.roundAt(now) + PIN_AHEAD`; `resolve`
  strictly in ticket order, drawing from the items that were free before this ticket (stamps, as `NftWrapper`); **the transfer
  happens in `resolve`**: `transferFrom(this, destination, tokenId)` (`transferPunk` for punks) inside a `try`. Success →
  `Delivered` in the same transaction, the guard counts one item out. Failure (a collection that refuses the destination) →
  `Resolved` with `deliverBy = now + DELIVERY_DEADLINE`; anyone may `deliver(ticketId)` again; after the deadline on both clocks
  anyone may `refund` (item back to the pending queue, `ONE_ITEM` back to the owner). `redraw` after 24 h if drand skipped the
  round. The ledger `totalSupply + openTickets × 1e18 == held × 1e18` and "every held item is in exactly one place" hold as in
  `docs/v4/nft-wrapper.md` §2.3, and `held` is now checkable on the same chain: `ownerOf(item.tokenId) == wrapper`.

Deliberately not built: `mint`/`mintDeposit` on somebody's word, the `EWN1` deposit binding (a holder simply calls `wrap`), the
delivery-and-refund race (delivery is atomic), and a delivery deadline that a canister has to watch.

**wX on 4663.** The vault (Wrap policy) or any holder bridges wX down: `approve(L1ERC20Gateway, amount)`,
`L1GatewayRouter.outboundTransfer(wX, to, amount, L2_GAS_LIMIT, L2_MAX_FEE, abi.encode(maxSubmissionCost, ""))` with
`msg.value = maxSubmissionCost + L2_GAS_LIMIT × L2_MAX_FEE`. On 4663 it is the `StandardArbERC20` at
`L1GatewayRouter.calculateL2TokenAddress(wX)` — computable before deployment, so pools can be planned against it. It is a plain
ERC-20 there: `IWrappedCollection`'s `held()`, `openTickets()`, `redeemToBurn()` and `refunded()` are the home chain's. A
consumer on 4663 that needs those views needs an adapter or a light-client read; none is built here (open problem 4).

**Redemption from 4663.** `L2GatewayRouter.outboundTransfer(wX_L1, to = the holder's own Ethereum address, 1e18, "")`; after
≈ 7 days (§2.3) and an `Outbox.executeTransaction` by anyone, the holder has 1e18 wX on the home chain and calls `redeem`.
Optional fast paths later: a liquidity provider on the home chain who buys 4663-wX at a discount, or a fast bridge quote —
neither is built and neither is needed for the wrapper to be correct.

## 5. Fee flow 4663 → home chain: earmark, release, receipt

### 5.1 `FeeEscrow` (4663, one per HomeVault)

Implements `INftVault.deposit(coin)` so an existing `NftFeeRouter` clone can pay it (the sweep share), and
`depositFor(coin, l2Recipient)` for a graduation budget whose wX should come back to a named 4663 contract. Both accept only
`launcher.routerOf(coin)` as the sender and only a coin whose `launcher.listingOf(coin)` is this escrow's listing; the ledger is
`NftVaultBase`'s (`unspentOf`, FIFO queue, `MIN_ENTRY`). ETH forced in is not counted.

`release(coin)` (**anyone**, reward): requires `unspentOf[coin] ≥ minRelease` (e.g. 0.05 ETH; below it the L1 execution gas
would eat the money); makes ticket `id` (numbered from **1**: ticket 0 is "unattributed" on the home chain and is never issued)
`{coin, amount, l2Recipient, mode = Native, at}`; `ArbSys.sendTxToL1{value: amount −
reward}(homeVault, abi.encodeCall(HomeVault.fund, (id)))`; pays `reward` to the caller. The L2→L1 message is the
ticket; the vault's `fund(id)` is its landing. Nothing else can spend the escrow's ETH: there is no other outbound path.

`releaseFast(coin, bridge, data)` (**keeper only**): the same ticket with `mode = Fast`, sent to an **allow-listed** bridge
depository (set at construction; a new one takes `CHANGE_DELAY` = 7 days on both clocks via the owner) by
`bridge.call{value}(data)`. The keeper chooses the quote; the contract checks only the destination and the amount. Bounded: a
fast ticket must be **confirmed** (§5.2) within `FAST_DEADLINE` (3 days), and while any fast ticket is overdue no further fast
release is possible (`flagOverdue`, anyone). Native releases are never blocked: they need nobody.

### 5.2 Receipt: the vault tells the escrow, through the native bridge

`HomeVault.acknowledge(ticket)` (anyone, payable for the retryable fee, no reward — an ack is not money): requires
`fundedNative[ticket] + fundedFast[ticket] > 0` and not yet acknowledged; sends
`Inbox.createRetryableTicket(escrow, 0, …, data = abi.encodeCall(FeeEscrow.confirm, (ticket, amountSeen)))`.
On 4663, `FeeEscrow.confirm` accepts **only** `msg.sender == alias(homeVault)` (`homeVault + 0x1111…1111`, Nitro's L1→L2
address aliasing) — an L1 contract's retryable executes on L2 from its aliased address and nothing else can produce that sender.
The ticket becomes `Confirmed { amountSeen }`. The proof of receipt therefore never trusts a keeper, a solver or an oracle: the
money is on the home chain iff the home-chain contract said so through the rollup's own inbox.

### 5.3 Proceeds down: buyback and burn on 4663

`HomeReceiver` (4663, one per FeeEscrow): `creditProceeds(ticket)` payable, **only from `alias(homeVault)`** (the settle
retryable of §3.5); `proceedsOf[coin] += msg.value` with `coin = escrow.ticket(ticket).coin`. `buyback(coin)` (**anyone**,
reward): spends `min(proceedsOf[coin], MAX_TRANCHE)` through `EtchRouter.buy(coin, minOut, DEAD, deadline)` — bought and burned in
one call, the coin's holders never touched — at most once per `COOLDOWN` per coin. `minOut` is derived from the pool's
current price (`StateLibrary.getSlot0` on the coin's pool key from the lock) less `MAX_SLIPPAGE_BPS` (500): a sandwicher must
move the price more than 5% to make the tranche revert, and the tranche bounds what one block can extract. That is a bound, not
an oracle; open problem 5.

### 5.4 wX down: the report and the release

Under `Wrap`, `buy*` ends with `wrapper.wrap(tokenId, this)`, then bridges the `1e18` wX to `receiver` through the gateway
(§4) and sends a second retryable `HomeReceiver.creditWrapped(ticket, units)` (only from `alias(homeVault)`). The receiver
holds wX and `wrappedOwed[ticket]`; `claimWrapped(ticket)` (anyone) transfers `min(owed, balance)` to
`escrow.ticket(ticket).l2Recipient`. The token deposit and the report are two L1 transactions; whichever lands second completes
the claim. Every wei of fee and every unit of wX leaves the receiver only to an address the escrow's ticket named on 4663.

## 6. Caps and outbound limits, in Solidity: `HomeGuard`

One `HomeGuard` per home chain, shared by every `HomeVault` and `HomeWrapper` there, `owner` = the timelock. It is
`docs/v5/custody-hardening.md` H2 and H3 with the canister replaced by a contract:

- **Custody caps** (H2): per collection `{maxItems, maxValueWei, maxItemWei}` and global `{maxItems, maxValueWei}`. A
  collection is registered by the owner **at stage 0** (`3` items, `10 ETH`, per-item `= maxValueWei`, as
  `docs/v5/launch-gates.md`), and only registered contracts (the vault and the wrapper of that collection) may call
  `admit(collection, valueWei)` (a buy, a wrap) and `release(collection, valueWei)` (an item left). `raise(collection, …)` /
  `raiseGlobal(…)` are owner-only, **at most ×4 per call** on each number, and at most once per `RAISE_INTERVAL` (3 days, both
  clocks) per collection, so a chain of proposals cannot compound faster than the timelock shows them. Lowering is instant.
- **Outbound limits** (H3): a 24-hour sliding window of items out per collection (`perCollectionItems`, stage 0: 6) and global
  (`globalItems`), and of wei out (`globalValueWei`: rewards paid, proceeds and fees bridged). Every outbound act calls
  `out(collection, items, wei)`; over the limit it reverts `OutflowWait(which, until)`. The **tripwire**: the canister's counted
  "distinct asks over the limit", which a chain cannot record (a reverted call leaves nothing); here the guard pauses when an act
  **consumes a whole day's limit within `TRIP_SPAN` (6 h) of the window's first act** — a rush, whether a bank run, a bug or a
  forged stream — and only the owner unpauses. Six items spread over a day are a limit, not an alarm. A vault's `buy*`, `relist`
  and `settle` are refused while paused (money in still works, as on 4663).
- **Delivery only to the ticket's destination** (H4): there is no delivery step to check — the transfer is in the same call as
  the proof: `claimWin` moves the NFT to the address whose ticket range was proven, `resolve` to the ticket's `destination`,
  `settle` to `receiver`, wX to `receiver`, the receiver to the escrow's `l2Recipient`. The tests assert each by ABI and by
  fuzz: no function takes a destination the caller chooses except `redeem`, whose destination is the redeemer's own choice
  fixed at the burn.

The bank-run constraint of H3 applies unchanged: `maxItems ≤ perCollectionItems × (timelockDelay − 1 day)`; the guard refuses a
raise that breaks it against its immutable `timelockDelay`.

## 7. Tests (`test/homechain/`)

- **Unit**: every refusal and event of `HomeVault` (both buys, all four policies, relist/settle, caps, rewards), `HomeWrapper`,
  `HomeGuard`, `FeeEscrow`, `HomeReceiver`, against a `MockPunks` (the real market's semantics: `punksOfferedForSale`,
  `buyPunk` crediting `pendingWithdrawals`, `offerPunkForSale`, `transferPunk`, `withdraw`), the existing `MockMarket` and
  `MockNft` from `test/nft/NftMocks.sol`, a `MockInbox`/`MockOutbox`/`MockArbSys` that record messages, a `MockGatewayRouter`.
- **Fuzz**: arbitrary fund/buy/relist/settle sequences keep `budget + proceedsHeld ≤ balance`; a caller's net gain over any
  sequence of calls is at most `reward × successful triggers`; markup within bounds; caps and windows bind at their edges.
- **Invariants**: backing (`HomeWrapper`: ledger and every held item owned by the wrapper on chain); caps never exceeded;
  outflow never exceeds a window's limit; rewards never exceed the budget (`budget` never underflows, balance ≥ budget +
  proceedsHeld); NFTs leave a Raffle vault only to a proven winner; nothing leaves a paused guard.
- **Seaport guard attack cases ported** from `test/nft/NftSeaportGuard.t.sol`: self-listed order, fee to the caller, tip, empty
  offer, another collection, averaging over the cap, other selectors, a truncated/overwritten calldata fuzz.
- **Exact ABI**: `HomeVault`, `HomeWrapper`, `HomeGuard`, `FeeEscrow`, `HomeReceiver`, asserted like `test/nft/NftSurface.t.sol`;
  no `withdraw/rescue/set*/execute/isValidSignature`.
- **Fork** (`HomeVault.fork.t.sol`, runs only when `block.chainid == 1`): a local anvil fork of Ethereum mainnet at block
  **26,040,025** on port **8641**:
  - the real CryptoPunksMarket: **punk #2346, offered to anyone for 34 ETH** by `0x9092…b5Dc` (read at that block); the vault,
    funded on the fork, buys it by anyone's `buyPunk`, relists at 1.2× (40.8 ETH), and a fork buyer takes the ask; `settle`
    withdraws and bridges the proceeds through the **real Inbox** (the retryable is a real L1 event on the fork);
  - the real Seaport 1.6 and the real BAYC: the fill that happened for real one block later (tx `0x958bbdef…`, block
    26,040,026: BAYC **#1304** for 8.2 ETH by `fulfillBasicOrder_efficient_6GL6yc`, open zone, 8.118 to the offerer
    `0x224a…5192` and 0.082 to OpenSea) is replayed **through the vault** with the recorded calldata
    (`test/fixtures/bayc-1304-seaport-fill.json`); the NFT arrives in the vault; the same calldata with a tip appended is refused;
  - `relist` validates a real order on real Seaport and a fork buyer fills it with `fulfillOrder`, then `settle` runs.

## 8. Open problems

1. **The keeper's cap** is still the price oracle for buys, an hour at a time. ~~A "cheapest ask" hint checked on chain is a
   cheap improvement not built here~~ — built on `contract-fixes` (§3.2, "The cheapest-ask hint"): a bounded list of hinted
   punk ids anyone may fill, each re-read from the order book at the buy; a dearer buy is refused while a cheaper hinted punk
   is offered. The cap still bounds the price; the hint bounds the choice among what is named.
2. **Restricted (SignedZone) listings** cannot be bought: the vault sends no zone `extraData`. OpenSea's Ethereum listings are
   99% open (`docs/v4/eth-nft-canister.md` §1), Base's mostly restricted. A zone-data path would reintroduce a keeper hand-in.
3. **Base-home collections** have no native route to 4663 shorter than a week down and two up (§2.3). The design is chain-agnostic
   (the vault takes any `inbox`/`outbox`), but on Base `inbox`/`outbox` would be a fast bridge's contracts, which is a different
   trust; not built.
4. **wX on 4663 is a plain `StandardArbERC20`**; `IWrappedCollection`'s custody views are only on the home chain. `LiquidityLock`
   and the curve type their wrapper as `IWrappedCollection`; a 4663 adapter (views answered from a keeper-posted, challengeable
   mirror, or a storage-proof) is needed before a home-chain collection can be pooled there.
5. **Buyback slippage** is bounded by spot − 5% and a tranche, not by an oracle.
6. **The `NftRegistry` / launcher wiring** (a listing whose vault is a `FeeEscrow` on 4663 and a `HomeVault` elsewhere) is not
   built: `FeeEscrow` implements `INftVault` so a router can pay it, and the registry side is one additive listing kind.
7. **Retryable redemption** after a failed auto-redeem (L2 gas spike) needs anyone to call `ArbRetryableTx.redeem` within 7 days;
   the keeper should watch `RetryableTicketCreated`. Nothing is lost if nobody does — the value refunds to the receiver — but a
   ticket's report can arrive late.
8. **Permissioned validators** (`validatorWhitelistDisabled = false`): a withdrawal's ~7 days assume Robinhood's validators keep
   asserting. If they stop, L2→L1 messages wait; ETH in the escrow is not lost, only late. Force inclusion covers censorship
   of L1→L2 messages (4 days); nothing covers a stopped validator set except the rollup owner.

## 9. Built: what the code settled (2026-09-23, commit `bb13304`)

Files: `src/homechain/{IHomeChain, HomeGuard, HomeWrapper, HomeVaultBase, HomeVaultRaffle, HomeVaultBurn, HomeVaultYoyo,
HomeVaultWrap, FeeEscrow, HomeReceiver, LockSpotQuote}.sol`, `test/homechain/*` (12 files), `script/homechain/DeployHome.s.sol`
(a fork-only dry run, never broadcast), `test/fixtures/bayc-1304-seaport-fill.json`. **No existing file was edited**: `SeaportGuard`
and `Seaport16` are imported from `src/nft/`, `Wait` from `src/lib/`, `INftLauncher`/`INftVault` from `src/nft/INft.sol`.

Where the code differs from the sections above, and why:

1. **One vault contract per policy** (§3). The single `HomeVault` compiled to 32,664 bytes; Ethereum's EIP-170 limit is 24,576.
   Split: `HomeVaultBase` 19,162 (as `HomeVaultBurn`), `HomeVaultRaffle` 22,519, `HomeVaultWrap` 20,005, `HomeVaultYoyo`
   **23,247** (1,329 bytes of margin — the next feature on the Yoyo path must be paid for in bytes). A policy is a type, not a
   branch, which is also the stronger claim: a Raffle vault has no `relist` selector at all (`HomeSurface.t.sol`).
2. **`receive()` reads no storage before the punks check.** CryptoPunksMarket's `withdraw()` pays `msg.sender.transfer(amount)`
   with the 2,300-gas stipend; the first draft's `receive` read `_buying` (a cold SLOAD, 2,100 gas) first and the real `withdraw`
   failed on the fork. Now the punks branch runs on immutables only and writes nothing; `settle` measures the balance around
   the `withdraw`. The fork test exercises the real 2,300-gas path (`test_fork_punk_buyRelistSellSettle`).
3. **The Seaport check is an external self-call** (`checkOrders`): with the check and the recording both inlined into `buySeaport`,
   via-IR failed with "variable … too deep in the stack" inside Seaport's decoder. Same guard, its own frame, a public view for
   free.
4. **The budget is the FIFO ledger, not a counter** (§3.1, §3.2): price + reward + a policy's extra are charged to the tickets in
   one purchase; a settle's reward comes from the proceeds. `accounted() = attributed + waiting + proceedsHeld` and the balance is
   never below it (invariant).
5. **The tripwire fires on consumption speed** (§6), because a chain cannot count refused asks.
6. **Wrap counting**: the vault admits the item with its price; on wrap it releases the value only (the item is now the wrapper's);
   the wrapper skips `admit` for its own vault's wraps (`HomeWrapper.vault`, computed ahead of the vault's deployment from the
   deployer's nonce) and always releases on delivery. Guard item counts move vault → wrapper without a double count (tests).
7. **The real filler's conduit key is dropped.** The recorded BAYC fill carried a non-zero `fulfillerConduitKey` (the original
   buyer's OpenSea conduit). It is the fulfiller's own field, outside the offerer's signature; the vault fills with no conduit and
   the guard refuses one (`BadFill`), so the trigger hands in the recorded parameters with that field zeroed. Every signed field
   is byte-identical and the real Seaport accepted the re-encoding.
8. `FeeEscrow.releaseFast` pays no reward (it is the keeper's job); tickets number from 1; `HomeReceiver.absorb` lets anyone
   attribute a dead retryable's refund (credited by ArbOS without a call) to a ticket's coin.

**Verification.**

- `forge build --offline`: clean (lint notes only). Sizes above; every new contract is under EIP-170.
- `forge test --offline --no-match-path "test/fork/*"`: **723 passed, 0 failed** (the whole repo). Of these, **112 are in
  `test/homechain/`**: `HomeVault.t.sol` 35 (construction, funding by ticket, the cap on both clocks, keeper rotation, both buys
  and their refusals, a swallowing market, the paused guard, per-item cap, a refusing caller, punks buy + atomic relist, yoyo
  settle for punks and ERC-721s, expired relist, burn, raffle end to end with cancel/void/redraw, wrap with the gateway and the
  report, acknowledge, attribution; 2 fuzzers: the caller's gain is exactly `reward × triggers` and the balance covers the
  ledger; the markup within bounds); `HomeSeaportGuard.t.sol` 17 (the attack cases of `NftSeaportGuard.t.sol` ported: self-listed,
  fee to the caller, `tx.origin` behind a bot, tip to an accomplice, empty offer, averaging over the cap, another collection, a
  non-candidate, disallowed selectors, shape and call-level rules, plus two decoder fuzzers); `HomeWrapper.t.sol` 15;
  `HomeGuard.t.sol` 13 (caps at their edges, ×4 and the interval, the bank-run check, the window on both clocks, per-collection
  vs. global, the tripwire on a rush and not on a slow day, a fuzzed window); `FeeEscrow.t.sol` 10 (with `HomeReceiver`);
  `HomeSurface.t.sol` 10 (exact ABI of all eight contracts; no `withdraw/rescue/set*/mint/isValidSignature`; no vault takes a
  destination from its caller); `LockSpotQuote.t.sol` 4; two invariant suites, 64 runs × 64 calls: the vault's (balance ≥ ledger,
  rewards ≤ triggers, caps and windows never exceeded, raffle NFTs leave only to the proven winner, yoyo asks are exactly the
  markup, no panics) and the wrapper's (ledger, on-chain backing `balanceOf(wrapper) == held`, every item in one place, strict
  order, the guard counts the wrapper, no panics).
- **Fork** (`test/homechain/HomeVault.fork.t.sol`, anvil forked from Ethereum at 26,040,025 on :8641, nothing sent anywhere):
  **6 passed**. Real punk #2346 bought by a stranger's `buyPunk` at the real 34 ETH ask, relisted at 40.8 ETH on the real order
  book, taken by a fork buyer, settled: the real market's `withdraw` paid the vault under the stipend and the real Inbox emitted
  `InboxMessageDelivered` for the proceeds retryable. Real BAYC #1304: the recorded fill replayed through the vault (8.118 ETH to the
  offerer, 0.082 to OpenSea), arrived; the same order with a tip refused by the vault and **paid when sent to Seaport directly**
  (the attack is real); `relist` validated on the real Seaport (`getOrderStatus` = validated) and filled by a fork buyer with
  `fulfillOrder` and an empty signature, then settled through the real Inbox; the Wrap vault wrapped the ape and the fresh wAPE
  went through the real `L1GatewayRouter` (standard gateway, `DepositInitiated`, two Inbox messages); a raffle delivered it to
  the proven winner.
- Not run: any deploy, any public-chain transaction, `DeployHome.s.sol` (a dry-run script with the verified addresses; it needs
  the 4663 side first).

## 10. The 4663 side of a home-chain collection: listing, adapter, fee routing (branch `contract-fixes`, 2026-09-23)

Written before its code, closing open problems 4 and 6 for what can be closed on 4663 and stating what stays on the home
chain. Everything new is in `src/homechain/` (`HomeListing`, `HomeWrappedAdapter`, `HomeSweepVault`, `HomeRedeemDesk`) and
`test/homechain/`; no existing contract is edited except three interfaces added to `IHomeChain.sol`.

### 10.1 What a coin on 4663 needs from a home-chain collection, and what it gets

`CurveFactory` launches coins for **one wrapped token** (`IWrappedCollection`) with a queue (`IItemQueue`), a seeder (the
`LiquidityLock`) and three `SweepVault`s. The lock pools that token against ETH, the curve pays the coin's sweep share to its
policy's vault, the vault holds and draws whole units (RAFFLE), retires them (BURN, `redeemToBurn`) or relists them (YOYO,
`redeemToRelist` + the custodian's `recordSale`). On 4663 the home chain's wX is a `StandardArbERC20` twin (§4) that satisfies
none of the custody views and cannot be burned for an item here. Three things therefore stand between the twin and the factory:

| need | answered by | where fulfilled |
|---|---|---|
| a token with `ONE_ITEM`, `custodian`, `held`, `openTickets`, `redeemToBurn`, `redeemToRelist`, `refunded`, `beacon` | **`HomeWrappedAdapter`** (§10.3): an ERC-20 minted 1:1 against twin wX deposited into it | 4663, whole |
| a burn/relist that a burn on 4663 cannot do (the NFT is on the home chain) | the adapter escrows the unit, sends a **redeem intent** up through the rollup's bridge; **`HomeRedeemDesk`** on the home chain acknowledges it down, the adapter dispatches the wX up, the desk burns it at `HomeWrapper` | intent, escrow, ack, dispatch, refund on 4663; the burn, the drand draw and the delivery on the home chain |
| the sweep share reaching the HomeVault's release ledger | **`HomeSweepVault`** (§10.4), a `SweepVault` whose credit anyone may `forward` to the listing's `FeeEscrow` | 4663; the escrow's `release` and the vault's `fund` as in §5 |
| a record the site, the indexer and the owner can read, and the roles/pause the vaults read | **`HomeListing`** (§10.2), one listing per (home chain, collection, policy) | 4663 |

What a coin on a home-chain collection is, then: launched by a `CurveFactory` whose `wrapped` is the adapter, **`SWEEP_ONLY`**
(§10.5: nothing on 4663 can convert an item budget into items — the `ItemQueue` the factory needs is deployed but nobody can
fill it, and a non-sweep-only coin would only reach `openRefunds` at its window's end, the curve's existing path), RAFFLE or BURN
(YOYO's `recordSale` needs a custodian on this chain, which the adapter is not; a YOYO vault is deployable and listable, but its
listings can never be settled here — a coin's creator should not choose it, §10.6). Its sweep share goes to the home vault, which
buys NFTs on the home chain and raffles or burns them there (§3.3); its 4663 vault can also hold adapter units (a keeper sweep at
the router over the wX/ETH pool, a coin quoted in wX) and draw or retire them.

### 10.2 `HomeListing`: the registry

One per 4663 deployment, alongside `NftRegistry`; like it, **no roles of its own**: `owner()`, `keeper()`, `treasury()` are
`ListingRegistry`'s, so every rotation still waits its seven public days and moves every system at once; `isPaused(id)` is this
registry's flag (the owner's `setPaused`, instant, stops sweeps and forwards: money still comes in). A `HomeSweepVault` is built
with this registry as its `registry` and its listing id, and reads exactly those four things from it.

`list(...)` (owner): `{homeChainId, collection, policy, name, homeVault, homeWrapper, wx, adapter, escrow, receiver, sweepVault}`,
once per `(homeChainId, collection, policy)`, `policy` one of Raffle/Burn/Yoyo (`Wrap` has no 4663 vault). It verifies the bundle
against the chain before recording it: the vault's `registry()` is this contract and its `listingId()` is the id being made, its
`policy()` matches, its `wrapper()` is `adapter`, its `escrow()` is `escrow`; the escrow's `listingId()` is the id, its
`homeVault()` is `homeVault`, its `launcher()` is the vault (§10.4); the adapter's `wx()` is `wx` and `homeWrapper()` is
`homeWrapper`; the receiver's `escrow()` is `escrow` (a zero receiver is allowed: BURN needs none). `factory` is read from the
vault (`launcher()`), not given. The home chain's addresses (`homeVault`, `homeWrapper`) cannot be checked from here and are
recorded as given: they are what the escrow and the adapter were built with, which *are* checked. Views: `count`, `listing(id)`,
`vaultOf(id)` (the sweep vault), `ref(id)` = `eip155:<chain>:<collection>`.

**Order of deployment**, because the vault and the escrow name each other: listing id `n` is `count()` at the time; the adapter,
the queue, the lock, the factory, the router; then per policy P: the **escrow** with `launcher` = the vault's address computed
ahead (the deployer's next nonce, as `HomeWrapper.vault` is computed in §9.6) and `listingId` = `n + P`; then the **vault** with
that escrow — its constructor refuses an escrow that does not name it; then the factory's `wire`/`wireYoyo` and the queue's
`wire`; then the owner's `list` for each policy in order. Until `list` is made the vault's `isPaused` reverts (`NoSuchListing`),
so nothing sweeps or forwards; deposits are accepted (they check only the router).

### 10.3 `HomeWrappedAdapter`: the twin's face on 4663

`HomeWrappedAdapter is ERC20, IWrappedCollection`. Immutable: `wx` (the twin), `homeWrapper` (the L1 token, = `HomeWrapper`),
`homeDesk` (the home chain's `HomeRedeemDesk`: the intent's target and the wX's destination when dispatched), `homeDeskAlias`
(its aliased address: the only sender of `acknowledge`), `l2Router` (`L2GatewayRouter`, §2.2), `arbSys` (0x64), `beacon`
(a 4663 `DrandEvmnet`, for the sweep vaults' draws), `parentBlockSeconds`. `custodian()` is the adapter itself, as
`HomeWrapper.custodian()` is the wrapper: custody is a contract, and nothing on 4663 holds a custodian's role — every
custodian-only path of the vaults, the queue and the desk (`recordSale`, `recordItem`, `fill`) is unreachable for a home-chain
collection, on purpose.

- **In**: `deposit(amount, to)`, anyone: pulls `amount` of twin (measured by balance, so a fee-on-transfer twin cannot inflate)
  and mints `amount` of adapter units to `to`. **Out**: `withdraw(amount, to)`: burns the caller's units, sends `amount` of twin.
  Instant, 1:1, permissionless both ways; a holder who bridged wX down deposits it to trade or launch; a raffle winner withdraws
  to bridge up and redeem at `HomeWrapper` (§4, "Redemption from 4663").
- **Ledger**: `escrowed` = twin held for units and tickets; the invariant is `totalSupply() + openTickets × ONE_ITEM == escrowed
  == wx.balanceOf(adapter) − stray()`. `held()` = `escrowed / ONE_ITEM`: whole items whose claim is in this contract — exact for
  this contract's books, a lower bound of the home custody (the twin's total supply also lives in wallets and pools). Twin sent
  here by plain transfer is `stray()`: never counted, never spendable.
- **`redeemToBurn()` / `redeemToRelist()`**: burn `ONE_ITEM` of the caller's units (the caller is a sweep vault, which never
  approves anything — this is why the adapter must be the token the vault holds, not a wrapper around an approval); open ticket
  `id` `{kind, owner, Open, openedAt/Block}`; `openTickets++`; `ArbSys.sendTxToL1(homeDesk, abi.encodeCall(HomeRedeemDesk.intent,
  (id, kind, owner)))` — the intent, ≈ 7 days up (§2.3), executed on the home chain by anyone's `Outbox.executeTransaction`.
  The unit's twin stays escrowed here.
- **`acknowledge(id)`**, only from `homeDeskAlias` (a retryable from the desk, §5.2's proof shape): `Open → Acknowledged`. From
  then on the ticket cannot be refunded: the home chain has committed to it.
- **`dispatch(id)`**, anyone, `Acknowledged → Dispatched`: `openTickets--`, `escrowed -= ONE_ITEM`,
  `l2Router.outboundTransfer(homeWrapper, homeDesk, ONE_ITEM, "")` — the standard gateway burns the twin from this contract
  (no approval: `bridgeBurn` is the gateway's) and the L1 gateway releases `ONE_ITEM` of wX to the desk ≈ 7 days later. This is
  **fulfilment** on 4663: the wX left, to the desk and nowhere else.
- **`refund(id)`**, anyone, `Open` and `ACK_DEADLINE` (21 days: three times the bridge, so a slow assertion or a late executor is
  not a refund) elapsed on both clocks since the burn: `Refunded`, `openTickets--`, `ONE_ITEM` minted back to the ticket's owner
  — the twin never moved. `refunded(id)` is what `SweepVault.reclaim` reads. A refunded ticket's intent may still be executed on
  the home chain later; the desk then holds a `Received` intent nobody acknowledges (it acknowledges only what it chooses to; an
  ack for a refunded ticket is refused here, `WrongStage`, and the retryable is simply lost with its fee).

**The invariants**, as tests hold them: twin leaves the adapter only by `withdraw` against burned units or by `dispatch` of an
acknowledged ticket — never by a refund (which mints the unit back) and never otherwise; every open ticket is backed by exactly
`ONE_ITEM` of twin here (the ledger equality above); one ticket per burn, one dispatch per ticket, one intent per ticket; caps
are the home chain's (the desk's burn resolves through `HomeWrapper`, whose delivery goes through the guard's window, §6) — the
adapter adds no caps of its own because nothing of value leaves it except to the desk.

Why `redeemToBurn` and `redeemToRelist` can cross a bridge honestly and `redeem(destination)` cannot: neither has an owner-facing
outcome — the NFT goes to `DEAD` or to the home vault's YOYO listing — so which home-chain unit completes which 4663 ticket
changes nobody's payout. A redemption to a person needs the ticket's destination fixed at the burn and delivered to that person;
that stays the home chain's `HomeWrapper.redeem`, reached by bridging the twin up oneself (§4). This is also why the desk needs no
per-ticket attribution of the bridged tokens, which the standard gateway could not give it anyway.

### 10.4 `HomeSweepVault` and the escrow: the sweep share's road home

`HomeSweepVault is SweepVault`: the whole vault, unchanged, plus an immutable `escrow` (the listing's `FeeEscrow`, which must name
this vault as its `launcher`) and `forward(coin)` (**anyone**, not paused): moves all of `credit[coin]` to
`escrow.deposit{value}(coin)`. The escrow's `_deposit` accepts `launcher.routerOf(coin) == msg.sender` and
`launcher.listingOf(coin) == listingId`; the vault answers both **for itself**: `routerOf(coin)` is the vault when the coin is
this factory's and of this vault's policy (`CurveFactory.policyOf`), `listingOf(coin)` its listing id, `creatorOf` the factory's.
So the escrow trusts exactly one contract, fixed at its construction, for exactly the coins that contract carries — the same shape
as `NftFeeRouter` → `FeeEscrow`, one hop longer. From there §5 is unchanged: anyone `release`s at `minRelease` for the reward, the
home vault is `fund`ed by ticket, buys, raffles over the coin's holders or burns.

The keeper's `sweep` at the router and anyone's `forward` draw from the same `credit`; whichever comes first takes it. Both spend
the coin's fee on the coin's collection — items bought on the home chain, or wX bought on 4663 — so the race is between two
honest uses, and the vault's books hold either way. A coin's `depositWrapped` (a coin quoted in wX) and a sweep's units are
adapter units in the vault: drawn (RAFFLE) or retired through the adapter (BURN).

### 10.5 `HomeRedeemDesk`: the home chain's side of an intent (built; deploy with the vault)

On the home chain, next to `HomeWrapper`. Immutable `outbox`, `inbox`, `adapter` (the 4663 address), `wrapper`. `intent(id, kind,
owner)`: only `msg.sender == outbox` with `outbox.l2ToL1Sender() == adapter` — the rollup's own proof that the adapter burned a
unit; `Received`. `acknowledge(id)` (anyone, payable for the retryable, no reward): `Received → Acknowledged`; a retryable to the
adapter's `acknowledge(id)`, refund addresses the adapter. `complete(id)` (anyone): `Acknowledged`, `wrapper.balanceOf(desk) ≥
ONE_ITEM` (the dispatched wX has landed: the gateway's release and this call are two L1 transactions; before it, `NotArrived`),
then `wrapper.redeemToBurn()` or `redeemToRelist()` — a `HomeWrapper` ticket, drawn and delivered by §4, through the guard. One
completion per intent; the desk holds wX only between a landing and a completion. Not built: a bound on how long an intent may
wait for its ack (an operator that never acknowledges leaves the 4663 ticket to `refund`, which is the point), and any path
for the desk's wX other than `complete`.

### 10.6 What stays spec

- **A 4663 deployment script** for a home-chain listing (`script/homechain/DeployHome.s.sol` covers the home chain only). The
  order is §10.2; the addresses it needs from the home chain (`HomeVault` per policy, `HomeWrapper`, the desk) come first, and
  the twin's address is `L1GatewayRouter.calculateL2TokenAddress(HomeWrapper)` before any deposit (§2.2).
- **YOYO on 4663** for a home-chain coin: `SweepVault.recordSale` is the custodian's, and the adapter is nobody's custodian. The
  YOYO policy of a home-chain coin is the home vault's (§3.4–3.5); a 4663 YOYO vault may be listed for completeness but its
  `relist` produces tickets whose sale is never recorded here. A factory for a home-chain collection can simply not `wireYoyo`.
- **The graduation queue**: `ItemQueue` over the adapter is deployable and required by the factory, but unfillable; home-chain
  coins launch `SWEEP_ONLY`. A queue that turns an item budget into a `FeeEscrow.depositFor(coin, curve)` and counts the wX the
  receiver `claimWrapped`s back as its fill is the honest next step (the receiver delivers twin, which would need `deposit`ing).
- **`HomeReceiver.claimWrapped`** delivers twin wX to the escrow ticket's `l2Recipient`; a recipient that needs adapter units
  (a curve, a queue) must deposit it. For a `Wrap`-policy home vault paired with a 4663 coin nothing here is wired; §5.4 stands.
- **The desk's ack budget**: `acknowledge` is payable by anyone, like `HomeVault.acknowledge`; a keeper is expected to make it.
- **The home-chain guard** does not see the desk: its burns count out through `HomeWrapper` as any redemption. No cap on 4663.

### 10.7 Built (2026-09-23, branch `contract-fixes`)

`src/homechain/{HomeWrappedAdapter, HomeRedeemDesk, HomeSweepVault, HomeListing}.sol` (7,133 / 2,581 / 18,875 / 6,528 bytes of
runtime), three interfaces appended to `IHomeChain.sol`; `test/homechain/{HomeAdapter, HomeRedeemDesk, HomeListing}.t.sol` and
four exact-ABI tests in `HomeSurface.t.sol` (the desk and the listing are also in the no-way-out-by-name check; the adapter's
`withdraw` is the unwrap of the caller's own units and is stated as meant). No existing contract was edited. Where the code
settled differently from §10.1–10.6: nothing; one detail worth stating — `HomeSweepVault`'s constructor reads the escrow back
(`launcher() == this`, `listingId()`), so the §10.2 order is enforced by the vault, not only by the listing.

`forge test --offline --no-match-path "test/fork/*"`: **921 passed, 0 failed** (the whole repo; 893 before this branch). Of the
new 23 here: `HomeAdapter.t.sol` 9 (construction and the interface; 1:1 both ways; stray twin never counted; burn and relist open
tickets, escrow the twin and send the intent with the right calldata; refund only open, only after 21 days on both clocks, unit
back, twin unmoved, a late ack refused; ack only from the desk's alias, then no refund; dispatch anyone after the ack, twin
burned through the gateway to the desk only, once; a gateway that does not burn leaves the ticket acknowledged; a 40-step fuzz of
deposit/withdraw/burn/ack/dispatch/refund holding the ledger and "twin leaves only by withdrawal or dispatch, every dispatch to
the desk"); `HomeRedeemDesk.t.sol` 4 (intent only from the Outbox with the adapter as L2 sender, once, kinds 0/1; the ack is a
retryable to the adapter with both refunds to the adapter; completion waits for the landing, burns at the wrapper for the kind —
`DEAD` for a burn, the home vault for a relist — once per intent); `HomeListing.t.sol` 6 (the verified bundle and the borrowed
roles; every refusal of `list` and the vault's own `EscrowNotMine`/`NoEscrow`; pause stops forwards only; the sweep share of a
SWEEP_ONLY coin: launch → fill → `flushVaultFees` → `forward` by anyone → `release` by anyone for the reward → ArbSys message
`fund(1)` to the home vault for the amount less the reward, with the escrow refusing a stranger and the wrong policy's vault; the
BURN road: units swept at the venue, `retire` → adapter ticket + intent, `reclaim` refused until the refund, refund after 21 days
and the coin's unit back, a second retire acknowledged and dispatched to the desk, `reclaim` refused for it; a non-sweep-only
coin fills nothing and reaches `openRefunds`).
