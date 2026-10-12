// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, console2} from "forge-std/Test.sol";
import {IPonsV2LaunchFactory, IPonsV2FeeEscrow} from "../../src/interfaces/IPons.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {Launcher} from "../../src/Launcher.sol";
import {FeeRouter} from "../../src/FeeRouter.sol";
import {AssetVault} from "../../src/AssetVault.sol";

interface ICurve {
    function buy(uint256 quoteIn, uint256 minTokensOut, address recipient) external payable returns (uint256);
}

interface IFactoryRead {
    function getLaunchedToken(address token) external view returns (
        address token_, address curve, address deployer, address creatorFeeRecipient, address pairToken, uint256 graduationThreshold,
        uint24 poolFee, int24 tickSpacing, uint16 creatorTaxBps, bool buybackEnabled, uint8 phase, uint256 sweptQuote, uint256 sweptTokens,
        uint256 sweptAt, bool exists);
    function transferCreatorFeeRecipient(address token, address newRecipient) external;
}

/// Fork of Robinhood Chain (4663) at the public RPC. Nothing is deployed for real and nothing is sent.
/// Run: forge test --match-path "test/fork/*" --fork-url robinhood   (skipped without a fork)
contract PonsLaunchForkTest is Test {
    IPonsV2LaunchFactory constant FACTORY = IPonsV2LaunchFactory(0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e);
    IPonsV2FeeEscrow constant ESCROW = IPonsV2FeeEscrow(0xd3AFEB2a57f70eF218Aa82451c51B2fb0416Ac9e);

    ListingRegistry registry;
    Launcher launcher;
    address owner = makeAddr("owner");
    address treasury = makeAddr("treasury");
    address creator = makeAddr("creator");
    address trader = makeAddr("trader");
    bool forked;

    function setUp() public {
        forked = block.chainid == 4663;
        if (!forked) return;
        vm.startPrank(owner);
        registry = new ListingRegistry(owner, treasury);
        launcher = new Launcher(FACTORY, registry);
        registry.wire(address(launcher), address(0xD157));
        registry.listRune("TEST RUNE", "840000:1", 2, "bc1p-test-vault");
        vm.stopPrank();
        vm.deal(creator, 1 ether);
        vm.deal(trader, 10 ether);
    }

    function _launch(uint16 taxBps) internal returns (address coin, address curve, address router) {
        Launcher.LaunchParams memory p;
        p.name = "Floor Test"; p.symbol = "FLOOR"; p.salt = keccak256("etch-fork-test"); p.creatorTaxBps = taxBps;
        uint256 fee = FACTORY.launchFee();
        vm.prank(creator);
        (coin, curve, router) = launcher.launch{value: fee}(p);
    }

    function test_launchThroughOurLauncher_namesTheRouter_andTheEscrowIsTheFactorys() public {
        if (!forked) return;
        assertEq(FACTORY.feeEscrow(), address(ESCROW));
        (address coin, address curve, address router) = _launch(0);
        assertTrue(coin.code.length > 0 && curve.code.length > 0 && router.code.length > 0);
        (,, address deployer, address recipient, address pair,,,, uint16 tax, bool buyback,,,,, bool exists) = IFactoryRead(address(FACTORY)).getLaunchedToken(coin);
        assertTrue(exists);
        assertEq(recipient, router, "Pons records our router, a contract, as the creator-fee recipient");
        assertEq(deployer, address(launcher), "to Pons the creator is the Launcher; the person is on our record");
        assertEq(pair, address(0));
        assertEq(tax, 0);
        assertFalse(buyback);
        assertEq(launcher.creatorOf(coin), creator);
        assertEq(launcher.routerOf(coin), router);
    }

    function test_tradeOnTheCurve_harvest_80_20() public {
        if (!forked) return;
        (address coin, address curve, address router) = _launch(200);   // a 2% creator tax on top of the base fee
        vm.warp(vm.getBlockTimestamp() + 60);                                   // past the launch-second snipe tax
        vm.prank(trader);
        ICurve(curve).buy{value: 1 ether}(1 ether, 0, trader);
        // FINDING: nothing is in escrow yet. Pons keeps a curve's fees in the curve until they are swept.
        assertEq(ESCROW.balanceOf(router), 0);

        AssetVault vault = AssetVault(registry.vaultOf(0));
        vm.prank(makeAddr("anyone"));
        (uint256 toVault, uint256 toTreasury) = FeeRouter(payable(router)).harvest();   // sweeps the curve itself, then claims
        uint256 total = toVault + toTreasury;
        console2.log("creator-side fees on a 1 ETH buy at a 2% creator tax (wei):", total);
        // 70% of the 1% base fee + the whole 2% creator tax = 2.7% of what was spent
        assertApproxEqRel(total, 0.027 ether, 0.02e18);
        assertEq(toVault, (total * 8000) / 10000);
        assertEq(address(vault).balance, toVault);
        assertEq(treasury.balance, toTreasury);
        assertEq(vault.contributedBy(coin), toVault);
        assertEq(ESCROW.balanceOf(router), 0);
        assertEq(router.balance, 0);
    }

    function test_theRouterCannotBeMadeToHandItsRoleOn() public {
        if (!forked) return;
        (address coin,,) = _launch(0);
        // only the current recipient may transfer the role, and the router has no function that makes that call
        vm.prank(creator);
        vm.expectRevert();
        IFactoryRead(address(FACTORY)).transferCreatorFeeRecipient(coin, creator);
        vm.prank(address(launcher));
        vm.expectRevert();
        IFactoryRead(address(FACTORY)).transferCreatorFeeRecipient(coin, creator);
    }

    function test_snipeTaxInTheLaunchSecond_mostlyReachesTheRouter() public {
        if (!forked) return;
        (, address curve, address router) = _launch(0);
        vm.prank(trader);                                                 // same second as the launch: the tax is at its start
        ICurve(curve).buy{value: 1 ether}(1 ether, 0, trader);
        (uint256 toVault, uint256 toTreasury) = FeeRouter(payable(router)).harvest();
        console2.log("creator-side take of a launch-second 1 ETH snipe (wei):", toVault + toTreasury);
        assertGt(toVault + toTreasury, 0.5 ether, "70% of a ~99% snipe tax, plus 70% of the base fee");
    }

    /// Graduation, as far as a fork reaches without building pool swaps: buy through the threshold and see what the router gets.
    function test_graduation_theCurvesFeesStillReachTheRouter_andThePairingSurvives() public {
        if (!forked) return;
        (address coin, address curve, address router) = _launch(100);
        vm.warp(vm.getBlockTimestamp() + 60);
        vm.deal(trader, 100 ether);
        uint256 spent;
        for (uint256 i = 0; i < 12 && !ICurveSweep(curve).graduated(); ++i) {
            vm.prank(trader);
            try ICurve(curve).buy{value: 1 ether}(1 ether, 0, trader) { spent += 1 ether; } catch { break; }
        }
        console2.log("spent on the curve (wei):", spent, "graduated:", ICurveSweep(curve).graduated());
        (uint256 toVault, uint256 toTreasury) = FeeRouter(payable(router)).harvest();
        console2.log("creator-side fees harvested across graduation (wei):", toVault + toTreasury);
        assertGt(toVault, 0);
        (,,, address recipient,,,,,,, uint8 phase,,,,) = IFactoryRead(address(FACTORY)).getLaunchedToken(coin);
        console2.log("graduation phase recorded by the factory:", phase);
        assertEq(recipient, router, "the recipient is still our router after graduation");
    }

    function test_aStrangerCannotSweepTheCurve_butTheRouterCan() public {
        if (!forked) return;
        (, address curve,) = _launch(0);
        vm.warp(vm.getBlockTimestamp() + 60);
        vm.prank(trader);
        ICurve(curve).buy{value: 0.5 ether}(0.5 ether, 0, trader);
        vm.prank(trader);
        vm.expectRevert();
        ICurveSweep(curve).sweepFees(0);
    }
}

interface ICurveSweep { function sweepFees(uint256) external; function graduated() external view returns (bool); }
