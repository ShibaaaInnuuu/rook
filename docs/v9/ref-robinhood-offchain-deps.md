# Reference: everything outside the contracts that depends on the home chain (4663), at `robinhood-stop-2026-10-09`

Read-only map made 2026-10-09 for the Solana migration (`solana-migration.md`): the canister, the services, the web apps, tests and CI. Line refs are to the tag.


I only read files and changed nothing. Paths below are relative to /Users/jlohayon/Documents/Rook. Line numbers are as of HEAD 5c8d1ea. The line-touch counts are my estimates, not measurements.

## Summary

- **The canister:** about 40% of its 41,259 source lines is code that reads from or sends to 4663. All of it goes through one set of helpers in `canister.rs`: replicated JSON-RPC reads, a "pin" mechanism, a one-node send, and the threshold ECDSA key on derivation path `"evm"`. Three flows sit on top of that layer:
  - the custodian (frog redemptions);
  - the frog/item side (floor modules, Relay conversions to BTC, mint desk, deposits, sales, hand-ins);
  - the Ethereum NFT side.
- **The same key on two chains:** that one `"evm"` key also signs the Ethereum-mainnet Seaport fills. On Solana you would add an Ed25519 key for the home chain (IC `sign_with_schnorr` with Ed25519) and keep the secp256k1 key for Ethereum.
- **What survives unchanged:** the Bitcoin and Seaport security code (PSBT, sighash, runestone, taptree, custody, owbuy/frogbuy judges, seaport, ordapi) does not touch the home chain.

---

## A. Canister `autobuyer/canister-rs/src` (41,259 lines; tests `tests/` 24,615 lines)

### A.0 The shared home-chain layer, `canister.rs` (3,195 lines)

**Install config (`Init`, canister.rs:34-89)**
- Fields: `chain_id` (4663), `vault`, `rpc_urls` (at least two, must be distinct, :367-372), `wrapper`, `wrapper_from_block` (a 4663 block), `evm_chains` (Ethereum), `eth_nft`, `frog_funding`.
- `validate` (:349) calls `chains::validate(evm_chains, home)`, which refuses any NFT chain equal to the home chain.
- `PRODUCTION_EVM_CHAIN = 4663` (:202) and `REHEARSAL_EVM_CHAIN = 46630` (:198, Robinhood testnet) are hard-coded.
- `open_purchase` refuses on 4663 (:1641).

**Signing key**
- `evm_address()` (:753): `ecdsa_public_key` on derivation path `b"evm"`, keccak of the public key, low 20 bytes.
- `sign_evm` / `sign_evm_recoverable` (:2119-2227) recover the recovery id with k256.
- Bitcoin uses Schnorr on paths `b"btc"` (vault buyer, :628), `b"custody"` and `b"float"` (`frognet.rs:25`).
- The key name comes from `cfg().key_name`; `"key_1"` needs `accept_production_key_name`.

**RPC providers**
- `rpc`, `rpc_view` and `rpc_agreed` (:809-885): every provider is asked through replicated outcalls (`is_replicated: true`), and the bodies must be identical.
- Transforms: `strip_volatile`, `json_rpc_norm`, `head_bucket`, `block_at`, `receipt_at_pin`, `finalized_bucket`, `logs_view`, `our_txs_view`.
- `IcRpc` implements `evmread::Rpc` (:902-932) with `eth_blockNumber`, `eth_getBlockByNumber`, `eth_call` and `eth_getLogs`.
- `assert_chain()` (:1204) checks `eth_chainId` on every tick.
- Providers in the launch config: `rpc.mainnet.chain.robinhood.com` and `rpc.ordofi.network`; the fueler/monitor moved to a Dwellir Robinhood archive URL.

**Pins and finality**
- `pin()` (:939) calls `evmread::fast_pin`. Each head is rounded down to `pin::BUCKET = 1024` blocks (pin.rs:13). The comment there notes Robinhood's ~0.1 s blocks and that the RPC keeps only about 6,200 blocks of state.
- A monotonic floor applies (`PIN_FLOORS.home.admit_pin("4663", …)`).
- Finality is read as a block number only:
  - `MAX_LOG_SPAN_BLOCKS = 8000` (ordofi refuses log spans over 10,000), `MAX_FINALITY_LAG = 50000`, `MAX_FINALIZED_SPREAD = 8192` (evmread.rs:1-24).
  - 4663's `finalized` lags 8,096-12,023 blocks.

**Sending**
- `send_raw_once` (:962) sends from a single node (`is_replicated: false`).
- `broadcast_evm` (:1131) refuses bytes signed for another chain id and then reads back.
- `evm_seen` (:996): receipt at the pin, otherwise `eth_getTransactionByHash`.
- `home_nonce_tx` (:1071) bisects `eth_getTransactionCount` over the roughly 6,200-block state window to find which transaction used a nonce.
- `home_payout_tx` and `home_receipt_logs` (:1022, :1045) read Relay `RelayCallExecuted` logs on 4663.

