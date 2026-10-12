// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "../floor/FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// Second review (R2) N-1, now a regression (docs/audit/2026-09-26/review2/fixes-R3-contracts.md §1): the module's exit is live
/// through a keeper incident. (a) A fresh keeper posting a 1-wei target cannot zero burns: the burn side counts the target no lower
/// than the floor could legally reach within the next hour. (b) While the keeper is paused, burns go on at once, at the lower of the
/// backing per unit and the floor the keeper left (`pauseFloor()`, R3 check A3-M1) less the base spread, and nothing can be minted
/// into that price; the backing per unit alone once the stale window has passed.
contract R2Floor is FloorBase {
    address guardian = makeAddr("guardian");
    address victim = makeAddr("victim");

    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
        p.maxItemsPerDay = 3;
    }

    function _tryBurn(address who, uint256 units) internal returns (bool ok, bytes memory err, uint256 got) {
        vm.startPrank(who);
        wrapper.approve(address(module), units);
        bytes memory ret;
        (ok, ret) = address(module).call(abi.encodeCall(FloorModule.burn, (units, 0, who)));
        vm.stopPrank();
        if (ok) got = abi.decode(ret, (uint256));
        else err = ret;
    }

    /// The least a fresh burn may pay per whole unit under a hostile target: one hour's legal move below the accepted floor (or
    /// the day band), the fills price's band, less the widest spread.
    function _freshFloor() internal view returns (uint256) {
        uint256 a = module.accepted();
        uint256 lo = a * (10_000 - 500) / 10_000;
        uint256 p = a * (10_000 - 300) / 10_000;                         // price() is within fillBandBps of accepted
        return (lo < p ? lo : p) * (10_000 - 1_000) / 10_000;
    }

    /// N-1a: 8 days of 1-wei posts at full confidence. The module stays fresh and never escapes, as before, but every burn pays
    /// at least the next hour's legal floor less the spread, and a victim who minted can burn back.
    function test_N1a_aTinyTargetNoLongerFreezesBurns() public {
        _mint(alice, 3 * module.mintPrice());
        uint256 units = wrapper.balanceOf(alice);
        assertGt(units, 2e18);
        for (uint256 i = 0; i < 8 * 24 + 2; ++i) {
            _skip(50 minutes);
            _post(1);
            if (i % 24 == 0) {
                (bool ok0,, uint256 got0) = _tryBurn(alice, 1e16);
                assertTrue(ok0, "a burn goes through on every day of the attack");
                assertGe(got0 * 100, _freshFloor(), "and pays at least the bounded floor");
            }
        }
        assertTrue(module.fresh(), "fresh the whole time");
        assertFalse(module.escaped());
        assertEq(module.target(), 1);
        (uint256 p, uint256 room) = module.burnCapacity();
        assertGe(p, _freshFloor(), "a burn price bounded below by the next hour's legal floor");
        assertGt(room, 0);
        assertGt(module.quoteBurn(1e18), 0);
        (bool ok,, uint256 got) = _tryBurn(alice, 1e18);
        assertTrue(ok, "burn accepted");
        assertGe(got, _freshFloor());

        // a victim who mints now can burn back, losing at most the hour's move and the spreads
        uint256 mp = module.mintPrice();
        uint256 minted = _mint(victim, mp);
        (ok,, got) = _tryBurn(victim, minted);
        assertTrue(ok, "the victim exits");
        assertGe(got * 10_000, mp * 7_900, "at most ~5% move + spreads lost on the round trip");
        assertLt(got, mp, "never a profit");
        _books();
    }

    /// N-1b: pause, then burns work at +0, +3 h 1 min, +3 days and +9 days: inside the stale window at the lower of the backing per
    /// unit and the pause floor less the base spread (A3-M1), after it at the backing per unit less the base spread; mints, floats
    /// and conversions stay refused, so nobody can buy into that price; the stale fallback's skim does not open early.
    function test_N1b_burnsGoOnThroughAKeeperPause() public {
        vm.prank(owner);
        registry.setGuardian(guardian);
        _mint(alice, 3 * module.mintPrice());
        _calm();
        vm.prank(guardian);
        registry.pauseKeeper();
        assertTrue(module.burnsOpen(), "paused: burns go on");
        assertFalse(module.exitAtBacking(), "paused: at the pause floor, not the backing");
        assertEq(module.pauseFloor(), module.accepted() * (10_000 - 300) / 10_000, "the floor the keeper left, at the fill band's low edge (R4 check A4-L1)");
        assertFalse(module.escaped(), "not the stale window: skim's sweep stays shut");

        uint256[4] memory at = [uint256(0), module.FLOOR_TTL() + 1 minutes, 3 days, 6 days];
        for (uint256 i = 0; i < at.length; ++i) {
            _skip(at[i]);
            uint256 beta = module.betaReserve();
            bool esc = module.escaped();
            uint256 unit = esc ? beta : (beta < module.pauseFloor() ? beta : module.pauseFloor());
            (bool ok,, uint256 got) = _tryBurn(alice, 1e17);
            assertTrue(ok, "burns work throughout the pause");
            assertEq(got, (unit * (10_000 - 15) / 10_000) * 1e17 / 1e18, "at the pause floor (or, escaped, the backing) less the base spread");
            assertEq(esc, i == 3, "escaped only after the week");
        }
        // nothing can be minted into it
        vm.deal(victim, 10 ether);
        vm.prank(victim);
        vm.expectRevert(FloorModule.Stale.selector);
        module.mint{value: 1 ether}(victim, 0);
        (uint256 mPrice, uint256 mRoom) = module.mintCapacity();
        assertEq(mPrice + mRoom, 0, "no mint capacity while paused");
        assertEq(module.itemCap(), 0, "no conversion priced by the keeper while paused");
        vm.expectRevert(FloorModule.CapStale.selector);
        module.snapshotCap();
        _books();
    }

    /// Minting is refused while paused even inside the first hour, when the floor is still fresh (the pause, not staleness).
    function test_N1b_noMintIntoTheBackingPriceEvenWhileFresh() public {
        _mint(alice, 3 * module.mintPrice());
        vm.prank(owner);
        registry.pauseKeeper();
        assertTrue(module.fresh());
        vm.deal(victim, 10 ether);
        vm.prank(victim);
        vm.expectRevert(FloorModule.Paused.selector);
        module.mint{value: 1 ether}(victim, 0);
        assertEq(module.itemCap(), 0);
        vm.expectRevert(FloorModule.CapStale.selector);
        module.snapshotCap();
    }

    /// N-1b within the first hour: the floor is still fresh but the keeper is paused, so a hostile last post cannot zero the
    /// burns: they pay the pause floor (no lower than the next hour's legal floor) less the base spread (A3-M1).
    function test_N1b_aPauseRightAfterAHostilePostPaysThePauseFloor() public {
        _mint(alice, 3 * module.mintPrice());
        _skip(50 minutes);
        _post(1);
        (uint256 hostile,) = module.burnCapacity();
        vm.prank(owner);
        registry.pauseKeeper();
        assertTrue(module.fresh());
        (uint256 paused,) = module.burnCapacity();
        uint256 b = module.betaReserve();
        uint256 pf = module.pauseFloor();
        assertGe(pf, module.accepted() * (10_000 - 500) / 10_000, "a 1-wei post cannot lower it past the hour's legal move");
        assertEq(paused, (b < pf ? b : pf) * (10_000 - 15) / 10_000);
        assertGe(paused, hostile, "the pause never pays less than the hostile floor");
        (bool ok,,) = _tryBurn(alice, 1e18);
        assertTrue(ok);
    }

    /// The pause is lifted: the module is stale until the next post (an ordinary outage), then prices burns by the floor again.
    function test_N1b_afterTheUnpauseTheFloorPricesAgain() public {
        _mint(alice, 3 * module.mintPrice());
        vm.prank(owner);
        registry.pauseKeeper();
        _skip(2 days);
        vm.prank(owner);
        registry.unpauseKeeper();
        assertFalse(module.exitAtBacking());
        (bool ok, bytes memory err,) = _tryBurn(alice, 1e17);
        assertFalse(ok, "stale and not paused: an ordinary outage");
        assertEq(bytes4(err), FloorModule.Stale.selector);
        _post(module.accepted());
        (ok,,) = _tryBurn(alice, 1e17);
        assertTrue(ok);
    }
}
