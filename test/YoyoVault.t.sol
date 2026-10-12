// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SweepBase} from "./utils/SweepBase.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {SweepVault} from "../src/SweepVault.sol";
import {CurveToken} from "../src/curve/CurveToken.sol";
import {MintDesk} from "../src/curve/MintDesk.sol";

/// A venue for a YOYO vault: sells wFROG for sweeps and the coin for buybacks, each at a price, with the same ways to misbehave
/// as `MockVenue`.
contract MockYoyoVenue {
    Wrapper public immutable w;
    IERC20 public immutable coin;
    constructor(Wrapper w_, IERC20 coin_) { w = w_; coin = coin_; }
    receive() external payable {}
    function buy(uint256 weiPerWhole) external payable { w.transfer(msg.sender, (msg.value * 1e18) / weiPerWhole); }
    function buyCoin(uint256 weiPerCoin) external payable { coin.transfer(msg.sender, (msg.value * 1e18) / weiPerCoin); }
    function buyCoinAndRefund(uint256 weiPerCoin, uint256 use) external payable {
        coin.transfer(msg.sender, (use * 1e18) / weiPerCoin);
        (bool ok,) = msg.sender.call{value: msg.value - use}("");
        require(ok);
    }
    function buyCoinFor(uint256 weiPerCoin, address to) external payable { coin.transfer(to, (msg.value * 1e18) / weiPerCoin); }
    function keep() external payable {}
}

/// docs/v5/policies-and-fees.md §1: the YOYO policy — relist at a markup over the cap, the sale's proceeds buy the coin back
/// and burn it, nothing ever leaves otherwise. On `SweepBase`'s frog wrapper, with a YOYO vault of its own (listing id 4's
/// pause flag, like the BURN vault) and a venue that sells both wFROG and the coin.
contract YoyoBase is SweepBase {
    SweepVault yoyo;
    MockYoyoVenue yoyoVenue;
    CurveToken coinY;
    address routerY = makeAddr("routerY");
    address alice = makeAddr("alice");
    address constant DEAD = 0x000000000000000000000000000000000000dEaD;
    uint256 constant COIN_PRICE = 0.0001 ether;    // the venue's price per coin

    function setUp() public virtual override {
        super.setUp();
        coinY = new CurveToken("Yoyo", "YOYO", 1_000_000_000e18, address(this));
        yoyoVenue = new MockYoyoVenue(wrapper, IERC20(address(coinY)));
        coinY.transfer(address(yoyoVenue), 500_000_000e18);
        yoyo = new SweepVault(registry, BURN_LISTING, wrapper, SweepVault.Policy.YOYO, address(yoyoVenue), launcher, desk, PBS);
        _ceiling(yoyo);
        launcher.set(address(coinY), routerY, creatorA, BURN_LISTING);
        vm.deal(routerY, 1_000 ether);
        for (uint256 i = 0; i < 8; ++i) _deposit(address(yoyoVenue));
    }

    /// Fund the coin, post the cap, sweep `n` whole wFROG into the YOYO vault.
    function _sweepY(uint256 n) internal {
        _fundCoin(yoyo, address(coinY), routerY, n * PRICE);
        _postCap(yoyo, PRICE);
        yoyo.sweep(address(coinY), n * PRICE, abi.encodeCall(MockYoyoVenue.buy, (PRICE)), n * ONE);
    }

    /// Relist one unit, resolve the ticket and have the custodian "deliver" (list) it.
    function _relistAndList() internal returns (uint256 id) {
        id = yoyo.relist(address(coinY));
        _resolveTicket(id, bytes32(uint256(7)));
        vm.prank(custodian);
        wrapper.recordDelivery(id, keccak256(abi.encode("listing", id)));
    }

    function _sale(uint256 id, uint256 amount) internal {
        vm.deal(custodian, custodian.balance + amount);
        vm.prank(custodian);
        yoyo.recordSale{value: amount}(id);
    }

    function _buyback(uint256 spend, uint256 minOut) internal returns (uint256 spent, uint256 burned) {
        vm.prank(keeper);
        (spent, burned) = yoyo.buyback(address(coinY), spend, abi.encodeCall(MockYoyoVenue.buyCoin, (COIN_PRICE)), minOut);
    }

    /// The vault holds every wei it owes, and every wFROG a coin is charged with.
    function _yoyoBooks() internal view {
        assertGe(address(yoyo).balance, yoyo.totalCredit() + yoyo.atDesk() + yoyo.totalProceeds(), "every wei it owes");
        assertGe(wrapper.balanceOf(address(yoyo)), yoyo.totalUnits() + yoyo.totalReserved(), "every unit it owes");
        assertEq(coinY.balanceOf(address(yoyo)), 0, "the vault never keeps the coin");
    }
}