**`CanisterIo` (:1324-1474)**
- Reads `purchaseCount` / `purchase(id)` of `AutoBuyerVault`, `eth_getBalance`, nonce at the pin, base fee, and `eth_estimateGas` (highest answer across providers).

**Transaction envelope**
- EIP-1559 (`evmtx::Tx1559`): `max_priority_fee 0`, `max_fee = base × FEE_MULTIPLE (4)`, gas = estimate + 30% (looper.rs:19-21).
- "One transaction of ours canister-wide": the `evm_pending()` flags in custodynet, frognet and ethnet gate each other (canister.rs:1586-1621).

**Estimate:** ~900-1,200 lines touched (outcall layer, transforms, Init, key, chain guards).

### A.1 Pure EVM plumbing

| Module | Lines | 4663 role | Port |
|---|---|---|---|
| `evmread.rs` | 326 | Multi-provider pin, finality, `eth_getLogs` paging, sized for the two 4663 providers | Replace with Solana (slot/commitment, `getSignaturesForAddress`/`getTransaction`). Rewrite ~326 |
| `pin.rs` | 423 | Bucket rounding, `json_rpc_norm`, receipt at pin, pin floors | Rewrite ~300. Keep the Ethereum use, since `nftnet::pin_on` reuses it |
| `evmsend.rs` | 291 | `eth_sendRawTransaction` classification, `chain_id_of`, payout/tx views | Keep for Ethereum. Add a Solana `sendTransaction` equivalent (~250 new) |
| `evmtx.rs` | 305 | AutoBuyerVault ABI (`announce`, `pull`, `settleRune`, `returnFunds`, `purchase`) plus `Tx1559` | Keep `Tx1559` (Ethereum). Delete the vault ABI |
| `abi.rs` | 93 | Solidity ABI encoder | Keep for Ethereum. Add Borsh/Anchor instruction encoding (~300 new) |
| `ticketlog.rs` | 432 | Wrapper `Redeemed(uint256,address,uint64,string)` and `Resolved(uint256,uint256,uint64)` logs at finality from a cursor | Rewrite (~432): Solana program events or account state at `finalized` commitment |

### A.2 Legacy v2 vault buyer (runes)

`looper.rs` (483), `state.rs` (521), `policy.rs` (410), `listing.rs` (176), `bridge.rs` (130), `bridgecall.rs` (296), `relayquote.rs` (173), `relay.rs` (113), `venue.rs` (43).

- **Reads:** AutoBuyerVault `purchase`, `purchaseCount`; ListingRegistry `registry`, `listingId`, `attributed`, `capWei`, `capExpiry`, `maxInFlight`, `inFlight`, `openPurchases`, `isBlocked`, `payersFor`, `listing` (policy.rs:129-139).
- **Writes:** `announce`, `pull`, `settleRune`, `returnFunds`.
- **Bridges:** Across `depositV3` from `ACROSS_SPOKE_4663` with `WETH_4663` (bridgecall.rs:24-27); Chainflip on Arbitrum; Relay `depositNative` to `RELAY_DEPOSITORY_4663` (relayquote.rs:21).
- **Port:** delete, or rewrite if the rune vault is kept. ~1,500 lines.

### A.3 Custodian (frog redemptions)

**`redeem.rs` (1,305)**
- Reads the Wrapper's `ticketCount`, `ticket(id)`, `registry`, `listingId`, `vaultOf`, `custodian`, `acquisition` (redeem.rs:19-33).
- Writes `recordDelivery(uint256,bytes32)` and `markUndeliverable(uint256)` (:39).
- The `Redeemed` topic is checked at :1277.
- The Bitcoin delivery build/sign is chain-agnostic.
- ~300 lines touched.

**`delivery.rs` (242)**
- Send/Wait/Refuse against the ticket "as read from 4663"; deadlines use 4663's parent-chain (L1) block divisor (`Wait.sol`, :13).
- Clock semantics change (Arbitrum `block.number` is L1 → Solana slot/unix time). ~80 lines.

**`custodynet.rs` (550)**
- Ticket reads at an agreed block and `block_clock` (`eth_getBlockByHash`: timestamp and L1 block); the rest is Bitcoin. ~120 lines.

**`outbound.rs` (732)**
- `verify_btc` is chain-agnostic.
- The ERC-721 `transferFrom` / `approve` checks (:352-358) are for Ethereum. Keep both. The ticket "read at a finalized 4663 block with its Redeemed log" (H9) is rewired. ~40 lines.

**`conflict.rs` (617):** Bitcoin only. Reuse.

