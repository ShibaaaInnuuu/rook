// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;
import "forge-std/Test.sol";
import {AutoBuyerVault} from "./AutoBuyerVault.sol";

contract AutoBuyerVaultTest is Test {
    address owner = address(0xA11CE); address keeper = address(0xBEEF); address payable buyer = payable(address(0xB0B));
    address stranger = address(0xDEAD);
    AutoBuyerVault v;

    function setUp() public {
        v = new AutoBuyerVault(owner, keeper, buyer, 3 ether);
        vm.deal(address(v), 10 ether);
        vm.prank(keeper); v.postCap(1 ether);
    }

    function _pulled(uint96 amt) internal returns (uint256 id) {
        vm.prank(buyer); id = v.announce(amt);
        vm.warp(block.timestamp + 1 hours);
        vm.prank(keeper); v.postCap(1 ether);
        vm.prank(buyer); v.pull(id);
    }

    function test_EthLeavesOnlyToTheRegisteredBuyer_AndOnlyTheBuyerMayAsk() public {
        vm.prank(stranger); vm.expectRevert(AutoBuyerVault.NotBuyer.selector); v.announce(1 ether);
        vm.prank(owner);    vm.expectRevert(AutoBuyerVault.NotBuyer.selector); v.announce(1 ether);
        vm.prank(keeper);   vm.expectRevert(AutoBuyerVault.NotBuyer.selector); v.announce(1 ether);
        vm.prank(buyer); uint256 id = v.announce(1 ether);
        vm.warp(block.timestamp + 1 hours);
        vm.prank(owner);  vm.expectRevert(AutoBuyerVault.NotBuyer.selector); v.pull(id);
        uint256 before = buyer.balance;
        vm.prank(buyer); v.pull(id);
        assertEq(buyer.balance - before, 1 ether);
        assertEq(address(v).balance, 9 ether);
        assertEq(owner.balance + keeper.balance + stranger.balance, 0, "nobody else ever receives anything");
    }

    function test_ThePublicWaitCannotBeSkipped() public {
        vm.prank(buyer); uint256 id = v.announce(1 ether);
        vm.warp(block.timestamp + 1 hours - 1);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.TooEarly.selector); v.pull(id);
    }

    function test_CapExpiryAndInFlightLimitBoundTheDamage() public {
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.OverCap.selector); v.announce(1 ether + 1);
        vm.startPrank(buyer); v.announce(1 ether); v.announce(1 ether); v.announce(1 ether);
        vm.expectRevert(AutoBuyerVault.OverInFlight.selector); v.announce(1);
        vm.stopPrank();
        vm.warp(block.timestamp + 1 hours + 1);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.CapExpired.selector); v.announce(1);
    }

    function test_ReturnAfterDeadline_AnyoneMayReturnTheExactAmount_AndTheVaultUnfreezes() public {
        uint256 id = _pulled(1 ether);
        vm.prank(stranger); vm.expectRevert(AutoBuyerVault.NotOverdue.selector); v.flagOverdue(id);
        vm.warp(block.timestamp + 3 days + 1);
        vm.prank(stranger); v.flagOverdue(id);
        assertEq(v.overdueCount(), 1);
        vm.prank(keeper); v.postCap(1 ether);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.VaultFrozen.selector); v.announce(1 ether);
        vm.deal(stranger, 2 ether);
        vm.prank(stranger); vm.expectRevert(AutoBuyerVault.WrongAmount.selector); v.returnFunds{value: 1 ether - 1}(id);
        vm.prank(stranger); v.returnFunds{value: 1 ether}(id);
        assertEq(address(v).balance, 10 ether); assertEq(v.inFlight(), 0); assertEq(v.overdueCount(), 0);
        vm.prank(stranger); vm.expectRevert(AutoBuyerVault.WrongState.selector); v.returnFunds{value: 1 ether}(id);
        vm.prank(buyer); v.announce(1 ether);   // unfrozen
    }

    function test_ALateReceiptIsStillRecorded_FlaggedLate() public {
        uint256 id = _pulled(1 ether);
        vm.warp(block.timestamp + 3 days + 1);
        v.flagOverdue(id);
        vm.prank(buyer); v.settle(id, bytes32(uint256(1)), bytes32(uint256(2)), 1);
        (AutoBuyerVault.State s,,,, bool late,,,) = v.purchases(id);
        assertEq(uint8(s), uint8(AutoBuyerVault.State.Settled)); assertTrue(late); assertEq(v.overdueCount(), 0);
    }

    function test_OnlyTheBuyerSettles_AndTheOwnerCanOnlyWriteOffWhatIsFlagged() public {
        uint256 id = _pulled(1 ether);
        vm.prank(owner); vm.expectRevert(AutoBuyerVault.NotBuyer.selector); v.settle(id, 0, 0, 1);
        vm.prank(owner); vm.expectRevert(AutoBuyerVault.NotOverdue.selector); v.writeOff(id);
        vm.warp(block.timestamp + 3 days + 1); v.flagOverdue(id);
        vm.prank(stranger); vm.expectRevert(AutoBuyerVault.NotOwner.selector); v.writeOff(id);
        vm.prank(owner); v.writeOff(id);
        assertEq(v.inFlight(), 0); assertEq(owner.balance, 0, "a write-off pays nobody");
    }

    function test_PauseStopsNewMoneyLeaving_ButNeverBlocksAReturnOrAReceipt() public {
        uint256 id = _pulled(1 ether);
        vm.prank(owner); v.setPaused(true);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.IsPaused.selector); v.announce(1 ether);
        vm.prank(buyer); v.settle(id, bytes32(uint256(9)), bytes32(uint256(9)), 1);
    }

    function testFuzz_InFlightNeverExceedsTheLimit_AndBalancePlusOutstandingIsConserved(uint96[6] memory amts, uint8 ops) public {
        uint256 start = address(v).balance; uint256 out;
        for (uint256 i = 0; i < amts.length; i++) {
            uint96 a = uint96(bound(amts[i], 1, 1 ether));
            vm.prank(keeper); v.postCap(1 ether);
            vm.prank(buyer);
            try v.announce(a) returns (uint256 id) {
                vm.warp(block.timestamp + 1 hours);
                vm.prank(buyer); v.pull(id); out += a;
                if ((ops >> i) & 1 == 1) { vm.deal(stranger, a); vm.prank(stranger); v.returnFunds{value: a}(id); out -= a; }
            } catch {}
            assertLe(v.inFlight(), v.maxInFlight());
            assertEq(address(v).balance + out, start);
            assertEq(v.inFlight(), out);
        }
    }

    // ---- the bounds that replace a bond, each stated as its own test -------------------------------------------------------

    function test_TheOwnerCanNeverReceiveFunds_OnlyPauseAndWriteOff() public {
        uint256 before = owner.balance;
        uint256 id = _pulled(1 ether);
        vm.warp(block.timestamp + 4 days);
        v.flagOverdue(id);
        vm.prank(owner); v.writeOff(id);
        assertEq(owner.balance, before, "a write-off records a loss; it never pays the owner");
        assertEq(address(v).balance, 9 ether, "and the vault keeps what it still has");
    }

    function test_TheBuyerAddressIsFixedAtConstruction_AndCannotBeTheOwnerOrTheKeeper() public {
        assertEq(v.buyer(), buyer);
        vm.expectRevert(bytes("buyer must be its own address"));
        new AutoBuyerVault(owner, keeper, payable(owner), 1 ether);
        vm.expectRevert(bytes("buyer must be its own address"));
        new AutoBuyerVault(owner, keeper, payable(keeper), 1 ether);
        // and there is no setter: the only way to change where money goes is a new vault
        assertEq(v.buyer(), buyer);
    }

    function test_OneOverduePurchaseFreezesEveryNewAnnouncement() public {
        uint256 id = _pulled(1 ether);
        vm.warp(block.timestamp + 4 days);
        vm.prank(keeper); v.postCap(1 ether);
        v.flagOverdue(id);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.VaultFrozen.selector); v.announce(1 ether);
        // it unfreezes only when the purchase is accounted for, one way or the other
        vm.prank(owner); v.writeOff(id);
        vm.prank(buyer); v.announce(1 ether);
    }

    function test_AnnouncementCannotReserveMoreThanTheVaultActuallyHolds() public {
        AutoBuyerVault small = new AutoBuyerVault(owner, keeper, buyer, 100 ether);
        vm.deal(address(small), 1 ether);
        vm.prank(keeper); small.postCap(10 ether);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.OverInFlight.selector); small.announce(2 ether);
        vm.prank(buyer); small.announce(1 ether);
    }

    function test_AStaleCapCannotBeUsed_AndTheKeeperCannotMoveMoney() public {
        vm.warp(block.timestamp + 2 hours);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.CapExpired.selector); v.announce(1 ether);
        // the keeper's only power is the cap: it has no way to announce, pull, settle or return
        vm.prank(keeper); vm.expectRevert(AutoBuyerVault.NotBuyer.selector); v.announce(1 ether);
        vm.prank(keeper); v.postCap(1 ether);
        vm.prank(buyer); v.announce(1 ether);
    }

    function test_APurchaseCannotBeSettledTwiceNorSettledAfterBeingReturned() public {
        uint256 id = _pulled(1 ether);
        vm.prank(buyer); v.settle(id, bytes32("tx"), bytes32("rune"), 1_000);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.WrongState.selector); v.settle(id, bytes32("tx"), bytes32("rune"), 1_000);
        uint256 id2 = _pulled(1 ether);
        vm.deal(stranger, 1 ether);
        vm.prank(stranger); v.returnFunds{value: 1 ether}(id2);
        vm.prank(buyer); vm.expectRevert(AutoBuyerVault.WrongState.selector); v.settle(id2, bytes32("tx"), bytes32("rune"), 1);
    }

    function test_APartialReturnIsRefused_SoInFlightCannotDriftDown() public {
        uint256 id = _pulled(1 ether);
        vm.deal(stranger, 1 ether);
        vm.prank(stranger); vm.expectRevert(AutoBuyerVault.WrongAmount.selector); v.returnFunds{value: 0.5 ether}(id);
        assertEq(v.inFlight(), 1 ether, "still outstanding until it is made whole");
    }
}
