// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {SweepBase, MockVenue} from "../../utils/SweepBase.sol";
import {Clock} from "../../utils/Clock.sol";
import {SweepVault} from "../../../src/SweepVault.sol";
import {Wrapper} from "../../../src/Wrapper.sol";
import {IRandomnessBeacon} from "../../../src/interfaces/IRandomnessBeacon.sol";

/// A beacon like drand's: a round has exactly one valid signature, and it exists only once the round's time has come. (The suites'
/// MockBeacon takes any signature; this one is etched over it, with the same storage layout.)
contract UniqueBeacon is IRandomnessBeacon {
    mapping(uint64 => bytes32) public randomnessOf;
    function roundAt(uint256 timestamp) external pure returns (uint64) { return uint64(timestamp / 3) + 1; }
    function timeOf(uint64 round) public pure returns (uint256) { return (uint256(round) - 1) * 3; }
    function signatureOf(uint64 round) public pure returns (bytes memory) { return abi.encodePacked(keccak256(abi.encode("drand", round))); }
    function randomnessFor(uint64 round) public pure returns (bytes32) { return keccak256(signatureOf(round)); }
    function prove(uint64 round, bytes calldata signature) external returns (bytes32 r) {
        r = randomnessOf[round];
        if (r != bytes32(0)) return r;
        require(block.timestamp >= timeOf(round), "the round does not exist yet");
        require(keccak256(signature) == keccak256(signatureOf(round)), "InvalidSignature");
        r = randomnessFor(round);
        randomnessOf[round] = r;
    }
}

/// Anyone, at any time, with any signature they like, resolves draws and tickets; the keeper announces and opens draws honestly.
contract FairnessHandler is Test {
    SweepVault public raffle;
    Wrapper public w;
    UniqueBeacon public beacon;
    address public keeper;
    address public coin;
    address[4] public actors;
    /// The round each draw and ticket was first seen pinned to: it must never change.
    mapping(uint256 => uint64) public drawRound;
    mapping(uint256 => uint64) public ticketRound;
    uint256 public resolvedDraws;
    uint256 public resolvedTickets;
    uint256 public refusedForgeries;

    constructor(SweepVault r, Wrapper w_, UniqueBeacon b, address k, address c) {
        raffle = r;
        w = w_;
        beacon = b;
        keeper = k;
        coin = c;
        actors = [makeAddr("f1"), makeAddr("f2"), makeAddr("f3"), makeAddr("f4")];
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / 12 + 1);
    }

    function wait(uint256 secs) external { _skip(bound(secs, 1, 3 days)); }

    /// The keeper, honest: announce, wait for the seed round, open with the sentinel committing the count and the time.
    function announceAndOpen(uint256 total) external {
        total = bound(total, 1, 1e24);
        vm.prank(keeper);
        try raffle.announceDraw(coin) returns (uint256 id) {
            SweepVault.Draw memory d = raffle.draw(id);
            if (vm.getBlockTimestamp() < beacon.timeOf(d.seedRound)) _skip(beacon.timeOf(d.seedRound) - vm.getBlockTimestamp());
            uint64 at = raffle.snapshotTimeOf(id, beacon.randomnessFor(d.seedRound));
            bytes32 holder = raffle.leaf(actors[0], 0, total);
            bytes32 sentinel = raffle.leaf(address(0), total, at);
            bytes32 root = holder < sentinel ? keccak256(abi.encodePacked(holder, sentinel)) : keccak256(abi.encodePacked(sentinel, holder));
            bytes32[] memory proof = new bytes32[](1);
            proof[0] = holder;
            bytes memory sig = beacon.signatureOf(d.seedRound);   // read before the prank
            vm.prank(keeper);
            raffle.openDraw(id, root, total, "x", proof, sig);
        } catch {}
    }

    function request(uint256 i, uint256 who) external {
        uint256 n = raffle.drawCount();
        if (n == 0) return;
        uint256 id = i % n;
        vm.prank(actors[who % 4]);
        try raffle.requestDraw(id) { drawRound[id] = raffle.draw(id).round; } catch {}
    }

    /// Anyone resolves a draw, at any time; a forged signature first, which must be refused.
    function resolveDraw(uint256 i, uint256 who, bytes32 junk) external {
        uint256 n = raffle.drawCount();
        if (n == 0) return;
        uint256 id = i % n;
        SweepVault.Draw memory d = raffle.draw(id);
        if (d.stage != SweepVault.Stage.Pinned) return;
        vm.startPrank(actors[who % 4]);
        if (beacon.randomnessOf(d.round) == bytes32(0)) {
            try raffle.resolve(id, abi.encodePacked(junk)) { } catch { refusedForgeries++; }
        }
        bytes memory sig = beacon.signatureOf(d.round);
        try raffle.resolve(id, sig) { resolvedDraws++; } catch {}
        vm.stopPrank();
    }

    function voidDraw(uint256 i) external {
        uint256 n = raffle.drawCount();
        if (n == 0) return;
        try raffle.voidDraw(i % n) {} catch {}
    }

    function redeem(uint256 who) external {
        address a = actors[who % 4];
        if (w.balanceOf(a) < 1e18) return;
        vm.prank(a);
        try w.redeem("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0") returns (uint256 id) {
            ticketRound[id] = w.ticket(id).round;
        } catch {}
    }

    function resolveTicket(uint256 who) external {
        uint256 id = w.nextToResolve();
        if (id >= w.ticketCount()) return;
        bytes memory sig = beacon.signatureOf(w.ticket(id).round);
        vm.prank(actors[who % 4]);
        try w.resolve(id, sig) { resolvedTickets++; } catch {}
    }

    function refund(uint256 i) external {
        uint256 n = w.ticketCount();
        if (n == 0) return;
        if (w.ticket(i % n).stage == Wrapper.Stage.Resolved) {
            vm.prank(w.custodian());
            w.markUndeliverable(i % n);                      // the custodian's attestation (review2 R3 §4)
        }
        try w.refund(i % n) {} catch {}
    }
}

