// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {CurveBase, MockSeeder} from "./utils/CurveBase.sol";
import {Clock} from "./utils/Clock.sol";
import {SweepVault} from "../src/SweepVault.sol";
import {FrogCurve} from "../src/curve/FrogCurve.sol";
import {CurveFactory} from "../src/curve/CurveFactory.sol";
import {CurveToken} from "../src/curve/CurveToken.sol";
import {ItemQueue} from "../src/curve/ItemQueue.sol";
import {ICapSource} from "../src/interfaces/ICapSource.sol";
import {ILiquiditySeeder} from "../src/interfaces/ILiquiditySeeder.sol";

/// A factory stand-in for curves deployed directly by a test: names whatever curve it is told as a coin's router.
contract MockCapSource is ICapSource {
    address public wrapped;
    ILiquiditySeeder public seeder;
    mapping(address => address) public routerOf;
    constructor(address w, ILiquiditySeeder s) { wrapped = w; seeder = s; }
    function set(address coin, address curve) external { routerOf[coin] = curve; }
    function freshCap() external pure returns (uint256, bool) { return (0.646 ether, true); }
}

/// docs/v3/curve.md: the curve, the queue-backed graduation, the failure path and the fees, on the worked example's numbers.
contract FrogCurveTest is CurveBase {
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    uint256 constant FROG = 0.64 ether;

    // ---- the terms ----

    function test_theWorkedExampleSplitsTheRaiseAsDocumented() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        assertEq(c.raise(), 5 ether);
        assertEq(c.platformFee(), 0.25 ether);
        assertEq(c.poolEth(), 0.63 ether);
        assertEq(c.itemBudget(), 4.12 ether);
        assertEq(c.forSale() + c.reserved(), SUPPLY);
        assertApproxEqRel(c.reserved(), RESERVED, 1e6, "the reserve moves by base units at most");
    }

    function test_anEarlyCoinRaisesMoreButThePlatformCutIsCapped() public {
        seeder.setDepth(1 ether);                                // the collection's shared pool is still thin
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        assertEq(c.raise(), 8 ether);
        assertEq(c.platformFee(), 0.25 ether, "5% of 8 is 0.40; capped at 0.25");
        assertEq(c.poolEth(), 1.008 ether);
        assertEq(c.itemBudget(), 8 ether - 0.25 ether - 1.008 ether);
    }

    function test_theCurveEndsAtThePoolsOpeningPrice() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        _fill(c, alice);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        // (itemBudget − poolEth) / R = 3.49 / 200M ETH per coin
        assertApproxEqRel(c.price(), c.poolOpenPrice(), 1e9, "no jump at graduation (to 1e-9)");
        assertApproxEqRel(c.poolOpenPrice(), 17.45e9, 1e6);
    }

    function testFuzz_theEndPriceMatchesForAnyRaise(uint96 raise, bool sweepOnly) public {
        uint256 r = bound(raise, 0.1 ether, 500 ether);
        MockCapSource src = new MockCapSource(address(wrapper), seeder);
        ItemQueue q = new ItemQueue(wrapper, payee, 500, 0.001 ether, 1_000, PBS);
        q.wire(src);
        FrogCurve.Config memory cfg = _template();
        cfg.raise = r;
        cfg.sweepOnly = sweepOnly;
        FrogCurve c = new FrogCurve(cfg, "A", "A", wrapper, curveRaffle, seeder, q, creator, platform, PBS);
        src.set(address(c.token()), address(c));
        _buy(c, alice, r * 2);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        // within 1e-9, plus one unit of integer rounding: at a 500 ETH raise the price is about 9.5e8 units, so one unit is 1.05e-9
        // (the nightly deep campaign of 2026-10-09: 949999999 against 950000000)
        uint256 p = c.price();
        uint256 o = c.poolOpenPrice();
        assertLe(p > o ? p - o : o - p, o / 1e9 + 1, "no jump at graduation (to 1e-9, or one unit)");
        assertEq(c.reserveEth(), r - c.itemBudget(), "the raise less the item budget is escrowed here");
        assertEq(address(q).balance, c.itemBudget(), "the item budget is in the queue");
        assertEq(CurveToken(address(c.token())).balanceOf(address(c)), c.reserved(), "everything for sale is sold");
        _curveBooks(c);
    }

    // ---- trading ----

    function test_aBuyPaysOnePercentSplitFiftyThirtyTwenty() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE);
        uint256 out = _buy(c, alice, 1 ether);
        assertEq(tok.balanceOf(alice), out);
        assertEq(c.feesForVault(), 0.005 ether);
        assertEq(c.creatorOwed(), 0.003 ether);
        assertEq(c.platformOwed(), 0.002 ether);
        assertEq(c.reserveEth(), 0.99 ether);
        _curveBooks(c);
    }

    function test_aRoundTripLosesTheFeesAndNothingElse() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE);
        uint256 out = _buy(c, alice, 1 ether);
        vm.startPrank(alice);
        tok.approve(address(c), out);
        uint256 back = c.sell(out, 0);
        vm.stopPrank();
        assertApproxEqRel(back, 0.99 ether * 99 / 100, 1e12);
        assertLe(back, 0.9801 ether);
        _curveBooks(c);
    }

    function test_slippageIsRefused() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        vm.expectRevert(FrogCurve.Slippage.selector);
        c.buy{value: 1 ether}(type(uint256).max);
    }

    function test_theLastBuyIsCappedTheChangeReturnedAndTheBudgetQueued() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE);
        vm.deal(alice, 20 ether);
        vm.prank(alice);
        c.buy{value: 20 ether}(0);
        // 5 ETH net at 1%: gross = ceil(5 / 0.99)
        assertApproxEqAbs(alice.balance, 20 ether - uint256(5 ether) * 10_000 / 9_900, 2);
        assertEq(c.reserveEth(), 0.25 ether + 0.63 ether, "fee and pool side stay here");
        ItemQueue.Entry memory e = queue.entry(address(tok));
        assertEq(uint8(e.status), uint8(ItemQueue.Status.Queued));
        assertEq(e.deposited, 4.12 ether);
        _curveBooks(c);
        _queueBooks();
    }

    function test_theCurveClosesAtTheRaise() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE);
        _fill(c, alice);
        vm.deal(bob, 1 ether);
        vm.prank(bob);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.buy{value: 1 ether}(0);
        vm.startPrank(alice);
        tok.approve(address(c), 1e18);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.sell(1e18, 0);
        vm.stopPrank();
    }

    // ---- graduating ----

    function test_graduationSeedsBothPoolsAtWhatTheUnitsCostAndPaysOnlyTheCappedFee() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE);
        _fill(c, alice);
        assertEq(_fillQueue(c, FROG), 6, "4.12 at 0.64 a frog: six frogs, then 0.28 left, under the 10% tail");
        uint256 platformBefore = c.platformOwed();
        c.graduate();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduated));
        assertEq(seeder.token(), address(tok));
        assertEq(seeder.tokenAmount(), c.reserved());
        // the coin's pool gets the units that cost itemBudget − poolEth = 3.49 of the 3.84 spent
        assertEq(seeder.wrappedForToken(), uint256(6e18) * 3.49 ether / 3.84 ether);
        assertEq(seeder.wrappedForToken() + seeder.wrappedForShared(), 6e18);
        assertEq(seeder.ethIn(), 0.63 ether + 0.28 ether, "pool side plus the tail the queue did not spend");
        assertEq(c.platformOwed() - platformBefore, 0.25 ether);
        assertEq(c.reserveEth(), 0);
        assertEq(tok.balanceOf(address(c)), 0);
        assertEq(wrapper.balanceOf(address(c)), 0);
        assertEq(payee.balance, 6 * FROG, "the canister was paid per frog, from the queue");
        _curveBooks(c);
        _queueBooks();
    }

    function test_graduateBeforeTheQueueFillsIsRefused() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        _fill(c, alice);
        for (uint256 i = 0; i < 5; ++i) _item(FROG);
        vm.expectRevert(FrogCurve.NotFilled.selector);
        c.graduate();
    }

    function test_aSeederThatTakesLessIsRefused() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        _fill(c, alice);
        _fillQueue(c, FROG);
        seeder.setShort(true);
        vm.expectRevert(FrogCurve.SeederShort.selector);
        c.graduate();
    }

    function test_aSweepOnlyCoinGraduatesAtOnceIntoCoinEth() public {
        (FrogCurve c, CurveToken tok) = _launchAs(SweepVault.Policy.BURN, true);
        assertEq(c.itemBudget(), 0);
        assertEq(c.poolEth(), 4.75 ether);
        _fill(c, alice);
        assertEq(queue.length(), 0, "nothing queued");
        c.graduate();
        assertTrue(seeder.viaEth());
        assertEq(seeder.ethIn(), 4.75 ether);
        assertEq(seeder.tokenAmount(), c.reserved());
        assertEq(seeder.token(), address(tok));
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.openRefunds();
        _curveBooks(c);
    }

    function test_aSweepOnlyCoinNeverRefunds() public {
        (FrogCurve c,) = _launchAs(SweepVault.Policy.BURN, true);
        _fill(c, alice);
        Clock.skip(4 days);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.openRefunds();
    }

    // ---- the failure path ----

    function test_refundsOpenOnlyAfterTheWindowOnBothClocks() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        _fill(c, alice);
        vm.expectRevert(FrogCurve.TooEarly.selector);
        c.openRefunds();
        Clock.skipTimeOnly(4 days);
        vm.expectRevert(FrogCurve.TooEarly.selector);
        c.openRefunds();
        Clock.skip(3 days);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding));
    }

    function test_refundsPayEthAndTheUnitsAlreadyBoughtProRata() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE);
        uint256 a = _buy(c, alice, 2 ether);
        _fill(c, bob);
        uint256 b = tok.balanceOf(bob);
        _item(FROG);
        _item(FROG);                                             // two frogs, then the floor ran away
        Clock.skip(3 days);
        c.openRefunds();
        assertEq(c.refundEthPool(), 5 ether - 2 * FROG, "fee (not taken), pool side, and the queue's unspent ETH");
        assertEq(c.refundWrappedPool(), 2 * ONE);
        assertEq(uint8(queue.entry(address(tok)).status), uint8(ItemQueue.Status.Cancelled));

        vm.startPrank(alice);
        tok.approve(address(c), a);
        (uint256 ea, uint256 wa) = c.refund(a);
        vm.stopPrank();
        vm.startPrank(bob);
        tok.approve(address(c), b);
        (uint256 eb, uint256 wb) = c.refund(b);
        vm.stopPrank();

        assertEq(a + b, c.forSale());
        assertApproxEqAbs(ea + eb, 5 ether - 2 * FROG, 2);
        assertApproxEqAbs(wa + wb, 2 * ONE, 2);
        assertApproxEqRel(ea * c.forSale() / a, c.refundEthPool(), 1e6, "pro rata");
        assertEq(wrapper.balanceOf(alice), wa);
        assertLe(c.reserveEth(), 2);
        _curveBooks(c);
        _queueBooks();
    }

    function test_noRefundsOnceFilled() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        _fill(c, alice);
        _fillQueue(c, FROG);
        Clock.skip(4 days);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.openRefunds();
        c.graduate();                                            // late is fine: anyone may still graduate it
    }

    function test_thePlatformTakesNoGraduationFeeOnAFailure() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        _fill(c, alice);
        uint256 owed = c.platformOwed();
        Clock.skip(3 days);
        c.openRefunds();
        assertEq(c.platformOwed(), owed, "only the trade fees it earned");
    }

    // ---- fees ----

    function test_vaultFeesAreCreditedToTheCoin() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE);
        _buy(c, alice, 1 ether);
        c.flushVaultFees();
        assertEq(curveRaffle.credit(address(tok)), 0.005 ether);
        assertEq(curveBurn.credit(address(tok)), 0, "a raffle coin's fees never reach the burn vault");
    }

    function test_theCreatorClaimsOnlyTheirs() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        _buy(c, alice, 1 ether);
        vm.prank(alice);
        vm.expectRevert(FrogCurve.NotCreator.selector);
        c.claimCreatorFees();
        vm.prank(creator);
        c.claimCreatorFees();
        assertEq(creator.balance, 0.003 ether);
        c.claimPlatformFees();
        assertEq(platform.balance, 0.002 ether);
    }

    function _graduated(SweepVault.Policy p) internal returns (FrogCurve c, CurveToken tok) {
        (c, tok) = _launch(p);
        _fill(c, alice);
        _fillQueue(c, FROG);
        c.graduate();
    }

    function test_coinPoolWrappedFeesFeedTheSameSettlement() public {
        (FrogCurve c, CurveToken tok) = _graduated(SweepVault.Policy.BURN);
        _deposit(address(this));
        _deposit(address(this));
        wrapper.transfer(address(c), 2 * ONE);                   // the coin pool's LP fees, as the lock hands them over
        c.forwardWrappedFees();
        assertEq(curveBurn.units(address(tok)), ONE);
        assertEq(wrapper.balanceOf(creator), 0.6e18);
        assertEq(wrapper.balanceOf(platform), 0.4e18);
        curveBurn.retire(address(tok));                          // and it burns like any other wFROG the coin's fees bought
    }

    function test_coinPoolTokenFeesBurnTheVaultsShare() public {
        (FrogCurve c, CurveToken tok) = _graduated(SweepVault.Policy.RAFFLE);
        vm.prank(alice);
        tok.transfer(address(c), 1_000e18);
        c.forwardTokenFees();
        assertEq(tok.balanceOf(c.DEAD()), 500e18);
        assertEq(tok.balanceOf(creator), 300e18);
        assertEq(tok.balanceOf(platform), 200e18);
    }

    function test_onlyTheSeederPaysPoolFees() public {
        (FrogCurve c,) = _graduated(SweepVault.Policy.RAFFLE);
        vm.deal(alice, 1 ether);
        vm.startPrank(alice);
        vm.expectRevert(FrogCurve.NotSeeder.selector);
        c.onPoolFees{value: 1}();
        vm.expectRevert(FrogCurve.NotSeeder.selector);
        c.depositPoolFees{value: 1}(0);
        vm.stopPrank();
    }

    function test_seederFeesGoEntirelyToTheCoinsVault() public {
        (FrogCurve c, CurveToken tok) = _graduated(SweepVault.Policy.RAFFLE);
        _deposit(address(seeder));
        vm.deal(address(seeder), 1 ether);
        vm.startPrank(address(seeder));
        wrapper.approve(address(c), ONE);
        c.depositPoolFees{value: 0.1 ether}(ONE);
        vm.stopPrank();
        assertEq(curveRaffle.units(address(tok)), ONE);
        assertEq(curveRaffle.credit(address(tok)), 0.1 ether);
    }

    function test_wrappedFeesAreNotForwardedBeforeGraduation() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.BURN);
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.forwardWrappedFees();
        vm.expectRevert(FrogCurve.WrongState.selector);
        c.forwardTokenFees();
    }

    // ---- the factory ----

    function test_theFactoryIsTheCoinsLauncher() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.BURN);
        assertEq(factory.routerOf(address(tok)), address(c));
        assertEq(factory.creatorOf(address(tok)), creator);
        assertEq(factory.listingOf(address(tok)), BURN_LISTING);
        assertEq(address(c.vault()), address(curveBurn));
    }

    function test_wireOnceAndOnlyByTheDeployer() public {
        vm.prank(stranger);
        vm.expectRevert(CurveFactory.NotDeployer.selector);
        factory.wire(curveRaffle, curveBurn);
        vm.expectRevert(CurveFactory.AlreadyWired.selector);
        factory.wire(curveRaffle, curveBurn);
    }

    function test_wireRefusesVaultsOfTheWrongPolicyOrLauncher() public {
        CurveFactory f = new CurveFactory(wrapper, seeder, queue, platform, _template(), _targets(), PBS);
        vm.expectRevert(CurveFactory.BadVault.selector);
        f.wire(curveBurn, curveRaffle);
        vm.expectRevert(CurveFactory.BadVault.selector);
        f.wire(curveRaffle, curveBurn);                          // their launcher is the other factory
    }

    function test_aRouterOfAnotherCoinCannotDeposit() public {
        (FrogCurve c1,) = _launch(SweepVault.Policy.RAFFLE);
        (, CurveToken t2) = _launch(SweepVault.Policy.RAFFLE);
        vm.deal(address(c1), 1 ether);
        vm.prank(address(c1));
        vm.expectRevert(SweepVault.NotRouter.selector);
        curveRaffle.deposit{value: 1 ether}(address(t2));
    }

    function test_targetsStepDownAsTheCollectionsSharedPoolDeepens() public {
        seeder.setDepth(0);
        (uint256 r, bool early, bool opening) = factory.raiseFor(false);
        assertEq(r, EARLY);                                      // one frog plus the pool side is far below 8 ETH
        assertTrue(early && opening);
        seeder.setDepth(1.99 ether);
        (r, early, opening) = factory.raiseFor(false);
        assertEq(r, EARLY);
        assertTrue(early && !opening);
        seeder.setDepth(2 ether);
        (r, early, opening) = factory.raiseFor(false);
        assertEq(r, NORMAL);
        assertFalse(early || opening);
        (r,,) = factory.raiseFor(true);
        assertEq(r, NORMAL, "SWEEP_ONLY: always the normal target");
    }

    function test_theOpeningTargetBuysOneItemOnAnExpensiveCollection() public {
        seeder.setDepth(0);
        _postCap(curveRaffle, 5.1 ether);                        // a Bored Ape at 5 ETH, all-in
        (uint256 r,, bool opening) = factory.raiseFor(false);
        assertTrue(opening);
        // one item at the cap plus the 5% buffer, plus the 0.25 fee cap, grossed up for the 12.6% pool side: 6.41 < 8
        assertEq(r, EARLY, "the early target already buys one ape");
        _postCap(curveRaffle, 7 ether);                          // a 7 ETH item: 8.70 > 8
        (r,, opening) = factory.raiseFor(false);
        uint256 expect = (uint256(7 ether) * 10_500 / 10_000 + 0.25 ether) * 10_000 / 8_740;
        assertApproxEqAbs(r, expect, 1);
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        assertGe(c.itemBudget(), uint256(7 ether) * 10_500 / 10_000, "a lone first coin can buy one item by itself");
    }

    function test_theOpeningTargetNeedsAFreshCap() public {
        seeder.setDepth(0);
        Clock.skip(2 hours);
        vm.expectRevert(CurveFactory.NoCap.selector);
        factory.raiseFor(false);
        vm.prank(creator);
        vm.expectRevert(CurveFactory.NoCap.selector);
        factory.launch("A", "A", SweepVault.Policy.RAFFLE, false);
    }

    function test_aTargetAboveTheMaximumIsRefusedButSweepOnlyStillLaunches() public {
        seeder.setDepth(0);
        _postCap(curveRaffle, 40 ether);
        vm.expectRevert();
        factory.raiseFor(false);
        (FrogCurve c,) = _launchAs(SweepVault.Policy.RAFFLE, true);
        assertEq(c.raise(), NORMAL);
    }

    function test_theEarlyTargetHasAHardCeiling() public {
        CurveFactory.Targets memory t = _targets();
        t.earlyRaise = 10 ether + 1;
        vm.expectRevert(CurveFactory.BadInput.selector);
        new CurveFactory(wrapper, seeder, queue, platform, _template(), t, PBS);
        t.earlyRaise = 10 ether;
        new CurveFactory(wrapper, seeder, queue, platform, _template(), t, PBS);
    }
}
