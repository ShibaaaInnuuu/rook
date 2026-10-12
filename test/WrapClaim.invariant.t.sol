// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ClaimBase} from "./utils/ClaimBase.sol";
import {Clock} from "./utils/Clock.sol";
import {Merkle} from "./utils/Merkle.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {WrapClaim} from "../src/WrapClaim.sol";

/// Every actor, in any order, against the wrapper AND its claim contract together: wrap, count, publish a root (honest or
/// over-promising), claim, cancel, roll over, move, burn, resolve, deliver, refund, redraw, and time.
/// @dev As the wrapper's campaign: every catch inspects the revert and a Panic anywhere is a violation. A claim the books say
/// must succeed and does not is `stuck`; a claim that pays anything but its leaf is a `violation`.
contract ClaimHandler is Test {
    Wrapper public w;
    WrapClaim public c;
    address keeper; address owner;
    address[3] actors;
    uint256[4] public frogs;            // 0 and 1 wrapped in setUp (coin A); 2 late (coin A); 3 late (coin B)
    address[4] creators;
    string constant TAPROOT = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";

    mapping(uint256 frog => uint256[3]) amounts;   // the allocation behind each frog's current root

    uint256 public panics;
    uint256 public stuck;
    uint256 public violations;
    uint256 public opens;
    uint256 public carriedOpens;        // opens whose total carried a previous frog's unclaimed tokens
    uint256 public claims;
    uint256 public cancels;
    uint256 public rollovers;
    uint256 public redeems;
    uint256 public resolves;
    uint256 public deliveries;
    uint256 public refunds;

    constructor(Wrapper w_, WrapClaim c_, address keeper_, address owner_, address[3] memory actors_, uint256[4] memory frogs_,
        address[4] memory creators_)
    {
        w = w_; c = c_; keeper = keeper_; owner = owner_; actors = actors_; frogs = frogs_; creators = creators_;
    }

    function _isPanic(bytes memory r) private pure returns (bool) {
        return r.length >= 4 && r[0] == 0x4e && r[1] == 0x48 && r[2] == 0x7b && r[3] == 0x71;
    }

    function _caught(bytes memory r) private { if (_isPanic(r)) panics++; }

    function _elapsed(uint64 t, uint64 b) private view returns (bool) { return block.timestamp >= t && block.number >= b; }

    function _leaves(uint256 frog) private view returns (bytes32[] memory l) {
        l = new bytes32[](3);
        for (uint256 i = 0; i < 3; ++i) l[i] = c.leaf(actors[i], amounts[frog][i]);
    }

    // ---- the claim contract ----

    function wrapLate(uint8 pick) external {
        uint256 k = 2 + pick % 2;
        if (w.wrapped(frogs[k])) return;
        address creator = creators[k];
        vm.prank(creator);
        try w.wrap(frogs[k]) {} catch (bytes memory r) { _caught(r); }
    }

    function count(uint8 pick) external {
        uint256 aid = frogs[pick % 4];
        if (!w.wrapped(aid)) return;
        try c.count(aid) {} catch (bytes memory r) { _caught(r); }
    }

    function open(uint8 pick, bool overPromise, uint256 seed) external {
        uint256 aid = frogs[pick % 4];
        if (!w.wrapped(aid)) return;
        WrapClaim.Distribution memory d = c.distribution(aid);
        if (d.open || d.rolled) return;
        uint256 total = c.ONE_ITEM() + c.carry(pick % 4 == 3 ? coinB : coinA);
        uint256[3] memory a;
        if (overPromise) {
            a[0] = total; a[1] = total; a[2] = total;
        } else {
            a[0] = bound(seed, 1, total - 2);
            a[1] = bound(uint256(keccak256(abi.encode(seed))), 1, total - a[0] - 1);
            a[2] = total - a[0] - a[1];
        }
        amounts[aid] = a;
        address expectCoin = pick % 4 == 3 ? coinB : coinA;
        bytes32 root = Merkle.root(_leaves(aid));        // built BEFORE the prank: `leaf` is an external call
        bool carried = total > c.ONE_ITEM();
        vm.prank(keeper);
        try c.open(aid, root, "ipfs://list") {
            opens++;
            if (carried) carriedOpens++;
            if (c.distribution(aid).coin != expectCoin) violations++;   // the vault's record of who paid, not ours
        } catch (bytes memory r) { _caught(r); }
    }

    /// Frogs 0–2 were paid for by coin A, frog 3 by coin B: the handler's own note, which `open` checks against the vault's.
    address public coinA; address public coinB;
    function setCoins(address a, address b) external { coinA = a; coinB = b; }

    function claim(uint8 pick, uint8 who) external {
        uint256 aid = frogs[pick % 4];
        WrapClaim.Distribution memory d = c.distribution(aid);
        if (!d.open) return;
        uint256 i = who % 3;
        address holder = actors[i];
        uint256 amount = amounts[aid][i];
        bytes32[] memory proof = Merkle.proof(_leaves(aid), i);
        bool must = !d.rolled && _elapsed(d.challengeEnds, d.challengeEndsBlock) && !_elapsed(d.claimsEnd, d.claimsEndBlock)
            && !c.claimed(aid, holder) && amount > 0 && d.claimed + amount <= d.total;
        uint256 before = w.balanceOf(holder);
        try c.claim(aid, holder, amount, proof) {
            claims++;
            if (w.balanceOf(holder) != before + amount) violations++;
            if (!must) violations++;                    // it paid when the books say it must not
        } catch (bytes memory r) {
            _caught(r);
            if (must) stuck++;
        }
    }

    function cancel(uint8 pick) external {
        vm.prank(owner);
        try c.cancel(frogs[pick % 4]) { cancels++; } catch (bytes memory r) { _caught(r); }
    }

    function rollOver(uint8 pick) external {
        try c.rollOver(frogs[pick % 4]) { rollovers++; } catch (bytes memory r) { _caught(r); }
    }

    // ---- the wrapper ----

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
        Clock.skip(bound(secs, 1 minutes, 20 days));
    }
}

