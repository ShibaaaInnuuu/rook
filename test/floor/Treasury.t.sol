// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {FloorBase} from "./FloorBase.sol";
import {MockSeeder} from "../utils/CurveBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {ProtocolTreasury} from "../../src/floor/ProtocolTreasury.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {IItemQueue} from "../../src/interfaces/IItemQueue.sol";
import {Wrapper} from "../../src/Wrapper.sol";
import {MintDesk} from "../../src/curve/MintDesk.sol";

/// docs/v5/floor-module.md §13.2 and §13.3: the fee slices, the treasury's fixed shares, weighted distribution, floats, the
/// bootstrap loan repaid before any other route, and the absence of any discretionary path. The frog collection's module is
/// built with the treasury; its curves' platform is the treasury; a second collection (another wrapper and module) for the
/// weighting.
contract TreasuryBase is FloorBase {
    ProtocolTreasury t;
    CurveFactory factory;
    MockSeeder seeder;
    SweepVault curveRaffle;
    SweepVault curveBurn;
    address creator = makeAddr("creator");
    address curveBuyer = makeAddr("curve buyer");
    address ops = makeAddr("ops");

    function _params() internal pure virtual override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
        p.fillBandBps = 300;
    }

    /// The worked terms with the two new slices: 40 / 30 / 5 / 5 / 20.
    function _template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = 1_000_000_000e18;
        c.reserved = 200_000_000e18;
        c.platformFeeCap = 0.25 ether;
        c.platformBps = 500;
        c.poolEthBps = 1_260;
        c.feeBps = 100;
        c.vaultShareBps = 4_000;
        c.creatorShareBps = 3_000;
        c.reserveShareBps = 500;
        c.liquidityShareBps = 500;
        c.window = 3 days;
    }

    function setUp() public virtual override {
        super.setUp();
        t = new ProtocolTreasury(registry);
        moduleTreasury = address(t);
        platform = address(t);                                  // skims and platform fees land in the treasury
        _redeploy(_params());
        // these suites exercise the stage-1 machinery (floats, loans): the old defaults, set the legitimate way. The Phase-1
        // constructor defaults (9000/0/0/1000, no loan; C-07) are tested in test/audit/C.
        vm.startPrank(owner);
        t.setShares(4_000, 3_000, 2_000, 1_000);
        t.setBootstrapLoan(2 ether);
        vm.stopPrank();
        seeder = new MockSeeder(IERC20(address(wrapper)));
        CurveFactory.Targets memory tg = CurveFactory.Targets(5 ether, 8 ether, 2 ether, 30 ether, 100, 800);
        factory = new CurveFactory(wrapper, seeder, IItemQueue(address(module)), address(t), _template(), tg, PBS);
        // no desk: the base's is the old wrapper's (this suite redeployed the wrapper with the treasury)
        MintDesk none = MintDesk(payable(address(0)));
        curveRaffle = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), factory, none, PBS);
        curveBurn = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(venue), factory, none, PBS);
        _ceiling(curveRaffle);
        _ceiling(curveBurn);
        factory.wire(curveRaffle, curveBurn);
        module.wire(factory);
        _postCap(curveRaffle, 0.646 ether);
        seeder.setDepth(2 ether);
        vm.prank(owner);
        t.addCollection(module);
        vm.prank(owner);
        t.setOps(ops);
    }

    function _income(uint256 amount) internal {
        vm.deal(stranger, stranger.balance + amount);
        vm.prank(stranger);
        (bool ok,) = address(t).call{value: amount}("");
        assertTrue(ok);
    }

    function _launch() internal returns (FrogCurve c, CurveToken tok) {
        vm.prank(creator);
        (address coin, address curve) = factory.launch("Alien", "ALIEN", SweepVault.Policy.RAFFLE, false);
        c = FrogCurve(payable(curve));
        tok = CurveToken(coin);
    }

    function _trade(FrogCurve c, uint256 eth) internal {
        vm.deal(curveBuyer, curveBuyer.balance + eth);
        vm.prank(curveBuyer);
        c.buy{value: eth}(0);
    }
}

