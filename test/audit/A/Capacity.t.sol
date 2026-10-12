// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {AuditABase} from "./AuditABase.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {FrogCurve} from "../../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../../src/curve/CurveToken.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";

/// Regression for A-03 (fixes-A-graduation.md, option (c)). At stage 0 the module's 40-unit hard cap fills after a handful of
/// graduations. Before the fix every later curve closed into a Queued graduation and refunded after 3 days (fees kept, average
/// price), and a public mint could force that on a chosen closing buy. Now:
///  - a launch of a coin that buys items is refused while the module has no room for its budget (`launchesOpen()` false);
///  - a curve launched while there was room cannot CLOSE until there is room again: its closing buy reverts `QueueFull`, it
///    keeps trading (holders sell back at the curve's price), and it closes — filled at once — when room returns.
contract CapacityTest is AuditABase {
    address trader = makeAddr("trader");

    function _fresh() internal {
        _postCap(curveRaffle, CAP);
        _floor(CAP);
    }

    function _nextBudget() internal view returns (uint256) {
        (uint256 raise,,) = factory.raiseFor(false);
        return factory.itemBudgetFor(raise);
    }

    function test_A03_aFullModuleStopsLaunchesAndHoldsClosesInTrading() public {
        // a public minter holds ~4.9 ETH-backed units (recoverable by burning; the audit's front-run lever, B-07)
        vm.deal(trader, 10 ether);
        vm.prank(trader);
        uint256 minted = module.mint{value: Math.mulDiv(module.mintPrice(), 49, 10)}(trader, 0);
        assertGt(minted, 4.8e18);

        // graduations until there is room for one more budget but not two
        uint256 n;
        while (module.hasRoomFor(2 * _nextBudget())) {
            (FrogCurve g,) = _launch(SweepVault.Policy.RAFFLE, false);
            _skipBoth(20 minutes);
            _fresh();
            _fillCurve(g);
            g.graduate();
            n++;
        }
        emit log_named_uint("graduations before the module is nearly full", n);
        emit log_named_decimal_uint("ETH-backed units", wrapper.ethBacked(), 18);
        assertTrue(module.hasRoomFor(_nextBudget()) && factory.launchesOpen(), "room for exactly one more");

        // two curves launch while there is room for either; the first to close takes it
        (FrogCurve a,) = _launch(SweepVault.Policy.RAFFLE, false);
        (FrogCurve b, CurveToken tb) = _launch(SweepVault.Policy.BURN, false);
        _skipBoth(20 minutes);
        _fresh();
        vm.deal(buyer, 100 ether);
        vm.prank(buyer);
        b.buy{value: 1 ether}(0);                                // an early holder of b
        _fillCurve(a);
        assertTrue(a.canGraduate());
        a.graduate();

        // the module is now full for a budget: no new launch, and b cannot close
        assertFalse(factory.launchesOpen(), "the site shows launches paused");
        vm.prank(creator);
        vm.expectRevert(CurveFactory.QueueFull.selector);
        factory.launch("Late", "LATE", SweepVault.Policy.RAFFLE, false);
        (address sweepCoin,) = _launchSweepOnly();
        assertTrue(sweepCoin != address(0), "SWEEP_ONLY launches do not depend on the module");

        assertFalse(b.canClose());
        uint256 amount = b.raise() * 2;
        vm.deal(mallory, amount);
        vm.prank(mallory);
        vm.expectRevert(FrogCurve.QueueFull.selector);
        b.buy{value: amount}(0);
        assertEq(uint8(b.state()), uint8(FrogCurve.State.Trading), "still trading: nobody is bound to a refund");

        // b's holder is not stuck: it sells back to the curve at the curve's own price
        uint256 held = tb.balanceOf(buyer);
        vm.startPrank(buyer);
        tb.approve(address(b), held / 2);
        uint256 got = b.sell(held / 2, 0);
        vm.stopPrank();
        assertGt(got, 0);

        // room returns (the minter burns its units at the module) and b closes, filled at once
        vm.startPrank(trader);
        IERC20(address(wrapper)).approve(address(module), minted);
        module.burn(minted, 0, trader);
        vm.stopPrank();
        assertTrue(b.canClose());
        assertTrue(factory.launchesOpen());
        vm.deal(mallory, amount);
        vm.prank(mallory);
        b.buy{value: amount}(0);
        assertEq(uint8(module.grad(address(tb)).status), uint8(FloorModule.GradStatus.Filled), "filled at the close");
        b.graduate();
        assertEq(uint8(b.state()), uint8(FrogCurve.State.Graduated));
    }

    function _launchSweepOnly() internal returns (address coin, address curve) {
        vm.prank(creator);
        (coin, curve) = factory.launch("Sweep", "SWP", SweepVault.Policy.RAFFLE, true);
    }

    /// `hasRoomFor` counts the budgets already queued (a close at a stale floor waits Queued for `fill`) before any new one.
    function test_A03_theModuleCountsQueuedBudgets() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE, false);
        _skipBoth(module.FLOOR_TTL() + 1);
        _postCap(curveRaffle, CAP);
        _fillCurve(c);
        assertEq(uint8(module.grad(address(tok)).status), uint8(FloorModule.GradStatus.Queued));
        assertEq(module.gradEth(), c.itemBudget());
        uint256 unitPrice = module.gradPrice();
        uint256 roomEth = Math.mulDiv(module.maxEthBacked() - wrapper.ethBacked(), unitPrice, 1e18);
        assertTrue(module.hasRoomFor(roomEth - c.itemBudget() - 1e15));
        assertFalse(module.hasRoomFor(roomEth - c.itemBudget() + 1e15));
    }
}