/// Invariant (docs/audit/2026-09-26/fixes-B-C-keeper-draws.md, C-02/C-04/E-03): a draw's and a ticket's outcome is fixed when it is
/// pinned — independent of who resolves it and when, and nobody can replace the round.
contract DrawFairnessInvariantTest is SweepBase {
    FairnessHandler h;
    UniqueBeacon ub;

    function setUp() public override {
        super.setUp();
        vm.etch(address(beacon), address(new UniqueBeacon()).code);
        ub = UniqueBeacon(address(beacon));
        _stockVenue(30);
        _fundCoin(raffle, coinR, routerR, 30 * PRICE);
        _postCap(raffle, PRICE);
        raffle.sweep(coinR, 30 * PRICE, abi.encodeCall(MockVenue.buy, (PRICE)), 30 * ONE);
        h = new FairnessHandler(raffle, wrapper, ub, keeper, coinR);
        for (uint256 i = 0; i < 6; ++i) _deposit(h.actors(i % 4));   // frogs in custody, and holders to redeem them
        targetContract(address(h));
        bytes4[] memory s = new bytes4[](9);
        s[0] = FairnessHandler.wait.selector;
        s[1] = FairnessHandler.announceAndOpen.selector;
        s[2] = FairnessHandler.request.selector;
        s[3] = FairnessHandler.resolveDraw.selector;
        s[4] = FairnessHandler.voidDraw.selector;
        s[5] = FairnessHandler.redeem.selector;
        s[6] = FairnessHandler.resolveTicket.selector;
        s[7] = FairnessHandler.refund.selector;
        s[8] = FairnessHandler.resolveDraw.selector;
        targetSelector(FuzzSelector({addr: address(h), selectors: s}));
    }

    /// Every pinned, resolved or claimed draw is on the round it was first pinned to, and a resolved one's winner is that round's,
    /// whoever resolved it and whenever.
    function invariant_aDrawsOutcomeIsFixedAtItsPin() public view {
        for (uint256 id = 0; id < raffle.drawCount(); ++id) {
            SweepVault.Draw memory d = raffle.draw(id);
            if (d.stage == SweepVault.Stage.Pinned || d.stage == SweepVault.Stage.Resolved || d.stage == SweepVault.Stage.Claimed
                || (d.stage == SweepVault.Stage.Voided && d.round != 0)) {
                assertEq(d.round, h.drawRound(id), "the round was replaced");
            }
            if (d.stage == SweepVault.Stage.Resolved || d.stage == SweepVault.Stage.Claimed) {
                assertEq(d.winningTicket, uint256(keccak256(abi.encode(ub.randomnessFor(d.round), address(raffle), id))) % d.totalTickets,
                    "the winner is not the pinned round's");
            }
            if (d.stage == SweepVault.Stage.Open || d.stage == SweepVault.Stage.Pinned || d.stage == SweepVault.Stage.Resolved) {
                assertEq(d.snapshotAt, raffle.snapshotTimeOf(id, ub.randomnessFor(d.seedRound)), "the snapshot time is the seed's");
            }
        }
    }

    /// A ticket keeps the round pinned at its burn, for its whole life.
    function invariant_aTicketsRoundNeverChanges() public view {
        for (uint256 id = 0; id < wrapper.ticketCount(); ++id) {
            assertEq(wrapper.ticket(id).round, h.ticketRound(id), "a ticket was re-pinned");
        }
    }

    /// Not vacuous: driven by hand, draws and tickets really resolve, by different callers at different times, and forgeries fail.
    function test_theHandlerResolvesDrawsAndTickets() public {
        h.announceAndOpen(10);
        h.wait(15 minutes);
        h.request(0, 1);
        h.wait(2 days);
        h.resolveDraw(0, 2, keccak256("forged"));
        assertEq(h.resolvedDraws(), 1);
        assertEq(h.refusedForgeries(), 1);
        h.redeem(0);
        h.wait(1 hours);
        h.resolveTicket(3);
        assertEq(h.resolvedTickets(), 1);
        invariant_aDrawsOutcomeIsFixedAtItsPin();
        invariant_aTicketsRoundNeverChanges();
    }
}
