# Deploying the whole stack: order, files, the owner's steps, and the rehearsal (v6)

Written 2026-09-23 on branch `web-deploy` (from `9c6df31`), **before its code**. It extends docs/deploy/contracts-mainnet.md
(the v1/v2 go-live: registry, beacon, launcher, distributor, an autonomous-buyer listing, the NFT side) with everything that came
after it: the wrapped collection, its floor module, its curves and pools, the protocol treasury, the three settlement policies
and the offer board. Nothing here is deployed, nothing here sends to a public chain, and nothing here reads a key: the scripts
take a `--sender` (unlocked at a local node; on 4663 a hardware wallet or, since 2026-09-25, a Foundry keystore whose address is
the deployer's: docs/v6/phase1-scope.md §8.1) exactly as `script/deploy.sh` does.

Files: `script/DeployStack.s.sol` (the Foundry script), `script/StackChecks.sol` (its refusals, cheatcode-free so a test can
run them), `script/deploy-stack.sh` (the only supported way to run it), `script/golive-check.sh` (extended: it reads a stack file
back), `scripts/golive-dryrun.sh stack` (the rehearsal on a local fork), `script/StackRehearsal.s.sol` (what the rehearsal
drives: a launch, a graduation, a module mint, a sweep, a raffle draw).

## 1. What exists before this script runs

`script/deploy.sh` (registry, beacon, launcher, distributor, wired, roles proposed), `script/list-auto.sh` (an inscriptions
listing naming the canister as its buyer), and optionally `script/deploy-nft.sh` (NftRegistry + NftLauncher, wired). The stack
script reads all of it from the chain through the registry it is given and refuses if any of it is missing (§4).

**The wrapper is deployed here, through `WrapClaim`, not by `deploy-wrapclaim.sh`.** `WrapClaim` builds its `Wrapper` in its
own constructor, so that wrapper's `deployer` is the WrapClaim contract — and `Wrapper.wireFloorModule`, `initMintLimit` and
`proposeMintLimit` are the deployer's. Until 2026-09-23 that meant a WrapClaim-built wrapper could never be wired to a floor
module, and the stack deployed a bare `Wrapper` with `distribution` = the timelock as a workaround (the first as-run in §7).
Closed on branch `contract-fixes`: **`WrapClaim` forwards the three calls, from the registry's owner only** —
`WrapClaim.wireFloorModule(module)`, `initMintLimit(limit)`, `proposeMintLimit(limit)`; `acceptMintLimit` stays public on the
wrapper. The wrapper itself is unchanged: its `deployer` is still the claim contract, and a wrapper deployed directly (the tests,
`NftWrapper`) is wired by whoever deployed it, as before. The owner is read live from the registry, so a change of the registry's
owner moves this power with it, as it moves `WrapClaim.cancel`. **Corrected 2026-09-25** (docs/v6/phase1-scope.md §7 item 5): this
said such a rotation takes "seven public days, `ListingRegistry.proposeRoles`". It does not: `proposeRoles` rotates the keeper,
purchaser and attestor, and waits seven days; ownership moves by `Ownable2Step` (`transferOwnership`, then `acceptOwnership` by the
new owner) with **no delay**. The registry only refuses a new owner that some vault treats as its buyer.

Consequence for this script: the wrapper's wiring is **the owner's step**, not the deployer's. On our own chains the unlocked
owner makes it in the same run (before the rehearsal's first mint); on a public chain it is printed for the multisig with the
other owner calls (§6, steps 0a–0b), and the go-live read-back reports it as "still to be made" until it is. The wrapper's
wiring checks are the same as ever: `wireFloorModule` once, before anything was minted or burned; `initMintLimit` once, before
anything was minted — so the multisig makes them before the first `wrap`, `wrapDeposit` or module mint, or they are refused.

## 2. Dependency order

One `ProtocolTreasury` and one `CollectionDirectory` per deployment; then, per collection, in this order (each line needs the
ones above it):

| # | contract | constructor needs | wired by | wiring call |
|---|---|---|---|---|
| 0 | `ListingRegistry`, `DrandEvmnet`, `Launcher`, `Distributor` | — | the owner | `wire`, `proposeRoles` (docs/deploy/contracts-mainnet.md, already done) |
| 0' | `NftRegistry` + `NftLauncher` | registry, beacon | the owner | `wire(nftLauncher)`, `wireNft(nftRegistry)` (deploy-nft.sh, already done) |
| 1 | `ProtocolTreasury` | registry | the owner | `setOps(ops)`, `addCollection(module)` per collection |
| 2 | `CollectionDirectory` | admin = the owner | the owner | `add(factory)` per collection |
| 3 | `WrapClaim`, which builds its `Wrapper` (Bitcoin) or `NftWrapper` (an EVM collection) | registry, distributor, beacon, listing id, custodian, custody address / beacon, chain id, collection, custodian | the owner through `WrapClaim` (Bitcoin) / the deployer (`NftWrapper`) | `initMintLimit(stage mints a day)`, then `wireFloorModule(module)` after 5 |
| 4 | `MintDesk` | wrapper, payee (the canister's ETH address) | — | — |
| 5 | `FloorModule` | wrapper, keeper (= `registry.keeper()`), payee, platform = treasury, treasury, `Params` | the deployer | `wire(factory)` after 7 |
| 6 | `LiquidityLock` | PoolManager (docs/v3/pools.md §1), wrapper, layout | the deployer | `wire(factory)` after 7; `wireTreasury(treasury)` |
| 7 | `CurveFactory` | wrapper, lock (seeder), module (as `IItemQueue`), platform = treasury, the `FrogCurve.Config` template, `Targets` | the deployer | `wire(raffle, burn)` after 9, `wireYoyo(yoyo)` |
| 8 | `EtchRouter` | PoolManager, lock, module | — | — |
| 9 | `SweepVault` × 3 (RAFFLE, BURN, YOYO) | registry, the collection's listing id, wrapper, policy, venue = router, launcher = factory, desk | — | (the owner may `proposeCapCeiling`) |
| 10 | `OfferBoard` (Bitcoin collections only) | wrapper, bond, reward, slashDelay, expiry, rewardDayCap, maxOpen, maxOpenPerSubmitter | — | — |

`FrogCurve` and `CurveToken` are not deployed here: the factory creates one of each per launch (`CurveFactory._launch`), from
the template it was built with. `ItemQueue` is kept and deployable but is not in this stack: the module is the factory's
queue (docs/v5/floor-module.md §2). The v4 addresses (PoolManager `0x8366…0951`) are those measured in docs/v3/pools.md §1;
the script refuses a chain where the PoolManager has no code.

Why this order and not another: the module needs the wrapper's address (constructor) and the wrapper must be wired to the
module before its first mint; the factory needs the lock and the module; the router needs the lock and the module; the vaults
need the router (their venue must have code) and the factory (their launcher); the factory's `wire` checks each vault names it;
`module.wire(factory)` checks `factory.wrapped()`; `directory.add(factory)` checks `factory.queue().factory() == factory`, so it
comes after `module.wire`; `treasury.addCollection(module)` checks `module.treasury() == treasury`.

## 3. The addresses file

`deployments/stack-<chainid>.json`, written by the script and completed by the shell wrapper (`fromBlock`, the chain's own L2
height read before anything was sent — docs/deployments/README.md says why). It is **also a valid `WRAPPED_CONFIG`** for the
indexer (docs/v5/ux-and-reserves.md §3): point `WRAPPED_CONFIG` at it and the API serves every collection in it, and the site
reads the collection's contracts from the API. The keeper reads `forKeeper.*`.

```
{
  "chainId": 4663, "label": "stack", "fromBlock": <L2 height>, "stage": 0,
  "registry": "0x…", "owner": "0x…", "keeper": "0x…", "treasury": "0x…", "directory": "0x…", "poolManager": "0x…",
  "ownerStepsMade": false,                       // true on our own chains, where the unlocked owner made them
  "collections": [ { "slug": "frogs", "name": "…", "symbol": "wFROG", "kind": "inscriptions", "fromBlock": …,
      "wrapClaim": "0x…" (Bitcoin only: the claim that built the wrapper and forwards its wiring, §1; null for an EVM collection),
      "wrapper": "0x…", "listingId": 2, "factory": "0x…", "lock": "0x…", "router": "0x…", "queue": "0x…" (= floorModule),
      "floorModule": "0x…", "mintDesk": "0x…", "raffleVault": "0x…", "burnVault": "0x…", "yoyoVault": "0x…", "offerBoard": "0x…",
      "custody": { "kind": "bitcoin", "address": "bc1p…" } } ],
  "forKeeper": { "floorModules": ["0x…"], "sweepVaults": ["0x…", …], "offerBoards": ["0x…"] },
  "caps": { "stage": 0, "mintsPerDayPerCollection": 3, "itemsPerDayPerCollection": 3, "custodyItemsPerCollection": 3,
            "custodyEthPerCollection": "10 ETH", "note": "docs/v5/launch-gates.md G7; the canister's install argument holds the same numbers" }
}
```

A file with `label: "stack"` is never read by the site as *the* deployment (`web/src/lib/deployments.ts` `pick` keeps only an
unlabelled or `mainnet` file for that), and `web/scripts/sync-content.mjs` does not bundle it: the site sees the stack through the
indexer. On a real chain the file is never overwritten (the wrapper refuses, as `deploy.sh` does).

## 4. Refusals (`script/StackChecks.sol`, `test/StackChecks.t.sol`)

The confirm word is `deploy-etch-stack:<chainid>:<deployer, lowercase>` — a sixth word, so no other step's confirm can stand in
for this one. Then, before anything is broadcast: the chain is 4663 or ours (never 46630); the registry has code and is wired
(its distributor is set, since the wrapper reads the beacon from it); the listing exists, is not a rune, and on a public chain
is autonomous (its buyer is the canister); the custodian is an account with no code (a threshold-ECDSA EOA; on the fork the
dev accounts carry EIP-7702 code, which is why the dry run uses a code-less address) and the custody address is taproot; the
PoolManager has code; the name and symbol are given; **the stage is 0 on a public chain** (launch-gates G7: custody caps start at
the canary stage; a higher first stage is refused, and the canister's install argument must say the same); every constant in
the module's `Params` is inside the module's own constructor bounds (so the refusal is here, not a revert mid-broadcast).

## 5. Custody caps at stage 0 (G7)

The G7 table is the canister's install argument; what this stack can set on chain to match it: `Wrapper.initMintLimit(3)`
(three mints a day per collection at stage 0, the wrapper's H11 rate), `FloorModule.Params.maxItemsPerDay = 3` (item arrivals
the module pays for in 24 h), `OfferBoard.rewardDayCap = 0.2 ETH`. The module's ETH caps are basis-risk bounds, not custody
(docs/v5/floor-module.md §6.2): they stay at the frog defaults (5 items an hour each way, 10 items of public ETH backing, 40
hard), scaled by `ETCH_STACK_ITEM_UNITS` for a dearer collection. The stage and its numbers are written into the file so the
go-live check can compare them with `custody_caps()` (docs/v5/launch-gates.md, "Checked by code"). Since 2026-09-25 it does:
`golive-check.sh … deployments/stack-<chainid>.json --canister <buyer>` reads the buyer's `custody_caps()` (dfx, anonymous) and
holds each collection's cap under `btc:0x<wrapper>` to the stage's items and wei; without `--canister` it says it did not read it
(docs/v6/phase1-scope.md §8.2, K9). It also compares each listing's `btcVault` with its wrapper's `custodyAddress()`.

## 6. The owner's (timelock's) steps

On our own chains (31337, the staging fork) the owner is an unlocked account and the script makes them. On a public chain the
deployer owns nothing and the script **prints** each call's target and calldata for the multisig, in this order:

0. For a Bitcoin collection, the wrapper's wiring, through the claim contract (§1): (a) `WrapClaim.initMintLimit(mints)` with
   the stage's mints a day (§5), (b) `WrapClaim.wireFloorModule(module)`. Both must land **before the first mint** of any kind
   (the wrapper refuses them afterwards, `NotFresh`), so they are the first calls the multisig makes, and the go-live check
   reports them as still to be made until they have. `NftWrapper` is wired by the deployer in the script itself, as before.
1. `ProtocolTreasury.setOps(ops)` — `ETCH_OPS`; unset, the operations share joins the reserves pot (docs/v5/floor-module.md §13.3).
2. `ProtocolTreasury.addCollection(module)` for each collection.
3. `CollectionDirectory.add(factory)` for each collection. **Corrected 2026-09-25** (docs/v6/phase1-scope.md §7 item 8): this said
   "the canister reads `queues()` from it". No canister code reads the directory: the canister is configured by the controller's
   `set_floor_sink(module)` and `set_item_config(factory, desk, queues)` before the handover (`frognet.rs:386`). The directory is a
   public list for anyone else (`CollectionDirectory.queues()`, docs/v3/curve.md §"Discovery"); `golive-check.sh` reads it back.
4. Recommended, not required for anything to work: `SweepVault.proposeCapCeiling(ceiling)` on each of the three vaults
   (H12; `ETCH_CAP_CEILING`, suggested 1.5 × the floor), accepted by anyone after seven days.

For a Bitcoin collection nothing stays with the deploy key: `Wrapper.proposeMintLimit` is the wrapper's deployer's, and that
deployer is the claim contract, which forwards it from the registry's owner only (`WrapClaim.proposeMintLimit`: lower at once,
or propose a raise anyone accepts after a week on both clocks). One thing still does for an EVM collection: `NftWrapper` is
deployed directly by the deploy key, whose `proposeMintLimit` it keeps. If that is not acceptable, deploy the `NftWrapper` from
the timelock itself (a Safe can send a creation transaction); an `ETCH_WRAPPER_EXISTING` input for the script is not built.

## 7. The rehearsal (`scripts/golive-dryrun.sh stack`)

A local anvil fork of Robinhood Chain under **our staging chain id** on port 8651 (`STACK_PORT`), `--auto-impersonate` so the
canister's code-less address can act. Then: `deploy.sh` → `list-auto.sh` (a one-member collection whose root is the leaf of a
demo inscription, so a deposit could be attested) → `deploy-stack.sh` → `golive-check.sh` with the stack file → the drive:

1. **launch**: the creator launches a RAFFLE coin at the 1% tier; the buyer's first buy comes seconds after launch, so it pays
   the launch fee (≈ +49% at that moment, all to the vault: docs/v5/policies-and-fees.md §2); more buys fill the raise.
2. **graduation**: the keeper posts the floor at the module and the cap at the vaults; `graduate()` converts the item budget
   through the module at once (ETH-backed units), the lock seeds the coin's pool and the shared pool in the real PoolManager.
3. **module mint**: a trader mints wX for 0.2 ETH at `floor × (1 + spread)`; `quoteMint` equals what came out.
4. **sweep**: `flushVaultFees` credits the coin; `sweep(coin, spend, buyWrapped(...), minOut)` at the router buys ≥ 1 wX under
   the cap.
5. **raffle draw**: the keeper opens a draw of one whole wX with a one-leaf snapshot (the buyer holds every ticket); the
   challenge window passes on both clocks (the fork's time is moved 15 minutes and 80 blocks); `requestDraw` pins a drand
   round 20 rounds ahead of the fork's clock. The script reads that round from `api.drand.sh` (a public read; the only network
   access beside the fork's upstream RPC), `resolve`s with the real signature, `claim`s for the buyer, and checks one whole wX
   moved from the vault to the winner. If the round does not exist yet it waits and prints the countdown;
   `DRYRUN_NO_DRAND=1` stops after the pin and says so — the draw is then rehearsed to the pin only, which the summary states.

**The fork's clock.** The fork is taken from an archive read-only RPC (`STACK_FORK_RPC`, default `https://rpc.ordofi.network`
since 2026-09-24; it was `https://robinhood.drpc.org`, which that day stopped answering any state read at an explicit block
number) at a block `STACK_FORK_BACK` (default 2,000 ≈ 3.5 minutes; ordofi is load-balanced, so the block must be one every
backend holds) behind the head. The drand round a draw pins is then about 13 minutes in the future and the script waits for
it; with the archive upstream nothing expires during the wait. `STACK_FORK_BACK=13000` (≈ 22 minutes, the old default) puts
the pinned round in the past and removes the wait, where the upstream serves that deep. The plain public RPC cannot do this (it serves historical state for about fifteen minutes only, docs/staging.md) and
it is what the first runs used: forking at the head, the 16-minute drand wait outran that window, the first attempt failed the
resolve's simulation with forge's `FatalExternalError` (an upstream read the RPC no longer answered) and the second — with the
BN254 precompiles warmed by a `verify` of fixture round 1000 while the block was still served — got as far as broadcasting the
resolve, whose simulation printed the winning ticket, and then **anvil aborted** (`Abort trap: 6`) while mining it. The
precompile warm-up is kept; the archive fork at an older block is what removes both the wait and the expiry.

Nothing is sent to any public chain: every node is 127.0.0.1, every sender an anvil account or an impersonated code-less
address. The refusals (`refusals` mode) are extended with the stack's: no confirm, a confirm for another chain, a stage above 0
on a node reporting 4663, and 4663 without a hardware wallet or keystore (and, since 2026-09-25, a keystore for another address;
the deployer's keystore is accepted up to the typed chain id); `stack` mode adds the ones that need a registry (a custodian with
code, the demo rune listing, a stage past the table, a key in the environment).

**A fork that outlives its block.** The public RPC serves historical state for about fifteen minutes (docs/staging.md), and the
drand wait is longer. Anvil reads an account from upstream the first time it is touched, so the rehearsal touches the BN254
precompiles the beacon's `verify` uses right after the deployment — a `verify` of fixture round 1000 — while the fork block is
still served; the first run without it failed the resolve with forge's `FatalExternalError` (an upstream read the RPC no longer
answered), not with a bad signature.

**As run (2026-09-23, `scripts/golive-dryrun.sh stack`: 4663 forked from the archive RPC at block 70,772,872 — 22 minutes
behind the head — under chain id 9466301, port 8651): DRY RUN: ALL PASS.** `deploy.sh`, `list-auto.sh`, `deploy-stack.sh`
(17 contracts; the file written), the precompile warm-up, `golive-check.sh` 110 checks ok (the three cap-ceiling notes), the
five refusals; the drive: launch at the 1% tier, the first buy paying the 50% launch fee, the raise reached and filled by the
module at once, `graduate()` seeding the real PoolManager (10.9 ETH-backed units for the graduation), a 0.2 ETH mint equal to
its `quoteMint` (0.3249 wX), a 0.8 ETH sweep buying 1.2985 wX at the router under the cap, draw 0 opened, pinned to drand
evmnet round 20890856 (published 349 s before the pin, wall clock), resolved with that round's real signature (winning ticket
594 of 1,000), claimed for the buyer, one whole wX moved from the vault to the winner. `refusals` mode: ALL PASS with the
stack's four (4663 without a hardware wallet, 4663 at a stage above 0, the rehearsal on a node reporting 4663, 46630).

