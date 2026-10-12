// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SweepBase} from "../utils/SweepBase.sol";
import {Merkle} from "../utils/Merkle.sol";
import {SweepVault} from "../../src/SweepVault.sol";

/// R4 check of A3-I2: the empty-proof refusal is bypassed by one junk sibling (Info: any bad root is the monitor's).
contract Audit4Draws is SweepBase {
    address guardian = makeAddr("guardian");
    address holder = makeAddr("holder");

    function setUp() public override {
        super.setUp();
        _stockVenue(40);
        _sweepWhole(raffle, coinR, routerR, 30);
        vm.prank(owner);
        registry.setGuardian(guardian);
    }

    function _skip(uint256 secs) internal {
        vm.warp(vm.getBlockTimestamp() + secs);
        vm.roll(vm.getBlockNumber() + secs / PBS + 1);
    }

    function _drand(uint64 round) internal pure returns (bytes32) { return keccak256(abi.encode("drand", round)); }
    function _sig(uint256 id) internal view returns (bytes memory) { return abi.encodePacked(_drand(raffle.draw(id).seedRound)); }
    function _at(uint256 id) internal view returns (uint64) { return raffle.snapshotTimeOf(id, _drand(raffle.draw(id).seedRound)); }

    function _announce() internal returns (uint256 id) {
        vm.prank(keeper);
        id = raffle.announceDraw(coinR);
        _skip(2 minutes);
    }

    /// A keeper-only re-roll that does not need the Empty stage: open with totalTickets = N > 0 and root = the sentinel leaf alone
    /// (`leaf(0, N, at)`, empty proof). `MerkleProof.verify([], root, root)` is true, so the open is accepted; the pin consumes the
    /// carry; nobody can ever claim (the only leaf is holder 0), so after VOID_AFTER the unit returns and the next draw pins a NEW
    /// seed. Same class as a false empty (a bad root the monitor must catch), but it passes through the ordinary pin path, and a
    /// monitor keyed only on `totalTickets == 0` for "false empty" pages would not flag it as such.
    function test_audit4_sentinelPlusJunkNodeStillConsumesTheCarry() public {
        uint256 first = _announce();
        _skip(6 hours);
        raffle.voidDraw(first);
        uint256 id = _announce();
        assertEq(raffle.seedOrigin(id), first);
        uint64 at = _at(id);
        bytes32[] memory tree = new bytes32[](2);
        tree[0] = keccak256("junk: no holder can prove this");
        tree[1] = raffle.leaf(address(0), 7, at);
        bytes32 root = Merkle.root(tree);
        bytes memory sig = _sig(id);
        vm.prank(keeper);
        raffle.openDraw(id, root, 7, "u", Merkle.proof(tree, 1), sig);   // accepted: a one-node proof
        _skip(16 minutes);
        raffle.requestDraw(id);
        assertEq(raffle.carriedSeed(coinR), 0, "carry consumed by a draw nobody can win");
        SweepVault.Draw memory d = raffle.draw(id);
        _skip(2 minutes);
        raffle.resolve(id, abi.encodePacked(_drand(d.round)));
        vm.expectRevert(SweepVault.BadProof.selector);
        raffle.claim(id, address(0), 7, at, new bytes32[](0));
        _skip(7 days + 1);
        raffle.voidDraw(id);
        uint256 next = _announce();
        assertEq(raffle.seedOrigin(next), next, "a fresh seed: the carried snapshot time was discarded");
    }
}
