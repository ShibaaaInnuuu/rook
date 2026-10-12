// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {NftWrapBase} from "./NftWrapBase.sol";
import {NftWrapper} from "../../src/nft/NftWrapper.sol";
import {Clock} from "../utils/Clock.sol";

/// Every actor, in any order, against the NFT wrapper: the canister mints (fresh ids, held ids, replayed attestations) and
/// records deliveries; holders move, redeem and burn; anyone resolves, refunds and redraws; time passes.
/// @dev As test/Wrapper.invariant.t.sol: every catch inspects the revert, a Panic anywhere is a violation, and counters prove the
/// campaign reached the states that matter.
contract NftWrapHandler is Test {
    NftWrapper public w;
    address custodian;
    address[3] actors;
    uint256 constant IDS = 12;          // a small id space, so re-mints of held and delivered ids really happen

    uint256 public panics;
    uint256 public stuck;
    uint256 public mints;
    uint256 public heldRemints;         // a mint of a held id that SUCCEEDED. Must stay zero.
    uint256 public replays;             // a replayed (id, tx) that SUCCEEDED. Must stay zero.
    uint256 public resolves;
    uint256 public deliveries;
    uint256 public refunds;
    uint256 public redeems;
    uint256 public burnsDelivered;
    mapping(uint256 => uint256) public nonceOf;   // how many distinct txs each id has been attested by

    constructor(NftWrapper w_, address custodian_, address[3] memory actors_) {
        w = w_; custodian = custodian_; actors = actors_;
    }

    function _isPanic(bytes memory r) private pure returns (bool) {
        return r.length >= 4 && r[0] == 0x4e && r[1] == 0x48 && r[2] == 0x7b && r[3] == 0x71;
    }

    function _caught(bytes memory r) private { if (_isPanic(r)) panics++; }

    function mint(uint256 tokenId, uint8 who, bool deposit) external {
        tokenId = tokenId % IDS;
        bool wasHeld = w.itemOf(tokenId) != 0;
        bytes32 txh = keccak256(abi.encode(tokenId, nonceOf[tokenId]));
        vm.prank(custodian);
        if (deposit) {
            try w.mintDeposit(tokenId, txh, actors[who % 3]) { mints++; nonceOf[tokenId]++; if (wasHeld) heldRemints++; }
            catch (bytes memory r) { _caught(r); }
        } else {
            try w.mint(tokenId, txh, actors[who % 3]) { mints++; nonceOf[tokenId]++; if (wasHeld) heldRemints++; }
            catch (bytes memory r) { _caught(r); }
        }
    }

    /// The canister replays an attestation it already used: never mints.
    function replay(uint256 tokenId) external {
        tokenId = tokenId % IDS;
        if (nonceOf[tokenId] == 0) return;
        bytes32 txh = keccak256(abi.encode(tokenId, nonceOf[tokenId] - 1));
        vm.prank(custodian);
        try w.mint(tokenId, txh, actors[0]) { replays++; } catch (bytes memory r) { _caught(r); }
    }

    function move(uint8 from, uint8 to, uint256 amount) external {
        address f = actors[from % 3];
        amount = bound(amount, 0, w.balanceOf(f));
        vm.prank(f);
        try w.transfer(actors[to % 3], amount) {} catch (bytes memory r) { _caught(r); }
    }

    function redeem(uint8 who, bool toBurn) external {
        address a = actors[who % 3];
        if (w.balanceOf(a) < w.ONE_ITEM()) return;
        vm.prank(a);
        if (toBurn) {
            try w.redeemToBurn() { redeems++; } catch (bytes memory r) { _caught(r); }
        } else {
            try w.redeem(a) { redeems++; } catch (bytes memory r) { _caught(r); }
        }
    }

    function resolveNext(bytes32 seed) external {
        uint256 id = w.nextToResolve();
        if (id >= w.ticketCount()) return;
        if (seed == bytes32(0)) seed = bytes32(uint256(1));
        // a ticket whose turn it is MUST resolve: any revert here is a liveness failure
        try w.resolve(id, abi.encodePacked(seed)) { resolves++; } catch { stuck++; }
    }

    function deliver(uint256 pick, bytes32 txh) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        uint256 id = pick % n;
        NftWrapper.Ticket memory t = w.ticket(id);
        if (t.stage != NftWrapper.Stage.Resolved) return;
        if (txh == bytes32(0)) txh = keccak256(abi.encode(id));
        vm.prank(custodian);
        try w.recordDelivery(id, txh) { deliveries++; if (t.destination == w.DEAD()) burnsDelivered++; }
        catch (bytes memory r) { _caught(r); }
    }

    function refund(uint256 pick) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        uint256 id = pick % n;
        if (w.ticket(id).stage != NftWrapper.Stage.Resolved) return;
        try w.refund(id) { refunds++; } catch (bytes memory r) { _caught(r); }
    }

    function redraw(uint256 pick) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        try w.redraw(pick % n) {} catch (bytes memory r) { _caught(r); }
    }

    function wait(uint256 secs) external {
        Clock.skip(bound(secs, 1 minutes, 8 days));
    }
}

