// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {console2} from "forge-std/console2.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Base} from "../utils/Base.sol";
import {MockSeeder} from "../utils/CurveBase.sol";
import {AuditNftABase} from "./AuditNftABase.sol";
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

/// Round-2 review (2026-09-27-nft, part A). The pooled-graduation harness of test/ethnft/Pooled.t.sol (real curves and factories,
/// stage-0 module defaults), helpers only.
abstract contract PooledHarness is Base {
    struct Col { EthNftWrapper w; FloorModule m; CurveFactory f; MockSeeder s; uint256 floor; }
    address payee = makeAddr("payee (the canister's EVM address on 4663)");
    address platform = makeAddr("platform");
    address creator = makeAddr("creator");
    address buyer = makeAddr("curve buyer");
    uint256 constant ONE = 1e18;
    uint256 nextToken = 1;
    // what the canister keeps back on Ethereum / what Relay takes (docs/v7/phase1-nft-pooled.md §1 headroom terms)
    uint256 constant RELAY_BPS = 36;             // 0.36 %
    uint256 constant GAS_MARGIN = 0.0032 ether;

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
        c.s.setDepth(10 ether);
        vm.prank(custodian);
        c.m.postFloor(floor, 10_000, 50);
    }

    function _graduate(Col memory c) internal returns (uint256 units) {
        vm.prank(creator);
        (, address curve) = c.f.launch("Coin", "COIN", SweepVault.Policy.RAFFLE, false);
        FrogCurve fc = FrogCurve(payable(curve));
        uint256 net0 = c.m.netUnits();
        _sk(11 minutes);
        uint256 a = c.m.accepted();
        vm.prank(custodian);
        c.m.postFloor(a, 10_000, 50);
        uint256 pay = fc.raise() * 2;
        vm.deal(buyer, buyer.balance + pay);
        vm.prank(buyer);
        fc.buy{value: pay}(0);
        fc.graduate();
        units = c.m.netUnits() - net0;
    }

    function _walk(Col memory c, uint256 to) internal {
        for (uint256 i = 0; i < 100 && c.m.accepted() != to; ++i) {
            _sk(1 hours);
            vm.prank(custodian);
            c.m.postFloor(to, 10_000, 50);
        }
        assertEq(c.m.accepted(), to);
    }

    function _sk(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _books(Col memory c) internal view {
        assertEq(c.w.totalSupply(), (c.w.held() - c.w.openTickets()) * ONE + c.w.ethBacked(), "supply == (held - open) x ONE + ethBacked");
        assertGe(address(c.m).balance, c.m.backing() + c.m.reserve() + c.m.surplus() + c.m.revenue() + c.m.gradEth()
            + c.m.rentalEscrow() + c.m.renterPool() + c.m.owedTotal(), "the module holds every wei it owes");
        assertEq(c.m.netUnits() + c.m.reserveUnits() + c.m.transitUnits(), c.w.ethBacked(), "C + R + T = E");
    }

    /// What reaches the canister's Ethereum address from an advance of `amount` (Relay 0.36 %, the gas margin).
    function _arrives(uint256 amount) internal pure returns (uint256) {
        return amount * (10_000 - RELAY_BPS) / 10_000 - GAS_MARGIN;
    }
}

/// A2-M-1: docs/v7/phase1-nft-pooled.md §3.4 says that while a pool's average backing cannot pay the cheapest acceptable listing,
/// "nothing is advanced ... No ETH leaves 4663, so there is no D4-style stranding on Ethereum". The contract does not gate that: it
/// only caps the advance at the average backing (plus revenue and surplus), and the canister advances `min(maxItemCost, limit)`
/// whenever the module wants an item (ethflow.rs NeedAdvance, floorsink::view), without comparing it to the floor. Pooled.t.sol's
/// test asserts only that `advance(7.5 ether)` (above the cap) reverts, never that `advance(maxItemCost())` does.
/// PASS = the weakness: the whole pooled budget leaves 4663 at an amount under the posted floor.
contract AuditNftA2_M1_PooledRiseAdvancesBelowTheFloor is PooledHarness {
    function test_A2_M1_aFloorRiseBetweenPooledGraduations_theAdvanceLeavesBelowTheFloor() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.79 ether, 9 ether, 9 ether, 300, 2_500);   // the launch config (J1 committed)
        _graduate(bayc);
        _walk(bayc, 7.469 ether);                                   // +10 % between the two graduations (days apart when pooled)
        _graduate(bayc);
        uint256 floor = bayc.m.accepted();
        uint256 max = bayc.m.maxItemCost();
        console2.log("posted floor (wei)", floor);
        console2.log("maxItemCost (what the canister advances)", max);
        console2.log("arrives on Ethereum (after Relay and the gas margin)", _arrives(max));
        assertGt(max, 0, "the module DOES let an advance through");
        assertLt(max, floor, "at less than the posted floor (the rank-2 listing)");
        uint256 bal = address(bayc.m).balance;
        vm.prank(custodian);
        bayc.m.advance(max);                                        // exactly what ethflow.rs's NeedAdvance sends
        assertEq(address(bayc.m).balance, bal - max, "the pooled ETH left 4663");
        assertEq(bayc.m.transitUnits(), ONE);
        _books(bayc);
        // the module is now blocked for further conversions until that ETH buys an item or comes back; an Arrived buy has no
        // give-back path (ethflow.rs give_back_now refuses it), so only a listing under what arrived ends it
        _graduate(bayc);
        vm.prank(custodian);
        vm.expectRevert(FloorModule.AdvanceOpen.selector);
        bayc.m.advance(1 ether);
    }

    /// The rise at which it starts: under ~5 % the pooled advance still covers the posted floor after Relay; above it, not.
    function test_A2_M1_theThreshold() public {
        uint16[6] memory bps = [uint16(300), 400, 500, 600, 800, 1_000];
        for (uint256 i = 0; i < bps.length; ++i) {
            Col memory c = _collection(address(uint160(0xB0 + i)), 6.79 ether, 9 ether, 9 ether, 300, 2_500);
            _graduate(c);
            _walk(c, 6.79 ether * (10_000 + uint256(bps[i])) / 10_000);
            _graduate(c);
            uint256 floor = c.m.accepted();
            uint256 max = c.m.maxItemCost();
            console2.log("rise bps", bps[i]);
            console2.log("  maxItemCost", max);
            if (max > GAS_MARGIN) console2.log("  arrives / floor (bps)", _arrives(max) * 10_000 / floor);
        }
    }
}

