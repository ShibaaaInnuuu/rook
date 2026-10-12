// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Clock} from "../utils/Clock.sol";
import {MockBeacon} from "../utils/Mocks.sol";
import {MockArbSys, MockERC20, MockL2GatewayRouter} from "./HomeMocks.sol";
import {HomeWrappedAdapter} from "../../src/homechain/HomeWrappedAdapter.sol";
import {IArbSys, IL2GatewayRouter, IHomeRedeemDeskIntent, AddressAlias} from "../../src/homechain/IHomeChain.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// docs/v6/home-chain-vault.md §10.3: the twin's face on 4663. Units 1:1 against twin both ways; a burn or relist escrows the
/// unit's twin, sends the intent up and opens a ticket; only the desk's alias acknowledges; anyone dispatches an acknowledged
/// ticket's twin up to the desk and nowhere else; an unacknowledged ticket refunds after 21 days on both clocks. The ledger
/// `totalSupply + openTickets × ONE_ITEM == escrowed == twin balance − stray` holds at every step.
contract HomeAdapterTest is Test {
    MockERC20 twin;
    MockL2GatewayRouter gateway;
    MockArbSys arbSys;
    MockBeacon beacon;
    HomeWrappedAdapter a;

    address homeWrapper = makeAddr("HomeWrapper on Ethereum");
    address homeDesk = makeAddr("HomeRedeemDesk on Ethereum");
    address deskAlias;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address vault = makeAddr("a sweep vault");
    uint256 constant ONE = 1e18;

    function setUp() public {
        vm.warp(1_790_000_000);
        vm.roll(70_000_000);
        twin = new MockERC20();
        gateway = new MockL2GatewayRouter(twin);
        arbSys = new MockArbSys();
        beacon = new MockBeacon();
        a = new HomeWrappedAdapter(IERC20(address(twin)), homeWrapper, homeDesk, IL2GatewayRouter(address(gateway)), IArbSys(address(arbSys)),
            IRandomnessBeacon(address(beacon)), 12, "Wrapped Punk (4663)", "wPUNK");
        deskAlias = AddressAlias.l2(homeDesk);
        twin.mint(alice, 10 * ONE);
        vm.prank(alice);
        twin.approve(address(a), type(uint256).max);
    }

    function _assertLedger() internal view {
        assertEq(a.totalSupply() + a.openTickets() * ONE, a.escrowed(), "supply + open tickets == escrowed");
        assertEq(a.escrowed() + a.stray(), twin.balanceOf(address(a)), "escrowed + stray == twin held");
    }

    function _deposit(address to, uint256 amount) internal {
        vm.prank(alice);
        a.deposit(amount, to);
    }

    // ---- construction and the interface ---------------------------------------------------------------------------------------------

    function test_constructorRefusals_andWhatItIs() public {
        vm.expectRevert(HomeWrappedAdapter.BadInput.selector);
        new HomeWrappedAdapter(IERC20(alice), homeWrapper, homeDesk, IL2GatewayRouter(address(gateway)), IArbSys(address(arbSys)),
            IRandomnessBeacon(address(beacon)), 12, "x", "x");                           // a twin with no code
        vm.expectRevert(HomeWrappedAdapter.BadInput.selector);
        new HomeWrappedAdapter(IERC20(address(twin)), homeWrapper, address(0), IL2GatewayRouter(address(gateway)), IArbSys(address(arbSys)),
            IRandomnessBeacon(address(beacon)), 12, "x", "x");
        assertEq(a.custodian(), address(a), "custody is a contract on the home chain; here, nobody");
        assertEq(a.ONE_ITEM(), ONE);
        assertEq(address(a.beacon()), address(beacon));
        assertEq(a.homeDeskAlias(), deskAlias);
        assertEq(a.held(), 0);
        assertEq(a.openTickets(), 0);
    }

    // ---- units against twin ----------------------------------------------------------------------------------------------------------

    function test_depositAndWithdrawAreOneToOneBothWays() public {
        _deposit(bob, 3 * ONE);
        assertEq(a.balanceOf(bob), 3 * ONE);
        assertEq(twin.balanceOf(address(a)), 3 * ONE);
        assertEq(a.held(), 3);
        _assertLedger();
        vm.prank(bob);
        a.withdraw(ONE + 1, alice);
        assertEq(a.balanceOf(bob), 2 * ONE - 1);
        assertEq(twin.balanceOf(alice), 7 * ONE + ONE + 1);
        assertEq(a.held(), 1, "whole items only");
        _assertLedger();
        vm.expectRevert(HomeWrappedAdapter.ZeroAmount.selector);
        a.deposit(0, bob);
        vm.prank(bob);
        vm.expectRevert(HomeWrappedAdapter.ZeroAmount.selector);
        a.withdraw(0, bob);
        vm.prank(bob);
        vm.expectRevert();                                     // more than bob has: the ERC-20's own refusal
        a.withdraw(3 * ONE, bob);
    }

    function test_twinSentByPlainTransferIsStrayAndNeverCounted() public {
        _deposit(bob, ONE);
        vm.prank(alice);
        twin.transfer(address(a), ONE);
        assertEq(a.stray(), ONE);
        assertEq(a.held(), 1);
        _assertLedger();
        vm.prank(bob);
        a.withdraw(ONE, bob);
        assertEq(a.stray(), ONE, "a withdrawal takes the unit's twin, not the stray");
        _assertLedger();
    }

    // ---- the intent, the ticket, the refund ---------------------------------------------------------------------------------------------

    function test_redeemToBurn_escrowsTheTwinAndSendsTheIntentUp() public {
        _deposit(vault, 2 * ONE);
        vm.prank(vault);
        uint256 id = a.redeemToBurn();
        assertEq(id, 0);
        assertEq(a.balanceOf(vault), ONE);
        assertEq(a.openTickets(), 1);
        assertEq(twin.balanceOf(address(a)), 2 * ONE, "the twin stays escrowed here");
        _assertLedger();
        HomeWrappedAdapter.Ticket memory t = a.ticket(0);
        assertEq(uint8(t.stage), uint8(HomeWrappedAdapter.Stage.Open));
        assertEq(uint8(t.kind), uint8(HomeWrappedAdapter.Kind.Burn));
        assertEq(t.owner, vault);
        MockArbSys.Sent memory m = arbSys.last();
        assertEq(m.destination, homeDesk, "the intent goes to the desk");
        assertEq(m.from, address(a));
        assertEq(m.data, abi.encodeCall(IHomeRedeemDeskIntent.intent, (0, 0, vault)));
        vm.prank(vault);
        uint256 id2 = a.redeemToRelist();
        assertEq(id2, 1);
        assertEq(uint8(a.ticket(1).kind), uint8(HomeWrappedAdapter.Kind.Relist));
        assertEq(arbSys.last().data, abi.encodeCall(IHomeRedeemDeskIntent.intent, (1, 1, vault)));
        assertEq(a.balanceOf(vault), 0);
        assertEq(a.openTickets(), 2);
        assertEq(a.held(), 2, "both units' twin is still here, on tickets");
        _assertLedger();
        vm.prank(vault);
        vm.expectRevert();                                     // no unit left to burn
        a.redeemToBurn();
    }

    function test_refund_onlyAnOpenTicket_onlyAfterTheDeadlineOnBothClocks_unitBackToTheOwner() public {
        _deposit(vault, ONE);
        vm.prank(vault);
        a.redeemToBurn();
        vm.expectRevert(HomeWrappedAdapter.TooEarly.selector);
        a.refund(0);
        Clock.skipTimeOnly(21 days);                           // one clock only: a sequencer stamping the future
        vm.expectRevert(HomeWrappedAdapter.TooEarly.selector);
        a.refund(0);
        Clock.skip(21 days);
        a.refund(0);
        assertEq(a.balanceOf(vault), ONE, "the unit is back");
        assertTrue(a.refunded(0));
        assertEq(a.openTickets(), 0);
        assertEq(twin.balanceOf(address(a)), ONE, "the twin never moved");
        _assertLedger();
        vm.expectRevert(HomeWrappedAdapter.WrongStage.selector);
        a.refund(0);
        vm.prank(deskAlias);
        vm.expectRevert(HomeWrappedAdapter.WrongStage.selector);   // a late ack for a refunded ticket
        a.acknowledge(0);
        vm.expectRevert(HomeWrappedAdapter.BadInput.selector);
        a.refund(7);
    }

    // ---- the acknowledgement and the dispatch --------------------------------------------------------------------------------------------

    function test_acknowledge_onlyFromTheDesksAlias_thenNoRefund() public {
        _deposit(vault, ONE);
        vm.prank(vault);
        a.redeemToBurn();
        vm.prank(homeDesk);                                    // the desk's own address is NOT its alias
        vm.expectRevert(HomeWrappedAdapter.NotHomeDesk.selector);
        a.acknowledge(0);
        vm.expectRevert(HomeWrappedAdapter.NotHomeDesk.selector);
        a.acknowledge(0);
        vm.prank(deskAlias);
        a.acknowledge(0);
        assertEq(uint8(a.ticket(0).stage), uint8(HomeWrappedAdapter.Stage.Acknowledged));
        vm.prank(deskAlias);
        vm.expectRevert(HomeWrappedAdapter.WrongStage.selector);
        a.acknowledge(0);
        Clock.skip(60 days);
        vm.expectRevert(HomeWrappedAdapter.WrongStage.selector);   // committed: never refunded
        a.refund(0);
        assertEq(a.openTickets(), 1);
        _assertLedger();
    }

    function test_dispatch_anyone_afterTheAck_burnsTheTwinThroughTheGatewayToTheDeskOnly() public {
        _deposit(vault, 2 * ONE);
        vm.prank(vault);
        a.redeemToBurn();
        vm.expectRevert(HomeWrappedAdapter.WrongStage.selector);   // not acknowledged yet
        a.dispatch(0);
        vm.prank(deskAlias);
        a.acknowledge(0);
        vm.prank(bob);
        a.dispatch(0);
        assertEq(uint8(a.ticket(0).stage), uint8(HomeWrappedAdapter.Stage.Dispatched));
        assertEq(a.openTickets(), 0);
        assertEq(twin.balanceOf(address(a)), ONE, "the ticket's twin was burned by the gateway");
        assertEq(a.escrowed(), ONE, "the other unit's twin is still escrowed");
        MockL2GatewayRouter.Withdrawal memory w = gateway.last();
        assertEq(w.l1Token, homeWrapper);
        assertEq(w.to, homeDesk, "to the desk and nowhere else");
        assertEq(w.amount, ONE);
        assertEq(w.from, address(a));
        _assertLedger();
        vm.expectRevert(HomeWrappedAdapter.WrongStage.selector);   // once
        a.dispatch(0);
        assertFalse(a.refunded(0));
    }

    function test_dispatch_aGatewayThatDoesNotBurnIsRefused() public {
        _deposit(vault, ONE);
        vm.prank(vault);
        a.redeemToBurn();
        vm.prank(deskAlias);
        a.acknowledge(0);
        gateway.setRefuse(true);
        vm.expectRevert("gateway paused");
        a.dispatch(0);
        assertEq(uint8(a.ticket(0).stage), uint8(HomeWrappedAdapter.Stage.Acknowledged), "still acknowledged: dispatch again later");
        _assertLedger();
    }

    // ---- the ledger under any sequence ------------------------------------------------------------------------------------------------------

    /// Deposits, withdrawals, burns, acks, dispatches and refunds in any order: the twin here is always exactly the units plus the
    /// open tickets, twin leaves only by withdrawal or dispatch, and every dispatch went to the desk.
    function testFuzz_ledgerHoldsAndTwinLeavesOnlyByWithdrawalOrDispatch(uint256 seed, uint8 steps) public {
        uint256 n = bound(steps, 1, 40);
        uint256 withdrawn; uint256 dispatched;
        uint256 aliceStart = twin.balanceOf(alice);
        for (uint256 i = 0; i < n; ++i) {
            uint256 r = uint256(keccak256(abi.encode(seed, i)));
            uint256 op = r % 6;
            if (op == 0) {                                        // deposit up to 2 units to the vault
                uint256 amt = bound(r >> 8, 1, 2 * ONE);
                if (twin.balanceOf(alice) >= amt) _deposit(vault, amt);
            } else if (op == 1) {                                 // withdraw some of the vault's units to alice
                uint256 bal = a.balanceOf(vault);
                if (bal != 0) { uint256 amt = bound(r >> 8, 1, bal); vm.prank(vault); a.withdraw(amt, alice); withdrawn += amt; }
            } else if (op == 2) {                                 // burn
                if (a.balanceOf(vault) >= ONE) { vm.prank(vault); a.redeemToBurn(); }
            } else if (op == 3) {                                 // ack the oldest open ticket
                uint256 id = _first(HomeWrappedAdapter.Stage.Open);
                if (id != type(uint256).max) { vm.prank(deskAlias); a.acknowledge(id); }
            } else if (op == 4) {                                 // dispatch the oldest acknowledged ticket
                uint256 id = _first(HomeWrappedAdapter.Stage.Acknowledged);
                if (id != type(uint256).max) { a.dispatch(id); dispatched += ONE; }
            } else {                                              // let 21 days pass and refund the oldest open ticket
                Clock.skip(21 days);
                uint256 id = _first(HomeWrappedAdapter.Stage.Open);
                if (id != type(uint256).max) a.refund(id);
            }
            _assertLedger();
        }
        uint256 depositedNet = aliceStart - twin.balanceOf(alice) + withdrawn;   // alice's twin that went in, before withdrawals came back
        assertEq(twin.balanceOf(address(a)), depositedNet - withdrawn - dispatched, "twin left only by withdrawal or dispatch");
        for (uint256 i = 0; i < gateway.count(); ++i) {
            (, address to,,) = gateway.withdrawals(i);
            assertEq(to, homeDesk);
        }
        assertEq(gateway.count() * ONE, dispatched, "one dispatch per dispatched ticket");
    }

    function _first(HomeWrappedAdapter.Stage s) internal view returns (uint256) {
        uint256 n = a.ticketCount();
        for (uint256 i = 0; i < n; ++i) if (a.ticket(i).stage == s) return i;
        return type(uint256).max;
    }
}