contract YoyoVaultTest is YoyoBase {
    function test_relistIsTheYoyoPolicysOnly_andNeedsAFreshCapAndAUnit() public {
        vm.expectRevert(SweepVault.WrongPolicy.selector);
        raffle.relist(coinR);
        vm.expectRevert(SweepVault.WrongPolicy.selector);
        burn.relist(coinX);
        vm.expectRevert(SweepVault.CapStale.selector);
        yoyo.relist(address(coinY));
        _postCap(yoyo, PRICE);
        vm.expectRevert(SweepVault.Insufficient.selector);
        yoyo.relist(address(coinY));
        _sweepY(2);
        _skip61();
        vm.expectRevert(SweepVault.CapStale.selector);
        yoyo.relist(address(coinY));
    }

    function _skip61() internal {
        vm.warp(vm.getBlockTimestamp() + 61 minutes);
        vm.roll(vm.getBlockNumber() + 61 minutes / PBS + 1);
    }

    function test_relistBurnsOneUnitToAListingAtTheMarkupOverTheCap() public {
        _sweepY(2);
        uint256 id = yoyo.relist(address(coinY));
        assertEq(yoyo.units(address(coinY)), ONE, "one unit relisted");
        assertEq(yoyo.listedOpen(address(coinY)), 1);
        assertEq(yoyo.retiredBy(id), address(coinY), "charged to the coin, as a retirement is");
        assertEq(yoyo.retiredCount(), 1);
        SweepVault.Listing memory l = yoyo.listing(id);
        assertEq(uint8(l.stage), uint8(SweepVault.ListingStage.Listed));
        assertEq(l.ask, PRICE * 11 / 10, "1.1x the cap");
        assertEq(keccak256(bytes(wrapper.ticket(id).destination)), keccak256(bytes(wrapper.custodyAddress())),
            "the ticket's destination is custody itself: the item stays and is listed");
        _yoyoBooks();
        // resolved and delivered as any ticket: the item leaves the wrapped set while it is on the market
        uint256 held = wrapper.held();
        _resolveTicket(id, bytes32(uint256(7)));
        vm.prank(custodian);
        wrapper.recordDelivery(id, keccak256("listing"));
        assertEq(wrapper.held(), held - 1);
    }

    function test_theMarkupIsTheOwners_withinTheBounds() public {
        vm.expectRevert(SweepVault.NotOwner.selector);
        yoyo.setMarkup(1_500);
        vm.startPrank(owner);
        vm.expectRevert(SweepVault.BadMarkup.selector);
        yoyo.setMarkup(999);
        vm.expectRevert(SweepVault.BadMarkup.selector);
        yoyo.setMarkup(2_001);
        yoyo.setMarkup(2_000);
        vm.stopPrank();
        _sweepY(1);
        uint256 id = yoyo.relist(address(coinY));
        assertEq(yoyo.listing(id).ask, PRICE * 12 / 10, "1.2x");
    }

    function test_aSaleIsTheCustodiansReport_atLeastTheAskLessTheAllowance() public {
        _sweepY(1);
        uint256 id = _relistAndList();
        uint256 ask = yoyo.listing(id).ask;
        uint256 min = ask * 95 / 100;
        vm.deal(alice, 1 ether);
        vm.prank(alice);
        vm.expectRevert(SweepVault.NotCustodian.selector);
        yoyo.recordSale{value: ask}(id);
        vm.deal(custodian, 1 ether);
        vm.prank(custodian);
        vm.expectRevert(abi.encodeWithSelector(SweepVault.UnderAsk.selector, min - 1, min));
        yoyo.recordSale{value: min - 1}(id);
        _sale(id, ask);
        assertEq(yoyo.proceeds(address(coinY)), ask);
        assertEq(yoyo.totalProceeds(), ask);
        assertEq(yoyo.listedOpen(address(coinY)), 0);
        assertEq(yoyo.retiredCount(), 0);
        assertEq(uint8(yoyo.listing(id).stage), uint8(SweepVault.ListingStage.Sold));
        vm.prank(custodian);
        vm.expectRevert(SweepVault.WrongStage.selector);
        yoyo.recordSale{value: ask}(id);                        // never twice
        _yoyoBooks();
    }

    function test_theBuybackIsTheKeepers_measured_andEveryCoinGoesToDead() public {
        _sweepY(1);
        uint256 id = _relistAndList();
        uint256 ask = yoyo.listing(id).ask;
        _sale(id, ask);
        bytes memory route = abi.encodeCall(MockYoyoVenue.buyCoin, (COIN_PRICE));
        vm.expectRevert(SweepVault.NotKeeper.selector);
        yoyo.buyback(address(coinY), ask, route, 1);
        vm.startPrank(keeper);
        vm.expectRevert(SweepVault.WrongPolicy.selector);
        raffle.buyback(coinR, 1, route, 1);
        vm.expectRevert(SweepVault.Insufficient.selector);
        yoyo.buyback(address(coinY), ask + 1, route, 1);
        vm.expectRevert(abi.encodeWithSelector(SweepVault.Slippage.selector, ask * 1e18 / COIN_PRICE, type(uint256).max));
        yoyo.buyback(address(coinY), ask, route, type(uint256).max);
        // a route that pays somebody else, or keeps the ETH: refused, nothing moved
        vm.expectRevert(abi.encodeWithSelector(SweepVault.Slippage.selector, 0, 1));
        yoyo.buyback(address(coinY), ask, abi.encodeCall(MockYoyoVenue.buyCoinFor, (COIN_PRICE, alice)), 1);
        vm.expectRevert(abi.encodeWithSelector(SweepVault.Slippage.selector, 0, 1));
        yoyo.buyback(address(coinY), ask, abi.encodeCall(MockYoyoVenue.keep, ()), 1);
        vm.stopPrank();
        assertEq(yoyo.proceeds(address(coinY)), ask, "nothing spent by a failed route");
        uint256 dead0 = coinY.balanceOf(DEAD);
        (uint256 spent, uint256 burned) = _buyback(ask, 1);
        assertEq(spent, ask);
        assertEq(burned, ask * 1e18 / COIN_PRICE);
        assertEq(coinY.balanceOf(DEAD), dead0 + burned, "burned");
        assertEq(coinY.balanceOf(address(yoyo)), 0);
        assertEq(yoyo.proceeds(address(coinY)), 0);
        assertEq(yoyo.totalBurnedBack(), ask);
        _yoyoBooks();
    }

    function test_aBuybackTakesItsRefundBack() public {
        _sweepY(1);
        uint256 id = _relistAndList();
        uint256 ask = yoyo.listing(id).ask;
        _sale(id, ask);
        vm.prank(keeper);
        (uint256 spent,) = yoyo.buyback(address(coinY), ask, abi.encodeCall(MockYoyoVenue.buyCoinAndRefund, (COIN_PRICE, ask / 2)), 1);
        assertEq(spent, ask / 2);
        assertEq(yoyo.proceeds(address(coinY)), ask - ask / 2, "the refund is proceeds again");
        _yoyoBooks();
    }

    function test_anUnsoldItemComesBack_throughADepositToTheVault() public {
        _sweepY(1);
        uint256 id = _relistAndList();
        vm.prank(alice);
        vm.expectRevert(SweepVault.NotCustodian.selector);
        yoyo.reclaimListing(id);
        vm.prank(custodian);
        vm.expectRevert(SweepVault.Insufficient.selector);
        yoyo.reclaimListing(id);                                // the unit is not back yet
        _deposit(address(yoyo));                                // the custodian re-deposits the delisted frog to the vault
        vm.prank(custodian);
        yoyo.reclaimListing(id);
        assertEq(yoyo.units(address(coinY)), ONE, "the coin's again");
        assertEq(yoyo.listedOpen(address(coinY)), 0);
        assertEq(uint8(yoyo.listing(id).stage), uint8(SweepVault.ListingStage.Returned));
        vm.prank(custodian);
        vm.expectRevert(SweepVault.WrongStage.selector);
        yoyo.reclaimListing(id);
        vm.deal(custodian, 1 ether);
        vm.prank(custodian);
        vm.expectRevert(SweepVault.WrongStage.selector);
        yoyo.recordSale{value: 1 ether}(id);                    // sold after a return: no
        _yoyoBooks();
    }

    function test_aRefundedRelistTicketComesBackThroughReclaim() public {
        _sweepY(1);
        uint256 id = yoyo.relist(address(coinY));
        _resolveTicket(id, bytes32(uint256(7)));
        vm.warp(vm.getBlockTimestamp() + 7 days + 72 hours + 1);   // the deadline and the refund grace
        vm.roll(vm.getBlockNumber() + (7 days + 72 hours) / PBS + 2);
        vm.prank(custodian);
        wrapper.markUndeliverable(id);                             // the custodian's attestation (review2 R3 §4)
        wrapper.refund(id);
        yoyo.reclaim(id);
        assertEq(yoyo.units(address(coinY)), ONE);
        assertEq(yoyo.listedOpen(address(coinY)), 0);
        assertEq(uint8(yoyo.listing(id).stage), uint8(SweepVault.ListingStage.Returned));
        _yoyoBooks();
    }

    function test_nothingLeavesButTheBuyback() public {
        _sweepY(2);
        uint256 id = _relistAndList();
        _sale(id, yoyo.listing(id).ask);
        uint256 bal = address(yoyo).balance;
        // every entry a stranger has, with value where it takes it: none moves the proceeds
        vm.deal(alice, 10 ether);
        vm.startPrank(alice);
        vm.expectRevert(SweepVault.NotAccepted.selector);
        (bool ok,) = address(yoyo).call{value: 1}("");
        ok;
        vm.expectRevert(SweepVault.NotKeeper.selector);
        yoyo.buyback(address(coinY), 1, "", 1);
        vm.stopPrank();
        assertEq(address(yoyo).balance, bal);
        assertEq(address(yoyo).balance, yoyo.totalCredit() + yoyo.totalProceeds(), "credit and proceeds, nothing else");
    }

    function test_theYoyoVaultExposesNoRecipient() public view {
        string[] memory sigs = vm.parseJsonKeys(vm.readFile("out/SweepVault.sol/SweepVault.json"), ".methodIdentifiers");
        for (uint256 i = 0; i < sigs.length; ++i) {
            bytes32 h = keccak256(bytes(sigs[i]));
            assertTrue(h != keccak256("withdraw(uint256)") && h != keccak256("withdraw(address,uint256)")
                && h != keccak256("sendProceeds(address,uint256)") && h != keccak256("setVenue(address)"), sigs[i]);
        }
    }
}
