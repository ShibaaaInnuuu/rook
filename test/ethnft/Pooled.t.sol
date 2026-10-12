// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

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

/// docs/v7/phase1-nft-pooled.md §3: pooled graduations are the Floor Module's own behaviour. Real curves, a real factory per
/// collection with its own targets, the module with the stage-0 defaults (25 % reserve, 3 % conversion, 5 % buffer). A graduation
/// whose budget is under one item leaves its units ETH-backed at its own module; the next graduation of the SAME collection
/// completes the pool; one advance buys one item; the ledger and the books hold throughout, and nothing crosses collections.
contract PooledGraduationTest is Base {
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
    address alice = makeAddr("alice");
    uint256 constant ONE = 1e18;
    uint256 nextToken = 1;

    function _params(uint16 convBps, uint16 reserveBps) internal pure returns (FloorModule.Params memory p) {
        p.baseSpreadBps = 15; p.volWeightBps = 10_000; p.staleSpreadBps = 100; p.stressSpreadBps = 300; p.maxSpreadBps = 1_000;
        p.maxMoveBps = 500; p.convBps = convBps; p.bufferBps = 500; p.mintPerHour = 5e18; p.burnPerHour = 5e18;
        p.maxPublicEthBacked = 10e18; p.maxEthBacked = 40e18; p.revenueTarget = 1 ether; p.rentTerm = 30 days; p.premiumBps = 1_000;
        p.rentShareBps = 5_000; p.maxRentals = 20; p.lowConfidenceSpreadBps = 100; p.fullConfidenceBps = 8_000; p.minConfidenceBps = 3_000;
        p.maxItemsPerDay = 3; p.reserveBps = reserveBps; p.fillBandBps = 300;
    }

    /// DeployStack's template (docs/v5/floor-module.md §13.2): 5 % platform (capped 0.25 ETH), 12.6 % shared-pool ETH side.
    function _template() internal pure returns (FrogCurve.Config memory c) {
        c.supply = 1_000_000_000e18; c.reserved = 200_000_000e18;
        c.platformFeeCap = 0.25 ether; c.platformBps = 500; c.poolEthBps = 1_260; c.feeBps = 100;
        c.vaultShareBps = 4_000; c.creatorShareBps = 3_000; c.reserveShareBps = 500; c.liquidityShareBps = 500;
        c.window = 3 days; c.launchFeeBps = 5_000; c.launchFeeDecay = 10 minutes;
    }

    function _collection(address nft, uint256 floor, uint256 normal, uint256 early, uint16 convBps, uint16 reserveBps) internal returns (Col memory c) {
        c.floor = floor;
        c.w = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), 1, nft, custodian, "w", "w", PBS);
        c.w.initMintLimit(1_000);
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
        c.s.setDepth(10 ether);                 // past the opening and early stages: the normal target
        vm.prank(custodian);
        c.m.postFloor(floor, 10_000, 50);
    }

    function _graduate(Col memory c) internal returns (uint256 units) {
        vm.prank(creator);
        (, address curve) = c.f.launch("Coin", "COIN", SweepVault.Policy.RAFFLE, false);
        FrogCurve fc = FrogCurve(payable(curve));
        uint256 net0 = c.m.netUnits();
        Base_skip(11 minutes);                         // past the decaying launch fee (docs/v5/floor-module.md §2)
        uint256 a = c.m.accepted();
        vm.prank(custodian);
        c.m.postFloor(a, 10_000, 50);                  // the poster keeps the floor fresh
        uint256 pay = fc.raise() * 2;
        vm.deal(buyer, buyer.balance + pay);
        vm.prank(buyer);
        fc.buy{value: pay}(0);
        fc.graduate();
        units = c.m.netUnits() - net0;
    }

    /// The canister's whole buy on the 4663 side, at the most the module pays now.
    function _buy(Col memory c, uint256 cost) internal {
        _buyWith(c, c.m.maxItemCost(), cost);
    }

    /// The same, advancing `amount` (the canister's own: the item cap within the collection's purchase limit).
    function _buyWith(Col memory c, uint256 amount, uint256 cost) internal {
        require(amount > 0, "nothing to buy");
        vm.prank(custodian);
        c.m.advance(amount);
        vm.prank(custodian);
        c.w.mint(nextToken, keccak256(abi.encode(nextToken)), address(c.m));
        nextToken++;
        vm.prank(custodian);
        c.m.settleAdvance(cost > amount ? amount : cost);
    }

    function _books(Col memory c) internal view {
        assertEq(c.w.totalSupply(), (c.w.held() - c.w.openTickets()) * ONE + c.w.ethBacked(), "supply == (held - open) x ONE + ethBacked");
        assertGe(address(c.m).balance, c.m.backing() + c.m.reserve() + c.m.surplus() + c.m.revenue() + c.m.gradEth()
            + c.m.rentalEscrow() + c.m.renterPool() + c.m.owedTotal(), "the module holds every wei it owes");
        assertEq(c.m.netUnits() + c.m.reserveUnits() + c.m.transitUnits(), c.w.ethBacked(), "C + R + T = E");
    }

    function _walk(Col memory c, uint256 to) internal {
        for (uint256 i = 0; i < 100 && c.m.accepted() != to; ++i) {
            Base_skip(1 hours);
            vm.prank(custodian);
            c.m.postFloor(to, 10_000, 50);
        }
        assertEq(c.m.accepted(), to);
    }

    function Base_skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    // ---- a pooled collection: BAYC at 9 ETH with the defaults (its config before J1 was decided), two graduations per ape ----

    function test_BAYC_atNineEth_poolsTwoGraduationsPerApe() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.79 ether, 9 ether, 9 ether, 300, 2_500);
        assertEq(bayc.f.normalRaise(), 9 ether);
        assertEq(bayc.f.itemBudgetFor(9 ether), 7.616 ether, "9 - 0.25 - 1.134");
        uint256 u1 = _graduate(bayc);
        assertApproxEqRel(u1, 0.8167e18, 0.001e18, "0.75 x 7.616 / (6.79 x 1.03)");
        assertEq(bayc.m.maxItemCost(), 0, "under one convertible unit: nothing is advanced, nothing leaves 4663");
        _books(bayc);
        _graduate(bayc);
        assertGe(bayc.m.netUnits(), 1.05e18, "the pool covers one ape plus the buffer");
        uint256 max = bayc.m.maxItemCost();
        assertApproxEqAbs(max, 6.9937 ether, 10, "one ape at the floor + 3 % (the backing per unit, to the wei)");
        uint256 before = bayc.m.betaConvertible();
        _buy(bayc, 6.8 ether);
        assertEq(bayc.w.held(), 1, "one ape");
        assertApproxEqRel(bayc.m.netUnits(), 0.633e18, 0.002e18, "the rest of the pool waits for the next graduation");
        assertGe(bayc.m.betaConvertible(), before, "no pooled unit's backing fell");
        assertEq(bayc.m.maxItemCost(), 0);
        _books(bayc);
    }

    // ---- BAYC as the launch decided it (J1, 2026-09-27): 10 ETH, 10 % conversion, no reserve: one graduation per ape ----
    // docs/v7/phase1-nft-pooled.md §1.1: at the 6.79 floor one graduation converts 8.49 / (6.79 x 1.10) = 1.137 units. It buys one ape
    // alone while the floor at its graduation is at most 7.3506 (+8.26 %), and a graduation at 6.79 is still advanced (the canister's
    // gate: Relay's estimated 0.5 %, the floor + 1 %, 0.003 ETH) after a rise to 7.3549 (+8.32 %). The canister advances
    // min(maxItemCost, max_purchase_wei = 8 ETH) (floorsink::view).

    uint256 constant BAYC_LIMIT = 8 ether;       // deployments/launch/nft-pilot.json maxPurchaseWei

    function _bayc(address nft) internal returns (Col memory) {
        return _collection(nft, 6.79 ether, 10 ether, 10 ether, 1_000, 0);
    }

    function _canisterAmount(Col memory c) internal view returns (uint256 a) {
        a = c.m.maxItemCost();
        if (a > BAYC_LIMIT) a = BAYC_LIMIT;
    }

    function test_BAYC_asDecided_oneGraduationBuysOneApe() public {
        Col memory bayc = _bayc(makeAddr("BAYC"));
        assertEq(bayc.f.normalRaise(), 10 ether);
        assertEq(bayc.f.itemBudgetFor(10 ether), 8.49 ether, "10 - 0.25 - 1.26");
        uint256 u = _graduate(bayc);
        assertApproxEqRel(u, 1.1367e18, 0.001e18, "8.49 / (6.79 x 1.10)");
        assertEq(bayc.m.reserveUnits(), 0, "no standing reserve");
        assertGe(bayc.m.netUnits(), 1.05e18, "one graduation covers one ape plus the buffer");
        uint256 amount = _canisterAmount(bayc);
        assertTrue(_canisterAdvances(bayc, amount), "the canister advances it at the floor it graduated at");
        _buyWith(bayc, _canisterAmount(bayc), 6.8 ether);
        assertEq(bayc.w.held(), 1, "one ape from one graduation");
        assertApproxEqRel(bayc.m.netUnits(), 0.1367e18, 0.01e18, "the rest waits, ETH-backed");
        _books(bayc);
    }

    function test_BAYC_asDecided_aRiseAfterTheGraduation_ofUpTo8_32pc_stillBuysTheApe() public {
        Col memory bayc = _bayc(makeAddr("BAYC"));
        _graduate(bayc);
        _walk(bayc, 7.354 ether);                       // +8.30 % before the canister starts the buy
        assertTrue(_canisterAdvances(bayc, _canisterAmount(bayc)), "inside the headroom: advanced");
        _buyWith(bayc, _canisterAmount(bayc), 7.354 ether);
        assertEq(bayc.w.held(), 1);
        _books(bayc);
        // past the edge the canister's gate refuses: the ETH stays on 4663, ETH-backed and burnable
        Col memory late = _bayc(makeAddr("BAYC late"));
        _graduate(late);
        _walk(late, 7.37 ether);                        // +8.54 %
        assertFalse(_canisterAdvances(late, _canisterAmount(late)), "past the headroom: the pool waits on 4663");
        _books(late);
    }

    function test_BAYC_asDecided_aRiseBeforeTheGraduation_ofUpTo8_26pc_stillOneGraduationOneApe() public {
        Col memory bayc = _bayc(makeAddr("BAYC"));
        _walk(bayc, 7.35 ether);                        // +8.25 % before the coin graduates
        _graduate(bayc);
        assertGe(bayc.m.netUnits(), 1.05e18, "8.49 / (7.35 x 1.10) = 1.0501: still one ape");
        uint256 amount = _canisterAmount(bayc);
        assertEq(amount, BAYC_LIMIT, "the item cap (8.09) is over the purchase limit: the canister advances 8 ETH");
        assertTrue(_canisterAdvances(bayc, amount));
        _buyWith(bayc, _canisterAmount(bayc), 7.35 ether);
        assertEq(bayc.w.held(), 1);
        _books(bayc);
        // above 7.3506 a graduation is under one ape plus the buffer: BAYC pools two graduations per ape from there
        Col memory dear = _bayc(makeAddr("BAYC dear"));
        _walk(dear, 7.36 ether);
        _graduate(dear);
        assertLt(dear.m.netUnits(), 1.05e18, "8.49 / (7.36 x 1.10) = 1.0487: pooled");
        _graduate(dear);
        assertTrue(_canisterAdvances(dear, _canisterAmount(dear)), "the second graduation completes it");
        _buyWith(dear, _canisterAmount(dear), 7.36 ether);
        assertEq(dear.w.held(), 1);
        _books(dear);
    }

    // ---- A2-L-1 (the operator, 2026-09-27): the canister advances only the buy's bound; the leftover it sweeps back is surplus ----
    // ethflow::advance_bound: (6.79 x 1.05 + 0.003) / 0.97 + 0.0002 = 7.35334 ETH, under the 7.469 item cap. The module already takes
    // any amount up to maxItemCost: the unit releases its whole backing and what was not advanced is this module's revenue.

    function test_A2L1_aPartialAdvance_keepsTheRestInTheModule_andAReturnRestoresTheBacking() public {
        Col memory bayc = _bayc(makeAddr("BAYC"));
        _graduate(bayc);
        uint256 cap = bayc.m.maxItemCost();
        uint256 f = 6.79 ether;
        uint256 bound = (f * 10_500 / 10_000 + 0.003 ether) * 10_000 / 9_700 + 1 + 0.0002 ether;
        assertLt(bound, cap);
        uint256 per = bayc.m.betaConvertible();
        uint256 bal = address(bayc.m).balance;
        vm.prank(custodian);
        bayc.m.advance(bound);
        assertEq(address(bayc.m).balance, bal - bound, "only the bound left the module");
        assertApproxEqAbs(bayc.m.revenue(), per - bound, 2, "the rest of the unit's backing stays, as the module's revenue");
        assertGt(bayc.m.revenue(), 0.115 ether);
        _books(bayc);
        // the ape costs less than the bound: settled at its share, no ETH moves
        vm.prank(custodian);
        bayc.w.mint(nextToken, keccak256(abi.encode(nextToken)), address(bayc.m));
        nextToken++;
        vm.prank(custodian);
        bayc.m.settleAdvance(6.82 ether);
        assertEq(bayc.w.held(), 1);
        _books(bayc);

        // the same advance coming back after Relay both ways (48 h, D4): what was never advanced is restored in full from that revenue;
        // only Relay's fees on what was advanced are a loss to the units (review round 3 A3-L-1), as with a whole-cap advance
        Col memory back = _bayc(makeAddr("BAYC back"));
        _graduate(back);
        uint256 per2 = back.m.betaConvertible();
        uint256 c = back.m.netUnits();
        vm.prank(custodian);
        back.m.advance(bound);
        uint256 v = bound * 9_930 / 10_000;
        vm.deal(custodian, v);
        vm.prank(custodian);
        back.m.returnAdvance{value: v}();
        assertApproxEqAbs(back.m.betaConvertible(), per2 - (bound - v) * ONE / c, 3, "the loss is Relay's fees and nothing more");
        assertEq(back.m.revenue(), 0, "the kept part went back into the backing");
        _books(back);
    }

    function test_A2L1_aSweptLeftover_isThatModulesSurplus_andTopsUpItsConversions() public {
        Col memory bayc = _bayc(makeAddr("BAYC"));
        Col memory milady = _collection(makeAddr("Milady"), 0.93 ether, 5 ether, 8 ether, 300, 2_500);
        _graduate(bayc);
        _graduate(milady);
        _walk(bayc, 7.37 ether);                        // past the headroom: the gate would refuse the pool
        uint256 before = bayc.m.maxItemCost();
        uint256 mSurplus = milady.m.surplus();
        vm.deal(payee, 0.4 ether);
        vm.prank(payee);                                // the canister's 4663 address, after the way back
        bayc.m.fundReserve{value: 0.4 ether}();
        assertEq(bayc.m.surplus(), 0.4 ether, "the leftover joins BAYC's surplus");
        assertEq(milady.m.surplus(), mSurplus, "and no other module's");
        assertGt(bayc.m.maxItemCost(), before, "it tops up BAYC's next conversion");
        assertTrue(_canisterAdvances(bayc, _canisterAmount(bayc)), "enough to buy the ape after the rise");
        _books(bayc);
        _books(milady);
    }

    // ---- Pudgy at the default 5 ETH: 0.89 per graduation, two per item ----

    function test_Pudgy_atTheDefaultTarget_poolsTwoGraduations() public {
        Col memory pudgy = _collection(makeAddr("Pudgy"), 3.353 ether, 5 ether, 8 ether, 300, 2_500);
        uint256 u1 = _graduate(pudgy);
        assertApproxEqRel(u1, 0.8947e18, 0.001e18, "0.75 x 4.12 / (3.353 x 1.03)");
        assertEq(pudgy.m.maxItemCost(), 0);
        _graduate(pudgy);
        _buy(pudgy, 3.36 ether);
        assertApproxEqRel(pudgy.m.netUnits(), 0.789e18, 0.003e18);
        _books(pudgy);
    }

    // ---- strictly per collection ----

    function test_oneCollectionsPoolNeverFundsOrBacksAnothers() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.79 ether, 9 ether, 9 ether, 300, 2_500);
        Col memory milady = _collection(makeAddr("Milady"), 0.93 ether, 5 ether, 8 ether, 300, 2_500);
        _graduate(bayc);
        _graduate(milady);
        uint256 baycEth = address(bayc.m).balance;
        uint256 baycUnits = bayc.m.netUnits();
        // Milady's pool buys; BAYC's is untouched
        _buy(milady, 0.94 ether);
        assertEq(address(bayc.m).balance, baycEth, "Milady's advance took nothing of BAYC's pool");
        assertEq(bayc.m.netUnits(), baycUnits);
        assertEq(bayc.m.maxItemCost(), 0, "BAYC still waits for its own second graduation");
        // an item minted through BAYC's wrapper can never settle Milady's advance
        uint256 adv = milady.m.maxItemCost();
        vm.prank(custodian);
        milady.m.advance(adv);
        vm.prank(custodian);
        bayc.w.mint(900, keccak256("x"), address(milady.m));
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NoWrapped.selector);
        milady.m.settleAdvance(0.9 ether);
        // and BAYC's module cannot advance: it has no whole unit
        vm.prank(custodian);
        vm.expectRevert();
        bayc.m.advance(1 ether);
        _books(milady);
        _books(bayc);
    }

    /// Audit 2026-09-27-nft A-L-3 (PoC `test_A_L3_oneCollectionsPauseHaltsAnotherCollection`, now red): the guardian's floor
    /// pause of one collection (D8) is that collection's PosterSource, not the registry: the other collection's module still takes
    /// posts, advances and closes.
    function test_A_L3_oneCollectionsFloorPauseNeverHaltsAnother() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.79 ether, 9 ether, 9 ether, 300, 2_500);
        Col memory milady = _collection(makeAddr("Milady"), 0.93 ether, 5 ether, 8 ether, 300, 2_500);
        _graduate(milady);
        PosterSource baycPoster = PosterSource(address(bayc.m.registry()));
        vm.prank(owner);                                   // the Safe here; the guardian may too (EthNftFlow.t.sol)
        baycPoster.pause();
        assertEq(bayc.m.keeper(), address(0));
        assertEq(bayc.m.maxItemCost(), 0, "BAYC's inflows stop");
        assertEq(milady.m.keeper(), custodian, "Milady's poster is untouched");
        vm.prank(custodian);
        milady.m.postFloor(0.93 ether, 10_000, 50);
        assertGt(milady.m.maxItemCost(), 0, "and Milady's module still converts");
        _buy(milady, 0.94 ether);
        _books(milady);
    }

    // ---- the floor rises between graduations: the pool waits, the next graduation completes it ----

    /// The canister's advance gate (audit 2026-09-27-nft round 2, A2-M-1; `ethflow::advance_covers`, the same numbers): an advance of
    /// `amount` must reach Ethereum, after Relay's estimated fee (50 bps) and the 4663 gas keep (0.0002 ETH), as at least the module's
    /// accepted floor + 1 % and the Ethereum gas reserve (0.003 ETH). The canister starts no buy, and signs no advance, otherwise.
    function _canisterAdvances(Col memory c, uint256 amount) internal view returns (bool) {
        uint256 accepted = c.m.accepted();
        if (accepted == 0 || amount == 0) return false;
        uint256 arrive = amount - 0.0002 ether - amount * 50 / 10_000;
        return arrive >= accepted * 10_100 / 10_000 + 0.003 ether;
    }

    /// Audit 2026-09-27-nft round 2, A2-M-1: this test was vacuous (it tried `advance(7.5 ether)`, above `maxItemCost()`, never the
    /// canister's own call). What holds, as built: the MODULE would pay `advance(maxItemCost())` out of a blend short of the new floor
    /// (it has no floor gate: a frog's advance is the one inscription's price, which may be under the floor), and the CANISTER is what
    /// refuses it (`ethflow::advance_covers`, driven in `autobuyer/canister-rs/tests/audit_nft_r2.rs`
    /// `r2_a2_m1_a_module_whose_item_cap_does_not_cover_its_floor_never_advances`): nothing leaves 4663. A third graduation at the new
    /// floor lifts the blend; once it covers the floor the canister advances it, and the module pays it.
    function test_aFloorRiseWhilePooled_waitsThenTheNextGraduationPricesInTheNewFloor() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.79 ether, 9 ether, 9 ether, 300, 2_500);
        _graduate(bayc);
        _walk(bayc, 7.469 ether);
        _graduate(bayc);                               // priced at the new floor
        uint256 max = bayc.m.maxItemCost();
        uint256 avg = bayc.m.backing() * ONE / bayc.m.netUnits();
        assertLe(max, avg + bayc.m.revenue() + bayc.m.surplus(), "never above the average backing and the pots");
        assertLt(max, 7.469 ether, "the blend is short of the new floor");
        assertGt(max, 0, "the module itself would pay it out");
        assertFalse(_canisterAdvances(bayc, max), "the canister's gate refuses the advance: the pool waits on 4663");
        assertEq(bayc.m.transitUnits(), 0);
        _books(bayc);
        // a third graduation at the new floor lifts the average; the pool still never pays more than its backing
        _graduate(bayc);
        uint256 max3 = bayc.m.maxItemCost();
        assertGe(max3, max);
        if (_canisterAdvances(bayc, max3)) {
            vm.prank(custodian);
            bayc.m.advance(max3);
            assertEq(bayc.m.transitUnits(), ONE, "covered: it converts");
        }
        _books(bayc);
    }

    /// A2-M-1 (PoC `AuditNftA2_M1_PooledRiseAdvancesBelowTheFloor`): at +10 % between BAYC's two graduations the canister advanced the
    /// blend and stranded it on Ethereum (`AdvanceOpen` for every later graduation). The canister's gate now refuses at +10 % (and
    /// from about +2 %: the scan), and lets a rise within the headroom (+1 %) convert as designed; the unpooled collections keep their
    /// full cap's headroom.
    function test_A2_M1_theCanistersGateRefusesAShortPoolAndLetsACoveredOneConvert() public {
        uint16[4] memory bps = [uint16(100), 300, 500, 1_000];
        bool[4] memory advances = [true, false, false, false];
        for (uint256 i = 0; i < bps.length; ++i) {
            Col memory c = _collection(address(uint160(0xB0 + i)), 6.79 ether, 9 ether, 9 ether, 300, 2_500);
            _graduate(c);
            _walk(c, 6.79 ether * (10_000 + uint256(bps[i])) / 10_000);
            _graduate(c);
            uint256 max = c.m.maxItemCost();
            assertEq(_canisterAdvances(c, max), advances[i], "the gate at this rise");
            if (advances[i]) {
                vm.prank(custodian);
                c.m.advance(max);
                assertEq(c.m.transitUnits(), ONE);
            }
            _books(c);
        }
        // an unpooled collection advances its full cap at the floor it graduated at
        Col memory milady = _collection(makeAddr("Milady"), 0.93 ether, 5 ether, 8 ether, 300, 2_500);
        _graduate(milady);
        assertTrue(_canisterAdvances(milady, milady.m.maxItemCost()));
    }

    // ---- holder deposits while pooled (D5) ----

    function test_aDepositWhilePooled_isRedeemable_andTheLedgerHolds() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.79 ether, 9 ether, 9 ether, 300, 2_500);
        _graduate(bayc);
        uint256 net = bayc.m.netUnits();
        vm.prank(custodian);
        bayc.w.mintDeposit(4242, keccak256("deposit"), alice);
        assertEq(bayc.m.netUnits(), net, "a deposit does not count toward the pool");
        _books(bayc);
        // an ETH-backed holder (a public mint) redeems the deposited ape
        vm.deal(buyer, 10 ether);
        vm.prank(buyer);
        bayc.m.mint{value: 7.2 ether}(buyer, 0);
        vm.prank(buyer);
        uint256 t = bayc.w.redeem(buyer);
        bayc.w.resolve(t, abi.encodePacked(bytes32(uint256(7))));
        vm.prank(custodian);
        bayc.w.recordDelivery(t, keccak256("delivered"));
        assertEq(bayc.w.held(), 0);
        _books(bayc);
    }
}
