// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "../floor/FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// R3 check of `exitAtBacking()` (N-1b): pause/unpause timing and who pays.
contract Audit3Floor is FloorBase {
    address guardian = makeAddr("guardian");
    address attacker = makeAddr("attacker");

    function setUp() public override {
        super.setUp();
        vm.prank(owner);
        registry.setGuardian(guardian);
    }

    /// An honest 20 % fall of the floor (the keeper following the market), an outsider mints at the new floor while the module is
    /// fresh, then ANY pause (a guardian false positive, the monitor's unverified-draw deadline pause) lets it burn at the diluted
    /// average backing per unit. Measures the outsider's profit and the existing holder's loss of backing per unit.
    function test_audit3_mintLowThenBurnAtBackingDuringAPause() public {
        _calm();
        uint256 aliceUnits = _mint(alice, 3 * module.mintPrice());
        uint256 beta0 = module.betaConvertible();

        _walkTo(F * 8 / 10);                                        // an honest fall, within the day band
        _calm();
        _calm();
        uint256 paid = 4 * module.mintPrice();
        uint256 units = _mint(attacker, paid);
        uint256 beta1 = module.betaConvertible();
        emit log_named_decimal_uint("beta before (alice's backing / unit)", beta0, 18);
        emit log_named_decimal_uint("beta after attacker's mint", beta1, 18);

        // control: without a pause the round trip loses the spreads
        uint256 quoteFresh = module.quoteBurn(units);
        assertLt(quoteFresh, paid, "fresh: no profit");

        vm.prank(guardian);
        registry.pauseKeeper();
        assertTrue(module.exitAtBacking());
        uint256 got = _burn(attacker, units);
        emit log_named_decimal_uint("attacker paid", paid, 18);
        emit log_named_decimal_uint("attacker got (paused burn)", got, 18);
        emit log_named_decimal_uint("attacker profit", got - paid, 18);
        uint256 beta2 = module.betaConvertible();
        emit log_named_decimal_uint("alice's backing / unit after", beta2, 18);
        emit log_named_decimal_uint("alice's loss of backing", (beta0 - beta2) * aliceUnits / 1e18, 18);
        assertGt(got, paid, "an outsider profits from the pause");
        assertLt(beta2, beta0, "at the existing holders' expense");
        _books();
    }

    /// A compromised keeper that simply stops posting a good floor (confidence under the minimum) still freezes every burn until
    /// someone pauses it, for up to FLOOR_TTL + STALE_ESCAPE: the R3 fix makes the pause end the freeze, not the freeze itself.
    function test_audit3_confidenceZeroPostsFreezeBurnsUntilAPause() public {
        _mint(alice, 2 * module.mintPrice());
        _calm();
        conf = 0;
        for (uint256 i = 0; i < 6 * 24; ++i) {                       // six days of confidence-0 posts
            _skip(50 minutes);
            _post(F);
        }
        assertFalse(module.fresh());
        assertFalse(module.exitAtBacking(), "not paused, not yet escaped");
        (uint256 p, uint256 room) = module.burnCapacity();
        assertEq(p + room, 0, "burns frozen for six days and counting");
        vm.prank(guardian);
        registry.pauseKeeper();
        (p, room) = module.burnCapacity();
        assertGt(p, 0, "the pause ends it");
    }
}