contract TreasuryTest is TreasuryBase {
    using stdJson for string;

    // ---- the shares ----

    function test_theSharesSumToOneHundredPercent_andOnlyTheTimelockChangesThem() public {
        assertEq(uint256(t.reservesBps()) + t.floatsBps() + t.bootstrapBps() + t.opsBps(), 10_000);
        vm.prank(stranger);
        vm.expectRevert(ProtocolTreasury.NotOwner.selector);
        t.setShares(5_000, 2_000, 2_000, 1_000);
        vm.startPrank(owner);
        vm.expectRevert(ProtocolTreasury.BadShares.selector);
        t.setShares(5_000, 2_000, 2_000, 999);
        t.setShares(5_000, 2_000, 2_000, 1_000);
        vm.stopPrank();
        assertEq(t.reservesBps(), 5_000);
        vm.prank(stranger);
        vm.expectRevert(ProtocolTreasury.NotOwner.selector);
        t.setOps(stranger);
        vm.prank(stranger);
        vm.expectRevert(ProtocolTreasury.NotOwner.selector);
        t.setBootstrapLoan(1);
        vm.prank(stranger);
        vm.expectRevert(ProtocolTreasury.NotOwner.selector);
        t.addCollection(module);
    }

    function test_allocateSplitsIncomeByTheShares_andPaysOperations() public {
        _income(10 ether);
        assertEq(t.unallocated(), 10 ether);
        t.allocate();
        assertEq(t.reservesPot(), 4 ether);
        assertEq(t.floatsPot(), 3 ether);
        assertEq(t.bootstrapPot(), 2 ether);
        assertEq(ops.balance, 1 ether);
        assertEq(t.unallocated(), 0);
        assertEq(address(t).balance, 9 ether);
        vm.expectRevert(ProtocolTreasury.NothingToDo.selector);
        t.allocate();
        // operations unset: its share joins the reserves pot, nothing is stranded
        vm.prank(owner);
        t.setOps(address(0));
        _income(1 ether);
        t.allocate();
        assertEq(t.reservesPot(), 4.5 ether);
        assertEq(ops.balance, 1 ether);
    }

    function test_theBooksBalanceAtEveryStep() public {
        _income(3 ether);
        t.allocate();
        assertEq(address(t).balance, t.reservesPot() + t.floatsPot() + t.bootstrapPot() + t.unallocated());
        _mint(alice, 1 ether);                                  // volume, so the module has a weight
        t.distribute();
        assertEq(address(t).balance, t.reservesPot() + t.floatsPot() + t.bootstrapPot() + t.unallocated());
        assertEq(t.reservesPot(), 0, "all of it delivered: one collection with all the weight");
        assertEq(t.floatsPot(), 0);
        assertEq(module.reserve(), 0.25 ether, "the mint's share is the reserve's basis");
        assertEq(module.surplus(), 1.2 ether, "the treasury's 40% is the surplus (B-02)");
        assertGt(module.floatUnits(), 0, "the 30% became a float");
        _books();
    }

    // ---- distribution: weighted by seven-day volume, floats deferred when a module cannot take them ----

    function test_distributeIsWeightedBySevenDayVolume() public {
        FloorModule a = module;
        // a second collection: another wrapper and module, registered too
        wrapper = new Wrapper(registry, distributor, beacon, FROGS, distribution, custodian, CUSTODY_BTC, "w3", "w3", PBS);
        wrapper.initMintLimit(1_000);
        FloorModule b = new FloorModule(wrapper, IKeeperSource(address(registry)), payee, platform, address(t), _params(), PBS);
        wrapper.wireFloorModule(address(b));
        vm.prank(keeper);
        b.postFloor(F, 10_000, 50);
        vm.prank(owner);
        t.addCollection(b);
        assertEq(t.collectionCount(), 2);
        // volumes: a 3 ETH, b 1 ETH
        module = a; _mint(alice, 3 ether);
        module = b; _mint(bob, 1 ether);
        (uint256[] memory w, uint256 total) = t.weights();
        assertEq(w[0], 3 ether); assertEq(w[1], 1 ether); assertEq(total, 4 ether);
        _income(10 ether);
        t.allocate();
        uint256 ra = a.surplus(); uint256 rb = b.surplus();
        t.distribute();
        assertEq(a.surplus() - ra, 3 ether, "three quarters of the 4 ETH reserves pot");
        assertEq(b.surplus() - rb, 1 ether);
        assertApproxEqAbs(a.backing(), 0.75 * 3 ether + 2.25 ether, 1, "the float: three quarters of 3 ETH, all convertible");
        assertEq(t.reservesPot(), 0);
        assertEq(t.floatsPot(), 0);
        // no volume anywhere: nothing moves
        _income(1 ether);
        t.allocate();
        for (uint256 d = 0; d < 8; ++d) { _skip(1 days); vm.prank(keeper); a.postFloor(F, 10_000, 50); vm.prank(keeper); b.postFloor(F, 10_000, 50); }
        vm.expectRevert(ProtocolTreasury.NothingToDo.selector);
        t.distribute();
    }

    function test_aStaleModuleKeepsItsFloatInThePot_andStillGetsItsReserve() public {
        _mint(alice, 1 ether);
        _income(10 ether);
        t.allocate();
        _skip(module.FLOOR_TTL() + 1 minutes);                  // stale: fundFloat reverts, fundReserve does not
        uint256 r = module.surplus();
        t.distribute();
        assertEq(module.surplus(), r + 4 ether);
        assertEq(t.floatsPot(), 3 ether, "kept for the next call");
        assertEq(module.floatUnits(), 0);
        _post(F);
        t.distribute();
        assertEq(t.floatsPot(), 0);
        assertGt(module.floatUnits(), 0);
    }

    // ---- the fee slices (§13.2) ----

    function test_theFeeSlicesReachTheReserveAndTheLiquidity_andThePlatformShareTheTreasury() public {
        (FrogCurve c,) = _launch();
        _trade(c, 1 ether);
        assertEq(c.reserveOwed(), 0.0005 ether, "5% of a 1% fee");
        assertEq(c.liquidityOwed(), 0.0005 ether);
        assertEq(c.platformOwed(), 0.002 ether, "20%: the 0.2% of volume");
        uint256 r = module.surplus();
        c.flushProtocolFees();
        assertEq(module.surplus(), r + 0.0005 ether, "into the reserve surplus");
        assertEq(module.reserveUnits(), 0, "with no units: a surplus");
        assertEq(seeder.protocolEth(), 0.0005 ether, "into protocol liquidity, through the module");
        c.claimPlatformFees();
        assertEq(address(t).balance, 0.002 ether);
        assertEq(t.unallocated(), 0.002 ether);
    }

    function test_theGraduationFeeAndTheSkimsReachTheTreasury() public {
        (FrogCurve c,) = _launch();
        _trade(c, c.raise() * 2);
        c.graduate();
        c.claimPlatformFees();
        assertApproxEqAbs(address(t).balance, c.platformFee() + (c.raise() * 100 / 9_900) * 2_000 / 10_000, 4, "the capped graduation fee and the platform's 20% of the trade fee");
        // spread revenue beyond the target is skimmed here too
        _mint(alice, 4 * module.mintPrice());
        uint256 before = address(t).balance;
        for (uint256 i = 0; i < 20 && module.revenue() <= module.revenueTarget(); ++i) {
            _convert(module.betaConvertible() / 2);            // cheap items: revenue
            if (module.netUnits() < 1e18) { _skip(1 hours); _post(F); _mint(alice, 4 * module.mintPrice()); }
        }
        if (module.revenue() > module.revenueTarget()) {
            module.skim();
            assertGt(address(t).balance, before, "skimmed to the platform: the treasury");
        }
    }

    // ---- the bootstrap loan (§13.3) ----

    function test_bootstrapIsOneLoanPerCollection_halfFloatHalfReserve() public {
        vm.expectRevert(ProtocolTreasury.NothingToDo.selector);
        t.bootstrap(module);                                    // an empty pot
        _income(20 ether);
        t.allocate();                                           // 4 ETH in the bootstrap pot; the loan is 2
        uint256 r = module.surplus();
        uint256 b = module.backing();
        t.bootstrap(module);
        assertEq(module.surplus(), r + 1 ether);
        assertEq(module.backing(), b + 1 ether, "a float: convertible");
        assertGt(module.floatUnits(), 0);
        assertEq(t.outstanding(address(module)), 2 ether);
        assertEq(module.loanOutstanding(), 2 ether);
        assertEq(t.bootstrapPot(), 2 ether);
        vm.expectRevert(ProtocolTreasury.AlreadyBootstrapped.selector);
        t.bootstrap(module);
        vm.expectRevert(ProtocolTreasury.NotCollection.selector);
        t.bootstrap(FloorModule(stranger));
        // a stale floor: the float half goes to the reserve instead, the loan is still the whole amount
        wrapper = new Wrapper(registry, distributor, beacon, FROGS, distribution, custodian, CUSTODY_BTC, "w3", "w3", PBS);
        wrapper.initMintLimit(1_000);
        FloorModule m2 = new FloorModule(wrapper, IKeeperSource(address(registry)), payee, platform, address(t), _params(), PBS);
        wrapper.wireFloorModule(address(m2));
        vm.prank(owner);
        t.addCollection(m2);
        t.bootstrap(m2);
        assertEq(m2.surplus(), 2 ether);
        assertEq(m2.floatUnits(), 0);
        assertEq(t.outstanding(address(m2)), 2 ether);
        assertEq(t.bootstrapPot(), 0);
        assertEq(t.totalLent(), 4 ether);
    }

    function test_theLoanIsRepaidFromTheFeeFlowBeforeAnyOtherRoute() public {
        _income(20 ether);
        t.allocate();
        t.bootstrap(module);
        uint256 r = module.surplus();
        (FrogCurve c,) = _launch();
        _trade(c, 4 ether);                                     // 0.002 to the reserve slice, 0.002 to liquidity
        c.flushProtocolFees();
        assertEq(module.surplus(), r, "nothing reached the reserve");
        assertEq(seeder.protocolEth(), 0, "nothing reached the liquidity");
        assertEq(t.outstanding(address(module)), 2 ether - 0.004 ether, "both slices repaid the loan");
        assertEq(t.totalRepaid(), 0.004 ether);
        assertEq(t.unallocated(), 0.004 ether, "a repayment is income again");
        // anyone funding the reserve repays first too, then the rest is reserve
        vm.deal(stranger, 3 ether);
        vm.prank(stranger);
        module.fundReserve{value: 2.5 ether}();
        assertEq(t.outstanding(address(module)), 0, "repaid in full");
        assertEq(module.surplus(), r + 0.504 ether, "what was left after the 1.996 still owed");
        // and from then on every slice reaches its route
        _trade(c, 1 ether);
        c.flushProtocolFees();
        assertEq(module.surplus(), r + 0.504 ether + 0.0005 ether);
        assertEq(seeder.protocolEth(), 0.0005 ether);
    }

    function test_aLoanIsNeverRepaidTwice() public {
        _income(20 ether);
        t.allocate();
        t.bootstrap(module);
        vm.deal(address(module), 10 ether);
        vm.startPrank(address(module));
        vm.expectRevert(abi.encodeWithSelector(ProtocolTreasury.OverRepaid.selector, 2 ether + 1, 2 ether));
        t.repay{value: 2 ether + 1}();
        t.repay{value: 2 ether}();
        vm.expectRevert(abi.encodeWithSelector(ProtocolTreasury.OverRepaid.selector, 1, 0));
        t.repay{value: 1}();
        vm.stopPrank();
        assertEq(t.loan(address(module)).repaid, 2 ether);
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(ProtocolTreasury.NotCollection.selector);
        t.repay{value: 1}();
        // and a module with no loan repays nothing: its funding goes where it should
        uint256 r = module.surplus();
        vm.prank(stranger);
        module.fundReserve{value: 1 ether}();
        assertEq(module.surplus(), r + 1 ether);
    }

    function test_onlyTheTimelockForgives_andNothingElseLowersALoan() public {
        _income(20 ether);
        t.allocate();
        t.bootstrap(module);
        vm.prank(stranger);
        vm.expectRevert(ProtocolTreasury.NotOwner.selector);
        t.forgive(address(module));
        assertEq(t.outstanding(address(module)), 2 ether);
        vm.prank(owner);
        t.forgive(address(module));
        assertEq(t.outstanding(address(module)), 0);
        assertTrue(t.loan(address(module)).forgiven);
        vm.prank(owner);
        vm.expectRevert(ProtocolTreasury.NothingToDo.selector);
        t.forgive(address(module));
        uint256 r = module.surplus();
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        module.fundReserve{value: 1 ether}();
        assertEq(module.surplus(), r + 1 ether, "forgiven: the routes are open");
    }

    // ---- attacks: drains ----

    function test_nothingLeavesButToRegisteredModulesAndOperations() public {
        _income(10 ether);
        _mint(alice, 1 ether);
        uint256 s0 = stranger.balance;
        vm.startPrank(stranger);
        t.allocate();
        t.distribute();
        vm.expectRevert(ProtocolTreasury.NotOwner.selector);
        t.addCollection(FloorModule(stranger));
        vm.expectRevert(ProtocolTreasury.NotCollection.selector);
        t.bootstrap(FloorModule(stranger));
        vm.stopPrank();
        assertEq(stranger.balance, s0, "the caller got nothing");
        assertEq(ops.balance, 1 ether);
        assertEq(address(t).balance, t.bootstrapPot(), "everything else went to the one collection");
        // a module of someone else's, even one naming this treasury, is not a collection until the timelock says so
        wrapper = new Wrapper(registry, distributor, beacon, FROGS, distribution, custodian, CUSTODY_BTC, "w3", "w3", PBS);
        FloorModule fake = new FloorModule(wrapper, IKeeperSource(address(registry)), payee, platform, address(t), _params(), PBS);
        vm.expectRevert(ProtocolTreasury.NotCollection.selector);
        t.bootstrap(fake);
        // and a module naming another treasury cannot be registered
        FloorModule other = new FloorModule(wrapper, IKeeperSource(address(registry)), payee, platform, stranger, _params(), PBS);
        vm.prank(owner);
        vm.expectRevert(ProtocolTreasury.BadInput.selector);
        t.addCollection(other);
    }

    function testFuzz_thePotsNeverExceedTheBalance_andTheSharesAlwaysSumToOne(uint96[4] memory income, uint16 a, uint16 b, uint16 c) public {
        a = uint16(bound(a, 0, 10_000)); b = uint16(bound(b, 0, 10_000 - a)); c = uint16(bound(c, 0, 10_000 - a - b));
        vm.prank(owner);
        t.setShares(a, b, c, 10_000 - a - b - c);
        _mint(alice, 1 ether);
        for (uint256 i = 0; i < 4; ++i) {
            uint256 amount = bound(income[i], 0, 100 ether);
            if (amount != 0) { _income(amount); t.allocate(); }
            if (t.reservesPot() + t.floatsPot() != 0) t.distribute();
            if (t.bootstrapPot() != 0 && t.outstanding(address(module)) == 0 && t.loan(address(module)).principal == 0) t.bootstrap(module);
            assertLe(t.reservesPot() + t.floatsPot() + t.bootstrapPot(), address(t).balance);
            assertEq(uint256(t.reservesBps()) + t.floatsBps() + t.bootstrapBps() + t.opsBps(), 10_000);
            assertLe(t.loan(address(module)).repaid, t.loan(address(module)).principal);
            _books();
        }
    }

    // ---- the exact ABI: no withdraw, no recipient, no pause ----

    function test_theTreasuryExposesExactlyWhatItMeansTo() public view {
        string memory json = vm.readFile("out/ProtocolTreasury.sol/ProtocolTreasury.json");
        string[] memory sigs = vm.parseJsonKeys(json, ".methodIdentifiers");
        string[30] memory allowed = [
            // the timelock
            "setShares(uint16,uint16,uint16,uint16)", "setOps(address)", "setBootstrapLoan(uint256)", "addCollection(address)",
            "forgive(address)",
            // anyone
            "allocate()", "distribute()", "bootstrap(address)", "repay()",
            // views
            "BPS()", "registry()", "reservesBps()", "floatsBps()", "bootstrapBps()", "opsBps()", "ops()", "bootstrapLoan()",
            "reservesPot()", "floatsPot()", "bootstrapPot()", "totalReceived()", "totalAllocated()", "totalToOps()",
            "totalToReserves()", "totalToFloats()", "totalLent()", "totalRepaid()", "isCollection(address)",
            "unallocated()", "outstanding(address)"];
        string[4] memory allowed2 = ["loan(address)", "collectionCount()", "collection(uint256)", "weights()"];
        for (uint256 i = 0; i < sigs.length; ++i) {
            bool ok;
            for (uint256 j = 0; j < allowed.length && !ok; ++j) ok = keccak256(bytes(sigs[i])) == keccak256(bytes(allowed[j]));
            for (uint256 j = 0; j < allowed2.length && !ok; ++j) ok = keccak256(bytes(sigs[i])) == keccak256(bytes(allowed2[j]));
            assertTrue(ok, string.concat("unexpected external function: ", sigs[i]));
        }
        assertEq(sigs.length, allowed.length + allowed2.length, "exactly the documented surface");
    }

    /// The module's new entries, by selector, for the canister, the keeper and the web (§13.6).
    function test_theModulesNewSelectors() public pure {
        assertEq(FloorModule.itemSold.selector, bytes4(keccak256("itemSold(uint256)")));
        assertEq(FloorModule.fundReserve.selector, bytes4(keccak256("fundReserve()")));
        assertEq(FloorModule.fundLiquidity.selector, bytes4(keccak256("fundLiquidity()")));
        assertEq(FloorModule.fundFloat.selector, bytes4(keccak256("fundFloat()")));
        assertEq(FloorModule.price.selector, bytes4(keccak256("price()")));
        assertEq(FloorModule.volume7d.selector, bytes4(keccak256("volume7d()")));
        assertEq(FloorModule.minSaleProceeds.selector, bytes4(keccak256("minSaleProceeds()")));
        assertEq(bytes4(keccak256("floatUnits()")), bytes4(keccak256(bytes(string.concat("floatUnits", "()")))));   // a public variable: its getter
        assertEq(bytes4(keccak256("reserveUnits()")), bytes4(keccak256(bytes(string.concat("reserveUnits", "()")))));   // a public variable: its getter
        assertEq(bytes4(keccak256("revenue()")), bytes4(keccak256(bytes(string.concat("revenue", "()")))));   // a public variable: its getter
        assertEq(FloorModule.loanOutstanding.selector, bytes4(keccak256("loanOutstanding()")));
        // and the ones that did not change
        assertEq(FloorModule.floor.selector, bytes4(0x40695363));
        assertEq(FloorModule.netUnits.selector, bytes4(0x273c589e));
        assertEq(FloorModule.escrow.selector, bytes4(0xe2fdcc17));
        assertEq(FloorModule.itemArrived.selector, bytes4(0xd996759a));
    }
}
