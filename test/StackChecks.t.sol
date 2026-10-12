// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {StackChecks} from "../script/StackChecks.sol";
import {CurveFactory} from "../src/curve/CurveFactory.sol";
import {FloorModule} from "../src/floor/FloorModule.sol";

/// docs/v6/deploy-stack.md §4: every reason the stack deployment is refused. The library is called through this harness so a
/// revert can be expected, exactly as test/DeployChecks.t.sol does for the first deployment.
contract StackHarness {
    function check(StackChecks.Plan memory p) external pure { StackChecks.check(p); }
    function checkParams(FloorModule.Params memory p) external pure { StackChecks.checkParams(p); }
    function expected(uint256 chainId, address deployer) external pure returns (string memory) { return StackChecks.expectedConfirm(chainId, deployer); }
    function stageOf(uint256 s) external pure returns (StackChecks.Stage memory) { return StackChecks.stageOf(s); }
    function defaults(uint256 unit, uint256 stage) external pure returns (FloorModule.Params memory) { return StackChecks.defaultParams(unit, stage); }
    function fileFor(uint256 c) external pure returns (string memory) { return StackChecks.fileFor(c); }
    function checkNftSet(address[] memory c, uint256 n, uint256 s, uint256 g) external pure { StackChecks.checkNftSet(c, n, s, g); }
    function checkNftSlugs(string[] memory g) external pure { StackChecks.checkNftSlugs(g); }
    function checkCol(uint256 c, uint64 n, address a, CurveFactory.Targets memory t, uint16 cv, uint16 rv) external pure {
        StackChecks.checkNftCollection(c, n, a, t, cv, rv);
    }
}

