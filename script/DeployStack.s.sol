// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {Distributor} from "../src/Distributor.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {WrapClaim} from "../src/WrapClaim.sol";
import {EthNftWrapper} from "../src/nft/EthNftWrapper.sol";
import {PosterSource} from "../src/floor/PosterSource.sol";
import {SweepVault} from "../src/SweepVault.sol";
import {MintDesk} from "../src/curve/MintDesk.sol";
import {FrogCurve} from "../src/curve/FrogCurve.sol";
import {CurveFactory} from "../src/curve/CurveFactory.sol";
import {CollectionDirectory} from "../src/curve/CollectionDirectory.sol";
import {LiquidityLock} from "../src/pools/LiquidityLock.sol";
import {EtchRouter} from "../src/pools/EtchRouter.sol";
import {FloorModule} from "../src/floor/FloorModule.sol";
import {ProtocolTreasury} from "../src/floor/ProtocolTreasury.sol";
import {WakeDesk} from "../src/floor/WakeDesk.sol";
import {OfferBoard} from "../src/handin/OfferBoard.sol";
import {IWrappedCollection} from "../src/interfaces/IWrappedCollection.sol";
import {DeployChecks} from "./DeployChecks.sol";
import {StackChecks} from "./StackChecks.sol";

/// @title DeployStack
/// @notice docs/v6/deploy-stack.md. Deploys, in dependency order, everything that came after the v1/v2 go-live: the protocol
/// treasury and the collection directory (one each), then per collection the wrapper (for a Bitcoin collection: `WrapClaim`,
/// which builds it), the mint desk, the floor module, the liquidity lock, the curve factory, the router, the three policy vaults
/// (RAFFLE, BURN, YOYO) and — for a Bitcoin collection — the offer board; wires what the deployer wires; makes the owner's calls
/// on our own chains (the wrapper's wiring through `WrapClaim` first, §6 step 0) and PRINTS them on a public one;
/// writes deployments/stack-<chainid>.json (the shell wrapper completes it: `fromBlock`, `collections` as a list).
///
///   ETCH_STACK_CONFIRM=deploy-etch-stack:<chainid>:<deployer, lowercase> ETCH_REGISTRY=0x… ETCH_WRAPPER_LISTING=<id> \
///   ETCH_WRAPPER_NAME="Wrapped Bitcoin Frog" ETCH_WRAPPER_SYMBOL=wFROG ETCH_WRAPPER_CUSTODIAN=<canister evm_address()> \
///   ETCH_WRAPPER_CUSTODY_ADDRESS=<its custody_address()> ETCH_CAP_CEILING=<wei> [ETCH_STAGE=0] [ETCH_OPS=0x…] [ETCH_PAYEE=…] \
///   [ETCH_GUARDIAN=0x…] \
///   [ETCH_NFT_CHAIN_ID=1 ETCH_NFT_COLLECTION=0x… ETCH_NFT_NAME=… ETCH_NFT_SYMBOL=… ETCH_NFT_LISTING=<id>] [ETCH_STACK_KIND=nft] \
///   [ETCH_NFT_COLLECTIONS=0x…,0x… ETCH_NFT_NAMES=…,… ETCH_NFT_SYMBOLS=…,… ETCH_NFT_SLUGS=…,…]   (several collections: D1) \
///   [ETCH_NFT_NORMAL_RAISES=<wei>,… ETCH_NFT_EARLY_RAISES=<wei>,… ETCH_NFT_CONV_BPS_LIST=…,… ETCH_NFT_RESERVE_BPS_LIST=…,…] \
///   [ETCH_WAKE_BOND=<wei> [ETCH_WAKE_WINDOW=<seconds>]]   (docs/v7/dormant-collections.md §3.3: the WakeDesk) \
///   script/deploy-stack.sh [<rpc>] <deployer> [--ledger|--trezor]
/// docs/v7/phase1-nft.md §3.4: an EVM collection is an `EthNftWrapper` whose module reads its keeper from a `PosterSource` naming the
/// canister (the custodian) — the canister posts that module's floor; the registry's pause still zeroes it. `ETCH_STACK_KIND=nft`
/// deploys the EVM collection alone (Phase 1 on Ethereum NFTs): no WrapClaim, no Bitcoin wrapper, no offer board.
/// docs/v7/phase1-nft-pooled.md §2: each EVM collection has its own graduation targets and its module's `convBps` / `reserveBps`:
/// `ETCH_NFT_NORMAL_RAISES`, `ETCH_NFT_EARLY_RAISES`, `ETCH_NFT_CONV_BPS_LIST`, `ETCH_NFT_RESERVE_BPS_LIST` (comma lists in the order
/// of `ETCH_NFT_COLLECTIONS`; one collection: `ETCH_NFT_NORMAL_RAISE` …). Unset: the frog stack's defaults. Each collection is
/// bounded by `StackChecks.checkNftCollection` before anything of it is deployed.
/// Nothing here reads a private key. Every contract is created from its compiled artifact (`vm.getCode`) so this script's own
/// bytecode stays small; every immutable is read back before it is written down.
/// docs/audit/2026-09-26/fixes-B-C-keeper-draws.md §4: the module reads its keeper from the registry (no key is bound in it);
/// `ETCH_CAP_CEILING` is required on a real chain (a vault takes no cap without a ceiling, and the owner's first ceiling applies
/// at once); `ETCH_GUARDIAN` names the key that may pause the keeper and cancel unpinned draws.
contract DeployStack is Script {
    address constant POOL_MANAGER = 0x8366a39CC670B4001A1121B8F6A443A643e40951;   // docs/v3/pools.md §1, measured on 4663
    uint256 constant DESK_TTL = 6 hours;
    /// Our own chains only: the rehearsal's ceiling when ETCH_CAP_CEILING is unset (the rehearsal's cap is 0.66 ETH).
    uint256 constant LOCAL_CAP_CEILING = 1 ether;

    struct Roles { address deployer; address owner; address keeper; address custodian; address payee; }
    struct Collection {
        string slug; string name; string symbol; bool nft; uint256 listingId; string custody; uint64 nftChainId; address nftCollection;
        address claim; address wrapper; address keeperSource; address desk; address module; address lock; address factory; address router;
        address raffle; address burn; address yoyo; address board;
        // docs/v7/phase1-nft-pooled.md §2: per collection — the factory's targets and the module's conversion and reserve
        CurveFactory.Targets targets; uint16 convBps; uint16 reserveBps;
    }

    Roles r;
    address[] nftCols;
    string[] nftNames;
    string[] nftSymbols;
    string[] nftSlugs;
    address treasury;
    address directory;
    address wakeDesk;
    uint256 wakeBond;
    uint256 pbs;
    uint256 stage;
    bool local;
    bool ownerSteps;
    string ownerCalls;   // what the multisig must send, accumulated for the log
    ListingRegistry _reg; // for the file's per-collection reads

    function run() external {
        r.deployer = msg.sender;
        ListingRegistry registry = ListingRegistry(vm.envAddress("ETCH_REGISTRY"));
        uint256 listingId = vm.envUint("ETCH_WRAPPER_LISTING");
        stage = vm.envOr("ETCH_STAGE", uint256(0));
        StackChecks.Plan memory p;
        p.chainId = block.chainid;
        p.deployer = r.deployer;
        p.confirm = vm.envOr("ETCH_STACK_CONFIRM", string(""));
        p.registry = address(registry);
        p.registryHasCode = address(registry).code.length != 0;
        p.listingId = listingId;
        p.custodian = vm.envOr("ETCH_WRAPPER_CUSTODIAN", address(0));
        p.custodianHasCode = p.custodian.code.length != 0;
        p.custodyAddress = vm.envOr("ETCH_WRAPPER_CUSTODY_ADDRESS", string(""));
        p.name = vm.envOr("ETCH_WRAPPER_NAME", string(""));
        p.symbol = vm.envOr("ETCH_WRAPPER_SYMBOL", string(""));
        p.poolManagerHasCode = POOL_MANAGER.code.length != 0;
        p.stage = stage;
        // docs/v7/phase1-nft.md §9 D1: several Ethereum collections at once (comma lists), or one (the single-collection variables)
        nftCols = vm.envOr("ETCH_NFT_COLLECTIONS", ",", new address[](0));
        if (nftCols.length == 0 && vm.envOr("ETCH_NFT_COLLECTION", address(0)) != address(0)) {
            nftCols = new address[](1);
            nftCols[0] = vm.envAddress("ETCH_NFT_COLLECTION");
            nftNames = new string[](1); nftNames[0] = vm.envString("ETCH_NFT_NAME");
            nftSymbols = new string[](1); nftSymbols[0] = vm.envString("ETCH_NFT_SYMBOL");
            nftSlugs = new string[](1); nftSlugs[0] = vm.envOr("ETCH_NFT_SLUG", string("nft"));
        } else if (nftCols.length != 0) {
            nftNames = vm.envString("ETCH_NFT_NAMES", ",");
            nftSymbols = vm.envString("ETCH_NFT_SYMBOLS", ",");
            nftSlugs = vm.envString("ETCH_NFT_SLUGS", ",");
        }
        if (nftCols.length != 0) {
            StackChecks.checkNftSet(nftCols, nftNames.length, nftSymbols.length, nftSlugs.length);
            StackChecks.checkNftSlugs(nftSlugs);
        }
        p.nftCollection = nftCols.length != 0 ? nftCols[0] : address(0);
        p.nftChainId = uint64(vm.envOr("ETCH_NFT_CHAIN_ID", uint256(0)));
        p.nft = p.nftCollection != address(0) || p.nftChainId != 0;
        p.nftOnly = keccak256(bytes(vm.envOr("ETCH_STACK_KIND", string("btc")))) == keccak256("nft");
        string memory ref;
        if (p.registryHasCode) {
            p.registryDistributor = registry.distributor();
            if (listingId < registry.count()) {
                ListingRegistry.Listing memory l = registry.listing(listingId);
                p.listingIsRune = l.kind == ListingRegistry.Kind.Rune;
                p.listingAutonomous = l.autonomous;
                ref = l.ref;
            }
        }
        StackChecks.check(p);          // every refusal happens here, before anything is broadcast
        uint256 unit = vm.envOr("ETCH_STACK_ITEM_UNITS", uint256(1e18));
        FloorModule.Params memory params = StackChecks.defaultParams(unit, stage);
        StackChecks.checkParams(params);

        string memory file = StackChecks.fileFor(block.chainid);
        local = DeployChecks.isOurs(block.chainid);
        // REHEARSAL ONLY (scripts/golive-dryrun.sh stack; fixes-W2-monitor.md §3): on our own chains, leave the owner's steps
        // unmade and print them, as on 4663, so the go-live check can be shown to FAIL a stack whose step 7 was skipped.
        ownerSteps = local && !vm.envOr("ETCH_REHEARSE_UNMADE_OWNER_STEPS", false);
        if (!local) require(!vm.exists(file), "a stack file is never overwritten on a real chain");
        pbs = vm.envOr("ETCH_PARENT_BLOCK_SECONDS", DeployChecks.parentBlockSeconds(block.chainid));
        require(pbs != 0, "unknown chain: set ETCH_PARENT_BLOCK_SECONDS to its measured parent block time");
        uint256 ceiling = vm.envOr("ETCH_CAP_CEILING", uint256(0));
        if (ceiling == 0) {
            require(local, "ETCH_CAP_CEILING is required: the vaults take no cap without a ceiling (fixes-B-C-keeper-draws.md C-01)");
            ceiling = LOCAL_CAP_CEILING;
        }

        r.owner = registry.owner();
        r.keeper = registry.keeper();
        r.custodian = p.custodian;
        r.payee = vm.envOr("ETCH_PAYEE", p.custodian);
        require(r.keeper != address(0), "the registry has no keeper yet: proposeRoles first");

        // ---- 1, 2: one treasury, one directory ----
        vm.startBroadcast(r.deployer);
        treasury = _create("ProtocolTreasury.sol:ProtocolTreasury", abi.encode(registry));
        directory = _create("CollectionDirectory.sol:CollectionDirectory", abi.encode(r.owner));
        vm.stopBroadcast();
        require(address(ProtocolTreasury(payable(treasury)).registry()) == address(registry), "read-back: treasury");
        require(CollectionDirectory(directory).admin() == r.owner, "read-back: directory");

        // ---- 3–10, the Bitcoin collection (not in an NFT-only stack) ----
        Collection memory btc;
        if (!p.nftOnly) {
            btc.slug = vm.envOr("ETCH_STACK_SLUG", ref);
            btc.name = p.name; btc.symbol = p.symbol; btc.listingId = listingId; btc.custody = p.custodyAddress;
            btc.targets = _targets(); btc.convBps = params.convBps; btc.reserveBps = params.reserveBps;
            // docs/v6/deploy-stack.md §1: WrapClaim builds the wrapper and is its `deployer`; the wrapper's wiring is the owner's,
            // through the claim (step 0 of §6), so it is NOT made here by the deploy key
            vm.startBroadcast(r.deployer);
            btc.claim = _create("WrapClaim.sol:WrapClaim", abi.encode(registry, Distributor(p.registryDistributor), Distributor(p.registryDistributor).beacon(),
                listingId, r.custodian, p.custodyAddress, p.name, p.symbol, pbs));
            vm.stopBroadcast();
            {
                WrapClaim c = WrapClaim(btc.claim);
                Wrapper w = c.wrapper();
                btc.wrapper = address(w);
                require(address(c.registry()) == address(registry) && c.listingId() == listingId && c.parentBlockSeconds() == pbs, "read-back: claim");
                require(address(w.registry()) == address(registry) && address(w.distributor()) == p.registryDistributor && w.listingId() == listingId
                    && w.distribution() == btc.claim && w.custodian() == r.custodian && w.totalSupply() == 0 && w.deployer() == btc.claim
                    && w.floorModule() == address(0) && keccak256(bytes(w.custodyAddress())) == keccak256(bytes(p.custodyAddress)), "read-back: wrapper");
            }
            _collection(btc, params, registry);
        }

        // ---- the same stack over each EVM collection named (D1: several at once), each its own wrapper, poster and module ----
        Collection[] memory nfts = new Collection[](p.nft ? nftCols.length : 0);
        for (uint256 i = 0; i < nfts.length; ++i) nfts[i] = _nftCollection(i, p.nftChainId, listingId, registry, params, p.registryDistributor);

        // ---- docs/v7/dormant-collections.md §3.3: the WakeDesk, bonded wake requests for the dormant collections (a bond named) ----
        wakeBond = vm.envOr("ETCH_WAKE_BOND", uint256(0));
        if (wakeBond != 0) {
            require(nfts.length != 0, "ETCH_WAKE_BOND: a wake desk serves Ethereum NFT collections, and this stack has none");
            uint256 wakeWindow = vm.envOr("ETCH_WAKE_WINDOW", uint256(3 days));
            vm.startBroadcast(r.deployer);
            wakeDesk = _create("WakeDesk.sol:WakeDesk", abi.encode(directory, treasury, wakeBond, wakeWindow, pbs));
            vm.stopBroadcast();
            WakeDesk d = WakeDesk(wakeDesk);
            require(address(d.directory()) == directory && d.treasury() == treasury && d.bond() == wakeBond && d.window() == wakeWindow
                && d.parentBlockSeconds() == pbs, "read-back: wake desk");
        }

        // ---- the owner's steps ----
        address ops = vm.envOr("ETCH_OPS", address(0));
        address guardian = vm.envOr("ETCH_GUARDIAN", address(0));
        if (ownerSteps) {
            vm.startBroadcast(r.owner);
            if (ops != address(0)) ProtocolTreasury(payable(treasury)).setOps(ops);
            if (guardian != address(0) && registry.guardian() != guardian) registry.setGuardian(guardian);
            if (!p.nftOnly) _ownerSteps(btc, ceiling);
            for (uint256 i = 0; i < nfts.length; ++i) _ownerSteps(nfts[i], ceiling);
            vm.stopBroadcast();
            if (!p.nftOnly) {
                require(ProtocolTreasury(payable(treasury)).isCollection(btc.module) && CollectionDirectory(directory).indexOf(btc.wrapper) == 1, "read-back: owner steps");
                require(Wrapper(btc.wrapper).floorModule() == btc.module && Wrapper(btc.wrapper).maxMintsPerDay() == StackChecks.stageOf(stage).mintsPerDay,
                    "read-back: the wrapper's wiring through the claim");
                require(SweepVault(payable(btc.raffle)).capCeiling() == ceiling && SweepVault(payable(btc.burn)).capCeiling() == ceiling,
                    "read-back: the cap ceilings are active");
            } else {
                _readBackNftOwnerSteps(nfts, ceiling);
            }
        } else {
            console2.log("The deployer owns nothing. The owner (multisig) must now make these calls, in this order:");
            if (!p.nftOnly) {
                console2.log("  (the first two, on the claim contract, must land BEFORE the first mint of any kind: the wrapper refuses them afterwards)");
                _printWrapperSteps(btc);
            }
            if (ops != address(0)) _print("ProtocolTreasury.setOps(ops)", treasury, abi.encodeCall(ProtocolTreasury.setOps, (ops)));
            else console2.log("  (ETCH_OPS unset: the operations share joins the reserves pot until setOps is called)");
            if (guardian != address(0)) _print("ListingRegistry.setGuardian(guardian)", address(registry), abi.encodeCall(ListingRegistry.setGuardian, (guardian)));
            else console2.log("  (ETCH_GUARDIAN unset: only the Safe can pause the keeper and cancel unpinned draws)");
            if (!p.nftOnly) _printOwnerSteps(btc, ceiling);
            for (uint256 i = 0; i < nfts.length; ++i) _printOwnerSteps(nfts[i], ceiling);
        }

        // ---- the file ----
        string memory root = "stack";
        vm.serializeUint(root, "chainId", block.chainid);
        vm.serializeString(root, "label", "stack");
        vm.serializeUint(root, "deployedAtBlock", block.number);
        vm.serializeUint(root, "stage", stage);
        vm.serializeAddress(root, "registry", address(registry));
        vm.serializeAddress(root, "owner", r.owner);
        vm.serializeAddress(root, "keeper", r.keeper);
        vm.serializeAddress(root, "custodian", r.custodian);
        vm.serializeAddress(root, "payee", r.payee);          // where itemArrived and the mint desk pay (ETCH_PAYEE, else the custodian)
        vm.serializeAddress(root, "treasury", treasury);
        vm.serializeAddress(root, "directory", directory);
        if (wakeDesk != address(0)) {
            vm.serializeAddress(root, "wakeDesk", wakeDesk);
            vm.serializeUint(root, "wakeBond", wakeBond);
        }
        vm.serializeAddress(root, "poolManager", POOL_MANAGER);
        vm.serializeAddress(root, "ops", ops);
        vm.serializeAddress(root, "guardian", guardian);
        vm.serializeUint(root, "capCeiling", ceiling);
        vm.serializeBool(root, "ownerStepsMade", ownerSteps);
        vm.serializeString(root, "caps", _capsJson());
        _reg = registry;
        string memory cols;                                  // the "cols" object, keyed by slug; the shell turns it into a list
        if (!p.nftOnly) cols = _collectionJson(btc);
        for (uint256 i = 0; i < nfts.length; ++i) cols = _collectionJson(nfts[i]);
        string memory json = vm.serializeString(root, "collections", cols);
        vm.writeJson(json, file);
        console2.log("treasury ", treasury);
        console2.log("directory", directory);
        if (wakeDesk != address(0)) console2.log("wakeDesk ", wakeDesk);
        if (!p.nftOnly) {
            console2.log("wrapClaim", btc.claim);
            console2.log("wrapper  ", btc.wrapper, "->", file);
        }
        for (uint256 i = 0; i < nfts.length; ++i) console2.log("nft wrapper", nfts[i].wrapper, "poster source", nfts[i].keeperSource);
    }

    /// One EVM collection's wrapper and poster source, then its steps 4–10 (docs/v7/phase1-nft.md §3.4; D1: called per collection).
    function _nftCollection(uint256 i, uint64 chainId, uint256 listingId, ListingRegistry registry, FloorModule.Params memory params, address distributor)
        internal returns (Collection memory nft)
    {
        nft.nft = true; nft.nftChainId = chainId; nft.nftCollection = nftCols[i];
        nft.name = nftNames[i]; nft.symbol = nftSymbols[i]; nft.slug = nftSlugs[i];
        nft.listingId = vm.envOr("ETCH_NFT_LISTING", listingId);      // the ListingRegistry listing whose pause flag its vaults read
        // docs/v7/phase1-nft-pooled.md §2: this collection's own graduation targets and module conversion / reserve, bounded
        nft.targets = _targets();
        nft.targets.normalRaise = _perCol("ETCH_NFT_NORMAL_RAISES", "ETCH_NFT_NORMAL_RAISE", i, nft.targets.normalRaise);
        nft.targets.earlyRaise = _perCol("ETCH_NFT_EARLY_RAISES", "ETCH_NFT_EARLY_RAISE", i, nft.targets.earlyRaise);
        nft.convBps = _bps(_perCol("ETCH_NFT_CONV_BPS_LIST", "ETCH_NFT_CONV_BPS", i, params.convBps));
        nft.reserveBps = _bps(_perCol("ETCH_NFT_RESERVE_BPS_LIST", "ETCH_NFT_RESERVE_BPS", i, params.reserveBps));
        StackChecks.checkNftCollection(block.chainid, chainId, nft.nftCollection, nft.targets, nft.convBps, nft.reserveBps);
        vm.startBroadcast(r.deployer);
        nft.wrapper = _create("EthNftWrapper.sol:EthNftWrapper", abi.encode(registry, nft.listingId, Distributor(distributor).beacon(),
            chainId, nft.nftCollection, r.custodian, nft.name, nft.symbol, pbs));
        // the canister posts this collection's floor (docs/v7/phase1-nft.md §3.2); the registry's pause zeroes it
        nft.keeperSource = _create("PosterSource.sol:PosterSource", abi.encode(registry, r.custodian));
        vm.stopBroadcast();
        EthNftWrapper nw = EthNftWrapper(nft.wrapper);
        require(nw.chainId() == chainId && nw.collection() == nft.nftCollection && nw.custodian() == r.custodian && nw.deployer() == r.deployer
            && address(nw.registry()) == address(registry) && nw.listingId() == nft.listingId && nw.parentBlockSeconds() == pbs, "read-back: nft wrapper");
        PosterSource ps = PosterSource(nft.keeperSource);
        require(address(ps.registry()) == address(registry) && ps.poster() == r.custodian, "read-back: poster source");
        _collection(nft, params, registry);
    }

    function _readBackNftOwnerSteps(Collection[] memory nfts, uint256 ceiling) internal view {
        for (uint256 i = 0; i < nfts.length; ++i) {
            require(ProtocolTreasury(payable(treasury)).isCollection(nfts[i].module) && CollectionDirectory(directory).indexOf(nfts[i].wrapper) == i + 1,
                "read-back: owner steps");
            require(SweepVault(payable(nfts[i].raffle)).capCeiling() == ceiling && SweepVault(payable(nfts[i].burn)).capCeiling() == ceiling,
                "read-back: the cap ceilings are active");
        }
    }

    /// Steps 4–10 for one collection, each read back before the next depends on it.
    function _collection(Collection memory c, FloorModule.Params memory defaults, ListingRegistry registry) internal {
        IWrappedCollection w = IWrappedCollection(c.wrapper);
        FloorModule.Params memory params = defaults;
        params.convBps = c.convBps;
        params.reserveBps = c.reserveBps;
        StackChecks.checkParams(params);
        vm.startBroadcast(r.deployer);
        c.desk = _create("MintDesk.sol:MintDesk", abi.encode(w, r.payee, DESK_TTL, pbs));
        if (c.keeperSource == address(0)) c.keeperSource = address(registry);
        c.module = _create("FloorModule.sol:FloorModule", abi.encode(w, c.keeperSource, r.payee, treasury, treasury, params, pbs));
        if (c.nft) {
            // the EthNftWrapper was deployed directly, so the deploy key is its deployer and wires it here; a Bitcoin collection's
            // wrapper is wired by the owner through WrapClaim (`_ownerSteps`)
            EthNftWrapper(c.wrapper).wireFloorModule(c.module);
            EthNftWrapper(c.wrapper).initMintLimit(StackChecks.stageOf(stage).mintsPerDay);
        }
        c.lock = _create("LiquidityLock.sol:LiquidityLock", abi.encode(POOL_MANAGER, w, _layout(), pbs));
        c.factory = _create("CurveFactory.sol:CurveFactory", abi.encode(w, c.lock, c.module, treasury, _template(), c.targets, pbs));
        c.router = _create("EtchRouter.sol:EtchRouter", abi.encode(POOL_MANAGER, c.lock, c.module));
        c.raffle = _create("SweepVault.sol:SweepVault", abi.encode(registry, c.listingId, w, uint8(SweepVault.Policy.RAFFLE), c.router, c.factory, c.desk, pbs));
        c.burn = _create("SweepVault.sol:SweepVault", abi.encode(registry, c.listingId, w, uint8(SweepVault.Policy.BURN), c.router, c.factory, c.desk, pbs));
        c.yoyo = _create("SweepVault.sol:SweepVault", abi.encode(registry, c.listingId, w, uint8(SweepVault.Policy.YOYO), c.router, c.factory, c.desk, pbs));
        CurveFactory(c.factory).wire(SweepVault(payable(c.raffle)), SweepVault(payable(c.burn)));
        CurveFactory(c.factory).wireYoyo(SweepVault(payable(c.yoyo)));
        FloorModule(payable(c.module)).wire(CurveFactory(c.factory));
        LiquidityLock(payable(c.lock)).wire(CurveFactory(c.factory));
        LiquidityLock(payable(c.lock)).wireTreasury(treasury);
        if (!c.nft) {
            // docs/v6/offer-handins.md §2: the suggested 4663 parameters; hand-ins are Bitcoin-only
            c.board = _create("OfferBoard.sol:OfferBoard", abi.encode(w, 0.01 ether, 0.01 ether, 1 hours, 24 hours, 0.2 ether, 64, 4, pbs));
        }
        vm.stopBroadcast();
        _readBack(c, registry);
    }

    function _readBack(Collection memory c, ListingRegistry registry) internal view {
        FloorModule m = FloorModule(payable(c.module));
        CurveFactory f = CurveFactory(c.factory);
        LiquidityLock l = LiquidityLock(payable(c.lock));
        if (c.nft) require(EthNftWrapper(c.wrapper).floorModule() == c.module && EthNftWrapper(c.wrapper).maxMintsPerDay() == StackChecks.stageOf(stage).mintsPerDay,
            "read-back: wrapper.floorModule");
        else require(WrapClaim(c.claim).wrapper() == Wrapper(c.wrapper) && Wrapper(c.wrapper).floorModule() == address(0), "read-back: claim.wrapper");
        require(address(MintDesk(c.desk).wrapper()) == c.wrapper && MintDesk(c.desk).payee() == r.payee, "read-back: desk");
        require(address(m.wrapper()) == c.wrapper && address(m.registry()) == c.keeperSource && m.keeper() == (c.nft ? r.custodian : r.keeper) && m.payee() == r.payee && m.platform() == treasury && m.treasury() == treasury
            && address(m.factory()) == c.factory && address(m.seeder()) == c.lock && m.parentBlockSeconds() == pbs, "read-back: module");
        require(address(l.poolManager()) == POOL_MANAGER && address(l.wrapped()) == c.wrapper && address(l.factory()) == c.factory && l.treasury() == treasury, "read-back: lock");
        require(m.convBps() == c.convBps && m.reserveBps() == c.reserveBps, "read-back: module conversion and reserve");
        require(f.normalRaise() == c.targets.normalRaise && f.earlyRaise() == c.targets.earlyRaise && f.earlyUntilEth() == c.targets.earlyUntilEth
            && f.maxRaise() == c.targets.maxRaise, "read-back: factory targets");
        require(f.wrapped() == c.wrapper && address(f.seeder()) == c.lock && address(f.queue()) == c.module && f.platform() == treasury
            && address(f.raffleVault()) == c.raffle && address(f.burnVault()) == c.burn && address(f.yoyoVault()) == c.yoyo, "read-back: factory");
        EtchRouter rt = EtchRouter(payable(c.router));
        require(address(rt.poolManager()) == POOL_MANAGER && address(rt.lock()) == c.lock && address(rt.module()) == c.module, "read-back: router");
        address[3] memory vs = [c.raffle, c.burn, c.yoyo];
        for (uint256 i = 0; i < 3; ++i) {
            SweepVault v = SweepVault(payable(vs[i]));
            require(address(v.registry()) == address(registry) && v.listingId() == c.listingId && address(v.wrapper()) == c.wrapper && uint8(v.policy()) == i
                && v.venue() == c.router && address(v.launcher()) == c.factory && address(v.desk()) == c.desk, "read-back: vault");
        }
        if (!c.nft) {
            OfferBoard b = OfferBoard(payable(c.board));
            require(address(b.wrapper()) == c.wrapper && b.custodian() == r.custodian && b.bond() == 0.01 ether && b.reward() == 0.01 ether, "read-back: board");
        }
    }

    function _ownerSteps(Collection memory c, uint256 ceiling) internal {
        if (!c.nft) {
            // §6 step 0: the wrapper's wiring, through the claim contract, before anything can have been minted
            WrapClaim(c.claim).initMintLimit(StackChecks.stageOf(stage).mintsPerDay);
            WrapClaim(c.claim).wireFloorModule(c.module);
        }
        ProtocolTreasury(payable(treasury)).addCollection(FloorModule(payable(c.module)));
        CollectionDirectory(directory).add(CurveFactory(c.factory));
        // the first ceiling applies at once (it only tightens an unbounded vault): no acceptCapCeiling step to forget
        SweepVault(payable(c.raffle)).proposeCapCeiling(ceiling);
        SweepVault(payable(c.burn)).proposeCapCeiling(ceiling);
        SweepVault(payable(c.yoyo)).proposeCapCeiling(ceiling);
    }

    function _printWrapperSteps(Collection memory c) internal view {
        uint256 mints = StackChecks.stageOf(stage).mintsPerDay;
        _print(string.concat("WrapClaim.initMintLimit(", vm.toString(mints), ") [", c.slug, "]"), c.claim, abi.encodeCall(WrapClaim.initMintLimit, (mints)));
        _print(string.concat("WrapClaim.wireFloorModule(module) [", c.slug, "]"), c.claim, abi.encodeCall(WrapClaim.wireFloorModule, (c.module)));
    }

    function _printOwnerSteps(Collection memory c, uint256 ceiling) internal view {
        _print(string.concat("ProtocolTreasury.addCollection(module) [", c.slug, "]"), treasury, abi.encodeCall(ProtocolTreasury.addCollection, (FloorModule(payable(c.module)))));
        _print(string.concat("CollectionDirectory.add(factory) [", c.slug, "]"), directory, abi.encodeCall(CollectionDirectory.add, (CurveFactory(c.factory))));
        console2.log("  (the first ceiling applies at once; until it lands the keeper's caps are refused)");
        _print("SweepVault(raffle).proposeCapCeiling", c.raffle, abi.encodeCall(SweepVault.proposeCapCeiling, (ceiling)));
        _print("SweepVault(burn).proposeCapCeiling", c.burn, abi.encodeCall(SweepVault.proposeCapCeiling, (ceiling)));
        _print("SweepVault(yoyo).proposeCapCeiling", c.yoyo, abi.encodeCall(SweepVault.proposeCapCeiling, (ceiling)));
    }

    function _print(string memory what, address to, bytes memory data) internal pure {
        console2.log("  ", what);
        console2.log("    to:", to);
        console2.logBytes(data);
    }

    function _collectionJson(Collection memory c) internal returns (string memory) {
        string memory k = string.concat("col-", c.slug);
        vm.serializeString(k, "slug", c.slug);
        vm.serializeString(k, "name", c.name);
        vm.serializeString(k, "symbol", c.symbol);
        vm.serializeString(k, "kind", c.nft ? "nft" : "inscriptions");
        vm.serializeUint(k, "listingId", c.listingId);
        // what golive-check must not assume: the wrapper's unit and whether the listing is paused, read from the chain
        vm.serializeUint(k, "itemUnits", IWrappedCollection(c.wrapper).ONE_ITEM());
        vm.serializeBool(k, "listingPaused", _reg.isPaused(c.listingId));
        vm.serializeAddress(k, "wrapClaim", c.claim);
        vm.serializeAddress(k, "wrapper", c.wrapper);
        vm.serializeAddress(k, "factory", c.factory);
        vm.serializeAddress(k, "lock", c.lock);
        vm.serializeAddress(k, "router", c.router);
        vm.serializeAddress(k, "queue", c.module);
        vm.serializeAddress(k, "floorModule", c.module);
        vm.serializeAddress(k, "mintDesk", c.desk);
        vm.serializeAddress(k, "raffleVault", c.raffle);
        vm.serializeAddress(k, "burnVault", c.burn);
        vm.serializeAddress(k, "yoyoVault", c.yoyo);
        vm.serializeAddress(k, "offerBoard", c.board);
        string memory ck = string.concat("custody-", c.slug);
        string memory custody;
        if (c.nft) {
            vm.serializeString(ck, "kind", "evm");
            vm.serializeUint(ck, "chainId", c.nftChainId);
            vm.serializeAddress(ck, "collection", c.nftCollection);
            custody = vm.serializeAddress(ck, "custodian", r.custodian);
            vm.serializeAddress(k, "floorPoster", r.custodian);
            vm.serializeAddress(k, "posterSource", c.keeperSource);
        } else {
            vm.serializeString(ck, "kind", "bitcoin");
            custody = vm.serializeString(ck, "address", c.custody);
        }
        string memory tk = string.concat("targets-", c.slug);
        vm.serializeUint(tk, "normalRaise", c.targets.normalRaise);
        vm.serializeUint(tk, "earlyRaise", c.targets.earlyRaise);
        vm.serializeUint(tk, "earlyUntilEth", c.targets.earlyUntilEth);
        string memory targets = vm.serializeUint(tk, "maxRaise", c.targets.maxRaise);
        string memory mk = string.concat("mparams-", c.slug);
        vm.serializeUint(mk, "convBps", c.convBps);
        string memory mparams = vm.serializeUint(mk, "reserveBps", c.reserveBps);
        vm.serializeString(k, "targets", targets);
        vm.serializeString(k, "moduleParams", mparams);
        string memory json = vm.serializeString(k, "custody", custody);
        return vm.serializeString("cols", c.slug, json);
    }

    function _capsJson() internal returns (string memory) {
        StackChecks.Stage memory s = StackChecks.stageOf(stage);
        string memory k = "caps";
        vm.serializeUint(k, "stage", stage);
        vm.serializeUint(k, "mintsPerDayPerCollection", s.mintsPerDay);
        vm.serializeUint(k, "itemsPerDayPerCollection", s.itemsPerDay);
        vm.serializeUint(k, "custodyItemsPerCollection", s.custodyItems);
        vm.serializeUint(k, "custodyWeiPerCollection", s.custodyEth);
        vm.serializeUint(k, "timelockDelayHours", s.timelockDelayHours);
        return vm.serializeString(k, "note", "docs/v5/launch-gates.md G7: the canister's custody_caps() must say the same stage");
    }

    /// The creation code from the compiled artifact plus the constructor's arguments, created from the broadcasting sender.
    function _create(string memory artifact, bytes memory args) internal returns (address a) {
        bytes memory code = bytes.concat(vm.getCode(artifact), args);
        assembly { a := create(0, add(code, 0x20), mload(code)) }
        require(a != address(0), string.concat("create failed: ", artifact));
    }

    // ---- the constants of docs/v3/pools.md §3 and docs/v5/policies-and-fees.md, as the tests hold them ----

    function _layout() internal pure returns (LiquidityLock.Params memory p) {
        p.sharedFee = 3000; p.sharedSpacing = 60; p.coinSpacing = 200;
        p.nearHalf = 960; p.wideHalf = 2640; p.bidWidth = 1920;
        p.nearBps = 5000; p.wideBps = 3000; p.maxDeviation = 1000; p.maxShift = 960; p.minMove = 480; p.moveEvery = 1 days;
    }

    /// The 40 / 30 / 5 / 5 / 20 split of docs/v5/floor-module.md §13.2, the 1% default tier, the decaying launch fee of §2.
    function _template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = 1_000_000_000e18; c.reserved = 200_000_000e18;
        c.platformFeeCap = 0.25 ether; c.platformBps = 500; c.poolEthBps = 1_260; c.feeBps = 100;
        c.vaultShareBps = 4_000; c.creatorShareBps = 3_000; c.reserveShareBps = 500; c.liquidityShareBps = 500;
        c.window = 3 days; c.launchFeeBps = 5_000; c.launchFeeDecay = 10 minutes;
    }

    /// Audit 2026-09-27-nft round 2 (A2-I-2): a basis-point value is bounded BEFORE the cast (65,836 would otherwise read as 300 and
    /// pass the bounds check).
    function _bps(uint256 v) internal pure returns (uint16) {
        require(v <= type(uint16).max, "a basis-point value over 65,535");
        return uint16(v);
    }

    /// docs/v7/phase1-nft-pooled.md §2: collection `i`'s value from a comma list in `ETCH_NFT_COLLECTIONS` order (every entry
    /// named, or the list unset), else the single-collection variable, else the default. Round 2 (A2-I-2): a one-collection variable
    /// never applies to several collections (a value meant for one would land on all five); golive-check --launch then holds the
    /// result against deployments/launch/nft-pilot.json.
    function _perCol(string memory list, string memory one, uint256 i, uint256 dflt) internal view returns (uint256) {
        uint256[] memory v = vm.envOr(list, ",", new uint256[](0));
        if (v.length == 0) {
            require(nftCols.length == 1 || !vm.envExists(one), string.concat(one, " names one collection's value; with several collections use ", list));
            return vm.envOr(one, dflt);
        }
        require(v.length == nftCols.length, string.concat(list, " names one value per collection"));
        return v[i];
    }

    function _targets() internal pure returns (CurveFactory.Targets memory t) {
        t.normalRaise = 5 ether; t.earlyRaise = 8 ether; t.earlyUntilEth = 2 ether; t.maxRaise = 30 ether;
        t.minFeeBps = 100; t.maxFeeBps = 800;
    }
}
