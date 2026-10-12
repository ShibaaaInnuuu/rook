// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ClaimBase} from "./utils/ClaimBase.sol";
import {Clock} from "./utils/Clock.sol";
import {Merkle} from "./utils/Merkle.sol";
import {WrapClaim} from "../src/WrapClaim.sol";
import {Wrapper} from "../src/Wrapper.sol";

/// Step 2 of docs/v3/wrapper.md, every transition and every refusal: a frog's tokens published by root, held for the
/// challenge, claimed to the holders in the leaves, closed after 30 days, and what nobody claimed carried to the coin's next.
contract WrapClaimTest is ClaimBase {
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    // ---- binding ----

    function test_theClaimAndItsWrapperPointAtEachOtherByConstruction() public {
        assertEq(wrapper.distribution(), address(claim));
        assertEq(address(claim.wrapper()), address(wrapper));
        uint256 aid = _frog(0);
        assertEq(wrapper.balanceOf(address(claim)), ONE, "the frog's whole token lands on the claim contract");
        assertTrue(wrapper.wrapped(aid));
        _assertLedger();
    }

    // ---- counting and opening ----

    function test_aWrappedFrogIsCountedAsUnopenedOnceAndOnlyFromTheWrappersOwnRecord() public {
        uint256 aid = _frog(0);
        assertEq(claim.unopened(), 0, "wrap calls nothing here: the count is taken when someone looks");
        claim.count(aid);
        claim.count(aid);                                   // idempotent
        assertEq(claim.unopened(), 1);
        uint256 notWrapped = _receipt(1);
        vm.expectRevert(WrapClaim.NotWrapped.selector);
        claim.count(notWrapped);
    }

    function test_onlyTheKeeperOpensAndOnlyAWrappedFrogWithARealRootAndList() public {
        uint256 aid = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE / 2, bob, ONE / 2);
        bytes32 root = Merkle.root(_leaves(h, n));
        vm.prank(stranger);
        vm.expectRevert(WrapClaim.NotKeeper.selector);
        claim.open(aid, root, "ipfs://x");
        vm.startPrank(keeper);
        vm.expectRevert(WrapClaim.BadInput.selector);
        claim.open(aid, bytes32(0), "ipfs://x");
        vm.expectRevert(WrapClaim.BadInput.selector);
        claim.open(aid, root, "");
        vm.stopPrank();
        uint256 notWrapped = _receipt(1);
        vm.prank(keeper);
        vm.expectRevert(WrapClaim.NotWrapped.selector);
        claim.open(notWrapped, root, "ipfs://x");

        vm.prank(keeper);
        claim.open(aid, root, "ipfs://x");
        WrapClaim.Distribution memory d = claim.distribution(aid);
        assertTrue(d.open);
        assertEq(d.coin, coinA, "the coin is the vault's record of who paid, not the keeper's");
        assertEq(d.total, ONE, "the total is one frog: fixed by the chain");
        assertEq(claim.unopened(), 0);
        vm.prank(keeper);
        vm.expectRevert(WrapClaim.WrongStage.selector);
        claim.open(aid, root, "ipfs://x");
    }

    // ---- the challenge ----

    function test_nothingIsClaimableForFifteenMinutesOnBothClocks() public {
        uint256 aid = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE / 2, bob, ONE / 2);
        _open(aid, h, n);
        bytes32[] memory proof = Merkle.proof(_leaves(h, n), 0);
        bytes memory held_ = abi.encodeWithSelector(WrapClaim.ChallengeWindow.selector, claim.distribution(aid).challengeEnds);
        vm.expectRevert(held_);
        claim.claim(aid, alice, ONE / 2, proof);
        Clock.skipTimeOnly(1 hours);                        // a sequencer stamping the future moves one clock only
        vm.expectRevert(held_);
        claim.claim(aid, alice, ONE / 2, proof);
        Clock.skip(15 minutes);
        claim.claim(aid, alice, ONE / 2, proof);
        assertEq(wrapper.balanceOf(alice), ONE / 2);
    }

    // ---- claiming ----

    function test_aClaimPaysTheHolderInTheLeafNeverTheCallerAndOnlyOnce() public {
        uint256 aid = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, 3 * ONE / 4, bob, ONE / 4);
        _open(aid, h, n);
        Clock.skip(15 minutes);
        bytes32[] memory proof = Merkle.proof(_leaves(h, n), 0);
        vm.prank(stranger);
        claim.claim(aid, alice, 3 * ONE / 4, proof);
        assertEq(wrapper.balanceOf(alice), 3 * ONE / 4);
        assertEq(wrapper.balanceOf(stranger), 0, "the caller gets nothing");
        vm.expectRevert(WrapClaim.AlreadyClaimed.selector);
        claim.claim(aid, alice, 3 * ONE / 4, proof);
        bytes32[] memory bobs = Merkle.proof(_leaves(h, n), 1);      // built BEFORE expectRevert: `leaf` is an external call
        vm.expectRevert(WrapClaim.BadProof.selector);
        claim.claim(aid, bob, ONE, bobs);                              // bob's proof, a bigger amount
        _claim(aid, h, n, 1);
        assertEq(wrapper.balanceOf(bob), ONE / 4);
        assertEq(wrapper.balanceOf(address(claim)), 0);
        assertEq(claim.distribution(aid).claimed, ONE);
        _assertLedger();
    }

    function test_aZeroHolderOrZeroAmountIsRefused() public {
        uint256 aid = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE, bob, 0);
        _open(aid, h, n);
        Clock.skip(15 minutes);
        bytes32[] memory proof = Merkle.proof(_leaves(h, n), 1);
        vm.expectRevert(WrapClaim.BadInput.selector);
        claim.claim(aid, bob, 0, proof);
    }

    function test_aRootThatPromisesMoreThanOneFrogCannotPayPastIt() public {
        uint256 aid = _frog(0);
        _frog(1);                                            // a second frog's token sits in the same contract
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE, bob, ONE);
        _open(aid, h, n);
        Clock.skip(15 minutes);
        _claim(aid, h, n, 0);
        bytes32[] memory proof = Merkle.proof(_leaves(h, n), 1);
        vm.expectRevert(WrapClaim.OverTotal.selector);
        claim.claim(aid, bob, ONE, proof);
        assertEq(wrapper.balanceOf(address(claim)), ONE, "the other frog's token is untouched");
    }

    // ---- the owner's one power ----

    function test_theOwnerCancelsAWrongRootInsideItsWindowAndNeverAfter() public {
        uint256 aid = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE, bob, ONE);   // wrong: two frogs' worth
        _open(aid, h, n);
        vm.prank(stranger);
        vm.expectRevert(WrapClaim.NotOwner.selector);
        claim.cancel(aid);
        vm.prank(owner);
        claim.cancel(aid);
        assertFalse(claim.distribution(aid).open);
        assertEq(claim.unopened(), 1, "unopened again, waiting for a corrected root");
        (h, n) = _pair(alice, ONE / 2, bob, ONE / 2);
        _open(aid, h, n);
        Clock.skip(15 minutes);
        vm.prank(owner);
        vm.expectRevert(WrapClaim.WindowOver.selector);
        claim.cancel(aid);
        uint256 neverOpened = _frog(1);                     // made BEFORE the prank: `_frog` makes calls of its own
        vm.prank(owner);
        vm.expectRevert(WrapClaim.WrongStage.selector);
        claim.cancel(neverOpened);
    }

    // ---- closing, and what nobody claimed ----

    function test_claimsCloseAfterThirtyDaysOnBothClocksAndTheRestRollsToTheCoinsNextFrog() public {
        uint256 first = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE / 4, bob, 3 * ONE / 4);
        _open(first, h, n);
        Clock.skip(15 minutes);
        _claim(first, h, n, 0);
        vm.expectRevert(WrapClaim.TooEarly.selector);
        claim.rollOver(first);
        Clock.skipTimeOnly(31 days);                         // one clock only: still open, still too early
        vm.expectRevert(WrapClaim.TooEarly.selector);
        claim.rollOver(first);
        Clock.skip(31 days);
        bytes32[] memory proof = Merkle.proof(_leaves(h, n), 1);
        vm.expectRevert(WrapClaim.ClaimsClosed.selector);
        claim.claim(first, bob, 3 * ONE / 4, proof);
        claim.rollOver(first);
        assertEq(claim.carry(coinA), 3 * ONE / 4, "bob's unclaimed part waits for coin A's next frog");
        vm.expectRevert(WrapClaim.WrongStage.selector);
        claim.rollOver(first);

        uint256 second = _frog(1);
        (h, n) = _pair(alice, ONE, bob, 3 * ONE / 4);
        _open(second, h, n);
        assertEq(claim.distribution(second).total, ONE + 3 * ONE / 4, "the next frog's total carries it");
        assertEq(claim.carry(coinA), 0);
        Clock.skip(15 minutes);
        _claim(second, h, n, 0);
        _claim(second, h, n, 1);
        assertEq(wrapper.balanceOf(address(claim)), 0);
        _assertLedger();
    }

    function test_carryBelongsToItsCoinAndNoOther() public {
        uint256 a = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE / 2, bob, ONE / 2);
        _open(a, h, n);
        Clock.skip(31 days);
        claim.rollOver(a);
        assertEq(claim.carry(coinA), ONE);
        uint256 b = _frogBy(1, coinB, creatorB);
        _open(b, h, n);
        assertEq(claim.distribution(b).coin, coinB);
        assertEq(claim.distribution(b).total, ONE, "coin B's frog carries nothing of coin A's");
        assertEq(claim.carry(coinA), ONE);
    }

    function test_cancellingAFrogThatCarriedReturnsTheCarryToItsCoin() public {
        uint256 a = _frog(0);
        (address[] memory h, uint256[] memory n) = _pair(alice, ONE / 2, bob, ONE / 2);
        _open(a, h, n);
        Clock.skip(31 days);
        claim.rollOver(a);
        uint256 b = _frog(1);
        _open(b, h, n);
        assertEq(claim.carry(coinA), 0);
        vm.prank(owner);
        claim.cancel(b);
        assertEq(claim.carry(coinA), ONE, "the carried token goes back where it was");
    }

    // ---- claimed tokens are the real thing ----

    function test_aClaimedTokenRedeemsForAFrogAndTheLedgerHoldsThroughout() public {
        uint256 aid = _frog(0);
        address[] memory one = new address[](1); one[0] = alice;
        uint256[] memory amt = new uint256[](1); amt[0] = ONE;
        _open(aid, one, amt);
        Clock.skip(15 minutes);
        _claim(aid, one, amt, 0);
        _assertLedger();
        uint256 t = _redeem(alice);
        _assertLedger();
        Clock.skip(2 minutes);
        _resolve(t, keccak256("r"));
        assertEq(wrapper.ticket(t).acquisitionId, aid);
        vm.prank(custodian);
        wrapper.recordDelivery(t, TXID);
        _assertLedger();
        assertEq(wrapper.totalSupply(), 0);
    }

    // ---- the wrapper's wiring, through the claim (docs/v6/deploy-stack.md §1) ----

    /// The claim contract is the wrapper's deployer, so the wrapper's deployer-only calls are reachable only through it, and
    /// through it only by the registry's owner: the timelock wires the floor module, and the wired module then mints.
    function test_theOwnerWiresTheFloorModuleThroughTheClaimAndNobodyElseCan() public {
        address module = address(new StandInModule(address(wrapper)));   // a contract that names this wrapper (B-05)
        assertEq(wrapper.deployer(), address(claim));
        vm.prank(owner);
        vm.expectRevert(Wrapper.NotDeployer.selector);
        wrapper.wireFloorModule(module);               // not even the owner, directly: the wrapper's deployer is the claim
        vm.prank(keeper);
        vm.expectRevert(WrapClaim.NotOwner.selector);
        claim.wireFloorModule(module);
        vm.expectRevert(WrapClaim.NotOwner.selector);
        claim.wireFloorModule(module);
        vm.prank(owner);
        claim.wireFloorModule(module);
        assertEq(wrapper.floorModule(), module);
        vm.prank(owner);
        vm.expectRevert(Wrapper.AlreadyWired.selector);   // the wrapper's own checks still apply behind the passthrough
        claim.wireFloorModule(module);
        vm.prank(module);
        wrapper.moduleMint(alice, 1e17);
        assertEq(wrapper.ethBacked(), 1e17);
        assertEq(wrapper.balanceOf(alice), 1e17);
    }

    /// The daily mint limit: set once by the owner through the claim before anything was minted, lowered at once, raised only
    /// after the wrapper's week on both clocks — and never by the deploy key, which owns nothing.
    function test_theOwnerSetsTheMintLimitThroughTheClaim() public {
        vm.expectRevert(WrapClaim.NotOwner.selector);
        claim.initMintLimit(3);
        vm.prank(owner);
        vm.expectRevert(Wrapper.NotDeployer.selector);
        wrapper.initMintLimit(3);
        vm.prank(owner);
        claim.initMintLimit(3);
        assertEq(wrapper.maxMintsPerDay(), 3);
        _frog(0);                                       // something minted: initMintLimit is closed for good
        vm.prank(owner);
        vm.expectRevert(Wrapper.NotFresh.selector);
        claim.initMintLimit(4);
        vm.expectRevert(WrapClaim.NotOwner.selector);
        claim.proposeMintLimit(2);
        vm.prank(owner);
        claim.proposeMintLimit(2);                      // lowering takes effect at once
        assertEq(wrapper.maxMintsPerDay(), 2);
        vm.prank(owner);
        claim.proposeMintLimit(10);                     // a raise waits
        assertEq(wrapper.maxMintsPerDay(), 2);
        assertEq(wrapper.pendingMintLimit(), 10);
        vm.expectRevert(Wrapper.TooEarly.selector);
        wrapper.acceptMintLimit();
        Clock.skip(7 days);
        wrapper.acceptMintLimit();                      // anyone, on the wrapper, as before
        assertEq(wrapper.maxMintsPerDay(), 10);
    }

    /// A module wired after the first wrap is refused, through the claim as directly: the owner's step comes first (§6, step 0).
    function test_wiringAfterTheFirstMintIsRefusedThroughTheClaimToo() public {
        _frog(0);
        address module = address(new StandInModule(address(wrapper)));
        vm.prank(owner);
        vm.expectRevert(Wrapper.NotFresh.selector);
        claim.wireFloorModule(module);
    }

    // ---- fuzzed allocations ----

    /// Any split of one frog among up to eight holders pays each exactly its leaf, and never a wei past the total.
    function testFuzz_anySplitPaysEachHolderExactlyItsLeaf(uint256 seed, uint8 count_) public {
        uint256 k = bound(count_, 1, 8);
        address[] memory h = new address[](k);
        uint256[] memory n = new uint256[](k);
        uint256 left = ONE;
        for (uint256 i = 0; i < k; ++i) {
            h[i] = address(uint160(uint256(keccak256(abi.encode(seed, i))) | 1));
            n[i] = i == k - 1 ? left : bound(uint256(keccak256(abi.encode(seed, i, "amount"))), 1, left - (k - 1 - i));
            left -= n[i];
        }
        for (uint256 i = 0; i < k; ++i) if (n[i] == 0) return;   // the split above never makes one, but a zero leaf is refused
        uint256 aid = _frog(0);
        _open(aid, h, n);
        Clock.skip(15 minutes);
        for (uint256 i = 0; i < k; ++i) {
            _claim(aid, h, n, i);
            assertEq(wrapper.balanceOf(h[i]), n[i]);
        }
        assertEq(claim.distribution(aid).claimed, ONE);
        assertEq(wrapper.balanceOf(address(claim)), 0);
        _assertLedger();
    }
}

/// Stands in for a floor module in the wiring tests: the one thing `wireFloorModule` reads is that it names the wrapper (B-05).
contract StandInModule {
    address public wrapped;
    constructor(address w) { wrapped = w; }
}
