// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {WrapBase} from "./utils/WrapBase.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {Distributor} from "../src/Distributor.sol";
import {Clock} from "./utils/Clock.sol";

/// Every actor, in any order, against the wrapper: wrap, move tokens, burn, resolve, deliver, refund, redraw, and time.
/// @dev In the style of docs/security/internal-review-v1.md I5: a bare `try … catch {}` hides a required transition that
/// panicked, and the books then balance because nothing happened. Every catch here inspects the revert, and a Panic anywhere
/// is a violation. `resolves`, `deliveries` and `refunds` prove the campaign reached the states that matter.
contract WrapHandler is Test {
    Wrapper public w;
    address keeper; address distribution; address creator;
    address[3] actors;
    uint256 lateFrog;                   // receipted and ruled out of any draw in setUp, wrapped whenever the campaign chooses
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";

    uint256 public panics;
    uint256 public resolves;
    uint256 public deliveries;
    uint256 public refunds;
    uint256 public redeems;
    uint256 public stuck;               // a ticket whose turn it was failed to resolve, for any reason. Must stay zero.

    constructor(Wrapper w_, address keeper_, address distribution_, address creator_, uint256 lateFrog_, address[3] memory actors_) {
        w = w_; keeper = keeper_; distribution = distribution_; creator = creator_; lateFrog = lateFrog_; actors = actors_;
    }

    function _isPanic(bytes memory r) private pure returns (bool) {
        return r.length >= 4 && r[0] == 0x4e && r[1] == 0x48 && r[2] == 0x7b && r[3] == 0x71;
    }

    function _caught(bytes memory r) private { if (_isPanic(r)) panics++; }

    function wrapTheLateFrog() external {
        if (w.wrapped(lateFrog)) return;
        vm.prank(creator);
        try w.wrap(lateFrog) {} catch (bytes memory r) { _caught(r); }
    }

    function claim(uint8 who) external {
        uint256 one = w.ONE_ITEM();             // read BEFORE the prank: a call in the argument list would consume it
        if (w.balanceOf(distribution) < one) return;
        vm.prank(distribution);
        try w.transfer(actors[who % 3], one) {} catch (bytes memory r) { _caught(r); }
    }

    function move(uint8 from, uint8 to, uint256 amount) external {
        address f = actors[from % 3];
        amount = bound(amount, 0, w.balanceOf(f));
        vm.prank(f);
        try w.transfer(actors[to % 3], amount) {} catch (bytes memory r) { _caught(r); }
    }

    function redeem(uint8 who) external {
        address a = actors[who % 3];
        if (w.balanceOf(a) < w.ONE_ITEM()) return;
        vm.prank(a);
        try w.redeem(TAPROOT) { redeems++; } catch (bytes memory r) { _caught(r); }
    }

    function resolveNext(bytes32 seed) external {
        uint256 id = w.nextToResolve();
        if (id >= w.ticketCount()) return;
        if (seed == bytes32(0)) seed = bytes32(uint256(1));
        // a ticket whose turn it is, with a burn behind it, MUST resolve: any revert here is a liveness failure
        try w.resolve(id, abi.encodePacked(seed)) { resolves++; } catch { stuck++; }
    }

    function deliver(uint256 pick, bytes32 txid) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        uint256 id = pick % n;
        if (w.ticket(id).stage != Wrapper.Stage.Resolved) return;
        if (txid == bytes32(0)) txid = keccak256(abi.encode(id));
        // the keeper tries too, and is always refused: only the custodian records (B-03, C-05); read before the prank
        address recorder = uint256(txid) % 2 == 0 ? keeper : w.custodian();
        vm.prank(recorder);
        try w.recordDelivery(id, txid) { deliveries++; } catch (bytes memory r) { _caught(r); }
    }

    function refund(uint256 pick) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        uint256 id = pick % n;
        if (w.ticket(id).stage != Wrapper.Stage.Resolved) return;
        vm.prank(w.custodian());
        w.markUndeliverable(id);                     // the custodian never sent it (review2 R3 §4)
        try w.refund(id) { refunds++; } catch (bytes memory r) { _caught(r); }
    }

    /// E-03: a ticket nobody resolved is refunded a week after its pin (there is no redraw: the pinned round is final).
    function refundPinned(uint256 pick) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        uint256 id = pick % n;
        if (w.ticket(id).stage != Wrapper.Stage.Pinned) return;
        try w.refund(id) { refunds++; } catch (bytes memory r) { _caught(r); }
    }

    function wait(uint256 secs) external {
        Clock.skip(bound(secs, 1 minutes, 8 days));
    }
}

