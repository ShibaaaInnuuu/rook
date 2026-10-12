# Reference: the Robinhood Chain contract system as it stood at `robinhood-stop-2026-10-09`

Read-only functional map made 2026-10-09 for the Solana migration (`solana-migration.md`). Describes `src/` at tag `robinhood-stop-2026-10-09`; file:line refs are to that tag.


Everything below was read from source; no files were modified. All paths are under `/Users/jlohayon/Documents/Rook`.

Three facts frame the whole system:
- **The real custodian is off-chain.** It is an IC canister (`autobuyer/`). Its threshold-ECDSA EVM address is the "custodian" and "poster" in every contract. It holds the NFTs on Ethereum (and the frogs on Bitcoin), and it is the only minter of wrapped units.
- **The 72 h timelock is not on the EVM side.** It is the canister's timelock (`timelock-rs`), which gates canister upgrades only. On-chain governance is a Safe plus 7-day delays.
- **No contract is upgradeable.** There are no proxies, `Initializable` or `delegatecall` anywhere in `src/`. Everything is immutable, with one-time `wire*` calls.

---

## 0. What the current launch actually deploys

Sources: `docs/v7/golive-runbook.md` §9.2, `deployments/launch/nft-pilot.json`, `docs/v6/phase1-scope.md` §1, `docs/v7/phase1-nft.md` §0.

The deploy runs in three steps:
1. **`script/deploy.sh` → `script/Deploy.s.sol`.** It deploys `ListingRegistry` (owner = Safe), `DrandEvmnet`, `Launcher` (Pons V2 at `0x7eD5…EC7e`, which clones `FeeRouter`) and `Distributor`. The Safe then calls `wire(launcher, distributor)` and `proposeRoles(keeper, purchaser, attestor)` (`Deploy.s.sol:51-69`). The script refuses an owner that is the deployer or an EIP-7702-delegated EOA (`:44-46, :107-109`).
2. **`script/list-auto.sh` → `ListAuto.s.sol`.** On 4663 it only prints calldata. The Safe sends `listInscriptionsAuto("Bitcoin Frogs", "bitcoin-frogs", root 0x801f…418e, custody_address, buyer=canister evm_address, maxInFlight=min)`. That call creates an `AutoBuyerVault`.
3. **One `script/deploy-stack.sh` → `DeployStack.s.sol` run** of the default ("btc") kind, with the 11 Ethereum collections added through `ETCH_NFT_*` lists. It deploys:
   - one `ProtocolTreasury` and one `CollectionDirectory` (`:163-164`);
   - for Bitcoin Frogs: a `WrapClaim`, which itself builds the `Wrapper` (`:178`);
   - for each NFT collection: an `EthNftWrapper` and a `PosterSource(registry, custodian)` (`:297-300`);
   - for every collection: `MintDesk`, `FloorModule`, `LiquidityLock`, `CurveFactory`, `EtchRouter` and three `SweepVault`s (RAFFLE, BURN, YOYO), wired together (`:320-353`);
   - for Frogs only: an `OfferBoard` (`:349`);
   - one `WakeDesk` (`:198-208`), since the bond is set (`ETCH_WAKE_BOND=0.002 ETH`, `ETCH_WAKE_WINDOW=604800`).

The pool manager is Uniswap v4 at `0x8366a39C…0951` on 4663 (`:56`). The Safe's owner steps are printed for it to send (`:385-412`):
- `WrapClaim.initMintLimit(3)` and `WrapClaim.wireFloorModule`;
- `ProtocolTreasury.setOps` and `addCollection(module)`;
- `CollectionDirectory.add(factory)`;
- `ListingRegistry.setGuardian`;
- `SweepVault.proposeCapCeiling` on all three vaults (the first ceiling applies at once).

**Collections.** The NFT collections are Milady, Azuki, MAYC, Pudgy, BAYC, Doodles, Moonbirds, CloneX, Lil Pudgys, Checks VV and Identity MD, all on chain 1. Frogs is the one Bitcoin collection. The defaults are normalRaise 5 ETH, earlyRaise 8, earlyUntilEth 2, maxRaise 30, convBps 300, reserveBps 2500. BAYC is overridden to 10/10 ETH, convBps 1000, reserveBps 0. Stage 0 means 3 mints a day per wrapper and module `maxItemsPerDay` 3 (`script/StackChecks.sol:53-58, 171-195`).

**Paths ON at launch:**
- curve launches and trading;
- graduation → FloorModule → canister advance → buy (Seaport on Ethereum; operator hand-ins on Bitcoin);
- FloorModule mint and burn;
- redemption and delivery;
- NFT holder deposits (`eth_nft.deposits=true`).

**Paths OFF at launch:**
- YOYO;
- MintDesk (deployed, never served);
- OfferBoard (deployed for Bitcoin, unfunded);
- rentals;
- Bitcoin holder deposits;
- `Wrapper.wrap` / the AutoBuyerVault purchase path;
- Pons `Launcher` coins (deployed, not offered);
- `NftWrapper` / `NftRegistry` / `src/homechain/*` (not deployed);
- bootstrap loans and floats (the treasury starts at 90/0/0/10).

