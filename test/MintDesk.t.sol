// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "./utils/SweepBase.sol";
import {Clock} from "./utils/Clock.sol";
import {MintDesk} from "../src/curve/MintDesk.sol";
import {SweepVault} from "../src/SweepVault.sol";

/// docs/v3/curve.md, "Public mint": ETH escrowed for one frog, swapped for its wFROG in one step, or collected back.
contract MintDeskTest is SweepBase {
    address alice = makeAddr("alice");
    uint256 constant FLOOR = 0.64 ether;

    function _request(address who, uint256 amount) internal returns (uint256 id) {
        vm.deal(who, who.balance + amount);
        vm.prank(who);
        id = desk.request{value: amount}();
    }

    /// The canister mints the frog it bought to the desk, then fills the request at `cost`.
    function _fill(uint256 id, uint256 cost) internal {
        _deposit(address(desk));
        vm.prank(custodian);
        desk.fulfill(id, cost);
    }

    function test_aFilledRequestSwapsTheWrappedForTheCostAndReturnsTheChange() public {
        uint256 id = _request(alice, 0.7 ether);
        _fill(id, FLOOR);
        assertEq(payee.balance, FLOOR);
        vm.prank(alice);
        (uint256 w, uint256 eth) = desk.collect(id);
        assertEq(w, ONE);
        assertEq(eth, 0.06 ether);
        assertEq(wrapper.balanceOf(alice), ONE);
        assertEq(alice.balance, 0.06 ether);
        assertEq(address(desk).balance, 0);
        assertEq(desk.escrowed(), 0);
    }

    function test_theCanisterCannotChargeMoreThanWasEscrowed() public {
        uint256 id = _request(alice, 0.7 ether);
        _deposit(address(desk));
        vm.prank(custodian);
        vm.expectRevert(MintDesk.BadInput.selector);
        desk.fulfill(id, 0.7 ether + 1);
    }

    function test_noFillWithoutAWrappedFrogHere() public {
        uint256 id = _request(alice, 0.7 ether);
        vm.prank(custodian);
        vm.expectRevert(MintDesk.NoWrapped.selector);
        desk.fulfill(id, FLOOR);
    }

    function test_oneWrappedFillsOneRequest() public {
        uint256 a = _request(alice, 0.7 ether);
        uint256 b = _request(alice, 0.7 ether);
        _fill(a, FLOOR);
        vm.prank(custodian);
        vm.expectRevert(MintDesk.NoWrapped.selector);
        desk.fulfill(b, FLOOR);
    }

    function test_onlyTheCustodianFills() public {
        uint256 id = _request(alice, 0.7 ether);
        _deposit(address(desk));
        vm.prank(keeper);
        vm.expectRevert(MintDesk.NotCustodian.selector);
        desk.fulfill(id, FLOOR);
    }

    function test_anExpiredRequestIsCollectedInFullAndCannotBeFilled() public {
        uint256 id = _request(alice, 0.7 ether);
        vm.prank(alice);
        vm.expectRevert(MintDesk.WrongStatus.selector);
        desk.collect(id);                                     // still open, not expired
        Clock.skipTimeOnly(6 hours);                          // expired on either clock alone
        _deposit(address(desk));
        vm.prank(custodian);
        vm.expectRevert(MintDesk.Expired.selector);
        desk.fulfill(id, FLOOR);
        vm.prank(alice);
        (uint256 w, uint256 eth) = desk.collect(id);
        assertEq(w, 0);
        assertEq(eth, 0.7 ether);
    }

    function test_onlyTheRequesterCollects() public {
        uint256 id = _request(alice, 0.7 ether);
        _fill(id, FLOOR);
        vm.prank(stranger);
        vm.expectRevert(MintDesk.NotRequester.selector);
        desk.collect(id);
    }

    function test_collectOnce() public {
        uint256 id = _request(alice, 0.7 ether);
        _fill(id, FLOOR);
        vm.prank(alice);
        desk.collect(id);
        vm.prank(alice);
        vm.expectRevert(MintDesk.WrongStatus.selector);
        desk.collect(id);
    }

    function testFuzz_theDeskAlwaysHoldsWhatItOwes(uint96 amount, uint96 cost, bool fill, bool expire) public {
        amount = uint96(bound(amount, 1, 100 ether));
        cost = uint96(bound(cost, 0, amount));
        uint256 id = _request(alice, amount);
        if (fill) _fill(id, cost);
        if (expire) Clock.skip(6 hours);
        assertEq(address(desk).balance, desk.escrowed());
        assertGe(wrapper.balanceOf(address(desk)), desk.assigned());
        vm.prank(alice);
        try desk.collect(id) returns (uint256 w, uint256 eth) {
            assertEq(w + eth > 0 || amount == cost, true);
            assertEq(eth, fill ? amount - cost : amount);
        } catch {
            assertTrue(!fill && !expire);
        }
        assertEq(address(desk).balance, desk.escrowed());
    }

    // ---- the vault's canister route ----

    function test_aVaultSweepsThroughTheDeskChargedToOneCoin() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, 0.7 ether);
        vm.prank(keeper);                                     // keeper-only (audit A-05 / C-10)
        uint256 id = raffle.sweepViaDesk(coinR, 0.7 ether);
        assertEq(raffle.credit(coinR), 0.3 ether);
        assertEq(raffle.atDesk(), 0.7 ether);
        _fill(id, FLOOR);
        raffle.collectDesk(id);
        assertEq(raffle.units(coinR), ONE);
        assertEq(raffle.credit(coinR), 0.36 ether);
        assertEq(raffle.atDesk(), 0);
        _assertVaultBooks(raffle);
    }

    function test_theVaultNeverEscrowsMoreThanTheCapForOneFrog() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, 0.7 ether);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.BadInput.selector);
        raffle.sweepViaDesk(coinR, 0.7 ether + 1);
    }

    function test_theDeskRouteNeedsAFreshCap() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, 0.7 ether);
        Clock.skip(1 hours);
        vm.prank(keeper);
        vm.expectRevert(SweepVault.CapStale.selector);
        raffle.sweepViaDesk(coinR, 0.7 ether);
    }

    function test_onlyTheKeeperOpensADeskRequest() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, 0.7 ether);
        vm.prank(stranger);
        vm.expectRevert(SweepVault.NotKeeper.selector);
        raffle.sweepViaDesk(coinR, 0.7 ether);
    }

    function test_anExpiredDeskRequestComesBackToTheSameCoin() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, 0.7 ether);
        vm.prank(keeper);                                     // keeper-only (audit A-05 / C-10)
        uint256 id = raffle.sweepViaDesk(coinR, 0.7 ether);
        Clock.skip(6 hours);
        raffle.collectDesk(id);
        assertEq(raffle.credit(coinR), 1 ether);
        assertEq(raffle.units(coinR), 0);
        vm.expectRevert(SweepVault.WrongStage.selector);
        raffle.collectDesk(id);
    }

    function test_nobodyElseCanCollectTheVaultsRequest() public {
        _fundCoin(raffle, coinR, routerR, 1 ether);
        _postCap(raffle, 0.7 ether);
        vm.prank(keeper);                                     // keeper-only (audit A-05 / C-10)
        uint256 id = raffle.sweepViaDesk(coinR, 0.7 ether);
        _fill(id, FLOOR);
        vm.prank(stranger);
        vm.expectRevert(MintDesk.NotRequester.selector);
        desk.collect(id);                                     // so the wFROG can only arrive through collectDesk, attributed
    }
}