### A.4 Item and frog side: `frogflow.rs` (2,595), `frognet.rs` (856) and the adapters

**`frogflow::send` / `send_value` (:2175-2194)** is the single 4663 sender for this side. It saves raw bytes, then sends.

**`evm_act` (:2206-2335), in priority order:**
1. Reports: `report_call` (:2196) → `curvebuy::record_frog`, `buyqueue::record_item`, `floorsink::item_arrived`, or `convflow::report_call` (`settleAdvance`) for advanced frogs. Desk requests get `mintdesk::fulfill`.
2. Mints: `mint_frog` (:2400) calls `Wrapper.wrapDeposit(string,bytes32[],bytes32,uint32,address)` with a Merkle proof, after `depositMinted(bytes32)`. The target is the sink, module or desk.
3. Floor claims for NFT wants: `itemArrived(cost)` after reading the module's `wrapper()` and the wrapper's `balanceOf(module)` (:2241-2278).
4. `convflow::evm_act`: `advance(uint256)`, the Relay `depositNative` on 4663, `returnAdvance()`.
5. Holder deposits: `wrapDeposit`.
6. Sink triggers: FrogCurve `graduate` / `openRefunds`, BuyQueue `refundExpired`.
7. `handinflow::evm_act` (OfferBoard `claim` / `slash`).
8. `saleflow::evm_act`: `redeemToRelist`, `recordDelivery`, `itemSold{value}`, `recordSale{value}`, `wrapDeposit`, `reclaimListing`.

**`FrogIo` trait (:296-407)**
- 4663 methods: `eth_read`, `clock`, `our_evm`, `evm_nonce`, `base_fee`, `estimate_gas`, `sign_evm`, `send_evm`, `evm_seen`, `our_balance`.
- The Bitcoin, ord and outflow methods are agnostic.

**`frognet.rs`**
- `home_clock` (:157): pinned 4663 block time plus parent (L1) block.
- `set_item_config` (factory, desk, queues on 4663), `relay_quote`, `relay_give_back`, `relay_arrived_exit`.
- ~200 lines.

**Adapters (each is mostly a 4663 ABI; the pure rules in `itemsink.rs` stay):**

| Module | Lines | Selectors |
|---|---|---|
| `curvebuy.rs` | 179 | `state`, `frogs`, `frogsBought`, `frogSpent`, `frogBudget`, `buffer`, `deadline`, `payee`, `wrapper`, `factory`, `recordFrog`, `openRefunds`, `graduate`, `coinCount`, `coins`, `routerOf`, `balanceOf` (:12-30) |
| `buyqueue.rs` | 141 | `queued`, `itemCap`, `buffer`, `itemsRecorded`, `headDeadline`, `recordItem`, `refundExpired` |
| `floorsink.rs` | 239 | `netUnits`, `escrow`, `buffer`, `floor`, `maxItemCost`, `itemsArrived`, `itemArrived`, `wrapper`, `payee`, `price`, `floatUnits`, `minSaleProceeds`, `escrowBalance` (:29-44) |
| `mintdesk.rs` | 183 | `requestCount`, `requestOf`, `isExpired`, `ttl`, `assigned`, `fulfill` |
| `floortrigger.rs` / `floornet.rs` | 271 / 147 | Module and limit config keyed by 20-byte addresses |
| `deposits.rs` | 668 | OP_RETURN `"EWD1" ‖ chainId(4) ‖ wrapper(20) ‖ recipient(20)` (:15-17). Wrapper `wrapDeposit`, `depositMinted`, `inscriptionHeld`, `leafOfFrog`, `custodian`, `registry`, `listingId`, `listing` |
| `handin.rs`, `handinflow.rs`, `handinnet.rs` | 688 / 157 / 98 | OfferBoard `submit`, `claim`, `slash`, `refund`, `openIds`, `openCount`, `submissionOf`, `params`, `slashable`, `expired`. Off in Phase 1 |
| `sale.rs`, `saleflow.rs`, `salenet.rs` | 625 / 610 / 148 | `itemSold`, `redeemToRelist`, `recordSale`, `reclaimListing`, `listing`, `wrapper`. Reports carry ETH value; balance read at the pin |

- The `deposits.rs` port needs a Solana recipient (32 bytes) and a program/mint id. 4+4+32+32 = 72 bytes still fits an 80-byte OP_RETURN.
- `itemsink.rs` (311): pure, chain-agnostic. Its `Clock {time, parent_block}` assumes Arbitrum L1 block numbers.

**Estimate:** frogflow ~500, adapters ~900, deposits ~200, hand-ins and sales ~550.

### A.5 Relay conversions (home ETH → BTC): `convflow.rs` (638), `relayconv.rs` (1,083)