**Sweeps (RAFFLE/BURN):** the docs disagree.
- They are ON for the frog stack, but need the keeper EOA to post caps and run draws (`phase1-scope.md:55`).
- `phase1-nft.md:352` says caps and draws are "OFF for sweeps in Phase 1" for NFT collections.
- In every case the vaults are deployed and the curve fees accrue in them.

---

## 1. Contract-by-contract

### Governance / registry

**`ListingRegistry`** — `src/ListingRegistry.sol`, 431 lines.
- **Ownership:** Ownable2Step, owned by the Safe. `renounceOwnership` reverts (`:162`).
- **State:**
  - listings (kind, paused, autonomous, vault, membershipRoot, name, ref, btcVault);
  - roles: keeper, purchaser, attestor and guardian;
  - `keeperPaused` and `keeperResumedAt(Block)`;
  - `isAutoBuyer` (an auto-buyer may hold no other role);
  - `treasury` (the v1 FeeRouter 20% recipient, not the ProtocolTreasury).
- **Owner-only:**
  - `listInscriptions`, `listInscriptionsAuto` (`:185`), `listRune(Auto)`;
  - `setPaused(id)` (`:240`), instant;
  - `wire` and `wireNft` (once);
  - `setGuardian` (instant);
  - `proposeRoles`/`acceptRoles` (first set instant, later 7 days; `CHANGE_DELAY` at `:32`);
  - `proposeRoot`/`acceptRoot` and `proposeBtcVault`/`acceptBtcVault` (7 days);
  - `setBondBps`, `proposeTreasury`.
- **Owner or guardian:** `pauseKeeper()` (`:295`). Only the owner may `unpauseKeeper()` (`:302`). While paused, `keeper()` returns `address(0)`, which halts every keeper-priced inflow everywhere.
- **Anyone:** `isMember(id, inscriptionId, proof)` (keccak double-hash leaf, OZ sorted-pair Merkle).
- **Events:** `Listed`, `AutoBuyerNamed`, `Paused`, `RolesProposed/Set`, `RootProposed/Set`, `BtcVaultProposed/Set`, `KeeperPaused`, `GuardianSet`, `Treasury*`.

**`PosterSource`** — `src/floor/PosterSource.sol`, 80 lines. This is the `IKeeperSource` for NFT FloorModules.
- `keeper()` returns the canister (immutable `poster`), or zero if this source or the registry is paused (`:63`).
- `pause()` can be called by the registry owner or the guardian (`:41`); `unpause()` by the owner only. This is the per-collection floor kill switch driven by the monitor's D8 guard.
- **Events:** `PosterPaused/Unpaused`.

**`CollectionDirectory`** — 57 lines.
- Append-only list of (wrapper, factory, queue=module, lock).
- `add(factory)` is admin-only (admin = the Safe, immutable). It checks that the contracts name each other.
- The canister and the WakeDesk read it.
- **Event:** `Added`.

### Coin launch and trading

**`CurveFactory`** — `src/curve/CurveFactory.sol`, 232 lines. One per collection. No owner.
- `wire(raffle, burn)` and `wireYoyo` are called once by the deployer.
- **`launch(name, symbol, policy, sweepOnly[, feeBps])`** — anyone (`:140-182`).
  - It deploys a `FrogCurve` with `new`; the curve deploys its own `CurveToken`.
  - It records `routerOf/creatorOf/listingOf/policyOf/feeOf` and `coins[]`.
  - It refuses with `QueueFull` if the module has no room for the item budget (`:165`).
- **`raiseFor`** (`:185-199`) picks the raise:
  - `earlyRaise` while the lock's `sharedEthIn < earlyUntilEth`, otherwise `normalRaise`;
  - an "opening" coin (sharedEthIn == 0) needs at least one item at the fresh sweep-vault cap × (1+buffer), plus the fee cap, grossed up for poolEth;
  - anything over `maxRaise` reverts `TooExpensive`;
  - `HARD_MAX_EARLY_RAISE = 10 ETH`.
- `itemBudgetFor(raise) = raise − min(5%·raise, 0.25 ETH) − 12.6%·raise` (`:203-207`).
- Fee tier: 1–8% (`_targets`, `DeployStack:525-528`).
- **Events:** `Launched`, `FeeTier`.

**`FrogCurve`** — `src/curve/FrogCurve.sol`, 563 lines. One per coin.
- **Template** (`DeployStack:497-502`):
  - supply 1e9·1e18, of which 200M is reserved for the coin's own pool;
  - platformBps 500 with a cap of 0.25 ETH;
  - poolEthBps 1260;
  - feeBps 100 (1%);
  - fee split vault 40 / creator 30 / reserve 5 / liquidity 5 / platform the remaining 20;
  - window 3 days;
  - launch fee +50%, decaying linearly over 10 minutes, all of it to the sweep vault.
