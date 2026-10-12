// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Base} from "./utils/Base.sol";
import {Launcher} from "../src/Launcher.sol";
import {FeeRouter} from "../src/FeeRouter.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {IPonsV2LaunchFactory, IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "../src/interfaces/IPons.sol";
import {IEtchVault} from "../src/interfaces/IEtchVault.sol";
import {AssetVault} from "../src/AssetVault.sol";
import {MockEscrow} from "./utils/Mocks.sol";

/// A factory that records what it was asked, for the Launcher's own logic. The real one is exercised in test/fork.
contract MockFactory {
    MockEscrow public immutable escrow = new MockEscrow();
    uint256 public launchFee = 0.0005 ether;
    IPonsV2LaunchFactory.TokenParams public last;
    address public lastPair;
    address public lastExempt;
    uint256 public n;
    function feeEscrow() external view returns (address) { return address(escrow); }
    function memeHook() external pure returns (address) { return address(0xB00C); }
    function launchToken(IPonsV2LaunchFactory.TokenParams calldata p, uint256, address pairToken, address[] calldata exempt)
        external payable returns (address token, address curve)
    {
        require(msg.value == launchFee, "LaunchFeeNotPaid");
        last = p; lastPair = pairToken; lastExempt = exempt[0]; n++;
        token = address(uint160(uint256(keccak256(abi.encode("token", n)))));
        curve = address(uint160(uint256(keccak256(abi.encode("curve", n)))));
    }
}

contract LauncherTest is Base {
    MockFactory factory;
    Launcher real;

    function setUp() public override {
        super.setUp();
        factory = new MockFactory();
        real = new Launcher(IPonsV2LaunchFactory(address(factory)), registry);
        vm.deal(creatorA, 1 ether);
    }

    function _params(uint256 listingId, bytes32 salt) internal pure returns (Launcher.LaunchParams memory p) {
        p.name = "Coin"; p.symbol = "COIN"; p.salt = salt; p.listingId = listingId; p.creatorTaxBps = 300;
    }

    function test_launchNamesAFreshRouter_recordsThePerson_nativeOnly_buybackOff() public {
        address predicted = real.predictRouter(creatorA, "s1");
        vm.prank(creatorA);
        (address coin,, address router) = real.launch{value: 0.0005 ether}(_params(1, "s1"));
        assertEq(router, predicted);
        (,,,,, address recipient, uint16 tax, bool buyback,,) = factory.last();
        assertEq(recipient, router);
        assertEq(tax, 300);
        assertFalse(buyback, "a buyback would divert the creator bucket away from the vault");
        assertEq(factory.lastPair(), address(0), "native ETH only");
        assertEq(factory.lastExempt(), creatorA);
        assertEq(real.creatorOf(coin), creatorA);
        assertEq(real.routerOf(coin), router);
        assertEq(real.listingOf(coin), 1);
        FeeRouter r = FeeRouter(payable(router));
        assertEq(r.coin(), coin);
        assertEq(address(r.vault()), registry.vaultOf(1));
        (IEtchVault v, IPonsV2FeeEscrow e, IPonsV2BondingCurve c, IPonsV2MemeHook h) = (r.vault(), r.escrow(), r.curve(), r.hook());
        vm.expectRevert(FeeRouter.NotLauncher.selector);          // nobody but the Launcher initialises a router (docs/phase3.md L1)
        r.initialize(address(1), registry, v, e, c, h);
        vm.prank(address(real));
        vm.expectRevert(FeeRouter.AlreadyInitialised.selector);   // and not even the Launcher re-points one
        r.initialize(address(1), registry, v, e, c, h);
        // the implementation itself is locked: it was never a coin's router and can never be made to look like one
        FeeRouter impl = FeeRouter(payable(real.routerImplementation()));
        vm.prank(address(real));
        vm.expectRevert(FeeRouter.AlreadyInitialised.selector);
        impl.initialize(address(2), registry, v, e, c, h);
    }

    function test_exactlyTheLaunchFee() public {
        vm.startPrank(creatorA);
        vm.expectRevert(abi.encodeWithSelector(Launcher.WrongLaunchFee.selector, 0, 0.0005 ether));
        real.launch(_params(0, "a"));
        vm.expectRevert(abi.encodeWithSelector(Launcher.WrongLaunchFee.selector, 1 ether, 0.0005 ether));
        real.launch{value: 1 ether}(_params(0, "a"));
        vm.stopPrank();
    }

    function test_noLaunchOnAPausedOrMissingListing() public {
        vm.prank(owner);
        registry.setPaused(0, true);
        vm.startPrank(creatorA);
        vm.expectRevert(Launcher.ListingPaused.selector);
        real.launch{value: 0.0005 ether}(_params(0, "a"));
        vm.expectRevert(ListingRegistry.NoListing.selector);
        real.launch{value: 0.0005 ether}(_params(9, "a"));
        vm.stopPrank();
    }

    function test_theSameSaltTwiceFails_anotherCreatorsSaltDoesNot() public {
        vm.prank(creatorA);
        real.launch{value: 0.0005 ether}(_params(0, "same"));
        vm.prank(creatorA);
        vm.expectRevert();
        real.launch{value: 0.0005 ether}(_params(0, "same"));
        vm.deal(creatorB, 1 ether);
        vm.prank(creatorB);
        real.launch{value: 0.0005 ether}(_params(0, "same"));
        assertEq(real.coinCount(), 2);
    }

    function test_theLauncherHasNobodyToAsk() public {
        bytes4[3] memory sels = [bytes4(keccak256("owner()")), bytes4(keccak256("setFactory(address)")), bytes4(keccak256("withdraw()"))];
        for (uint256 i = 0; i < sels.length; ++i) {
            (bool ok,) = address(real).call(abi.encodeWithSelector(sels[i], address(this)));
            assertFalse(ok);
        }
    }
}