contract NftWrapperInvariantTest is NftWrapBase {
    NftWrapHandler handler;
    address[3] actors;

    function setUp() public override {
        super.setUp();
        actors = [alice, bob, carol];
        handler = new NftWrapHandler(w, custodian, actors);
        targetContract(address(handler));
        bytes4[] memory actions = new bytes4[](9);
        actions[0] = NftWrapHandler.mint.selector;
        actions[1] = NftWrapHandler.replay.selector;
        actions[2] = NftWrapHandler.move.selector;
        actions[3] = NftWrapHandler.redeem.selector;
        actions[4] = NftWrapHandler.resolveNext.selector;
        actions[5] = NftWrapHandler.deliver.selector;
        actions[6] = NftWrapHandler.refund.selector;
        actions[7] = NftWrapHandler.redraw.selector;
        actions[8] = NftWrapHandler.wait.selector;
        targetSelector(FuzzSelector({addr: address(handler), selectors: actions}));
    }

    /// Each held NFT backs one whole token or one open ticket, never both: supply == (held − openTickets) × 1e18.
    function invariant_supplyIsHeldLessOpenTickets() public view {
        _assertLedger();
    }

    /// Every held NFT is in exactly one place: drawable, waiting, or named by one resolved ticket. And each is held under its
    /// own token id: no token id is on the books twice.
    function invariant_everyHeldNftIsInExactlyOnePlace() public view {
        uint256 free = w.freeCount();
        uint256 pending = w.pendingCount();
        uint256 n = w.ticketCount();
        uint256[] memory seen = new uint256[](free + pending + n);
        uint256 k;
        for (uint256 i = 0; i < free; ++i) seen[k++] = w.freeAt(i);
        for (uint256 i = 0; i < pending; ++i) { (uint256 itemId,) = w.pendingAt(i); seen[k++] = itemId; }
        for (uint256 i = 0; i < n; ++i) {
            NftWrapper.Ticket memory t = w.ticket(i);
            if (t.stage == NftWrapper.Stage.Resolved) seen[k++] = t.itemId;
        }
        assertEq(k, w.held(), "held NFTs, counted where they are");
        for (uint256 i = 0; i < k; ++i) {
            uint256 tokenId = w.item(seen[i]).tokenId;
            assertEq(w.itemOf(tokenId), seen[i] + 1, "a held item whose token id does not point back at it");
            for (uint256 j = i + 1; j < k; ++j) {
                assertTrue(seen[i] != seen[j], "one item in two places: promised twice");
                assertTrue(tokenId != w.item(seen[j]).tokenId, "one token id held twice");
            }
        }
    }

    /// The token ids marked held are exactly as many as `held`.
    function invariant_heldIsTheCountOfHeldTokenIds() public view {
        uint256 count;
        for (uint256 t = 0; t < 12; ++t) if (w.itemOf(t) != 0) count++;
        assertEq(count, w.held());
    }

    /// A tokenId can't be minted twice while held, and an attestation never mints twice.
    function invariant_noDoubleMint() public view {
        assertEq(handler.heldRemints(), 0, "a held token id minted again");
        assertEq(handler.replays(), 0, "a replayed attestation minted");
        assertEq(w.itemCount(), handler.mints());
    }

    /// Items minted = held + delivered: nothing appears or vanishes off the books.
    function invariant_everyItemIsHeldOrDelivered() public view {
        assertEq(w.itemCount(), w.held() + handler.deliveries());
        assertEq(w.burned(), handler.burnsDelivered());
    }

    /// Tickets resolve in order: everything before `nextToResolve` has resolved, nothing after it has.
    function invariant_ticketsResolveInOrder() public view {
        uint256 next = w.nextToResolve();
        uint256 n = w.ticketCount();
        assertLe(next, n);
        for (uint256 i = 0; i < n; ++i) {
            bool pinned = w.ticket(i).stage == NftWrapper.Stage.Pinned;
            assertEq(pinned, i >= next, "a ticket resolved out of turn");
        }
    }

    /// Nobody holds a token nobody minted.
    function invariant_everyTokenIsSomewhereWeCanSee() public view {
        uint256 sum;
        for (uint256 i = 0; i < 3; ++i) sum += w.balanceOf(actors[i]);
        assertEq(sum, w.totalSupply());
    }

    function invariant_noRequiredTransitionEverPanics() public view {
        assertEq(handler.panics(), 0);
        assertEq(handler.stuck(), 0, "a ticket whose turn it was could not resolve");
    }

    /// Not a vacuous harness: driven by hand along every transition, it really moves NFTs and every invariant holds.
    function test_theHandlerReallyMovesNfts() public {
        handler.mint(1, 0, false);
        handler.mint(2, 1, true);
        handler.mint(3, 2, false);
        handler.mint(1, 0, false);                     // held: refused
        handler.replay(2);                             // refused
        assertEq(handler.mints(), 3);
        handler.redeem(0, false);
        handler.redeem(1, true);
        handler.mint(4, 2, false);                     // after two burns: stamped for the third ticket onward
        handler.redeem(2, false);
        assertEq(handler.redeems(), 3);
        handler.resolveNext(keccak256("a"));
        handler.resolveNext(keccak256("b"));
        handler.resolveNext(keccak256("c"));
        assertEq(handler.resolves(), 3);
        handler.deliver(0, keccak256("tx0"));
        handler.deliver(1, keccak256("tx1"));
        assertEq(handler.deliveries(), 2);
        assertEq(handler.burnsDelivered(), 1);
        handler.wait(7 days);
        handler.wait(1 hours);
        handler.refund(2);
        assertEq(handler.refunds(), 1);
        handler.redeem(2, false);
        handler.wait(1 days);
        handler.redraw(3);
        handler.resolveNext(keccak256("d"));
        assertEq(handler.resolves(), 4);
        // a delivered id comes back by a new transaction
        uint256 back = w.item(w.ticket(0).itemId).tokenId;
        handler.mint(back, 0, true);
        assertEq(handler.mints(), 5);
        invariant_supplyIsHeldLessOpenTickets();
        invariant_everyHeldNftIsInExactlyOnePlace();
        invariant_heldIsTheCountOfHeldTokenIds();
        invariant_noDoubleMint();
        invariant_everyItemIsHeldOrDelivered();
        invariant_ticketsResolveInOrder();
        invariant_everyTokenIsSomewhereWeCanSee();
        invariant_noRequiredTransitionEverPanics();
    }
}