**Flow:**
1. The FloorModule advances one unit's budget to the canister's EVM address (`advance(uint256)`).
2. The runner hands in a Relay quote for "ETH on 4663 → native BTC at the float address".
3. The canister signs `depositNative(address,bytes32 id)` to `RELAY_DEPOSITORY_4663` (0x4cd00e38…bc31; allocator `RELAY_ALLOCATOR_4663` pinned at relayconv.rs:17-24).
4. It watches for arrival (BTC at the float, 3 confirmations) or a refund (our 4663 balance rising by at least 50%).
5. It ends with `settleAdvance(cost)` or `returnAdvance()` (selectors at :439-442).

**Key facts:**
- The safety argument recomputes Relay's EIP-712 `hashStruct(Order)` from `protocol.v2.orderData`. The `id` passed to `depositNative` binds recipient, minimum sats and refunds (:1-11).
- `ORIGIN = "robinhood"`, `DESTINATION = "bitcoin"`, `RELAY_BITCOIN = 8_253_038` (:158-164).
- `GAS_KEEP_WEI = 2e14` is kept per advance for 4663 gas.
- Payout recognition decodes the depository's `execute` call request and its `RelayCallExecuted` log on 4663 (:24-150).

**With SOL as the home currency:**
- The origin becomes Relay's Solana chain. Relay's Solana deposit mechanism is a program or deposit address, not the EVM `depositNative(address,bytes32)` call. So the "recompute the order id that the signed transaction carries" binding must be re-derived for Solana and verified live; I did not verify this.
- The amounts become lamports, and the rate bound becomes SOL/BTC (`rates.rs` and `oracle.rs` are ETH/BTC today).
- Estimate: ~800-1,000 lines rewritten, plus the fixtures `tests/fixtures/relay-quote-4663-*.json`.

### A.6 Ethereum NFT side: `ethflow.rs` (5,816), `ethnet.rs` (752), `ethnft.rs` (1,677), `relayeth.rs` (338), `dormancy.rs` (1,521)

There are two transaction lines (ethflow.rs:6-8): the home line (4663, one transaction canister-wide) and the Ethereum line.

**Home steps (`Step`, :500-521), all sent through `send_home` (:3495):**

| Step | Call (ethflow.rs line) |
|---|---|
| `Advance` | `FloorModule.advance(amount)` (:3848) |
| `Deposit` | Relay `depositNative` on 4663 for the 4663→Ethereum leg (:3912) |
| `Mint` | `EthNftWrapper.mint(false, token, tx, module)` (:3770) |
| `Settle` | `settleAdvance(cost)` (:3777) |
| `GiveBack` | `returnAdvance()` or `fundReserve()` with value (:3797-3799; `FUND_RESERVE` :4435) |
| `Record` | `recordDelivery(ticket, tx)` (:3635) |
| `Mark` | `markUndeliverable` (:3646) |
| `Sent` | `noteSent(ticket, tx)` (:3535) |
| `Post` | `FloorModule.postFloor(price, confidence, depth)`, clamped by `floorBounds()` (:3583-3605) |
| `MintDeposit` | `mint(true, …)` for holder deposits (:3981) |
| `Fill` | `FloorModule.fill(coin)` (:5646) |
| `Served` | `WakeDesk.markServed(id)` (:5736) |

**Home reads**
- Wrapper: `ticketCount`, `ticket`, `item`, `itemOf`, `sentTx`, `undeliverable`, `chainId`, `collection`, `custodian` (ethnft.rs:968-987).
- Module: `accepted`, `keeper`, `advanced`, `floorBounds`.
- Dormancy reads (dormancy.rs:1027-1048): `requestCount`, `requestAt`, `window`, `servedAt`, `totalSupply`, `held`, `rentedHeld`, `gradEth`, `gradUnits`, `floatUnits`, `factory`, `coinCount`, `coins`, `routerOf`, `state`, `isFilled`.
- `home_finalized_time`: tickets are final once resolved before 4663's finalized block time.

**Nonce-line recovery**
- `HOME_NONCE_TAKEN_SECS`, `home_nonce_tx`, `home_effect` (:4215), `home_search_starts` (:138), `HOME_NONCE_SEARCH_BLOCKS`.
- All of this is built around EVM account nonces and the roughly 10-minute state window of Robinhood's RPC (:105-120).

**`ethnet.rs`:** the `EthIo` home impl (`home_sign` :278 builds the EIP-1559 transaction for 4663), plus the endpoints `eth_give_back`, `eth_credit_payout(chain_id…)`, `eth_home_clear`, `eth_wake`, `eth_fill`.

**`relayeth.rs`:** `TO_ETHEREUM` (4663→1) and `FROM_ETHEREUM` (1→4663) legs, each with the 4663 depository and allocator (:25-31).