contract WrapClaimInvariantTest is ClaimBase {
    ClaimHandler handler;
    address[3] actors = [makeAddr("alice"), makeAddr("bob"), makeAddr("carol")];
    uint256[4] frogs;

    function setUp() public override {
        super.setUp();
        frogs[0] = _frog(0);
        frogs[1] = _frog(1);
        frogs[2] = _receiptBy(2, coinA, creatorA);
        frogs[3] = _receiptBy(3, coinB, creatorB);
        address[4] memory creators = [creatorA, creatorA, creatorA, creatorB];
        handler = new ClaimHandler(wrapper, claim, keeper, owner, actors, frogs, creators);
        handler.setCoins(coinA, coinB);
        targetContract(address(handler));
        bytes4[] memory actions = new bytes4[](15);
        actions[0] = ClaimHandler.wrapLate.selector;
        actions[1] = ClaimHandler.count.selector;
        actions[2] = ClaimHandler.open.selector;
        actions[3] = ClaimHandler.claim.selector;
        actions[4] = ClaimHandler.cancel.selector;
        actions[5] = ClaimHandler.rollOver.selector;
        actions[6] = ClaimHandler.move.selector;
        actions[7] = ClaimHandler.redeem.selector;
        actions[8] = ClaimHandler.resolveNext.selector;
        actions[9] = ClaimHandler.deliver.selector;
        actions[10] = ClaimHandler.refund.selector;
        actions[11] = ClaimHandler.refundPinned.selector;
        actions[12] = ClaimHandler.wait.selector;
        actions[13] = ClaimHandler.claim.selector;       // claims weighted up: they are the thing under test
        actions[14] = ClaimHandler.open.selector;
        targetSelector(FuzzSelector({addr: address(handler), selectors: actions}));
    }

    /// The wrapper's ledger, unmoved by anything the claim contract does: each held frog backs one token or one open ticket.
    function invariant_theWrappersLedgerHolds() public view {
        _assertLedger();
    }

    /// Step 2's books: every token the claim contract holds is in exactly one place — a wrapped frog nobody has counted yet, a
    /// counted frog with no root, an open distribution's unclaimed part, or a coin's carry.
    function invariant_everyTokenTheClaimHoldsIsAccountedOnce() public view {
        uint256 expected = claim.unopened() * ONE + claim.carry(coinA) + claim.carry(coinB);
        for (uint256 k = 0; k < 4; ++k) {
            if (wrapper.wrapped(frogs[k]) && !claim.counted(frogs[k])) expected += ONE;
            WrapClaim.Distribution memory d = claim.distribution(frogs[k]);
            if (d.open && !d.rolled) expected += d.total - d.claimed;
        }
        assertEq(wrapper.balanceOf(address(claim)), expected, "the claim contract's balance, counted where it is");
    }

    /// No distribution ever pays past its total, and the total of an open one is at least one frog.
    function invariant_noDistributionPaysPastItsTotal() public view {
        for (uint256 k = 0; k < 4; ++k) {
            WrapClaim.Distribution memory d = claim.distribution(frogs[k]);
            assertLe(d.claimed, d.total);
            if (d.open) assertGe(d.total, ONE);
        }
    }

    /// Every token is somewhere we can see: the claim contract and the three actors hold the whole supply.
    function invariant_everyTokenIsSomewhereWeCanSee() public view {
        uint256 sum = wrapper.balanceOf(address(claim));
        for (uint256 i = 0; i < 3; ++i) sum += wrapper.balanceOf(actors[i]);
        assertEq(sum, wrapper.totalSupply());
    }

    /// Nothing panics; every claim the books say must pay, pays exactly its leaf; every ticket whose turn it is resolves.
    function invariant_claimsPayExactlyAndNothingIsStuck() public view {
        assertEq(handler.panics(), 0);
        assertEq(handler.stuck(), 0, "a claim or a resolution the books required did not happen");
        assertEq(handler.violations(), 0, "a claim paid other than its leaf, or paid when it must not");
    }

    /// Not a vacuous harness: driven by hand through every transition of both contracts — a root cancelled and republished,
    /// claims, an over-promising root stopped at its total, a roll-over carried into the coin's next frog, a second coin, and
    /// claimed tokens burned, resolved, delivered and refunded — and every invariant holds at the end.
    function test_theHandlerReallyMovesTokensAndFrogs() public {
        handler.open(0, true, 1);                       // an over-promising root for frog 0...
        handler.cancel(0);                              // ...withdrawn inside its window
        assertEq(handler.cancels(), 1);
        handler.open(0, false, 11);                     // and republished honestly
        handler.open(1, true, 0);                       // frog 1: over-promises, left to stand
        handler.wait(16 minutes);
        handler.claim(0, 0); handler.claim(0, 1);       // carol never claims her part of frog 0
        handler.claim(1, 0);                            // the first claim against an over-promising root pays a whole token
        handler.claim(1, 1);                            // the second is stopped at the total, not stuck
        assertEq(handler.claims(), 3);
        handler.wait(20 days); handler.wait(11 days);
        handler.rollOver(1);                            // nothing left of frog 1: rolls over zero
        handler.rollOver(0);                            // carol's part goes to coin A's carry
        assertEq(handler.rollovers(), 2);
        assertGt(claim.carry(coinA), 0);
        handler.wrapLate(0);                            // frog 2, coin A
        handler.open(2, false, 7);
        assertEq(handler.carriedOpens(), 1, "frog 2's total carried carol's unclaimed part");
        handler.wrapLate(1);                            // frog 3, coin B
        handler.count(3);
        handler.open(3, false, 99);
        handler.wait(16 minutes);
        handler.claim(2, 0); handler.claim(2, 1); handler.claim(2, 2);
        handler.claim(3, 2);
        assertEq(handler.opens(), 5);
        handler.redeem(0); handler.redeem(1); handler.redeem(2);
        assertGt(handler.redeems(), 0, "claimed tokens really do burn for frogs");
        handler.wait(2 minutes);
        handler.resolveNext(keccak256("a")); handler.resolveNext(keccak256("b"));
        assertGt(handler.resolves(), 0);
        handler.deliver(0, keccak256("tx"));
        handler.wait(8 days);
        handler.wait(3 days);                          // the refund grace
        handler.refund(1);
        assertGt(handler.deliveries() + handler.refunds(), 0);
        invariant_theWrappersLedgerHolds();
        invariant_everyTokenTheClaimHoldsIsAccountedOnce();
        invariant_noDistributionPaysPastItsTotal();
        invariant_everyTokenIsSomewhereWeCanSee();
        invariant_claimsPayExactlyAndNothingIsStuck();
    }
}
