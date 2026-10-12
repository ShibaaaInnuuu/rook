// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {CurveBase} from "./utils/CurveBase.sol";
import {FrogCurve} from "../src/curve/FrogCurve.sol";
import {CurveFactory} from "../src/curve/CurveFactory.sol";
import {CurveToken} from "../src/curve/CurveToken.sol";
import {SweepVault} from "../src/SweepVault.sol";
import {MockYoyoVenue} from "./YoyoVault.t.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// docs/v5/policies-and-fees.md §2 (the launch fee that decays, to the vault) and §3 (the creator's fee tier, fixed at launch),
/// on the worked curve terms with a 50% launch fee decaying over ten minutes.
contract LaunchFeeTest is CurveBase {
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    function _template() internal pure override returns (FrogCurve.Config memory c) {
        c = super._template();
        c.launchFeeBps = 5_000;
        c.launchFeeDecay = 10 minutes;
    }

    function _skipBoth(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    // ---- the decay ----

    function test_aBuyAtLaunchPaysTheHighFee_allOfTheExtraToTheVault() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        assertEq(c.launchExtraBps(), 5_000);
        uint256 out = _buy(c, alice, 1 ether);
        assertGt(out, 0);
        // 51% of the ETH is fee: 1% split 50/30/20, the 50% all to the vault
        assertEq(c.feesForVault(), 0.005 ether + 0.5 ether, "the vault's half of the base fee, and the whole extra");
        assertEq(c.creatorOwed(), 0.003 ether, "the creator sees only the base fee");
        assertEq(c.platformOwed(), 0.002 ether, "so does the platform");
        assertEq(c.reserveEth(), 0.49 ether, "what the curve kept");
    }

    function test_theExtraDecaysLinearly_toNothingAtTheEnd() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        _skipBoth(5 minutes);
        assertEq(c.launchExtraBps(), 2_500, "half way: half the extra");
        uint256 v0 = c.feesForVault();
        _buy(c, alice, 1 ether);
        assertEq(c.feesForVault() - v0, 0.005 ether + 0.25 ether);
        _skipBoth(5 minutes);
        assertEq(c.launchExtraBps(), 0);
        v0 = c.feesForVault();
        uint256 cr0 = c.creatorOwed();
        _buy(c, bob, 1 ether);
        assertEq(c.feesForVault() - v0, 0.005 ether, "the base fee alone");
        assertEq(c.creatorOwed() - cr0, 0.003 ether);
        _skipBoth(1 days);
        assertEq(c.launchExtraBps(), 0);
    }

    function test_theSequencersClockAloneDoesNotShortenTheWindow() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        vm.warp(vm.getBlockTimestamp() + 10 minutes);           // time only: the parent chain has not moved
        assertEq(c.launchExtraBps(), 5_000, "the slower clock counts");
        vm.roll(vm.getBlockNumber() + 10 minutes / PBS);
        assertEq(c.launchExtraBps(), 0);
    }

    function test_sellsPayTheBaseFeeOnly() public {
        (FrogCurve c, CurveToken tok) = _launch(SweepVault.Policy.RAFFLE);
        uint256 out = _buy(c, alice, 1 ether);
        uint256 v0 = c.feesForVault();
        vm.startPrank(alice);
        IERC20(address(tok)).approve(address(c), out);
        uint256 got = c.sell(out, 0);
        vm.stopPrank();
        assertGt(got, 0);
        // the sell's fee is 1% of its gross; the vault's half of that is what was added
        assertLt(c.feesForVault() - v0, got / 50, "far below any launch extra");
    }

    function test_theLastBuyReachesTheRaiseExactly_grossedUpAtTheTotalRate() public {
        (FrogCurve c,) = _launch(SweepVault.Policy.RAFFLE);
        uint256 raise = c.raise();
        uint256 balBefore = alice.balance;
        vm.deal(alice, balBefore + raise * 4);
        vm.prank(alice);
        c.buy{value: raise * 4}(0);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        assertEq(c.v(), c.vEnd(), "the raise reached exactly");
        // she paid raise / (1 − 0.51) gross, the rest came back
        uint256 gross = (raise * 10_000 + 4_899) / 4_900;
        assertApproxEqAbs(alice.balance, balBefore + raise * 4 - gross, 2);
        assertEq(c.feesForVault(), (gross - raise) * 5_000 / 5_100 + (gross - raise) * 100 / 5_100 * 5_000 / 10_000, "the extra and the vault's half");
    }

    function test_theConfigIsBounded() public {
        FrogCurve.Config memory cfg = _template();
        cfg.raise = 5 ether;
        cfg.launchFeeBps = 9_001;
        vm.expectRevert(FrogCurve.BadConfig.selector);
        new FrogCurve(cfg, "A", "A", wrapper, curveRaffle, seeder, queue, creator, platform, PBS);
        cfg.launchFeeBps = 5_000;
        cfg.launchFeeDecay = 59;
        vm.expectRevert(FrogCurve.BadConfig.selector);
        new FrogCurve(cfg, "A", "A", wrapper, curveRaffle, seeder, queue, creator, platform, PBS);
        cfg.launchFeeBps = 0;
        cfg.launchFeeDecay = 0;
        FrogCurve c = new FrogCurve(cfg, "A", "A", wrapper, curveRaffle, seeder, queue, creator, platform, PBS);
        assertEq(c.launchExtraBps(), 0, "disabled");
    }
}

