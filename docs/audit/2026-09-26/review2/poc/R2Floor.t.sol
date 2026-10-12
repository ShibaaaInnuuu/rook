// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "../floor/FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// Second review (R2). N-1: the module's exit is not live through a keeper incident, contrary to "burns are never paused" /
/// "holders can burn throughout" (FloorModule.burn NatSpec; fixes-B-C-keeper-draws.md §0, §5 item 16).
contract R2Floor is FloorBase {
    address guardian = makeAddr("guardian");
    address victim = makeAddr("victim");

    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
        p.maxItemsPerDay = 3;
    }

    function _tryBurn(address who, uint256 units) internal returns (bool ok, bytes memory err) {
        vm.startPrank(who);
        wrapper.approve(address(module), units);
        (ok, err) = address(module).call(abi.encodeCall(FloorModule.burn, (units, 0, who)));
        vm.stopPrank();
    }

    /// N-1a: a compromised (not yet paused) keeper keeps the module FRESH with good confidence and a raw `target` of 1 wei. Burns
    /// price at min(price(), target) so every burn pays 0 and reverts; since the module is fresh, `escaped()` never becomes true.
    /// Mints still work at the normal price, so new victims can walk in. Lasts until someone pauses the keeper.
    function test_N1a_freshTinyTargetFreezesEveryBurn_noEscapeEver() public {
        _mint(alice, 3 * module.mintPrice());
        uint256 units = wrapper.balanceOf(alice);
        assertGt(units, 2e18);
        for (uint256 i = 0; i < 8 * 24 + 2; ++i) {         // 8 days of posts, every 50 minutes
            _skip(50 minutes);
            _post(1);                                         // target = 1 wei; accepted barely moves (clamped); confidence 100%
        }
        assertTrue(module.fresh(), "fresh the whole time");
        assertFalse(module.escaped(), "the stale fallback never opens");
        assertEq(module.target(), 1);
        assertGt(module.accepted(), 0.05 ether, "accepted is still a real floor (it walks down 20%/day at most)");
        (uint256 p, uint256 room) = module.burnCapacity();
        assertEq(p, 0, "no burn price");
        assertEq(room, 0, "no burn room");
        assertEq(module.quoteBurn(1e18), 0);
        (bool ok, bytes memory err) = _tryBurn(alice, 1e18);
        assertFalse(ok, "burn refused");
        assertEq(bytes4(err), FloorModule.Slippage.selector, "pays 0 -> Slippage");
        // a victim can still mint at the normal price (max(price(), target))
        uint256 mp = module.mintPrice();
        assertGt(mp, 0.05 ether);
        uint256 got = _mint(victim, mp);
        assertGt(got, 0.9e18, "the module keeps taking ETH it will not pay back");
        (ok,) = _tryBurn(victim, got);
        assertFalse(ok);
        emit log_named_decimal_uint("accepted floor after 8 days", module.accepted(), 18);
        emit log_named_decimal_uint("ETH held for ETH-backed units, not burnable", module.backing() + module.reserve(), 18);
    }

    /// N-1b: the recovery path itself (pause -> rotate 7 days -> unpause) freezes burns for ~7 days: an hour after the pause the
    /// floor is stale, and the fallback waits FLOOR_TTL + STALE_ESCAPE from the last good post.
    function test_N1b_aKeeperPauseFreezesBurnsForAWeek() public {
        vm.prank(owner);
        registry.setGuardian(guardian);
        _mint(alice, 3 * module.mintPrice());
        _calm();
        uint64 lastGood = module.lastGoodAt();
        vm.prank(guardian);
        registry.pauseKeeper();
        (bool ok,) = _tryBurn(alice, 1e17);
        assertTrue(ok, "within the hour, burns work (what test_B09 checks)");
        _skip(61 minutes);
        bytes memory err;
        (ok, err) = _tryBurn(alice, 1e17);
        assertFalse(ok, "an hour later: refused");
        assertEq(bytes4(err), FloorModule.Stale.selector);
        _skip(3 days);
        (ok,) = _tryBurn(alice, 1e17);
        assertFalse(ok, "three days into the incident: still refused");
        _skip(6 days);
        assertTrue(module.escaped());
        (ok,) = _tryBurn(alice, 1e17);
        assertTrue(ok, "only after lastGood + 7 d + 1 h");
        assertEq(module.lastGoodAt(), lastGood);
    }
}