- **Math** (`:206-228`): a virtual constant product.
  - The end price is P = (itemBudget − poolEth)/reserved, so the curve's last price equals the coin pool's opening price.
  - Solved as B = E·S/(P·S−E), A = P·B, v0 = A−E, t0 = B+S.
- **`buy(minOut[,deadline])`** (`:252-296`):
  - The buy that reaches the raise is clipped and closes the curve.
  - It refuses `QueueFull` if the module cannot convert.
  - It sets state Graduating and calls `queue.enqueue{value: itemBudget}`.
- **`sell`** (`:309-323`): curve trading only.
- **`graduate()`** — anyone (`:352-377`), once the module `isFilled`.
  - `queue.release(token)` returns the units.
  - toCoin = units·(budget−poolEth)/spent go to COIN/wX with the reserved supply.
  - The rest of the units plus poolEth go to the shared wX/ETH pool via `seeder.seed`.
  - The platform fee is credited.
  - SWEEP_ONLY coins go straight to COIN/ETH.
- **`openRefunds`** (`:386-405`): after the 3-day window if unfilled, or after window + `STUCK_GRACE` (4 days) if filled but stuck. `refund` (`:416`) pays pro-rata ETH plus any units already bought.
- **Fees:**
  - `flushVaultFees` → `vault.deposit`;
  - `claimCreatorFees` (creator only);
  - `claimPlatformFees` → ProtocolTreasury;
  - `flushProtocolFees` → `module.fundReserve` / `fundLiquidity` (`:455-467`);
  - the lock-only hooks `onPoolFees`, `depositPoolFees`, `forwardWrappedFees`, `forwardTokenFees` (the vault's coin-fee share is burned to `0x…dEaD`).
- **Events:** `Bought`, `Sold`, `GraduationStarted`, `Graduated`, `RefundsOpened`, `Refunded`, `PoolFees`, `*FeesForwarded`, `ProtocolFeesFlushed`.

**`CurveToken`** — 13 lines. A plain OZ ERC-20 whose whole supply is minted to the curve. No owner.

**`LiquidityLock`** — `src/pools/LiquidityLock.sol`, 829 lines. Protocol-owned Uniswap v4 liquidity.
- It holds positions inside the PoolManager and never passes a negative liquidity delta, so **the liquidity is locked forever**.
- **Pools:**
  - each coin gets a hookless pool, COIN/wX (or COIN/ETH for SWEEP_ONLY), with LP fee = the curve's feeBps, tick spacing 200, one ±180,000-tick position (`:788-796`);
  - each collection gets one shared native-ETH/wX pool at 0.30%, spacing 60, built as layers: near ±960 (50%), wide ±2640 (30%), far ±180k, plus an ETH-only bid (`DeployStack:490-494`).
- **Price guard:** shared adds are priced against the module's fresh `price()` within 200 ticks (`:418-456`), otherwise deferred.
- **Who calls what:**
  - `seed`/`seedEth`: the curve only;
  - `addProtocolLiquidity`: anyone, with ETH;
  - `seedCoin`, `deploy`, `moveCenter`, `claimShared` and `collectCoinFees`: anyone.
- **Fees:** coin-pool fees go to the curve. Shared-pool fees go pro rata to each coin's sweep vault. Protocol positions send ETH fees to the treasury and compound their wX.

**`EtchRouter`** — 276 lines. Stateless; anyone may call.
- `buy`/`sell` route ETH↔COIN through the coin pool and the shared pool.
- `buyWrapped`/`sellWrapped` route ETH↔wX to whichever of the shared pool and the FloorModule is cheaper.
- It is the `venue` SweepVault calls.

### Floor, backing and custody

**`FloorModule`** — `src/floor/FloorModule.sol`, 1,469 lines. One per collection. **No owner and no setter**; every parameter is immutable (`:57-81, 145-167`).
- **Constants:**
  - `FLOOR_TTL` = 3 h (`:112`);
  - `MAX_DAY_MOVE_BPS` = 2,000 (`:122`);
  - `STALE_ESCAPE` = 7 days;
  - `RESUME_GRACE` = 6 h;
  - `FILL_WINDOW` = 24 h with 8 fills.
- **Defaults** (`StackChecks.defaultParams`):
  - spreads: baseSpread 15, maxMove 500/h, staleSpread 100, stress 300, max 1,000;
  - mint and burn: 5 items/h each;
  - ETH-backed caps: maxPublicEthBacked 10, maxEthBacked 40;
  - revenueTarget 1 ETH;
  - confidence: minConfidence 3,000, full 8,000;
  - reserveBps 2,500, fillBand 300, buffer 500.

*Keeper and poster functions* (keeper read live from `registry.keeper()` — the ListingRegistry for Frogs, the PosterSource / canister for NFTs):
- `postFloor(price, confidenceBps, depthItems)` (`:389-417`) clamps the move to ±maxMove per hour of the slower clock and ±20% of the day anchor.
- `postRentalRoot` (rentals are OFF).

*Anyone:*
- **`mint(to, minUnits)`** (`:457-478`): pay ETH, receive wX at max(fills-TWAP, target)·(1+spread). This needs a fresh floor and no pause. It is bounded by the hourly bucket, `publicCap = min(10 items, depthItems)` and room left for graduations. `reserveBps` of the ETH and units go into the reserve tier (`_intake` `:481`).
- **`burn(units, minEth, to)`** (`:504-522`): reserve-tier units are burned first.
  - Each tier pays min(floor·(1−spread), its own backing per unit).
  - Burns still work while paused, at `pauseFloor`, and after 7 days with no good post (`escaped`) at backing per unit.
- `fill(coin)`, `snapshotCap()`, `skim()` (revenue above `revenueTarget` → platform), `fundReserve()` (→ `surplus`, after repaying any loan), `fundLiquidity()` (→ lock), `endRental`/`settleRental`/`payDebt`/`claim`.

*Custodian (canister) only:*
- **`advance(amount)`** (`:687-713`). One advance may be open at a time; amount ≤ `maxItemCost` = min(itemCap = accepted·(1+convBps), backing per unit + revenue + surplus), and only if there is arrival room today.
  - It moves one convertible unit into `transitUnits` and pays `payee` (= the canister's 4663 address) the ETH.
  - Any shortfall comes from revenue, then surplus.
- **`settleAdvance(cost)`** (`:718-732`): requires the wrapper to have minted one unit to the module. It burns that unit with `moduleBurn`, turning one ETH-backed unit into an item-backed one, and records a fill.
- **`returnAdvance()`** (payable, `:738-761`): restores the basis; any loss beyond revenue and surplus is shared by the convertible units.
- `itemArrived(cost)` (`:634-661`): the older pay-on-arrival path.
- `itemSold` (floats), `reclaimStray`, `rentIn`.

*Curve-only (IItemQueue):*
- **`enqueue(coin)`** (`:826-836`) followed by `_tryFill` (`:844-858`). It converts the budget at `gradPrice = max(price, upTarget)·(1+convBps)` (`:1259`) into ETH-backed units minted to the module (`gradUnits`), capped by `maxEthBacked`.
- `release` and `cancel`.

*Treasury only:* `fundFloat` (floats are 0 at launch).

*The books* (`:195-221`):
- balance ≥ backing + reserve + surplus + revenue + gradEth + rentalEscrow + renterPool + owedTotal;
- `netUnits = ethBacked − transit − reserveUnits` is the canister's buy signal: it buys when netUnits ≥ 1 + buffer.

*Events:* `FloorPosted`, `Minted`, `Burned`, `ItemArrived`, `Advanced`, `AdvanceSettled`, `AdvanceReturned`, `GradQueued/Filled/Released/Cancelled`, `ReserveFunded`, `LiquidityFunded`, `Skimmed`, `FillRecorded`, `CapSnapshot`, `LoanRepaid`, rental events.

**`EthNftWrapper`** — `src/nft/EthNftWrapper.sol`, 474 lines. An ERC-20 where 1e18 = one ERC-721 held at the canister's address on chain 1.
- **Immutables:** registry, listingId, beacon, chainId (must differ from this chain), collection, custodian (must have no code), deployer.
- **Deployer-only** (the **deploy EOA**, not the Safe; `DeployStack:303`):
  - `initMintLimit` (once, before any mint);
  - `wireFloorModule` (once, and only a module whose `wrapped()` is this wrapper);
  - `proposeMintLimit` (lowering is instant; a raise waits 7 days and then anyone may `acceptMintLimit`).
  - **Governance gap:** this deployer power stays with the deploy key.
- **Custodian only:**
  - `mint(tokenId, purchaseTx, to)` (`:285`) and `mintDeposit(tokenId, depositTx, to)` (`:292`): refused while the listing is paused, at most once per tokenId, at most once per (tokenId, tx), and subject to H11's mints per day;
  - `recordDelivery`, `noteSent`, `markUndeliverable`.
- **Module only:** `moduleMint`/`moduleBurn` (these track `ethBacked`).
- **Anyone:**
  - `redeem(ethDest)` and `redeemToBurn()` (to `0x…dEaD` on Ethereum) burn 1e18 and open a ticket pinned to drand round = roundAt(now)+20 (`:336-349`);
  - `resolve(id, sig)` must go strictly in order and draws an item from those free when the ticket was burned (`:353-379`); it sets `deliverBy` = +7 days;
  - `refund(id)`: from Pinned after 7 days; from Resolved after deliverBy+72 h if marked undeliverable, or after +30 days if no `noteSent` (`:432-458`).
- `redeemToRelist` reverts, and rentals revert.
- **Invariant:** totalSupply = (held − openTickets)·1e18 + ethBacked.
- **Events:** `Wrapped`, `Deposited`, `Redeemed`, `Resolved`, `Delivered`, `DeliverySent`, `Undeliverable`, `Refunded`, `FloorModuleWired`, `MintLimit*`.

**`Wrapper`** (Bitcoin frogs) — `src/Wrapper.sol`, 640 lines. The same machine keyed by inscription.
- `wrapDeposit(inscriptionId, proof, txid, vout, to)` (`:423`) is custodian only and checks Merkle membership via the registry. Frog buys use `to = module`.
- `wrap(acquisitionId)` is the v2 path and is OFF.
- `redeem(taproot)` checks a bech32m taproot address; `redeemToBurn` sends to a NUMS taproot.
- It supports rentals (OFF).
- **Deployer = WrapClaim**, so wiring is forwarded from the Safe.

**`WrapClaim`** — 209 lines.
- It deploys its own `Wrapper` and is that Wrapper's `distribution` and `deployer`.
- The Safe forwards `wireFloorModule`, `initMintLimit` and `proposeMintLimit` through it.
- The keeper-root claim flow for `wrap` (`open`/`claim`/`rollOver`, 15-minute challenge, 30-day claims) is unused at launch.

**`ProtocolTreasury`** — `src/floor/ProtocolTreasury.sol`, 270 lines.
- It receives the platform trade fee, graduation fees, module `skim`s, protocol-LP ETH fees and forfeited wake bonds.
- **Owner (Safe), instant:** `setShares` (starts 9000/0/0/1000), `setOps`, `setBootstrapLoan`, `addCollection(module)`, `forgive`.
- **Anyone:**
  - `allocate()` pushes the ops share to `ops`;
  - `distribute()` sends the reserves pot to modules by `volume7d` via `fundReserve`, and the floats pot via `fundFloat`;
  - `bootstrap(module)`.
- **Module only:** `repay()`.
- There is no withdraw function, but the Safe can redirect everything instantly (stated at `:18-24`).

**`WakeDesk`** — `src/floor/WakeDesk.sol`, 290 lines. No owner.
- `request(wrapper)` costs exactly `bond` (0.002 ETH). It is refused if a request for that wrapper is already open or the module is `fresh()` (`:121`).
- `reclaim(id, coinIndex)` (`:137`) returns the bond if the requester's own coin, launched after the request and not sweep-only, is Graduating or Graduated, before window + 1-day grace.
- `forfeit(id)` (`:162`): if the wake was served inside the window, the bond goes to the treasury; otherwise it goes back to the requester.
- `markServed(id)`: anyone; the canister sends it after its post.
- `withdraw`.
- The canister polls `requestCount`/`requestAt`.
- **Events:** `Requested`, `Reclaimed`, `Forfeited`, `Refunded`, `Served`, `Owed`, `Withdrawn`.

### Sweeps and randomness

**`SweepVault`** — `src/SweepVault.sol`, 742 lines. RAFFLE, BURN or YOYO, fixed at construction.
- **Router-only (the coin's curve):** `deposit(coin)` and `depositWrapped`.
- **Keeper only:**
  - `postCap(wei per wX)` (`:280`): ±20% steps, at least 10 minutes apart, never above `capCeiling`;
  - `announceDraw` (`:414`), `openDraw` (`:471`; root plus a sentinel leaf committing count and snapshot time);
  - `sweepViaDesk`.
- **Owner:** `proposeCapCeiling` (the first ceiling or a lowering is instant; a raise waits 7 days); `setMarkup` (YOYO).
- **Owner or guardian:** `cancelDraw`.
- **Anyone:**
  - `sweep(coin, spend, swapData, minOut)` (`:337`) calls the venue (EtchRouter) and measures the result, requiring spent ≤ cap·bought;
  - draw steps `requestDraw` (after a 15-minute challenge; pins round+20), `resolve`, `claim` (Merkle range proof), `voidDraw`;
  - BURN: `retire` → `wrapper.redeemToBurn`, and `reclaim`.
- `CAP_TTL` = 1 h; there is no withdraw.

**`DrandEvmnet`** + **`BN254HashToCurve`** — 87 + 118 lines.
- They verify drand evmnet signatures on-chain (bls-bn254, a round every 3 s, genesis 1727521075) using the EVM pairing precompile `0x08` (`DrandEvmnet.sol:81`).
- `prove(round, sig)` can be called by anyone.

**What uses randomness:**
- wrapper redemption (which NFT or frog leaves);
- SweepVault RAFFLE (the snapshot time inside the previous 24 h, then the winning ticket);
- `Distributor` lots (v1/v2, OFF).

### Deployed but inert or peripheral

- **`MintDesk`** (135 lines): `request` is open to anyone, `fulfill` is custodian only, `collect`. OFF because the canister is given no desk.
- **`OfferBoard`** (297 lines): bonded PSBT hand-ins for Bitcoin. OFF.
- **`AutoBuyerVault`** (497 lines): created by `listInscriptionsAuto`; buyer = canister, immutable. Inert: `open_purchase` is refused.
- **`Distributor`** (434 lines): provides `beacon()` to the wrappers, which is used. Its lot machinery is OFF.
- **`Launcher` + `FeeRouter`** (87 + 117 lines): Pons V2 clones via `Clones.cloneDeterministic` (CREATE2, `Launcher.sol:62`). Deployed, not offered.

---

## 2. Coin lifecycle, end to end

1. **Launch.** Anyone calls `CurveFactory.launch`, which creates a curve and token. The raise is 5 ETH, or 8 ETH while the shared pool has received less than 2 ETH, or larger for the opening coin; BAYC is fixed at 10 ETH.
2. **Trading.** Anyone trades on the curve, paying the fee tier (1% default, plus the decaying launch fee).
   - The fee splits 40% sweep vault, 30% creator, 5% the collection's reserve, 5% protocol liquidity, 20% ProtocolTreasury.
3. **Close.** The buy that reaches the raise closes the curve.
   - The platform's graduation fee is min(5%, 0.25 ETH).
   - The shared-pool ETH side is 12.6%.
   - The item budget (e.g. 4.12 ETH from 5, 6.742 from 8, 8.49 from 10) goes to `FloorModule.enqueue`.
4. **Conversion at the module.** The module converts at once at `gradPrice = floor·(1+convBps)` into ETH-backed wX.
   - `reserveBps` (25% by default) of that ETH and those units go to the never-converted reserve tier.
   - So a graduated coin is backed partly by NFTs and partly by ETH.
5. **Graduation.** Anyone calls `graduate()`.
   - The **DEX is Uniswap v4 on 4663** (the PoolManager singleton, hookless pools).
   - COIN/wX gets the 200M reserved coins and the units worth (budget − poolEth).
   - The shared ETH/wX pool gets poolEth plus the remaining units.
   - **All of this liquidity is permanently locked in `LiquidityLock`.**
6. **After graduation.**
   - Coin pool LP fees: wX fees are split 40/30/30 (vault/creator/platform); coin fees are 40% burned, 30% creator, 30% platform.
   - Shared-pool fees go to each coin's sweep vault, where they fund RAFFLE/BURN sweeps of wX.
   - `EtchRouter` arbitrages the shared pool against the module.
7. **Failure paths.**
   - If the module cannot convert (stale or paused floor, or maxEthBacked full), `QueueFull` keeps the curve trading.
   - An unfilled curve refunds after 3 days (or 3+4 days while the keeper is paused).

---

## 3. NFT-backed floor and custody loop

The flow is from `phase1-nft.md` §2.1.

1. The canister sees netUnits ≥ 1.05 and calls `advance(≤ maxItemCost)`. The ETH goes to the canister's 4663 address.
2. The canister bridges that ETH through Relay to its own Ethereum address.
3. It judges a Seaport listing and fills it on Ethereum from its threshold-ECDSA address.
4. After finality, it calls `EthNftWrapper.mint(tokenId, buyTx, module)` and then `settleAdvance(cost)`.
5. If the buy fails, it calls `returnAdvance` (payable), possibly after a reverse Relay leg.

**Floor posts.** For NFT modules the poster is the canister, via `PosterSource`.
- It posts every 20 minutes from verified Seaport listings, bounded by on-chain Seaport/Blur sales.
- For Frogs, the keeper EOA posts from the Ordinals Wallet book.
- A post older than 3 h (`FLOOR_TTL`) is stale, and mints, conversions and graduations stop. Burns continue under the rules in §1.

**Holder deposits.**
- On Ethereum: `safeTransferFrom(holder, canister, id, "EWN1"‖uint32 4663‖wrapper‖recipient)` (48 bytes). The canister verifies it past finality and calls `mintDeposit`.
- On Bitcoin (OFF): an OP_RETURN carrying an EVM address, then `wrapDeposit`.

**Redemption.**
1. The holder burns a ticket (`Redeemed` event).
2. Anyone resolves it with a drand signature.
3. The canister calls `transferFrom` on Ethereum, then `noteSent` and `recordDelivery`.

The canister reads tickets by state calls at a pin, not from logs.

**Caps.**
- On-chain:
  - per-wrapper mints per day (H11);
  - module `maxItemsPerDay`;
  - module mint/burn hourly buckets, maxPublicEthBacked and maxEthBacked;
  - SweepVault `capCeiling` (`ETCH_CAP_CEILING`).
- In the canister, not on-chain:
  - custody caps (custodyItems/custodyWei per collection, a global of 48 items / 75 ETH at step 1);
  - outflow limits;
  - the dormancy limits (`nft-pilot.json`).

**Wake flow.**
1. Anyone posts the bond at the WakeDesk.
2. The canister, whose install argument names the desk, wakes the collection within its dormancy caps (max_active 10, 12 wakes a day, 12 h timeout) and starts posting floors.
3. It sends `markServed`.
4. The bond comes back to the requester if their coin graduates, or goes to the treasury on `forfeit`.

---

## 4. Governance summary

- **Safe** (2-of-2 at launch) is the ListingRegistry owner. Every `onlyOwner` read elsewhere goes through `registry.owner()`, and the CollectionDirectory admin is the Safe too.
- **Instant Safe powers:**
  - pausing a listing (stops wrapper mints, sweeps and draws);
  - pausing or unpausing the keeper;
  - setGuardian;
  - cancelling draws;
  - every ProtocolTreasury setter;
  - lowering a cap ceiling or a mint limit;
  - `CollectionDirectory.add`;
  - ownership transfer (two steps, no delay).
- **7-day public delays:** roles, membership root, btcVault, registry treasury, raising a cap ceiling, raising a wrapper's mint limit.
- **Guardian:** may only stop things — `pauseKeeper`, `PosterSource.pause`, `cancelDraw`.
- **The 72 h timelock** is the canister's (proposers p1/p2, guardians g1–g3 with k=2). It gates canister upgrades only.
  - Trust note (`phase1-scope.md:38-39`): an upgraded canister could mint unbacked wX at up to 3 a day.
- **Immutable:** every module parameter, the curve template and targets, lock parameters, PosterSource's poster, the AutoBuyerVault buyer, the wrapper's custodian, chain and collection, and the WakeDesk bond and window.
- **Deploy key:** after `wire*`, its only remaining power is `EthNftWrapper.proposeMintLimit`.

---

## 5. Treasury, fees and token

- **ProtocolTreasury inflows:** 20% of curve trade fees, graduation fees, FloorModule `skim` (revenue above 1 ETH, or everything once escaped), protocol-LP ETH fees, forfeited wake bonds.
- **Outflow:** at launch, 90% to reserves (distributed to modules by 7-day volume via `fundReserve` into `surplus`, which tops up conversions) and 10% to `ops`.
- **Platform token:** **none deployed.**
  - `src/Treasury.sol` and `script/DeployToken.s.sol` are the protocol-token v1 design, marked "NOT RUN" pending counsel.
  - `docs/v8/index-token.md` (rBLUE) is design only.
- **The "fueler":** I found no contract by that name. The nearest on-chain concepts are the treasury's floats and bootstrap loans (both 0) and the canister's `frog_funding` Relay settings.

---

## 6. Oracle and price inputs on-chain

- `FloorModule.postFloor` takes (price, confidence, depth) from the poster. The module's `price()` is a 24 h TWAP of its own fills, clamped to ±3% of the accepted floor.
- `SweepVault.postCap` comes from the keeper EOA.
- `LiquidityLock` reads the module's `price()` or `freshCap`.
- drand is the only randomness. No Chainlink. No ETH/USD.

---

## 7. EVM-specific assumptions that need redesign on Solana

1. **Native-ETH value flows.** `msg.value` deposits, `call{value}` pushes with `TransferFailed` reverts, `receive()` sender guards (`FrogCurve:233`, `LiquidityLock` PoolManager-only, `SweepVault:271`), and the 50k gas stipend in `WakeDesk:170`.
   - On Solana this becomes lamport/system transfers from PDA vaults, or wSOL.
   - **Big economic change:** raises would be in SOL while NFTs are priced in ETH. Floor posts, `gradPrice`, `maxItemCost` and advances need an ETH/SOL conversion (an oracle or a canister quote), and there is FX risk on every advance.
2. **Amounts and decimals.** ERC-20 uses 18 decimals with 1e18 = one item, and the curve supply is 1e27. SPL amounts are u64 (max ≈1.8e19) with ≤9 decimals, so everything must be rescaled. The u256 `mulDiv` math becomes u128 with checked math.
3. **ERC-20 approve/transferFrom pulls and allowance post-checks** (`FrogCurve:373,376`, module burn pulls). These become SPL transfers with signer or delegate, or PDA authority.
4. **Uniswap v4 singleton** (`unlock` callback, flash accounting, hookless PoolKeys, tick math). The replacement is a CPI into a Solana CLMM (Orca Whirlpools, Raydium CLMM or Meteora DLMM), with PDA-owned positions that are never withdrawn. Position NFTs must be held by a lock PDA, and fee collection must be routed. EtchRouter's best-venue routing would need re-implementation.
5. **Contracts creating contracts** (`new FrogCurve`/`CurveToken`, `new AutoBuyerVault`, WrapClaim deploying Wrapper) and Clones/CREATE2. These become PDAs plus new SPL mints in one program per role.
6. **Dual-clock `Wait`** (`src/lib/Wait.sol`): `block.timestamp` plus Arbitrum's `block.number` (= L1 blocks at 12 s), with `parentBlockSeconds` in every constructor. This becomes Clock `unix_timestamp` plus slot; Solana has no "parent chain" to anchor to.
7. **Role checks.** `msg.sender == registry.keeper()` and similar dynamic reads become account constraints that load the registry or PosterSource accounts.
8. **ReentrancyGuard and self-calls** (`LiquidityLock.seedCoinPoolSelf`, used as try/catch). CPI reentrancy is mostly impossible on Solana, but try/catch has no equivalent, so a failed CPI aborts the whole transaction. Deferred seeding needs explicit retry instructions.
9. **Unbounded storage and loops** (`_free`/`_pending` arrays, the `ProtocolTreasury.distribute` loop over collections, `coins[]`). These need account realloc limits and compute budgeting, likely pagination.
10. **drand BN254 pairing via precompile 0x08.** Solana has `alt_bn128` pairing syscalls; compute cost must be checked (~1.4M CU budget). A VRF (Switchboard/ORAO) is the alternative.
11. **Merkle proofs.** OZ keccak sorted-pair proofs can stay, since Solana has a keccak syscall.
12. **20-byte addresses throughout the canister interface.** This affects custodian/poster identity, redeem `destination` (it stays an Ethereum address, but the owner is a Solana pubkey), the EWN1 deposit claim (`uint32 4663‖wrapper(20)‖recipient(20)` must carry 32-byte pubkeys and a new chain tag), and the Bitcoin OP_RETURN recipient.
    - The canister would sign Solana transactions with threshold Ed25519, and must call Solana RPC instead of `eth_call` at a pin.
    - The "custodian has no code" check becomes "the signer is the canister's Ed25519 key".
13. **Events as indexing.** The indexer, keeper and monitor fold ~80 event types (`indexer/src/abis.ts`, `fold.ts`). Solana logs are truncatable, so this needs Anchor events plus account polling or Geyser.
14. **Bridge.** Relay 4663 → Ethereum (`advance` → `depositNative`) must become Solana → Ethereum. Relay supports Solana, but the canister's judge rules are EVM-specific.
15. **Deploy and governance plumbing.** The Safe becomes Squads. EIP-7702 owner checks, the 98,304-byte code limit, Foundry scripts and the `stack-4663.json` output all need replacing.
16. **Ethereum-side pieces that stay:** Seaport/OpenSea buying, Blur sale parsing, ERC-721 custody at the canister's ETH address, transfers to `0x…dEaD`, and the Bitcoin taproot/NUMS side.

---

## 8. Size and classification (src/, 14,146 lines across 65 files)

**Core, live in the launch (~5.3k lines):**
| Contract | Lines |
| --- | --- |
| FloorModule | 1,469 |
| LiquidityLock | 829 |
| FrogCurve | 563 |
| EthNftWrapper | 474 |
| ListingRegistry | 431 |
| WakeDesk | 290 |
| EtchRouter | 276 |
| ProtocolTreasury | 270 |
| CurveFactory | 232 |
| BN254HashToCurve | 118 |
| DrandEvmnet | 87 |
| PosterSource | 80 |
| Wait | 72 |
| CollectionDirectory | 57 |
| LiquidityAmounts | 36 |
| CurveToken | 13 |
| interfaces | ~330 |

**Bitcoin-frog stack, live:** Wrapper 640, WrapClaim 209, BtcAddress 53.

**Deployed, partly ON or inert:**
| Contract | Lines | State |
| --- | --- | --- |
| SweepVault | 742 | RAFFLE/BURN keeper-driven; YOYO off |
| AutoBuyerVault | 497 | inert |
| Distributor | 434 | only `beacon()` used |
| OfferBoard | 297 | off |
| MintDesk | 135 | off |
| FeeRouter | 117 | off |
| Launcher | 87 | off |

**Legacy, not deployed (~5.3k lines):**
- `src/homechain/*`, 15 files, about 2,456 lines (v6; `DeployHome.s.sol` is chain 1 only and never broadcast);
- `src/nft/*` apart from EthNftWrapper, about 1,934 lines (v4): NftWrapper 539, NftVaultBase 407, NftRemoteVault 316, NftRegistry 172, NftSweepVault 134, NftFeeRouter 106, SeaportGuard 102, NftLauncher 80, Seaport16 48, INft 30;
- `ItemQueue` 336 (v3, superseded by FloorModule);
- `Treasury` 94 (protocol token, not run);
- `AssetVault` 502 (v1; only the non-auto `listInscriptions` creates it, though its init code sits in the registry).

**What a Solana port must reproduce:** the floor module, curve, factory and lock/router (to be replaced by a CLMM integration), the wrapper ticket machine, the poster and pause switches, the treasury, the WakeDesk, the directory, the registry's roles and pauses, and randomness. That is about 6k lines of Solidity logic.

**Key files:**
- `src/floor/FloorModule.sol`
- `src/curve/FrogCurve.sol`
- `src/curve/CurveFactory.sol`
- `src/pools/LiquidityLock.sol`
- `src/pools/EtchRouter.sol`
- `src/nft/EthNftWrapper.sol`
- `src/Wrapper.sol`
- `src/WrapClaim.sol`
- `src/floor/PosterSource.sol`
- `src/floor/ProtocolTreasury.sol`
- `src/floor/WakeDesk.sol`
- `src/ListingRegistry.sol`
- `src/SweepVault.sol`
- `src/rand/DrandEvmnet.sol`
- `src/lib/Wait.sol`
- `script/DeployStack.s.sol`
- `script/StackChecks.sol`
- `script/Deploy.s.sol`
- `script/deploy-stack.sh`
- `script/list-auto.sh`
- `deployments/launch/nft-pilot.json`
- `docs/v7/phase1-nft.md`
- `docs/v7/golive-runbook.md`
- `docs/v6/phase1-scope.md`