**With SOL as the home currency:**
- The 4663→Ethereum leg becomes Solana→Ethereum (SOL→ETH swap-bridge): the Solana deposit is judged differently, and the reverse leg pays SOL.
- The advance gate and buy cap compare home-currency wei against an Ethereum floor in ETH (`ADVANCE_FEE_EST_BPS`, `ADVANCE_MARGIN_BPS`, `buy_cap`, `cost_of`, ethflow.rs:68-80; ethnft.rs:937-965). They need a SOL/ETH rate source and a price-exposure margin.
- The module's `postFloor` would be denominated in SOL, or kept in ETH with an on-chain conversion.

**Estimate:** ethflow ~1,800-2,200, ethnet ~250, ethnft ~300 (wrapper and module ABI), relayeth ~200, dormancy ~300.

**Unchanged on this side:** the Ethereum half is reusable — Seaport judge, transfer judge, floor evidence from OpenSea listings and Seaport/Blur sale logs, deposit judge on Ethereum, the gas pot, the Ethereum pin through `nftnet.rs` and `chains.rs`.

### A.7 Caps, limits and the remaining modules

- `custodycap.rs` (582): pure. Collection keys are `btc:0x<wrapper20>` and `<chainId>:0x<collection>` (:170-177), so the wrapper key format changes. ~40 lines.
- `outflow.rs` (496): pure, reusable.
- `chains.rs` (484): `validate(chains, home)` and `judge_relay_evm(body, home, dest…)` (:281-309) assume an EVM home. ~120 lines.
- `rates.rs` / `oracle.rs` (294 / 354): ETH-in-BTC reference from tickers plus the IC exchange-rate canister. Need SOL/BTC and SOL/ETH. ~150 lines.

### A.8 Reusable as-is (home-chain-agnostic)

- **Bitcoin core:** `psbt.rs` (1,346), `sighash.rs` (275), `runestone.rs` (777), `taptree.rs` (186), `custody.rs` (750), `btcsend.rs` (368), `btcnet.rs` (213), `conflict.rs` (617), `outbound::verify_btc`, `runeindex.rs` (349).
- **Judges:** `frogbuy.rs` (682), `owbuy.rs` (1,023, Ordinals Wallet protected listings).
- **Ordinals providers:** `ordapi.rs` (437), `ordnet.rs` (282) — ordinals.com, Ordinals Wallet, UniSat.
- **Ethereum mainnet:** `seaport.rs` (1,117), `nftnet.rs` (561).
- **Pure rules:** `itemsink.rs`, `outflow.rs`, most of `custodycap.rs`, the dormancy lifecycle, the `mintdesk` money rule, the `handin`/`sale` rules.
- **Timelock:** `autobuyer/timelock-rs` has zero home-chain references.

Roughly 11-12k lines are reusable unchanged. About 14-17k lines are touched or rewritten, plus about 10-12k test lines: `tests/fakerpc` (fake 4663 providers), `frogs.rs`, `ethnft.rs`, `redeem.rs`, `audit_nft_r*.rs`, `dormant.rs`, `sales.rs`, `handins.rs`, `loop.rs`, `providers.rs`.

### A.9 Solana-specific design breaks to plan for

1. **Signing:** Ed25519 threshold Schnorr on the IC for Solana. Keep the secp256k1 `"evm"` key for Ethereum. Today one key gives the same address on 4663 and Ethereum.
2. **Saved-bytes idempotence:** the core invariant is "sign, save raw bytes, send; rebroadcast the same bytes; one nonce, so at most one mines" (frogflow.rs:2175-2194, ethflow `HomeTx`, `EthTx` versions). Solana transactions expire with their recent blockhash (~60-90 s). A home transaction would need a durable-nonce account per signer line, or an explicit re-sign protocol. The `home_nonce_tx` / `HOME_NONCE_TAKEN_SECS` machinery disappears and is replaced by signature-status checks.
3. **Pinned reads:** every state read is an `eth_call` at an agreed block hash. Solana RPCs serve current state only (`minContextSlot`, no historical `getAccountInfo`). IC consensus on replicated reads of moving account state needs DFINITY's SOL RPC canister consensus or rounded projections in transforms. The 4663 finality and log-span workarounds (`MAX_LOG_SPAN_BLOCKS`, the 6,200-block state window, `finalized` lag) all become Solana `finalized` commitment, which is about 13 s.
4. **Clocks:** `Clock {time, parent_block}` uses Arbitrum's L1 `block.number` (frognet.rs:157-178; delivery.rs:13). Contract deadlines must move to slots or unix time.

---

## B. Off-chain services

### `ops/` (runner, fueler, reconciler, offer tool, rehearsals)

