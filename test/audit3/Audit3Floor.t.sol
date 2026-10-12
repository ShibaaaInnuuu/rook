// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "../floor/FloorBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";

/// R3 check of `exitAtBacking()` (N-1b), now a regression (docs/audit/2026-09-26/review3/fixes-R4-contracts.md §1): a keeper pause
/// prices burns at the floor the keeper left (`pauseFloor()`), never above the backing per unit, so a unit minted before the pause
/// never exits above its cost. The backing per unit itself is paid only once `escaped()`.
abstract contract Audit3FloorCases is FloorBase {
    address guardian = makeAddr("guardian");
    address attacker = makeAddr("attacker");

    function setUp() public virtual override {
        super.setUp();
        vm.prank(owner);
        registry.setGuardian(guardian);
    }

    function _post(uint256 price, uint16 confidence) internal {
        vm.prank(keeper);
        module.postFloor(price, confidence, depth);
    }

    /// A random keeper history: calm posts, a fall or a rise at random speeds (targets from half to twice the floor), a 1-wei
    /// post, a fill (a conversion at a random cost under the cap), a low-confidence post, a held-back fall, random delays.
    function _history(uint256 seed, uint256 steps) internal {
        for (uint256 i = 0; i < steps; ++i) {
            uint256 r = uint256(keccak256(abi.encode(seed, i)));
            _skip(bound(r, 5 minutes, module.FLOOR_TTL() + 10 minutes));   // past the TTL too: the stale branch
            uint256 a = module.accepted();
            uint256 kind = (r >> 64) % 6;
            if (kind == 0) _post(a, 10_000);
            else if (kind == 1) _post(a * bound(r >> 96, 5_000, 20_000) / 10_000, 10_000);
            else if (kind == 2) _post(1, 10_000);
            else if (kind == 3) _post(a * bound(r >> 96, 5_000, 12_000) / 10_000, uint16(bound(r >> 160, 0, 10_000)));
            else if (kind == 4) {
                _post(a * bound(r >> 96, 8_000, 11_000) / 10_000, 10_000);
                uint256 max = module.maxItemCost();
                if (module.fresh() && module.netUnits() >= 2e18 && max != 0) _convert(bound(r >> 128, max / 3 + 1, max));
            } else {
                _post(a * 7 / 10, 10_000);                           // a fall the move limit holds back: target < accepted
            }
        }
    }

    /// The reviewer's PoC, re-run: an honest 20 % fall, an outsider mints at the new floor, then a guardian pause. The burn during
    /// the pause now pays the frozen floor less the base spread, not the diluted average backing: the outsider loses the spreads.
    function test_audit3_mintLowThenBurnDuringAPauseNoLongerProfits() public {
        _calm();
        _mint(alice, 3 * module.mintPrice());
        _walkTo(F * 8 / 10);                                        // an honest fall, within the day band
        _calm();
        _calm();
        uint256 paid = 4 * module.mintPrice();
        uint256 units = _mint(attacker, paid);
        assertGt(module.beta(), module.accepted(), "the average backing sits above the fallen floor: the attack's lever");

        vm.prank(guardian);
        registry.pauseKeeper();
        assertFalse(module.exitAtBacking(), "a pause is not the stale fallback");
        assertTrue(module.burnsOpen(), "burns stay open through the pause");
        assertEq(module.pauseFloor(), module.accepted() * (10_000 - 300) / 10_000,
            "an ordinary keeper: the pause floor is the accepted floor it left, at the fill band's low edge (R4 check A4-L1)");
        uint256 quoted = module.quoteBurn(units);
        uint256 kept0 = module.revenue() + module.surplus();
        uint256 got = _burn(attacker, units);
        assertEq(got, quoted, "the quote is exact");
        emit log_named_decimal_uint("attacker paid", paid, 18);
        emit log_named_decimal_uint("attacker got (paused burn)", got, 18);
        assertLt(got, paid, "the round trip through a pause loses the spreads (was +0.206 ETH)");
        assertLe(got, units * module.pauseFloor() / 1e18, "never above the frozen floor");
        assertGt(module.revenue() + module.surplus(), kept0, "the cushion above the floor stays in the pots");

        // the week-long stale fallback still pays the backing, as it always did (B-04), and only then
        _skip(module.STALE_ESCAPE() + module.FLOOR_TTL() + 1 hours);
        assertTrue(module.escaped());
        assertTrue(module.exitAtBacking());
        assertEq(module.pauseFloor(), 0, "escaped: no floor is read");
        uint256 b = module.reserveUnits() != 0 ? module.betaReserve() : module.betaConvertible();
        assertEq(module.burnPrice(), b * (10_000 - module.baseSpreadBps()) / 10_000, "escaped: the backing less the base spread");
        _books();
    }

    /// The fill band's gap: in a fall the move limit is holding back (target below accepted) with fills below the floor, a mint
    /// costs less than `accepted`. Paying `accepted` during a pause would still leave a profit; the pause floor follows the
    /// mint's own lower bound instead.
    function test_audit3_theFillBandGapIsClosedToo() public {
        _calm();
        _mint(alice, 3 * module.mintPrice());
        _calm();
        _convert(module.accepted() * 80 / 100);                     // a cheap purchase: the fills price falls toward it
        for (uint256 i = 0; i < 5; ++i) _calm();
        _skip(1 hours);
        _post(module.accepted() / 2);                               // the keeper wants half: the move limit holds it at -5 %
        uint256 a = module.accepted();
        assertLt(module.target(), a);
        _skip(59 minutes);                                          // let the volatility decay: the cheapest mint of the hour
        assertEq(module.price(), a * (10_000 - 300) / 10_000, "the fills price sits at the band's low edge");
        uint256 mp = module.mintPrice();
        assertLt(mp, a * (10_000 - 15) / 10_000, "a mint costs less than a burn at the accepted floor would pay");
        uint256 units = _mint(attacker, mp);

        vm.prank(guardian);
        registry.pauseKeeper();
        uint256 pf = module.pauseFloor();
        assertLt(pf, a, "the pause floor is below accepted in a held-back fall");
        assertLe(pf, a * (10_000 - 300) / 10_000);
        uint256 got = _burn(attacker, units);
        assertLt(got, mp, "no profit through the band's gap");
        _books();
    }

    /// A 1-wei post right before the pause cannot zero a paused burn either (N-1a still holds).
    function test_audit3_aTinyPostBeforeThePauseCannotZeroIt() public {
        _mint(alice, 3 * module.mintPrice());
        _skip(50 minutes);
        _post(1, 10_000);
        vm.prank(owner);
        registry.pauseKeeper();
        uint256 a = module.accepted();
        assertGe(module.pauseFloor(), a * (10_000 - 500) / 10_000, "at least the next hour's legal floor");
        (uint256 p, uint256 room) = module.burnCapacity();
        assertGt(p, 0);
        assertGt(room, 0);
        assertGt(_burn(alice, 1e18), 0);
    }

    /// The pause floor only falls while the pause lasts (the day anchor rolling over is the only moving part), and burns are
    /// never refused inside the stale window.
    function test_audit3_thePauseFloorNeverRisesDuringAPause() public {
        _mint(alice, 3 * module.mintPrice());
        _walkTo(F * 85 / 100);
        _post(F / 2, 10_000);
        vm.prank(owner);
        registry.pauseKeeper();
        uint256 last = module.pauseFloor();
        for (uint256 i = 0; i < 14; ++i) {
            _skip(11 hours);
            uint256 now_ = module.pauseFloor();
            assertLe(now_, last, "never rises");
            assertGt(now_, 0);
            assertTrue(module.burnsOpen());
            last = now_;
        }
        assertGt(_burn(alice, 1e17), 0);
    }

    /// Unpause timing: a pause lifted within the hour leaves the module fresh. A burn in the pause and a mint after it lose; after a
    /// long pause the module is stale until the keeper's first post, so nothing is priced by the frozen floor after the unpause.
    function test_audit3_aShortPauseAndUnpauseIsNoLever() public {
        _mint(alice, 3 * module.mintPrice());
        _walkTo(F * 8 / 10);
        _calm();
        uint256 paid = module.mintPrice();
        uint256 units = _mint(attacker, paid);
        vm.prank(owner);
        registry.pauseKeeper();
        uint256 got = _burn(attacker, units);
        vm.prank(owner);
        registry.unpauseKeeper();
        assertTrue(module.fresh(), "lifted within the hour: still fresh");
        uint256 back = _mint(attacker, got);
        assertLt(back, units, "burn in the pause, mint after it: fewer units than before");
        assertLt(got, paid);
        vm.prank(owner);
        registry.pauseKeeper();
        _skip(2 days);
        vm.prank(owner);
        registry.unpauseKeeper();
        assertFalse(module.burnsOpen(), "stale and not paused: the ordinary outage until the first post");
        vm.deal(attacker, 1 ether);
        vm.prank(attacker);
        vm.expectRevert(FloorModule.Stale.selector);
        module.mint{value: 1 ether}(attacker, 0);
        _books();
    }

    /// The keeper that posts at confidence 0 (A3-I1, documented and unchanged): burns wait for a pause or the week, and the pause
    /// now ends the freeze at the pause floor, not at the backing.
    function test_audit3_confidenceZeroPostsFreezeBurnsUntilAPause() public {
        _mint(alice, 2 * module.mintPrice());
        _calm();
        conf = 0;
        for (uint256 i = 0; i < 6 * 24; ++i) {                       // six days of confidence-0 posts
            _skip(50 minutes);
            _post(F);
        }
        assertFalse(module.fresh());
        assertFalse(module.burnsOpen(), "not paused, not yet escaped");
        (uint256 p, uint256 room) = module.burnCapacity();
        assertEq(p + room, 0, "burns frozen for six days and counting");
        vm.prank(guardian);
        registry.pauseKeeper();
        (p, room) = module.burnCapacity();
        assertGt(p, 0, "the pause ends it");
        assertLe(p, module.pauseFloor(), "at the pause floor");
    }

    /// Fuzz: whatever the keeper did before, a mint, a delay, maybe one more keeper post (R4 check A4-L1: any target from half to
    /// twice the floor, any confidence), a pause of up to six days and a burn of everything never profits, and a paused burn never
    /// pays above the frozen floor less the base spread.
    function testFuzz_audit3_aPauseNeverPaysAMintBurnRoundTrip(uint256 seed, uint256 amount, uint256 delay, uint256 hold) public {
        _calm();
        _mint(alice, 3 * module.mintPrice());
        _history(seed, bound(seed >> 200, 0, 14));
        if (!module.fresh()) _post(module.accepted(), 10_000);
        _skip(bound(delay, 0, 50 minutes));
        (uint256 mp, uint256 maxIn) = module.mintCapacity();
        if (mp == 0 || maxIn < 1e12) return;
        uint256 paid = bound(amount, 1e12, maxIn);
        uint256 units = _mint(attacker, paid);

        _skip(bound(delay >> 64, 0, 3 hours));
        if ((seed >> 250) % 2 == 1) {
            // A4-L1: a post between the mint and the pause (the target swinging back, or the accepted floor moving either way)
            uint256 r = uint256(keccak256(abi.encode(seed, "between")));
            _post(module.accepted() * bound(r, 5_000, 20_000) / 10_000, uint16(bound(r >> 64, 0, 10_000)));
            _skip(bound(r >> 128, 0, 50 minutes));
        }
        vm.prank(guardian);
        registry.pauseKeeper();
        _skip(bound(hold, 0, 6 days));
        assertFalse(module.escaped(), "inside the stale window");
        uint256 pf = module.pauseFloor();
        assertGt(pf, 0, "a paused burn always has a price");
        (, uint256 room) = module.burnCapacity();
        assertGe(room, units, "within the hourly cap");
        uint256 got = _burn(attacker, units);
        assertLe(got, paid, "a pause never pays a mint-burn round trip");
        assertLe(got, units * pf / 1e18 * (10_000 - module.baseSpreadBps()) / 10_000 + 2, "never above the frozen floor");
        _books();
    }
}

/// The PoC's parameters (the reserve share 0: the convertible tier alone).
contract Audit3Floor is Audit3FloorCases {}

/// The Phase-1 reserve share (25 %): the reserve tier is capped the same way.
contract Audit3FloorReserve is Audit3FloorCases {
    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
    }
}