**As run again (2026-09-23, branch `contract-fixes`, after the WrapClaim passthrough of §1): DRY RUN: ALL PASS.** The wrapper
now comes from `WrapClaim` (17 → 18 contracts: the claim and its wrapper), the owner's step 0 is made by the unlocked owner,
`golive-check.sh` 116 checks ok (six new: the claim has code, `wrapClaim.wrapper`, `wrapper.distribution`, `wrapper.deployer`
are the pair, `wrapClaim.registry` and `.listingId`; the three cap-ceiling notes), the five refusals, the same drive: the 50%
launch fee at the first buy, 10.9 ETH-backed units at graduation, a 0.2 ETH mint equal to its quote, a 0.8 ETH sweep buying
1.2985 wX, draw 0 pinned to round 20891852, resolved with its real signature (winning ticket 186), one whole wX to the winner.

**A race in the harness, found on this run and fixed.** Two runs in a row failed the resolve with `InvalidSignature()` on a
signature that verified locally. `phase draw` wrote `rehearsal-pin-<chainid>.json` from forge's *simulation* of `requestDraw`;
the broadcast transaction mined a few seconds later, and on a 3-second beacon the vault pinned a later round (20891710 in the
file, 20891712 on chain). The shell now reads the pinned round back from the raffle vault (`draw(uint256)`) and prints both.