/// §3: the creator's fee tier.
contract FeeTierTest is CurveBase {
    address alice = makeAddr("alice");

    function _targets() internal pure override returns (CurveFactory.Targets memory t) {
        t = super._targets();
        t.minFeeBps = 100;
        t.maxFeeBps = 800;
    }

    function test_theTierIsChosenAtLaunch_withinTheBounds_andPublished() public {
        vm.prank(creator);
        (address coin, address curve) = factory.launch("Five", "FIVE", SweepVault.Policy.RAFFLE, false, 500);
        assertEq(FrogCurve(payable(curve)).feeBps(), 500);
        assertEq(factory.feeOf(coin), 500);
        vm.prank(creator);
        vm.expectRevert(abi.encodeWithSelector(CurveFactory.BadFeeTier.selector, 50, 100, 800));
        factory.launch("Low", "LOW", SweepVault.Policy.RAFFLE, false, 50);
        vm.prank(creator);
        vm.expectRevert(abi.encodeWithSelector(CurveFactory.BadFeeTier.selector, 900, 100, 800));
        factory.launch("High", "HIGH", SweepVault.Policy.RAFFLE, false, 900);
        // the four-argument launch: the template's 1%
        vm.prank(creator);
        (address coin2, address curve2) = factory.launch("One", "ONE", SweepVault.Policy.RAFFLE, false);
        assertEq(FrogCurve(payable(curve2)).feeBps(), 100);
        assertEq(factory.feeOf(coin2), 100);
    }

    function test_theSplitScalesWithTheTier() public {
        vm.prank(creator);
        (, address curve) = factory.launch("Five", "FIVE", SweepVault.Policy.RAFFLE, false, 500);
        FrogCurve c = FrogCurve(payable(curve));
        _buy(c, alice, 1 ether);
        assertEq(c.feesForVault(), 0.025 ether, "a 5% coin sends 2.5% to sweeps");
        assertEq(c.creatorOwed(), 0.015 ether);
        assertEq(c.platformOwed(), 0.01 ether);
    }

    function test_theFeeTierIsImmutable() public view {
        string[] memory fns = vm.parseJsonKeys(vm.readFile("out/FrogCurve.sol/FrogCurve.json"), ".methodIdentifiers");
        for (uint256 i = 0; i < fns.length; ++i) {
            bytes memory b = bytes(fns[i]);
            assertTrue(!(b.length >= 3 && b[0] == "s" && b[1] == "e" && b[2] == "t"), string.concat("a setter: ", fns[i]));
        }
        fns = vm.parseJsonKeys(vm.readFile("out/CurveFactory.sol/CurveFactory.json"), ".methodIdentifiers");
        for (uint256 i = 0; i < fns.length; ++i) {
            bytes memory b = bytes(fns[i]);
            assertTrue(!(b.length >= 3 && b[0] == "s" && b[1] == "e" && b[2] == "t"), string.concat("a setter: ", fns[i]));
        }
        assertEq(factory.minFeeBps(), 100);
        assertEq(factory.maxFeeBps(), 800);
    }

    function test_aFactoryWithNoTiersTakesOnlyTheTemplatesFee() public {
        vm.prank(creator);
        vm.expectRevert(abi.encodeWithSelector(CurveFactory.BadFeeTier.selector, 500, 0, 0));
        _noTierFactory().launch("Five", "FIVE", SweepVault.Policy.RAFFLE, false, 500);
    }

    function _noTierFactory() internal returns (CurveFactory f) {
        CurveFactory.Targets memory t = super._targets();
        f = new CurveFactory(wrapper, seeder, queue, platform, _template(), t, PBS);
    }

    // ---- YOYO through the factory ----

    function test_aYoyoLaunchNeedsTheYoyoVault_wiredOnce() public {
        vm.prank(creator);
        vm.expectRevert(CurveFactory.NoYoyo.selector);
        factory.launch("Yo", "YO", SweepVault.Policy.YOYO, false);
        SweepVault yoyoV = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.YOYO, address(venue), factory, desk, PBS);
        vm.prank(creator);
        vm.expectRevert(CurveFactory.NotDeployer.selector);
        factory.wireYoyo(yoyoV);
        vm.expectRevert(CurveFactory.BadVault.selector);
        factory.wireYoyo(curveBurn);
        factory.wireYoyo(yoyoV);
        vm.expectRevert(CurveFactory.AlreadyWired.selector);
        factory.wireYoyo(yoyoV);
        vm.prank(creator);
        (address coin, address curve) = factory.launch("Yo", "YO", SweepVault.Policy.YOYO, false);
        assertEq(address(FrogCurve(payable(curve)).vault()), address(yoyoV));
        assertEq(uint8(factory.policyOf(coin)), uint8(SweepVault.Policy.YOYO));
        assertEq(factory.listingOf(coin), BURN_LISTING);
    }
}