**Runner** (`ops/src/runner.ts`, 308; `deployments/launch/railway/runner.env.example`)
- Holds no chain key: `OPERATOR_IDENTITY_PEM` is an IC identity only. It calls canister `tick`.
- Fetches Relay quotes for frog conversions (`ops/src/relay.ts`: `ROBINHOOD = 4663`, `RELAY_DEPOSITORY_4663`, legs `"robinhood"`/`"bitcoin"`, a pre-judge mirroring `relayconv`, relay.ts:17-164).
- Fetches Relay quotes for the Ethereum step (`ops/src/ethnft.ts`: `LEGS {"4663>1", "1>4663"}`, :66-99).
- Also: OpenSea listings and evidence, Ethereum logs (`ETH_LOGS_RPC`, Dwellir), Ordinals Wallet protected and UniSat offers.
- Cadence: `RUNNER_PENDING_EVERY_S` exists because of 4663's ~10-minute state window (runner.ts:117-136, :299-302).
- Port: ~350 lines (relay.ts, ethnft.ts legs, runner cadence). OpenSea, ordinals and UniSat parts are agnostic.

**Fueler** (`ops/src/fueler.ts` 155, `fuel/*`)
- `FUELER_EVM_KEY` is a hot EOA with the same address on 4663 and Ethereum.
- The allowlist (fuel/engine.ts:73-83) permits, on 4663: `claimPlatformFees()` on curves, `ProtocolTreasury.allocate()`, a forward to the cold payee, and Relay `depositNative` (4663→Ethereum). On Ethereum: the ckETH helper deposit. Then ICPSwap ckETH→ICP and CMC `notify_top_up`.
- RPCs: `FUELER_RPC_4663` and `FUELER_RPC_1`.
- Port: claim/allocate on Solana, a Solana key, SOL→ETH via Relay. ~500 lines.

**Reconciler** (`ops/src/reconcile.ts` 304, `backing.ts` 104)
- No key. Reads 4663 with `eth_call` and `eth_getBalance`: FloorModule `backing`, `reserve`, `surplus`, `revenue`, `gradEth`, `rentalEscrow`, `renterPool`, `owedTotal`, `reserveUnits`, `accepted`, `fresh`, `escaped`, `postedAt`, `lastGoodAt`, `FLOOR_TTL`, `transitUnits`, `advanced`, `advancedAt`; Wrapper `ethBacked`.
- Uses the indexer's `WrappedService` over `stack-4663.json`, the canister's `custody_caps`, and the Bitcoin providers.
- Port ~300 lines.

**Offer tool** (`ops/src/offer.ts` 213): reads the ListingRegistry membership root from 4663 (`listingRoot`, :136-172, default robinhood RPC). Otherwise Bitcoin/ord.

**Agnostic:** `judge.ts`, `location.ts`, `owprotected.ts`, `unisatbuy.ts`, `frogmath.ts`, `ic.ts`, `identity.ts`, `stage.ts`, `ethRecords.ts`.

**Rehearsals:**
- `ops/rehearsal/relay` (4663 Relay quote judge).
- `ops/rehearsal/cketh` (Relay/fueler page).
- `ops/rehearsal/ic-outcall` (Ethereum provider pair from the IC; agnostic).

### `keeper/` (production keeper, `keeper/service/`; dry-run floor keeper, `keeper/src/`)

- **Keys:** `KEEPER_PRIVATE_KEY` must equal `registry.keeper()` and each module's `keeper()`. `KEEPER_RPC_URLS` are robinhood and ordofi; `KEEPER_CHAIN_ID=4663`.
- **Signs on 4663** (abis.ts):
  - `postFloor` for classic and Bitcoin modules (floor from Ordinals Wallet / ord books, `keeper/src/floor*.ts`, `ordapi.ts`);
  - `postCap(weiPerWrapped)` on sweep vaults;
  - `announceDraw(coin)` and `openDraw(id, root, totalTickets, uri, sentinelProof, seedSignature)`.
- **Draw snapshots:** coin `Transfer` logs through `tools/snapshot/src/core.ts`, at the last block at or before `snapshotAt`, uploaded to S3/R2 (service/draws.ts).
- **Sender:** EIP-1559 sender with gas and fee caps (sender.ts, signer.ts with viem `privateKeyToAccount`).
- **Resolver** (`resolver-main.ts`, `resolver.ts`): `RESOLVER_PRIVATE_KEY` is any funded 4663 account. It calls `Wrapper.resolve(id, drandSignature)` and `SweepVault.resolve` / `requestDraw`.
- **Drand:** `service/drand.ts` (agnostic). On-chain verification uses `DrandEvmnet` (BN254).
- **Port:** Solana signer and sender, SPL holder snapshots (no historical state, so the holder index must be built from transactions). ~1,500-2,000 of ~3,300 lines.
- **Agnostic:** `keeper/src/floorsources.ts`, `ordapi.ts`, `opensea.ts`, `floor.ts` math, `drand.ts`, `s3.ts`.

