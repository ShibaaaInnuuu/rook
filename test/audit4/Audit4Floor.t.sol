// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "../floor/FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// R4 check A4-L1, now a regression (docs/audit/2026-09-26/review4/fixes-golive-lows.md §1). The reviewer's PoC
/// (review4/poc/Audit4Floor.t.sol) made +1.1 % (+1.4 % with the reserve) by minting under a low target at the fills price, letting
/// the keeper's next post put the target back at the accepted floor, then any pause: `F_p` was `accepted`. The pause floor is now
/// capped at the fill band's low edge and at the least a public mint paid before its spread in the 12 to 24 hours before the pause,
/// so the same moves lose, and so does a mint before a rise of the accepted floor itself.
abstract contract Audit4FloorCases is FloorBase {
    address guardian = makeAddr("guardian");
    address attacker = makeAddr("attacker");

    function setUp() public virtual override {
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

    /// The reviewer's PoC, re-run: the target dips, the outsider mints at the fills price, the keeper's next post puts the target
    /// back at the accepted floor (accepted moves < 0.1 %), then a guardian pause. The paused burn now loses.
    function test_audit4_targetSwingThenPauseNoLongerPaysTheFillBand() public {
        uint256 a = _setUpLowFills();
        _skip(1 minutes);
        _postC(a * 90 / 100);                                          // the target dips: mint at max(price(), target)
        uint256 paid = 2 * module.mintPrice();
        uint256 units = _mint(attacker, paid);

        _skip(1 minutes);
        _postC(a);                                                     // the target recovers; accepted ~ unchanged
        assertLt(module.quoteBurn(units), paid, "fresh: no profit");

        vm.prank(guardian);
        registry.pauseKeeper();
        uint256 pf = module.pauseFloor();
        assertLe(pf, module.accepted() * (10_000 - 300) / 10_000, "at most the fill band's low edge");
        assertLe(pf * units / 1e18, paid, "never above what the mint paid, spread aside");
        uint256 got = _burn(attacker, units);
        emit log_named_decimal_uint("attacker paid", paid, 18);
        emit log_named_decimal_uint("attacker got (paused burn)", got, 18);
        assertLt(got, paid, "a pause no longer pays a round trip across two posts of the same floor (was +1.1 %, +1.4 %)");
        _books();
    }

    /// The control: the pause while the low target still stands (one post) was already a loss, and still is.
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

    /// The same shape through the accepted floor itself: a calm mint, then an hour later a post that raises accepted by the whole
    /// hour's move limit (5 %), then a pause. The mint's own price caps the pause floor (the mint low), so it loses too.
    function test_audit4_aRiseOfTheAcceptedFloorBetweenTheMintAndThePauseLoses() public {
        _calm();
        _mint(alice, 3 * module.mintPrice());
        _calm();
        _calm();
        uint256 a0 = module.accepted();
        uint256 paid = 2 * module.mintPrice();
        uint256 units = _mint(attacker, paid);
        _skip(1 hours);
        _postC(a0 * 2);                                                // held to +5 %
        assertGt(module.accepted(), a0 * 104 / 100, "accepted rose by the hour's move limit");
        vm.prank(guardian);
        registry.pauseKeeper();
        uint256 pf = module.pauseFloor();
        assertLe(pf, a0, "capped by the mint's own price, not the risen floor");
        uint256 got = _burn(attacker, units);
        assertLt(got, paid, "mint, a rise, a pause, a burn: a loss");
        _books();
    }

    /// With no mint in the lookback, an ordinary keeper's pause floor is the band's low edge; a mint in the lookback lowers it to
    /// what that mint paid, and nothing changes it while the pause lasts (nobody can mint or post).
    function test_audit4_thePauseFloorIsTheBandEdgeOrTheCheapestRecentMint() public {
        _mint(alice, 3 * module.mintPrice());
        _skip(1 days);
        _post(module.accepted());
        _calm();
        uint256 a = module.accepted();
        vm.prank(owner);
        registry.pauseKeeper();
        assertEq(module.pauseFloor(), a * (10_000 - 300) / 10_000, "no mint in the lookback: the band's low edge");
        vm.prank(owner);
        registry.unpauseKeeper();
        _calm();
        _postC(a * 90 / 100);                                          // the target dips under the band
        _skip(1 minutes);
        uint256 base = module.mintPrice() * 10_000 / (10_000 + _sMint());
        _mint(attacker, module.mintPrice());
        _calm();
        _postC(a);
        vm.prank(owner);
        registry.pauseKeeper();
        uint256 pf = module.pauseFloor();
        assertLe(pf, base + 1, "the cheapest recent mint caps it");
        for (uint256 i = 0; i < 12; ++i) {
            _skip(11 hours);
            assertLe(module.pauseFloor(), pf, "never rises during the pause");
            assertGt(module.pauseFloor(), 0);
        }
    }

    function _sMint() internal view returns (uint256 s) {
        (s,) = module.spreads();
    }
}

/// The PoC's parameters (the reserve share 0: the convertible tier alone).
contract Audit4Floor is Audit4FloorCases {}

/// The Phase-1 reserve share (25 %).
contract Audit4FloorReserve is Audit4FloorCases {
    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
    }
}
