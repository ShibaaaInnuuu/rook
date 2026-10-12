// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

// Round-3 re-review (2026-09-27-nft), part A. Runs from test/audit_nft_a3/ in a private copy of the repository (never the worktree).
// Each test that is named `..._weakness` PASSES while the weakness is present.

import {console2} from "forge-std/console2.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Base} from "../utils/Base.sol";
import {MockSeeder} from "../utils/CurveBase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {PosterSource} from "../../src/floor/PosterSource.sol";
import {EthNftWrapper, IListingPause} from "../../src/nft/EthNftWrapper.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {MintDesk} from "../../src/curve/MintDesk.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";
import {IItemQueue} from "../../src/interfaces/IItemQueue.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";

/// The helpers are test/ethnft/Pooled.t.sol's, copied (the stage-0 module defaults, a real factory and curve per collection).
contract AuditNftA3 is Base {
    struct Col {
        EthNftWrapper w;
        FloorModule m;
        CurveFactory f;
        MockSeeder s;
        uint256 floor;
    }

    address payee = makeAddr("payee (the canister's EVM address on 4663)");
    address platform = makeAddr("platform");
    address creator = makeAddr("creator");
    address buyer = makeAddr("curve buyer");
    uint256 constant ONE = 1e18;

    // ethflow.rs / relayconv.rs / ethnft.rs constants at HEAD 4949da0
    uint256 constant GAS_KEEP = 0.0002 ether;            // relayconv::GAS_KEEP_WEI (kept on 4663 from each advance)
    uint256 constant ETH_GAS_RESERVE = 0.003 ether;      // ethnft::ETH_GAS_RESERVE_WEI (kept on Ethereum by back_now)

    function _params(uint16 convBps, uint16 reserveBps) internal pure returns (FloorModule.Params memory p) {
        p.baseSpreadBps = 15; p.volWeightBps = 10_000; p.staleSpreadBps = 100; p.stressSpreadBps = 300; p.maxSpreadBps = 1_000;
        p.maxMoveBps = 500; p.convBps = convBps; p.bufferBps = 500; p.mintPerHour = 5e18; p.burnPerHour = 5e18;
        p.maxPublicEthBacked = 10e18; p.maxEthBacked = 40e18; p.revenueTarget = 1 ether; p.rentTerm = 30 days; p.premiumBps = 1_000;
        p.rentShareBps = 5_000; p.maxRentals = 20; p.lowConfidenceSpreadBps = 100; p.fullConfidenceBps = 8_000; p.minConfidenceBps = 3_000;
        p.maxItemsPerDay = 3; p.reserveBps = reserveBps; p.fillBandBps = 300;
    }

    function _template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = 1_000_000_000e18; c.reserved = 200_000_000e18;
        c.platformFeeCap = 0.25 ether; c.platformBps = 500; c.poolEthBps = 1_260; c.feeBps = 100;
        c.vaultShareBps = 4_000; c.creatorShareBps = 3_000; c.reserveShareBps = 500; c.liquidityShareBps = 500;
        c.window = 3 days; c.launchFeeBps = 5_000; c.launchFeeDecay = 10 minutes;
    }

    function _collection(address nft, uint256 floor, uint256 normal, uint256 early, uint16 convBps, uint16 reserveBps) internal returns (Col memory c) {
        c.floor = floor;
        c.w = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), 1, nft, custodian, "w", "w", PBS);
        c.w.initMintLimit(3);                                      // stage 0's limit, not the suite's 1,000
        PosterSource ps = new PosterSource(IKeeperSource(address(registry)), custodian);
        c.m = new FloorModule(IFloorWrapped(address(c.w)), IKeeperSource(address(ps)), payee, platform, address(0), _params(convBps, reserveBps), PBS);
        c.w.wireFloorModule(address(c.m));
        c.s = new MockSeeder(IERC20(address(c.w)));
        c.f = new CurveFactory(c.w, c.s, IItemQueue(address(c.m)), platform, _template(),
            CurveFactory.Targets(normal, early, 2 ether, 30 ether, 100, 800), PBS);
        MintDesk desk = new MintDesk(c.w, payee, 6 hours, PBS);
        SweepVault r = new SweepVault(registry, 0, c.w, SweepVault.Policy.RAFFLE, address(beacon), c.f, desk, PBS);
        SweepVault b = new SweepVault(registry, 0, c.w, SweepVault.Policy.BURN, address(beacon), c.f, desk, PBS);
        c.f.wire(r, b);
        c.m.wire(c.f);
        c.s.setDepth(10 ether);
        vm.prank(custodian);
        c.m.postFloor(floor, 10_000, 50);
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _post(Col memory c) internal {
        uint256 a = c.m.accepted();
        vm.prank(custodian);
        c.m.postFloor(a, 10_000, 50);
    }

    function _graduate(Col memory c) internal returns (uint256 units) {
        vm.prank(creator);
        (, address curve) = c.f.launch("Coin", "COIN", SweepVault.Policy.RAFFLE, false);
        FrogCurve fc = FrogCurve(payable(curve));
        uint256 net0 = c.m.netUnits();
        _skip(11 minutes);
        _post(c);
        uint256 pay = fc.raise() * 2;
        vm.deal(buyer, buyer.balance + pay);
        vm.prank(buyer);
        fc.buy{value: pay}(0);
        fc.graduate();
        units = c.m.netUnits() - net0;
    }

    function _books(Col memory c) internal view {
        assertEq(c.w.totalSupply(), (c.w.held() - c.w.openTickets()) * ONE + c.w.ethBacked(), "supply == (held - open) x ONE + ethBacked");
        assertGe(address(c.m).balance, c.m.backing() + c.m.reserve() + c.m.surplus() + c.m.revenue() + c.m.gradEth()
            + c.m.rentalEscrow() + c.m.renterPool() + c.m.owedTotal(), "the module holds every wei it owes");
        assertEq(c.m.netUnits() + c.m.reserveUnits() + c.m.transitUnits(), c.w.ethBacked(), "C + R + T = E");
    }

    /// ethflow::advance_covers, as the canister computes it (the same numbers as Pooled.t.sol's `_canisterAdvances`).
    function _covers(Col memory c, uint256 amount) internal view returns (bool) {
        uint256 accepted = c.m.accepted();
        uint256 arrive = amount - GAS_KEEP - amount * 50 / 10_000;
        return arrive >= accepted * 10_100 / 10_000 + ETH_GAS_RESERVE;
    }

    /// What reaches 4663 again after the canister's round trip of an advance of `amount` (the 4663 gas keep; Relay 4663 → 1 at
    /// `feeBps`; `back_now` keeps ETH_GAS_RESERVE on Ethereum; Relay 1 → 4663 at `feeBps`; `Returning { min(rise, value) }`).
    function _roundTrip(uint256 amount, uint256 feeBps) internal pure returns (uint256) {
        uint256 arrived = (amount - GAS_KEEP) * (10_000 - feeBps) / 10_000;
        uint256 ask = arrived - ETH_GAS_RESERVE;
        return ask * (10_000 - feeBps) / 10_000;
    }

    function _unitBacking(Col memory c) internal view returns (uint256) {
        return c.m.backing() * ONE / c.m.netUnits();
    }

    // =================================================================================================================================
    // A3-L-1: the way back has no bound of its own. Every round trip returns less than was advanced; the module takes the shortfall from revenue,
    // then surplus, then as a loss to the convertible units, and the canister starts the next buy of the same collection at once (no
    // cooldown, no attempt count across buys). What ends it is the A2-M-1 gate once the per-unit backing has fallen to about\n    // floor x 1.015; the module's own bound is maxItemsPerDay (3 advances a day).
    // =================================================================================================================================

    function test_A3_L1_repeatedWayBacks_eatTheBAYCPoolUntilTheGateStops_weakness() public {
        // BAYC as launched (9 ETH, the defaults), at CoinGecko's 2026-09-27 floor of 6.3 ETH: two graduations pool one ape
        Col memory bayc = _collection(makeAddr("BAYC"), 6.3 ether, 9 ether, 9 ether, 300, 2_500);
        _graduate(bayc);
        _graduate(bayc);
        _books(bayc);
        uint256 perUnit0 = _unitBacking(bayc);
        uint256 net0 = bayc.m.netUnits();
        uint256 value0 = bayc.m.backing() + bayc.m.revenue() + bayc.m.surplus();
        console2.log("start: netUnits", net0, "backing/unit", perUnit0);
        console2.log("       revenue", bayc.m.revenue(), "surplus", bayc.m.surplus());
        uint256 totalLoss;
        uint256 trips;
        for (uint256 i = 0; i < 20; ++i) {
            _post(bayc);
            uint256 max = bayc.m.maxItemCost();
            assertGt(max, 0, "the module wants one ape (the module itself would keep paying)");
            if (!_covers(bayc, max)) break;                   // the A2-M-1 gate: the only thing that ends the loop
            vm.prank(custodian);
            bayc.m.advance(max);
            // no listing under the bound for 48 h (or the operator's eth_give_back at once): the way back
            uint256 back = _roundTrip(max, 36);                 // Relay's measured 0.36 % each way
            vm.deal(custodian, back);
            vm.prank(custodian);
            bayc.m.returnAdvance{value: back}();
            totalLoss += max - back;
            ++trips;
            console2.log("trip", trips, max, back);
            _books(bayc);                                     // the unit ledger holds throughout
            _skip(8 hours + 1);                               // the module's own limit: 3 advances a day
        }
        uint256 value1 = bayc.m.backing() + bayc.m.revenue() + bayc.m.surplus();
        uint256 perUnit1 = _unitBacking(bayc);
        console2.log("trips before the gate refused", trips);
        console2.log("backing/unit after", perUnit1, "total cost", totalLoss);
        assertEq(bayc.m.netUnits(), net0, "nothing was bought");
        assertEq(bayc.w.held(), 0, "no ape");
        assertEq(value0 - value1, totalLoss, "every wei of every round trip came out of this collection's pool (revenue 0, surplus 0)");
        assertGe(trips, 4, "four round trips before the gate stops it");
        assertLt(perUnit1, bayc.m.accepted() * 10_200 / 10_000, "each unit is now backed at ~1.5 % over the floor, not the 3 % it was minted at");
        assertGt(totalLoss, 0.19 ether, "about 0.05 ETH a trip");
        // a new graduation tops the pool up at floor + 3 %: the gate opens again and the next trip may eat the new units' margin too
        _graduate(bayc);
        _post(bayc);
        assertTrue(_covers(bayc, bayc.m.maxItemCost()), "the next graduation re-opens the loop");
        _books(bayc);
    }

    /// The same round trip at the fee bound the judge accepts on each leg (relay_max_fee_bps 300): one trip costs ~6 % of the ape.
    function test_A3_L1_oneWayBackAtTheJudgesFeeBound_weakness() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.3 ether, 9 ether, 9 ether, 300, 2_500);
        _graduate(bayc);
        _graduate(bayc);
        uint256 perUnit0 = _unitBacking(bayc);
        uint256 max = bayc.m.maxItemCost();
        vm.prank(custodian);
        bayc.m.advance(max);
        uint256 back = _roundTrip(max, 300);
        vm.deal(custodian, back);
        vm.prank(custodian);
        bayc.m.returnAdvance{value: back}();
        console2.log("advanced", max, "returned", back);
        console2.log("backing/unit before", perUnit0, "after", _unitBacking(bayc));
        assertGt(max - back, 0.38 ether, "one round trip at 300 bps a leg");
        assertLt(_unitBacking(bayc), perUnit0);
        _books(bayc);
    }

    /// Control: the module accounts a short return exactly as documented (revenue, then surplus, then a shared loss), and an
    /// unpooled collection (Milady) behaves alike: the property at issue is the repetition, not the accounting.
    function test_A3_control_aShortReturnIsTakenFromRevenueThenSurplusThenTheUnits() public {
        Col memory milady = _collection(makeAddr("Milady"), 0.93 ether, 5 ether, 8 ether, 300, 2_500);
        _graduate(milady);
        uint256 rev0 = milady.m.revenue();
        uint256 sur0 = milady.m.surplus();
        uint256 backing0 = milady.m.backing();
        uint256 max = milady.m.maxItemCost();
        uint256 c = milady.m.netUnits();
        uint256 released = backing0 * ONE / c;
        vm.prank(custodian);
        milady.m.advance(max);
        uint256 revAfterAdvance = milady.m.revenue();
        uint256 back = _roundTrip(max, 36);
        vm.deal(custodian, back);
        vm.prank(custodian);
        milady.m.returnAdvance{value: back}();
        uint256 need = released - back;
        uint256 fromRev = need < revAfterAdvance ? need : revAfterAdvance;
        assertEq(milady.m.revenue(), revAfterAdvance - fromRev, "revenue first");
        uint256 left = need - fromRev;
        uint256 fromSur = left < sur0 ? left : sur0;
        assertEq(milady.m.surplus(), sur0 - fromSur, "then surplus");
        assertEq(milady.m.backing(), backing0 - (left - fromSur), "then the convertible units");
        console2.log("milady: revenue", rev0, "->", milady.m.revenue());
        _books(milady);
    }
    // =================================================================================================================================
    // A3-I-1: test/ethnft/Pooled.t.sol `test_aFloorRiseWhilePooled_…`'s last branch (`if (_canisterAdvances(bayc, max3)) { advance … }`)
    // never runs: at +10 % the third graduation does not lift the blend over the gate, so the test's "once it covers the floor the
    // canister advances it" is asserted nowhere. The same steps, with the same numbers:
    // =================================================================================================================================

    function _walk(Col memory c, uint256 to) internal {
        for (uint256 i = 0; i < 100 && c.m.accepted() != to; ++i) {
            _skip(1 hours);
            vm.prank(custodian);
            c.m.postFloor(to, 10_000, 50);
        }
        assertEq(c.m.accepted(), to);
    }

    function test_A3_I1_pooledFloorRiseTest_theThirdGraduationBranchIsDead() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.79 ether, 9 ether, 9 ether, 300, 2_500);
        _graduate(bayc);
        _walk(bayc, 7.469 ether);
        _graduate(bayc);
        assertFalse(_covers(bayc, bayc.m.maxItemCost()), "after the second: waits (as the test asserts)");
        _graduate(bayc);
        uint256 max3 = bayc.m.maxItemCost();
        console2.log("third graduation: maxItemCost", max3, "netUnits", bayc.m.netUnits());
        assertFalse(_covers(bayc, max3), "after the third: STILL refused, so Pooled.t.sol's `if` body never runs");
        uint256 n = 3;
        while (!_covers(bayc, bayc.m.maxItemCost()) && n < 12) { _graduate(bayc); ++n; }
        console2.log("graduations until the gate lets the pool convert", n);
        assertGt(n, 3);
    }
}