### Indexer (`indexer/`)

- `RPC_URL` points at the robinhood RPC; `CONFIRMATIONS=20`. `follow.ts` (361) polls `eth_getLogs` for:
  - registry, factory and vault events (abis.ts:5-87);
  - curve `Bought`/`Sold`/`Graduated` and coin `Transfer` (rook/abis.ts);
  - Uniswap v4 PoolManager `Swap`/`Initialize`.
- USD price: `usd.ts` reads the WETH/USDG v3 pool `0x52e65b…71ca` on 4663.
- `wrapped/` and `v8/` read FloorModule and wrapper state (v8/abis.ts: `Wrapped`, `Deposited`, `Redeemed`, `Resolved`, `Delivered`, `Undeliverable`, `Refunded`, `AdvanceSettled`).
- The catalog uses `RPC_OF.robinhood` (catalogSources.ts:210).
- **Port:** follower, ABIs and USD source rewritten (~1,500 lines).
- **Agnostic:** db, views, candles, api, routes, v8 draw/odds/rarity/drand, catalog OpenSea parts.

### Monitor (`monitor/src`)

- **4663 reads:** `evm.mjs` (895, the 4663 canary: Wrapper and FloorModule custodian-gated events matched to canister records, registry and vault logs, draw roots), `chain.mjs` (keeper/resolver liveness from 4663 state), `holders.mjs` (per-provider ERC-20 holder index), `draws.mjs`, `abi.mjs`, `stack.mjs`.
- **Key:** `guardian.mjs` holds `MONITOR_GUARDIAN_KEY` and signs on 4663: `pauseKeeper()`, `PosterSource.pause()`, `SweepVault.cancelDraw(id)` (:105-131).
- **Mixed:** `ethnft.mjs` reads `home_rpc` (4663) and the Ethereum pair; `fueler.mjs` reads nonces on 4663 and Ethereum (`rpcs_4663`, `rpcs_1`).
- **Config:** `config.mjs` requires `evm.rpcs` to be exactly two 4663 RPCs, plus `evm.chain_id`.
- **Port:** ~2,000 lines.
- **Agnostic:** `canary.mjs` (Bitcoin custody), `custodyhealth`, `conversions`, `records`, `watcher` / `argdiff` / `candid` / `staged` (IC timelock), `alerts`, `liveness`, `ic.mjs`.

### `script/`, `scripts/`, `staging/`, `tools/`, `deployments/`

- `script/golive-check.sh` (1,706, cast-based): Safe ceremony checks, EIP-7702 no-code checks, the 4663 `--phase1` default.
- `chain-guard.sh`: on 4663 requires ledger or keystore.
- `deploy*.sh`.
- `scripts/golive-dryrun.sh` (944) and `keeper-fork-test.sh` (241): anvil forks of 4663 from `rpc.ordofi.network` under staging chain id 9466301.
- `scripts/timelock-watch.mjs`: IC only, agnostic.
- `staging/`: a hosted anvil fork of 4663 with proxy, faucet, sim and smoke; chain id 9466301. The whole environment is EVM-specific.
- `tools/snapshot`: ERC-20 Transfer-log snapshots. Algorithm reusable; data source not.
- `script/homechain/DeployHome.s.sol`: Ethereum-side vault using Robinhood's Arbitrum Inbox/Outbox. 4663-specific.
- `deployments/launch/railway/*.env.example`:
  - keeper / resolver / reconciler: `*_CHAIN_ID=4663`, `stack-4663.json`, robinhood/ordofi RPCs;
  - fueler: `FUELER_TREASURY` on 4663;
  - indexer: `RPC_URL`, `WRAPPED_CONFIG=stack-4663.json`;
  - monitor: guardian key and Dwellir.
- `nft-pilot.json` and `monitor-nft.json`: canister install argument with 4663 `rpc_urls`, launch caps and stages (amounts in wei).

---

## C. Web apps

**`web-rook/` (current site, ~10k lines)**