## 8. Open problems

1. ~~**WrapClaim-built wrappers cannot take a floor module**~~ — closed (§1): `WrapClaim` forwards `wireFloorModule`,
   `initMintLimit` and `proposeMintLimit` from the registry's owner; the stack deploys the wrapper through `WrapClaim` again and
   the wiring is the owner's step 0 (§6). `test/WrapClaim.t.sol` (the passthrough, its refusals, the wired module minting) and
   `test/security/WrapClaimSecurity.t.sol` (the exact ABI, 21 functions) hold it.
2. ~~**The wrapper's deployer keeps `proposeMintLimit`**~~ — closed for Bitcoin collections by the same passthrough; still true
   of a directly-deployed `NftWrapper` (§6).
3. **NFT collections**: the script deploys the same stack over an `NftWrapper` when `ETCH_NFT_COLLECTION` is set (no
   `OfferBoard`: hand-ins are Bitcoin-only, docs/v6/offer-handins.md §7.4). The NFT side's remote-buyer listing
   (`NftRegistry.listNft`) is still `deploy-nft.sh`'s demo or the owner's own call; the stack does not list.
4. **The staging fork's `/info`** (staging/chain, another package) does not yet serve the stack file; the indexer does
   (`WRAPPED_CONFIG`).
5. **The rehearsal depends on an archive RPC** (`rpc.ordofi.network` since 2026-09-24, keyless, read-only; `robinhood.drpc.org`
   no longer serves historical state) for the older fork block; with the
   plain public RPC (`STACK_FORK_RPC=https://rpc.mainnet.chain.robinhood.com STACK_FORK_BACK=0`) it forks at the head, waits
   about sixteen minutes for the drand round, and anvil has aborted at the resolve twice (§7): not a path to rely on.
6. **The rehearsal's conversion step** (the canister wrapping a bought frog into the module) is not driven: it needs a
   membership proof for the listing root, which the dry run's listing has (one member), but the mint desk / module conversion
   path is the canister's and is rehearsed in `autobuyer/` against its own fake world.