contract StackChecksTest is Test {
    StackHarness h;
    address deployer = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    address custodian = address(0xb0b0e1);
    string constant TAPROOT = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";

    function setUp() public { h = new StackHarness(); }

    function _good(uint256 chainId) internal view returns (StackChecks.Plan memory p) {
        p.chainId = chainId;
        p.deployer = deployer;
        p.confirm = h.expected(chainId, deployer);
        p.registry = address(0x1234);
        p.registryHasCode = true;
        p.registryDistributor = address(0x5678);
        p.listingId = 2;
        p.listingAutonomous = true;
        p.custodian = custodian;
        p.custodyAddress = TAPROOT;
        p.name = "Wrapped Bitcoin Frog";
        p.symbol = "wFROG";
        p.poolManagerHasCode = true;
        p.stage = 0;
    }

    /// docs/v7/phase1-nft.md §3.4: the Ethereum collection alone needs no Bitcoin custody address, and needs the collection.
    function test_anNftOnlyStackNeedsItsCollectionAndNoBitcoinCustody() public {
        StackChecks.Plan memory p = _good(4663);
        p.nftOnly = true;
        vm.expectRevert(StackChecks.NftCollectionMissing.selector);
        h.check(p);
        p.nft = true;
        p.nftChainId = 1;
        p.nftCollection = address(0x5a);
        p.custodyAddress = "";
        p.name = "";
        p.symbol = "";
        h.check(p);
        p.nftChainId = 4663;
        vm.expectRevert(StackChecks.NftChainIsThisChain.selector);
        h.check(p);
        // a Bitcoin stack still needs its taproot custody
        p = _good(4663);
        p.custodyAddress = "";
        vm.expectRevert(StackChecks.CustodyNotTaproot.selector);
        h.check(p);
        // the canister is the custodian AND the NFT module's poster: never an address with code
        p = _good(4663);
        p.nftOnly = true; p.nft = true; p.nftChainId = 1; p.nftCollection = address(0x5a);
        p.custodianHasCode = true;
        vm.expectRevert(StackChecks.CustodianHasCode.selector);
        h.check(p);
    }

    /// docs/v7/phase1-nft.md §9 D1 (2026-09-27): several Ethereum collections at launch, each once, each with its texts.
    function test_severalNftCollectionsEachOnceWithTheirTexts() public {
        address[] memory c = new address[](3);
        c[0] = 0x5Af0D9827E0c53E4799BB226655A1de152A425a5; c[1] = 0xED5AF388653567Af2F388E6224dC7C4b3241C544; c[2] = 0x60E4d786628Fea6478F785A6d7e704777c86a7c6;
        h.checkNftSet(c, 3, 3, 3);
        vm.expectRevert(StackChecks.NftListsDiffer.selector);
        h.checkNftSet(c, 3, 2, 3);
        c[2] = c[0];
        vm.expectRevert(abi.encodeWithSelector(StackChecks.NftCollectionTwice.selector, c[0]));
        h.checkNftSet(c, 3, 3, 3);
        c[2] = address(0);
        vm.expectRevert(StackChecks.NftCollectionMissing.selector);
        h.checkNftSet(c, 3, 3, 3);
        vm.expectRevert(StackChecks.NftCollectionMissing.selector);
        h.checkNftSet(new address[](0), 0, 0, 0);
    }

    /// Audit 2026-09-27-nft D-L-7: a repeated slug (the stack file's key) would drop a deployed collection from the file.
    function test_D_L_7_slugsAreKeysAndNamedOnce() public {
        string[] memory g = new string[](3);
        g[0] = "milady"; g[1] = "azuki"; g[2] = "mayc";
        h.checkNftSlugs(g);
        g[2] = "milady";
        vm.expectRevert(abi.encodeWithSelector(StackChecks.NftSlugTwice.selector, "milady"));
        h.checkNftSlugs(g);
        g[2] = "";
        vm.expectRevert(abi.encodeWithSelector(StackChecks.NftSlugBad.selector, ""));
        h.checkNftSlugs(g);
        g[2] = "MAYC";
        vm.expectRevert(abi.encodeWithSelector(StackChecks.NftSlugBad.selector, "MAYC"));
        h.checkNftSlugs(g);
    }

    /// docs/v7/phase1-nft-pooled.md §2: each collection's own targets and module parameters, bounded.
    function test_perCollectionTargetsAndModuleParamsAreBounded() public {
        CurveFactory.Targets memory t = CurveFactory.Targets(9 ether, 9 ether, 2 ether, 30 ether, 100, 800);
        h.checkCol(4663, 1, address(0xbc), t, 300, 2_500);            // a 9 ETH target with the defaults (BAYC's earlier config)
        // BAYC as the launch decided it (J1, 2026-09-27; deployments/launch/nft-pilot.json): 10 ETH, convBps 1,000, reserveBps 0
        h.checkCol(4663, 1, address(0xbc), CurveFactory.Targets(10 ether, 10 ether, 2 ether, 30 ether, 100, 800), 1_000, 0);
        h.checkCol(4663, 1, address(0xbc), CurveFactory.Targets(10 ether, 10 ether, 2 ether, 30 ether, 100, 800), 1_600, 0);   // the edges
        vm.expectRevert(abi.encodeWithSelector(StackChecks.CollectionOutOfBounds.selector, "normalRaise: 1 to 10 ETH"));
        h.checkCol(4663, 1, address(0xbc), CurveFactory.Targets(11 ether, 11 ether, 2 ether, 30 ether, 100, 800), 300, 2_500);
        vm.expectRevert(abi.encodeWithSelector(StackChecks.CollectionOutOfBounds.selector, "earlyRaise: normalRaise to 10 ETH"));
        h.checkCol(4663, 1, address(0xbc), CurveFactory.Targets(9 ether, 8 ether, 2 ether, 30 ether, 100, 800), 300, 2_500);
        vm.expectRevert(abi.encodeWithSelector(StackChecks.CollectionOutOfBounds.selector, "convBps: 300 to 1,600"));
        h.checkCol(4663, 1, address(0xbc), t, 1_700, 2_500);
        vm.expectRevert(abi.encodeWithSelector(StackChecks.CollectionOutOfBounds.selector, "reserveBps: at most 2,500"));
        h.checkCol(4663, 1, address(0xbc), t, 300, 2_600);
        vm.expectRevert(StackChecks.NftChainIsThisChain.selector);
        h.checkCol(4663, 4663, address(0xbc), t, 300, 2_500);
        vm.expectRevert(StackChecks.NftCollectionMissing.selector);
        h.checkCol(4663, 1, address(0), t, 300, 2_500);
    }

    function test_theConfirmWordIsItsOwn() public view {
        assertEq(h.expected(4663, deployer), "deploy-etch-stack:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        h.check(_good(4663));
        h.check(_good(31337));
        h.check(_good(9466301));
    }

    function test_refusedWithAnotherStepsConfirm() public {
        StackChecks.Plan memory p = _good(4663);
        p.confirm = "deploy-etch:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
        vm.expectRevert(abi.encodeWithSelector(StackChecks.ConfirmMismatch.selector, h.expected(4663, deployer)));
        h.check(p);
        p.confirm = "deploy-etch-stack:31337:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";   // another chain's
        vm.expectRevert(abi.encodeWithSelector(StackChecks.ConfirmMismatch.selector, h.expected(4663, deployer)));
        h.check(p);
    }

    function test_refusedOnAChainThatIsNotOurs_includingThePublicTestnet() public {
        StackChecks.Plan memory p = _good(46630);
        vm.expectRevert(abi.encodeWithSelector(StackChecks.UnknownChain.selector, 46630));
        h.check(p);
        p = _good(1);
        vm.expectRevert(abi.encodeWithSelector(StackChecks.UnknownChain.selector, 1));
        h.check(p);
    }

    function test_refusedWithoutARegistryThatIsWired() public {
        StackChecks.Plan memory p = _good(4663);
        p.registryHasCode = false;
        vm.expectRevert(StackChecks.RegistryIsNotAContract.selector);
        h.check(p);
        p = _good(4663);
        p.registryDistributor = address(0);
        vm.expectRevert(StackChecks.RegistryNotWired.selector);
        h.check(p);
    }

    function test_refusedOverARuneOrABondedListingOnAPublicChain() public {
        StackChecks.Plan memory p = _good(4663);
        p.listingIsRune = true;
        vm.expectRevert(abi.encodeWithSelector(StackChecks.ListingIsRune.selector, 2));
        h.check(p);
        p = _good(4663);
        p.listingAutonomous = false;
        vm.expectRevert(abi.encodeWithSelector(StackChecks.ListingNotAutonomous.selector, 2));
        h.check(p);
        p = _good(31337);
        p.listingAutonomous = false;   // allowed on our own chains, against the v1 demo listings
        h.check(p);
    }

    function test_theCustodianIsACodelessAccountAndCustodyIsTaproot() public {
        StackChecks.Plan memory p = _good(4663);
        p.custodian = address(0);
        vm.expectRevert(StackChecks.CustodianMissing.selector);
        h.check(p);
        p = _good(4663);
        p.custodianHasCode = true;
        vm.expectRevert(StackChecks.CustodianHasCode.selector);
        h.check(p);
        p = _good(4663);
        p.custodyAddress = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";   // segwit v0: not taproot
        vm.expectRevert(StackChecks.CustodyNotTaproot.selector);
        h.check(p);
    }

    function test_refusedWithoutThePoolManagerOrTheText() public {
        StackChecks.Plan memory p = _good(4663);
        p.poolManagerHasCode = false;
        vm.expectRevert(StackChecks.PoolManagerMissing.selector);
        h.check(p);
        p = _good(4663);
        p.symbol = "";
        vm.expectRevert(StackChecks.TextMissing.selector);
        h.check(p);
    }

    /// docs/v5/launch-gates.md G7: custody caps start at stage 0; a first deployment at a later stage is refused on a public chain.
    function test_stageZeroOnAPublicChain_theTableIsG7s() public {
        StackChecks.Plan memory p = _good(4663);
        p.stage = 1;
        vm.expectRevert(abi.encodeWithSelector(StackChecks.StageNotZero.selector, 1));
        h.check(p);
        p = _good(31337);
        p.stage = 3;                   // our chains may rehearse any stage of the table
        h.check(p);
        p.stage = 4;
        vm.expectRevert(abi.encodeWithSelector(StackChecks.StageTooHigh.selector, 4));
        h.check(p);
        StackChecks.Stage memory s0 = h.stageOf(0);
        assertEq(s0.mintsPerDay, 3); assertEq(s0.itemsPerDay, 3); assertEq(s0.custodyItems, 3); assertEq(s0.custodyEth, 10 ether); assertEq(s0.timelockDelayHours, 72);
        StackChecks.Stage memory s3 = h.stageOf(3);
        assertEq(s3.custodyItems, 192); assertEq(s3.custodyEth, 640 ether); assertEq(s3.timelockDelayHours, 24 * 7);
        // every stage is x4 the one before, as the table says
        for (uint256 i = 1; i < 4; ++i) assertEq(h.stageOf(i).custodyItems, 4 * h.stageOf(i - 1).custodyItems);
    }

    function test_anNftCollectionNeedsItsChainAndContract_andNotThisChain() public {
        StackChecks.Plan memory p = _good(4663);
        p.nft = true;
        vm.expectRevert(StackChecks.NftCollectionMissing.selector);
        h.check(p);
        p.nftCollection = address(0xabc);
        p.nftChainId = 4663;
        vm.expectRevert(StackChecks.NftChainIsThisChain.selector);
        h.check(p);
        p.nftChainId = 1;
        h.check(p);
    }

    /// The defaults are docs/v5/floor-module.md §6.2's frog table with §13's reserve share, and they pass the module's own bounds.
    function test_theDefaultParamsAreTheDocumentedOnes_andPassTheModulesBounds() public view {
        FloorModule.Params memory p = h.defaults(1e18, 0);
        assertEq(p.baseSpreadBps, 15); assertEq(p.maxSpreadBps, 1_000); assertEq(p.maxMoveBps, 500); assertEq(p.convBps, 300); assertEq(p.bufferBps, 500);
        assertEq(p.mintPerHour, 5e18); assertEq(p.burnPerHour, 5e18); assertEq(p.maxPublicEthBacked, 10e18); assertEq(p.maxEthBacked, 40e18);
        assertEq(p.reserveBps, 2_500); assertEq(p.fillBandBps, 300); assertEq(p.maxItemsPerDay, 3);
        h.checkParams(p);
        FloorModule.Params memory apes = h.defaults(1e18 / 4, 1);     // a dearer collection: fewer items in every cap
        assertEq(apes.mintPerHour, 1.25e18); assertEq(apes.maxItemsPerDay, 6);
        h.checkParams(apes);
    }

    function test_paramsOutsideTheModulesBoundsAreRefusedBeforeAnythingIsSent() public {
        FloorModule.Params memory p = h.defaults(1e18, 0);
        p.reserveBps = 5_001;
        vm.expectRevert(abi.encodeWithSelector(StackChecks.ParamsOutOfBounds.selector, "reserveBps"));
        h.checkParams(p);
        p = h.defaults(1e18, 0);
        p.maxPublicEthBacked = p.maxEthBacked + 1;
        vm.expectRevert(abi.encodeWithSelector(StackChecks.ParamsOutOfBounds.selector, "maxPublicEthBacked"));
        h.checkParams(p);
        p = h.defaults(1e18, 0);
        p.maxItemsPerDay = 0;
        vm.expectRevert(abi.encodeWithSelector(StackChecks.ParamsOutOfBounds.selector, "minConfidence/maxItemsPerDay"));
        h.checkParams(p);
    }

    function test_theFileIsPerChain() public view {
        assertEq(h.fileFor(4663), "deployments/stack-4663.json");
        assertEq(h.fileFor(9466301), "deployments/stack-9466301.json");
    }
}
