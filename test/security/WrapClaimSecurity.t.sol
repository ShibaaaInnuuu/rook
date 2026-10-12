// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ClaimBase} from "../utils/ClaimBase.sol";
import {Clock} from "../utils/Clock.sol";
import {Merkle} from "../utils/Merkle.sol";
import {WrapClaim} from "../../src/WrapClaim.sol";

/// Step 2's attack surface: a proof carried to the wrong frog, a sequencer that stamps the future to close claims early, a
/// caller who pushes somebody else's claim, and the contract's whole ABI — nothing moves its tokens but `claim`.
contract WrapClaimSecurityTest is ClaimBase {
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    /// A proof for one frog's root is not a proof for another's, even when the same leaf is in both: `claimed` is per frog,
    /// and a leaf only pays against the root it was proven under.
    function test_aProofForOneFrogDoesNotPayFromAnother() public {
        uint256 a = _frog(0);
        uint256 b = _frog(1);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE, bob, ONE);
        address[] memory onlyBob = new address[](1); onlyBob[0] = bob;
        uint256[] memory whole = new uint256[](1); whole[0] = ONE;
        _open(a, h, n);                                 // frog A: alice and bob, a whole token each (the second overpays)
        _open(b, onlyBob, whole);                       // frog B: bob alone
        Clock.skip(15 minutes);
        bytes32[] memory alicesA = Merkle.proof(_leaves(h, n), 0);
        vm.expectRevert(WrapClaim.BadProof.selector);
        claim.claim(b, alice, ONE, alicesA);            // alice's proof under A, presented at B
        claim.claim(a, alice, ONE, alicesA);
        bytes32[] memory bobsA = Merkle.proof(_leaves(h, n), 1);
        vm.expectRevert(WrapClaim.OverTotal.selector);
        claim.claim(a, bob, ONE, bobsA);                // A's root overpays; it cannot reach B's token to do it
        bytes32[] memory bobsB = new bytes32[](0);
        claim.claim(b, bob, ONE, bobsB);                // B's own leaf still pays in full
        assertEq(wrapper.balanceOf(bob), ONE);
        assertEq(wrapper.balanceOf(address(claim)), 0);
    }

    /// A sequencer stamping the future cannot close claims early, so it cannot force a roll-over of tokens that were claimable:
    /// with only the timestamp past the 30 days, holders can still claim and nobody can roll over.
    function test_aJumpedTimestampNeitherClosesClaimsNorRollsThemOver() public {
        uint256 aid = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE / 2, bob, ONE / 2);
        _open(aid, h, n);
        Clock.skip(15 minutes);
        Clock.skipTimeOnly(40 days);
        vm.expectRevert(WrapClaim.TooEarly.selector);
        claim.rollOver(aid);
        _claim(aid, h, n, 0);
        assertEq(wrapper.balanceOf(alice), ONE / 2, "still claimable: the parent chain has not moved");
    }

    /// Pushing a holder's claim for them only ever sends the holder their own leaf, so doing it first gains a caller nothing,
    /// and a holder whose claim was pushed is paid exactly once.
    function testFuzz_pushingSomeoneElsesClaimOnlyPaysThem(address pusher) public {
        vm.assume(pusher != alice && pusher != bob && pusher != address(claim) && pusher != address(0));
        uint256 aid = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE / 3, bob, ONE - ONE / 3);
        _open(aid, h, n);
        Clock.skip(15 minutes);
        uint256 before = wrapper.balanceOf(pusher);
        bytes32[] memory proof = Merkle.proof(_leaves(h, n), 0);
        vm.prank(pusher);
        claim.claim(aid, alice, ONE / 3, proof);
        assertEq(wrapper.balanceOf(pusher), before);
        assertEq(wrapper.balanceOf(alice), ONE / 3);
        vm.prank(alice);
        vm.expectRevert(WrapClaim.AlreadyClaimed.selector);
        claim.claim(aid, alice, ONE / 3, proof);
    }

    /// A cancel inside the window can only return a frog to unopened and a carry to its own coin; it moves no token. And the
    /// window is on both clocks: a jumped timestamp does not end the owner's chance to withdraw a wrong root early.
    function test_aCancelMovesNoTokenAndTheWindowIsNotShortenedByTheSequencer() public {
        uint256 aid = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE, bob, ONE);
        _open(aid, h, n);
        Clock.skipTimeOnly(1 hours);
        uint256 before = wrapper.balanceOf(address(claim));
        vm.prank(owner);
        claim.cancel(aid);
        assertEq(wrapper.balanceOf(address(claim)), before);
        assertEq(claim.unopened(), 1);
    }

    // ---- no way to take ----

    /// The claim contract's whole ABI, exactly: tokens leave only through `claim`, to a leaf's holder. There is no pause,
    /// setter, sweep, rescue, redeem or owner transfer. A function added later fails here until somebody decides who may call it.
    function test_theClaimExposesExactlyWhatItMeansTo() public view {
        string[21] memory list = [
            // publishing, claiming, closing
            "count(uint256)", "open(uint256,bytes32,string)", "cancel(uint256)", "claim(uint256,address,uint256,bytes32[])",
            "rollOver(uint256)",
            // the wrapper's deployer-only wiring, forwarded from the registry's owner (docs/v6/deploy-stack.md §1): the claim
            // contract is the wrapper's deployer, so without these a WrapClaim-built wrapper could never take a floor module
            "wireFloorModule(address)", "initMintLimit(uint256)", "proposeMintLimit(uint256)",
            // what it was built with, and constants
            "registry()", "listingId()", "wrapper()", "parentBlockSeconds()", "ONE_ITEM()", "CHALLENGE()", "CLAIM_PERIOD()",
            // the books
            "distribution(uint256)", "claimed(uint256,address)", "carry(address)", "unopened()", "counted(uint256)",
            "leaf(address,uint256)"
        ];
        string memory json = vm.readFile("out/WrapClaim.sol/WrapClaim.json");
        string[] memory actual = vm.parseJsonKeys(json, ".methodIdentifiers");
        uint256 expected = list.length;
        assertEq(actual.length, expected, "the claim contract exposes a function that is not on the list, or lost one that is");
        for (uint256 j = 0; j < expected; ++j) {
            bool found;
            for (uint256 i = 0; i < actual.length; ++i) {
                if (keccak256(bytes(actual[i])) == keccak256(bytes(list[j]))) { found = true; break; }
            }
            assertTrue(found, string.concat("missing: ", list[j]));
        }
    }

    /// The claim contract never holds a ticket: it has no path to `redeem`, so every ticket's owner is somebody who claimed.
    function test_theClaimContractNeverRedeems() public {
        uint256 aid = _frog(0);
        address[] memory one = new address[](1); one[0] = alice;
        uint256[] memory whole = new uint256[](1); whole[0] = ONE;
        _open(aid, one, whole);
        Clock.skip(15 minutes);
        claim.claim(aid, alice, ONE, new bytes32[](0));
        uint256 t = _redeem(alice);
        assertEq(wrapper.ticket(t).owner, alice);
        assertEq(wrapper.ticketCount(), 1);
    }
}