/// A2-L (J1's alternative): docs/v7/phase1-nft-pooled.md §1 says of BAYC at 10 ETH / convBps 1,100 / reserve 0 that "the part not
/// spent on the ape becomes the module's revenue, as every conversion's leftover does". On the advance path it does not:
/// `settleAdvance(cost)` moves no ETH, and `advanced - cost` stays at the canister's Ethereum address (its gas pot, phase1-nft §5).
contract AuditNftA2_L_J1LeftoverLeaves is PooledHarness {
    function test_A2_L_J1Alternative_theLeftoverOfEveryApeLeavesTheModule() public {
        Col memory bayc = _collection(makeAddr("BAYC"), 6.79 ether, 10 ether, 10 ether, 1_100, 0);
        _graduate(bayc);
        uint256 max = bayc.m.maxItemCost();
        uint256 revBefore = bayc.m.revenue();
        uint256 balBefore = address(bayc.m).balance;
        vm.prank(custodian);
        bayc.m.advance(max);
        vm.prank(custodian);
        bayc.w.mint(1, keccak256("fill"), address(bayc.m));
        uint256 cost = 6.79 ether * 10_500 / 10_000;                // even at the canister's full 5 % premium over the floor
        vm.prank(custodian);
        bayc.m.settleAdvance(cost);
        console2.log("advanced", max);
        console2.log("ape cost", cost);
        console2.log("left at the canister's Ethereum address, not the module's revenue", max - cost);
        assertEq(address(bayc.m).balance, balBefore - max, "the module paid the whole advance");
        assertLt(bayc.m.revenue(), revBefore + 1e9, "no leftover came back as revenue (dust only)");
        assertGt(max - cost, 0.4 ether, "about 0.41 ETH (at +5 %) to 0.75 ETH (at the floor) per ape");
        _books(bayc);
    }
}

/// Holds: the per-collection PosterSource switch behaves as the registry's for everything the module reads (pause floor, refund
/// hold, RESUME_GRACE), and adds no power to the guardian.
contract AuditNftA2_Holds_PosterSwitch is AuditNftABase {
    function test_A2_holds_mintThenPosterPauseThenBurn_isALoss() public {
        _graduate(4.12 ether);
        _holdFor(F, 6);
        vm.deal(mallory, 1 ether);
        vm.prank(mallory);
        uint256 units = module.mint{value: 1 ether}(mallory, 0);
        vm.prank(guardian);
        poster.pause();
        assertTrue(module.burnsOpen(), "burns stay open (pause floor)");
        uint256 got = _burn(mallory, units);
        console2.log("paid 1 ETH, got back", got);
        assertLt(got, 1 ether, "no mint -> poster pause -> burn round trip pays");
        _books();
    }

    function test_A2_holds_refundHoldAndGrace_followTheLaterResume() public {
        assertFalse(module.holdsRefunds());
        vm.prank(guardian);
        poster.pause();
        assertTrue(module.holdsRefunds(), "held while this collection is paused");
        vm.prank(guardian);
        vm.expectRevert(PosterSource.NotOwner.selector);
        poster.unpause();
        // the registry pauses and resumes meanwhile: still held (the poster's own switch)
        vm.prank(guardian);
        registry.pauseKeeper();
        _skip(1 hours);
        vm.prank(owner);
        registry.unpauseKeeper();
        assertTrue(module.holdsRefunds());
        _skip(1 hours);
        vm.prank(owner);
        poster.unpause();
        assertTrue(module.holdsRefunds(), "RESUME_GRACE from the poster's resume");
        _skip(6 hours + 1);
        _post(F);
        assertFalse(module.holdsRefunds());
        // an unpause of an unpaused poster stamps nothing (no free grace extension)
        uint64 at = poster.resumedAt();
        _skip(1 hours);
        vm.prank(owner);
        poster.unpause();
        assertEq(poster.resumedAt(), at);
        assertFalse(module.holdsRefunds());
    }

    function test_A2_holds_aDepositDuringAPosterPause_burnsAtThePauseFloor() public {
        _graduate(4.12 ether);
        _holdFor(F, 6);
        vm.prank(guardian);
        poster.pause();
        _deposit(7777, mallory);                                    // the wrapper's mintDeposit does not read the poster's switch
        uint256 got = _burn(mallory, ONE);
        console2.log("deposit burned during the pause for", got);
        assertLe(got, F, "never above the posted floor (pause floor x (1 - base))");
        _books();
    }
}

