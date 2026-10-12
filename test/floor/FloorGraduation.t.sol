// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {FloorBase} from "./FloorBase.sol";
import {MockSeeder} from "../utils/CurveBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {CollectionDirectory} from "../../src/curve/CollectionDirectory.sol";
import {IItemQueue} from "../../src/interfaces/IItemQueue.sol";

/// docs/v5/floor-module.md §6.4: a graduating curve converts its whole item budget through the module in one call, at the
/// all-in floor, with no item deadline; the canister then converts the units to items through the netting signal. The curve and
/// the factory are unchanged except for the type of their queue (`IItemQueue`).
contract FloorGraduationTest is FloorBase {
    CurveFactory factory;
    MockSeeder seeder;
    SweepVault curveRaffle;
    SweepVault curveBurn;
    address creator = makeAddr("creator");
    address curveBuyer = makeAddr("curve buyer");

    function _template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = 1_000_000_000e18;
        c.reserved = 200_000_000e18;
        c.platformFeeCap = 0.25 ether;
        c.platformBps = 500;
        c.poolEthBps = 1_260;
        c.feeBps = 100;
        c.vaultShareBps = 5_000;
        c.creatorShareBps = 3_000;
        c.window = 3 days;
    }

    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.maxEthBacked = 12e18;                  // small, so a second graduation has to wait for conversions
    }

    function setUp() public override {
        super.setUp();
        seeder = new MockSeeder(IERC20(address(wrapper)));
        CurveFactory.Targets memory t = CurveFactory.Targets(5 ether, 8 ether, 2 ether, 30 ether, 100, 800);
        factory = new CurveFactory(wrapper, seeder, IItemQueue(address(module)), platform, _template(), t, PBS);
        curveRaffle = new SweepVault(registry, RAFFLE_LISTING, wrapper, SweepVault.Policy.RAFFLE, address(venue), factory, desk, PBS);
        curveBurn = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.BURN, address(venue), factory, desk, PBS);
        _ceiling(curveRaffle);
        _ceiling(curveBurn);
        factory.wire(curveRaffle, curveBurn);
        module.wire(factory);
        _postCap(curveRaffle, 0.646 ether);
        seeder.setDepth(2 ether);                // past the early stage: 5 ETH targets
    }

    function _launchAndFill() internal returns (FrogCurve c, CurveToken tok) {
        vm.prank(creator);
        (address coin, address curve) = factory.launch("Alien", "ALIEN", SweepVault.Policy.RAFFLE, false);
        c = FrogCurve(payable(curve));
        tok = CurveToken(coin);
        uint256 amount = c.raise() * 2;
        vm.deal(curveBuyer, curveBuyer.balance + amount);
        vm.prank(curveBuyer);
        c.buy{value: amount}(0);
    }

    function test_aGraduationConvertsItsBudgetAtOnce() public {
        (FrogCurve c, CurveToken tok) = _launchAndFill();
        uint256 budget = c.itemBudget();
        assertEq(budget, 4.12 ether, "the worked example's budget");
        FloorModule.Grad memory g = module.grad(address(tok));
        assertEq(uint8(g.status), uint8(FloorModule.GradStatus.Filled), "no waiting for items");
        uint256 price = module.gradPrice();
        assertEq(price, _ceil(F * 10_300, 10_000), "the all-in floor: F x 1.03, what the old queue paid per item");
        assertEq(g.units, budget * 1e18 / price, "6.32 wFROG for 4.12 ETH");
        assertEq(module.backing(), budget);
        assertEq(wrapper.ethBacked(), g.units);
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
        assertEq(seeder.wrappedForToken() + seeder.wrappedForShared(), g.units, "every unit went to the pools");
        assertEq(seeder.wrappedForToken(), g.units * (budget - c.poolEth()) / budget, "the coin pool opens at the curve's last price");
        assertEq(seeder.ethIn(), c.poolEth(), "no tail: the module converted every wei");
        _books();
    }

    function test_theCanisterThenConvertsTheUnitsToItems() public {
        (FrogCurve c,) = _launchAndFill();
        c.graduate();
        uint256 want = (module.netUnits() - module.buffer()) / 1e18;
        assertEq(want, 6, "6.32 ETH-backed units: six items to buy");
        uint256 cost = module.beta();                          // the canister buys at the all-in floor
        for (uint256 i = 0; i < want; ++i) _convert(cost);
        assertEq(wrapper.held(), 6);
        assertLt(wrapper.ethBacked(), 1.05e18, "what is left is under one item plus the buffer");
        assertLt(module.netUnits(), 1e18 + module.buffer(), "the canister wants nothing more");
        _books();
    }

    function test_aStaleFloorAtTheClose_theGraduationWaits_thenAnyoneFillsIt() public {
        _skip(module.FLOOR_TTL());                             // stale
        _postCap(curveRaffle, 0.646 ether);                    // the opening rule reads the vault's cap, not the module's
        (FrogCurve c, CurveToken tok) = _launchAndFill();
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));
        assertEq(module.gradEth(), c.itemBudget());
        vm.expectRevert(FrogCurve.NotFilled.selector);
        c.graduate();
        vm.expectRevert(FloorModule.NoRoom.selector);
        module.fill(address(tok));
        _post(F);
        module.fill(address(tok));
        c.graduate();
        assertEq(module.gradEth(), 0);
        _books();
    }

    function test_neverFresh_theWindowPasses_andHoldersRefundTheirETH() public {
        _skip(module.FLOOR_TTL());
        _postCap(curveRaffle, 0.646 ether);
        (FrogCurve c, CurveToken tok) = _launchAndFill();
        _skip(3 days + 1 hours);
        c.openRefunds();
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Cancelled));
        assertEq(c.refundWrappedPool(), 0, "no units were bought");
        assertEq(c.refundEthPool(), c.reserveEth());
        assertEq(module.gradEth(), 0);
        _books();
    }

    /// Audit 2026-09-26 A-03: the hard cap no longer lets a curve close into a Queued graduation bound to refund. A launch is
    /// refused while the module has no room for its budget, and a curve launched while there was room cannot CLOSE until there
    /// is room again: it keeps trading (holders can sell back at the curve's price), and closes, filled at once, when room
    /// returns. (Before the fix the second close went Queued and waited for conversions, or refunded after 3 days.)
    function test_theHardCapHoldsALaterCloseInTrading_untilConversionsMakeRoom() public {
        vm.prank(creator);
        (, address curve2) = factory.launch("Beta", "BETA", SweepVault.Policy.RAFFLE, false);   // room for it now (0 + 6.32 ≤ 12)
        FrogCurve c2 = FrogCurve(payable(curve2));
        (FrogCurve c1,) = _launchAndFill();
        c1.graduate();                                         // 6.32 units ETH-backed
        assertFalse(module.hasRoomFor(c2.itemBudget()), "another 6.32 would pass 12");
        assertFalse(factory.launchesOpen());
        vm.prank(creator);
        vm.expectRevert(CurveFactory.QueueFull.selector);
        factory.launch("Gamma", "GAMMA", SweepVault.Policy.RAFFLE, false);

        uint256 amount = c2.raise() * 2;
        vm.deal(curveBuyer, curveBuyer.balance + amount);
        assertFalse(c2.canClose());
        vm.prank(curveBuyer);
        vm.expectRevert(FrogCurve.QueueFull.selector);
        c2.buy{value: amount}(0);
        vm.prank(curveBuyer);
        c2.buy{value: 1 ether}(0);                              // buys short of the close still go through
        assertEq(uint8(c2.state()), uint8(FrogCurve.State.Trading));

        _convert(module.beta());                               // 5.32 + 6.32 ≤ 12
        assertTrue(c2.canClose());
        assertTrue(factory.launchesOpen());
        vm.prank(curveBuyer);
        c2.buy{value: amount}(0);
        assertEq(uint8(module.grad(address(c2.token())).status), uint8(FloorModule.GradStatus.Filled), "filled at the close");
        c2.graduate();
        _books();
    }

    function test_onlyACurveUsesTheGraduationLane() public {
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotCurve.selector);
        module.enqueue{value: 1 ether}(stranger, 0, 0);
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotCurve.selector);
        module.release(stranger);
        vm.prank(stranger);
        vm.expectRevert(FloorModule.NotCurve.selector);
        module.cancel(stranger);
        vm.expectRevert(FloorModule.AlreadyWired.selector);
        module.wire(factory);
    }

    function test_theDirectoryListsTheModuleAsTheCollectionsQueue() public {
        CollectionDirectory dir = new CollectionDirectory(address(this));
        dir.add(factory);
        address[] memory qs = dir.queues();
        assertEq(qs[0], address(module), "the canister finds the module where it found the queue");
    }

    function _ceil(uint256 a, uint256 b) internal pure returns (uint256) { return (a + b - 1) / b; }
}
