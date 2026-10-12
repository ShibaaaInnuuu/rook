// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "../floor/FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// R4 check of the pause floor `F_p = min(burnTarget, max(a(1-fb), min(target, a)))`.
/// The bound "no mint ever paid less than F_p" holds for mints under the LAST post only. A mint under an EARLIER post of the same
/// accepted floor, made while the target sat below `a` and the fills price sat at the band's low edge, costs `a(1-fb)(1+sMint)`;
/// if the keeper's next post puts the target back at or above `a`, F_p = a and a pause pays `a(1-base)` for it. The fresh burn at
/// that same state pays `price()(1-sBurn) = a(1-fb)(1-sBurn)`: the pause is worth ~fb to a pre-positioned outsider.
contract Audit4Floor is FloorBase {
    address guardian = makeAddr("guardian");
    address attacker = makeAddr("attacker");

    function setUp() public override {
        super.setUp();
        vm.prank(owner);
        registry.setGuardian(guardian);
    }

    function _postC(uint256 p) internal {
        vm.prank(keeper);
        module.postFloor(p, 10_000, depth);
    }

    function _setUpLowFills() internal returns (uint256 a) {
        _calm();
        _mint(alice, 5 * module.mintPrice());
        _calm();
        _calm();
        // an honest conversion bought cheap (the market dipped under the keeper's floor): a low fill
        _convert(module.accepted() / 2);
        // three calm hours: the fills TWAP falls under the band, so price() = a(1 - fb)
        for (uint256 i = 0; i < 3; ++i) _calm();
        a = module.accepted();
        assertEq(module.price(), a * (10_000 - 300) / 10_000, "fills price at the band's low edge");
    }

    /// The target dips (held-back fall) one minute after a calm post, the outsider mints at the fills price, the keeper's next post
    /// puts the target back at the accepted floor, then any pause. Accepted moved by < 0.1 % in between.
    function test_audit4_targetSwingThenPausePaysTheFillBand() public {
        uint256 a = _setUpLowFills();
        _skip(1 minutes);
        _postC(a * 90 / 100);                                          // the target dips: mint at max(price(), target)
        uint256 mp = module.mintPrice();
        emit log_named_decimal_uint("accepted", module.accepted(), 18);
        emit log_named_decimal_uint("mint price (per unit)", mp, 18);
        uint256 paid = 2 * mp;
        uint256 units = _mint(attacker, paid);

        _skip(1 minutes);
        _postC(a);                                                     // the target recovers; accepted ~ unchanged
        uint256 freshQuote = module.quoteBurn(units);
        emit log_named_decimal_uint("fresh burn quote (no pause)", freshQuote, 18);
        assertLt(freshQuote, paid, "fresh: no profit");

        vm.prank(guardian);
        registry.pauseKeeper();
        emit log_named_decimal_uint("pause floor", module.pauseFloor(), 18);
        uint256 got = _burn(attacker, units);
        emit log_named_decimal_uint("attacker paid", paid, 18);
        emit log_named_decimal_uint("attacker got (paused burn)", got, 18);
        if (got > paid) emit log_named_decimal_uint("PROFIT", got - paid, 18);
        else emit log_named_decimal_uint("loss", paid - got, 18);
        assertGt(got, paid, "a pause pays a round trip across two posts of the same floor");
        _books();
    }

    /// Same, but the pause comes while the target is still low (single post): no profit (the documented bound).
    function test_audit4_control_samePost() public {
        uint256 a = _setUpLowFills();
        _skip(1 minutes);
        _postC(a * 90 / 100);
        uint256 paid = 2 * module.mintPrice();
        uint256 units = _mint(attacker, paid);
        vm.prank(guardian);
        registry.pauseKeeper();
        uint256 got = _burn(attacker, units);
        assertLt(got, paid, "control: same post, no profit");
    }
}

/// The Phase-1 reserve share (25 %).
contract Audit4FloorReserve is Audit4Floor {
    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
    }
}