contract WrapperInvariantTest is WrapBase {
    WrapHandler handler;
    address[3] actors = [makeAddr("alice"), makeAddr("bob"), makeAddr("carol")];

    function setUp() public override {
        super.setUp();
        _frog(0); _frog(1); _frog(2);
        uint256 late = _receipt(3);
        vm.prank(creatorA);
        distributor.choose(0, late, Distributor.Mode.Never, address(0));
        handler = new WrapHandler(wrapper, keeper, distribution, creatorA, late, actors);
        targetContract(address(handler));
        // the actions only: the handler is a `Test`, and its inherited functions would otherwise take most of the calls
        bytes4[] memory actions = new bytes4[](9);
        actions[0] = WrapHandler.wrapTheLateFrog.selector;
        actions[1] = WrapHandler.claim.selector;
        actions[2] = WrapHandler.move.selector;
        actions[3] = WrapHandler.redeem.selector;
        actions[4] = WrapHandler.resolveNext.selector;
        actions[5] = WrapHandler.deliver.selector;
        actions[6] = WrapHandler.refund.selector;
        actions[7] = WrapHandler.refundPinned.selector;
        actions[8] = WrapHandler.wait.selector;
        targetSelector(FuzzSelector({addr: address(handler), selectors: actions}));
    }

    /// docs/v3/wrapper.md, the ledger as corrected: each held frog backs one whole token or one open ticket, never both.
    function invariant_supplyIsHeldLessOpenTickets() public view {
        _assertLedger();
    }

    /// Every held frog is in exactly one place: drawable, waiting, or named by one open ticket.
    function invariant_everyHeldFrogIsInExactlyOnePlace() public view {
        uint256 free = wrapper.freeCount();
        uint256 pending = wrapper.pendingCount();
        uint256 n = wrapper.ticketCount();
        uint256[] memory seen = new uint256[](free + pending + n);
        uint256 k;
        for (uint256 i = 0; i < free; ++i) seen[k++] = wrapper.freeAt(i);
        for (uint256 i = 0; i < pending; ++i) { (uint256 aid,) = wrapper.pendingAt(i); seen[k++] = aid; }
        for (uint256 i = 0; i < n; ++i) {
            Wrapper.Ticket memory t = wrapper.ticket(i);
            if (t.stage == Wrapper.Stage.Resolved) seen[k++] = t.acquisitionId;
        }
        assertEq(k, wrapper.held(), "held frogs, counted where they are");
        for (uint256 i = 0; i < k; ++i) {
            assertTrue(wrapper.wrapped(seen[i]), "a frog on the books that was never wrapped");
            for (uint256 j = i + 1; j < k; ++j) assertTrue(seen[i] != seen[j], "one frog in two places: promised twice");
        }
    }

    /// Tickets resolve in order: everything before `nextToResolve` has resolved, nothing after it has.
    function invariant_ticketsResolveInOrder() public view {
        uint256 next = wrapper.nextToResolve();
        uint256 n = wrapper.ticketCount();
        assertLe(next, n);
        for (uint256 i = 0; i < n; ++i) {
            Wrapper.Stage st = wrapper.ticket(i).stage;
            if (i < next) assertTrue(st != Wrapper.Stage.Pinned, "a ticket skipped while still pinned");
            // after the turn: pinned, or refunded while pinned (E-03), never resolved out of turn
            else assertTrue(st == Wrapper.Stage.Pinned || st == Wrapper.Stage.Refunded, "a ticket resolved out of turn");
        }
        if (next < n) assertEq(uint8(wrapper.ticket(next).stage), uint8(Wrapper.Stage.Pinned), "the turn is a pinned ticket's");
    }

    /// Nobody holds a token nobody minted: the supply is exactly what the distribution contract and the actors hold.
    function invariant_everyTokenIsSomewhereWeCanSee() public view {
        uint256 sum = wrapper.balanceOf(distribution);
        for (uint256 i = 0; i < 3; ++i) sum += wrapper.balanceOf(actors[i]);
        assertEq(sum, wrapper.totalSupply());
    }

    /// A ticket whose turn it is always resolves: `NothingToDraw` is unreachable while the ledger holds, and nothing panics.
    function invariant_noRequiredTransitionEverPanics() public view {
        assertEq(handler.panics(), 0);
        assertEq(handler.stuck(), 0, "a ticket whose turn it was could not resolve");
    }

    /// Not a vacuous harness: driven by hand along every transition — a late wrap, burns, in-order resolution, a delivery,
    /// a refund — it really moves frogs, and every invariant holds at the end.
    function test_theHandlerReallyMovesFrogs() public {
        handler.claim(0); handler.claim(1); handler.claim(2);
        handler.redeem(0);
        handler.redeem(1);
        handler.wrapTheLateFrog();                     // after two burns: stamped for the third ticket onward
        handler.claim(2);
        handler.redeem(2);
        handler.redeem(2);
        assertEq(handler.redeems(), 4);
        handler.resolveNext(keccak256("a"));
        handler.resolveNext(keccak256("b"));
        handler.resolveNext(keccak256("c"));
        assertEq(handler.resolves(), 3);
        handler.deliver(0, keccak256("tx"));
        assertEq(handler.deliveries(), 1);
        handler.wait(7 days);
        handler.wait(3 days);                          // the refund grace
        handler.wait(1 hours);
        handler.refund(1);
        assertEq(handler.refunds(), 1, "an undelivered ticket really does refund");
        handler.wait(1 days);
        handler.resolveNext(keccak256("d"));           // the fourth ticket, still pinned days on: its round is still final
        assertEq(handler.resolves(), 4);
        invariant_supplyIsHeldLessOpenTickets();
        invariant_everyHeldFrogIsInExactlyOnePlace();
        invariant_ticketsResolveInOrder();
        invariant_everyTokenIsSomewhereWeCanSee();
        invariant_noRequiredTransitionEverPanics();
    }
}