/// Holds (the fixer's claim, re-run): round 1's ledger fuzz without the now-refused relist, with graduations small enough to pool
/// (0.7 ETH at a 0.93 floor: 0.55 units each), deposits, the two remaining redeems, advance / settle / return, mints and burns.
contract AuditNftA2_Holds_PooledLedger is AuditNftABase {
    uint256[11] hits;

    function testFuzz_A2_pooledLedgerHolds(uint256 seed) public { _run(seed); }

    function test_A2_pooledLedger_theRunReachesEveryAction() public {
        for (uint256 s = 1; s <= 20; ++s) { if (s > 1) setUp(); _run(s); }
        for (uint256 i = 0; i < 11; ++i) { console2.log("action", i, "taken", hits[i]); assertGt(hits[i], 0); }
    }

    function _run(uint256 seed) internal {
        uint256 token = 1;
        address[3] memory hs = [alice, bob, mallory];
        for (uint256 step = 0; step < 60; ++step) {
            seed = uint256(keccak256(abi.encode(seed, step)));
            uint256 a = seed % 11;
            address h = hs[(seed >> 8) % 3];
            if (a == 0 && w.mintRoom()) {
                _deposit(token++, h); hits[0]++;
            } else if (a == 1 && module.transitUnits() == 0 && module.maxItemCost() != 0) {
                uint256 amt = module.maxItemCost();
                vm.prank(custodian); module.advance(amt); hits[1]++;
            } else if (a == 2 && module.transitUnits() != 0 && w.mintRoom()) {
                _mintPurchase(token++, address(module));
                uint256 cost = module.advanced() * 97 / 100;
                vm.prank(custodian); module.settleAdvance(cost); hits[2]++;
            } else if (a == 3 && module.transitUnits() != 0 && (seed >> 16) % 4 == 0) {
                uint256 adv = module.advanced();
                vm.deal(custodian, adv); vm.prank(custodian); module.returnAdvance{value: adv * 99 / 100}(); hits[3]++;
            } else if (a == 4) {
                _skip(1 minutes); _graduate(0.7 ether); hits[4]++;                    // a pooled graduation (0.55 units)
            } else if (a == 5) {
                (uint256 p, uint256 mu) = module.burnCapacity();
                uint256 u = w.balanceOf(h) < mu ? w.balanceOf(h) : mu;
                if (u > 0 && p > 0 && u * p / ONE > 0) { vm.startPrank(h); w.approve(address(module), u); module.burn(u, 0, h); vm.stopPrank(); hits[5]++; }
            } else if (a == 6 && w.balanceOf(h) >= ONE && w.held() >= w.openTickets() + 1) {
                vm.prank(h);
                if ((seed >> 24) % 2 == 0) w.redeem(h); else w.redeemToBurn();
                hits[6]++;
            } else if (a == 7 && w.nextToResolve() < w.ticketCount()) {
                _skip(2 minutes); _resolve(w.nextToResolve()); hits[7]++;
            } else if (a == 8) {
                uint256 n = w.ticketCount();
                for (uint256 i = 0; i < n; ++i) {
                    if (w.ticket(i).stage == EthNftWrapper.Stage.Resolved) {
                        vm.prank(custodian);
                        if ((seed >> 32) % 2 == 0) w.recordDelivery(i, keccak256(abi.encode(i))); else w.markUndeliverable(i);
                        hits[8]++;
                        break;
                    }
                }
            } else if (a == 9) {
                _skip(11 days); _post(F);
                uint256 n = w.ticketCount();
                for (uint256 i = 0; i < n; ++i) {
                    EthNftWrapper.Stage s = w.ticket(i).stage;
                    if ((s == EthNftWrapper.Stage.Resolved && w.undeliverable(i)) || s == EthNftWrapper.Stage.Pinned) { w.refund(i); hits[9]++; }
                }
            } else {
                // a public mint lands in a holder's hands so pooled ETH-backed units redeem deposited NFTs too
                (, uint256 maxIn) = module.mintCapacity();
                if (maxIn > 0.1 ether) { vm.deal(h, 0.1 ether); vm.prank(h); module.mint{value: 0.1 ether}(h, 0); }
                _skip(1 hours); _post(F); hits[10]++;
            }
            _books();
            assertGe(w.held(), w.openTickets(), "every open ticket has an item");
        }
    }
}