- **Wallet stack:** wagmi 2.19.5, @wagmi/core 2.22.1 `injected()` only (no RainbowKit, no WalletConnect), viem 2.56.8, react-query.
- **Chains:** `Web3.tsx` configures `robinhood {id 4663, rpc.mainnet.chain.robinhood.com}`, local 31337, and staging 9466301 (`lib/chains.ts`). Explorer is `explorer.mainnet.chain.robinhood.com` (`lib/explorer.ts`).
- **Single send path:** `ui/wallet.tsx` `TxButton` (`switchChainAsync`, `writeContractAsync`, `waitForTransactionReceipt`). Phase-1 guards in `rook/phase.ts` allow sending only with the bundled `src/deployments/stack-4663.json` (`rook/stack.ts`).
- **Pages and calls:**
  - `Launch.tsx`: reads WakeDesk `openOf`, `requestAt`, `window` and FloorModule `fresh`, `postedAt`; writes `WakeDesk.request(wrapper){bond}` and `CurveFactory.launch(name, symbol, policy, sweepOnly, feeBps)`.
  - `ui/Ticket.tsx` (trading): FrogCurve `buy`, `sell`, `refund`, `graduate`, `openRefunds`; ERC20 `approve`; EtchRouter `buy`/`sell`.
  - `lib/liveQuote.ts`: Uniswap v4 StateView `getSlot0`, V4Quoter, LiquidityLock, FloorModule `price`, `spreads`, `quoteMint`, `quoteBurn`.
  - `ui/Raffles.tsx`: `requestDraw`, `voidDraw`, `resolve(id, sig)`, `claim(...)`.
  - `Redeem.tsx`: `Wrapper.redeem(destination)`, `resolve`.
  - `Wallet.tsx` / portfolio, `Reserves.tsx`, `ReservesEth.tsx` (indexer `/v1/v8`), `Trust.tsx` (`rook/phase1Trust.ts`, static).
- **ABIs:** `lib/abis.ts`, `lib/abisV3.ts`, `rook/abis.ts`, `rook/v8/core/abis.ts`.
- **Indexer:** `VITE_ETCH_API` (`api/source.ts`), with fixtures as fallback.
- **Port:** replace wagmi with a Solana wallet adapter / Wallet Standard plus Anchor clients; rewrite TxButton, the quote reads and the deployment loaders.
- **Reusable:** `api/` types, `rook/v8/core` (drand verify, odds, rarity, ethReserve), `bech32m.ts`, charts, UI, content.
- The repo already has Solana catalog fixtures (`web-rook/public/fixtures/catalog/*-solana.json`).

**`web/` (legacy "etch" site):** same wagmi/viem 4663 pattern (`WxGate`, `vaults`, `OneClick`, Pons coins).

---

## D. Tests and CI that are 4663-specific (`.github/workflows/ci.yml`)

- **Global env:** `CODE_SIZE_LIMIT 98304` and `INITCODE_SIZE_LIMIT 196608` are 4663's limits (:13-19).
- **`contracts` job:** forge build, a size check against those limits, `forge test --no-match-path test/fork/*`, `script/deploy.test.sh`. `contracts-deep` is the same with deep campaigns.
- **`fork` job (nightly):** `forge test --match-path test/fork/* --fork-url https://rpc.mainnet.chain.robinhood.com` (:218-226).
- **`node` matrix:** tools/snapshot, keeper, staging, monitor, ops, autobuyer JS (whose `evm.mjs` / `evm-rehearsal.mjs` target 46630).
- **`indexer`:** builds forge ABIs and replays recorded 4663 fixtures.
- **`web`:** both sites.
- **`rust`:** `cargo test` for canister-rs and timelock-rs. The canister tests use `tests/fakerpc`, which models the 4663 providers' limits.
- **`depth-probe.yml`:** Ethereum-mainnet log retention. Agnostic; keep.
- **Harnesses outside CI:** `script/golive-check.sh` plus `scripts/audit2_golive_check.test.sh` (515), `golive-dryrun.sh`, `keeper-fork-test.sh` (anvil 4663 forks), `keeper/test/dwellir-4663.test.ts`, and the `autobuyer/scripts/*-rehearsal.sh` scripts (`nft-fork-rehearsal` is Ethereum).

---

## E. Chain-agnostic and reusable as-is

- **Canister:** psbt, sighash, runestone, taptree, custody, btcsend, btcnet, conflict, outbound (BTC half), frogbuy, owbuy, ordapi, ordnet, runeindex, seaport, nftnet, the Ethereum parts of chains/ethnft, itemsink, outflow, the custodycap logic, the dormancy lifecycle, the pure rules of mintdesk/handin/sale.
- **Timelock:** all of `timelock-rs`.
- **Ops:** judge, location, owprotected, unisatbuy, frogmath, ic, identity, OpenSea and ordinals fetching, the ic-outcall rehearsal.
- **Keeper:** floor sources and drand.
- **Monitor:** Bitcoin canary, IC watcher, argdiff, candid, staged, alerts, liveness, conversions, records.
- **Indexer:** API, db, views, candles, v8 core.
- **Web-rook:** UI, v8 core, bech32m.
- **CI:** depth-probe.
- **Data:** the Bitcoin Frogs members file and root.

**Rough total for the port outside Solidity:** about 15k canister source lines and about 10k canister test lines rewritten, plus about 7-9k service lines and about 3-4k web lines.
